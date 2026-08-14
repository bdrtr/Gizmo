//! Per-frame render *setup* shared between the two render paths.
//!
//! The engine has two renderers: the game's DEFERRED path (`default_render_pass`
//! → `passes.rs`, full G-buffer + SSAO/SSR/SSGI/TAA) and the studio's FORWARD
//! editor path (`gizmo-studio::execute_render_pipeline`, plus grid/gizmo/collider
//! overlays). The passes genuinely differ and stay separate, but the per-frame
//! *setup* that feeds them — light collection, shadow cascades, batching and
//! frustum culling — is the same, and it used to be copy-pasted between the two
//! files. Every fix to that setup then had to be applied twice, and whenever it
//! wasn't the two renderers silently diverged (the "derive cascade splits from
//! the camera" and "cull shadow casters against the light frustum, not the camera
//! frustum" fixes both had to be duplicated). This module single-sources it.

use crate::core::World;
use crate::math::{Vec3, Vec4};
use crate::renderer::components::{DirectionalLight, LightRole, PointLight, SpotLight};
use crate::renderer::gpu_types::LightData;
use crate::renderer::MAX_LIGHTS;
use gizmo_physics_core::components::{GlobalTransform, Transform};

/// Point + spot + sun lights collected from the world for one frame, ready to be
/// dropped into `SceneUniforms`.
pub struct SceneLights {
    /// Up to [`MAX_LIGHTS`] point/spot lights (the shader's fixed light array).
    pub lights: [LightData; MAX_LIGHTS],
    pub num_lights: u32,
    /// Direction the sun points along (normalized). Default down-vector when the
    /// scene has no `LightRole::Sun`.
    pub sun_dir: Vec3,
    /// Sun colour in rgb, intensity in w. `w == 0` means "no sun" — the deferred
    /// lighting shader keys off this exactly like the old inline code did.
    pub sun_col: Vec4,
    /// Whether the scene actually contains a `LightRole::Sun`. The studio forward
    /// shader signals "sun present" through `sun_direction.w` (1.0 vs 0.0); this
    /// carries that bit so the studio path stays behaviourally identical.
    pub has_sun: bool,
    /// Index into `lights` of the point light that owns the single point-shadow cube,
    /// or `-1` when there is no point light. There is only one point-shadow cubemap, so
    /// exactly one point light casts; the caller renders that light's cube and the shader
    /// only samples it for this index (avoids applying one cube to every point light).
    pub shadow_point_index: i32,
}

