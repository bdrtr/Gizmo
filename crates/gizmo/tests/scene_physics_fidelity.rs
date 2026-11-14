//! **A scene saved in the middle of play goes on the way it was going.**
//!
//! `scene_fidelity` (in `gizmo-studio`) holds every component to "registered, or named with the
//! reason it is not". A registration is not proof, though: the values have to come back, and the
//! system that reads them has to pick them up again. For what a picture shows, the GPU tests in
//! `systems::render` compare frames. Physics and play cannot be compared that way, so they are
//! compared by **what happens next**: each test below builds a small world, runs it for a while,
//! saves it through `gizmo::full_scene_registry()` (what the editor's ▶/⏹ snapshot and a scene
//! file both use), opens it into a fresh world, keeps running the same systems, and asserts on
//! the outcome the component exists for.
//!
//! Why not a bit-exact continuation of the whole world: a fresh `PhysicsWorld` has no warm-start
//! cache, so anything resting in contact settles along a slightly different path after a reopen.
//! That is the solver's state, not a scene's. Where a body is in free flight, or a system has no
//! hidden state (cloth, rope, timers, spin), the continuation **is** compared to the original's.
//!
//! Each test fails when its component's registration is taken out — checked by hand, one by one,
//! when this file was written (2026-11-14).

use gizmo::core::component::EntityName;
use gizmo::core::commands::CommandQueue;
use gizmo::core::system::System;
use gizmo::core::world::World;
use gizmo::core::Entity;
use gizmo::math::{Quat, Vec3};
use gizmo::physics::components::{
    Breakable, CombineMode, ExternalForce, KinematicTarget, PhysicsMaterial, RigidBody, Velocity,
};
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{Collider, ColliderShape, Transform};
use gizmo::scene::scene::SceneData;

// ── The harness ────────────────────────────────────────────────────────────────────────────────

/// Save `world` the way the editor does and open it into a fresh one that `fresh` has given its
/// resources — a scene holds entities, not the physics world's settings.
fn reopen(world: &World, fresh: impl Fn(&mut World)) -> World {
    let registry = gizmo::full_scene_registry();
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("gizmo_play_fidelity_{}_{n}.scene", std::process::id()));
    let path = path.to_string_lossy().to_string();
    SceneData::save(world, &path, &registry).expect("save");
    let mut reopened = World::new();
    fresh(&mut reopened);
    let loaded = SceneData::load_into(&path, &mut reopened, &registry);
    let _ = std::fs::remove_file(&path);
    loaded.expect("load");
    reopened
}

fn named(world: &mut World, name: &str) -> Entity {
    let e = world.spawn();
    world.add_component(e, EntityName::new(name));
    e
}

fn find(world: &World, name: &str) -> Option<Entity> {
    world.query::<(Entity, &EntityName)>().iter().find(|(_, n)| n.0 == name).map(|(e, _)| e)
}

fn entity(world: &World, name: &str) -> Entity {
    find(world, name).unwrap_or_else(|| panic!("no entity named {name:?}"))
}

fn position(world: &World, name: &str) -> Vec3 {
    world.query::<&Transform>().get(entity(world, name)).expect("a transform").position
}

fn velocity(world: &World, name: &str) -> Vec3 {
    world.query::<&Velocity>().get(entity(world, name)).expect("a velocity").linear
}

fn physics(world: &mut World) {
    world.insert_resource(PhysicsWorld::new());
    world.insert_resource(CommandQueue::default());
}

fn step(world: &mut World, frames: usize) {
    for _ in 0..frames {
        gizmo::systems::cpu_physics_step_system(world, DT);
        world.apply_commands();
    }
}

const DT: f32 = 1.0 / 60.0;

fn ground(world: &mut World) {
    let e = named(world, "ground");
    world.add_component(e, Transform::new(Vec3::new(0.0, -1.0, 0.0)));
    world.add_component(e, RigidBody::new_static());
    world.add_component(e, Velocity::default());
    world.add_component(e, Collider::box_collider(Vec3::new(50.0, 1.0, 50.0)));
}

fn ball(world: &mut World, name: &str, at: Vec3) -> Entity {
    let e = named(world, name);
    let collider = Collider::sphere(0.5);
    let mut rb = RigidBody::new(1.0, true);
    rb.update_inertia_from_collider(&collider);
    world.add_component(e, Transform::new(at));
    world.add_component(e, rb);
    world.add_component(e, Velocity::default());
    world.add_component(e, collider);
    e
}

