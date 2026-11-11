//! Materials the engine does not have, registered by the game.
//!
//! # Two kinds: a surface the engine lights, and a pipeline of the game's own
//!
//! **A [`SurfaceMaterial`]** says what a surface *is* at a fragment — its colour, normal, roughness,
//! metalness, emission, alpha — in a WGSL function, and the engine does the rest: it is composed into
//! the engine's own G-buffer shader and its own lit forward shader, in place of the engine's
//! material. So on the desktop's deferred path it is written to the G-buffer exactly as `Pbr` is, and
//! gets everything that reads it: the deferred lighting, SSAO, SSR, SSGI, decals, the velocity TAA and
//! motion blur use. Where a draw is forward — the browser, a phone, the editor viewport, a transparent
//! or glass surface — the same function is lit by the forward shader. Drawn by
//! [`MaterialType::CustomSurface`](crate::components::MaterialType::CustomSurface):
//!
//! ```wgsl
//! #import gizmo::surface::{Surface, SurfaceInput}
//! #import gizmo::material::{standard_surface}
//!
//! // The material it is drawn with, banded in red by height.
//! fn material_surface(input: SurfaceInput) -> Surface {
//!     var surface = standard_surface(input);
//!     let band = step(0.5, fract(input.world_position.y * 4.0));
//!     surface.albedo = mix(surface.albedo, vec3<f32>(0.8, 0.1, 0.1), band);
//!     return surface;
//! }
//! ```
//!
//! `standard_surface` is what `MaterialType::Pbr` draws at that fragment, so a surface can start from
//! the engine's material and change part of it, or build its own from `SurfaceInput`'s fields
//! (`gizmo::surface` in WGSL: position, UV, vertex colour, the geometric frame, the instance's
//! albedo, roughness, metalness and emission, the face, the pixel, the clock and the camera). A
//! surface that cuts sets `alpha` under `alpha_cutoff`, and the engine's prepass cuts there too.
//! Anisotropy, clear coat and subsurface still come from the instance's
//! [`Material`](crate::components::Material), as for every lit draw.
//!
//! It does not need a G-buffer channel of its own, which is what an earlier version of this module
//! said made the design impossible: a surface fills the channels every lit draw fills.
//!
//! **A [`CustomMaterial`]** is a whole pipeline of the game's own — its shader does all of the
//! shading, and the engine only binds the groups and draws it, forward, after the deferred resolve,
//! with the depth buffer the rest of the scene wrote. Right for what is not a lit surface: a
//! hologram, a force field, a screen. Drawn by
//! [`MaterialType::Custom`](crate::components::MaterialType::Custom). It can still cast shadows —
//! the shadow pass writes depth only and does not care what a fragment would have shaded to.
//!
//! # Where the game's own data goes — and why not a bind group of its own
//!
//! The same for both kinds.
//!
//! There is no free bind group. The engine's layout is
//! `0 = scene, 1 = material, 2 = shadow, 3 = skeleton, 4 = instance` on native, and
//! `0 = scene, 1 = material, 2 = skeleton, 3 = instance` on the web. Native asks for
//! `max_bind_groups: 6` and spends five; **the web asks for 4 and spends four**. So a design where
//! a custom material brings its own group would work on the desktop and fail to compile a shader
//! in the browser, which is the worst of the two possible failures.
//!
//! The room that does exist is inside **group 1**, and it is not small: the material bind group is
//! seven entries — base colour, sampler, normal, metallic-roughness, emissive, occlusion, and a
//! uniform buffer — and a custom shader is free to mean something else by all of them. Four
//! textures and a uniform buffer, with no new group and no platform split. A surface that reads the
//! engine's material imports its bindings from `gizmo::material`; one that means something else by
//! them declares group 1 itself and does not import that module.
//!
//! [`AssetManager::material`](crate::asset::AssetManager::material) is how they are filled;
//! `params` is the caller's own buffer:
//!
//! ```ignore
//! let bind_group = AssetManager::material()
//!     .base_colour(&flow_field)     // whatever your shader decides these mean
//!     .normal(&noise)
//!     .params(&my_uniforms)
//!     .build(&mut assets, device, queue, layout);
//! ```
//!
//! Vertex layout and groups 0 and 2–4 stay the engine's, because the batching path fills them and
//! a pipeline that disagreed about them would not be a custom material — it would be a second
//! renderer sharing an encoder.
//!
//! # Registration
//!
//! ```no_run
//! # use gizmo_renderer::custom_material::{CustomMaterial, MaterialRegistry};
//! # use gizmo_renderer::components::MaterialType;
//! # fn demo(registry: &mut MaterialRegistry, pipeline: wgpu::RenderPipeline) {
//! let id = registry.register(CustomMaterial::new("hologram", pipeline));
//! let material_type = MaterialType::Custom(id);
//! # }
//! ```
//!
//! `MaterialType::Custom(id)` is a variant rather than a wrapper enum, which is a deviation from
//! how `docs/API_DEPTH.md` sketched this (`MaterialKind { BuiltIn(..), Custom(..) }`). The variant
//! costs nothing the wrapper would have bought: `Material::material_type` keeps its type, all 56
//! existing uses keep compiling, and [`routing`](crate::routing) stays exhaustive — the one
//! property that module exists to have.
//!
//! [`MaterialType::Custom`](crate::components::material::MaterialType::Custom) carries the id, so nothing about `Material` changed and every existing
//! call site still compiles. `routing.rs` gains one arm and stays exhaustive — a tenth built-in
//! variant is still a compile error there, which is the property that module exists for.

