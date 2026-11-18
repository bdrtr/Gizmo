use crate::deferred::DeferredState;

/// The eight numbers that shape the screen-space reflection march.
///
/// Every one of these was a literal in `ssr.wgsl`, which is why `CAPABILITY_GAPS.md` §B recorded
/// "0 shaping knobs" against a pass that demonstrably works — the `ssr` demo measures the floor
/// under each cube picking up that cube's colour, +37.6 R / +61.1 G / +38.7 B.
///
/// The march is in screen space: the ray is projected and walked across the screen in at most
/// [`max_steps`](Self::max_steps) equal steps (one a pixel at the finest) until it leaves it or has
/// gone [`max_distance`](Self::max_distance), so how far it reaches is the screen's business, not
/// a step size's. Until 2026-11-19 it walked equal **world** steps, `step_size × max_steps` = 20 m by
/// default: a wet street, whose reflections are the lights and signs a hundred metres down it,
/// showed none of them.
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
    /// How far a ray may go, in metres. Default **500** — past where almost every ray has left
    /// the screen: the march's steps are spread over the part of the ray that is on screen and
    /// nearer than this, so a shorter one buys finer steps over less of the world.
    pub max_distance: f32,
    /// The most steps a ray takes across the screen. The cost knob: a ray shorter on screen than
    /// this many pixels takes one a pixel. Default **48**.
    pub max_steps: f32,
    /// How thick a surface is assumed to be, in metres: the ray hits it when, in one step, it goes
    /// from in front of the surface's back (this far behind its front) to behind its front.
    ///
    /// Too small and the ray passes through thin geometry; too large and it reports a hit on
    /// something it passed behind. Default **1.0**.
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
            max_distance: 500.0,
            max_steps: 48.0,
            thickness: 1.0,
            start_offset: 0.1,
            edge_fade: 0.1,
        }
    }
}
use crate::pipeline::{load_shader, SceneState};

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
    /// Their uniform buffer, with the frame's noise index after them ([`upload`](Self::upload)).
    pub params_buffer: wgpu::Buffer,
}

/// What the march's uniform holds: the knobs, and which frame's noise to start the steps with.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SsrUniform {
    params: SsrParams,
    frame: u32,
    _pad: [u32; 3],
}

impl SsrState {
    /// Writes [`params`](Self::params) to the GPU with the frame's noise index: where in its first
    /// step each pixel's ray starts is noise, so that the steps' banding becomes grain a temporal
    /// filter averages away. `frame` is TAA's frame counter, or 0 without TAA, where a fixed grain
    /// is better than one that flickers.
    pub fn upload(&self, queue: &wgpu::Queue, frame: u32) {
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&SsrUniform { params: self.params, frame, _pad: [0; 3] }));
    }

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
                contents: bytemuck::bytes_of(&SsrUniform { params, frame: 0, _pad: [0; 3] }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            },
        );

        let ssr_bind_group = Self::mk_ssr_bg(
            device,
            &ssr_bgl,
            hdr_view,
            &deferred.normal_roughness_view,
            &deferred.world_position_view,
            &deferred.emissive_view,
            &deferred.albedo_metallic_view,
            &deferred.velocity_view,
            &linear_sampler,
            &params_buffer,
        );

        let apply_bind_group = Self::mk_apply_bg(device, &apply_bgl, &ssr_view, &linear_sampler, &deferred.seen_view);

        let ssr_pipeline = Self::mk_ssr_pipeline(device, scene, &ssr_bgl, &deferred.irradiance.bind_group_layout);
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
            &deferred.emissive_view,
            &deferred.albedo_metallic_view,
            &deferred.velocity_view,
            &self.nearest_sampler,
            &self.params_buffer,
        );
        self.apply_bind_group =
            Self::mk_apply_bg(device, &self.apply_bgl, &ssr_view, &self.nearest_sampler, &deferred.seen_view);

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
                wgpu::BindGroupLayoutEntry {
                    // t_emissive: RT5, whose alpha is the surface's reflectance
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    // t_albedo_metallic: RT0
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    // RT4: the specular tint's hue in `zw` (`gbuffer.wgsl`)
                    binding: 7,
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
                // How much of the G-buffer's surface the frame shows (`DeferredState::seen_view`):
                // the term is added in proportion, so not over what was drawn forward in front of it.
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
            ],
        })
    }

    fn mk_ssr_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        hdr_view: &wgpu::TextureView,
        normal_view: &wgpu::TextureView,
        pos_view: &wgpu::TextureView,
        emissive_view: &wgpu::TextureView,
        albedo_metallic_view: &wgpu::TextureView,
        velocity_view: &wgpu::TextureView,
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
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(emissive_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(albedo_metallic_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(velocity_view),
                },
            ],
        })
    }

    fn mk_apply_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        ssr_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        seen: &wgpu::TextureView,
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(seen),
                },
            ],
        })
    }

    fn mk_ssr_pipeline(
        device: &wgpu::Device,
        scene: &SceneState,
        bgl: &wgpu::BindGroupLayout,
        reflections: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let shader = crate::pipeline::shaders::load_shader_composed_with(
            device,
            "demo/assets/shaders/ssr.wgsl",
            include_str!("shaders/ssr.wgsl"),
            "SSR Shader",
            &[("REFLECTION_GROUP", naga_oil::compose::ShaderDefValue::UInt(2))],
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ssr_layout"),
            // Group 2 is the irradiance volume's, which holds the reflection probes.
            bind_group_layouts: &[Some(&scene.global_bind_group_layout), Some(bgl), Some(reflections)],
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

crate::view_targets::screen_targets! {
    SsrState => SsrTargets {
        key apply_bgl: wgpu::BindGroupLayout;
        apply_bind_group: wgpu::BindGroup,
        ssr_bind_group: wgpu::BindGroup,
        ssr_texture: wgpu::Texture,
        ssr_view: wgpu::TextureView,
        width: u32,
        height: u32,
    }
}
