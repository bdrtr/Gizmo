//! GPU mesh rendering for the Gizmo engine (wgpu pipelines, materials, instancing).
//!
//! ## Frustum culling (CPU-side, before instancing)
//!
//! The renderer does not iterate entities; **your render loop** should build the instance list.
//! Before pushing each [`InstanceRaw`](gpu_types::InstanceRaw), skip meshes outside the camera
//! frustum using the same `view_proj` (`projection * view`) matrix you upload in
//! [`SceneUniforms`](gpu_types::SceneUniforms):
//!
//! ```
//! # use gizmo_math::{Aabb, Mat4, Vec3};
//! use gizmo_renderer::{visible_in_frustum, Frustum};
//!
//! # let view_proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 100.0)
//! #     * Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
//! # // Stands in for `mesh.bounds`: a unit cube in local space.
//! # let bounds = Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5));
//! # let mut instances = 0;
//! let frustum = Frustum::from_matrix(&view_proj);
//! for model_matrix in [
//!     Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0)), // in front of the camera
//!     Mat4::from_translation(Vec3::new(0.0, 0.0, 50.0)),  // behind it
//! ] {
//!     if !visible_in_frustum(&frustum, &model_matrix, bounds) {
//!         continue;
//!     }
//! #   instances += 1;
//!     // ...push an `InstanceRaw` for this mesh.
//! }
//! # assert_eq!(instances, 1, "the box behind the camera is culled, not instanced");
//! ```
//!
//! [`Mesh`](components::Mesh) carries a local-space [`Aabb`](gizmo_math::Aabb) (`bounds`);
//! `visible_in_frustum` transforms it by the instance model matrix and tests against the six planes.
//! This pairs with batched `draw(vertex_range, instance_start..instance_end)` so culled instances
//! are never written to the instance buffer. The `demo`, `gizmo-studio`, and
//! `gizmo::systems::default_render_pass` pipelines already apply this pattern.
//!
//! Implementation: [`frustum_cull`] re-exports [`Frustum`] and helpers from `gizmo-math`.

pub mod asset;
pub mod async_assets;
pub mod backdrop;
pub mod components;
pub mod csm;
pub mod debug_renderer;
pub mod decal;
pub mod deferred;
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
mod test_gpu;
mod texture_quality;
pub mod volumetric;
pub mod web_profile;
pub mod asset_loading;

pub use backdrop::{
    backdrop_clip_position, backdrop_rgba, camera_locked_model, is_camera_locked,
    BACKDROP_NDC_DEPTH,
};
pub use frustum_cull::{classify_visibility, visible_in_frustum, Frustum, Visibility};
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
    PointLight, SpotLight, Sprite,
};
pub use csm::{
    cascade_split_distances, compute_directional_cascades, directional_cascade_view_projs,
    shadow_distance_fade, ShadowCascades, CASCADE_COUNT, CASCADE_LAMBDA, SHADOW_DISTANCE,
    SHADOW_FADE_FRACTION, SHADOW_MAP_RES,
};
pub use debug_renderer::{GizmoRendererSystem, Gizmos};
pub use decal::DecalState;
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
