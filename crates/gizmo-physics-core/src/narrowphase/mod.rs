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
        let (ref_pos, ref_rot, ref_h, inc_pos, inc_rot, inc_h, ref_is_a) =
            if is_face_axis(normal, &ax, 0.707) {
                (pos_a, rot_a, ha, pos_b, rot_b, hb, true)
            } else if is_face_axis(normal, &bx, 0.707) {
                (pos_b, rot_b, hb, pos_a, rot_a, ha, false)
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
                    (pos_a, rot_a, ha, pos_b, rot_b, hb, true)
                } else {
                    (pos_b, rot_b, hb, pos_a, rot_a, ha, false)
                }
            };

        // `clip_box_box` measures depth along a normal that must point reference→incident,
        // but `normal` follows the A→B convention. When B is the reference those are
        // opposite, so flip the normal going in and flip the contacts back to A→B coming
        // out. Without this the primary path sampled the reference box's FAR face in
        // `ref_face_d`, so every penetration came out inflated by ~2·(ref extent) — a
        // rotated box resting on an axis-aligned one got blown apart by the solver. The
        // empty-result fallback below already did this flip; the primary path did not.
        let clip_normal = if ref_is_a { normal } else { -normal };
        let mut contacts = clip_box_box(
            clip_normal, min_pen, ref_pos, ref_rot, ref_h, inc_pos, inc_rot, inc_h,
        );
        if !ref_is_a {
            for c in &mut contacts {
                c.normal = -c.normal; // restore A→B convention
            }
        }

        // Fallback: swap reference / incident faces.
        // Sutherland–Hodgman can yield zero points when the incident face is
        // much larger than the reference face and all corners project outside
        // the reference slab bounds.
        if contacts.is_empty() {
            contacts = clip_box_box(
                -clip_normal, min_pen, inc_pos, inc_rot, inc_h, ref_pos, ref_rot, ref_h,
            );
            // The swapped clip tags contacts with `-clip_normal`; convert back to A→B.
            if ref_is_a {
                for c in &mut contacts {
                    c.normal = -c.normal;
                }
            }
        }

        // Ultimate fallback to GJK when clipping completely fails (rare,
        // e.g. very thin boxes or heavily rounded geometry).
        if contacts.is_empty() {
            tracing::trace!(
                min_pen,
                "box-box SAT overlapped but Sutherland-Hodgman clipping produced no contacts; falling back to GJK/EPA"
            );
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

        // Per-pair narrowphase result (hot path → trace only). `contact_count == 0`
        // means the pair did not actually overlap this frame.
        tracing::trace!(contact_count = contacts.len(), "narrowphase manifold generated");

        contacts
    }
}

// The contact-generation helpers moved to `contacts`; import the ones the pair methods above
// call so their bodies stay verbatim. The 377-line test suite moved to `tests`.
mod contacts;
use contacts::{box_corners, clip_box_box, is_face_axis, mk_contact, sat_penetration};

#[cfg(test)]
mod tests;
