use crate::deferred::DeferredState;
use crate::pipeline::{load_shader, load_shader_composed, SceneState};

/// GPU uniform for the SSGI temporal-accumulation resolve. Byte-compatible with
/// `SsgiTemporalParams` in `ssgi_temporal.wgsl` (64 + 4 + 12 = 80 bytes, 16-aligned).
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct SsgiTemporalParams {
    prev_view_proj: [[f32; 4]; 4],
    alpha: f32,
    _pad: [f32; 3],
}

pub struct SsgiState {
    pub ssgi_texture: wgpu::Texture,
    pub ssgi_view: wgpu::TextureView,

    pub ssgi_blurred_texture: wgpu::Texture,
    pub ssgi_blurred_view: wgpu::TextureView,

    pub ssgi_pipeline: wgpu::RenderPipeline,
    ssgi_bgl: wgpu::BindGroupLayout,
    pub ssgi_bind_group: wgpu::BindGroup,

    // ── Temporal accumulation (denoise the 1-spp raymarch) ──────────────────────
    // Ping-pong half-res history: resolve reads history[parity] + raw ssgi, writes
    // history[!parity]; the blur then reads that just-written accumulation buffer.
    history_a: wgpu::Texture,
    history_a_view: wgpu::TextureView,
    history_b: wgpu::Texture,
    history_b_view: wgpu::TextureView,
    frame_parity: bool,
    pub frame_index: u32,
    prev_vp: [[f32; 4]; 4],
    temporal_params_buffer: wgpu::Buffer,
    pub temporal_pipeline: wgpu::RenderPipeline,
    temporal_bgl: wgpu::BindGroupLayout,
    temporal_bg_read_a: wgpu::BindGroup, // history=A → output=B
    temporal_bg_read_b: wgpu::BindGroup, // history=B → output=A

    pub blur_pipeline: wgpu::RenderPipeline,
    blur_bgl: wgpu::BindGroupLayout,
    // Blur reads the accumulated history (ping-pong), not the raw ssgi buffer.
    blur_bg_a: wgpu::BindGroup, // reads history A
    blur_bg_b: wgpu::BindGroup, // reads history B

    pub apply_pipeline: wgpu::RenderPipeline,
    apply_bgl: wgpu::BindGroupLayout,
    pub apply_bind_group: wgpu::BindGroup,

    _nearest_sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,

    pub width: u32,
    pub height: u32,
}

