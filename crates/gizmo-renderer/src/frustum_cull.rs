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
/// (`!is_transparent`, `albedo_alpha >= 0.99`) and lit (not `Unlit`/`Skybox`/`Backdrop`/`Grid`).
///
/// **`model` must be the transform the geometry is actually DRAWN with.** For a camera-locked
/// material that is not the authored matrix — put it through
/// [`camera_locked_model`](crate::backdrop::camera_locked_model) first, or this culls a
/// backdrop that is on screen.
pub fn classify_visibility(
    camera_frustum: &Frustum,
    cascade_frusta: &[Frustum],
    model: &Mat4,
    local_aabb: Aabb,
    material_type: crate::components::MaterialType,
    is_transparent: bool,
    albedo_alpha: f32,
) -> Visibility {
    classify_visibility_world(
        camera_frustum,
        cascade_frusta,
        local_aabb.transform(model),
        material_type,
        is_transparent,
        albedo_alpha,
    )
}

/// [`classify_visibility`] for a caller that already holds the **world-space** box.
///
/// Same decision, same caster predicate, same result — the difference is arithmetic that is
/// not repeated. [`classify_visibility`] used to reach `visible_in_frustum` once per frustum,
/// and each of those re-ran [`Aabb::transform`] on the local box: with one camera and four
/// shadow cascades that is 5 Arvo transforms of the same box to produce the same 5 answers.
/// For 8 192 meshes it was ~40 600 `Aabb::transform` calls a frame where 8 192 would do.
///
/// It also matters for correctness of anything built on top: a spatial index stores a
/// world-space box, and passing that same value here means the index's box and the exact
/// test's box are literally the same `Aabb`, so "the index says skip ⇒ the exact test would
/// have culled" is a statement about one number rather than two independently rounded ones.
///
/// **`world_aabb` must be the box of the transform the geometry is actually DRAWN with.** For
/// a camera-locked material that is not the authored matrix — see
/// [`camera_locked_model`](crate::backdrop::camera_locked_model).
pub fn classify_visibility_world(
    camera_frustum: &Frustum,
    cascade_frusta: &[Frustum],
    world_aabb: Aabb,
    material_type: crate::components::MaterialType,
    is_transparent: bool,
    albedo_alpha: f32,
) -> Visibility {
    if camera_frustum.intersects_aabb(world_aabb) {
        return Visibility::Camera;
    }
    let is_caster = !is_transparent
        && albedo_alpha >= 0.99
        && !matches!(
            material_type,
            crate::components::MaterialType::Unlit
                | crate::components::MaterialType::Skybox
                // A backdrop is scenery painted at infinity. Casting from it would drop the
                // whole world into its shadow. True whether it follows the camera or stays
                // where it was authored — what disqualifies it is being a painting, not where
                // the painting hangs.
                | crate::components::MaterialType::Backdrop
                | crate::components::MaterialType::BackdropPlaced
                | crate::components::MaterialType::Grid
                // Water draws in a forward pass with its own pipeline and is skipped by every
                // shadow pass in both hosts, so an off-screen water surface kept as a caster is
                // an instance uploaded and a range drawn for a shadow map nothing writes.
                | crate::components::MaterialType::Water
        );
    if is_caster
        && cascade_frusta
            .iter()
            .any(|f| f.intersects_aabb(world_aabb))
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
        // Water is NOT — it was until 2026-08-24, when it was still routed as PBR and this line
        // asserted `ShadowOnly` with the comment "water is lit too". It draws in a forward pass
        // now and every shadow pass in both hosts skips it, so keeping an off-screen water
        // surface would upload an instance for a shadow map nothing writes.
        assert_eq!(
            classify_visibility(&cam, &[cascade], &behind, unit_aabb(), MaterialType::Water, false, 1.0),
            Visibility::Culled
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
        // Unlit / Skybox / Backdrop / Grid materials never cast.
        for mt in [
            MaterialType::Unlit,
            MaterialType::Skybox,
            MaterialType::Backdrop,
            MaterialType::Grid,
        ] {
            assert_eq!(
                classify_visibility(&cam, &[cascade], &behind, unit_aabb(), mt, false, 1.0),
                Visibility::Culled,
                "{mt:?} must not be a shadow caster"
            );
        }
    }

    // A camera-locked backdrop is authored around the origin but DRAWN around the camera. Cull
    // it with the authored matrix and it disappears the moment the player drives away from the
    // middle of the map — the geometry fills the screen and the frustum test says it is gone.
    // `camera_locked_model` is the fix, and it is the caller's job to apply it, so this pins
    // both halves: the raw matrix culls, the locked one does not.
    #[test]
    fn a_camera_locked_backdrop_must_be_culled_against_its_locked_transform() {
        let eye = Vec3::new(0.0, 0.0, 905.0);
        let cam = cam_frustum(eye);
        // The backdrop dome, authored 10 units in front of the origin.
        let authored = Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0));
        let bounds = Aabb::new(Vec3::splat(-2.0), Vec3::splat(2.0));

        // Raw: the camera is 915 units away from where the matrix says it is → culled.
        assert_eq!(
            classify_visibility(&cam, &[], &authored, bounds, MaterialType::Backdrop, false, 1.0),
            Visibility::Culled,
            "premise: the authored transform really is outside this camera's frustum"
        );

        // Locked: the transform the vertex shader actually uses → on screen.
        let drawn = crate::backdrop::camera_locked_model(MaterialType::Backdrop, &authored, eye);
        assert_eq!(
            classify_visibility(&cam, &[], &drawn, bounds, MaterialType::Backdrop, false, 1.0),
            Visibility::Camera,
            "a camera-locked backdrop is always in front of the camera; culling it away is \
             how 191 backdrop meshes never reach the screen"
        );
    }

    /// The world-space entry point is not an approximation of the local-space one — it is the
    /// same decision with the transform hoisted out of the loop. Anything that culls through a
    /// spatial index goes through `classify_visibility_world`, so a divergence here would show
    /// up as geometry that appears or disappears depending on which path a frame took.
    ///
    /// **Read this as a tripwire, not as evidence.** `classify_visibility` currently *delegates*
    /// to `classify_visibility_world`, so as written the two cannot disagree and this test
    /// cannot fail. It earns its place only if someone re-inlines the local-space body — which
    /// is exactly the change that would reintroduce the divergence. Do not cite it as proof
    /// that the classification is right; the thing it pins is that there is one of it.
    #[test]
    fn the_world_space_entry_point_decides_exactly_what_the_local_space_one_does() {
        let cam = cam_frustum(Vec3::new(0.0, 0.0, 5.0));
        let cascades = [
            cam_frustum(Vec3::new(0.0, 0.0, 60.0)),
            cam_frustum(Vec3::new(300.0, 0.0, 5.0)),
        ];
        let mats = [
            MaterialType::Pbr,
            MaterialType::Unlit,
            MaterialType::Water,
            MaterialType::Backdrop,
            MaterialType::Skybox,
            MaterialType::Grid,
            MaterialType::BakedLit,
        ];
        // A spread of placements: on screen, behind, off to the side, in a cascade, and
        // scaled/rotated so `Aabb::transform`'s Arvo path is genuinely exercised.
        let models = [
            Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0)),
            Mat4::from_translation(Vec3::new(0.0, 0.0, 50.0)),
            Mat4::from_translation(Vec3::new(305.0, 0.0, 0.0)),
            Mat4::from_scale_rotation_translation(
                Vec3::new(3.0, 0.5, 2.0),
                gizmo_math::Quat::from_rotation_y(0.7),
                Vec3::new(0.0, 0.0, 55.0),
            ),
            Mat4::from_scale_rotation_translation(
                Vec3::splat(40.0),
                gizmo_math::Quat::from_rotation_x(1.1),
                Vec3::ZERO,
            ),
        ];
        for m in &models {
            for mt in mats {
                for transparent in [false, true] {
                    for alpha in [1.0f32, 0.99, 0.5] {
                        let local = classify_visibility(
                            &cam, &cascades, m, unit_aabb(), mt, transparent, alpha,
                        );
                        let world = classify_visibility_world(
                            &cam,
                            &cascades,
                            unit_aabb().transform(m),
                            mt,
                            transparent,
                            alpha,
                        );
                        assert_eq!(local, world, "{mt:?} transparent={transparent} alpha={alpha}");
                    }
                }
            }
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
