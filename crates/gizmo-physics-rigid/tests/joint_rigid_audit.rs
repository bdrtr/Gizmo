//! **Independent measurement** of the rigid-row soft reformulation (`JointSolver::rigid_hertz`).
//!
//! Every test in this file is `#[ignore]`d: these are measurements printed for a human, not
//! gates. They carry assertions anyway — a measurement whose headline claim cannot fail is not
//! worth re-running — but `cargo test` will not run them.
//!
//! ```text
//! cargo test -p gizmo-physics-rigid --test joint_rigid_audit -- --ignored --nocapture
//! ```
//!
//! # Why this file exists next to `joint_rigid_stiffness.rs`
//!
//! That file was written by the change's author. This one was written against the same claims
//! by someone who did not build it, and it deliberately shares **no helper, no constant and no
//! scene** with it. Where the two agree, the agreement is evidence; a shared fixture would only
//! have made it a shared bug.
//!
//! # How "before" is measured
//!
//! `rigid_hertz = 0` selects the legacy Baumgarte path, and the solver keeps the old expression
//! `β·error/dt` literal precisely so that path stays bit-identical to the pre-change build
//! (`golden_state.rs::golden_hinge_pendulum_swing_legacy_baumgarte` reproduces all five
//! pre-change constants exactly). So BEFORE and AFTER are both measured here, in one binary, on
//! one scene, with the branch as the only difference — no cross-build comparison, no rebuild
//! drift.
//!
//! The chain scene below is calibrated against `docs/FIXPLAN.md`'s recorded numbers before it is
//! used to judge anything: see `q0_the_scene_reproduces_the_recorded_baseline`.
//!
//! # One caveat that applies to the whole file, stated once
//!
//! `warm_start_factor` did not exist in the build that recorded the 17.54 / 4.97 / 4.83
//! collapse — that measurement used a warm start that was written and then reverted. The knob
//! in the tree today is a REIMPLEMENTATION of it. On the legacy path it still collapses the
//! heavy chains, and at 4000 substeps it lands on the author's recorded legacy figures to four
//! decimals (see `q1b`), but it is measurably GENTLER on the 1 kg chain than the reverted
//! version was. So "legacy + f = 1.0" here is the same defect, not the same trajectory.

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{Joint, PhysicsWorld, RigidBody, Velocity};
use std::time::Instant;

/// The chain's frame step. `PhysicsWorld` substeps this internally at 1/240, so one frame is
/// four solver passes and 400 frames is the 1600 substeps every recorded chain number used.
const FRAME_DT: f32 = 1.0 / 60.0;
/// The horizon `docs/FIXPLAN.md`'s cold and warm-start chain tables were taken at.
const FRAMES: usize = 400;
/// The horizon its commit-5 acceptance table was taken at (4000 substeps).
const LONG_FRAMES: usize = 1000;
/// Links in the chain; the anchor sits at y = LINKS so a fully taut chain puts its tip at y = 0
/// and "length" reads directly as the chain's stretched length in metres.
const LINKS: usize = 16;
const ANCHOR_Y: f32 = LINKS as f32;
const G: f32 = 9.81;
/// The shipped `rigid_hertz`. Spelled out rather than read from `Default` so that a change to
/// the default shows up here as a disagreement instead of silently retuning the audit.
const SHIPPED_HZ: f32 = 200.0;
/// The legacy Baumgarte path = the pre-change solver.
const LEGACY_HZ: f32 = 0.0;

// ── scene: a 16-link rope chain with a heavy tip ─────────────────────────────

/// Anchor (static, y = 16) → 16 dynamic links on 1 m rope joints, 1 kg each but `tip_mass` on
/// the last one. Starts perfectly taut and vertical, so the only thing the solver has to do is
/// hold it up: any length beyond 16.0 m is constraint error, not swing.
fn chain(tip_mass: f32, iterations: usize, rigid_hertz: f32, warm_factor: f32) -> PhysicsWorld {
    let mut w = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0));
    w.joint_solver.iterations = iterations;
    w.joint_solver.rigid_hertz = rigid_hertz;
    w.joint_solver.warm_start_factor = warm_factor;

    let mut anchor = RigidBody::new_static();
    anchor.wake_up();
    w.add_body(
        BodyHandle::from_id(0),
        anchor,
        Transform::new(Vec3::new(0.0, ANCHOR_Y, 0.0)),
        Velocity::default(),
        Collider::sphere(0.1),
    );
    for i in 1..=LINKS {
        let m = if i == LINKS { tip_mass } else { 1.0 };
        let mut rb = RigidBody::new(m, true);
        let col = Collider::box_collider(Vec3::splat(0.1));
        rb.update_inertia_from_collider(&col);
        rb.wake_up();
        w.add_body(
            BodyHandle::from_id(i as u32),
            rb,
            Transform::new(Vec3::new(0.0, ANCHOR_Y - i as f32, 0.0)),
            Velocity::default(),
            col,
        );
    }
    for i in 1..=LINKS {
        w.joints.push(Joint::rope(
            BodyHandle::from_id(i as u32 - 1),
            BodyHandle::from_id(i as u32),
            Vec3::ZERO,
            Vec3::ZERO,
            1.0,
        ));
    }
    w
}

/// What a chain run reports.
///
/// **`final_len` is here only to reproduce the recorded tables, not to judge anything.** The
/// legacy 200 kg chain never settles — traced frame by frame it wanders between 16.05 and 16.31
/// with 2–3.5 m/s spikes right up to the last frame — so a single end-of-run sample of it is a
/// sample of an oscillation. Everything below judges the WINDOW statistics instead.
struct Chain {
    /// Anchor y minus tip y at the last frame. 16.0 is the exact rigid answer.
    final_len: f32,
    /// Mean chain length over the last quarter of the run.
    mean_len: f32,
    /// Peak-to-peak swing of that length over the same window — how much the "settled" chain
    /// is actually breathing.
    p2p_len: f32,
    /// Largest speed any link reached anywhere in that window. A settled chain reads ≤ 0.05.
    tail_max_v: f32,
    /// Largest link speed at the very last frame. Same caveat as `final_len` — reported only
    /// because the recorded tables are end-of-run samples.
    final_max_v: f32,
}

