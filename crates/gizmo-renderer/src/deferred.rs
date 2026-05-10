use crate::gpu_types::Vertex;
use crate::pipeline::{load_shader, SceneState};

/// G-Buffer textures, pipelines and bind groups for the deferred rendering path.
pub struct DeferredState {
    // G-buffer colour targets
    pub albedo_metallic_tex: wgpu::Texture,
    pub albedo_metallic_view: wgpu::TextureView,
    pub normal_roughness_tex: wgpu::Texture,
    pub normal_roughness_view: wgpu::TextureView,
    pub world_position_tex: wgpu::Texture,
    pub world_position_view: wgpu::TextureView,

    // Geometry pass (writes to 3 MRTs)
    pub gbuffer_pipeline: wgpu::RenderPipeline,

    // Z-Prepass (Depth only)
    pub z_prepass_pipeline: wgpu::RenderPipeline,

    // Lighting pass (fullscreen triangle → HDR texture)
    pub lighting_pipeline: wgpu::RenderPipeline,

    // Bind group used by the lighting pass to read the G-buffers
    pub gbuffer_bind_group_layout: wgpu::BindGroupLayout,
    pub gbuffer_bind_group: wgpu::BindGroup,
    pub gbuf_sampler: wgpu::Sampler,

    pub width: u32,
    pub height: u32,
}

impl DeferredState {
    pub fn new(device: &wgpu::Device, scene: &SceneState, width: u32, height: u32) -> Self {
        let (
            albedo_metallic_tex,
            albedo_metallic_view,
            normal_roughness_tex,
            normal_roughness_view,
            world_position_tex,
            world_position_view,
            gbuf_sampler,
        ) = Self::create_gbuffer_textures(device, width, height);

        let gbuffer_bind_group_layout = Self::create_gbuffer_layout(device);

        let gbuffer_bind_group = Self::create_gbuffer_bind_group(
            device,
            &gbuffer_bind_group_layout,
            &albedo_metallic_view,
            &normal_roughness_view,
            &world_position_view,
            &gbuf_sampler,
        );

        let z_prepass_pipeline = Self::create_z_prepass_pipeline(device, scene);
        let gbuffer_pipeline = Self::create_gbuffer_pipeline(device, scene);
        let lighting_pipeline =
            Self::create_lighting_pipeline(device, scene, &gbuffer_bind_group_layout);

        Self {
            albedo_metallic_tex,
            albedo_metallic_view,
            normal_roughness_tex,
            normal_roughness_view,
            world_position_tex,
            world_position_view,
            gbuffer_pipeline,
            z_prepass_pipeline,
            lighting_pipeline,
            gbuffer_bind_group_layout,
            gbuffer_bind_group,
            gbuf_sampler,
            width,
            height,
        }
    }

