//! **The scene's fluid is the fluid that runs.** [`fluid_scene_system`] hands the GPU solver the
//! scene's [`FluidSimulation`] — or no fluid, when it has none — and every [`FluidInteractor`] as an
//! obstacle, once a frame from the frame preparation, ahead of the step.
//!
//! Until 2026-11-14 neither component steered anything: whether a fluid ran was a renderer flag two
//! demos set, the solver's dials were constants in its constructor, and the obstacles were whatever
//! the game uploaded itself (`gpu_fluid_coupling_system`, every collider with a velocity, which a
//! game had to install as its render stage's settings).

use crate::core::World;
use crate::renderer::components::FluidInteractor;
use crate::renderer::gpu_fluid::{FluidCollider, FluidSettings, FluidState, MAX_FLUID_COLLIDERS};
use crate::renderer::Renderer;
use gizmo_core::Entity;
use gizmo_physics_core::components::{FluidSimulation, GlobalTransform};

/// Where each interactor was last frame, for the velocity of one with no `Velocity` of its own;
/// and whether a second fluid has been said.
#[derive(Default)]
struct FluidSceneState {
    last_position: std::collections::HashMap<Entity, gizmo_math::Vec3>,
    warned_second_fluid: bool,
    warned_too_many_interactors: bool,
    /// The bodies the water pushed last frame: one it no longer pushes has its push taken off.
    #[cfg(feature = "physics")]
    pushed: Vec<Entity>,
}

/// The solver's settings for `sim`.
pub fn fluid_settings(sim: &FluidSimulation) -> FluidSettings {
    FluidSettings {
        rest_density: sim.target_density,
        relaxation: sim.relaxation,
        viscosity: sim.viscosity,
        xsph: sim.xsph,
        vorticity: sim.vorticity,
        surface_tension: sim.surface_tension,
        cohesion: sim.cohesion,
        solver_iterations: sim.solver_iterations,
        gravity: sim.gravity,
        particle_radius: sim.particle_radius,
        bounds_min: sim.bounds_min.to_array(),
        bounds_max: sim.bounds_max.to_array(),
        particles: sim.particles,
    }
}

