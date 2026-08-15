//! Does the ECS rollback path actually restore the simulation, or only its poses?
//!
//! This crate has two rollback implementations. `RollbackSession` treats `PhysicsWorld` as
//! authoritative and snapshots it with `WorldSnapshot` — the one whose fields each carry a comment
//! naming the divergence that earned them a place. `RollbackManager`, the one the windowed app
//! wires up, treats the ECS as authoritative and snapshots six numbers per entity: position,
//! rotation, the two velocities and the sleep flag.
//!
//! Everything else `WorldSnapshot` carries is state the physics crate documents as **not
//! derivable** from transforms and velocities: the substep accumulator ("it decides how many
//! substeps the next frame runs, so restoring poses without it makes the resimulation diverge"),
//! the contact cache's warm-start impulses, the joints' one-way `is_broken` latch and their
//! latched reference poses, and the force fields and fluid zones gameplay can change mid-window.
//!
//! The measurement is the standard one for this engine: run a scenario straight through, run the
//! same scenario with a rollback in the middle, and compare `state_hash`. A rollback that restores
//! the simulation gives the same hash. A rollback that restores the picture does not.

#![cfg(all(feature = "rollback", feature = "client-server"))]

use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_net::rollback::RollbackManager;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

const DT: f32 = 1.0 / 60.0;

/// A floor and a few boxes dropped onto it: contacts, sleeping, and a substep accumulator that
/// does not divide evenly into the frame.
fn scene() -> (World, PhysicsWorld) {
    let mut world = World::new();
    let mut pw = PhysicsWorld::new();

    let mut ground = RigidBody::new_static();
    ground.wake_up();
    let ground_e = world.spawn();
    let ground_t = Transform::new(Vec3::new(0.0, -1.0, 0.0));
    let ground_c = Collider::box_collider(Vec3::new(20.0, 1.0, 20.0));
    world.add_component(ground_e, ground_t);
    world.add_component(ground_e, Velocity::default());
    world.add_component(ground_e, ground);
    world.add_component(ground_e, ground_c.clone());
    pw.add_body(
        BodyHandle::from_id(ground_e.id()),
        ground,
        ground_t,
        Velocity::default(),
        ground_c,
    );

    for i in 0..4 {
        let mut rb = RigidBody::new(1.0, true);
        rb.wake_up();
        let col = Collider::box_collider(Vec3::splat(0.5));
        rb.update_inertia_from_collider(&col);
        let t = Transform::new(Vec3::new(i as f32 * 0.9 - 1.5, 2.0 + i as f32 * 1.3, 0.0));
        let e = world.spawn();
        world.add_component(e, t);
        world.add_component(e, Velocity::default());
        world.add_component(e, rb);
        world.add_component(e, col.clone());
        pw.add_body(BodyHandle::from_id(e.id()), rb, t, Velocity::default(), col);
    }
    (world, pw)
}

/// One frame the way the app runs it: the ECS is authoritative, the step syncs it into the
/// physics world, steps, and copies the results back.
fn step(world: &mut World, pw: &mut PhysicsWorld) {
    let bodies: Vec<(BodyHandle, RigidBody, Transform, Velocity, Collider)> = {
        let transforms = world.borrow::<Transform>();
        let velocities = world.borrow::<Velocity>();
        let rigid_bodies = world.borrow::<RigidBody>();
        let colliders = world.borrow::<Collider>();
        world
            .iter_alive_entities()
            .into_iter()
            .filter_map(|e| {
                Some((
                    BodyHandle::from_id(e.id()),
                    *rigid_bodies.get(e.id())?,
                    *transforms.get(e.id())?,
                    *velocities.get(e.id())?,
                    colliders.get(e.id())?.clone(),
                ))
            })
            .collect()
    };
    pw.sync_bodies(bodies.iter());
    pw.step(DT).ok();

    // Results back to the ECS.
    let mut transforms = unsafe { world.borrow_mut_unchecked::<Transform>() };
    let mut velocities = unsafe { world.borrow_mut_unchecked::<Velocity>() };
    let mut rigid_bodies = unsafe { world.borrow_mut_unchecked::<RigidBody>() };
    for (i, handle) in pw.entities.iter().enumerate() {
        let id = handle.id();
        if let Some(mut t) = transforms.get_mut(id) {
            *t = pw.transforms[i];
        }
        if let Some(mut v) = velocities.get_mut(id) {
            *v = pw.velocities[i];
        }
        if let Some(mut rb) = rigid_bodies.get_mut(id) {
            *rb = pw.rigid_bodies[i];
        }
    }
}

