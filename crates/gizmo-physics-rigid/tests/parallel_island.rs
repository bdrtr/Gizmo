//! A large island's coloured sweep (`solver/parallel.rs`): the island's contact blocks coloured so
//! that no two of a colour share a dynamic body, each colour solved on several threads.
//!
//! What it must keep: a world steps to the same bits whatever the thread pool's size (the colours'
//! blocks are independent, so who solves which cannot matter), and a pile solved this way settles
//! and sleeps as a pile should.
//!
//! The threshold is lowered to 16 blocks here so a pile small enough for a debug test takes the
//! path; a game's islands take it from 256 (`ConstraintSolver::parallel_island_blocks`).
#![cfg(not(target_arch = "wasm32"))]

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

/// `side³` boxes dropped in a loose grid, each turned a little, onto a static ground.
fn pile(side: u32, parallel_island_blocks: usize) -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    world.solver.parallel_island_blocks = parallel_island_blocks;
    world.add_body(
        BodyHandle::from_id(0),
        RigidBody::new_static(),
        Transform::new(Vec3::new(0.0, -1.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(30.0, 1.0, 30.0)),
    );
    let axis = Vec3::new(1.0, 1.0, 0.0).normalize();
    let mut id = 1;
    for y in 0..side {
        for x in 0..side {
            for z in 0..side {
                let mut rb = RigidBody::new(1.0, true);
                rb.calculate_box_inertia(1.0, 1.0, 1.0);
                let half = (side as f32 - 1.0) / 2.0;
                let p = Vec3::new((x as f32 - half) * 1.3, 1.0 + y as f32 * 1.3, (z as f32 - half) * 1.3);
                world.add_body(
                    BodyHandle::from_id(id),
                    rb,
                    Transform::new(p).with_rotation(Quat::from_axis_angle(axis, id as f32 * 0.37 % std::f32::consts::TAU)),
                    Velocity::default(),
                    Collider::box_collider(Vec3::splat(0.5)),
                );
                id += 1;
            }
        }
    }
    world
}

fn hash_after(threads: usize, parallel_island_blocks: usize, frames: u32) -> u64 {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("pool");
    pool.install(|| {
        let mut world = pile(4, parallel_island_blocks);
        for _ in 0..frames {
            world.step(1.0 / 60.0).expect("step");
        }
        world.state_hash()
    })
}

#[test]
fn a_coloured_island_steps_to_the_same_bits_on_one_thread_and_on_four() {
    let one = hash_after(1, 16, 90);
    let four = hash_after(4, 16, 90);
    assert_eq!(one, four, "the coloured sweep depends on the thread count");
    // And it was the coloured sweep that ran: the serial one visits the blocks in another order.
    let serial = hash_after(4, 0, 90);
    assert_ne!(one, serial, "the pile never took the coloured path — the test proves nothing");
}

#[test]
fn a_pile_solved_by_colour_settles_and_sleeps() {
    let mut world = pile(4, 16);
    let mut asleep_at = None;
    for frame in 0..900 {
        world.step(1.0 / 60.0).expect("step");
        if (1..world.entities.len()).all(|i| world.rigid_bodies[i].is_sleeping) {
            asleep_at = Some(frame);
            break;
        }
    }
    let lowest = (1..world.entities.len()).map(|i| world.transforms[i].position.y).fold(f32::INFINITY, f32::min);
    let fastest = (1..world.entities.len()).map(|i| world.velocities[i].linear.length()).fold(0.0, f32::max);
    // A box resting on the ground has its centre at 0.5; a few millimetres into it is the soft contact's.
    assert!(lowest > 0.48, "a box sank into the ground: centre at {lowest}");
    assert!(asleep_at.is_some(), "the pile never slept in 15 s; fastest body still at {fastest} m/s");
}
