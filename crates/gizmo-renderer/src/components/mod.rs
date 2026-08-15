pub mod animation;
pub mod camera;
pub mod decal;
pub mod light;
pub mod material;
pub mod mesh;
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
