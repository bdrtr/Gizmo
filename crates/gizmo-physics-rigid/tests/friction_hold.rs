//! How far a box held at a fraction of its friction limit drifts.
//!
//! # What this measures, and why it is a test rather than a note
//!
//! Friction in this solver is **velocity-level**: every row is `acc_t − rel·t/k_t`, so it resists
//! tangential *velocity* and never tangential *displacement*. The normal channel has a positional
//! counterpart (`pen0` reaches the bias in all three sweeps); the tangent channel has none. The
//! consequence is that a contact which is *not* sliding still loses a little ground each step,
//! because `λ_n` fluctuates between sweeps and a velocity row cannot undo a displacement that has
//! already happened.
//!
//! That residual was measured on 2026-08-17 at **0.000071 m/s at 99 % of the static limit**, i.e.
//! about 14 mm over 200 s, after the friction-cone fix cut it 7.5×. It was recorded in
//! docs/ENGINE.md §3 as a number in prose, with no test attached — so nothing would notice it
//! getting worse, and nothing would confirm a fix. This file is that measurement, made repeatable.
//!
//! The bound below is deliberately **not** the measured value: a test that asserts the current
//! number turns any change into a failure and any improvement into a chore. It asserts the
//! property the number is evidence for — the drift stays in millimetres over a long hold — and
//! prints the rate so a change is visible in the log.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::components::{CombineMode, PhysicsMaterial};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 60.0;
const GRAVITY: f32 = 9.81;
const MASS: f32 = 1.0;
const MU_S: f32 = 0.6;

fn scene() -> World {
    let mut w = World::new();
    w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -GRAVITY, 0.0)));
    w
}

fn material() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 0.0,
        static_friction: MU_S,
        dynamic_friction: 0.5,
        friction_combine: CombineMode::Average,
        restitution_combine: CombineMode::Min,
        ..Default::default()
    }
}

