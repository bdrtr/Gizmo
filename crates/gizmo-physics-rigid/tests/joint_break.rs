//! `break_force` / `break_torque` — what the threshold actually measures.
//!
//! The threshold is public, persisted (`SceneData`), and exposed in the editor inspector as
//! a force in newtons. For that to mean anything it has to be compared against the force the
//! joint really carries. It was not: every joint type summed the MAGNITUDES of its rows
//! (`Σ|λᵢ|`) from inside the iteration loop. These tests pin the two ways that was wrong.
//!
//! No committed test exercised a finite threshold on a row-based joint — `world/tests.rs`
//! uses `f32::MAX` and the only break test in `joints_behavior.rs` is a Spring, which is
//! force-based and has a single row-free impulse. So this file is new coverage, not a
//! re-blessing of old numbers.

use gizmo_math::Vec3;
use gizmo_physics_core::BodyHandle;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::{Joint, JointData, PhysicsWorld, RigidBody, Velocity};

/// A 1 kg box welded to a static anchor, with gravity of magnitude `g` pointing along
/// `dir`. The weld's anchor sits on the box's centre of mass, so the reaction is purely
/// linear and its magnitude is exactly `m·g` — whatever direction gravity points.
fn welded_load(break_force: f32, iterations: usize, gravity: Vec3) -> PhysicsWorld {
    let mut world = PhysicsWorld::new().with_gravity(gravity);
    world.joint_solver.iterations = iterations;

    let mut anchor = RigidBody::new_static();
    anchor.wake_up();
    world.add_body(
        BodyHandle::from_id(1),
        anchor,
        Transform::new(Vec3::ZERO),
        Velocity::default(),
        Collider::sphere(0.1),
    );

    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::splat(0.2));
    rb.update_inertia_from_collider(&col);
    world.add_body(
        BodyHandle::from_id(2),
        rb,
        Transform::new(Vec3::new(2.0, 0.0, 0.0)),
        Velocity::default(),
        col,
    );

    let mut j = Joint::fixed(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::new(2.0, 0.0, 0.0), // anchor on A, at B's centre of mass
        Vec3::ZERO,
    );
    j.break_force = break_force;
    j.break_torque = f32::MAX; // isolate the linear check
    world.joints.push(j);
    world
}

fn run(world: &mut PhysicsWorld, steps: usize) -> bool {
    for _ in 0..steps {
        world.step(1.0 / 60.0).ok();
    }
    world.joints[0].is_broken
}

/// **Rotating gravity must not change the verdict.** Same 1 kg mass, same 9.81 N of load,
/// once straight down and once spread equally over all three axes. The weld's three linear
/// rows are the world X/Y/Z axes, so the axis-aligned case loads ONE row and the diagonal
/// case loads all three at `1/√3` each. The net reaction is identical; the L1 sum of row
/// magnitudes is `√3` times larger in the diagonal case.
///
/// So the old `Σ|λᵢ|` check reported a joint under diagonal load as carrying 73% more force
/// than the same joint under axis-aligned load, and broke it at a threshold the aligned one
/// survived. Nothing physical distinguishes the two — only the arbitrary choice of world
/// axes for the rows. On a ball-socket it is worse: cone, twist and swing rows are not even
/// orthogonal, so there is no bound on the overstatement.
///
/// Written as a property rather than an absolute number: the true reaction here is not
/// simply `m·g` (the Baumgarte bias contributes, and the measured threshold is ≈22.6 N),
/// so an absolute assertion would be pinning a solver detail. The DIRECTION-independence is
/// the physics.
#[test]
fn break_force_measures_the_net_reaction_not_the_sum_of_axis_magnitudes() {
    let aligned = Vec3::new(0.0, -9.81, 0.0);
    let diagonal = -Vec3::splat(9.81 / 3.0_f32.sqrt());
    assert!(
        (aligned.length() - diagonal.length()).abs() < 1e-3,
        "the two gravities must be the same magnitude, or the test proves nothing"
    );

    // Above the load: both survive. Old code broke the diagonal one (√3 × 22.6 ≈ 39 N > 30).
    for (name, g) in [("aligned", aligned), ("diagonal", diagonal)] {
        let mut w = welded_load(30.0, 10, g);
        assert!(
            !run(&mut w, 120),
            "{name} gravity: a weld under 9.81 N of load must survive a 30 N threshold —              the L1 row sum overstated the diagonal case by √3 and snapped it"
        );
    }

    // Below the load: both break. Without this the test above would pass on a joint that
    // simply never breaks.
    for (name, g) in [("aligned", aligned), ("diagonal", diagonal)] {
        let mut w = welded_load(15.0, 10, g);
        assert!(
            run(&mut w, 120),
            "{name} gravity: the threshold must still be a real threshold"
        );
    }
}