/// How a run is classified. Definitions are fixed here so the labels in the tables mean
/// something checkable.
#[derive(PartialEq)]
enum Verdict {
    /// Mean length within 1 m of the rigid answer and nothing in the window above 1 m/s.
    Held,
    /// Held its length, but still carrying 1–5 m/s. Not a collapse; not a settled mechanism.
    Ringing,
    /// Mean length off by more than 1 m, or more than 5 m/s left in the window.
    Destroyed,
}

impl Chain {
    fn verdict(&self) -> Verdict {
        if !self.mean_len.is_finite() || (self.mean_len - ANCHOR_Y).abs() > 1.0 || self.tail_max_v > 5.0 {
            Verdict::Destroyed
        } else if self.tail_max_v > 1.0 {
            Verdict::Ringing
        } else {
            Verdict::Held
        }
    }
    fn tag(&self) -> &'static str {
        match self.verdict() {
            Verdict::Held => "",
            Verdict::Ringing => "RINGING",
            Verdict::Destroyed => "DESTROYED",
        }
    }
}

fn measure(tip_mass: f32, iterations: usize, hz: f32, warm: f32, frames: usize) -> Chain {
    let mut w = chain(tip_mass, iterations, hz, warm);
    let window_from = frames - frames / 4;
    let (mut sum, mut n) = (0.0f64, 0usize);
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    let mut tail_max_v = 0.0f32;
    for f in 0..frames {
        w.step(FRAME_DT).ok();
        if f >= window_from {
            let len = ANCHOR_Y - w.transforms[LINKS].position.y;
            sum += len as f64;
            n += 1;
            lo = lo.min(len);
            hi = hi.max(len);
            for i in 1..=LINKS {
                tail_max_v = tail_max_v.max(w.velocities[i].linear.length());
            }
        }
    }
    let mut final_max_v = 0.0f32;
    for i in 1..=LINKS {
        final_max_v = final_max_v.max(w.velocities[i].linear.length());
    }
    Chain {
        final_len: ANCHOR_Y - w.transforms[LINKS].position.y,
        mean_len: (sum / n as f64) as f32,
        p2p_len: hi - lo,
        tail_max_v,
        final_max_v,
    }
}

// ── Q0: the scene is the recorded one ────────────────────────────────────────

/// **Calibration, and it runs first for a reason.** Every table below compares a legacy column
/// against a soft column; if my chain is not the chain `docs/FIXPLAN.md` measured, the legacy
/// column is not the recorded baseline and none of the comparisons transfer.
///
/// This scene was rebuilt from the FIXPLAN prose alone — 16 links, 1 m rope joints, 1 kg each,
/// static anchor at y = 16, default gravity — and it reproduces the recorded pre-change cold sag
/// to under 0.6 mm on every one of nine cells. That also pins two things the prose left implicit:
/// the horizon (400 frames of 1/60) and the fact that the recorded runs did not force the links
/// awake (forcing them awake changes nothing here; measured separately).
#[test]
#[ignore = "measurement; run explicitly with --ignored"]
fn q0_the_scene_reproduces_the_recorded_baseline() {
    let recorded = [
        (1.0f32, [16.0066f32, 16.0014, 16.0002]),
        (20.0, [16.0229, 16.0056, 16.0012]),
        (200.0, [16.1657, 16.0443, 16.0106]),
    ];
    println!("\nQ0  legacy path (rigid_hertz = 0) vs docs/FIXPLAN.md's recorded pre-change sag");
    println!("       tip   iter    measured    recorded       diff");
    let mut worst = 0.0f32;
    for (mass, row) in recorded {
        for (col, iters) in [10usize, 40, 160].into_iter().enumerate() {
            let m = measure(mass, iters, LEGACY_HZ, 0.0, FRAMES).final_len;
            let d = m - row[col];
            worst = worst.max(d.abs());
            println!("    {mass:>6} kg  {iters:>4}   {m:>9.4}   {:>9.4}  {d:>+9.5}", row[col]);
        }
    }
    println!("    worst |diff| = {worst:.5} m");
    assert!(
        worst < 1e-3,
        "this scene is not the recorded one — worst deviation {worst:.5} m from FIXPLAN's \
         pre-change column; every table below would be comparing against the wrong baseline"
    );
}

// ── Q1: is the warm-start instability gone? ──────────────────────────────────

/// **The acceptance probe.** `warm_start_factor = 1.0` is the natural factor — the value the
/// CONTACT solver runs its own warm start at — and it is what destroyed this chain before the
/// change: a recorded 17.54 m and 30 m/s on a 1 kg tip, 4.97 m on a 20 kg tip, 4.83 m and
/// 44 m/s on a 200 kg tip, against a converged 16.000.
///
/// Both paths are swept in one run, so the collapse is reproduced and then removed side by side.
#[test]
#[ignore = "measurement; run explicitly with --ignored"]
fn q1_warm_start_at_factor_one() {
    println!("\nQ1  16-link chain, 10 iterations, {FRAMES} frames @ 1/60");
    println!("    len = mean chain length over the last quarter (16.000 = rigid); p2p = its swing;");
    println!("    v = fastest link anywhere in that window.");
    println!("           legacy Baumgarte (hz = 0)              soft rows (hz = 200)");
    println!("      tip     f       len     p2p       v            len     p2p       v");
    let mut soft_held = true;
    let mut legacy_lost = 0;
    for mass in [1.0f32, 20.0, 200.0] {
        for f in [0.0f32, 1.0] {
            let l = measure(mass, 10, LEGACY_HZ, f, FRAMES);
            let s = measure(mass, 10, SHIPPED_HZ, f, FRAMES);
            println!(
                "  {mass:>6} kg  {f:>4.2}  {:>8.3} {:>7.3} {:>7.3} {:<10}  {:>8.3} {:>7.3} {:>7.3} {}",
                l.mean_len, l.p2p_len, l.tail_max_v, l.tag(),
                s.mean_len, s.p2p_len, s.tail_max_v, s.tag()
            );
            if f == 1.0 {
                if l.verdict() != Verdict::Held {
                    legacy_lost += 1;
                }
                soft_held &= s.verdict() != Verdict::Destroyed;
            }
        }
    }
    assert!(
        legacy_lost == 3,
        "control failed: the legacy path did NOT lose all three chains at factor 1.0 (only \
         {legacy_lost}), so this run does not show the soft path fixing anything"
    );
    assert!(
        soft_held,
        "the warm-start instability is NOT gone: a chain is still destroyed at factor 1.0 on the \
         soft path"
    );
}