/// A box resting on a static plate, pushed sideways at `fraction` of what static friction can
/// hold. Returns how far it drifted, in metres, over `seconds`.
///
/// The push is applied through `RigidBody::force_accumulator`, **not** as a velocity change, and
/// that distinction is the difference between measuring creep and measuring nothing.
///
/// A velocity kick of `F·dt/m` arrives at the solver as tangential motion that has *already
/// happened*, so `friction_limit` sees a sliding contact and charges the dynamic coefficient. With
/// the default material (`μ_s` 0.6, `μ_d` 0.5) anything above 83 % of the static limit then runs
/// away, and the first version of this harness measured 117 km of drift at "90 % of the limit" —
/// a number about the harness, not about the solver. A force is resisted *before* it becomes
/// velocity, which is what a box on a slope actually experiences.
fn drift_at(fraction: f32, seconds: f32) -> f32 {
    let mut w = scene();

    // Ground: a wide static box whose top is exactly y = 0.
    let g = w.spawn();
    w.add_component(g, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    w.add_component(g, RigidBody::new_static());
    w.add_component(g, Velocity::default());
    let mut ground = Collider::box_collider(Vec3::new(20.0, 0.5, 20.0));
    ground.material = material();
    w.add_component(g, ground);

    // The box.
    let b = w.spawn();
    w.add_component(b, Transform::new(Vec3::new(0.0, 0.25, 0.0)));
    let mut rb = RigidBody::new(MASS, true);
    let mut collider = Collider::box_collider(Vec3::splat(0.25));
    collider.material = material();
    rb.update_inertia_from_collider(&collider);
    w.add_component(b, rb);
    w.add_component(b, Velocity::default());
    w.add_component(b, collider);

    // Let it settle before the push, so the measurement is of the hold and not of the landing.
    for _ in 0..120 {
        physics_step_system(&w, DT);
    }
    let start = w.borrow::<Transform>().get(b.id()).unwrap().position;
    let push = fraction * MU_S * MASS * GRAVITY; // newtons the plate can just hold

    let steps = (seconds / DT) as usize;
    for _ in 0..steps {
        // Woken along with the push. Sleeping would make this measure nothing at all: the
        // question is what a contact does *while it is being solved*, and a sleeping body is not
        // solved. Waking it is also honest about the scene — something is pushing it.
        let mut bodies = w.borrow_mut::<RigidBody>();
        if let Some(mut rb) = bodies.get_mut(b.id()) {
            rb.wake_up();
            rb.force_accumulator.x += push;
        }
        drop(bodies);
        physics_step_system(&w, DT);
    }

    let end = w.borrow::<Transform>().get(b.id()).unwrap().position;
    (end - start).length()
}

/// A box pushed at 90 % of its limit holds: millimetres over three minutes, not metres.
#[test]
fn a_box_pushed_below_its_friction_limit_holds_for_minutes() {
    let seconds = 200.0;
    let drift = drift_at(0.90, seconds);
    println!("[creep] 90 % of limit: {:.6} m over {seconds} s ({:.9} m/s)", drift, drift / seconds);
    assert!(
        drift < 0.05,
        "a box at 90 % of its static limit drifted {drift:.4} m in {seconds} s — friction is not \
         holding it at all"
    );
}

/// At 99 % the residual is what §3's "friction has no positional term" item is about: a velocity
/// row cannot undo a displacement that already happened, so a little ground is lost each step.
///
/// The bound is millimetres, not the measured value — see the module docs for why. What the number
/// in the log is for is noticing a change.
#[test]
fn the_residual_creep_at_the_friction_limit_stays_in_millimetres() {
    let seconds = 200.0;
    let drift = drift_at(0.99, seconds);
    let rate = drift / seconds;
    println!("[creep] 99 % of limit: {drift:.6} m over {seconds} s ({rate:.9} m/s)");
    assert!(
        drift < 0.05,
        "the residual creep at 99 % of the static limit is {drift:.4} m over {seconds} s. It was \
         0.0142 m when this test was written; five centimetres means the tangent channel has \
         stopped holding rather than merely lacking a positional term"
    );
}

/// A force is a force at every frame rate — the defect writing this file turned up.
///
/// The world runs fixed 1/240 s substeps, and `force_accumulator` used to be drained by the
/// integrator on the FIRST substep of each frame. So a 1/60 s frame is four substeps, the force
/// landed on one of them, and the body received a quarter of the impulse asked for. Measured
/// before the fix, 10 N on 1 kg for one second:
///
/// | frame rate | v after 1 s |
/// |---|---|
/// | 240 fps | 9.95 m/s |
/// | 120 fps | 4.97 m/s |
/// |  60 fps | 2.49 m/s |
/// |  30 fps | 1.24 m/s |
///
/// The same push, the same second, and the acceleration halves every time the frame rate halves —
/// which makes a thruster, a wind volume or a custom gravity field behave differently on every
/// machine. Forces are drained once per FRAME now, so every substep integrates `F·substep_dt` and
/// the sum is `F·frame_dt`.
///
/// No collider, no gravity, no contact: this measures the force channel and nothing else.
#[test]
fn a_force_produces_the_same_acceleration_at_every_frame_rate() {
    fn speed_after_one_second(dt: f32) -> f32 {
        let mut w = World::new();
        w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));
        let b = w.spawn();
        w.add_component(b, Transform::new(Vec3::ZERO));
        w.add_component(b, RigidBody::new(MASS, false));
        w.add_component(b, Velocity::default());
        w.add_component(b, Collider::box_collider(Vec3::splat(0.25)));
        for _ in 0..((1.0 / dt).round() as usize) {
            let mut bodies = w.borrow_mut::<RigidBody>();
            if let Some(mut rb) = bodies.get_mut(b.id()) {
                rb.wake_up();
                rb.force_accumulator.x += 10.0;
            }
            drop(bodies);
            physics_step_system(&w, dt);
        }
        w.borrow::<Velocity>().get(b.id()).unwrap().linear.x
    }

    let rates = [1.0 / 240.0, 1.0 / 120.0, 1.0 / 60.0, 1.0 / 30.0];
    let speeds: Vec<f32> = rates.iter().copied().map(speed_after_one_second).collect();
    println!("[force] v after 1 s of 10 N at 240/120/60/30 fps: {speeds:?}");

    for (dt, v) in rates.iter().zip(&speeds) {
        assert!(
            (v - 10.0).abs() < 0.1,
            "10 N on 1 kg for 1 s must reach 10 m/s; at dt = {dt} it reached {v}"
        );
    }
    let spread = speeds.iter().copied().fold(f32::MIN, f32::max)
        - speeds.iter().copied().fold(f32::MAX, f32::min);
    assert!(
        spread < 0.01,
        "the same force gave different speeds at different frame rates (spread {spread}) — this \
         is the substep-drain bug coming back"
    );
}

/// The premise the two tests above rest on: push it *past* the limit and it must actually move.
///
/// Without this, a solver that simply froze every contact would pass both — and "nothing ever
/// slides" is a worse bug than a millimetre of creep.
#[test]
fn a_box_pushed_past_its_limit_does_slide() {
    let drift = drift_at(1.5, 2.0);
    println!("[creep] 150 % of limit: {drift:.4} m over 2 s");
    assert!(
        drift > 0.05,
        "pushed half again past its limit the box moved only {drift:.4} m — friction is not \
         releasing, which would make every creep assertion above vacuous"
    );
}
