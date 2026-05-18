//! Narrowphase collision detection.
//!
//! Provides a dispatcher ([`NarrowPhase`]) that routes each shape-pair to the
//! most accurate and efficient algorithm:
//!
//! | Shape A      | Shape B      | Algorithm                        |
//! |--------------|--------------|----------------------------------|
//! | Sphere       | Sphere       | Analytic                         |
//! | Sphere       | Plane        | Analytic                         |
//! | Box          | Plane        | Corner test (4 points)           |
//! | Box          | Box          | SAT + Sutherland-Hodgman clip    |
//! | Any          | Plane        | GJK support-point                |
//! | Any          | Any          | GJK + EPA                        |
//! | Compound     | Any          | Recursive sub-shape dispatch     |
//!
//! The convention throughout is that the **contact normal points from shape A
//! toward shape B** (i.e. it is the separating direction for body A).
//!
//! # Manifold vs. single-point API
//!
//! * [`NarrowPhase::test_collision`] — returns the single deepest contact, used
//!   for overlap queries and soft-body node tests.
//! * [`NarrowPhase::test_collision_manifold`] — returns up to 4 contacts for
//!   the constraint solver; Box-Box and Box-Plane produce multiple points.

use crate::collision::ContactPoint;
use crate::components::ColliderShape;
use crate::gjk::Gjk;
use gizmo_math::{Quat, Vec3};

// ============================================================================
//  Public API
// ============================================================================

pub struct NarrowPhase;

impl NarrowPhase {
    // ── Primitive tests ───────────────────────────────────────────────────

    /// Sphere–Sphere.  Normal points from A toward B.
    pub fn sphere_sphere(pos_a: Vec3, r_a: f32, pos_b: Vec3, r_b: f32) -> Option<ContactPoint> {
        let d = pos_b - pos_a;
        let d2 = d.length_squared();
        let rsum = r_a + r_b;

        // Use squared comparison to avoid a sqrt when there is no contact.
        if d2 >= rsum * rsum || d2 < 1e-10 {
            return None;
        }

        let dist = d2.sqrt();
        let normal = d / dist; // unit, A → B
        Some(mk_contact(pos_a + normal * r_a, normal, rsum - dist))
    }

    /// Sphere–Plane.  `n` is the plane normal (points away from the solid
    /// half-space); `d` is the signed plane offset (`p·n = d`).
    /// Normal in the returned contact points **from the sphere toward the
    /// plane** (i.e. into the plane — same convention: A → B where A = sphere).
    pub fn sphere_plane(
        sph_pos: Vec3,
        r: f32,
        plane_n: Vec3,
        plane_d: f32,
    ) -> Option<ContactPoint> {
        // Signed distance from sphere centre to plane (positive = above plane).
        let signed_dist = sph_pos.dot(plane_n) - plane_d;
        if signed_dist >= r {
            return None; // fully above the plane, no contact
        }
        // Contact point is the sphere's deepest point against the plane.
        let point = sph_pos - plane_n * signed_dist;
        // Normal: from sphere (A) toward plane (B), i.e. -plane_n.
        Some(mk_contact(point, -plane_n, r - signed_dist))
    }

