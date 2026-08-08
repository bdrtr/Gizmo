//! Reynolds-style steering behaviours for agent movement.
//!
//! Every behaviour in this module takes the agent's current world position and linear
//! velocity and returns a *steering vector*: `desired_velocity - current_velocity`, with its
//! magnitude clamped to `max_force`. Positions and radii are in metres, velocities and
//! `max_speed` in metres per second.
//!
//! Dimensionally the result is therefore a velocity difference (m/s), but callers integrate
//! it as an acceleration — [`crate::system::ai_navigation_system`] does
//! `velocity += steering * dt` — so `max_force` acts as a per-agent responsiveness budget in
//! m/s², not a force in newtons. No mass appears anywhere in this module, and no function
//! takes `dt`: frame-rate independence is entirely the caller's responsibility.
//!
//! None of these functions can return a non-finite vector for degenerate input (agent
//! standing on its target, empty or entirely out-of-range neighbour lists, a neighbour
//! exactly coincident with the agent, a zero radius). This is a load-bearing property with a
//! regression test behind it, because a steering vector feeds straight into velocity
//! integration where a single NaN corrupts the whole simulation.

use gizmo_math::Vec3;

/// Limits a steering vector's magnitude to `max_force`, leaving its direction untouched.
///
/// Shorter vectors are returned unchanged and the comparison is done on squared lengths, so
/// there is no square root on the common path. [`Vec3::ZERO`] maps to [`Vec3::ZERO`].
///
/// `max_force` is expected to be non-negative; only its square is compared, so a negative
/// value clamps to `|max_force|` *and flips the vector's direction*.
#[inline]
pub fn clamp_force(v: Vec3, max_force: f32) -> Vec3 {
    if v.length_squared() > max_force * max_force {
        v.normalize() * max_force
    } else {
        v
    }
}

/// Steers the agent straight at `target_pos` at full speed.
///
/// Returns `normalize(target_pos - current_pos) * max_speed - current_vel`, clamped to
/// `max_force`.
///
/// Seek has no braking term: the desired speed is `max_speed` however close the target is, so
/// an agent driven by seek alone overshoots and orbits its destination. Use [`arrive`] when it
/// has to stop there.
///
/// Returns [`Vec3::ZERO`] once the agent is within about 3.5e-4 m of the target (the squared
/// distance falls below `f32::EPSILON`), which is what keeps the `normalize` from producing a
/// NaN on a zero-length offset.
pub fn seek(
    current_pos: Vec3,
    target_pos: Vec3,
    current_vel: Vec3,
    max_speed: f32,
    max_force: f32,
) -> Vec3 {
    let to_target = target_pos - current_pos;
    if to_target.length_squared() < f32::EPSILON {
        return Vec3::ZERO;
    }

    let desired_velocity = to_target.normalize() * max_speed;
    let steering = desired_velocity - current_vel;
    clamp_force(steering, max_force)
}

/// Like [`seek`], but ramps the desired speed down inside `slowing_radius` so the agent
/// settles on the target instead of overshooting it.
///
/// The desired speed is `max_speed * (distance / slowing_radius)` while the agent is closer
/// than `slowing_radius` metres and `max_speed` outside it; the return value is that desired
/// velocity minus `current_vel`, clamped to `max_force`. The ramp is linear in distance, not in
/// time: the requested speed only decays asymptotically toward zero, so the ramp alone never
/// brings the agent to a full stop — the dead zone below does.
///
/// Returns [`Vec3::ZERO`] inside a fixed 0.01 m dead zone around the target. That dead zone is
/// hard-coded and independent of `slowing_radius`, so it also defines the residual position
/// error the agent can be left with.
///
/// A `slowing_radius` of zero (or negative) is safe rather than a division by zero: the dead
/// zone check runs first and the `distance < slowing_radius` test then always fails, degrading
/// the behaviour to plain [`seek`].
pub fn arrive(
    current_pos: Vec3,
    target_pos: Vec3,
    current_vel: Vec3,
    max_speed: f32,
    max_force: f32,
    slowing_radius: f32,
) -> Vec3 {
    let to_target = target_pos - current_pos;
    let distance = to_target.length();

    if distance < 0.01 {
        return Vec3::ZERO;
    }

    // Yavaşlama çemberi içinde hızımızı mesafeye orantılı düşürüyoruz
    let desired_speed = if distance < slowing_radius {
        max_speed * (distance / slowing_radius)
    } else {
        max_speed
    };

    let desired_velocity = (to_target / distance) * desired_speed;
    let steering = desired_velocity - current_vel;
    clamp_force(steering, max_force)
}

