use crate::gpu_types::PostProcessUniforms;
use wgpu::util::DeviceExt;

/// The post-processing state — the HDR target, the bloom's chain and the composite, their pipelines
/// and their resources.
pub struct PostProcessState {
    /// Layout of the "one HDR texture + sampler" group every pass in the chain binds.
    pub post_bind_group_layout: wgpu::BindGroupLayout,
    /// Layout of the composite pass's group: the bloom, and the depth for DoF.
    pub composite_bloom_bind_group_layout: wgpu::BindGroupLayout,
    /// The [`PostProcessUniforms`] buffer, rewritten each frame.
    pub post_params_buffer: wgpu::Buffer,
    /// Its layout.
    pub post_params_bind_group_layout: wgpu::BindGroupLayout,
    /// Its bind group. Never rebuilt — the buffer outlives every resize.
    pub post_params_bind_group: wgpu::BindGroup,
    /// The bloom's passes ([`BloomPipelines`]); `None` on the forward-only path, which has no bloom.
    pub bloom_pipelines: Option<BloomPipelines>,
    /// The composite pass: tone map, add bloom, and apply DoF, grain, vignette and aberration.
    pub composite_pipeline: wgpu::RenderPipeline,
    /// The module the pipelines were built from, kept to build them again for another
    /// [`CompositeVariant`].
    pub(crate) shader: wgpu::ShaderModule,
    /// What the composite was last built for. See [`sync_composite`].
    pub(crate) composite_variant: CompositeVariant,
    /// The colour look-up table the composite reads; an empty one of two texels while none is set, which
    /// mode 0 never reads.
    pub(crate) lut_view: wgpu::TextureView,
    /// The HDR target the world is rendered into, ahead of tone mapping.
    pub hdr_texture: wgpu::Texture,
    /// Its view.
    pub hdr_texture_view: wgpu::TextureView,
    /// It as an input to the chain.
    pub hdr_bind_group: wgpu::BindGroup,
    /// The composite pass's group 3: the exposure it multiplies by. `None` in the browser, whose
    /// reduced post shader has no such stage. See [`ExposureBinding`].
    pub exposure: Option<ExposureBinding>,
    /// The composite pass's group 4: the G-buffer's velocity target, for object motion blur.
    /// `None` in the browser. See [`VelocityBinding`].
    pub velocity: Option<VelocityBinding>,
    /// The composite pass's group 5: local exposure's grid. `None` in the browser. See
    /// [`crate::local_exposure`].
    pub local: Option<crate::local_exposure::LocalExposureBinding>,
    /// The bloom's levels, the first of which is the bloom the composite adds.
    pub bloom: BloomChain,
    /// The bloom and the depth buffer, as the composite pass reads them.
    pub composite_bloom_bind_group: wgpu::BindGroup,
}

/// The bloom's three passes, all drawn from `post_process.wgsl`: the first level from the frame's bright
/// part (`fs_bloom_prefilter`), each level after from the one above it (`fs_bloom_down`), and each level
/// taking in the one below it, coming back up (`fs_bloom_up`, blended by the scatter).
pub struct BloomPipelines {
    /// The first level, from the frame.
    pub prefilter: wgpu::RenderPipeline,
    /// A level from the one above it.
    pub down: wgpu::RenderPipeline,
    /// A level's share of the one below it, blended in by the constant ([`Renderer::bloom_scatter`](crate::Renderer::bloom_scatter)).
    pub up: wgpu::RenderPipeline,
}

/// The most levels the bloom's chain has.
pub const BLOOM_LEVELS: u32 = 8;
/// The short side, in texels, that the chain halves down to and no further.
pub const BLOOM_SMALLEST: u32 = 8;
/// The bloom's default strength (`Renderer::bloom_intensity`, `PostProcess::bloom_intensity`): Unreal's.
/// Its bloom adds six blurs of the frame at an intensity of 0.675, tinted 0.35, 0.14, 0.12, 0.07, 0.07 and
/// 0.06 — about 0.54 of the frame's light, all told. This chain's levels add to one, so its intensity is
/// that share. It was 0.8 while the bloom was a blur a couple of pixels wide, where most of what it added
/// fell back on the clipped highlight that made it; spread across the frame, 0.8 of a sun's reflection in
/// a mirror whitened the whole mirror.
pub const DEFAULT_BLOOM_INTENSITY: f32 = 0.5;
/// [`Renderer::bloom_scatter`](crate::Renderer::bloom_scatter)'s default: as Unity's bloom has it, wide
/// enough that a street lamp's glow reaches across the road, and most of the light still near the lamp.
pub const DEFAULT_BLOOM_SCATTER: f32 = 0.7;

/// The size of the bloom's first level for a `width × height` frame, half each way, and how many levels
/// its chain has: halving until the short side would fall under [`BLOOM_SMALLEST`], at most
/// [`BLOOM_LEVELS`]. Seven for 1080p, from 960×540 down to 15×8.
#[must_use]
pub fn bloom_levels(width: u32, height: u32) -> ((u32, u32), u32) {
    let first = ((width / 2).max(1), (height / 2).max(1));
    let (mut short, mut levels) = (first.0.min(first.1), 1);
    while levels < BLOOM_LEVELS && short / 2 >= BLOOM_SMALLEST {
        short /= 2;
        levels += 1;
    }
    (first, levels)
}

/// The bloom's levels: one texture, its first mip half the frame's size and each after half the one
/// before ([`bloom_levels`]). Each level is a render target and, in a group of its own, a source.
#[derive(Clone)]
pub struct BloomChain {
    texture: wgpu::Texture,
    targets: Vec<wgpu::TextureView>,
    sources: Vec<wgpu::BindGroup>,
    /// The first level: the finished bloom, as the composite reads it.
    pub view: wgpu::TextureView,
}

