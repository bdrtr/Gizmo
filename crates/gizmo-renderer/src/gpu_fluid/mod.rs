#[cfg(test)]
mod fluid_tests;
/// The fluid solver's compute and render pipelines.
pub mod pipeline;
/// The solver itself: its buffers, its passes and its screen-space surface.
pub mod system;
/// The water itself, saved with a scene.
pub mod state;
/// The structs the fluid shaders read — particles, hashes, colliders and parameters.
pub mod types;

pub use state::FluidState;
pub use system::GpuFluidSystem;
pub use types::*;