// ── Rigid bodies ───────────────────────────────────────────────────────────────────────────────

/// `PhysicsMaterial` overrides the collider's own material. A dead ball (restitution 0 on its
/// collider) made bouncy by the component is saved in the air, before it ever lands: after the
/// reopen it has to bounce, and its twin without the component has to not.
#[test]
fn a_material_component_still_makes_a_body_bounce() {
    let mut world = World::new();
    physics(&mut world);
    ground(&mut world);
    let dead = Collider::sphere(0.5).with_restitution(0.0);
    for (name, x) in [("bouncy", 0.0), ("dead", 4.0)] {
        let e = ball(&mut world, name, Vec3::new(x, 3.0, 0.0));
        world.add_component(e, dead.clone());
    }
    let bouncy = entity(&world, "bouncy");
    world.add_component(
        bouncy,
        PhysicsMaterial { restitution: 0.9, restitution_combine: CombineMode::Max, ..Default::default() },
    );
    step(&mut world, 10);
    assert!(position(&world, "bouncy").y > 1.5, "saved before the first impact");

    let mut world = reopen(&world, physics);
    let mut peak = [f32::MIN; 2];
    let mut landed = [false; 2];
    for _ in 0..150 {
        step(&mut world, 1);
        for (i, name) in ["bouncy", "dead"].into_iter().enumerate() {
            let (y, vy) = (position(&world, name).y, velocity(&world, name).y);
            if vy > 0.5 {
                landed[i] = true;
            }
            if landed[i] {
                peak[i] = peak[i].max(y);
            }
        }
    }
    assert!(landed[0] && peak[0] > 1.5, "the reopened ball bounced to {} — its material did not come back", peak[0]);
    assert!(!landed[1] || peak[1] < 0.8, "the control without the component bounced to {}", peak[1]);
}

/// A body pushed sideways by an `ExternalForce`, in free flight: the reopened one keeps
/// accelerating, along the same path as the original.
#[test]
fn an_external_force_keeps_pushing() {
    let mut world = World::new();
    physics(&mut world);
    let e = ball(&mut world, "pushed", Vec3::ZERO);
    world.query_mut::<&mut RigidBody>().get_mut(e).unwrap().use_gravity = false;
    world.add_component(e, ExternalForce::new(Vec3::new(6.0, 0.0, 0.0)));
    step(&mut world, 30);
    let at_save = velocity(&world, "pushed").x;
    assert!(at_save > 2.0, "the force was pushing before the save ({at_save})");

    let mut reopened = reopen(&world, physics);
    step(&mut world, 60);
    step(&mut reopened, 60);
    let (original, again) = (velocity(&world, "pushed"), velocity(&reopened, "pushed"));
    assert!(again.x > at_save + 4.0, "the reopened body stopped being pushed: {at_save} → {}", again.x);
    assert!((original - again).length() < 1e-3, "free flight goes the same way: {original} vs {again}");
}

/// A kinematic body sent somewhere by a `KinematicTarget` still goes there after a reopen.
#[test]
fn a_kinematic_target_is_still_reached() {
    let mut world = World::new();
    physics(&mut world);
    let e = named(&mut world, "lift");
    world.add_component(e, Transform::new(Vec3::ZERO));
    world.add_component(e, RigidBody::new_kinematic());
    world.add_component(e, Velocity::default());
    world.add_component(e, Collider::box_collider(Vec3::splat(0.5)));
    world.add_component(e, KinematicTarget::new(Vec3::new(3.0, 2.0, 0.0), Quat::from_rotation_y(0.5)));

    let mut world = reopen(&world, physics);
    step(&mut world, 3);
    let lift = entity(&world, "lift");
    let t = *world.query::<&Transform>().get(lift).unwrap();
    assert!((t.position - Vec3::new(3.0, 2.0, 0.0)).length() < 1e-3, "the lift stayed at {}", t.position);
    assert!(t.rotation.angle_between(Quat::from_rotation_y(0.5)) < 1e-3, "and it turned");
}

