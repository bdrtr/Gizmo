//! **A kinematic body driven to a pose.** A game knows where a tilting board, a door or a lift
//! should be; with a fixed-step driver a velocity computed from the frame time turns it by the
//! wrong amount, because a frame runs zero, one or several steps. `KinematicTarget` makes the
//! step do it: the body reaches the target in each step and lands on it exactly, and what rests
//! on it is carried by the motion.

use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{KinematicTarget, RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 120.0;

fn step(world: &mut World) {
    gizmo_physics_rigid::system::physics_step_system(world, DT);
    world.apply_commands();
}

fn board(world: &mut World) -> gizmo_core::entity::Entity {
    let board = world.spawn();
    world.add_component(board, Transform::new(Vec3::ZERO));
    world.add_component(board, RigidBody::new_kinematic());
    world.add_component(board, Velocity::default());
    world.add_component(board, Collider::box_collider(Vec3::new(4.0, 0.1, 4.0)));
    board
}

/// Moved by a target, the body arrives in one step and stays there exactly; a later target moves
/// it on; a dynamic body ignores the component.
#[test]
fn a_target_is_reached_in_one_step_and_held_exactly() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let b = board(&mut world);
    let goal = KinematicTarget::new(Vec3::new(0.0, 0.5, 0.0), Quat::from_rotation_x(0.15));
    world.add_component(b, goal);
    step(&mut world);
    let t = *world.borrow::<Transform>().get(b).unwrap();
    assert_eq!((t.position, t.rotation), (goal.position, goal.rotation), "not exactly on the target");
    let v = *world.borrow::<Velocity>().get(b).unwrap();
    assert!((v.angular.x - 0.15 / DT).abs() < 1e-2, "the step's velocity is the motion: {v:?}");
    // Held: many steps later, still exactly there, not moving.
    for _ in 0..60 {
        step(&mut world);
    }
    let t = *world.borrow::<Transform>().get(b).unwrap();
    assert_eq!((t.position, t.rotation), (goal.position, goal.rotation));
    assert!(world.borrow::<Velocity>().get(b).unwrap().angular.length() < 1e-4);

    // On a dynamic body it is ignored: the body falls.
    let ball = world.spawn();
    world.add_component(ball, Transform::new(Vec3::new(20.0, 5.0, 0.0)));
    world.add_component(ball, RigidBody::new(1.0, true));
    world.add_component(ball, Velocity::default());
    world.add_component(ball, Collider::sphere(0.2));
    world.add_component(ball, KinematicTarget::new(Vec3::new(20.0, 5.0, 0.0), Quat::IDENTITY));
    for _ in 0..60 {
        step(&mut world);
    }
    assert!(world.borrow::<Transform>().get(ball).unwrap().position.y < 4.0);
}

/// A board tilted step by step towards +x carries a resting ball and rolls it downhill, while the
/// ball stays on the board — what a marble maze is made of.
#[test]
fn a_tilting_board_rolls_the_ball_on_it_downhill() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let b = board(&mut world);
    let ball = world.spawn();
    world.add_component(ball, Transform::new(Vec3::new(0.0, 0.31, 0.0)));
    world.add_component(ball, RigidBody::new(0.5, true));
    world.add_component(ball, Velocity::default());
    world.add_component(ball, Collider::sphere(0.2));
    for _ in 0..60 {
        step(&mut world);
    }
    // Tilt about z by up to 8°, a little each step (a mouse moving), so +x goes down.
    for i in 1..=120 {
        let angle = -(8f32.to_radians()) * (i as f32 / 120.0);
        world.add_component(b, KinematicTarget::new(Vec3::ZERO, Quat::from_rotation_z(angle)));
        step(&mut world);
    }
    for _ in 0..60 {
        step(&mut world);
    }
    let p = world.borrow::<Transform>().get(ball).unwrap().position;
    assert!(p.x > 0.5, "the ball did not roll downhill: {p:?}");
    // Still on the board's surface: the surface at x is at -x·tan(8°) + 0.1, plus the radius.
    let surface = -p.x * 8f32.to_radians().tan() + 0.1 + 0.2;
    assert!((p.y - surface).abs() < 0.08, "the ball left the board: {p:?} (surface {surface})");
}

