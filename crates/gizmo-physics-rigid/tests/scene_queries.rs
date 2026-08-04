//! Scene query behaviour: filtering, overlap, shape casting, point queries.
//!
//! These are the queries gameplay code runs after the step — character movement, hitboxes,
//! placement checks, line of sight. Before 0.10 the public surface was three unfilterable
//! raycasts, so most of what is asserted here was simply not expressible.

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::components::{Collider, Transform};
use gizmo_physics_core::raycast::Ray;
use gizmo_physics_core::{BodyHandle, BoxShape, ColliderShape, SphereShape};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::{PhysicsWorld, QueryFilter};

/// A world with one static box of half-extent 0.5 at the origin.
fn world_with_box_at(positions: &[Vec3]) -> (PhysicsWorld, Vec<BodyHandle>) {
    let mut w = PhysicsWorld::new();
    let mut handles = Vec::new();
    for (n, p) in positions.iter().enumerate() {
        let h = BodyHandle::from_id(n as u32);
        w.add_body(
            h,
            RigidBody::new_static(),
            Transform::new(*p),
            Velocity::default(),
            Collider::box_collider(Vec3::splat(0.5)),
        );
        handles.push(h);
    }
    // Queries read the broadphase, which is (re)built during the step.
    w.step(1.0 / 60.0).expect("step");
    (w, handles)
}

fn unit_box() -> ColliderShape {
    ColliderShape::Box(BoxShape {
        half_extents: Vec3::splat(0.5),
    })
}

// ───────────────────────── overlap ─────────────────────────

#[test]
fn overlap_shape_finds_the_bodies_it_intersects_and_not_the_others() {
    let (w, h) = world_with_box_at(&[Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0)]);

    let near = w.overlap_shape(&unit_box(), Vec3::new(0.4, 0.0, 0.0), Quat::IDENTITY, QueryFilter::default());
    assert_eq!(near, vec![h[0]], "a box straddling the origin must find only the body there");

    let far = w.overlap_shape(&unit_box(), Vec3::new(5.2, 0.0, 0.0), Quat::IDENTITY, QueryFilter::default());
    assert_eq!(far, vec![h[1]]);

    let between = w.overlap_shape(&unit_box(), Vec3::new(2.5, 0.0, 0.0), Quat::IDENTITY, QueryFilter::default());
    assert!(between.is_empty(), "empty space must report nothing, got {between:?}");
}

/// The placement check every builder game needs: does this footprint fit?
#[test]
fn overlap_answers_whether_a_footprint_is_clear() {
    let (w, _) = world_with_box_at(&[Vec3::ZERO]);
    let big = ColliderShape::Box(BoxShape { half_extents: Vec3::new(2.0, 0.5, 2.0) });

    assert!(
        !w.overlap_shape(&big, Vec3::new(1.5, 0.0, 0.0), Quat::IDENTITY, QueryFilter::default()).is_empty(),
        "a 4x4 footprint centred 1.5 away from a 1x1 box must overlap it"
    );
    assert!(
        w.overlap_shape(&big, Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY, QueryFilter::default()).is_empty(),
        "the same footprint far away must be clear"
    );
}

// ───────────────────────── filtering ─────────────────────────

#[test]
fn exclusion_skips_the_named_bodies() {
    let (w, h) = world_with_box_at(&[Vec3::ZERO]);

    let unfiltered = w.overlap_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, QueryFilter::default());
    assert_eq!(unfiltered, vec![h[0]]);

    let excluded = w.overlap_shape(
        &unit_box(),
        Vec3::ZERO,
        Quat::IDENTITY,
        QueryFilter::default().excluding(&h[..1]),
    );
    assert!(excluded.is_empty(), "the excluded body must not be reported");
}

#[test]
fn a_layer_mask_selects_which_colliders_are_visible() {
    let mut w = PhysicsWorld::new();
    let on_layer_1 = BodyHandle::from_id(1);
    let on_layer_2 = BodyHandle::from_id(2);

    let mut c1 = Collider::box_collider(Vec3::splat(0.5));
    c1.collision_layer = gizmo_physics_core::components::CollisionLayer::new(1);
    let mut c2 = Collider::box_collider(Vec3::splat(0.5));
    c2.collision_layer = gizmo_physics_core::components::CollisionLayer::new(2);

    w.add_body(on_layer_1, RigidBody::new_static(), Transform::new(Vec3::ZERO), Velocity::default(), c1);
    w.add_body(on_layer_2, RigidBody::new_static(), Transform::new(Vec3::new(0.2, 0.0, 0.0)), Velocity::default(), c2);
    w.step(1.0 / 60.0).unwrap();

    let both = w.overlap_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, QueryFilter::default());
    assert_eq!(both.len(), 2, "an unmasked query sees both layers");

    let only_1 = w.overlap_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, QueryFilter::default().on_layer(1));
    assert_eq!(only_1, vec![on_layer_1]);

    let only_2 = w.overlap_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, QueryFilter::default().on_layer(2));
    assert_eq!(only_2, vec![on_layer_2]);
}

