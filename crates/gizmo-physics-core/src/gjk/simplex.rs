use super::*;

impl Gjk {
    /// GJK that returns the final simplex for EPA
    pub(crate) fn gjk_with_simplex<F>(support: F) -> Option<Vec<SupportPoint>>
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

        // ROBUSTNESS (audit 2026-06-29): a high-aspect-ratio shape (e.g. a thin box —
        // a tiny face next to huge side extents) makes the support jump between far
        // corners, so the simplex reduction can degenerate: `closest_point_on_simplex`
        // can return a NaN barycentre, a spurious near-origin point (→ a false "0 gap /
        // intersecting"), or bounce back to a far corner. Trusting the final simplex
        // state then yields a wildly wrong distance (observed: 14 m for a true 0.01 m
        // gap), which let a Mach-scale CCD bullet tunnel a thick wall.
        //
        // Two independent, always-valid quantities make this robust:
        //   • best_point/best_sq — the smallest *positive* separation actually seen
        //     (an upper bound on the true distance; never corrupted by a later NaN);
        //   • lb_max — the largest duality lower bound `a·closest_hat` (the true distance
        //     is never below it). The result is `max(best, lb_max)`, so a spurious
        //     collapse to ~0 cannot report contact when the shapes are demonstrably apart.
        let mut best_point = closest_point;
        let mut best_sq = if min_dist_sq > 1e-10 {
            min_dist_sq
        } else {
            f32::INFINITY
        };
        let mut lb_max = 0.0f32;
        let mut stalls = 0u32;

        for _ in 0..32 {
            direction = -closest_point;
            let dir_n = direction.normalize();
            if !dir_n.is_finite() {
                break; // closest degenerated to ~origin / NaN — keep the best so far
            }
            let a = support(dir_n);

            // Duality lower bound: the true distance is never less than how far the
            // support reaches toward the origin. `direction` has magnitude
            // `current_dist`, so `-a·direction/current_dist == a·closest_hat`.
            let current_dist = min_dist_sq.sqrt();
            let lower_bound = -a.dot(direction) / current_dist;
            if lower_bound > lb_max {
                lb_max = lower_bound;
            }
            // Convergence (relative + absolute — an absolute-only threshold never trips
            // for shapes whose support is imprecise at this scale, e.g. large extents).
            if current_dist - lower_bound < 1e-4 * current_dist.max(1.0) + 1e-6 {
                break;
            }

            simplex.push(a);
            closest_point = Self::closest_point_on_simplex(&mut simplex);
            min_dist_sq = closest_point.length_squared();

            if !min_dist_sq.is_finite() || min_dist_sq < 1e-8 {
                // Degenerate reduction OR (genuine) origin enclosure — either way stop;
                // `best_sq`/`lb_max` below resolve which it actually was.
                break;
            }
            if min_dist_sq + 1e-10 < best_sq {
                best_sq = min_dist_sq;
                best_point = closest_point;
                stalls = 0;
            } else {
                // No real progress; cycling between far support corners → stop.
                stalls += 1;
                if stalls >= 2 {
                    break;
                }
            }
        }

        let dist = if best_sq.is_finite() {
            best_sq.sqrt().max(lb_max.max(0.0))
        } else {
            lb_max.max(0.0)
        };
        let normal = if dist > 1e-6 && best_point.length_squared() > 1e-12 {
            best_point.normalize()
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
                    // Guard `d1 / (d1 - d3)`: when d1 == d3 == 0 the edge collapses and a
                    // bare divide yields NaN. Fall back to the newest vertex `a`.
                    match gizmo_math::safe_recip(d1, d1 - d3) {
                        Some(v) => {
                            *simplex = vec![b, a];
                            return a + ab * v;
                        }
                        None => return a,
                    }
                }

                let vb = d5 * d2 - d1 * d6;
                if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
                    // Guard `d2 / (d2 - d6)` against the d2 == d6 == 0 degenerate edge.
                    match gizmo_math::safe_recip(d2, d2 - d6) {
                        Some(w) => {
                            *simplex = vec![c, a];
                            return a + ac * w;
                        }
                        None => return a,
                    }
                }

                let va = d3 * d6 - d5 * d4;
                if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
                    // Guard `(d4 - d3) / ((d4 - d3) + (d5 - d6))`: when both terms are zero
                    // the edge bc collapses. Fall back to the newest vertex `a`.
                    match gizmo_math::safe_recip(d4 - d3, (d4 - d3) + (d5 - d6)) {
                        Some(w) => {
                            *simplex = vec![c, b];
                            return b + (c - b) * w;
                        }
                        None => return a,
                    }
                }

                // Guarded barycentric: a collinear/zero-area triangle makes (va+vb+vc)≈0,
                // and a bare `1.0/0.0` would return a NaN closest point that poisons the
                // GJK distance search. Fall back to the newest vertex (a finite, valid
                // point) — `Gjk::distance` keeps the best iterate, so this stays correct.
                match gizmo_math::safe_recip(1.0, va + vb + vc) {
                    Some(denom) => a + ab * (vb * denom) + ac * (vc * denom),
                    None => a,
                }
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

}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression for findings 6/7/8: degenerate triangle simplices whose edge-reduction
    // denominators collapse to 0 used to produce a `0/0 = NaN` (or `Inf`) closest point.
    // The `safe_recip` guards must yield a finite fallback instead.

    #[test]
    fn closest_point_edge_ac_degenerate_is_finite() {
        // a == b at (1,0,0), c at (0,1,0). Triangle stored as [c, b, a].
        // Reaches the `vc <= 0 && d1 >= 0 && d3 <= 0` branch with d1 == d3 == 0
        // (finding 6): `d1 / (d1 - d3)` would be 0/0 without the guard.
        let mut simplex = vec![
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ];
        let cp = Gjk::closest_point_on_simplex(&mut simplex);
        assert!(cp.is_finite(), "closest point must be finite, got {cp:?}");
    }

    #[test]
    fn closest_point_collinear_triangle_is_finite() {
        // Zero-area (collinear) triangle: every edge-reduction denominator and the
        // fall-through barycentric denominator degenerate. All guards (findings 6/7/8
        // and the pre-existing line-224 guard) must cooperate to keep the result finite
        // rather than emitting a NaN/Inf that poisons the GJK distance search.
        for coords in [
            [
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(3.0, 0.0, 0.0),
            ],
            [
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 2.0, 0.0),
            ],
        ] {
            let mut simplex = coords.to_vec();
            let cp = Gjk::closest_point_on_simplex(&mut simplex);
            assert!(cp.is_finite(), "closest point must be finite, got {cp:?}");
        }
    }

    #[test]
    fn closest_point_coincident_triangle_is_finite() {
        // All three vertices coincident: the most degenerate triangle possible. Every
        // divisor in the reduction is zero; the guards must still yield a finite point.
        let mut simplex = vec![
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(1.0, 2.0, 3.0),
        ];
        let cp = Gjk::closest_point_on_simplex(&mut simplex);
        assert!(cp.is_finite(), "closest point must be finite, got {cp:?}");
    }
}