/// Gives the GPU fluid the scene's: its settings from the first [`FluidSimulation`] (no particles
/// at all without one), and every [`FluidInteractor`] as an obstacle where its `GlobalTransform`
/// puts it, moving as it moves.
pub fn fluid_scene_system(world: &mut World, renderer: &mut Renderer, dt: f32) {
    let Some(fluid) = renderer.gpu_fluid.as_mut() else { return };
    let sims: Vec<(Entity, FluidSimulation)> = world.query::<(Entity, &FluidSimulation)>().iter().map(|(e, s)| (e, s.clone())).collect();
    let settings = match sims.first() {
        Some((_, sim)) => fluid_settings(sim),
        None => FluidSettings { particles: 0, ..fluid.settings() },
    };
    fluid.apply_settings(&renderer.device, &renderer.queue, settings);
    // **The water itself.** What a scene file brought goes back into the solver after the settings
    // (which laid the box out afresh), and the fluid's entity holds a link to the particles, which
    // a save reads (`FluidState`).
    if let Some(&(water, _)) = sims.first() {
        let count = fluid.settings().particles;
        if world.query::<&FluidState>().get(water).is_none() {
            world.add_component(water, FluidState::default());
        }
        let mut states = world.query_mut::<&mut FluidState>();
        if let Some(mut state) = states.get_mut(water) {
            if let Some(saved) = state.take_saved(settings.rest_density) {
                if saved.len() != count as usize {
                    tracing::warn!(saved = saved.len(), particles = count, "[Fluid] the saved water has another particle count than its FluidSimulation; what fits is used");
                }
                fluid.write_particles(&renderer.queue, &saved[..saved.len().min(count as usize)]);
            }
            state.link(&renderer.device, &renderer.queue, &fluid.particles_buffer, count);
        }
    }
    if sims.is_empty() && world.get_resource::<FluidSceneState>().is_none() {
        return;
    }

    let interactors: Vec<(Entity, FluidInteractor, gizmo_math::Vec3, Option<gizmo_math::Vec3>)> = {
        let globals = world.query::<&GlobalTransform>();
        #[cfg(feature = "physics")]
        let velocities = world.query::<&gizmo_physics_rigid::components::Velocity>();
        world
            .query::<(Entity, &FluidInteractor)>()
            .iter()
            .filter_map(|(e, i)| {
                let at = globals.get(e)?.matrix.w_axis.truncate();
                #[cfg(feature = "physics")]
                let moving = velocities.get(e).map(|v| v.linear);
                #[cfg(not(feature = "physics"))]
                let moving = None;
                Some((e, *i, at, moving))
            })
            .collect()
    };
    let mut state = world.get_resource_mut_or_default::<FluidSceneState>();
    if sims.len() > 1 && !state.warned_second_fluid {
        tracing::warn!(fluids = sims.len(), "[Fluid] the scene has more than one FluidSimulation; the first is the fluid, the others are ignored");
        state.warned_second_fluid = true;
    }
    if interactors.len() > MAX_FLUID_COLLIDERS && !state.warned_too_many_interactors {
        tracing::warn!(interactors = interactors.len(), max = MAX_FLUID_COLLIDERS, "[Fluid] more FluidInteractors than the solver holds; the rest are not in the water");
        state.warned_too_many_interactors = true;
    }
    let mut colliders = Vec::with_capacity(interactors.len().min(MAX_FLUID_COLLIDERS));
    let mut seen = std::collections::HashMap::with_capacity(interactors.len());
    let mut in_water: Vec<(Entity, FluidInteractor, gizmo_math::Vec3)> = Vec::with_capacity(colliders.capacity());
    for (entity, interactor, at, moving) in interactors.into_iter().take(MAX_FLUID_COLLIDERS) {
        let velocity = moving.unwrap_or_else(|| match state.last_position.get(&entity) {
            Some(was) if dt > 0.0 => (at - *was) / dt,
            _ => gizmo_math::Vec3::ZERO,
        });
        seen.insert(entity, at);
        colliders.push(FluidCollider::of(&interactor.shape, at, velocity));
        in_water.push((entity, interactor, velocity));
    }
    state.last_position = seen;
    drop(state);
    let Some(fluid) = renderer.gpu_fluid.as_ref() else { return };
    fluid.set_colliders(&renderer.queue, &colliders);
    #[cfg(feature = "physics")]
    {
        let pushed = lift(world, renderer, &colliders, &in_water);
        let before = std::mem::replace(&mut world.get_resource_mut_or_default::<FluidSceneState>().pushed, pushed.clone());
        let mut forces = world.query_mut::<&mut gizmo_physics_rigid::components::EnvironmentForce>();
        for gone in before.into_iter().filter(|e| !pushed.contains(e)) {
            if let Some(mut f) = forces.get_mut(gone) {
                *f = gizmo_physics_rigid::components::EnvironmentForce::default();
            }
        }
    }
    #[cfg(not(feature = "physics"))]
    let _ = in_water;
}

/// How hard the water drags on a body it closes over, per second: a fully wetted body loses this
/// share of its velocity's momentum each second, so a floating one settles rather than bobbing for
/// ever.
#[cfg(feature = "physics")]
pub const FLUID_DRAG_PER_SECOND: f32 = 2.0;

