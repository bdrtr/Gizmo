use crate::gpu_types::PostProcessUniforms;
use wgpu::util::DeviceExt;

/// The post-processing state — the HDR, bloom, blur and composite pipelines and their
/// resources.
pub struct PostProcessState {
    /// Layout of the "one HDR texture + sampler" group every pass in the chain binds.
    pub post_bind_group_layout: wgpu::BindGroupLayout,
    /// Layout of the blur pass's direction group.
    pub blur_params_bind_group_layout: wgpu::BindGroupLayout,
    /// Layout of the composite pass's group: the blurred bloom, and the depth for DoF.
    pub composite_bloom_bind_group_layout: wgpu::BindGroupLayout,
    /// The [`PostProcessUniforms`] buffer, rewritten each frame.
    pub post_params_buffer: wgpu::Buffer,
    /// Its layout.
    pub post_params_bind_group_layout: wgpu::BindGroupLayout,
    /// Its bind group. Never rebuilt — the buffer outlives every resize.
    pub post_params_bind_group: wgpu::BindGroup,
    /// The bloom extract pass: everything above the threshold, at half resolution.
    pub bloom_extract_pipeline: wgpu::RenderPipeline,
    /// The separable blur pass, run twice — horizontally, then vertically.
    pub bloom_blur_pipeline: wgpu::RenderPipeline,
    /// The composite pass: tone map, add bloom, and apply DoF, grain, vignette and aberration.
    pub composite_pipeline: wgpu::RenderPipeline,
    /// The module the three pipelines were built from, kept to build them again for another
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
    /// The extract pass's output.
    pub bloom_extract_texture_view: wgpu::TextureView,
    /// It as an input to the blur.
    pub bloom_extract_bind_group: wgpu::BindGroup,
    /// The blur's output — the ping half of the two-pass blur.
    pub bloom_blur_texture_view: wgpu::TextureView,
    /// It as an input.
    pub bloom_blur_bind_group: wgpu::BindGroup,
    /// The blurred bloom and the depth buffer, as the composite pass reads them.
    pub composite_bloom_bind_group: wgpu::BindGroup,
    /// The horizontal blur's step (this said "both steps"; it only ever held the horizontal one).
    /// Rewritten every frame from `Renderer::bloom_radius`.
    pub blur_params_buffer: wgpu::Buffer,
    /// The vertical blur's step, likewise.
    pub blur_v_params_buffer: wgpu::Buffer,
    /// The horizontal blur's parameters — a texel step along X.
    pub blur_h_bind_group: wgpu::BindGroup,
    /// The vertical blur's — a texel step along Y. One separable blur run twice costs `2n`
    /// samples where a true 2-D kernel costs `n²`.
    pub blur_v_bind_group: wgpu::BindGroup,
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

    let blur_params_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur_params_bind_group_layout"),
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

    let composite_variant = CompositeVariant::default();
    let (bloom_extract_pipeline, bloom_blur_pipeline, composite_pipeline) = build_post_pipelines(
        device,
        &post_shader,
        &post_bind_group_layout,
        &blur_params_bind_group_layout,
        &composite_bloom_bind_group_layout,
        &post_params_bind_group_layout,
        exposure.as_ref().map(|e| &e.layout),
        velocity.as_ref().map(|v| &v.layout),
        surface_format,
        composite_variant,
    );

    let post_sampler = create_post_sampler(device);
    let lut_view = crate::color_lut::unset_view(device);
    let (
        hdr_texture,
        hdr_texture_view,
        hdr_bind_group,
        bloom_extract_texture_view,
        bloom_extract_bind_group,
        bloom_blur_texture_view,
        bloom_blur_bind_group,
        composite_bloom_bind_group,
    ) = create_post_textures(
        device,
        &post_bind_group_layout,
        &composite_bloom_bind_group_layout,
        &post_sampler,
        width,
        height,
        depth_view,
        &lut_view,
    );

    let (blur_params_buffer, blur_v_params_buffer, blur_h_bind_group, blur_v_bind_group) =
        create_blur_buffers(device, &blur_params_bind_group_layout, width, height);

