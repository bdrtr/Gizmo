//! Creating the render pipelines (core + shadow).

use super::layouts::LayoutRefs;

/// The fixed-function state of one baked-lit pipeline variant.
///
/// A built `wgpu::RenderPipeline` is opaque — nothing can ask it afterwards whether it blends
/// — so the choice is made here, in a pure function, where a test can read it. That matters
/// for this pipeline in particular: the transparent variant exists precisely because a
/// `blend: None` state had been silently discarding the fragment alpha the shader computed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct BakedLitState {
    pub(super) blend: Option<wgpu::BlendState>,
    pub(super) depth_write: bool,
    pub(super) cull: Option<wgpu::Face>,
}

/// Which state a baked-lit draw gets, by `Material::is_transparent`.
///
/// Opaque: a solid world. Backface culling and no blend are free wins, and depth-write lets
/// the depth buffer resolve draw order.
///
/// Transparent: a decal, a skid mark, a pane of glass. Alpha blending (otherwise the layer
/// paints over what is under it), no depth write (the batcher already sorts these
/// back-to-front and only draw order can composite them correctly), and no culling, because
/// such a layer is usually a single plane whose back face is as valid a view as its front.
pub(super) fn baked_lit_state(is_transparent: bool) -> BakedLitState {
    if is_transparent {
        BakedLitState {
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            depth_write: false,
            cull: None,
        }
    } else {
        BakedLitState {
            blend: None,
            depth_write: true,
            cull: Some(wgpu::Face::Back),
        }
    }
}

/// The forward pipelines [`SceneState::core`](super::SceneState::core) holds: every one
/// `build_core_pipelines` makes.
pub struct CorePipelines {
    /// The main opaque PBR pipeline, back faces culled.
    pub render: wgpu::RenderPipeline,
    /// The same, with culling off, for materials marked double-sided.
    pub render_double_sided: wgpu::RenderPipeline,
    /// Line-topology, for wireframe views.
    pub wireframe: wgpu::RenderPipeline,
    /// Albedo straight through, no lighting — see [`MaterialType::Unlit`](crate::components::MaterialType::Unlit).
    /// Opaque: it does not blend (`pipelines::OPAQUE`).
    pub unlit: wgpu::RenderPipeline,
    /// The same for a transparent unlit draw blending by alpha; the other blend modes have
    /// [`blend_unlit`](Self::blend_unlit).
    pub unlit_blended: wgpu::RenderPipeline,
    /// Baked lighting × texture, with the sun's cascade term. See `shaders/baked_lit.wgsl`.
    pub baked_lit: wgpu::RenderPipeline,
    /// The same, culling nothing, for a double-sided baked-lit material (`Material::double_sided`). The
    /// forward path drew every opaque baked-lit surface with the culling pipeline until 2026-11-19, so a
    /// double-sided one lost its back faces there — and kept them through the G-buffer
    /// (`DeferredState::gbuffer_baked_pipelines`), which had both.
    pub baked_lit_double_sided: wgpu::RenderPipeline,
    /// The same shader with alpha blending and no depth write, for a `BakedLit` material
    /// marked transparent — a decal or skid-mark layer, which the opaque variant would paint
    /// over the road as solid geometry no matter what alpha the shader produced.
    pub baked_lit_transparent: wgpu::RenderPipeline,
    /// A solid baked-lit surface's share of the z-prepass, culling back faces and nothing: its depth alone,
    /// from the same vertex stage as its forward draw (`baked_lit.wgsl`'s position is `@invariant`), so
    /// that the forward pass shades only what is in front of it.
    pub baked_lit_depth: [wgpu::RenderPipeline; 2],
    /// The generated atmospheric sky — a gradient from the sun's direction and colour, ignoring
    /// the mesh. Contrast [`backdrop`](Self::backdrop).
    pub sky: wgpu::RenderPipeline,
    /// The mesh's own painted sky/panorama — see `crate::backdrop`.
    pub backdrop: wgpu::RenderPipeline,
    /// The animated water surface: `shaders/water.wgsl`, Gerstner displacement in the vertex
    /// stage and a Fresnel sky term in the fragment stage. Forward-only — the G-buffer is filled
    /// by a shader that does not displace, so a water surface routed through it would be shaded at
    /// the flat plane's depth. Selected by [`Routing::is_water`](crate::routing::Routing::is_water).
    pub water: wgpu::RenderPipeline,
    /// The same, culling off — an ocean surface is looked at from underneath.
    pub water_double_sided: wgpu::RenderPipeline,
    /// What a [`PlanarReflection`](crate::components::PlanarReflection) draws with once captured.
    pub mirror: wgpu::RenderPipeline,
    /// Water with a [`PlanarReflection`](crate::components::PlanarReflection), once captured.
    pub water_mirror: wgpu::RenderPipeline,
    /// Alpha-blended, no depth write, for the transparent bucket.
    pub transparent: wgpu::RenderPipeline,
    /// The editor's ground grid.
    pub grid: wgpu::RenderPipeline,
    /// The lit transparent pipeline under each of `BlendMode::VARIANTS`.
    pub blend_lit: [wgpu::RenderPipeline; 3],
    /// The lit pipelines with anisotropy, clear coat and subsurface, on the path where the ones above
    /// leave them out. See [`LitSet`](super::LitSet).
    pub lobes: Option<super::LitSet>,
    /// [`render`](Self::render) with `DETAILS`, on the forward-only path, whose
    /// plain one leaves them out: a single-sided opaque draw that needs a cut-out, a LOD crossfade or
    /// its own reflectance. `None` on the desktop path, whose lit pipelines always have them.
    pub render_details: Option<wgpu::RenderPipeline>,
    /// [`unlit`](Self::unlit) with `KEEPS`, on the forward-only path, whose plain one
    /// cannot discard: a cut-out, a LOD crossfade. `None` on the desktop path, whose unlit pipeline can.
    pub unlit_keeps: Option<wgpu::RenderPipeline>,
    /// The lit shader with `TRANSMISSION` on: single-sided, then double-sided.
    pub transmissive: [wgpu::RenderPipeline; 2],
    /// The unlit pipeline under each of them — and, unlike `unlit`, not writing depth: a blended
    /// draw is sorted, and a glow writing depth would hide the glow behind it.
    pub blend_unlit: [wgpu::RenderPipeline; 3],
}

