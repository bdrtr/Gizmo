//! A CCD body a hair from a surface is stopped only by moving into it.
//!
//! Under a millimetre of gap, the speculative contact took its normal from the bodies' relative
//! velocity, not from the surface. A ball resting a fraction of a millimetre from a wall had the
//! velocity gravity gave it that step, so the wall's contact pointed against gravity and stopped
//! the ball's whole motion: measured in `pinbol`, a ball beside the rail of a tilted table hung
//! there with the normal (0, 0.978, −0.208) — the tilt — instead of rolling down. The same rule
//! slowed a CCD ball rolling on its own by 9 % (0.665 m in a second against 0.723 m without CCD).

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

const TILT: f32 = 12.0;

/// How far a ball of radius 5 cm rolls down a 12°-tilted floor in a second, starting `wall_gap`
/// from a wall running down the slope and drifting into it at `drift` m/s (`None`: no wall).
fn rolled(ccd: bool, wall_gap: Option<f32>, drift: f32) -> f32 {
    let t = TILT.to_radians();
    let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81 * t.cos(), 9.81 * t.sin()));
    let mut floor = RigidBody::new_static();
    floor.wake_up();
    world.add_body(BodyHandle::from_id(1), floor, Transform::new(Vec3::new(0.0, -0.05, 0.0)), Velocity::default(), Collider::box_collider(Vec3::new(5.0, 0.05, 5.0)));
    let r = 0.05;
    if let Some(gap) = wall_gap {
        let mut wall = RigidBody::new_static();
        wall.wake_up();
        world.add_body(BodyHandle::from_id(3), wall, Transform::new(Vec3::new(r + gap + 0.05, 0.08, 0.0)), Velocity::default(), Collider::box_collider(Vec3::new(0.05, 0.08, 5.0)));
    }
    let col = Collider::sphere(r);
    let mut rb = RigidBody::new(0.08, true);
    rb.ccd_enabled = ccd;
    rb.update_inertia_from_collider(&col);
    rb.wake_up();
    world.add_body(BodyHandle::from_id(2), rb, Transform::new(Vec3::new(0.0, r, 0.0)), Velocity::new(Vec3::new(drift, 0.0, 0.0)), col);
    let ball = world.entities.iter().position(|e| e.id() == 2).unwrap();
    for _ in 0..60 {
        world.step(1.0 / 60.0).unwrap();
    }
    world.transforms[ball].position.z
}

#[test]
fn a_ccd_ball_beside_a_wall_rolls_down() {
    let free = rolled(false, None, 0.0);
    for drift in [0.02f32, 0.05, 0.2] {
        let d = rolled(true, Some(0.002), drift);
        assert!(d > free * 0.9, "a CCD ball drifting into a wall at {drift} m/s rolled {d:.3} m in a second; without CCD or wall {free:.3} m");
    }
}

#[test]
fn ccd_does_not_slow_a_rolling_ball() {
    let (plain, ccd) = (rolled(false, None, 0.0), rolled(true, None, 0.0));
    assert!(ccd > plain * 0.97, "a CCD ball rolled {ccd:.3} m, a plain one {plain:.3} m");
}
