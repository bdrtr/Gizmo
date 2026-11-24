//! The wide block solver (`solver/wide.rs`) solves a coloured island as the scalar block step does,
//! four blocks at a time: the same island comes to rest in the same place with it on and off.

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

/// A 8 × 8 × 4 stack of half-metre boxes on the ground, side by side and a hair apart in height: one
/// island of some 800 contact blocks, so it is swept colour by colour (`parallel_island_blocks` is
/// 256) — with gaps between the columns it is 64 islands of four, and never is.
fn stack(wide: bool, hz: u32) -> PhysicsWorld {
    let mut w = PhysicsWorld::new().with_substep_rate(hz).expect("rate");
    w.solver.wide_blocks = wide;
    w.add_body(
        BodyHandle::from_id(1),
        RigidBody::new_static(),
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)),
    );
    let mut id = 2;
    for layer in 0..4 {
        for i in 0..8 {
            for k in 0..8 {
                let mut rb = RigidBody::new(1.0, true);
                let col = Collider::box_collider(Vec3::splat(0.25));
                rb.update_inertia_from_collider(&col);
                let at = Vec3::new(i as f32 * 0.5 - 1.75, 0.25 + layer as f32 * 0.501, k as f32 * 0.5 - 1.75);
                w.add_body(BodyHandle::from_id(id), rb, Transform::new(at), Velocity::default(), col);
                id += 1;
            }
        }
    }
    w
}

fn settle(w: &mut PhysicsWorld) {
    for _ in 0..120 {
        w.step(1.0 / 60.0).expect("step");
    }
}

#[test]
fn a_coloured_stack_rests_where_the_scalar_step_rests_it() {
    for hz in [60u32, 240] {
        let (mut on, mut off) = (stack(true, hz), stack(false, hz));
        settle(&mut on);
        settle(&mut off);
        let mut worst = 0.0f32;
        for i in 1..on.transforms.len() {
            worst = worst.max((on.transforms[i].position - off.transforms[i].position).length());
        }
        assert!(worst < 2e-3, "{hz} Hz: a box rests {worst} m from where the scalar step rests it");
        let fastest = on.velocities.iter().map(|v| v.linear.length()).fold(0.0, f32::max);
        assert!(fastest < 0.05, "{hz} Hz: the stack is still moving at {fastest} m/s");
        let lowest = on.transforms[1..].iter().map(|t| t.position.y).fold(f32::MAX, f32::min);
        assert!(lowest > 0.24, "{hz} Hz: a box sank to {lowest}");
    }
}