/// One forward pipeline over the engine's vertex layout, drawing into the HDR target against the shared
/// depth buffer: every pipeline [`build_core_pipelines`] makes, and a game's surface's
/// ([`SurfaceMaterial`](crate::custom_material::SurfaceMaterial)), so the two cannot disagree about
/// any of it.
pub(crate) fn forward_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entry: &str,
    label: &str,
    depth_write: bool,
    cull: Option<wgpu::Face>,
    blend: Option<wgpu::BlendState>,
    polygon_mode: wgpu::PolygonMode,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(crate::gpu_types::GpuVertex::desc())],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: cull,
            unclipped_depth: false,
            polygon_mode,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview_mask: None,
        cache: None,
    })
}

/// `module`'s vertex stage alone, writing depth: an opaque surface's share of the z-prepass, drawn with the
/// same vertex stage as its forward draw so that the depths meet exactly (its position is `@invariant`).
pub(crate) fn depth_only_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    label: &str,
    cull: Option<wgpu::Face>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(crate::gpu_types::GpuVertex::desc())],
        },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: cull,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// The layout of a pipeline drawn from `shader.wgsl`'s lit entry points. On the desktop path it is the
/// five groups every forward pipeline has plus the irradiance volume's at 5, for the reflection probes
/// (`#ifdef PROBES`); the forward-only path has four groups and puts the probes on group 0.
pub(super) fn lit_pipeline_layout(device: &wgpu::Device, layouts: &LayoutRefs) -> wgpu::PipelineLayout {
    #[cfg(not(gizmo_forward_only))]
    let groups = [
        Some(layouts.global),
        Some(layouts.texture),
        Some(layouts.shadow),
        Some(layouts.skeleton),
        Some(layouts.instance),
        Some(layouts.indirect), // 5
    ];
    #[cfg(gizmo_forward_only)]
    let groups = [Some(layouts.global), Some(layouts.texture), Some(layouts.skeleton), Some(layouts.instance)];
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Lit Forward Pipeline Layout"),
        bind_group_layouts: &groups,
        immediate_size: 0,
    })
}

/// Every lit pipeline a draw on one tier can need, from that tier's `module`: opaque single- and
/// double-sided, blended by alpha, and blended under each of the other modes. The opaque ones do not
/// blend: see [`OPAQUE`].
pub(crate) fn lit_set(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    name: &str,
) -> super::LitSet {
    let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
    let fill = wgpu::PolygonMode::Fill;
    super::LitSet {
        render: forward_pipeline(device, layout, module, "fs_main", &format!("Render Pipeline ({name})"), true, Some(wgpu::Face::Back), OPAQUE, fill),
        render_double_sided: forward_pipeline(device, layout, module, "fs_main", &format!("Render TwoSided Pipeline ({name})"), true, None, OPAQUE, fill),
        transparent: forward_pipeline(device, layout, module, "fs_main", &format!("Transparent Pipeline ({name})"), false, None, alpha, fill),
        blend: crate::components::BlendMode::VARIANTS.map(|mode| {
            forward_pipeline(device, layout, module, lit_fragment_entry(mode), &format!("Transparent Pipeline ({mode:?}, {name})"), false, None, Some(mode.state()), fill)
        }),
    }
}

/// The blend of every pipeline an opaque draw takes: none. Opaque means opaque, as it does in the
/// G-buffer, which writes a surface's texels solid whatever their alpha. These pipelines write depth
/// and draw in no particular order, so blending a texel by its alpha shows whatever happened to be
/// drawn behind it first: until 2026-11-11 a cut-out's kept texels under 1 showed the sky, or the other
/// of two batches a hash map gave first, in the editor viewport, on the forward-only path's `DETAILS`
/// tier and wherever an unlit or a game's surface was drawn forward — 1 577 pixels changing between
/// identical frames in the editor. A surface that shows through says so with
/// `Material::is_transparent` and is drawn, sorted, by the blended pipelines.
pub(crate) const OPAQUE: Option<wgpu::BlendState> = None;

