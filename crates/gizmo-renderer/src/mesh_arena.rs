//! Shared vertex and index buffers: many meshes in one buffer, each at its own range.
//!
//! A [`Mesh`] made the ordinary way has a buffer of its own, so every draw of it binds that buffer
//! — and wgpu validates and tracks each bind, on the CPU, when the frame's commands are finished.
//! A city merged into 14 600 meshes is 14 600 vertex buffers, and as many binds a frame. Meshes
//! placed here share a few large buffers ("slabs"): a run of draws over one slab binds it once, and
//! draws that also share a pipeline and a material can be one indirect multi-draw.
//!
//! A mesh made here is an ordinary [`Mesh`] whose [`first_vertex`](Mesh::first_vertex) and
//! [`first_index`](Mesh::first_index) say where in [`vbuf`](Mesh::vbuf) / [`ibuf`](Mesh::ibuf) it
//! starts; the draw path honours them for every mesh (a mesh with its own buffer starts at 0). Its
//! ranges are held for as long as any clone of the mesh lives, and go back to the arena with the
//! last one.
//!
//! The arena belongs to a [`Renderer`](crate::Renderer) — [`Renderer::mesh_arena`](crate::Renderer::mesh_arena)
//! — and writes through its queue: what is placed is on the GPU by the next submission, which is
//! before anything can draw it.

use crate::components::Mesh;
use crate::gpu_types::Vertex;
use gizmo_math::Vec3;
use std::ops::Range;
use std::sync::{Arc, Mutex, Weak};

/// Vertices a vertex slab holds: 512 Ki of [`Vertex`]'s 104 bytes, 52 MiB.
const VERTEX_SLAB: u32 = 1 << 19;
/// Indices an index slab holds: 8 Mi `u32`s, 32 MiB.
const INDEX_SLAB: u32 = 1 << 23;

/// Many meshes' vertices and indices in a few shared buffers — see the [module](self).
///
/// Cloning hands out another handle to the same arena.
#[derive(Clone)]
pub struct MeshArena {
    pools: Arc<Mutex<Pools>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// How much of an arena is in use — for a game to print, and for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ArenaStats {
    /// Vertex buffers the arena has made.
    pub vertex_slabs: usize,
    /// Vertices placed and not yet given back.
    pub vertices: u64,
    /// Index buffers the arena has made.
    pub index_slabs: usize,
    /// Indices placed and not yet given back.
    pub indices: u64,
}

struct Pools {
    vertices: Pool,
    indices: Pool,
}

struct Pool {
    /// Bytes per element.
    stride: u64,
    /// Elements a slab holds unless one mesh needs more.
    slab_elements: u32,
    usage: wgpu::BufferUsages,
    label: &'static str,
    slabs: Vec<Slab>,
}

struct Slab {
    buffer: Arc<wgpu::Buffer>,
    /// Free ranges, in elements, sorted by start and never adjacent (freed neighbours merge).
    free: Vec<Range<u32>>,
    capacity: u32,
}

/// A range of one slab, held by the meshes that draw from it — given back when dropped.
pub(crate) struct Lease {
    pools: Weak<Mutex<Pools>>,
    index: bool,
    slab: usize,
    range: Range<u32>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let Some(pools) = self.pools.upgrade() else { return };
        let mut pools = pools.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let pool = if self.index { &mut pools.indices } else { &mut pools.vertices };
        if let Some(slab) = pool.slabs.get_mut(self.slab) {
            slab.give_back(self.range.clone());
        }
    }
}

impl Slab {
    /// The first free range long enough for `len`, taken from its start.
    fn take(&mut self, len: u32) -> Option<u32> {
        let at = self.free.iter().position(|r| r.end - r.start >= len)?;
        let start = self.free[at].start;
        self.free[at].start += len;
        if self.free[at].is_empty() {
            self.free.remove(at);
        }
        Some(start)
    }

    fn give_back(&mut self, range: Range<u32>) {
        if range.is_empty() {
            return;
        }
        let at = self.free.partition_point(|r| r.start < range.start);
        self.free.insert(at, range);
        // Merge with the neighbour after, then the one before.
        if at + 1 < self.free.len() && self.free[at].end == self.free[at + 1].start {
            self.free[at].end = self.free[at + 1].end;
            self.free.remove(at + 1);
        }
        if at > 0 && self.free[at - 1].end == self.free[at].start {
            self.free[at - 1].end = self.free[at].end;
            self.free.remove(at);
        }
    }

