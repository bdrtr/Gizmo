//! The renderer's ECS components — what an entity needs to be drawn, lit or looked at.
//!
//! Every submodule is re-exported here, so `components::Mesh` and `components::mesh::Mesh` are the
//! same type and callers need not know which file a component lives in.

/// Skinning and animation playback components.
pub mod animation;
/// Cameras: 3-D and 2-D, perspective and orthographic.
pub mod camera;
/// Projected decals.
pub mod decal;
/// Punctual lights: point, spot and directional.
pub mod light;
/// Materials and the shading route each one takes.
pub mod material;
/// Meshes and their per-entity render settings.
pub mod mesh;
/// Everything else an entity can carry: terrain, LOD groups, particle emitters, render targets,
/// fluid markers and the frame's render statistics.
pub mod misc;

pub use animation::*;
pub use camera::*;
pub use decal::*;
pub use light::*;
pub use material::*;
pub use mesh::*;
pub use misc::*;

gizmo_core::impl_component!(
    Mesh,
    Material,
    Skeleton,
    MeshRenderer,
    Camera,
    Camera2D,
    PointLight,
    Terrain,
    DirectionalLight,
    SpotLight,
    LodGroup,
    LodLevel,
    ParticleEmitter,
    EditorRenderTarget,
    GameRenderTarget,
    RenderStats,
    Decal
);
gizmo_core::impl_component!(FluidParticle, FluidHandle, FluidPhase, FluidInteractor);
