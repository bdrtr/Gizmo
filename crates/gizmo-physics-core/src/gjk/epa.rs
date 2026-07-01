use super::*;

impl Gjk {
    /// EPA (Expanding Polytope Algorithm) for contact information
    pub(crate) fn epa(
        mut simplex: Vec<SupportPoint>,
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: gizmo_math::Quat,
    ) -> Option<ContactPoint> {
        let support = |dir: Vec3| {
            let sa = Self::support_point(shape_a, pos_a, rot_a, dir);
            let sb = Self::support_point(shape_b, pos_b, rot_b, -dir);
            SupportPoint { v: sa - sb, a: sa, b: sb }
        };

        let mut faces: Vec<(usize, usize, usize)> = Vec::new();
        let mut edges = Vec::new();

        if simplex.len() == 4 {
            // Wind every initial face OUTWARD relative to the tetrahedron's
            // opposite (interior) vertex — a purely geometric test, NOT relative
            // to the origin. For shallow contacts the origin can lie on or just
            // outside a face, which would make an origin-based orientation test
            // flip the normal the wrong way; the 4th-vertex test never does.
            // Each tuple is (a, b, c, opposite) where `opposite` is the lone
            // vertex not on the face.
            let initial_faces = [(0, 1, 2, 3), (0, 3, 1, 2), (0, 2, 3, 1), (1, 3, 2, 0)];
            for (a, b, c, opp) in initial_faces {
                let n = Self::compute_face_normal(&simplex, a, b, c);
                // If the winding normal points TOWARD the interior vertex, the
                // face is wound inward — swap two vertices to flip it outward.
                if n.dot(simplex[opp].v - simplex[a].v) > 0.0 {
                    faces.push((a, c, b));
                } else {
                    faces.push((a, b, c));
                }
            }
        } else {
            return None;
        }

        for _ in 0..EPA_MAX_ITERATIONS {
            let (_closest_face_idx, normal, distance) = Self::find_closest_face(&simplex, &faces)?;

            let support_point = support(normal);
            let support_distance = support_point.v.dot(normal);

            if support_distance - distance < EPA_TOLERANCE {
                break;
            }

            let new_point_idx = simplex.len();
            simplex.push(support_point);

            edges.clear();
            let mut i = 0;
            while i < faces.len() {
                let (a, b, c) = faces[i];
                let face_normal = Self::compute_face_normal(&simplex, a, b, c);
                let to_point = simplex[new_point_idx].v - simplex[a].v;

                if face_normal.dot(to_point) > 0.0 {
                    Self::add_edge(&mut edges, a, b);
                    Self::add_edge(&mut edges, b, c);
                    Self::add_edge(&mut edges, c, a);
                    faces.swap_remove(i);
                } else {
                    i += 1;
                }
            }

            // Stitch new faces from the horizon edges to the new vertex. The
            // surviving directed edges (after add_edge cancelled the shared
            // interior edges) wind consistently around the hole, so each new
            // face (e1 → e2 → new_point) inherits the correct OUTWARD orientation
            // from the faces it replaced — no origin-based flip needed.
            for (e1, e2) in &edges {
                faces.push((*e1, *e2, new_point_idx));
            }
        }

        let (closest_idx, normal, penetration) = Self::find_closest_face(&simplex, &faces)?;

        // Temas noktası: en yakın EPA yüzündeki Minkowski köşelerinin origin'e en
        // yakın noktasının barycentric ağırlıkları, AYNI köşelerin SAKLANAN witness
        // (support) noktalarına uygulanır. Bu, temas noktasını her iki yüzeyde de
        // doğru özelliğe (köşe/kenar/yüz) yerleştirir.
        //
        // Witness'ler taşınmadan önce support yönleri Minkowski köşelerinden "tahmin"
        // ediliyordu — bu anlamsızdı ve teması yanlış yere (ör. tekerlek merkezine)
        // koyabiliyordu. Artık doğru.
        let (fa, fb, fc) = faces[closest_idx];
        let sa = simplex[fa];
        let sb = simplex[fb];
        let sc = simplex[fc];

        // Origin'in yüze izdüşümü = normal * penetration (en yakın yüz origin'den bu uzaklıkta).
        let closest_on_face = normal * penetration;
        let contact_point = match Self::barycentric_coords(sa.v, sb.v, sc.v, closest_on_face) {
            Some((u, v, w)) => {
                let pt_a = sa.a * u + sb.a * v + sc.a * w; // A yüzeyindeki temas
                let pt_b = sa.b * u + sb.b * v + sc.b * w; // B yüzeyindeki temas
                (pt_a + pt_b) * 0.5
            }
            None => {
                // Dejenere yüz: deepest-support orta-noktasına düş.
                let pt_a = Self::support_point(shape_a, pos_a, rot_a, -normal);
                let pt_b = Self::support_point(shape_b, pos_b, rot_b, normal);
                (pt_a + pt_b) * 0.5
            }
        };

        Some(ContactPoint {
            point: contact_point,
            normal,
            penetration,
            local_point_a: contact_point - pos_a,
            local_point_b: contact_point - pos_b,
            normal_impulse: 0.0,
            tangent_impulse: Vec3::ZERO,
        })
    }

