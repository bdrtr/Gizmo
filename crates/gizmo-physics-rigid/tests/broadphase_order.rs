//! A step's outcome is a function of the bodies, not of the broadphase's history. The tree a
//! broadphase keeps has a shape that depends on the order bodies went in and moved, and the
//! candidate pairs came out in that shape's order — and so did the collision events, which what
//! reacts to them (debris spawned by the fracture system, a game's handler) turns into entity ids
//! and a different simulation. A rollback restores bodies, not a tree's history.

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

/// A ground and a loose pile of boxes falling onto it and each other.
fn pile() -> PhysicsWorld {
    let mut w = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    w.add_body(BodyHandle::from_id(0), RigidBody::new_static(), Transform::new(Vec3::new(0.0, -0.5, 0.0)), Velocity::default(), Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)));
    for k in 1..=40u32 {
        let mut rb = RigidBody::new(1.0, true);
        rb.calculate_box_inertia(1.0, 1.0, 1.0);
        let at = Vec3::new((k % 5) as f32 * 0.9 - 2.0, 0.5 + (k / 5) as f32 * 0.95, ((k * 3) % 4) as f32 * 0.4 - 0.6);
        w.add_body(BodyHandle::from_id(k), rb, Transform::new(at), Velocity::default(), Collider::box_collider(Vec3::splat(0.5)));
    }
    w
}

/// The same bodies' broadphase, rebuilt in the opposite order: another tree shape.
fn rebuild_backwards(w: &mut PhysicsWorld) {
    let boxes: Vec<_> = (0..w.entities.len())
        .rev()
        .map(|i| (w.entities[i], w.rigid_bodies[i].is_static(), w.colliders[i].compute_aabb(w.transforms[i].position, w.transforms[i].rotation)))
        .collect();
    w.spatial_hash.clear();
    for (e, fixed, aabb) in boxes {
        if fixed {
            w.spatial_hash.insert_static(e, aabb);
        } else {
            w.spatial_hash.insert(e, aabb);
        }
    }
}

fn events(w: &PhysicsWorld) -> Vec<(u32, u32, String)> {
    w.collision_events().iter().map(|e| (e.entity_a.id(), e.entity_b.id(), format!("{:?}", e.event_type))).collect()
}

#[test]
fn the_same_bodies_give_the_same_events_whatever_shape_the_broadphase_has() {
    let (mut a, mut b) = (pile(), pile());
    for step in 0..120 {
        if step == 20 {
            rebuild_backwards(&mut b);
        }
        a.step(1.0 / 60.0).unwrap();
        b.step(1.0 / 60.0).unwrap();
        assert_eq!(events(&a), events(&b), "step {step}: the collision events came in another order");
    }
    assert!(!events(&a).is_empty(), "premise: the pile is in contact");
    assert_eq!(a.state_hash(), b.state_hash(), "the same bodies ended in another state");
}
