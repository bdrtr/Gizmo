//! GPU mesh rendering for the Gizmo engine (wgpu pipelines, materials, instancing).
//!
//! ## Frustum culling (CPU-side, before instancing)
//!
//! The renderer does not iterate entities; **your render loop** should build the instance list.
//! Before pushing each [`InstanceRaw`](gpu_types::InstanceRaw), skip meshes outside the camera
//! frustum using the same `view * projection` matrix you upload in [`SceneUniforms`](gpu_types::SceneUniforms):
//!
//! ```ignore
//! use gizmo_renderer::{Frustum, visible_in_frustum};
//! let frustum = Frustum::from_matrix(&view_proj);
//! if !visible_in_frustum(&frustum, &model_matrix, &mesh.bounds) {
//!     continue;
//! }
//! ```
//!
//! [`Mesh`](components::Mesh) carries a local-space [`Aabb`](gizmo_math::Aabb) (`bounds`);
//! `visible_in_frustum` transforms it by the instance model matrix and tests against the six planes.
//! This pairs with batched `draw(vertex_range, instance_start..instance_end)` so culled instances
//! are never written to the instance buffer. The `demo`, `gizmo-studio`, and
//! `gizmo::default_systems::default_render_pass` pipelines already apply this pattern.
//!
//! Implementation: [`frustum_cull`] re-exports [`Frustum`] and helpers from `gizmo-math`.

pub mod animation;
pub mod animation_state_machine;
pub mod asset;
pub mod async_assets;
pub mod components;
pub mod csm;
pub mod deferred;
pub mod decal;
pub mod debug_renderer;
pub mod frustum_cull;
pub mod game_ui;
pub mod gpu_cull;
pub mod gpu_fluid;
pub mod gpu_particles;
pub mod gpu_physics;
pub mod gpu_types;
pub mod hot_reload;
pub mod pipeline;
pub mod post_process;
pub mod renderer;
pub mod ssao;
pub mod ssr;
pub mod taa;
pub mod volumetric;

pub use frustum_cull::{visible_in_frustum, Frustum};

pub use animation::{AnimationClip, Keyframe, SkeletonHierarchy, SkeletonJoint, Track};
pub use animation_state_machine::{
    ActiveBlend, AnimationState, AnimationStateMachine, AnimationTransition,
};
pub mod animation_system;
pub use animation_system::{animation_state_machine_update_system, animation_update_system};
pub use asset::{
    decode_obj_vertices_for_async, decode_rgba_image_file, AssetManager, GltfNodeData,
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
    cascade_split_distances, directional_cascade_view_projs, CASCADE_COUNT, SHADOW_MAP_RES,
};
pub use debug_renderer::{GizmoRendererSystem, Gizmos};
pub use game_ui::{Anchor, UiCanvas, UiElement, UiKind};
pub use gpu_types::{
    InstanceRaw, LightData, PostProcessUniforms, SceneUniforms, ShadowVsUniform, Vertex,
};
pub use deferred::DeferredState;
pub use decal::DecalState;
pub use gpu_cull::{DrawIndirectArgs, GpuCullState, MeshBoundsRaw};
pub use ssao::{SsaoParams, SsaoState};
pub use taa::TaaState;
pub use hot_reload::AssetWatcher;
pub use pipeline::SceneState;
pub use post_process::PostProcessState;
pub use renderer::Renderer;
