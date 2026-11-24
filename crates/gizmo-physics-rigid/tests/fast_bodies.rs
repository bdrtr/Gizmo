//! A fast body does not pass through what it hits, at any substep rate, without asking for CCD.
//!
//! The world's 240 Hz was what kept a 0.5 m sphere from passing through another at 320 m/s: each
//! substep runs collision detection, and a body moving less than its own size per substep cannot skip
//! over anything. At 60 Hz that margin is a quarter. A body that moves a fair part of its own size in
//! one substep is now swept in the broad phase and given speculative contacts on its own, so the
//! contacts close the gap instead of discovering it from the far side.

use gizmo_math::Vec3;
use gizmo_physics_core::{
    components::{CombineMode, PhysicsMaterial},
    BodyHandle, Collider, Transform,
};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

const RATES: [u32; 3] = [60, 120, 240];

fn world(hz: u32) -> PhysicsWorld {
    PhysicsWorld::new().with_gravity(Vec3::ZERO).with_substep_rate(hz).expect("rate")
}

fn elastic() -> PhysicsMaterial {
    PhysicsMaterial { restitution: 1.0, static_friction: 0.0, dynamic_friction: 0.0, restitution_combine: CombineMode::Max, ..Default::default() }
}

fn ball(world: &mut PhysicsWorld, id: u32, x: f32, r: f32, vx: f32) {
    world.add_body(
        BodyHandle::from_id(id),
        RigidBody::new(1.0, false),
        Transform::new(Vec3::new(x, 0.0, 0.0)),
        Velocity::new(Vec3::new(vx, 0.0, 0.0)),
        Collider::sphere(r).with_material(elastic()),
    );
}

/// `cradle_passthrough`'s row of touching 0.5 m spheres, the first arriving at up to 320 m/s, no CCD.
///
/// Two laws, at every rate. **The striking ball never passes the one it strikes**: it moves fast
/// before the step, so the pair closes fast for its size and has a speculative contact (`closes_fast`). **The struck balls
/// keep their order while one substep's travel stays under 4/3 m** — the margin the 240 Hz design had
/// at 320 m/s: a ball set moving in the solve has no speculative contact with the next one yet, so it
/// is caught by overlap on the next substep, and past that travel it is beyond it. At 60 Hz that is
/// 80 m/s; a contact distance (contacts for pairs near but not touching, with real points) would lift
/// it and is its own piece of work.
#[test]
fn a_touching_row_keeps_its_order_at_any_rate() {
    for hz in RATES {
        for v in [20.0f32, 40.0, 80.0, 160.0, 320.0] {
            let mut w = world(hz);
            let spacing = 1.01;
            ball(&mut w, 0, -spacing * 2.0, 0.5, v);
            for i in 1..5 {
                ball(&mut w, i, (i as f32 - 1.0) * spacing, 0.5, 0.0);
            }
            let chain_holds = v / hz as f32 <= 4.0 / 3.0 + 1e-4;
            for _ in 0..120 {
                w.step(1.0 / 60.0).expect("step");
                let gap = w.transforms[1].position.x - w.transforms[0].position.x;
                assert!(gap > 0.0, "{hz} Hz, {v} m/s: the striking ball passed the struck one (gap {gap})");
                if chain_holds {
                    for i in 2..5 {
                        let gap = w.transforms[i].position.x - w.transforms[i - 1].position.x;
                        assert!(gap > 0.0, "{hz} Hz, {v} m/s: balls {} and {i} swapped (gap {gap})", i - 1);
                    }
                }
            }
        }
    }
}

/// A 0.05 m ball at 2400 m/s against a 4 cm wall, no CCD — `ccd_analytical`'s rung 2, which went
/// through the wall until the fast-body rule: it stops in front of the wall at every rate. 2400 m/s
/// is 10 m a substep at 240 Hz and 40 at 60.
#[test]
fn a_fast_small_ball_does_not_pass_a_thin_wall_at_any_rate() {
    for hz in RATES {
        let mut w = world(hz);
        w.add_body(
            BodyHandle::from_id(0),
            RigidBody::new_static(),
            Transform::new(Vec3::new(10.0, 0.0, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(0.02, 5.0, 5.0)),
        );
        w.add_body(
            BodyHandle::from_id(1),
            RigidBody::new(1.0, false),
            Transform::new(Vec3::ZERO),
            Velocity::new(Vec3::new(2400.0, 0.0, 0.0)),
            Collider::sphere(0.05),
        );
        for _ in 0..60 {
            w.step(1.0 / 60.0).expect("step");
        }
        let x = w.transforms[1].position.x;
        assert!(x < 10.0, "{hz} Hz: the ball went through the wall to x = {x}");
        assert!(x.is_finite());
    }
}