/// Collect the scene's dynamic lights (point + spot, capped at 10) and the sun.
///
/// Each light's world transform prefers a synced `GlobalTransform` (so a parented
/// light follows its parent, matching how meshes are placed) and falls back to the
/// light's own `Transform` when it has none — the same robustness the camera path
/// uses. Previously the game path queried `(&Light, &GlobalTransform)` (dropping
/// any light without a global) while the studio path read the raw `Transform`
/// (ignoring parenting); this unifies both onto the correct-and-robust rule.
pub fn collect_scene_lights(world: &World) -> SceneLights {
    let globals = world.borrow::<GlobalTransform>();
    let locals = world.borrow::<Transform>();

    // (position, rotation) in world space, GlobalTransform-preferred, Transform-fallback.
    let world_tf = |e| {
        globals
            .get(e)
            .map(|g| {
                let (_, rot, pos) = g.matrix.to_scale_rotation_translation();
                (pos, rot)
            })
            .or_else(|| locals.get(e).map(|t| (t.position, t.rotation)))
    };

    let mut lights = [LightData::default(); MAX_LIGHTS];
    let mut num_lights = 0usize;
    // The first collected point light owns the single point-shadow cube.
    let mut shadow_point_index: i32 = -1;

    if let Some(q) = world.query::<&PointLight>() {
        for (e, light) in q.iter() {
            if num_lights >= MAX_LIGHTS {
                break;
            }
            let Some((pos, _)) = world_tf(e) else { continue };
            if shadow_point_index < 0 {
                shadow_point_index = num_lights as i32;
            }
            lights[num_lights] = LightData {
                position: [pos.x, pos.y, pos.z, light.intensity],
                color: [light.color.x, light.color.y, light.color.z, light.radius],
                direction: [0.0, -1.0, 0.0, 0.0],
                params: [0.0, 0.0, 0.0, 0.0], // params.y = 0 → PointLight
            };
            num_lights += 1;
        }
    }

    if let Some(q) = world.query::<&SpotLight>() {
        for (e, light) in q.iter() {
            if num_lights >= MAX_LIGHTS {
                break;
            }
            let Some((pos, rot)) = world_tf(e) else { continue };
            let dir = rot.mul_vec3(Vec3::new(0.0, 0.0, -1.0)).normalize();
            // The shaders compare the cone against `dot(-L, spot_dir)` (a cosine), so the
            // cutoffs must be COSINES of the cone angles — every lighting shader documents
            // `w = inner_cutoff_cos`, `params.x = outer_cutoff_cos`. `SpotLight` stores the
            // angles in radians (its ctor clamps inner ≤ outer), so convert here. Passing the
            // raw radians made the cone a hard cut at the wrong angle with no falloff; the
            // studio path used to `.cos()` these itself, the game path never did (its spots
            // were broken) — single-sourcing the fix corrects both.
            lights[num_lights] = LightData {
                position: [pos.x, pos.y, pos.z, light.intensity],
                color: [light.color.x, light.color.y, light.color.z, light.radius],
                direction: [dir.x, dir.y, dir.z, light.inner_angle.cos()],
                params: [light.outer_angle.cos(), 1.0, 0.0, 0.0], // params.y = 1 → SpotLight
            };
            num_lights += 1;
        }
    }

    let mut sun_dir = Vec3::new(0.0, -1.0, 0.0);
    let mut sun_col = Vec4::new(0.0, 0.0, 0.0, 0.0); // w = 0 → no sun
    let mut has_sun = false;
    if let Some(q) = world.query::<&DirectionalLight>() {
        for (e, light) in q.iter() {
            if light.role == LightRole::Sun {
                if let Some((_, rot)) = world_tf(e) {
                    // Light convention: points along its local -Z.
                    sun_dir = rot.mul_vec3(Vec3::new(0.0, 0.0, -1.0)).normalize();
                    sun_col = Vec4::new(light.color.x, light.color.y, light.color.z, light.intensity);
                    has_sun = true;
                }
                break; // first sun wins
            }
        }
    }

    SceneLights {
        lights,
        num_lights: num_lights as u32,
        sun_dir,
        sun_col,
        has_sun,
        shadow_point_index,
    }
}

/// Which light the directional cascades are fitted to. The two render paths answer differently,
/// and this is the whole of the difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowCaster {
    /// The sun, and nothing else — the game's rule. A scene with no sun still gets cascades
    /// (fitted to the collector's default down-vector) but `SunFrame::present` is false, so the
    /// shader never samples them.
    SunOnly,
    /// The sun if there is one, otherwise the first collected light, aimed at the origin — the
    /// editor's rule. A scene being lit by hand often has no sun yet, and a viewport with no
    /// shadows at all reads as broken rather than as unlit.
    ///
    /// "First collected light" is what the code does; the comment it replaced said "first point
    /// light", which is the same thing only while the scene has one — [`collect_scene_lights`]
    /// fills points before spots, so a scene of nothing but spotlights casts from a spot.
    SunOrFirstLight,
}

/// What the two render paths derive from the world for one frame.
pub struct SceneSetup {
    /// Ready for `SceneUniforms::new`.
    pub frame: crate::renderer::SceneFrame,
    /// The cascade matrices the frame carries, kept as `Mat4` because the shadow pass writes them
    /// to its own per-cascade uniform buffers.
    pub cascade_view_projs: [gizmo_math::Mat4; 4],
    /// The collected lights. The caller needs `shadow_point_index` (and that light's position and
    /// radius) to decide whether to render the point-shadow cube and where to put it.
    pub lights: SceneLights,
}