/// The same probe at the author's own horizon — 1000 frames = 4000 substeps — which is where
/// their acceptance table was taken. Two things it is for.
///
/// **Cross-check.** On the legacy path this lands on their published legacy figures to four
/// decimals (200 kg: 4.010 m / 13.99 m/s; 1 kg: max|v| 1.0421) from a scene built independently
/// from prose. That is what says the two of us are measuring the same system.
///
/// **Honesty about f = 1.0.** The author reports that at 200 Hz factor 1.0 survives but does not
/// go quiet — a slowly growing limit cycle. This is where that shows up, and it is the reason
/// the shipped default for the knob is 0.0.
#[test]
#[ignore = "measurement; run explicitly with --ignored"]
fn q1b_warm_start_at_the_acceptance_horizon() {
    println!("\nQ1b  same chain, {LONG_FRAMES} frames @ 1/60 = 4000 substeps");
    println!("           legacy Baumgarte (hz = 0)              soft rows (hz = 200)");
    println!("      tip     f       len     p2p       v            len     p2p       v");
    for (mass, f) in [(1.0f32, 1.0f32), (200.0, 0.0), (200.0, 0.875), (200.0, 1.0)] {
        let l = measure(mass, 10, LEGACY_HZ, f, LONG_FRAMES);
        let s = measure(mass, 10, SHIPPED_HZ, f, LONG_FRAMES);
        println!(
            "  {mass:>6} kg  {f:>4.3}  {:>8.3} {:>7.3} {:>7.3} {:<10}  {:>8.3} {:>7.3} {:>7.3} {}",
            l.mean_len, l.p2p_len, l.tail_max_v, l.tag(),
            s.mean_len, s.p2p_len, s.tail_max_v, s.tag()
        );
        assert!(
            s.verdict() != Verdict::Destroyed,
            "soft path lost the {mass} kg chain at f = {f} over {LONG_FRAMES} frames"
        );
    }

    // The cross-check itself, in the units the author published: end-of-run sample, legacy path.
    // They report 4.010 m / 13.99 m/s for the 200 kg chain and max|v| 1.0421 for the 1 kg one.
    // Two scenes built independently from prose agreeing to four decimals on a CHAOTIC
    // trajectory is not a coincidence — it is the same scene and the same solver.
    println!("\n    cross-check against the author's published legacy figures (end-of-run):");
    // The 1 kg row has no published length — they reported only its max|v| — hence the `None`.
    for (mass, expect_len, expect_v) in [(200.0f32, Some(4.010f32), 13.99f32), (1.0, None, 1.0421)] {
        let c = measure(mass, 10, LEGACY_HZ, 1.0, LONG_FRAMES);
        let theirs = expect_len.map_or("  n/a".to_string(), |v| format!("{v:.3}"));
        println!(
            "      {mass:>6} kg  f=1.0  legacy  len {:.4} (theirs {theirs})   max|v| {:.4} (theirs {expect_v:.4})",
            c.final_len, c.final_max_v
        );
        assert!(
            (c.final_max_v - expect_v).abs() < 0.01,
            "the cross-check missed: max|v| {:.4} against the author's {expect_v:.4}. Either \
             this scene is not theirs or the warm-start operator has changed since.",
            c.final_max_v
        );
    }
}

// ── Q2: did cold behaviour regress? ──────────────────────────────────────────

/// The real cost of the change. A softer rigid row may legitimately sag MORE at rest, and this
/// says how much: the same masses and iteration counts as Q0, legacy against soft, cold.
///
/// The interesting cell is 160 iterations, because that is where the two formulations differ in
/// KIND rather than in tuning — Baumgarte converges towards zero error given enough sweeps, a
/// spring converges to `a/ω²` and stops. Raising `iterations` cannot buy that back.
///
/// The assertion is on the SHIPPED cell (10 iterations), which is what a user actually gets.
#[test]
#[ignore = "measurement; run explicitly with --ignored"]
fn q2_cold_sag() {
    println!("\nQ2  cold (warm_start_factor = 0), {FRAMES} frames @ 1/60");
    println!("    mean chain length over the last quarter, and its peak-to-peak swing");
    println!("       tip   iter      legacy   (p2p)        soft   (p2p)       delta");
    for mass in [1.0f32, 20.0, 200.0] {
        for iters in [10usize, 40, 160] {
            let l = measure(mass, iters, LEGACY_HZ, 0.0, FRAMES);
            let s = measure(mass, iters, SHIPPED_HZ, 0.0, FRAMES);
            println!(
                "    {mass:>6} kg  {iters:>4}   {:>9.4} ({:>6.4})   {:>9.4} ({:>6.4})   {:>+9.4}",
                l.mean_len, l.p2p_len, s.mean_len, s.p2p_len, s.mean_len - l.mean_len
            );
            if iters == 10 {
                let (le, se) = (
                    (l.mean_len - ANCHOR_Y).abs(),
                    (s.mean_len - ANCHOR_Y).abs(),
                );
                assert!(
                    se <= le + 1e-3,
                    "cold regression at the SHIPPED iteration count: the {mass} kg chain is \
                     {se:.4} m off true on the soft path against {le:.4} m on the legacy path"
                );
            }
        }
    }
}

/// Where the softness floor actually is: sag against iteration count on both paths, pushed far
/// past any shippable budget. Baumgarte should keep closing on 16.000; the spring should stall.
#[test]
#[ignore = "measurement; run explicitly with --ignored"]
fn q2b_the_converged_floor() {
    println!("\nQ2b  200 kg tip, cold, {FRAMES} frames — does raising `iterations` still help?");
    println!("    iter      legacy        soft");
    for iters in [10usize, 40, 160, 640] {
        let l = measure(200.0, iters, LEGACY_HZ, 0.0, FRAMES).mean_len;
        let s = measure(200.0, iters, SHIPPED_HZ, 0.0, FRAMES).mean_len;
        println!("    {iters:>4}   {l:>9.4}   {s:>9.4}");
    }
}