/// Pushes the agent away from every obstacle whose influence sphere it is currently inside.
///
/// `obstacles` is a slice of `(center_world_position, avoidance_radius)` in metres. An entry
/// contributes only while `0 < distance < avoidance_radius`; obstacles outside their own radius
/// are ignored, so this is a proximity repulsion and not a predictive avoidance — it never
/// looks ahead along `current_vel` and cannot steer around an obstacle it has not yet reached.
///
/// Each contribution is `(agent - center) / distance²`, i.e. an escape direction of magnitude
/// `1/distance`, and the contributions of overlapping obstacles are summed. The sum's direction
/// is then kept while its magnitude is remapped to `min(|sum|, 1) * max_speed`, so the escape
/// speed grows as the agent closes in and saturates at `max_speed`. Because `|sum|` is measured
/// in inverse metres, that saturation knee sits at a hard-coded 1 m: a single obstacle closer
/// than 1 m always yields full `max_speed`, and beyond 1 m the response falls off as
/// `1/distance`. The behaviour is therefore not scale-invariant — on a world built at a very
/// different unit scale, tune `max_force` and the radii accordingly.
///
/// Returns [`Vec3::ZERO`] when no obstacle is in range. An obstacle whose centre coincides with
/// the agent (distance at or below `f32::EPSILON`) is skipped instead of dividing by zero, so a
/// dead-centre overlap produces no escape force at all.
pub fn avoid_obstacles(
    current_pos: Vec3,
    current_vel: Vec3,
    obstacles: &[(Vec3, f32)],
    max_speed: f32,
    max_force: f32,
) -> Vec3 {
    let mut desired_velocity = Vec3::ZERO;
    let mut count = 0;

    for &(obs_pos, obs_radius) in obstacles {
        let diff = current_pos - obs_pos;
        let dist = diff.length();

        if dist > f32::EPSILON && dist < obs_radius {
            // Engele yaklaşıldıkça kaçış kuvveti artar (ters-kare yasası ile yakınlık ölçeklenir)
            let force = diff / (dist * dist);
            desired_velocity += force;
            count += 1;
        }
    }

    if count > 0 {
        // Biriken kuvvet hem yön hem yakınlık büyüklüğünü taşır; yönü normalize edip
        // büyüklüğü max_speed'e ORANTILI ölçekleyerek yakınlık bilgisini koruyoruz.
        let raw = desired_velocity.length();
        if raw > f32::EPSILON {
            // raw büyüdükçe (engel yakınlaştıkça) istenen kaçış hızı max_speed'e kadar artar.
            let scaled_speed = raw.min(1.0) * max_speed;
            desired_velocity = (desired_velocity / raw) * scaled_speed;
        }
        let steering = desired_velocity - current_vel;
        return clamp_force(steering, max_force);
    }

    Vec3::ZERO
}

/// Flocking separation: pushes the agent away from neighbours that are crowding it.
///
/// `neighbors` are world-space positions in metres; only those with
/// `0 < distance < separation_radius` contribute. The accumulation and remapping are identical
/// to [`avoid_obstacles`]: each neighbour adds `(agent - neighbour) / distance²`, and the sum is
/// rescaled to `min(|sum|, 1) * max_speed`, which saturates at full `max_speed` for any single
/// neighbour closer than 1 m.
///
/// Returns [`Vec3::ZERO`] when no neighbour is inside `separation_radius`. Neighbours exactly
/// coincident with the agent (distance at or below `f32::EPSILON`) are skipped rather than
/// producing a NaN — two agents spawned at the same point will not push each other apart, so
/// jitter their spawn positions.
pub fn separate(
    current_pos: Vec3,
    current_vel: Vec3,
    neighbors: &[Vec3],
    separation_radius: f32,
    max_speed: f32,
    max_force: f32,
) -> Vec3 {
    let mut desired_velocity = Vec3::ZERO;
    let mut count = 0;

    for &neighbor in neighbors {
        let diff = current_pos - neighbor;
        let dist = diff.length();

        if dist > f32::EPSILON && dist < separation_radius {
            // Ters-kare yasası: komşu yakınlaştıkça itme büyüklüğü artar.
            let force = diff / (dist * dist);
            desired_velocity += force;
            count += 1;
        }
    }

    if count > 0 {
        // Yönü koru, büyüklüğü yakınlığa ORANTILI olarak max_speed'e kadar ölçekle.
        let raw = desired_velocity.length();
        if raw > f32::EPSILON {
            let scaled_speed = raw.min(1.0) * max_speed;
            desired_velocity = (desired_velocity / raw) * scaled_speed;
        }
        let steering = desired_velocity - current_vel;
        return clamp_force(steering, max_force);
    }

    Vec3::ZERO
}

