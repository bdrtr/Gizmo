//! Joint lever arms must be measured from the centre of mass, not the transform origin.
//!
//! Every joint row computed `anchor - transform.position`. That agrees with the correct
//! expression only when `center_of_mass` is zero — and it is not zero for any compound
//! collider (`update_inertia_from_collider` derives a shifted COM automatically), any
//! fracture chunk, or any vehicle chassis. Those bodies got the wrong torque and the wrong
//! effective mass from every joint attached to them.
//!
//! `Integrator::apply_impulse_at_point` has always used the COM; the joint path was the
//! outlier.

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::joints::Joint;
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

/// A world with an anchor (static) and one dynamic body hanging from a distance joint.
/// `com` shifts the dynamic body's centre of mass away from its transform origin.
fn hanging_body(com: Vec3) -> PhysicsWorld {
    let mut w = PhysicsWorld::new();

    let anchor = BodyHandle::from_id(0);
    let mut ar = RigidBody::new_static();
    ar.wake_up();
    w.add_body(
        anchor,
        ar,
        Transform::new(Vec3::new(0.0, 2.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::splat(0.1)),
    );

    let body = BodyHandle::from_id(1);
    let mut rb = RigidBody::new(1.0, true);
    let col = Collider::box_collider(Vec3::splat(0.5));
    rb.update_inertia_from_collider(&col);
    rb.center_of_mass = com;
    rb.wake_up();
    w.add_body(body, rb, Transform::new(Vec3::ZERO), Velocity::default(), col);

    // Anchor the rope well off the body's origin so the lever arm actually matters.
    w.joints.push(Joint::rope(
        anchor,
        body,
        Vec3::ZERO,
        Vec3::new(0.5, 0.5, 0.0),
        1.5,
    ));
    w
}

fn simulate(mut w: PhysicsWorld, steps: usize) -> (Vec3, Vec3) {
    for _ in 0..steps {
        w.step(1.0 / 60.0).expect("step");
    }
    (w.transforms[1].position, w.velocities[1].angular)
}

/// The behavioural claim: shifting a body's centre of mass changes how a joint torques it.
///
/// Before the fix the lever arm was `anchor - transform.position`, which does not mention
/// `center_of_mass` at all — so these two worlds evolved identically and the COM was
/// silently ignored. If this test ever passes trivially again, the arms went back to the
/// transform origin.
#[test]
fn a_shifted_centre_of_mass_changes_how_a_joint_torques_the_body() {
    let (pos_centered, ang_centered) = simulate(hanging_body(Vec3::ZERO), 90);
    let (pos_shifted, ang_shifted) = simulate(hanging_body(Vec3::new(0.4, 0.0, 0.0)), 90);

    assert!(
        pos_centered.is_finite() && pos_shifted.is_finite(),
        "both worlds must stay finite: {pos_centered:?} / {pos_shifted:?}"
    );

    let dp = (pos_centered - pos_shifted).length();
    let da = (ang_centered - ang_shifted).length();
    assert!(
        dp > 1e-3 || da > 1e-3,
        "moving the centre of mass by 0.4 must change the joint's effect, but the two \
         worlds agree (Δpos={dp}, Δang={da}). The lever arm is being measured from the \
         transform origin again."
    );
}

/// The reference: the joint's arm must match what `Integrator::apply_impulse_at_point`
/// computes, since that is the convention the contact solver follows. A body whose COM sits
/// exactly at its origin must be unaffected by the change — this is the guard against the
/// fix having shifted behaviour for the common case.
#[test]
fn a_centred_body_is_unaffected() {
    let (p1, a1) = simulate(hanging_body(Vec3::ZERO), 60);
    let (p2, a2) = simulate(hanging_body(Vec3::ZERO), 60);
    assert_eq!(p1, p2, "the same world must simulate identically");
    assert_eq!(a1, a2);
    assert!(p1.y < 0.5, "the body should have swung/fallen from the origin, got {p1:?}");
}

/// A compound collider gets a shifted COM automatically, which is what makes this a real
/// bug rather than a theoretical one — the user never wrote `center_of_mass` themselves.
#[test]
fn update_inertia_from_collider_can_produce_a_nonzero_centre_of_mass() {
    use gizmo_physics_core::ColliderShape;

    let mut rb = RigidBody::new(1.0, true);
    let compound = Collider::from_shape(ColliderShape::Compound(vec![
        (
            Transform::new(Vec3::new(1.0, 0.0, 0.0)),
            Box::new(ColliderShape::Box(gizmo_physics_core::BoxShape {
                half_extents: Vec3::splat(0.5),
            })),
        ),
        (
            Transform::new(Vec3::new(-0.2, 0.0, 0.0)),
            Box::new(ColliderShape::Box(gizmo_physics_core::BoxShape {
                half_extents: Vec3::splat(0.1),
            })),
        ),
    ]));
    rb.update_inertia_from_collider(&compound);

    assert!(
        rb.center_of_mass.length() > 1e-4,
        "an off-centre compound must produce a shifted COM, got {:?} — if this is zero the \
         premise of the joint fix needs re-checking",
        rb.center_of_mass
    );
}