/// `world.joint_solver.iterations` is a public field and a solver-QUALITY knob: changing it
/// must not change what breaks.
///
/// A REGRESSION GUARD, not evidence for the change that introduced it. The audit predicted
/// the old per-iteration check would scale the reported force with the iteration count —
/// measured, it does not, and this test passes on the old code too. The reason is the same
/// one that made the accumulation ratchet inert: a joint row zeroes `Jv` on its first
/// iteration, so iterations 2..N contribute ≈0 to the sum and there is nothing to
/// multiply. The guard is still worth keeping — it fails the moment anyone moves the check
/// back inside the loop now that a real accumulated total exists.
#[test]
fn break_force_does_not_depend_on_the_solver_iteration_count() {
    let g = Vec3::new(0.0, -9.81, 0.0);
    for threshold in [15.0_f32, 30.0] {
        let mut few = welded_load(threshold, 4, g);
        let mut many = welded_load(threshold, 20, g);
        let (a, b) = (run(&mut few, 120), run(&mut many, 120));
        assert_eq!(
            a, b,
            "break verdict at {threshold} N changed with the iteration count \
             (4 iterations: {a}, 20 iterations: {b}) — iterations is a solver-quality knob"
        );
    }
    // …and the two thresholds must actually straddle the load, or the test is vacuous.
    assert!(run(&mut welded_load(15.0, 10, g), 120));
    assert!(!run(&mut welded_load(30.0, 10, g), 120));
}

/// A slider's suspension spring carries real load, and until now it was invisible to
/// `break_force`: only the constraint rows were summed, so a "breakable" shock absorber
/// could hold any load forever. Same for the hinge torsional spring. Both now report into
/// the joint's net impulse.
#[test]
fn a_suspension_spring_reports_its_load_to_break_force() {
    let build = |break_force: f32| -> PhysicsWorld {
        let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
        let mut anchor = RigidBody::new_static();
        anchor.wake_up();
        world.add_body(
            BodyHandle::from_id(1),
            anchor,
            Transform::new(Vec3::new(0.0, 5.0, 0.0)),
            Velocity::default(),
            Collider::sphere(0.1),
        );
        let mut rb = RigidBody::new(5.0, true);
        rb.wake_up();
        let col = Collider::box_collider(Vec3::splat(0.2));
        rb.update_inertia_from_collider(&col);
        world.add_body(
            BodyHandle::from_id(2),
            rb,
            Transform::new(Vec3::new(0.0, 4.0, 0.0)),
            Velocity::default(),
            col,
        );
        let mut j = Joint::slider(
            BodyHandle::from_id(1),
            BodyHandle::from_id(2),
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::ZERO,
            Vec3::Y,
        );
        if let JointData::Slider(ref mut d) = j.data {
            d.use_spring = true;
            d.spring_stiffness = 400.0;
            d.spring_damping = 10.0;
            d.spring_rest_position = 0.0;
        }
        j.break_force = break_force;
        j.break_torque = f32::MAX;
        world.joints.push(j);
        world
    };

    let mut strong = build(f32::MAX);
    assert!(!run(&mut strong, 240), "an unbreakable slider must not break");

    // The spring holds a 5 kg mass — ~49 N of load once it settles. A 20 N threshold has to
    // trip. Before this, the spring's impulse never reached the break check at all.
    let mut weak = build(20.0);
    assert!(
        run(&mut weak, 240),
        "a suspension spring carrying ~49 N must trip a 20 N break_force"
    );
}

/// A static anchor at the origin and a 1 kg box at `offset`, joined by `make` with the
/// anchors at both bodies' origins — so the joint's positional error IS `offset` and its
/// linear rows all carry a real, non-zero error.
fn offset_pair(
    offset: Vec3,
    break_force: f32,
    make: fn(BodyHandle, BodyHandle, Vec3, Vec3) -> Joint,
) -> PhysicsWorld {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);

    let mut anchor = RigidBody::new_static();
    anchor.wake_up();
    world.add_body(
        BodyHandle::from_id(1),
        anchor,
        Transform::new(Vec3::ZERO),
        Velocity::default(),
        Collider::sphere(0.1),
    );

    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::splat(0.2));
    rb.update_inertia_from_collider(&col);
    world.add_body(
        BodyHandle::from_id(2),
        rb,
        Transform::new(offset),
        Velocity::default(),
        col,
    );

    let mut j = make(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::ZERO,
    );
    j.break_force = break_force;
    j.break_torque = f32::MAX;
    world.joints.push(j);
    world
}