/// **The water lifts what stands in it.** For every interactor on a moving body, how much of it the
/// water is against (`GpuFluidSystem::count_wetting`, counted from the water as it is now) gives an
/// `EnvironmentForce`: `buoyancy_factor` × its weight × that share, upwards, and a drag of
/// [`FLUID_DRAG_PER_SECOND`] × its mass × its velocity × the share against its motion. A factor of 0
/// is a body the water does not lift at all, and has no force; one out of the water has none.
#[cfg(feature = "physics")]
fn lift(world: &mut World, renderer: &Renderer, colliders: &[FluidCollider], in_water: &[(Entity, FluidInteractor, gizmo_math::Vec3)]) -> Vec<Entity> {
    use gizmo_physics_rigid::components::{EnvironmentForce, RigidBody};
    let Some(fluid) = renderer.gpu_fluid.as_ref() else { return Vec::new() };
    let bodies: Vec<Option<f32>> = {
        let rbs = world.query::<&RigidBody>();
        in_water.iter().map(|(e, i, _)| rbs.get(*e).filter(|rb| rb.is_dynamic() && i.buoyancy_factor != 0.0).map(|rb| rb.mass)).collect()
    };
    if bodies.iter().all(Option::is_none) {
        return Vec::new();
    }
    let Some(wet) = fluid.count_wetting(&renderer.device, &renderer.queue, colliders) else { return Vec::new() };
    let mut pushed = Vec::new();
    let g = fluid.settings().gravity;
    for (((entity, interactor, velocity), mass), share) in in_water.iter().zip(&bodies).zip(&wet) {
        let Some(mass) = *mass else { continue };
        let force = gizmo_math::Vec3::Y * (interactor.buoyancy_factor * mass * g * share) - *velocity * (FLUID_DRAG_PER_SECOND * mass * share);
        let push = EnvironmentForce { force, torque: gizmo_math::Vec3::ZERO };
        pushed.push(*entity);
        let mut forces = world.query_mut::<&mut EnvironmentForce>();
        match forces.get_mut(*entity) {
            Some(mut f) => *f = push,
            None => {
                drop(forces);
                world.add_component(*entity, push);
            }
        }
    }
    pushed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::gpu_fluid::GpuFluidSystem;
    use crate::systems::render::{draw_ahead_of_views, prepare_frame, SimulationDetail};
    use gizmo_math::Vec3;
    use gizmo_physics_core::components::Transform;

    fn usable_gpu() -> bool {
        pollster::block_on(Renderer::headless_adapter_available())
            && !pollster::block_on(Renderer::headless_adapter_is_software())
    }

    /// A renderer with a small pool: the default hundred thousand has hung this machine's GPU in a
    /// test (see the fluid tests in `systems::render`).
    async fn renderer_with_pool(pool: u32) -> Renderer {
        let mut renderer = crate::test_gpu::headless_renderer(64, 64).await;
        renderer.taa = None;
        renderer.ssgi = None;
        renderer.ssr = None;
        let hdr = renderer.post.hdr_texture.format();
        let (w, h) = (renderer.config.width, renderer.config.height);
        renderer.gpu_fluid = Some(GpuFluidSystem::new(&renderer.device, &renderer.queue, pool, &renderer.scene.global_bind_group_layout, hdr, w, h));
        renderer
    }

    fn step(world: &mut World, renderer: &mut Renderer, frames: usize) {
        for _ in 0..frames {
            prepare_frame(world, renderer);
            draw_ahead_of_views(world, renderer, SimulationDetail::Full);
        }
    }

    fn particles(renderer: &Renderer) -> Vec<crate::renderer::gpu_fluid::FluidParticle> {
        let fluid = renderer.gpu_fluid.as_ref().unwrap();
        fluid.read_particles(&renderer.device, &renderer.queue, fluid.settings().particles).expect("read back")
    }

    /// **A `FluidSimulation` is the fluid that runs, and without one there is none.** The solver's
    /// block holds the component's dials and box after a frame, the water stands inside that box,
    /// as many particles as it asked for are stepped, and an edit reaches the GPU on the next frame.
    /// Taken away, nothing is stepped. Until 2026-11-14 the component steered nothing and a
    /// renderer flag decided whether any fluid ran.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_fluid_simulation_is_the_fluid_that_runs() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !usable_gpu() {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = renderer_with_pool(8_000).await;
            let mut world = World::new();
            step(&mut world, &mut renderer, 1);
            assert_eq!(renderer.gpu_fluid.as_ref().unwrap().active_particles, 0, "a fluid ran in a scene with none");

            let mut sim = FluidSimulation::default();
            sim.bounds_min = Vec3::new(2.0, 1.0, -1.5);
            sim.bounds_max = Vec3::new(5.0, 4.0, 1.5);
            sim.particles = 3_000;
            sim.relaxation = 300.0;
            sim.target_density = 900.0;
            let water = world.spawn();
            world.add_component(water, sim.clone());
            step(&mut world, &mut renderer, 20);

            let fluid = renderer.gpu_fluid.as_ref().unwrap();
            assert_eq!(fluid.active_particles, 3_000, "not as many particles as the scene asked for");
            let params = fluid.read_params(&renderer.device, &renderer.queue).expect("params");
            assert_eq!(params.bounds_min, [2.0, 1.0, -1.5]);
            assert_eq!(params.bounds_max, [5.0, 4.0, 1.5]);
            assert_eq!((params.relaxation, params.rest_density), (300.0, 900.0));
            assert!((params.mass - 900.0 * (0.1f32 * 0.77).powi(3)).abs() < 1e-5, "mass {}", params.mass);
            let ps = particles(&renderer);
            assert!(ps.iter().all(|p| p.phase != 0xFFFF_FFFF), "some of the water was never placed");
            for p in &ps {
                let at = Vec3::from_array(p.position);
                assert!(at.is_finite(), "a particle left the simulation: {at:?}");
                assert!(
                    at.cmpge(sim.bounds_min - 0.01).all() && at.cmple(sim.bounds_max + 0.01).all(),
                    "a particle is outside the fluid's box: {at:?}"
                );
            }
            let mean_y = ps.iter().map(|p| p.position[1]).sum::<f32>() / ps.len() as f32;
            assert!(mean_y < 2.0, "the water does not stand on the floor of its box (mean height {mean_y})");

            world.query_mut::<&mut FluidSimulation>().get_mut(water).unwrap().relaxation = 900.0;
            step(&mut world, &mut renderer, 1);
            let fluid = renderer.gpu_fluid.as_ref().unwrap();
            assert_eq!(fluid.read_params(&renderer.device, &renderer.queue).unwrap().relaxation, 900.0, "an edit did not reach the solver");

            world.remove_component::<FluidSimulation>(water);
            step(&mut world, &mut renderer, 1);
            assert_eq!(renderer.gpu_fluid.as_ref().unwrap().active_particles, 0, "the fluid ran on with its component gone");
        });
    }

    /// **A `FluidInteractor` is in the water.** A ball held in the middle of a pool pushes the water
    /// out of itself: after it has stood there a while no particle is inside it, where without it
    /// the pool is full. It reaches the solver from its transform — nothing about it is filled in
    /// by hand.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_fluid_interactor_pushes_the_water_out_of_itself() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !usable_gpu() {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        let centre = Vec3::new(0.0, 0.25, 0.0);
        let inside = |ps: &[crate::renderer::gpu_fluid::FluidParticle], r: f32| {
            ps.iter().filter(|p| (Vec3::from_array(p.position) - centre).length() < r).count()
        };
        pollster::block_on(async {
            let mut sim = FluidSimulation::default();
            sim.bounds_min = Vec3::new(-1.5, 0.0, -1.5);
            sim.bounds_max = Vec3::new(1.5, 2.0, 1.5);
            sim.particles = 6_000;
            let pool = |with_ball: bool| {
                let sim = sim.clone();
                async move {
                    let mut renderer = renderer_with_pool(6_000).await;
                    let mut world = World::new();
                    world.spawn_bundle((sim,));
                    if with_ball {
                        let ball = world.spawn();
                        world.add_component(ball, Transform::new(centre));
                        world.add_component(ball, GlobalTransform::default());
                        world.add_component(ball, FluidInteractor::sphere(0.3));
                    }
                    step(&mut world, &mut renderer, 90);
                    let n = renderer.gpu_fluid.as_ref().unwrap().read_params(&renderer.device, &renderer.queue).unwrap().num_colliders;
                    (particles(&renderer), n)
                }
            };
            let (without, none) = pool(false).await;
            let (with, one) = pool(true).await;
            assert_eq!((none, one), (0, 1), "the solver's obstacle count");
            let full = inside(&without, 0.25);
            assert!(full > 20, "the pool has no water where the ball goes ({full}) — the test would prove nothing");
            let left = inside(&with, 0.25);
            assert!(left * 10 <= full, "the ball did not push the water out: {left} particles inside it, {full} without it");
        });
    }

    /// **The water reopens where it was, and goes on as it was going.** A block of water is let go
    /// in the corner of its tank and caught mid-slosh; the scene is saved, opened by a fresh
    /// renderer and world, and both run the same frames on. Every particle of the reopened water is
    /// where the original's is. Laid out afresh instead — what a reopen did before `FluidState` —
    /// it would stand flat on the floor.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_water_reopens_where_it_was_and_goes_on_the_same() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !usable_gpu() {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        let path = std::env::temp_dir().join(format!("gizmo-fluid-reopen-{}.scene", std::process::id()));
        let path = path.to_string_lossy().into_owned();
        pollster::block_on(async {
            let mut sim = FluidSimulation::default();
            sim.bounds_min = Vec3::new(-1.0, 0.0, -0.5);
            sim.bounds_max = Vec3::new(1.0, 2.0, 0.5);
            sim.particles = 4_000;
            let mut renderer = renderer_with_pool(4_000).await;
            let mut world = World::new();
            world.spawn_bundle((sim.clone(),));
            step(&mut world, &mut renderer, 1);
            // A dam: the water stacked against the left wall, then let go.
            let dam: Vec<_> = particles(&renderer)
                .into_iter()
                .enumerate()
                .map(|(i, mut p)| {
                    let (x, y, z) = ((i % 10) as f32, ((i / 10) % 40) as f32, (i / 400) as f32);
                    p.position = [-0.95 + x * 0.077, 0.05 + y * 0.077, -0.45 + z * 0.077];
                    p.predicted_position = p.position;
                    p
                })
                .collect();
            renderer.gpu_fluid.as_ref().unwrap().write_particles(&renderer.queue, &dam);
            step(&mut world, &mut renderer, 25);
            crate::scene::SceneData::save(&world, &path, &crate::full_scene_registry()).expect("saves");
            let caught = particles(&renderer);

            let mut reopened_renderer = renderer_with_pool(4_000).await;
            let mut reopened = World::new();
            crate::scene::SceneData::load_into(&path, &mut reopened, &crate::full_scene_registry()).expect("loads");
            prepare_frame(&mut reopened, &mut reopened_renderer);
            let back = particles(&reopened_renderer);
            for (a, b) in caught.iter().zip(&back) {
                assert_eq!((a.position, a.velocity), (b.position, b.velocity), "a particle came back somewhere else");
            }
            let spread = |ps: &[crate::renderer::gpu_fluid::FluidParticle]| ps.iter().map(|p| p.position[1]).fold(0.0f32, f32::max);
            assert!(spread(&caught) > 0.6, "the water was not caught in motion (top {})", spread(&caught));

            draw_ahead_of_views(&reopened, &mut reopened_renderer, SimulationDetail::Full);
            draw_ahead_of_views(&world, &mut renderer, SimulationDetail::Full);
            step(&mut world, &mut renderer, 10);
            step(&mut reopened, &mut reopened_renderer, 10);
            let (a, b) = (particles(&renderer), particles(&reopened_renderer));
            let worst = a.iter().zip(&b).map(|(p, q)| (Vec3::from_array(p.position) - Vec3::from_array(q.position)).length()).fold(0.0f32, f32::max);
            assert_eq!(worst, 0.0, "the reopened water went on otherwise: a particle {worst} m from where the original's is");
        });
        let _ = std::fs::remove_file(&path);
    }

    /// **The same water stepped twice goes the same way, bit for bit.** Two pools given one dam
    /// break are compared after 1, 6, 16 and 36 steps. The solver wrote corrected positions and new
    /// velocities in place while each particle's neighbours were still reading them, so the order
    /// the GPU ran its threads in went into the water: measured 2026-11-14, 1.3 mm apart after one
    /// step and 70 cm after 36. A reopened scene could not go on the way it was going, because the
    /// original did not either.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_same_water_stepped_twice_goes_the_same_way() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !usable_gpu() {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let mut sim = FluidSimulation::default();
            sim.bounds_min = Vec3::new(-1.0, 0.0, -0.5);
            sim.bounds_max = Vec3::new(1.0, 2.0, 0.5);
            sim.particles = 4_000;
            let run = || {
                let sim = sim.clone();
                async move {
                    let mut renderer = renderer_with_pool(4_000).await;
                    let mut world = World::new();
                    world.spawn_bundle((sim,));
                    step(&mut world, &mut renderer, 1);
                    let dam: Vec<_> = particles(&renderer).into_iter().enumerate().map(|(i, mut p)| {
                        let (x, y, z) = ((i % 10) as f32, ((i / 10) % 40) as f32, (i / 400) as f32);
                        p.position = [-0.95 + x * 0.077, 0.05 + y * 0.077, -0.45 + z * 0.077];
                        p.predicted_position = p.position; p.velocity = [0.0;3];
                        p
                    }).collect();
                    renderer.gpu_fluid.as_ref().unwrap().write_particles(&renderer.queue, &dam);
                    let mut out = Vec::new();
                    for n in [1usize, 5, 10, 20] {
                        step(&mut world, &mut renderer, n);
                        out.push(particles(&renderer));
                    }
                    out
                }
            };
            let (a, b) = (run().await, run().await);
            for (k, (x, y)) in a.iter().zip(&b).enumerate() {
                let worst = x.iter().zip(y).map(|(p, q)| (Vec3::from_array(p.position) - Vec3::from_array(q.position)).length()).fold(0.0f32, f32::max);
                assert_eq!(worst, 0.0, "checkpoint {k}: the two runs are {worst} m apart");
            }
            let top = a[3].iter().map(|p| p.position[1]).fold(0.0f32, f32::max);
            assert!(top < 3.0 && a[3].iter().any(|p| p.position[0] > 0.0), "the dam did not break — the water did not move");
        });
    }

    /// **The water lifts what stands in it, as much as its buoyancy says.** Three 1 kg balls are let
    /// go under the surface of one pool: with a factor of 0 the water does not lift it and it sinks
    /// to the floor; with 3 it is lifted three times its weight when under and rises until about a
    /// third of it is wet, floating at the surface; 1 stays between. Before 2026-11-14
    /// `buoyancy_factor` was read by nothing.
    #[test]
    #[ignore = "requires a GPU adapter"]
    #[cfg(feature = "physics")]
    fn the_water_lifts_a_body_as_much_as_its_buoyancy_says() {
        use gizmo_physics_core::Collider;
        use gizmo_physics_rigid::components::{RigidBody, Velocity};
        let _gpu = crate::test_gpu::gpu_lock();
        if !usable_gpu() {
            eprintln!("skipping: no usable GPU adapter");
            return;
        }
        pollster::block_on(async {
            let mut renderer = renderer_with_pool(8_000).await;
            let mut world = World::new();
            world.insert_resource(gizmo_physics_rigid::world::PhysicsWorld::new());
            world.insert_resource(gizmo_core::commands::CommandQueue::default());
            let mut sim = FluidSimulation::default();
            sim.bounds_min = Vec3::new(-1.0, 0.0, -1.0);
            sim.bounds_max = Vec3::new(1.0, 2.5, 1.0);
            sim.particles = 8_000;
            world.spawn_bundle((sim,));
            world.spawn_bundle((Transform::new(Vec3::ZERO), GlobalTransform::default(), RigidBody::new(0.0, false), Velocity::default(), Collider::plane(Vec3::Y, 0.0)));
            let ball = |world: &mut World, x: f32, factor: f32| {
                let col = Collider::sphere(0.15);
                let mut rb = RigidBody::new(1.0, true);
                rb.update_inertia_from_collider(&col);
                world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.6, 0.0)),
                    GlobalTransform::default(),
                    rb,
                    Velocity::default(),
                    col,
                    FluidInteractor::sphere(0.15).with_buoyancy(factor),
                ))
            };
            let balls = [ball(&mut world, -0.55, 0.0), ball(&mut world, 0.0, 1.0), ball(&mut world, 0.55, 3.0)];
            let surface = |r: &Renderer| {
                let mut ys: Vec<f32> = particles(r).iter().map(|p| p.position[1]).collect();
                ys.sort_by(f32::total_cmp);
                ys[ys.len() * 98 / 100]
            };
            step(&mut world, &mut renderer, 1);
            for _ in 0..180 {
                gizmo_physics_rigid::system::physics_step_system(&world, 1.0 / 60.0);
                step(&mut world, &mut renderer, 1);
            }
            let y = |e| world.query::<&Transform>().get(e).unwrap().position.y;
            let (sunk, neutral, floating) = (y(balls[0]), y(balls[1]), y(balls[2]));
            let top = surface(&renderer);
            eprintln!("surface {top}, balls at {sunk} / {neutral} / {floating}");
            assert!(sunk < 0.25, "the ball the water does not lift is not on the floor: {sunk}");
            assert!(floating > top - 0.2, "the buoyant ball does not float at the surface ({top}): {floating}");
            assert!(sunk < neutral && neutral < floating, "the three are not in buoyancy order: {sunk} / {neutral} / {floating}");
        });
    }
}
