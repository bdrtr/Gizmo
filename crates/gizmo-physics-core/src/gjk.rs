use crate::collision::ContactPoint;
use crate::components::{BoxShape, CapsuleShape, ColliderShape, SphereShape};
use gizmo_math::Vec3;

const EPA_TOLERANCE: f32 = 0.001;
const EPA_MAX_ITERATIONS: usize = 32;

/// GJK/EPA simpleksindeki tek bir köşe: Minkowski-fark noktası ve onu üreten
/// her iki şekildeki destek (witness) noktaları. Witness'ler EPA sonunda doğru
/// temas noktasını barycentric olarak geri kurmak için taşınır — aksi halde
/// temas noktası yanlış özelliğe (ör. tekerlek merkezine) düşebiliyordu.
#[derive(Clone, Copy)]
struct SupportPoint {
    /// Minkowski farkı: support_a(d) - support_b(-d)
    v: Vec3,
    /// A şekli üzerindeki destek noktası (witness)
    a: Vec3,
    /// B şekli üzerindeki destek noktası (witness)
    b: Vec3,
}

pub struct Gjk;

impl Gjk {
    /// Test if two shapes are colliding using GJK
    pub fn test_collision(
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: gizmo_math::Quat,
    ) -> bool {
        let support = |dir: Vec3| {
            let sa = Self::support_point(shape_a, pos_a, rot_a, dir);
            let sb = Self::support_point(shape_b, pos_b, rot_b, -dir);
            SupportPoint { v: sa - sb, a: sa, b: sb }
        };

        Self::gjk_with_simplex(support).is_some()
    }

    /// Get contact information using GJK + EPA
    pub fn get_contact(
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

        if let Some(simplex) = Self::gjk_with_simplex(support) {
            if let Some(contact) = Self::epa(simplex, shape_a, pos_a, rot_a, shape_b, pos_b, rot_b)
            {
                Some(contact)
            } else {
                // EPA failed (likely degenerate simplex), but we KNOW they intersect.
                // Return a basic contact point for triggers and solver fallback.
                Some(ContactPoint {
                    point: (pos_a + pos_b) * 0.5,
                    normal: (pos_b - pos_a).try_normalize().unwrap_or(Vec3::Y),
                    penetration: 0.01,
                    ..Default::default()
                })
            }
        } else {
            None
        }
    }

    /// GJK that returns the final simplex for EPA
    fn gjk_with_simplex<F>(support: F) -> Option<Vec<SupportPoint>>
    where
        F: Fn(Vec3) -> SupportPoint,
    {
        let mut simplex: Vec<SupportPoint> = Vec::with_capacity(4);
        let mut direction = Vec3::new(1.0, 0.0, 0.0);

        // First point
        simplex.push(support(direction));
        direction = -simplex[0].v;

        // FIX 3: More robust degenerate direction fallback — derive from existing simplex
        // rather than always falling back to Vec3::X which can be parallel to the simplex
        const MAX_ITERATIONS: usize = 32;
        for _ in 0..MAX_ITERATIONS {
            direction = direction.try_normalize().unwrap_or_else(|| {
                // Derive a perpendicular direction from the current simplex
                if simplex.len() >= 2 {
                    let edge = simplex[simplex.len() - 1].v - simplex[0].v;
                    let perp = if edge.x.abs() <= edge.y.abs() && edge.x.abs() <= edge.z.abs() {
                        Vec3::new(1.0, 0.0, 0.0)
                    } else if edge.y.abs() <= edge.z.abs() {
                        Vec3::new(0.0, 1.0, 0.0)
                    } else {
                        Vec3::new(0.0, 0.0, 1.0)
                    };
                    edge.cross(perp).try_normalize().unwrap_or(Vec3::X)
                } else {
                    Vec3::X
                }
            });

            let a = support(direction);

            if a.v.dot(direction) < 0.0 {
                return None; // No collision
            }

            simplex.push(a);

            if Self::handle_simplex(&mut simplex, &mut direction) {
                return Some(simplex); // Collision detected
            }
        }

        None
    }

