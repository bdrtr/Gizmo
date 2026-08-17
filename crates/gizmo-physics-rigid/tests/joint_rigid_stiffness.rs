//! A rigid joint row (`compliance == 0`) is a SPRING at `JointSolver::rigid_hertz`, and this
//! file is what says so.
//!
//! # What changed and why
//!
//! A rigid row used to be solved as a hard equality with a Baumgarte push — `β·C/dt`, velocity
//! clamped, `mass_scale = 1`, `impulse_scale = 0`. That last zero is the defect: there is no
//! `−impulse_scale·λ` feedback term, so the row converges to a hard equality with no compliance
//! at all. (Not, as an earlier draft of this comment said, because the row accumulates its own
//! residual — with `mass_scale = 1, impulse_scale = 0` the coefficient on λ in the update is
//! already zero. The memory lives in the Gauss–Seidel coupling between rows; see
//! `JointSolver::rigid_hertz`.) Measured consequence
//! (`docs/ENGINE.md` §7 commit 5): joint warm start at its natural factor of 1.0 — the value
//! the CONTACT solver runs its own warm start at — destroyed a 16-link chain, and the identical
//! chain at `compliance = 1e-6`, i.e. taking the soft branch, was stable and better than cold.
//!
//! Rigid rows now take the same Box2D-v3 soft formulation, with ω from a frequency rather than
//! from `√(k/α)` (undefined at `compliance == 0`). The observable contract is a closed form:
//!
//! ```text
//! static constraint error = a / ω²,   ω = 2π · min(rigid_hertz, 1/dt)
//! ```
//!
//! `a` is the acceleration the row holds off. Note what is NOT in it: the mass, the iteration
//! count, and — below the clamp — the substep rate. Neither Baumgarte nor a compliance can
//! express that, which is why each is a separate test below.
//!
//! # The one row class that stayed hard
//!
//! `a/ω²` is a STATIC error, and a row only has one because it has a position term to balance
//! the load against. A row with no position term — `fixed.rs`'s 3-axis angular lock, the only
//! one in the crate — has no static answer at all: softening it left `impulse_scale · v` behind
//! with nothing to take it back, and the weld angle drifted linearly and forever. It is
//! therefore excluded from the reformulation and stays a hard velocity constraint on every
//! path; `a_welded_lock_does_not_creep_under_sustained_torque` and
//! `a_welded_locks_drift_does_not_grow_with_time` are what hold that.
//!
//! # The cost, stated where it is measured
//!
//! Baumgarte converged to ZERO error given enough iterations; a spring does not. Every number
//! here is therefore a stiffness LOSS relative to the old solver, and the tests are bounds on
//! how much. `joint_compliance.rs::zero_compliance_is_still_rigid` carries the same bar for the
//! headline case.

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::world::EntityIndexMap;
use gizmo_physics_rigid::{Joint, JointSolver, PhysicsWorld, RigidBody, Velocity};

/// `ω²` at the shipped default: `(2π·200)²`.
const OMEGA_SQ: f32 = (2.0 * std::f32::consts::PI * 200.0) * (2.0 * std::f32::consts::PI * 200.0);
const G: f32 = 9.81;

// ── the hanging rope: one rigid row, one known load ──────────────────────────

/// A mass hanging on a rigid rope of rest length 2, left to settle. Returns the steady-state
/// stretch beyond the rest length — i.e. the row's static constraint error.
///
/// Same shape as `joint_compliance.rs::hanging_rope_stretch`, deliberately: that file measures
/// `compliance > 0` on this scene, this one measures `compliance == 0` on it, and the two are
/// only comparable because the scene is the same.
fn rigid_rope_stretch(mass: f32, iterations: usize, rigid_hertz: f32) -> f32 {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0));
    world.joint_solver.iterations = iterations;
    world.joint_solver.rigid_hertz = rigid_hertz;

    let mut a = RigidBody::new_static();
    a.wake_up();
    world.add_body(
        BodyHandle::from_id(1),
        a,
        Transform::new(Vec3::new(0.0, 10.0, 0.0)),
        Velocity::default(),
        Collider::sphere(0.1),
    );

    let mut rb = RigidBody::new(mass, true);
    rb.wake_up();
    let col = Collider::sphere(0.3);
    rb.update_inertia_from_collider(&col);
    world.add_body(
        BodyHandle::from_id(2),
        rb,
        Transform::new(Vec3::new(0.0, 8.0, 0.0)),
        Velocity::default(),
        col,
    );

    world.joints.push(Joint::rope(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::ZERO,
        2.0,
    ));

    for _ in 0..2000 {
        world.step(1.0 / 240.0).ok();
    }
    (world.transforms[1].position - Vec3::new(0.0, 10.0, 0.0)).length() - 2.0
}

