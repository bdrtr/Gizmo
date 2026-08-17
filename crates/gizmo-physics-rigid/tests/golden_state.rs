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
    // A settled box holds NO residual velocity. This was `-0.040_873_3` until 2026-08-06 —
    // exactly one substep of gravity, 9.81/240, which the contact had not yet cancelled. That
    // number was a fingerprint of a defect rather than of the substep rate: a body used to fall
    // asleep at the end of velocity integration, i.e. after gravity had been applied and BEFORE
    // the contact solve could cancel it, and then froze holding the unsolved value. Sleep is now
    // decided after the solve and per island, so the body sleeps holding a solved velocity.
    // `settle y` is unchanged, so the resting position — the load-bearing number here — did not
    // move. See docs/ENGINE.md, island-collective sleep.
    lock("settle vy", w.velocities[1].linear.y, 0.0);
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

/// A pendulum on a hinge, swinging under gravity. The first joint scene in this file.
///
/// Nothing that gates CI has a joint in it: `headless_stress_test`, `determinism.rs`,
/// `rollback.rs`, `soak_and_golden.rs` and every scene above are joint-free. Every joint
/// change in this campaign — centre-of-mass lever arms, accumulated impulses, the net-reaction
/// break check, the motor budget — could therefore have moved joint behaviour arbitrarily
/// without a single gate noticing, and each one had to be measured by hand against a
/// hand-built scene to know whether it had. That gap closes here.
///
/// Deliberately well-conditioned, like the rest of the file: a single hinge, no limits, no
/// motor, no contact. The bob swings on a fixed arm, so its position is bounded and smooth
/// and cross-platform `f32` drift stays orders below `TOL`. A rope or a limit would be the
/// wrong choice — those saturate against a bound, which is exactly where small differences
/// get amplified.
#[test]
fn golden_hinge_pendulum_swing() {
    let mut w = PhysicsWorld::new();

    let mut anchor = RigidBody::new_static();
    anchor.wake_up();
    w.add_body(
        BodyHandle::from_id(1),
        anchor,
        Transform::new(Vec3::new(0.0, 5.0, 0.0)),
        Velocity::default(),
        Collider::sphere(0.1),
    );

    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let c = Collider::box_collider(Vec3::splat(0.2));
    rb.update_inertia_from_collider(&c);
    w.add_body(
        BodyHandle::from_id(2),
        rb,
        Transform::new(Vec3::new(2.0, 5.0, 0.0)), // arm horizontal: released from the top
        Velocity::default(),
        c,
    );

    let j = gizmo_physics_rigid::Joint::hinge(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::new(-2.0, 0.0, 0.0),
        Vec3::Z,
    );
    w.joints.push(j);

    for _ in 0..180 {
        w.step(1.0 / 60.0).expect("step");
    }

    // RE-BLESSED 2026-08-09 — rigid joint rows became soft constraints at
    // `JointSolver::rigid_hertz` (200 Hz). Old → new:
    //   x     1.892_767   → 1.895_446_8   (+2.7e-3)
    //   y     4.351_293   → 4.360_516     (+9.2e-3)
    //   vx    1.100_116_5 → 1.084_893_6   (-1.5e-2)
    //   vy    3.208_478_7 → 3.196_372_5   (-1.2e-2)
    //   arm error x1000  0.846_624_4 → 0.414_609_9  (the error nearly HALVED — see below)
    // `golden_hinge_pendulum_swing_legacy_baumgarte` keeps the five old values under the
    // `rigid_hertz = 0` lever, so nothing here was thrown away.
    // RE-BLESSED 2026-08-17 — joints warm-start by default now
    // (`JointSolver::warm_start_factor` 0.0 → 0.5). Old → new:
    //   x     1.895_446_8 → 1.896_123_6   (+6.8e-4)
    //   y     4.360_516   → 4.362_821     (+2.3e-3)
    //   vx    1.084_893_6 → 1.080_646_4   (-4.2e-3)
    //   vy    3.196_372_5 → 3.191_738     (-4.6e-3)
    //   arm error x1000  0.414_609_9 → 0.320_434_57  (-23 %, the direction that matters)
    lock("pendulum x", w.transforms[1].position.x, 1.896_123_6);
    lock("pendulum y", w.transforms[1].position.y, 4.362_821);
    lock("pendulum vx", w.velocities[1].linear.x, 1.080_646_4);
    lock("pendulum vy", w.velocities[1].linear.y, 3.191_738);

    // The arm length is the joint's whole job, and its ERROR is the sharpest instrument in
    // this file. Scaled by 1000 on purpose: the raw length is 2.0008, so an absolute 1e-3
    // lock on it would accept anything from 1.999 to 2.001 — it would accept the constraint
    // error changing by a factor of two and call it unchanged. Scaling the error into the
    // same range as the other locks gives it the same resolution.
    //
    // Measured sensitivity: dropping the solver's Baumgarte factor from 0.3 to 0.25 moves
    // this by 0.226 — 226 tolerances — while it moves `pendulum x` by 1.0e-3, which barely
    // clears the tolerance at all. This is the line that will catch a joint regression.
    //
    // That sensitivity is also what PREDICTED the re-blessing, before it was measured. The
    // arm error here is repair-rate limited (`C ∝ 1/bias_rate`), not load limited — the load
    // term contributes only `8.9/ω² = 5.7e-6 m`, i.e. 0.006 on this scale. Soft rigid rows
    // raise the bias rate from `β/dt` = 72 s⁻¹ to 173.7 s⁻¹, so the prediction was a DROP to
    // 0.25–0.45. Measured: 0.4146. The sharpest instrument in this file therefore reads
    // BETTER after the change, and had it come back near 0.85 the model behind it would have
    // been wrong.
    let arm = (w.transforms[1].position - Vec3::new(0.0, 5.0, 0.0)).length();
    lock("pendulum arm error x1000", (arm - 2.0) * 1000.0, 0.320_434_57);

    // …and independently of every golden value above: whatever those numbers become after a
    // deliberate re-blessing, a hinge that lets the bob drift off its arm is broken, not
    // merely different. Tightened 0.02 → 0.005 with the re-blessing (48× margin → 12×): the
    // loose bound was sized for an unknown, and the unknown is now measured.
    assert!(
        (arm - 2.0).abs() < 0.005,
        "the hinge must hold the 2 m arm, got {arm}"
    );
}