/// One direct solver pass at a caller-chosen `dt`. `PhysicsWorld::step` always substeps at
/// `FIXED_DT`, so this — the embedding API, `JointSolver` being public — is the only way a
/// `dt` of zero reaches the joint solver, and it is the way a paused frame does.
fn solve_once(world: &mut PhysicsWorld, dt: f32) {
    world.joint_solver.solve_joints(
        &mut world.joints,
        &world.entity_index_map,
        &world.rigid_bodies,
        &world.transforms,
        &mut world.velocities,
        dt,
    );
}

/// **A zero-length step must leave the world exactly as it found it.**
///
/// Everything the joint solver computes is a rate. `break_force` is compared against
/// `‖Σλᵢnᵢ‖ / dt`, which at `dt == 0` is `x/0` = `+inf` for a joint carrying any load at
/// all — and `inf > break_force` holds for EVERY finite threshold, so one call with a zero
/// dt snapped every breakable joint in the world at once, irreversibly (nothing clears
/// `is_broken`). Only a joint left at the constructor's `f32::INFINITY` survived, since
/// `inf > inf` is false.
///
/// The Baumgarte term `position_bias·error/dt` fails the same way one level down: on a row
/// whose error is exactly ZERO it is `0/0`, and the NaN λ that follows is multiplied into the
/// row direction and written straight into the body's velocity. The two failures need
/// different scenes, so this test uses two:
///
/// * a **ball-socket**, which is three linear rows and no angular ones (`solve_fixed_joint`'s
///   angular lock is gated to real Fixed joints), so every row is loaded, the pass impulse
///   stays finite and it is purely the division by dt that decides the break;
/// * a **Fixed** joint, which adds three angular rows at exactly zero error — the `0/0`.
///
/// Keeping them apart matters: on the weld the NaN reaches the linear rows through the body's
/// angular velocity by the second iteration, the reported force becomes NaN, and `NaN >
/// break_force` is FALSE. A weld therefore does NOT break at `dt == 0`, and asserting that it
/// does not would have passed on the broken code for the wrong reason.
#[test]
fn a_zero_length_step_breaks_nothing_and_writes_no_nan() {
    // ── (a) the break check ──
    let mut w = offset_pair(Vec3::splat(0.5), f32::MAX, Joint::ball_socket);
    let before = w.velocities.clone();

    solve_once(&mut w, 0.0);

    assert!(
        !w.joints[0].is_broken,
        "dt == 0: no time passed, so no impulse was delivered and nothing can have broken — \
         `‖Σλn‖/0` = +inf cleared even an f32::MAX threshold"
    );
    assert_eq!(
        w.velocities, before,
        "dt == 0 must not move a velocity at all — the Baumgarte term saturates at \
         max_correction_speed instead of vanishing, so a zero-length step kicked the body"
    );

    // …and the threshold really is live at a real dt, or the assertions above prove nothing:
    // the same joint under the same error carries ~520 N, which a 1 N threshold must trip.
    let mut live = offset_pair(Vec3::splat(0.5), 1.0, Joint::ball_socket);
    solve_once(&mut live, 1.0 / 60.0);
    assert!(
        live.joints[0].is_broken,
        "control: this joint is breakable and loaded — a 1 N threshold must trip at a real dt"
    );

    // …and an f32::MAX threshold survives that same real step, so "did not break" above is a
    // statement about dt, not about the load.
    let mut strong = offset_pair(Vec3::splat(0.5), f32::MAX, Joint::ball_socket);
    solve_once(&mut strong, 1.0 / 60.0);
    assert!(
        !strong.joints[0].is_broken,
        "control: an f32::MAX threshold must survive the load at a real dt"
    );

    // ── (b) the 0/0 rows ──
    let mut weld = offset_pair(Vec3::splat(0.5), f32::INFINITY, Joint::fixed);
    solve_once(&mut weld, 0.0);
    assert!(
        weld.velocities[1].linear.is_finite() && weld.velocities[1].angular.is_finite(),
        "dt == 0 must not write NaN into a body — a Fixed joint's angular rows carry exactly \
         zero error, so `position_bias·0/0` is NaN and λ with it: got linear {:?} angular {:?}",
        weld.velocities[1].linear,
        weld.velocities[1].angular
    );
}

