//! A steady push gives the same motion at every frame rate.
//!
//! A game drives physics with a fixed step: each rendered frame runs zero, one or several steps.
//! The body's force accumulators are emptied by every step, so a force written into them once a
//! frame acted on only some of the steps. `ExternalForce` is applied on every step it is on the
//! body for.

use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{ExternalForce, RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

const STEP: f32 = 1.0 / 60.0;

#[derive(Clone, Copy, Debug)]
enum Push {
    /// `force_accumulator = F` every frame.
    Assign,
    /// `force_accumulator += F` every frame.
    Add,
    /// An `ExternalForce` component, set once.
    Component,
}

/// The speed a 1 kg body reaches after one simulated second of 10 N, at `fps` frames a second.
fn speed_after_one_second(fps: f32, push: Push) -> f32 {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));
    let col = Collider::sphere(0.5);
    let mut rb = RigidBody::new(1.0, false);
    rb.linear_damping = 0.0;
    rb.update_inertia_from_collider(&col);
    let body = world.spawn_bundle((Transform::new(Vec3::ZERO), rb, Velocity::default(), col));
    if let Push::Component = push {
        let e = world.live(body).unwrap();
        world.add_component(e, ExternalForce::new(Vec3::new(10.0, 0.0, 0.0)));
    }

    let frame = 1.0 / fps;
    let mut accumulator = 0.0f32;
    let mut stepped = 0.0f32;
    while stepped < 1.0 - 1e-4 {
        match push {
            Push::Assign => world.borrow_mut::<RigidBody>().get_mut(body).unwrap().force_accumulator = Vec3::new(10.0, 0.0, 0.0),
            Push::Add => world.borrow_mut::<RigidBody>().get_mut(body).unwrap().force_accumulator += Vec3::new(10.0, 0.0, 0.0),
            Push::Component => {}
        }
        accumulator += frame;
        while accumulator >= STEP - 1e-6 && stepped < 1.0 - 1e-4 {
            gizmo_physics_rigid::system::physics_step_system(&world, STEP);
            accumulator -= STEP;
            stepped += STEP;
        }
    }
    world.borrow::<Velocity>().get(body).unwrap().linear.x
}

#[test]
fn an_external_force_is_the_same_push_at_every_frame_rate() {
    if std::env::var_os("PUSH_TABLE").is_some() {
        for push in [Push::Assign, Push::Add, Push::Component] {
            let row: Vec<String> = [30.0, 60.0, 144.0].iter().map(|&f| format!("{:.1}", speed_after_one_second(f, push))).collect();
            eprintln!("{push:?}: 30/60/144 fps -> {}", row.join(" / "));
        }
    }
    for fps in [30.0, 60.0, 144.0] {
        let v = speed_after_one_second(fps, Push::Component);
        assert!((v - 10.0).abs() < 0.2, "{fps} fps: 10 N on 1 kg for 1 s gave {v:.2} m/s, not 10");
    }
}

/// `at_point` is the force plus the torque of its lever arm.
#[test]
fn a_force_off_centre_turns_the_body() {
    let push = ExternalForce::at_point(Vec3::new(0.0, 0.0, -10.0), Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO);
    assert_eq!(push.force, Vec3::new(0.0, 0.0, -10.0));
    assert_eq!(push.torque, Vec3::new(0.0, 10.0, 0.0));
}
