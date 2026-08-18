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
    /// The [`MAX_LIGHTS`] most important point/spot lights for this frame, nearest first
    /// (see [`collect_scene_lights`] for what "most important" means and why the order matters).
    pub lights: [LightData; MAX_LIGHTS],
    /// How many entries of `lights` are live this frame; the rest are stale and must not be
    /// read.
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

/// One light that *could* go in the frame's array, with the key that decides whether it does.
struct Candidate {
    data: LightData,
    is_point: bool,
    /// Distance from the camera to the light's sphere of influence — `0.0` when the camera is
    /// inside it. Ascending: the lights whose light actually reaches the viewer come first.
    surface_dist: f32,
    /// How far this light throws, as a tie-break among the many that all score `0.0` above.
    /// Descending: between two lights around the camera, the brighter/larger one matters more.
    reach: f32,
    /// Final tie-break, and the reason the selection is stable: entity id is a property of the
    /// world, not of iteration order.
    entity_id: u32,
}

/// Collect the scene's dynamic lights (point + spot, capped at [`MAX_LIGHTS`]) and the sun.
///
/// Each light's world transform prefers a synced `GlobalTransform` (so a parented
/// light follows its parent, matching how meshes are placed) and falls back to the
/// light's own `Transform` when it has none — the same robustness the camera path
/// uses. Previously the game path queried `(&Light, &GlobalTransform)` (dropping
/// any light without a global) while the studio path read the raw `Transform`
/// (ignoring parenting); this unifies both onto the correct-and-robust rule.
///
/// # Which lights survive the cap, and why it is not "the first ten"
///
/// The shader array holds [`MAX_LIGHTS`] — the constant, deliberately not a number written here:
/// it has been 10, then 32, then 256 within one week, and this line said "32" for a day after the
/// last of those. It used to be
/// filled in ECS iteration order, `break`-ing on the first light that did not fit, which had three
/// consequences, all visible:
///
/// 1. **Distance did not enter into it.** A light 500 units behind the camera took a slot from one
///    lighting the wall in front of it.
/// 2. **Point lights starved spot lights.** Points were collected in their own loop first, so a
///    full array of points anywhere in the level meant no spotlight was lit at all — and the studio's
///    "cast shadows from the first light" rule then followed that arbitrary choice.
/// 3. **The chosen set changed as the world changed shape.** Archetype iteration order is stable
///    only while the archetype set is: spawning, despawning or adding a component reorders it, so
///    which lights were live could change from frame to frame with the scene standing still.
///    That reads as flicker, and it is the reason this is ranked rather than merely capped.
///
/// So every light is now scored against the camera and the best [`MAX_LIGHTS`] win, points and
/// spots competing in one pool. The score is the distance from `cam_pos` to the light's **sphere
/// of influence** (`radius`), which is exact rather than a heuristic: every lighting shader
/// windows attenuation with `clamp(1 - (d/r)^4, 0, 1)`, so a light contributes precisely nothing
/// past its radius. Lights reaching the camera all score `0.0` and are then ordered by
/// `intensity * radius`, and any remaining tie by entity id — which is what makes the selection a
/// pure function of the world state, and therefore the same on every frame that state is.
///
/// Lights whose sphere of influence does not reach the **camera frustum** are dropped before the
/// ranking runs: nothing they light is on screen, so a slot spent on one is a slot taken from a
/// light that is. `Frustum::intersects_sphere` is a plane test that errs toward keeping (a sphere
/// straddling a plane counts as inside), which is the direction a cull must err in. For a spot
/// light the sphere is a bound on its cone, so the test is conservative there twice over.
///
/// The jitter TAA adds to `view_proj` is sub-pixel and cannot decide this test, so the frame's own
/// matrix is used rather than plumbing an unjittered one through.
pub fn collect_scene_lights(
    world: &World,
    cam_pos: Vec3,
    view_proj: gizmo_math::Mat4,
) -> SceneLights {
    let frustum = gizmo_math::Frustum::from_matrix(&view_proj);
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

    // Every light in the scene, not just the first MAX_LIGHTS: the cap is applied by the ranking
    // below, and a `break` here is what used to make the choice arbitrary. One allocation per
    // frame for the whole light set — if a scene ever makes that measurable, the shape to reach
    // for is a fixed top-N insertion, not a smaller cap.
    let mut candidates: Vec<Candidate> = Vec::new();

    // Distance from the camera to a light's sphere of influence, and how far the light throws.
    // Non-finite input (a NaN transform, a deserialized garbage intensity) must not be allowed to
    // win a slot, so it sorts last instead of poisoning the comparison.
    // `None` → this light cannot affect the frame and is not a candidate at all.
    let score = |pos: Vec3, intensity: f32, radius: f32| -> Option<(f32, f32)> {
        if !pos.is_finite() || !intensity.is_finite() || !radius.is_finite() {
            // Sorts last rather than being dropped: a NaN light is a scene bug, and silently
            // losing it hides that, while letting it win a slot would poison the frame.
            return Some((f32::MAX, 0.0));
        }
        if !frustum.intersects_sphere(pos, radius.max(0.0)) {
            return None;
        }
        Some(((cam_pos.distance(pos) - radius).max(0.0), intensity * radius))
    };

    if let Some(q) = world.query::<&PointLight>() {
        for (e, light) in q.iter() {
            let Some((pos, _)) = world_tf(e) else { continue };
            let Some((surface_dist, reach)) = score(pos, light.intensity, light.radius) else {
                continue;
            };
            candidates.push(Candidate {
                data: LightData {
                    position: [pos.x, pos.y, pos.z, light.intensity],
                    color: [light.color.x, light.color.y, light.color.z, light.radius],
                    direction: [0.0, -1.0, 0.0, 0.0],
                    params: [0.0, 0.0, 0.0, 0.0], // params.y = 0 → PointLight
                },
                is_point: true,
                surface_dist,
                reach,
                entity_id: e,
            });
        }
    }

    if let Some(q) = world.query::<&SpotLight>() {
        for (e, light) in q.iter() {
            let Some((pos, rot)) = world_tf(e) else { continue };
            let dir = rot.mul_vec3(Vec3::new(0.0, 0.0, -1.0)).normalize();
            // The shaders compare the cone against `dot(-L, spot_dir)` (a cosine), so the
            // cutoffs must be COSINES of the cone angles — every lighting shader documents
            // `w = inner_cutoff_cos`, `params.x = outer_cutoff_cos`. `SpotLight` stores the
            // angles in radians (its ctor clamps inner ≤ outer), so convert here. Passing the
            // raw radians made the cone a hard cut at the wrong angle with no falloff; the
            // studio path used to `.cos()` these itself, the game path never did (its spots
            // were broken) — single-sourcing the fix corrects both.
            let Some((surface_dist, reach)) = score(pos, light.intensity, light.radius) else {
                continue;
            };
            candidates.push(Candidate {
                data: LightData {
                    position: [pos.x, pos.y, pos.z, light.intensity],
                    color: [light.color.x, light.color.y, light.color.z, light.radius],
                    direction: [dir.x, dir.y, dir.z, light.inner_angle.cos()],
                    params: [light.outer_angle.cos(), 1.0, 0.0, 0.0], // params.y = 1 → SpotLight
                },
                is_point: false,
                surface_dist,
                reach,
                entity_id: e,
            });
        }
    }

    // Nearest influence first, then the light that throws furthest, then entity id. `total_cmp`
    // rather than `partial_cmp().unwrap()`: the scores are sanitized above, and a total order is
    // what makes this a sort at all.
    candidates.sort_unstable_by(|a, b| {
        a.surface_dist
            .total_cmp(&b.surface_dist)
            .then(b.reach.total_cmp(&a.reach))
            .then(a.entity_id.cmp(&b.entity_id))
    });

    let mut lights = [LightData::default(); MAX_LIGHTS];
    let mut num_lights = 0usize;
    // The highest-ranked point light *that made the cut* owns the single point-shadow cube — a
    // light outside the array is not lit at all, so casting its shadow would be a cube with no
    // light in it.
    let mut shadow_point_index: i32 = -1;
    for c in candidates.iter().take(MAX_LIGHTS) {
        if c.is_point && shadow_point_index < 0 {
            shadow_point_index = num_lights as i32;
        }
        lights[num_lights] = c.data;
        num_lights += 1;
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
    /// "First collected light" is what the code does, and since lights are ranked it now means
    /// something a scene author can predict: the light with the most influence at the camera,
    /// point or spot (see [`collect_scene_lights`]). It used to mean whichever light ECS iteration
    /// reached first, which is why the comment before it claimed "first point light" — true only
    /// while the old collector filled points ahead of spots.
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
    /// Which of those lights reach which cluster of the view volume.
    ///
    /// Computed here because this is where the camera and the collected lights are both in hand,
    /// and both render paths need the same answer. **The caller must upload it**
    /// (`renderer.scene.upload_clusters`): the lighting shaders read their light list from these
    /// buffers, so a path that skips the upload lights nothing at all — which is the failure mode
    /// to expect if a new render path appears and forgets.
    pub clusters: crate::renderer::clustered::ClusterAssignment,
}

/// The per-frame inputs that are *not* the world: the camera, and the handful of decisions each
/// render path makes for itself.
pub struct SceneSetupInputs {
    /// The camera the frame is rendered through.
    pub camera: crate::renderer::CameraFrame,
    /// Viewport aspect and the camera's vertical FOV — the cascade fit needs the frustum, and
    /// [`crate::renderer::CameraFrame`] carries the matrix rather than the angles it came from.
    pub aspect: f32,
    /// The camera's vertical field of view, in radians.
    pub cam_fov: f32,
    /// Which light casts the cascaded shadows this frame, if any.
    pub shadow_caster: ShadowCaster,
    /// Sky, ambient and fog — everything lighting the scene that is not a light.
    pub environment: crate::renderer::EnvironmentFrame,
    /// Whether this path renders the point-shadow cube. A path that does not must leave it off:
    /// the lookup would sample whatever the cube held on the last frame that did.
    pub point_shadows_enabled: bool,
    /// Time since the scene started, in seconds; what animated shaders advance on.
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

    let lights = collect_scene_lights(world, inputs.camera.position, inputs.camera.view_proj);

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

    // Cluster assignment over the lights that survived collection. The indices it produces are
    // indices into `lights.lights`, which is exactly what the shader indexes.
    let grid = crate::renderer::clustered::ClusterGrid::default();
    let spheres: Vec<crate::renderer::clustered::LightSphere> = lights.lights
        [..lights.num_lights as usize]
        .iter()
        .map(|l| crate::renderer::clustered::LightSphere {
            center: Vec3::new(l.position[0], l.position[1], l.position[2]),
            radius: l.color[3],
        })
        .collect();
    let clusters = crate::renderer::clustered::assign_lights(
        grid,
        crate::renderer::clustered::ClusterView {
            view_proj: inputs.camera.view_proj,
            camera_pos: inputs.camera.position,
            forward: inputs.camera.forward,
            near: inputs.camera.near,
            far: inputs.camera.far,
        },
        &spheres,
    );
    if clusters.dropped > 0 {
        tracing::debug!(
            dropped = clusters.dropped,
            "[Render] bir küme dolduğu için ışık ataması düştü"
        );
    }

    SceneSetup {
        clusters,
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

    /// A camera at the origin looking down **+Z**, wide and deep enough to contain every light
    /// these tests place. The frustum cull is real, so a test that wants to measure the *ranking*
    /// has to hand it a view that keeps every candidate; `Mat4::IDENTITY` would not (it is the NDC
    /// cube, and would silently cull almost everything here).
    fn wide_view() -> gizmo_math::Mat4 {
        let view = gizmo_math::Mat4::look_at_rh(Vec3::ZERO, Vec3::Z, Vec3::Y);
        let proj = gizmo_math::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 20_000.0);
        proj * view
    }

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

        let l = collect_scene_lights(&world, Vec3::ZERO, wide_view());
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

    /// Ranking decides the order, not the light's kind: a spot nearer the camera than a point
    /// comes first, which is the property the old collector could not have (it filled every point
    /// before looking at a single spot). The `Transform`-only fallback still resolves, and the
    /// point-shadow index follows the light to wherever the ranking put it.
    #[test]
    fn the_nearer_light_ranks_first_even_when_it_is_the_spot() {
        let mut world = World::new();
        // A point light 50 units away, carrying a GlobalTransform.
        let p = world.spawn();
        world.add_component(
            p,
            GlobalTransform {
                matrix: gizmo_math::Mat4::from_translation(Vec3::new(0.0, 0.0, 50.0)),
            },
        );
        world.add_component(p, PointLight::new(Vec3::ONE, 5.0, 12.0));
        // A spot light 4 units away with ONLY a Transform → resolves via the fallback.
        let s = world.spawn();
        world.add_component(s, Transform::new(Vec3::new(1.0, 2.0, 3.0)));
        world.add_component(s, SpotLight::new(Vec3::ONE, 7.0, 20.0, 0.3, 0.5));

        let l = collect_scene_lights(&world, Vec3::ZERO, wide_view());
        assert_eq!(l.num_lights, 2);
        assert_eq!(l.lights[0].params[1], 1.0, "the nearby spot outranks the distant point");
        assert_eq!(l.lights[1].params[1], 0.0);
        // Spot position came from its Transform (GlobalTransform-less) fallback.
        assert_eq!(l.lights[0].position, [1.0, 2.0, 3.0, 7.0]);
        assert_eq!(l.shadow_point_index, 1, "the caster index points at the point light's slot");
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

        let l = collect_scene_lights(&world, Vec3::ZERO, wide_view());
        assert_eq!(l.num_lights, 1);
        assert_eq!(l.shadow_point_index, -1, "no point light → no point-shadow caster");
    }

    /// Spawn a point light at `z` with a `Transform`, and return its entity.
    fn point_at(world: &mut World, z: f32) -> gizmo_core::entity::Entity {
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new(0.0, 0.0, z)));
        world.add_component(e, PointLight::new(Vec3::ONE, 5.0, 10.0));
        e
    }

    /// The z coordinates in the frame's light array, in order.
    fn selected_z(l: &SceneLights) -> Vec<f32> {
        l.lights[..l.num_lights as usize].iter().map(|d| d.position[2]).collect()
    }

    /// The cap drops the *far* lights, not the ones ECS iteration happened to reach last.
    ///
    /// Two more lights than the array holds, spawned far-to-near so iteration order is the exact
    /// opposite of importance: the old collector kept the furthest ones and threw away the light
    /// standing next to the camera. Sized from `MAX_LIGHTS` rather than a literal — this test was
    /// written when the cap was 10 and would have quietly stopped testing the cap when it became
    /// 32.
    #[test]
    fn the_lights_that_lose_a_slot_are_the_distant_ones() {
        let mut world = World::new();
        for i in (1..=MAX_LIGHTS + 2).rev() {
            point_at(&mut world, i as f32 * 100.0);
        }

        let l = collect_scene_lights(&world, Vec3::ZERO, wide_view());
        assert_eq!(l.num_lights as usize, MAX_LIGHTS, "the array is full");
        assert_eq!(
            selected_z(&l),
            (1..=MAX_LIGHTS).map(|i| i as f32 * 100.0).collect::<Vec<_>>(),
            "the nearest MAX_LIGHTS survive, nearest first"
        );
    }

    /// The same world state must select the same lights however the archetypes are laid out.
    ///
    /// This is the flicker regression. Adding a component moves an entity to another archetype,
    /// which reorders iteration; with a `break`-at-ten collector that silently changed *which*
    /// lights were lit while the scene stood still. Here nothing about the lights changes, so
    /// nothing about the selection may.
    #[test]
    fn the_selection_does_not_depend_on_archetype_order() {
        let mut world = World::new();
        let mut lights = Vec::new();
        for i in 1..=12 {
            lights.push(point_at(&mut world, i as f32 * 10.0));
        }
        let before = collect_scene_lights(&world, Vec3::ZERO, wide_view());

        // Move three of the winners into a different archetype, leaving them the same lights in
        // the same places. (`GlobalTransform::default()` is the identity, and the collector
        // prefers it — so these three now resolve to the origin. Give each the matrix that
        // reproduces its own position, or the test would be measuring a move, not a reorder.)
        for &e in &[lights[0], lights[4], lights[9]] {
            let z = world.query::<&Transform>().unwrap().get(e.id()).unwrap().position.z;
            world.add_component(
                e,
                GlobalTransform {
                    matrix: gizmo_math::Mat4::from_translation(Vec3::new(0.0, 0.0, z)),
                },
            );
        }
        let after = collect_scene_lights(&world, Vec3::ZERO, wide_view());

        assert_eq!(selected_z(&before), selected_z(&after), "same world, same ten lights");
        assert_eq!(before.shadow_point_index, after.shadow_point_index);
    }

    /// A full array of distant point lights no longer starves a spotlight at the camera.
    ///
    /// The old collector filled the array from its point-light loop and `break`-ed before the spot
    /// loop ran, so this scene lit zero spotlights — and in the studio, whose shadow caster is
    /// "the first light", cast shadows from the furthest point light in the level.
    #[test]
    fn a_near_spot_takes_a_slot_from_a_full_array_of_distant_points() {
        let mut world = World::new();
        for i in 1..=MAX_LIGHTS {
            point_at(&mut world, i as f32 * 100.0);
        }
        let furthest = MAX_LIGHTS as f32 * 100.0;
        let s = world.spawn();
        world.add_component(s, Transform::new(Vec3::new(0.0, 0.0, 2.0)));
        world.add_component(s, SpotLight::new(Vec3::ONE, 7.0, 20.0, 0.3, 0.5));

        let l = collect_scene_lights(&world, Vec3::ZERO, wide_view());
        assert_eq!(l.num_lights as usize, MAX_LIGHTS);
        assert_eq!(l.lights[0].params[1], 1.0, "the spot at the camera is lit, and ranks first");
        assert!(
            !selected_z(&l).contains(&furthest),
            "the furthest point light is the one that gives up its slot: {:?}",
            selected_z(&l)
        );
    }

    /// A light whose sphere of influence is entirely off screen does not take a slot.
    ///
    /// The same light, at the same distance, in front of the camera instead of behind it, does —
    /// so this measures the cull and not the ranking. Without it a level's worth of lights behind
    /// the player competed for the frame's ten slots on distance alone, and won.
    #[test]
    fn a_light_that_cannot_reach_the_screen_is_not_a_candidate() {
        let behind = {
            let mut world = World::new();
            point_at(&mut world, -50.0); // camera looks down +Z, so this is behind it
            collect_scene_lights(&world, Vec3::ZERO, wide_view())
        };
        assert_eq!(behind.num_lights, 0, "a light behind the camera lights nothing on screen");

        let in_front = {
            let mut world = World::new();
            point_at(&mut world, 50.0);
            collect_scene_lights(&world, Vec3::ZERO, wide_view())
        };
        assert_eq!(in_front.num_lights, 1, "the same light in view must be collected");

        // The cull errs toward keeping: a light behind the camera whose radius reaches across it
        // still lights what is in front, so it must survive.
        let straddling = {
            let mut world = World::new();
            let e = world.spawn();
            world.add_component(e, Transform::new(Vec3::new(0.0, 0.0, -2.0)));
            world.add_component(e, PointLight::new(Vec3::ONE, 5.0, 20.0));
            collect_scene_lights(&world, Vec3::ZERO, wide_view())
        };
        assert_eq!(
            straddling.num_lights, 1,
            "a light just behind the camera with a 20 m radius still lights the scene in front"
        );
    }
}