/// A `Breakable` still shatters after a reopen — and its health, half spent before the save,
/// still counts: a blast the full pool would survive breaks it.
#[test]
fn a_breakable_still_breaks_and_remembers_its_damage() {
    use gizmo::physics::components::{Explosion, ExplosionFalloff};
    let mut world = World::new();
    physics(&mut world);
    let e = named(&mut world, "crate");
    world.add_component(e, Transform::new(Vec3::new(2.0, 0.0, 0.0)));
    world.add_component(e, Velocity::default());
    world.add_component(e, Collider::box_collider(Vec3::splat(0.5)));
    world.add_component(
        e,
        Breakable { max_pieces: 6, threshold: 1.0, current_health: 40.0, max_health: 100.0, ..Default::default() },
    );

    let mut world = reopen(&world, physics);
    let blast = world.spawn();
    world.add_component(blast, Transform::new(Vec3::ZERO));
    world.add_component(
        blast,
        Explosion {
            force_radius: 10.0,
            force: 100.0,
            damage: 60.0,
            damage_radius: 10.0,
            falloff: ExplosionFalloff::None,
            offset: Vec3::ZERO,
            is_active: true,
        },
    );
    gizmo::physics::system::physics_explosion_system(&world, DT);
    world.apply_commands();
    let debris = world.query::<&RigidBody>().iter().count();
    assert!(debris > 1, "a crate with 40 of 100 health took 60 damage and did not shatter ({debris} bodies)");
    assert!(find(&world, "crate").is_none(), "the crate is replaced by its pieces");
}

// ── Characters, cars, divers ───────────────────────────────────────────────────────────────────

/// A character walking with a target velocity is saved mid-stride and walks on.
#[test]
fn a_character_keeps_walking() {
    use gizmo::physics::components::CharacterController;
    let mut world = World::new();
    physics(&mut world);
    ground(&mut world);
    let e = named(&mut world, "walker");
    world.add_component(
        e,
        CharacterController { target_velocity: Vec3::new(2.0, 0.0, 0.0), is_grounded: true, ..Default::default() },
    );
    world.add_component(e, Collider::capsule(0.3, 0.6));
    world.add_component(e, Transform::new(Vec3::new(0.0, 0.9, 0.0)));
    world.add_component(e, Velocity::default());
    let walk = |world: &World, frames: usize| {
        for _ in 0..frames {
            gizmo_physics_dynamics::systems::character_controller_system(world, DT);
        }
    };
    walk(&world, 30);
    let at_save = position(&world, "walker").x;
    assert!(at_save > 0.5, "walking before the save ({at_save})");

    let world = reopen(&world, physics);
    walk(&world, 60);
    let after = position(&world, "walker");
    assert!(after.x - at_save > 1.5, "the reopened character stopped: {at_save} → {}", after.x);
    assert!(after.y > 0.3, "and it stayed on the ground, not through it ({})", after.y);
}

/// A car under full throttle, saved at speed, drives on.
#[test]
fn a_car_keeps_driving() {
    use gizmo::physics::vehicle::{Axle, VehicleController, Wheel};
    let mut world = World::new();
    physics(&mut world);
    ground(&mut world);
    let e = named(&mut world, "car");
    let collider = Collider::box_collider(Vec3::new(0.9, 0.3, 2.0));
    let mut rb = RigidBody::new(900.0, true);
    rb.update_inertia_from_collider(&collider);
    let mut vc = VehicleController::new();
    for (x, z, axle, is_left) in [
        (-0.8, -1.4, Axle::Front, true),
        (0.8, -1.4, Axle::Front, false),
        (-0.8, 1.4, Axle::Rear, true),
        (0.8, 1.4, Axle::Rear, false),
    ] {
        vc.add_wheel(Wheel {
            attachment_local_pos: Vec3::new(x, 0.0, z),
            axle_type: axle,
            is_left,
            radius: 0.35,
            suspension_rest_length: 0.4,
            suspension_max_travel: 0.3,
            ..Default::default()
        });
    }
    vc.throttle_input = 1.0;
    world.add_component(e, rb);
    world.add_component(e, collider);
    world.add_component(e, Transform::new(Vec3::new(0.0, 0.6, 0.0)));
    world.add_component(e, Velocity::default());
    world.add_component(e, vc);
    let drive = |world: &mut World, frames: usize| {
        for _ in 0..frames {
            gizmo_physics_dynamics::systems::vehicle_controller_system(world, DT);
            gizmo::systems::cpu_physics_step_system(world, DT);
        }
    };
    drive(&mut world, 120);
    let (z_at_save, v_at_save) = (position(&world, "car").z, velocity(&world, "car").length());
    assert!(v_at_save > 1.0, "moving before the save ({v_at_save} m/s)");

    let mut reopened = reopen(&world, physics);
    drive(&mut world, 120);
    drive(&mut reopened, 120);
    let gone = z_at_save - position(&reopened, "car").z;
    let gone_original = z_at_save - position(&world, "car").z;
    assert!(gone > 2.0, "the reopened car stopped driving: {gone} m in 2 s");
    assert!(
        (gone - gone_original).abs() < 0.1 * gone_original,
        "and it drove like the original: {gone} m against {gone_original} m"
    );
}

