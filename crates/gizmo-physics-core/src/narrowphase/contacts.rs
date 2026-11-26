//! Contact-generation geometry shared by the narrowphase shape-pair tests: SAT penetration,
//! face-axis classification, box-corner enumeration, the `ContactPoint` builder, contact-set
//! reduction, and box-box clipping. Extracted verbatim from `narrowphase.rs` (pure move); the
//! helpers used by the pair methods in the parent module are `pub(super)`.

use super::*;

/// Signed penetration along `axis` between two oriented boxes.
///
/// Returns the overlap (positive = penetrating, negative = separated).
/// Caller must check for negative values and return early.
#[inline]
pub(super) fn sat_penetration(
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

// ============================================================================
//  Geometry helpers
// ============================================================================

/// Compute all 8 corners of an oriented box.
pub(super) fn box_corners(pos: Vec3, rot: Quat, h: Vec3) -> [Vec3; 8] {
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

/// How far a point of a touching patch may be from the other surface and still be one of its
/// contacts: in front of it, at its separation as a negative depth, which the solve lets close by
/// that much in a substep — a speculative point, as Box2D's manifolds carry them out to their
/// speculative distance.
///
/// A patch was what touched, to a tenth of a millimetre. A cylinder landing lying 0.001 rad off
/// level had one end touching and the other 0.2 mm above the floor: struck on the one end alone,
/// it turned 3 rad/s, came down on the other, and fell asleep 5 mm into the floor and tilted; a box
/// let go 0.006 rad off level slept 2.2° over on every floor at 60 Hz (2026-11-26). The point that
/// is about to touch is in the patch now, and the impact is the whole face's.
pub(super) const SPECULATIVE_MARGIN: f32 = 0.01;

/// A patch point's depth, from how far `behind` the other surface it is (negative in front of it):
/// on it within `tolerance`, a speculative point out to [`SPECULATIVE_MARGIN`], and none past that.
#[inline]
pub(super) fn patch_depth(behind: f32, tolerance: f32) -> Option<f32> {
    if behind >= -tolerance {
        Some(behind.max(0.0))
    } else if behind >= -SPECULATIVE_MARGIN {
        Some(behind)
    } else {
        None
    }
}

/// Build a `ContactPoint` with zeroed warm-start fields.
#[inline]
pub(super) fn mk_contact(point: Vec3, normal: Vec3, penetration: f32) -> ContactPoint {
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

/// Reduce `contacts` to the 4 points that best represent the contact patch: the deepest, the one
/// farthest from it, the one making the largest triangle with those two, and the one making the
/// largest quadrilateral with all three (the largest diagonal cross product, as Bullet's
/// `calcArea4Points` measures it).
///
/// **By area, not by distance.** The three after the deepest were each the point farthest from the
/// nearest one already chosen. On a regular patch every candidate ties, and the pick fell where the
/// ties broke: a hexagonal prism on a floor kept the corners at 120°, 180°, 240° and 300°, half its
/// base, with its centre of mass on the edge of the four — it tipped onto the open side when it
/// landed and fell asleep 2° over (2026-11-25). The largest quadrilateral of a symmetric patch holds
/// its centre.
pub(super) fn select_4_contacts(mut contacts: Vec<ContactPoint>) -> Vec<ContactPoint> {
    /// Depths this close are one depth: rounding, not a tilt.
    const AS_DEEP: f32 = 1e-4;
    if contacts.len() <= 4 {
        return contacts;
    }

    let n = contacts.len();
    let at = |i: usize| contacts[i].point;

    // Step 1 — deepest; of points as deep as it, the one farthest from the patch's middle. A flat
    // patch is all one depth, and which point the rounding made deepest was where the quadrilateral
    // started: of a box on a mesh where six triangles meet, the triangles' patches include points on
    // their edges inside the box's face, one of them started it, and the box was held up on three
    // corners and the middle of an edge — struck flat on the floor, it turned (2026-11-26). The
    // farthest point of a patch is a corner of it.
    let deepest = contacts.iter().map(|c| c.penetration).fold(f32::NEG_INFINITY, f32::max);
    let middle = contacts.iter().map(|c| c.point).sum::<Vec3>() / n as f32;
    let mut i0 = 0;
    let mut farthest = f32::NEG_INFINITY;
    for (i, c) in contacts.iter().enumerate() {
        let out = (c.point - middle).length_squared();
        if c.penetration >= deepest - AS_DEEP && out > farthest {
            (i0, farthest) = (i, out);
        }
    }
    // The best of the points not yet chosen by `score`, the first of equals.
    let best = |chosen: &[usize], score: &dyn Fn(Vec3) -> f32| -> Option<usize> {
        let mut found: Option<(usize, f32)> = None;
        for i in (0..n).filter(|i| !chosen.contains(i)) {
            let s = score(at(i));
            if found.is_none_or(|(_, b)| s > b) {
                found = Some((i, s));
            }
        }
        found.map(|(i, _)| i)
    };

    // Inline, because this branch keeps exactly 4 out of more than 4 — the index set cannot
    // outgrow the array. `vec![i0]` allocated for a single `usize` and then reallocated 1 -> 4
    // on the first push below, two allocations per reduced contact set per substep.
    let mut chosen: arrayvec::ArrayVec<usize, 4> = arrayvec::ArrayVec::new();
    chosen.push(i0);
    let p0 = at(i0);
    // Step 2 — farthest from the deepest.
    if let Some(i1) = best(&chosen, &|p| (p - p0).length_squared()) {
        chosen.push(i1);
        let p1 = at(i1);
        // Step 3 — the largest triangle with those two.
        if let Some(i2) = best(&chosen, &|p| (p1 - p0).cross(p - p0).length_squared()) {
            chosen.push(i2);
            let p2 = at(i2);
            // Step 4 — the largest quadrilateral with all three: of the three ways to pair four
            // points into diagonals, the convex hull's are the ones with the largest cross product.
            let area4 = |p: Vec3| {
                let a = (p0 - p1).cross(p2 - p);
                let b = (p0 - p2).cross(p1 - p);
                let c = (p0 - p).cross(p1 - p2);
                a.length_squared().max(b.length_squared()).max(c.length_squared())
            };
            // Of quadrilaterals as large, the one centred nearest the patch's middle: a regular
            // hexagon's largest four corners are a rectangle about its centre, or as large a kite
            // to one side of it, and the first of equals was where the ties broke.
            let largest = (0..n).filter(|i| !chosen.contains(i)).map(|i| area4(at(i))).fold(0.0f32, f32::max);
            let centre = |p: Vec3| -((p0 + p1 + p2 + p) * 0.25 - middle).length_squared();
            if let Some(i3) = best(&chosen, &|p| if area4(p) >= largest * (1.0 - 1e-4) { centre(p) } else { f32::NEG_INFINITY }) {
                chosen.push(i3);
            }
        }
    }

    // Refill the input buffer instead of collecting into a fresh one. It holds more than 4
    // elements on this branch, so it already has the capacity for the at-most-4 we keep and
    // `extend_from_slice` after `clear` provably cannot reallocate.
    //
    // Staging through a stack array is not optional: the kept order is CHOSEN order — deepest
    // first, then the three greedy spread picks — not ascending index order, so the points
    // cannot be compacted in place with a retain-style sweep without changing the output.
    let mut picked = [ContactPoint::default(); 4];
    for (slot, &i) in chosen.iter().enumerate() {
        picked[slot] = contacts[i];
    }
    let kept = chosen.len();
    contacts.clear();
    contacts.extend_from_slice(&picked[..kept]);
    contacts
}

/// Sutherland–Hodgman box-vs-box clip.
///
/// The incident box's face that faces the reference box — the one whose outward normal points
/// most against `normal` — is clipped against the four side planes of the reference face, and
/// every vertex of what is left that lies on or behind the reference box's support plane is a
/// contact, its depth measured along `normal`. Returns up to 4 contacts selected for maximum
/// coverage.
///
/// **A clip, not a corner test.** This tested the incident box's eight corners against the
/// reference face's slab and kept those inside it — which finds the contact patch only when its
/// corners are the incident box's own corners. In a deep, rotated overlap the deep part of the
/// patch is bounded by the slab's sides, not by corners: the incident corners inside the slab were
/// only the ones barely touching, every contact came back at depth 0, and the solver had nothing
/// to push apart. Found by `box_box_collision_is_symmetric` on CI (2026-11-10): two boxes 3.3
/// deep into each other, depth 0 one way round and 3.3 the other. The clip puts vertices on the
/// slab's sides where the incident face crosses them, so the deep part is in the manifold.
///
/// `normal` must point **from reference toward incident** (A → B convention
/// from the caller's perspective).
pub(super) fn clip_box_box(
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

    // Reference-box support (farthest extent) along the CONTACT NORMAL. Each contact is
    // tagged with `normal` and the solver applies `penetration` along `normal`, so depth
    // must be measured along `normal` too. When the reference face axis diverges from the
    // normal (rotated boxes — is_face_axis admits up to 45°), the old `.dot(face_dir)` at the
    // face centre over/under-reported depth and gave asymmetric depths across a flat face →
    // spurious torque or unresolved overlap. The box's support along the normal (all three
    // axes' projections, not just the face axis) makes each contact's depth the true MTV.
    let ref_face_d = ref_pos.dot(normal)
        + ref_axes[0].dot(normal).abs() * ref_h_arr[0]
        + ref_axes[1].dot(normal).abs() * ref_h_arr[1]
        + ref_axes[2].dot(normal).abs() * ref_h_arr[2];

    // Tangent axes and their half-extents for the 4 side-slab clipping planes.
    let t0 = ref_axes[(face_idx + 1) % 3];
    let t1 = ref_axes[(face_idx + 2) % 3];
    let e0 = ref_h_arr[(face_idx + 1) % 3];
    let e1 = ref_h_arr[(face_idx + 2) % 3];

    // Tolerance to avoid floating-point edge-case rejections.
    const SLAB_TOLERANCE: f32 = 1e-3;

    /// The same idea as `SLAB_TOLERANCE`, for the depth test, and it was missing.
    ///
    /// The test used to be `signed_depth <= 0.0`, which rejects a corner that is EXACTLY on the
    /// reference face. Two boxes placed at exact contact — every stack in this engine's tests and
    /// a perfectly ordinary way to build a scene — have all four corners at `signed_depth == 0.0`,
    /// so all four were rejected, Sutherland–Hodgman returned empty, the swapped-reference retry
    /// returned empty for the same reason, and the pair fell through to the GJK/EPA fallback,
    /// which returns ONE contact. A one-point manifold carries no tilt-resisting torque whatever
    /// the block solver does with it, and the point GJK returns is not at the centre: its offset
    /// grows with the other collider's size, reaching the resting box's own edge, so it delivers a
    /// torque impulse the geometry does not call for (measured at about 0.03 rad/s for a unit
    /// cube). Below a support half-extent of about 1.5 the interface never recovered at all — the
    /// centred one-point contact held the box up without torque, so it never sank, so
    /// `signed_depth` never became positive and the clip path was never reached again.
    ///
    /// See `gizmo-physics-rigid/tests/solver_quality.rs::what_does_a_manifold_look_like_when_it_is_born`.
    const DEPTH_TOLERANCE: f32 = 1e-4;

    // The incident face: the face of the incident box whose outward normal points most against
    // `normal` (which points reference → incident), i.e. the one turned towards the reference box.
    let inc_axes = [inc_rot.mul_vec3(Vec3::X), inc_rot.mul_vec3(Vec3::Y), inc_rot.mul_vec3(Vec3::Z)];
    let inc_h_arr = [inc_h.x, inc_h.y, inc_h.z];
    let (k, _) = inc_axes
        .iter()
        .enumerate()
        .map(|(i, a)| (i, a.dot(normal).abs()))
        .fold((0, 0.0f32), |(bi, bv), (i, v)| if v > bv { (i, v) } else { (bi, bv) });
    let outward = if inc_axes[k].dot(normal) > 0.0 { -inc_axes[k] } else { inc_axes[k] };
    let centre = inc_pos + outward * inc_h_arr[k];
    let u = inc_axes[(k + 1) % 3] * inc_h_arr[(k + 1) % 3];
    let v = inc_axes[(k + 2) % 3] * inc_h_arr[(k + 2) % 3];
    let mut polygon: arrayvec::ArrayVec<Vec3, 8> = arrayvec::ArrayVec::new();
    polygon.extend([centre + u + v, centre - u + v, centre - u - v, centre + u - v]);

    // Clipped against the reference face's four side planes, each `p · axis <= offset`.
    for (axis, extent) in [(t0, e0), (-t0, e0), (t1, e1), (-t1, e1)] {
        polygon = clip_to_half_space(&polygon, axis, ref_pos.dot(axis) + extent + SLAB_TOLERANCE);
        if polygon.is_empty() {
            return Vec::new();
        }
    }

    // The reference face itself, turned towards the incident box. A clipped point is a contact only
    // if it is behind this plane — inside the reference box — and not merely behind its support
    // plane along `normal`: when the reference box is tilted against the incident face (a crate
    // pitching as it slides along a pedestal is the reference, the pedestal's top the incident
    // face), the face clipped to the crate's footprint runs under the crate's RAISED edge too, and
    // every such point sat behind the support plane at the depth of the lowest edge. Those
    // contacts held the raised side up as if it were buried, and their friction stopped a crate
    // knocked at 3 m/s in a tenth of a second (measured on `poligon`'s pedestals).
    let face_axis = ref_axes[face_idx];
    let face_normal = if face_axis.dot(normal) >= 0.0 { face_axis } else { -face_axis };
    let face_d = ref_pos.dot(face_normal) + ref_h_arr[face_idx];
    // The reference face's normal is within 45° of `normal` for a face contact and within ~55°
    // for an edge one (the most aligned of three orthogonal axes), so this never divides by less
    // than about 0.57.
    let along = face_normal.dot(normal).max(0.5);

    let contacts: Vec<ContactPoint> = polygon
        .iter()
        .filter_map(|&point| {
            let support_depth = ref_face_d - point.dot(normal);
            let face_depth = face_d - point.dot(face_normal);
            // Inside the reference box (behind its face), and behind its support plane — or in front
            // of both by no more than the speculative margin, a point about to touch.
            if support_depth < -SPECULATIVE_MARGIN || face_depth < -SPECULATIVE_MARGIN {
                return None;
            }
            // The point's own depth: how far it travels along `normal` before it leaves the
            // reference box through that face — never more than the support plane's depth. For
            // boxes whose faces line up (every resting stack) the two are the same number.
            let depth = patch_depth((face_depth / along).min(support_depth), DEPTH_TOLERANCE)?;
            Some(mk_contact(point, normal, depth))
        })
        .collect();

    select_4_contacts(contacts)
}

/// One Sutherland–Hodgman step: the part of the convex `polygon` with `p · axis <= offset`. A
/// quadrilateral clipped by four half-spaces has at most eight vertices, so it stays on the stack.
fn clip_to_half_space(polygon: &[Vec3], axis: Vec3, offset: f32) -> arrayvec::ArrayVec<Vec3, 8> {
    let mut out = arrayvec::ArrayVec::new();
    for (i, &a) in polygon.iter().enumerate() {
        let b = polygon[(i + 1) % polygon.len()];
        let (da, db) = (a.dot(axis) - offset, b.dot(axis) - offset);
        if da <= 0.0 {
            let _ = out.try_push(a);
        }
        if (da <= 0.0) != (db <= 0.0) {
            let _ = out.try_push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

// ============================================================================
//  Polyhedra — a box or a convex hull against a convex hull: clipped faces
// ============================================================================

/// One face of a polyhedral shape, in world space: its outward unit normal, its plane
/// (`normal · x = offset` on the face) and its corners, counter-clockwise seen from outside.
pub(super) struct PolyFace {
    normal: Vec3,
    offset: f32,
    corners: Vec<Vec3>,
}

/// The face of a box or a convex hull whose outward normal is most aligned with `dir`, or `None` for
/// another shape or a hull with no faces (one built from a degenerate point set).
///
/// A hull's flat face is several of its triangles in one plane: the face is every vertex in the
/// best triangle's plane, so a box-like hull gives its four corners and not one triangle's three.
pub(super) fn polyhedral_face(shape: &ColliderShape, pos: Vec3, rot: Quat, dir: Vec3) -> Option<PolyFace> {
    let local_dir = rot.inverse() * dir;
    let (local_n, mut local) = match shape {
        ColliderShape::Box(b) => {
            let h = b.half_extents;
            let a = local_dir.abs();
            let k = if a.x >= a.y && a.x >= a.z { 0 } else if a.y >= a.z { 1 } else { 2 };
            let s = if local_dir[k] >= 0.0 { 1.0 } else { -1.0 };
            let mut n = Vec3::ZERO;
            n[k] = s;
            let (i, j) = ((k + 1) % 3, (k + 2) % 3);
            let corner = |u: f32, v: f32| {
                let mut p = Vec3::ZERO;
                p[k] = s * h[k];
                p[i] = u * h[i];
                p[j] = v * h[j];
                p
            };
            (n, vec![corner(1.0, 1.0), corner(-1.0, 1.0), corner(-1.0, -1.0), corner(1.0, -1.0)])
        }
        ColliderShape::ConvexHull(hull) => {
            let (verts, faces) = (&hull.vertices, &hull.faces);
            let mut best: Option<(f32, Vec3)> = None;
            for f in faces.iter() {
                let [a, b, c] = f.map(|i| verts[i as usize]);
                let n = (b - a).cross(c - a);
                let len = n.length();
                if len < 1e-12 {
                    continue;
                }
                let n = n / len;
                let align = n.dot(local_dir);
                if best.is_none_or(|(bv, _)| align > bv) {
                    best = Some((align, n));
                }
            }
            let (_, n) = best?;
            let d = verts.iter().map(|v| v.dot(n)).fold(f32::MIN, f32::max);
            let size = verts.iter().map(|v| v.length()).fold(0.0f32, f32::max);
            let tol = 1e-4 + 1e-3 * size;
            (n, verts.iter().copied().filter(|v| (v.dot(n) - d).abs() <= tol).collect())
        }
        _ => return None,
    };
    if local.len() < 3 {
        return None;
    }
    // Counter-clockwise about the normal: by angle around the centroid in the face's plane.
    let centre = local.iter().copied().sum::<Vec3>() / local.len() as f32;
    let u = (local[0] - centre).normalize_or_zero();
    let v = local_n.cross(u);
    local.sort_by(|a, b| {
        let (pa, pb) = (*a - centre, *b - centre);
        pa.dot(v).atan2(pa.dot(u)).total_cmp(&pb.dot(v).atan2(pb.dot(u)))
    });
    local.dedup_by(|a, b| (*a - *b).length_squared() < 1e-12);
    let normal = rot * local_n;
    let corners: Vec<Vec3> = local.into_iter().map(|p| pos + rot * p).collect();
    Some(PolyFace { normal, offset: normal.dot(corners[0]), corners })
}

/// The part of a convex shape, in world space, that faces `dir` and that a contact along `dir` can
/// lie all along: a box's or a hull's face (`polyhedral_face`), the flat end of a cylinder or a cone
/// (eight points of its rim), or a line down the side of a cylinder, a cone or a lying capsule (its
/// two ends). `None` for a shape that touches at a point whatever its pose — a sphere, a capsule
/// end-on, a cone on its apex — and for the rest when a point is what faces `dir`.
///
/// A round shape at rest on a floor touches it along one of these, and GJK/EPA reports one point of
/// it a step. On that one point a cylinder or a lying capsule rocked and sank: at 60 Hz, 7 to 14 cm
/// into a plane or a box, and through a triangle mesh altogether (2026-11-25).
pub(super) fn support_feature(shape: &ColliderShape, pos: Vec3, rot: Quat, dir: Vec3) -> Option<Vec<Vec3>> {
    /// How far off a round shape's axis `dir` may lean (as `|dir · axis|`, the cosine) for its flat
    /// end to face it rather than its side: past 60°, the side does.
    const END_ON: f32 = 0.5;
    let d = (rot.inverse() * dir).normalize_or_zero();
    if d == Vec3::ZERO {
        return None;
    }
    // Eight points round a circle of `radius` at height `y`, and the point of the rim nearest `d`.
    let rim = |radius: f32, y: f32| -> Vec<Vec3> {
        (0..8)
            .map(|k| {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                Vec3::new(radius * a.cos(), y, radius * a.sin())
            })
            .collect()
    };
    let towards = |radius: f32| -> Option<Vec3> {
        let side = Vec3::new(d.x, 0.0, d.z).normalize_or_zero();
        (side != Vec3::ZERO).then_some(side * radius)
    };
    let local: Vec<Vec3> = match shape {
        ColliderShape::Box(_) | ColliderShape::ConvexHull(_) => return polyhedral_face(shape, pos, rot, dir).map(|f| f.corners),
        ColliderShape::Cylinder(c) => {
            if d.y.abs() >= END_ON {
                rim(c.radius, c.half_height * d.y.signum())
            } else {
                let side = towards(c.radius)?;
                vec![side + Vec3::Y * c.half_height, side - Vec3::Y * c.half_height]
            }
        }
        ColliderShape::Cone(c) => {
            if d.y <= -END_ON {
                rim(c.radius, -c.half_height)
            } else {
                let side = towards(c.radius)?;
                vec![Vec3::Y * c.half_height, side - Vec3::Y * c.half_height]
            }
        }
        ColliderShape::Capsule(c) => {
            if d.y.abs() >= END_ON {
                return None;
            }
            // The two end spheres' points facing `d`: the line the capsule lies along.
            let out = d * c.radius;
            vec![Vec3::Y * c.half_height + out, -Vec3::Y * c.half_height + out]
        }
        _ => return None,
    };
    Some(local.into_iter().map(|p| pos + rot * p).collect())
}

/// The contact patch of two polyhedra touching along `normal` (A towards B, from GJK/EPA): the face
/// of either most aligned with it is the reference, the other's most opposed face is clipped to the
/// reference face's sides, and what of it lies behind the reference face is in contact — up to four
/// points. Empty when either shape has no faces or nothing survives, and the caller keeps EPA's one
/// point then.
///
/// GJK/EPA gives one point a step. A hull resting on a face had one support at a time, wandering
/// between its corners, and rocked for ever: not one of six debris-shaped hulls dropped on a floor
/// slept in ten seconds, each still turning at 0.02–0.15 rad/s (2026-11-24).
pub(super) fn clip_polyhedra(
    (shape_a, pos_a, rot_a): (&ColliderShape, Vec3, Quat),
    (shape_b, pos_b, rot_b): (&ColliderShape, Vec3, Quat),
    normal: Vec3,
) -> Vec<ContactPoint> {
    let (reference, incident) = match (polyhedral_face(shape_a, pos_a, rot_a, normal), polyhedral_face(shape_b, pos_b, rot_b, -normal)) {
        (Some(face_a), Some(face_b)) => {
            if face_a.normal.dot(normal) >= face_b.normal.dot(-normal) {
                (face_a, face_b.corners)
            } else {
                (face_b, face_a.corners)
            }
        }
        // A round shape against a box or a hull: the polyhedron's face is the reference, and what of
        // the round shape faces it (`support_feature`) is clipped to it.
        (Some(face_a), None) => match support_feature(shape_b, pos_b, rot_b, -normal) {
            Some(feature) => (face_a, feature),
            None => return Vec::new(),
        },
        (None, Some(face_b)) => match support_feature(shape_a, pos_a, rot_a, normal) {
            Some(feature) => (face_b, feature),
            None => return Vec::new(),
        },
        (None, None) => return Vec::new(),
    };
    clip_to_reference(incident, &reference.corners, reference.normal, reference.offset, normal)
}

/// The patch of a shape on one triangle of a surface — a mesh's or a terrain's — whose face it
/// touches along `normal` (the shape towards the surface, the triangle's face normal reversed): what
/// of the shape faces `normal` (`support_feature`), clipped to the triangle's sides, and what
/// of it lies below the triangle's plane, at its depth below it — up to four points. `triangle` is
/// wound with its normal towards the shape. Empty for a shape that touches at a point, and the
/// caller keeps the one point it had.
///
/// A triangle was one convex body to GJK/EPA, which gives one point: a box resting inside a
/// triangle had one support, two across an edge, and rocked on them. At 60 Hz a 10 cm box dropped
/// on a mesh floor took back less than gravity's `g·dt` each step, sank 16 cm a second, and fell
/// through once its centre crossed the plane (2026-11-25); at 240 Hz the four-times-tighter step had
/// hidden it.
pub(super) fn clip_on_triangle(shape: &ColliderShape, pos: Vec3, rot: Quat, triangle: [Vec3; 3], normal: Vec3) -> Vec<ContactPoint> {
    let Some(incident) = support_feature(shape, pos, rot, normal) else {
        return Vec::new();
    };
    let up = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]).normalize_or_zero();
    if up == Vec3::ZERO {
        return Vec::new();
    }
    clip_to_reference(incident, &triangle, up, up.dot(triangle[0]), normal)
}

/// `polygon` (the incident face's corners) clipped to the sides of the `reference` face — its
/// corners counter-clockwise about `ref_n`, its outward normal, with `ref_n · x = ref_offset` on
/// it — and the points of what is left that lie behind the reference face, as contacts along
/// `normal` at their depth behind it: up to four.
fn clip_to_reference(mut polygon: Vec<Vec3>, reference: &[Vec3], ref_n: Vec3, ref_offset: f32, normal: Vec3) -> Vec<ContactPoint> {
    const SLAB_TOLERANCE: f32 = 1e-3;
    const DEPTH_TOLERANCE: f32 = 1e-4;
    let k = reference.len();
    for i in 0..k {
        let (a, b) = (reference[i], reference[(i + 1) % k]);
        let side = (b - a).cross(ref_n).normalize_or_zero();
        if side == Vec3::ZERO {
            continue;
        }
        let offset = side.dot(a) + SLAB_TOLERANCE;
        let mut out = Vec::with_capacity(polygon.len() + 1);
        for (j, &p) in polygon.iter().enumerate() {
            let q = polygon[(j + 1) % polygon.len()];
            let (dp, dq) = (p.dot(side) - offset, q.dot(side) - offset);
            if dp <= 0.0 {
                out.push(p);
            }
            if (dp <= 0.0) != (dq <= 0.0) {
                out.push(p + (q - p) * (dp / (dp - dq)));
            }
        }
        polygon = out;
        if polygon.is_empty() {
            return Vec::new();
        }
    }
    // A point's depth: how far behind the reference face it is, along `normal` (the reference
    // face's normal is the best aligned of its faces, so the division never gets near zero).
    let along = ref_n.dot(normal).abs().max(0.5);
    let contacts: Vec<ContactPoint> = polygon
        .into_iter()
        .filter_map(|p| {
            let behind = ref_offset - p.dot(ref_n);
            let depth = patch_depth(behind, DEPTH_TOLERANCE)?;
            Some(mk_contact(p, normal, depth / along))
        })
        .collect();
    // A side's crossing comes out twice when the incident feature is a line (two points): one
    // contact of it.
    let mut contacts = contacts;
    contacts.dedup_by(|a, b| a.point.distance_squared(b.point) < 1e-12);
    if contacts.len() > 1 && contacts[0].point.distance_squared(contacts[contacts.len() - 1].point) < 1e-12 {
        contacts.pop();
    }
    select_4_contacts(contacts)
}

/// The contacts of a shape with a plane (`normal · x = offset`, the solid side behind it): every
/// point of what faces the plane (`support_feature`) that is behind it, at the plane, and the
/// shape's deepest point, which a rim's eight points can miss — up to four. Empty when nothing is
/// behind the plane; `None` for a shape that touches at a point, and the caller keeps its one point.
pub(super) fn feature_on_plane(shape: &ColliderShape, pos: Vec3, rot: Quat, normal: Vec3, offset: f32, deepest: ContactPoint) -> Option<Vec<ContactPoint>> {
    let feature = support_feature(shape, pos, rot, -normal)?;
    let mut contacts = vec![deepest];
    for p in feature {
        let behind = offset - p.dot(normal);
        let Some(depth) = patch_depth(behind, 0.0) else { continue };
        // On the plane: a point about to touch is put where it will.
        let on = p + normal * behind;
        if !contacts.iter().any(|c| c.point.distance_squared(on) < 1e-12) {
            contacts.push(mk_contact(on, -normal, depth));
        }
    }
    Some(select_4_contacts(contacts))
}
