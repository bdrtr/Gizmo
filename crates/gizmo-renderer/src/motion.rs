//! Per-object motion: where each drawn object was last frame, and the G-buffer's velocity target.
//!
//! Camera motion blur (`PostProcess::motion_blur`) only ever needed the camera's last
//! view-projection. Object motion needs each object's last **model** matrix too — nothing kept one
//! (`InstanceRaw` carries only `model`) — and a per-pixel record of how far the surface moved.
//!
//! - [`MotionHistory`] remembers the model matrix every entity was drawn with, frame to frame.
//! - [`MotionGpu`] uploads the previous matrices in step with the instance buffer, plus this and
//!   last frame's unjittered view-projections, for the G-buffer pass (group 5).
//! - The G-buffer pass writes `velocity` (`Rg16Float`, screen-UV units) = where the surface is
//!   now minus where it was, camera and object motion together. A still object under a still
//!   camera writes exactly zero.

use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// The format of the G-buffer's velocity target. Four bytes: the G-buffer already spends 28 of the
/// 32 bytes per sample a colour pass may.
pub const VELOCITY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

type Cols = [[f32; 4]; 4];

/// The model matrix every entity was drawn with this frame and the one before.
///
/// Advanced once per frame ([`begin_frame`](Self::begin_frame)) however many views draw it: a
/// split screen's second view asks again and gets the same answer instead of a history it just
/// advanced. Owned by the renderer, so two renderers on one thread (tests do this) never share it.
#[derive(Debug, Default)]
pub struct MotionHistory {
    frame: u64,
    entries: HashMap<u32, Entry>,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    seen: u64,
    prev: Cols,
    cur: Cols,
}

impl MotionHistory {
    /// Starts a frame, and forgets anything not drawn for two.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        self.entries.retain(|_, e| e.seen + 2 >= frame);
    }

    /// The matrix `entity` was drawn with last frame, given the one it is drawn with now. An entity
    /// seen for the first time — or after a gap — gets `current` back: no motion, rather than a
    /// streak from wherever it last was.
    pub fn previous(&mut self, entity: u32, current: Cols) -> Cols {
        let frame = self.frame;
        match self.entries.get_mut(&entity) {
            Some(e) if e.seen == frame => e.prev,
            Some(e) if e.seen + 1 == frame => {
                e.prev = e.cur;
                e.cur = current;
                e.seen = frame;
                e.prev
            }
            _ => {
                self.entries.insert(entity, Entry { seen: frame, prev: current, cur: current });
                current
            }
        }
    }
}

/// The uniform block of `gbuffer.wgsl`'s group 5.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct MotionUniform {
    view_proj: Cols,
    prev_view_proj: Cols,
}

/// Group 5 of the G-buffer pass: the view-projections and the previous model per instance.
#[non_exhaustive]
pub struct MotionGpu {
    /// The group's layout.
    pub layout: wgpu::BindGroupLayout,
    /// The group.
    pub bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    previous: wgpu::Buffer,
    capacity: usize,
}

impl MotionGpu {
    /// The layout: binding 0 the two view-projections, binding 1 the previous models.
    pub fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("motion_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }

    /// Builds the group with room for `capacity` instances.
    pub fn new(device: &wgpu::Device, capacity: usize) -> Self {
        let layout = Self::layout(device);
        let identity = gizmo_math::Mat4::IDENTITY.to_cols_array_2d();
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("motion_view_projs"),
            contents: bytemuck::bytes_of(&MotionUniform { view_proj: identity, prev_view_proj: identity }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let capacity = capacity.max(1);
        let previous = Self::previous_buffer(device, capacity);
        let bind_group = Self::bind(device, &layout, &uniform, &previous);
        Self { layout, bind_group, uniform, previous, capacity }
    }

    fn previous_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("motion_previous_models"),
            size: (capacity * std::mem::size_of::<Cols>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, uniform: &wgpu::Buffer, previous: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("motion_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: previous.as_entire_binding() },
            ],
        })
    }

    /// Uploads this frame's and last frame's **unjittered** view-projections.
    pub fn set_view_projs(&self, queue: &wgpu::Queue, view_proj: Cols, prev_view_proj: Cols) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&MotionUniform { view_proj, prev_view_proj }));
    }

    /// Uploads the previous model of every instance, in instance-buffer order. Grows the buffer
    /// when it must.
    pub fn upload_previous(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, previous_models: &[Cols]) {
        if previous_models.len() > self.capacity {
            self.capacity = previous_models.len().next_power_of_two();
            self.previous = Self::previous_buffer(device, self.capacity);
            self.bind_group = Self::bind(device, &self.layout, &self.uniform, &self.previous);
        }
        if !previous_models.is_empty() {
            queue.write_buffer(&self.previous, 0, bytemuck::cast_slice(previous_models));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32) -> Cols {
        gizmo_math::Mat4::from_translation(gizmo_math::Vec3::new(x, 0.0, 0.0)).to_cols_array_2d()
    }

    /// The history advances once per frame, however often it is asked; a newcomer has no motion; and
    /// an entity missing for a frame starts over rather than streaking from where it was.
    #[test]
    fn history_advances_once_per_frame_and_forgets_gaps() {
        let mut h = MotionHistory::default();
        h.begin_frame();
        assert_eq!(h.previous(7, at(0.0)), at(0.0), "first sight: no motion");
        h.begin_frame();
        assert_eq!(h.previous(7, at(1.0)), at(0.0), "last frame's matrix");
        assert_eq!(h.previous(7, at(1.0)), at(0.0), "a second view this frame gets the same answer");
        h.begin_frame();
        assert_eq!(h.previous(7, at(2.0)), at(1.0));
        h.begin_frame(); // not drawn this frame
        h.begin_frame();
        assert_eq!(h.previous(7, at(9.0)), at(9.0), "after a gap: no streak from x = 2");
    }
}