/// A diver under water, half out of breath, is saved and keeps running out at the same rate.
#[test]
fn a_diver_keeps_running_out_of_air() {
    use gizmo::physics::world::{FluidZone, ZoneShape};
    use gizmo_physics_dynamics::oxygen::{oxygen_system, Oxygen};
    let water = |world: &mut World| {
        let mut pw = PhysicsWorld::new();
        pw.fluid_zones.push(FluidZone {
            shape: ZoneShape::Box { min: Vec3::new(-50.0, -30.0, -50.0), max: Vec3::new(50.0, 0.0, 50.0) },
            ..Default::default()
        });
        world.insert_resource(pw);
    };
    let mut world = World::new();
    water(&mut world);
    let e = named(&mut world, "diver");
    world.add_component(e, Transform::new(Vec3::new(0.0, -5.0, 0.0)));
    world.add_component(e, Oxygen { current: 20.0, max: 20.0, depletion_rate: 2.0, refill_rate: 5.0, head_offset: 0.6 });
    for _ in 0..120 {
        oxygen_system(&world, DT);
    }
    let reopened = reopen(&world, water);
    for _ in 0..60 {
        oxygen_system(&world, DT);
        oxygen_system(&reopened, DT);
    }
    let air = |w: &World| w.query::<&Oxygen>().get(entity(w, "diver")).expect("its oxygen").current;
    assert!((air(&reopened) - 14.0).abs() < 0.05, "20 s of air, 3 s under at 2/s: {} left", air(&reopened));
    assert_eq!(air(&reopened), air(&world), "the same breath, the same way");
}

// ── Soft bodies ────────────────────────────────────────────────────────────────────────────────

/// A cloth hanging by two corners, a rope fixed at one end and a soft block, all swinging or
/// falling in empty space: saved mid-motion, each continues exactly as the original does — the
/// soft solvers keep no state outside the component, so this one is compared node for node.
#[test]
fn soft_bodies_swing_on_from_where_they_were() {
    use gizmo::physics::soft_body::{Cloth, Rope, SoftBodyMesh};
    let mut world = World::new();
    physics(&mut world);

    let mut cloth = Cloth::new(6, 6, 0.2, 0.1);
    cloth.pin_node(30);
    cloth.pin_node(35);
    for n in &mut cloth.nodes {
        n.position.z += n.position.y * 0.5; // tilted, so it swings
        n.prev_position = n.position;
    }
    let e = named(&mut world, "cloth");
    world.add_component(e, Transform::new(Vec3::new(0.0, 10.0, 0.0)));
    world.add_component(e, cloth);

    let e = named(&mut world, "rope");
    world.add_component(e, Transform::new(Vec3::new(5.0, 10.0, 0.0)));
    world.add_component(e, Rope::new(Vec3::new(5.0, 10.0, 0.0), Vec3::X, 8, 0.25, 0.1, true, false));

    let e = named(&mut world, "block");
    world.add_component(e, Transform::new(Vec3::new(-5.0, 10.0, 0.0)));
    world.add_component(
        e,
        SoftBodyMesh::block(Vec3::new(-5.0, 10.0, 0.0), Vec3::splat(0.6), [2, 2, 2], 2.0, 5.0e4, 0.3).expect("a block"),
    );

    step(&mut world, 20);
    let mut reopened = reopen(&world, physics);
    step(&mut world, 40);
    step(&mut reopened, 40);

    let cloth_nodes = |w: &World| -> Vec<Vec3> {
        w.query::<&Cloth>().get(entity(w, "cloth")).expect("the cloth").nodes.iter().map(|n| n.position).collect()
    };
    let rope_nodes = |w: &World| -> Vec<Vec3> {
        w.query::<&Rope>().get(entity(w, "rope")).expect("the rope").nodes.iter().map(|n| n.position).collect()
    };
    let block_nodes = |w: &World| -> Vec<Vec3> {
        w.query::<&SoftBodyMesh>().get(entity(w, "block")).expect("the block").nodes.iter().map(|n| n.position).collect()
    };
    for (what, a, b) in [
        ("cloth", cloth_nodes(&world), cloth_nodes(&reopened)),
        ("rope", rope_nodes(&world), rope_nodes(&reopened)),
        ("soft block", block_nodes(&world), block_nodes(&reopened)),
    ] {
        assert_eq!(a.len(), b.len(), "{what}: node count");
        let worst = a.iter().zip(&b).map(|(p, q)| (*p - *q).length()).fold(0.0, f32::max);
        assert!(worst < 1e-5, "{what} went its own way after the reopen: a node is {worst} m off");
    }
    let start = rope_nodes(&reopened);
    assert!(start[0] == Vec3::new(5.0, 10.0, 0.0) && start[8].y < 9.5, "the rope hangs from its fixed end");
}