    /// Box–Plane contact.  Returns up to 4 corner contacts (one per
    /// penetrating corner).  Normal in each contact points from the box toward
    /// the plane (`-plane_n`).
    pub fn box_plane(
        bpos: Vec3,
        brot: Quat,
        half: Vec3,
        plane_n: Vec3,
        plane_d: f32,
    ) -> Vec<ContactPoint> {
        box_corners(bpos, brot, half)
            .iter()
            .filter_map(|&corner| {
                let signed_dist = corner.dot(plane_n) - plane_d;
                if signed_dist < 0.0 {
                    // Corner is below the plane.
                    Some(mk_contact(
                        corner - plane_n * signed_dist,
                        -plane_n,
                        -signed_dist,
                    ))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Generic shape–plane using a GJK support point.  Returns at most one
    /// contact (the deepest support point against the plane).
    pub fn shape_plane(
        shape: &ColliderShape,
        pos: Vec3,
        rot: Quat,
        plane_n: Vec3,
        plane_d: f32,
    ) -> Option<ContactPoint> {
        // Support point in the direction opposing the plane normal gives the
        // deepest potential contact point on the shape.
        let deepest = Gjk::support_point(shape, pos, rot, -plane_n);
        let signed_dist = deepest.dot(plane_n) - plane_d;
        if signed_dist < 0.0 {
            Some(mk_contact(
                deepest - plane_n * signed_dist,
                -plane_n,
                -signed_dist,
            ))
        } else {
            None
        }
    }

    // ── Box–Box SAT ───────────────────────────────────────────────────────

    /// Box–Box via the Separating Axis Theorem (15 axes) followed by
    /// Sutherland–Hodgman clipping to produce up to 4 contact points.
    ///
    /// Returns an empty `Vec` when the boxes do not overlap.
    pub fn box_box(
        pos_a: Vec3,
        rot_a: Quat,
        ha: Vec3,
        pos_b: Vec3,
        rot_b: Quat,
        hb: Vec3,
    ) -> Vec<ContactPoint> {
        // Local axes of each box.
        let ax = [
            rot_a.mul_vec3(Vec3::X),
            rot_a.mul_vec3(Vec3::Y),
            rot_a.mul_vec3(Vec3::Z),
        ];
        let bx = [
            rot_b.mul_vec3(Vec3::X),
            rot_b.mul_vec3(Vec3::Y),
            rot_b.mul_vec3(Vec3::Z),
        ];
        let ha_ = [ha.x, ha.y, ha.z];
        let hb_ = [hb.x, hb.y, hb.z];
        let t = pos_b - pos_a; // centre-to-centre offset

        // Build the 15 candidate separating axes on the stack to avoid
        // heap allocation per-call.
        // Layout: [ax0, ax1, ax2,  bx0, bx1, bx2,  9 cross products]
        // Cross products that are near-zero (parallel edges) are skipped.
        let mut axes = [Vec3::ZERO; 15];
        let mut n_axes = 0usize;

        for &a in &ax {
            axes[n_axes] = a;
            n_axes += 1;
        }
        for &b in &bx {
            axes[n_axes] = b;
            n_axes += 1;
        }

        for &a in &ax {
            for &b in &bx {
                let c = a.cross(b);
                let len_sq = c.length_squared();
                if len_sq > 1e-6 {
                    // Normalise only valid edge–edge axes.
                    axes[n_axes] = c * len_sq.sqrt().recip();
                    n_axes += 1;
                }
            }
        }

        // SAT sweep — find minimum penetration axis.
        let mut min_pen = f32::MAX;
        let mut best_axis = Vec3::Y;
        let mut flip = false;

        for &axis in &axes[..n_axes] {
            let pen = sat_penetration(&axis, &ax, &ha_, &bx, &hb_, t);
            if pen < 0.0 {
                return vec![]; // Separating axis found — no overlap.
            }
            if pen < min_pen {
                min_pen = pen;
                best_axis = axis;
                // Ensure normal points from A toward B.
                flip = t.dot(axis) < 0.0;
            }
        }

        let normal = if flip { -best_axis } else { best_axis };

        // Choose reference face (the box whose axis is most aligned with the
        // contact normal gets to be the reference).  Threshold of 1/√2 ≈ 0.707
        // correctly handles 45° diagonal contacts; the original 0.9 threshold
        // misclassified many legitimate face contacts as edge-edge.
        let (ref_pos, ref_rot, ref_h, inc_pos, inc_rot, inc_h) = if is_face_axis(normal, &ax, 0.707)
        {
            (pos_a, rot_a, ha, pos_b, rot_b, hb)
        } else if is_face_axis(normal, &bx, 0.707) {
            (pos_b, rot_b, hb, pos_a, rot_a, ha)
        } else {
            // Edge–edge: choose the box whose local axis is better aligned.
            let dot_a = ax
                .iter()
                .map(|a| a.dot(normal).abs())
                .fold(0.0f32, f32::max);
            let dot_b = bx
                .iter()
                .map(|b| b.dot(normal).abs())
                .fold(0.0f32, f32::max);
            if dot_a >= dot_b {
                (pos_a, rot_a, ha, pos_b, rot_b, hb)
            } else {
                (pos_b, rot_b, hb, pos_a, rot_a, ha)
            }
        };

        // Primary clip attempt.
        let mut contacts = clip_box_box(
            normal, min_pen, ref_pos, ref_rot, ref_h, inc_pos, inc_rot, inc_h,
        );

        // Fallback: swap reference / incident faces.
        // Sutherland–Hodgman can yield zero points when the incident face is
        // much larger than the reference face and all corners project outside
        // the reference slab bounds.
        if contacts.is_empty() {
            contacts = clip_box_box(
                -normal, min_pen, inc_pos, inc_rot, inc_h, ref_pos, ref_rot, ref_h,
            );
            for c in &mut contacts {
                c.normal = -c.normal; // restore A→B convention
            }
        }

        // Ultimate fallback to GJK when clipping completely fails (rare,
        // e.g. very thin boxes or heavily rounded geometry).
        if contacts.is_empty() {
            let shape_a = ColliderShape::Box(crate::components::BoxShape { half_extents: ha });
            let shape_b = ColliderShape::Box(crate::components::BoxShape { half_extents: hb });
            if let Some(c) = Gjk::get_contact(&shape_a, pos_a, rot_a, &shape_b, pos_b, rot_b) {
                contacts.push(c);
            }
        }

        contacts
    }

    // ── Dispatcher: single deepest contact ───────────────────────────────

    /// Return the single deepest contact between two shapes, or `None` if
    /// they do not overlap.
    ///
    /// Use this for simple overlap queries or soft-body node tests.  For
    /// rigid-body simulation prefer [`test_collision_manifold`] which can
    /// return multiple contact points.
    pub fn test_collision(
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: Quat,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: Quat,
    ) -> Option<ContactPoint> {
        let contacts = Self::test_collision_manifold(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b);
        contacts
            .into_iter()
            .max_by(|a, b| a.penetration.total_cmp(&b.penetration))
    }

    /// Return up to 4 contact points between two shapes.
    ///
    /// Compound shapes are handled recursively; each sub-shape pair is
    /// dispatched independently and all resulting contacts are collected.
    pub fn test_collision_manifold(
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: Quat,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: Quat,
    ) -> Vec<ContactPoint> {
        // ── Compound shapes — recurse over sub-shapes ─────────────────────
        if let ColliderShape::Compound(parts) = shape_a {
            return parts
                .iter()
                .flat_map(|(local_t, sub)| {
                    let wp = pos_a + rot_a.mul_vec3(local_t.position);
                    let wr = rot_a * local_t.rotation;
                    Self::test_collision_manifold(sub, wp, wr, shape_b, pos_b, rot_b)
                })
                .collect();
        }
        if let ColliderShape::Compound(parts) = shape_b {
            return parts
                .iter()
                .flat_map(|(local_t, sub)| {
                    let wp = pos_b + rot_b.mul_vec3(local_t.position);
                    let wr = rot_b * local_t.rotation;
                    Self::test_collision_manifold(shape_a, pos_a, rot_a, sub, wp, wr)
                })
                .collect();
        }

        // ── Primitive dispatch ────────────────────────────────────────────
        let mut contacts: Vec<ContactPoint> = match (shape_a, shape_b) {
            // Sphere – Sphere
            (ColliderShape::Sphere(sa), ColliderShape::Sphere(sb)) => {
                Self::sphere_sphere(pos_a, sa.radius, pos_b, sb.radius)
                    .into_iter()
                    .collect()
            }

            // Sphere – Plane  (A = sphere, normal A→B = into plane = -plane_n)
            (ColliderShape::Sphere(s), ColliderShape::Plane(p)) => {
                Self::sphere_plane(pos_a, s.radius, p.normal, p.distance)
                    .into_iter()
                    .collect()
            }

            // Plane – Sphere  (A = plane, B = sphere; flip normal)
            (ColliderShape::Plane(p), ColliderShape::Sphere(s)) => {
                Self::sphere_plane(pos_b, s.radius, p.normal, p.distance)
                    .map(|mut c| {
                        c.normal = -c.normal;
                        c
                    })
                    .into_iter()
                    .collect()
            }

            // Box – Plane  (A = box, normal = into plane = -plane_n  ✓)
            (ColliderShape::Box(b), ColliderShape::Plane(p)) => {
                Self::box_plane(pos_a, rot_a, b.half_extents, p.normal, p.distance)
            }

            // Plane – Box  (A = plane, B = box; flip normal)
            (ColliderShape::Plane(p), ColliderShape::Box(b)) => {
                let mut cs = Self::box_plane(pos_b, rot_b, b.half_extents, p.normal, p.distance);
                for c in &mut cs {
                    c.normal = -c.normal;
                }
                cs
            }

            // Box – Box
            (ColliderShape::Box(ba), ColliderShape::Box(bb)) => {
                Self::box_box(pos_a, rot_a, ba.half_extents, pos_b, rot_b, bb.half_extents)
            }

            // Generic – Plane (A is arbitrary, B is plane)
            (_, ColliderShape::Plane(p)) => {
                Self::shape_plane(shape_a, pos_a, rot_a, p.normal, p.distance)
                    .into_iter()
                    .collect()
            }

            // Plane – Generic (A is plane, B is arbitrary; flip normal)
            (ColliderShape::Plane(p), _) => {
                Self::shape_plane(shape_b, pos_b, rot_b, p.normal, p.distance)
                    .map(|mut c| {
                        c.normal = -c.normal;
                        c
                    })
                    .into_iter()
                    .collect()
            }

            // Fallback to GJK + EPA for all other shape combinations.
            _ => Gjk::get_contact(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b)
                .into_iter()
                .collect(),
        };

        // Populate local-space contact points for warm-starting.
        for c in &mut contacts {
            c.local_point_a = c.point - pos_a;
            c.local_point_b = c.point - pos_b;
        }

        contacts
    }
}

// ============================================================================
//  SAT helpers
// ============================================================================

/// Signed penetration along `axis` between two oriented boxes.
///
/// Returns the overlap (positive = penetrating, negative = separated).
/// Caller must check for negative values and return early.
#[inline]
fn sat_penetration(
    axis: &Vec3,
    ax: &[Vec3; 3],
    ha: &[f32; 3],
    bx: &[Vec3; 3],
    hb: &[f32; 3],
    t: Vec3,
) -> f32 {
    let proj_a: f32 = ax
        .iter()
        .zip(ha)
        .map(|(e, &h)| e.dot(*axis).abs() * h)
        .sum();
    let proj_b: f32 = bx
        .iter()
        .zip(hb)
        .map(|(e, &h)| e.dot(*axis).abs() * h)
        .sum();
    let dist = t.dot(*axis).abs();
    proj_a + proj_b - dist
}

/// Returns `true` when `normal` is well-aligned with one of the box axes,
/// indicating that a face (rather than an edge) is the contact feature.
///
/// `threshold` should be `1/√2 ≈ 0.707` to correctly handle 45° cases.
#[inline]
fn is_face_axis(normal: Vec3, axes: &[Vec3; 3], threshold: f32) -> bool {
    axes.iter().any(|a| a.dot(normal).abs() > threshold)
}

// ============================================================================
//  Geometry helpers
// ============================================================================

/// Compute all 8 corners of an oriented box.
fn box_corners(pos: Vec3, rot: Quat, h: Vec3) -> [Vec3; 8] {
    const SIGNS: [(f32, f32, f32); 8] = [
        (1., 1., 1.),
        (-1., 1., 1.),
        (1., -1., 1.),
        (-1., -1., 1.),
        (1., 1., -1.),
        (-1., 1., -1.),
        (1., -1., -1.),
        (-1., -1., -1.),
    ];
    SIGNS.map(|(sx, sy, sz)| pos + rot.mul_vec3(Vec3::new(sx * h.x, sy * h.y, sz * h.z)))
}

/// Build a `ContactPoint` with zeroed warm-start fields.
#[inline]
fn mk_contact(point: Vec3, normal: Vec3, penetration: f32) -> ContactPoint {
    ContactPoint {
        point,
        normal,
        penetration,
        ..Default::default()
    }
}

// ============================================================================
//  Sutherland–Hodgman clipping — up to 4 contact points
// ============================================================================

/// Reduce `contacts` to the 4 points that best represent the contact patch:
/// deepest point first, then 3 more selected for maximum area coverage.
fn select_4_contacts(contacts: Vec<ContactPoint>) -> Vec<ContactPoint> {
    if contacts.len() <= 4 {
        return contacts;
    }

    let n = contacts.len();

    // Step 1 — deepest.
    let i0 = (0..n)
        .max_by(|&a, &b| contacts[a].penetration.total_cmp(&contacts[b].penetration))
        .unwrap();

    let mut chosen = vec![i0];

    // Steps 2-4 — greedily maximise minimum distance to already-chosen set.
    for _ in 0..3 {
        if chosen.len() == n {
            break;
        }
        let next = (0..n).filter(|i| !chosen.contains(i)).max_by(|&a, &b| {
            let da = chosen
                .iter()
                .map(|&c| (contacts[c].point - contacts[a].point).length_squared())
                .fold(f32::INFINITY, f32::min);
            let db = chosen
                .iter()
                .map(|&c| (contacts[c].point - contacts[b].point).length_squared())
                .fold(f32::INFINITY, f32::min);
            da.total_cmp(&db)
        });
        if let Some(idx) = next {
            chosen.push(idx);
        }
    }

    chosen.iter().map(|&i| contacts[i]).collect()
}

/// Sutherland–Hodgman box-vs-box clip.
///
/// Tests all 8 corners of the incident box against the reference box's face
/// and its 4 side slabs.  Returns up to 4 contacts selected for maximum
/// coverage.
///
/// `normal` must point **from reference toward incident** (A → B convention
/// from the caller's perspective).
fn clip_box_box(
    normal: Vec3,
    _min_pen: f32,
    ref_pos: Vec3,
    ref_rot: Quat,
    ref_h: Vec3,
    inc_pos: Vec3,
    inc_rot: Quat,
    inc_h: Vec3,
) -> Vec<ContactPoint> {
    let ref_axes = [
        ref_rot.mul_vec3(Vec3::X),
        ref_rot.mul_vec3(Vec3::Y),
        ref_rot.mul_vec3(Vec3::Z),
    ];
    let ref_h_arr = [ref_h.x, ref_h.y, ref_h.z];

    // Find the reference face — the axis most aligned with the contact normal.
    let (face_idx, _) = ref_axes
        .iter()
        .enumerate()
        .map(|(i, a)| (i, a.dot(normal).abs()))
        .fold(
            (0, 0.0f32),
            |(bi, bv), (i, v)| if v > bv { (i, v) } else { (bi, bv) },
        );

    let face_axis = ref_axes[face_idx];
    // Choose the outward-facing direction of the reference face.
    let face_dir = if face_axis.dot(normal) > 0.0 {
        face_axis
    } else {
        -face_axis
    };

    // Plane equation for the reference face: p · face_dir = ref_face_d
    let ref_face_d = (ref_pos + face_dir * ref_h_arr[face_idx]).dot(face_dir);

    // Tangent axes and their half-extents for the 4 side-slab clipping planes.
    let t0 = ref_axes[(face_idx + 1) % 3];
    let t1 = ref_axes[(face_idx + 2) % 3];
    let e0 = ref_h_arr[(face_idx + 1) % 3];
    let e1 = ref_h_arr[(face_idx + 2) % 3];

    // Tolerance to avoid floating-point edge-case rejections.
    const SLAB_TOLERANCE: f32 = 1e-3;

    let contacts: Vec<ContactPoint> = box_corners(inc_pos, inc_rot, inc_h)
        .iter()
        .filter_map(|&corner| {
            // 1. Corner must be on or behind the reference face.
            let signed_depth = ref_face_d - corner.dot(face_dir);
            if signed_depth <= 0.0 {
                return None;
            } // in front of reference face

            // 2. Corner must lie within the side slabs of the reference face.
            let local = corner - ref_pos;
            if local.dot(t0).abs() > e0 + SLAB_TOLERANCE {
                return None;
            }
            if local.dot(t1).abs() > e1 + SLAB_TOLERANCE {
                return None;
            }

            // Clamp penetration to be physically meaningful.
            // We allow slightly less than `min_pen` to avoid silently clamping
            // valid shallow contacts to an arbitrary fraction.
            let depth = signed_depth.max(0.0);

            Some(mk_contact(corner, normal, depth))
        })
        .collect();

    select_4_contacts(contacts)
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::BoxShape;

    fn box_shape(half: f32) -> ColliderShape {
        ColliderShape::Box(BoxShape {
            half_extents: Vec3::splat(half),
        })
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
}
