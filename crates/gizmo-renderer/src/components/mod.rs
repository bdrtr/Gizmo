pub mod animation;
pub mod camera;
pub mod light;
pub mod material;
pub mod mesh;
pub mod misc;
pub mod sprite;

pub use animation::*;
pub use camera::*;
pub use light::*;
pub use material::*;
pub use mesh::*;
pub use misc::*;
pub use sprite::*;

gizmo_core::impl_component!(
    Mesh,
    Material,
    Skeleton,
    AnimationPlayer,
    AnimationStateMachine,
    MeshRenderer,
    Camera,
    Sprite,
    Camera2D,
    PointLight,
    Terrain,
    DirectionalLight,
    SpotLight,
    LodGroup,
    LodLevel,
    ParticleEmitter,
    EditorRenderTarget,
    GameRenderTarget
);
gizmo_core::impl_component!(FluidParticle, FluidHandle, FluidPhase, FluidInteractor);
