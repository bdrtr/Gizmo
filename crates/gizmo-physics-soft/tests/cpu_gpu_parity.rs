//! The GPU compute path must agree with the CPU reference integrator.
//!
//! `SoftBodyMesh::step` is the reference: it is what `system::soft_body_step_system` drives,
//! and it is what every other test in this crate covers. `gpu_compute::GpuCompute` is an
//! alternative for the same simulation, so where the two disagree the GPU side is wrong by
//! definition. Two such disagreements are pinned here.
//!
//! **1 — damping.** `SoftBodyMesh::damping` is a per-SECOND RETENTION FACTOR (default 0.99).
//! The CPU applies `v *= damping.powf(dt)`; the `integrate` pass of `soft_body.wgsl` used to
//! apply `v *= max(1 - damping*dt, 0)`, reading the same field as a RATE in 1/s. Comparing the
//! effective continuous decay rates `-ln(factor)/dt` at damping = 0.99:
//!
//! | dt    | CPU rate      | old GPU rate | ratio  |
//! |-------|---------------|--------------|--------|
//! | 1/240 | 0.0100503 / s | 0.99205 / s  |  98.7x |
//! | 1/60  | 0.0100503 / s | 0.99826 / s  |  99.3x |
//! | 1/30  | 0.0100503 / s | 1.00670 / s  | 100.2x |
//!
//! (1/30 is `soft_body_step_system`'s clamp; the ratio tends to `-0.99/ln(0.99)` = 98.5 as
//! `dt` → 0 and grows slowly with `dt`. The audit's "~100x" is therefore accurate.) At
//! dt = 1/60 the per-step retention was 0.98350 against the CPU's 0.99983, so after one
//! simulated second a body kept 36.9% of its velocity on the GPU against 99.0% on the CPU.
//!
//! **2 — double advance on collision.** The `integrate` pass advances the node by `v·dt` and
//! writes the result back. The Rust-side readback then ran the swept collider test, which
//! advances its input along the velocity *again* on a hit — and it was handed the position the
//! shader had already advanced. So a colliding node moved up to `2·|v|·dt` in one step and
//! could bounce off a surface it never reached. The shared `resolve_swept_step` now takes the
//! PRE-step position on both paths.
//!
//! **What these tests do and do not prove.** No GPU is involved and none is available: the
//! `gpu_physics` feature is off by default and `GpuCompute::step_soft_bodies` needs a real
//! adapter, so nothing here executes the shader or that function. Instead:
//!
//! * The CPU expression is pinned by running it (`cpu_damping_is_a_per_second_retention_factor`).
//! * The shader expression is pinned by parsing `soft_body.wgsl` with naga — the same front end
//!   wgpu uses — and inspecting the `integrate` entry point's IR. That is a real, executable
//!   check of the source of truth the GPU compiles, and it fails on the pre-fix shader.
//! * The double-advance fix is pinned through `resolve_swept_step`, the helper the GPU readback
//!   now calls, with the old call shape asserted alongside it so the test cannot go vacuous.
//!   The one link NOT covered is the readback call site itself (`gpu_compute.rs`) — that it
//!   passes `node.position` rather than `integrated_pos` is verifiable only by reading it.
//!
//! Layout parity (Rust `#[repr(C)]` vs the WGSL structs) is a separate concern and lives in
//! `tests/wgsl_layout.rs`; this file is about semantics.

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_soft::{resolve_node_collision, resolve_swept_step, SoftBodyMesh};
use naga::{BinaryOperator, Expression, Handle, MathFunction, TypeInner};

const SHADER: &str = include_str!("../src/soft_body.wgsl");

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

    // The rate reading the shader used to apply, evaluated over the same second. Asserted, not
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
// 2. Damping — the shader expression, checked through naga's IR.
// =====================================================================================

fn parse_shader() -> naga::Module {
    naga::front::wgsl::parse_str(SHADER)
        .unwrap_or_else(|e| panic!("soft_body.wgsl does not parse: {e:?}"))
}

fn entry_point<'m>(module: &'m naga::Module, name: &str) -> &'m naga::Function {
    &module
        .entry_points
        .iter()
        .find(|ep| ep.name == name)
        .unwrap_or_else(|| panic!("soft_body.wgsl declares no `{name}` entry point"))
        .function
}