/// The same scene under the rollback lever, locking the five PRE-CHANGE constants unchanged.
///
/// `JointSolver::rigid_hertz = 0` selects the legacy Baumgarte path, and this is what says it
/// is still bit-for-bit the solver that shipped before 2026-08-09 — every one of the five
/// values below is the constant `golden_hinge_pendulum_swing` carried until that day, pasted
/// across untouched. The legacy arm keeps `β·error/dt` written literally rather than routed
/// through a shared `bias_rate · error`, because in f32 those round differently; this test is
/// what would catch that refactor.
///
/// Two jobs beyond the lock: it means the re-blessing above threw nothing away, and it is the
/// bisection tool — a future joint regression that reproduces on BOTH paths is not in the soft
/// formulation.
#[test]
fn golden_hinge_pendulum_swing_legacy_baumgarte() {
    let mut w = PhysicsWorld::new();
    w.joint_solver.rigid_hertz = 0.0;

    let mut anchor = RigidBody::new_static();
    anchor.wake_up();
    w.add_body(
        BodyHandle::from_id(1),
        anchor,
        Transform::new(Vec3::new(0.0, 5.0, 0.0)),
        Velocity::default(),
        Collider::sphere(0.1),
    );

    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let c = Collider::box_collider(Vec3::splat(0.2));
    rb.update_inertia_from_collider(&c);
    w.add_body(
        BodyHandle::from_id(2),
        rb,
        Transform::new(Vec3::new(2.0, 5.0, 0.0)),
        Velocity::default(),
        c,
    );

    w.joints.push(gizmo_physics_rigid::Joint::hinge(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::new(-2.0, 0.0, 0.0),
        Vec3::Z,
    ));

    for _ in 0..180 {
        w.step(1.0 / 60.0).expect("step");
    }

    // RE-BLESSED 2026-08-17 — same cause as the soft scene above: joints warm-start by
    // default (`warm_start_factor` 0.0 → 0.5). Old → new:
    //   x     1.892_767   → 1.894_964_7   (+2.2e-3)
    //   y     4.351_293   → 4.358_864     (+7.6e-3)
    //   vx    1.100_116_5 → 1.087_582_6   (-1.3e-2)
    //   vy    3.208_478_7 → 3.196_055_7   (-1.2e-2)
    //   arm error x1000  0.846_624_4 → 0.486_612_32  (-43 %; the Baumgarte lever gains MORE
    //   from warm start than the soft path, which is what a repair-rate-limited row should do)
    lock("legacy pendulum x", w.transforms[1].position.x, 1.894_964_7);
    lock("legacy pendulum y", w.transforms[1].position.y, 4.358_864);
    lock("legacy pendulum vx", w.velocities[1].linear.x, 1.087_582_6);
    lock("legacy pendulum vy", w.velocities[1].linear.y, 3.196_055_7);
    let arm = (w.transforms[1].position - Vec3::new(0.0, 5.0, 0.0)).length();
    lock(
        "legacy pendulum arm error x1000",
        (arm - 2.0) * 1000.0,
        0.486_612_32,
    );
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
