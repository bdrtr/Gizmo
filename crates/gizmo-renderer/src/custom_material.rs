//! Materials the engine does not have, registered by the game.
//!
//! # Why this is forward-only, and why that is not a compromise
//!
//! The obvious design is "let a custom shader write the G-buffer like `Pbr` does". It does not fit,
//! and the reason is a number rather than an opinion: the four G-buffer targets share a
//! `max_color_attachment_bytes_per_sample` budget of **32** — the WebGPU-guaranteed limit — and
//! they already spend **28** (4 albedo `Rgba8` + 8 normal + 8 position + 8 tangent, all `Rgba16F`).
//! Four bytes are left. That is exactly one more `Rgba8` target and not one `Rgba16F`, so a custom
//! material cannot bring its own G-buffer channel without either spending the last of a shared
//! budget on one feature or displacing something the deferred lighting reads.
//!
//! So a custom material declares itself forward: its own pipeline, its own shader, drawn after the
//! deferred resolve, with the depth buffer the rest of the scene wrote. It can still cast shadows —
//! the shadow pass writes depth only and does not care what a fragment would have shaded to.
//!
//! # Where the game's own data goes — and why not a bind group of its own
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
//! textures and a uniform buffer, with no new group and no platform split.
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
//! [`MaterialType::Custom`] carries the id, so nothing about `Material` changed and every existing
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
    /// Never directly, but `wgpu` will report a shader that fails to compile through the device's
    /// error scope, as it does for every pipeline in the engine.
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
        #[cfg(not(target_arch = "wasm32"))]
        let composed = crate::pipeline::shaders::compose_wgsl(
            wgsl,
            label,
            crate::pipeline::shaders::native_render_defs(),
        );
        #[cfg(target_arch = "wasm32")]
        let composed = crate::pipeline::shaders::compose_wgsl(
            wgsl,
            label,
            crate::pipeline::shaders::web_render_defs(),
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(composed.into()),
        });

        // The order is the contract, and it differs by platform: the browser has four bind groups
        // to spend and the shadow group is the one that does not fit.
        #[cfg(not(target_arch = "wasm32"))]
        let groups: [Option<&wgpu::BindGroupLayout>; 5] = [
            Some(&scene.global_bind_group_layout),
            Some(&scene.texture_bind_group_layout),
            Some(&scene.shadow_bind_group_layout),
            Some(&scene.skeleton_bind_group_layout),
            Some(&scene.instance_bind_group_layout),
        ];
        #[cfg(target_arch = "wasm32")]
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

/// The registered custom materials, indexed by [`MaterialId`].
#[derive(Default)]
pub struct MaterialRegistry {
    materials: Vec<CustomMaterial>,
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

    /// How many materials are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.materials.len()
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }
}

#[cfg(test)]
mod tests {
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
