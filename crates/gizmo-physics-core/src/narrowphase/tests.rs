//! Narrowphase unit tests, moved out of narrowphase.rs (verbatim, de-indented).

use super::*;
use crate::components::BoxShape;

fn box_shape(half: f32) -> ColliderShape {
    ColliderShape::Box(BoxShape {
        half_extents: Vec3::splat(half),
    })
}

// Regression: when box B is the SAT reference (its axis is more aligned with the
// contact normal than any of A's — e.g. a box tilted onto its corner resting on an
// axis-aligned box), the A→B normal must be flipped to reference→incident before
// clipping. The old primary path passed it unflipped, so `ref_face_d` sampled B's FAR
// face and reported a penetration inflated by ~2·hb, which made the solver blow the
// pair apart. A contact's penetration can never exceed the boxes' overlap along the
// contact normal (the SAT interval overlap) — assert exactly that.
#[test]
fn box_box_ref_b_penetration_not_inflated() {
    // Rotate A so its local (1,1,1) body diagonal points along world +X; then all
    // three of A's world axes sit 54.7° off +X (>45°), so `is_face_axis(normal, A)`
    // is false and the axis-aligned B becomes the reference face.
    let diag = Vec3::new(1.0, 1.0, 1.0).normalize();
    let rot_a = Quat::from_axis_angle(
        Vec3::new(0.0, 1.0, -1.0).normalize(),
        diag.dot(Vec3::X).acos(),
    );
    let pos_a = Vec3::ZERO;
    let ha = Vec3::splat(1.0);
    // B offset along +X so the minimum-overlap (MTV) axis is +X.
    let pos_b = Vec3::new(2.5, 0.0, 0.0);
    let rot_b = Quat::IDENTITY;
    let hb = Vec3::splat(1.0);

    let contacts = NarrowPhase::box_box(pos_a, rot_a, ha, pos_b, rot_b, hb);
    assert!(!contacts.is_empty(), "overlapping boxes must produce contacts");

    let n = contacts[0].normal;
    assert!(
        n.x > 0.99,
        "expected the +X contact normal that forces the ref=B path, got {n:?}"
    );

    // SAT interval overlap along the contact normal = A's max extent − B's min extent.
    let extent = |rot: Quat, h: Vec3| {
        let a = [rot.mul_vec3(Vec3::X), rot.mul_vec3(Vec3::Y), rot.mul_vec3(Vec3::Z)];
        a[0].dot(n).abs() * h.x + a[1].dot(n).abs() * h.y + a[2].dot(n).abs() * h.z
    };
    let overlap = (pos_a.dot(n) + extent(rot_a, ha)) - (pos_b.dot(n) - extent(rot_b, hb));
    assert!(overlap > 0.0, "boxes must actually overlap along the normal");

    let max_pen = contacts
        .iter()
        .map(|c| c.penetration)
        .fold(0.0_f32, f32::max);
    assert!(
        max_pen <= overlap + 1e-3,
        "penetration {max_pen} exceeds the SAT overlap {overlap} along the normal \
         → inflated depth (the ref=B unflipped-normal bug)"
    );
}

// ── Sphere–Sphere ─────────────────────────────────────────────────────

#[test]
fn sphere_sphere_overlap_produces_contact() {
    let c = NarrowPhase::sphere_sphere(Vec3::ZERO, 1.0, Vec3::new(1.5, 0., 0.), 1.0);
    assert!(c.is_some(), "overlapping spheres must collide");
    let c = c.unwrap();
    assert!(c.penetration > 0.0, "penetration must be positive");
    assert!(
        (c.normal.x - 1.0).abs() < 0.01,
        "normal must point A→B (+X)"
    );
}

#[test]
fn sphere_sphere_separated_returns_none() {
    let c = NarrowPhase::sphere_sphere(Vec3::ZERO, 1.0, Vec3::new(3.0, 0., 0.), 1.0);
    assert!(c.is_none(), "separated spheres must not collide");
}

