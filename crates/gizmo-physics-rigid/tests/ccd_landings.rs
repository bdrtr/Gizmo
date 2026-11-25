//! A CCD body lands, bounces and passes by as one without CCD does.
//!
//! The speculative contacts carry a CCD body to what it falls onto and give its bounce. Until
//! 2026-11-25 a backstop also swept every CCD body's centre against each static collider's bounding
//! box and stopped it a centimetre short of the box: a ball dropped onto a 30° ramp stopped in
//! mid-air 5 m over it, at the top of the ramp's box; and a ball dropped 10 m onto a floor at 60 Hz,
//! whose speculative contact let it close the gap exactly, was stopped again at the box's face with
//! its velocity zeroed under the bounce already worked out for it — it left at 17.2 m/s where it
//! came in at 13.6, and rose to 14.8 m. Only a kinematic body, which no impulse stops, is still held
//! at static surfaces, by the speculative contact itself (`PhysicsWorld::kinematic_ccd_step`).

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

const BALL: u32 = 2;
const R: f32 = 0.1;

fn ball(ccd: bool) -> (RigidBody, Collider) {
    let shape = Collider::sphere(R);
    let mut rb = RigidBody::new(1.0, true);
    rb.update_inertia_from_collider(&shape);
    rb.ccd_enabled = ccd;
    (rb, shape)
}

/// The highest the ball's bottom rises between its first and second landings on a 20 m box floor
/// whose top is at y = 0, dropped from `height` at `hz`.
fn first_bounce(height: f32, hz: u32, ccd: bool) -> f32 {
    let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap();
    w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(Vec3::new(0.0, -0.5, 0.0)), Velocity::default(), Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)));
    let (rb, shape) = ball(ccd);
    w.add_body(BodyHandle::from_id(BALL), rb, Transform::new(Vec3::new(0.3, height + R, 0.2)), Velocity::default(), shape);
    let (mut peak, mut landed, mut rising) = (f32::MIN, false, false);
    for _ in 0..1200 {
        w.step(1.0 / 60.0).unwrap();
        let i = *w.entity_index_map.get(&BALL).unwrap();
        let (y, vy) = (w.transforms[i].position.y - R, w.velocities[i].linear.y);
        if !landed {
            landed = vy > 0.0;
            continue;
        }
        if vy > 0.0 {
            rising = true;
        } else if rising {
            return peak.max(y);
        }
        peak = peak.max(y);
    }
    panic!("dropped from {height} m at {hz} Hz (CCD {ccd}): no first bounce in 20 s");
}

/// Over a spread of heights: the old failure needed the landing to end exactly on the box's face, a
/// coincidence of rounding that one height or another meets (9.8 m did, 10 m did not).
#[test]
fn a_ccd_ball_bounces_as_high_as_one_without_ccd() {
    for hz in [60, 240] {
        for height in [1.5, 2.2, 3.0, 4.1, 5.3, 6.6, 8.0, 9.8, 12.0, 17.0, 30.0] {
            let (plain, with_ccd) = (first_bounce(height, hz, false), first_bounce(height, hz, true));
            // e = 0.3: a bounce of about e² of the drop.
            assert!(plain < 0.15 * height, "premise at {hz} Hz from {height} m: {plain}");
            assert!((with_ccd - plain).abs() <= 0.05 * plain, "{hz} Hz, dropped from {height} m: {with_ccd} m with CCD, {plain} m without");
        }
    }
}

/// A ball falling onto a ramp tilted 30° falls freely until it is at the ramp.
#[test]
fn a_ccd_ball_falls_freely_onto_a_tilted_ramp() {
    for hz in [60, 240] {
        falls_freely_onto_a_tilted_ramp(hz);
    }
}

fn falls_freely_onto_a_tilted_ramp(hz: u32) {
    let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap();
    let tilt = Quat::from_rotation_z(30f32.to_radians());
    let mut ramp = Transform::new(Vec3::new(0.0, -0.5, 0.0));
    ramp.rotation = tilt;
    w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), ramp, Velocity::default(), Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)));
    let (rb, shape) = ball(true);
    w.add_body(BodyHandle::from_id(BALL), rb, Transform::new(Vec3::new(0.0, 10.0, 0.2)), Velocity::default(), shape);
    let up = tilt * Vec3::Y;
    let over_ramp = |p: Vec3| (p - Vec3::new(0.0, -0.5, 0.0)).dot(up) - 0.5 - R;
    let g = w.integrator.gravity.y;
    for frame in 1..=120 {
        w.step(1.0 / 60.0).unwrap();
        let i = *w.entity_index_map.get(&BALL).unwrap();
        let p = w.transforms[i].position;
        if over_ramp(p) < 0.5 {
            return;
        }
        let free = g * frame as f32 / 60.0;
        let vy = w.velocities[i].linear.y;
        assert!((vy - free).abs() < 0.1, "{hz} Hz, frame {frame}, {:.2} m over the ramp: falling at {vy} m/s, free fall {free}", over_ramp(p));
    }
    panic!("{hz} Hz: never reached the ramp");
}

