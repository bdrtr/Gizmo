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
/// Each contribution is the escape direction scaled by `radius / distance - 1` — zero at the edge
/// of the obstacle's radius, 1 at half of it, growing without bound towards its centre — and the
/// contributions of overlapping obstacles are summed. The sum's direction is then kept while its
/// magnitude is remapped to `min(|sum|, 1) * max_speed`: the escape speed rises **continuously**
/// from nothing at the radius to full `max_speed` within half of it, the same at any scale.
///
/// It used to be `1 / distance` in inverse metres, which had two faults (see [`separate`], where
/// they were measured): a jump at the radius, from `max_speed / radius` straight to nothing, and a
/// saturation knee at a hard-coded 1 m, so the response depended on the world's unit — under a
/// 1 m radius it was all-or-nothing.
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
            // Engele yaklaşıldıkça kaçış artar: yarıçapta sıfır, yarısında 1.
            let force = diff / dist * (obs_radius / dist - 1.0);
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
/// to [`avoid_obstacles`]: each neighbour adds its escape direction scaled by
/// `separation_radius / distance - 1`, and the sum is rescaled to `min(|sum|, 1) * max_speed`.
/// A single neighbour pushes with nothing at the radius and full `max_speed` within half of it.
///
/// It used to add `(agent - neighbour) / distance²` — a push of `max_speed / distance` that
/// jumped from `max_speed / separation_radius` to nothing as a neighbour crossed the radius, and
/// saturated at a hard-coded 1 m whatever the radius. A flock packed near its radius chattered
/// across that edge and never came to rest: measured in `coban`, twelve sheep penned and braking
/// to graze still moved at 0.04 m/s after a minute; with the push falling to zero at the radius
/// they settle (0.007 m/s).
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
            let force = diff / dist * (separation_radius / dist - 1.0);
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

/// Flocking alignment: matches the agent's velocity to the average velocity of its neighbours —
/// Reynolds' *velocity matching*, heading and speed both.
///
/// `neighbors` are `(position, linear_velocity)` pairs in metres and m/s; only those within
/// `alignment_radius` metres of the agent contribute. The desired velocity is their mean, capped
/// at `max_speed`: a flock walking slowly asks for a slow walk, and a flock at rest asks for rest
/// (the result is `-current_vel` clamped to `max_force`, so the agent brakes with it).
///
/// It used to **normalise** the mean, asking for full `max_speed` along the flock's heading
/// however slowly the flock moved. Nothing could then bring a flock to a stop: measured in
/// `coban`, six sheep that had split from the flock walked on at 0.47 m/s for over a minute, the
/// dog 46 m away and each sheep braking to graze — every bit of speed the brake took, the
/// alignment asked back.
///
/// Returns [`Vec3::ZERO`] when no neighbour is in range. Opposing headings cancel to a zero mean,
/// and the agent brakes.
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
        let desired_velocity = avg_vel.clamp_length_max(max_speed);
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

    /// Weight on the [`arrive`] term toward the optional target, which slows the agent inside
    /// [`slowing_radius`](Self::slowing_radius) and stops it on the target. Default `0.0` (off).
    ///
    /// Gated like the flocking weights. To steer to a stop, set this and set
    /// [`seek`](Self::seek) to `0.0`: with both, the seek term still pulls at full speed. Until
    /// 2026-11-22 this field was read by nothing — it defaulted to `1.0` and the target term was
    /// always a seek — so its default is now the `0.0` that matches what every caller got.
    pub arrive: f32,
    /// The radius in metres inside which the [`arrive`](Self::arrive) term slows down. Default
    /// `2.0`.
    pub slowing_radius: f32,

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
    /// [`alignment`] asks for the neighbours' mean velocity, so this weights how strongly an agent
    /// keeps the flock's pace — a flock at rest included.
    pub alignment: f32,
}