/// If `expr` is a read of a member of the `params` uniform, returns that member's name.
///
/// naga lowers `params.damping` to an `AccessIndex` on the `params` global — a pointer, since
/// it is a `var<uniform>` — wrapped in a `Load`. Both nesting orders are peeled here so the
/// check does not depend on which one a given naga version emits. Anything that is not a flat
/// single-member read of that one global returns `None`.
fn params_member<'m>(
    module: &'m naga::Module,
    func: &naga::Function,
    mut expr: Handle<Expression>,
) -> Option<&'m str> {
    let mut member: Option<u32> = None;
    loop {
        match &func.expressions[expr] {
            Expression::Load { pointer } => expr = *pointer,
            Expression::AccessIndex { base, index } => {
                if member.is_some() {
                    return None; // nested access — not the flat `params.<field>` shape
                }
                member = Some(*index);
                expr = *base;
            }
            Expression::GlobalVariable(handle) => {
                let global = &module.global_variables[*handle];
                if global.name.as_deref() != Some("params") {
                    return None;
                }
                let TypeInner::Struct { members, .. } = &module.types[global.ty].inner else {
                    return None;
                };
                return members.get(member? as usize)?.name.as_deref();
            }
            _ => return None,
        }
    }
}

/// The `integrate` pass must damp with `pow(params.damping, params.dt)` — byte for byte the
/// semantics of the CPU reference's `velocity *= damping.powf(dt)`.
///
/// This fails on the pre-fix shader: `max(1.0 - params.damping * params.dt, 0.0)` contains no
/// `pow` at all, so the search below finds nothing.
#[test]
fn integrate_pass_damps_with_pow_like_the_cpu_reference() {
    let module = parse_shader();
    let integrate = entry_point(&module, "integrate");

    // Positive control, first. A `params_member` that silently returned `None` everywhere —
    // because a naga version emits a shape it does not peel — would turn the real assertion
    // below into a vacuous pass. Prove it resolves the reads this shader actually makes.
    let resolved: Vec<&str> = integrate
        .expressions
        .iter()
        .filter_map(|(h, _)| params_member(&module, integrate, h))
        .collect();
    assert!(
        resolved.contains(&"dt") && resolved.contains(&"damping"),
        "positive control failed: `params_member` must resolve the `params` reads in the \
         `integrate` pass, but it found {resolved:?}. If this line fails, the RESOLVER is \
         broken, not the shader — fix it before trusting anything else in this file."
    );

    let damps_with_pow = integrate.expressions.iter().any(|(_, e)| match e {
        Expression::Math {
            fun: MathFunction::Pow,
            arg,
            arg1: Some(exponent),
            ..
        } => {
            params_member(&module, integrate, *arg) == Some("damping")
                && params_member(&module, integrate, *exponent) == Some("dt")
        }
        _ => false,
    });
    assert!(
        damps_with_pow,
        "the `integrate` pass of soft_body.wgsl must apply `pow(params.damping, params.dt)`. \
         `SoftBodyMesh::damping` is a per-second RETENTION FACTOR and the CPU reference applies \
         `v *= damping.powf(dt)`; a rate reading such as `max(1 - damping*dt, 0)` damps ~99x \
         harder at the 0.99 default (0.99826/s against 0.010050/s at dt = 1/60)."
    );
}

/// …and it must not ALSO keep the rate reading around. `damping` may be multiplied by nothing:
/// the only shape that made the two paths diverge was `damping * dt`.
#[test]
fn integrate_pass_never_multiplies_damping_by_the_timestep() {
    let module = parse_shader();
    let integrate = entry_point(&module, "integrate");

    for (_, expr) in integrate.expressions.iter() {
        if let Expression::Binary {
            op: BinaryOperator::Multiply,
            left,
            right,
        } = expr
        {
            for operand in [*left, *right] {
                assert_ne!(
                    params_member(&module, integrate, operand),
                    Some("damping"),
                    "`params.damping` is a per-second retention factor, so multiplying it by \
                     anything (this is the `1 - damping*dt` rate reading) is a unit error. \
                     Apply it as `pow(params.damping, params.dt)`."
                );
            }
        }
    }
}

// =====================================================================================
// 3. Double advance on collision.
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
/// The GPU readback broke both halves by sweeping from the position the `integrate` pass had
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
