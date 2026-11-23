//! Two disagreements the GPU soft-body path once had with the CPU reference, pinned on the CPU.
//!
//! `SoftBodyMesh::step` is the reference. `gpu_compute::GpuCompute` once had its own integrator
//! in `soft_body.wgsl`, and where the two disagreed the GPU side was wrong by definition. Since
//! 2026-11-23 there is one integrator: the GPU computes the element forces and the step ends in
//! the CPU's own `SoftBodyMesh::advance_nodes` (`tests/gpu_step.rs` runs the two side by side).
//! The two disagreements stay pinned here, on the code that is now the only copy of each.
//!
//! **1 — damping.** `SoftBodyMesh::damping` is a per-SECOND RETENTION FACTOR (default 0.99).
//! The CPU applies `v *= damping.powf(dt)`; the shader's integrator applied
//! `v *= max(1 - damping*dt, 0)`, reading the same field as a RATE in 1/s. Comparing the
//! effective continuous decay rates `-ln(factor)/dt` at damping = 0.99:
//!
//! | dt    | CPU rate      | old GPU rate | ratio  |
//! |-------|---------------|--------------|--------|
//! | 1/240 | 0.0100503 / s | 0.99205 / s  |  98.7x |
//! | 1/60  | 0.0100503 / s | 0.99826 / s  |  99.3x |
//! | 1/30  | 0.0100503 / s | 1.00670 / s  | 100.2x |
//!
//! At dt = 1/60 the per-step retention was 0.98350 against the CPU's 0.99983, so after one
//! simulated second a body kept 36.9% of its velocity on the GPU against 99.0% on the CPU.
//!
//! **2 — double advance on collision.** The shader advanced the node by `v·dt`, and the readback
//! then ran the swept collider test, which advances its input along the velocity *again* on a
//! hit — handed the position the shader had already advanced. So a colliding node moved up to
//! `2·|v|·dt` in one step and could bounce off a surface it never reached. The shared
//! `resolve_swept_step` takes the PRE-step position.
//!
//! Layout parity (Rust `#[repr(C)]` vs the WGSL structs) lives in `tests/wgsl_layout.rs`.

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_soft::{resolve_node_collision, resolve_swept_step, SoftBodyMesh};

// =====================================================================================
// 1. Damping — the CPU reference expression.
// =====================================================================================

/// `damping` is the fraction of velocity a node keeps over one SECOND, however that second is
/// subdivided — not a rate. This is the expression the shader has to match.
#[test]
fn cpu_damping_is_a_per_second_retention_factor() {
    let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid material params");
    assert!(
        (sb.damping - 0.99).abs() < 1e-9,
        "the numbers below assume the 0.99 default; got {}",
        sb.damping
    );

    // One node, no elements, no gravity: the force on it is exactly zero, so the ONLY thing
    // `step` does to the velocity is damp it. The observed factor IS the damping expression.
    sb.add_node(Vec3::ZERO, 1.0);
    sb.nodes[0].velocity = Vec3::new(1.0, 0.0, 0.0);

    let dt = 1.0 / 60.0;
    sb.step(dt, Vec3::ZERO, &[]);
    let after_one_step = sb.nodes[0].velocity.x;
    let expected = 0.99f32.powf(dt); // 0.99983251
    assert!(
        (after_one_step - expected).abs() < 1e-7,
        "one step must retain damping^dt = {expected}, got {after_one_step}"
    );

    // 59 more steps = one simulated second = exactly the damping factor. Being able to state
    // the answer in seconds without knowing `dt` is the whole point of the `powf` form.
    for _ in 1..60 {
        sb.step(dt, Vec3::ZERO, &[]);
    }
    let after_one_second = sb.nodes[0].velocity.x;
    assert!(
        (after_one_second - 0.99).abs() < 1e-4,
        "after one simulated second a 0.99-damped node must retain 0.99 of its velocity, \
         got {after_one_second}"
    );

    // The rate reading the shader's integrator applied, evaluated over the same second. Asserted, not
    // just commented, so the two readings of this field can never be mistaken for each other.
    let rate_reading = (1.0f32 - 0.99 * dt).max(0.0).powi(60);
    assert!(
        (rate_reading - 0.3685).abs() < 1e-3,
        "sanity on the comparison figure: `max(1 - 0.99*dt, 0)^60` = {rate_reading}"
    );
    assert!(
        after_one_second > 0.9,
        "the CPU path must not behave like the rate reading ({rate_reading} after one second)"
    );
}

// =====================================================================================
// 2. Double advance on collision.
// =====================================================================================

/// A thin axis-aligned wall 0.1 m thick in X (and effectively unbounded in Y/Z), centred at
/// `x` — so its near face sits at `x - 0.05`.
fn wall(x: f32) -> (BodyHandle, Transform, Collider) {
    (
        BodyHandle::from_id(1),
        Transform::new(Vec3::new(x, 0.0, 0.0)),
        Collider::box_collider(Vec3::new(0.05, 5.0, 5.0)),
    )
}

