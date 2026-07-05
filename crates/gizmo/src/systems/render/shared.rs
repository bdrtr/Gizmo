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
            lights[num_lights] = LightData {
                position: [pos.x, pos.y, pos.z, light.intensity],
                color: [light.color.x, light.color.y, light.color.z, light.radius],
                direction: [dir.x, dir.y, dir.z, light.inner_angle],
                params: [light.outer_angle, 1.0, 0.0, 0.0], // params.y = 1 → SpotLight
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
