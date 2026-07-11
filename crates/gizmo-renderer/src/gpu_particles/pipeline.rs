use super::types::GpuParticle;

pub struct ParticlePipelines {
    pub compute_pipeline: wgpu::ComputePipeline,
    pub compute_bind_group: wgpu::BindGroup,
    pub render_pipeline: wgpu::RenderPipeline,
    /// Group 1: sahne derinlik dokusu (soft particles için FS'te örneklenir). Bind group
    /// her frame güncel `depth_texture_view` ile oluşturulur (resize'da view değişir).
    pub depth_bind_group_layout: wgpu::BindGroupLayout,
    /// Group 2: flipbook/SubUV atlas dokusu + sampler (duman sprite'ları). Varsayılan 1×1
    /// beyaz; `set_flipbook` gerçek atlas'ı yükler. `misc.z` bayrağı FS'te açar/kapatır.
    pub flipbook_bind_group_layout: wgpu::BindGroupLayout,
}

pub fn create_particle_pipelines(
    device: &wgpu::Device,
    global_bind_group_layout: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
    params_buffer: &wgpu::Buffer,
    particles_buffer: &wgpu::Buffer,
) -> ParticlePipelines {
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
            ],
            label: Some("particle_compute_layout"),
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
                resource: particles_buffer.as_entire_binding(),
            },
        ],
        label: Some("particle_compute_bind_group"),
    });

    let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Particle Compute Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/particle_compute.wgsl").into()),
    });

    let compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Particle Compute Pipeline Layout"),
        bind_group_layouts: &[Some(&compute_bind_group_layout)],
        immediate_size: 0,
    });

    let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Particle Compute Pipeline"),
        layout: Some(&compute_pipeline_layout),
        module: &compute_shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });

    // Composed so particle_render.wgsl can `#import gizmo::common::{SceneUniforms}`.
    let render_shader = crate::pipeline::load_shader_composed(
        device,
        "crates/gizmo-renderer/src/shaders/particle_render.wgsl",
        include_str!("../shaders/particle_render.wgsl"),
        "Particle Render Shader",
    );

    // Group 1: sahne derinlik dokusu (soft particles). textureLoad ile okunur → sampler yok.
    let depth_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("particle_depth_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    });

    // Group 2: flipbook atlas dokusu (filterable) + sampler.
    let flipbook_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle_flipbook_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // binding 2: flipbook config uniform (x=tiles/kenar, y=açık 1/0)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Particle Render Pipeline Layout"),
        bind_group_layouts: &[
            Some(global_bind_group_layout),
            Some(&depth_bind_group_layout),
            Some(&flipbook_bind_group_layout),
        ],
        immediate_size: 0,
    });

    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Particle Render Pipeline"),
        layout: Some(&render_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &render_shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![4 => Float32x2],
                },
                GpuParticle::desc(),
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: &render_shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        // Depth attachment YOK: particle'lar ayrı pass'te (soft-particle derinlik örneklemesi
        // için) çizilir; occlusion + soft-fade FS'te sahne derinliğinden manuel yapılır.
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
            cache: None,
    });

    ParticlePipelines {
        compute_pipeline,
        compute_bind_group,
        render_pipeline,
        depth_bind_group_layout,
        flipbook_bind_group_layout,
    }
}
