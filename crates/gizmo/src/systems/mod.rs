// Each system module is gated on the dependencies it actually uses. `Transform` lives in
// `gizmo-physics-core`, so every transform-touching system needs the `physics` feature —
// that is not an accident of this file, it is where the type is defined.
#[cfg(all(feature = "audio", feature = "render", feature = "physics"))]
pub mod audio;
#[cfg(feature = "physics")]
pub mod auto_collider;
pub mod chunk_system;
#[cfg(all(feature = "render", feature = "physics"))]
pub mod fluid;
#[cfg(feature = "render")]
pub mod fps_look;
pub mod lifetime;
#[cfg(feature = "physics")]
pub mod physics;
#[cfg(feature = "render")]
pub mod render;
pub mod spin;
#[cfg(all(feature = "render", feature = "physics"))]
pub mod streaming;
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
#[cfg(feature = "render")]
pub use render::*;
pub use spin::*;
#[cfg(all(feature = "render", feature = "physics"))]
pub use streaming::*;
pub use transform::*;
