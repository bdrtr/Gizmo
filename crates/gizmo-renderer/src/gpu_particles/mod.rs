/// The particle system's compute and render pipelines.
pub mod pipeline;
/// The system itself: its buffer, its simulation pass and its draw.
pub mod system;
/// The structs the particle shaders read.
pub mod types;

pub use system::{explosion_particles, GpuParticleSystem};
pub use types::*;