/// **The falsifier.** Everything else in this file, and every number in the design that
/// produced it, rests on the claim that a rigid row is a spring at `rigid_hertz` and that its
/// frequency is independent of the row's effective mass `k`. If this misses, that claim is
/// wrong somewhere and none of the rest means anything.
///
/// Predicted `g/ω² = 6.21e-6 m`; measured 6.199e-6. The f32 noise floor on a 2 m length is
/// ~2.4e-7, so 20% is a real band and not a rounding artefact.
#[test]
fn a_rigid_row_is_a_spring_at_the_configured_frequency() {
    let predicted = G / OMEGA_SQ;
    let measured = rigid_rope_stretch(1.0, 10, 200.0);
    let ratio = measured / predicted;
    assert!(
        (ratio - 1.0).abs() < 0.2,
        "a rigid row must be a spring at rigid_hertz: predicted a/ω² = {predicted:.9} m, \
         measured {measured:.9} m — ratio {ratio:.4}"
    );
}

/// …and the frequency, not the compliance, is what is fixed: **the sag does not depend on the
/// mass.**
///
/// The joint analogue of `contact_soft_stability.rs::penetration_recovery_does_not_depend_on_mass`,
/// and the property that separates a constant-FREQUENCY law from a constant-STIFFNESS one. A
/// compliance cannot produce it (Hooke gives `stretch ∝ mass`, which is what
/// `joint_compliance.rs::compliance_is_an_inverse_stiffness` asserts three orders of magnitude
/// of), and neither can Baumgarte, which has no static answer at all.
#[test]
fn rigid_row_stiffness_does_not_depend_on_mass() {
    let reference = rigid_rope_stretch(1.0, 10, 200.0);
    for mass in [100.0_f32, 1000.0] {
        let measured = rigid_rope_stretch(mass, 10, 200.0);
        let ratio = measured / reference;
        assert!(
            (ratio - 1.0).abs() < 0.05,
            "{mass} kg stretched {measured:.9} m against {reference:.9} m at 1 kg (ratio \
             {ratio:.4}) — a rigid row is parameterised by FREQUENCY, so `a/ω²` must be the \
             same for every mass hanging on it"
        );
    }
}

/// …and it is a property of the JOINT, not of the solver loop.
///
/// Exact companion of `joint_compliance.rs::compliance_does_not_depend_on_the_iteration_count`.
/// `iterations` is a public quality knob: raising it must buy a closer approach to the same
/// answer, never a different answer. Under the old rigid path this was true only in the limit —
/// Baumgarte's answer is zero error and the loop crawls toward it — so a scene tuned at one
/// iteration count behaved differently at another right up until it converged.
#[test]
fn rigid_row_stiffness_does_not_depend_on_the_iteration_count() {
    let reference = rigid_rope_stretch(1.0, 5, 200.0);
    for iterations in [10_usize, 20, 40] {
        let measured = rigid_rope_stretch(1.0, iterations, 200.0);
        assert!(
            (measured - reference).abs() < 1e-6,
            "stretch moved from {reference:.9} at 5 iterations to {measured:.9} at \
             {iterations} — stiffness must be a joint property, not a solver-loop artefact"
        );
    }
}