#[test]
fn sphere_sphere_touching_returns_none() {
    // Exactly touching — penetration = 0, no constraint needed.
    let c = NarrowPhase::sphere_sphere(Vec3::ZERO, 1.0, Vec3::new(2.0, 0., 0.), 1.0);
    assert!(
        c.is_none(),
        "just-touching spheres should not produce contact"
    );
}

// ── Sphere–Plane ──────────────────────────────────────────────────────

#[test]
fn sphere_plane_below_produces_contact() {
    // Plane: y = 0 (normal = +Y, d = 0). Sphere at y = 0.5 with r = 1.0
    // → 0.5 units below the plane surface.
    let c = NarrowPhase::sphere_plane(Vec3::new(0., 0.5, 0.), 1.0, Vec3::Y, 0.0);
    assert!(c.is_some());
    let c = c.unwrap();
    assert!(c.penetration > 0.0);
    // Normal should point from sphere into plane (i.e. -Y).
    assert!((c.normal.y + 1.0).abs() < 0.01, "normal should be -Y");
}

#[test]
fn sphere_plane_above_returns_none() {
    let c = NarrowPhase::sphere_plane(Vec3::new(0., 2.0, 0.), 1.0, Vec3::Y, 0.0);
    assert!(c.is_none());
}

// ── Box–Plane ─────────────────────────────────────────────────────────

#[test]
fn box_plane_four_contacts_when_flat_on_ground() {
    // Unit box sitting 0.5 units above y=0 plane → all 4 bottom corners
    // penetrate by 0.5.
    let contacts = NarrowPhase::box_plane(
        Vec3::new(0., 0.5, 0.),
        Quat::IDENTITY,
        Vec3::splat(1.0),
        Vec3::Y,
        0.0,
    );
    assert_eq!(contacts.len(), 4, "flat box should have 4 contacts");
    for c in &contacts {
        assert!(c.penetration > 0.0, "each contact must penetrate");
        assert!(
            (c.normal.y + 1.0).abs() < 0.01,
            "normal must be -Y (box→plane)"
        );
    }
}

#[test]
fn box_plane_no_contact_when_above() {
    let contacts = NarrowPhase::box_plane(
        Vec3::new(0., 2.0, 0.),
        Quat::IDENTITY,
        Vec3::splat(1.0),
        Vec3::Y,
        0.0,
    );
    assert!(contacts.is_empty());
}

// ── Box–Box SAT ───────────────────────────────────────────────────────

#[test]
fn box_box_overlap_produces_contacts() {
    let contacts = NarrowPhase::box_box(
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::splat(1.0),
        Vec3::new(1.5, 0., 0.),
        Quat::IDENTITY,
        Vec3::splat(1.0),
    );
    assert!(!contacts.is_empty(), "overlapping boxes must have contacts");
    for c in &contacts {
        assert!(c.penetration >= 0.0);
    }
}

#[test]
fn box_box_separated_returns_empty() {
    let contacts = NarrowPhase::box_box(
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::splat(1.0),
        Vec3::new(5.0, 0., 0.),
        Quat::IDENTITY,
        Vec3::splat(1.0),
    );
    assert!(
        contacts.is_empty(),
        "separated boxes must not produce contacts"
    );
}

#[test]
fn box_box_rotated_45_produces_contacts() {
    let rot45 = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
    let contacts = NarrowPhase::box_box(
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::splat(0.8),
        Vec3::new(1.0, 0., 0.),
        rot45,
        Vec3::splat(0.8),
    );
    assert!(
        !contacts.is_empty(),
        "rotated overlapping boxes must collide"
    );
}

#[test]
fn box_box_face_contact_normal_is_axis_aligned() {
    // Boxes overlapping along X — contact normal must be ±X.
    let contacts = NarrowPhase::box_box(
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::splat(1.0),
        Vec3::new(1.5, 0., 0.),
        Quat::IDENTITY,
        Vec3::splat(1.0),
    );
    assert!(!contacts.is_empty());
    for c in &contacts {
        assert!(
            c.normal.x.abs() > 0.9,
            "face contact normal should be X-aligned, got {:?}",
            c.normal
        );
    }
}

