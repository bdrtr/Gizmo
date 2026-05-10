use wgpu::util::DeviceExt;

use crate::pipeline::SceneState;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshBoundsRaw {
    pub world_center: [f32; 3],
    pub radius: f32,
}

/// Matches wgpu indirect draw args layout exactly (16 bytes).
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
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

        let shader = crate::pipeline::load_shader(
            device,
            "demo/assets/shaders/mesh_cull.wgsl",
            include_str!("shaders/mesh_cull.wgsl"),
            "Mesh Cull Shader",
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu_cull_layout"),
            bind_group_layouts: &[
                &scene.global_bind_group_layout, // group 0: SceneUniforms (view_proj)
                &bgl,                            // group 1: bounds + indirect + params
            ],
            push_constant_ranges: &[],
        });

        let cull_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gpu_cull_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "main",
            compilation_options: Default::default(),
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
    pub fn prepare(
        &self,
        queue: &wgpu::Queue,
        bounds: &[MeshBoundsRaw],
        draw_args: &[DrawIndirectArgs],
    ) {
        if bounds.is_empty() {
            return;
        }
        let count = bounds.len().min(self.capacity as usize);
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
    }

    /// Byte offset into `indirect_buffer` for draw item `i`.
    pub fn indirect_offset(i: usize) -> wgpu::BufferAddress {
        (i * std::mem::size_of::<DrawIndirectArgs>()) as wgpu::BufferAddress
    }
}