/// A registered custom material. Only meaningful to the [`MaterialRegistry`] that minted it.
///
/// `Copy` and `Eq` so it can live inside [`MaterialType`](crate::components::MaterialType), which
/// is both — and serialisable, because a saved scene has to name the material it used. An id from a
/// different run is only as valid as the registration order that produced it; see
/// [`MaterialRegistry::register`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub struct MaterialId(pub u32);

/// Everything the draw loop needs to render one custom material.
pub struct CustomMaterial {
    /// A debug label — what a wgpu validation error will name.
    pub label: String,
    /// The render pipeline.
    ///
    /// Must declare the engine's group layout: `0` scene uniforms, `1` the material bind group,
    /// and — on native — `2` shadow, `3` skeleton, `4` instance. A shader may leave later groups
    /// undeclared if it does not read them; it may not renumber them, and there is no spare group
    /// to add. The module docs say where a custom material's data goes instead.
    pub pipeline: wgpu::RenderPipeline,
    /// Whether this material's geometry is drawn into the shadow cascades.
    ///
    /// Default `true`: a custom material is usually an ordinary opaque surface with an unusual
    /// look, and a solid object that casts no shadow reads as a bug. Set it false for anything
    /// transparent or volumetric, where a depth-only cast would stamp a solid silhouette.
    pub casts_shadows: bool,
}

impl CustomMaterial {
    /// A custom material with the defaults: casts shadows.
    #[must_use]
    pub fn new(label: impl Into<String>, pipeline: wgpu::RenderPipeline) -> Self {
        Self {
            label: label.into(),
            pipeline,
            casts_shadows: true,
        }
    }

    /// Keeps this material out of the shadow pass.
    #[must_use]
    pub fn without_shadows(mut self) -> Self {
        self.casts_shadows = false;
        self
    }
}

/// How a custom material's pipeline is built, for the fields where more than one answer is
/// reasonable.
///
/// Everything not here is fixed by the engine's contract: the vertex layout, the bind-group layout,
/// the `Rgba16Float` HDR target, `Depth32Float` with `LessEqual`, and CCW front faces. A pipeline
/// disagreeing about any of those does not draw in this renderer's forward pass, so making them
/// options would only move the failure later.
#[derive(Clone, Copy, Debug)]
pub struct CustomPipelineOptions {
    /// Which faces to cull. `None` draws both — a cloth, a leaf, a plane seen from behind.
    pub cull_mode: Option<wgpu::Face>,
    /// Blending, or `None` for opaque.
    ///
    /// If you set this, also call [`CustomMaterial::without_shadows`] and give the material an
    /// albedo alpha below 1 — a transparent surface that casts a depth-only shadow stamps a solid
    /// silhouette, which looks like a bug in the shadow map rather than in the material.
    pub blend: Option<wgpu::BlendState>,
    /// Whether fragments write depth. `false` for anything blended.
    pub depth_write: bool,
    /// The vertex entry point.
    pub vertex_entry: &'static str,
    /// The fragment entry point.
    pub fragment_entry: &'static str,
}

impl Default for CustomPipelineOptions {
    fn default() -> Self {
        Self {
            cull_mode: Some(wgpu::Face::Back),
            blend: None,
            depth_write: true,
            vertex_entry: "vs_main",
            fragment_entry: "fs_main",
        }
    }
}

