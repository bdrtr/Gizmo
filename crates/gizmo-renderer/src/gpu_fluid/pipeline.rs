use wgpu::util::DeviceExt;
use crate::gpu_types::Vertex;

pub struct FluidPipelines {
    pub compute_bind_group_layout: wgpu::BindGroupLayout,
    pub compute_bind_group: wgpu::BindGroup,
    
    pub pipeline_clear: wgpu::ComputePipeline,
    pub pipeline_hash: wgpu::ComputePipeline,
    pub pipeline_sort: wgpu::ComputePipeline,
    pub pipeline_offsets: wgpu::ComputePipeline,
    pub pipeline_density: wgpu::ComputePipeline,
    pub pipeline_integrate: wgpu::ComputePipeline,
    
    pub render_bind_group: wgpu::BindGroup,
    pub render_pipeline: wgpu::RenderPipeline,
}

pub fn create_fluid_pipelines(
    device: &wgpu::Device,
    global_bind_group_layout: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
    params_buffer: &wgpu::Buffer,
    particles_buffer: &wgpu::Buffer,
    grid_buffer: &wgpu::Buffer,
    colliders_buffer: &wgpu::Buffer,
    sort_buffer: &wgpu::Buffer,
    sort_params_buffer: &wgpu::Buffer,
) -> FluidPipelines {
    let compute_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
            label: Some("fluid_compute_layout"),
        });

    let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &compute_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: particles_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: grid_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: colliders_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: sort_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 5, resource: sort_params_buffer.as_entire_binding() },
        ],
        label: Some("fluid_compute_bg"),
    });

    let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Fluid Compute Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fluid_compute.wgsl").into()),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Fluid Compute Pipeline Layout"),
        bind_group_layouts: &[&compute_bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline_clear = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("fluid_clear"), layout: Some(&pipeline_layout), module: &compute_shader, entry_point: "clear_grid",
    });
    let pipeline_hash = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("fluid_hash"), layout: Some(&pipeline_layout), module: &compute_shader, entry_point: "hash_pass",
    });
    let pipeline_sort = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("fluid_sort"), layout: Some(&pipeline_layout), module: &compute_shader, entry_point: "bitonic_sort_pass",
    });
    let pipeline_offsets = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("fluid_offsets"), layout: Some(&pipeline_layout), module: &compute_shader, entry_point: "grid_offsets_pass",
    });
    let pipeline_density = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Fluid Density Pipeline"), layout: Some(&pipeline_layout), module: &compute_shader, entry_point: "calc_density",
    });
    let pipeline_integrate = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Fluid Integrate Pipeline"), layout: Some(&pipeline_layout), module: &compute_shader, entry_point: "integrate",
    });

    // Render pipeline
    let render_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Fluid Render BG Layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }
        ],
    });
    let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Fluid Render BG"),
        layout: &render_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 1, resource: particles_buffer.as_entire_binding() },
        ],
    });

    let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Fluid Render Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fluid_render.wgsl").into()),
    });

    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Fluid Render Pipeline Layout"),
        bind_group_layouts: &[global_bind_group_layout, &render_bind_group_layout],
        push_constant_ranges: &[],
    });

    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Fluid Render Pipeline"),
        layout: Some(&render_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &render_shader, entry_point: "vs_main",
            buffers: &[Vertex::desc()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &render_shader, entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(), multiview: None,
    });

    FluidPipelines {
        compute_bind_group_layout,
        compute_bind_group,
        pipeline_clear,
        pipeline_hash,
        pipeline_sort,
        pipeline_offsets,
        pipeline_density,
        pipeline_integrate,
        render_bind_group,
        render_pipeline,
    }
}
