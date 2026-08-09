//! Waking travels along a jointed mechanism in ONE step, not one link per substep.
//!
//! A sleeping body does not integrate, so a joint pulling on one silently has its correction
//! swallowed and the mechanism looks broken. `pipeline.rs` already wakes the sleeping end of
//! a joint whose other end is a mover — but as a SINGLE PASS over `world.joints` in array
//! order, so it propagates one link per pass in the best case and, in the wrong order, one
//! link per SUBSTEP.
//!
//! Islands solve exactly this problem for contacts (`island.rs` union-finds manifolds so a
//! whole pile wakes together). Joints were never part of island construction, so a jointed
//! chain got the single-pass approximation instead.

use gizmo_math::Vec3;
use gizmo_physics_core::BodyHandle;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::{Joint, PhysicsWorld, RigidBody, Velocity};

const LINKS: u32 = 12;

/// A hanging chain of `LINKS` bodies, every one of them asleep, with the joints pushed in
/// natural order (anchor-first). Waking the DEEPEST body then has to travel the whole chain
/// against the array order.
fn sleeping_chain() -> PhysicsWorld {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));

    let mut anchor = RigidBody::new_static();
    anchor.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        anchor,
        Transform::new(Vec3::new(0.0, 20.0, 0.0)),
        Velocity::default(),
        Collider::sphere(0.1),
    );

    for i in 1..=LINKS {
        let mut rb = RigidBody::new(1.0, true);
        // Asleep and settled: this is a mechanism that has been hanging still.
        rb.is_sleeping = true;
        rb.sleep_counter = 1000;
        let col = Collider::box_collider(Vec3::splat(0.1));
        rb.update_inertia_from_collider(&col);
        world.add_body(
            BodyHandle::from_id(i),
            rb,
            Transform::new(Vec3::new(0.0, 20.0 - i as f32, 0.0)),
            Velocity::default(),
            col,
        );
    }

    // Anchor-first order, so a single pass walks the chain the WRONG way relative to a
    // disturbance applied at the far end.
    for i in 0..LINKS {
        world.joints.push(Joint::rope(
            BodyHandle::from_id(i),
            BodyHandle::from_id(i + 1),
            Vec3::ZERO,
            Vec3::ZERO,
            1.0,
        ));
    }
    world
}

fn awake_count(w: &PhysicsWorld) -> usize {
    (1..=LINKS as usize)
        .filter(|&i| !w.rigid_bodies[i].is_sleeping)
        .count()
}

/// Yank the far end of a sleeping chain: after ONE step the whole chain must be awake.
///
/// Physically this is not a subtlety. The chain is inextensible; disturbing the bottom link
/// loads every link above it in the same instant. A solver that wakes them one at a time is
/// simulating a mechanism that does not exist — the upper links keep absorbing joint
/// corrections they never integrate, so the chain behaves as if it were pinned partway down.
#[test]
fn waking_travels_the_whole_chain_in_one_step() {
    let mut w = sleeping_chain();
    assert_eq!(awake_count(&w), 0, "precondition: the chain starts fully asleep");

    // Disturb the DEEPEST link — the far end from the anchor.
    let last = LINKS as usize;
    w.rigid_bodies[last].wake_up();
    w.velocities[last].linear = Vec3::new(6.0, 0.0, 0.0);

    w.step(1.0 / 60.0).ok();

    assert_eq!(
        awake_count(&w),
        LINKS as usize,
        "only {} of {LINKS} links woke in one step — waking must reach a fixed point within \
         the substep, not crawl one joint per pass in array order",
        awake_count(&w)
    );
}

/// The joint solver now skips a joint whose two ends are both inert, so it never writes into a
/// sleeping mechanism (`JointSolver::solve_joints`). That gate is only safe because the
/// joint-graph wake pass runs immediately BEFORE the solve, in the same substep — nothing in
/// the type system enforces that ordering, and a refactor that moved the wake pass after the
/// solver would cost a woken chain one whole substep of being unconstrained while its bodies
/// integrate. That is what this test catches: the chain must be both awake AND solved on the
/// step it is disturbed.
///
/// Measured against the property rather than a constant: an unconstrained link falls
/// `g·(1/60)² ≈ 2.7 mm` in the step, and the joints hold it to a small fraction of that.
#[test]
fn waking_a_component_is_solved_on_the_same_step_it_wakes() {
    let mut w = sleeping_chain();
    let last = LINKS as usize;
    w.rigid_bodies[last].wake_up();
    w.velocities[last].linear = Vec3::new(6.0, 0.0, 0.0);

    let before: Vec<Vec3> = (1..=last).map(|i| w.transforms[i].position).collect();
    w.step(1.0 / 60.0).ok();

    // Every link is 1 m below the one above it and the ropes are exactly 1 m: any link that
    // integrated a substep without its joints being solved separates measurably.
    for i in 1..=last {
        let above = if i == 1 {
            w.transforms[0].position
        } else {
            w.transforms[i - 1].position
        };
        let span = (w.transforms[i].position - above).length();
        assert!(
            span <= 1.002,
            "link {i} is {span:.5} m from the one above it after one step — the chain woke but \
             was not solved, so it integrated unconstrained. The joint-graph wake pass must \
             stay BEFORE `solve_joints`, not after it."
        );
    }
    assert!(
        before[last - 1] != w.transforms[last].position,
        "precondition: the disturbed link must actually have moved"
    );
}

/// The converse, so the fix cannot be "wake everything". A chain nobody touches stays asleep:
/// waking must propagate from a real mover, not from the mere presence of a joint.
#[test]
fn an_undisturbed_chain_stays_asleep() {
    let mut w = sleeping_chain();
    for _ in 0..30 {
        w.step(1.0 / 60.0).ok();
    }
    assert_eq!(
        awake_count(&w),
        0,
        "a chain at rest must stay asleep — {} links woke with nothing disturbing them",
        awake_count(&w)
    );
}
