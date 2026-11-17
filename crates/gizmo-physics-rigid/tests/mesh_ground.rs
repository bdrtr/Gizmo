//! A triangle mesh as ground: a body on it is pushed up out of it, never through it and never off
//! an edge the surface goes on past. The terrain tests' twins (`heightfield_collider.rs`), for the
//! shape a city or a track is built from: each triangle went to GJK/EPA as a plate of its own, so
//! a box sunk tilted into a flat mesh fell through to y = −10, and a box sliding from one mesh onto
//! the next stopped at the seam.
//!
//! Driven through the real ECS path (`physics_step_system`).

use gizmo_core::world::World;
use gizmo_core::Entity;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::components::{CombineMode, PhysicsMaterial};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 240.0;

fn slippery() -> PhysicsMaterial {
    PhysicsMaterial {
        static_friction: 0.1,
        dynamic_friction: 0.1,
        friction_combine: CombineMode::Min,
        restitution: 0.0,
        ..Default::default()
    }
}

/// A flat `n`×`n` grid of 1 m quads, centred on its origin, its triangles wound facing up — or
/// down, with `upside_down`, which must not matter.
fn grid(n: i32, upside_down: bool) -> Collider {
    let mut v = Vec::new();
    let mut idx = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            v.push(Vec3::new(i as f32 - n as f32 / 2.0, 0.0, j as f32 - n as f32 / 2.0));
        }
    }
    let row = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * row + i;
            let quad = [a, a + row, a + row + 1, a, a + row + 1, a + 1];
            if upside_down {
                idx.extend_from_slice(&[quad[0], quad[2], quad[1], quad[3], quad[5], quad[4]]);
            } else {
                idx.extend_from_slice(&quad);
            }
        }
    }
    Collider::trimesh(v, idx).with_material(slippery())
}

fn scene() -> World {
    let mut w = World::new();
    w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    w
}

fn ground(world: &mut World, at: Vec3, mesh: Collider) {
    let e = world.spawn();
    world.add_component(e, Transform::new(at));
    world.add_component(e, RigidBody::new_static());
    world.add_component(e, Velocity::default());
    world.add_component(e, mesh);
}

/// A car-sized box.
fn car_box(world: &mut World, at: Vec3, rot: Quat) -> Entity {
    let c = Collider::box_collider(Vec3::new(0.9, 0.3, 2.0)).with_material(slippery());
    let mut rb = RigidBody::new(1.0, true);
    rb.update_inertia_from_collider(&c);
    let e = world.spawn();
    world.add_component(e, Transform::new(at).with_rotation(rot));
    world.add_component(e, rb);
    world.add_component(e, Velocity::default());
    world.add_component(e, c);
    e
}

fn step(world: &World, n: usize) {
    for _ in 0..n {
        physics_step_system(world, DT);
    }
}

fn push(world: &mut World, e: Entity, v: Vec3) {
    let mut q = world.query_mut::<(&mut RigidBody, &mut Velocity)>();
    let (mut rb, mut vel) = q.get_mut(e).unwrap();
    rb.wake_up();
    vel.linear = v;
}

/// **A body sunk into a mesh comes up out of it** — whichever way the mesh is wound.
#[test]
fn a_tilted_box_sunk_into_a_mesh_comes_up_out_of_it() {
    for upside_down in [false, true] {
        let mut world = scene();
        ground(&mut world, Vec3::ZERO, grid(10, upside_down));
        let b = car_box(&mut world, Vec3::new(0.3, 0.5, 0.0), Quat::from_rotation_x(0.35));
        step(&world, 480);
        let y = world.query::<&Transform>().get(b).unwrap().position.y;
        assert!(y > 0.25 && y < 1.0, "wound {}: it should rest on the mesh, centre ~0.3 m up: {y}", if upside_down { "down" } else { "up" });
    }
}

/// **A body crossing from one mesh to the next is neither stopped nor thrown.** On slippery
/// ground a box sent at 4 m/s slides 6 m in two seconds whatever is under it; across the seam of
/// two meshes it stopped with its nose on the seam.
#[test]
fn a_box_sliding_across_the_seam_of_two_meshes_is_neither_stopped_nor_thrown() {
    let mut world = scene();
    ground(&mut world, Vec3::new(0.0, 0.0, -5.0), grid(10, false));
    ground(&mut world, Vec3::new(0.0, 0.0, 5.0), grid(10, false));
    let b = car_box(&mut world, Vec3::new(0.0, 0.31, -3.0), Quat::IDENTITY);
    step(&world, 240);
    push(&mut world, b, Vec3::new(0.0, 0.0, 4.0));
    let (mut farthest, mut fastest_up) = (f32::MIN, 0.0f32);
    for _ in 0..480 {
        step(&world, 1);
        farthest = farthest.max(world.query::<&Transform>().get(b).unwrap().position.z);
        fastest_up = fastest_up.max(world.query::<&Velocity>().get(b).unwrap().linear.y.abs());
    }
    assert!(farthest > 2.5, "stopped at the seam: its centre got to z = {farthest}");
    assert!(fastest_up < 0.5, "thrown at the seam: |vy| reached {fastest_up} m/s");
}

/// **A CCD ball rolling from one mesh onto the next is not bumped by the one it leaves.** The
/// mesh beside it offered its border edge as a speculative contact, sideways: a 7 mm hop at the
/// seam (on terrain, the same edge threw it 0.9 m).
#[test]
fn a_ccd_ball_rolling_across_the_seam_of_two_meshes_is_not_bumped() {
    let mut world = scene();
    ground(&mut world, Vec3::new(0.0, 0.0, -5.0), grid(10, false));
    ground(&mut world, Vec3::new(0.0, 0.0, 5.0), grid(10, true));
    let col = Collider::sphere(0.3).with_material(slippery());
    let mut rb = RigidBody::new(1.0, true);
    rb.calculate_sphere_inertia(0.3);
    rb.ccd_enabled = true;
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::new(0.0, 0.31, -3.0)));
    world.add_component(e, rb);
    world.add_component(e, Velocity::default());
    world.add_component(e, col);
    step(&world, 240);
    push(&mut world, e, Vec3::new(0.0, 0.0, 8.0));
    let (mut farthest, mut highest) = (f32::MIN, f32::MIN);
    for _ in 0..240 {
        step(&world, 1);
        let p = world.query::<&Transform>().get(e).unwrap().position;
        farthest = farthest.max(p.z);
        highest = highest.max(p.y);
    }
    assert!(farthest > 3.0, "stopped at the seam: got to z = {farthest}");
    assert!(highest < 0.304, "bumped up at the seam: centre rose {:.1} mm", (highest - 0.3) * 1000.0);
}
