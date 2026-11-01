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
/// Lights: point (sphere, tube), spot, directional and rectangle.
pub mod light;
/// Materials and the shading route each one takes.
pub mod material;
/// Meshes and their per-entity render settings.
pub mod mesh;
/// Everything else an entity can carry: terrain, LOD groups, particle emitters, render targets,
/// fluid markers and the frame's render statistics.
pub mod misc;
/// The authored look — bloom, vignette, depth of field — on the camera that renders it.
pub mod post_process;
/// Where the scene is captured for specular reflections.
pub mod reflection_probe;
/// The [`Sprite`](sprite::Sprite) component — an image the engine draws.
pub mod sprite;
/// The [`Text`](text::Text) component — a string the engine draws.
pub mod text;
/// The [`Tilemap`](tilemap::Tilemap) component — a grid of tiles from one tileset image.
pub mod tilemap;

pub use animation::*;
pub use camera::*;
pub use decal::*;
pub use light::*;
pub use material::*;
pub use mesh::*;
pub use misc::*;
pub use post_process::*;
pub use reflection_probe::*;
pub use sprite::*;
pub use text::*;
pub use tilemap::*;

gizmo_core::impl_component!(
    Mesh,
    Material,
    MaterialDesc,
    Skeleton,
    MeshRenderer,
    Camera,
    PointLight,
    Terrain,
    DirectionalLight,
    SpotLight,
    RectLight,
    LodGroup,
    LodLevel,
    ParticleEmitter,
    EditorRenderTarget,
    GameRenderTarget,
    RenderStats,
    Decal,
    PostProcess,
    ReflectionProbe,
    Sprite,
    SpriteAnimation,
    Text,
    Tilemap
);
gizmo_core::impl_component!(FluidParticle, FluidHandle, FluidPhase, FluidInteractor);