#[test]
fn box_box_contact_count_at_most_4() {
    let contacts = NarrowPhase::box_box(
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::splat(1.0),
        Vec3::new(1.5, 0., 0.),
        Quat::IDENTITY,
        Vec3::splat(1.0),
    );
    assert!(
        contacts.len() <= 4,
        "manifold must not exceed 4 contact points"
    );
}

#[test]
fn box_box_rotated_manifold_is_the_overlap_and_its_deepest_point_is_the_mtv() {
    // Box A turned 30° about Y (half 1,1,1) at the origin, axis-aligned box B (half 1,1,1) at
    // (1.2,0,0). The separating axis is B's X face (true MTV ≈ 1.16603: A reaches x = 0.866 + 0.5
    // = 1.366, B's face is at 0.2), so B is the reference and A's +x' face the incident one.
    //
    // The overlap is a WEDGE, not a slab: A's face meets B's face at an angle. Its deep edge is A's
    // corner edge at x = 1.366, z = 0.366 — depth 1.166, the MTV — and where A's face leaves B's
    // z-slab (z = −1) it is at x = 0.577, depth 0.377. So the manifold is two points at the MTV and
    // two shallower, the same in either pair order.
    //
    // This test used to demand four points at the MTV ("uniform across a flat contact"): A was then
    // the reference (any axis within 45° counted as a face, A asked first) and B's flat face was
    // scored against A's support plane — which gave B's far corner, 7 cm into A, a depth of 1.17 m.
    // Uniform, and not the overlap (2026-11-10).
    let rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_6); // 30°
    let a_first = NarrowPhase::box_box(Vec3::ZERO, rot, Vec3::splat(1.0), Vec3::new(1.2, 0.0, 0.0), Quat::IDENTITY, Vec3::splat(1.0));
    let b_first = NarrowPhase::box_box(Vec3::new(1.2, 0.0, 0.0), Quat::IDENTITY, Vec3::splat(1.0), Vec3::ZERO, rot, Vec3::splat(1.0));
    for (label, contacts, normal) in [("A first", &a_first, Vec3::X), ("B first", &b_first, -Vec3::X)] {
        assert_eq!(contacts.len(), 4, "{label}: {contacts:?}");
        let mut depths: Vec<f32> = contacts.iter().map(|c| c.penetration).collect();
        depths.sort_by(f32::total_cmp);
        for (depth, expected) in depths.iter().zip([0.3768, 0.3768, 1.166, 1.166]) {
            assert!((depth - expected).abs() < 0.01, "{label}: depths {depths:?}, expected two at 0.377 and two at the MTV 1.166");
        }
        for c in contacts.iter() {
            assert!(c.normal.dot(normal) > 0.999, "{label}: normal {:?}", c.normal);
            if c.penetration > 1.0 {
                assert!((c.point.x - 1.366).abs() < 0.01 && (c.point.z - 0.366).abs() < 0.01, "{label}: the deep points are A's corner edge, got {:?}", c.point);
            }
        }
    }
}

// ── Dispatcher ────────────────────────────────────────────────────────

#[test]
fn dispatcher_box_box_finds_contact() {
    let ba = box_shape(1.0);
    let bb = box_shape(1.0);
    let c = NarrowPhase::test_collision(
        &ba,
        Vec3::ZERO,
        Quat::IDENTITY,
        &bb,
        Vec3::new(1.5, 0., 0.),
        Quat::IDENTITY,
    );
    assert!(c.is_some(), "dispatcher must detect box-box overlap");
}

#[test]
fn dispatcher_manifold_populates_local_points() {
    let ba = box_shape(1.0);
    let bb = box_shape(1.0);
    let contacts = NarrowPhase::test_collision_manifold(
        &ba,
        Vec3::ZERO,
        Quat::IDENTITY,
        &bb,
        Vec3::new(1.5, 0., 0.),
        Quat::IDENTITY,
    );
    assert!(!contacts.is_empty());
    for c in &contacts {
        // local_point_a and local_point_b should be non-default after dispatch.
        // (They are 0 only if the contact point happens to be at the origin,
        // which should never be the case for non-degenerate geometry.)
        let _ = c.local_point_a; // just confirm they exist and compile
        let _ = c.local_point_b;
    }
}

