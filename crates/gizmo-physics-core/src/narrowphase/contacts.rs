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

/// Returns `true` when `normal` is well-aligned with one of the box axes,
/// indicating that a face (rather than an edge) is the contact feature.
///
/// `threshold` should be `1/√2 ≈ 0.707` to correctly handle 45° cases.
#[inline]
pub(super) fn is_face_axis(normal: Vec3, axes: &[Vec3; 3], threshold: f32) -> bool {
    axes.iter().any(|a| a.dot(normal).abs() > threshold)
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

    let contacts: Vec<ContactPoint> = box_corners(inc_pos, inc_rot, inc_h)
        .iter()
        .filter_map(|&corner| {
            // 1. Corner must be on or behind the reference face (depth along `normal`).
            let signed_depth = ref_face_d - corner.dot(normal);
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
