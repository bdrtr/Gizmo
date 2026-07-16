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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::MaterialType;
    use gizmo_math::Vec3;

    // A perspective camera at `eye` looking down −Z (matches the studio/game
    // convention). Pure CPU: extracts Gribb–Hartmann planes from proj·view.
    fn cam_frustum(eye: Vec3) -> Frustum {
        let view = Mat4::look_at_rh(eye, eye + Vec3::new(0.0, 0.0, -1.0), Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 100.0);
        Frustum::from_matrix(&(proj * view))
    }

    fn unit_aabb() -> Aabb {
        Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5))
    }

    #[test]
    fn visible_in_frustum_tracks_the_transformed_aabb() {
        let f = cam_frustum(Vec3::new(0.0, 0.0, 5.0));
        // In front of the camera (−Z) → visible.
        let front = Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0));
        assert!(visible_in_frustum(&f, &front, unit_aabb()));
        // Behind the camera → culled.
        let behind = Mat4::from_translation(Vec3::new(0.0, 0.0, 50.0));
        assert!(!visible_in_frustum(&f, &behind, unit_aabb()));
        // Far to the side → culled.
        let side = Mat4::from_translation(Vec3::new(100.0, 0.0, -10.0));
        assert!(!visible_in_frustum(&f, &side, unit_aabb()));
    }

    #[test]
    fn in_camera_frustum_classifies_as_camera_regardless_of_material() {
        let cam = cam_frustum(Vec3::new(0.0, 0.0, 5.0));
        let front = Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0));
        // Camera visibility short-circuits before the caster predicate: even a
        // transparent unlit object in view is drawn in the main pass.
        assert_eq!(
            classify_visibility(&cam, &[], &front, unit_aabb(), MaterialType::Unlit, true, 0.1),
            Visibility::Camera
        );
    }

    #[test]
    fn opaque_lit_caster_outside_view_but_in_cascade_is_shadow_only() {
        let cam = cam_frustum(Vec3::new(0.0, 0.0, 5.0));
        let cascade = cam_frustum(Vec3::new(0.0, 0.0, 60.0));
        let behind = Mat4::from_translation(Vec3::new(0.0, 0.0, 50.0));
        // Preconditions: outside the camera frustum, inside the cascade frustum.
        assert!(!visible_in_frustum(&cam, &behind, unit_aabb()));
        assert!(visible_in_frustum(&cascade, &behind, unit_aabb()));

        assert_eq!(
            classify_visibility(&cam, &[cascade], &behind, unit_aabb(), MaterialType::Pbr, false, 1.0),
            Visibility::ShadowOnly
        );
        // Water is lit too → also a caster.
        assert_eq!(
            classify_visibility(&cam, &[cascade], &behind, unit_aabb(), MaterialType::Water, false, 1.0),
            Visibility::ShadowOnly
        );
    }

    #[test]
    fn caster_with_no_cascade_containing_it_is_culled() {
        let cam = cam_frustum(Vec3::new(0.0, 0.0, 5.0));
        let behind = Mat4::from_translation(Vec3::new(0.0, 0.0, 50.0));
        // No cascades at all.
        assert_eq!(
            classify_visibility(&cam, &[], &behind, unit_aabb(), MaterialType::Pbr, false, 1.0),
            Visibility::Culled
        );
        // A cascade that does not contain the object.
        let far_cascade = cam_frustum(Vec3::new(500.0, 0.0, 5.0));
        assert_eq!(
            classify_visibility(&cam, &[far_cascade], &behind, unit_aabb(), MaterialType::Pbr, false, 1.0),
            Visibility::Culled
        );
    }

    #[test]
    fn transparent_or_faded_or_unlit_objects_are_not_casters() {
        let cam = cam_frustum(Vec3::new(0.0, 0.0, 5.0));
        let cascade = cam_frustum(Vec3::new(0.0, 0.0, 60.0));
        let behind = Mat4::from_translation(Vec3::new(0.0, 0.0, 50.0));

        // Transparent → not a caster.
        assert_eq!(
            classify_visibility(&cam, &[cascade], &behind, unit_aabb(), MaterialType::Pbr, true, 1.0),
            Visibility::Culled
        );
        // Faded (albedo alpha below the 0.99 opacity gate) → not a caster.
        assert_eq!(
            classify_visibility(&cam, &[cascade], &behind, unit_aabb(), MaterialType::Pbr, false, 0.5),
            Visibility::Culled
        );
        // Unlit / Skybox / Grid materials never cast.
        for mt in [MaterialType::Unlit, MaterialType::Skybox, MaterialType::Grid] {
            assert_eq!(
                classify_visibility(&cam, &[cascade], &behind, unit_aabb(), mt, false, 1.0),
                Visibility::Culled,
                "{mt:?} must not be a shadow caster"
            );
        }
    }

    #[test]
    fn caster_opacity_gate_boundary_is_inclusive_at_0_99() {
        let cam = cam_frustum(Vec3::new(0.0, 0.0, 5.0));
        let cascade = cam_frustum(Vec3::new(0.0, 0.0, 60.0));
        let behind = Mat4::from_translation(Vec3::new(0.0, 0.0, 50.0));
        // Exactly at the gate (`>= 0.99`) → still a caster.
        assert_eq!(
            classify_visibility(&cam, &[cascade], &behind, unit_aabb(), MaterialType::Pbr, false, 0.99),
            Visibility::ShadowOnly
        );
        // Just below → culled.
        assert_eq!(
            classify_visibility(&cam, &[cascade], &behind, unit_aabb(), MaterialType::Pbr, false, 0.98),
            Visibility::Culled
        );
    }
}