// ── Q3: is the constraint still stiff? ───────────────────────────────────────
//
// Criterion, stated before the numbers: a joint that was rigid must still LOOK rigid, and the
// bar is **0.1 mm (1e-4 m) of steady-state linear constraint error under load, and 1 mrad of
// angular error**. 0.1 mm is a fiftieth of the solver's own contact slop (5 mm) and is under a
// pixel at any camera distance where a 1 m joint fills the screen; 1 mrad tips a 1 m arm by
// 1 mm. Under those, a player cannot see it; over them, the joint sags.
//
// Each scene is run at 1 kg and at 100 kg, so the table also shows whether the error scales with
// the load (a compliance would) or not (a frequency will not) — that is the falsifiable half of
// the claim "a rigid row is a spring at `rigid_hertz`", checked on three joint kinds rather than
// on the rope alone.
//
// 1200 frames = 20 s, links forced awake so the measurement is of the solver's steady state and
// not of the sleep system freezing a transient.

const LIN_BAR: f32 = 1e-4;
const ANG_BAR: f32 = 1e-3;
/// 20 s at 1/60. Long enough that all three scenes are stationary to the printed digits.
const SETTLE_FRAMES: usize = 1200;

fn settle(w: &mut PhysicsWorld) {
    for _ in 0..SETTLE_FRAMES {
        for rb in w.rigid_bodies.iter_mut() {
            rb.wake_up();
        }
        w.step(FRAME_DT).ok();
    }
}

/// Rope, rest length 2, hanging mass: the cleanest single-row scene in the crate. Returns the
/// steady-state stretch beyond rest length.
fn rope_stretch(mass: f32, hz: f32) -> f32 {
    let mut w = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0));
    w.joint_solver.rigid_hertz = hz;
    let top = Vec3::new(0.0, 20.0, 0.0);

    let mut a = RigidBody::new_static();
    a.wake_up();
    w.add_body(BodyHandle::from_id(1), a, Transform::new(top), Velocity::default(), Collider::sphere(0.05));
    let mut b = RigidBody::new(mass, true);
    let col = Collider::sphere(0.25);
    b.update_inertia_from_collider(&col);
    b.wake_up();
    w.add_body(
        BodyHandle::from_id(2),
        b,
        Transform::new(top - Vec3::new(0.0, 2.0, 0.0)),
        Velocity::default(),
        col,
    );
    w.joints.push(Joint::rope(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO, 2.0));
    settle(&mut w);
    (w.transforms[1].position - top).length() - 2.0
}

/// A CANTILEVER weld: the weld point is 1 m out from the body's centre of mass, so gravity loads
/// the joint with `m·g` newtons AND `m·g·1` newton-metres at once. Returns
/// `(separation of the two anchor points, angular droop in radians)`.
///
/// A weld with the load hanging on the pin would only exercise the three linear rows; a real
/// weld that "sags" sags in ANGLE, which is what the three angular rows are for.
fn weld_droop(mass: f32, hz: f32) -> (f32, f32) {
    let mut w = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0));
    w.joint_solver.rigid_hertz = hz;
    let wall = Vec3::new(0.0, 10.0, 0.0);
    let pin = Vec3::new(1.0, 10.0, 0.0);
    let com = Vec3::new(2.0, 10.0, 0.0);

    let mut a = RigidBody::new_static();
    a.wake_up();
    w.add_body(
        BodyHandle::from_id(1),
        a,
        Transform::new(wall),
        Velocity::default(),
        Collider::box_collider(Vec3::splat(0.2)),
    );
    let mut b = RigidBody::new(mass, true);
    let col = Collider::box_collider(Vec3::new(1.0, 0.1, 0.1));
    b.update_inertia_from_collider(&col);
    b.wake_up();
    w.add_body(BodyHandle::from_id(2), b, Transform::new(com), Velocity::default(), col);
    w.joints.push(Joint::fixed(BodyHandle::from_id(1), BodyHandle::from_id(2), pin - wall, pin - com));
    settle(&mut w);
    let q = w.transforms[1].rotation;
    let lin = (w.transforms[1].position + q * (pin - com) - pin).length();
    (lin, Quat::IDENTITY.angle_between(q))
}

/// A hinge whose FREE axis is vertical and whose load is horizontally offset from the pin: the
/// weight has to be carried by the linear rows and the torque by a LOCKED angular row, with
/// nothing for the free DOF to give way on. Returns `(pin separation, axis tilt in radians)`.
fn hinge_droop(mass: f32, hz: f32) -> (f32, f32) {
    let mut w = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0));
    w.joint_solver.rigid_hertz = hz;
    let top = Vec3::new(0.0, 10.0, 0.0);
    let pin = Vec3::new(0.0, 9.0, 0.0);
    let com = Vec3::new(0.5, 9.0, 0.0);

    let mut a = RigidBody::new_static();
    a.wake_up();
    w.add_body(BodyHandle::from_id(1), a, Transform::new(top), Velocity::default(), Collider::sphere(0.1));
    let mut b = RigidBody::new(mass, true);
    let col = Collider::box_collider(Vec3::new(1.0, 0.1, 0.1));
    b.update_inertia_from_collider(&col);
    b.wake_up();
    w.add_body(BodyHandle::from_id(2), b, Transform::new(com), Velocity::default(), col);
    w.joints.push(Joint::hinge(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        pin - top,
        pin - com,
        Vec3::Y,
    ));
    settle(&mut w);
    let q = w.transforms[1].rotation;
    let lin = (w.transforms[1].position + q * (pin - com) - pin).length();
    (lin, (q * Vec3::Y).angle_between(Vec3::Y))
}