/// A node may never travel further in one step than its swept distance `|v|·dt`, and may never
/// bounce off a surface it did not reach.
///
/// The GPU readback broke both halves by sweeping from the position the shader's integrator had
/// already produced. The second half of this test performs that exact old call and asserts it
/// disagrees — if it ever stops disagreeing, the first half has stopped testing anything.
#[test]
fn swept_step_does_not_advance_a_node_twice() {
    let dt = 1.0 / 60.0;
    let pre_step = Vec3::ZERO;
    let velocity = Vec3::new(60.0, 0.0, 0.0); // swept distance = 1 m
    let swept = velocity.length() * dt;

    // Wall spanning x ∈ [1.50, 1.60]: 1.5 m away, half a metre beyond this step's reach.
    let colliders = vec![wall(1.55)];

    // What an integrator produces — the CPU's `next_pos`, or the shader's written-back node.
    let integrated = pre_step + velocity * dt; // x ≈ 1.0

    let out = resolve_swept_step(pre_step, integrated, velocity, 1.0, dt, &colliders, &[]);
    let (position, out_velocity) = (out.position, out.velocity);

    assert!(
        (position.x - integrated.x).abs() < 1e-5,
        "the wall is out of reach this step, so the node must land exactly where the \
         integrator put it (x = {}), got x = {}",
        integrated.x,
        position.x
    );
    assert!(
        (position - pre_step).length() <= swept + 1e-4,
        "a node moved {} m in a step whose swept distance is {swept} m",
        (position - pre_step).length()
    );
    assert!(
        out_velocity.x > 0.0,
        "nothing was reached, so nothing may reflect the velocity; got vx = {}",
        out_velocity.x
    );

    // NON-VACUITY / the defect itself. This is the call the GPU readback used to make: sweep
    // from the ALREADY-integrated position. From x ≈ 1.0 the wall is 0.5 m ahead, inside the
    // sweep window, so the node is advanced a further 0.4 m to x ≈ 1.4 — 1.4 m of travel in a
    // 1.0 m step — and its velocity is reflected off a wall it never got to.
    let bad = resolve_node_collision(integrated, velocity, 1.0, dt, &colliders, &[]);
    let (bad_position, bad_velocity, bad_hit) = (bad.position, bad.velocity, bad.collided());
    assert!(
        bad_hit,
        "control: sweeping from the integrated position must find the wall, or this \
         comparison proves nothing"
    );
    assert!(
        (bad_position - pre_step).length() > swept * 1.3,
        "control: the old call shape must overshoot the swept distance; it moved {} m",
        (bad_position - pre_step).length()
    );
    assert!(
        bad_velocity.x < 0.0,
        "control: the old call shape must bounce off the unreached wall; got vx = {}",
        bad_velocity.x
    );
}

/// The fix must not have turned the sweep off: a surface genuinely inside the step's reach is
/// still resolved, with the node parked 0.1 m short of it and its velocity reflected.
#[test]
fn swept_step_still_resolves_a_real_collision() {
    let dt = 1.0 / 60.0;
    let pre_step = Vec3::ZERO;
    let velocity = Vec3::new(60.0, 0.0, 0.0); // swept distance = 1 m

    // Wall spanning x ∈ [0.50, 0.60] — well inside this step's reach.
    let colliders = vec![wall(0.55)];
    let integrated = pre_step + velocity * dt;

    let out = resolve_swept_step(pre_step, integrated, velocity, 1.0, dt, &colliders, &[]);
    let (position, out_velocity) = (out.position, out.velocity);

    assert!(
        (position.x - 0.4).abs() < 1e-4,
        "the node must park 0.1 m short of the x = 0.50 face, got x = {}",
        position.x
    );
    assert!(
        out_velocity.x < 0.0,
        "an approached surface must reflect the inward velocity, got vx = {}",
        out_velocity.x
    );
}

/// The CPU reference obeys the same bound, through the same helper — this is what "the GPU path
/// must match the CPU path" means concretely, and it guards the extraction of
/// `resolve_swept_step` out of `SoftBodyMesh::step`.
#[test]
fn cpu_step_does_not_advance_a_node_twice_either() {
    let dt = 1.0 / 60.0;
    let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid material params");
    sb.add_node(Vec3::ZERO, 1.0);
    sb.nodes[0].velocity = Vec3::new(60.0, 0.0, 0.0);

    // Same out-of-reach wall as above.
    let colliders = vec![wall(1.55)];
    sb.step(dt, Vec3::ZERO, &colliders);

    // No elements and no gravity, so the only thing that happened to the velocity is damping;
    // the position advances by that damped velocity, once.
    let expected_x = 60.0 * 0.99f32.powf(dt) * dt;
    assert!(
        (sb.nodes[0].position.x - expected_x).abs() < 1e-5,
        "one step must advance the node exactly once (expected x = {expected_x}), got x = {}",
        sb.nodes[0].position.x
    );
    assert!(
        sb.nodes[0].velocity.x > 0.0,
        "the wall is out of reach, so there must be no bounce; got vx = {}",
        sb.nodes[0].velocity.x
    );
}