    PostProcessState {
        post_bind_group_layout,
        blur_params_bind_group_layout,
        composite_bloom_bind_group_layout,
        post_params_buffer,
        post_params_bind_group_layout,
        post_params_bind_group,
        bloom_extract_pipeline,
        bloom_blur_pipeline,
        composite_pipeline,
        shader: post_shader,
        composite_variant,
        lut_view,
        hdr_texture,
        hdr_texture_view,
        hdr_bind_group,
        bloom_extract_texture_view,
        bloom_extract_bind_group,
        bloom_blur_texture_view,
        bloom_blur_bind_group,
        composite_bloom_bind_group,
        blur_params_buffer,
        blur_v_params_buffer,
        blur_h_bind_group,
        blur_v_bind_group,
        exposure,
        velocity,
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
    blur_bgl: &wgpu::BindGroupLayout,
    composite_bloom_bgl: &wgpu::BindGroupLayout,
    post_params_bgl: &wgpu::BindGroupLayout,
    exposure_bgl: Option<&wgpu::BindGroupLayout>,
    velocity_bgl: Option<&wgpu::BindGroupLayout>,
    surface_format: wgpu::TextureFormat,
    variant: CompositeVariant,
) -> (
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
) {
    let extract_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Bloom Extract Pipeline Layout"),
        bind_group_layouts: &[Some(post_bgl), Some(blur_bgl), Some(post_params_bgl)],
        immediate_size: 0,
    });
    let bloom_extract_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Bloom Extract Pipeline"),
        layout: Some(&extract_layout),
        vertex: wgpu::VertexState {
            module: post_shader,
            entry_point: Some("vs_fullscreen"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: post_shader,
            entry_point: Some("fs_bright_extract"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
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

    let blur_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Bloom Blur Pipeline Layout"),
        bind_group_layouts: &[Some(post_bgl), Some(blur_bgl)],
        immediate_size: 0,
    });
    let bloom_blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Bloom Blur Pipeline"),
        layout: Some(&blur_layout),
        vertex: wgpu::VertexState {
            module: post_shader,
            entry_point: Some("vs_fullscreen"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: post_shader,
            entry_point: Some("fs_blur"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
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

    let composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Composite Pipeline Layout"),
        // Group 3 only where the composite reads an exposure buffer (native).
        bind_group_layouts: &[
            Some(post_bgl),
            Some(composite_bloom_bgl),
            Some(post_params_bgl),
            exposure_bgl,
            velocity_bgl,
        ][..match (exposure_bgl, velocity_bgl) {
            (Some(_), Some(_)) => 5,
            (Some(_), None) => 4,
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

    (
        bloom_extract_pipeline,
        bloom_blur_pipeline,
        composite_pipeline,
    )
}

/// Recompiles just the three post-process pipelines against a new shader module, leaving every
/// texture and bind group in place — what shader hot-reload calls.
pub fn rebuild_post_pipelines(renderer: &mut crate::Renderer, post_shader: &wgpu::ShaderModule) {
    let variant = renderer.post.composite_variant;
    let (e, b, c) = build_post_pipelines(
        &renderer.device,
        post_shader,
        &renderer.post.post_bind_group_layout,
        &renderer.post.blur_params_bind_group_layout,
        &renderer.post.composite_bloom_bind_group_layout,
        &renderer.post.post_params_bind_group_layout,
        renderer.post.exposure.as_ref().map(|e| &e.layout),
        renderer.post.velocity.as_ref().map(|v| &v.layout),
        renderer.config.format,
        variant,
    );
    renderer.post.bloom_extract_pipeline = e;
    renderer.post.bloom_blur_pipeline = b;
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
/// (Re)creates every size-dependent target in the chain and the bind groups that read them,
/// returning them in the order the caller assigns them: HDR texture, view and group; the extract
/// view and group; the blur view and group; and the composite group.
pub fn create_post_textures(
    device: &wgpu::Device,
    post_bgl: &wgpu::BindGroupLayout,
    composite_bloom_bgl: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
    depth_view: &wgpu::TextureView,
    lut_view: &wgpu::TextureView,
) -> (
    wgpu::Texture,
    wgpu::TextureView,
    wgpu::BindGroup,
    wgpu::TextureView,
    wgpu::BindGroup,
    wgpu::TextureView,
    wgpu::BindGroup,
    wgpu::BindGroup,
) {
    let make = |label: &str| -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: post_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
            label: Some(&format!("{}_bind_group", label)),
        });
        (tex, view, bg)
    };

    let (hdr_t, hdr_v, hdr_bg) = make("HDR Texture");
    let (_be_t, be_v, be_bg) = make("Bloom Extract Texture");
    let (_bb_t, bb_v, bb_bg) = make("Bloom Blur Texture");

    let cb_bg = composite_bloom_group(device, composite_bloom_bgl, &be_v, sampler, depth_view, lut_view);

    (hdr_t, hdr_v, hdr_bg, be_v, be_bg, bb_v, bb_bg, cb_bg)
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

/// Builds the blur direction buffer and its two bind groups — one stepping a texel along X, one
/// along Y. Size-dependent, because a texel step is `1 / width`.
pub fn create_blur_buffers(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    width: u32,
    height: u32,
) -> (wgpu::Buffer, wgpu::Buffer, wgpu::BindGroup, wgpu::BindGroup) {
    let [h_data, v_data] = blur_steps(width, height, 1.0);

    let h_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Blur H Params"),
        contents: bytemuck::cast_slice(&h_data),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let v_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Blur V Params"),
        contents: bytemuck::cast_slice(&v_data),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let h_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: h_buf.as_entire_binding(),
        }],
        label: Some("blur_h_bind_group"),
    });
    let v_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: v_buf.as_entire_binding(),
        }],
        label: Some("blur_v_bind_group"),
    });
    (h_buf, v_buf, h_bg, v_bg)
}

