//! What many resting bodies cost a step — a measurement, not a gate (run it in release:
//! `cargo test --release -p gizmo-physics-rigid --test many_sleeping_bodies -- --ignored --nocapture`).
//!
//! The shape of `nfsu2-gizmo`'s breakable props (MOTOR-NOTLARI #26): a few hundred small boxes
//! resting on static terrain cells of a few thousand triangles each. A body that has come to rest
//! and gone to sleep should cost next to nothing; the report measured the frame at 20.8 ms without
//! them and 29.1 ms with them.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 60.0;
const CELLS: usize = 4; // 4 × 4 cells
const CELL: f32 = 64.0;
const GRID: usize = 32; // 32 × 32 quads = 2 048 triangles a cell

/// A flat-ish terrain cell of `GRID`² quads, its origin at `(cx, 0, cz)`.
fn cell(cx: f32, cz: f32) -> Collider {
    let mut vertices = Vec::new();
    for j in 0..=GRID {
        for i in 0..=GRID {
            let (x, z) = (i as f32 / GRID as f32 * CELL, j as f32 / GRID as f32 * CELL);
            let y = 0.05 * ((x + cx) * 0.37).sin() * ((z + cz) * 0.29).cos();
            vertices.push(Vec3::new(x, y, z));
        }
    }
    let mut indices = Vec::new();
    let row = (GRID + 1) as u32;
    for j in 0..GRID as u32 {
        for i in 0..GRID as u32 {
            let a = j * row + i;
            indices.extend_from_slice(&[a, a + row, a + 1, a + 1, a + row, a + row + 1]);
        }
    }
    Collider::trimesh(vertices, indices)
}

fn scene(boxes: usize) -> World {
    scene_with(boxes, false)
}

/// `asleep`: every box starts asleep, as a game that places props at rest does.
fn scene_with(boxes: usize, asleep: bool) -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    for cz in 0..CELLS {
        for cx in 0..CELLS {
            let at = Vec3::new(cx as f32 * CELL, 0.0, cz as f32 * CELL);
            let e = world.spawn();
            world.add_component(e, Transform::new(at));
            world.add_component(e, RigidBody::new_static());
            world.add_component(e, Velocity::default());
            world.add_component(e, cell(at.x, at.z));
        }
    }
    for k in 0..boxes {
        let side = (CELLS as f32 * CELL) as usize / 11;
        let (i, j) = (k % side, k / side);
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new(3.0 + i as f32 * 11.0, 0.6, 3.0 + j as f32 * 11.0)));
        let mut rb = RigidBody::new(80.0, true);
        rb.calculate_box_inertia(0.8, 0.8, 0.8);
        rb.is_sleeping = asleep;
        world.add_component(e, rb);
        world.add_component(e, Velocity::default());
        world.add_component(e, Collider::box_collider(Vec3::splat(0.4)));
    }
    world
}

