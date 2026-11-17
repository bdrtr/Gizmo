//! `RigidBody::contact_spin_share`: the share of a contact impulse's turning effect a body takes.

use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 120.0;

/// A box, turned 30°, slides into a wall at 10 m/s with no gravity; returns its spin and how far
/// past the wall's face it got after a second.
fn hit_the_wall(share: f32) -> (f32, f32) {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));
    let wall = world.spawn();
    world.add_component(wall, Transform::new(Vec3::new(0.0, 0.0, -5.5)));
    world.add_component(wall, RigidBody::new_static());
    world.add_component(wall, Velocity::default());
    world.add_component(wall, Collider::box_collider(Vec3::new(10.0, 2.0, 0.5)));

    let body = world.spawn();
    world.add_component(body, Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.52)));
    let mut rb = RigidBody::new(1200.0, true);
    rb.calculate_box_inertia(1.8, 1.2, 4.2);
    rb.contact_spin_share = share;
    world.add_component(body, rb);
    world.add_component(body, Velocity::new(Vec3::new(0.0, 0.0, -10.0)));
    world.add_component(body, Collider::box_collider(Vec3::new(0.9, 0.6, 2.1)));

    let mut deepest = f32::NEG_INFINITY;
    for _ in 0..120 {
        gizmo_physics_rigid::system::physics_step_system(&world, DT);
        world.apply_commands();
        let t = world.query::<&Transform>().get(body).copied().unwrap();
        // The nearest point of the turned box to the wall, along −Z.
        let corners = [(-0.9, -2.1), (0.9, -2.1), (-0.9, 2.1), (0.9, 2.1)];
        let front = corners
            .iter()
            .map(|&(x, z)| (t.position + t.rotation * Vec3::new(x, 0.0, z)).z)
            .fold(f32::INFINITY, f32::min);
        deepest = deepest.max(-5.0 - front);
    }
    let spin = world.query::<&Velocity>().get(body).unwrap().angular.length();
    (spin, deepest)
}

/// **Four tenths of the spin, and the wall still holds** (`nfsu2-gizmo` MOTOR-NOTLARI #22b): a car
/// that brushes a wall should not be sent spinning off it. Default `1.0` is the real inertia.
#[test]
fn a_smaller_contact_spin_share_turns_a_body_less_and_still_stops_it() {
    let (full, full_depth) = hit_the_wall(1.0);
    let (share, share_depth) = hit_the_wall(0.4);
    assert!(full > 0.2, "the turned box did not spin off the wall: {full}");
    assert!(share < full * 0.75, "share 0.4 spun at {share} rad/s against {full}");
    for (depth, what) in [(full_depth, "1.0"), (share_depth, "0.4")] {
        assert!(depth < 0.05, "with share {what} the box went {depth} m into the wall");
    }
}