/// …and it does not depend on the SUBSTEP RATE either, below the `1/dt` clamp.
///
/// **A forward guard, not a regression test — it is green on the old code too**, and saying so
/// matters: the old rigid path converged to zero error at every `dt`, so it satisfies this
/// trivially. What the test defends is that the fix did not buy frequency-parameterisation at
/// the price of a dt-dependent stiffness, which is what any design that pins `bias_rate = β/dt`
/// would have produced.
///
/// Hand-rolled rather than driven through `PhysicsWorld`, which always substeps at its own
/// `FIXED_DT` — a bare `JointSolver::solve_joints` is a supported embedding path (see
/// `EntityIndexMap`), and it is the only way to vary `dt` at all.
#[test]
fn rigid_row_stiffness_does_not_depend_on_the_substep_rate() {
    fn settle(dt: f32, steps: usize) -> f32 {
        let solver = JointSolver::default();
        let mut anchor = RigidBody::new_static();
        anchor.wake_up();
        let mut bob = RigidBody::new(1.0, true);
        bob.wake_up();
        let bodies = [anchor, bob];
        let mut transforms = [
            Transform::new(Vec3::new(0.0, 10.0, 0.0)),
            Transform::new(Vec3::new(0.0, 8.0, 0.0)),
        ];
        let mut velocities = [Velocity::default(), Velocity::default()];
        let map: EntityIndexMap = [(1u32, 0usize), (2u32, 1usize)].into_iter().collect();
        let mut joints = vec![Joint::rope(
            BodyHandle::from_id(1),
            BodyHandle::from_id(2),
            Vec3::ZERO,
            Vec3::ZERO,
            2.0,
        )];

        for _ in 0..steps {
            velocities[1].linear.y -= G * dt;
            solver.solve_joints(&mut joints, &map, &bodies, &transforms, &mut velocities, dt);
            transforms[1].position += velocities[1].linear * dt;
        }
        (transforms[1].position - Vec3::new(0.0, 10.0, 0.0)).length() - 2.0
    }

    let coarse = settle(1.0 / 240.0, 4000);
    let fine = settle(1.0 / 480.0, 8000);
    let ratio = fine / coarse;
    assert!(
        (ratio - 1.0).abs() < 0.05,
        "halving the substep moved the static stretch from {coarse:.9} m to {fine:.9} m \
         (ratio {ratio:.4}) — `a/ω²` contains no dt, so a rigid row's stiffness must not \
         either"
    );
}

/// **The rollback lever works.** `rigid_hertz = 0` restores the Baumgarte path exactly, and on
/// this scene that means a stretch at the f32 noise floor rather than 6.2 µm.
///
/// Pinned because the lever is the supported way out of this change if a downstream scene turns
/// out to need the old stiffness, and a lever nobody exercises rots.
/// `golden_state.rs::golden_hinge_pendulum_swing_legacy_baumgarte` pins the same path against
/// the pre-change golden constants.
#[test]
fn zero_rigid_hertz_restores_the_baumgarte_path() {
    let legacy = rigid_rope_stretch(1.0, 10, 0.0);
    let soft = rigid_rope_stretch(1.0, 10, 200.0);
    assert!(
        legacy.abs() < 1e-6,
        "the legacy Baumgarte path converges to zero constraint error, measured {legacy:.9} m"
    );
    assert!(
        legacy < soft * 0.5,
        "the legacy path must be measurably stiffer than the 200 Hz spring \
         (legacy {legacy:.9} m, soft {soft:.9} m) — if these agree, the branch is not live"
    );
}

// ── the defect itself: warm start must stop pumping ──────────────────────────

const CHAIN_LINKS: u32 = 16;

/// The `docs/ENGINE.md` B4 chain, tip mass parameterised.
fn chain(tip_mass: f32) -> PhysicsWorld {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0));
    let mut anchor = RigidBody::new_static();
    anchor.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        anchor,
        Transform::new(Vec3::new(0.0, 16.0, 0.0)),
        Velocity::default(),
        Collider::sphere(0.1),
    );
    for i in 1..=CHAIN_LINKS {
        let mass = if i == CHAIN_LINKS { tip_mass } else { 1.0 };
        let mut rb = RigidBody::new(mass, true);
        let col = Collider::box_collider(Vec3::splat(0.1));
        rb.update_inertia_from_collider(&col);
        rb.wake_up();
        world.add_body(
            BodyHandle::from_id(i),
            rb,
            Transform::new(Vec3::new(0.0, 16.0 - i as f32, 0.0)),
            Velocity::default(),
            col,
        );
    }
    for i in 1..=CHAIN_LINKS {
        world.joints.push(Joint::rope(
            BodyHandle::from_id(i - 1),
            BodyHandle::from_id(i),
            Vec3::ZERO,
            Vec3::ZERO,
            1.0,
        ));
    }
    world
}

/// Run the chain and return (total constraint error of the tip in metres, max |v| over the
/// links). The links are kept awake on purpose: a sleeping chain proves nothing about a solver.
fn run_chain(
    tip_mass: f32,
    iterations: usize,
    rigid_hertz: f32,
    warm_start_factor: f32,
    substeps: usize,
) -> (f32, f32) {
    let mut w = chain(tip_mass);
    w.joint_solver.iterations = iterations;
    w.joint_solver.rigid_hertz = rigid_hertz;
    w.joint_solver.warm_start_factor = warm_start_factor;
    for _ in 0..substeps {
        for i in 1..=CHAIN_LINKS as usize {
            w.rigid_bodies[i].wake_up();
        }
        w.step(1.0 / 240.0).ok();
    }
    // 16 links of rest length 1 hanging from y = 16 put a taut tip at y = 0.
    let error = -w.transforms[CHAIN_LINKS as usize].position.y;
    let max_v = (1..=CHAIN_LINKS as usize)
        .map(|i| w.velocities[i].linear.length())
        .fold(0.0_f32, f32::max);
    (error, max_v)
}

