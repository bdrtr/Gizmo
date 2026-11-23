use super::types::GpuBox;

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
    /// Stage 0: the step's forces (gravity), before anything is solved.
    pub pipeline_apply_forces: wgpu::ComputePipeline,
    /// Stage 1: clear the broadphase grid.
    pub pipeline_clear: wgpu::ComputePipeline,
    /// Stage 2: bin every body into the grid.
    pub pipeline_build: wgpu::ComputePipeline,
    /// Stage 3: turn candidate pairs into contact manifolds.
    pub pipeline_narrowphase: wgpu::ComputePipeline,
    /// Stage 4: the contact solver, run for several iterations.
    pub pipeline_solve: wgpu::ComputePipeline,
    /// Stage 4b: the bounce, once per contact, after the solve.
    pub pipeline_restitution: wgpu::ComputePipeline,
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
    use crate::descriptors::{buffer_entry, compute_pipeline, whole_buffer_entries};
    use wgpu::BufferBindingType::{Storage, Uniform};
    let compute = wgpu::ShaderStages::COMPUTE;
    let compute_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        entries: &[
            buffer_entry(0, compute, Uniform),
            buffer_entry(1, compute, Storage { read_only: false }),
            buffer_entry(2, compute, Storage { read_only: false }),
            buffer_entry(3, compute, Storage { read_only: false }),
            buffer_entry(4, compute, Storage { read_only: true }),
            buffer_entry(5, compute, Storage { read_only: false }),
            buffer_entry(6, compute, Storage { read_only: false }),
            buffer_entry(7, compute, Storage { read_only: false }),
        ],
        label: Some("physics_compute_layout"),
    });

    let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &compute_bind_group_layout,
        entries: &whole_buffer_entries(&[
            params_buffer,
            boxes_buffer,
            grid_heads_buffer,
            linked_nodes_buffer,
            colliders_buffer,
            awake_flags_buffer,
            joints_buffer,
            box_contacts_buffer,
        ]),
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

    let stage = |label: &str, entry: &str| compute_pipeline(device, label, &compute_pipeline_layout, &compute_shader, entry);
    let pipeline_clear = stage("Physics Clear", "clear_grid");
    let pipeline_build = stage("Physics Build", "build_grid");
    let pipeline_narrowphase = stage("Physics Narrowphase", "narrowphase");
    let pipeline_solve = stage("Physics Solve", "solve_collisions_safe");
    let pipeline_apply_forces = stage("Physics Apply Forces", "apply_forces");
    let pipeline_restitution = stage("Physics Restitution", "apply_restitution");
    let pipeline_integrate = stage("Physics Integrate", "integrate");
    let pipeline_solve_joints = stage("Physics Solve Joints", "solve_joints");

    // Composed so physics_render.wgsl can `#import gizmo::common::{SceneUniforms}`.
    let render_shader = crate::pipeline::load_shader_composed(
        device,
        "demo/assets/shaders/physics_render.wgsl",
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
            buffers: &[Some(crate::gpu_types::GpuVertex::desc()), Some(GpuBox::desc())],
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

    let culling_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        entries: &[
            buffer_entry(0, compute, Uniform),
            buffer_entry(1, compute, Storage { read_only: true }),
            buffer_entry(2, compute, Storage { read_only: false }),
            buffer_entry(3, compute, Storage { read_only: false }),
        ],
        label: Some("physics_culling_layout"),
    });

    let culling_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &culling_bind_group_layout,
        entries: &whole_buffer_entries(&[params_buffer, boxes_buffer, culled_boxes_buffer, indirect_buffer]),
        label: Some("physics_culling_bind_group"),
    });

    // Composed so physics_culling.wgsl can `#import gizmo::common::{SceneUniforms}`.
    let culling_shader = crate::pipeline::load_shader_composed(
        device,
        "demo/assets/shaders/physics_culling.wgsl",
        include_str!("../shaders/physics_culling.wgsl"),
        "Physics Culling Shader",
    );

    let culling_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Physics Culling Pipeline Layout"),
        bind_group_layouts: &[Some(global_bind_group_layout), Some(&culling_bind_group_layout)],
        immediate_size: 0,
    });

    let pipeline_culling = compute_pipeline(device, "Physics Culling Pipeline", &culling_pipeline_layout, &culling_shader, "cull_main");

    PhysicsPipelines {
        compute_bind_group_layout,
        compute_bind_group,
        pipeline_apply_forces,
        pipeline_clear,
        pipeline_build,
        pipeline_narrowphase,
        pipeline_solve,
        pipeline_restitution,
        pipeline_integrate,
        pipeline_solve_joints,
        render_pipeline,
        culling_bind_group_layout,
        culling_bind_group,
        pipeline_culling,
    }
}
