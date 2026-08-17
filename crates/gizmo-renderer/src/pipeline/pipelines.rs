//! Creating the render pipelines (core + shadow).

use super::layouts::LayoutRefs;
use super::shaders::load_shader;
#[cfg(not(target_arch = "wasm32"))]
use super::shaders::load_shader_composed;
#[cfg(target_arch = "wasm32")]
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
    pub(super) transparent: wgpu::RenderPipeline,
    pub(super) grid: wgpu::RenderPipeline,
}

pub(super) fn build_core_pipelines(device: &wgpu::Device, layouts: &LayoutRefs) -> CorePipelines {
    // WASM: max 4 bind groups (Chrome WebGPU), shadow kaldırıldı
    // Native: 5 bind groups (shadow dahil)
    #[cfg(not(target_arch = "wasm32"))]
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
    #[cfg(target_arch = "wasm32")]
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

    #[cfg(not(target_arch = "wasm32"))]
    let shader = load_shader_composed(
        device,
        "demo/assets/shaders/shader.wgsl",
        include_str!("../shaders/shader.wgsl"),
        "Shader",
    );
    #[cfg(target_arch = "wasm32")]
    let shader = load_shader_composed_web(device, include_str!("../shaders/shader.wgsl"), "Shader");

    #[cfg(not(target_arch = "wasm32"))]
    let unlit_shader = load_shader_composed(
        device,
        "demo/assets/shaders/unlit.wgsl",
        include_str!("../shaders/unlit.wgsl"),
        "Unlit Shader",
    );
    #[cfg(not(target_arch = "wasm32"))]
    let baked_lit_shader = load_shader_composed(
        device,
        "demo/assets/shaders/baked_lit.wgsl",
        include_str!("../shaders/baked_lit.wgsl"),
        "Baked-Lit Shader",
    );
    #[cfg(target_arch = "wasm32")]
    let unlit_shader = load_shader_composed_web(device, include_str!("../shaders/unlit.wgsl"), "Unlit Shader");
    #[cfg(target_arch = "wasm32")]
    let baked_lit_shader =
        load_shader_composed_web(device, include_str!("../shaders/baked_lit.wgsl"), "Baked-Lit Shader");

    #[cfg(not(target_arch = "wasm32"))]
    let water_shader = load_shader_composed(
        device,
        "demo/assets/shaders/water.wgsl",
        include_str!("../shaders/water.wgsl"),
        "Water Shader",
    );
    #[cfg(target_arch = "wasm32")]
    let water_shader = load_shader_composed_web(device, include_str!("../shaders/water.wgsl"), "Water Shader");

    #[cfg(not(target_arch = "wasm32"))]
    let sky_shader = load_shader_composed(
        device,
        "demo/assets/shaders/sky.wgsl",
        include_str!("../shaders/sky.wgsl"),
        "Sky Shader",
    );
    #[cfg(target_arch = "wasm32")]
    let sky_shader = load_shader_composed_web(device, include_str!("../shaders/sky.wgsl"), "Sky Shader");

    #[cfg(not(target_arch = "wasm32"))]
    let backdrop_shader = load_shader_composed(
        device,
        "demo/assets/shaders/backdrop.wgsl",
        include_str!("../shaders/backdrop.wgsl"),
        "Backdrop Shader",
    );
    #[cfg(target_arch = "wasm32")]
    let backdrop_shader =
        load_shader_composed_web(device, include_str!("../shaders/backdrop.wgsl"), "Backdrop Shader");

    #[cfg(not(target_arch = "wasm32"))]
    let grid_shader = load_shader_composed(
        device,
        "demo/assets/shaders/grid.wgsl",
        include_str!("../shaders/grid.wgsl"),
        "Grid Shader",
    );
    #[cfg(target_arch = "wasm32")]
    let grid_shader = load_shader_composed_web(device, include_str!("../shaders/grid.wgsl"), "Grid Shader");

    let create_main = |sm: &wgpu::ShaderModule,
                       label: &str,
                       depth_write: bool,
                       cull: Option<wgpu::Face>,
                       blend: Option<wgpu::BlendState>,
                       polygon_mode: wgpu::PolygonMode| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: sm,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(Vertex::desc())],
            },
            fragment: Some(wgpu::FragmentState {
                module: sm,
                entry_point: Some("fs_main"),
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

    let opaque_state = baked_lit_state(false);
    let blended_state = baked_lit_state(true);
    let backdrop = crate::backdrop::backdrop_state();

    CorePipelines {
        render: create_main(
            &shader,
            "Render Pipeline",
            true,
            Some(wgpu::Face::Back),
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        render_double_sided: create_main(
            &shader,
            "Render TwoSided Pipeline",
            true,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        wireframe: create_main(
            &shader,
            "Wireframe Pipeline",
            true,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            // WebGPU/WebGL2 PolygonMode::Line desteklemiyor
            if cfg!(target_arch = "wasm32") {
                wgpu::PolygonMode::Fill
            } else {
                wgpu::PolygonMode::Line
            },
        ),
        transparent: create_main(
            &shader,
            "Transparent Pipeline",
            false,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        unlit: create_main(
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
            &baked_lit_shader,
            "Baked-Lit Transparent Pipeline",
            blended_state.depth_write,
            blended_state.cull,
            blended_state.blend,
            wgpu::PolygonMode::Fill,
        ),
        sky: create_main(
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
            &backdrop_shader,
            "Backdrop Pipeline",
            backdrop.depth_write,
            backdrop.cull,
            backdrop.blend,
            wgpu::PolygonMode::Fill,
        ),
        water: create_main(
            &water_shader,
            "Water Pipeline",
            true,
            Some(wgpu::Face::Back),
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
        grid: create_main(
            &grid_shader,
            "Grid Pipeline",
            false,
            None,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::PolygonMode::Fill,
        ),
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