/// **The regression test for the defect this change exists to fix.** RED without it, by four
/// orders of magnitude on the velocity.
///
/// A warm start hands the solver last substep's λ. On a rigid row with no feedback term that is
/// an integrator with no leak: the Baumgarte residual accumulates and the mechanism blows up.
/// Measured on the legacy path (`rigid_hertz = 0`), 10 iterations, 4000 substeps, factor 1.0:
///
/// | tip | legacy path | soft path |
/// |---|---|---|
/// | 200 kg | error 4.01 m ← DESTROYED, max\|v\| 13.99 | error 0.089 m, max\|v\| 0.27 |
/// | 1 kg | max\|v\| 1.04 ← ringing | max\|v\| 0.0001 |
///
/// The bars below are deliberately loose against the soft column and impossible for the legacy
/// one: this asserts "the mechanism is no longer destroyed", which is the claim, and not a
/// specific quality of warm start. **That decision is now made** (2026-08-17): the default is
/// `warm_start_factor = 0.5`, measured below in `JointSolver::warm_start_factor`'s docs. The bars
/// here stay loose on purpose — they assert "the mechanism is not destroyed", and a committed
/// scene that pinned warm start's exact numbers would turn a tuning knob into a contract.
#[test]
fn warm_starting_a_rigid_row_does_not_pump_energy() {
    // The ordinary chain first: the earlier round's finding was that this is NOT only a
    // high-mass-ratio problem, and a 1 kg tip ringing at 1 m/s is the proof.
    let (error, max_v) = run_chain(1.0, 10, 200.0, 1.0, 4000);
    assert!(
        error.abs() < 0.05,
        "1 kg tip, warm start at 1.0: the chain must hang taut, constraint error {error:.4} m"
    );
    assert!(
        max_v < 0.05,
        "1 kg tip, warm start at 1.0: the chain must be at rest, max|v| {max_v:.4} m/s \
         (legacy Baumgarte path: 1.04)"
    );

    // …and then the ill-conditioned one. 200:1 at ten iterations is not expected to be
    // beautiful — only bounded.
    let (error, max_v) = run_chain(200.0, 10, 200.0, 1.0, 4000);
    assert!(
        error.abs() < 1.0,
        "200 kg tip, warm start at 1.0: constraint error {error:.4} m — the legacy path \
         collapsed to 12 m of error here"
    );
    assert!(
        max_v < 2.0,
        "200 kg tip, warm start at 1.0: max|v| {max_v:.4} m/s — the legacy path held 14 m/s \
         of residual motion that never decayed"
    );
}

/// The price of the fix, pinned: the converged chain now rests on a SAG FLOOR that no iteration
/// count removes.
///
/// Constraint error, 200 kg tip, 160 iterations: 0.0112 m before this change (Baumgarte grinds
/// toward zero), 0.0387 m after. A constant-frequency law sags as `F/(ω²·m_eff)`, and this
/// chain's anchor row carries ≈4200 m/s² — 430× what an ordinary joint carries, which is why
/// this scene and not a weld is what bounds the choice of `rigid_hertz`.
///
/// The crate had no rest-under-load assertion at a high mass ratio at all. This is it, and the
/// number it guards is the honest cost of the change: "raise `iterations`" stops working as
/// advice past ≈0.04 m on this scene.
#[test]
fn the_chain_sag_floor_is_bounded() {
    let (error, max_v) = run_chain(200.0, 160, 200.0, 0.0, 4000);
    assert!(
        error.abs() < 0.06,
        "converged 200 kg chain: constraint error {error:.4} m (expected ≈0.039). Above this \
         the joints are visibly soft; if it has grown, `rigid_hertz` or the load path moved"
    );
    assert!(
        max_v < 0.01,
        "converged 200 kg chain: it must be at REST for the error to mean anything, \
         max|v| {max_v:.4}"
    );
}

// ── the row class that must NOT be softened ──────────────────────────────────

