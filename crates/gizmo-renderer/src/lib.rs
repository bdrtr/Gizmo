//! GPU mesh rendering for the Gizmo engine (wgpu pipelines, materials, instancing).
//!
//! ## Frustum culling (CPU-side, before instancing)
//!
//! The renderer does not iterate entities; **your render loop** builds the instance list. So the
//! cull is yours too, and how you write it decides whether the frame costs `O(visible)` or
//! `O(everything you own)`.
//!
//! **Hold a [`RenderAabbTree`] across frames and query it.** Insert each renderable's
//! world-space box once, update it when it moves, remove it when it dies, and each frame ask
//! for the keys that survive the camera frustum and every shadow cascade. What comes back is a
//! conservative *superset* of the visible set, so your exact test still runs — on a few hundred
//! candidates instead of every mesh you own.
//!
//! Note the shape of the loop below: it still walks **every** renderable and uses the candidate
//! set only to *skip*. That is not an accident, and it is not the same as iterating the
//! candidate list. See the comment on the guard.
//!
//! ```
//! use gizmo_math::{Aabb, Mat4, Vec3};
//! use gizmo_renderer::{classify_visibility_world, Frustum, RenderAabbTree, Visibility};
//! use gizmo_renderer::components::MaterialType;
//!
//! # let view_proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 100.0)
//! #     * Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
//! # // Stands in for `mesh.bounds`: a unit cube in local space.
//! # let bounds = Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5));
//! // ── once, at load ──────────────────────────────────────────────────────────────
//! let mut index = RenderAabbTree::new();
//! let models = [
//!     Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0)), // in front of the camera
//!     Mat4::from_translation(Vec3::new(0.0, 0.0, 50.0)),  // behind it
//! ];
//! for (key, model) in models.iter().enumerate() {
//!     // The key is yours to choose — an entity id, an index into your own array. It must be
//!     // small and dense. Insert the box of the transform the mesh is actually DRAWN with.
//!     index.insert(key as u32, bounds.transform(model));
//! }
//!
//! // ── every frame ────────────────────────────────────────────────────────────────
//! // ...`index.insert(key, new_box)` for whatever moved — an object that stayed inside its
//! // fat box costs one containment test and returns `false`.
//! // ...`index.insert(key, box)` for whatever was SPAWNED, too.
//! // ...`index.remove(key)`, or `index.retain(|k| still_alive(k))`, for whatever died.
//! let camera = Frustum::from_matrix(&view_proj);
//! let cascades: Vec<Frustum> = vec![]; // your shadow cascades, if you have them
//!
//! let mut candidates = Vec::new();
//! let mut frusta = vec![camera];
//! frusta.extend_from_slice(&cascades);
//! index.query_frusta(&frusta, &mut candidates); // sorted ascending, deduplicated
//!
//! # let mut instances = 0;
//! for (key, model) in models.iter().enumerate() {
//!     let key = key as u32;
//!     // THE GUARD — and note which way it fails. Skip a mesh only when the index KNOWS about
//!     // it and did not nominate it. A key the index never received — spawned this frame,
//!     // refused by `insert` (an empty `Mesh::bounds` is), deliberately not indexed (a
//!     // camera-locked backdrop), or simply forgotten by your maintenance — falls through to
//!     // the exact test and is drawn.
//!     //
//!     // Iterating `candidates` directly instead is the same loop with the `index.contains`
//!     // half deleted, and it converts every maintenance gap into geometry that silently
//!     // stops being drawn. Fail open: a false positive costs one exact test, a false
//!     // negative is an invisible building.
//!     if index.contains(key) && candidates.binary_search(&key).is_err() {
//!         continue;
//!     }
//!     // The index is a SKIP FILTER, never the decision. Run the exact test on what survives.
//!     let world_aabb = bounds.transform(model);
//!     match classify_visibility_world(
//!         &camera, &cascades, world_aabb, MaterialType::Pbr, false, 1.0,
//!     ) {
//!         Visibility::Culled => continue,
//!         Visibility::Camera => { /* main passes */ }
//!         Visibility::ShadowOnly => { /* shadow maps only */ }
//!     }
//! #   instances += 1;
//!     // ...push an `InstanceRaw` for this mesh.
//! }
//! # assert_eq!(instances, 1, "the box behind the camera is culled, not instanced");
//! ```
//!
//! [`Mesh`](components::Mesh) carries a local-space [`Aabb`](gizmo_math::Aabb) (`bounds`);
//! [`Aabb::transform`](gizmo_math::Aabb::transform) puts it in world space. This pairs with
//! batched `draw(vertex_range, instance_start..instance_end)` so culled instances are never
//! written to the instance buffer.
//!
//! ### Which function to reach for
//!
//! * [`RenderAabbTree`] — the spatial index. Use it whenever you have more than a few hundred
//!   renderables. See [`visibility`] for the correctness argument (the candidate set is a
//!   superset by construction, so the draw list is unchanged) and for the two things that must
//!   **not** be indexed: camera-locked materials, and anything whose drawn box you cannot
//!   compute without the camera.
//! * [`classify_visibility_world`] — the exact per-object decision (camera / shadow-only /
//!   culled) against a world-space box. One `Aabb::transform` for the camera *and* every
//!   cascade.
//! * [`classify_visibility`] — the same, taking a model matrix and a local box. Convenience;
//!   it transforms the box for you.
//! * [`visible_in_frustum`] — the single-object primitive, one frustum, no material logic.
//!   Fine for a handful of objects, a debug overlay, or a one-off test.
//!
//! A spatial index answers "what might be on screen". If you also have a cell/region system for
//! streaming, LOD tiers or gameplay partitioning, **keep it** — a BVH has no stable region
//! identity and cannot answer those questions. The two are complementary.
//!
//! Implementation: [`frustum_cull`] re-exports [`Frustum`] and helpers from `gizmo-math`;
//! [`visibility`] holds the index.

