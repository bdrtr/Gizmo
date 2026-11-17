//! `RigidBody::contact_mass`: a kinematic body that counts as a finite mass in contacts.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 120.0;

/// A 1 200 kg car at 15 m/s runs into a kinematic car standing still, moved by its velocity, with
/// no gravity. Returns the two velocities along the line of travel after a second, and the sum of
/// the kinematic car's `contact_response` over it.
fn collide(contact_mass: Option<f32>) -> (f32, f32, f32) {
    collide_with(contact_mass, false)
}

/// `dynamic`: the other car is an ordinary dynamic body of 1 200 kg instead — the reference.
fn collide_with(contact_mass: Option<f32>, dynamic: bool) -> (f32, f32, f32) {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));
    let rail = world.spawn();
    world.add_component(rail, Transform::new(Vec3::new(0.0, 0.0, -6.0)));
    let mut rb = if dynamic { RigidBody::new(1200.0, true) } else { RigidBody::new_kinematic() };
    if dynamic {
        rb.calculate_box_inertia(1.8, 1.2, 4.2);
    }
    rb.contact_mass = contact_mass;
    world.add_component(rail, rb);
    world.add_component(rail, Velocity::default());
    world.add_component(rail, Collider::box_collider(Vec3::new(0.9, 0.6, 2.1)));

    let car = world.spawn();
    world.add_component(car, Transform::new(Vec3::ZERO));
    let mut rb = RigidBody::new(1200.0, true);
    rb.calculate_box_inertia(1.8, 1.2, 4.2);
    world.add_component(car, rb);
    world.add_component(car, Velocity::new(Vec3::new(0.0, 0.0, -15.0)));
    world.add_component(car, Collider::box_collider(Vec3::new(0.9, 0.6, 2.1)));

    let mut reported = 0.0;
    for _ in 0..120 {
        gizmo_physics_rigid::system::physics_step_system(&world, DT);
        world.apply_commands();
        reported -= world.query::<&RigidBody>().get(rail).unwrap().contact_response.z;
    }
    let along = |e| -world.query::<&Velocity>().get(e).unwrap().linear.z;
    (along(car), along(rail), reported)
}

/// **A car runs into a rail car of the same mass as into a car** (`nfsu2-gizmo` MOTOR-NOTLARI
/// #20): it ends as it would against a dynamic car of that mass — momentum kept, the car not
/// thrown back; without a contact mass the kinematic body is a wall, and the car bounces off it.
#[test]
fn a_kinematic_body_with_a_contact_mass_shares_the_momentum_of_a_collision() {
    let (car, rail, reported) = collide(Some(1200.0));
    let (ref_car, ref_other, _) = collide_with(None, true);
    assert!(
        (car - ref_car).abs() < 0.3 && (rail - ref_other).abs() < 0.3,
        "against a rail car {car} / {rail} m/s, against a dynamic car {ref_car} / {ref_other}"
    );
    let momentum = car + rail;
    assert!((momentum - 15.0).abs() < 1.5, "momentum along the line {momentum} m/s·(1 200 kg), was 15: car {car}, rail {rail}");
    assert!((0.0..10.0).contains(&car), "the car went on at {car} m/s");
    assert!(rail > 5.0, "the rail car took {rail} m/s");
    assert!((reported - rail).abs() < 1e-3, "reported {reported} m/s, took {rail}");

    let (wall_car, wall, wall_reported) = collide(None);
    assert_eq!((wall, wall_reported), (0.0, 0.0), "nothing taken or reported without a contact mass");
    assert!(wall_car < 0.0, "without a contact mass the car bounces off: {wall_car}");
}
