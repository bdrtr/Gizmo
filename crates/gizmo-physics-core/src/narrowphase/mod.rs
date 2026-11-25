//! Narrowphase collision detection.
//!
//! Provides a dispatcher ([`NarrowPhase`]) that routes each shape-pair to the
//! most accurate and efficient algorithm:
//!
//! | Shape A      | Shape B      | Algorithm                        |
//! |--------------|--------------|----------------------------------|
//! | Sphere       | Sphere       | Analytic                         |
//! | Sphere       | Plane        | Analytic                         |
//! | Box          | Plane        | Corner test (up to 8 points)     |
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
//! * [`NarrowPhase::test_collision_manifold`] — returns the contact manifold for
//!   the constraint solver; Box-Box and Box-Plane produce multiple points. The
//!   count is **not** capped at 4 — see that method's own docs for the per-arm
//!   bounds, and do not design against a 4-point assumption.

use crate::collision::ContactPoint;
use crate::components::ColliderShape;
use crate::gjk::Gjk;
use gizmo_math::{Quat, Vec3};

// ============================================================================
//  Public API
// ============================================================================

/// Namespace for the exact shape-pair tests and the dispatcher that routes between
/// them. It holds no state — every method is an associated function, and nothing
/// here is cached between calls.
///
/// Shapes are positioned by an explicit world-space `pos`/`rot` pair rather than by
/// a [`Transform`](crate::components::Transform), so no scale is involved anywhere:
/// a collider's size comes from the shape itself. Contact positions are world-space
/// metres, penetrations are metres, and a `normal` points from shape A toward shape
/// B — the argument order of the call, not any id ordering.
///
/// Pairs are tested exactly as given; there is no broadphase, no layer filtering and
/// no self-pair check here.
pub struct NarrowPhase;


/// The face axis of a box (`rotation` is the box's) closest to `normal`, signed to match it.
fn face_axis(normal: Vec3, rotation: Quat) -> Vec3 {
    [Vec3::X, Vec3::Y, Vec3::Z]
        .iter()
        .map(|&a| rotation * a)
        .max_by(|a, b| a.dot(normal).abs().total_cmp(&b.dot(normal).abs()))
        .map_or(normal, |a| if a.dot(normal) < 0.0 { -a } else { a })
}

/// Flattens a compound's per-part contacts, dropping the **seam** contacts of its box parts.
///
/// A compound of boxes laid edge to edge — a floor of tiles, a maze board — has seams where the
/// parts meet, and a body resting a hair inside the surface touches the next box's top *edge*
/// there as well as the face it rests on. That edge contact's normal leans back against the
/// motion. Measured on a ball with CCD rolling down seventeen tiles (`labirent`, 2026-11-03): 1.01 m
/// in two seconds against 2.21 m on one slab (the analytic value), and 2.03 m with these contacts
/// dropped; 2.23 m since a speculative contact with nothing left to close stopped being solved as
/// a resting one (the last 8 % was the landing, not a seam). With the rule removed it is 0.89 m
/// again.
///
/// A box part's edge or corner contact (its normal along none of the box's axes) is dropped when
/// another part holds the body with a **face** contact on the same plane — within 45° of that
/// face, its point on the face's plane: the parts are one surface there, and a surface has no
/// edge. A real edge — a step, a ledge, a wall top — is out of the neighbour's plane and keeps its
/// contact; a part touched alone keeps everything; parts that are not boxes are left alone.
fn drop_seam_contacts(per_part: Vec<(Option<Quat>, Vec<ContactPoint>)>) -> Vec<ContactPoint> {
    if per_part.iter().filter(|(_, c)| !c.is_empty()).count() < 2 {
        return per_part.into_iter().flat_map(|(_, c)| c).collect();
    }
    // Every face contact of a box part: its part, point and normal.
    let faces: Vec<(usize, Vec3, Vec3, f32)> = per_part
        .iter()
        .enumerate()
        .flat_map(|(i, (rot, contacts))| {
            contacts.iter().filter_map(move |c| {
                let r = (*rot)?;
                (face_axis(c.normal, r).dot(c.normal) >= 0.999).then_some((i, c.point, c.normal, c.penetration))
            })
        })
        .collect();
    let mut out = Vec::new();
    for (i, (rot, contacts)) in per_part.into_iter().enumerate() {
        for c in contacts {
            let seam = rot.is_some_and(|r| {
                let face = face_axis(c.normal, r);
                face.dot(c.normal) < 0.999
                    && faces.iter().any(|&(j, point, normal, pen)| {
                        j != i
                            && normal.dot(face) > std::f32::consts::FRAC_1_SQRT_2
                            && (point - c.point).dot(face).abs()
                                <= SEAM_PLANE_TOLERANCE + c.penetration.abs().max(pen.abs())
                    })
            });
            if !seam {
                out.push(c);
            }
        }
    }
    out
}

