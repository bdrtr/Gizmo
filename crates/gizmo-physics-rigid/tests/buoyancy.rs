//! What floats, floats as it should: at the depth its density says, upright, and slowed by the
//! water it turns in.
//!
//! Until 2026-11-04 a `FluidZone` pushed only on a body whose **origin** was inside it, with one
//! force at the centre of mass worked out from the body's vertical extent alone. Measured with
//! this file before the change, three things followed:
//!
//! * a box of density 300 floated **half** under (its origin at the surface, where the push
//!   switches off), not at the 30 % Archimedes gives;
//! * a hull tipped 30° stayed tipped — the one force had no lever arm, so nothing righted it;
//! * a body spinning in water kept spinning — the drag acted on linear velocity only.
//!
//! The force is now summed over a 3×3×3 grid of cells through the collider's local bounds, each
//! pushed up by the water it displaces and dragged by its own velocity, at its own point.

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::world::{FluidZone, ZoneShape};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

const DT: f32 = 1.0 / 60.0;

/// A pool 40 m across whose surface is at y = 0, damped like water: ½·ρ·C_d·A·v² for a 1 m² face
/// at C_d ≈ 1 is 500·v² N.
fn pool() -> PhysicsWorld {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    world.fluid_zones.push(FluidZone {
        shape: ZoneShape::Box { min: Vec3::new(-20.0, -10.0, -20.0), max: Vec3::new(20.0, 0.0, 20.0) },
        density: 1000.0,
        linear_drag: 300.0,
        quadratic_drag: 500.0,
        ..Default::default()
    });
    world
}

/// Adds a dynamic box of `half` extents and `density` kg/m³ at `pos`, turned by `rot`.
fn add_box(world: &mut PhysicsWorld, id: u32, half: Vec3, density: f32, pos: Vec3, rot: Quat) {
    let col = Collider::box_collider(half);
    let mut rb = RigidBody::new(density * col.volume(), true);
    rb.update_inertia_from_collider(&col);
    rb.wake_up();
    let mut t = Transform::new(pos);
    t.rotation = rot;
    world.add_body(BodyHandle::from_id(id), rb, t, Velocity::default(), col);
}

fn run(world: &mut PhysicsWorld, seconds: f32) {
    for _ in 0..(seconds / DT) as usize {
        world.step(DT).unwrap();
    }
}

/// Archimedes: a body of density ρ in water floats with the fraction ρ/1000 of its height under.
#[test]
fn a_light_box_floats_at_the_depth_its_density_gives() {
    for density in [300.0_f32, 700.0] {
        let mut world = pool();
        add_box(&mut world, 1, Vec3::splat(0.5), density, Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY);
        run(&mut world, 12.0);
        let y = world.transforms[0].position.y;
        let under = (0.5 - y) / 1.0; // the share of the 1 m height below y = 0
        let want = density / 1000.0;
        assert!(
            (under - want).abs() < 0.05,
            "density {density}: {:.0} % under water, Archimedes says {:.0} % (centre at y {y:.3})",
            under * 100.0,
            want * 100.0
        );
    }
}

/// A flat hull tipped 30° about its long axis comes back upright.
///
/// It rolls back and forth on the way, and the roll dies slowly — measured, the largest tilt in
/// each second goes 29.9, 24.6, 20.8 … 9.8 (tenth) … 2.2° (thirtieth): about 8 % a second, from
/// the drag on the cells as they swing. Before the cells it stayed at 30.0° for good.
#[test]
fn a_tipped_hull_rights_itself() {
    let mut world = pool();
    let tilt = Quat::from_rotation_z(30f32.to_radians());
    add_box(&mut world, 1, Vec3::new(1.0, 0.25, 2.0), 400.0, Vec3::new(0.0, 0.05, 0.0), tilt);
    let mut worst = Vec::new();
    for second in 0..30 {
        let mut most = 0f32;
        for _ in 0..60 {
            world.step(DT).unwrap();
            let up = world.transforms[0].rotation * Vec3::Y;
            most = most.max(up.angle_between(Vec3::Y).to_degrees());
        }
        worst.push((second, most));
    }
    assert!(worst[9].1 < 15.0, "still tipped up to {:.1}° in the tenth second (started at 30°)", worst[9].1);
    assert!(worst[29].1 < 4.0, "still tipped up to {:.1}° in the thirtieth second", worst[29].1);
}

/// Water slows a spinning body as it slows a moving one.
#[test]
fn water_slows_a_spinning_body() {
    let mut world = pool();
    add_box(&mut world, 1, Vec3::new(1.0, 0.25, 2.0), 400.0, Vec3::new(0.0, -0.05, 0.0), Quat::IDENTITY);
    world.velocities[0].angular = Vec3::new(0.0, 3.0, 0.0);
    run(&mut world, 4.0);
    let spin = world.velocities[0].angular.y;
    assert!(spin.abs() < 1.0, "a hull spun at 3 rad/s still turns at {spin:.2} rad/s after 4 s");
}

/// A ball — no orientation to right — floats as it did, and a heavy one still sinks.
#[test]
fn a_ball_floats_and_a_stone_sinks() {
    let mut world = pool();
    let ball = Collider::sphere(0.5);
    let mut rb = RigidBody::new(200.0 * ball.volume(), true);
    rb.update_inertia_from_collider(&ball);
    rb.wake_up();
    world.add_body(BodyHandle::from_id(1), rb, Transform::new(Vec3::new(0.0, 1.0, 0.0)), Velocity::default(), ball);
    add_box(&mut world, 2, Vec3::splat(0.25), 2500.0, Vec3::new(3.0, 1.0, 0.0), Quat::IDENTITY);
    run(&mut world, 8.0);
    let ball_y = world.transforms[0].position.y;
    let stone_y = world.transforms[1].position.y;
    assert!(ball_y > -0.5 && ball_y < 0.5, "the ball is not at the surface: y {ball_y:.2}");
    assert!(stone_y < -5.0, "the stone did not sink: y {stone_y:.2}");
}