impl BloomChain {
    /// The chain for a `width × height` frame, each level's source group in `post_bgl` with `sampler`. A
    /// texel on the forward-only path, which has no bloom: its composite binds it and never samples it.
    pub fn new(device: &wgpu::Device, post_bgl: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, width: u32, height: u32) -> Self {
        let ((w, h), levels) = if cfg!(gizmo_forward_only) { ((1, 1), 1) } else { bloom_levels(width, height) };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Bloom Chain"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let level = |mip: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("Bloom Level"),
                base_mip_level: mip,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let targets: Vec<wgpu::TextureView> = (0..levels).map(level).collect();
        let sources = targets
            .iter()
            .map(|view| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("bloom_level_bind_group"),
                    layout: post_bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                    ],
                })
            })
            .collect();
        Self { view: level(0), texture, targets, sources }
    }

    /// How many levels the chain has.
    #[must_use]
    pub fn levels(&self) -> u32 {
        self.targets.len() as u32
    }

    /// The texture, every level a mip of it: for reading the bloom back.
    #[must_use]
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }
}

impl PostProcessState {
    /// Records the bloom: the bright part of the HDR target filtered down the chain level by level and
    /// back up, each level on the way up taking `scatter` (0–1) of the one below it and keeping the rest
    /// of its own — weights that add to one, so the bloom holds the bright part's light at any scatter,
    /// tight at 0 and wide towards 1. Into [`BloomChain::view`]. Nothing on the forward-only path.
    pub fn record_bloom(&self, encoder: &mut wgpu::CommandEncoder, scatter: f32) {
        let Some(pipelines) = self.bloom_pipelines.as_ref() else { return };
        let scatter = f64::from(if scatter.is_finite() { scatter.clamp(0.0, 1.0) } else { DEFAULT_BLOOM_SCATTER });
        let chain = &self.bloom;
        let mut draw = |label: &str,
                        target: &wgpu::TextureView,
                        pipeline: &wgpu::RenderPipeline,
                        source: &wgpu::BindGroup,
                        params: bool,
                        blend: Option<f64>| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // On the way up the level keeps what it holds, which the blend weighs.
                        load: if blend.is_some() { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::BLACK) },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, source, &[]);
            if params {
                pass.set_bind_group(2, &self.post_params_bind_group, &[]);
            }
            if let Some(s) = blend {
                pass.set_blend_constant(wgpu::Color { r: s, g: s, b: s, a: s });
            }
            pass.draw(0..3, 0..1);
        };
        let levels = chain.targets.len();
        draw("Bloom Prefilter", &chain.targets[0], &pipelines.prefilter, &self.hdr_bind_group, true, None);
        for i in 1..levels {
            draw("Bloom Down", &chain.targets[i], &pipelines.down, &chain.sources[i - 1], false, None);
        }
        for i in (0..levels.saturating_sub(1)).rev() {
            draw("Bloom Up", &chain.targets[i], &pipelines.up, &chain.sources[i + 1], false, Some(scatter));
        }
    }
}

/// Builds the whole post-process chain: shaders, three pipelines, every target and every bind
/// group.
///
/// The shader is loaded through the disk-override path on native (so it hot-reloads) and from a
/// separate, reduced `post_process_wasm.wgsl` in the browser.
pub fn build_post_process_resources(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    depth_view: &wgpu::TextureView,
) -> PostProcessState {
    let post_shader = post_shader_module(device);

    let post_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post_bind_group_layout"),
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
        });

    // WASM: depth texture'ı composite layout'tan kaldır (textureLoad kullanılıyor, sampler gerekmez)
    // Native: depth texture sampler ile kalır
    #[cfg(not(gizmo_forward_only))]
    let composite_bloom_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite_bloom_bind_group_layout"),
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
                // The colour look-up table (`crate::color_lut`), read through binding 1's sampler.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D3,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
            ],
        });
    #[cfg(gizmo_forward_only)]
    let composite_bloom_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite_bloom_bind_group_layout"),
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
                // The colour look-up table (`crate::color_lut`), read through binding 1's sampler.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D3,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
            ],
        });

    let post_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Post Process Params Buffer"),
        // The neutral look lives in `PostProcessUniforms::default()` (frame_uniforms.rs), which is
        // where the per-knob reasoning went with it — this was its only definition before.
        contents: bytemuck::cast_slice(&[PostProcessUniforms::default()]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let post_params_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post_params_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

    let post_params_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &post_params_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: post_params_buffer.as_entire_binding(),
        }],
        label: Some("post_params_bind_group"),
    });

    #[cfg(not(gizmo_forward_only))]
    let exposure = Some(build_exposure_binding(device));
    #[cfg(gizmo_forward_only)]
    let exposure: Option<ExposureBinding> = None;
    #[cfg(not(gizmo_forward_only))]
    let velocity = Some(build_velocity_binding(device));
    #[cfg(gizmo_forward_only)]
    let velocity: Option<VelocityBinding> = None;
    #[cfg(not(gizmo_forward_only))]
    let local = Some(crate::local_exposure::build_binding(device));
    #[cfg(gizmo_forward_only)]
    let local: Option<crate::local_exposure::LocalExposureBinding> = None;

    let composite_variant = CompositeVariant::default();
    let (bloom_pipelines, composite_pipeline) = build_post_pipelines(
        device,
        &post_shader,
        &post_bind_group_layout,
        &composite_bloom_bind_group_layout,
        &post_params_bind_group_layout,
        exposure.as_ref().map(|e| &e.layout),
        velocity.as_ref().map(|v| &v.layout),
        local.as_ref().map(|l| &l.layout),
        surface_format,
        composite_variant,
    );

    let post_sampler = create_post_sampler(device);
    let lut_view = crate::color_lut::unset_view(device);
    let (hdr_texture, hdr_texture_view, hdr_bind_group, bloom, composite_bloom_bind_group) = create_post_textures(
        device,
        &post_bind_group_layout,
        &composite_bloom_bind_group_layout,
        &post_sampler,
        width,
        height,
        depth_view,
        &lut_view,
    );

    PostProcessState {
        post_bind_group_layout,
        composite_bloom_bind_group_layout,
        post_params_buffer,
        post_params_bind_group_layout,
        post_params_bind_group,
        bloom_pipelines,
        composite_pipeline,
        shader: post_shader,
        composite_variant,
        lut_view,
        hdr_texture,
        hdr_texture_view,
        hdr_bind_group,
        bloom,
        composite_bloom_bind_group,
        exposure,
        velocity,
        local,
    }
}

