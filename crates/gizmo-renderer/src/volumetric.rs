
use crate::pipeline::{load_shader_composed, SceneState};

/// The six numbers that shape the volumetric march.
///
/// Every one of these used to be a literal inside `volumetric.wgsl`, which is why
/// `CAPABILITY_GAPS.md` recorded "0 shaping fields" against an effect that demonstrably moves
/// 19.55 % of the frame. They are the shader's own defaults, so a state built and left alone
/// renders exactly what it rendered before this struct existed — locked by
/// `the_defaults_are_the_shader_literals_they_replaced`.
///
/// Written to the GPU each frame from [`VolumetricState::params`]; changing a field is enough,
/// there is nothing to rebuild.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VolumetricParams {
    /// Henyey-Greenstein asymmetry, `g`. 0 scatters evenly in every direction; approaching 1
    /// throws the light forward, so a shaft is bright only when the camera looks near the sun.
    /// Shader default **0.55**.
    pub phase_g: f32,
    /// Raymarch steps per pixel. The single cost knob: halving it roughly halves the pass, at the
    /// price of banding the jitter has to hide. Shader default **16**.
    pub steps: f32,
    /// How far a ray marches when it hits no geometry, in metres — the sky's march length, and the
    /// cap on every other ray. Shader default **100**.
    pub max_distance: f32,
    /// Scattering coefficient for the directional light. Shader default **0.0015**.
    pub sun_scatter: f32,
    /// Scattering coefficient for the local lights — point, spot and rectangle — per metre of air:
    /// what of each reaches a point of the air as it would reach a surface there (the inverse square,
    /// windowed at the radius), through a spot's cone and cookie, on a rectangle's lit side and past
    /// the light's shadow when it casts one, scattered towards the camera by the phase function.
    /// Default **0.0005**, a thin haze: a light of intensity 40 seen a metre off gives its air a halo of
    /// about 0.04 (`I · σ · phase · π / b`), and a camera standing in a light of 60 — a reflection probe at
    /// a room's lamp — air of about 0.6 down every ray, as the windowed `(1 − d/r)²` this replaced gave
    /// with its 0.0008 (2026-11-20). Matching that falloff's air two metres from a light instead took
    /// 0.04, a fog: every ray near a light went white. A light of a real lamp's intensity (a thousand)
    /// wants about the default.
    pub bulb_scatter: f32,
    /// How far past a shadow map's stored depth a point of the air is compared, in metres (times
    /// `ShadowSettings::depth_bias`), so that a march step landing on a surface is not shadowed by
    /// it: for the sun's cascades and the local lights' layers alike. Shader default **0.16**.
    pub shadow_bias: f32,
    /// Padding to a 32-byte uniform. Not a knob.
    pub _pad: [f32; 2],
}

impl Default for VolumetricParams {
    fn default() -> Self {
        Self {
            phase_g: 0.55,
            steps: 16.0,
            max_distance: 100.0,
            sun_scatter: 0.0015,
            bulb_scatter: 0.0005,
            shadow_bias: 0.16,
            _pad: [0.0; 2],
        }
    }
}

/// Whether a renderer's volumetric light is on as it is built ([`VolumetricState::enabled`]): on the
/// desktop, and **off on the forward-only path** — a browser's, a phone's — where a game turns it on.
/// Measured on an Adreno 618 (`nisanci`, portrait, twenty readings each, one sitting, 2026-11-23): 119.6 fps
/// without it, 30.1 with the desktop's march (half the frame's size a side), 54.3 with a sixteenth of the
/// pixels — 61.7 since the passes it does not need are skipped (`set_frame_scatters`, `veil_stale`),
/// against 114.1 and 121.9 off. The owner's choice: built there, off until asked for.
pub const ENABLED_BY_DEFAULT: bool = !cfg!(gizmo_forward_only);

/// The march's divisor as a state is built ([`VolumetricState::march_divisor`]): 2 on the desktop, 4 on
/// the forward-only path, where what the march costs a phone is its pixels (see [`ENABLED_BY_DEFAULT`]).
pub const DEFAULT_MARCH_DIVISOR: u32 = if cfg!(gizmo_forward_only) { 4 } else { 2 };

