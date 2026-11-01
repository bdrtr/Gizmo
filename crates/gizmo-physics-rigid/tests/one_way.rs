//! One-way platforms (`Collider::one_way`): a body jumps up through one from below and lands on
//! it from above. Each run is paired with the same scene and a SOLID platform, because "the ball
//! ended up on top" proves nothing unless a solid platform would have stopped it.

use gizmo_core::world::World;
use gizmo_math::{Quat, Vec2, Vec3};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

const PLATFORM_Y: f32 = 2.0; // centre; 0.5 thick → top at 2.25, underside at 1.75
const R: f32 = 0.5;

/// A ball resting on the ground is launched straight up at 9 m/s (apex ≈ 4.6 m, well above the
/// platform). Returns where it is after three seconds.
fn launch(platform: Collider, platform_rotation: Quat) -> Vec3 {
    let mut w = World::new();
    w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let solid = |w: &mut World, pos: Vec3, rot: Quat, c: Collider| {
        let e = w.spawn();
        let mut t = Transform::new(pos);
        t.rotation = rot;
        w.add_component(e, t);
        w.add_component(e, RigidBody::new_static());
        w.add_component(e, Velocity::default());
        w.add_component(e, c);
    };
    solid(&mut w, Vec3::new(0.0, -0.5, 0.0), Quat::IDENTITY, Collider::rect_2d(Vec2::new(20.0, 1.0), 2.0));
    solid(&mut w, Vec3::new(0.0, PLATFORM_Y, 0.0), platform_rotation, platform);

    let ball = w.spawn();
    w.add_component(ball, Transform::new(Vec3::new(0.0, R, 0.0)));
    w.add_component(ball, RigidBody::new(1.0, true).planar_xy());
    w.add_component(ball, Velocity::new(Vec3::new(0.0, 9.0, 0.0)));
    w.add_component(ball, Collider::circle_2d(R));
    for _ in 0..180 {
        gizmo_physics_rigid::system::physics_step_system(&w, 1.0 / 60.0);
    }
    let p = w.borrow::<Transform>().get(ball.id()).unwrap().position;
    p
}

fn platform() -> Collider {
    Collider::rect_2d(Vec2::new(4.0, 0.5), 2.0)
}

#[test]
fn a_ball_jumps_up_through_a_one_way_platform_and_lands_on_it() {
    let solid = launch(platform(), Quat::IDENTITY);
    assert!(solid.y < 1.0, "a SOLID platform should have sent it back to the ground: {solid:?}");

    let one_way = launch(platform().one_way(Vec3::Y), Quat::IDENTITY);
    let top = PLATFORM_Y + 0.25 + R;
    assert!((one_way.y - top).abs() < 0.05, "it should rest on the platform's top (y = {top}): {one_way:?}");
}

/// The direction is the collider's own and turns with the body: a platform turned upside down
/// stops things from below and lets them out the top — so the ball bounces off its underside.
#[test]
fn the_one_way_direction_turns_with_the_body() {
    let flipped = launch(platform().one_way(Vec3::Y), Quat::from_rotation_z(std::f32::consts::PI));
    assert!(flipped.y < 1.0, "an upside-down one-way platform let the ball through from below: {flipped:?}");
}

#[test]
fn the_rule_keeps_landings_and_drops_everything_else() {
    let up = Vec3::Y;
    assert!(Collider::one_way_keeps(up, Vec3::Y, Vec3::new(0.0, -3.0, 0.0)), "falling onto it from above");
    assert!(!Collider::one_way_keeps(up, -Vec3::Y, Vec3::new(0.0, 3.0, 0.0)), "rising into it from below");
    assert!(!Collider::one_way_keeps(up, Vec3::Y, Vec3::new(0.0, 3.0, 0.0)), "still rising while inside");
    assert!(!Collider::one_way_keeps(up, Vec3::X, Vec3::ZERO), "a side hit passes");
    assert_eq!(platform().one_way(Vec3::ZERO).one_way, None, "no direction, no one-way");
    assert_eq!(platform().one_way(Vec3::new(0.0, 5.0, 0.0)).one_way, Some(Vec3::Y));
}
