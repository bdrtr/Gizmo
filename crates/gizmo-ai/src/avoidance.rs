//! **Reciprocal collision avoidance (ORCA)** — how a navigation agent picks a velocity that will not
//! run into its neighbours over the next few seconds, the way Godot's `NavigationAgent` avoidance and
//! Unity's `NavMeshAgent` obstacle avoidance do (both are RVO2's algorithm, as this is).
//!
//! Each neighbour rules out, for the next [`time_horizon`](Neighbour) seconds, a half-plane of the
//! agent's velocity space: the velocities that would bring the two closer than their radii allow,
//! with the agent taking **half** the responsibility for the change (the neighbour, running the same
//! rule, takes the other half — what makes it reciprocal, and what keeps two agents from both
//! swerving the same way and swerving back). The new velocity is the one nearest the agent's
//! preferred one that every half-plane allows, within its top speed — a small 2D linear program;
//! when the half-planes leave nothing (a crowd closing in), the velocity that breaks them least.
//!
//! It works in the XZ plane, `Vec2(x, z)`: agents walk on the ground. A neighbour that does not
//! avoid ([`Neighbour::reciprocal`] false — a player, an agent with avoidance off) is treated as
//! moving on regardless, and the agent takes the whole of the change.
//!
//! Port of the agent half of RVO2 (van den Berg, Guy, Snape, Lin, Manocha; Apache-2.0), its
//! `linearProgram1/2/3` included.

use gizmo_math::Vec2;

const EPSILON: f32 = 1.0e-5;
/// How far, in radians, an agent meeting a neighbour head-on turns its preference to the right (see
/// [`avoid`]).
const KEEP_RIGHT: f32 = 0.06;
/// The sine of how nearly straight at a neighbour the closing velocity must point to be head-on.
const HEAD_ON: f32 = 0.05;

/// One neighbour, as the agent sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Neighbour {
    /// Where it is, XZ.
    pub position: Vec2,
    /// How it is moving, XZ.
    pub velocity: Vec2,
    /// Its body's radius.
    pub radius: f32,
    /// Whether it avoids too, and so takes half of the change.
    pub reciprocal: bool,
}

/// A half-plane of allowed velocities: those on the left of `direction` through `point`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Line {
    point: Vec2,
    direction: Vec2,
}