/// Volumetric lighting — the shafts of light ("god rays") raymarched through the sun's cascades and
/// the local lights' shadow layers.
///
/// Marched at reduced resolution — half each way by default on the desktop, a quarter on the
/// forward-only path ([`DEFAULT_MARCH_DIVISOR`]), [`set_march_divisor`](Self::set_march_divisor) for
/// another — and applied over the HDR target through a depth-aware upsample: each pixel takes the march's
/// four nearest texels weighted by how near their surfaces stand to its own, so the air marched past a
/// lamp post does not spill onto the post. The effect is low-frequency; the cost is the march's pixels.
///
/// On every path: it reads the frame's depth, the sun's cascades, the local lights' layers and the
/// clusters, which the forward-only path (a browser's, a phone's) has as the desktop does, and draws the
/// veil through the shadow pipelines ([`veil_pass`](Self::veil_pass)), which every path builds. Built
/// over the G-buffer's depth until 2026-11-23, and so on no browser or phone.
pub struct VolumetricState {
    /// The marched scattering term in front of the nearest surface that blends — all of it where
    /// none does — added over the finished frame.
    pub volumetric_texture: wgpu::Texture,
    /// Its view.
    pub volumetric_view: wgpu::TextureView,
    /// The scattering term behind the nearest surface that blends, added ahead of what blends, which
    /// shows it as much as it lets through. `COPY_SRC`, as the front is.
    pub behind_texture: wgpu::Texture,
    /// Its view.
    pub behind_view: wgpu::TextureView,
    /// The depth of the nearest surface that blends — glass, a transparent surface — drawn alone,
    /// depth only, ahead of them: where the march splits the air between the two terms. Frame-sized.
    pub veil_depth_texture: wgpu::Texture,
    /// Its view, the depth attachment of that pass.
    pub veil_depth_view: wgpu::TextureView,
    /// That pass's group 0, in the shadow pass's layout: the camera's matrix where a cascade's stands, so
    /// that the pass draws through the shadow pipelines — depth only, both faces, cutting what a
    /// cut-out cuts — which every path has. Written each view by [`write_veil_pass`](Self::write_veil_pass).
    pub veil_pass: wgpu::BindGroup,
    veil_pass_buffer: wgpu::Buffer,

    /// The raymarch pass.
    pub volumetric_pipeline: wgpu::RenderPipeline,
    volumetric_bgl: wgpu::BindGroupLayout,
    /// Its depth and shadow-map inputs. Rebuilt on resize.
    pub volumetric_bind_group: wgpu::BindGroup,

    /// The apply pass, adding the scattering over the HDR target.
    pub apply_pipeline: wgpu::RenderPipeline,
    apply_bgl: wgpu::BindGroupLayout,
    /// Its input for the air in front. Rebuilt on resize.
    pub apply_bind_group: wgpu::BindGroup,
    /// Its input for the air behind. Rebuilt on resize.
    pub behind_apply_bind_group: wgpu::BindGroup,

    linear_sampler: wgpu::Sampler,

    /// The width of the frame it was built for, in pixels; the march target is this over
    /// [`march_divisor`](Self::march_divisor).
    pub width: u32,
    /// That frame's height.
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
    /// off, so switching back on costs no rebuild. [`ENABLED_BY_DEFAULT`] as built: off on the
    /// forward-only path.
    pub enabled: bool,
    /// Whether anything in the view drawn now scatters into the air: a sun with
    /// [`sun_scatter`](VolumetricParams::sun_scatter), or a light with either coefficient. Set for each
    /// view by the frame ([`set_frame_scatters`](Self::set_frame_scatters)); without anything to scatter
    /// the pass is skipped as if off, and the frame is what it would be with no air at all.
    scatters: bool,
    /// Whether the veil's target holds anything but the clear depth: something was drawn into it, or it
    /// is new. A frame with nothing that blends skips the veil's pass while it is clear.
    veil_stale: bool,

    /// The six shaping numbers. Write to them directly; they reach the GPU on the next frame.
    pub params: VolumetricParams,
    /// The frame's size over the march's, each way: 2 (the desktop's default) marches a quarter of its
    /// pixels, 4 (the forward-only path's) a sixteenth. Set through [`set_march_divisor`](Self::set_march_divisor), which rebuilds the targets.
    march_divisor: u32,
    /// Their uniform buffer.
    pub params_buffer: wgpu::Buffer,
}