/// The composite pass's **exposure input** — group 3, one read-only storage `f32`.
///
/// It is where [`AutoExposureState`](crate::auto_exposure::AutoExposureState) delivers its result
/// without the CPU seeing it. While auto exposure is off the pass binds
/// [`neutral_bind_group`](Self::neutral_bind_group), whose buffer holds exactly `1.0`, so the
/// multiplication leaves every pixel bit-identical.
#[non_exhaustive]
pub struct ExposureBinding {
    /// The group-3 layout, shared by the neutral group and every auto-exposure state's.
    pub layout: wgpu::BindGroupLayout,
    /// A buffer holding `1.0`.
    pub neutral_buffer: wgpu::Buffer,
    /// Group 3 bound to [`neutral_buffer`](Self::neutral_buffer).
    pub neutral_bind_group: wgpu::BindGroup,
}

/// The composite pass's **velocity input** — group 4, one texture: the G-buffer's per-pixel motion
/// (`DeferredState::velocity_view`, see [`crate::motion`]), which camera *and object* motion blur
/// read. Built per frame around the deferred state's view; a renderer without one binds
/// [`placeholder_bind_group`](Self::placeholder_bind_group), a 1×1 zero.
#[non_exhaustive]
pub struct VelocityBinding {
    /// The group-4 layout.
    pub layout: wgpu::BindGroupLayout,
    /// Group 4 bound to a 1×1 texture of zero velocity.
    pub placeholder_bind_group: wgpu::BindGroup,
}

/// The layout of the composite pass's velocity group (see [`VelocityBinding`]).
pub fn velocity_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("velocity_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                view_dimension: wgpu::TextureViewDimension::D2,
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
            },
            count: None,
        }],
    })
}

/// Group 4 around `view` — a velocity texture.
pub fn velocity_bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, view: &wgpu::TextureView) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("velocity_bind_group"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) }],
    })
}

#[cfg(not(gizmo_forward_only))]
fn build_velocity_binding(device: &wgpu::Device) -> VelocityBinding {
    let layout = velocity_layout(device);
    let placeholder = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("velocity_placeholder"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::motion::VELOCITY_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let placeholder_bind_group = velocity_bind_group(device, &layout, &placeholder);
    VelocityBinding { layout, placeholder_bind_group }
}

/// The layout of the composite pass's exposure group (see [`ExposureBinding`]).
pub fn exposure_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("exposure_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

#[cfg(not(gizmo_forward_only))]
fn build_exposure_binding(device: &wgpu::Device) -> ExposureBinding {
    let layout = exposure_layout(device);
    let neutral_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("neutral_exposure"),
        contents: bytemuck::cast_slice(&[1.0f32, 0.0, 0.0, 0.0]),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let neutral_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("neutral_exposure"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: neutral_buffer.as_entire_binding() }],
    });
    ExposureBinding { layout, neutral_buffer, neutral_bind_group }
}

#[allow(clippy::too_many_arguments)]
fn build_post_pipelines(
    device: &wgpu::Device,
    post_shader: &wgpu::ShaderModule,
    post_bgl: &wgpu::BindGroupLayout,
    composite_bloom_bgl: &wgpu::BindGroupLayout,
    post_params_bgl: &wgpu::BindGroupLayout,
    exposure_bgl: Option<&wgpu::BindGroupLayout>,
    velocity_bgl: Option<&wgpu::BindGroupLayout>,
    local_bgl: Option<&wgpu::BindGroupLayout>,
    surface_format: wgpu::TextureFormat,
    variant: CompositeVariant,
) -> (Option<BloomPipelines>, wgpu::RenderPipeline) {
    // The bloom's, on the desktop path: the forward-only shader has no bloom to draw them from.
    let bloom_pipelines = (!cfg!(gizmo_forward_only)).then(|| {
        let pipeline = |label: &str, layout: &wgpu::PipelineLayout, entry: &str, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: post_shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: post_shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        // The prefilter reads the frame and the parameters (its threshold), at group 2 as the composite
        // reads them; the levels read a level.
        let prefilter_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Bloom Prefilter Pipeline Layout"),
            bind_group_layouts: &[Some(post_bgl), None, Some(post_params_bgl)],
            immediate_size: 0,
        });
        let level_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Bloom Level Pipeline Layout"),
            bind_group_layouts: &[Some(post_bgl)],
            immediate_size: 0,
        });
        // Up the chain: the scatter (the blend constant) of what the pass draws, the rest of what the level holds.
        let scatter = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Constant,
            dst_factor: wgpu::BlendFactor::OneMinusConstant,
            operation: wgpu::BlendOperation::Add,
        };
        BloomPipelines {
            prefilter: pipeline("Bloom Prefilter Pipeline", &prefilter_layout, "fs_bloom_prefilter", wgpu::BlendState::REPLACE),
            down: pipeline("Bloom Down Pipeline", &level_layout, "fs_bloom_down", wgpu::BlendState::REPLACE),
            up: pipeline("Bloom Up Pipeline", &level_layout, "fs_bloom_up", wgpu::BlendState { color: scatter, alpha: scatter }),
        }
    });

    let composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Composite Pipeline Layout"),
        // Group 3 only where the composite reads an exposure buffer (native).
        bind_group_layouts: &[
            Some(post_bgl),
            Some(composite_bloom_bgl),
            Some(post_params_bgl),
            exposure_bgl,
            velocity_bgl,
            local_bgl,
        ][..match (exposure_bgl, velocity_bgl, local_bgl) {
            (Some(_), Some(_), Some(_)) => 6,
            (Some(_), Some(_), None) => 5,
            (Some(_), None, _) => 4,
            _ => 3,
        }],
        immediate_size: 0,
    });
    let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Composite Pipeline"),
        layout: Some(&composite_layout),
        vertex: wgpu::VertexState {
            module: post_shader,
            entry_point: Some("vs_fullscreen"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: post_shader,
            entry_point: Some("fs_composite"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &composite_constants(surface_format, variant)[..],
                ..Default::default()
            },
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
            cache: None,
    });

    (bloom_pipelines, composite_pipeline)
}