impl CustomMaterial {
    /// Compiles WGSL into a pipeline that fits the engine's forward pass, and wraps it.
    ///
    /// This exists because without it the door opens onto a wall: `CustomMaterial` asks for a
    /// `wgpu::RenderPipeline`, and building one that the forward pass will accept means knowing the
    /// bind-group layout **in the right order for the right platform**, the vertex layout, the
    /// target format and the depth state. All of that is the engine's contract rather than the
    /// game's choice, and getting one of them wrong produces a validation error at pipeline
    /// creation that names none of this.
    ///
    /// The WGSL goes through the same composer the engine's own shaders do, so `#import
    /// gizmo::common::{SceneUniforms}` and `@group(#{SKELETON_GROUP})` / `#{INSTANCE_GROUP}` work
    /// here exactly as they do in `unlit.wgsl` — which is what saves a game from transcribing
    /// `SceneUniforms` by hand and from hard-coding group indices that differ between native and
    /// the web.
    ///
    /// The shader must declare group 0 as the scene uniforms and group 1 as the material bind
    /// group; it may leave the later groups undeclared. See the module docs for what goes where —
    /// there is no spare bind group, and group 1's seven entries are the room.
    ///
    /// # Panics
    ///
    /// If the shader declares the instance buffer with another element size than
    /// [`InstanceRaw`](crate::gpu_types::InstanceRaw)'s: it would read the wrong instance for every
    /// index above 0, with nothing to say so, and the record grows when the engine needs a field (it
    /// gained `optics` on 2026-11-08). The message names both sizes. Otherwise never directly, but `wgpu`
    /// will report a shader that fails to compile through the device's error scope, as it does for
    /// every pipeline in the engine.
    #[must_use]
    pub fn from_wgsl(
        device: &wgpu::Device,
        scene: &crate::pipeline::SceneState,
        label: &str,
        wgsl: &str,
        options: CustomPipelineOptions,
    ) -> Self {
        // Composed, not raw. The engine's own shaders are written against `#import
        // gizmo::common::{SceneUniforms}` and `@group(#{SKELETON_GROUP})`, and a custom material
        // gets the same two affordances — otherwise a game would have to transcribe `SceneUniforms`
        // by hand (and re-transcribe it whenever it changes) and hard-code group indices that
        // differ between native and the web. The defs are the platform's own.
        #[cfg(not(gizmo_forward_only))]
        let composed = crate::pipeline::shaders::compose_wgsl(
            wgsl,
            label,
            crate::pipeline::shaders::native_render_defs(),
        );
        #[cfg(gizmo_forward_only)]
        let composed = crate::pipeline::shaders::compose_wgsl(
            wgsl,
            label,
            crate::pipeline::shaders::web_render_defs(),
        );
        if let Err(problem) = check_instance_stride(&composed) {
            panic!("custom material `{label}`: {problem}");
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(composed.into()),
        });

        // The order is the contract, and it differs by platform: the browser has four bind groups
        // to spend and the shadow group is the one that does not fit.
        #[cfg(not(gizmo_forward_only))]
        let groups: [Option<&wgpu::BindGroupLayout>; 5] = [
            Some(&scene.global_bind_group_layout),
            Some(&scene.texture_bind_group_layout),
            Some(&scene.shadow_bind_group_layout),
            Some(&scene.skeleton_bind_group_layout),
            Some(&scene.instance_bind_group_layout),
        ];
        #[cfg(gizmo_forward_only)]
        let groups: [Option<&wgpu::BindGroupLayout>; 4] = [
            Some(&scene.global_bind_group_layout),
            Some(&scene.texture_bind_group_layout),
            Some(&scene.skeleton_bind_group_layout),
            Some(&scene.instance_bind_group_layout),
        ];

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &groups,
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some(options.vertex_entry),
                compilation_options: Default::default(),
                buffers: &[Some(crate::gpu_types::Vertex::desc())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(options.fragment_entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    // The forward pass draws into the HDR target, not the surface.
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: options.blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: options.cull_mode,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(options.depth_write),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self::new(label, pipeline)
    }
}

/// A registered [`SurfaceMaterial`]. Only meaningful to the [`MaterialRegistry`] that minted it.
///
/// A separate type from [`MaterialId`], so a surface cannot be named where a pipeline of the game's
/// own is meant, or the other way round: the two are drawn by different code, and a mix-up would
/// draw nothing with nothing to say why.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub struct SurfaceId(pub u32);

/// Why a [`SurfaceMaterial`] could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SurfaceError {
    /// The WGSL did not compose with the engine's shaders around it, or the result did not
    /// validate: a syntax error, a type that does not match, no `material_surface` defined, a binding
    /// that collides with one of the engine's. The text is the shader compiler's, and names the shader
    /// it was composed into.
    Compose(String),
}

impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compose(message) => write!(f, "a surface material's WGSL did not compose: {message}"),
        }
    }
}

impl std::error::Error for SurfaceError {}

/// A surface the game describes in WGSL and the engine lights — see the [module docs](self).
///
/// The WGSL defines `fn material_surface(input: SurfaceInput) -> Surface`, and is composed into the
/// engine's G-buffer shader (on the deferred path) and its lit forward shader (everywhere a draw is
/// forward) in place of the engine's material. The pipelines are built when it is, and again whenever
/// the engine's are ([`rebuild_pipelines`](crate::pipeline::rebuild_pipelines): a hot reload, a change
/// in the reflection probes a phone's lit shader is compiled for), from the WGSL it keeps.
pub struct SurfaceMaterial {
    label: String,
    wgsl: String,
    pipelines: SurfacePipelines,
}

