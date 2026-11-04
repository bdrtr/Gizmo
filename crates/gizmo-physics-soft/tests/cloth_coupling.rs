//! Cloth pushes back: a hammock holds a ball.
//!
//! Cloth was strictly one-way — it draped over rigid bodies and pushed none back — so a ball
//! dropped on a hammock went straight through it, the cloth parting around it on the way down
//! (measured: a 1 kg ball dropped 0.8 m onto a hammock 2 m up ends on the floor, y 0.25).
//! `cloth_step_system` now resolves each contact against the body's motion and pays the body
//! back (`Cloth::step_coupled`).
//!
//! What is asserted is that the ball is **held**, not that it settles: the cloth has next to no
//! internal damping, so a ball keeps bouncing on it at about the speed it arrived with (measured,
//! 5 kg: 4.3 m/s peaks a second after a 4 m/s drop) — energy kept, not gained.

#![cfg(feature = "rigid-coupling")]

use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;
use gizmo_physics_soft::cloth::Cloth;

const G: Vec3 = Vec3::new(0.0, -9.81, 0.0);
const N: usize = 11;
const HEIGHT: f32 = 2.0;

/// A 2 m × 2 m hammock pinned on all four edges at `HEIGHT`, and a ball of `mass` 0.8 m above it.
fn scene(mass: f32) -> (World, u32) {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(G));
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Velocity::default());
    world.add_component(ground, Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)));

    let spacing = 2.0 / (N - 1) as f32;
    let mut cloth = Cloth::new(N, N, spacing, 0.02);
    for n in &mut cloth.nodes {
        let p = Vec3::new(n.position.x - 1.0, HEIGHT, n.position.y - 1.0);
        n.position = p;
        n.prev_position = p;
    }
    for i in 0..N {
        for j in [0, N - 1] {
            cloth.pin_node(j * N + i);
            cloth.pin_node(i * N + j);
        }
    }
    let c = world.spawn();
    world.add_component(c, cloth);

    let ball = world.spawn();
    let col = Collider::sphere(0.25);
    let mut rb = RigidBody::new(mass, true);
    rb.update_inertia_from_collider(&col);
    world.add_component(ball, Transform::new(Vec3::new(0.0, HEIGHT + 0.8, 0.0)));
    world.add_component(ball, rb);
    world.add_component(ball, Velocity::default());
    world.add_component(ball, col);
    (world, ball.id())
}

/// The lowest the ball gets over three seconds.
fn lowest(mass: f32) -> f32 {
    let (world, ball) = scene(mass);
    let dt = 1.0 / 60.0;
    let mut lowest = f32::MAX;
    for _ in 0..180 {
        gizmo_physics_rigid::system::physics_step_system(&world, dt);
        gizmo_physics_soft::system::cloth_step_system(&world, dt, G);
        lowest = lowest.min(world.borrow::<Transform>().get(ball).unwrap().position.y);
    }
    lowest
}

#[test]
fn a_hammock_holds_a_ball() {
    let y = lowest(1.0);
    assert!(y > HEIGHT - 0.5, "a 1 kg ball went through the hammock: lowest y {y:.2}");
}

#[test]
fn a_hammock_holds_a_heavy_ball() {
    let y = lowest(5.0);
    assert!(y > HEIGHT - 1.0, "a 5 kg ball went through the hammock: lowest y {y:.2}");
}
