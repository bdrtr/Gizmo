//! Creating the render pipelines (core + shadow).

use super::layouts::LayoutRefs;
use super::shaders::load_shader;
#[cfg(not(gizmo_forward_only))]
use super::shaders::load_shader_composed;
#[cfg(gizmo_forward_only)]
use super::shaders::load_shader_composed_web;
use crate::gpu_types::Vertex;

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

pub(super) struct CorePipelines {
    pub(super) render: wgpu::RenderPipeline,
    pub(super) render_double_sided: wgpu::RenderPipeline,
    pub(super) wireframe: wgpu::RenderPipeline,
    pub(super) unlit: wgpu::RenderPipeline,
    pub(super) baked_lit: wgpu::RenderPipeline,
    pub(super) baked_lit_transparent: wgpu::RenderPipeline,
    pub(super) sky: wgpu::RenderPipeline,
    /// The mesh's own painted sky/panorama — see `crate::backdrop`.
    pub(super) backdrop: wgpu::RenderPipeline,
    pub(super) water: wgpu::RenderPipeline,
    /// The same, culling off — an ocean surface is looked at from underneath.
    pub(super) water_double_sided: wgpu::RenderPipeline,
    pub(super) mirror: wgpu::RenderPipeline,
    pub(super) water_mirror: wgpu::RenderPipeline,
    pub(super) transparent: wgpu::RenderPipeline,
    pub(super) grid: wgpu::RenderPipeline,
    /// The lit transparent pipeline under each of `BlendMode::VARIANTS`.
    pub(super) blend_lit: [wgpu::RenderPipeline; 3],
    /// The lit pipelines with anisotropy, clear coat and subsurface, on the path where the ones above
    /// leave them out. See [`LitLobes`](super::LitLobes).
    pub(super) lobes: Option<super::LitLobes>,
    /// See [`SceneState::render_details_pipeline`](super::SceneState::render_details_pipeline).
    pub(super) render_details: Option<wgpu::RenderPipeline>,
    /// See [`SceneState::unlit_keeps_pipeline`](super::SceneState::unlit_keeps_pipeline).
    pub(super) unlit_keeps: Option<wgpu::RenderPipeline>,
    /// The lit shader with `TRANSMISSION` on: single-sided, then double-sided.
    pub(super) transmissive: [wgpu::RenderPipeline; 2],
    /// The unlit pipeline under each of them — and, unlike `unlit`, not writing depth: a blended
    /// draw is sorted, and a glow writing depth would hide the glow behind it.
    pub(super) blend_unlit: [wgpu::RenderPipeline; 3],
}