/// The bloom blur's horizontal and vertical steps, in UV, for a `width × height` frame and a
/// [`Renderer::bloom_radius`](crate::Renderer::bloom_radius) of `radius` (1 = one texel of the
/// full frame between taps, the blur this engine always had).
pub fn blur_steps(width: u32, height: u32, radius: f32) -> [[f32; 4]; 2] {
    let r = if radius.is_finite() { radius.clamp(0.0, 16.0) } else { 1.0 };
    [[r / width.max(1) as f32, 0.0, 0.0, 0.0], [0.0, r / height.max(1) as f32, 0.0, 0.0]]
}

/// Runs the post-processing render passes in order.
#[tracing::instrument(skip_all, level = "trace")]
pub fn run_post_processing(
    renderer: &crate::Renderer,
    encoder: &mut wgpu::CommandEncoder,
    output_view: &wgpu::TextureView,
    region: Option<[f32; 4]>,
) {
    // Pass 1: Bright Extract (WASM'da KAPALI)
    #[cfg(not(gizmo_forward_only))]
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Bloom Extract Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.bloom_extract_texture_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        pass.set_pipeline(&renderer.post.bloom_extract_pipeline);
        pass.set_bind_group(0, &renderer.post.hdr_bind_group, &[]);
        pass.set_bind_group(1, &renderer.post.blur_h_bind_group, &[]);
        pass.set_bind_group(2, &renderer.post.post_params_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    // Pass 2: the separable blur, horizontal then vertical, `bloom_iterations` times (WASM'da
    // KAPALI). Each round widens the halo by about the kernel's reach again, so three rounds reach
    // roughly √3 as far as one; `bloom_radius` spreads the taps themselves.
    #[cfg(not(gizmo_forward_only))]
    {
        let [h, v] = blur_steps(renderer.config.width, renderer.config.height, renderer.bloom_radius);
        renderer.queue.write_buffer(&renderer.post.blur_params_buffer, 0, bytemuck::cast_slice(&h));
        renderer.queue.write_buffer(&renderer.post.blur_v_params_buffer, 0, bytemuck::cast_slice(&v));
    }
    #[cfg(not(gizmo_forward_only))]
    for _ in 0..renderer.bloom_iterations.clamp(1, 8) {
        // Pass 2a: Yatay Blur (WASM'da KAPALI)
        #[cfg(not(gizmo_forward_only))]
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Blur Horizontal"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.bloom_blur_texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&renderer.post.bloom_blur_pipeline);
            pass.set_bind_group(0, &renderer.post.bloom_extract_bind_group, &[]);
            pass.set_bind_group(1, &renderer.post.blur_h_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        // Pass 2b: Dikey Blur (ping-pong) (WASM'da KAPALI)
        #[cfg(not(gizmo_forward_only))]
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Blur Vertical"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.bloom_extract_texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&renderer.post.bloom_blur_pipeline);
            pass.set_bind_group(0, &renderer.post.bloom_blur_bind_group, &[]);
            pass.set_bind_group(1, &renderer.post.blur_v_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

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
        pass.draw(0..3, 0..1);
    }

    // Native records bloom extract + H/V blur + composite; WASM strips bloom and
    // records only the composite/tonemap pass.
    #[cfg(not(gizmo_forward_only))]
    let passes = 4u32;
    #[cfg(gizmo_forward_only)]
    let passes = 1u32;
    tracing::trace!(passes, "[PostProcess] recorded post-processing chain");
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
