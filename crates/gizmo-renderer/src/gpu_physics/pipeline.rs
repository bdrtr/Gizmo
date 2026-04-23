use wgpu::util::DeviceExt;
use crate::gpu_types::Vertex;
use super::types::GpuBox;

pub struct PhysicsPipelines {
    pub compute_bind_group_layout: wgpu::BindGroupLayout,
    pub compute_bind_group: wgpu::BindGroup,
    pub pipeline_clear: wgpu::ComputePipeline,
    pub pipeline_build: wgpu::ComputePipeline,
    pub pipeline_solve: wgpu::ComputePipeline,
    pub pipeline_integrate: wgpu::ComputePipeline,

    pub render_pipeline: wgpu::RenderPipeline,

    pub culling_bind_group_layout: wgpu::BindGroupLayout,
    pub culling_bind_group: wgpu::BindGroup,
    pub pipeline_culling: wgpu::ComputePipeline,
}

pub fn create_physics_pipelines(
    device: &wgpu::Device,
    global_bind_group_layout: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    params_buffer: &wgpu::Buffer,
    boxes_buffer: &wgpu::Buffer,
    grid_heads_buffer: &wgpu::Buffer,
    linked_nodes_buffer: &wgpu::Buffer,
    colliders_buffer: &wgpu::Buffer,
    awake_flags_buffer: &wgpu::Buffer,
    culled_boxes_buffer: &wgpu::Buffer,
    indirect_buffer: &wgpu::Buffer,
) -> PhysicsPipelines {
    let compute_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
            label: Some("physics_compute_layout"),
        });

    let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &compute_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: boxes_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: grid_heads_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: linked_nodes_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: colliders_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 5, resource: awake_flags_buffer.as_entire_binding() },
        ],
        label: Some("physics_compute_bind_group"),
    });

    let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Physics Compute Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/physics_compute.wgsl").into()),
    });

    let compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Physics Compute Pipeline Layout"),
        bind_group_layouts: &[&compute_bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline_clear = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("Physics Clear"), layout: Some(&compute_pipeline_layout), module: &compute_shader, entry_point: "clear_grid" });
    let pipeline_build = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("Physics Build"), layout: Some(&compute_pipeline_layout), module: &compute_shader, entry_point: "build_grid" });
    let pipeline_solve = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("Physics Solve"), layout: Some(&compute_pipeline_layout), module: &compute_shader, entry_point: "solve_collisions_safe" });
    let pipeline_integrate = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("Physics Integrate"), layout: Some(&compute_pipeline_layout), module: &compute_shader, entry_point: "integrate" });

    let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Physics Render Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/physics_render.wgsl").into()),
    });

    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Physics Render Pipeline Layout"),
        bind_group_layouts: &[global_bind_group_layout],
        push_constant_ranges: &[],
    });

    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Physics Render Pipeline"),
        layout: Some(&render_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &render_shader, entry_point: "vs_main",
            buffers: &[Vertex::desc(), GpuBox::desc()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &render_shader, entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState { format: output_format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, front_face: wgpu::FrontFace::Ccw, cull_mode: Some(wgpu::Face::Back), ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState { format: depth_format, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::Less, stencil: wgpu::StencilState::default(), bias: wgpu::DepthBiasState::default() }),
        multisample: wgpu::MultisampleState::default(), multiview: None,
    });

    let culling_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
        ],
        label: Some("physics_culling_layout"),
    });

    let culling_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &culling_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: boxes_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: culled_boxes_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: indirect_buffer.as_entire_binding() },
        ],
        label: Some("physics_culling_bind_group"),
    });

    let culling_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Physics Culling Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/physics_culling.wgsl").into()),
    });

    let culling_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Physics Culling Pipeline Layout"),
        bind_group_layouts: &[global_bind_group_layout, &culling_bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline_culling = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Physics Culling Pipeline"), layout: Some(&culling_pipeline_layout), module: &culling_shader, entry_point: "cull_main",
    });

    PhysicsPipelines {
        compute_bind_group_layout,
        compute_bind_group,
        pipeline_clear,
        pipeline_build,
        pipeline_solve,
        pipeline_integrate,
        render_pipeline,
        culling_bind_group_layout,
        culling_bind_group,
        pipeline_culling,
    }
}