    /// Compute distance and closest points using GJK (for non-intersecting shapes)
    /// Returns (distance, normal_from_b_to_a)
    pub fn distance<F>(support: F) -> Option<(f32, Vec3)>
    where
        F: Fn(Vec3) -> Vec3,
    {
        let mut simplex = Vec::with_capacity(4);
        let mut direction = Vec3::X;

        let p = support(direction);
        simplex.push(p);

        let mut closest_point = p;
        let mut min_dist_sq = p.length_squared();

        for _ in 0..32 {
            if min_dist_sq < 1e-8 {
                return Some((0.0, Vec3::X)); // Intersecting, use fallback normal
            }

            direction = -closest_point;
            let a = support(direction.normalize());

            // Convergence check: how much closer can we get in this direction?
            let current_dist = min_dist_sq.sqrt();
            let lower_bound = -a.dot(direction) / current_dist;
            if current_dist - lower_bound < 0.0001 {
                break;
            }

            simplex.push(a);
            closest_point = Self::closest_point_on_simplex(&mut simplex);
            min_dist_sq = closest_point.length_squared();
        }

        let dist = min_dist_sq.sqrt();
        let normal = if dist > 1e-6 {
            closest_point / dist
        } else {
            Vec3::X
        };
        Some((dist, normal))
    }

    fn closest_point_on_simplex(simplex: &mut Vec<Vec3>) -> Vec3 {
        match simplex.len() {
            1 => simplex[0],
            2 => {
                let b = simplex[0];
                let a = simplex[1];
                let ab = b - a;
                let ao = -a;
                let t = ao.dot(ab) / ab.length_squared().max(1e-8);
                if t <= 0.0 {
                    simplex.remove(0);
                    a
                } else if t >= 1.0 {
                    simplex.remove(1);
                    b
                } else {
                    a + ab * t
                }
            }
            3 => {
                let c = simplex[0];
                let b = simplex[1];
                let a = simplex[2];

                let ab = b - a;
                let ac = c - a;
                let ap = -a;

                let d1 = ab.dot(ap);
                let d2 = ac.dot(ap);
                if d1 <= 0.0 && d2 <= 0.0 {
                    *simplex = vec![a];
                    return a;
                }

                let bp = -b;
                let d3 = ab.dot(bp);
                let d4 = ac.dot(bp);
                if d3 >= 0.0 && d4 <= d3 {
                    *simplex = vec![b];
                    return b;
                }

                let cp = -c;
                let d5 = ab.dot(cp);
                let d6 = ac.dot(cp);
                if d6 >= 0.0 && d5 <= d6 {
                    *simplex = vec![c];
                    return c;
                }

                let vc = d1 * d4 - d3 * d2;
                if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
                    let v = d1 / (d1 - d3);
                    *simplex = vec![b, a];
                    return a + ab * v;
                }

                let vb = d5 * d2 - d1 * d6;
                if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
                    let w = d2 / (d2 - d6);
                    *simplex = vec![c, a];
                    return a + ac * w;
                }

                let va = d3 * d6 - d5 * d4;
                if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
                    let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
                    *simplex = vec![c, b];
                    return b + (c - b) * w;
                }

