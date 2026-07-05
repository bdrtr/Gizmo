//! CPU-side frustum culling before filling the instance buffer.
//!
//! Extract six planes from the view–projection matrix and test each instance’s world-space AABB
//! (`Mesh::bounds` transformed by the instance model matrix). Skipping invisible instances reduces
//! work on the GPU when combined with instanced `draw(..., start..end)` batching.

pub use gizmo_math::{Aabb, Frustum, Mat4};

/// Returns `true` if the world AABB of `local_aabb` after `model_matrix` intersects `frustum`.
#[inline]
pub fn visible_in_frustum(frustum: &Frustum, model_matrix: &Mat4, local_aabb: Aabb) -> bool {
    frustum.intersects_aabb(local_aabb.transform(model_matrix))
}

/// Where one object lands relative to the camera + shadow cascades for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// Inside the camera frustum — draw it in the main passes (and shadow maps).
    Camera,
    /// Outside the camera frustum but a shadow caster inside a cascade's light
    /// frustum — draw into the shadow maps only, so it still casts a shadow into view.
    ShadowOnly,
    /// Neither visible nor a relevant caster — skip entirely.
    Culled,
}

/// Single-source the per-object visibility decision the game (deferred) and studio
/// (forward) render paths both make while batching. Both used to inline this with
/// subtly different tests — the game culled against a bounding *sphere*, the studio
/// against the *AABB*, and the "is this a shadow caster" predicate differed — so a
/// fix to one silently missed the other. This uses the tighter AABB test for the
/// camera and every cascade, and one caster predicate: a caster is opaque
/// (`!is_transparent`, `albedo_alpha >= 0.99`) and lit (not `Unlit`/`Skybox`/`Grid`).
pub fn classify_visibility(
    camera_frustum: &Frustum,
    cascade_frusta: &[Frustum],
    model: &Mat4,
    local_aabb: Aabb,
    material_type: crate::components::MaterialType,
    is_transparent: bool,
    albedo_alpha: f32,
) -> Visibility {
    if visible_in_frustum(camera_frustum, model, local_aabb) {
        return Visibility::Camera;
    }
    let is_caster = !is_transparent
        && albedo_alpha >= 0.99
        && !matches!(
            material_type,
            crate::components::MaterialType::Unlit
                | crate::components::MaterialType::Skybox
                | crate::components::MaterialType::Grid
        );
    if is_caster
        && cascade_frusta
            .iter()
            .any(|f| visible_in_frustum(f, model, local_aabb))
    {
        Visibility::ShadowOnly
    } else {
        Visibility::Culled
    }
}