/// The pipelines a surface is drawn with, as the engine's own material has them.
struct SurfacePipelines {
    /// The G-buffer pass and its prepass. `None` where there is no G-buffer: the forward-only path, or
    /// a renderer built without one.
    deferred: Option<DeferredSurface>,
    /// Every lit forward draw: on the desktop path with everything in, on the forward-only path the
    /// `DETAILS` tier, since only the surface's own code knows whether it cuts.
    lit: crate::pipeline::LitSet,
    /// The forward-only path's `LOBES` tier, for a material with anisotropy, clear coat or subsurface.
    /// `None` on the desktop path, whose `lit` has them.
    lobes: Option<crate::pipeline::LitSet>,
    /// Glass, single-sided then double-sided.
    glass: [wgpu::RenderPipeline; 2],
    /// The shadow pass, a closed mesh then an open one, always through the cut-out stage: only the
    /// surface's own code knows where it is cut. `None` on the forward-only path, which has no shadow
    /// pass.
    shadow: Option<[wgpu::RenderPipeline; 2]>,
}

/// The surface's G-buffer pipelines, single-sided then double-sided each.
struct DeferredSurface {
    gbuffer: [wgpu::RenderPipeline; 2],
    /// Always the prepass with a fragment stage: the surface decides where it is cut, so the depth it
    /// leaves must have the same holes as the G-buffer.
    prepass: [wgpu::RenderPipeline; 2],
}

impl SurfaceMaterial {
    /// Composes `wgsl` into the engine's shaders and builds every pipeline the surface can be drawn
    /// with on this renderer.
    ///
    /// # Errors
    ///
    /// [`SurfaceError::Compose`] when the WGSL does not compose or validate inside any of them.
    pub fn from_wgsl(renderer: &crate::Renderer, label: &str, wgsl: &str) -> Result<Self, SurfaceError> {
        let pipelines = SurfacePipelines::build(&renderer.device, &renderer.scene, renderer.deferred.as_ref(), label, wgsl)?;
        Ok(Self { label: label.to_string(), wgsl: wgsl.to_string(), pipelines })
    }

    /// The name it was built under, which is also what a validation error about it names.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The G-buffer pass's pipeline for a draw of this surface, `None` where there is no G-buffer.
    #[must_use]
    pub fn gbuffer_pipeline(&self, double_sided: bool) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.deferred.as_ref().map(|d| &d.gbuffer[usize::from(double_sided)])
    }

    /// The depth prepass's, likewise.
    #[must_use]
    pub fn prepass_pipeline(&self, double_sided: bool) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.deferred.as_ref().map(|d| &d.prepass[usize::from(double_sided)])
    }

    /// The lit forward pipeline for a draw of this surface, as
    /// [`SceneState::lit_pipeline`](crate::pipeline::SceneState::lit_pipeline) chooses one for the
    /// engine's own material: `lobes` is the material using anisotropy, clear coat or subsurface,
    /// which only the forward-only path has a tier for.
    #[must_use]
    pub fn lit_pipeline(
        &self,
        lobes: bool,
        transparent: bool,
        double_sided: bool,
        mode: crate::components::BlendMode,
    ) -> &wgpu::RenderPipeline {
        let set = self.pipelines.lobes.as_ref().filter(|_| lobes).unwrap_or(&self.pipelines.lit);
        match (transparent, crate::components::BlendMode::VARIANTS.iter().position(|m| *m == mode)) {
            (true, Some(i)) => &set.blend[i],
            (true, None) => &set.transparent,
            (false, _) if double_sided => &set.render_double_sided,
            (false, _) => &set.render,
        }
    }

    /// The glass pipeline, for a draw of this surface that
    /// [is transmissive](crate::components::Material::is_transmissive).
    #[must_use]
    pub fn transmissive_pipeline(&self, double_sided: bool) -> &wgpu::RenderPipeline {
        &self.pipelines.glass[usize::from(double_sided)]
    }

    /// The shadow pass's pipeline for a draw of this surface: by its back faces alone when
    /// `back_faces` — a mesh that closes ([`Mesh::closed`](crate::components::Mesh::closed)), in the
    /// sun's cascades — and by both otherwise, as
    /// [`SceneState::shadow_pipeline_for`](crate::pipeline::SceneState::shadow_pipeline_for) chooses;
    /// cutting wherever the surface cuts. `None` where there is no shadow pass, the forward-only path.
    ///
    /// The surface is handed the frame's camera and clock there as in the picture, so a surface that
    /// dissolves over time casts what is left of it. Two of `SurfaceInput`'s fields mean the shadow
    /// map's own rather than the picture's: `frag_coord` is the shadow map's pixel, and
    /// `front_facing` is the face turned to the light.
    #[must_use]
    pub fn shadow_pipeline(&self, back_faces: bool) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.shadow.as_ref().map(|shadow| &shadow[usize::from(!back_faces)])
    }

    /// Builds the pipelines again from the WGSL it was made with, against the engine's shaders as they
    /// are now. It composed once, and the engine's shaders it is composed into are the engine's to
    /// keep valid, so a failure here is a broken engine shader rather than the game's: it is logged
    /// and the old pipelines are kept.
    fn rebuild(&mut self, device: &wgpu::Device, scene: &crate::pipeline::SceneState, deferred: Option<&crate::deferred::DeferredState>) {
        match SurfacePipelines::build(device, scene, deferred, &self.label, &self.wgsl) {
            Ok(pipelines) => self.pipelines = pipelines,
            Err(e) => tracing::error!("surface material `{}` kept its old pipelines: {e}", self.label),
        }
    }
}

