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
use gizmo_physics_core::components::{GlobalTransform, Transform};

/// Point + spot + sun lights collected from the world for one frame, ready to be
/// dropped into `SceneUniforms`.
pub struct SceneLights {
    /// Up to 10 point/spot lights (the shader's fixed light array).
    pub lights: [LightData; 10],
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

    let mut lights = [LightData {
        position: [0.0; 4],
        color: [0.0; 4],
        direction: [0.0, -1.0, 0.0, 0.0],
        params: [0.0; 4],
    }; 10];
    let mut num_lights = 0usize;

    if let Some(q) = world.query::<&PointLight>() {
        for (e, light) in q.iter() {
            if num_lights >= 10 {
                break;
            }
            let Some((pos, _)) = world_tf(e) else { continue };
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
            if num_lights >= 10 {
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
    }
}
