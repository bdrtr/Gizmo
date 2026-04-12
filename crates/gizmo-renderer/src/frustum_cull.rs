//! CPU-side frustum culling before filling the instance buffer.
//!
//! Extract six planes from the view–projection matrix and test each instance’s world-space AABB
//! (`Mesh::bounds` transformed by the instance model matrix). Skipping invisible instances reduces
//! work on the GPU when combined with instanced `draw(..., start..end)` batching.

pub use gizmo_math::{Aabb, Frustum, Mat4};

/// Returns `true` if the world AABB of `local_aabb` after `model_matrix` intersects `frustum`.
#[inline]
pub fn visible_in_frustum(frustum: &Frustum, model_matrix: &Mat4, local_aabb: &Aabb) -> bool {
    frustum.contains_aabb(&local_aabb.transform(model_matrix))
}