/// `step`, with the physics world living in the ECS as a resource — which is where the manager
/// looks for it.
fn step_in_world(world: &mut World) {
    let mut pw = world.remove_resource::<PhysicsWorld>().expect("physics world");
    step(world, &mut pw);
    world.insert_resource(pw);
}

/// The straight-through run: what the rollback is supposed to reproduce.
fn ground_truth(ticks: u32) -> u64 {
    let (mut world, mut pw) = scene();
    for _ in 0..ticks {
        step(&mut world, &mut pw);
    }
    pw.state_hash()
}

/// A rollback followed by resimulation lands on the same world the straight run reached.
///
/// This is the property the whole mechanism exists for, and it did not hold: the ECS snapshot puts
/// the poses back and the physics world keeps the accumulator, warm-start impulses, joints and
/// force fields it had at the newer tick. Measured before the fix at C9C742C69915AEB4 against a
/// ground truth of 992A5D823D90AA67 — two different worlds, same starting poses.
#[test]
fn a_rollback_reproduces_the_run_it_rolled_back_into() {
    const TOTAL: u32 = 40;
    const ROLLBACK_AT: u32 = 30;
    const REWIND: u32 = 6;

    let expected = ground_truth(TOTAL);

    let (mut world, pw) = scene();
    // The manager owns both halves of the snapshot, so the world has to own the physics.
    world.insert_resource(pw);
    let mut manager = RollbackManager::new(64);

    for _ in 0..ROLLBACK_AT {
        step_in_world(&mut world);
        manager.end_frame(&world);
    }

    // Rewind, exactly the way the app does.
    manager.rollback_target_tick = Some(u64::from(ROLLBACK_AT - REWIND));
    assert!(manager.begin_frame(&mut world), "the rollback should have fired");

    // `end_frame` saves tick T *after* the (T+1)-th step, so restoring tick T leaves the world
    // T+1 steps in. Getting that wrong is its own silent divergence — one extra step looks exactly
    // like an incomplete restore.
    let steps_done = ROLLBACK_AT - REWIND + 1;
    for _ in steps_done..TOTAL {
        step_in_world(&mut world);
        manager.record_resimulated_tick(&world);
    }

    let after_rollback = world.get_resource::<PhysicsWorld>().unwrap().state_hash();
    assert_eq!(
        after_rollback, expected,
        "a rollback + resimulation produced a different world than the straight run \
         ({after_rollback:016X} vs {expected:016X})"
    );
}

