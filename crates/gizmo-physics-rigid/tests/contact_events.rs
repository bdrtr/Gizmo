//! A solid contact's events: `Started` when it begins and `Ended` when it ends, and `Persisting` on
//! every substep in between only when the world asks for it (`persisting_contact_events`).

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, CollisionEventType, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

/// A box dropped onto the ground from just above it, left to rest for half a second at 240 Hz, then
/// lifted away: every event the pair gave, in order.
fn drop_rest_lift(persisting: bool) -> Vec<CollisionEventType> {
    let mut w = PhysicsWorld::new();
    w.persisting_contact_events = persisting;
    w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(Vec3::new(0.0, -0.5, 0.0)), Velocity::default(), Collider::box_collider(Vec3::new(5.0, 0.5, 5.0)));
    let mut rb = RigidBody::new(1.0, true);
    let col = Collider::box_collider(Vec3::splat(0.25));
    rb.update_inertia_from_collider(&col);
    w.add_body(BodyHandle::from_id(2), rb, Transform::new(Vec3::new(0.0, 0.26, 0.0)), Velocity::default(), col);
    let mut events = Vec::new();
    for frame in 0..60 {
        if frame == 30 {
            w.rigid_bodies[1].wake_up();
            w.transforms[1].position.y = 3.0;
        }
        w.step(1.0 / 60.0).expect("step");
        events.extend(w.collision_events().iter().map(|e| e.event_type));
    }
    events
}

#[test]
fn a_resting_contact_reports_its_start_and_its_end_and_nothing_between() {
    let events = drop_rest_lift(false);
    assert_eq!(events, [CollisionEventType::Started, CollisionEventType::Ended], "{events:?}");
}

/// With `persisting_contact_events` on, the same contact reports `Persisting` on every substep it
/// lasts awake, as it always did before the flag.
#[test]
fn asked_for_a_contact_going_on_reports_it_every_substep() {
    let events = drop_rest_lift(true);
    assert_eq!(events.first(), Some(&CollisionEventType::Started));
    assert_eq!(events.last(), Some(&CollisionEventType::Ended));
    // Until the box sleeps, a quarter of a second in: a dormant pair is not tested, and says nothing.
    let going_on = events.iter().filter(|e| **e == CollisionEventType::Persisting).count();
    assert!(going_on >= 40, "a contact of about a quarter of a second awake at 240 Hz: {going_on} Persisting events");
    assert!(events[1..events.len() - 1].iter().all(|e| *e == CollisionEventType::Persisting), "{events:?}");
}