impl Default for SteeringWeights {
    fn default() -> Self {
        Self {
            seek: 1.0,
            arrive: 0.0,
            slowing_radius: 2.0,
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
/// - `target_pos` — `None` drops the goal terms. `Some` runs [`seek`] weighted by
///   [`SteeringWeights::seek`], and [`arrive`] weighted by [`SteeringWeights::arrive`] when that
///   is positive; set the seek weight to `0.0` and the arrive weight above it to steer to a
///   stop.
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
        if weights.arrive > 0.0 {
            total_force += arrive(current_pos, target, current_vel, max_speed, max_force, weights.slowing_radius)
                * weights.arrive;
        }
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

    /// Alignment matches the neighbours' velocity, speed included: a slow flock asks for its slow
    /// pace, not for full speed along its heading. And a group whose agents brake comes to a stop
    /// with alignment on — before, the normalised heading asked back every bit of speed the brake
    /// took, and two braking agents settled at a steady walk of 1.11 m/s forever.
    #[test]
    fn alignment_matches_speed_so_a_braking_flock_stops() {
        let slow = [(Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.1, 0.0, 0.0))];
        let s = alignment(Vec3::ZERO, Vec3::ZERO, &slow, 3.0, 2.0, 100.0);
        assert!((s - Vec3::new(0.1, 0.0, 0.0)).length() < 1e-6, "asks for the flock's 0.1 m/s, got {s:?}");
        let fast = [(Vec3::new(1.0, 0.0, 0.0), Vec3::new(9.0, 0.0, 0.0))];
        let s = alignment(Vec3::ZERO, Vec3::ZERO, &fast, 3.0, 2.0, 100.0);
        assert!((s.length() - 2.0).abs() < 1e-6, "capped at max_speed, got {s:?}");

        // Two agents walking side by side, each braking (−0.8 v), aligned with each other.
        let mut agents = [(Vec3::ZERO, Vec3::new(0.5, 0.0, 0.0)), (Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.5, 0.0, 0.0))];
        let dt = 1.0 / 60.0;
        for _ in 0..600 {
            let before = agents;
            for (i, agent) in agents.iter_mut().enumerate() {
                let other = [before[1 - i]];
                let steer = alignment(agent.0, agent.1, &other, 3.0, 2.0, 4.0) - agent.1 * 0.8;
                agent.1 = (agent.1 + steer.clamp_length_max(4.0) * dt).clamp_length_max(2.0);
                agent.0 += agent.1 * dt;
            }
        }
        let speed = agents[0].1.length().max(agents[1].1.length());
        assert!(speed < 0.01, "ten seconds of braking stops an aligned pair: {speed:.3} m/s");
    }

    /// The push falls to nothing at the radius — no jump as a neighbour or an obstacle crosses it —
    /// reaches full speed at half the radius, and is the same at any scale. Before, a neighbour
    /// just inside a 1.2 m radius pushed at 84 % of `max_speed` and one just outside at nothing,
    /// and the saturation knee sat at 1 m whatever the radius.
    #[test]
    fn separation_and_avoidance_fall_to_zero_at_the_radius_and_ignore_scale() {
        let (speed, force) = (2.0, 1000.0);
        let sep = |d: f32, r: f32| separate(Vec3::ZERO, Vec3::ZERO, &[Vec3::new(d, 0.0, 0.0)], r, speed, force).length();
        let avoid = |d: f32, r: f32| avoid_obstacles(Vec3::ZERO, Vec3::ZERO, &[(Vec3::new(d, 0.0, 0.0), r)], speed, force).length();
        for push in [&sep as &dyn Fn(f32, f32) -> f32, &avoid] {
            assert!(push(1.19, 1.2) < 0.02 * speed, "just inside the radius: {:.3}", push(1.19, 1.2));
            assert!((push(0.6, 1.2) - speed).abs() < 1e-4, "full speed at half the radius: {:.3}", push(0.6, 1.2));
            assert!(push(0.9, 1.2) > push(1.1, 1.2), "closer pushes harder");
            // Ten times the size, the same push.
            for (d, r) in [(0.3, 1.2), (0.9, 1.2), (1.1, 1.2)] {
                assert!((push(d, r) - push(10.0 * d, 10.0 * r)).abs() < 1e-4, "scale-free at d = {d}");
            }
        }
    }

    /// The arrive weight steers an agent to a stop on its target. It was read by nothing, and the
    /// target term was always a seek: with the seek weight at zero the agent never moved.
    #[test]
    fn the_arrive_weight_brings_an_agent_to_a_stop_on_its_target() {
        let weights = SteeringWeights { seek: 0.0, arrive: 1.0, ..Default::default() };
        let target = Vec3::new(6.0, 0.0, 0.0);
        let (mut pos, mut vel) = (Vec3::ZERO, Vec3::ZERO);
        let dt = 1.0 / 60.0;
        for _ in 0..60 * 20 {
            let steer = combined_steering(pos, vel, Some(target), &[], &[], &weights, 3.0, 6.0, (1.0, 1.0, 1.0));
            vel = (vel + steer * dt).clamp_length_max(3.0);
            pos += vel * dt;
        }
        assert!(pos.distance(target) < 0.05 && vel.length() < 0.05, "stopped on the target: at {pos:?}, {vel:?}");
        assert_eq!(SteeringWeights::default().arrive, 0.0, "off by default, as it was in effect");
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
