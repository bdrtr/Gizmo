//! How long the parts of a frame take **on the GPU**: timestamps written between them, read back a
//! few frames later without waiting for them.
//!
//! The CPU's half of a frame is timed with the clock (`Renderer::draw_laps`); the GPU runs the
//! recorded commands later and elsewhere, and a frame bound by the GPU shows only as a CPU waiting —
//! not as which pass it waits for. [`GpuTimer`] writes a timestamp at each [`mark`](GpuTimer::mark)
//! (`wgpu::CommandEncoder::write_timestamp`), resolves the frame's marks into a buffer at
//! [`end_frame`](GpuTimer::end_frame), and [`collect`](GpuTimer::collect) reads finished frames back
//! from a ring of three — mapped when the GPU is done with them, never waited on — as one
//! `(label, time)` per mark: the time since the mark before it.
//!
//! Needs `TIMESTAMP_QUERY` and `TIMESTAMP_QUERY_INSIDE_ENCODERS`, which the renderer asks for where the
//! adapter has them (a desktop's and most phones' Vulkan, not a browser): without them there is no
//! timer and marks cost nothing.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Timestamps a frame can hold.
const CAPACITY: u32 = 64;
/// Frames in flight between a frame's resolve and its read-back.
const SLOTS: usize = 3;

const FREE: u8 = 0;
const COPIED: u8 = 1;
const MAPPING: u8 = 2;
const MAPPED: u8 = 3;

struct Slot {
    buffer: wgpu::Buffer,
    labels: Vec<&'static str>,
    state: Arc<AtomicU8>,
}

/// GPU timestamps between the parts of a frame — see the [module](self).
pub struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    next: usize,
    labels: Vec<&'static str>,
    /// Nanoseconds a timestamp tick is.
    period: f32,
}

impl GpuTimer {
    /// The features a timer needs.
    pub const FEATURES: wgpu::Features =
        wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

    /// A timer, when the device has [`FEATURES`](Self::FEATURES).
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(Self::FEATURES) {
            return None;
        }
        let bytes = u64::from(CAPACITY) * 8;
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("gpu timer"),
            ty: wgpu::QueryType::Timestamp,
            count: CAPACITY,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu timer resolve"),
            size: bytes,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..SLOTS)
            .map(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpu timer read-back"),
                    size: bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                labels: Vec::new(),
                state: Arc::new(AtomicU8::new(FREE)),
            })
            .collect();
        Some(Self { queries, resolve, slots, next: 0, labels: Vec::new(), period: queue.get_timestamp_period() })
    }

    /// Writes a timestamp: the end of the part called `label`, which began at the mark before it.
    pub fn mark(&mut self, encoder: &mut wgpu::CommandEncoder, label: &'static str) {
        if (self.labels.len() as u32) < CAPACITY {
            encoder.write_timestamp(&self.queries, self.labels.len() as u32);
            self.labels.push(label);
        }
    }

    /// Resolves this frame's marks into the next free slot of the ring, in `encoder` — which must be
    /// submitted after every encoder that marked. A frame whose slot is still being read is dropped.
    pub fn end_frame(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let n = self.labels.len() as u32;
        let slot = &mut self.slots[self.next];
        if n < 2 || slot.state.load(Ordering::Acquire) != FREE {
            self.labels.clear();
            return;
        }
        encoder.resolve_query_set(&self.queries, 0..n, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &slot.buffer, 0, u64::from(n) * 8);
        slot.labels = std::mem::take(&mut self.labels);
        slot.state.store(COPIED, Ordering::Release);
        self.next = (self.next + 1) % SLOTS;
    }

    /// Reads back every frame the GPU has finished, into `out` as `(label, time)`: each mark's time
    /// since the one before it. Frames resolved since the last call (their encoder submitted since)
    /// are asked to map; none is waited for. Call once a frame, before the frame's first mark.
    pub fn collect(&mut self, device: &wgpu::Device, out: &mut Vec<(&'static str, std::time::Duration)>) {
        for slot in &mut self.slots {
            if slot.state.load(Ordering::Acquire) == COPIED {
                slot.state.store(MAPPING, Ordering::Release);
                let state = slot.state.clone();
                slot.buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                    state.store(if result.is_ok() { MAPPED } else { FREE }, Ordering::Release);
                });
            }
        }
        let _ = device.poll(wgpu::PollType::Poll);
        for slot in &mut self.slots {
            if slot.state.load(Ordering::Acquire) != MAPPED {
                continue;
            }
            {
                let n = slot.labels.len();
                if let Ok(data) = slot.buffer.slice(..(n as u64 * 8)).get_mapped_range() {
                    let ticks: &[u64] = bytemuck::cast_slice(&data);
                    for i in 1..n.min(ticks.len()) {
                        let ns = ticks[i].saturating_sub(ticks[i - 1]) as f64 * f64::from(self.period);
                        out.push((slot.labels[i], std::time::Duration::from_nanos(ns as u64)));
                    }
                }
            }
            slot.buffer.unmap();
            slot.labels.clear();
            slot.state.store(FREE, Ordering::Release);
        }
    }
}