impl SurfacePipelines {
    fn build(
        device: &wgpu::Device,
        scene: &crate::pipeline::SceneState,
        deferred: Option<&crate::deferred::DeferredState>,
        label: &str,
        wgsl: &str,
    ) -> Result<Self, SurfaceError> {
        use naga_oil::compose::ShaderDefValue::Bool;
        let module = |file: &str, embedded: &str, name: &str, extra: &[(&str, naga_oil::compose::ShaderDefValue)]| {
            // The engine's own shaders are read from disk first on the desktop path, as every one of
            // its pipelines reads them, so a hot-reloaded `gbuffer.wgsl` reaches a surface too.
            #[cfg(not(gizmo_forward_only))]
            let (source, defs) = (
                std::fs::read_to_string(file).unwrap_or_else(|_| embedded.to_string()),
                surface_defs(crate::pipeline::shaders::native_render_defs(), extra),
            );
            // The forward-only path's lit shaders are compiled for the reflection probes the scene has.
            #[cfg(gizmo_forward_only)]
            let (source, defs) = {
                let _ = file;
                let probes = scene.forward_probes.shape.defs();
                let defs: Vec<_> = probes.iter().copied().chain(extra.iter().copied()).collect();
                (embedded.to_string(), surface_defs(crate::pipeline::shaders::web_render_defs(), &defs))
            };
            let name = format!("{label} ({name})");
            let composed = crate::pipeline::shaders::try_compose_wgsl(&source, &name, defs, Some(wgsl))
                .map_err(SurfaceError::Compose)?;
            Ok::<_, SurfaceError>(device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&name),
                source: wgpu::ShaderSource::Wgsl(composed.into()),
            }))
        };
        let lit_wgsl = include_str!("shaders/shader.wgsl");
        let lit_file = "demo/assets/shaders/shader.wgsl";
        // What the forward-only path builds its `DETAILS` tier with, and its glass; on the desktop path
        // every lit shader has them already.
        let details: &[(&str, naga_oil::compose::ShaderDefValue)] =
            if cfg!(gizmo_forward_only) { &[("DETAILS", Bool(true))] } else { &[] };
        let with_lobes: &[(&str, naga_oil::compose::ShaderDefValue)] =
            &[("DETAILS", Bool(true)), ("LOBES", Bool(true))];

        let layout = crate::pipeline::lit_pipeline_layout(device, scene);
        let lit = crate::pipeline::lit_set(device, &layout, &module(lit_file, lit_wgsl, "lit", details)?, label);
        let lobes = if cfg!(gizmo_forward_only) {
            Some(crate::pipeline::lit_set(device, &layout, &module(lit_file, lit_wgsl, "lobes", with_lobes)?, &format!("{label}, lobes")))
        } else {
            None
        };
        let glass_module = module(
            lit_file,
            lit_wgsl,
            "glass",
            &[("DETAILS", Bool(true)), ("LOBES", Bool(true)), ("TRANSMISSION", Bool(true))],
        )?;
        let glass = [Some(wgpu::Face::Back), None].map(|cull| {
            crate::pipeline::forward_pipeline(
                device,
                &layout,
                &glass_module,
                "fs_main",
                &format!("{label} (glass)"),
                true,
                cull,
                None,
                wgpu::PolygonMode::Fill,
            )
        });

        let shadow = if cfg!(gizmo_forward_only) {
            None
        } else {
            let caster = module(
                "demo/assets/shaders/shadow.wgsl",
                include_str!("shaders/shadow.wgsl"),
                "shadow",
                &[("CUT_OUT", Bool(true))],
            )?;
            let groups = [
                &scene.shadow_pass_bind_group_layout,
                &scene.texture_bind_group_layout,
                &scene.skeleton_bind_group_layout,
                &scene.instance_bind_group_layout,
            ];
            Some([Some(wgpu::Face::Front), None].map(|cull| {
                crate::pipeline::shadow_pipeline(device, groups, &caster, cull, true, &format!("{label} (shadow)"))
            }))
        };

        let deferred = match deferred {
            Some(state) => {
                let geometry = module("demo/assets/shaders/gbuffer.wgsl", include_str!("shaders/gbuffer.wgsl"), "G-buffer", &[])?;
                let motion = &state.motion.layout;
                let sides = [Some(wgpu::Face::Back), None];
                Some(DeferredSurface {
                    gbuffer: sides.map(|cull| {
                        crate::deferred::DeferredState::create_gbuffer_pipeline(device, scene, motion, &geometry, cull, label)
                    }),
                    prepass: sides.map(|cull| {
                        crate::deferred::DeferredState::create_z_prepass_pipeline(device, scene, motion, &geometry, cull, true, label)
                    }),
                })
            }
            None => None,
        };
        Ok(Self { deferred, lit, lobes, glass, shadow })
    }
}