/// `probes`: which reflection probes the forward-only path's lit shaders read, which is what a scene has
/// ([`ProbeShape`](super::ProbeShape), `SceneState::set_forward_reflections`). The desktop's read any,
/// deciding per pixel. `sun_shadows`, `local_light_maps` and `lights` likewise: see `SceneDefs`.
///
/// Three families, each with its own shaders: the lit surfaces (`lit_pipelines`), the unlit and
/// baked-lit ones (`unlit_pipelines`), and the scene's own surfaces — sky, backdrop, water,
/// mirrors, grid (`scene_pipelines`).
pub(super) fn build_core_pipelines(
    device: &wgpu::Device,
    layouts: &LayoutRefs,
    probes: super::ProbeShape,
    sun_shadows: bool,
    local_light_maps: bool,
    lights: super::LightShape,
    decals: bool,
) -> CorePipelines {
    let layouts = ForwardLayouts::new(device, layouts);
    let scene = SceneDefs::new(probes, sun_shadows, local_light_maps, lights, decals);
    let LitPipelines { render, render_double_sided, wireframe, transparent, blend_lit, lobes, render_details, transmissive } =
        lit_pipelines(device, &layouts, &scene);
    let UnlitPipelines {
        unlit,
        unlit_blended,
        unlit_keeps,
        blend_unlit,
        baked_lit,
        baked_lit_double_sided,
        baked_lit_transparent,
        baked_lit_depth,
    } = unlit_pipelines(device, &layouts.plain, &scene);
    let ScenePipelines { sky, backdrop, water, water_double_sided, water_mirror, mirror, grid } =
        scene_pipelines(device, &layouts.plain);
    CorePipelines {
        render,
        render_double_sided,
        wireframe,
        unlit,
        unlit_blended,
        baked_lit,
        baked_lit_double_sided,
        baked_lit_transparent,
        baked_lit_depth,
        sky,
        backdrop,
        water,
        water_double_sided,
        mirror,
        water_mirror,
        transparent,
        grid,
        blend_lit,
        lobes,
        render_details,
        unlit_keeps,
        transmissive,
        blend_unlit,
    }
}

/// The forward pipelines' layouts.
struct ForwardLayouts {
    /// Every forward pipeline's groups: global, texture, shadow, skeleton and instance on the
    /// desktop; the same without the shadow group on the forward-only path — WebGPU allows four.
    plain: wgpu::PipelineLayout,
    /// The lit shader's: on the desktop `plain` and the irradiance volume's group at 5, for its
    /// reflection probes (`shader.wgsl`'s `#ifdef PROBES`), which a forward pass binds once — a bound
    /// group stays bound across pipeline switches. `plain` on the forward-only path, whose probes are
    /// on group 0.
    lit: wgpu::PipelineLayout,
    /// The wireframe's: on the desktop `plain` and its line colour at 5; `plain` on the
    /// forward-only path, whose wireframe is the lit shader's fill.
    wire: wgpu::PipelineLayout,
}

impl ForwardLayouts {
    #[cfg(not(gizmo_forward_only))]
    fn new(device: &wgpu::Device, layouts: &LayoutRefs) -> Self {
        let with = |label: &str, sixth: Option<&wgpu::BindGroupLayout>| {
            let groups = [
                Some(layouts.global),
                Some(layouts.texture),
                Some(layouts.shadow),
                Some(layouts.skeleton),
                Some(layouts.instance),
                sixth,
            ];
            let groups = if sixth.is_some() { &groups[..] } else { &groups[..5] };
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                immediate_size: 0,
            })
        };
        Self {
            plain: with("Render Pipeline Layout", None),
            lit: lit_pipeline_layout(device, layouts),
            wire: with("Wireframe Pipeline Layout", Some(layouts.wire_color)),
        }
    }

    #[cfg(gizmo_forward_only)]
    fn new(device: &wgpu::Device, layouts: &LayoutRefs) -> Self {
        let plain = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[
                Some(layouts.global),   // 0
                Some(layouts.texture),  // 1
                Some(layouts.skeleton), // 2
                Some(layouts.instance), // 3
            ],
            immediate_size: 0,
        });
        Self { lit: plain.clone(), wire: plain.clone(), plain }
    }
}