fn det(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// The velocity an agent at `position` moving at `velocity`, of `radius`, wanting to move at
/// `preferred`, no faster than `max_speed`, should take among `neighbours` so as not to touch any
/// of them within `time_horizon` seconds; `time_step` is the frame's, used when two already overlap
/// (to separate them within it).
#[must_use]
pub fn avoid(
    position: Vec2,
    velocity: Vec2,
    radius: f32,
    preferred: Vec2,
    max_speed: f32,
    neighbours: &[Neighbour],
    time_horizon: f32,
    time_step: f32,
) -> Vec2 {
    let inv_horizon = 1.0 / time_horizon.max(1.0e-3);
    let lines: Vec<Line> = neighbours
        .iter()
        .map(|other| {
            let relative_position = other.position - position;
            let relative_velocity = velocity - other.velocity;
            let dist_sq = relative_position.length_squared();
            let combined = radius + other.radius;
            let combined_sq = combined * combined;
            let (direction, u);
            if dist_sq > combined_sq {
                // No collision yet: the velocity obstacle's cut-off circle or one of its legs.
                let w = relative_velocity - inv_horizon * relative_position;
                let w_len_sq = w.length_squared();
                let dot1 = w.dot(relative_position);
                if dot1 < 0.0 && dot1 * dot1 > combined_sq * w_len_sq {
                    let w_len = w_len_sq.sqrt();
                    let unit_w = w / w_len.max(EPSILON);
                    direction = Vec2::new(unit_w.y, -unit_w.x);
                    u = (combined * inv_horizon - w_len) * unit_w;
                } else {
                    let leg = (dist_sq - combined_sq).sqrt();
                    direction = if det(relative_position, w) > 0.0 {
                        Vec2::new(
                            relative_position.x * leg - relative_position.y * combined,
                            relative_position.x * combined + relative_position.y * leg,
                        ) / dist_sq
                    } else {
                        -Vec2::new(
                            relative_position.x * leg + relative_position.y * combined,
                            -relative_position.x * combined + relative_position.y * leg,
                        ) / dist_sq
                    };
                    u = relative_velocity.dot(direction) * direction - relative_velocity;
                }
            } else {
                // Already overlapping: get apart within this frame.
                let inv_step = 1.0 / time_step.max(1.0e-4);
                let w = relative_velocity - inv_step * relative_position;
                let w_len = w.length();
                let unit_w = if w_len > EPSILON { w / w_len } else { Vec2::X };
                direction = Vec2::new(unit_w.y, -unit_w.x);
                u = (combined * inv_step - w_len) * unit_w;
            }
            let share = if other.reciprocal { 0.5 } else { 1.0 };
            Line { point: velocity + share * u, direction }
        })
        .collect();

    // Keep to the right. Two agents meeting exactly head-on are a mirror image of each other, and
    // the velocity nearest each one's preference that both half-planes allow is to slow down
    // straight ahead — so they stop, face to face, for good (RVO2's demos break it with random
    // noise; a deterministic engine cannot). Turned a few degrees to its right, each prefers to pass
    // on its right, and the mirror is broken the same way for both. Only while some neighbour is met
    // head-on: the preference is this frame's velocity carried on, and turned every frame it would
    // walk the agent round in circles.
    let head_on = neighbours.iter().any(|other| {
        let to = other.position - position;
        let closing = velocity - other.velocity;
        closing.length_squared() > 1.0e-4 && closing.dot(to) > 0.0 && det(to.normalize_or_zero(), closing.normalize_or_zero()).abs() < HEAD_ON
    });
    let preferred = if head_on { Vec2::from_angle(-KEEP_RIGHT).rotate(preferred) } else { preferred };
    let mut result = Vec2::ZERO;
    let failed = linear_program2(&lines, max_speed, preferred, false, &mut result);
    if failed < lines.len() {
        linear_program3(&lines, failed, max_speed, &mut result);
    }
    result
}

/// The best velocity on line `line_no` allowed by every line before it, within `radius`; `false` if
/// there is none.
fn linear_program1(lines: &[Line], line_no: usize, radius: f32, opt: Vec2, direction_opt: bool, result: &mut Vec2) -> bool {
    let line = lines[line_no];
    let dot = line.point.dot(line.direction);
    let discriminant = dot * dot + radius * radius - line.point.length_squared();
    if discriminant < 0.0 {
        return false;
    }
    let root = discriminant.sqrt();
    let (mut t_left, mut t_right) = (-dot - root, -dot + root);
    for other in &lines[..line_no] {
        let denominator = det(line.direction, other.direction);
        let numerator = det(other.direction, line.point - other.point);
        if denominator.abs() <= EPSILON {
            if numerator < 0.0 {
                return false;
            }
            continue;
        }
        let t = numerator / denominator;
        if denominator >= 0.0 {
            t_right = t_right.min(t);
        } else {
            t_left = t_left.max(t);
        }
        if t_left > t_right {
            return false;
        }
    }
    *result = if direction_opt {
        if opt.dot(line.direction) > 0.0 { line.point + t_right * line.direction } else { line.point + t_left * line.direction }
    } else {
        let t = line.direction.dot(opt - line.point);
        line.point + t.clamp(t_left, t_right) * line.direction
    };
    true
}

/// The velocity nearest `opt` (or, `direction_opt`, furthest along it) inside `radius` that every
/// line allows; returns the index of the first line it could not satisfy, or `lines.len()`.
fn linear_program2(lines: &[Line], radius: f32, opt: Vec2, direction_opt: bool, result: &mut Vec2) -> usize {
    *result = if direction_opt {
        opt * radius
    } else if opt.length_squared() > radius * radius {
        opt.normalize_or_zero() * radius
    } else {
        opt
    };
    for (i, line) in lines.iter().enumerate() {
        if det(line.direction, line.point - *result) > 0.0 {
            let previous = *result;
            if !linear_program1(lines, i, radius, opt, direction_opt, result) {
                *result = previous;
                return i;
            }
        }
    }
    lines.len()
}

/// When no velocity satisfies every line: the one that breaks the worst of them least, from line
/// `begin` on.
fn linear_program3(lines: &[Line], begin: usize, radius: f32, result: &mut Vec2) {
    let mut distance = 0.0;
    for i in begin..lines.len() {
        if det(lines[i].direction, lines[i].point - *result) > distance {
            let mut projected = Vec::with_capacity(i);
            for j in 0..i {
                let determinant = det(lines[i].direction, lines[j].direction);
                let point = if determinant.abs() <= EPSILON {
                    if lines[i].direction.dot(lines[j].direction) > 0.0 {
                        continue;
                    }
                    0.5 * (lines[i].point + lines[j].point)
                } else {
                    lines[i].point + (det(lines[j].direction, lines[i].point - lines[j].point) / determinant) * lines[i].direction
                };
                projected.push(Line { point, direction: (lines[j].direction - lines[i].direction).normalize_or_zero() });
            }
            let previous = *result;
            let along = Vec2::new(-lines[i].direction.y, lines[i].direction.x);
            if linear_program2(&projected, radius, along, true, result) < projected.len() {
                *result = previous;
            }
            distance = det(lines[i].direction, lines[i].point - *result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steps `agents` (position, preferred velocity toward its goal) for `seconds`, each choosing its
    /// velocity by [`avoid`] among the others; returns the closest any two came, and where each ended.
    fn simulate(starts: &[(Vec2, Vec2)], radius: f32, speed: f32, seconds: f32) -> (f32, Vec<Vec2>) {
        let dt = 1.0 / 60.0;
        let mut pos: Vec<Vec2> = starts.iter().map(|s| s.0).collect();
        let goals: Vec<Vec2> = starts.iter().map(|s| s.1).collect();
        let mut vel = vec![Vec2::ZERO; pos.len()];
        let mut closest = f32::INFINITY;
        for _ in 0..(seconds / dt) as usize {
            let next: Vec<Vec2> = (0..pos.len())
                .map(|i| {
                    let to_goal = goals[i] - pos[i];
                    let preferred = if to_goal.length() > speed * dt { to_goal.normalize() * speed } else { to_goal / dt };
                    let neighbours: Vec<Neighbour> = (0..pos.len())
                        .filter(|&j| j != i)
                        .map(|j| Neighbour { position: pos[j], velocity: vel[j], radius, reciprocal: true })
                        .collect();
                    avoid(pos[i], vel[i], radius, preferred, speed, &neighbours, 2.0, dt)
                })
                .collect();
            vel = next;
            for (p, v) in pos.iter_mut().zip(&vel) {
                *p += *v * dt;
            }
            for i in 0..pos.len() {
                for j in i + 1..pos.len() {
                    closest = closest.min(pos[i].distance(pos[j]));
                }
            }
        }
        (closest, pos)
    }

    /// Two agents walking straight at each other pass without touching and both arrive. Without
    /// avoidance they would meet head-on at the middle.
    #[test]
    fn two_agents_head_on_pass_each_other() {
        let (closest, end) = simulate(&[(Vec2::new(-5.0, 0.0), Vec2::new(5.0, 0.0)), (Vec2::new(5.0, 0.0), Vec2::new(-5.0, 0.0))], 0.4, 1.5, 12.0);
        assert!(closest >= 0.8 - 0.02, "they came {closest} m apart, under their two radii");
        assert!(end[0].distance(Vec2::new(5.0, 0.0)) < 0.2 && end[1].distance(Vec2::new(-5.0, 0.0)) < 0.2, "both arrived: {end:?}");
    }

    /// Eight agents on a circle, each walking to the point across: the classic crossing, where every
    /// path goes through the middle. Nobody touches, everybody arrives.
    #[test]
    fn a_circle_of_agents_crosses_without_touching() {
        let starts: Vec<(Vec2, Vec2)> = (0..8)
            .map(|k| {
                let a = k as f32 / 8.0 * std::f32::consts::TAU;
                let p = Vec2::new(a.cos(), a.sin()) * 6.0;
                (p, -p)
            })
            .collect();
        let (closest, end) = simulate(&starts, 0.4, 1.5, 25.0);
        assert!(closest >= 0.8 - 0.05, "two of them came {closest} m apart");
        for ((_, goal), at) in starts.iter().zip(&end) {
            assert!(at.distance(*goal) < 0.3, "an agent ended {} m from its goal", at.distance(*goal));
        }
    }

    /// With nobody around, the preferred velocity is the answer, cut to the top speed.
    #[test]
    fn alone_an_agent_goes_where_it_wants_no_faster_than_it_can() {
        let v = avoid(Vec2::ZERO, Vec2::ZERO, 0.4, Vec2::new(3.0, 4.0), 2.0, &[], 2.0, 1.0 / 60.0);
        assert!((v - Vec2::new(1.2, 1.6)).length() < 1e-5, "{v:?}");
    }

    /// A neighbour that does not avoid is left to come on: the agent steps fully out of its way.
    #[test]
    fn an_agent_gets_out_of_the_way_of_one_that_does_not_avoid() {
        let (r, speed, dt) = (0.4, 1.5, 1.0 / 60.0);
        let mut me = Vec2::new(0.0, 0.0);
        let mut mine = Vec2::ZERO;
        let mut it = Vec2::new(6.0, 0.05);
        let its = Vec2::new(-1.5, 0.0);
        let mut closest = f32::INFINITY;
        for _ in 0..(8.0 / dt) as usize {
            let n = Neighbour { position: it, velocity: its, radius: r, reciprocal: false };
            mine = avoid(me, mine, r, Vec2::ZERO, speed, &[n], 2.0, dt);
            me += mine * dt;
            it += its * dt;
            closest = closest.min(me.distance(it));
        }
        assert!(closest >= 0.8 - 0.02, "it came within {closest} m of an agent standing in its way");
    }
}