/// Steady-state constraint error at rest under load — rope, weld cantilever, hinge — legacy
/// against soft, at 1 kg and 100 kg.
///
/// Five of the six rows are stiffness questions and hold the bar. The sixth was not a stiffness
/// question at all — the Fixed joint's angular lock had no static answer, it CREPT — and it is
/// measured, explained and now guarded in
/// [`q3b_the_fixed_joints_angular_lock_does_not_creep`]. It stays out of this test's assertions
/// so that the two failure modes cannot mask one another.
#[test]
#[ignore = "measurement; run explicitly with --ignored"]
fn q3_constraint_error_at_rest_under_load() {
    println!("\nQ3  steady-state constraint error at rest under load (10 iterations, 20 s)");
    println!("    criterion: linear < {LIN_BAR:.0e} m, angular < {ANG_BAR:.0e} rad");
    println!("    scene                       mass       legacy         soft   soft/legacy");

    let mut worst_lin = 0.0f32;
    let mut worst_ang = 0.0f32;
    let row = |name: &str, mass: f32, l: f32, s: f32| {
        // The legacy path reaches EXACTLY zero on both angular rows (it converges a hard
        // equality, and in f32 the quaternion never leaves identity), so the ratio is not a
        // number there — say so rather than printing 5e36.
        let ratio = if l == 0.0 {
            "  n/a (0)".to_string()
        } else {
            format!("{:8.2}", s / l)
        };
        println!("    {name:<26} {mass:>6}  {l:>11.3e}  {s:>11.3e}   {ratio:>11}");
    };
    for mass in [1.0f32, 100.0] {
        let (l, s) = (rope_stretch(mass, LEGACY_HZ), rope_stretch(mass, SHIPPED_HZ));
        worst_lin = worst_lin.max(s.abs());
        row("rope stretch          [m]", mass, l, s);
    }
    for mass in [1.0f32, 100.0] {
        let (ll, la) = weld_droop(mass, LEGACY_HZ);
        let (sl, sa) = weld_droop(mass, SHIPPED_HZ);
        worst_lin = worst_lin.max(sl);
        row("weld cantilever, lin  [m]", mass, ll, sl);
        row("weld cantilever, ang[rad]", mass, la, sa);
        println!("      ^ velocity-only row: this was 20 s of CREEP, not a droop — see q3b");
    }
    for mass in [1.0f32, 100.0] {
        let (ll, la) = hinge_droop(mass, LEGACY_HZ);
        let (sl, sa) = hinge_droop(mass, SHIPPED_HZ);
        worst_lin = worst_lin.max(sl);
        worst_ang = worst_ang.max(sa);
        row("hinge pin, lin        [m]", mass, ll, sl);
        row("hinge axis tilt     [rad]", mass, la, sa);
    }
    println!(
        "    worst soft (excluding the weld's angular lock): linear {worst_lin:.3e} m \
         (bar {LIN_BAR:.0e}), angular {worst_ang:.3e} rad (bar {ANG_BAR:.0e})"
    );
    assert!(
        worst_lin < LIN_BAR,
        "a rigid joint no longer looks rigid: worst linear constraint error at rest is \
         {worst_lin:.3e} m against a {LIN_BAR:.0e} m bar"
    );
    assert!(
        worst_ang < ANG_BAR,
        "a rigid joint no longer looks rigid: worst angular constraint error at rest is \
         {worst_ang:.3e} rad against a {ANG_BAR:.0e} rad bar"
    );
}