// ── Gameplay helpers ───────────────────────────────────────────────────────────────────────────

/// A repeating timer is saved between ticks and finishes when it would have.
#[test]
fn a_timer_finishes_on_time() {
    use gizmo::core::Timer;
    let mut world = World::new();
    let e = named(&mut world, "clock");
    let mut t = Timer::repeating(0.5);
    t.tick(0.3);
    world.add_component(e, t);
    let mut world = reopen(&world, |_| {});
    let e = entity(&world, "clock");
    let mut q = world.query_mut::<&mut Timer>();
    let mut t = q.get_mut(e).expect("the timer");
    assert_eq!(t.tick(0.15), 0, "0.45 s of 0.5");
    assert_eq!(t.tick(0.1), 1, "and done at 0.55");
}

/// Something with a lifetime, half spent, still goes when its time is up; something below the
/// kill height still goes when it falls through it.
#[test]
fn lifetimes_still_end() {
    use gizmo::systems::lifetime::{DespawnAfter, DespawnBelowY, LifetimeSystem};
    let mut world = World::new();
    physics(&mut world);
    let e = named(&mut world, "spark");
    world.add_component(e, Transform::new(Vec3::new(10.0, 0.0, 0.0)));
    world.add_component(e, DespawnAfter::secs(1.0));
    let e = ball(&mut world, "faller", Vec3::ZERO);
    world.add_component(e, DespawnBelowY::new(-3.0));
    for _ in 0..36 {
        LifetimeSystem.run(&world, DT);
        step(&mut world, 1);
    }
    assert!(find(&world, "spark").is_some() && find(&world, "faller").is_some(), "both alive at the save");

    let mut world = reopen(&world, physics);
    for _ in 0..20 {
        LifetimeSystem.run(&world, DT);
        step(&mut world, 1);
    }
    assert!(find(&world, "spark").is_some(), "0.93 s of 1 s: the spark is not due yet");
    for _ in 0..30 {
        LifetimeSystem.run(&world, DT);
        step(&mut world, 1);
    }
    assert!(find(&world, "spark").is_none(), "1.43 s of a 1 s lifetime and the spark is still here");
    assert!(find(&world, "faller").is_none(), "the ball fell past -3 and was not removed");
}

/// A spinning wheel, saved mid-turn, turns on from the same angle.
#[test]
fn a_spin_turns_on_from_its_angle() {
    use gizmo::systems::spin::{Spin, SpinSystem};
    let mut world = World::new();
    let e = named(&mut world, "wheel");
    world.add_component(e, Transform::new(Vec3::ZERO));
    world.add_component(e, Spin::new(Vec3::Y, 2.0).with_rest_rotation(Quat::from_rotation_x(0.3)));
    for _ in 0..30 {
        SpinSystem.run(&world, DT);
    }
    let reopened = reopen(&world, |_| {});
    for _ in 0..30 {
        SpinSystem.run(&world, DT);
        SpinSystem.run(&reopened, DT);
    }
    let rot = |w: &World| w.query::<&Transform>().get(entity(w, "wheel")).unwrap().rotation;
    let expected = Quat::from_rotation_x(0.3) * Quat::from_rotation_y(2.0);
    assert!(rot(&reopened).angle_between(rot(&world)) < 1e-5, "the same turn as the original");
    assert!(rot(&reopened).angle_between(expected) < 1e-3, "one second at 2 rad/s on its rest pose");
}