    /// Barycentric coordinates of point p projected onto triangle (a, b, c).
    /// Returns None if the triangle is degenerate.
    fn barycentric_coords(a: Vec3, b: Vec3, c: Vec3, p: Vec3) -> Option<(f32, f32, f32)> {
        // Standard barycentric formula using Cramer's rule.
        let ab = b - a;
        let ac = c - a;
        let ap = p - a;

        let d00 = ab.dot(ab);
        let d01 = ab.dot(ac);
        let d11 = ac.dot(ac);
        let d20 = ap.dot(ab);
        let d21 = ap.dot(ac);

        let denom = d00 * d11 - d01 * d01;
        if denom.abs() < 1e-8 {
            return None;
        }

        let v = (d11 * d20 - d01 * d21) / denom;
        let w = (d00 * d21 - d01 * d20) / denom;
        let u = 1.0 - v - w;

        Some((u, v, w))
    }

    fn find_closest_face(
        simplex: &[SupportPoint],
        faces: &[(usize, usize, usize)],
    ) -> Option<(usize, Vec3, f32)> {
        let mut min_distance = f32::INFINITY;
        let mut closest_idx = 0;
        let mut closest_normal = Vec3::ZERO;

        for (i, &(a, b, c)) in faces.iter().enumerate() {
            let normal = Self::compute_face_normal(simplex, a, b, c);
            let distance = normal.dot(simplex[a].v);

            if distance < min_distance {
                min_distance = distance;
                closest_idx = i;
                closest_normal = normal;
            }
        }

        if min_distance == f32::INFINITY {
            None
        } else {
            Some((closest_idx, closest_normal, min_distance))
        }
    }

    /// Normal of the face from its STORED winding order (a → b → c), via the
    /// right-hand rule — with NO origin-based flipping.
    ///
    /// The polytope keeps a consistent OUTWARD winding by construction: the
    /// initial tetrahedron winds every face away from its opposite (interior)
    /// vertex, and each face created during expansion inherits its orientation
    /// from the directed horizon edges. So the winding normal already points
    /// outward. The previous implementation re-derived orientation from
    /// `normal_raw · v_a` ("away from origin"); for shallow / grazing contacts
    /// the origin sits on — or just outside — the closest face, making that
    /// sign test unreliable. It could then flip the contact normal inward
    /// (objects pulled together instead of pushed apart) or, during expansion,
    /// mislabel which faces "see" the new support point and corrupt the
    /// polytope. Winding order is purely geometric and immune to that.
    pub(crate) fn compute_face_normal(simplex: &[SupportPoint], a: usize, b: usize, c: usize) -> Vec3 {
        let ab = simplex[b].v - simplex[a].v;
        let ac = simplex[c].v - simplex[a].v;
        ab.cross(ac).try_normalize().unwrap_or(Vec3::X)
    }

    fn add_edge(edges: &mut Vec<(usize, usize)>, a: usize, b: usize) {
        let reverse = (b, a);
        let forward = (a, b);

        if let Some(pos) = edges.iter().position(|&e| e == reverse) {
            edges.swap_remove(pos);
        } else if let Some(pos) = edges.iter().position(|&e| e == forward) {
            edges.swap_remove(pos);
        } else {
            edges.push((a, b));
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression for finding 35: `barycentric_coords` used to compute a first,
    // discarded set of dot products (with an incorrect denominator based on the wrong
    // vectors) before recomputing the real Cramer's-rule solution. With the dead block
    // removed the standard formula must still produce correct, verifiable coordinates.

    #[test]
    fn barycentric_coords_recovers_known_point() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(1.0, 0.0, 0.0);
        let c = Vec3::new(0.0, 1.0, 0.0);

        // Interior point reconstructed from known weights must round-trip.
        let (eu, ev, ew) = (0.5, 0.3, 0.2);
        let p = a * eu + b * ev + c * ew;

        let (u, v, w) = Gjk::barycentric_coords(a, b, c, p).expect("non-degenerate triangle");
        assert!((u - eu).abs() < 1e-5, "u={u}");
        assert!((v - ev).abs() < 1e-5, "v={v}");
        assert!((w - ew).abs() < 1e-5, "w={w}");
        assert!((u + v + w - 1.0).abs() < 1e-5);
    }

    #[test]
    fn barycentric_coords_rejects_degenerate_triangle() {
        // Collinear (zero-area) triangle → None, not a NaN-laden coordinate triple.
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(1.0, 0.0, 0.0);
        let c = Vec3::new(2.0, 0.0, 0.0);
        assert!(Gjk::barycentric_coords(a, b, c, Vec3::new(0.5, 0.0, 0.0)).is_none());
    }
}
