use wgpu::util::DeviceExt;
use std::sync::Arc;
use crate::gpu_types::{Vertex, LightData, SceneUniforms};

/// Sahne render durumu — pipeline'lar, shadow, skeleton ve global bind group'lar
pub struct SceneState {
    pub render_pipeline: wgpu::RenderPipeline,
    pub render_double_sided_pipeline: wgpu::RenderPipeline,
    pub unlit_pipeline: wgpu::RenderPipeline,
    pub sky_pipeline: wgpu::RenderPipeline,
    pub water_pipeline: wgpu::RenderPipeline,
    pub shadow_pipeline: wgpu::RenderPipeline,
    pub transparent_pipeline: wgpu::RenderPipeline,
    pub global_uniform_buffer: wgpu::Buffer,
    pub global_bind_group_layout: wgpu::BindGroupLayout,
    pub global_bind_group: wgpu::BindGroup,
    pub shadow_bind_group_layout: wgpu::BindGroupLayout,
    pub shadow_bind_group: wgpu::BindGroup,
    pub shadow_texture_view: wgpu::TextureView,
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    pub skeleton_bind_group_layout: wgpu::BindGroupLayout,
    pub dummy_skeleton_bind_group: Arc<wgpu::BindGroup>,
    pub instance_bind_group_layout: wgpu::BindGroupLayout,
    pub instance_buffer: wgpu::Buffer,
    pub instance_bind_group: wgpu::BindGroup,
}

fn load_shader(device: &wgpu::Device, file_path: &str, fallback_src: &str, label: &str) -> wgpu::ShaderModule {
    let source = std::fs::read_to_string(file_path).unwrap_or_else(|_| fallback_src.to_string());
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

pub fn build_scene_pipelines(device: &wgpu::Device) -> SceneState {
    // Global Uniform Buffer
    let initial_uniforms = SceneUniforms {
        view_proj: [[0.0; 4]; 4],
        camera_pos: [0.0; 4],
        sun_direction: [0.0, -1.0, 0.0, 0.0],
        sun_color: [1.0, 1.0, 1.0, 0.0],
        lights: [LightData { position: [0.0; 4], color: [0.0; 4] }; 10],
        light_view_proj: [[0.0; 4]; 4],
        num_lights: 0,
        _padding: [0; 3],
    };
    let global_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Global Uniform Buffer"),
        contents: bytemuck::cast_slice(&[initial_uniforms]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    // Shadow Texture
    let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
        size: wgpu::Extent3d { width: 4096, height: 4096, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        label: Some("shadow_texture"), view_formats: &[],
    });
    let shadow_texture_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    });

    // ---- Bind Group Layouts ----
    let global_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("global_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });

    let global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &global_bind_group_layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: global_uniform_buffer.as_entire_binding() }],
        label: Some("global_bind_group"),
    });

    let shadow_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow_bind_group_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { multisampled: false, view_dimension: wgpu::TextureViewDimension::D2, sample_type: wgpu::TextureSampleType::Depth },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
        ],
    });

    let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &shadow_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&shadow_texture_view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
        ],
        label: Some("shadow_bind_group"),
    });

    let texture_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("texture_bind_group_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { multisampled: false, view_dimension: wgpu::TextureViewDimension::D2, sample_type: wgpu::TextureSampleType::Float { filterable: true } },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let skeleton_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("skeleton_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0, visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });

    let dummy_skeleton_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Dummy Skeleton Buffer"),
        contents: bytemuck::cast_slice(&[[[0.0f32; 4]; 4]; 64]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let dummy_skeleton_bind_group = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &skeleton_bind_group_layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: dummy_skeleton_buffer.as_entire_binding() }],
        label: Some("dummy_skeleton_bind_group"),
    }));

    let instance_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("instance_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0, visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Instance Buffer"),
        size: (100_000 * std::mem::size_of::<crate::InstanceRaw>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &instance_bind_group_layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: instance_buffer.as_entire_binding() }],
        label: Some("instance_bind_group"),
    });

    // ---- Render Pipelines ----
    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[&global_bind_group_layout, &texture_bind_group_layout, &shadow_bind_group_layout, &skeleton_bind_group_layout, &instance_bind_group_layout],
        push_constant_ranges: &[],
    });

    let shader       = load_shader(device, "demo/assets/shaders/shader.wgsl",  include_str!("shader.wgsl"),  "Shader");
    let unlit_shader = load_shader(device, "demo/assets/shaders/unlit.wgsl",   include_str!("unlit.wgsl"),   "Unlit Shader");
    let water_shader = load_shader(device, "demo/assets/shaders/water.wgsl",   include_str!("water.wgsl"),   "Water Shader");
    let sky_shader   = load_shader(device, "demo/assets/shaders/sky.wgsl",     include_str!("sky.wgsl"),     "Sky Shader");
    let shadow_shader= load_shader(device, "demo/assets/shaders/shadow.wgsl",  include_str!("shadow.wgsl"),  "Shadow Shader");

    let create_main = |sm: &wgpu::ShaderModule, label: &str, depth_write: bool, cull: Option<wgpu::Face>| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState { module: sm, entry_point: "vs_main", buffers: &[Vertex::desc()] },
            fragment: Some(wgpu::FragmentState {
                module: sm, entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: cull, // Arka yüzeyleri isteğe göre açıp kapat
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: depth_write,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(), bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            multiview: None,
        })
    };

    let render_pipeline = create_main(&shader, "Render Pipeline", true, Some(wgpu::Face::Back));
    let render_double_sided_pipeline = create_main(&shader, "Render TwoSided Pipeline", true, None);
    // Şeffaf objelerde cull_mode: None hayat kurtarır (camın arka yüzü, yapraklar vb.)
    let transparent_pipeline = create_main(&shader, "Transparent Pipeline", false, None);
    let unlit_pipeline  = create_main(&unlit_shader, "Unlit Pipeline", true, Some(wgpu::Face::Back));
    let sky_pipeline  = create_main(&sky_shader, "Sky Pipeline", false, Some(wgpu::Face::Back));
    let water_pipeline  = create_main(&water_shader, "Water Pipeline", true, Some(wgpu::Face::Back));

    // Shadow pipeline
    let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Shadow Pipeline Layout"),
        bind_group_layouts: &[&global_bind_group_layout, &skeleton_bind_group_layout, &instance_bind_group_layout],
        push_constant_ranges: &[],
    });
    let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Shadow Pipeline"),
        layout: Some(&shadow_layout),
        vertex: wgpu::VertexState { module: &shadow_shader, entry_point: "vs_main", buffers: &[Vertex::desc()] },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList, front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Front), polygon_mode: wgpu::PolygonMode::Fill,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
        }),
        multisample: wgpu::MultisampleState::default(), multiview: None,
    });

    SceneState {
        render_pipeline, render_double_sided_pipeline, unlit_pipeline, sky_pipeline, water_pipeline, shadow_pipeline, transparent_pipeline,
        global_uniform_buffer, global_bind_group_layout, global_bind_group,
        shadow_bind_group_layout, shadow_bind_group, shadow_texture_view,
        texture_bind_group_layout, skeleton_bind_group_layout, dummy_skeleton_bind_group,
        instance_bind_group_layout, instance_buffer, instance_bind_group,
    }
}

