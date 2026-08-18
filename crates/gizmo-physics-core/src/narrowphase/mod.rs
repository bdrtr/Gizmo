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

    /// The candidate separating axes for two oriented boxes: each box's three face normals plus
    /// the nine edge–edge cross products, built on the stack so the sweep allocates nothing.
    ///
    /// Returns `(a_axes, b_axes, candidates, count)` — the count is below 15 whenever an edge
    /// pair is parallel, since a near-zero cross product is not an axis and normalising it would
    /// be a division by ~0.
    ///
    /// Shared by [`NarrowPhase::box_box`] and [`NarrowPhase::box_box_overlap`]: the second exists
    /// because a fighting game's hit detection wants the *answer*, not a manifold, and two copies
    /// of a fifteen-axis sweep is the kind of duplicate that drifts.
    fn box_separating_axes(rot_a: Quat, rot_b: Quat) -> ([Vec3; 3], [Vec3; 3], [Vec3; 15], usize) {
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

        // Layout: [ax0, ax1, ax2,  bx0, bx1, bx2,  9 cross products]
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

        (ax, bx, axes, n_axes)
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
        let (ax, bx, axes, n_axes) = Self::box_separating_axes(rot_a, rot_b);
        let ha_ = [ha.x, ha.y, ha.z];
        let hb_ = [hb.x, hb.y, hb.z];
        let t = pos_b - pos_a;

        for &axis in &axes[..n_axes] {
            if sat_penetration(&axis, &ax, &ha_, &bx, &hb_, t) < 0.0 {
                return false;
            }
        }
        true
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
        let (ax, bx, axes, n_axes) = Self::box_separating_axes(rot_a, rot_b);
        let ha_ = [ha.x, ha.y, ha.z];
        let hb_ = [hb.x, hb.y, hb.z];
        let t = pos_b - pos_a; // centre-to-centre offset

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
            if let Some(c) = Gjk::get_contact(shape, pos, rot, &face, mesh_pos, mesh_rot) {
                out.push(c);
            }
        }

        out.sort_by(|a, b| b.penetration.total_cmp(&a.penetration));
        out.truncate(4);
        out
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

        // One hull wrapper, refilled per triangle — hoisted for the reason the mesh path spells
        // out: building it inside the loop costs three allocations per candidate triangle.
        let mut face = ColliderShape::ConvexHull(crate::components::ConvexHullShape {
            vertices: std::sync::Arc::new(Vec::with_capacity(3)),
            faces: std::sync::Arc::new(Vec::new()),
        });

        for row in row0..row1 {
            for col in col0..col1 {
                for tri in hf.cell_triangles(row, col) {
                    let ColliderShape::ConvexHull(hull) = &mut face else {
                        unreachable!("`face` is constructed as a ConvexHull above")
                    };
                    let verts = std::sync::Arc::make_mut(&mut hull.vertices);
                    verts.clear();
                    verts.extend_from_slice(&tri);
                    if let Some(c) = Gjk::get_contact(shape, pos, rot, &face, hf_pos, hf_rot) {
                        out.push(c);
                    }
                }
            }
        }

        out.sort_by(|a, b| b.penetration.total_cmp(&a.penetration));
        out.truncate(4);
        out
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
    /// assigns the returned buffer to [`ContactManifold::contacts`] directly, after which
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
        });
        let _ = NarrowPhase::test_collision_manifold(
            &b, Vec3::new(0.0, 0.4, 0.0), Quat::IDENTITY, &broken, Vec3::ZERO, Quat::IDENTITY);
    }
}

// The contact-generation helpers moved to `contacts`; import the ones the pair methods above
// call so their bodies stay verbatim. The 377-line test suite moved to `tests`.
mod contacts;
use contacts::{box_corners, clip_box_box, is_face_axis, mk_contact, sat_penetration};

#[cfg(test)]
mod tests;