/// The defs a surface's shader is composed under: the platform's, `CUSTOM_SURFACE`, and `extra` in
/// order. `#ifdef` asks whether a name is there, so a `false` takes the name away rather than adding
/// it, as the engine's own `load_shader_composed_*_with` do: a forward-only surface was composed with
/// its probe defs' `false` left in, which compiled the probe code in, and every pixel of it differed
/// from the engine's material until this said so.
fn surface_defs(
    mut defs: std::collections::HashMap<String, naga_oil::compose::ShaderDefValue>,
    extra: &[(&str, naga_oil::compose::ShaderDefValue)],
) -> std::collections::HashMap<String, naga_oil::compose::ShaderDefValue> {
    use naga_oil::compose::ShaderDefValue::Bool;
    for (name, value) in std::iter::once(("CUSTOM_SURFACE", Bool(true))).chain(extra.iter().copied()) {
        if value == Bool(false) {
            defs.remove(name);
        } else {
            defs.insert(name.to_string(), value);
        }
    }
    defs
}

/// The registered custom materials, indexed by [`MaterialId`], and the registered surfaces, by
/// [`SurfaceId`].
#[derive(Default)]
pub struct MaterialRegistry {
    materials: Vec<CustomMaterial>,
    surfaces: Vec<SurfaceMaterial>,
}

impl MaterialRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a material and returns its id.
    ///
    /// Ids are handed out in registration order and are never reused, so an id is stable for the
    /// life of the registry. It is **not** stable across runs unless registration order is: a
    /// scene serialising `MaterialType::Custom(MaterialId(2))` and reloading against a differently
    /// ordered registration will get a different material, silently. Register in a fixed order, or
    /// look ids up by name with [`id_of`](Self::id_of).
    pub fn register(&mut self, material: CustomMaterial) -> MaterialId {
        let id = MaterialId(
            u32::try_from(self.materials.len()).expect("a registry never reaches 4 billion"),
        );
        self.materials.push(material);
        id
    }

    /// The material behind an id, or `None` if it was minted by a different registry.
    #[must_use]
    pub fn get(&self, id: MaterialId) -> Option<&CustomMaterial> {
        self.materials.get(id.0 as usize)
    }

    /// The id of the first material registered under `label`.
    ///
    /// The order-independent way to name a material, which is what a saved scene should use.
    #[must_use]
    pub fn id_of(&self, label: &str) -> Option<MaterialId> {
        self.materials
            .iter()
            .position(|m| m.label == label)
            .map(|i| MaterialId(i as u32))
    }

    /// Registers a surface and returns its id. Ids are handed out as [`register`](Self::register)'s
    /// are, in an order of their own, with the same caveat about a saved scene.
    pub fn register_surface(&mut self, surface: SurfaceMaterial) -> SurfaceId {
        let id = SurfaceId(u32::try_from(self.surfaces.len()).expect("a registry never reaches 4 billion"));
        self.surfaces.push(surface);
        id
    }

    /// The surface behind an id, or `None` if it was minted by a different registry.
    #[must_use]
    pub fn surface(&self, id: SurfaceId) -> Option<&SurfaceMaterial> {
        self.surfaces.get(id.0 as usize)
    }

    /// The id of the first surface registered under `label`.
    #[must_use]
    pub fn surface_id_of(&self, label: &str) -> Option<SurfaceId> {
        self.surfaces
            .iter()
            .position(|s| s.label == label)
            .map(|i| SurfaceId(i as u32))
    }

    /// Builds every registered surface's pipelines again, as the engine's own are rebuilt: see
    /// [`rebuild_pipelines`](crate::pipeline::rebuild_pipelines).
    pub(crate) fn rebuild_surfaces(
        &mut self,
        device: &wgpu::Device,
        scene: &crate::pipeline::SceneState,
        deferred: Option<&crate::deferred::DeferredState>,
    ) {
        for surface in &mut self.surfaces {
            surface.rebuild(device, scene, deferred);
        }
    }

    /// How many materials of both kinds are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.materials.len() + self.surfaces.len()
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.materials.is_empty() && self.surfaces.is_empty()
    }
}