/// A board made of tiles with a gap (`Collider::compound`): a ball over a tile rests on it, a
/// ball over the gap falls through — a maze's floor and its holes.
#[test]
fn a_compound_board_holds_a_ball_on_a_tile_and_drops_one_through_the_gap() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let tile = |x: f32| (Transform::new(Vec3::new(x, -0.1, 0.0)), Collider::box_collider(Vec3::new(0.5, 0.1, 0.5)));
    let board = world.spawn();
    world.add_component(board, Transform::new(Vec3::ZERO));
    world.add_component(board, RigidBody::new_kinematic());
    world.add_component(board, Velocity::default());
    // Tiles at x = −1 and x = +1; nothing at x = 0.
    world.add_component(board, Collider::compound([tile(-1.0), tile(1.0)]));
    world.add_component(board, KinematicTarget::new(Vec3::ZERO, Quat::IDENTITY));
    let ball = |world: &mut World, x: f32| {
        let b = world.spawn();
        world.add_component(b, Transform::new(Vec3::new(x, 0.5, 0.0)));
        world.add_component(b, RigidBody::new(0.2, true));
        world.add_component(b, Velocity::default());
        world.add_component(b, Collider::sphere(0.3));
        b
    };
    let on_tile = ball(&mut world, 1.0);
    let over_gap = ball(&mut world, 0.0);
    for _ in 0..120 {
        step(&mut world);
    }
    let y = |w: &World, e: gizmo_core::entity::Entity| w.borrow::<Transform>().get(e).unwrap().position.y;
    assert!((y(&world, on_tile) - 0.3).abs() < 0.05, "on the tile: {}", y(&world, on_tile));
    assert!(y(&world, over_gap) < -1.0, "through the gap: {}", y(&world, over_gap));
}

/// A ball with CCD rolling down a floor of seventeen box tiles (one compound) goes nearly as far
/// as on one slab: the tiles' seams do not catch it. Before seam contacts were dropped it covered
/// 1.01 m in two seconds against the slab's 2.21 m (the analytic value for a rolling sphere on
/// this slope); 2.03 m after. The last 8 % was not the seams at all: it was the one speculative
/// contact on landing (the ball starts 5 cm up), which wrote its zero gap as `-0.0` — not
/// `< 0.0`, so the solver took it for a resting contact and applied friction at the ball's
/// centre, stopping the 0.156 m/s it carried down the slope. 2.23 m now. (The slab run lands
/// through a real contact, so it never saw it.)
#[test]
fn a_ball_rolls_across_tile_seams_as_on_one_slab() {
    let run = |tiles: bool| {
        let dt = 1.0 / 240.0;
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
        let tilt = Quat::from_rotation_z(-0.16);
        let board = world.spawn();
        world.add_component(board, Transform::new(Vec3::ZERO).with_rotation(tilt));
        world.add_component(board, RigidBody::new_static());
        let floor = if tiles {
            Collider::compound((-8..=8).map(|x| {
                (Transform::new(Vec3::new(x as f32, -0.1, 0.0)), Collider::box_collider(Vec3::new(0.5, 0.1, 0.5)))
            }))
        } else {
            Collider::compound([(Transform::new(Vec3::new(0.0, -0.1, 0.0)), Collider::box_collider(Vec3::new(8.5, 0.1, 0.5)))])
        };
        world.add_component(board, floor);
        let ball = world.spawn();
        world.add_component(ball, Transform::new(tilt * Vec3::new(-6.0, 0.35, 0.0)));
        let mut rb = RigidBody::new(0.2, true);
        rb.ccd_enabled = true;
        world.add_component(ball, rb);
        world.add_component(ball, Velocity::default());
        world.add_component(ball, Collider::sphere(0.3));
        let start = world.borrow::<Transform>().get(ball).unwrap().position;
        for _ in 0..480 {
            gizmo_physics_rigid::system::physics_step_system(&world, dt);
            world.apply_commands();
        }
        (world.borrow::<Transform>().get(ball).unwrap().position - start).length()
    };
    let (slab, tiles) = (run(false), run(true));
    let analytic = 0.5 * 9.81 * 0.16f32.sin() * 5.0 / 7.0 * 4.0;
    assert!((slab - analytic).abs() < 0.1 * analytic, "slab {slab} vs analytic {analytic}");
    assert!((tiles - slab).abs() < 0.03 * slab, "tiles {tiles} vs slab {slab}");
}