/// Flocking cohesion: steers the agent toward the centroid of the neighbours around it.
///
/// Takes the unweighted mean of the positions in `neighbors` that lie within `cohesion_radius`
/// metres — distance inside the radius does not change a neighbour's influence — and returns
/// [`seek`] toward that centroid. Because it delegates to `seek` and not [`arrive`], the agent
/// approaches the group at full `max_speed` and does not decelerate as it gets there.
///
/// Neighbours coincident with the agent (distance at or below `f32::EPSILON`) are excluded from
/// the mean. Returns [`Vec3::ZERO`] when that leaves no neighbour in range.
pub fn cohesion(
    current_pos: Vec3,
    current_vel: Vec3,
    neighbors: &[Vec3],
    cohesion_radius: f32,
    max_speed: f32,
    max_force: f32,
) -> Vec3 {
    let mut center = Vec3::ZERO;
    let mut count = 0;

    for &neighbor in neighbors {
        let dist = (current_pos - neighbor).length();
        if dist > f32::EPSILON && dist < cohesion_radius {
            center += neighbor;
            count += 1;
        }
    }

    if count > 0 {
        center /= count as f32;
        // Merkez noktasına seek uygularız
        return seek(current_pos, center, current_vel, max_speed, max_force);
    }

    Vec3::ZERO
}

/// Flocking alignment: matches the agent's heading to the average heading of its neighbours.
///
/// `neighbors` are `(position, linear_velocity)` pairs in metres and m/s; only those within
/// `alignment_radius` metres of the agent contribute. The mean neighbour velocity is
/// **normalised** before use, so the agent matches the flock's *direction* at full `max_speed`
/// however slowly the flock is actually moving — this behaviour copies heading, not speed.
///
/// Returns [`Vec3::ZERO`] when no neighbour is in range. When neighbours are in range but their
/// velocities cancel out (opposing headings, or a stationary flock), the desired velocity is
/// zero and the result is `-current_vel` clamped to `max_force`: the agent brakes rather than
/// doing nothing.
pub fn alignment(
    current_pos: Vec3,
    current_vel: Vec3,
    neighbors: &[(Vec3, Vec3)], // (neighbor_pos, neighbor_vel)
    alignment_radius: f32,
    max_speed: f32,
    max_force: f32,
) -> Vec3 {
    let mut avg_vel = Vec3::ZERO;
    let mut count = 0;

    for &(neighbor_pos, neighbor_vel) in neighbors {
        let dist = (current_pos - neighbor_pos).length();
        if dist > f32::EPSILON && dist < alignment_radius {
            avg_vel += neighbor_vel;
            count += 1;
        }
    }

    if count > 0 {
        avg_vel /= count as f32;
        let desired_velocity = avg_vel.normalize_or_zero() * max_speed;
        let steering = desired_velocity - current_vel;
        return clamp_force(steering, max_force);
    }

    Vec3::ZERO
}

/// Per-behaviour blend weights consumed by [`combined_steering`].
///
/// All fields are dimensionless multipliers applied to a behaviour's output before the weighted
/// sum is clamped to `max_force`. Since the sum is clamped as a whole, they set the *relative*
/// priority of behaviours competing for one force budget, not their absolute strength — scaling
/// every weight by the same factor changes almost nothing.
///
/// [`combined_steering`] additionally treats most of them as on/off gates and only evaluates a
/// behaviour whose weight is strictly `> 0.0`, so on a gated behaviour a negative weight
/// disables it rather than inverting it. [`seek`](Self::seek) is the exception: it is never
/// gated, and a negative weight there does invert the goal term.
///
/// The type is `#[non_exhaustive]`, so from outside this crate build it from [`Default`] and
/// assign the fields you want to change rather than using a struct literal.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SteeringWeights {
    /// Weight on the [`seek`] term toward the optional target. Default `1.0`.
    ///
    /// This is the one field [`combined_steering`] does not gate: whenever a target is supplied
    /// the seek call is made and then multiplied by this weight, so `0.0` still costs the
    /// evaluation but contributes nothing.
    pub seek: f32,

    /// Intended weight for an arrival/braking term. Default `1.0`.
    ///
    /// **Currently dead**: [`combined_steering`] never reads this field and always uses [`seek`]
    /// toward the target, never [`arrive`], so changing it has no effect on anything in this
    /// crate. Blend [`arrive`] yourself if you need the agent to stop.
    pub arrive: f32,

    /// Weight on [`avoid_obstacles`]. Default `5.0`.
    ///
    /// The largest default: because the weighted sum is clamped to a single `max_force` budget,
    /// obstacle escape has to outbid the goal-seeking and flocking terms to survive the clamp
    /// when they pull in opposite directions.
    pub avoid: f32,

    /// Weight on [`separate`]. Default `1.5`, i.e. above the [`cohesion`] weight.
    ///
    /// Separation and cohesion pull in opposite directions over the same neighbour set, and this
    /// higher weight is what stops a flock from collapsing onto its own centroid.
    pub separate: f32,

    /// Weight on [`cohesion`]. Default `1.0`, i.e. below [`separate`].
    ///
    /// Raising it above the separation weight inverts that balance, and the flock converges on
    /// its own centroid instead of spreading out.
    pub cohesion: f32,

    /// Weight on [`alignment`]. Default `1.0`.
    ///
    /// Note that [`alignment`] always requests full `max_speed` along the flock's mean heading,
    /// so this weight scales a full-speed heading correction rather than a small nudge.
    pub alignment: f32,
}

