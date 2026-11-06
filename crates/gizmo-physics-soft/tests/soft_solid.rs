//! A soft body is a solid you can drop, stand on the floor and stack.
//!
//! Measured before 2026-11-04, a 1 m block of 10 kg (E = 20 000 Pa):
//!
//! * **dropped** 1.5 m onto the floor it exploded inside a second (nodes at 10¹⁵ m) — one
//!   explicit step of 1/60 s is far past what the material is stable for, at E = 1000 too;
//! * **resting** on the floor through short (sub-)steps it sank 0.33 m in ten seconds — the
//!   collider sweep ignores moves under 1e-5 m and nothing put a node that was inside back out;
//! * a node **leaving** the floor was held on it while its velocity grew (a ray from the surface
//!   meets it at distance zero, and that was taken as a hit);
//! * **stacked**, one block fell straight through the other — soft bodies only met rigid ones.

#![cfg(feature = "rigid-coupling")]

use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;
use gizmo_physics_soft::soft_contact::{resolve_soft_contacts, surface_faces};
use gizmo_physics_soft::SoftBodyMesh;

const G: Vec3 = Vec3::new(0.0, -9.81, 0.0);
const DT: f32 = 1.0 / 60.0;

fn jelly(min: Vec3, e: f32) -> SoftBodyMesh {
    let mut b = SoftBodyMesh::block(min, Vec3::ONE, [2, 2, 2], 10.0, e, 0.3).unwrap();
    b.damping = 0.5;
    b
}

fn floor_world() -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(G));
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)),
    ));
    world
}

fn run(world: &World, seconds: f32) {
    for _ in 0..(seconds / DT) as usize {
        gizmo_physics_rigid::system::physics_step_system(world, DT);
        gizmo_physics_soft::system::soft_body_step_system(world, DT, G);
    }
}

fn span_y(b: &SoftBodyMesh) -> (f32, f32) {
    b.nodes.iter().fold((f32::MAX, f32::MIN), |(lo, hi), n| (lo.min(n.position.y), hi.max(n.position.y)))
}

fn fastest(b: &SoftBodyMesh) -> f32 {
    b.nodes.iter().map(|n| n.velocity.length()).fold(0.0, f32::max)
}

#[test]
fn a_block_is_conforming_and_its_surface_faces_out() {
    let b = SoftBodyMesh::block(Vec3::ZERO, Vec3::new(2.0, 1.0, 3.0), [2, 1, 3], 6.0, 1000.0, 0.3).unwrap();
    assert_eq!(b.nodes.len(), 3 * 2 * 4);
    assert_eq!(b.elements.len(), 6 * 2 * 3);
    let faces = surface_faces(&b);
    // Two triangles per square of the box's skin: 2·(2·1 + 1·3 + 2·3) squares.
    assert_eq!(faces.len(), 2 * 2 * (2 + 3 + 6), "inner faces left unmatched: the split is not conforming");
    let centre = Vec3::new(1.0, 0.5, 1.5);
    for f in faces {
        let [a, bb, c] = f.map(|i| b.nodes[i as usize].position);
        assert!((bb - a).cross(c - a).dot(a - centre) > 0.0, "a surface face points into the body");
    }
}

#[test]
fn a_dropped_block_lands_and_settles() {
    let mut world = floor_world();
    let id = world.spawn_bundle((jelly(Vec3::new(-0.5, 1.5, -0.5), 20_000.0),)).id();
    run(&world, 6.0);
    let q = world.borrow::<SoftBodyMesh>();
    let b = q.get(id).unwrap();
    let (lo, hi) = span_y(b);
    assert!(b.nodes.iter().all(|n| n.position.is_finite()), "the block blew up");
    assert!(lo > -0.05 && hi < 1.25 && hi - lo > 0.85, "not a block on the floor: y {lo:.2}..{hi:.2}");
    assert!(fastest(b) < 0.3, "still moving at {:.2} m/s after 6 s", fastest(b));
}

#[test]
fn a_block_on_the_floor_does_not_sink() {
    let mut world = floor_world();
    let id = world.spawn_bundle((jelly(Vec3::new(-0.5, 0.0, -0.5), 20_000.0),)).id();
    run(&world, 6.0);
    let (lo, _) = span_y(world.borrow::<SoftBodyMesh>().get(id).unwrap());
    assert!(lo > -0.02, "the resting block sank to {lo:.3}");
}