/// The scenario the first version of this test could not have distinguished.
///
/// `Joint::is_broken` is a **one-way latch** living in `PhysicsWorld`, not in the ECS. A joint that
/// snaps inside the rollback window therefore cannot be un-snapped by restoring components: the
/// resimulation runs without a joint the original run still had, which is the divergence
/// `WorldSnapshot`'s own comment describes. Nothing about transforms and velocities can express
/// it.
///
/// Kept separate from the plain-boxes case on purpose. That one passes with or without the physics
/// half of the restore — measured — because a scene of falling boxes with no joints, no force
/// fields and the default zero warm-start factor has no carried state to lose. A test that cannot
/// tell the two implementations apart is not evidence about either.
#[test]
fn a_joint_that_broke_inside_the_window_is_whole_again_after_the_rollback() {
    use gizmo_physics_rigid::joints::Joint;

    // Ground truth: a hanging box on a weak joint, run straight through.
    let build = || {
        let mut world = World::new();
        let mut pw = PhysicsWorld::new();

        let anchor_e = world.spawn();
        let anchor_rb = RigidBody::new_static();
        let anchor_t = Transform::new(Vec3::new(0.0, 5.0, 0.0));
        let anchor_c = Collider::box_collider(Vec3::splat(0.25));
        world.add_component(anchor_e, anchor_t);
        world.add_component(anchor_e, Velocity::default());
        world.add_component(anchor_e, anchor_rb);
        world.add_component(anchor_e, anchor_c.clone());
        pw.add_body(
            BodyHandle::from_id(anchor_e.id()),
            anchor_rb,
            anchor_t,
            Velocity::default(),
            anchor_c,
        );

        let load_e = world.spawn();
        let mut load_rb = RigidBody::new(40.0, true);
        load_rb.wake_up();
        let load_c = Collider::box_collider(Vec3::splat(0.5));
        load_rb.update_inertia_from_collider(&load_c);
        let load_t = Transform::new(Vec3::new(0.0, 3.2, 0.0));
        world.add_component(load_e, load_t);
        world.add_component(load_e, Velocity::default());
        world.add_component(load_e, load_rb);
        world.add_component(load_e, load_c.clone());
        pw.add_body(
            BodyHandle::from_id(load_e.id()),
            load_rb,
            load_t,
            Velocity::default(),
            load_c,
        );

        // Weak enough that the hanging mass snaps it a few ticks in.
        let mut joint = Joint::distance(
            BodyHandle::from_id(anchor_e.id()),
            BodyHandle::from_id(load_e.id()),
            Vec3::ZERO,
            Vec3::ZERO,
            0.0,
            2.0,
        );
        joint.break_force = 5.0;
        pw.joints.push(joint);
        (world, pw)
    };

    const TOTAL: u32 = 30;
    const ROLLBACK_AT: u32 = 20;
    const REWIND: u32 = 12;

    let expected = {
        let (mut w, mut pw) = build();
        for _ in 0..TOTAL {
            step(&mut w, &mut pw);
        }
        pw.state_hash()
    };

    let (mut world, pw) = build();
    world.insert_resource(pw);
    let mut manager = RollbackManager::new(64);
    // WHEN it breaks is the whole scenario: before the rollback target it proves nothing, after
    // the rollback point it never happens. Tuned to snap around tick 12, target 8, rewind at 20.
    let mut broke_at = None;
    for tick in 0..ROLLBACK_AT {
        step_in_world(&mut world);
        manager.end_frame(&world);
        if broke_at.is_none() && world.get_resource::<PhysicsWorld>().unwrap().joints[0].is_broken {
            broke_at = Some(tick);
        }
    }
    let broke_at = broke_at.expect("the joint never snapped — this scenario proves nothing");
    assert!(
        broke_at > (ROLLBACK_AT - REWIND) && broke_at < ROLLBACK_AT,
        "the joint snapped at tick {broke_at}, outside the rollback window \
         ({}..{ROLLBACK_AT}) — the test would pass without restoring anything",
        ROLLBACK_AT - REWIND
    );

    manager.rollback_target_tick = Some(u64::from(ROLLBACK_AT - REWIND));
    assert!(manager.begin_frame(&mut world), "the rollback should have fired");
    assert!(
        !world.get_resource::<PhysicsWorld>().unwrap().joints[0].is_broken,
        "the joint is still broken after rolling back to before it broke"
    );

    let steps_done = ROLLBACK_AT - REWIND + 1;
    for _ in steps_done..TOTAL {
        step_in_world(&mut world);
        manager.record_resimulated_tick(&world);
    }

    let after = world.get_resource::<PhysicsWorld>().unwrap().state_hash();
    assert_eq!(after, expected, "the resimulation diverged from the straight run");
}