pub mod asset;
pub mod async_assets;
pub mod backdrop;
pub mod routing;
pub mod components;
pub mod csm;
pub mod debug_renderer;
pub mod decal;
pub mod deferred;
pub mod draw_order;
pub mod frame_uniforms;
pub mod frustum_cull;
pub mod fxaa;
pub mod gi;
pub mod gpu_cull;
pub mod gpu_fluid;
pub mod gpu_particles;
pub mod gpu_smoke;
pub mod gpu_physics;
pub mod gpu_types;
pub mod hot_reload;
pub mod pipeline;
pub mod post_process;
pub mod renderer;
pub mod ssao;
pub mod ssgi;
pub mod ssr;
pub mod taa;
#[cfg(test)]
mod shader_contract;
#[cfg(test)]
mod test_gpu;
mod texture_quality;
pub mod visibility;
pub mod volumetric;
pub mod web_profile;
pub mod asset_loading;

pub use backdrop::{
    backdrop_clip_position, backdrop_rgba, camera_locked_model, instance_model, is_camera_locked,
    BACKDROP_NDC_DEPTH,
};
pub use frame_uniforms::{
    CameraFrame, EnvironmentFrame, SceneFrame, ShadowFrame, SunFrame, UnderwaterFog, MAX_LIGHTS,
};
pub use frustum_cull::{
    classify_visibility, classify_visibility_world, visible_in_frustum, Frustum, Visibility,
};
// `NO_KEY` is deliberately NOT re-exported at the crate root — the name only means anything
// next to the index. Reach it as `gizmo_renderer::visibility::NO_KEY`.
pub use visibility::{RenderAabbTree, VisibleSet};
pub use web_profile::{PostProcessLevel, ShadowQuality, WebProfile};

pub use gizmo_animation::skeletal::{
    ActiveBlend, AnimationClip, AnimationPlayer, AnimationState, AnimationStateMachine,
    AnimationTransition, BoneAttachment, InterpolationMode, Keyframe, SkeletonHierarchy,
    SkeletonJoint, Track,
};
pub mod animation_system;
pub use animation_system::{animation_state_machine_update_system, animation_update_system};
pub use gizmo_animation::skeletal::decompose_mat4;
pub use asset::{
    decode_obj_vertices_for_async, decode_rgba_image_file, AssetError, AssetManager, GltfNodeData,
    ObjIndexKind,
};
pub use async_assets::{
    AsyncAssetLoader, CompletedAsyncLoads, GltfImportCompletion, GltfImportError,
    ObjLoadCompletion, TextureReloadCompletion,
};
pub use components::{
    Camera, Camera2D, DirectionalLight, LodGroup, LodLevel, Material, Mesh, MeshRenderer,
    PointLight, SpotLight,
};
pub use csm::{
    cascade_split_distances, compute_directional_cascades, directional_cascade_view_projs,
    shadow_distance_fade, ShadowCascades, CASCADE_COUNT, CASCADE_LAMBDA, SHADOW_DISTANCE,
    SHADOW_FADE_FRACTION, SHADOW_MAP_RES,
};
pub use debug_renderer::{GizmoRendererSystem, Gizmos};
pub use decal::DecalState;
pub use draw_order::{batch_depth, sort_back_to_front};
pub use deferred::{
    DeferredState, GBUFFER_ALBEDO_METALLIC_FORMAT, GBUFFER_NORMAL_ROUGHNESS_FORMAT,
    GBUFFER_WORLD_POSITION_FORMAT, GBUFFER_WORLD_TANGENT_FORMAT,
};
pub use gi::{LightProbe, ProbeGrid, SHCoeffs};
pub use gpu_cull::{DrawIndirectArgs, GpuCullState, MeshBoundsRaw};
pub use gpu_types::{
    InstanceRaw, LightData, MaterialParams, PostProcessUniforms, SceneUniforms, ShadowVsUniform,
    Vertex,
};
pub use hot_reload::AssetWatcher;
pub use pipeline::SceneState;
pub use post_process::PostProcessState;
pub use renderer::{RenderContext, Renderer};
pub use ssao::{SsaoParams, SsaoState};
pub use ssgi::SsgiState;
pub use taa::TaaState;
pub use fxaa::FxaaState;