/// Recompiles just the post-process pipelines against a new shader module, leaving every texture and
/// bind group in place — what shader hot-reload calls.
pub fn rebuild_post_pipelines(renderer: &mut crate::Renderer, post_shader: &wgpu::ShaderModule) {
    let variant = renderer.post.composite_variant;
    let (bloom, c) = build_post_pipelines(
        &renderer.device,
        post_shader,
        &renderer.post.post_bind_group_layout,
        &renderer.post.composite_bloom_bind_group_layout,
        &renderer.post.post_params_bind_group_layout,
        renderer.post.exposure.as_ref().map(|e| &e.layout),
        renderer.post.velocity.as_ref().map(|v| &v.layout),
        renderer.post.local.as_ref().map(|l| &l.layout),
        renderer.config.format,
        variant,
    );
    renderer.post.bloom_pipelines = bloom;
    renderer.post.composite_pipeline = c;
    renderer.post.shader = post_shader.clone();
    renderer.post.composite_variant = variant;
}

/// What the reduced composite (`post_process_wasm.wgsl`) is built for: its pipeline constants, which a
/// phone compiles as constants and so pays for no curve or dither it is not using. The desktop composite
/// reads the same choices from its parameters every frame and has no such constants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CompositeVariant {
    /// `TonemapCurve::as_f32`.
    pub(crate) tonemap_curve: f32,
    pub(crate) deband_dither: bool,
    /// Whether there is a vignette at all; how strong stays a parameter.
    pub(crate) vignette: bool,
    /// Whether there is film grain at all.
    pub(crate) film_grain: bool,
    /// The colour look-up table's mode, 0 for none.
    pub(crate) color_lut: f32,
}

impl Default for CompositeVariant {
    fn default() -> Self {
        Self {
            tonemap_curve: crate::gpu_types::TonemapCurve::default().as_f32(),
            deband_dither: false,
            vignette: false,
            film_grain: false,
            color_lut: 0.0,
        }
    }
}

impl CompositeVariant {
    /// What a frame's parameters ask for: the renderer's, or a camera's `PostProcess` grade on top.
    pub(crate) fn of(uniforms: &PostProcessUniforms) -> Self {
        Self {
            tonemap_curve: uniforms.tonemap_curve,
            deband_dither: uniforms.deband_dither > 0.5,
            vignette: uniforms.vignette_intensity > 0.0,
            film_grain: uniforms.film_grain_intensity > 0.0,
            color_lut: uniforms.color_lut[0],
        }
    }
}

/// Builds the post pipelines again when this frame's `uniforms` ask for another tone curve, dither,
/// vignette or grain than they were built for, on the forward-only path, whose composite takes those as
/// pipeline constants. Call it once a frame with what is about to be written; it does nothing when nothing
/// changed, and nothing on the desktop.
pub fn sync_composite(renderer: &mut crate::Renderer, uniforms: &PostProcessUniforms) {
    let wanted = CompositeVariant::of(uniforms);
    if !cfg!(gizmo_forward_only) || wanted == renderer.post.composite_variant {
        return;
    }
    renderer.post.composite_variant = wanted;
    let shader = renderer.post.shader.clone();
    rebuild_post_pipelines(renderer, &shader);
}

/// The source of the post shader: `post_common.wgsl` (the parameters, the tone curves, the vignette and the
/// dither, which both composites share) and then `main`, the composite itself.
pub(crate) fn post_shader_source(common: &str, main: &str) -> String {
    format!("{common}\n{main}")
}

/// The post shader this path draws with: the desktop's composite, read from disk first so it hot-reloads,
/// or the reduced one in the browser and on a phone, each behind `post_common.wgsl`.
pub(crate) fn post_shader_module(device: &wgpu::Device) -> wgpu::ShaderModule {
    #[cfg(not(gizmo_forward_only))]
    let source = {
        let read = |path: &str, fallback: &str| std::fs::read_to_string(path).unwrap_or_else(|_| fallback.to_string());
        post_shader_source(
            &read("demo/assets/shaders/post_common.wgsl", include_str!("shaders/post_common.wgsl")),
            &read("demo/assets/shaders/post_process.wgsl", include_str!("shaders/post_process.wgsl")),
        )
    };
    #[cfg(gizmo_forward_only)]
    let source =
        post_shader_source(include_str!("shaders/post_common.wgsl"), include_str!("shaders/post_process_wasm.wgsl"));
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Post-Processing Shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

fn create_post_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

// WASM: composite-bloom bind group'un depth girdisi native dalında — `depth_view`
// web'de kullanılmaz (hedefli allow, native lint gücü korunur).
#[cfg_attr(gizmo_forward_only, allow(unused_variables))]
/// (Re)creates every size-dependent target in the chain and the bind groups that read them, in the order
/// the caller assigns them: the HDR texture, view and group; the bloom's chain; and the composite group.
pub fn create_post_textures(
    device: &wgpu::Device,
    post_bgl: &wgpu::BindGroupLayout,
    composite_bloom_bgl: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
    depth_view: &wgpu::TextureView,
    lut_view: &wgpu::TextureView,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup, BloomChain, wgpu::BindGroup) {
    let hdr = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("HDR Texture"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let hdr_view = hdr.create_view(&wgpu::TextureViewDescriptor::default());
    let hdr_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: post_bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&hdr_view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
        label: Some("HDR Texture_bind_group"),
    });
    let bloom = BloomChain::new(device, post_bgl, sampler, width, height);
    let cb_bg = composite_bloom_group(device, composite_bloom_bgl, &bloom.view, sampler, depth_view, lut_view);
    (hdr, hdr_view, hdr_bg, bloom, cb_bg)
}