                let denom = 1.0 / (va + vb + vc);
                let v = vb * denom;
                let w = vc * denom;
                a + ab * v + ac * w
            }
            4 => {
                let d = simplex[0];
                let c = simplex[1];
                let b = simplex[2];
                let a = simplex[3];

                let abc = (b - a).cross(c - a);
                let acd = (c - a).cross(d - a);
                let adb = (d - a).cross(b - a);
                let bdc = (c - b).cross(d - b);

                if abc.dot(-a) > 0.0 {
                    *simplex = vec![c, b, a];
                    return Self::closest_point_on_simplex(simplex);
                }
                if acd.dot(-a) > 0.0 {
                    *simplex = vec![d, c, a];
                    return Self::closest_point_on_simplex(simplex);
                }
                if adb.dot(-a) > 0.0 {
                    *simplex = vec![b, d, a];
                    return Self::closest_point_on_simplex(simplex);
                }
                if bdc.dot(-b) > 0.0 {
                    *simplex = vec![d, c, b];
                    return Self::closest_point_on_simplex(simplex);
                }

                Vec3::ZERO
            }
            _ => Vec3::ZERO,
        }
    }

    /// Exact TOI (Time of Impact) using Conservative Advancement
    pub fn conservative_advancement(
        shape_a: &ColliderShape,
        mut pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        vel_a: Vec3,
        shape_b: &ColliderShape,
        mut pos_b: Vec3,
        rot_b: gizmo_math::Quat,
        vel_b: Vec3,
        max_t: f32,
    ) -> Option<(f32, Vec3)> {
        let mut t = 0.0;
        let rel_vel = vel_a - vel_b;

        if rel_vel.length_squared() < 1e-6 {
            return None;
        }

        for _ in 0..32 {
            let support = |dir: Vec3| {
                let sa = Self::support_point(shape_a, pos_a, rot_a, dir);
                let sb = Self::support_point(shape_b, pos_b, rot_b, -dir);
                sa - sb
            };

            if let Some((dist, normal)) = Self::distance(support) {
                if dist < 0.001 {
                    return Some((t, normal));
                }

                let closing_vel = -rel_vel.dot(normal);

                if closing_vel <= 0.0 {
                    return None;
                }

                let delta_t = dist / closing_vel;
                t += delta_t;

                if t > max_t {
                    return None;
                }

                pos_a += vel_a * delta_t;
                pos_b += vel_b * delta_t;
            } else {
                return None;
            }
        }

        None
    }

    /// Speculative contact for continuous collision detection (CCD).
    ///
    /// When two **separated** shapes are on a collision course this frame, this
    /// emits a contact whose *negative penetration encodes the separation gap*.
    /// The constraint solver reads that gap (`penetration < 0` ⇒ velocity bias
    /// `gap/dt`) and lets the body advance **exactly up to the surface this step,
    /// never past it** — instead of tunnelling through (no constraint) or freezing
    /// far short (the old `penetration = 0` behaviour, which stopped the body at
    /// its start-of-frame position).
    ///
    /// The body is intentionally halted a hair (`SKIN`) short of contact so the
    /// *next* frame still measures a clean, GJK-reliable gap and converges to a
    /// full stop without ever overlapping.
    ///
    /// The normal is oriented A→B. The contact is anchored at the **inverse-mass-
    /// weighted centre** of the two bodies — which collapses onto the *dynamic*
    /// body's centre of mass when the other is static. That makes the dynamic
    /// body's lever arm `r × n ≈ 0`, so the impulse is a pure translational stop
    /// with no spurious spin (anchoring on the static body instead would give the
    /// far-away dynamic body a huge lever arm and a near-useless impulse). This
    /// targets *translational* tunnelling; angular sweeps and the residual
    /// rotational coupling between two far-apart fast dynamic bodies are out of scope.
    ///
    /// `inv_mass_a` / `inv_mass_b` are the bodies' inverse masses (0 for static).
    pub fn speculative_contact(
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        vel_a: Vec3,
        inv_mass_a: f32,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: gizmo_math::Quat,
        vel_b: Vec3,
        inv_mass_b: f32,
        dt: f32,
    ) -> Option<ContactPoint> {
        /// Resting standoff: the body stops this far short of the surface so the
        /// next frame still has a measurable, GJK-reliable separation.
        const SKIN: f32 = 0.01;
        /// Below this gap the GJK separating axis is unreliable (it degenerates to
        /// a `Vec3::X` fallback), so we orient the normal from the approach instead.
        const AXIS_RELIABLE_GAP: f32 = 1e-3;

        if dt <= 0.0 {
            return None;
        }

        // Separation distance + separating axis at the *current* configuration.
        let support = |dir: Vec3| {
            Self::support_point(shape_a, pos_a, rot_a, dir)
                - Self::support_point(shape_b, pos_b, rot_b, -dir)
        };
        let (gap, axis) = Self::distance(support)?;
        let gap = gap.max(0.0);

        let rel_vel = vel_a - vel_b;

        // Contact normal, pointing A→B. `distance` returns the B→A separating axis,
        // so negate it. When the gap is tiny that axis is unreliable, so derive the
        // normal from the approach direction (which equals A→B while A closes on B).
        let normal = if gap > AXIS_RELIABLE_GAP {
            -axis
        } else {
            rel_vel.try_normalize()?
        };

        // Closing speed of A onto B along the normal. Not approaching ⇒ no contact.
        let closing = rel_vel.dot(normal);
        if closing <= 1e-4 {
            return None;
        }

        // Only engage on the step where the bodies actually meet; otherwise a later
        // (closer) frame handles it. This keeps the manifold list minimal and avoids
        // constraining pairs that merely share a fattened broadphase cell.
        if gap > closing * dt {
            return None;
        }

        // How far the solver may let the body close this step (stop SKIN short).
        let allowed_close = (gap - SKIN).max(0.0);

        // Anchor at the inverse-mass-weighted centre: collapses onto the dynamic
        // body when the other is static ⇒ that body's lever arm vanishes ⇒ the
        // normal impulse is a clean linear stop.
        let inv_sum = inv_mass_a + inv_mass_b;
        let point = if inv_sum > 1e-12 {
            (pos_a * inv_mass_a + pos_b * inv_mass_b) / inv_sum
        } else {
            (pos_a + pos_b) * 0.5
        };

        Some(ContactPoint {
            point,
            normal,
            // Negative ⇒ speculative gap; solver bias allows closing exactly this much.
            penetration: -allowed_close,
            local_point_a: point - pos_a,
            local_point_b: point - pos_b,
            normal_impulse: 0.0,
            tangent_impulse: Vec3::ZERO,
        })
    }

    fn handle_simplex(simplex: &mut Vec<SupportPoint>, direction: &mut Vec3) -> bool {
        match simplex.len() {
            2 => Self::line_case(simplex, direction),
            3 => Self::triangle_case(simplex, direction),
            4 => Self::tetrahedron_case(simplex, direction),
            _ => false,
        }
    }

    fn line_case(simplex: &mut Vec<SupportPoint>, direction: &mut Vec3) -> bool {
        let a = simplex[1].v;
        let b = simplex[0].v;

        let ab = b - a;
        let ao = -a;

        if ab.dot(ao) > 0.0 {
            let mut cross = ab.cross(ao);
            if cross.length_squared() < 1e-6 {
                cross = if ab.x.abs() > ab.y.abs() {
                    Vec3::new(ab.y, -ab.x, 0.0)
                } else {
                    Vec3::new(0.0, ab.z, -ab.y)
                };
            }
            *direction = cross.cross(ab);
        } else {
            simplex.remove(0);
            *direction = ao;
        }

        false
    }

    fn triangle_case(simplex: &mut Vec<SupportPoint>, direction: &mut Vec3) -> bool {
        let a = simplex[2];
        let b = simplex[1];
        let c = simplex[0];

        let ab = b.v - a.v;
        let ac = c.v - a.v;
        let ao = -a.v;

        let abc = ab.cross(ac);

        if abc.cross(ac).dot(ao) > 0.0 {
            if ac.dot(ao) > 0.0 {
                *simplex = vec![c, a];
                let mut cross = ac.cross(ao);
                if cross.length_squared() < 1e-6 {
                    cross = if ac.x.abs() > ac.y.abs() {
                        Vec3::new(ac.y, -ac.x, 0.0)
                    } else {
                        Vec3::new(0.0, ac.z, -ac.y)
                    };
                }
                *direction = cross.cross(ac);
            } else {
                *simplex = vec![b, a];
                let mut cross = ab.cross(ao);
                if cross.length_squared() < 1e-6 {
                    cross = if ab.x.abs() > ab.y.abs() {
                        Vec3::new(ab.y, -ab.x, 0.0)
                    } else {
                        Vec3::new(0.0, ab.z, -ab.y)
                    };
                }
                *direction = cross.cross(ab);
            }
        } else if ab.cross(abc).dot(ao) > 0.0 {
            *simplex = vec![b, a];
            let mut cross = ab.cross(ao);
            if cross.length_squared() < 1e-6 {
                cross = if ab.x.abs() > ab.y.abs() {
                    Vec3::new(ab.y, -ab.x, 0.0)
                } else {
                    Vec3::new(0.0, ab.z, -ab.y)
                };
            }
            *direction = cross.cross(ab);
        } else {
            if abc.dot(ao) > 0.0 {
                *direction = abc;
            } else {
                simplex.swap(0, 1);
                *direction = -abc;
            }
        }

        false
    }

    fn tetrahedron_case(simplex: &mut Vec<SupportPoint>, direction: &mut Vec3) -> bool {
        let a = simplex[3].v;
        let b = simplex[2].v;
        let c = simplex[1].v;
        let d = simplex[0].v;

        let ab = b - a;
        let ac = c - a;
        let ad = d - a;
        let ao = -a;

        let abc = ab.cross(ac);
        let acd = ac.cross(ad);
        let adb = ad.cross(ab);

        if abc.dot(ao) > 0.0 {
            simplex.remove(0);
            return Self::triangle_case(simplex, direction);
        }

        if acd.dot(ao) > 0.0 {
            simplex.remove(2);
            simplex.swap(0, 1);
            return Self::triangle_case(simplex, direction);
        }

        if adb.dot(ao) > 0.0 {
            simplex.remove(1);
            return Self::triangle_case(simplex, direction);
        }

        true
    }

    /// EPA (Expanding Polytope Algorithm) for contact information
    fn epa(
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
        let ab = b - a;
        let ac = c - a;

        let d3 = ab.dot(b - a); // = ab·ab
        let d4 = ac.dot(b - a);
        let d5 = ab.dot(c - a);
        let d6 = ac.dot(c - a); // = ac·ac

        let denom = d3 * d6 - d4 * d5;
        if denom.abs() < 1e-8 {
            return None; // Degenerate triangle
        }

        // Correct standard formula using Cramer's rule
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
    fn compute_face_normal(simplex: &[SupportPoint], a: usize, b: usize, c: usize) -> Vec3 {
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

    /// Get support point for a shape in a given direction
    pub fn support_point(
        shape: &ColliderShape,
        pos: Vec3,
        rot: gizmo_math::Quat,
        dir: Vec3,
    ) -> Vec3 {
        let local_dir = rot.inverse() * dir;

        let local_support = match shape {
            ColliderShape::Sphere(s) => Self::sphere_support(s, local_dir),
            ColliderShape::Box(b) => Self::box_support(b, local_dir),
            ColliderShape::Capsule(c) => Self::capsule_support(c, local_dir),
            ColliderShape::Plane(_) => {
                debug_assert!(false, "Plane shapes must use separate collision detection");
                Vec3::ZERO
            }
            ColliderShape::TriMesh(tm) => {
                let mut best_dot = f32::NEG_INFINITY;
                let mut best_pt = Vec3::ZERO;

                if !tm.bvh.nodes.is_empty() {
                    let mut stack = Vec::with_capacity(64);
                    stack.push(0);

                    let abs_dir = gizmo_math::Vec3A::new(
                        local_dir.x.abs(),
                        local_dir.y.abs(),
                        local_dir.z.abs(),
                    );
                    let dir_a = gizmo_math::Vec3A::new(local_dir.x, local_dir.y, local_dir.z);

                    while let Some(node_idx) = stack.pop() {
                        let node = &tm.bvh.nodes[node_idx];

                        let center = node.aabb.center();
                        let half_extents = node.aabb.half_extents();

                        let max_node_dot = center.dot(dir_a)
                            + half_extents.x * abs_dir.x
                            + half_extents.y * abs_dir.y
                            + half_extents.z * abs_dir.z;

                        if max_node_dot < best_dot {
                            continue;
                        }

                        if node.is_leaf() {
                            let start = (node.first_tri_index * 3) as usize;
                            let end = start + (node.tri_count * 3) as usize;
                            for i in start..end {
                                let v = tm.vertices[tm.indices[i] as usize];
                                let d = v.dot(local_dir);
                                if d > best_dot {
                                    best_dot = d;
                                    best_pt = v;
                                }
                            }
                        } else {
                            if node.left_child >= 0 {
                                stack.push(node.left_child as usize);
                            }
                            if node.right_child >= 0 {
                                stack.push(node.right_child as usize);
                            }
                        }
                    }
                } else {
                    for v in tm.vertices.iter() {
                        let d = v.dot(local_dir);
                        if d > best_dot {
                            best_dot = d;
                            best_pt = *v;
                        }
                    }
                }
                best_pt
            }
            ColliderShape::ConvexHull(ch) => {
                let mut best_dot = f32::NEG_INFINITY;
                let mut best_pt = Vec3::ZERO;
                for v in ch.vertices.iter() {
                    let d = v.dot(local_dir);
                    if d > best_dot {
                        best_dot = d;
                        best_pt = *v;
                    }
                }
                best_pt
            }
            crate::components::ColliderShape::Compound(_) => {
                debug_assert!(
                    false,
                    "Compound shapes must use separate collision detection"
                );
                Vec3::ZERO
            }
        };

        pos + rot * local_support
    }

    fn sphere_support(sphere: &SphereShape, dir: Vec3) -> Vec3 {
        dir.try_normalize().unwrap_or(Vec3::X) * sphere.radius
    }

    fn box_support(box_shape: &BoxShape, dir: Vec3) -> Vec3 {
        Vec3::new(
            if dir.x > 0.0 {
                box_shape.half_extents.x
            } else {
                -box_shape.half_extents.x
            },
            if dir.y > 0.0 {
                box_shape.half_extents.y
            } else {
                -box_shape.half_extents.y
            },
            if dir.z > 0.0 {
                box_shape.half_extents.z
            } else {
                -box_shape.half_extents.z
            },
        )
    }

    fn capsule_support(capsule: &CapsuleShape, dir: Vec3) -> Vec3 {
        let dir_normalized = dir.try_normalize().unwrap_or(Vec3::X);
        let sphere_center = if dir_normalized.y > 0.0 {
            Vec3::new(0.0, capsule.half_height, 0.0)
        } else {
            Vec3::new(0.0, -capsule.half_height, 0.0)
        };
        sphere_center + dir_normalized * capsule.radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::{Quat, Vec3};

    #[test]
    fn test_sphere_vs_sphere_collision() {
        let shape = ColliderShape::Sphere(SphereShape { radius: 1.0 });

        assert!(Gjk::test_collision(
            &shape,
            Vec3::ZERO,
            Quat::IDENTITY,
            &shape,
            Vec3::new(1.5, 0.0, 0.0),
            Quat::IDENTITY
        ));
        assert!(!Gjk::test_collision(
            &shape,
            Vec3::ZERO,
            Quat::IDENTITY,
            &shape,
            Vec3::new(2.5, 0.0, 0.0),
            Quat::IDENTITY
        ));
    }

    #[test]
    fn test_box_vs_box_collision() {
        let shape = ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(1.0, 1.0, 1.0),
        });

        assert!(Gjk::test_collision(
            &shape,
            Vec3::ZERO,
            Quat::IDENTITY,
            &shape,
            Vec3::new(1.5, 0.0, 0.0),
            Quat::IDENTITY
        ));
        assert!(!Gjk::test_collision(
            &shape,
            Vec3::ZERO,
            Quat::IDENTITY,
            &shape,
            Vec3::new(2.5, 0.0, 0.0),
            Quat::IDENTITY
        ));
    }

    #[test]
    fn test_epa_contact_generation() {
        let shape_a = ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(1.0, 1.0, 1.0),
        });
        let shape_b = ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(1.0, 1.0, 1.0),
        });

        let contact = Gjk::get_contact(
            &shape_a,
            Vec3::ZERO,
            Quat::IDENTITY,
            &shape_b,
            Vec3::new(1.5, 0.0, 0.0),
            Quat::IDENTITY,
        );

        assert!(contact.is_some(), "EPA failed to generate contact");
        let contact = contact.unwrap();

        assert!(
            (contact.penetration - 0.5).abs() < 0.001,
            "Penetration depth is wrong: {}",
            contact.penetration
        );
        assert!(
            (contact.normal.x.abs() - 1.0).abs() < 0.001,
            "Normal is wrong: {:?}",
            contact.normal
        );
    }

    #[test]
    fn test_speculative_contact_approaching() {
        // Two spheres approaching each other — should produce a contact
        let shape = ColliderShape::Sphere(SphereShape { radius: 0.5 });

        let contact = Gjk::speculative_contact(
            &shape,
            Vec3::new(-5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(10.0, 0.0, 0.0),
            1.0,
            &shape,
            Vec3::new(5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(-10.0, 0.0, 0.0),
            1.0,
            1.0,
        );

        assert!(
            contact.is_some(),
            "Speculative contact missed approaching spheres"
        );
    }

    #[test]
    fn test_compute_face_normal_follows_winding_not_origin() {
        // Regression (EPA face orientation): the face normal must come from the
        // stored winding order a→b→c (right-hand rule), NOT from a "point away
        // from the origin" heuristic. Here the triangle is wound so the
        // right-hand-rule normal is +Z, yet the whole face sits just BELOW the
        // origin (z < 0) — the situation a shallow/grazing contact creates, with
        // the origin on the OUTER side of the closest face. The old code computed
        // normal·v_a = -0.01 < 0 and flipped the normal to -Z (inward), which is
        // what corrupted shallow contacts (normal pointing the wrong way / wrong
        // faces marked visible during expansion). The winding normal must stay +Z.
        let mk = |x: f32, y: f32, z: f32| SupportPoint {
            v: Vec3::new(x, y, z),
            a: Vec3::ZERO,
            b: Vec3::ZERO,
        };
        let simplex = [mk(0.0, 0.0, -0.01), mk(1.0, 0.0, -0.01), mk(0.0, 1.0, -0.01)];
        let n = Gjk::compute_face_normal(&simplex, 0, 1, 2);
        assert!(
            n.z > 0.9,
            "face normal must follow winding a→b→c (expected ≈ +Z), got {:?}",
            n
        );
        // Reversing the winding must flip the normal (proves it is winding-driven,
        // not origin-driven — both windings sit on the same side of the origin).
        let n_rev = Gjk::compute_face_normal(&simplex, 0, 2, 1);
        assert!(
            n_rev.z < -0.9,
            "reversed winding must flip the normal, got {:?}",
            n_rev
        );
    }

    #[test]
    fn test_epa_shallow_contact_normal_outward() {
        // Behaviour guard for the EPA fix: a very shallow box/box overlap must
        // still yield a positive penetration and a normal along the separating
        // axis (±X), pointing consistently — never inward and never NaN.
        let shape = ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(1.0, 1.0, 1.0),
        });
        // Overlap of only 0.02 along X (origin sits very close to the closest
        // Minkowski face — the regime that tripped the origin heuristic).
        let contact = Gjk::get_contact(
            &shape,
            Vec3::ZERO,
            Quat::IDENTITY,
            &shape,
            Vec3::new(1.98, 0.0, 0.0),
            Quat::IDENTITY,
        )
        .expect("EPA must produce a contact for a shallow overlap");

        assert!(
            contact.penetration > 0.0 && contact.penetration < 0.1,
            "shallow penetration should be small and positive, got {}",
            contact.penetration
        );
        assert!(contact.normal.is_finite(), "normal must be finite");
        assert!(
            (contact.normal.length() - 1.0).abs() < 1e-3,
            "normal must be unit length, got {}",
            contact.normal.length()
        );
        assert!(
            contact.normal.x.abs() > 0.99,
            "separating axis should be X, got normal {:?}",
            contact.normal
        );
    }

    #[test]
    fn test_speculative_contact_separating() {
        // Two spheres moving apart — should NOT produce a contact
        let shape = ColliderShape::Sphere(SphereShape { radius: 0.5 });

        let contact = Gjk::speculative_contact(
            &shape,
            Vec3::new(-5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(-10.0, 0.0, 0.0),
            1.0,
            &shape,
            Vec3::new(5.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(10.0, 0.0, 0.0),
            1.0,
            1.0,
        );

        assert!(
            contact.is_none(),
            "Speculative contact incorrectly fired for separating shapes"
        );
    }
}