/// Triggers are invisible by default. A movement query that stopped on a trigger volume
/// would be a bug; an "what am I standing in" query wants exactly them.
#[test]
fn triggers_are_skipped_unless_asked_for() {
    let mut w = PhysicsWorld::new();
    let trigger = BodyHandle::from_id(1);
    let mut c = Collider::box_collider(Vec3::splat(0.5));
    c.is_trigger = true;
    w.add_body(trigger, RigidBody::new_static(), Transform::new(Vec3::ZERO), Velocity::default(), c);
    w.step(1.0 / 60.0).unwrap();

    assert!(
        w.overlap_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, QueryFilter::default()).is_empty(),
        "the default filter must not report trigger volumes"
    );
    assert_eq!(
        w.overlap_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, QueryFilter::default().with_triggers()),
        vec![trigger],
        "with_triggers() must report them"
    );
}

/// The filtered raycast exists because post-filtering cannot substitute for it: `raycast`
/// returns only the closest hit, so discarding it afterwards loses whatever was behind.
#[test]
fn a_filtered_raycast_sees_past_an_excluded_body() {
    let (w, h) = world_with_box_at(&[Vec3::new(1.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 0.0)]);
    let ray = Ray::new(Vec3::new(-2.0, 0.0, 0.0), Vec3::X);

    let closest = w.raycast(&ray, 20.0).expect("something is in the way");
    assert_eq!(closest.entity, h[0], "the unfiltered ray stops at the nearer box");

    let past = w
        .raycast_filtered(&ray, 20.0, QueryFilter::default().excluding(&h[..1]))
        .expect("the far box is still there");
    assert_eq!(
        past.entity, h[1],
        "excluding the near body must reveal the one behind it, not return nothing"
    );
    assert!(past.distance > closest.distance);
}

// ───────────────────────── point query ─────────────────────────

#[test]
fn point_query_reports_containment() {
    let (w, h) = world_with_box_at(&[Vec3::ZERO]);

    assert_eq!(
        w.point_query(Vec3::new(0.1, 0.1, 0.1), QueryFilter::default()),
        vec![h[0]],
        "a point inside the box must report it"
    );
    assert!(
        w.point_query(Vec3::new(3.0, 0.0, 0.0), QueryFilter::default()).is_empty(),
        "a point in empty space must report nothing"
    );
}

// ───────────────────────── shape cast ─────────────────────────

/// The query a character controller needs and could not previously express. A ray has no
/// volume, so sweeping a capsule meant centre-line rays that walk through anything the
/// centre line misses.
#[test]
fn cast_shape_stops_at_the_first_body_in_the_way() {
    let (w, h) = world_with_box_at(&[Vec3::new(5.0, 0.0, 0.0)]);

    let hit = w
        .cast_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, Vec3::X, 10.0, QueryFilter::default())
        .expect("the box is directly ahead");
    assert_eq!(hit.entity, h[0]);
    // Two half-extents of 0.5 touch when the centres are 1.0 apart, so the sweep travels ~4.
    assert!(
        (hit.distance - 4.0).abs() < 0.2,
        "expected to travel about 4 units before contact, got {}",
        hit.distance
    );
}

/// A ray offset from the box's centre line misses it; a swept box of the same offset does
/// not. This is precisely the failure mode of the built-in character controller's
/// three-centre-line-rays approach.
#[test]
fn a_swept_volume_hits_what_a_centre_ray_would_miss() {
    let (w, h) = world_with_box_at(&[Vec3::new(5.0, 0.0, 0.0)]);

    // Offset by 0.9 in Z: outside the box's 0.5 half-extent, so a ray misses.
    let origin = Vec3::new(0.0, 0.0, 0.9);
    let ray = Ray::new(origin, Vec3::X);
    assert!(
        w.raycast(&ray, 20.0).is_none(),
        "a ray at this offset must miss the box entirely"
    );

    // The same path swept with a 0.5-half-extent box reaches it (0.9 < 0.5 + 0.5).
    let hit = w
        .cast_shape(&unit_box(), origin, Quat::IDENTITY, Vec3::X, 20.0, QueryFilter::default())
        .expect("the swept volume is wide enough to clip the box");
    assert_eq!(hit.entity, h[0]);
}

#[test]
fn cast_shape_reports_nothing_when_the_path_is_clear() {
    let (w, _) = world_with_box_at(&[Vec3::new(5.0, 0.0, 0.0)]);
    assert!(
        w.cast_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, Vec3::Y, 10.0, QueryFilter::default())
            .is_none(),
        "sweeping straight up must not find the box beside us"
    );
}

/// Starting already overlapping is reported at distance zero rather than sweeping past,
/// so a caller can detect "spawned inside geometry" instead of tunnelling out of it.
#[test]
fn a_cast_that_starts_overlapping_reports_zero_distance() {
    let (w, h) = world_with_box_at(&[Vec3::ZERO]);

    let hit = w
        .cast_shape(&unit_box(), Vec3::new(0.2, 0.0, 0.0), Quat::IDENTITY, Vec3::X, 10.0, QueryFilter::default())
        .expect("we start inside the box");
    assert_eq!(hit.entity, h[0]);
    assert_eq!(hit.distance, 0.0);
}