/// Whether a composed custom shader's instance buffer, if it declares one, has the engine's element size.
/// The group is the platform's `INSTANCE_GROUP`, binding 0.
fn check_instance_stride(composed: &str) -> Result<(), String> {
    let group = if cfg!(gizmo_forward_only) { 3 } else { 4 };
    let module = naga::front::wgsl::parse_str(composed).map_err(|e| e.to_string())?;
    let want = std::mem::size_of::<crate::gpu_types::InstanceRaw>() as u32;
    for (_, var) in module.global_variables.iter() {
        let Some(binding) = &var.binding else { continue };
        if binding.group != group || binding.binding != 0 {
            continue;
        }
        if let naga::TypeInner::Array { stride, .. } = module.types[var.ty].inner {
            if stride != want {
                return Err(format!(
                    "its instance struct is {stride} bytes and the engine's `InstanceRaw` is {want}: declare every \
                     field `gpu_types::InstanceRaw` has, in order (`model_matrix_0`..`3`, `albedo_color`, `pbr`, \
                     `ambient`, `emissive`, `optics`), or `instances[i]` reads the wrong instance for every i > 0"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// A custom shader declaring the instance struct as it was before `optics` (eight vec4s) is told
    /// so, with both sizes, rather than reading the wrong instance for every index above 0; the
    /// struct as it is now, and a shader with no instance buffer at all, pass.
    #[test]
    fn a_custom_shader_with_the_old_instance_struct_is_told_so() {
        let group = if cfg!(gizmo_forward_only) { 3 } else { 4 };
        let shader = |fields: &str| {
            format!(
                "struct InstanceRaw {{ {fields} }};\n\
                 @group({group}) @binding(0) var<storage, read> instances: array<InstanceRaw>;\n\
                 @vertex fn vs_main(@builtin(instance_index) i: u32) -> @builtin(position) vec4<f32> {{ return instances[i].m0; }}"
            )
        };
        let old = "m0: vec4<f32>, m1: vec4<f32>, m2: vec4<f32>, m3: vec4<f32>, a: vec4<f32>, p: vec4<f32>, b: vec4<f32>, e: vec4<f32>,";
        let err = super::check_instance_stride(&shader(old)).expect_err("eight vec4s are the old record");
        assert!(err.contains("128") && err.contains("144") && err.contains("optics"), "{err}");
        let now = format!("{old} o: vec4<f32>,");
        assert_eq!(super::check_instance_stride(&shader(&now)), Ok(()));
        let none = "@vertex fn vs_main() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); }";
        assert_eq!(super::check_instance_stride(none), Ok(()), "a shader that reads no instances");
    }

    /// **A surface composes into every shader it is drawn with, on both paths.** On the desktop: the
    /// G-buffer shader, the lit shader and its glass. On the forward-only path: the `DETAILS` and
    /// `LOBES` tiers of the lit shader and its glass. Composed means validated, so a surface that fails
    /// in one of them fails to build at all instead of drawing on one path and not on another.
    #[test]
    fn a_surface_composes_into_every_shader_it_is_drawn_with() {
        use crate::pipeline::shaders::{native_render_defs, try_compose_wgsl, web_render_defs};
        use naga_oil::compose::ShaderDefValue::Bool;
        let surface = "#import gizmo::surface::{Surface, SurfaceInput}\n\
                       #import gizmo::material::{standard_surface}\n\
                       fn material_surface(input: SurfaceInput) -> Surface {\n\
                           var s = standard_surface(input);\n\
                           s.albedo = vec3<f32>(0.123, 0.456, 0.789) * input.time;\n\
                           return s;\n\
                       }\n";
        let gbuffer = include_str!("shaders/gbuffer.wgsl");
        let lit = include_str!("shaders/shader.wgsl");
        let with = |mut defs: std::collections::HashMap<String, naga_oil::compose::ShaderDefValue>, extra: &[&str]| {
            defs.insert("CUSTOM_SURFACE".to_string(), Bool(true));
            for def in extra {
                defs.insert((*def).to_string(), Bool(true));
            }
            defs
        };
        let cases = [
            (gbuffer, with(native_render_defs(), &[]), "desktop G-buffer"),
            (lit, with(native_render_defs(), &[]), "desktop lit"),
            (lit, with(native_render_defs(), &["TRANSMISSION"]), "desktop glass"),
            (lit, with(web_render_defs(), &["DETAILS"]), "forward-only lit"),
            (lit, with(web_render_defs(), &["DETAILS", "LOBES"]), "forward-only lobes"),
            (lit, with(web_render_defs(), &["DETAILS", "LOBES", "TRANSMISSION"]), "forward-only glass"),
        ];
        for (source, defs, name) in cases {
            let composed = try_compose_wgsl(source, name, defs, Some(surface)).unwrap_or_else(|e| panic!("{name}: {e}"));
            // The surface's own numbers are in what is drawn: its function is the one called.
            assert!(composed.contains("0.123f") && composed.contains("0.789f"), "{name} lost the surface's function");
        }
        // And without one, the engine's material is: `evaluate_surface` samples the normal map.
        let engine = try_compose_wgsl(gbuffer, "engine G-buffer", native_render_defs(), None).expect("the engine's G-buffer composes");
        assert!(engine.contains("t_normal") && !engine.contains("0.123f"));
    }

    /// A def a surface is composed under as `false` is not there, so its code is not either: the probe
    /// code of a forward-only surface in a scene with no probes. It was, and it changed every pixel of
    /// the surface against the engine's material on that path.
    #[test]
    fn a_false_def_leaves_its_code_out_of_a_surface() {
        use crate::pipeline::shaders::{try_compose_wgsl, web_render_defs};
        use naga_oil::compose::ShaderDefValue::Bool;
        let surface = "#import gizmo::surface::{Surface, SurfaceInput}\n\
                       #import gizmo::material::{standard_surface}\n\
                       fn material_surface(input: SurfaceInput) -> Surface { return standard_surface(input); }\n";
        let lit = include_str!("shaders/shader.wgsl");
        let composed = |background| {
            let defs = super::surface_defs(web_render_defs(), &[("PROBE_BACKGROUND", Bool(background)), ("DETAILS", Bool(true))]);
            assert_eq!(defs.contains_key("PROBE_BACKGROUND"), background);
            assert!(defs.contains_key("CUSTOM_SURFACE") && defs.contains_key("DETAILS"));
            try_compose_wgsl(lit, "probes", defs, Some(surface)).expect("composes")
        };
        assert!(!composed(false).contains("probe_background"), "a scene with no probe pays for one");
        assert!(composed(true).contains("probe_background"), "a scene with a probe lost it");
    }

    /// A surface may mean something else by group 1, as a [`CustomMaterial`] may: one that declares its
    /// own binding 6 and imports nothing from `gizmo::material` composes, and the engine's bindings are
    /// not in what it composes to.
    #[test]
    fn a_surface_can_mean_its_own_group_1() {
        use crate::pipeline::shaders::{native_render_defs, try_compose_wgsl};
        let surface = "#import gizmo::surface::{Surface, SurfaceInput}\n\
                       struct Mine { tint: vec4<f32>, speed: vec4<f32>, a: vec4<f32>, b: vec4<f32> };\n\
                       @group(1) @binding(6) var<uniform> mine: Mine;\n\
                       @group(1) @binding(0) var noise: texture_2d<f32>;\n\
                       @group(1) @binding(1) var noise_sampler: sampler;\n\
                       fn material_surface(input: SurfaceInput) -> Surface {\n\
                           var s: Surface;\n\
                           let n = textureSample(noise, noise_sampler, input.uv * mine.speed.xy).r;\n\
                           s.base_color = mine.tint.rgb * n;\n\
                           s.albedo = s.base_color;\n\
                           s.alpha = 1.0;\n\
                           s.alpha_cutoff = 0.0;\n\
                           s.N = normalize(input.N);\n\
                           s.roughness = 0.5;\n\
                           s.metallic = 0.0;\n\
                           s.emissive = vec3<f32>(0.0);\n\
                           return s;\n\
                       }\n";
        let mut defs = native_render_defs();
        defs.insert("CUSTOM_SURFACE".to_string(), naga_oil::compose::ShaderDefValue::Bool(true));
        for (source, name) in [(include_str!("shaders/gbuffer.wgsl"), "G-buffer"), (include_str!("shaders/shader.wgsl"), "lit")] {
            let composed = try_compose_wgsl(source, name, defs.clone(), Some(surface)).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!composed.contains("t_normal") && !composed.contains("MaterialParams"), "{name} carries the engine's material");
        }
    }

    /// A surface that does not compose is the game's error, handed back: a syntax error, and a module
    /// without `material_surface`, whose message names the function.
    #[test]
    fn a_surface_that_does_not_compose_is_an_error() {
        use crate::pipeline::shaders::{native_render_defs, try_compose_wgsl};
        let mut defs = native_render_defs();
        defs.insert("CUSTOM_SURFACE".to_string(), naga_oil::compose::ShaderDefValue::Bool(true));
        let gbuffer = include_str!("shaders/gbuffer.wgsl");
        assert!(try_compose_wgsl(gbuffer, "broken", defs.clone(), Some("fn material_surface(")).is_err());
        let missing = try_compose_wgsl(gbuffer, "missing", defs, Some("fn something_else() {}")).expect_err("no material_surface");
        assert!(missing.contains("material_surface"), "{missing}");
    }

    use super::*;

    #[test]
    fn ids_are_handed_out_in_order_and_resolve_back() {
        // No GPU needed: this is the bookkeeping half, and it is the half that can silently
        // mis-resolve a saved scene.
        let r = MaterialRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.get(MaterialId(0)).map(|m| m.label.as_str()), None);
        assert_eq!(r.id_of("nothing"), None);
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn an_id_from_nowhere_resolves_to_none_rather_than_a_neighbour() {
        // The failure this guards is a scene loaded against a smaller registry: `get` must not
        // wrap, saturate, or hand back material 0.
        let r = MaterialRegistry::new();
        assert!(r.get(MaterialId(7)).is_none());
        assert!(r.get(MaterialId(u32::MAX)).is_none());
    }
}
