//! The 2-D shapes — `Collider::{polygon_2d, polyline_2d, circle_2d, rect_2d}` — on bodies locked to
//! the XY plane, end to end through the ECS physics step.
//!
//! Each test asserts a position the geometry dictates, not merely "it did not explode": a shape
//! built in the wrong frame, or a strip whose triangles face nowhere, lands somewhere else.

use gizmo_core::world::World;
use gizmo_math::{Vec2, Vec3};
use gizmo_physics_core::{Collider, ColliderShape, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

fn world() -> World {
    let mut w = World::new();
    w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    w
}

fn spawn(w: &mut World, pos: Vec3, body: RigidBody, col: Collider) -> u32 {
    let e = w.spawn();
    w.add_component(e, Transform::new(pos));
    w.add_component(e, body);
    w.add_component(e, Velocity::default());
    w.add_component(e, col);
    e.id()
}

fn dynamic(mass: f32, col: &Collider) -> RigidBody {
    let mut rb = RigidBody::new(mass, true).planar_xy();
    rb.update_inertia_from_collider(col);
    rb
}

fn step(w: &World, n: usize) {
    for _ in 0..n {
        gizmo_physics_rigid::system::physics_step_system(w, 1.0 / 60.0);
    }
}

fn pos(w: &World, id: u32) -> Vec3 {
    w.borrow::<Transform>().get(id).unwrap().position
}

/// A triangle on its flat base settles on a rectangle ground: base on the ground's top, upright,
/// on the plane.
#[test]
fn a_triangle_settles_on_its_base() {
    let mut w = world();
    spawn(&mut w, Vec3::new(0.0, -0.5, 0.0), RigidBody::new_static(), Collider::rect_2d(Vec2::new(20.0, 1.0), 1.0));
    // Base from (-1, 0) to (1, 0), apex at (0, 1.5); the origin sits on the base's midpoint.
    let tri = Collider::polygon_2d(&[Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.5)], 1.0);
    let t = spawn(&mut w, Vec3::new(0.0, 2.0, 0.0), dynamic(1.0, &tri), tri);
    step(&w, 240);

    let p = pos(&w, t);
    let rot = w.borrow::<Transform>().get(t).unwrap().rotation;
    assert!(p.y.abs() < 0.03, "the base should rest on the ground's top (y = 0), the origin is at {p:?}");
    assert_eq!(p.z, 0.0, "off the plane");
    assert!((rot * Vec3::Y).y > 0.999, "it tipped over: up is {:?}", rot * Vec3::Y);
}

/// A ball on a polyline slope rolls downhill and stays on the line rather than through it.
#[test]
fn a_ball_rolls_down_a_polyline_slope() {
    let mut w = world();
    // Down to the right: (−10, 5) → (0, 0) → (10, −1).
    let line = [Vec2::new(-10.0, 5.0), Vec2::new(0.0, 0.0), Vec2::new(10.0, -1.0)];
    spawn(&mut w, Vec3::ZERO, RigidBody::new_static(), Collider::polyline_2d(&line, 2.0));
    let ball = Collider::circle_2d(0.5);
    let b = spawn(&mut w, Vec3::new(-6.0, 4.0, 0.0), dynamic(1.0, &ball), ball);
    step(&w, 120);

    let p = pos(&w, b);
    assert!(p.x > -4.0, "it did not roll downhill: {p:?}");
    // Height of the line under the ball.
    let ground = if p.x < 0.0 { -0.5 * p.x } else { -0.1 * p.x };
    assert!(p.y > ground, "the ball is below the line it rolls on: {p:?}, line at {ground}");
    assert!(p.y < ground + 1.5, "the ball is not on the line at all: {p:?}, line at {ground}");
    assert_eq!(p.z, 0.0);
}

#[test]
fn the_shapes_are_built_in_the_plane_with_their_depth() {
    let tri = Collider::polygon_2d(&[Vec2::ZERO, Vec2::new(2.0, 0.0), Vec2::new(0.0, 1.0)], 0.4);
    let ColliderShape::ConvexHull(h) = &tri.shape else { panic!("a hull") };
    assert_eq!(h.vertices.len(), 6, "a triangle prism has six corners");
    assert!(h.vertices.iter().all(|v| (v.z.abs() - 0.2).abs() < 1e-6), "depth 0.4 → z = ±0.2");

    let ColliderShape::Box(b) = Collider::rect_2d(Vec2::new(3.0, 1.0), 2.0).shape else { panic!("a box") };
    assert_eq!(b.half_extents, Vec3::new(1.5, 0.5, 1.0));

    let ColliderShape::TriMesh(m) = Collider::polyline_2d(&[Vec2::ZERO, Vec2::X, Vec2::new(2.0, 1.0)], 1.0).shape else {
        panic!("a mesh")
    };
    assert_eq!(m.indices.len(), 12, "two segments → two quads → four triangles");
}