    fn used(&self) -> u64 {
        u64::from(self.capacity) - self.free.iter().map(|r| u64::from(r.end - r.start)).sum::<u64>()
    }
}

impl Pool {
    /// `len` elements somewhere: the first slab with room, or a new one — of the usual size, or
    /// as large as `len` when that is more.
    fn place(&mut self, device: &wgpu::Device, len: u32) -> (usize, u32) {
        for (i, slab) in self.slabs.iter_mut().enumerate() {
            if let Some(start) = slab.take(len) {
                return (i, start);
            }
        }
        let capacity = self.slab_elements.max(len);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(self.label),
            size: u64::from(capacity) * self.stride,
            usage: self.usage,
            mapped_at_creation: false,
        });
        let mut slab = Slab { buffer: Arc::new(buffer), free: vec![Range { start: 0, end: capacity }], capacity };
        let start = slab.take(len).expect("a new slab holds what it was sized for");
        self.slabs.push(slab);
        (self.slabs.len() - 1, start)
    }
}

/// Where a placed range is: the buffer, its first element, and the lease that keeps it.
pub(crate) struct Placed {
    pub(crate) buffer: Arc<wgpu::Buffer>,
    pub(crate) first: u32,
    pub(crate) lease: Lease,
}

impl MeshArena {
    /// An empty arena over `device`, writing through `queue`. Slabs are made as they are needed.
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let pools = Pools {
            vertices: Pool {
                stride: std::mem::size_of::<Vertex>() as u64,
                slab_elements: VERTEX_SLAB,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                label: "MeshArena vertices",
                slabs: Vec::new(),
            },
            indices: Pool {
                stride: 4,
                slab_elements: INDEX_SLAB,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                label: "MeshArena indices",
                slabs: Vec::new(),
            },
        };
        Self { pools: Arc::new(Mutex::new(pools)), device: device.clone(), queue: queue.clone() }
    }

    fn place(&self, index: bool, bytes: &[u8], len: u32) -> Placed {
        let mut pools = self.pools.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let pool = if index { &mut pools.indices } else { &mut pools.vertices };
        let (slab, first) = pool.place(&self.device, len);
        let buffer = pool.slabs[slab].buffer.clone();
        let offset = u64::from(first) * pool.stride;
        drop(pools);
        self.queue.write_buffer(&buffer, offset, bytes);
        Placed {
            buffer,
            first,
            lease: Lease { pools: Arc::downgrade(&self.pools), index, slab, range: first..first + len },
        }
    }

    pub(crate) fn place_vertices(&self, vertices: &[Vertex]) -> Placed {
        self.place(false, bytemuck::cast_slice(vertices), vertices.len() as u32)
    }

    // Indexed meshes are deduplicated by meshopt, which the browser build does not have.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn place_indices(&self, indices: &[u32]) -> Placed {
        self.place(true, bytemuck::cast_slice(indices), indices.len() as u32)
    }

    /// A mesh from a triangle list (every three vertices one triangle), placed here — what
    /// [`Mesh::from_vertices`] makes with a buffer of its own, LOD included.
    #[must_use]
    pub fn triangle_list(&self, vertices: &[Vertex], source: impl Into<String>) -> Mesh {
        Mesh::triangle_list_in(self, vertices, Vec3::ZERO, source.into())
    }

    /// A mesh with its vertices deduplicated and an index list, placed here — what
    /// [`Mesh::new_indexed`] makes with buffers of its own. Indices are always 32-bit here, so that
    /// every mesh's share one index buffer.
    #[must_use]
    pub fn indexed(&self, vertices: &[Vertex], center_offset: Vec3, source: impl Into<String>) -> Mesh {
        Mesh::indexed_in(self, vertices, center_offset, source.into())
    }

    /// What the arena holds now.
    #[must_use]
    pub fn stats(&self) -> ArenaStats {
        let pools = self.pools.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        ArenaStats {
            vertex_slabs: pools.vertices.slabs.len(),
            vertices: pools.vertices.slabs.iter().map(Slab::used).sum(),
            index_slabs: pools.indices.slabs.len(),
            indices: pools.indices.slabs.iter().map(Slab::used).sum(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Slab;
    use std::ops::Range;

    fn slab(capacity: u32) -> Slab {
        // The buffer is never touched by these tests; a slab's bookkeeping is all they read.
        Slab { buffer: dummy_buffer(), free: vec![Range { start: 0, end: capacity }], capacity }
    }

    fn dummy_buffer() -> std::sync::Arc<wgpu::Buffer> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::NOOP,
            backend_options: wgpu::BackendOptions {
                noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
                ..Default::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
        let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a noop device");
        std::sync::Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 4,
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        }))
    }

    fn noop() -> (wgpu::Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::NOOP,
            backend_options: wgpu::BackendOptions {
                noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
                ..Default::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a noop device")
    }

    fn triangle(at: f32) -> Vec<crate::gpu_types::Vertex> {
        [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]
            .into_iter()
            .map(|[x, y]| crate::gpu_types::Vertex { position: [x + at, y, 0.0], ..Default::default() })
            .collect()
    }

    /// Meshes made in an arena share its buffers, each at its own range; a dropped mesh's range is
    /// given back — when its last clone goes — and the next mesh takes it.
    #[test]
    fn arena_meshes_share_a_buffer_at_their_own_ranges() {
        let (device, queue) = noop();
        let arena = super::MeshArena::new(&device, &queue);
        let a = arena.triangle_list(&triangle(0.0), "a");
        let b = arena.triangle_list(&triangle(5.0), "b");
        assert!(std::sync::Arc::ptr_eq(&a.vbuf, &b.vbuf), "one buffer");
        assert_eq!((a.first_vertex, b.first_vertex), (0, 3));
        assert_eq!((a.vertex_count, b.vertex_count), (3, 3));
        assert_eq!(arena.stats().vertices, 6);
        let a_again = a.clone();
        drop(a);
        assert_eq!(arena.stats().vertices, 6, "a clone still holds the range");
        drop(a_again);
        assert_eq!(arena.stats().vertices, 3);
        let c = arena.triangle_list(&triangle(9.0), "c");
        assert_eq!(c.first_vertex, 0, "the freed range is taken again");
        // Indexed: its own range of a shared 32-bit index buffer, its vertices deduplicated.
        let quad: Vec<_> = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
            .into_iter()
            .map(|[x, y]| crate::gpu_types::Vertex { position: [x, y, 0.0], ..Default::default() })
            .collect();
        let first = arena.indexed(&quad, gizmo_math::Vec3::ZERO, "quad 1");
        let second = arena.indexed(&quad, gizmo_math::Vec3::ZERO, "quad 2");
        #[cfg(not(target_arch = "wasm32"))]
        {
            assert_eq!(first.index_format, wgpu::IndexFormat::Uint32);
            assert_eq!((first.vertex_count, first.index_count), (4, 6));
            assert_eq!(second.first_index, first.first_index + 6);
            assert_eq!(second.first_vertex, first.first_vertex + 4);
            assert!(std::sync::Arc::ptr_eq(first.ibuf.as_ref().expect("indexed"), second.ibuf.as_ref().expect("indexed")));
        }
        drop((b, c, first, second));
        assert_eq!(arena.stats().vertices, 0);
        assert_eq!(arena.stats().indices, 0);
    }

    /// A mesh larger than a slab gets one sized for it; smaller ones keep going into the first
    /// slab with room.
    #[test]
    fn a_mesh_larger_than_a_slab_gets_one_of_its_own() {
        let (device, _queue) = noop();
        let mut pool = super::Pool {
            stride: 4,
            slab_elements: 100,
            usage: wgpu::BufferUsages::VERTEX,
            label: "test",
            slabs: Vec::new(),
        };
        assert_eq!(pool.place(&device, 60), (0, 0));
        assert_eq!(pool.place(&device, 150), (1, 0), "too big for a slab of 100: one of 150");
        assert_eq!(pool.slabs[1].capacity, 150);
        assert_eq!(pool.place(&device, 40), (0, 60), "back to the first slab with room");
        assert_eq!(pool.place(&device, 10), (2, 0), "neither has room: a new slab");
    }

    /// Ranges are taken first-fit, given back in any order, and freed neighbours merge back into
    /// one range — so a slab emptied in any order is one free range again.
    #[test]
    fn a_slab_takes_and_gives_back_ranges() {
        let mut s = slab(100);
        let a = s.take(10).expect("room");
        let b = s.take(20).expect("room");
        let c = s.take(30).expect("room");
        assert_eq!((a, b, c), (0, 10, 30));
        assert_eq!(s.take(41), None, "40 left");
        s.give_back(b..b + 20);
        assert_eq!(s.take(15), Some(10), "first fit: the hole b left");
        s.give_back(10..25);
        s.give_back(a..a + 10);
        s.give_back(c..c + 30);
        assert_eq!(s.free, vec![Range { start: 0, end: 100 }], "every range back, merged into one");
        assert_eq!(s.used(), 0);
    }
}