impl SsgiState {
    pub fn new(
        device: &wgpu::Device,
        scene: &SceneState,
        deferred: &DeferredState,
        hdr_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Self {
        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // SSGI usually runs at half or full resolution. Half res is better for performance.
        let half_w = (width / 2).max(1);
        let half_h = (height / 2).max(1);

        let (ssgi_texture, ssgi_view) = Self::mk_tex(device, half_w, half_h, "ssgi_texture");
        let (ssgi_blurred_texture, ssgi_blurred_view) =
            Self::mk_tex(device, half_w, half_h, "ssgi_blurred_texture");
        let (history_a, history_a_view) = Self::mk_tex(device, half_w, half_h, "ssgi_history_a");
        let (history_b, history_b_view) = Self::mk_tex(device, half_w, half_h, "ssgi_history_b");

        let ssgi_bgl = Self::mk_ssgi_bgl(device);
        let blur_bgl = Self::mk_blur_bgl(device);
        let apply_bgl = Self::mk_apply_bgl(device);
        let temporal_bgl = Self::mk_temporal_bgl(device);

        let ssgi_bind_group = Self::mk_ssgi_bg(
            device,
            &ssgi_bgl,
            hdr_view,
            &deferred.normal_roughness_view,
            &deferred.world_position_view,
            &linear_sampler,
            &deferred.albedo_metallic_view,
        );

        let temporal_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ssgi_temporal_params"),
            size: std::mem::size_of::<SsgiTemporalParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let temporal_bg_read_a = Self::mk_temporal_bg(
            device,
            &temporal_bgl,
            &temporal_params_buffer,
            &ssgi_view,
            &history_a_view,
            &deferred.world_position_view,
            &linear_sampler,
        );
        let temporal_bg_read_b = Self::mk_temporal_bg(
            device,
            &temporal_bgl,
            &temporal_params_buffer,
            &ssgi_view,
            &history_b_view,
            &deferred.world_position_view,
            &linear_sampler,
        );

        // Blur reads the accumulated history (ping-pong), never the raw ssgi buffer.
        let blur_bg_a = Self::mk_blur_bg(device, &blur_bgl, &history_a_view, &linear_sampler);
        let blur_bg_b = Self::mk_blur_bg(device, &blur_bgl, &history_b_view, &linear_sampler);
        let apply_bind_group =
            Self::mk_apply_bg(device, &apply_bgl, &ssgi_blurred_view, &linear_sampler);

        let ssgi_pipeline = Self::mk_ssgi_pipeline(device, scene, &ssgi_bgl);
        let blur_pipeline = Self::mk_blur_pipeline(device, &blur_bgl);
        let apply_pipeline = Self::mk_apply_pipeline(device, &apply_bgl);
        let temporal_pipeline = Self::mk_temporal_pipeline(device, &temporal_bgl);

        let identity: [[f32; 4]; 4] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];

        Self {
            ssgi_texture,
            ssgi_view,
            ssgi_blurred_texture,
            ssgi_blurred_view,
            ssgi_pipeline,
            ssgi_bgl,
            ssgi_bind_group,
            history_a,
            history_a_view,
            history_b,
            history_b_view,
            frame_parity: false,
            frame_index: 0,
            prev_vp: identity,
            temporal_params_buffer,
            temporal_pipeline,
            temporal_bgl,
            temporal_bg_read_a,
            temporal_bg_read_b,
            blur_pipeline,
            blur_bgl,
            blur_bg_a,
            blur_bg_b,
            apply_pipeline,
            apply_bgl,
            apply_bind_group,
            _nearest_sampler: nearest_sampler,
            linear_sampler,
            width,
            height,
        }
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        deferred: &DeferredState,
        hdr_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let half_w = (width / 2).max(1);
        let half_h = (height / 2).max(1);
        let (t1, v1) = Self::mk_tex(device, half_w, half_h, "ssgi_texture");
        let (t2, v2) = Self::mk_tex(device, half_w, half_h, "ssgi_blurred_texture");
        let (ha, hav) = Self::mk_tex(device, half_w, half_h, "ssgi_history_a");
        let (hb, hbv) = Self::mk_tex(device, half_w, half_h, "ssgi_history_b");

        self.ssgi_bind_group = Self::mk_ssgi_bg(
            device,
            &self.ssgi_bgl,
            hdr_view,
            &deferred.normal_roughness_view,
            &deferred.world_position_view,
            &self.linear_sampler,
            &deferred.albedo_metallic_view,
        );
        self.temporal_bg_read_a = Self::mk_temporal_bg(
            device,
            &self.temporal_bgl,
            &self.temporal_params_buffer,
            &v1,
            &hav,
            &deferred.world_position_view,
            &self.linear_sampler,
        );
        self.temporal_bg_read_b = Self::mk_temporal_bg(
            device,
            &self.temporal_bgl,
            &self.temporal_params_buffer,
            &v1,
            &hbv,
            &deferred.world_position_view,
            &self.linear_sampler,
        );
        self.blur_bg_a = Self::mk_blur_bg(device, &self.blur_bgl, &hav, &self.linear_sampler);
        self.blur_bg_b = Self::mk_blur_bg(device, &self.blur_bgl, &hbv, &self.linear_sampler);
        self.apply_bind_group =
            Self::mk_apply_bg(device, &self.apply_bgl, &v2, &self.linear_sampler);

        self.ssgi_texture = t1;
        self.ssgi_view = v1;
        self.ssgi_blurred_texture = t2;
        self.ssgi_blurred_view = v2;
        self.history_a = ha;
        self.history_a_view = hav;
        self.history_b = hb;
        self.history_b_view = hbv;
        // Stale history from the old resolution would ghost — restart accumulation.
        self.frame_parity = false;
        self.frame_index = 0;
        self.width = width;
        self.height = height;
    }

    /// Upload the temporal-resolve uniform: `self.prev_vp` (last frame's unjittered
    /// view-proj) drives reprojection; `alpha` is the blend weight (1.0 = ignore
    /// history, used on the first frame / after a reset).
    pub fn update_params(&self, queue: &wgpu::Queue, alpha: f32) {
        let data = SsgiTemporalParams {
            prev_view_proj: self.prev_vp,
            alpha,
            _pad: [0.0; 3],
        };
        queue.write_buffer(&self.temporal_params_buffer, 0, bytemuck::bytes_of(&data));
    }

    /// Store the current frame's unjittered view-proj for reprojection next frame.
    pub fn store_prev_vp(&mut self, vp: [[f32; 4]; 4]) {
        self.prev_vp = vp;
    }

    /// Advance ping-pong parity and frame counter (call once per frame, after the passes).
    pub fn advance_frame(&mut self) {
        self.frame_parity = !self.frame_parity;
        self.frame_index = self.frame_index.wrapping_add(1);
    }

