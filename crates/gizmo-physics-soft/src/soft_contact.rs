//! Soft bodies touching soft bodies: a node of one that is inside the other is put back on the
//! other's surface.
//!
//! Each [`SoftBodyMesh`] only ever met rigid colliders, so a jelly dropped on a jelly fell
//! straight through it. Here, for every ordered pair of bodies whose bounds overlap, every node of
//! the second that lies inside a tetrahedron of the first is moved to the nearest point of the
//! first's **surface** (the faces that belong to one tetrahedron only), and the triangle it lands
//! on gives way by its share — both by their inverse masses, the triangle's spread over its three
//! corners by the barycentric weights of the point. Any velocity of the node *into* that surface,
//! relative to the surface's own, is taken out by an equal and opposite impulse between the node
//! and the corners, so momentum is conserved exactly.

use crate::soft_body::SoftBodyMesh;
use gizmo_math::{Mat3, Vec3};

/// The faces of `body` that belong to exactly one tetrahedron, each wound so its normal
/// `(b − a) × (c − a)` points out of the body.
pub fn surface_faces(body: &SoftBodyMesh) -> Vec<[u32; 3]> {
    use std::collections::HashMap;
    // Key: the sorted corners. Value: (how many tetrahedra share it, the outward winding).
    let mut faces: HashMap<[u32; 3], (u32, [u32; 3])> = HashMap::new();
    for e in &body.elements {
        let t = e.node_indices;
        for (face, opposite) in [([t[0], t[1], t[2]], t[3]), ([t[0], t[1], t[3]], t[2]), ([t[0], t[2], t[3]], t[1]), ([t[1], t[2], t[3]], t[0])] {
            let p = |i: u32| body.nodes[i as usize].position;
            let (a, b, c) = (p(face[0]), p(face[1]), p(face[2]));
            let outward = if (b - a).cross(c - a).dot(p(opposite) - a) > 0.0 { [face[0], face[2], face[1]] } else { face };
            let mut key = face;
            key.sort_unstable();
            faces.entry(key).and_modify(|f| f.0 += 1).or_insert((1, outward));
        }
    }
    let mut out: Vec<[u32; 3]> = faces.into_values().filter(|(n, _)| *n == 1).map(|(_, f)| f).collect();
    out.sort_unstable(); // HashMap order is not replay-stable; the contacts below must be.
    out
}

fn bounds(body: &SoftBodyMesh) -> (Vec3, Vec3) {
    body.nodes.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), n| (lo.min(n.position), hi.max(n.position)))
}

/// Closest point to `p` on triangle `abc`, and its barycentric weights `(wa, wb, wc)`.
fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> (Vec3, [f32; 3]) {
    // Ericson, Real-Time Collision Detection §5.1.5.
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, [1.0, 0.0, 0.0]);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return (b, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, [1.0 - v, v, 0.0]);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return (c, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + (c - b) * w, [0.0, 1.0 - w, w]);
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    (a + ab * v + ac * w, [1.0 - v - w, v, w])
}