/// **The one place the soft reformulation is not merely softer but structurally wrong**, and a
/// closed-form account of it.
///
/// `solve_fixed_joint`'s 3-axis angular lock calls `apply_angular_constraint` with
/// `error = 0.0`. It is a pure VELOCITY constraint with no positional term — the comment there
/// says so outright ("Velocity-level lock: the solver runs every sub-step before integration,
/// so no relative rotation accumulates"). That reasoning holds only while the row drives the
/// relative angular velocity to EXACTLY zero, which is what `mass_scale = 1, impulse_scale = 0`
/// did. A soft row does not: at equilibrium under a sustained torque it settles at
///
/// ```text
/// residual relative angular velocity = α · dt / c,   c = dt·ω·(2ζ + dt·ω)
/// ```
///
/// and with no position term in the row there is nothing to work that residual off. It
/// integrates: **the weld's angle was unbounded and linear in time** — 0.127 rad (7.3°) at 40 s
/// under 10 rad/s², growing forever, where the legacy path read exactly zero.
///
/// **FIXED during review (2026-08-09).** `apply_angular_constraint_soft` now keeps a row with
/// `error == 0.0` — one with no position term at all — a HARD velocity constraint on every
/// path. The warm-start diagnosis that motivated `rigid_hertz` does not reach such a row: with
/// no bias there is no Baumgarte residual to integrate into λ, and the per-row update
/// `λ_{n+1} = (bias − v₀)/k` is memoryless in the hard form exactly as `s·(bias − v₀)/k` is in
/// the soft one. The assertions below were inverted with the fix: every row in the table must
/// now read zero, and the closed form is kept only as the number the drift would return to.
///
/// **What never crept, which is what localised it:** the slider's angular lock (real quaternion
/// error against `initial_relative_rotation`), the D6 `Locked` angular rows (real `rvec` error)
/// and the hinge's axis alignment (`axis_a × axis_b`). Those have position terms and get a
/// bounded `α/ω²` droop. `JointData::Fixed` is the crate's only velocity-only row class, and
/// the fully-locked D6 in the identical scene below is the control that says so.
///
/// One reading to retire with it: an offset anchor does **not** cap this. Rotating a body about
/// a pinned anchor leaves that anchor coincident, so the linear rows see nothing — the 1 m
/// cantilever measured here crept at the full closed-form rate.
#[test]
#[ignore = "measurement; run explicitly with --ignored"]
fn q3b_the_fixed_joints_angular_lock_does_not_creep() {
    /// The world's internal substep. `PhysicsWorld` is fixed at 240 Hz.
    const SUBSTEP_DT: f32 = 1.0 / 240.0;

    /// Cantilever again, but returning the angle at several horizons and optionally welded with
    /// a fully-locked D6 instead of a `Fixed`, and optionally left free to fall asleep.
    fn creep(hz: f32, mass: f32, arm: f32, d6: bool, keep_awake: bool, secs: &[f32]) -> (Vec<f32>, f32) {
        let mut w = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0));
        w.joint_solver.rigid_hertz = hz;
        let wall = Vec3::new(0.0, 10.0, 0.0);
        let pin = Vec3::new(1.0, 10.0, 0.0);
        let com = pin + Vec3::new(arm, 0.0, 0.0);

        let mut a = RigidBody::new_static();
        a.wake_up();
        w.add_body(
            BodyHandle::from_id(1),
            a,
            Transform::new(wall),
            Velocity::default(),
            Collider::box_collider(Vec3::splat(0.2)),
        );
        let mut b = RigidBody::new(mass, true);
        let col = Collider::box_collider(Vec3::new(arm, 0.1, 0.1));
        b.update_inertia_from_collider(&col);
        b.wake_up();
        let inv_i = b.inv_world_inertia_tensor(Quat::IDENTITY);
        // The pin has to carry m·g, and it does so `arm` metres from the centre of mass, so the
        // body sees a constant torque m·g·arm about Z. α is that over the row's effective
        // inertia about Z — read off the body itself so no collider convention is assumed.
        let alpha = mass * G * arm * inv_i.mul_vec3(Vec3::Z).dot(Vec3::Z);
        w.add_body(BodyHandle::from_id(2), b, Transform::new(com), Velocity::default(), col);
        let (ha, hb) = (BodyHandle::from_id(1), BodyHandle::from_id(2));
        w.joints.push(if d6 {
            Joint::d6(ha, hb, pin - wall, pin - com)
        } else {
            Joint::fixed(ha, hb, pin - wall, pin - com)
        });

        let mut out = Vec::new();
        let mut frame = 0usize;
        for &s in secs {
            let target = (s * 60.0) as usize;
            while frame < target {
                if keep_awake {
                    for rb in w.rigid_bodies.iter_mut() {
                        rb.wake_up();
                    }
                }
                w.step(FRAME_DT).ok();
                frame += 1;
            }
            out.push(Quat::IDENTITY.angle_between(w.transforms[1].rotation));
        }
        (out, alpha)
    }

    /// `α · dt / c` with the solver's own `c`. Zero on the legacy path, where the row has no
    /// `impulse_scale` term at all and drives the residual velocity to zero outright.
    fn predicted_rate(alpha: f32, hz: f32) -> f32 {
        if hz <= 0.0 {
            return 0.0;
        }
        let omega = 2.0 * std::f32::consts::PI * hz.min(1.0 / SUBSTEP_DT);
        let c = SUBSTEP_DT * omega * (2.0 * 1.0 + SUBSTEP_DT * omega);
        alpha * SUBSTEP_DT / c
    }

    let secs = [5.0f32, 10.0, 20.0, 40.0];
    println!("\nQ3b  Fixed-joint angular lock: weld angle (rad) vs time, 1 m cantilever, awake");
    println!("    joint / path        mass        5 s       10 s       20 s       40 s   rad/s   predicted");
    let mut checked = 0;
    for (name, hz, d6) in [
        ("Fixed, legacy hz=0 ", LEGACY_HZ, false),
        ("Fixed, soft  hz=200", SHIPPED_HZ, false),
        // 150 Hz is the setting `docs/FIXPLAN.md` names as the price of turning warm start on
        // at factor 1.0, so what it costs in creep belongs in the same table as the decision.
        ("Fixed, soft  hz=150", 150.0f32, false),
        ("Fixed, soft  hz=100", 100.0f32, false),
        ("D6 locked, soft    ", SHIPPED_HZ, true),
    ] {
        for mass in [1.0f32, 100.0] {
            let (a, alpha) = creep(hz, mass, 1.0, d6, true, &secs);
            let rate = (a[3] - a[1]) / 30.0;
            let pred = predicted_rate(alpha, hz);
            println!(
                "    {name} {mass:>7}  {:>9.3e}  {:>9.3e}  {:>9.3e}  {:>9.3e}  {rate:>7.2e}  {pred:>9.2e}",
                a[0], a[1], a[2], a[3]
            );
            if !d6 && hz > 0.0 {
                // The row is velocity-only, so ANY non-zero rate here integrates forever: at
                // 40 s the closed form `α·dt/c` would put `pred·40` rad on the clock, and the
                // bar below is four orders of magnitude under that.
                assert!(
                    a[3] < 1e-6,
                    "{name}: a velocity-only weld row drifted {:.3e} rad at 40 s. That is the \
                     soft row's `impulse_scale·v` residual with nothing to take it back — the \
                     closed form α·dt/c = {pred:.3e} rad/s (α = {alpha:.2} rad/s²) would give \
                     {:.3e} rad here. Rows with no position term must stay hard.",
                    a[3],
                    pred * 40.0
                );
                // …and the shape, not just the magnitude: a drift that is linear in time is
                // the signature, and it is invisible to any single-horizon bar.
                assert!(
                    a[3] <= a[1] + 1e-7,
                    "{name}: drift GREW between 10 s ({:.3e}) and 40 s ({:.3e}) — linear in \
                     time is exactly the defect this row class had",
                    a[1],
                    a[3]
                );
                checked += 1;
            }
            if d6 {
                assert!(
                    a[3] < 1e-3,
                    "control failed: a fully-locked D6 in the same scene ALSO creeps ({:.3e} rad \
                     at 40 s), so the defect is not specific to Fixed's velocity-only row",
                    a[3]
                );
            }
            if hz == LEGACY_HZ {
                assert!(
                    a[3] < 1e-6,
                    "control failed: the LEGACY path creeps too ({:.3e} rad at 40 s), so this is \
                     not a regression introduced by the soft rows",
                    a[3]
                );
            }
        }
    }
    assert_eq!(checked, 6, "the soft-path creep rows did not all run");

    // Kept from the pre-fix measurement: sleep was the only mitigation on offer, and it did not
    // work — a body creeping at 1e-4 rad/s never stops rotating, so it never sleeps.
    println!("\n    the same weld left free to fall asleep (no forced wake):");
    for (name, hz) in [("legacy hz=0 ", LEGACY_HZ), ("soft   hz=200", SHIPPED_HZ)] {
        let (a, _) = creep(hz, 1.0, 1.0, false, false, &secs);
        println!(
            "    {name}   5 s {:>9.3e}   10 s {:>9.3e}   20 s {:>9.3e}   40 s {:>9.3e}",
            a[0], a[1], a[2], a[3]
        );
    }
}