    /// (temporal resolve bind group, output history view) for the current frame.
    /// Reads history[parity] + raw ssgi, writes history[!parity].
    pub fn current_temporal_io(&self) -> (&wgpu::BindGroup, &wgpu::TextureView) {
        if !self.frame_parity {
            (&self.temporal_bg_read_a, &self.history_b_view)
        } else {
            (&self.temporal_bg_read_b, &self.history_a_view)
        }
    }

    /// Blur bind group that reads this frame's freshly-written accumulation buffer.
    pub fn current_blur_bg(&self) -> &wgpu::BindGroup {
        if !self.frame_parity {
            &self.blur_bg_b // B was just written by the resolve
        } else {
            &self.blur_bg_a
        }
    }

    fn mk_tex(
        device: &wgpu::Device,
        w: u32,
        h: u32,
        label: &str,
    ) -> (wgpu::Texture, wgpu::TextureView) {
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
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let v = t.create_view(&wgpu::TextureViewDescriptor::default());
        (t, v)
    }

    fn mk_ssgi_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssgi_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // binding 4: albedo (RT0.rgb) — receiver surface colour, so gathered GI is
                // tinted/absorbed by the surface it lands on instead of applied untinted.
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
            ],
        })
    }

    fn mk_blur_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssgi_blur_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    }

    fn mk_apply_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssgi_apply_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    }

    fn mk_ssgi_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        hdr: &wgpu::TextureView,
        nrm: &wgpu::TextureView,
        pos: &wgpu::TextureView,
        samp: &wgpu::Sampler,
        albedo: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ssgi_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(nrm),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(pos),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(samp),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(albedo),
                },
            ],
        })
    }

    fn mk_blur_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        ssgi_raw: &wgpu::TextureView,
        samp: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ssgi_blur_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(ssgi_raw),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(samp),
                },
            ],
        })
    }

    fn mk_apply_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        ssgi_blurred: &wgpu::TextureView,
        samp: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ssgi_apply_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(ssgi_blurred),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(samp),
                },
            ],
        })
    }

    fn mk_ssgi_pipeline(
        device: &wgpu::Device,
        scene: &SceneState,
        bgl: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let shader = load_shader_composed(
            device,
            "demo/assets/shaders/ssgi.wgsl",
            include_str!("shaders/ssgi.wgsl"),
            "SSGI Shader",
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ssgi_layout"),
            bind_group_layouts: &[Some(&scene.global_bind_group_layout), Some(bgl)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ssgi_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
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
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    fn mk_blur_pipeline(
        device: &wgpu::Device,
        bgl: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let shader = load_shader(
            device,
            "demo/assets/shaders/ssgi_blur.wgsl",
            include_str!("shaders/ssgi_blur.wgsl"),
            "SSGI Blur Shader",
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ssgi_blur_layout"),
            bind_group_layouts: &[Some(bgl)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ssgi_blur_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
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
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    fn mk_apply_pipeline(
        device: &wgpu::Device,
        bgl: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let shader = load_shader(
            device,
            "demo/assets/shaders/ssgi_apply.wgsl",
            include_str!("shaders/ssgi_apply.wgsl"),
            "SSGI Apply Shader",
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ssgi_apply_layout"),
            bind_group_layouts: &[Some(bgl)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ssgi_apply_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    fn mk_temporal_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssgi_temporal_bgl"),
            entries: &[
                // 0: SsgiTemporalParams uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<SsgiTemporalParams>() as u64,
                        ),
                    },
                    count: None,
                },
                // 1: t_current — raw ssgi this frame (textureLoad → non-filterable)
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
                // 2: t_history — accumulated last frame (textureSample bilinear → filterable)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                // 3: t_position — full-res world position (textureLoad → non-filterable)
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                // 4: s_linear (only paired with t_history)
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn mk_temporal_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        params_buf: &wgpu::Buffer,
        current: &wgpu::TextureView,
        history: &wgpu::TextureView,
        position: &wgpu::TextureView,
        samp: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ssgi_temporal_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(current),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(history),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(position),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(samp),
                },
            ],
        })
    }

    fn mk_temporal_pipeline(
        device: &wgpu::Device,
        bgl: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let shader = load_shader(
            device,
            "demo/assets/shaders/ssgi_temporal.wgsl",
            include_str!("shaders/ssgi_temporal.wgsl"),
            "SSGI Temporal Shader",
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ssgi_temporal_layout"),
            bind_group_layouts: &[Some(bgl)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ssgi_temporal_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_resolve"),
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
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }
}
