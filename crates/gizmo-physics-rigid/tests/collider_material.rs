//! Regression: the ECS→PhysicsWorld collider gather in `physics_step_system` used
//! to rebuild the collider with `Collider::from_shape`, dropping the authored
//! `PhysicsMaterial` — so custom restitution/friction never reached the solver
//! (an elastic restitution=1 ball behaved as the default 0.3). This drives a
//! head-on equal-mass elastic collision through the ECS path and checks the
//! momentum transfers (only possible if the material survived the gather).

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::components::{CombineMode, PhysicsMaterial};
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
