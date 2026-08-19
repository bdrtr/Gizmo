// Each system module is gated on the dependencies it actually uses. `Transform` lives in
// `gizmo-physics-core`, so every transform-touching system needs the `physics` feature —
// that is not an accident of this file, it is where the type is defined.
#[cfg(all(feature = "audio", feature = "render", feature = "physics"))]
/// Spatial audio: the listener follows the camera and emitters follow their entities.
pub mod audio;
#[cfg(feature = "physics")]
/// Deriving a collider from an entity's visual scale, so the physics and the mesh cannot drift
/// apart.
pub mod auto_collider;
/// World streaming by chunk: which square of the world is loaded around the player.
pub mod chunk_system;
#[cfg(all(feature = "render", feature = "physics"))]
/// Feeding the scene's colliders into the GPU fluid simulation.
pub mod fluid;
#[cfg(feature = "render")]
pub mod fps_look;
/// Entities that despawn themselves after a set time — particles, decals, debris.
pub mod lifetime;
/// The bridge between the ECS and the physics world: stepping it, and drawing its debug view.
#[cfg(feature = "physics")]
pub mod physics;
/// One frame of a running game — shared by the editor's Play mode and an exported game, which is
/// what makes "the export behaves like Play" a fact rather than a promise.
#[cfg(feature = "physics")]
pub mod play;
/// The facade's render path: gathering the scene, batching it and recording the frame.
#[cfg(feature = "render")]
pub mod render;
/// A tiny demo system that spins whatever carries its component — used in examples and tests.
pub mod spin;
#[cfg(all(feature = "render", feature = "physics"))]
/// Texture and mesh streaming: what is resident, and what is loaded as the camera approaches.
pub mod streaming;
#[cfg(all(feature = "render", feature = "physics"))]
/// Turning a `Terrain` recipe into the `Mesh` a draw path can draw — on load and in an exported
/// game, not only when an editor slider moves.
pub mod terrain;
/// Transform propagation — turning a hierarchy of local transforms into world matrices.
pub mod transform;

#[cfg(all(feature = "audio", feature = "render", feature = "physics"))]
pub use audio::*;
#[cfg(feature = "physics")]
pub use auto_collider::*;
pub use chunk_system::*;
#[cfg(all(feature = "render", feature = "physics"))]
pub use fluid::*;
#[cfg(feature = "render")]
pub use fps_look::*;
pub use lifetime::*;
#[cfg(feature = "physics")]
pub use physics::*;
#[cfg(feature = "physics")]
pub use play::{PlayLoop, PlayReport};
#[cfg(feature = "render")]
pub use render::*;
pub use spin::*;
#[cfg(all(feature = "render", feature = "physics"))]
pub use streaming::*;
#[cfg(all(feature = "render", feature = "physics"))]
pub use terrain::*;
pub use transform::*;