/// The composite's group 1: the bloom, its sampler, the depth (the desktop's, for depth of field) and the
/// colour look-up table. Built on a resize and when a table is set.
pub(crate) fn composite_bloom_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    bloom: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    #[cfg_attr(gizmo_forward_only, allow(unused_variables))] depth: &wgpu::TextureView,
    lut: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let shared = [
        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(bloom) },
        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(lut) },
    ];
    #[cfg(not(gizmo_forward_only))]
    let entries =
        [&shared[..], &[wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) }]].concat();
    #[cfg(gizmo_forward_only)]
    let entries = shared.to_vec();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("composite_bloom_bind_group"),
        layout,
        entries: &entries,
    })
}

/// Runs the post-processing render passes in order: the bloom ([`run_bloom`]), then the composite
/// ([`run_composite`]).
#[tracing::instrument(skip_all, level = "trace")]
pub fn run_post_processing(
    renderer: &crate::Renderer,
    encoder: &mut wgpu::CommandEncoder,
    output_view: &wgpu::TextureView,
    region: Option<[f32; 4]>,
) {
    run_bloom(renderer, encoder);
    run_composite(renderer, encoder, output_view, region);
}

/// The bloom ([`PostProcessState::record_bloom`]), spread as [`Renderer::bloom_scatter`](crate::Renderer::bloom_scatter)
/// says. The forward-only path has none.
pub fn run_bloom(renderer: &crate::Renderer, encoder: &mut wgpu::CommandEncoder) {
    renderer.post.record_bloom(encoder, renderer.bloom_scatter);
}