/// The per-frame inputs that are *not* the world: the camera, and the handful of decisions each
/// render path makes for itself.
pub struct SceneSetupInputs {
    pub camera: crate::renderer::CameraFrame,
    /// Viewport aspect and the camera's vertical FOV — the cascade fit needs the frustum, and
    /// [`crate::renderer::CameraFrame`] carries the matrix rather than the angles it came from.
    pub aspect: f32,
    pub cam_fov: f32,
    pub shadow_caster: ShadowCaster,
    pub environment: crate::renderer::EnvironmentFrame,
    /// Whether this path renders the point-shadow cube. A path that does not must leave it off:
    /// the lookup would sample whatever the cube held on the last frame that did.
    pub point_shadows_enabled: bool,
    pub elapsed_time: f32,
}

/// Build one frame's scene state, for either render path.
///
/// # Why this is one function
///
/// Light collection and the cascade orchestration were single-sourced here after each had been
/// fixed twice; what sat *around* them — the sun-present flag, which light the cascades follow,
/// the identity-matrix fallback, the point-shadow caster index — was still written out twice, in
/// two files, by whoever last touched one of them. That is the same shape of bug in a smaller
/// package: nothing forces the two to be compared, so they diverge in the direction of whichever
/// path someone was debugging.
///
/// Now the only difference between the two callers is the arguments they pass, which is a thing a
/// test can hold side by side — see `gizmo-studio/tests/render_parity.rs`. What deliberately stays
/// out of here is everything downstream: the deferred G-buffer recorder and the editor's forward
/// recorder genuinely differ, and merging those is not the goal.
pub fn collect_scene_setup(world: &World, inputs: &SceneSetupInputs) -> SceneSetup {
    use gizmo_math::Mat4;

    let lights = collect_scene_lights(world);

    let shadow_dir = match inputs.shadow_caster {
        // Always fits, sun or not. `sun_dir` is the collector's down-vector default when the
        // scene has no sun, and the cascades built from it are simply never sampled.
        ShadowCaster::SunOnly => Some(lights.sun_dir),
        ShadowCaster::SunOrFirstLight => {
            if lights.has_sun {
                Some(lights.sun_dir)
            } else if lights.num_lights > 0 {
                let p = lights.lights[0].position;
                Some((Vec3::ZERO - Vec3::new(p[0], p[1], p[2])).normalize())
            } else {
                None
            }
        }
    };

    let cascades = crate::renderer::compute_directional_cascades(
        inputs.camera.position,
        inputs.camera.forward,
        inputs.aspect,
        inputs.cam_fov,
        inputs.camera.near,
        inputs.camera.far,
        shadow_dir.unwrap_or(Vec3::new(0.0, -1.0, 0.0)),
    );
    // Nothing to cast from → identity, and the splits stay meaningful for the shadow-distance
    // fade. Only the editor's policy can reach this; the game's always has a direction.
    let cascade_view_projs =
        if shadow_dir.is_some() { cascades.view_projs } else { [Mat4::IDENTITY; 4] };

    SceneSetup {
        frame: crate::renderer::SceneFrame {
            camera: inputs.camera,
            sun: crate::renderer::SunFrame {
                direction: lights.sun_dir,
                color: [
                    lights.sun_col.x,
                    lights.sun_col.y,
                    lights.sun_col.z,
                    lights.sun_col.w,
                ],
                present: lights.has_sun,
            },
            lights: lights.lights,
            num_lights: lights.num_lights,
            shadows: crate::renderer::ShadowFrame {
                cascade_view_projs,
                cascade_splits: cascades.splits,
                // Inert unless `point_shadows_enabled` is also set — the shader tests both — so
                // both paths can carry the same index and only the flag decides.
                point_caster: u32::try_from(lights.shadow_point_index).ok(),
                point_shadows_enabled: inputs.point_shadows_enabled,
            },
            environment: inputs.environment,
            elapsed_time: inputs.elapsed_time,
        },
        cascade_view_projs,
        lights,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::World;
    use crate::renderer::components::{PointLight, SpotLight};
    use gizmo_physics_core::components::GlobalTransform;

    // Regression: the shaders compare the spotlight cone against `dot(-L, spot_dir)`
    // (a cosine) and every lighting shader documents the cutoffs as cosines, but
    // `SpotLight` stores the cone half-angles in radians. The game render path fed
    // the raw radians (broken cone), and unifying light collection briefly spread
    // that to the studio too; collection must convert the angles to cosines.
    #[test]
    fn spotlight_cutoffs_are_stored_as_cosines() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, GlobalTransform::default());
        // inner_angle = 0.4 rad, outer_angle = 0.6 rad (radians, ctor clamps inner ≤ outer).
        world.add_component(e, SpotLight::new(Vec3::ONE, 10.0, 30.0, 0.4, 0.6));

        let l = collect_scene_lights(&world);
        assert_eq!(l.num_lights, 1);
        let spot = l.lights[0];
        assert_eq!(spot.params[1], 1.0, "params.y == 1 marks a spot light");
        assert!(
            (spot.direction[3] - 0.4_f32.cos()).abs() < 1e-5,
            "inner cutoff must be cos(inner_angle), got {}",
            spot.direction[3]
        );
        assert!(
            (spot.params[0] - 0.6_f32.cos()).abs() < 1e-5,
            "outer cutoff must be cos(outer_angle), got {}",
            spot.params[0]
        );
        // Tighter inner cone → larger cosine, so the falloff (inner - outer) is positive.
        assert!(spot.direction[3] > spot.params[0]);
    }

    // Point lights come before spot lights, and a light with only a `Transform`
    // (no synced `GlobalTransform`) is still collected via the fallback.
    #[test]
    fn point_before_spot_and_transform_fallback() {
        let mut world = World::new();
        // A point light carrying a GlobalTransform (also registers the component).
        let p = world.spawn();
        world.add_component(p, GlobalTransform::default());
        world.add_component(p, PointLight::new(Vec3::ONE, 5.0, 12.0));
        // A spot light with ONLY a Transform → must resolve via the Transform fallback.
        let s = world.spawn();
        world.add_component(s, Transform::new(Vec3::new(1.0, 2.0, 3.0)));
        world.add_component(s, SpotLight::new(Vec3::ONE, 7.0, 20.0, 0.3, 0.5));

        let l = collect_scene_lights(&world);
        assert_eq!(l.num_lights, 2);
        assert_eq!(l.lights[0].params[1], 0.0, "point light packed first");
        assert_eq!(l.lights[1].params[1], 1.0, "spot light packed second");
        // Spot position came from its Transform (GlobalTransform-less) fallback.
        assert_eq!(l.lights[1].position, [1.0, 2.0, 3.0, 7.0]);
        // The point light (index 0) owns the single point-shadow cube.
        assert_eq!(l.shadow_point_index, 0, "first point light is the shadow caster");
    }

    // With no point light there is no point-shadow caster: the index must be -1 so the
    // shader (which reads caster_index + 1) sees 0 = "no point shadow this frame" and the
    // caller skips rendering the cube.
    #[test]
    fn no_point_light_has_no_shadow_caster() {
        let mut world = World::new();
        let s = world.spawn();
        world.add_component(s, GlobalTransform::default());
        world.add_component(s, SpotLight::new(Vec3::ONE, 7.0, 20.0, 0.3, 0.5));

        let l = collect_scene_lights(&world);
        assert_eq!(l.num_lights, 1);
        assert_eq!(l.shadow_point_index, -1, "no point light → no point-shadow caster");
    }
}