/// What the forward-only path builds its lit shaders for: the scene's reflection probes
/// (`SceneState::set_forward_reflections`), whether they read the sun's shadow
/// (`forward_sun_shadows`) and the local lights' shadows and cookies (`forward_local_light_maps`),
/// the kinds of local light it has (`forward_lights`), and whether they project decals
/// (`forward_decals_drawn`). A branch a phone does not take still
/// costs it. The desktop's shaders are built with all of it, deciding per pixel, and take none.
#[derive(Default)]
struct SceneDefs {
    probes: Vec<(&'static str, naga_oil::compose::ShaderDefValue)>,
    lights: Vec<(&'static str, naga_oil::compose::ShaderDefValue)>,
}

impl SceneDefs {
    fn new(probes: super::ProbeShape, sun_shadows: bool, local_light_maps: bool, lights: super::LightShape, decals: bool) -> Self {
        if !cfg!(gizmo_forward_only) {
            return Self::default();
        }
        let on = naga_oil::compose::ShaderDefValue::Bool;
        let [local, sized, rect] = lights.defs();
        Self {
            probes: probes.defs().to_vec(),
            lights: vec![("SUN_SHADOWS", on(sun_shadows)), ("LOCAL_LIGHT_MAPS", on(local_light_maps)), local, sized, rect, ("DECALS", on(decals))],
        }
    }

    /// A lit shader's: the probes and the lights, then `extra`.
    fn lit(&self, extra: &[&'static str]) -> Vec<(&'static str, naga_oil::compose::ShaderDefValue)> {
        let extra = extra.iter().map(|name| (*name, naga_oil::compose::ShaderDefValue::Bool(true)));
        self.probes.iter().chain(&self.lights).copied().chain(extra).collect()
    }
}

/// One of the forward pipelines' shaders, for the path this build draws with: on the desktop
/// composed under its defs, a copy at `demo/assets/shaders/<file>` taking the compiled-in one's
/// place (hot reload); on the forward-only path under that path's, from the compiled-in copy.
/// `defs` are set over the path's own.
#[cfg(not(gizmo_forward_only))]
fn path_shader(
    device: &wgpu::Device,
    file: &str,
    source: &str,
    label: &str,
    defs: &[(&str, naga_oil::compose::ShaderDefValue)],
) -> wgpu::ShaderModule {
    super::shaders::load_shader_composed_with(device, &format!("demo/assets/shaders/{file}"), source, label, defs)
}

#[cfg(gizmo_forward_only)]
fn path_shader(
    device: &wgpu::Device,
    _file: &str,
    source: &str,
    label: &str,
    defs: &[(&str, naga_oil::compose::ShaderDefValue)],
) -> wgpu::ShaderModule {
    super::shaders::load_shader_composed_web_with(device, source, label, defs)
}

/// A filled pipeline drawing `module`'s `fs_main`: what all but two of the forward pipelines are.
fn filled(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    label: &str,
    depth_write: bool,
    cull: Option<wgpu::Face>,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    forward_pipeline(device, layout, module, "fs_main", label, depth_write, cull, blend, wgpu::PolygonMode::Fill)
}

const LIT_WGSL: &str = include_str!("../shaders/shader.wgsl");
const UNLIT_WGSL: &str = include_str!("../shaders/unlit.wgsl");

/// The pipelines drawn from `shader.wgsl`.
struct LitPipelines {
    render: wgpu::RenderPipeline,
    render_double_sided: wgpu::RenderPipeline,
    wireframe: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    blend_lit: [wgpu::RenderPipeline; 3],
    lobes: Option<super::LitSet>,
    render_details: Option<wgpu::RenderPipeline>,
    transmissive: [wgpu::RenderPipeline; 2],
}

/// The lit surfaces: opaque, double-sided, blended under each mode, glass, and the wireframe. The
/// desktop has one lit shader with everything in. The forward-only path builds them in tiers, so a
/// plain surface does not pay for what another needs: the plain shader, `DETAILS` for a
/// double-sided, blended or cut-out draw (`render_details`), and `LOBES` for a material's
/// anisotropy, clear coat and subsurface (`lobes`). See `SceneState::lit_pipeline`.
fn lit_pipelines(device: &wgpu::Device, layouts: &ForwardLayouts, scene: &SceneDefs) -> LitPipelines {
    let shader = path_shader(device, "shader.wgsl", LIT_WGSL, "Shader", &scene.lit(&[]));
    // The middle tier on the forward-only path. On the desktop it is the one shader there is.
    #[cfg(gizmo_forward_only)]
    let details_module = path_shader(device, "shader.wgsl", LIT_WGSL, "Lit Shader With Details", &scene.lit(&["DETAILS"]));
    #[cfg(gizmo_forward_only)]
    let details = &details_module;
    #[cfg(not(gizmo_forward_only))]
    let details = &shader;
    #[cfg(gizmo_forward_only)]
    let render_details = Some(filled(device, &layouts.lit, details, "Render Pipeline (details)", true, Some(wgpu::Face::Back), OPAQUE));
    #[cfg(not(gizmo_forward_only))]
    let render_details = None;
    // The forward-only path's second set: the same states as `render`, `render_double_sided`,
    // `transparent` and `blend_lit` below, from the shader with the lobes in.
    #[cfg(gizmo_forward_only)]
    let lobes = Some(lit_set(
        device,
        &layouts.lit,
        &path_shader(device, "shader.wgsl", LIT_WGSL, "Lit Shader With Lobes", &scene.lit(&["DETAILS", "LOBES"])),
        "lobes",
    ));
    #[cfg(not(gizmo_forward_only))]
    let lobes = None;
    // The wireframe draws the same shader with `WIREFRAME` on, which lets a frame's
    // `wireframe_color` replace the lit colour. Native only: the web has no line mode, so its
    // "wireframe" is a fill and keeps the lit shader. Probes off: their group 5 is this pipeline's
    // line colour, and a line in the lit colour can spare a reflection.
    #[cfg(not(gizmo_forward_only))]
    let wire_module = path_shader(
        device,
        "shader.wgsl",
        LIT_WGSL,
        "Wireframe Shader",
        &[
            ("WIREFRAME", naga_oil::compose::ShaderDefValue::Bool(true)),
            ("PROBES", naga_oil::compose::ShaderDefValue::Bool(false)),
            ("IRRADIANCE", naga_oil::compose::ShaderDefValue::Bool(false)),
        ],
    );
    #[cfg(not(gizmo_forward_only))]
    let wire = &wire_module;
    // It draws every kind of surface, and a debugging view has nothing to save.
    #[cfg(gizmo_forward_only)]
    let wire = details;
    // Glass: the lit shader reading the frame behind it. The lobes are on, as they are on every
    // desktop lit pipeline: a phone that draws glass has already chosen to pay for a copy of the
    // frame. No blend, since what shows through is part of what it computes. (`DETAILS` and `LOBES`
    // are the desktop's anyway.)
    let glass = path_shader(device, "shader.wgsl", LIT_WGSL, "Glass Shader", &scene.lit(&["DETAILS", "LOBES", "TRANSMISSION"]));
    let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
    LitPipelines {
        // See `OPAQUE`: on every path, as in the G-buffer.
        render: filled(device, &layouts.lit, &shader, "Render Pipeline", true, Some(wgpu::Face::Back), OPAQUE),
        render_double_sided: filled(device, &layouts.lit, details, "Render TwoSided Pipeline", true, None, OPAQUE),
        wireframe: forward_pipeline(
            device,
            &layouts.wire,
            wire,
            "fs_main",
            "Wireframe Pipeline",
            true,
            None,
            alpha,
            // WebGPU/WebGL2 PolygonMode::Line desteklemiyor
            if cfg!(gizmo_forward_only) { wgpu::PolygonMode::Fill } else { wgpu::PolygonMode::Line },
        ),
        transparent: filled(device, &layouts.lit, details, "Transparent Pipeline", false, None, alpha),
        // Same state as `transparent` (no depth write, no culling), each mode's blend.
        blend_lit: crate::components::BlendMode::VARIANTS.map(|mode| {
            forward_pipeline(
                device,
                &layouts.lit,
                details,
                lit_fragment_entry(mode),
                &format!("Transparent Pipeline ({mode:?})"),
                false,
                None,
                Some(mode.state()),
                wgpu::PolygonMode::Fill,
            )
        }),
        lobes,
        render_details,
        transmissive: [
            filled(device, &layouts.lit, &glass, "Glass Pipeline", true, Some(wgpu::Face::Back), None),
            filled(device, &layouts.lit, &glass, "Glass TwoSided Pipeline", true, None, None),
        ],
    }
}

/// The pipelines drawn from `unlit.wgsl` and `baked_lit.wgsl`.
struct UnlitPipelines {
    unlit: wgpu::RenderPipeline,
    unlit_blended: wgpu::RenderPipeline,
    unlit_keeps: Option<wgpu::RenderPipeline>,
    blend_unlit: [wgpu::RenderPipeline; 3],
    baked_lit: wgpu::RenderPipeline,
    baked_lit_double_sided: wgpu::RenderPipeline,
    baked_lit_transparent: wgpu::RenderPipeline,
    baked_lit_depth: [wgpu::RenderPipeline; 2],
}

/// The unlit and baked-lit surfaces, on `layout` (no probe group).
fn unlit_pipelines(device: &wgpu::Device, layout: &wgpu::PipelineLayout, scene: &SceneDefs) -> UnlitPipelines {
    let unlit = path_shader(device, "unlit.wgsl", UNLIT_WGSL, "Unlit Shader", &[]);
    // Unlit that may discard, on the forward-only path. Blended unlit draws take it too: they are small,
    // and a crossfade reaches them. On the desktop path it is the one unlit shader there is.
    #[cfg(gizmo_forward_only)]
    let keeps_module = path_shader(
        device,
        "unlit.wgsl",
        UNLIT_WGSL,
        "Unlit Shader That May Discard",
        &[("KEEPS", naga_oil::compose::ShaderDefValue::Bool(true))],
    );
    #[cfg(gizmo_forward_only)]
    let keeps = &keeps_module;
    #[cfg(not(gizmo_forward_only))]
    let keeps = &unlit;
    #[cfg(gizmo_forward_only)]
    let unlit_keeps = Some(filled(device, layout, keeps, "Unlit Pipeline (may discard)", true, None, OPAQUE));
    #[cfg(not(gizmo_forward_only))]
    let unlit_keeps = None;
    // With the lit shader's lights on the forward-only path (`SceneState::forward_lights`,
    // `forward_local_light_maps`): what the water on a wet baked-lit surface reflects (`water_glints`).
    let baked = path_shader(device, "baked_lit.wgsl", include_str!("../shaders/baked_lit.wgsl"), "Baked-Lit Shader", &scene.lights);
    let opaque_state = baked_lit_state(false);
    let blended_state = baked_lit_state(true);
    let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
    UnlitPipelines {
        unlit: filled(device, layout, &unlit, "Unlit Pipeline", true, None, OPAQUE),
        // A transparent unlit draw blending by alpha, which the opaque `unlit` drew until 2026-11-11
        // (see `OPAQUE`). It may discard, and it writes depth, as that pipeline did for it.
        unlit_blended: filled(device, layout, keeps, "Unlit Pipeline (blended)", true, None, alpha),
        unlit_keeps,
        blend_unlit: crate::components::BlendMode::VARIANTS
            .map(|mode| filled(device, layout, keeps, &format!("Unlit Pipeline ({mode:?})"), false, None, Some(mode.state()))),
        // Backface-culled and opaque, unlike `unlit`. Baked-lit geometry is a solid world rather
        // than a decal or a billboard, so both halves of that are free wins: no blend, and no
        // second pass over every triangle facing away.
        baked_lit: filled(device, layout, &baked, "Baked-Lit Pipeline", opaque_state.depth_write, opaque_state.cull, opaque_state.blend),
        // A double-sided baked-lit material's: the same, culling nothing.
        baked_lit_double_sided: filled(device, layout, &baked, "Baked-Lit TwoSided Pipeline", opaque_state.depth_write, None, opaque_state.blend),
        // …but "a solid world" is not all of it. A decal, a skid mark or a glass pane is
        // baked-lit too, and every other stage already treated those as transparent: the
        // batcher sorts them back-to-front, the z-prepass and the shadow pass skip them. Only
        // the pipeline disagreed — `blend: None` and `depth_write: true` meant the fragment
        // alpha the shader had computed all along was discarded, and the layer painted over
        // the road as opaque geometry. Same shader, transparent state, chosen by
        // `Material::is_transparent` (see `baked_lit_state`).
        baked_lit_transparent: filled(
            device,
            layout,
            &baked,
            "Baked-Lit Transparent Pipeline",
            blended_state.depth_write,
            blended_state.cull,
            blended_state.blend,
        ),
        baked_lit_depth: [
            depth_only_pipeline(device, layout, &baked, "Baked-Lit Depth Pipeline", opaque_state.cull),
            depth_only_pipeline(device, layout, &baked, "Baked-Lit Depth TwoSided Pipeline", None),
        ],
    }
}

/// The scene's own surfaces.
struct ScenePipelines {
    sky: wgpu::RenderPipeline,
    backdrop: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    water_double_sided: wgpu::RenderPipeline,
    water_mirror: wgpu::RenderPipeline,
    mirror: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
}

/// The sky, the scene's painted backdrop, water, planar mirrors and the editor's grid, on
/// `layout` (no probe group).
fn scene_pipelines(device: &wgpu::Device, layout: &wgpu::PipelineLayout) -> ScenePipelines {
    const WATER_WGSL: &str = include_str!("../shaders/water.wgsl");
    let water = path_shader(device, "water.wgsl", WATER_WGSL, "Water Shader", &[]);
    let water_mirror = path_shader(
        device,
        "water.wgsl",
        WATER_WGSL,
        "Water Mirror Shader",
        &[("PLANAR", naga_oil::compose::ShaderDefValue::Bool(true))],
    );
    let mirror = path_shader(device, "planar_mirror.wgsl", include_str!("../shaders/planar_mirror.wgsl"), "Planar Mirror Shader", &[]);
    let sky = path_shader(device, "sky.wgsl", include_str!("../shaders/sky.wgsl"), "Sky Shader", &[]);
    let backdrop_shader = path_shader(device, "backdrop.wgsl", include_str!("../shaders/backdrop.wgsl"), "Backdrop Shader", &[]);
    let grid = path_shader(device, "grid.wgsl", include_str!("../shaders/grid.wgsl"), "Grid Shader", &[]);
    let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
    let backdrop = crate::backdrop::backdrop_state();
    ScenePipelines {
        sky: filled(device, layout, &sky, "Sky Pipeline", false, None, alpha),
        // The scene's own painted backdrop. Everything that makes it a backdrop rather than
        // ordinary unlit geometry is in this one state plus the shader: no depth write (so it
        // cannot occlude the world), no culling (a panel or an inside-out dome), and alpha
        // blending (so a cut-out skyline edge survives). The remaining property — drawn first
        // — belongs to the batcher, which is the only place that knows the frame's draw order.
        backdrop: filled(device, layout, &backdrop_shader, "Backdrop Pipeline", backdrop.depth_write, backdrop.cull, backdrop.blend),
        // Alpha-blended AND depth-writing, which the transparent pipeline is not. Water is a
        // surface rather than a pane: what is under it should be occluded by it, and two water
        // batches at different depths should resolve against each other rather than against the
        // order the batcher happened to drain them in.
        water: filled(device, layout, &water, "Water Pipeline", true, Some(wgpu::Face::Back), alpha),
        // `Material::with_double_sided` on water is not the no-op it is on the transparent
        // pipeline (which is built `cull_mode: None` already): the water pipeline culls back
        // faces, so a swimmer looking up at the surface sees nothing without this variant.
        water_double_sided: filled(device, layout, &water, "Water Double-Sided Pipeline", true, None, alpha),
        // Double-sided whatever the material says: there is a capture only from above, and a wave's
        // far side is hidden by its near one in the depth test.
        water_mirror: filled(device, layout, &water_mirror, "Water Mirror Pipeline", true, None, alpha),
        mirror: filled(device, layout, &mirror, "Planar Mirror Pipeline", true, Some(wgpu::Face::Back), None),
        grid: filled(device, layout, &grid, "Grid Pipeline", false, None, alpha),
    }
}

/// The shadow pass's pipelines, by whether the mesh closes and whether the draw cuts: see
/// [`SceneState::shadow_pipeline_for`](super::SceneState::shadow_pipeline_for).
pub(super) struct ShadowPipelines {
    /// No fragment stage: single-sided by its front faces for a closed mesh, then both faces for an
    /// open one.
    pub(super) plain: [wgpu::RenderPipeline; 2],
    /// `shadow.wgsl`'s `CUT_OUT`, likewise: a cut-out or a LOD crossfade, cut where the picture is.
    pub(super) cut_out: [wgpu::RenderPipeline; 2],
}

/// Builds [`ShadowPipelines`].
///
/// A closed mesh is drawn by its FRONT faces — what the light meets first — and an open one by both.
/// A soft shadow measures its penumbra from the depth of what blocks the light (`sun_shadow_visibility`
/// and `local_shadow_visibility` in `common.wgsl`), and until 2026-11-11 the closed meshes were drawn by
/// their BACK faces, which put that depth at a solid's far side: a cube's shadow came out a third as
/// soft as its light threw. That culling was the acne cure — the lit face's depth never stored, it could
/// not shadow itself — and the lookups cure it now as they do for an open mesh, which always had its lit
/// face in the map: a normal offset of two texels, which at a grazing angle lifts the lookup a distance
/// `offset / cos θ` off the surface along the ray, more than its filter's taps climb it (`1.5 texels ×
/// tan θ`); a slope-scaled depth bias in metres; and across a soft filter's width, each tap compared
/// against the receiver's own plane. An open mesh has no back faces to cast with: drawn by them alone, a
/// plane facing the sun cast nothing (0 shadow pixels against a cube's 236, measured 2026-11-11). The
/// local lights' layers draw every mesh by both faces: a light can be inside a solid, which its front
/// faces alone would let it shine out of.
pub(super) fn build_shadow_pipelines(device: &wgpu::Device, layouts: &LayoutRefs) -> ShadowPipelines {
    use naga_oil::compose::ShaderDefValue::Bool;
    let source = include_str!("../shaders/shadow.wgsl");
    let plain = path_shader(device, "shadow.wgsl", source, "Shadow Shader", &[]);
    let cut = path_shader(device, "shadow.wgsl", source, "Shadow Cut-Out Shader", &[("CUT_OUT", Bool(true))]);
    let groups = [layouts.shadow_pass, layouts.texture, layouts.skeleton, layouts.instance];
    let front = Some(wgpu::Face::Back);
    ShadowPipelines {
        plain: [
            shadow_pipeline(device, groups, &plain, front, false, "Shadow Pipeline"),
            shadow_pipeline(device, groups, &plain, None, false, "Shadow Pipeline (both faces)"),
        ],
        cut_out: [
            shadow_pipeline(device, groups, &cut, front, true, "Shadow Pipeline (cut-outs)"),
            shadow_pipeline(device, groups, &cut, None, true, "Shadow Pipeline (cut-outs, both faces)"),
        ],
    }
}

/// One shadow pass pipeline drawn with `shader`, which is `shadow.wgsl` composed: with `cut_out` it
/// has the fragment stage that cuts, `fs_cut_out`. `groups` are the pass's light, the material, the
/// skeleton and the instances, which every shadow pipeline has in that order, so a pass binds the
/// same groups whichever pipeline draws. A game's surface builds its own through this
/// ([`SurfaceMaterial`](crate::custom_material::SurfaceMaterial)).
pub(crate) fn shadow_pipeline(
    device: &wgpu::Device,
    groups: [&wgpu::BindGroupLayout; 4],
    shader: &wgpu::ShaderModule,
    cull_mode: Option<wgpu::Face>,
    cut_out: bool,
    label: &str,
) -> wgpu::RenderPipeline {
    let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Shadow Pipeline Layout"),
        bind_group_layouts: &groups.map(Some),
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&shadow_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(crate::gpu_types::GpuVertex::desc())],
        },
        // No colour targets either way, and no fragment stage at all unless the draw cuts, which
        // is what keeps the pass cheap.
        fragment: cut_out.then(|| wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_cut_out"),
            compilation_options: Default::default(),
            targets: &[],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            // See `build_shadow_pipelines`.
            cull_mode,
            polygon_mode: wgpu::PolygonMode::Fill,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            // Small: the lookups carry the biases (see `build_shadow_pipelines`), and a large one here
            // would peter-pan — a grey gap between a box and its shadow.
            bias: wgpu::DepthBiasState {
                constant: 1,
                slope_scale: 1.0,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// The entry point of `shader.wgsl` that draws `mode`: each treats fog as that blend needs.
fn lit_fragment_entry(mode: crate::components::BlendMode) -> &'static str {
    use crate::components::BlendMode;
    match mode {
        BlendMode::Alpha => "fs_main",
        BlendMode::Additive => "fs_additive",
        BlendMode::Multiply => "fs_multiply",
        BlendMode::Premultiplied => "fs_premultiplied",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Plumbing vertex alpha into a pipeline that ignores it fixes nothing, so the blend state
    // is the half of the fix that has to be pinned. A built RenderPipeline cannot be
    // interrogated, and this crate cannot read a pixel — `baked_lit_state` exists so the
    // decision is assertable on its own.
    #[test]
    fn transparent_baked_lit_actually_blends() {
        let t = baked_lit_state(true);
        assert_eq!(
            t.blend,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            "a transparent baked-lit draw must blend — with `None` the shader's alpha is \
             computed and then thrown away, which is what made decals opaque"
        );
        assert!(
            !t.depth_write,
            "a blended layer must not write depth: the batcher sorts these back-to-front and \
             a depth write would let the nearest one occlude the ones drawn after it"
        );
        assert_eq!(t.cull, None, "a one-plane decal must be visible from both sides");
    }

    // The other half: nothing about the opaque world may change. These are the exact values
    // the single baked-lit pipeline was built with before the transparent variant existed.
    #[test]
    fn opaque_baked_lit_state_is_unchanged() {
        let o = baked_lit_state(false);
        assert_eq!(o.blend, None, "opaque baked-lit must stay unblended");
        assert!(o.depth_write, "opaque baked-lit must keep writing depth");
        assert_eq!(o.cull, Some(wgpu::Face::Back), "opaque baked-lit must stay backface-culled");
    }

    /// **The forward-only path's room, measured by wgpu** — the browser's and the phone's, where
    /// WebGPU's 16 sampled textures a stage is the limit the device has, not a margin. Its layout is
    /// four groups (global, material, skeleton, instance), and group 0 carries the probe cubes, the
    /// sun's cascades and the local lights' maps that the desktop keeps in groups of its own. As
    /// [`the_forward_lit_layout_has_room_for_material_maps`]: the skeleton group, which has no
    /// texture, is swapped for one of `n` fragment textures, and the largest `n` that validates is
    /// the room left. Nothing measured it until 2026-11-18, when the texture pool's array took a
    /// slot of the material group on every path.
    #[cfg(gizmo_forward_only)]
    #[test]
    fn the_forward_only_layout_fits_webgpu() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::NOOP,
            backend_options: wgpu::BackendOptions {
                noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
                ..Default::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
        let limits = wgpu::Limits { max_bind_groups: 4, max_sampled_textures_per_shader_stage: 16, ..wgpu::Limits::default() };
        let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: limits, ..Default::default() }))
            .expect("a noop device");
        let layouts = crate::pipeline::layouts::build_layouts(&device);
        let room = (0..=16u32)
            .take_while(|&n| {
                let entries: Vec<wgpu::BindGroupLayoutEntry> = (0..n)
                    .map(|binding| wgpu::BindGroupLayoutEntry {
                        binding,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    })
                    .collect();
                let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
                let probe = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("room"), entries: &entries });
                let _layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("forward-only, skeleton swapped"),
                    bind_group_layouts: &[Some(&layouts.global), Some(&layouts.texture), Some(&probe), Some(&layouts.instance)],
                    immediate_size: 0,
                });
                pollster::block_on(scope.pop()).is_none()
            })
            .last()
            .unwrap_or(0);
        eprintln!("the forward-only layout has room for {room} more sampled textures");
        assert!(room >= 1, "the forward-only layout has room for {room} more sampled textures of 16");
    }

    /// **Room for another material map, measured by wgpu.** WebGPU allows 16 sampled textures a
    /// stage, and a pipeline layout counts every one its groups offer whether a shader reads it or
    /// not. On the desktop the forward lit layout held 15 — two of them on group 0 for the
    /// forward-only path alone — so a clear-coat map failed validation at 17 (2026-11-17). Asked here
    /// of wgpu itself (a hand count was off by two): the skeleton group, which has no texture, is
    /// swapped for one of `n` fragment textures, and the largest `n` the layout validates with is the
    /// room left. The noop backend validates limits as a real device does, so this needs no GPU.
    #[cfg(not(gizmo_forward_only))]
    #[test]
    fn the_forward_lit_layout_has_room_for_material_maps() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::NOOP,
            backend_options: wgpu::BackendOptions {
                noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
                ..Default::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the noop adapter");
        // WebGPU's 16 a stage, and enough groups for the native forward pass's six.
        let limits = wgpu::Limits { max_bind_groups: 6, max_sampled_textures_per_shader_stage: 16, ..wgpu::Limits::default() };
        let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: limits, ..Default::default() }))
            .expect("a noop device");
        let layouts = crate::pipeline::layouts::build_layouts(&device);
        let indirect = crate::irradiance::IrradianceState::layout(&device);
        let room = (0..=16u32)
            .take_while(|&n| {
                let entries: Vec<wgpu::BindGroupLayoutEntry> = (0..n)
                    .map(|binding| wgpu::BindGroupLayoutEntry {
                        binding,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    })
                    .collect();
                let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
                let probe = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("room"), entries: &entries });
                let _layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("lit, skeleton swapped"),
                    bind_group_layouts: &[
                        Some(&layouts.global),
                        Some(&layouts.texture),
                        Some(&layouts.shadow),
                        Some(&probe),
                        Some(&layouts.instance),
                        Some(&indirect),
                    ],
                    immediate_size: 0,
                });
                pollster::block_on(scope.pop()).is_none()
            })
            .last()
            .unwrap_or(0);
        // 3 until 2026-11-18, when the texture pool's array (material binding 8) took one; the
        // forward-only layout has the same two left (`the_forward_only_layout_fits_webgpu`).
        assert!(room >= 2, "the forward lit layout has room for {room} more sampled textures of 16");
        eprintln!("the forward lit layout has room for {room} more sampled textures");
    }
}