/// The composite: the frame with its bloom, exposed, tone-mapped and finished (motion blur, depth of
/// field, aberration, vignette, grain), into `output_view` — into `region` of it, for a split screen.
pub fn run_composite(
    renderer: &crate::Renderer,
    encoder: &mut wgpu::CommandEncoder,
    output_view: &wgpu::TextureView,
    region: Option<[f32; 4]>,
) {
    // Pass 3: Composite + Tone Mapping
    // Group 4, the G-buffer's velocity, for motion blur: around this frame's target when there is
    // a deferred state, else the 1×1 zero.
    let velocity_group = renderer.post.velocity.as_ref().map(|v| match renderer.deferred.as_ref() {
        Some(def) => velocity_bind_group(&renderer.device, &v.layout, &def.velocity_view),
        None => v.placeholder_bind_group.clone(),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Composite + Tone Mapping Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Into a region of a shared target (split screen), the rest of the target is
                    // another view's and must survive.
                    load: if region.is_some() {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        if let Some([x, y, w, h]) = region {
            pass.set_viewport(x, y, w, h, 0.0, 1.0);
        }
        pass.set_pipeline(&renderer.post.composite_pipeline);
        pass.set_bind_group(0, &renderer.post.hdr_bind_group, &[]);
        pass.set_bind_group(1, &renderer.post.composite_bloom_bind_group, &[]);
        pass.set_bind_group(2, &renderer.post.post_params_bind_group, &[]);
        if let Some(exposure) = &renderer.post.exposure {
            let group = renderer
                .auto_exposure
                .as_ref()
                .filter(|a| a.enabled)
                .map(|a| a.composite_bind_group())
                .unwrap_or(&exposure.neutral_bind_group);
            pass.set_bind_group(3, group, &[]);
        }
        if let Some(group) = velocity_group.as_ref() {
            pass.set_bind_group(4, group, &[]);
        }
        if let Some(local) = &renderer.post.local {
            let group = renderer
                .local_exposure
                .as_ref()
                .filter(|l| l.enabled)
                .and_then(|l| l.composite_bind_group())
                .unwrap_or(&local.neutral_bind_group);
            pass.set_bind_group(5, group, &[]);
        }
        pass.draw(0..3, 0..1);
    }

    tracing::trace!("[PostProcess] recorded the composite");
}

/// The reduced path's composite constants: the sRGB encode when the surface will not do it
/// (`ENCODE_SRGB`), the tone curve and the dither. The desktop composite declares none of them, and wgpu
/// rejects a constant a shader does not declare.
#[cfg(gizmo_forward_only)]
fn composite_constants(surface_format: wgpu::TextureFormat, variant: CompositeVariant) -> Vec<(&'static str, f64)> {
    vec![
        ("ENCODE_SRGB", f64::from(u8::from(composite_encodes_srgb(surface_format)))),
        ("TONEMAP_CURVE", f64::from(variant.tonemap_curve)),
        ("DEBAND_DITHER", f64::from(u8::from(variant.deband_dither))),
        ("VIGNETTE", f64::from(u8::from(variant.vignette))),
        ("FILM_GRAIN", f64::from(u8::from(variant.film_grain))),
        ("COLOR_LUT", f64::from(variant.color_lut)),
    ]
}

/// Whether the reduced path's composite has to encode sRGB for a surface of `format`: when the
/// hardware will not, on write.
#[cfg_attr(not(gizmo_forward_only), allow(dead_code))]
fn composite_encodes_srgb(format: wgpu::TextureFormat) -> bool {
    !format.is_srgb()
}


#[cfg(not(gizmo_forward_only))]
fn composite_constants(_surface_format: wgpu::TextureFormat, _variant: CompositeVariant) -> Vec<(&'static str, f64)> {
    Vec::new()
}


crate::view_targets::screen_targets! {
    PostProcessState => PostTargets {
        key post_bind_group_layout: wgpu::BindGroupLayout;
        hdr_texture: wgpu::Texture,
        hdr_texture_view: wgpu::TextureView,
        hdr_bind_group: wgpu::BindGroup,
        bloom: BloomChain,
        composite_bloom_bind_group: wgpu::BindGroup,
    }
}

#[cfg(test)]
mod srgb_tests {
    /// A browser canvas (`bgra8unorm`, `rgba8unorm`) gets the encode; an sRGB surface (Android's
    /// Vulkan, every desktop) does not, or the image would be encoded twice and washed out.
    #[test]
    fn the_reduced_composite_encodes_only_for_a_surface_that_will_not() {
        use wgpu::TextureFormat::*;
        assert!(super::composite_encodes_srgb(Bgra8Unorm) && super::composite_encodes_srgb(Rgba8Unorm));
        assert!(!super::composite_encodes_srgb(Bgra8UnormSrgb) && !super::composite_encodes_srgb(Rgba8UnormSrgb));
    }

    /// The reduced composite is valid WGSL and declares the constant the pipeline sets — wgpu
    /// rejects a pipeline constant its shader does not declare, and this shader is only ever
    /// compiled by a browser or a phone, never by a desktop test.
    #[test]
    fn the_reduced_composite_declares_its_srgb_constant() {
        let src = super::post_shader_source(
            include_str!("shaders/post_common.wgsl"),
            include_str!("shaders/post_process_wasm.wgsl"),
        );
        let module = naga::front::wgsl::parse_str(&src).expect("post_process_wasm.wgsl parses");
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .expect("post_process_wasm.wgsl validates");
        for name in ["ENCODE_SRGB", "TONEMAP_CURVE", "DEBAND_DITHER", "VIGNETTE", "FILM_GRAIN", "COLOR_LUT"] {
            assert!(
                module.overrides.iter().any(|(_, o)| o.name.as_deref() == Some(name)),
                "the pipeline sets {name}; the shader must declare it"
            );
        }
    }
}

#[cfg(test)]
mod reduced_composite_tests {
    use crate::gpu_types::PostProcessUniforms;

    /// Draws `post_process_wasm.wgsl`'s composite, as the browser and a phone build it, over one row of
    /// HDR values on this machine's GPU, and reads back what it wrote.
    fn run(curve: f32, dither: bool, params: PostProcessUniforms, input: &[f32]) -> Option<Vec<[f32; 3]>> {
        run_with(curve, dither, params, input, None)
    }

    fn run_with(
        curve: f32,
        dither: bool,
        params: PostProcessUniforms,
        input: &[f32],
        lut: Option<&crate::color_lut::ColorLut>,
    ) -> Option<Vec<[f32; 3]>> {
        let (vignette, grain) = (params.vignette_intensity > 0.0, params.film_grain_intensity > 0.0);
        let _gpu = crate::test_gpu::gpu_lock();
        pollster::block_on(async {
            let (device, queue) = crate::test_gpu::headless_device().await?;
            let width = input.len() as u32;
            let format = wgpu::TextureFormat::Rgba16Float;
            let texels: Vec<u16> =
                input.iter().flat_map(|&v| [v, v, v, 1.0]).map(|v| half::f16::from_f32(v).to_bits()).collect();
            let source = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                source.as_image_copy(),
                bytemuck::cast_slice(&texels),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 8), rows_per_image: Some(1) },
                wgpu::Extent3d { width, height: 1, depth_or_array_layers: 1 },
            );
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
            let group0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
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
            });
            let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D3,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                ],
            });
            let lut_view = match lut {
                Some(lut) => lut.upload(&device, &queue),
                None => crate::color_lut::unset_view(&device),
            };
            let linear = device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let group2 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
            let buffer = wgpu::util::DeviceExt::create_buffer_init(
                &device,
                &wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::bytes_of(&params),
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            );
            let view = source.create_view(&Default::default());
            let bind0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &group0,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                ],
            });
            let bind1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &empty,
                entries: &[
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&linear) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&lut_view) },
                ],
            });
            let bind2 = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &group2,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
            });
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(
                    super::post_shader_source(
                        include_str!("shaders/post_common.wgsl"),
                        include_str!("shaders/post_process_wasm.wgsl"),
                    )
                    .into(),
                ),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&group0), Some(&empty), Some(&group2)],
                immediate_size: 0,
            });
            let flag = |on: bool| f64::from(u8::from(on));
            let constants = [
                ("ENCODE_SRGB", 0.0),
                ("TONEMAP_CURVE", f64::from(curve)),
                ("DEBAND_DITHER", flag(dither)),
                ("VIGNETTE", flag(vignette)),
                ("FILM_GRAIN", flag(grain)),
                ("COLOR_LUT", f64::from(params.color_lut[0])),
            ];
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_composite"),
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 256.max(u64::from(width) * 8),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let out_view = target.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &out_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    ..Default::default()
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind0, &[]);
                pass.set_bind_group(1, &bind1, &[]);
                pass.set_bind_group(2, &bind2, &[]);
                pass.draw(0..3, 0..1);
            }
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256.max(width * 8)), rows_per_image: Some(1) },
                },
                wgpu::Extent3d { width, height: 1, depth_or_array_layers: 1 },
            );
            queue.submit(Some(encoder.finish()));
            let slice = staging.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
            let bytes = slice.get_mapped_range().expect("mapped").to_vec();
            let half = |i: usize| half::f16::from_bits(u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]])).to_f32();
            Some((0..width as usize).map(|p| [half(p * 4), half(p * 4 + 1), half(p * 4 + 2)]).collect())
        })
    }

    fn plain(exposure: f32) -> PostProcessUniforms {
        PostProcessUniforms { exposure, vignette_intensity: 0.0, film_grain_intensity: 0.0, ..Default::default() }
    }

    fn aces(x: f32) -> f32 {
        ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
    }

    /// **The browser's and a phone's composite honour the curve, the exposure, the vignette and the
    /// dither.** It applied fixed ACES to the frame as it came and nothing else, so `Renderer::tonemap_curve`,
    /// `Camera::exposure`, `deband_dither` and the vignette did nothing there. Each is checked against the
    /// formula over one row of values, at the precision of a half float.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_reduced_composite_honours_curve_exposure_vignette_and_dither() {
        let input = [0.0, 0.1, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
        let close = |got: &[[f32; 3]], want: &dyn Fn(f32) -> f32, what: &str| {
            for (pixel, &x) in got.iter().zip(&input) {
                let w = want(x);
                assert!((pixel[0] - w).abs() <= 2e-3 + w * 2e-3, "{what}: {x} gave {}, wanted {w}", pixel[0]);
            }
        };
        let Some(reinhard) = run(1.0, false, plain(2.0), &input) else {
            eprintln!("skipping: no usable GPU adapter");
            return;
        };
        close(&reinhard, &|x| 2.0 * x / (1.0 + 2.0 * x), "Reinhard at exposure 2");
        close(&run(3.0, false, plain(1.0), &input).unwrap(), &aces, "ACES, the default");
        close(&run(0.0, false, plain(0.5), &input).unwrap(), &|x| (0.5 * x).clamp(0.0, 1.0), "none: clamped");

        // The vignette: none at 0, as documented; at 1 the edge of the row is darker than its middle.
        let flat = [1.0; 8];
        let none = run(0.0, false, plain(1.0), &flat).unwrap();
        assert!(none.iter().all(|p| (p[0] - 1.0).abs() < 2e-3), "vignette 0 is no vignette: {none:?}");
        let strong = run(0.0, false, PostProcessUniforms { vignette_intensity: 1.0, ..plain(1.0) }, &flat).unwrap();
        assert!(strong[0][0] < strong[3][0] && strong[3][0] <= 1.0, "vignette 1 darkens the edge: {strong:?}");

        // The dither moves a dark row's values by up to half an 8-bit step, and the curve alone does not.
        let dark = [0.02; 8];
        let still = run(0.0, false, plain(1.0), &dark).unwrap();
        let dithered = run(0.0, true, plain(1.0), &dark).unwrap();
        assert!(still.iter().all(|p| (p[0] - 0.02).abs() < 1e-4), "{still:?}");
        assert!(dithered.iter().any(|p| (p[0] - 0.02).abs() > 1e-4), "the dither moved nothing: {dithered:?}");
    }

    /// **The reduced composite reads a colour table as a grade and in the curve's place**, as the desktop's
    /// does: one shared function. An identity grade leaves a row as the curve made it; an identity table fed
    /// `x / (1 + x)` gives Reinhard; fed `log2` over -8..8 stops it gives that ramp, display-decoded; and a
    /// table that halves every channel halves the result.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_reduced_composite_reads_a_colour_table() {
        use crate::color_lut::{ColorLut, LutDomain};
        let input = [0.0, 0.1, 0.25, 0.5, 1.0, 2.0, 4.0];
        let with = |domain: LutDomain| PostProcessUniforms { color_lut: domain.params(), ..plain(1.0) };
        let identity = ColorLut::identity(33);
        let Some(graded) = run_with(1.0, false, with(LutDomain::Grade), &input, Some(&identity)) else {
            eprintln!("skipping: no usable GPU adapter");
            return;
        };
        let check = |got: &[[f32; 3]], want: &dyn Fn(f32) -> f32, what: &str| {
            for (pixel, &x) in got.iter().zip(&input) {
                let w = want(x);
                assert!((pixel[0] - w).abs() <= 4e-3 + w * 4e-3, "{what}: {x} gave {}, wanted {w}", pixel[0]);
            }
        };
        check(&graded, &|x| x / (1.0 + x), "an identity grade after Reinhard");
        let shaped = run_with(3.0, false, with(LutDomain::Reinhard), &input, Some(&identity)).unwrap();
        check(&shaped, &|x| x / (1.0 + x), "an identity table in the curve's place, fed x / (1 + x)");
        let log = LutDomain::Log2 { min_ev: -8.0, max_ev: 8.0 };
        let logged = run_with(3.0, false, with(log), &input, Some(&identity.clone().with_domain(log))).unwrap();
        let decode = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        check(&logged, &|x| decode(((x.max(1e-10).log2() + 8.0) / 16.0).clamp(0.0, 1.0)), "log2 over 16 stops");
        let half_text = {
            let mut t = String::from("LUT_3D_SIZE 2\n");
            for b in [0.0, 0.5] {
                for g in [0.0, 0.5] {
                    for r in [0.0, 0.5] {
                        t.push_str(&format!("{r} {g} {b}\n"));
                    }
                }
            }
            t
        };
        let halving = ColorLut::from_cube(&half_text).unwrap().with_domain(LutDomain::Reinhard);
        let halved = run_with(3.0, false, with(LutDomain::Reinhard), &input, Some(&halving)).unwrap();
        check(&halved, &|x| 0.5 * x / (1.0 + x), "a table that halves");
    }
}

