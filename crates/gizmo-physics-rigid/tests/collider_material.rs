//! **Everything an author sets on a `Collider` has to reach the solver**, not just its shape.
//!
//! The ECS→PhysicsWorld gather in `physics_step_system` rebuilds each body's collider every
//! frame, and it used to do that with `Collider::from_shape`, which fills every field but the
//! shape from `Default`. Each field was then rediscovered as its own bug and fixed by appending
//! one more `.with_*` call:
//!
//! - `material` — a custom restitution/friction never reached the solver, so an elastic
//!   (restitution 1) ball behaved as the default 0.3;
//! - `is_trigger` — a collider with the inspector's "Trigger (Tetikleyici)" box ticked was rebuilt
//!   as a solid one, so the player hit the door sensor instead of passing through it, and since
//!   the pipeline chooses between a contact manifold and a `TriggerEvent` on that same flag, **no
//!   ECS body could emit a trigger event at all**;
//! - `collision_layer` — layer filtering is opt-in, and opting in from the ECS did nothing.
//!
//! The gather now clones the authored collider and replaces only its shape, so the class is
//! closed rather than the three instances of it. These tests drive the ECS path, which is the
//! part that was broken: `scene_queries.rs` sets `is_trigger` too and passes, because it calls
//! `PhysicsWorld::add_body` directly and never crosses the bridge.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::components::{CollisionLayer, CombineMode, PhysicsMaterial};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

fn elastic() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 1.0,
        static_friction: 0.0,
        dynamic_friction: 0.0,
        restitution_combine: CombineMode::Max,
        ..Default::default()
    }
}

#[test]
fn collider_material_restitution_reaches_the_solver() {
    let mut world = World::new();
    // No gravity → a clean 1-D horizontal collision.
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));

    // Ball A (x=-1.02, moving +x at 5) strikes resting Ball B (x=0). r=0.5 each,
    // surfaces 0.02 apart. use_gravity=false so nothing perturbs the 1-D motion.
    let a = world.spawn();
    world.add_component(a, Transform::new(Vec3::new(-1.02, 0.0, 0.0)));
    world.add_component(a, RigidBody::new(1.0, false));
    world.add_component(a, Velocity::new(Vec3::new(5.0, 0.0, 0.0)));
    world.add_component(a, Collider::sphere(0.5).with_material(elastic()));

    let b = world.spawn();
    world.add_component(b, Transform::new(Vec3::ZERO));
    world.add_component(b, RigidBody::new(1.0, false));
    world.add_component(b, Velocity::default());
    world.add_component(b, Collider::sphere(0.5).with_material(elastic()));

    for _ in 0..40 {
        gizmo_physics_rigid::system::physics_step_system(&world, 1.0 / 120.0);
    }

    let vs = world.borrow::<Velocity>();
    let va = vs.get(a.id()).unwrap().linear.x;
    let vb = vs.get(b.id()).unwrap().linear.x;

    // Elastic equal-mass 1-D: the striker nearly stops, the target carries most of
    // the speed. With the material dropped (restitution → default 0.3) B would only
    // get ~2.9 and A would keep ~2.1 (a near-inelastic split).
    assert!(vb > 3.5, "elastic transfer — B should carry most of the speed, got vb={vb}");
    assert!(va < 1.6, "elastic transfer — A should nearly stop, got va={va}");
    // Momentum is (approximately) conserved either way — sanity check.
    assert!((va + vb - 5.0).abs() < 0.5, "momentum ~conserved, got {}", va + vb);
}

