use super::types::GpuBox;
use crate::gpu_types::Vertex;

/// Every pipeline the GPU rigid-body solver runs: one compute pass per stage of a step, plus the
/// draw and the frustum-culling pass that feeds it.
///
/// The stages must run in this order, each as its own dispatch, because each reads what the
/// previous one wrote across the whole buffer — a single fused kernel would need a global barrier
/// that a compute dispatch does not have.
pub struct PhysicsPipelines {
    /// Layout of the group holding every simulation buffer.
    pub compute_bind_group_layout: wgpu::BindGroupLayout,
    /// That group.
    pub compute_bind_group: wgpu::BindGroup,
    /// Stage 1: clear the broadphase grid.
    pub pipeline_clear: wgpu::ComputePipeline,
    /// Stage 2: bin every body into the grid.
    pub pipeline_build: wgpu::ComputePipeline,
    /// Stage 3: turn candidate pairs into contact manifolds.
    pub pipeline_narrowphase: wgpu::ComputePipeline,
    /// Stage 4: the contact solver, run for several iterations.
    pub pipeline_solve: wgpu::ComputePipeline,
    /// Stage 6: integrate velocities into positions, and update the sleep state.
    pub pipeline_integrate: wgpu::ComputePipeline,
    /// Stage 5: the joint solver.
    pub pipeline_solve_joints: wgpu::ComputePipeline,

    /// The instanced box draw, reading the simulation buffer directly.
    pub render_pipeline: wgpu::RenderPipeline,

    /// Layout of the culling pass's group.
    pub culling_bind_group_layout: wgpu::BindGroupLayout,
    /// That group.
    pub culling_bind_group: wgpu::BindGroup,
    /// Frustum culling on the GPU, writing the visible subset and an indirect draw count — so the
    /// CPU never learns how many bodies were visible, and never has to.
    pub pipeline_culling: wgpu::ComputePipeline,
}

/// Builds every physics pipeline and bind group over the given simulation buffers.
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
    joints_buffer: &wgpu::Buffer,
    box_contacts_buffer: &wgpu::Buffer,
    culled_boxes_buffer: &wgpu::Buffer,
    indirect_buffer: &wgpu::Buffer,
) -> PhysicsPipelines {
    let compute_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
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
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            label: Some("physics_compute_layout"),
        });

    let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &compute_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: boxes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: grid_heads_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: linked_nodes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: colliders_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: awake_flags_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: joints_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: box_contacts_buffer.as_entire_binding(),
            },
        ],
        label: Some("physics_compute_bind_group"),
    });

    let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Physics Compute Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/physics_compute.wgsl").into()),
    });

    let compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Physics Compute Pipeline Layout"),
        bind_group_layouts: &[Some(&compute_bind_group_layout)],
        immediate_size: 0,
    });

    let pipeline_clear = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Physics Clear"),
        layout: Some(&compute_pipeline_layout),
        module: &compute_shader,
        entry_point: Some("clear_grid"),
        compilation_options: Default::default(),
        cache: None,
    });
    let pipeline_build = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Physics Build"),
        layout: Some(&compute_pipeline_layout),
        module: &compute_shader,
        entry_point: Some("build_grid"),
        compilation_options: Default::default(),
        cache: None,
    });
    let pipeline_narrowphase = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Physics Narrowphase"),
        layout: Some(&compute_pipeline_layout),
        module: &compute_shader,
        entry_point: Some("narrowphase"),
        compilation_options: Default::default(),
        cache: None,
    });
    let pipeline_solve = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Physics Solve"),
        layout: Some(&compute_pipeline_layout),
        module: &compute_shader,
        entry_point: Some("solve_collisions_safe"),
        compilation_options: Default::default(),
        cache: None,
    });
    let pipeline_integrate = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Physics Integrate"),
        layout: Some(&compute_pipeline_layout),
        module: &compute_shader,
        entry_point: Some("integrate"),
        compilation_options: Default::default(),
        cache: None,
    });
    let pipeline_solve_joints = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Physics Solve Joints"),
        layout: Some(&compute_pipeline_layout),
        module: &compute_shader,
        entry_point: Some("solve_joints"),
        compilation_options: Default::default(),
        cache: None,
    });

    // Composed so physics_render.wgsl can `#import gizmo::common::{SceneUniforms}`.
    let render_shader = crate::pipeline::load_shader_composed(
        device,
        "crates/gizmo-renderer/src/shaders/physics_render.wgsl",
        include_str!("../shaders/physics_render.wgsl"),
        "Physics Render Shader",
    );

    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Physics Render Pipeline Layout"),
        bind_group_layouts: &[Some(global_bind_group_layout)],
        immediate_size: 0,
    });

    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Physics Render Pipeline"),
        layout: Some(&render_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &render_shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(Vertex::desc()), Some(GpuBox::desc())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &render_shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: Some(wgpu::BlendState::REPLACE),
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
            format: depth_format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
            cache: None,
    });

    let culling_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            label: Some("physics_culling_layout"),
        });

    let culling_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &culling_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: boxes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: culled_boxes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: indirect_buffer.as_entire_binding(),
            },
        ],
        label: Some("physics_culling_bind_group"),
    });

    // Composed so physics_culling.wgsl can `#import gizmo::common::{SceneUniforms}`.
    let culling_shader = crate::pipeline::load_shader_composed(
        device,
        "crates/gizmo-renderer/src/shaders/physics_culling.wgsl",
        include_str!("../shaders/physics_culling.wgsl"),
        "Physics Culling Shader",
    );

    let culling_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Physics Culling Pipeline Layout"),
        bind_group_layouts: &[Some(global_bind_group_layout), Some(&culling_bind_group_layout)],
        immediate_size: 0,
    });

    let pipeline_culling = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Physics Culling Pipeline"),
        layout: Some(&culling_pipeline_layout),
        module: &culling_shader,
        entry_point: Some("cull_main"),
        compilation_options: Default::default(),
        cache: None,
    });

    PhysicsPipelines {
        compute_bind_group_layout,
        compute_bind_group,
        pipeline_clear,
        pipeline_build,
        pipeline_narrowphase,
        pipeline_solve,
        pipeline_integrate,
        pipeline_solve_joints,
        render_pipeline,
        culling_bind_group_layout,
        culling_bind_group,
        pipeline_culling,
    }
}
