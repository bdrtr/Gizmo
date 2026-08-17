/// GPU FEM soft bodies — tetrahedral meshes solved on the GPU.
pub mod fem;
/// The compute and render pipelines the solver runs.
pub mod pipeline;
/// The solver itself: its buffers, its passes and its readback.
pub mod system;
/// The structs the physics shaders read — bodies, colliders, joints and contacts.
pub mod types;

#[cfg(test)]
mod fem_tests;

pub use fem::*;
pub use system::GpuPhysicsSystem;
pub use types::*;