/// Median milliseconds of the last `measured` of `steps` steps, and how many bodies sleep at the end.
fn run(world: &mut World, steps: usize, measured: usize) -> (f64, usize) {
    let mut times = Vec::new();
    for s in 0..steps {
        // A frame, as a schedule opens one: what the step reads as changed is measured against it.
        let next = world.tick;
        world.begin_change_frame(next);
        let t = std::time::Instant::now();
        gizmo_physics_rigid::system::physics_step_system(world, DT);
        world.apply_commands();
        if s >= steps - measured {
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    times.sort_by(f64::total_cmp);
    let asleep = world.query::<&RigidBody>().iter().filter(|rb| rb.is_dynamic() && rb.is_sleeping).count();
    (times[times.len() / 2], asleep)
}

#[test]
#[ignore = "a measurement; run in release with --nocapture"]
fn resting_bodies_cost_a_step() {
    let mut terrain = scene(0);
    let (bare, _) = run(&mut terrain, 120, 60);
    let mut props = scene(513);
    let (settling, _) = run(&mut props, 60, 30);
    // More steps at rest to sample a profiler against: `GIZMO_REST_STEPS=20000`.
    let rest_steps = std::env::var("GIZMO_REST_STEPS").ok().and_then(|s| s.parse().ok()).unwrap_or(600);
    let (rested, asleep) = run(&mut props, rest_steps, 120);
    // Placed asleep, 0.2 m above the ground (where a prop authored to stand on it would be).
    let mut placed = scene_with(513, true);
    let (first, _) = run(&mut placed, 30, 30);
    let (later, placed_asleep) = run(&mut placed, 300, 120);
    eprintln!(
        "{} terrain cells of {} triangles: {bare:.3} ms a step; with 513 boxes settling {settling:.3} ms, at rest {rested:.3} ms ({asleep} asleep); placed asleep: first steps {first:.3} ms, later {later:.3} ms ({placed_asleep} asleep)",
        CELLS * CELLS,
        GRID * GRID * 2
    );
}

/// What a world of many static bodies costs a step, and what copying it to a render world costs
/// after one: 14 600 static boxes (the order of a city's merged meshes, each with a collider), no
/// moving body at all. `GIZMO_STATIC_STEPS` for more steps to sample a profiler against.
#[test]
#[ignore = "a measurement; run in release with --nocapture"]
fn static_bodies_cost_a_step() {
    use gizmo_core::extract::Extractor;
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    for k in 0..14_600 {
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new((k % 120) as f32 * 10.0, 0.0, (k / 120) as f32 * 10.0)));
        world.add_component(e, RigidBody::new_static());
        world.add_component(e, Velocity::default());
        world.add_component(e, Collider::box_collider(Vec3::splat(4.0)));
    }
    let mut copy = World::new();
    let mut extractor = Extractor::new();
    extractor.component::<Transform>();
    let steps: usize = std::env::var("GIZMO_STATIC_STEPS").ok().and_then(|s| s.parse().ok()).unwrap_or(200);
    let (mut step, mut extract) = (Vec::new(), Vec::new());
    for s in 0..steps {
        let next = world.tick;
        world.begin_change_frame(next);
        let t = std::time::Instant::now();
        gizmo_physics_rigid::system::physics_step_system(&world, DT);
        world.apply_commands();
        let stepped = std::time::Instant::now();
        extractor.run(&mut world, &mut copy);
        if s >= steps / 2 {
            step.push((stepped - t).as_secs_f64() * 1000.0);
            extract.push(stepped.elapsed().as_secs_f64() * 1000.0);
        }
    }
    step.sort_by(f64::total_cmp);
    extract.sort_by(f64::total_cmp);
    eprintln!(
        "14 600 static bodies: {:.3} ms a step, {:.3} ms to extract their transforms",
        step[step.len() / 2],
        extract[extract.len() / 2]
    );
}

/// **A step marks changed only what it moved.** A static cell and a sleeping box are not stamped
/// `Changed` by a step; a falling box is. Every body used to be written back every step, so a
/// `Changed<Transform>` filter matched the whole static world every frame.
#[test]
fn a_step_marks_changed_only_the_bodies_it_moved() {
    use gizmo_core::query::Changed;
    use gizmo_core::Entity;
    let mut world = scene_with(8, true);
    let falling = world.spawn();
    world.add_component(falling, Transform::new(Vec3::new(5.0, 20.0, 5.0)));
    world.add_component(falling, RigidBody::new(10.0, true));
    world.add_component(falling, Velocity::default());
    world.add_component(falling, Collider::box_collider(Vec3::splat(0.5)));
    for _ in 0..5 {
        gizmo_physics_rigid::system::physics_step_system(&world, DT);
        world.apply_commands();
    }
    let before = world.tick;
    let next = world.tick;
    world.begin_change_frame(next);
    gizmo_physics_rigid::system::physics_step_system(&world, DT);
    world.apply_commands();
    let transforms: Vec<_> = world.query_since::<Entity, Changed<Transform>>(before).iter().collect();
    assert_eq!(transforms, vec![falling], "Transform changed by one step: {transforms:?}");
    let velocities: Vec<_> = world.query_since::<Entity, Changed<Velocity>>(before).iter().collect();
    assert_eq!(velocities, vec![falling], "Velocity changed by one step: {velocities:?}");
    let bodies: Vec<_> = world.query_since::<Entity, Changed<RigidBody>>(before).iter().collect();
    assert!(
        bodies.iter().all(|&e| e == falling),
        "RigidBody changed by one step on a body that did not move: {bodies:?}"
    );
    let asleep = world.query::<&RigidBody>().iter().filter(|rb| rb.is_dynamic() && rb.is_sleeping).count();
    assert_eq!(asleep, 8, "the placed boxes stay asleep");
}