/// Shader dosyaları değiştiğinde tüm pipeline'ları yeniden derler.
pub fn rebuild_pipelines(renderer: &mut crate::Renderer) {
    let device = &renderer.device;
    let load = |path: &str, fallback: &str, label: &str| -> wgpu::ShaderModule {
        let source = std::fs::read_to_string(path).unwrap_or_else(|_| fallback.to_string());
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        })
    };

    let shader        = load("demo/assets/shaders/shader.wgsl",       include_str!("shader.wgsl"),       "Shader");
    let unlit_shader  = load("demo/assets/shaders/unlit.wgsl",        include_str!("unlit.wgsl"),        "Unlit Shader");
    let water_shader  = load("demo/assets/shaders/water.wgsl",        include_str!("water.wgsl"),        "Water Shader");
    let shadow_shader = load("demo/assets/shaders/shadow.wgsl",       include_str!("shadow.wgsl"),       "Shadow Shader");
    let post_shader   = load("demo/assets/shaders/post_process.wgsl", include_str!("post_process.wgsl"), "Post-Processing Shader");

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[
            &renderer.scene.global_bind_group_layout, &renderer.scene.texture_bind_group_layout,
            &renderer.scene.shadow_bind_group_layout, &renderer.scene.skeleton_bind_group_layout,
            &renderer.scene.instance_bind_group_layout,
        ],
        push_constant_ranges: &[],
    });

    let create_main = |sm: &wgpu::ShaderModule, label: &str| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label), layout: Some(&layout),
            vertex: wgpu::VertexState { module: sm, entry_point: "vs_main", buffers: &[Vertex::desc()] },
            fragment: Some(wgpu::FragmentState {
                module: sm, entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, front_face: wgpu::FrontFace::Ccw, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(), bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false }, multiview: None,
        })
    };

    renderer.scene.render_pipeline = create_main(&shader, "Render Pipeline");
    renderer.scene.unlit_pipeline  = create_main(&unlit_shader, "Unlit Pipeline");
    renderer.scene.water_pipeline  = create_main(&water_shader, "Water Pipeline");

    let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Shadow Pipeline Layout"),
        bind_group_layouts: &[&renderer.scene.global_bind_group_layout, &renderer.scene.skeleton_bind_group_layout, &renderer.scene.instance_bind_group_layout],
        push_constant_ranges: &[],
    });
    renderer.scene.shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Shadow Pipeline"), layout: Some(&shadow_layout),
        vertex: wgpu::VertexState { module: &shadow_shader, entry_point: "vs_main", buffers: &[Vertex::desc()] },
        fragment: None,
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, front_face: wgpu::FrontFace::Ccw, cull_mode: Some(wgpu::Face::Front), ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(), bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
        }),
        multisample: wgpu::MultisampleState::default(), multiview: None,
    });

    crate::post_process::rebuild_post_pipelines(renderer, &post_shader);
}