#[test]
fn degenerate_casts_are_rejected_rather_than_guessed_at() {
    let (w, _) = world_with_box_at(&[Vec3::new(5.0, 0.0, 0.0)]);
    let f = QueryFilter::default();

    assert!(w.cast_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, Vec3::ZERO, 10.0, f).is_none(), "zero direction");
    assert!(w.cast_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, Vec3::X, 0.0, f).is_none(), "zero distance");
    assert!(w.cast_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, Vec3::X, -1.0, f).is_none(), "negative distance");
}

/// `cast_body` sweeps a body's own collider and must not immediately hit itself.
#[test]
fn cast_body_excludes_the_caster() {
    let (w, h) = world_with_box_at(&[Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0)]);

    let hit = w
        .cast_body(h[0], Vec3::X, 10.0, QueryFilter::default())
        .expect("the other box is ahead");
    assert_eq!(
        hit.entity, h[1],
        "sweeping a body must skip itself, or every cast returns distance 0 on the caster"
    );
    assert!(hit.distance > 0.0);
}

/// A sphere probe against a sphere body — exercises a different narrowphase path than the
/// box-box one every other test here takes.
#[test]
fn overlap_works_for_shape_pairs_other_than_box_box() {
    let mut w = PhysicsWorld::new();
    let ball = BodyHandle::from_id(1);
    w.add_body(
        ball,
        RigidBody::new_static(),
        Transform::new(Vec3::ZERO),
        Velocity::default(),
        Collider::from_shape(ColliderShape::Sphere(SphereShape { radius: 1.0 })),
    );
    w.step(1.0 / 60.0).unwrap();

    let probe = ColliderShape::Sphere(SphereShape { radius: 0.25 });
    assert_eq!(
        w.overlap_shape(&probe, Vec3::new(1.1, 0.0, 0.0), Quat::IDENTITY, QueryFilter::default()),
        vec![ball],
        "radii 1.0 + 0.25 overlap at centre distance 1.1"
    );
    assert!(
        w.overlap_shape(&probe, Vec3::new(2.0, 0.0, 0.0), Quat::IDENTITY, QueryFilter::default()).is_empty(),
        "and separate at 2.0"
    );
}

/// The sweep must not step over geometry thinner than a naive fixed subdivision would use.
/// The march step is derived from the smallest extent involved for exactly this reason.
#[test]
fn a_thin_wall_is_not_stepped_over_by_a_long_sweep() {
    let mut w = PhysicsWorld::new();
    let wall = BodyHandle::from_id(1);
    // 2 cm thick, across a 200 unit sweep — a 32- or even 512-sample fixed march would miss it.
    w.add_body(
        wall,
        RigidBody::new_static(),
        Transform::new(Vec3::new(100.0, 0.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(0.01, 5.0, 5.0)),
    );
    w.step(1.0 / 60.0).unwrap();

    let probe = ColliderShape::Sphere(SphereShape { radius: 0.05 });
    let hit = w
        .cast_shape(&probe, Vec3::ZERO, Quat::IDENTITY, Vec3::X, 200.0, QueryFilter::default())
        .expect("the wall is directly in the path and must be found");
    assert_eq!(hit.entity, wall);
    assert!(
        (hit.distance - 99.94).abs() < 0.2,
        "should stop just before the wall face at x≈99.94, got {}",
        hit.distance
    );
}

/// The returned distance must be the *first* contact, not any later one, when several
/// bodies lie along the path.
#[test]
fn cast_shape_returns_the_nearest_of_several_bodies_in_the_path() {
    let (w, h) = world_with_box_at(&[
        Vec3::new(9.0, 0.0, 0.0),
        Vec3::new(3.0, 0.0, 0.0),
        Vec3::new(6.0, 0.0, 0.0),
    ]);

    let hit = w
        .cast_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, Vec3::X, 20.0, QueryFilter::default())
        .expect("three boxes ahead");
    assert_eq!(
        hit.entity, h[1],
        "must report the box at x=3, the nearest, regardless of insertion order"
    );
    assert!((hit.distance - 2.0).abs() < 0.2, "got {}", hit.distance);
}

/// Filters apply to sweeps as they do to every other query.
#[test]
fn a_shape_cast_honours_the_layer_mask() {
    let mut w = PhysicsWorld::new();
    let blocker = BodyHandle::from_id(1);
    let mut c = Collider::box_collider(Vec3::splat(0.5));
    c.collision_layer = gizmo_physics_core::components::CollisionLayer::new(3);
    w.add_body(blocker, RigidBody::new_static(), Transform::new(Vec3::new(5.0, 0.0, 0.0)), Velocity::default(), c);
    w.step(1.0 / 60.0).unwrap();

    assert!(
        w.cast_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, Vec3::X, 20.0, QueryFilter::default()).is_some(),
        "an unmasked sweep finds it"
    );
    assert!(
        w.cast_shape(&unit_box(), Vec3::ZERO, Quat::IDENTITY, Vec3::X, 20.0, QueryFilter::default().on_layer(7))
            .is_none(),
        "a sweep masked to a different layer must not"
    );
}