/// An `AutoBoxCollider` is a request resolved on the first frame. A scene saved before that frame
/// — a game's setup, an editor scene that has not been played — keeps the request, and the
/// reopened world sizes the collider from the scale.
#[test]
fn an_unresolved_auto_box_collider_is_resolved_after_a_reopen() {
    use gizmo::systems::auto_collider::{AutoBoxCollider, AutoBoxColliderSystem};
    let mut world = World::new();
    let e = named(&mut world, "plank");
    world.add_component(e, Transform::new(Vec3::ZERO).with_scale(Vec3::new(4.0, 0.4, 1.0)));
    world.add_component(e, Collider::box_collider(Vec3::ONE));
    world.add_component(e, AutoBoxCollider::new());
    let mut world = reopen(&world, |w| w.insert_resource(CommandQueue::default()));
    AutoBoxColliderSystem.run(&world, DT);
    world.apply_commands();
    let plank = entity(&world, "plank");
    let shape = world.query::<&Collider>().get(plank).unwrap().shape.clone();
    assert!(
        matches!(shape, ColliderShape::Box(b) if b.half_extents == Vec3::new(4.0, 0.4, 1.0)),
        "the plank's collider was not sized from its scale: {shape:?}"
    );
}

/// The look direction of an FPS camera lives in its `FpsLook` (the camera is written from it each
/// frame): a reopened camera looks where it looked.
#[test]
fn an_fps_camera_looks_where_it_looked() {
    use gizmo::renderer::components::Camera;
    use gizmo::systems::fps_look::{FpsLook, FpsLookSystem};
    let mut world = World::new();
    let e = named(&mut world, "eye");
    world.add_component(e, Transform::new(Vec3::new(0.0, 1.7, 0.0)));
    world.add_component(e, Camera::new(1.0, 0.1, 100.0, 0.0, 0.0, true));
    world.add_component(e, FpsLook::new().looking(1.0, -0.3));
    let world = reopen(&world, |_| {});
    FpsLookSystem.run(&world, DT);
    let eye = entity(&world, "eye");
    let cam = world.query::<&Camera>().get(eye).cloned().expect("the camera");
    assert!((cam.yaw - 1.0).abs() < 1e-5 && (cam.pitch + 0.3).abs() < 1e-5, "looking at {} / {}", cam.yaw, cam.pitch);
}

/// A guard's behaviour tree, saved halfway through a wait, finishes the wait after the reopen and
/// does what comes next — the tree is data, its running state with it, and the leaves are names
/// the reopened world's `BtLibrary` resolves.
#[test]
fn a_behaviour_tree_goes_on_from_where_it_was() {
    use gizmo::ai::{behavior_tree_system, BehaviorTree, BtLibrary, BtNode, BtStatus};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let alarms = Arc::new(AtomicUsize::new(0));
    let library = |alarms: Arc<AtomicUsize>| {
        move |world: &mut World| {
            let alarms = alarms.clone();
            world.insert_resource(
                BtLibrary::new()
                    .with_condition("on_duty", |_, _| true)
                    .with_action("raise_alarm", move |_, _, _| {
                        alarms.fetch_add(1, Ordering::SeqCst);
                        BtStatus::Success
                    }),
            );
        }
    };
    let mut world = World::new();
    library(Arc::new(AtomicUsize::new(0)))(&mut world);
    let e = named(&mut world, "guard");
    world.add_component(
        e,
        BehaviorTree::new(BtNode::sequence([
            BtNode::condition("on_duty"),
            BtNode::wait(1.0),
            BtNode::action("raise_alarm"),
        ])),
    );
    for _ in 0..36 {
        behavior_tree_system(&mut world, DT);
    }

    let mut world = reopen(&world, library(alarms.clone()));
    for _ in 0..20 {
        behavior_tree_system(&mut world, DT);
    }
    assert_eq!(alarms.load(Ordering::SeqCst), 0, "0.93 s of a 1 s wait: not yet");
    for _ in 0..6 {
        behavior_tree_system(&mut world, DT);
    }
    assert_eq!(alarms.load(Ordering::SeqCst), 1, "1.03 s: the wait went on from 0.6 s, and the alarm went off");
}
