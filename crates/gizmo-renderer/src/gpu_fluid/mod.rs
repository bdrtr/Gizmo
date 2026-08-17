#[cfg(test)]
mod fluid_tests;
/// The fluid solver's compute and render pipelines.
pub mod pipeline;
/// The solver itself: its buffers, its passes and its screen-space surface.
pub mod system;
/// The structs the fluid shaders read — particles, hashes, colliders and parameters.
pub mod types;

pub use system::GpuFluidSystem;
pub use types::*;