impl VolumetricState {
    /// Builds every volumetric resource for a target of the given size, over the frame's depth
    /// (`depth_view`, the renderer's `depth_texture_view`), where each ray stops.
    pub fn new(
        device: &wgpu::Device,
        scene: &SceneState,
        depth_view: &wgpu::TextureView,
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

        let march_divisor = DEFAULT_MARCH_DIVISOR;
        let (march_w, march_h) = ((width / march_divisor).max(1), (height / march_divisor).max(1));
        let (volumetric_texture, volumetric_view) = Self::mk_tex(device, march_w, march_h);
        let (behind_texture, behind_view) = Self::mk_tex(device, march_w, march_h);
        let (veil_depth_texture, veil_depth_view) = Self::mk_veil_depth(device, width, height);

        let volumetric_bgl = Self::mk_volumetric_bgl(device);
        let apply_bgl = Self::mk_apply_bgl(device);

        let params = VolumetricParams::default();
        let params_buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("volumetric_params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            },
        );

        let volumetric_bind_group = Self::mk_volumetric_bg(
            device,
            &volumetric_bgl,
            depth_view,
            &linear_sampler,
            &params_buffer,
            &veil_depth_view,
        );

        let apply_bind_group = Self::mk_apply_bg(device, &apply_bgl, &volumetric_view, &linear_sampler, depth_view);
        let behind_apply_bind_group = Self::mk_apply_bg(device, &apply_bgl, &behind_view, &linear_sampler, depth_view);

        let veil_pass_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("volumetric veil pass"),
            size: std::mem::size_of::<crate::gpu_types::ShadowVsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let veil_pass = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("volumetric veil pass bg"),
            layout: &scene.shadow_pass_bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: veil_pass_buffer.as_entire_binding() }],
        });

        let volumetric_pipeline = Self::mk_volumetric_pipeline(device, scene, &volumetric_bgl);
        let apply_pipeline = Self::mk_apply_pipeline(device, &apply_bgl, scene);

        Self {
            enabled: ENABLED_BY_DEFAULT,
            scatters: true,
            veil_stale: true,
            params,
            params_buffer,
            volumetric_texture,
            volumetric_view,
            behind_texture,
            behind_view,
            veil_depth_texture,
            veil_depth_view,
            veil_pass,
            veil_pass_buffer,
            volumetric_pipeline,
            volumetric_bgl,
            volumetric_bind_group,
            apply_pipeline,
            apply_bgl,
            apply_bind_group,
            behind_apply_bind_group,
            linear_sampler,
            width,
            height,
            march_divisor,
        }
    }

    /// The frame's size over the march's, each way ([`march_divisor`](Self::march_divisor)).
    #[must_use]
    pub fn march_divisor(&self) -> u32 {
        self.march_divisor
    }

    /// Whether the pass draws in the view drawn now: [`enabled`](Self::enabled), and something there
    /// scatters ([`set_frame_scatters`](Self::set_frame_scatters)).
    #[must_use]
    pub fn draws(&self) -> bool {
        self.enabled && self.scatters
    }

    /// Says whether anything in the view about to be drawn scatters into the air, with these params:
    /// a sun (`sun`), or any light (`lights`). True until a frame says otherwise.
    pub fn set_frame_scatters(&mut self, sun: bool, lights: bool) {
        let p = &self.params;
        self.scatters = (sun && p.sun_scatter > 0.0) || (lights && (p.bulb_scatter > 0.0 || p.sun_scatter > 0.0));
    }

    /// Whether the veil's target has to be drawn this frame although nothing blends: to clear what an
    /// earlier frame drew into it, or a new target's contents.
    #[must_use]
    pub fn veil_stale(&self) -> bool {
        self.veil_stale
    }

    /// Records the veil's pass: `drew`, whether anything was drawn into it after its clear.
    pub fn veil_drawn(&mut self, drew: bool) {
        self.veil_stale = drew;
    }

    /// Marches one pixel of `divisor` × `divisor` of the frame's (1 – 8; [`DEFAULT_MARCH_DIVISOR`] as built), rebuilding the
    /// targets when it changes. The march's cost is its pixels: 4 is a quarter of 2's. `nfsu2-gizmo`'s
    /// lamp-lit streets took 4.3 ms of haze at 2 (1080p, AMD 780M).
    pub fn set_march_divisor(&mut self, device: &wgpu::Device, depth_view: &wgpu::TextureView, divisor: u32) {
        let divisor = divisor.clamp(1, 8);
        if divisor != self.march_divisor {
            self.march_divisor = divisor;
            self.resize(device, depth_view, self.width, self.height);
        }
    }

    /// Writes the veil pass's matrix — the view's own, `view_proj`, as its depth was drawn — with the
    /// frame's camera and clock, which a surface that cuts itself by them reads (`ShadowVsUniform`).
    /// Before each view's pass: each view is its own submission.
    pub fn write_veil_pass(&self, queue: &wgpu::Queue, view_proj: gizmo_math::Mat4, camera: gizmo_math::Vec3, clock: f32) {
        let uniform = crate::gpu_types::ShadowVsUniform::new(view_proj.to_cols_array_2d(), camera.to_array(), clock);
        queue.write_buffer(&self.veil_pass_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// Rebuilds the size-dependent textures and the bind groups that read them, over the frame's depth
    /// at that size (`depth_view`).
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let march_w = (width / self.march_divisor).max(1);
        let march_h = (height / self.march_divisor).max(1);
        let (t, v) = Self::mk_tex(device, march_w, march_h);
        self.volumetric_texture = t;
        self.volumetric_view = v;
        let (t, v) = Self::mk_tex(device, march_w, march_h);
        self.behind_texture = t;
        self.behind_view = v;
        let (t, v) = Self::mk_veil_depth(device, width, height);
        self.veil_depth_texture = t;
        self.veil_depth_view = v;
        self.veil_stale = true;

        self.volumetric_bind_group = Self::mk_volumetric_bg(
            device,
            &self.volumetric_bgl,
            depth_view,
            &self.linear_sampler,
            &self.params_buffer,
            &self.veil_depth_view,
        );

        self.apply_bind_group = Self::mk_apply_bg(device, &self.apply_bgl, &self.volumetric_view, &self.linear_sampler, depth_view);
        self.behind_apply_bind_group = Self::mk_apply_bg(device, &self.apply_bgl, &self.behind_view, &self.linear_sampler, depth_view);

        self.width = width;
        self.height = height;
    }

    fn mk_tex(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let t = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("volumetric_texture"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            // `COPY_SRC` so the scattering term can be read back on its own, before the apply pass
            // blurs it into a frame where the surfaces it lights outshine it.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let v = t.create_view(&wgpu::TextureViewDescriptor::default());
        (t, v)
    }

    fn mk_veil_depth(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let t = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("volumetric_veil_depth"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let v = t.create_view(&wgpu::TextureViewDescriptor::default());
        (t, v)
    }

    fn mk_volumetric_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("volumetric_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    // The frame's depth: where each ray stops.
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Depth,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    // sampler
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    // params
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    // The nearest surface that blends: where the air splits.
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Depth,
                    },
                    count: None,
                },
            ],
        })
    }

    fn mk_apply_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("volumetric_apply_bgl"),
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
                // The frame's depth: where each pixel's surface stands, for the upsample's weights.
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Depth,
                    },
                    count: None,
                },
            ],
        })
    }

    fn mk_volumetric_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        depth_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        params: &wgpu::Buffer,
        veil_depth: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("volumetric_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(veil_depth),
                },
            ],
        })
    }

    fn mk_apply_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        vol_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        depth_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("volumetric_apply_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(vol_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
            ],
        })
    }

    fn mk_volumetric_pipeline(
        device: &wgpu::Device,
        scene: &SceneState,
        bgl: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let shader = load_shader_composed(
            device,
            "demo/assets/shaders/volumetric.wgsl",
            include_str!("shaders/volumetric.wgsl"),
            "Volumetric Shader",
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("volumetric_layout"),
            bind_group_layouts: &[
                Some(&scene.global_bind_group_layout), // Group 0
                Some(&scene.shadow_bind_group_layout), // Group 1 (Shadow Maps)
                Some(bgl),                             // Group 2
            ],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("volumetric_pipeline"),
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
                // The air in front of the nearest surface that blends, then behind it.
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
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
        scene: &SceneState,
    ) -> wgpu::RenderPipeline {
        // Composed: it imports the scene's uniform, for the camera.
        let shader = crate::pipeline::load_shader_composed(
            device,
            "demo/assets/shaders/volumetric_apply.wgsl",
            include_str!("shaders/volumetric_apply.wgsl"),
            "Volumetric Apply Shader",
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("volumetric_apply_layout"),
            // Group 1: the scene's, for the camera that turns a pixel's depth into a distance.
            bind_group_layouts: &[Some(bgl), Some(&scene.global_bind_group_layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("volumetric_apply_pipeline"),
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
                    // Additive blending!
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
    VolumetricState => VolumetricTargets {
        key apply_bgl: wgpu::BindGroupLayout;
        apply_bind_group: wgpu::BindGroup,
        behind_apply_bind_group: wgpu::BindGroup,
        behind_texture: wgpu::Texture,
        behind_view: wgpu::TextureView,
        veil_depth_texture: wgpu::Texture,
        veil_depth_view: wgpu::TextureView,
        veil_stale: bool,
        volumetric_bind_group: wgpu::BindGroup,
        volumetric_texture: wgpu::Texture,
        volumetric_view: wgpu::TextureView,
        width: u32,
        height: u32,
    }
}