/// **A weld does not rotate, ever, and the soft rows must not have bought that away.**
///
/// A soft row leaves `impulse_scale · v_pre` behind and what takes it back is the row's
/// POSITION term. `fixed.rs`'s 3-axis angular lock has none — it passes a literal
/// `error = 0.0`, because `JointData::Fixed` carries no reference pose to servo toward — so
/// softening it uniformly with every other rigid row made the residual permanent and the weld
/// angle integrate **linearly and without bound**:
///
/// ```text
/// ω_residual = α·dt/c = 1.10e-4 rad/s per rad/s²,   c = dt·ω·(2ζ + dt·ω)
/// ```
///
/// which measured 7.3° in 40 s under a sustained 10 rad/s². `apply_angular_constraint_soft`
/// therefore keeps a row with no position term HARD on every path. The scene below is the one
/// that discriminates: the anchor sits on BOTH centres of mass, and — this is the part that is
/// easy to get wrong — an offset anchor does **not** help, because rotating a body about a
/// pinned anchor point leaves that anchor coincident and the linear rows see nothing at all.
///
/// The bound is 1e-5 rad over 5 s, not the 0.01 that a bounded-creep reading of this scene
/// would justify: the property is *no drift*, the legacy path delivered exactly that, and the
/// closed form above would put 0.0055 rad here if the row were ever softened again. Anything
/// between the two is the regression coming back at a smaller `rigid_hertz`.
#[test]
fn a_welded_lock_does_not_creep_under_sustained_torque() {
    let angle = welded_drift(5);
    assert!(
        angle < 1e-5,
        "a COM-anchored weld under a sustained 10 rad/s² drifted {angle:.9} rad over 5 s. \
         A weld must not rotate at all. 0.0055 rad is what a SOFT row would give here \
         (α·dt/c), and the drift is linear in time, so a 5-second bar of 0.01 would have \
         hidden 7.3° at 40 s — the row must keep `impulse_scale = 0`"
    );
}

/// …and the same weld run four times as long must not have drifted four times as far.
///
/// The defect this guards was LINEAR IN TIME, which is exactly the shape a single fixed-horizon
/// assertion cannot see (`docs/ENGINE.md` §8: choose the horizon past the onset). Two horizons
/// in one scene catch it whatever the rate turns out to be — a softened row at any
/// `rigid_hertz` quadruples this, a hard one leaves both at the noise floor.
#[test]
fn a_welded_locks_drift_does_not_grow_with_time() {
    let short = welded_drift(5);
    let long = welded_drift(20);
    assert!(
        short < 1e-5 && long < 1e-5,
        "a welded lock drifted {short:.9} rad in 5 s and {long:.9} rad in 20 s — its angular \
         rows are velocity-only, so any non-zero rate here integrates forever"
    );
}

/// A weld with its anchor on BOTH centres of mass, held awake under a constant 10 rad/s² about
/// Z for `seconds`, returning the absolute rotation the welded body ended up with.
///
/// The COM-coincident anchor is the discriminating geometry: it leaves the joint's three
/// angular rows alone with the torque. Note that an OFFSET anchor would not actually change
/// that — rotating a body about a pinned anchor point leaves the anchor coincident, so the
/// linear rows never see the rotation — but this scene removes the question entirely by
/// skipping the linear rows outright (`err_len < 1e-4`).
fn welded_drift(seconds: usize) -> f32 {
    let mut w = PhysicsWorld::new().with_gravity(Vec3::ZERO);

    let mut a = RigidBody::new_static();
    a.wake_up();
    w.add_body(
        BodyHandle::from_id(1),
        a,
        Transform::new(Vec3::ZERO),
        Velocity::default(),
        Collider::sphere(0.1),
    );
    let mut rb = RigidBody::new(1.0, true);
    let col = Collider::box_collider(Vec3::splat(0.2));
    rb.update_inertia_from_collider(&col);
    rb.wake_up();
    w.add_body(
        BodyHandle::from_id(2),
        rb,
        Transform::new(Vec3::ZERO),
        Velocity::default(),
        col,
    );
    w.joints.push(Joint::fixed(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::ZERO,
    ));

    let inertia_z = w.rigid_bodies[1].local_inertia.z;
    for _ in 0..(seconds * 240) {
        w.rigid_bodies[1].wake_up();
        w.rigid_bodies[1].torque_accumulator += Vec3::new(0.0, 0.0, 10.0 * inertia_z);
        w.step(1.0 / 240.0).ok();
    }

    let q: Quat = w.transforms[1].rotation;
    2.0 * q.w.abs().clamp(-1.0, 1.0).acos()
}