/// **The central claim, re-measured at 60× the author's precision.**
///
/// The whole design rests on one closed form: a rigid row's static error is `a/ω²` with
/// `ω = 2π·min(rigid_hertz, 1/dt)`, independent of mass and of iteration count. The author
/// checked it on a 2 m rope 10 m up in the air, where a 6.2 µm stretch is 26 f32 ulps of the
/// length and the body's own y coordinate only resolves 1.9 µm. That is a 2-significant-figure
/// measurement being reported to four.
///
/// This runs the same law on a **0.25 m rope anchored at the origin**, where the length
/// resolves to 3e-8 m and the same stretch is ~200 ulps, and it sweeps ω instead of trusting one
/// point — if the exponent were wrong (a/ω instead of a/ω², say) a single hertz value could not
/// tell. 30 Hz is included because it is the contact solver's setting, and the row shows exactly
/// why it was not copied over.
#[test]
#[ignore = "measurement; run explicitly with --ignored"]
fn q3c_the_static_error_law() {
    fn stretch(hz: f32, mass: f32, iterations: usize) -> f32 {
        let mut w = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0));
        w.joint_solver.rigid_hertz = hz;
        w.joint_solver.iterations = iterations;
        let top = Vec3::new(0.0, 0.25, 0.0);
        let mut a = RigidBody::new_static();
        a.wake_up();
        w.add_body(BodyHandle::from_id(1), a, Transform::new(top), Velocity::default(), Collider::sphere(0.05));
        let mut b = RigidBody::new(mass, true);
        let col = Collider::sphere(0.05);
        b.update_inertia_from_collider(&col);
        b.wake_up();
        w.add_body(BodyHandle::from_id(2), b, Transform::new(Vec3::ZERO), Velocity::default(), col);
        w.joints.push(Joint::rope(BodyHandle::from_id(1), BodyHandle::from_id(2), Vec3::ZERO, Vec3::ZERO, 0.25));
        settle(&mut w);
        (w.transforms[1].position - top).length() - 0.25
    }

    println!("\nQ3c  static error law on a 0.25 m rope at the origin (f32 quantum ~3e-8 m)");
    println!("    hertz    predicted g/w^2       measured      ratio    (m=100)    (160 iter)");
    let mut worst = 0.0f32;
    for hz in [30.0f32, 100.0, 150.0, 200.0, 240.0] {
        let omega = 2.0 * std::f32::consts::PI * hz.min(240.0);
        let predicted = G / (omega * omega);
        let m = stretch(hz, 1.0, 10);
        let heavy = stretch(hz, 100.0, 10);
        let converged = stretch(hz, 1.0, 160);
        let ratio = m / predicted;
        worst = worst.max((ratio - 1.0).abs());
        println!(
            "    {hz:>5.0}       {predicted:>11.4e}    {m:>11.4e}    {ratio:>7.4}  {heavy:>9.3e}  {converged:>9.3e}"
        );
        // Mass- and iteration-independence are the two properties that separate a FREQUENCY law
        // from a compliance and from Baumgarte. Both are asserted at every ω, not just one.
        assert!(
            (heavy - m).abs() <= 4.0 * 3e-8,
            "at {hz} Hz the static error moved from {m:.4e} to {heavy:.4e} when the mass went \
             1 kg -> 100 kg; a frequency-parameterised row must not depend on mass"
        );
        assert!(
            (converged - m).abs() <= 4.0 * 3e-8,
            "at {hz} Hz the static error moved from {m:.4e} to {converged:.4e} between 10 and \
             160 iterations; the soft row's answer is supposed to be the CONVERGED one"
        );
    }
    println!("    worst deviation from a/w^2: {:.2}%", worst * 100.0);
    assert!(
        worst < 0.05,
        "the a/w^2 law is off by {:.1}% somewhere in the sweep — the design's central closed \
         form does not hold",
        worst * 100.0
    );
}

/// **The joint-free prediction, checked.** The design predicted that `headless_stress_test`'s
/// determinism hash would not move, because that scene has no joints. I cannot run it from here
/// (it is a demo binary and this audit is scoped to `-p gizmo-physics-rigid`), so this is the
/// in-crate equivalent of that prediction and it is strictly sharper than "the joint-free
/// committed constants held": a contact-only tower is stepped to collapse and its `state_hash`
/// is compared bit for bit across `rigid_hertz` values three orders of magnitude apart.
///
/// If any `rigid_hertz` reached a scene with no joints in it, this is where it would show.
#[test]
#[ignore = "measurement; run explicitly with --ignored"]
fn q5_a_jointless_scene_is_bit_identical_across_rigid_hertz() {
    /// Returns `(state_hash, how far the top box moved)`. The displacement is the guard: two
    /// hashes of a scene that never moved would compare equal for the wrong reason.
    fn tower(hz: f32) -> (u64, f32) {
        let mut w = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0));
        w.joint_solver.rigid_hertz = hz;
        let mut ground = RigidBody::new_static();
        ground.wake_up();
        w.add_body(
            BodyHandle::from_id(0),
            ground,
            Transform::new(Vec3::new(0.0, -0.5, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)),
        );
        // Deliberately misaligned so the tower actually topples and the contact solver is
        // exercised hard, rather than a resting stack that sleeps after a second.
        for i in 0..24u32 {
            let mut rb = RigidBody::new(1.0, true);
            let col = Collider::box_collider(Vec3::splat(0.25));
            rb.update_inertia_from_collider(&col);
            rb.wake_up();
            let j = i as f32;
            w.add_body(
                BodyHandle::from_id(i + 1),
                rb,
                Transform::new(Vec3::new(0.02 * j, 0.25 + 0.5 * j, 0.01 * j)),
                Velocity::default(),
                col,
            );
        }
        let start = w.transforms[24].position;
        for _ in 0..600 {
            w.step(FRAME_DT).ok();
        }
        (w.state_hash(), (w.transforms[24].position - start).length())
    }

    let (base, moved) = tower(LEGACY_HZ);
    println!("\nQ5  24-box tower, no joints, 600 frames — state_hash across rigid_hertz");
    println!("    the top box travelled {moved:.3} m, so the scene is not a vacuous no-op");
    println!("    rigid_hertz          state_hash");
    println!("    {:>11}   {base:#018X}", "0 (legacy)");
    assert!(moved > 1.0, "the tower never toppled ({moved:.3} m) — this hash proves nothing");
    for hz in [30.0f32, SHIPPED_HZ, 1000.0] {
        let (h, _) = tower(hz);
        println!("    {hz:>11}   {h:#018X}");
        assert_eq!(
            h, base,
            "rigid_hertz = {hz} changed a scene with NO JOINTS in it — the branch is reaching \
             code it must not reach, and `headless_stress_test`'s hash would move too"
        );
    }
}