#[test]
fn a_block_dropped_on_a_block_rests_on_it() {
    let mut world = floor_world();
    let low = world.spawn_bundle((jelly(Vec3::new(-0.5, 0.0, -0.5), 20_000.0),)).id();
    let high = world.spawn_bundle((jelly(Vec3::new(-0.5, 1.5, -0.5), 20_000.0),)).id();
    run(&world, 6.0);
    let q = world.borrow::<SoftBodyMesh>();
    let (_, low_top) = span_y(q.get(low).unwrap());
    let (high_bottom, high_top) = span_y(q.get(high).unwrap());
    assert!(high_bottom > low_top - 0.1, "the upper block sank into the lower: its bottom {high_bottom:.2}, the lower's top {low_top:.2}");
    assert!(high_top > 1.8, "the stack is not two blocks high: top {high_top:.2}");
}

/// Soft against soft trades momentum and makes none.
#[test]
fn soft_contact_conserves_momentum() {
    let mut a = jelly(Vec3::new(-1.2, 0.0, 0.0), 5000.0);
    let mut b = jelly(Vec3::new(0.2, 0.3, 0.1), 5000.0);
    // Undamped: `damping` takes momentum out of each body by itself, which is not the contact's.
    a.damping = 1.0;
    b.damping = 1.0;
    for n in &mut a.nodes {
        n.velocity = Vec3::new(2.0, 0.0, 0.0);
    }
    for n in &mut b.nodes {
        n.velocity = Vec3::new(-1.0, 0.0, 0.0);
    }
    let momentum = |a: &SoftBodyMesh, b: &SoftBodyMesh| -> Vec3 { a.nodes.iter().chain(&b.nodes).map(|n| n.velocity * n.mass).sum() };
    let before = momentum(&a, &b);
    let mut touched = 0;
    let n = a.stable_substeps(DT).max(b.stable_substeps(DT));
    for _ in 0..60 {
        for _ in 0..n {
            a.step(DT / n as f32, Vec3::ZERO, &[]);
            b.step(DT / n as f32, Vec3::ZERO, &[]);
            touched += resolve_soft_contacts(&mut [&mut a, &mut b]);
        }
    }
    let after = momentum(&a, &b);
    assert!(touched > 0, "the blocks never met");
    assert!((after - before).length() < 1e-3 * before.length().max(1.0), "momentum {before:?} became {after:?}");
    let (ax, bx) = (a.nodes.iter().map(|n| n.position.x).fold(f32::MIN, f32::max), b.nodes.iter().map(|n| n.position.x).fold(f32::MAX, f32::min));
    assert!(ax < bx + 0.1, "the blocks passed into each other: a reaches {ax:.2}, b starts at {bx:.2}");
}

/// A node on a surface, moving away from it, is not stopped by it.
#[test]
fn a_node_leaving_the_floor_is_not_held_on_it() {
    let floor = [(gizmo_physics_core::BodyHandle::from_id(1), Transform::new(Vec3::new(0.0, -0.5, 0.0)), Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)))];
    let up = Vec3::new(0.0, 2.0, 0.0);
    let out = gizmo_physics_soft::soft_body::resolve_node_collision(Vec3::ZERO, up, 1.0, DT, &floor, &[]);
    assert!(out.impact.is_none(), "a node rising off the floor was taken as hitting it: {out:?}");
    assert_eq!(out.velocity, up);
}

/// Three blocks dropped a few centimetres off centre onto each other do not blow up.
///
/// A push that moved a node out of another body in one go stretched its elements so hard that the
/// next explicit step blew the stack apart — measured in exactly this scene (1 × 0.7 × 1 m, 8 kg,
/// dropped 1.6 m), nodes at thousands of m/s within two seconds of the third landing in a release
/// build, 40 m/s in this debug one. A push is at most 5 mm now, the rest taken over the next
/// sub-steps; the fastest node is then the 6 m/s of the drop itself.
#[test]
fn an_off_centre_stack_does_not_blow_up() {
    let mut world = floor_world();
    let mut fastest_seen = 0.0f32;
    for frame in 0..480 {
        if frame % 150 == 0 && frame < 450 {
            let top = world.borrow::<SoftBodyMesh>().iter().map(|b| span_y(b).1).fold(0.0, f32::max);
            let dx = [0.05f32, -0.04, 0.03][frame / 150];
            let mut b = SoftBodyMesh::block(Vec3::new(-0.5 + dx, top + 1.6, -0.5), Vec3::new(1.0, 0.7, 1.0), [2, 2, 2], 8.0, 15_000.0, 0.3).unwrap();
            b.damping = 0.4;
            world.spawn_bundle((b,));
        }
        run(&world, DT);
        fastest_seen = fastest_seen.max(world.borrow::<SoftBodyMesh>().iter().map(fastest).fold(0.0, f32::max));
    }
    assert!(fastest_seen < 15.0, "the stack blew up: a node reached {fastest_seen:.0} m/s");
}
