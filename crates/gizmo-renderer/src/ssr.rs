use crate::deferred::DeferredState;

/// The six numbers that shape the screen-space reflection march.
///
/// Every one of these was a literal in `ssr.wgsl`, which is why `CAPABILITY_GAPS.md` §B recorded
/// "0 shaping knobs" against a pass that demonstrably works — the `ssr` demo measures the floor
/// under each cube picking up that cube's colour, +37.6 R / +61.1 G / +38.7 B.
///
/// The defaults are the shader's own values, so a state built and left alone renders exactly what
/// it rendered before this struct existed — locked by `the_ssr_defaults_are_the_shader_literals`.
///
/// Written to the GPU each frame from [`SsrState::params`]; changing a field is enough.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SsrParams {
    /// Roughness above which a surface reflects nothing at all. Shader default **0.5**.
    ///
    /// A hard cut, not a fade: this is the early-out that keeps the march off matte surfaces
    /// entirely. [`fade_start`](Self::fade_start) and [`fade_end`](Self::fade_end) shape what
    /// happens below it.
    pub roughness_cutoff: f32,
    /// Where the reflection begins to fade with roughness. Shader default **0.1**.
    pub fade_start: f32,
    /// Where that fade reaches zero. Shader default **0.5**.
    pub fade_end: f32,
    /// World-space distance per march step. Shader default **1.0**.
    pub step_size: f32,
    /// How many steps a ray takes before giving up. The cost knob. Shader default **20**.
    pub max_steps: f32,
    /// How thick a surface is assumed to be when deciding whether the ray hit it, in metres.
    ///
    /// Too small and the ray passes through thin geometry; too large and it reports a hit on
    /// something well behind the surface. Shader default **1.0**.
    pub thickness: f32,
    /// How far along the reflection vector the march starts, to avoid self-intersection at the
    /// origin. Shader default **0.1**.
    pub start_offset: f32,
    /// Width of the screen-edge fade, in UV. Shader default **0.1**.
    pub edge_fade: f32,
}

impl Default for SsrParams {
    fn default() -> Self {
        Self {
            roughness_cutoff: 0.5,
            fade_start: 0.1,
            fade_end: 0.5,
            step_size: 1.0,
            max_steps: 20.0,
            thickness: 1.0,
            start_offset: 0.1,
            edge_fade: 0.1,
        }
    }
}
use crate::pipeline::{load_shader, load_shader_composed, SceneState};

/// Screen-space reflections: reflections marched through the depth buffer.
///
/// Only what is already on screen can be reflected, so a surface facing away from the camera
/// reflects nothing — the fallback is the environment probe, applied by the deferred lighting pass.
pub struct SsrState {
    /// The traced reflection colour.
    pub ssr_texture: wgpu::Texture,
    /// Its view.
    pub ssr_view: wgpu::TextureView,

    /// The trace pass.
    pub ssr_pipeline: wgpu::RenderPipeline,
    ssr_bgl: wgpu::BindGroupLayout,
    /// Its G-buffer and HDR inputs. Rebuilt on resize.
    pub ssr_bind_group: wgpu::BindGroup,

    /// The apply pass, compositing the reflection over the HDR target by the surface's Fresnel
    /// term.
    pub apply_pipeline: wgpu::RenderPipeline,
    apply_bgl: wgpu::BindGroupLayout,
    /// Its input. Rebuilt on resize.
    pub apply_bind_group: wgpu::BindGroup,

    nearest_sampler: wgpu::Sampler,

    /// The reflection target's width, in pixels.
    pub width: u32,
    /// Its height.
    pub height: u32,
    /// Whether the pass runs this frame.
    ///
    /// This is the **reversible** off switch. The destructive one — setting the renderer's
    /// `Option` field to `None` — frees the GPU objects and cannot be undone without rebuilding
    /// them; that is the only switch the engine used to have, and it is why a comparison render
    /// could not be turned back on.
    ///
    /// Off, the pass is skipped entirely rather than drawing a neutral result. That is safe here
    /// because every texture this state owns is read only by this state's own apply pass, which is
    /// skipped with it, and the apply pass composites into the HDR target with `LoadOp::Load` —
    /// so a skipped effect leaves the frame exactly as it found it. Resizing still happens while
    /// off, so switching back on costs no rebuild.
    pub enabled: bool,

