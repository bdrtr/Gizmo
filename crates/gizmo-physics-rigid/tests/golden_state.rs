//! Golden state: a behaviour LOCK, not a plausibility check.
//!
//! # Why this exists
//!
//! The determinism gate CI runs — `demo/src/bin/headless_stress_test.rs` — compares three
//! runs of the same binary **against each other** and against no committed constant. It
//! detects nondeterminism, which is what it was built for, and nothing else: change the
//! solver so the tower collapses into a completely different pile and it still prints
//! `DETERMINISM VERIFIED`. The hash it prints is never compared to anything.
//!
//! `tests/analytical.rs` covers the other half — closed-form physics — but a change can keep
//! physics plausible and still be wrong for a project: a different substep count, a shifted
//! damping default, a friction combine mode swapped. Those pass every closed-form assertion
//! and every soak test, and nothing notices.
//!
//! So: canonical scenes with **committed reference values**. If a change moves them, this
//! goes red and the diff shows by how much.
//!
//! # Why tolerances and not a hash
//!
//! `PhysicsWorld::state_hash` is bit-exact and explicitly **same-platform only**
//! (`docs/ENGINE.md` §5) — the simulation runs on `f32`/glam, and CI tests on Linux, macOS
//! and Windows. A committed hash would be red on two of the three for a reason that is not a
//! bug. Values plus a tolerance survive the platform spread while still catching any change
//! big enough to matter.
//!
//! Every scene here is deliberately **well-conditioned** — settling, sliding to rest,
//! free fall, a resting stack. Their end states converge, so cross-platform `f32` drift stays
//! far below the tolerance. A chaotic scene (the 200-box tower collapse) is the opposite:
//! its end state is arbitrarily sensitive to the last bit, so no tolerance can separate
//! "different platform" from "different physics". That scene is covered by the soak tests,
//! which assert bounds rather than values, and it is why this file does not include one.
//!
//! # Re-blessing
//!
//! A failure here is not automatically a bug — it means behaviour changed. If the change was
//! intended, update the constant and **say so in the commit message, with the old and new
//! value**. The failure message prints the measured value in the exact form to paste back.

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, PhysicsMaterial, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

/// Absolute tolerance for positions and velocities, in world units / units per second.
///
/// Chosen against measurement, not taste: the quantities pinned below are order 1–100, and
/// a real behaviour change (solver iterations, a damping default, a friction mode) moves
/// them by 1e-2 or more. Cross-platform `f32` drift on well-conditioned scenes is several
/// orders below that. 1e-3 sits in the gap.
const TOL: f32 = 1e-3;

#[track_caller]
fn lock(label: &str, measured: f32, reference: f32) {
    assert!(
        (measured - reference).abs() <= TOL,
        "{label}: behaviour changed.\n  \
         reference {reference:?}\n  \
         measured  {measured:?}\n  \
         delta     {:?} (tolerance {TOL:?})\n\n\
         If this change was intended, update the constant to {measured:?} and record the \
         old→new values in the commit message. If it was not, something moved the physics.",
        measured - reference
    );
}

fn ground(w: &mut PhysicsWorld) {
    let mut g = RigidBody::new_static();
    g.wake_up();
    w.add_body(
        BodyHandle::from_id(0),
        g,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)), // top face at y = 0
        Velocity::default(),
        Collider::box_collider(Vec3::new(50.0, 1.0, 50.0)),
    );
}

fn body(w: &mut PhysicsWorld, id: u32, pos: Vec3, half: f32, vel: Vec3, mat: PhysicsMaterial) {
    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let c = Collider::box_collider(Vec3::splat(half)).with_material(mat);
    rb.update_inertia_from_collider(&c);
    let v = Velocity {
        linear: vel,
        ..Default::default()
    };
    w.add_body(BodyHandle::from_id(id), rb, Transform::new(pos), v, c);
}

/// A box dropped from 5 m settles on the ground. Locks contact resolution and the resting
/// penetration the solver allows — the single most load-bearing number in the engine, since
/// every stack rests on it.
#[test]
fn golden_box_settling_on_the_ground() {
    let mut w = PhysicsWorld::new();
    ground(&mut w);
    body(
        &mut w,
        1,
        Vec3::new(0.0, 5.0, 0.0),
        0.5,
        Vec3::ZERO,
        PhysicsMaterial::default(),
    );
    for _ in 0..300 {
        w.step(1.0 / 60.0).expect("step");
    }

    lock("settle y", w.transforms[1].position.y, 0.498_650_88);
    // The residual downward velocity is one substep of gravity that the contact has not yet
    // cancelled. It is small, but it is a fingerprint of the substep rate and the solver's
    // relax pass, so it is worth pinning.
    lock("settle vy", w.velocities[1].linear.y, -0.040_873_3);
}