// ── Q4: what does it cost? ───────────────────────────────────────────────────

/// Wall-clock per frame on a joint-dominated scene, legacy vs soft vs soft + warm start.
///
/// The absolute numbers belong to this machine and this build profile; the RATIO is the answer,
/// and only rows from the same run may be compared. Each configuration is run three times and
/// the FASTEST is reported — under a noisy scheduler the minimum is the only robust statistic,
/// since noise can only add time.
///
/// A 64-link chain with no contacts, so the joint solver is essentially the whole frame: a mixed
/// scene would dilute the difference and understate it.
#[test]
#[ignore = "measurement; run explicitly with --ignored"]
fn q4_per_frame_cost() {
    const N: usize = 64;
    const WARMUP: usize = 40;
    const TIMED: usize = 400;

    fn long_chain(hz: f32, warm: f32) -> PhysicsWorld {
        let mut w = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0));
        w.joint_solver.rigid_hertz = hz;
        w.joint_solver.warm_start_factor = warm;
        let mut anchor = RigidBody::new_static();
        anchor.wake_up();
        w.add_body(
            BodyHandle::from_id(0),
            anchor,
            Transform::new(Vec3::new(0.0, N as f32, 0.0)),
            Velocity::default(),
            Collider::sphere(0.05),
        );
        for i in 1..=N {
            let mut rb = RigidBody::new(if i == N { 50.0 } else { 1.0 }, true);
            let col = Collider::box_collider(Vec3::splat(0.1));
            rb.update_inertia_from_collider(&col);
            rb.wake_up();
            w.add_body(
                BodyHandle::from_id(i as u32),
                rb,
                Transform::new(Vec3::new(0.0, (N - i) as f32, 0.0)),
                Velocity::default(),
                col,
            );
            w.joints.push(Joint::rope(
                BodyHandle::from_id(i as u32 - 1),
                BodyHandle::from_id(i as u32),
                Vec3::ZERO,
                Vec3::ZERO,
                1.0,
            ));
        }
        w
    }

    println!("\nQ4  {N}-link chain, 10 iterations, {TIMED} frames @ 1/60 (4 substeps each)");
    println!("    best of 3; compare only rows from the SAME run — the build profile matters.");
    println!("    'solver' is the engine's own PhysicsMetrics::solver_ms; the scene has no");
    println!("    contacts, so on this scene that stage IS the joint solver.");
    println!("    config                        µs/frame  solver µs  vs legacy");
    let mut base = 0.0f64;
    for (name, hz, warm) in [
        ("legacy Baumgarte (hz = 0)", LEGACY_HZ, 0.0f32),
        ("soft rigid rows (hz = 200)", SHIPPED_HZ, 0.0),
        ("soft + warm start f = 1.0", SHIPPED_HZ, 1.0),
    ] {
        let mut best = f64::MAX;
        let mut solver_us = 0.0f64;
        for _ in 0..3 {
            let mut w = long_chain(hz, warm);
            for _ in 0..WARMUP {
                for rb in w.rigid_bodies.iter_mut() {
                    rb.wake_up();
                }
                w.step(FRAME_DT).ok();
            }
            let t = Instant::now();
            let mut acc = 0.0f64;
            for _ in 0..TIMED {
                for rb in w.rigid_bodies.iter_mut() {
                    rb.wake_up();
                }
                w.step(FRAME_DT).ok();
                acc += w.metrics.solver_ms as f64 * 1000.0;
            }
            let per_frame = t.elapsed().as_secs_f64() / TIMED as f64 * 1e6;
            if per_frame < best {
                best = per_frame;
                solver_us = acc / TIMED as f64;
            }
        }
        if base == 0.0 {
            base = best;
        }
        println!("    {name:<29} {best:>8.1}  {solver_us:>9.1}   {:>8.3}x", best / base);
    }

    // The same three configurations with everything but the joint solver removed. `step` also
    // pays for broadphase, narrowphase, islands, integration and sleep bookkeeping over 65
    // bodies; this loop pays for nothing but the code the change actually touched, which is
    // where a per-row cost difference would have to show up if there is one.
    // The same three configurations with everything but the joint solver removed, from ONE
    // shared state. `step` also pays for broadphase, narrowphase, islands, integration and
    // sleep bookkeeping over 65 bodies; this loop pays for nothing but the code the change
    // touched. Warming each configuration up separately would have compared different chain
    // geometries — a rope row is skipped when it is slack, so the row COUNT would differ and
    // the measurement would be of the scene, not of the branch.
    println!("\n    isolated `JointSolver::solve_joints` — one substep, {N} joints, shared state:");
    println!("    config                          ns/call   ns/joint   vs legacy");
    let mut settled = long_chain(LEGACY_HZ, 0.0);
    for _ in 0..WARMUP {
        for rb in settled.rigid_bodies.iter_mut() {
            rb.wake_up();
        }
        settled.step(FRAME_DT).ok();
    }
    let mut iso_base = 0.0f64;
    for (name, hz, warm) in [
        ("legacy Baumgarte (hz = 0)", LEGACY_HZ, 0.0f32),
        ("soft rigid rows (hz = 200)", SHIPPED_HZ, 0.0),
        ("soft + warm start f = 1.0", SHIPPED_HZ, 1.0),
    ] {
        let mut js = settled.joint_solver;
        js.rigid_hertz = hz;
        js.warm_start_factor = warm;
        let mut best = f64::MAX;
        const CALLS: usize = 4000;
        for _ in 0..3 {
            let mut joints = settled.joints.clone();
            let mut vels = settled.velocities.clone();
            let t = Instant::now();
            for _ in 0..CALLS {
                js.solve_joints(
                    &mut joints,
                    &settled.entity_index_map,
                    &settled.rigid_bodies,
                    &settled.transforms,
                    &mut vels,
                    1.0 / 240.0,
                );
            }
            best = best.min(t.elapsed().as_secs_f64() / CALLS as f64 * 1e9);
            std::hint::black_box(&vels);
        }
        if iso_base == 0.0 {
            iso_base = best;
        }
        println!(
            "    {name:<29} {best:>9.0}  {:>9.1}   {:>8.3}x",
            best / N as f64,
            best / iso_base
        );
    }
}