impl Default for SteeringWeights {
    fn default() -> Self {
        Self {
            seek: 1.0,
            arrive: 1.0,
            avoid: 5.0,
            separate: 1.5,
            cohesion: 1.0,
            alignment: 1.0,
        }
    }
}

/// Evaluates the individual behaviours in this module and returns their weighted sum, clamped
/// once more to `max_force`.
///
/// Arguments:
/// - `target_pos` — `None` drops the goal term entirely. `Some` always runs [`seek`], never
///   [`arrive`], so this entry point does not brake on arrival.
/// - `obstacles` — `(center, avoidance_radius)` pairs in metres, forwarded to
///   [`avoid_obstacles`]; only consulted when the slice is non-empty.
/// - `neighbors` — `(position, linear_velocity)` pairs in metres and m/s. The positions are
///   copied into a temporary `Vec` for [`separate`] and [`cohesion`], so this allocates once per
///   call whenever either of those weights is positive.
/// - `radii` — `(separation, cohesion, alignment)` radii in metres, in that order. They are
///   positional and easy to transpose silently. Seek has no radius, and obstacle avoidance takes
///   its range from each obstacle's own entry rather than from here.
///
/// Each behaviour is already clamped to `max_force` on its own and the weighted sum is clamped
/// again, so the total never exceeds `max_force` and the behaviours compete for a single budget
/// rather than accumulating. Weights act as gates as described on [`SteeringWeights`].
///
/// Returns [`Vec3::ZERO`] when `target_pos` is `None` and both slices are empty.
///
/// The engine's own [`crate::system::ai_navigation_system`] does not call this — it composes
/// [`seek`], [`arrive`] and [`separate`] itself — so this is a building block for user-written
/// flocking systems rather than a path any built-in system exercises.
#[allow(clippy::too_many_arguments)]
pub fn combined_steering(
    current_pos: Vec3,
    current_vel: Vec3,
    target_pos: Option<Vec3>,
    obstacles: &[(Vec3, f32)],
    neighbors: &[(Vec3, Vec3)], // (pos, vel)
    weights: &SteeringWeights,
    max_speed: f32,
    max_force: f32,
    radii: (f32, f32, f32), // (separate, cohesion, alignment)
) -> Vec3 {
    let mut total_force = Vec3::ZERO;

    if let Some(target) = target_pos {
        total_force += seek(current_pos, target, current_vel, max_speed, max_force) * weights.seek;
    }

    if weights.avoid > 0.0 && !obstacles.is_empty() {
        total_force += avoid_obstacles(current_pos, current_vel, obstacles, max_speed, max_force)
            * weights.avoid;
    }

    if !neighbors.is_empty() {
        let (sep_r, coh_r, align_r) = radii;
        if weights.separate > 0.0 || weights.cohesion > 0.0 {
            let neighbor_positions: Vec<Vec3> = neighbors.iter().map(|n| n.0).collect();
            if weights.separate > 0.0 {
                total_force += separate(
                    current_pos,
                    current_vel,
                    &neighbor_positions,
                    sep_r,
                    max_speed,
                    max_force,
                ) * weights.separate;
            }
            if weights.cohesion > 0.0 {
                total_force += cohesion(
                    current_pos,
                    current_vel,
                    &neighbor_positions,
                    coh_r,
                    max_speed,
                    max_force,
                ) * weights.cohesion;
            }
        }
        if weights.alignment > 0.0 {
            total_force += alignment(
                current_pos,
                current_vel,
                neighbors,
                align_r,
                max_speed,
                max_force,
            ) * weights.alignment;
        }
    }

    clamp_force(total_force, max_force)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec3;

    /// The steering magnitude produced with a zero initial velocity equals the magnitude of
    /// the desired escape velocity; different distances must therefore give different magnitudes.
    #[test]
    fn avoid_obstacles_scales_with_proximity() {
        let obs = Vec3::ZERO;
        let radius = 4.0;
        let max_speed = 10.0;
        let max_force = 1000.0; // clamp devreye girmesin

        // Yakın ajan (dist=1) ile uzak ajan (dist=3) aynı engelden kaçıyor.
        let near = avoid_obstacles(
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::ZERO,
            &[(obs, radius)],
            max_speed,
            max_force,
        );
        let far = avoid_obstacles(
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::ZERO,
            &[(obs, radius)],
            max_speed,
            max_force,
        );

        // Yakın ajan kesinlikle daha güçlü itilmeli (eski kod eşit büyüklük veriyordu).
        assert!(
            near.length() > far.length() + 1.0,
            "yakın={} uzak={} yakınlık ölçeklemesi kaybolmuş",
            near.length(),
            far.length()
        );
    }

    #[test]
    fn separate_scales_with_proximity() {
        let separation_radius = 1.5;
        let max_speed = 10.0;
        let max_force = 1000.0;

        // Çok yakın komşu (dist=0.5) ile daha uzak komşu (dist=1.4).
        let near = separate(
            Vec3::ZERO,
            Vec3::ZERO,
            &[Vec3::new(0.5, 0.0, 0.0)],
            separation_radius,
            max_speed,
            max_force,
        );
        let far = separate(
            Vec3::ZERO,
            Vec3::ZERO,
            &[Vec3::new(1.4, 0.0, 0.0)],
            separation_radius,
            max_speed,
            max_force,
        );

        assert!(
            near.length() > far.length() + 1.0,
            "yakın={} uzak={} ayrılma yakınlık ölçeklemesi kaybolmuş",
            near.length(),
            far.length()
        );
    }

    // Degenerate inputs (agent on its target, coincident neighbours, opposing
    // velocities, zero radii) must never produce NaN/inf — a steering force feeds
    // straight into physics integration, where one NaN corrupts the whole sim.
    #[test]
    fn degenerate_inputs_never_produce_nan() {
        let finite = |v: Vec3, what: &str| {
            assert!(
                v.x.is_finite() && v.y.is_finite() && v.z.is_finite(),
                "{what} produced a non-finite force: {v:?}"
            );
        };

        // Agent exactly on its target.
        finite(seek(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 10.0, 100.0), "seek@target");
        finite(arrive(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 10.0, 100.0, 5.0), "arrive@target");
        // arrive with a zero slowing radius (no divide-by-zero).
        finite(arrive(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 10.0, 100.0, 0.0), "arrive/zero-radius");

        // Obstacle / neighbour coincident with the agent (dist == 0).
        finite(
            avoid_obstacles(Vec3::ZERO, Vec3::ZERO, &[(Vec3::ZERO, 4.0)], 10.0, 100.0),
            "avoid@obstacle",
        );
        finite(
            separate(Vec3::ZERO, Vec3::ZERO, &[Vec3::ZERO], 2.0, 10.0, 100.0),
            "separate@coincident",
        );
        finite(
            cohesion(Vec3::ZERO, Vec3::ZERO, &[Vec3::ZERO], 5.0, 10.0, 100.0),
            "cohesion@coincident",
        );

        // Alignment where neighbour velocities cancel to zero.
        finite(
            alignment(
                Vec3::ZERO,
                Vec3::ZERO,
                &[(Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
                  (Vec3::new(-1.0, 0.0, 0.0), Vec3::new(-1.0, 0.0, 0.0))],
                5.0,
                10.0,
                100.0,
            ),
            "alignment/opposing",
        );

        // Everything degenerate at once through the combined entry point.
        finite(
            combined_steering(
                Vec3::ZERO,
                Vec3::ZERO,
                Some(Vec3::ZERO),
                &[(Vec3::ZERO, 4.0)],
                &[(Vec3::ZERO, Vec3::ZERO)],
                &SteeringWeights::default(),
                10.0,
                100.0,
                (2.0, 5.0, 5.0),
            ),
            "combined/all-degenerate",
        );
    }
}