/// How far off a face's plane a neighbour's contact may sit and still count as on it.
const SEAM_PLANE_TOLERANCE: f32 = 0.01;

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

    /// Sphere–Box: the box's closest point to the sphere's centre, exactly, and out through the
    /// nearest face when the centre is inside it. Normal points from the sphere (A) toward the box
    /// (B); the point is on the box's surface, as [`Self::sphere_plane`]'s is on the plane.
    ///
    /// Until 2026-11-23 the pair went to GJK/EPA, whose contact point is the polytope's estimate:
    /// for a ball resting on a box's top it sat 2.7 mm off the centre with the normal tilted
    /// 4·10⁻⁵ rad (and not even mirrored when the pair was swapped). Every bounce spun the ball, so a
    /// ball dropped straight onto a floor rolled away diagonally — 0.82 m from a 3 m drop with a
    /// 0.25 m ball, 1.54 m with a 0.5 m one.
    pub fn sphere_box(sph_pos: Vec3, r: f32, box_pos: Vec3, box_rot: Quat, half: Vec3) -> Option<ContactPoint> {
        let local = box_rot.inverse() * (sph_pos - box_pos);
        let closest = local.clamp(-half, half);
        let off = local - closest;
        let d2 = off.length_squared();
        if d2 >= r * r {
            return None;
        }
        let (point, out, depth) = if d2 > 1e-12 {
            let dist = d2.sqrt();
            (closest, off / dist, r - dist)
        } else {
            // The centre is inside the box: out through the face it is nearest.
            let room = half - local.abs();
            let axis = if room.x <= room.y && room.x <= room.z { 0 } else if room.y <= room.z { 1 } else { 2 };
            let mut out = Vec3::ZERO;
            out[axis] = if local[axis] >= 0.0 { 1.0 } else { -1.0 };
            let mut point = local;
            point[axis] = half[axis] * out[axis];
            (point, out, r + room[axis])
        };
        // `out` points from the box toward the sphere; the contact's normal runs sphere → box.
        Some(mk_contact(box_pos + box_rot * point, -(box_rot * out), depth))
    }

    /// A sphere against a convex `shape`: the contact from its centre's closest point on the shape,
    /// by GJK's distance — exact on a flat face, and along the true normal — while the centre is
    /// outside the shape; `None` when it is inside, for EPA to answer. The sphere is A: the normal
    /// points from the sphere toward the shape, and the point is on the shape's surface.
    ///
    /// For the reason [`Self::sphere_box`] gives: EPA's contact point and normal are estimates, off
    /// a flat face's centre and tilted, so a ball dropped onto a hull floor or a cylinder's face
    /// spun at every bounce and rolled away (0.82 m and 0.49 m from a 3 m drop, measured
    /// 2026-11-23). Two GJK runs instead of GJK and EPA.
    pub fn sphere_convex(centre: Vec3, r: f32, shape: &ColliderShape, pos: Vec3, rot: Quat) -> Option<Vec<ContactPoint>> {
        let point = ColliderShape::Sphere(crate::components::SphereShape { radius: 0.0 });
        if Gjk::test_collision(&point, centre, Quat::IDENTITY, shape, pos, rot) {
            return None;
        }
        // `distance` answers for separated shapes, which a point outside the shape is: the gap,
        // and the direction from the shape to the centre.
        let (gap, from_shape) = Gjk::distance(|d| centre - Gjk::support_point(shape, pos, rot, -d))?;
        if gap >= r {
            return Some(Vec::new());
        }
        if gap <= 1e-6 {
            // Touching the surface with the centre: no direction to trust; EPA's.
            return None;
        }
        Some(vec![mk_contact(centre - from_shape * gap, -from_shape, r - gap)])
    }

    /// Box–Plane contact.  Returns up to **8** corner contacts — one per penetrating
    /// corner, with no reduction step; a box lying flat gives 4, one resting on a
    /// face-diagonal tilt can give more, and a fully submerged box gives all 8.
    /// Normal in each contact points from the box toward the plane (`-plane_n`).
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

    /// [`shape_plane`](Self::shape_plane)'s contact and, for a shape that can lie along the plane,
    /// the rest of what it rests on (`contacts::feature_on_plane`).
    fn shape_plane_contacts(shape: &ColliderShape, pos: Vec3, rot: Quat, plane_n: Vec3, plane_d: f32) -> Vec<ContactPoint> {
        let Some(deepest) = Self::shape_plane(shape, pos, rot, plane_n, plane_d) else {
            return Vec::new();
        };
        feature_on_plane(shape, pos, rot, plane_n, plane_d, deepest).unwrap_or_else(|| vec![deepest])
    }

    /// Whether a pair's contact is a patch clipped from the faces (`clip_polyhedra`): a box or a hull
    /// against a hull, or against a cylinder, a cone or a capsule, which can lie along its face.
    fn clips_faces(a: &ColliderShape, b: &ColliderShape) -> bool {
        let polyhedron = |s: &ColliderShape| matches!(s, ColliderShape::Box(_) | ColliderShape::ConvexHull(_));
        let lies = |s: &ColliderShape| polyhedron(s) || matches!(s, ColliderShape::Cylinder(_) | ColliderShape::Cone(_) | ColliderShape::Capsule(_));
        (polyhedron(a) || polyhedron(b)) && lies(a) && lies(b)
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

    /// The SAT's axes for two boxes, in the order they are tried: A's three faces, B's three, then
    /// the nine edge crosses that are not degenerate, normalised — each handed to `test` with its
    /// index, until `test` says to stop (`false`). Returns whether it stopped. The edge crosses are
    /// only worked out once all six faces have been tried: most candidate pairs are apart along a
    /// face, and the crosses with their square roots were most of what a separated pair cost.
    fn for_each_separating_axis(ax: &[Vec3; 3], bx: &[Vec3; 3], mut test: impl FnMut(usize, Vec3) -> bool) -> bool {
        let mut index = 0usize;
        for &axis in ax.iter().chain(bx.iter()) {
            if !test(index, axis) {
                return true;
            }
            index += 1;
        }
        for &a in ax {
            for &b in bx {
                let c = a.cross(b);
                let len_sq = c.length_squared();
                if len_sq > 1e-6 {
                    if !test(index, c * len_sq.sqrt().recip()) {
                        return true;
                    }
                    index += 1;
                }
            }
        }
        false
    }

    /// A box's three axes in the world.
    fn box_frame(rot: Quat) -> [Vec3; 3] {
        [rot.mul_vec3(Vec3::X), rot.mul_vec3(Vec3::Y), rot.mul_vec3(Vec3::Z)]
    }

    /// Do two oriented boxes overlap? The same fifteen-axis SAT sweep as
    /// [`NarrowPhase::box_box`], stopping at the first separating axis and building no manifold.
    ///
    /// This is the test a fighting game's hitbox/hurtbox pass wants: it asks whether the attack
    /// connected, not where or how deeply, and it runs over pairs the physics broadphase never
    /// sees (hit volumes are not colliders). Touching exactly — zero penetration on the tightest
    /// axis — counts as an overlap, which for a hit volume is the reading that does not drop a
    /// frame-perfect hit.
    pub fn box_box_overlap(
        pos_a: Vec3,
        rot_a: Quat,
        ha: Vec3,
        pos_b: Vec3,
        rot_b: Quat,
        hb: Vec3,
    ) -> bool {
        let (ax, bx) = (Self::box_frame(rot_a), Self::box_frame(rot_b));
        let ha_ = [ha.x, ha.y, ha.z];
        let hb_ = [hb.x, hb.y, hb.z];
        let t = pos_b - pos_a;

        let separated = Self::for_each_separating_axis(&ax, &bx, |_, axis| sat_penetration(&axis, &ax, &ha_, &bx, &hb_, t) >= 0.0);
        !separated
    }

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
        let (ax, bx) = (Self::box_frame(rot_a), Self::box_frame(rot_b));
        let ha_ = [ha.x, ha.y, ha.z];
        let hb_ = [hb.x, hb.y, hb.z];
        let t = pos_b - pos_a; // centre-to-centre offset

        // Least penetration over the axes, tried in order: the first separating one ends it.
        let mut min_pen = f32::MAX;
        let mut best_axis = Vec3::Y;
        let mut best_index = 0usize;
        let mut flip = false;
        let separated = Self::for_each_separating_axis(&ax, &bx, |index, axis| {
            let pen = sat_penetration(&axis, &ax, &ha_, &bx, &hb_, t);
            if pen < 0.0 {
                return false; // Separating axis found — no overlap.
            }
            if pen < min_pen {
                min_pen = pen;
                best_axis = axis;
                best_index = index;
                flip = t.dot(axis) < 0.0;
            }
            true
        });
        if separated {
            return vec![];
        }

        let normal = if flip { -best_axis } else { best_axis };

        // The reference is the box whose FACE the separating axis is — the face the other box is
        // pushed out of. It used to be whichever box had an axis within 45° of the normal, A asked
        // first: with A turned 30° against an axis-aligned B whose face IS the separating axis, A
        // was the reference, and the manifold measured B's flat face against A's corner — depths
        // that did not describe the overlap. An edge–edge axis belongs to neither box, and there
        // the better-aligned box is the reference, as before.
        let (ref_pos, ref_rot, ref_h, inc_pos, inc_rot, inc_h, ref_is_a) =
            if best_index < 3 {
                (pos_a, rot_a, ha, pos_b, rot_b, hb, true)
            } else if best_index < 6 {
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
    /// rigid-body simulation prefer [`test_collision_manifold`](Self::test_collision_manifold) which can
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

    /// Contacts between a convex shape and a **triangle mesh**, one triangle at a time.
    ///
    /// # Why this exists
    ///
    /// A `TriMesh` used to fall through to the GJK+EPA fallback, and GJK is a **convex**
    /// algorithm: its support function walks the mesh's BVH for the farthest vertex in a
    /// direction, which describes the mesh's *convex hull*. For a hull that is right; for a
    /// racetrack it is catastrophic. The hull of a closed oval ribbon is a filled disc, so a car
    /// driving inside the oval is "inside" the hull and gets pushed out sideways by a surface that
    /// is not there. A dip in the ground becomes a lid over it.
    ///
    /// So: ask the BVH which triangles the shape's own bounds actually reach, and test each one as
    /// its own convex shape. Three points *are* convex, so GJK is exactly right per triangle, and
    /// the concavity lives in which triangles get picked rather than in the algorithm.
    ///
    /// # What it costs, and the bound on it
    ///
    /// One GJK call per candidate triangle. The BVH keeps that proportional to what the shape
    /// overlaps rather than to mesh size — a car on a city block touches a handful of triangles,
    /// not the city. The contacts are then cut to the **four deepest**, which is what the solver
    /// takes anyway; keeping more would cost solver time for points it discards.
    fn shape_trimesh(
        shape: &ColliderShape,
        pos: Vec3,
        rot: Quat,
        mesh: &crate::components::TriMeshShape,
        mesh_pos: Vec3,
        mesh_rot: Quat,
    ) -> Vec<ContactPoint> {
        // The query box, in the mesh's own space. Six support queries give the shape's world
        // bounds without needing to know what shape it is; the corners then come back into mesh
        // space, and their AABB is conservative — never tighter than the truth, which is the safe
        // direction for a broad query.
        let axes = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        let inv = mesh_rot.inverse();
        for dir in axes {
            let p = inv * (Gjk::support_point(shape, pos, rot, dir) - mesh_pos);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        if !lo.is_finite() || !hi.is_finite() {
            return Vec::new();
        }
        // A skin of tolerance, so a shape resting exactly on a triangle plane still finds it.
        let skin = Vec3::splat(0.01);
        let query = gizmo_math::Aabb::new(lo - skin, hi + skin);

        let mut tris = Vec::new();
        mesh.bvh.query_aabb(query, &mut tris);
        if tris.is_empty() {
            return Vec::new();
        }

        let mut out: Vec<ContactPoint> = Vec::new();
        let mut patched = false;

        // The triangle-as-a-hull wrapper is built ONCE and refilled per triangle. It used to be
        // constructed inside the loop, which cost three allocations per candidate triangle, per
        // pair, per substep: `Arc::new(verts.clone())` is the Vec plus the ArcInner, and
        // `Arc::new(Vec::new())` a third for a list that is never read. That made this the
        // densest allocation site in the narrowphase — and one no container swap can fix, since
        // `ConvexHullShape` holds `Arc<Vec<_>>` by definition. Hoisting is the fix.
        //
        // `faces` stays empty for the reason it always did: the support function only reads
        // `vertices`, so a face list would be carried per triangle for nothing.
        let mut face = ColliderShape::ConvexHull(crate::components::ConvexHullShape {
            vertices: std::sync::Arc::new(Vec::with_capacity(3)),
            faces: std::sync::Arc::new(Vec::new()),
        });

        for tri in tris {
            let base = tri as usize * 3;

            // `make_mut` rather than `get_mut`: the refcount is 1 throughout (`Gjk::get_contact`
            // takes `&ColliderShape` and no path clones the Arc), so this never actually clones —
            // but unlike `get_mut` it cannot silently skip a triangle if that ever stops holding.
            let ColliderShape::ConvexHull(hull) = &mut face else {
                unreachable!("`face` is constructed as a ConvexHull above and never reassigned")
            };
            let verts = std::sync::Arc::make_mut(&mut hull.vertices);
            verts.clear();
            let mut ok = true;
            for k in 0..3 {
                match mesh.indices.get(base + k).and_then(|i| mesh.vertices.get(*i as usize)) {
                    Some(v) => verts.push(*v),
                    // A corrupt index is skipped rather than panicking: this is the hot path over
                    // data that may have come from a file.
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            let local = [verts[0], verts[1], verts[2]];
            if let Some(c) = Gjk::get_contact(shape, pos, rot, &face, mesh_pos, mesh_rot) {
                // The surface's rule, as for terrain (`terrain_contact`), on the side of the
                // triangle the shape is on: a mesh says nothing about which side is solid — a road
                // and a wall are both just triangles — and its winding is whatever the file had.
                let world = local.map(|v| mesh_pos + mesh_rot * v);
                let front = (world[1] - world[0]).cross(world[2] - world[0]).dot(pos - world[0]) >= 0.0;
                let t = tri as usize;
                let (tri_w, ridge_kind) = if front {
                    (world, crate::components::TriMeshShape::EDGE_RIDGE_FRONT)
                } else {
                    ([world[0], world[2], world[1]], crate::components::TriMeshShape::EDGE_RIDGE_BACK)
                };
                // Turned over, corners 1 and 2 swap, and with them the edges opposite them.
                let ridge = |edge: usize| mesh.edge_kind(t, if front { edge } else { [0, 2, 1][edge] }) == ridge_kind;
                if let Some(c) = Self::terrain_contact(c, shape, pos, rot, tri_w, ridge) {
                    patched |= Self::push_surface_contact(&mut out, c, shape, pos, rot, tri_w);
                }
            }
        }

        Self::surface_contacts(out, patched)
    }

    /// How close to a surface triangle's face normal a contact's normal is for the contact to be on
    /// the face (the cosine): `terrain_contact` and `push_surface_contact` read it alike.
    const ON_THE_FACE: f32 = 0.9995;

    /// How far above a surface triangle's plane, in metres, a shape's deepest point may be and the
    /// shape still touch it, when GJK/EPA says they touch (`terrain_contact`): rounding, not a gap.
    const TOUCHING: f32 = 1e-4;

    /// Adds one triangle's contact with a surface's shape to `out`: for a box or a convex hull on the
    /// triangle's face, the patch it rests on (`clip_on_triangle`), else the one point. Says whether
    /// it was a patch.
    fn push_surface_contact(out: &mut Vec<ContactPoint>, c: ContactPoint, shape: &ColliderShape, pos: Vec3, rot: Quat, triangle: [Vec3; 3]) -> bool {
        let n = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]).normalize_or_zero();
        // On the face, not over an edge — by `terrain_contact`'s own measure — the contact pushes the
        // shape straight off the plane. Along the face normal exactly, not EPA's: EPA's is up to 2° off
        // it here, and a lying cylinder given that sideways share at one end started to roll and never
        // stopped (2026-11-25).
        if n != Vec3::ZERO && n.dot(c.normal).abs() > Self::ON_THE_FACE {
            // Wound with its normal towards the shape, which is where the contact pushes it.
            let (towards, up) = if n.dot(c.normal) < 0.0 { (triangle, n) } else { ([triangle[0], triangle[2], triangle[1]], -n) };
            let patch = clip_on_triangle(shape, pos, rot, towards, -up);
            if !patch.is_empty() {
                out.extend(patch);
                return true;
            }
        }
        out.push(c);
        false
    }

    /// The contacts a surface keeps of its triangles': the four deepest, or, when a box or a hull
    /// lay on a face (`push_surface_contact`), the four that span the most of its patch — four of
    /// a resting box's corners at one depth are no use bunched in one triangle.
    fn surface_contacts(out: Vec<ContactPoint>, patched: bool) -> Vec<ContactPoint> {
        if !patched {
            return Self::four_deepest(out);
        }
        let mut kept: Vec<ContactPoint> = Vec::with_capacity(out.len());
        for c in out {
            if !kept.iter().any(|k| k.point.distance_squared(c.point) < 1e-8 && k.normal.dot(c.normal) > 0.9999) {
                kept.push(c);
            }
        }
        select_4_contacts(kept)
    }

    /// The four deepest of a surface's per-triangle contacts — what the solver keeps — with a
    /// point two triangles found (the shape's deepest point over an edge they share) taken once,
    /// not pushed on twice.
    fn four_deepest(mut out: Vec<ContactPoint>) -> Vec<ContactPoint> {
        out.sort_by(|a, b| b.penetration.total_cmp(&a.penetration));
        let mut kept: Vec<ContactPoint> = Vec::with_capacity(4);
        for c in out {
            if !kept.iter().any(|k| k.point.distance_squared(c.point) < 1e-8 && k.normal.dot(c.normal) > 0.9999) {
                kept.push(c);
            }
            if kept.len() == 4 {
                break;
            }
        }
        kept
    }

    /// Contacts between a shape and a terrain heightfield, generated per **cell**.
    ///
    /// The same idea as [`Self::shape_trimesh`] — a concave surface is not one convex shape, so
    /// each piece of it is tested as its own — with the tree replaced by arithmetic: the query
    /// shape's bounds in the field's frame divide straight into a range of cells, and each cell
    /// contributes its two triangles. Nothing is stored per triangle and nothing is traversed.
    ///
    /// The cost is bounded the same way: one GJK call per candidate triangle, and the candidates
    /// are what the shape's own box overlaps — a character on a hillside touches one or two
    /// cells whatever the terrain's size. Contacts are cut to the four deepest, which is what
    /// the solver keeps.
    ///
    /// A heightfield as the *query* shape is a programming error, not a case: it has no support
    /// function (see [`Gjk::support_point`]), and the dispatch above cuts the heightfield–
    /// heightfield pair before it gets here.
    fn shape_heightfield(
        shape: &ColliderShape,
        pos: Vec3,
        rot: Quat,
        hf: &crate::components::HeightfieldShape,
        hf_pos: Vec3,
        hf_rot: Quat,
    ) -> Vec<ContactPoint> {
        let (cells_x, cells_z) = hf.cell_counts();
        if cells_x == 0 || cells_z == 0 {
            return Vec::new();
        }

        // The query box in the field's own space, from six support queries — the same trick the
        // mesh path uses to get a shape's bounds without knowing what shape it is.
        let axes = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        let inv = hf_rot.inverse();
        for dir in axes {
            let p = inv * (Gjk::support_point(shape, pos, rot, dir) - hf_pos);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        if !lo.is_finite() || !hi.is_finite() {
            return Vec::new();
        }
        // A skin of tolerance, so a body resting exactly on the surface still finds its cell.
        let skin = Vec3::splat(0.01);
        let (lo, hi) = (lo - skin, hi + skin);

        // Vertically out of reach: terrain is a surface, so a shape above the highest sample or
        // below the lowest cannot touch it, and answering that with a comparison beats walking
        // the cells to find nothing.
        if lo.y > hf.local_aabb.max.y || hi.y < hf.local_aabb.min.y {
            return Vec::new();
        }

        let (col0, col1, row0, row1) = hf.cell_range(lo, hi);
        if col0 >= col1 || row0 >= row1 {
            return Vec::new();
        }

        let mut out: Vec<ContactPoint> = Vec::new();
        let mut patched = false;

        // One hull wrapper, refilled per triangle — hoisted for the reason the mesh path spells
        // out: building it inside the loop costs three allocations per candidate triangle.
        let mut face = ColliderShape::ConvexHull(crate::components::ConvexHullShape {
            vertices: std::sync::Arc::new(Vec::with_capacity(3)),
            faces: std::sync::Arc::new(Vec::new()),
        });

        for row in row0..row1 {
            for col in col0..col1 {
                for (k, tri) in hf.cell_triangles(row, col).into_iter().enumerate() {
                    let ColliderShape::ConvexHull(hull) = &mut face else {
                        unreachable!("`face` is constructed as a ConvexHull above")
                    };
                    let verts = std::sync::Arc::make_mut(&mut hull.vertices);
                    verts.clear();
                    verts.extend_from_slice(&tri);
                    if let Some(c) = Gjk::get_contact(shape, pos, rot, &face, hf_pos, hf_rot) {
                        let ridge = |edge: usize| Self::heightfield_edge_is_ridge(hf, row, col, k, edge);
                        let world = tri.map(|v| hf_pos + hf_rot * v);
                        if let Some(c) = Self::terrain_contact(c, shape, pos, rot, world, ridge) {
                            patched |= Self::push_surface_contact(&mut out, c, shape, pos, rot, world);
                        }
                    }
                }
            }
        }

        Self::surface_contacts(out, patched)
    }

    /// The contact a shape makes with one triangle of a **surface that goes on past it** — a
    /// terrain's — given what GJK/EPA found for the triangle on its own.
    ///
    /// EPA treats the triangle as the whole of the other body: a plate with no thickness and
    /// open edges. Two of its answers are wrong for terrain, and both were measured:
    ///
    /// - **Through the surface.** A plate can be left on either side, so a shape sunk deeper than
    ///   halfway is pushed out *below* it. A box tilted 20° and sunk 0.4 m into a flat field got
    ///   a normal pushing it down, along its own axis.
    /// - **Off an edge.** A shape that overlaps a triangle only a little is pushed off sideways,
    ///   over an edge that the next triangle continues — the "ghost" collision. In `dunya` a
    ///   parked car resting across the seam of two terrain cells took 0.7 m pushes along its
    ///   own tilted axis, with impulses of 8 000–21 000 N·s, and in half the runs fell through
    ///   the ground.
    ///
    /// The ground is solid below its surface, so a shape is moved out **upward, along the
    /// triangle's face normal**, by how far its deepest point lies below the triangle's plane —
    /// where that point is over this triangle; one over another triangle is that triangle's
    /// contact. EPA's answer stands where it is right: when its normal is the face normal
    /// already, and over a **ridge** — an edge `ridge` reports convex — when it points out of
    /// the front of the surface, which is what an edge's own normal does there.
    fn terrain_contact(
        c: ContactPoint,
        shape: &ColliderShape,
        pos: Vec3,
        rot: Quat,
        tri: [Vec3; 3],
        ridge: impl Fn(usize) -> bool,
    ) -> Option<ContactPoint> {
        let Some(face) = (tri[1] - tri[0]).cross(tri[2] - tri[0]).try_normalize() else {
            return Some(c);
        };
        // The shape is moved along `-c.normal`.
        let out_along = (-c.normal).dot(face);
        if out_along > Self::ON_THE_FACE {
            // On the face. A shape that lies along it gets its patch clipped from the face
            // (`push_surface_contact`), whatever EPA's point. One that touches at a point touches at
            // its deepest point — where EPA stopped is up to 1.6 mm off it, against a triangle, and
            // a ball's normal pushed there turned it: a ball dropped on a flat mesh rolled away and
            // never slept (2026-11-25). Over another triangle, that triangle has the contact.
            if contacts::support_feature(shape, pos, rot, -face).is_some() {
                return Some(c);
            }
            let deepest = Gjk::support_point(shape, pos, rot, -face);
            let depth = (tri[0] - deepest).dot(face);
            // EPA found them touching: a shape resting exactly on the surface may put its deepest
            // point a rounding error above the plane, and it is touching all the same. Dropping that
            // contact left a CCD ball on terrain only its speculative one, which has no friction,
            // and it rolled for 520 frames where a ball without CCD stopped in 14.
            if depth <= -Self::TOUCHING || !Self::barycentric(deepest + face * depth, tri).is_some_and(|w| w.iter().all(|&x| x > -1e-3)) {
                return None;
            }
            let depth = depth.max(0.0);
            return Some(ContactPoint { point: deepest + face * (depth * 0.5), normal: -face, penetration: depth, ..c });
        }
        // Which feature of the triangle EPA's contact is on: its witness on the triangle, in the
        // triangle's barycentric weights. A weight near zero puts it on the edge opposite.
        let on_tri = c.point - c.normal * (c.penetration * 0.5);
        let weights = Self::barycentric(on_tri, tri);
        let on_ridge = weights.is_some_and(|w| (0..3).any(|i| w[i] < 1e-3 && ridge(i)));
        if on_ridge && out_along > 0.0 {
            return Some(c);
        }
        let deepest = Gjk::support_point(shape, pos, rot, -face);
        let depth = (tri[0] - deepest).dot(face);
        if depth <= 0.0 {
            return None;
        }
        let on_plane = deepest + face * depth;
        if !Self::barycentric(on_plane, tri).is_some_and(|w| w.iter().all(|&x| x > -1e-3)) {
            return None;
        }
        let point = deepest + face * (depth * 0.5);
        Some(ContactPoint { point, normal: -face, penetration: depth, ..c })
    }

    /// `p`'s barycentric weights in triangle `t` (for `t[0]`, `t[1]`, `t[2]`), measured in the
    /// triangle's plane; `None` for a degenerate triangle.
    pub(crate) fn barycentric(p: Vec3, t: [Vec3; 3]) -> Option<[f32; 3]> {
        let (ab, ac, ap) = (t[1] - t[0], t[2] - t[0], p - t[0]);
        let (d00, d01, d11) = (ab.dot(ab), ab.dot(ac), ac.dot(ac));
        let (d20, d21) = (ap.dot(ab), ap.dot(ac));
        let denom = d00 * d11 - d01 * d01;
        if denom.abs() < 1e-12 {
            return None;
        }
        let v = (d11 * d20 - d01 * d21) / denom;
        let w = (d00 * d21 - d01 * d20) / denom;
        Some([1.0 - v - w, v, w])
    }

    /// Whether the edge of heightfield triangle `tri` (0 or 1, as [`HeightfieldShape::cell_triangles`]
    /// returns them) in cell `(row, col)` opposite its vertex `edge` is a **ridge**: the triangle
    /// across it bends down by more than 5°, so the edge stands out of the surface and a shape
    /// can rest on it. A flat or hollow edge is not, and nor is one on the field's border, past
    /// which terrain is taken to go on (a world is tiled out of fields).
    ///
    /// [`HeightfieldShape::cell_triangles`]: crate::components::HeightfieldShape::cell_triangles
    fn heightfield_edge_is_ridge(hf: &crate::components::HeightfieldShape, row: u32, col: u32, tri: usize, edge: usize) -> bool {
        // The two samples of the edge and the sample opposite it in the triangle across: for
        // `[p00, p10, p11]` and `[p00, p11, p01]`, edge `i` runs between the other two corners.
        let (r, c) = (row as i64, col as i64);
        let (a, b, across): ((i64, i64), (i64, i64), (i64, i64)) = match (tri, edge) {
            (0, 0) => ((r + 1, c), (r + 1, c + 1), (r + 2, c + 1)),
            (0, 1) => ((r + 1, c + 1), (r, c), (r, c + 1)),
            (0, _) => ((r, c), (r + 1, c), (r, c - 1)),
            (_, 0) => ((r + 1, c + 1), (r, c + 1), (r + 1, c + 2)),
            (_, 1) => ((r, c + 1), (r, c), (r - 1, c)),
            (_, _) => ((r, c), (r + 1, c + 1), (r + 1, c)),
        };
        let valid = |(r, c): (i64, i64)| r >= 0 && c >= 0 && r < hf.rows as i64 && c < hf.cols as i64;
        if !valid(across) {
            return false;
        }
        let at = |(r, c): (i64, i64)| hf.sample_position(r as u32, c as u32);
        let own = hf.cell_triangles(row, col)[tri];
        let (Some(n), Some(other)) = (
            (own[1] - own[0]).cross(own[2] - own[0]).try_normalize(),
            (at(b) - at(a)).cross(at(across) - at(a)).try_normalize(),
        ) else {
            return false;
        };
        // Both normals up: the triangle across is wound the other way round its shared edge.
        let other = if other.y < 0.0 { -other } else { other };
        n.dot(at(across) - at(a)) < 0.0 && n.dot(other) < 5f32.to_radians().cos()
    }

    /// Return the contact points between two shapes, empty when they do not
    /// overlap.
    ///
    /// Compound shapes are handled recursively; each sub-shape pair is
    /// dispatched independently and all resulting contacts are collected.
    ///
    /// **The returned count is not capped.** The triangle-mesh path keeps its four deepest
    /// points; box–box also reduces to four but by a SPREAD heuristic — deepest first, then
    /// greedily maximising distance from those already chosen — so it can keep a shallow
    /// far-apart corner over a deeper clustered one. A box against a
    /// plane emits one contact per penetrating corner — up to eight — and a
    /// compound concatenates whatever its sub-pairs produced.
    ///
    /// **Nothing downstream launders that count.** An earlier version of this paragraph
    /// said the result could be fed through
    /// [`ContactManifold::add_contact`](crate::collision::ContactManifold::add_contact)
    /// to get the manifold's 4-point reduction. That method has no production callers:
    /// this crate's own tests are its only ones, and `gizmo-physics-rigid`'s pipeline
    /// assigns the returned buffer to [`ContactManifold::contacts`](crate::collision::ContactManifold::contacts) directly, after which
    /// the solver consumes every point in it. Treat the count as uncapped, and do not size
    /// a fixed-capacity buffer from a 4-point assumption.
    ///
    /// Each returned contact's `local_point_a`/`local_point_b` are filled in here as
    /// the plain offsets `point - pos_a` and `point - pos_b`. They are *not* rotated
    /// into either body's local frame, and they are relative to the position passed in —
    /// which is the collider's transform origin, not necessarily a centre of mass. A caller
    /// that wants true body-local coordinates has to apply the inverse rotation itself.
    ///
    /// [`ColliderShape::Compound`] is the exception: its branch returns before this fill-in,
    /// so the offsets are relative to the SUB-PART's derived world position, not to the
    /// position you passed.
    ///
    /// The dispatch order matters and is not arbitrary: plane arms come before the
    /// triangle-mesh arms, because a plane must never reach
    /// [`Gjk::support_point`](crate::Gjk::support_point), and the mesh arms come
    /// before the GJK fallback, because GJK would collide against a mesh's convex
    /// hull instead of its actual surface.
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
            let per_part: Vec<(Option<Quat>, Vec<ContactPoint>)> = parts
                .iter()
                .map(|(local_t, sub)| {
                    let wp = pos_a + rot_a.mul_vec3(local_t.position);
                    let wr = rot_a * local_t.rotation;
                    let boxed = matches!(**sub, ColliderShape::Box(_)).then_some(wr);
                    (boxed, Self::test_collision_manifold(sub, wp, wr, shape_b, pos_b, rot_b))
                })
                .collect();
            return drop_seam_contacts(per_part);
        }
        if let ColliderShape::Compound(parts) = shape_b {
            let per_part: Vec<(Option<Quat>, Vec<ContactPoint>)> = parts
                .iter()
                .map(|(local_t, sub)| {
                    let wp = pos_b + rot_b.mul_vec3(local_t.position);
                    let wr = rot_b * local_t.rotation;
                    let boxed = matches!(**sub, ColliderShape::Box(_)).then_some(wr);
                    (boxed, Self::test_collision_manifold(shape_a, pos_a, rot_a, sub, wp, wr))
                })
                .collect();
            return drop_seam_contacts(per_part);
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

            // Sphere – Box, exactly (see `sphere_box`), and its mirror
            (ColliderShape::Sphere(sp), ColliderShape::Box(b)) => {
                Self::sphere_box(pos_a, sp.radius, pos_b, rot_b, b.half_extents).into_iter().collect()
            }
            (ColliderShape::Box(b), ColliderShape::Sphere(sp)) => Self::sphere_box(pos_b, sp.radius, pos_a, rot_a, b.half_extents)
                .map(|mut c| {
                    c.normal = -c.normal;
                    c
                })
                .into_iter()
                .collect(),


            // Generic – Plane (A is arbitrary, B is plane)
            (_, ColliderShape::Plane(p)) => Self::shape_plane_contacts(shape_a, pos_a, rot_a, p.normal, p.distance),

            // Plane – Generic (A is plane, B is arbitrary; flip normal)
            (ColliderShape::Plane(p), _) => {
                let mut cs = Self::shape_plane_contacts(shape_b, pos_b, rot_b, p.normal, p.distance);
                for c in &mut cs {
                    c.normal = -c.normal;
                }
                cs
            }

            // Anything – TriMesh, and its mirror. Position matters twice over. **Below the two
            // Plane arms**, because `(TriMesh, Plane)` must keep going to `shape_plane`: routing it
            // here would hand a `Plane` to `Gjk::support_point`, which that function documents as a
            // contract violation and asserts on. **Above the GJK fallback**, because a mesh is not
            // convex and the fallback collides against its convex hull. See [`Self::shape_trimesh`].
            (_, ColliderShape::TriMesh(tm)) => {
                Self::shape_trimesh(shape_a, pos_a, rot_a, tm, pos_b, rot_b)
            }
            (ColliderShape::TriMesh(tm), _) => {
                let mut cs = Self::shape_trimesh(shape_b, pos_b, rot_b, tm, pos_a, rot_a);
                for c in &mut cs {
                    c.normal = -c.normal;
                }
                cs
            }

            // Anything – Heightfield, and its mirror. Same placement rules as the TriMesh pair
            // above and for the same two reasons: below the Plane arms so a `(Heightfield, Plane)`
            // pair still reaches `shape_plane` rather than handing a plane to GJK, and above the
            // GJK fallback because terrain is concave — the fallback would collide against the
            // convex hull of the landscape, i.e. a lid over every valley.
            //
            // Two heightfields never generate contacts: both are static terrain, so the pair is
            // work with no possible outcome. The arm below reaches `shape_heightfield` with a
            // heightfield as the *query* shape, whose support function refuses — so the case is
            // cut here instead.
            (ColliderShape::Heightfield(_), ColliderShape::Heightfield(_)) => Vec::new(),
            (_, ColliderShape::Heightfield(hf)) => {
                Self::shape_heightfield(shape_a, pos_a, rot_a, hf, pos_b, rot_b)
            }
            (ColliderShape::Heightfield(hf), _) => {
                let mut cs = Self::shape_heightfield(shape_b, pos_b, rot_b, hf, pos_a, rot_a);
                for c in &mut cs {
                    c.normal = -c.normal;
                }
                cs
            }

            // A sphere against any other convex shape: its centre's closest point, by GJK's distance,
            // while the centre is outside (see `sphere_convex`); EPA when it is inside.
            (ColliderShape::Sphere(sp), _) => Self::sphere_convex(pos_a, sp.radius, shape_b, pos_b, rot_b)
                .unwrap_or_else(|| Gjk::get_contact(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b).into_iter().collect()),
            (_, ColliderShape::Sphere(sp)) => match Self::sphere_convex(pos_b, sp.radius, shape_a, pos_a, rot_a) {
                Some(mut cs) => {
                    for c in &mut cs {
                        c.normal = -c.normal;
                    }
                    cs
                }
                None => Gjk::get_contact(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b).into_iter().collect(),
            },

            // A convex hull against a hull or a box, or a cylinder, a cone or a capsule against either:
            // EPA's normal, then the faces clipped to a patch of up to four points (`clip_polyhedra`);
            // EPA's one point when the faces give none.
            (a, b) if Self::clips_faces(a, b) => {
                match Gjk::get_contact(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b) {
                    Some(epa) => {
                        let patch = clip_polyhedra((shape_a, pos_a, rot_a), (shape_b, pos_b, rot_b), epa.normal);
                        if patch.is_empty() { vec![epa] } else { patch }
                    }
                    None => Vec::new(),
                }
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

#[cfg(test)]
mod trimesh_tests {
    use super::*;
    use crate::components::{BoxShape, ColliderShape, TriMeshShape};
    use std::sync::Arc;

    /// A flat square of ground in the XZ plane at `y`, as two triangles spanning `±half`.
    fn quad(y: f32, half: f32) -> (Vec<Vec3>, Vec<u32>) {
        (
            vec![
                Vec3::new(-half, y, -half),
                Vec3::new(half, y, -half),
                Vec3::new(half, y, half),
                Vec3::new(-half, y, half),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    fn trimesh(vertices: Vec<Vec3>, mut indices: Vec<u32>) -> ColliderShape {
        let bvh = crate::bvh::BvhTree::build(&vertices, &mut indices).expect("bvh");
        ColliderShape::TriMesh(TriMeshShape {
            vertices: Arc::new(vertices),
            indices: Arc::new(indices),
            bvh: Arc::new(bvh),
                local_aabb: Default::default(),
                ridges: Default::default(),
        })
    }

    /// A box resting on a flat trimesh floor is pushed **up**, and barely.
    #[test]
    fn a_box_on_a_trimesh_floor_is_pushed_up() {
        let (v, i) = quad(0.0, 20.0);
        let floor = trimesh(v, i);
        let b = ColliderShape::Box(BoxShape { half_extents: Vec3::splat(0.5) });

        // Sunk 0.1 into the floor.
        let cs = NarrowPhase::test_collision_manifold(
            &b, Vec3::new(0.0, 0.4, 0.0), Quat::IDENTITY,
            &floor, Vec3::ZERO, Quat::IDENTITY,
        );
        assert!(!cs.is_empty(), "a box overlapping the floor must produce contacts");
        // Normal points A→B, i.e. from the box down into the floor.
        for c in &cs {
            assert!(c.normal.y < -0.7, "normal {:?} is not into the floor", c.normal);
            assert!(c.penetration > 0.0 && c.penetration < 0.5, "penetration {}", c.penetration);
        }

        // Clear of the floor: no contacts.
        let none = NarrowPhase::test_collision_manifold(
            &b, Vec3::new(0.0, 3.0, 0.0), Quat::IDENTITY,
            &floor, Vec3::ZERO, Quat::IDENTITY,
        );
        assert!(none.is_empty(), "a box well above the floor must not touch it");
    }

    /// **The bug this arm was added for.** A ring of ground with a hole in the middle is
    /// concave: standing in the hole must touch nothing.
    ///
    /// Under the old GJK+EPA fallback the mesh was reduced to its **convex hull** — the hole is
    /// filled in — so a body in the middle collided with a lid that is not there. That is what
    /// made a closed racetrack undrivable, and it is why this test puts the box where the mesh
    /// *is not*.
    #[test]
    fn a_hole_in_the_mesh_is_a_hole_and_not_its_convex_hull() {
        // Four quads forming a ring around an empty 4×4 centre, all at y = 0.
        let mut v = Vec::new();
        let mut i = Vec::new();
        for (cx, cz) in [(-6.0, 0.0), (6.0, 0.0), (0.0, -6.0), (0.0, 6.0)] {
            let (qv, qi) = quad(0.0, 2.0);
            let base = v.len() as u32;
            v.extend(qv.into_iter().map(|p| p + Vec3::new(cx, 0.0, cz)));
            i.extend(qi.into_iter().map(|k| k + base));
        }
        let ring = trimesh(v, i);
        let b = ColliderShape::Box(BoxShape { half_extents: Vec3::splat(0.5) });

        // Dead centre, at the height the ground would be: the hull says "solid", the mesh says
        // "nothing here".
        let inside = NarrowPhase::test_collision_manifold(
            &b, Vec3::new(0.0, 0.0, 0.0), Quat::IDENTITY,
            &ring, Vec3::ZERO, Quat::IDENTITY,
        );
        assert!(
            inside.is_empty(),
            "the hole is empty space; {} contact(s) means the convex hull is being collided with",
            inside.len()
        );

        // And the ring itself still collides, so the emptiness above is not simply "nothing works".
        let on_ring = NarrowPhase::test_collision_manifold(
            &b, Vec3::new(6.0, 0.4, 0.0), Quat::IDENTITY,
            &ring, Vec3::ZERO, Quat::IDENTITY,
        );
        assert!(!on_ring.is_empty(), "the ring's own surface must still be solid");
    }

    /// The mesh's transform is honoured: the same box and mesh, with the mesh moved and turned,
    /// collide exactly as if the box had been moved and turned the opposite way.
    #[test]
    fn a_moved_and_turned_mesh_is_collided_in_its_own_space() {
        let (v, i) = quad(0.0, 20.0);
        let floor = trimesh(v, i);
        let b = ColliderShape::Box(BoxShape { half_extents: Vec3::splat(0.5) });
        let mesh_pos = Vec3::new(3.0, 1.0, -2.0);
        let mesh_rot = Quat::from_rotation_y(0.7);

        let cs = NarrowPhase::test_collision_manifold(
            &b, mesh_pos + Vec3::new(0.0, 0.4, 0.0), Quat::IDENTITY,
            &floor, mesh_pos, mesh_rot,
        );
        assert!(!cs.is_empty(), "a moved floor is still a floor");
        for c in &cs {
            assert!(c.normal.y < -0.7, "normal {:?} is not into the floor", c.normal);
        }
    }

    /// The mirrored arm flips the normal, so `(mesh, box)` is the negation of `(box, mesh)` —
    /// the A→B convention this whole module keeps.
    #[test]
    fn the_mirrored_pair_flips_the_normal() {
        let (v, i) = quad(0.0, 20.0);
        let floor = trimesh(v, i);
        let b = ColliderShape::Box(BoxShape { half_extents: Vec3::splat(0.5) });
        let at = Vec3::new(0.0, 0.4, 0.0);

        let fwd = NarrowPhase::test_collision_manifold(&b, at, Quat::IDENTITY, &floor, Vec3::ZERO, Quat::IDENTITY);
        let rev = NarrowPhase::test_collision_manifold(&floor, Vec3::ZERO, Quat::IDENTITY, &b, at, Quat::IDENTITY);
        assert_eq!(fwd.len(), rev.len());
        assert!(!fwd.is_empty());
        assert!(rev[0].normal.y > 0.7, "mesh→box must point up out of the floor: {:?}", rev[0].normal);
    }

    /// A mesh against a **plane** keeps going to `shape_plane`, not to the new arm.
    ///
    /// This is a regression the new arms caused and this test now prevents. Placed above the two
    /// Plane arms, `(TriMesh, Plane)` matched the trimesh arm, which hands the plane to
    /// `Gjk::support_point` — a case that function documents as a contract violation and
    /// `debug_assert!`s on. The arms belong *below* Plane and *above* the GJK fallback, and the
    /// window between the two is the only correct place for them.
    #[test]
    fn a_mesh_against_a_plane_still_takes_the_plane_path() {
        let (v, i) = quad(0.0, 5.0);
        let mesh = trimesh(v, i);
        let plane = ColliderShape::Plane(crate::components::PlaneShape {
            normal: Vec3::Y,
            distance: 0.2,
        });
        // Both orders: neither may reach GJK with a plane in hand.
        let _ = NarrowPhase::test_collision_manifold(
            &mesh, Vec3::ZERO, Quat::IDENTITY, &plane, Vec3::ZERO, Quat::IDENTITY);
        let _ = NarrowPhase::test_collision_manifold(
            &plane, Vec3::ZERO, Quat::IDENTITY, &mesh, Vec3::ZERO, Quat::IDENTITY);
    }

    /// An empty mesh, and a mesh whose indices point past its vertices, produce no contacts and
    /// no panic. Meshes arrive from files.
    #[test]
    fn a_degenerate_mesh_is_survived() {
        let b = ColliderShape::Box(BoxShape { half_extents: Vec3::splat(0.5) });
        let empty = ColliderShape::TriMesh(TriMeshShape {
            vertices: Arc::new(Vec::new()),
            indices: Arc::new(Vec::new()),
            bvh: Arc::new(crate::bvh::BvhTree::default()),
                local_aabb: Default::default(),
                ridges: Default::default(),
        });
        assert!(NarrowPhase::test_collision_manifold(
            &b, Vec3::ZERO, Quat::IDENTITY, &empty, Vec3::ZERO, Quat::IDENTITY).is_empty());

        // A BVH that claims a triangle the index array cannot supply.
        let (v, i) = quad(0.0, 5.0);
        let bvh = crate::bvh::BvhTree::build(&v, &mut i.clone()).expect("bvh");
        let broken = ColliderShape::TriMesh(TriMeshShape {
            vertices: Arc::new(v),
            indices: Arc::new(vec![0, 1, 2]), // one triangle where the BVH indexes two
            bvh: Arc::new(bvh),
                local_aabb: Default::default(),
                ridges: Default::default(),
        });
        let _ = NarrowPhase::test_collision_manifold(
            &b, Vec3::new(0.0, 0.4, 0.0), Quat::IDENTITY, &broken, Vec3::ZERO, Quat::IDENTITY);
    }
}

// The contact-generation helpers moved to `contacts`; import the ones the pair methods above
// call so their bodies stay verbatim. The 377-line test suite moved to `tests`.
mod contacts;
use contacts::{box_corners, clip_box_box, clip_on_triangle, clip_polyhedra, feature_on_plane, mk_contact, sat_penetration, select_4_contacts};

#[cfg(test)]
mod tests;
