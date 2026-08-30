//! `physics_step_system`'s compound-collider gather, over a `Children` graph that is not a tree.
//!
//! The gather walks an entity's descendants every step and appends each child's collider to the
//! body's compound shape. It carried no visited set until 2026-08-30, and the two shapes of
//! non-tree graph break it in two different ways — which is why there are two tests here and not
//! one:
//!
//! - a **cycle** makes the walk never finish, so the physics step never returns. It is inside
//!   the per-frame budget, so a scene that loads with one hangs on its FIRST tick.
//! - a **diamond** — the same id in two `Children` lists — terminates on its own, and is the
//!   quieter of the two: the shared child's collider is added to the compound body TWICE. The
//!   body ends up with a doubled sub-shape it never authored, and nothing reports it.
//!
//! One `visited.insert` is both guards, so either test alone would catch its removal. Both are
//! here because they fail in ways a reader would not predict from the other.
//!
//! A cycle is reachable even though `HierarchyExt::add_child` refuses to build one: `Children` is
//! an ordinary component that `add_component` writes directly, and `SceneData::instantiate_entities`
//! writes a scene file's parent edges verbatim with no cycle rejection anywhere on that path.

use gizmo_core::component::Children;
use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, ColliderShape, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

/// A world with the physics resource and the hierarchy component registered.
fn world() -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new());
    world.register_component_type::<Children>();
    world
}

/// Spawns a body the outer query will pick up: `RigidBody` + `Collider` + `Transform`.
fn spawn_body(world: &mut World, at: Vec3) -> gizmo_core::entity::Entity {
    let e = world.spawn();
    world.add_component(e, RigidBody::default());
    world.add_component(e, Velocity::default());
    world.add_component(e, Transform::new(at));
    world.add_component(e, Collider::sphere(0.5));
    e
}

/// Spawns a child that CONTRIBUTES a shape: `Collider` + `Transform`, but no `RigidBody`, so it
/// is gathered into its parent's compound rather than simulated on its own.
fn spawn_child_collider(world: &mut World, at: Vec3) -> gizmo_core::entity::Entity {
    let e = world.spawn();
    world.add_component(e, Transform::new(at));
    world.add_component(e, Collider::sphere(0.25));
    e
}

/// Spawns a child that contributes NOTHING — a `Transform` and no `Collider`. Used to build the
/// cycle, so that the pre-fix failure is a walk that spins at flat memory instead of one that
/// allocates a sub-shape per iteration. A test whose failure mode is unbounded allocation takes
/// the machine down rather than going red, and `--no-fail-fast` would run the rest of the suite
/// beside it.
fn spawn_bare(world: &mut World, at: Vec3) -> gizmo_core::entity::Entity {
    let e = world.spawn();
    world.add_component(e, Transform::new(at));
    e
}

/// Reads back the number of sub-shapes the step gave this body's collider.
/// `None` means the collider is not a compound at all.
fn compound_len(world: &World, id: u32) -> Option<usize> {
    let physics = world.get_resource::<PhysicsWorld>().expect("physics world");
    let &idx = physics.entity_index_map.get(&id)?;
    match &physics.colliders[idx].shape {
        ColliderShape::Compound(parts) => Some(parts.len()),
        _ => None,
    }
}

/// Runs `f` on a worker thread and fails if it has not finished within `secs`.
///
/// The guard under test protects against a walk that never ends, and a walk that never ends has
/// no assertion to disagree with: the natural test does not fail, it hangs. A suite that hangs
/// covers less than one that goes red — the same argument `CLAUDE.md` makes for `--no-fail-fast`.
fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(std::time::Duration::from_secs(secs)) {
        Ok(value) => value,
        // Told apart on purpose: `expect` would report a panicking worker — whose own message
        // has already been printed — as a walk that looped for `secs` seconds.
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("the physics step did not finish within {secs}s — a `Children` cycle looped")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            panic!("the worker panicked before answering; its own message is above")
        }
    }
}

/// The guard's reason for existing: without it this step never returns.
#[test]
fn a_children_cycle_under_a_body_does_not_hang_the_step() {
    let compound = within(10, || {
        let mut world = world();
        let body = spawn_body(&mut world, Vec3::ZERO);
        let a = spawn_bare(&mut world, Vec3::X);
        let b = spawn_bare(&mut world, Vec3::Y);

        // body -> a -> b -> a. Written directly, bypassing `add_child`'s refusal.
        world.add_component(body, Children(vec![a.id()]));
        world.add_component(a, Children(vec![b.id()]));
        world.add_component(b, Children(vec![a.id()]));

        physics_step_system(&world, 1.0 / 60.0);
        // Neither cycle member carries a collider, so the body keeps its own single shape and
        // never becomes a compound. Reaching this line at all is the assertion.
        compound_len(&world, body.id())
    });

    assert_eq!(compound, None, "no child contributed a shape, so no compound was built");
}

/// A DIAMOND terminates on its own, so this is not about hanging: it is about counting.
/// The shared child's collider must reach the compound body once, not twice.
#[test]
fn a_diamond_adds_the_shared_child_collider_once() {
    let mut world = world();
    let body = spawn_body(&mut world, Vec3::ZERO);
    let left = spawn_bare(&mut world, Vec3::X);
    let right = spawn_bare(&mut world, Vec3::NEG_X);
    let shared = spawn_child_collider(&mut world, Vec3::Y);

    // body -> {left, right}, and BOTH name `shared` as their child.
    world.add_component(body, Children(vec![left.id(), right.id()]));
    world.add_component(left, Children(vec![shared.id()]));
    world.add_component(right, Children(vec![shared.id()]));

    physics_step_system(&world, 1.0 / 60.0);

    assert_eq!(
        compound_len(&world, body.id()),
        Some(2),
        "the body's own shape plus `shared` once — three means it was gathered twice"
    );
}