    /// The eight shaping numbers. Write to them directly; they reach the GPU on the next frame.
    pub params: SsrParams,
    /// Their uniform buffer.
    pub params_buffer: wgpu::Buffer,
}

impl SsrState {
    /// Builds every SSR resource for a target of the given size.
    pub fn new(
        device: &wgpu::Device,
        scene: &SceneState,
        deferred: &DeferredState,
        hdr_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Self {
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let half_w = (width / 2).max(1);
        let half_h = (height / 2).max(1);
        let (ssr_texture, ssr_view) = Self::mk_ssr_tex(device, half_w, half_h);

        let ssr_bgl = Self::mk_ssr_bgl(device);
        let apply_bgl = Self::mk_apply_bgl(device);

        let params = SsrParams::default();
        let params_buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("ssr_params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            },
        );

        let ssr_bind_group = Self::mk_ssr_bg(
            device,
            &ssr_bgl,
            hdr_view,
            &deferred.normal_roughness_view,
            &deferred.world_position_view,
            &linear_sampler,
            &params_buffer,
        );

        let apply_bind_group = Self::mk_apply_bg(device, &apply_bgl, &ssr_view, &linear_sampler);

        let ssr_pipeline = Self::mk_ssr_pipeline(device, scene, &ssr_bgl);
        let apply_pipeline = Self::mk_apply_pipeline(device, &apply_bgl);

        Self {
            enabled: true,
            params,
            params_buffer,
            ssr_texture,
            ssr_view,
            ssr_pipeline,
            ssr_bgl,
            ssr_bind_group,
            apply_pipeline,
            apply_bgl,
            apply_bind_group,
            nearest_sampler: linear_sampler,
            width,
            height,
        }
    }

    /// Rebuilds the size-dependent texture and the bind groups that read it.
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
        let (ssr_texture, ssr_view) = Self::mk_ssr_tex(device, half_w, half_h);

        self.ssr_bind_group = Self::mk_ssr_bg(
            device,
            &self.ssr_bgl,
            hdr_view,
            &deferred.normal_roughness_view,
            &deferred.world_position_view,
            &self.nearest_sampler,
            &self.params_buffer,
        );
        self.apply_bind_group =
            Self::mk_apply_bg(device, &self.apply_bgl, &ssr_view, &self.nearest_sampler);

        self.ssr_texture = ssr_texture;
        self.ssr_view = ssr_view;
        self.width = width;
        self.height = height;
    }

    fn mk_ssr_tex(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let t = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ssr_texture"),
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

    fn mk_ssr_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssr_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    // t_hdr
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    // t_normal_roughness
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
                    // t_world_position
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
                    // s_nearest
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    // params — the eight shaping numbers, see `SsrParams`.
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }

    fn mk_apply_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssr_apply_bgl"),
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

    fn mk_ssr_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        hdr_view: &wgpu::TextureView,
        normal_view: &wgpu::TextureView,
        pos_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ssr_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(pos_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: params.as_entire_binding(),
                },
            ],
        })
    }

    fn mk_apply_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        ssr_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ssr_apply_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(ssr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    fn mk_ssr_pipeline(
        device: &wgpu::Device,
        scene: &SceneState,
        bgl: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let shader = load_shader_composed(
            device,
            "demo/assets/shaders/ssr.wgsl",
            include_str!("shaders/ssr.wgsl"),
            "SSR Shader",
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ssr_layout"),
            bind_group_layouts: &[Some(&scene.global_bind_group_layout), Some(bgl)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ssr_pipeline"),
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
            "demo/assets/shaders/ssr_apply.wgsl",
            include_str!("shaders/ssr_apply.wgsl"),
            "SSR Apply Shader",
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ssr_apply_layout"),
            bind_group_layouts: &[Some(bgl)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ssr_apply_pipeline"),
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
                    // Screen Space Reflections are additive!
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
}