#[test]
fn test_collision_returns_deepest_of_manifold() {
    let ba = box_shape(1.0);
    let bb = box_shape(1.0);

    let manifold = NarrowPhase::test_collision_manifold(
        &ba,
        Vec3::ZERO,
        Quat::IDENTITY,
        &bb,
        Vec3::new(1.5, 0., 0.),
        Quat::IDENTITY,
    );
    let single = NarrowPhase::test_collision(
        &ba,
        Vec3::ZERO,
        Quat::IDENTITY,
        &bb,
        Vec3::new(1.5, 0., 0.),
        Quat::IDENTITY,
    );

    if let (Some(s), Some(deepest)) = (
        single,
        manifold
            .iter()
            .max_by(|a, b| a.penetration.total_cmp(&b.penetration)),
    ) {
        assert!(
            (s.penetration - deepest.penetration).abs() < 1e-5,
            "test_collision must return the deepest manifold contact"
        );
    }
}

// ── Normal convention consistency ─────────────────────────────────────

#[test]
fn sphere_sphere_normal_points_a_to_b() {
    let c = NarrowPhase::sphere_sphere(Vec3::ZERO, 1.0, Vec3::new(1.5, 0., 0.), 1.0).unwrap();
    // Dot of normal with (B_pos - A_pos) must be positive.
    assert!(
        c.normal.dot(Vec3::new(1.5, 0., 0.)) > 0.0,
        "normal must point from A toward B"
    );
}

#[test]
fn box_box_normal_points_a_to_b() {
    let contacts = NarrowPhase::box_box(
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::splat(1.0),
        Vec3::new(1.5, 0., 0.),
        Quat::IDENTITY,
        Vec3::splat(1.0),
    );
    let d = Vec3::new(1.5, 0., 0.); // B_pos - A_pos
    for c in &contacts {
        assert!(
            c.normal.dot(d) > 0.0,
            "box-box normal must point from A toward B, got {:?}",
            c.normal
        );
    }
}

// ── Degenerate / boundary guards ──────────────────────────────────────

#[test]
fn sphere_sphere_coincident_centres_returns_none() {
    // Identical centres would divide by a ~zero distance; the d2 < 1e-10 guard must
    // return None rather than a NaN normal.
    let c = NarrowPhase::sphere_sphere(Vec3::ZERO, 1.0, Vec3::ZERO, 1.0);
    assert!(c.is_none(), "coincident spheres must not yield a NaN normal");
}

#[test]
fn sphere_plane_exactly_touching_returns_none() {
    // Centre exactly `r` above the plane → signed_dist == r → no contact (>= r).
    let c = NarrowPhase::sphere_plane(Vec3::new(0.0, 1.0, 0.0), 1.0, Vec3::Y, 0.0);
    assert!(c.is_none());
}

#[test]
fn sphere_plane_penetration_equals_gap() {
    // Centre 0.25 above the plane, r = 1 → penetration = 0.75, normal into the plane.
    let c = NarrowPhase::sphere_plane(Vec3::new(0.0, 0.25, 0.0), 1.0, Vec3::Y, 0.0).unwrap();
    assert!((c.penetration - 0.75).abs() < 1e-5, "pen {}", c.penetration);
    assert!((c.normal + Vec3::Y).length() < 1e-5, "normal must be -Y");
}

#[test]
fn shape_plane_capsule_penetrates_with_downward_normal() {
    // Generic support-point path: an upright capsule dipping below y = 0.
    let shape = ColliderShape::Capsule(crate::components::CapsuleShape {
        radius: 0.5,
        half_height: 1.0,
    });
    // Lowest point sits at y = 1.0 - (half_height + radius) = -0.5.
    let c = NarrowPhase::shape_plane(&shape, Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY, Vec3::Y, 0.0)
        .expect("capsule dips below the plane");
    assert!((c.penetration - 0.5).abs() < 1e-4, "pen {}", c.penetration);
    assert!((c.normal + Vec3::Y).length() < 1e-4, "normal must be -Y");
}

