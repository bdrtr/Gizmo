use wgpu::util::DeviceExt;

use crate::pipeline::SceneState;

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshBoundsRaw {
    pub world_center: [f32; 3],
    pub radius: f32,
}

/// Matches wgpu indirect draw args layout exactly (16 bytes).
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DrawIndirectArgs {
    pub vertex_count: u32,
    pub instance_count: u32,
    pub first_vertex: u32,
    pub first_instance: u32,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CullParams {
    num_instances: u32,
    _pad: [u32; 3],
}

pub struct GpuCullState {
    pub mesh_bounds_buffer: wgpu::Buffer,
    pub indirect_buffer: wgpu::Buffer,
    params_buffer: wgpu::Buffer,
    cull_pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    pub capacity: u32,
}

impl GpuCullState {
    pub fn new(device: &wgpu::Device, scene: &SceneState, capacity: u32) -> Self {
        let mesh_bounds_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_cull_bounds"),
            size: capacity as u64 * std::mem::size_of::<MeshBoundsRaw>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let indirect_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_cull_indirect"),
            size: capacity as u64 * std::mem::size_of::<DrawIndirectArgs>() as u64,
            usage: wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gpu_cull_params"),
            contents: bytemuck::bytes_of(&CullParams {
                num_instances: 0,
                _pad: [0; 3],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu_cull_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu_cull_bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: mesh_bounds_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: indirect_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        let shader = crate::pipeline::load_shader_composed(
            device,
            "demo/assets/shaders/mesh_cull.wgsl",
            include_str!("shaders/mesh_cull.wgsl"),
            "Mesh Cull Shader",
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu_cull_layout"),
            bind_group_layouts: &[
                Some(&scene.global_bind_group_layout), // group 0: SceneUniforms (view_proj)
                Some(&bgl),                            // group 1: bounds + indirect + params
            ],
            immediate_size: 0,
        });

        let cull_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gpu_cull_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            mesh_bounds_buffer,
            indirect_buffer,
            params_buffer,
            cull_pipeline,
            bind_group,
            capacity,
        }
    }

    /// Upload per-frame bounds and initial draw args (instance_count = 0); GPU sets it to 1 if visible.
    #[tracing::instrument(skip_all, level = "trace")]
    pub fn prepare(
        &self,
        queue: &wgpu::Queue,
        bounds: &[MeshBoundsRaw],
        draw_args: &[DrawIndirectArgs],
    ) {
        if bounds.is_empty() {
            return;
        }
        let count = clamped_draw_count(bounds.len(), draw_args.len(), self.capacity);
        if count < bounds.len() {
            // Callers that pass more mesh bounds than the GPU buffer can hold (or a
            // shorter draw-args slice) silently lose the tail — those meshes never
            // get culled or drawn. Surface it so a missing object is diagnosable.
            tracing::warn!(
                submitted = bounds.len(),
                draw_args = draw_args.len(),
                capacity = self.capacity,
                uploaded = count,
                "[GpuCull] mesh bounds truncated to capacity; tail meshes will not draw"
            );
        }
        queue.write_buffer(
            &self.mesh_bounds_buffer,
            0,
            bytemuck::cast_slice(&bounds[..count]),
        );
        queue.write_buffer(
            &self.indirect_buffer,
            0,
            bytemuck::cast_slice(&draw_args[..count]),
        );
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::bytes_of(&CullParams {
                num_instances: count as u32,
                _pad: [0; 3],
            }),
        );
    }

    /// Encode the cull compute pass. Must run before any render pass that uses `indirect_buffer`.
    #[tracing::instrument(skip_all, level = "trace")]
    pub fn cull_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        global_bind_group: &wgpu::BindGroup,
        count: u32,
    ) {
        if count == 0 {
            return;
        }
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("GPU Mesh Cull"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&self.cull_pipeline);
        cpass.set_bind_group(0, global_bind_group, &[]);
        cpass.set_bind_group(1, &self.bind_group, &[]);
        cpass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        tracing::trace!(
            instances = count,
            workgroups = count.div_ceil(64),
            "[GpuCull] dispatched frustum cull"
        );
    }

    /// Byte offset into `indirect_buffer` for draw item `i`.
    pub fn indirect_offset(i: usize) -> wgpu::BufferAddress {
        (i * std::mem::size_of::<DrawIndirectArgs>()) as wgpu::BufferAddress
    }
}

/// Instances safe to upload in `prepare`: no more than the shortest of the bounds
/// slice, the draw-args slice, and the GPU buffer capacity, so neither
/// `bounds[..count]` nor `draw_args[..count]` can ever slice out of bounds when
/// the caller passes mismatched lengths.
fn clamped_draw_count(bounds_len: usize, draw_args_len: usize, capacity: u32) -> usize {
    bounds_len.min(draw_args_len).min(capacity as usize)
}

#[cfg(test)]
mod tests {
    use super::clamped_draw_count;

    #[test]
    fn clamped_draw_count_never_exceeds_shortest_input() {
        // Mismatched bounds/draw_args lengths must clamp to the shorter, so slicing
        // `bounds[..count]` / `draw_args[..count]` cannot panic.
        assert_eq!(clamped_draw_count(10, 3, 100), 3);
        assert_eq!(clamped_draw_count(3, 10, 100), 3);
        // Capacity is the binding constraint.
        assert_eq!(clamped_draw_count(10, 10, 4), 4);
        // Everything equal.
        assert_eq!(clamped_draw_count(5, 5, 5), 5);
        // Zero capacity uploads nothing.
        assert_eq!(clamped_draw_count(10, 10, 0), 0);
    }
}
