//! `compliance` is an inverse stiffness, and these tests make it mean that.
//!
//! It is a public, persisted, per-joint field documented as "0 = hard stop; larger = a soft,
//! springy limit that gives under load". It did not behave like one. The old implementation
//! added `compliance / dt²` to the row's effective mass (CFM regularisation) and stopped
//! there — but enlarging `k` only shrinks each iteration's step, so the series still
//! converges to the RIGID solution. Softness came entirely from `iterations` being finite:
//! `compliance` was a relaxation factor for the solver loop, not a physical property of the
//! joint. Doubling the iteration count changed how soft a ragdoll's shoulder was.
//!
//! Joints now use the same soft-constraint formulation as the contact solver
//! (`solver/tgs.rs`), with the row's frequency derived from its compliance and effective
//! mass. The observable consequence is Hooke's law, and that is what is asserted here.

use gizmo_math::Vec3;
use gizmo_physics_core::BodyHandle;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::{Joint, JointData, PhysicsWorld, RigidBody, Velocity};

/// A mass hanging on a rope of rest length 2, left to settle. Returns the steady-state
/// stretch beyond the rest length.
fn hanging_rope_stretch(compliance: f32, mass: f32, iterations: usize) -> f32 {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    world.joint_solver.iterations = iterations;

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

    let mut j = Joint::rope(
        BodyHandle::from_id(1),
        BodyHandle::from_id(2),
        Vec3::ZERO,
        Vec3::ZERO,
        2.0,
    );
    if let JointData::Distance(ref mut d) = j.data {
        d.compliance = compliance;
    }
    world.joints.push(j);

    // Long enough for the spring to settle at any of the compliances tested.
    for _ in 0..2000 {
        world.step(1.0 / 240.0).ok();
    }
    (world.transforms[1].position - Vec3::new(0.0, 10.0, 0.0)).length() - 2.0
}

/// **Hooke's law.** A compliant rope holding a load of `m·g` must stretch by `α·m·g`, because
/// `α` is by definition `1/K` and a spring at rest satisfies `F = K·x`.
///
/// Spanning two orders of magnitude in `α` and one in mass, because the two ways this can go
/// wrong are different: a formulation can get the stiffness right and the mass scaling wrong,
/// or vice versa. Both happened during this work —
///
/// * the shipped CFM version was wrong in `α`: at `α = 0.03`, `m = 1` it stretched 4.08 m
///   against a predicted 0.294 m, and the number moved with `iterations`;
/// * the first soft-constraint attempt was right in `α` at `m = 1` (0.2938 vs 0.2943) and
///   catastrophically wrong above it — the constraint went inert and the 4 kg case fell 331 m,
///   because the `impulse_scale` feedback was divided by `k` and the λ iteration diverges once
///   `impulse_scale > 2k`.
///
/// A single-mass, single-compliance test would have passed the second one.
#[test]
fn compliance_is_an_inverse_stiffness() {
    for (alpha, mass) in [
        (0.003_f32, 1.0_f32),
        (0.03, 1.0),
        (0.3, 1.0),
        (0.03, 4.0),
        (0.003, 10.0),
    ] {
        let predicted = alpha * mass * 9.81;
        let measured = hanging_rope_stretch(alpha, mass, 10);
        let ratio = measured / predicted;
        assert!(
            (ratio - 1.0).abs() < 0.02,
            "compliance {alpha} under {mass} kg: Hooke predicts a stretch of {predicted:.6} m \
             (α·m·g), measured {measured:.6} m — ratio {ratio:.4}, which is not a spring"
        );
    }
}

/// …and the stiffness is a property of the JOINT, not of the solver loop. `iterations` used
/// to change how soft the joint was, which meant a ragdoll tuned at one iteration count went
/// stiff or floppy at another.
#[test]
fn compliance_does_not_depend_on_the_iteration_count() {
    for alpha in [0.003_f32, 0.03] {
        let reference = hanging_rope_stretch(alpha, 1.0, 5);
        for iterations in [10, 20, 40] {
            let measured = hanging_rope_stretch(alpha, 1.0, iterations);
            assert!(
                (measured - reference).abs() < 1e-5,
                "compliance {alpha}: stretch moved from {reference:.6} at 5 iterations to \
                 {measured:.6} at {iterations} — softness must be a joint property, not a \
                 solver-loop artefact"
            );
        }
    }
}

/// A joint that was rigid must still LOOK rigid.
///
/// `compliance == 0` no longer takes a Baumgarte branch — it is a spring at
/// `JointSolver::rigid_hertz`, 200 Hz by default, whose static error is `a/ω²`
/// (`tests/joint_rigid_stiffness.rs` is what asserts that law; this is the bar on what it
/// costs). Baumgarte converged to ZERO error here, so the change is a pure stiffness loss on
/// this scene and the number below is how much of one is acceptable.
///
/// **Tightened 6.7× on 2026-08-09, from 1e-4 to 1.5e-5**, and the tightening is the point: the
/// predicted sag is 6.2e-6 m and the measured one 6.199e-6, against an f32 noise floor of
/// ~2.4e-7 on a 2 m length. At 1e-4 this test would have accepted a rigid rope sagging 16× more
/// than it does — including the naive mirror of the contact solver's settings
/// (`contact_hertz = 30, ζ = 10`), which sags 2.8e-4 m. It is red below ≈129 Hz.
#[test]
fn zero_compliance_is_still_rigid() {
    let stretch = hanging_rope_stretch(0.0, 1.0, 10);
    assert!(
        stretch.abs() < 1.5e-5,
        "a rope with no compliance must hold its length, stretched {stretch:.8} m"
    );
    // And it is stiffer than every compliant case by orders of magnitude, so the two
    // branches cannot be silently swapped.
    assert!(
        stretch < hanging_rope_stretch(0.003, 1.0, 10) * 0.01,
        "the rigid branch must be far stiffer than the softest compliant one"
    );
}