/// A fast kinematic body passes through a static box's bounds untouched when it misses the box:
/// a plank lying diagonally, the body crossing the empty corner of its bounding box.
#[test]
fn a_fast_kinematic_body_misses_a_static_box_it_does_not_reach() {
    for hz in [60, 240] {
        let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap().with_gravity(Vec3::ZERO);
        let mut plank = Transform::new(Vec3::ZERO);
        plank.rotation = Quat::from_rotation_z(45f32.to_radians());
        w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), plank, Velocity::default(), Collider::box_collider(Vec3::new(2.0, 0.2, 2.0)));
        // The bounds reach 1.56 m on x and y; the plank itself is 1.5 m from this line.
        let start = Vec3::new(-1.2, 1.2, -5.0);
        let v = Vec3::new(0.0, 0.0, 30.0);
        let mut rb = RigidBody::new_kinematic();
        rb.wake_up();
        w.add_body(BodyHandle::from_id(BALL), rb, Transform::new(start), Velocity::new(v), Collider::sphere(R));
        for _ in 0..20 {
            w.step(1.0 / 60.0).unwrap();
        }
        let i = *w.entity_index_map.get(&BALL).unwrap();
        let expected = start + v * (20.0 / 60.0);
        assert!((w.transforms[i].position - expected).length() < 1e-3, "{hz} Hz: at {:?}, moving freely it is at {expected:?}", w.transforms[i].position);
    }
}

/// A fast kinematic body driven into a static wall at an angle is held at the wall and slides on
/// along it.
#[test]
fn a_fast_kinematic_body_is_held_at_a_static_wall_and_slides_along_it() {
    for hz in [60, 240] {
        let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap().with_gravity(Vec3::ZERO);
        // The wall's face at x = 0.
        w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(Vec3::new(0.25, 0.0, 0.0)), Velocity::default(), Collider::box_collider(Vec3::new(0.25, 5.0, 50.0)));
        let mut rb = RigidBody::new_kinematic();
        rb.wake_up();
        w.add_body(BodyHandle::from_id(BALL), rb, Transform::new(Vec3::new(-2.0, 0.0, 0.0)), Velocity::new(Vec3::new(30.0, 0.0, 30.0)), Collider::sphere(R));
        let mut deepest = f32::MIN;
        for _ in 0..30 {
            w.step(1.0 / 60.0).unwrap();
            let i = *w.entity_index_map.get(&BALL).unwrap();
            deepest = deepest.max(w.transforms[i].position.x + R);
        }
        let i = *w.entity_index_map.get(&BALL).unwrap();
        let p = w.transforms[i].position;
        assert!(deepest <= 1e-3, "{hz} Hz: the body went {deepest} m into the wall");
        assert!(p.x + R > -0.02, "{hz} Hz: the body left the wall ({p:?})");
        assert!((p.z - 15.0).abs() < 0.05, "{hz} Hz: slid to z = {}, moving on at 30 m/s it is at 15", p.z);
    }
}

/// The ball's velocity and spin as it leaves a ramp tilted `tilt_deg`, dropped from `height`.
fn off_the_ramp(tilt_deg: f32, height: f32, hz: u32, ccd: bool) -> (Vec3, Vec3) {
    let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap();
    let tilt = Quat::from_rotation_z(tilt_deg.to_radians());
    let mut ramp = Transform::new(Vec3::new(0.0, -0.5, 0.0));
    ramp.rotation = tilt;
    w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), ramp, Velocity::default(), Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)));
    let (rb, shape) = ball(ccd);
    w.add_body(BodyHandle::from_id(BALL), rb, Transform::new(Vec3::new(0.0, height, 0.2)), Velocity::default(), shape);
    let up = tilt * Vec3::Y;
    let mut before = Vec3::ZERO;
    for _ in 0..600 {
        w.step(1.0 / 60.0).unwrap();
        let i = *w.entity_index_map.get(&BALL).unwrap();
        let v = w.velocities[i];
        if v.linear.dot(up) > 0.5 && before.dot(up) < -0.5 {
            return (v.linear, v.angular);
        }
        before = v.linear;
    }
    panic!("{tilt_deg}° from {height} m at {hz} Hz (CCD {ccd}): never left the ramp");
}

/// A CCD ball landing on a ramp leaves it as one without CCD does: turning, and sliding on at what
/// is left after friction. It kept all its speed along the ramp and no spin: its island was solved
/// on the split-impulse path, whose bounce gave the impact's normal impulse and no friction.
#[test]
fn a_ccd_ball_landing_on_a_ramp_turns_as_one_without_ccd() {
    for hz in [60, 240] {
        for (tilt, height) in [(10.0, 3.0), (30.0, 3.0)] {
            let (plain, spin_plain) = off_the_ramp(tilt, height, hz, false);
            let (with_ccd, spin_ccd) = off_the_ramp(tilt, height, hz, true);
            assert!(spin_plain.length() > 5.0, "premise at {hz} Hz, {tilt}°: the plain ball does not turn ({spin_plain:?})");
            assert!(
                (with_ccd - plain).length() <= 0.02 * plain.length() && (spin_ccd - spin_plain).length() <= 0.02 * spin_plain.length(),
                "{hz} Hz, {tilt}° from {height} m: with CCD {with_ccd:?} spinning {spin_ccd:?}, without {plain:?} spinning {spin_plain:?}"
            );
        }
    }
}