#[cfg(all(test, not(gizmo_forward_only)))]
mod bloom_tests {
    use crate::gpu_types::PostProcessUniforms;

    fn f32_to_f16(v: f32) -> u16 {
        let bits = v.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
        let mantissa = bits & 0x7f_ffff;
        if v == 0.0 {
            sign
        } else if exp >= 31 {
            sign | 0x7c00
        } else if exp <= 0 {
            sign
        } else {
            sign | ((exp as u16) << 10) | ((mantissa >> 13) as u16)
        }
    }

    fn f16_to_f32(h: u16) -> f32 {
        let sign = if h >> 15 == 1 { -1.0 } else { 1.0 };
        let exp = i32::from((h >> 10) & 0x1f);
        let frac = f32::from(h & 0x3ff);
        sign * match exp {
            0 => frac * 2f32.powi(-24),
            31 => f32::INFINITY,
            _ => (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
        }
    }

    /// The bloom's first level (the red channel of each texel, and the level's width) after the chain ran at
    /// `scatter` over a `side`² frame that is black but for a `patch`² square of 10 at its middle.
    fn bloom(side: u32, patch: u32, scatter: f32) -> Option<(Vec<f32>, u32)> {
        let _gpu = crate::test_gpu::gpu_lock();
        pollster::block_on(async {
            let (device, queue) = crate::test_gpu::headless_native_device().await?;
            let depth = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bloom_test_depth"),
                size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
            let mut post = super::build_post_process_resources(&device, wgpu::TextureFormat::Rgba8UnormSrgb, side, side, &depth_view);
            // The frame, as the HDR target would hold it, bound where the chain reads the HDR target.
            let (lo, hi) = ((side - patch) / 2, (side + patch) / 2);
            let bytes: Vec<u8> = (0..side * side)
                .flat_map(|i| {
                    let (x, y) = (i % side, i / side);
                    let v = if (lo..hi).contains(&x) && (lo..hi).contains(&y) { 10.0 } else { 0.0 };
                    [v, v, v, 1.0].map(f32_to_f16).into_iter().flat_map(u16::to_le_bytes)
                })
                .collect();
            let frame = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bloom_test_frame"),
                size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &frame, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(side * 8), rows_per_image: Some(side) },
                wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
            );
            let view = frame.create_view(&wgpu::TextureViewDescriptor::default());
            let sampler = super::create_post_sampler(&device);
            post.hdr_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom_test_frame_group"),
                layout: &post.post_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                ],
            });
            // The defaults' threshold (0.85) and softness: all of a 10 blooms, none of a 0.
            queue.write_buffer(&post.post_params_buffer, 0, bytemuck::cast_slice(&[PostProcessUniforms::default()]));
            let first = side / 2;
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bloom_test_readback"),
                size: u64::from(first * first * 8),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("bloom_test") });
            post.record_bloom(&mut encoder, scatter);
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: post.bloom.texture(), mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(first * 8), rows_per_image: Some(first) },
                },
                wgpu::Extent3d { width: first, height: first, depth_or_array_layers: 1 },
            );
            queue.submit([encoder.finish()]);
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
            let data = slice.get_mapped_range().ok()?;
            let red = data.as_chunks::<8>().0.iter().map(|t| f16_to_f32(u16::from_le_bytes([t[0], t[1]]))).collect();
            Some((red, first))
        })
    }

    /// **The bloom keeps the light it spreads, at any scatter, and spreads it further the higher the
    /// scatter.** A 16-pixel square of 10 on a black 256² frame: its bright part is all of it, 2,560. The
    /// first level is half the frame's size, so each texel stands for four pixels; its light, four times
    /// its sum, must be that 2,560 whatever the scatter — the levels' shares add to one going up the
    /// chain — and the share of it lying more than 32 pixels from the square's middle must grow with the
    /// scatter.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_bloom_keeps_the_light_it_spreads_and_spreads_it_further_with_the_scatter() {
        let (side, patch) = (256, 16);
        let mut far_shares = Vec::new();
        for scatter in [0.0, 0.7, 0.95] {
            let Some((red, first)) = bloom(side, patch, scatter) else {
                eprintln!("skipping: no usable GPU adapter");
                return;
            };
            let light: f32 = red.iter().sum::<f32>() * 4.0;
            let middle = first as f32 / 2.0;
            let far: f32 = red
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    let (x, y) = ((i % first as usize) as f32 + 0.5 - middle, (i / first as usize) as f32 + 0.5 - middle);
                    (x * x + y * y).sqrt() > 16.0
                })
                .map(|(_, v)| v)
                .sum::<f32>()
                * 4.0;
            eprintln!("BLOOM scatter {scatter}: light {light:.1}, beyond 32 px {:.3}", far / light);
            let expected = (patch * patch) as f32 * 10.0;
            assert!((light - expected).abs() < 0.02 * expected, "at a scatter of {scatter} the bloom holds {light:.1} of the square's {expected}");
            far_shares.push(far / light);
        }
        assert!(
            far_shares[0] < far_shares[1] && far_shares[1] < far_shares[2],
            "the share past 32 px grows with the scatter (0, 0.7, 0.95): {far_shares:?}"
        );
    }

    /// The chain for 1080p: seven levels, from half the frame down to eight texels on the short side.
    #[test]
    fn a_1080p_frame_blooms_through_seven_levels() {
        assert_eq!(super::bloom_levels(1920, 1080), ((960, 540), 7));
        assert_eq!(super::bloom_levels(1, 1), ((1, 1), 1));
        assert_eq!(super::bloom_levels(16, 16).1, 1, "a level of eight is where it stops");
        assert_eq!(super::bloom_levels(32, 32).1, 2);
    }
}
