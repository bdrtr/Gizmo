//! **A ball thrown along a floor slides, then rolls at 5/7 of its speed** — the textbook result,
//! and the one an inertia that does not match the collider gets wrong.
//!
//! `RigidBody::new` cannot know the shape, so it starts with a placeholder inertia, and the ECS
//! physics step used to keep it: a 20 cm, 6 kg ball (I = 0.096) turned as if I were ~1. Friction
//! could not spin it up, so it slid the whole lane and lost 80 % of its speed in 1.5 s
//! (`lobut`, measured). The step now derives the tensor from the collider the first time.

use gizmo_core::Entity;
use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

const HZ: f32 = 240.0;
const R: f32 = 0.2;
const V0: f32 = 9.6;

fn lane(ball_rb: RigidBody) -> (World, Entity) {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let floor = world.spawn();
    world.add_component(floor, Transform::new(Vec3::new(0.0, -0.1, -9.5)));
    world.add_component(floor, RigidBody::new_static());
    world.add_component(floor, Collider::box_collider(Vec3::new(0.9, 0.1, 10.5)));
    let ball = world.spawn();
    world.add_component(ball, Transform::new(Vec3::new(0.0, R, 0.0)));
    world.add_component(ball, ball_rb);
    world.add_component(ball, Velocity::new(Vec3::new(0.0, 0.0, -V0)));
    world.add_component(ball, Collider::sphere(R));
    (world, ball)
}

fn run(world: &mut World, seconds: f32) {
    for _ in 0..(seconds * HZ) as usize {
        gizmo_physics_rigid::system::physics_step_system(world, 1.0 / HZ);
        world.apply_commands();
    }
}

#[test]
fn a_thrown_ball_settles_into_rolling_at_five_sevenths_of_its_speed() {
    let (mut world, ball) = lane(RigidBody::new(6.0, true));
    run(&mut world, 1.2);
    let v = *world.borrow::<Velocity>().get(ball).unwrap();
    let speed = -v.linear.z;
    let expected = V0 * 5.0 / 7.0;
    assert!(
        (speed - expected).abs() / expected < 0.04,
        "rolling speed {speed:.2} m/s, the textbook says {expected:.2}"
    );
    let rim = -v.angular.x * R;
    assert!((rim - speed).abs() / speed < 0.03, "not rolling: ω·r = {rim:.2}, v = {speed:.2}");
    let inertia = world.borrow::<RigidBody>().get(ball).unwrap().local_inertia;
    let solid = 0.4 * 6.0 * R * R;
    assert!((inertia - Vec3::splat(solid)).length() < 1e-4, "inertia {inertia:?}, a solid sphere's is {solid}");
}

/// An inertia set by hand is the author's, and is not replaced.
#[test]
fn an_inertia_set_by_hand_is_kept() {
    let mut rb = RigidBody::new(6.0, true);
    rb.local_inertia = Vec3::splat(0.5);
    let (mut world, ball) = lane(rb);
    run(&mut world, 0.1);
    assert_eq!(world.borrow::<RigidBody>().get(ball).unwrap().local_inertia, Vec3::splat(0.5));
}