/// **A trigger authored in the ECS is a trigger in the solver**: it reports the overlap and
/// pushes nothing.
///
/// Both halves are asserted, because each on its own can pass for the wrong reason: an overlap
/// that reports nothing but also pushes nothing looks like a working trigger from the outside
/// (it is a body that simply missed), and a pair that reports an event while still solving a
/// contact is what a half-applied fix would produce.
#[test]
fn a_trigger_collider_authored_in_the_ecs_reports_and_never_pushes() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));

    // A moves +x at 5 m/s straight into B, which is a static trigger volume sitting on its path.
    let a = world.spawn();
    world.add_component(a, Transform::new(Vec3::new(-1.02, 0.0, 0.0)));
    world.add_component(a, RigidBody::new(1.0, false));
    world.add_component(a, Velocity::new(Vec3::new(5.0, 0.0, 0.0)));
    world.add_component(a, Collider::sphere(0.5));

    let b = world.spawn();
    world.add_component(b, Transform::new(Vec3::ZERO));
    world.add_component(b, RigidBody::new_static());
    world.add_component(b, Velocity::default());
    world.add_component(b, Collider::sphere(0.5).with_trigger(true));

    let mut saw_trigger = false;
    let mut saw_collision = false;
    for _ in 0..40 {
        gizmo_physics_rigid::system::physics_step_system(&world, 1.0 / 120.0);
        if let Ok(pw) = world.try_get_resource::<PhysicsWorld>() {
            saw_trigger |= !pw.trigger_events.is_empty();
            saw_collision |= !pw.collision_events.is_empty();
        }
    }

    assert!(
        saw_trigger,
        "the overlap produced no TriggerEvent — with `is_trigger` dropped at the gather, no ECS \
         body can ever produce one, which is what left Lua's `physics.triggers` always empty"
    );
    assert!(
        !saw_collision,
        "a trigger overlap must not build a contact manifold; the solver saw a collision"
    );

    let vs = world.borrow::<Velocity>();
    let va = vs.get(a.id()).unwrap().linear.x;
    assert!(
        va > 4.9,
        "the body passed through a sensor, so nothing should have slowed it — got vx={va}. \
         Before the fix it bounced off the door sensor instead."
    );
    let xs = world.borrow::<Transform>();
    assert!(
        xs.get(a.id()).unwrap().position.x > 0.5,
        "and it should be out the far side of the trigger by now"
    );
}

/// **A collision layer authored in the ECS filters in the solver.**
///
/// Layer filtering is opt-in — the default mask accepts everything — so the only way to see this
/// field arrive is to opt out of a collision that would otherwise happen. Same geometry as the
/// test above with the trigger replaced by mutually-exclusive layers: A on layer 1 accepting only
/// layer 1, B on layer 2 accepting only layer 2.
#[test]
fn a_collision_layer_authored_in_the_ecs_filters_in_the_solver() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));

    let a = world.spawn();
    world.add_component(a, Transform::new(Vec3::new(-1.02, 0.0, 0.0)));
    world.add_component(a, RigidBody::new(1.0, false));
    world.add_component(a, Velocity::new(Vec3::new(5.0, 0.0, 0.0)));
    world.add_component(
        a,
        Collider::sphere(0.5).with_layer(CollisionLayer::new(1).with_mask(1 << 1)),
    );

    let b = world.spawn();
    world.add_component(b, Transform::new(Vec3::ZERO));
    world.add_component(b, RigidBody::new(1.0, false));
    world.add_component(b, Velocity::default());
    world.add_component(
        b,
        Collider::sphere(0.5).with_layer(CollisionLayer::new(2).with_mask(1 << 2)),
    );

    for _ in 0..40 {
        gizmo_physics_rigid::system::physics_step_system(&world, 1.0 / 120.0);
    }

    let vs = world.borrow::<Velocity>();
    assert!(
        vs.get(a.id()).unwrap().linear.x > 4.9,
        "A accepts only layer 1 and B is on layer 2, so the pair must never be solved — \
         got vx={}",
        vs.get(a.id()).unwrap().linear.x
    );
    assert_eq!(
        vs.get(b.id()).unwrap().linear.x,
        0.0,
        "and B must not have been pushed by a collision that should have been filtered out"
    );
}
