//! A speculative contact is a constraint, not an overlap: events report pairs that touch.
//!
//! A pair found only by a speculative contact — a body that may close the gap this substep — has
//! not met yet, and may never: something else can stop it first. Its `Started` waits for the meeting:
//! for a solid pair, the solve leaning on the contact; for a trigger, the substep's motion sweeping
//! into it (`PhysicsWorld::confirm_triggers`). A CCD ball landing on a floor fired `Started` and
//! `Ended` on the kill-zone trigger 2 cm under it, which it never entered (2026-11-25).

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, CollisionEventType, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

const BALL: u32 = 2;
const UNDER: u32 = 3;

/// A 0.2 m CCD ball dropped from `height` onto a floor whose top is at y = 0, over `under` — a body
/// whose top is 2 cm below the floor's — for 4 s at `hz`. Returns the trigger and collision events
/// that name `under`, and the lowest the ball's bottom went.
fn drop_over(under: Collider, height: f32, hz: u32) -> (Vec<String>, f32) {
    let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap();
    w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(Vec3::new(0.0, -0.5, 0.0)), Velocity::default(), Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)));
    w.add_body(BodyHandle::from_id(UNDER), RigidBody::new_static(), Transform::new(Vec3::new(0.0, -1.02, 0.0)), Velocity::default(), under);
    let ball = Collider::sphere(0.1);
    let mut rb = RigidBody::new(1.0, true);
    rb.update_inertia_from_collider(&ball);
    rb.ccd_enabled = true;
    w.add_body(BodyHandle::from_id(BALL), rb, Transform::new(Vec3::new(0.3, height, 0.2)), Velocity::default(), ball);
    let (mut events, mut lowest) = (Vec::new(), f32::MAX);
    for frame in 0..240 {
        w.step(1.0 / 60.0).unwrap();
        let i = *w.entity_index_map.get(&BALL).unwrap();
        lowest = lowest.min(w.transforms[i].position.y - 0.1);
        let names_under = |a: BodyHandle, b: BodyHandle| a.id() == UNDER || b.id() == UNDER;
        events.extend(w.trigger_events().iter().filter(|e| names_under(e.trigger_entity, e.other_entity)).map(|e| format!("trigger {:?} at {frame}", e.event_type)));
        events.extend(w.collision_events().iter().filter(|e| names_under(e.entity_a, e.entity_b)).map(|e| format!("collision {:?} at {frame}", e.event_type)));
    }
    (events, lowest)
}

#[test]
fn a_ccd_ball_landing_on_a_floor_never_enters_the_kill_zone_under_it() {
    for hz in [60, 240] {
        for height in [0.5, 1.0, 3.0, 10.0] {
            let zone = Collider::box_collider(Vec3::new(10.0, 1.0, 10.0)).with_trigger(true);
            let (events, lowest) = drop_over(zone, height, hz);
            assert!(lowest > -0.02, "premise at {hz} Hz from {height} m: the ball went into the zone ({lowest})");
            assert!(events.is_empty(), "{hz} Hz, dropped from {height} m: {events:?}");
        }
    }
}

#[test]
fn a_ccd_ball_landing_on_a_floor_never_collides_with_a_body_under_it() {
    for hz in [60, 240] {
        for height in [1.0, 3.0, 10.0] {
            let (events, lowest) = drop_over(Collider::box_collider(Vec3::new(10.0, 1.0, 10.0)), height, hz);
            assert!(lowest > -0.02, "premise at {hz} Hz from {height} m: the ball reached the body ({lowest})");
            assert!(events.is_empty(), "{hz} Hz, dropped from {height} m: {events:?}");
        }
    }
}

/// A trigger thinner than a substep's travel is still entered: once, and left once.
#[test]
fn a_fast_ball_through_a_thin_trigger_starts_and_ends_once() {
    for hz in [60, 240] {
        for speed in [5.0, 20.0, 80.0, 160.0] {
            for ccd in [false, true] {
                let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap().with_gravity(Vec3::ZERO);
                w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(Vec3::ZERO), Velocity::default(), Collider::box_collider(Vec3::new(1.0, 0.05, 1.0)).with_trigger(true));
                let ball = Collider::sphere(0.05);
                let mut rb = RigidBody::new(1.0, true);
                rb.update_inertia_from_collider(&ball);
                rb.ccd_enabled = ccd;
                w.add_body(BodyHandle::from_id(BALL), rb, Transform::new(Vec3::new(0.1, 3.0, 0.2)), Velocity::new(Vec3::new(0.0, -speed, 0.0)), ball);
                let (mut started, mut ended) = (0, 0);
                for _ in 0..120 {
                    w.step(1.0 / 60.0).unwrap();
                    for e in w.trigger_events() {
                        match e.event_type {
                            CollisionEventType::Started => started += 1,
                            CollisionEventType::Ended => ended += 1,
                            _ => {}
                        }
                    }
                }
                assert_eq!((started, ended), (1, 1), "{hz} Hz, {speed} m/s, CCD {ccd}");
            }
        }
    }
}
