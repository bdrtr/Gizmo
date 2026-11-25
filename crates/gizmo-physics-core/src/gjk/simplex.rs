use super::*;

/// A speculative contact and the gap it was made for (`Gjk::speculative_meeting`).
#[derive(Debug, Clone, Copy)]
struct Speculative {
    contact: ContactPoint,
    gap: f32,
}

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

        // Exhausted the iteration budget without enclosing the origin. Usually a genuine
        // near-miss, but at this budget it can also be a numerically stalled search — a
        // trace helps distinguish "no contact" from "GJK gave up" when debugging.
        tracing::trace!(
            max_iterations = MAX_ITERATIONS,
            simplex_len = simplex.len(),
            "GJK did not converge within the iteration budget; treating as no collision"
        );
        None
    }

    /// The distance between two separated convex shapes, and the direction from B to A: GJK's
    /// [`distance`](Self::distance) between their *cores*, less the radii around them. A sphere's
    /// core is its centre and a capsule's its segment; any other shape is its own core, with no
    /// radius. Overlapping shapes give zero, as `distance` does.
    ///
    /// A rounded support is where GJK is weakest. Against a large flat face every support point it
    /// asks for comes from the face's far rim, the step each one offers shrinks with the square of
    /// the rim's distance, and in `f32` the search stops short: a 0.34 m capsule 0.367 m from a
    /// 16 m box came out 0.33 mm far (2026-11-25). A point or a segment against a polytope is a
    /// polytope, and GJK ends on one exactly, in a few steps. Box2D, Bullet and Jolt split a
    /// shape's convex radius off the same way.
    #[allow(clippy::too_many_arguments)]
    pub fn shape_distance(
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: gizmo_math::Quat,
    ) -> Option<(f32, Vec3)> {
        let (core_a, core_b) = (Self::rounded_core(shape_a), Self::rounded_core(shape_b));
        let (shape_a, radius_a) = core_a.as_ref().map_or((shape_a, 0.0), |(s, r)| (s, *r));
        let (shape_b, radius_b) = core_b.as_ref().map_or((shape_b, 0.0), |(s, r)| (s, *r));
        let (gap, axis) = Self::distance(|dir: Vec3| {
            Self::support_point(shape_a, pos_a, rot_a, dir) - Self::support_point(shape_b, pos_b, rot_b, -dir)
        })?;
        Some(((gap - radius_a - radius_b).max(0.0), axis))
    }

    /// A rounded shape as its core and the radius around it: a sphere as its centre, a capsule as
    /// its segment. `None` for a shape that is its own core.
    fn rounded_core(shape: &ColliderShape) -> Option<(ColliderShape, f32)> {
        match shape {
            ColliderShape::Sphere(s) => Some((ColliderShape::Sphere(SphereShape { radius: 0.0 }), s.radius)),
            ColliderShape::Capsule(c) => {
                Some((ColliderShape::Capsule(CapsuleShape { radius: 0.0, half_height: c.half_height }), c.radius))
            }
            _ => None,
        }
    }

    /// Compute distance and closest points using GJK (for non-intersecting shapes)
    /// Returns (distance, normal_from_b_to_a)
    ///
    /// For two shapes rather than a support function, ask
    /// [`shape_distance`](Self::shape_distance): it hands a rounded shape to this as its core.
    /// # Overlapping input
    ///
    /// Defined for **non-intersecting** shapes only. When the Minkowski difference contains
    /// the origin the result is meaningless: the magnitude is a positive upper bound taken
    /// from a stale iterate and the direction is arbitrary — there is no penetration
    /// channel, and the return is clamped at zero. Callers that must distinguish
    /// "separated by d" from "overlapping" should ask [`Gjk::test_collision`], which is the
    /// authoritative enclosure predicate; `conservative_advancement` does exactly that.
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

                // Inside the face: the foot of the origin on the triangle's plane, taken along the
                // plane's normal. Not from the barycentrics `vb`, `vc`: they are differences of
                // products of the edges' dot products, and for a sliver — two support points 6 cm
                // apart and a third 16 m away, as a box's far corners give — they cancel to a
                // point off the plane by millimetres. Its length is still right, but its direction
                // is tipped, and the next support, asked along it, comes from a corner 11 m to the
                // side: GJK stalled 1.5 mm short (2026-11-25). The cross product of the edges
                // keeps its direction at any aspect.
                // A collinear/zero-area triangle has no normal: fall back to the newest vertex (a
                // finite, valid point) — `Gjk::distance` keeps the best iterate, so this stays
                // correct.
                let n = ab.cross(ac);
                match gizmo_math::safe_recip(n.dot(a), n.length_squared()) {
                    Some(along) => n * along,
                    None => a,
                }
            }
            4 => {
                // The closest point of a tetrahedron the origin lies outside of is the closest of
                // its four faces' closest points: the boundary is their union (Ericson, *Real-Time
                // Collision Detection* §5.1.6 takes only the faces whose planes separate the
                // origin, which is an economy, and a fragile one — see below).
                //
                // This took the first face whose `cross` pointed at the origin, assuming every
                // face wound outwards, which the order the vertices arrive in does not give: for a
                // 0.1 m ball 3.6 cm over a 1 m box it went from a triangle 5.8 cm away to one
                // 1.2 m away, and the search, seeing no progress, stopped at 5.8. Telling a face's
                // side against its opposite vertex fixes the winding, but not a flat tetrahedron —
                // a new support point in an old face's plane, which a box face gives every time —
                // where that side is rounding noise and the face holding the answer can be
                // skipped. So every face is asked, and the origin is inside only for a tetrahedron
                // with a volume and the origin behind all four faces (2026-11-25).
                let points = [simplex[0], simplex[1], simplex[2], simplex[3]];
                let mut behind_all = true;
                let mut widest = 0.0f32;
                let mut longest = 0.0f32;
                let mut best: Option<(f32, Vec3, Vec<Vec3>)> = None;
                for (i, j, k, opposite) in [(0, 1, 2, 3), (0, 1, 3, 2), (0, 2, 3, 1), (1, 2, 3, 0)] {
                    let (a, b, c, d) = (points[i], points[j], points[k], points[opposite]);
                    let n = (b - a).cross(c - a);
                    behind_all &= n.dot(-a) * n.dot(d - a) > 0.0;
                    widest = widest.max(n.length());
                    longest = longest.max((b - a).length()).max((c - a).length()).max((c - b).length());
                    let mut face = vec![a, b, c];
                    let p = Self::closest_point_on_simplex(&mut face);
                    let d2 = p.length_squared();
                    if best.as_ref().is_none_or(|(b2, _, _)| d2 < *b2) {
                        best = Some((d2, p, face));
                    }
                }
                // Six times the volume, against the widest face times the longest edge: the
                // tetrahedron's height over that face, relative to its size.
                let volume = (points[1] - points[0]).dot((points[2] - points[0]).cross(points[3] - points[0]));
                let has_volume = volume.abs() > 1e-5 * widest * longest;
                match best {
                    Some((_, p, face)) if !(behind_all && has_volume) => {
                        *simplex = face;
                        p
                    }
                    _ => Vec3::ZERO,
                }
            }
            _ => Vec3::ZERO,
        }
    }

    /// Conservative-advancement time of impact for a **translational** sweep.
    ///
    /// Returns `(toi, normal)` where `normal` points from B to A — the same convention as
    /// [`Gjk::distance`], and the OPPOSITE of the A→B normal that `speculative_contact`
    /// puts into a [`ContactPoint`]. `toi` is a conservative *under*-estimate: never late,
    /// which is the safe direction for CCD.
    ///
    /// **Translation only.** `rot_a`/`rot_b` are fixed orientations; there is no angular
    /// velocity input, so a spinning thin body is not covered (same scope limit
    /// `speculative_contact` states for itself). `Plane` and `Compound` violate
    /// [`Gjk::support_point`]'s contract and must not be passed here.
    ///
    /// # Why the numerator is re-derived
    ///
    /// The advance rule `dt = gap / closing_speed` is the textbook Mirtich bound and is
    /// provably non-overshooting under pure translation — the Minkowski set translates
    /// rigidly, so a supporting plane's offset decays exactly linearly and the step lands
    /// at most *on* touching. That was never the bug.
    ///
    /// The bug was the numerator's provenance. [`Gjk::distance`] returns
    /// `max(best_sq.sqrt(), lb_max)` — an *upper* bound that can come from a stale iterate
    /// — paired with a normal from a different one. When the shapes touch exactly, its
    /// simplex reduction reaches the origin, trips the degenerate-collapse break and falls
    /// back to the previous positive iterate. Two unit boxes swept together with a 0.2
    /// lateral offset reported a 0.1414 gap at the exact moment of contact, so the touch
    /// test never fired, the next step carried them into overlap, and from there
    /// `distance()` (which has no penetration channel) returned a positive value with an
    /// arbitrary normal until the separation certificate rejected the whole sweep.
    ///
    /// So `distance()` supplies the *direction* only, and the gap is re-derived here as the
    /// certified supporting-plane offset `sep(n) = min_{p ∈ A⊖B} p·n = support(-n)·n`.
    /// That is never greater than the true distance for any unit `n`, so every step is at
    /// least as small as before, and it is positive exactly when `n` separates.
    /// Cost: one extra support call per iteration.
    #[allow(clippy::too_many_arguments)]
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
        /// Certified gap at or below which the pair counts as touching.
        const TOUCH: f32 = 1e-4;
        const MAX_ITERS: u32 = 32;

        // NaN-safe: a NaN horizon is not a valid budget and must reject, so the test is
        // written positively rather than as a negated comparison.
        if max_t.is_nan() || max_t <= 0.0 {
            return None;
        }

        // Already overlapping at t=0 is a hit at t=0, not a miss. `distance()` clamps its
        // result at zero and has no penetration channel, so ask the enclosure predicate
        // directly. This must precede the velocity early-out: a stationary interpenetrating
        // pair still has a TOI of 0.
        if Self::test_collision(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b) {
            let n = (pos_a - pos_b).try_normalize().unwrap_or(Vec3::X);
            return Some((0.0, n));
        }

        let rel_vel = vel_a - vel_b;
        let speed = rel_vel.length();
        if speed < 1e-6 {
            // Stationary, and provably apart by the test above.
            return None;
        }

        let mut t = 0.0f32;
        // Last axis measured at a gap wide enough for the GJK axis to be trustworthy.
        // At the touching iterate the axis degenerates (`distance` falls back to `Vec3::X`),
        // so the reported impact normal comes from the previous, reliable measurement — but a
        // rounded shape's is measured between cores, a radius apart when they touch, and is the
        // better for being taken there: from 3 m up, a ball 5 mm off an edge it is dropped on
        // comes straight down; where it touches, its normal leans 1.3°.
        let mut reliable_n: Option<Vec3> = None;
        let radius = |s: &ColliderShape| Self::rounded_core(s).map_or(0.0, |(_, r)| r);
        let rounded = radius(shape_a) + radius(shape_b) > 1e-3;

        for _ in 0..MAX_ITERS {
            let support = |dir: Vec3| {
                let sa = Self::support_point(shape_a, pos_a, rot_a, dir);
                let sb = Self::support_point(shape_b, pos_b, rot_b, -dir);
                sa - sb
            };

            let (_, axis) = Self::shape_distance(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b)?;
            let Some(n) = axis.try_normalize() else {
                break;
            };

            let sep = support(-n).dot(n);

            if sep <= TOUCH {
                return Some((t, if rounded { n } else { reliable_n.unwrap_or(n) }));
            }
            reliable_n = Some(n);

            let closing = -rel_vel.dot(n);
            if closing <= 1e-6 * speed {
                // `n` separates now, and under constant relative translation its plane
                // offset can only grow — the pair can never intersect. A proof, not a
                // heuristic, so returning None here is sound.
                return None;
            }

            let dt = sep / closing;

            if t + dt >= max_t {
                // Clamp to the horizon and TEST it, rather than dropping a contact that
                // lands exactly at `max_t`.
                //
                // The test is the same certified-gap criterion the loop uses, NOT
                // `test_collision`: a sweep whose TOI is exactly `max_t` ends *touching*,
                // and touching is not containment — GJK's enclosure predicate answers
                // false there, which would drop precisely the contact this branch exists
                // to keep.
                let remaining = max_t - t;
                let end_a = pos_a + vel_a * remaining;
                let end_b = pos_b + vel_b * remaining;
                let end_support = |dir: Vec3| {
                    Self::support_point(shape_a, end_a, rot_a, dir)
                        - Self::support_point(shape_b, end_b, rot_b, -dir)
                };
                if end_support(-n).dot(n) <= TOUCH {
                    let there = Self::shape_distance(shape_a, end_a, rot_a, shape_b, end_b, rot_b);
                    let n = there.filter(|_| rounded).and_then(|(_, axis)| axis.try_normalize()).unwrap_or(n);
                    return Some((max_t, n));
                }
                return None;
            }

            t += dt;
            pos_a += vel_a * dt;
            pos_b += vel_b * dt;
        }

        // Budget exhausted. A glancing approach makes `closing` small and progress
        // collapse; report the best conservative estimate reached rather than a silent
        // false miss, but only if we are demonstrably close.
        let support = |dir: Vec3| {
            Self::support_point(shape_a, pos_a, rot_a, dir)
                - Self::support_point(shape_b, pos_b, rot_b, -dir)
        };
        if let Some((_, axis)) = Self::shape_distance(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b) {
            if let Some(n) = axis.try_normalize() {
                if support(-n).dot(n) <= 1e-2 {
                    return Some((t, reliable_n.unwrap_or(n)));
                }
            }
        }
        tracing::trace!(
            max_iterations = MAX_ITERS,
            "conservative advancement did not converge within the iteration budget"
        );
        None
    }

    /// Speculative contact for continuous collision detection (CCD).
    ///
    /// When two **separated** shapes meet along their paths this frame — not merely close: one
    /// passing by the other has no contact — this emits a contact, on the normal they meet on,
    /// whose *negative penetration encodes the separation gap* along it.
    /// The constraint solver reads that gap (`penetration < 0` ⇒ velocity bias
    /// `gap/dt`) and lets the body advance **exactly up to the surface this step,
    /// never past it** — instead of tunnelling through (no constraint) or freezing
    /// far short (the old `penetration = 0` behaviour, which stopped the body at
    /// its start-of-frame position).
    ///
    /// A body arriving from farther than a hair (`SKIN`) is halted that hair short of contact, so
    /// the *next* step still measures a clean, GJK-reliable gap. A body already within it closes
    /// the rest and touches: from there the touching path's real contacts hold it, and they carry
    /// friction and rolling resistance, which a speculative contact does not. Halted short every
    /// step, a CCD body resting on terrain never touched it — a ball rolled on frictionless at a
    /// millimetre's height, for eight seconds from 1 m/s, where the same ball without CCD stops
    /// in a quarter of one.
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
    #[allow(clippy::too_many_arguments)]
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
        Self::speculative(shape_a, pos_a, rot_a, vel_a, inv_mass_a, shape_b, pos_b, rot_b, vel_b, inv_mass_b, dt).map(|s| s.contact)
    }

    /// The speculative contact, and the gap it was made for: what [`speculative_contact`](Self::speculative_contact)
    /// gives, with the separation along its normal that the pair has now. A caller that wants
    /// the contact where the pair meets — its real points, with the lever arms the impact will
    /// have — needs the gap to find that moment: the contact's own depth is what the solve may
    /// close, which stops short of the surface by a skin.
    #[allow(clippy::too_many_arguments)]
    pub fn speculative_meeting(
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
    ) -> Option<(ContactPoint, f32)> {
        Self::speculative(shape_a, pos_a, rot_a, vel_a, inv_mass_a, shape_b, pos_b, rot_b, vel_b, inv_mass_b, dt)
            .map(|s| (s.contact, s.gap))
    }

    /// [`speculative_contact`](Self::speculative_contact), with the gap kept beside the contact.
    fn speculative(
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
    ) -> Option<Speculative> {
        // A compound is its parts: the manifold path recurses the same way. Passed whole, it reached
        // `support_point`, whose contract it violates — a debug panic, and a zero support (a wrong
        // contact) in release — on the first step any CCD body (every kinematic one: they turn CCD
        // on) came near a multi-part collider. Of the parts' contacts, the closest is kept: it is
        // the one this step must not pass.
        let closest = |found: Option<Speculative>, c: Option<Speculative>| match (found, c) {
            (Some(f), Some(c)) => Some(if c.contact.penetration > f.contact.penetration { c } else { f }),
            (f, c) => f.or(c),
        };
        if let ColliderShape::Compound(parts) = shape_a {
            return parts.iter().fold(None, |found, (local, sub)| {
                let c = Self::speculative(
                    sub,
                    pos_a + rot_a.mul_vec3(local.position),
                    rot_a * local.rotation,
                    vel_a,
                    inv_mass_a,
                    shape_b,
                    pos_b,
                    rot_b,
                    vel_b,
                    inv_mass_b,
                    dt,
                );
                closest(found, c)
            });
        }
        if let ColliderShape::Compound(parts) = shape_b {
            return parts.iter().fold(None, |found, (local, sub)| {
                let c = Self::speculative(
                    shape_a,
                    pos_a,
                    rot_a,
                    vel_a,
                    inv_mass_a,
                    sub,
                    pos_b + rot_b.mul_vec3(local.position),
                    rot_b * local.rotation,
                    vel_b,
                    inv_mass_b,
                    dt,
                );
                closest(found, c)
            });
        }

        // **A surface is its pieces too.** A plane has no support function, and a heightfield or a
        // triangle mesh has one that describes its convex hull — a lid over every valley — rather
        // than the surface. The touching path never hands them to GJK whole (planes analytically,
        // fields and meshes a triangle at a time), and neither may this: a heightfield reached
        // `support_point`, logged an error and returned a zero support on every step a CCD body
        // was within a step of the ground without touching it (a golf ball rolling on its course:
        // about once a physics step, unseen in the game's headless test only because nothing logs
        // there), and in a debug build it is a panic.
        let is_surface = |s: &ColliderShape| {
            matches!(s, ColliderShape::Plane(_) | ColliderShape::Heightfield(_) | ColliderShape::TriMesh(_))
        };
        if is_surface(shape_a) && is_surface(shape_b) {
            // Two surfaces are two pieces of static world; nothing here moves one onto the other.
            return None;
        }
        match (shape_a, shape_b) {
            (_, ColliderShape::Plane(p)) => {
                // A's deepest point towards the plane, and the gap from it to the plane.
                let deepest = Self::support_point(shape_a, pos_a, rot_a, -p.normal);
                let gap = (deepest.dot(p.normal) - p.distance).max(0.0);
                return Self::speculative_emit(gap, -p.normal, vel_a - vel_b, dt, pos_a, inv_mass_a, pos_b, inv_mass_b);
            }
            (ColliderShape::Plane(p), _) => {
                let deepest = Self::support_point(shape_b, pos_b, rot_b, -p.normal);
                let gap = (deepest.dot(p.normal) - p.distance).max(0.0);
                return Self::speculative_emit(gap, p.normal, vel_a - vel_b, dt, pos_a, inv_mass_a, pos_b, inv_mass_b);
            }
            (_, ColliderShape::Heightfield(_) | ColliderShape::TriMesh(_)) => {
                return Self::speculative_against_surface(
                    (shape_a, pos_a, rot_a, vel_a, inv_mass_a),
                    (shape_b, pos_b, rot_b, vel_b, inv_mass_b),
                    false,
                    dt,
                );
            }
            (ColliderShape::Heightfield(_) | ColliderShape::TriMesh(_), _) => {
                return Self::speculative_against_surface(
                    (shape_b, pos_b, rot_b, vel_b, inv_mass_b),
                    (shape_a, pos_a, rot_a, vel_a, inv_mass_a),
                    true,
                    dt,
                );
            }
            _ => {}
        }

        /// Below this gap the GJK separating axis is unreliable (it degenerates to
        /// a `Vec3::X` fallback), so we orient the normal from the approach instead.
        const AXIS_RELIABLE_GAP: f32 = 1e-3;

        if dt <= 0.0 {
            return None;
        }

        // Separation distance + separating axis at the *current* configuration.
        let (gap, axis) = Self::shape_distance(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b)?;

        let rel_vel = vel_a - vel_b;

        // Contact normal, pointing A→B, and the gap along it.
        let (gap, normal) = if gap > AXIS_RELIABLE_GAP {
            // Apart along the axis by more than the motion closes along it: they do not meet, and
            // this much costs one measure. `axis` is B→A.
            let closing = -rel_vel.dot(axis);
            if closing <= 1e-4 || gap > closing * dt {
                return None;
            }
            // **Whether they meet, and on which normal, is asked of the path, not of where they
            // are now.** The axis between them now says that they close, not that they touch: a
            // ball flying past a box closes on its near corner until it is beside it. Taken as the
            // contact, it stopped the ball at the corner and bounced it off what it never reached
            // — at 60 Hz a 0.1 m ball at 20 m/s off a 0.6 m crate it was to pass 10 cm clear of,
            // and at 240 Hz any faster than 48 (2026-11-25). Swept, a pair that does not meet has no
            // contact; one that does has the normal it meets on, and the gap its motion closes
            // along that by then.
            let (toi, from_b) = Self::conservative_advancement(shape_a, pos_a, rot_a, vel_a, shape_b, pos_b, rot_b, vel_b, dt)?;
            (-rel_vel.dot(from_b) * toi, -from_b)
        } else {
            // Within a hair the axis is unreliable, so the surface's own normal is measured
            // instead: A is moved along its approach just past the surface and the penetrating
            // contact's normal taken. It used to be the approach direction ITSELF — so a body a
            // hair from a wall, moving along it under gravity, got a contact pointing against
            // gravity that stopped all its motion (measured in `pinbol`: a ball hung on the rail of
            // a tilted table, normal (0, 0.978, −0.208); and a lone CCD ball rolled 9 % short, its
            // own floor bounces taken the same way).
            let approach = rel_vel.try_normalize()?;
            let probe = pos_a + approach * (gap + 2.0 * AXIS_RELIABLE_GAP);
            (gap, Self::get_contact(shape_a, probe, rot_a, shape_b, pos_b, rot_b)?.normal)
        };

        Self::speculative_emit(gap, normal, rel_vel, dt, pos_a, inv_mass_a, pos_b, inv_mass_b)
    }

    /// The speculative contact for a separation `gap` along `normal` (A→B), or `None` when the
    /// bodies are not closing on each other fast enough to meet this step.
    #[allow(clippy::too_many_arguments)]
    fn speculative_emit(
        gap: f32,
        normal: Vec3,
        rel_vel: Vec3,
        dt: f32,
        pos_a: Vec3,
        inv_mass_a: f32,
        pos_b: Vec3,
        inv_mass_b: f32,
    ) -> Option<Speculative> {
        /// Resting standoff: the body stops this far short of the surface so the
        /// next frame still has a measurable, GJK-reliable separation.
        const SKIN: f32 = 0.01;

        if dt <= 0.0 {
            return None;
        }

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

        // How far the solver may let the body close this step: to SKIN short of the surface on
        // arrival, and the rest of the way from within it (see `speculative_contact`).
        let allowed_close = if gap <= SKIN { gap } else { gap - SKIN };

        // Anchor at the inverse-mass-weighted centre: collapses onto the dynamic
        // body when the other is static ⇒ that body's lever arm vanishes ⇒ the
        // normal impulse is a clean linear stop.
        let inv_sum = inv_mass_a + inv_mass_b;
        let point = if inv_sum > 1e-12 {
            (pos_a * inv_mass_a + pos_b * inv_mass_b) / inv_sum
        } else {
            (pos_a + pos_b) * 0.5
        };

        Some(Speculative { gap, contact: ContactPoint {
            point,
            normal,
            // Negative ⇒ speculative gap; solver bias allows closing exactly this much. Strictly
            // negative even when nothing may be closed: `-0.0` is not `< 0.0`, and the solver took
            // such a contact for a resting one — friction at the body's centre included.
            penetration: -allowed_close.max(f32::MIN_POSITIVE),
            local_point_a: point - pos_a,
            local_point_b: point - pos_b,
            normal_impulse: 0.0,
            tangent_impulse: Vec3::ZERO,
            sliding: false,
        } })
    }

    /// The speculative contact between a convex shape and a heightfield or triangle mesh: of the
    /// triangles its path meets within the step, the first. `surface_is_a` says which side of the
    /// pair the surface was on, so the normal keeps the caller's A→B orientation.
    ///
    /// The reach is the convex shape's bounds in the surface's own frame — exact along each of its
    /// axes, from two support queries per axis — swept by how far the two move relative to each
    /// other this step, plus the touching path's skin.
    ///
    /// **A surface answers for what comes down over it, along its own normal** — the touching
    /// path's rule (`NarrowPhase::terrain_contact`), asked where the two *meet*: a triangle's face is
    /// met where the shape's deepest point gets to its plane over it. Tested whole, a triangle
    /// offered its nearest feature — a field's border edge to a body over the next field, sideways:
    /// a CCD ball rolling across the seam of two fields was thrown 0.6 m up there. Asked where the
    /// body is *now*, a ball over the flat was answered by the flat and not by the hill its path ran
    /// into: of 600 level shots at a 3 m hill, 22 % ended inside a mesh of it at 240 Hz and 60 % at
    /// 60, 9 % and 45 % a field of it (2026-11-25).
    ///
    /// Between two faces that fold away from each other a shape comes down on neither, so their edge
    /// is swept for (`conservative_advancement`) and gives the normal it is met on — where that lies
    /// between the two faces' own. On a flat edge that is the face's normal and nothing beside it, on
    /// one folding in there is none, and a border has no second face: it is where another mesh may
    /// go on. However slight the fold: between two faces 2° apart there are 5 mm a 0.15 m ball's
    /// deepest point comes down on neither, and a mesh knows an edge for a ridge from 5°.
    fn speculative_against_surface(
        convex: (&ColliderShape, Vec3, gizmo_math::Quat, Vec3, f32),
        surface: (&ColliderShape, Vec3, gizmo_math::Quat, Vec3, f32),
        surface_is_a: bool,
        dt: f32,
    ) -> Option<Speculative> {
        let (c_shape, c_pos, c_rot, c_vel, c_inv) = convex;
        let (s_shape, s_pos, s_rot, s_vel, s_inv) = surface;
        if dt <= 0.0 {
            return None;
        }
        let mut lo = Vec3::ZERO;
        let mut hi = Vec3::ZERO;
        for (i, axis) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
            let w = s_rot.mul_vec3(axis);
            lo[i] = (Self::support_point(c_shape, c_pos, c_rot, -w) - s_pos).dot(w);
            hi[i] = (Self::support_point(c_shape, c_pos, c_rot, w) - s_pos).dot(w);
        }
        // The convex shape's motion as the surface sees it.
        let v = c_vel - s_vel;
        let swept = s_rot.inverse().mul_vec3(v * dt);
        let skin = Vec3::splat(0.01);
        let (lo, hi) = (lo.min(lo + swept) - skin, hi.max(hi + swept) + skin);
        if !lo.is_finite() || !hi.is_finite() {
            return None;
        }

        // The first face the path meets: when, and on which normal (from the surface). And the edges
        // it may meet between faces, each under its two ends in a fixed order (by their bits: a
        // mesh's triangles may share a corner by value): of each triangle in reach that has it, the
        // way out of the triangle across it, and how far the shape is from the triangle's plane
        // over how fast it closes on it.
        type Corner = [u32; 3];
        let mut first: Option<(f32, Vec3)> = None;
        let mut edges: Vec<((Corner, Corner), [Vec3; 2], Vec3, f32, f32)> = Vec::new();
        let mut face_met = |w: [Vec3; 3], up: Vec3| {
            let closing = -v.dot(up);
            let deepest = Self::support_point(c_shape, c_pos, c_rot, -up);
            let gap = (deepest - w[0]).dot(up);
            // Short of the plane all the way: short of the triangle, its edges with it.
            if gap > 0.0 && gap > closing * dt {
                return;
            }
            let key = |p: Vec3| -> Corner { p.to_array().map(f32::to_bits) };
            for e in 0..3 {
                let (p, q, across) = (w[(e + 1) % 3], w[(e + 2) % 3], w[e]);
                let Some(along) = (q - p).try_normalize() else { continue };
                let out = (p - across) - along * (p - across).dot(along);
                let (kp, kq) = (key(p), key(q));
                let (k, ends) = if kp <= kq { ((kp, kq), [p, q]) } else { ((kq, kp), [q, p]) };
                edges.push((k, ends, out.normalize_or_zero(), gap, closing));
            }
            if closing <= 1e-4 {
                return;
            }
            let when = gap.max(0.0) / closing;
            if first.is_some_and(|(t, _)| t <= when) {
                return;
            }
            let there = deepest + v * when;
            let on_plane = there - up * (there - w[0]).dot(up);
            if crate::narrowphase::NarrowPhase::barycentric(on_plane, w).is_some_and(|b| b.iter().all(|&x| x > -1e-3)) {
                first = Some((when, up));
            }
        };

        match s_shape {
            ColliderShape::Heightfield(hf) => {
                let (cells_x, cells_z) = hf.cell_counts();
                if cells_x == 0 || cells_z == 0 || lo.y > hf.local_aabb.max.y || hi.y < hf.local_aabb.min.y {
                    return None;
                }
                let (col0, col1, row0, row1) = hf.cell_range(lo, hi);
                for row in row0..row1 {
                    for col in col0..col1 {
                        for tri in hf.cell_triangles(row, col) {
                            let w = tri.map(|v| s_pos + s_rot * v);
                            if let Some(up) = (w[1] - w[0]).cross(w[2] - w[0]).try_normalize() {
                                face_met(w, up);
                            }
                        }
                    }
                }
            }
            ColliderShape::TriMesh(mesh) => {
                let mut tris = Vec::new();
                mesh.bvh.query_aabb(gizmo_math::Aabb::new(lo, hi), &mut tris);
                for t in tris {
                    let base = t as usize * 3;
                    let corner = |k: usize| mesh.indices.get(base + k).and_then(|i| mesh.vertices.get(*i as usize)).copied();
                    // A corrupt index is skipped, as the touching path skips it.
                    let (Some(a), Some(b), Some(c)) = (corner(0), corner(1), corner(2)) else { continue };
                    let w = [a, b, c].map(|v| s_pos + s_rot * v);
                    let Some(n) = (w[1] - w[0]).cross(w[2] - w[0]).try_normalize() else { continue };
                    // On the side of the triangle the body is on: a mesh does not say which is solid.
                    let front = n.dot(c_pos - w[0]) >= 0.0;
                    face_met(w, if front { n } else { -n });
                }
            }
            _ => unreachable!("only heightfields and triangle meshes are passed here"),
        }

        // One edge wrapper, refilled per edge.
        let mut edge = ColliderShape::ConvexHull(crate::components::ConvexHullShape {
            vertices: std::sync::Arc::new(Vec::with_capacity(2)),
            faces: std::sync::Arc::new(Vec::new()),
        });
        edges.sort_unstable_by_key(|e| e.0);
        for shared in edges.chunk_by(|a, b| a.0 == b.0) {
            // The border, or an edge the shape stays short of one of the two planes of.
            if shared.len() < 2 {
                continue;
            }
            // Only as far as the first thing met: an edge past a face is behind it, and one of whose
            // planes is come to no sooner than the face is met is no nearer.
            let within = first.map_or(dt, |(t, _)| t);
            if shared.iter().any(|&(_, _, _, gap, closing)| gap > 0.0 && gap >= closing * within * 0.9999) {
                continue;
            }
            let ColliderShape::ConvexHull(hull) = &mut edge else {
                unreachable!("`edge` is constructed as a ConvexHull above")
            };
            let verts = std::sync::Arc::make_mut(&mut hull.vertices);
            verts.clear();
            verts.extend_from_slice(&shared[0].1);
            // The ends are in the world already.
            let Some((when, n)) =
                Self::conservative_advancement(c_shape, c_pos, c_rot, c_vel, &edge, Vec3::ZERO, gizmo_math::Quat::IDENTITY, s_vel, within)
            else {
                continue;
            };
            // Already on it: the touching path's, which had its say. And met across one of its
            // faces, not between them: a body sunk a hair into a flat floor runs into the edge in
            // the middle of it sideways, and one is no more in its way than the floor is.
            let between = shared.iter().all(|&(_, _, out, _, _)| n.dot(out) >= -1e-3);
            if when > 0.0 && (when < within || first.is_none()) && between {
                first = Some((when, n));
            }
        }

        let (when, n) = first?;
        let gap = -v.dot(n) * when;
        let (pos_a, inv_a, pos_b, inv_b, normal, rel_vel) =
            if surface_is_a { (s_pos, s_inv, c_pos, c_inv, n, -v) } else { (c_pos, c_inv, s_pos, s_inv, -n, v) };
        Self::speculative_emit(gap.max(0.0), normal, rel_vel, dt, pos_a, inv_a, pos_b, inv_b)
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