pub(super) fn build_core_pipelines(device: &wgpu::Device, layouts: &LayoutRefs) -> CorePipelines {
    // WASM: max 4 bind groups (Chrome WebGPU), shadow kaldırıldı
    // Native: 5 bind groups (shadow dahil)
    #[cfg(not(gizmo_forward_only))]
    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[
            Some(layouts.global),   // 0
            Some(layouts.texture),  // 1
            Some(layouts.shadow),   // 2
            Some(layouts.skeleton), // 3
            Some(layouts.instance), // 4
        ],
        immediate_size: 0,
    });
    // The lit forward shader's own layout: the same five plus the irradiance volume's group at 5,
    // for its reflection probes (`shader.wgsl`'s `#ifdef PROBES`). Only pipelines built from that
    // shader use it; unlit, grid and backdrop keep five. A forward pass binds group 5 once — a
    // bound group stays bound across pipeline switches.
    #[cfg(not(gizmo_forward_only))]
    let lit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Lit Forward Pipeline Layout"),
        bind_group_layouts: &[
            Some(layouts.global),
            Some(layouts.texture),
            Some(layouts.shadow),
            Some(layouts.skeleton),
            Some(layouts.instance),
            Some(layouts.indirect), // 5
        ],
        immediate_size: 0,
    });
    #[cfg(gizmo_forward_only)]
    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[
            Some(layouts.global),   // 0
            Some(layouts.texture),  // 1
            Some(layouts.skeleton), // 2
            Some(layouts.instance), // 3
        ],
        immediate_size: 0,
    });

    #[cfg(not(gizmo_forward_only))]
    let shader = load_shader_composed(
        device,
        "demo/assets/shaders/shader.wgsl",
        include_str!("../shaders/shader.wgsl"),
        "Shader",
    );
    #[cfg(gizmo_forward_only)]
    let shader = load_shader_composed_web(device, include_str!("../shaders/shader.wgsl"), "Shader");
    // The wireframe draws the same shader with `WIREFRAME` on, which lets a frame's
    // `wireframe_color` replace the lit colour. Native only: the web has no line mode, so its
    // "wireframe" is a fill and keeps the lit shader.
    #[cfg(not(gizmo_forward_only))]
    let wire_shader_module = super::shaders::load_shader_composed_with(
        device,
        "demo/assets/shaders/shader.wgsl",
        include_str!("../shaders/shader.wgsl"),
        "Wireframe Shader",
        // Probes off: their group 5 is this pipeline's line colour, and a line in the lit colour can
        // spare a reflection.
        &[
            ("WIREFRAME", naga_oil::compose::ShaderDefValue::Bool(true)),
            ("PROBES", naga_oil::compose::ShaderDefValue::Bool(false)),
        ],
    );
    #[cfg(not(gizmo_forward_only))]
    let wire_shader = &wire_shader_module;

    #[cfg(not(gizmo_forward_only))]
    let unlit_shader = load_shader_composed(
        device,
        "demo/assets/shaders/unlit.wgsl",
        include_str!("../shaders/unlit.wgsl"),
        "Unlit Shader",
    );
    #[cfg(not(gizmo_forward_only))]
    let baked_lit_shader = load_shader_composed(
        device,
        "demo/assets/shaders/baked_lit.wgsl",
        include_str!("../shaders/baked_lit.wgsl"),
        "Baked-Lit Shader",
    );
    #[cfg(gizmo_forward_only)]
    let unlit_shader = load_shader_composed_web(device, include_str!("../shaders/unlit.wgsl"), "Unlit Shader");
    #[cfg(gizmo_forward_only)]
    let baked_lit_shader =
        load_shader_composed_web(device, include_str!("../shaders/baked_lit.wgsl"), "Baked-Lit Shader");

    #[cfg(not(gizmo_forward_only))]
    let mirror_shader = load_shader_composed(
        device,
        "demo/assets/shaders/planar_mirror.wgsl",
        include_str!("../shaders/planar_mirror.wgsl"),
        "Planar Mirror Shader",
    );
    #[cfg(gizmo_forward_only)]
    let mirror_shader =
        load_shader_composed_web(device, include_str!("../shaders/planar_mirror.wgsl"), "Planar Mirror Shader");

    let planar = [("PLANAR", naga_oil::compose::ShaderDefValue::Bool(true))];
    #[cfg(not(gizmo_forward_only))]
    let water_mirror_shader = super::shaders::load_shader_composed_with(
        device,
        "demo/assets/shaders/water.wgsl",
        include_str!("../shaders/water.wgsl"),
        "Water Mirror Shader",
        &planar,
    );
    #[cfg(gizmo_forward_only)]
    let water_mirror_shader = super::shaders::load_shader_composed_web_with(
        device,
        include_str!("../shaders/water.wgsl"),
        "Water Mirror Shader",
        &planar,
    );

    #[cfg(not(gizmo_forward_only))]
    let water_shader = load_shader_composed(
        device,
        "demo/assets/shaders/water.wgsl",
        include_str!("../shaders/water.wgsl"),
        "Water Shader",
    );
    #[cfg(gizmo_forward_only)]
    let water_shader = load_shader_composed_web(device, include_str!("../shaders/water.wgsl"), "Water Shader");

    #[cfg(not(gizmo_forward_only))]
    let sky_shader = load_shader_composed(
        device,
        "demo/assets/shaders/sky.wgsl",
        include_str!("../shaders/sky.wgsl"),
        "Sky Shader",
    );
    #[cfg(gizmo_forward_only)]
    let sky_shader = load_shader_composed_web(device, include_str!("../shaders/sky.wgsl"), "Sky Shader");

    #[cfg(not(gizmo_forward_only))]
    let backdrop_shader = load_shader_composed(
        device,
        "demo/assets/shaders/backdrop.wgsl",
        include_str!("../shaders/backdrop.wgsl"),
        "Backdrop Shader",
    );
    #[cfg(gizmo_forward_only)]
    let backdrop_shader =
        load_shader_composed_web(device, include_str!("../shaders/backdrop.wgsl"), "Backdrop Shader");

    #[cfg(not(gizmo_forward_only))]
    let grid_shader = load_shader_composed(
        device,
        "demo/assets/shaders/grid.wgsl",
        include_str!("../shaders/grid.wgsl"),
        "Grid Shader",
    );
    #[cfg(gizmo_forward_only)]
    let grid_shader = load_shader_composed_web(device, include_str!("../shaders/grid.wgsl"), "Grid Shader");

    #[cfg(not(gizmo_forward_only))]
    let lit_layout = &lit_pipeline_layout;
    #[cfg(gizmo_forward_only)]
    let lit_layout = &render_pipeline_layout;
    #[cfg(not(gizmo_forward_only))]
    let wire_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Wireframe Pipeline Layout"),
        bind_group_layouts: &[
            Some(layouts.global),
            Some(layouts.texture),
            Some(layouts.shadow),
            Some(layouts.skeleton),
            Some(layouts.instance),
            Some(layouts.wire_color), // 5
        ],
        immediate_size: 0,
    });
    #[cfg(not(gizmo_forward_only))]
    let wire_layout = &wire_pipeline_layout;
    #[cfg(gizmo_forward_only)]
    let wire_layout = &render_pipeline_layout;
    let create_at = |entry: &str,
                     layout: &wgpu::PipelineLayout,
                     sm: &wgpu::ShaderModule,
                     label: &str,
                     depth_write: bool,
                     cull: Option<wgpu::Face>,
                     blend: Option<wgpu::BlendState>,
                     polygon_mode: wgpu::PolygonMode| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: sm,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(Vertex::desc())],
            },
            fragment: Some(wgpu::FragmentState {
                module: sm,
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
    };
    let create_main = |layout: &wgpu::PipelineLayout,
                       sm: &wgpu::ShaderModule,
                       label: &str,
                       depth_write: bool,
                       cull: Option<wgpu::Face>,
                       blend: Option<wgpu::BlendState>,
                       polygon_mode: wgpu::PolygonMode| {
        create_at("fs_main", layout, sm, label, depth_write, cull, blend, polygon_mode)
    };

    let opaque_state = baked_lit_state(false);
    let blended_state = baked_lit_state(true);
    let backdrop = crate::backdrop::backdrop_state();

    // The forward-only path's second set: the same states as `render`, `render_double_sided`,
    // `transparent` and `blend_lit` below, from the shader with the lobes in.
    #[cfg(gizmo_forward_only)]
    let lobes = {
        let shader = super::shaders::load_shader_composed_web_with(
            device,
            include_str!("../shaders/shader.wgsl"),
            "Lit Shader With Lobes",
            &[
                ("DETAILS", naga_oil::compose::ShaderDefValue::Bool(true)),
                ("LOBES", naga_oil::compose::ShaderDefValue::Bool(true)),
            ],
        );
        let blend = wgpu::BlendState::ALPHA_BLENDING;
        Some(super::LitLobes {
            render: create_main(lit_layout, &shader, "Render Pipeline (lobes)", true, Some(wgpu::Face::Back), Some(blend), wgpu::PolygonMode::Fill),
            render_double_sided: create_main(lit_layout, &shader, "Render TwoSided Pipeline (lobes)", true, None, Some(blend), wgpu::PolygonMode::Fill),
            transparent: create_main(lit_layout, &shader, "Transparent Pipeline (lobes)", false, None, Some(blend), wgpu::PolygonMode::Fill),
            blend: crate::components::BlendMode::VARIANTS.map(|mode| {
                create_at(lit_fragment_entry(mode), lit_layout, &shader, &format!("Transparent Pipeline ({mode:?}, lobes)"), false, None, Some(mode.state()), wgpu::PolygonMode::Fill)
            }),
        })
    };
    #[cfg(not(gizmo_forward_only))]
    let lobes = None;

    // The middle tier on the forward-only path: what a double-sided, blended or cut-out draw needs and a
    // plain one does not. On the desktop path it is the one shader there is.
    #[cfg(gizmo_forward_only)]
    let details_shader_module = super::shaders::load_shader_composed_web_with(
        device,
        include_str!("../shaders/shader.wgsl"),
        "Lit Shader With Details",
        &[("DETAILS", naga_oil::compose::ShaderDefValue::Bool(true))],
    );
    #[cfg(gizmo_forward_only)]
    let details_shader = &details_shader_module;
    #[cfg(not(gizmo_forward_only))]
    let details_shader = &shader;
    // It draws every kind of surface, and a debugging view has nothing to save.
    #[cfg(gizmo_forward_only)]
    let wire_shader = details_shader;
    #[cfg(gizmo_forward_only)]
    let render_details = Some(create_main(
        lit_layout,
        details_shader,
        "Render Pipeline (details)",
        true,
        Some(wgpu::Face::Back),
        Some(wgpu::BlendState::ALPHA_BLENDING),
        wgpu::PolygonMode::Fill,
    ));
    #[cfg(not(gizmo_forward_only))]
    let render_details = None;

    // Unlit that may discard, on the forward-only path. Blended unlit draws take it too: they are small,
    // and a crossfade reaches them. On the desktop path it is the one unlit shader there is.
    #[cfg(gizmo_forward_only)]
    let unlit_keeps_module = super::shaders::load_shader_composed_web_with(
        device,
        include_str!("../shaders/unlit.wgsl"),
        "Unlit Shader That May Discard",
        &[("KEEPS", naga_oil::compose::ShaderDefValue::Bool(true))],
    );
    #[cfg(gizmo_forward_only)]
    let unlit_keeps_shader = &unlit_keeps_module;
    #[cfg(not(gizmo_forward_only))]
    let unlit_keeps_shader = &unlit_shader;
    #[cfg(gizmo_forward_only)]
    let unlit_keeps = Some(create_main(
        &render_pipeline_layout,
        unlit_keeps_shader,
        "Unlit Pipeline (may discard)",
        true,
        None,
        Some(wgpu::BlendState::ALPHA_BLENDING),
        wgpu::PolygonMode::Fill,
    ));
    #[cfg(not(gizmo_forward_only))]
    let unlit_keeps = None;

    // Glass: the lit shader reading the frame behind it. The lobes are on, as they are on every
    // desktop lit pipeline: a phone that draws glass has already chosen to pay for a copy of the
    // frame. No blend, since what shows through is part of what it computes.
    #[cfg(not(gizmo_forward_only))]
    let glass_shader = super::shaders::load_shader_composed_with(
        device,
        "demo/assets/shaders/shader.wgsl",
        include_str!("../shaders/shader.wgsl"),
        "Glass Shader",
        &[("TRANSMISSION", naga_oil::compose::ShaderDefValue::Bool(true))],
    );
    #[cfg(gizmo_forward_only)]
    let glass_shader = super::shaders::load_shader_composed_web_with(
        device,
        include_str!("../shaders/shader.wgsl"),
        "Glass Shader",
        &[
            ("DETAILS", naga_oil::compose::ShaderDefValue::Bool(true)),
            ("LOBES", naga_oil::compose::ShaderDefValue::Bool(true)),
            ("TRANSMISSION", naga_oil::compose::ShaderDefValue::Bool(true)),
        ],
    );
    let transmissive = [
        create_main(lit_layout, &glass_shader, "Glass Pipeline", true, Some(wgpu::Face::Back), None, wgpu::PolygonMode::Fill),
        create_main(lit_layout, &glass_shader, "Glass TwoSided Pipeline", true, None, None, wgpu::PolygonMode::Fill),
    ];

    CorePipelines {
        lobes,
        render_details,
        unlit_keeps,
        transmissive,
        render: create_main(
            lit_layout,
            &shader,
            "Render Pipeline",
            true,
            Some(wgpu::Face::Back),
            // Opaque means opaque on the forward-only path, as it does in the G-buffer: this tier has no
            // discard, and blending a texel away while writing its depth would cut whatever is drawn
            // behind it later. A surface that shows through says so with `Material::is_transparent` or
            // `Material::alpha_cutoff`, and is not on this tier.
            if cfg!(gizmo_forward_only) { None } else { Some(wgpu::BlendState::ALPHA_BLENDING) },
            wgpu::PolygonMode::Fill,
        ),
        render_double_sided: create_main(
            lit_layout,
            details_shader,
            "Render TwoSided Pipeline",
            true,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        wireframe: create_main(
            wire_layout,
            wire_shader,
            "Wireframe Pipeline",
            true,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            // WebGPU/WebGL2 PolygonMode::Line desteklemiyor
            if cfg!(gizmo_forward_only) {
                wgpu::PolygonMode::Fill
            } else {
                wgpu::PolygonMode::Line
            },
        ),
        transparent: create_main(
            lit_layout,
            details_shader,
            "Transparent Pipeline",
            false,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        unlit: create_main(
            &render_pipeline_layout,
            &unlit_shader,
            "Unlit Pipeline",
            true,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        // Backface-culled and opaque, unlike `unlit`. Baked-lit geometry is a solid world rather
        // than a decal or a billboard, so both halves of that are free wins: no blend, and no
        // second pass over every triangle facing away.
        baked_lit: create_main(
            &render_pipeline_layout,
            &baked_lit_shader,
            "Baked-Lit Pipeline",
            opaque_state.depth_write,
            opaque_state.cull,
            opaque_state.blend,
            wgpu::PolygonMode::Fill,
        ),
        // …but "a solid world" is not all of it. A decal, a skid mark or a glass pane is
        // baked-lit too, and every other stage already treated those as transparent: the
        // batcher sorts them back-to-front, the z-prepass and the shadow pass skip them. Only
        // the pipeline disagreed — `blend: None` and `depth_write: true` meant the fragment
        // alpha the shader had computed all along was discarded, and the layer painted over
        // the road as opaque geometry. Same shader, transparent state, chosen by
        // `Material::is_transparent` (see `baked_lit_state`).
        baked_lit_transparent: create_main(
            &render_pipeline_layout,
            &baked_lit_shader,
            "Baked-Lit Transparent Pipeline",
            blended_state.depth_write,
            blended_state.cull,
            blended_state.blend,
            wgpu::PolygonMode::Fill,
        ),
        sky: create_main(
            &render_pipeline_layout,
            &sky_shader,
            "Sky Pipeline",
            false,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        // The scene's own painted backdrop. Everything that makes it a backdrop rather than
        // ordinary unlit geometry is in this one state plus the shader: no depth write (so it
        // cannot occlude the world), no culling (a panel or an inside-out dome), and alpha
        // blending (so a cut-out skyline edge survives). The remaining property — drawn first
        // — belongs to the batcher, which is the only place that knows the frame's draw order.
        backdrop: create_main(
            &render_pipeline_layout,
            &backdrop_shader,
            "Backdrop Pipeline",
            backdrop.depth_write,
            backdrop.cull,
            backdrop.blend,
            wgpu::PolygonMode::Fill,
        ),
        // Alpha-blended AND depth-writing, which the transparent pipeline is not. Water is a
        // surface rather than a pane: what is under it should be occluded by it, and two water
        // batches at different depths should resolve against each other rather than against the
        // order the batcher happened to drain them in.
        water: create_main(
            &render_pipeline_layout,
            &water_shader,
            "Water Pipeline",
            true,
            Some(wgpu::Face::Back),
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        // `Material::with_double_sided` on water is not the no-op it is on the transparent
        // pipeline (which is built `cull_mode: None` already): the water pipeline culls back
        // faces, so a swimmer looking up at the surface sees nothing without this variant.
        water_double_sided: create_main(
            &render_pipeline_layout,
            &water_shader,
            "Water Double-Sided Pipeline",
            true,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        // Double-sided whatever the material says: there is a capture only from above, and a wave's
        // far side is hidden by its near one in the depth test.
        water_mirror: create_main(
            &render_pipeline_layout,
            &water_mirror_shader,
            "Water Mirror Pipeline",
            true,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        mirror: create_main(
            &render_pipeline_layout,
            &mirror_shader,
            "Planar Mirror Pipeline",
            true,
            Some(wgpu::Face::Back),
            None,
            wgpu::PolygonMode::Fill,
        ),
        grid: create_main(
            &render_pipeline_layout,
            &grid_shader,
            "Grid Pipeline",
            false,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        // Same state as `transparent` (no depth write, no culling), each mode's blend.
        blend_lit: crate::components::BlendMode::VARIANTS.map(|mode| {
            create_at(
                lit_fragment_entry(mode),
                lit_layout,
                details_shader,
                &format!("Transparent Pipeline ({mode:?})"),
                false,
                None,
                Some(mode.state()),
                wgpu::PolygonMode::Fill,
            )
        }),
        blend_unlit: crate::components::BlendMode::VARIANTS.map(|mode| {
            create_main(
                &render_pipeline_layout,
                unlit_keeps_shader,
                &format!("Unlit Pipeline ({mode:?})"),
                false,
                None,
                Some(mode.state()),
                wgpu::PolygonMode::Fill,
            )
        }),
    }
}

pub(super) fn build_shadow_pipeline(device: &wgpu::Device, layouts: &LayoutRefs) -> wgpu::RenderPipeline {
    let shadow_shader = load_shader(
        device,
        "demo/assets/shaders/shadow.wgsl",
        include_str!("../shaders/shadow.wgsl"),
        "Shadow Shader",
    );
    let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Shadow Pipeline Layout"),
        bind_group_layouts: &[Some(layouts.shadow_pass), Some(layouts.skeleton), Some(layouts.instance)],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Shadow Pipeline"),
        layout: Some(&shadow_layout),
        vertex: wgpu::VertexState {
            module: &shadow_shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(Vertex::desc())],
        },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            // FRONT-FACE CULLING: gölge haritasına yalnız ARKA yüzler yazılır. Aydınlık ön
            // yüzün derinliği hiç saklanmadığından kendini gölgeleyemez → self-shadow acne
            // (yalayan yüzlerdeki dikey grenli bantlar) tamamen biter. Normal-offset grazing
            // açıda etkisizdi (offset·L → 0); solid casterlar için kanonik çözüm budur.
            // (Katı geometri varsayar; tek-yüzlü ince mesh'ler gölge kaybedebilir.)
            cull_mode: Some(wgpu::Face::Front),
            polygon_mode: wgpu::PolygonMode::Fill,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            // Front-face culling arka yüzleri sakladığından bias çok az gerekir; peter-panning
            // (kutu ile gölgesi arası gri boşluk) olmasın diye düşük tutuldu.
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
}