    /// Recreate G-buffer textures and bind groups when the window is resized.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        let (
            albedo_metallic_tex,
            albedo_metallic_view,
            normal_roughness_tex,
            normal_roughness_view,
            world_position_tex,
            world_position_view,
            gbuf_sampler,
        ) = Self::create_gbuffer_textures(device, width, height);

        self.gbuffer_bind_group = Self::create_gbuffer_bind_group(
            device,
            &self.gbuffer_bind_group_layout,
            &albedo_metallic_view,
            &normal_roughness_view,
            &world_position_view,
            &gbuf_sampler,
        );

        self.albedo_metallic_tex = albedo_metallic_tex;
        self.albedo_metallic_view = albedo_metallic_view;
        self.normal_roughness_tex = normal_roughness_tex;
        self.normal_roughness_view = normal_roughness_view;
        self.world_position_tex = world_position_tex;
        self.world_position_view = world_position_view;
        self.gbuf_sampler = gbuf_sampler;
        self.width = width;
        self.height = height;
    }

    // ── helpers ─────────────────────────────────────────────────────────────

    fn create_gbuffer_textures(
        device: &wgpu::Device,
        w: u32,
        h: u32,
    ) -> (
        wgpu::Texture,
        wgpu::TextureView,
        wgpu::Texture,
        wgpu::TextureView,
        wgpu::Texture,
        wgpu::TextureView,
        wgpu::Sampler,
    ) {
        let mk = |label: &str, fmt: wgpu::TextureFormat| {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: fmt,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let v = t.create_view(&wgpu::TextureViewDescriptor::default());
            (t, v)
        };

        let (a, av) = mk("gbuf_albedo_metallic", wgpu::TextureFormat::Rgba16Float);
        let (n, nv) = mk("gbuf_normal_roughness", wgpu::TextureFormat::Rgba16Float);
        let (p, pv) = mk("gbuf_world_position", wgpu::TextureFormat::Rgba32Float);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        (a, av, n, nv, p, pv, sampler)
    }

    fn create_gbuffer_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gbuffer_bind_group_layout"),
            entries: &[
                // albedo_metallic
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                // normal_roughness
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                // world_position
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                // nearest sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        })
    }

    fn create_gbuffer_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        albedo_v: &wgpu::TextureView,
        normal_v: &wgpu::TextureView,
        pos_v: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gbuffer_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(albedo_v),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(normal_v),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(pos_v),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    fn create_gbuffer_pipeline(device: &wgpu::Device, scene: &SceneState) -> wgpu::RenderPipeline {
        let shader = load_shader(
            device,
            "demo/assets/shaders/gbuffer.wgsl",
            include_str!("shaders/gbuffer.wgsl"),
            "GBuffer Shader",
        );

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("GBuffer Pipeline Layout"),
            bind_group_layouts: &[
                &scene.global_bind_group_layout,   // 0: SceneUniforms
                &scene.texture_bind_group_layout,  // 1: albedo texture
                &scene.shadow_bind_group_layout, // 2: shadow (unused in G-pass but slot must exist)
                &scene.skeleton_bind_group_layout, // 3: skeleton
                &scene.instance_bind_group_layout, // 4: instances
            ],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("GBuffer Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[Vertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[
                    // RT0: albedo_metallic  Rgba8Unorm
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    // RT1: normal_roughness Rgba16Float
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    // RT2: world_position   Rgba32Float
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba32Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        })
    }

    fn create_z_prepass_pipeline(
        device: &wgpu::Device,
        scene: &SceneState,
    ) -> wgpu::RenderPipeline {
        let shader = load_shader(
            device,
            "demo/assets/shaders/gbuffer.wgsl",
            include_str!("shaders/gbuffer.wgsl"),
            "Z-Prepass Shader",
        );

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Z-Prepass Pipeline Layout"),
            bind_group_layouts: &[
                &scene.global_bind_group_layout,   // 0: SceneUniforms
                &scene.texture_bind_group_layout, // 1: albedo texture (unused but required by shader layout)
                &scene.shadow_bind_group_layout,  // 2: shadow
                &scene.skeleton_bind_group_layout, // 3: skeleton
                &scene.instance_bind_group_layout, // 4: instances
            ],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Z-Prepass Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[Vertex::desc()],
            },
            fragment: None, // NO COLOR TARGETS!
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
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
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        })
    }

    fn create_lighting_pipeline(
        device: &wgpu::Device,
        scene: &SceneState,
        gbuffer_layout: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let shader = load_shader(
            device,
            "demo/assets/shaders/deferred_lighting.wgsl",
            include_str!("shaders/deferred_lighting.wgsl"),
            "Deferred Lighting Shader",
        );

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Deferred Lighting Layout"),
            bind_group_layouts: &[
                &scene.global_bind_group_layout, // 0: SceneUniforms
                &scene.shadow_bind_group_layout, // 1: shadow CSM
                gbuffer_layout,                  // 2: G-buffers
            ],
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Deferred Lighting Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[], // fullscreen triangle — no vertex buffer
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None, // no depth write in lighting pass
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        })
    }
}