/// A box launched at 5 m/s slides to rest under friction. Locks the friction model, the
/// combine mode and the substep count all at once: the stopping distance is a function of
/// every one of them.
#[test]
fn golden_friction_stopping_distance() {
    let mut w = PhysicsWorld::new();
    ground(&mut w);
    let mat = PhysicsMaterial {
        static_friction: 0.5,
        dynamic_friction: 0.5,
        restitution: 0.0,
        ..Default::default()
    };
    body(
        &mut w,
        1,
        Vec3::new(0.0, 0.5, 0.0),
        0.5,
        Vec3::new(5.0, 0.0, 0.0),
        mat,
    );
    for _ in 0..240 {
        w.step(1.0 / 60.0).expect("step");
    }

    // Closed form for reference: v²/(2μg) = 25 / (2·0.5·9.81) = 2.548 m. The measured value
    // sits just under it, as a discrete solver should. `analytical.rs` asserts the physics;
    // this pins the number so a change to it cannot hide inside a loose band.
    lock("slide x", w.transforms[1].position.x, 2.520_943_4);
}

/// Free fall with horizontal velocity, never touching anything. Locks the integrator alone —
/// no contact, no solver, no friction. If this moves, gravity or the substep accumulator did.
#[test]
fn golden_ballistic_integration() {
    let mut w = PhysicsWorld::new();
    ground(&mut w);
    body(
        &mut w,
        1,
        Vec3::new(0.0, 100.0, 0.0),
        0.5,
        Vec3::new(3.0, 0.0, 0.0),
        PhysicsMaterial::default(),
    );
    for _ in 0..120 {
        w.step(1.0 / 60.0).expect("step");
    }

    lock("fall x", w.transforms[1].position.x, 5.940_277_6);
    lock("fall y", w.transforms[1].position.y, 80.470_085);
    lock("fall vx", w.velocities[1].linear.x, 2.940_599_2);
    lock("fall vy", w.velocities[1].linear.y, -19.424_74);
}

/// Two boxes resting in a stack. Locks how much a contact chain sinks — the quantity the
/// block-solver and warm-start work was about, and the one a regression there would move
/// first.
#[test]
fn golden_two_box_stack_rest_heights() {
    let mut w = PhysicsWorld::new();
    ground(&mut w);
    body(
        &mut w,
        1,
        Vec3::new(0.0, 0.5, 0.0),
        0.5,
        Vec3::ZERO,
        PhysicsMaterial::default(),
    );
    body(
        &mut w,
        2,
        Vec3::new(0.0, 1.5, 0.0),
        0.5,
        Vec3::ZERO,
        PhysicsMaterial::default(),
    );
    for _ in 0..300 {
        w.step(1.0 / 60.0).expect("step");
    }

    lock("stack lower y", w.transforms[1].position.y, 0.499_686);
    lock("stack upper y", w.transforms[2].position.y, 1.499_451_6);
}

/// The gate on the gate: `state_hash` must stay reproducible within a process.
///
/// This is what `headless_stress_test` actually checks, kept here so the property is
/// asserted in the test suite rather than only in a demo binary CI happens to run. It is
/// deliberately NOT compared to a committed constant — that would be a cross-platform
/// bit-exactness claim the engine does not make.
#[test]
fn state_hash_is_reproducible_for_identical_worlds() {
    let build = || {
        let mut w = PhysicsWorld::new();
        ground(&mut w);
        for i in 0..4 {
            body(
                &mut w,
                i + 1,
                Vec3::new(0.0, 0.5 + i as f32, 0.0),
                0.5,
                Vec3::ZERO,
                PhysicsMaterial::default(),
            );
        }
        for _ in 0..120 {
            w.step(1.0 / 60.0).expect("step");
        }
        w.state_hash()
    };
    assert_eq!(
        build(),
        build(),
        "two identical worlds stepped identically must hash the same"
    );
}