/// Barycentric weights of `p` (on the triangle's plane) in triangle `abc`.
fn barycentric(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<[f32; 3]> {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
    let denom = d00 * d11 - d01 * d01;
    if denom.abs() < 1e-12 {
        return None;
    }
    let v = (d11 * d20 - d01 * d21) / denom;
    let w = (d00 * d21 - d01 * d20) / denom;
    Some([1.0 - v - w, v, w])
}

/// Coulomb friction between two soft bodies: the most of a contact's slip one push can stop, as
/// a multiple of that push.
const FRICTION: f32 = 0.6;

/// How much deeper than the shallowest a way out may be and still be chosen for the node's motion.
const TIE: f32 = 0.005;

fn inverse_mass(body: &SoftBodyMesh, i: usize) -> f32 {
    let n = &body.nodes[i];
    if n.is_fixed || n.mass.is_nan() || n.mass <= 0.0 {
        0.0
    } else {
        1.0 / n.mass
    }
}

/// Pushes the nodes of `b` that are inside `a` out onto `a`'s surface, and returns how many were.
fn push_out(a: &mut SoftBodyMesh, a_faces: &[[u32; 3]], b: &mut SoftBodyMesh) -> usize {
    let (lo, hi) = bounds(a);
    // Each tetrahedron of `a` as (corner, inverse edge matrix, bounds) for the inside test.
    let tets: Vec<(Vec3, Mat3, Vec3, Vec3)> = a
        .elements
        .iter()
        .filter_map(|e| {
            let p = e.node_indices.map(|i| a.nodes[i as usize].position);
            let m = Mat3::from_cols(p[1] - p[0], p[2] - p[0], p[3] - p[0]);
            let det = m.determinant();
            (det.abs() > 1e-12).then(|| (p[0], m.inverse(), p[0].min(p[1]).min(p[2]).min(p[3]), p[0].max(p[1]).max(p[2]).max(p[3])))
        })
        .collect();
    let mut pushed = 0;
    for j in 0..b.nodes.len() {
        let wn = inverse_mass(b, j);
        let p = b.nodes[j].position;
        if p.cmplt(lo).any() || p.cmpgt(hi).any() {
            continue;
        }
        let inside = tets.iter().any(|(p0, inv, tlo, thi)| {
            if p.cmplt(*tlo).any() || p.cmpgt(*thi).any() {
                return false;
            }
            let u = *inv * (p - *p0);
            u.x >= 0.0 && u.y >= 0.0 && u.z >= 0.0 && u.x + u.y + u.z <= 1.0
        });
        if !inside {
            continue;
        }
        // Where the node leaves `a`. Every face it is behind (or on) and whose plane it projects
        // into is a way out, straight along that face's normal; the way out is the shallowest —
        // and among those within `TIE` of the shallowest, the one the node is moving into
        // fastest.
        //
        // Not "the nearest point of the surface", and not "the shallowest" alone. Blocks built
        // on the same grid put the upper one's nodes ON the lower one's corners and in its side
        // planes: the nearest point was that corner at no distance, the shallowest face a side
        // at rounding-noise depth, and either way nothing pushed the node up — it sank, and
        // dragged its block in after it, or a deeper side face was taken and the push blew the
        // pair apart. The face a falling node is falling through is the one it moves into.
        let vb = b.nodes[j].velocity;
        let mut candidates: Vec<(f32, [u32; 3], Vec3, [f32; 3], f32)> = Vec::new();
        for f in a_faces {
            let (fa, fb, fc) = (a.nodes[f[0] as usize].position, a.nodes[f[1] as usize].position, a.nodes[f[2] as usize].position);
            let normal = (fb - fa).cross(fc - fa).normalize_or_zero();
            let depth = -(p - fa).dot(normal);
            if normal == Vec3::ZERO || depth < -1e-5 {
                continue; // degenerate, or the node is in front of this face
            }
            let on_plane = p + normal * depth;
            let Some(w) = barycentric(on_plane, fa, fb, fc) else { continue };
            if w.iter().any(|&x| x < -1e-3) {
                continue;
            }
            let w = w.map(|x| x.max(0.0));
            let sum = w[0] + w[1] + w[2];
            let w = w.map(|x| x / sum);
            let face_v = a.nodes[f[0] as usize].velocity * w[0] + a.nodes[f[1] as usize].velocity * w[1] + a.nodes[f[2] as usize].velocity * w[2];
            candidates.push((depth.max(0.0), *f, on_plane, w, (vb - face_v).dot(normal)));
        }
        let shallowest = candidates.iter().map(|c| c.0).fold(f32::MAX, f32::min);
        let mut best: Option<(f32, [u32; 3], Vec3, [f32; 3])> = candidates
            .iter()
            .filter(|c| c.0 <= shallowest + TIE)
            .min_by(|x, y| x.4.total_cmp(&y.4))
            .map(|c| (c.0, c.1, c.2, c.3));
        if best.is_none() {
            for f in a_faces {
                let (fa, fb, fc) = (a.nodes[f[0] as usize].position, a.nodes[f[1] as usize].position, a.nodes[f[2] as usize].position);
                let (q, w) = closest_on_triangle(p, fa, fb, fc);
                let d = (q - p).length();
                if best.is_none_or(|(bd, ..)| d < bd) {
                    best = Some((d, *f, q, w));
                }
            }
        }
        let Some((_, face, q, bary)) = best else { continue };
        let corners = face.map(|i| i as usize);
        let wf: [f32; 3] = [inverse_mass(a, corners[0]), inverse_mass(a, corners[1]), inverse_mass(a, corners[2])];
        let total = wn + bary[0] * bary[0] * wf[0] + bary[1] * bary[1] * wf[1] + bary[2] * bary[2] * wf[2];
        if total.is_nan() || total <= 0.0 {
            continue;
        }
        pushed += 1;
        if std::env::var_os("SC_TRACE").is_some() && pushed <= 3 {
            let (fa, fb, fc) = (a.nodes[face[0] as usize].position, a.nodes[face[1] as usize].position, a.nodes[face[2] as usize].position);
            eprintln!("  node {j} p {p:.3} -> q {q:.3} depth {:.4} n {:.2} v {:.3}", (q - p).length(), (fb - fa).cross(fc - fa).normalize_or_zero(), b.nodes[j].velocity);
        }
        // Position: the node and the triangle meet at the surface, each moving by its share.
        let d = q - p;
        b.nodes[j].position += d * (wn / total);
        for k in 0..3 {
            a.nodes[corners[k]].position -= d * (bary[k] * wf[k] / total);
        }
        // Velocity: the part of the node's velocity into the surface, relative to the surface.
        let (fa, fb, fc) = (a.nodes[corners[0]].position, a.nodes[corners[1]].position, a.nodes[corners[2]].position);
        let normal = (fb - fa).cross(fc - fa).normalize_or_zero();
        if normal == Vec3::ZERO {
            continue;
        }
        let surface_v = a.nodes[corners[0]].velocity * bary[0] + a.nodes[corners[1]].velocity * bary[1] + a.nodes[corners[2]].velocity * bary[2];
        let vn = (b.nodes[j].velocity - surface_v).dot(normal);
        if vn < 0.0 {
            let lambda = -vn / total; // the impulse, N·s
            b.nodes[j].velocity += normal * (lambda * wn);
            for k in 0..3 {
                a.nodes[corners[k]].velocity -= normal * (lambda * bary[k] * wf[k]);
            }
            // Friction, Coulomb: the sliding is slowed by at most `FRICTION` times the push.
            // Without it a block resting on another slid off it: measured, a 1 m jelly drifted
            // 0.8 m sideways in five seconds on the small tilts the contact itself makes, then fell.
            let surface_v = a.nodes[corners[0]].velocity * bary[0] + a.nodes[corners[1]].velocity * bary[1] + a.nodes[corners[2]].velocity * bary[2];
            let rel = b.nodes[j].velocity - surface_v;
            let slip = rel - normal * rel.dot(normal);
            let speed = slip.length();
            if speed > 1e-9 {
                let along = slip / speed;
                let tangential = (speed / total).min(FRICTION * lambda);
                b.nodes[j].velocity -= along * (tangential * wn);
                for k in 0..3 {
                    a.nodes[corners[k]].velocity += along * (tangential * bary[k] * wf[k]);
                }
            }
        }
    }
    pushed
}

/// Resolves every contact between the soft bodies in `bodies`, both ways round for each
/// overlapping pair. Returns how many nodes were pushed out.
pub fn resolve_soft_contacts(bodies: &mut [&mut SoftBodyMesh]) -> usize {
    if bodies.len() < 2 {
        return 0;
    }
    let faces: Vec<Vec<[u32; 3]>> = bodies.iter().map(|b| surface_faces(b)).collect();
    let mut pushed = 0;
    for i in 0..bodies.len() {
        for j in 0..bodies.len() {
            if i == j {
                continue;
            }
            let ((ilo, ihi), (jlo, jhi)) = (bounds(bodies[i]), bounds(bodies[j]));
            if ilo.cmpgt(jhi).any() || jlo.cmpgt(ihi).any() {
                continue;
            }
            let (a, b) = if i < j {
                let (l, r) = bodies.split_at_mut(j);
                (&mut *l[i], &mut *r[0])
            } else {
                let (l, r) = bodies.split_at_mut(i);
                (&mut *r[0], &mut *l[j])
            };
            pushed += push_out(a, &faces[i], b);
        }
    }
    pushed
}