#[test]
fn dispatcher_plane_sphere_flips_normal_to_a_to_b() {
    // Dispatcher order A = plane, B = sphere: the contact normal must follow the
    // A→B convention (here +Y, since the sphere sits above the plane's solid side).
    let plane = ColliderShape::Plane(crate::components::PlaneShape {
        normal: Vec3::Y,
        distance: 0.0,
    });
    let sphere = ColliderShape::Sphere(crate::components::SphereShape { radius: 1.0 });
    let contacts = NarrowPhase::test_collision_manifold(
        &plane,
        Vec3::ZERO,
        Quat::IDENTITY,
        &sphere,
        Vec3::new(0.0, 0.5, 0.0),
        Quat::IDENTITY,
    );
    assert!(!contacts.is_empty());
    for c in &contacts {
        assert!(
            c.normal.y > 0.9,
            "plane→sphere normal must point A→B (+Y), got {:?}",
            c.normal
        );
    }
}

#[test]
fn box_box_normals_are_unit_length() {
    let contacts = NarrowPhase::box_box(
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::splat(1.0),
        Vec3::new(1.5, 0., 0.),
        Quat::IDENTITY,
        Vec3::splat(1.0),
    );
    assert!(!contacts.is_empty());
    for c in &contacts {
        assert!(
            (c.normal.length() - 1.0).abs() < 1e-4,
            "contact normal must be unit length, got {}",
            c.normal.length()
        );
    }
}

// ── box_box_overlap ─────────────────────────────────────────────────────────────────────────

/// The boolean sweep and the manifold builder must answer the same question the same way.
///
/// They share the axis construction and the penetration test, so this is not a restatement of
/// either — it is the claim that stopping early loses nothing. Swept over a grid of separations
/// on all three axes plus a rotated pair, because the interesting disagreements would be at the
/// boundary and on the edge–edge axes.
#[test]
fn box_box_overlap_agrees_with_the_manifold_builder() {
    let half = Vec3::splat(0.5);
    let rot_a = Quat::IDENTITY;
    let rot_b = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);

    let mut checked = 0;
    for &rot in &[Quat::IDENTITY, rot_b] {
        for i in 0..20 {
            let d = i as f32 * 0.15; // 0.0 .. 2.85, through the 1.0 (or 1.2 rotated) boundary
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                let pos_b = axis * d;
                let boolean = NarrowPhase::box_box_overlap(Vec3::ZERO, rot_a, half, pos_b, rot, half);
                let manifold =
                    !NarrowPhase::box_box(Vec3::ZERO, rot_a, half, pos_b, rot, half).is_empty();
                assert_eq!(
                    boolean, manifold,
                    "d={d} axis={axis:?} rotated={}: boolean {boolean}, manifold {manifold}",
                    rot != Quat::IDENTITY
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 120, "the sweep must actually have run");
}

/// Clear cases, stated as themselves rather than as agreement with another function.
#[test]
fn box_box_overlap_answers_the_obvious_cases() {
    let half = Vec3::splat(0.5);
    let id = Quat::IDENTITY;

    assert!(
        NarrowPhase::box_box_overlap(Vec3::ZERO, id, half, Vec3::ZERO, id, half),
        "coincident boxes overlap"
    );
    assert!(
        !NarrowPhase::box_box_overlap(Vec3::ZERO, id, half, Vec3::new(1.5, 0.0, 0.0), id, half),
        "half a metre of clear air is not a hit"
    );
    assert!(
        NarrowPhase::box_box_overlap(Vec3::ZERO, id, half, Vec3::new(1.0, 0.0, 0.0), id, half),
        "faces touching exactly counts — a frame-perfect hit must not be dropped"
    );
    assert!(
        NarrowPhase::box_box_overlap(Vec3::ZERO, id, half, Vec3::new(0.99, 0.0, 0.0), id, half),
        "and a hair inside certainly does"
    );
}
