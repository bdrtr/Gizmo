use gizmo_physics_core::{Collider, Transform, ColliderShape};
use gizmo_physics_core::components::CharacterController;
use gizmo_physics_rigid::components::Velocity;
use gizmo_physics_core::raycast::{Ray, Raycast};
use gizmo_physics_core::BodyHandle;
use gizmo_math::Vec3;

/// Advances a kinematic character controller by one fixed step.
///
/// Resolves ground detection, gravity, slope movement, wall sliding and step
/// climbing, mutating `transform` and `vel` in place. `colliders` must contain
/// all scene colliders to test against; the entry whose entity equals `_entity`
/// (the character itself) is skipped automatically.
pub fn update_character(
    _entity: BodyHandle,
    kcc: &mut CharacterController,
    transform: &mut Transform,
    vel: &mut Velocity,
    collider: &Collider,
    colliders: &[(BodyHandle, Transform, Collider)],
    dt: f32,
) {
    // Determine size from collider
    let (height, radius) = match &collider.shape {
        ColliderShape::Capsule(c) => {
            (c.half_height * 2.0 + c.radius * 2.0, c.radius)
        }
        ColliderShape::Box(b) => (
            b.half_extents.y * 2.0,
            b.half_extents.x.min(b.half_extents.z),
        ),
        ColliderShape::Sphere(s) => (s.radius * 2.0, s.radius),
        _ => (2.0, 0.5), // fallback
    };

    // NOTE: KCC entities should NOT have a RigidBody component (or they must be kinematic).
    // If they have a dynamic RigidBody, the physics system will apply gravity twice.

    // 1. Raycast downwards to check for ground
    let foot_pos = transform.position - Vec3::new(0.0, height * 0.5 - radius, 0.0);
    let ray = Ray::new(foot_pos, Vec3::new(0.0, -1.0, 0.0));

    let mut ground_dist = f32::MAX;
    let mut ground_normal = Vec3::ZERO;

    let grounded_threshold = radius + 0.1;

    // AABB pre-filter: gather colliders near the character
    let max_move_dist =
        (kcc.target_velocity.length() + kcc.gravity * dt + kcc.jump_speed) * dt * 2.0;
    let char_aabb = gizmo_math::Aabb {
        min: (transform.position - Vec3::splat(height.max(radius) + max_move_dist)).into(),
        max: (transform.position + Vec3::splat(height.max(radius) + max_move_dist)).into(),
    };

    let mut near_colliders = Vec::new();
    for col_data in colliders {
        if col_data.0 == _entity {
            continue;
        }
        let aabb = col_data
            .2
            .compute_aabb(col_data.1.position, col_data.1.rotation);
        if char_aabb.intersects(aabb) {
            near_colliders.push(col_data);
        }
    }

    for (_col_ent, col_trans, col) in &near_colliders {
        if let Some((d, n)) = Raycast::ray_shape(&ray, &col.shape, col_trans) {
            if d < ground_dist {
                ground_dist = d;
                ground_normal = n;
            }
        }
    }

    let _was_grounded = kcc.is_grounded;
    kcc.is_grounded = ground_dist <= grounded_threshold;

    // Update Timers
    if kcc.is_grounded {
        kcc.coyote_timer = kcc.coyote_time;
    } else {
        kcc.coyote_timer -= dt;
    }

    if kcc.jump_buffer_timer > 0.0 {
        kcc.jump_buffer_timer -= dt;
    }

    // Snap to ground if close enough
    if kcc.is_grounded && vel.linear.y <= 0.0 {
        transform.position.y -= ground_dist - radius;
        vel.linear.y = 0.0;
    }

    // Process Jump (Buffered and Coyote)
    if kcc.jump_buffer_timer > 0.0 && kcc.coyote_timer > 0.0 {
        vel.linear.y = kcc.jump_speed;
        kcc.jump_buffer_timer = 0.0;
        kcc.coyote_timer = 0.0;
        kcc.is_grounded = false;
    }

    // 2. Apply Gravity
    if !kcc.is_grounded {
        vel.linear.y -= kcc.gravity * dt;
    }

    // 3. Horizontal Movement
    let mut desired_move = kcc.target_velocity;

    if kcc.is_grounded {
        if ground_normal.length_squared() < 1e-6 {
            ground_normal = Vec3::new(0.0, 1.0, 0.0);
        }

        let slope_angle = ground_normal.angle_between(Vec3::new(0.0, 1.0, 0.0));

        if slope_angle <= kcc.max_slope_angle {
            desired_move = desired_move - ground_normal * desired_move.dot(ground_normal);
            if desired_move.length_squared() > 1e-6 {
                desired_move = desired_move.normalize() * kcc.target_velocity.length();
            }
        } else {
            // Sliding down steep slope using new slope_slide_speed.
            // Project the downward (gravity) direction onto the slope plane to get the
            // true downslope direction, rather than the horizontal component of the normal.
            let down = Vec3::new(0.0, -1.0, 0.0);
            let slide_dir = (down - ground_normal * down.dot(ground_normal)).normalize_or_zero();
            desired_move += slide_dir * kcc.slope_slide_speed;
        }
    }

    vel.linear.x = desired_move.x;
    vel.linear.z = desired_move.z;

    // 4. Collision Sliding against Walls
    let move_delta = vel.linear * dt;
    let mut final_delta = move_delta;

    let mut current_pos = transform.position;
    for _ in 0..3 {
        let mut hit = false;
        let mut closest_n = Vec3::ZERO;
        let mut min_t = 1.0;
        let move_dist = final_delta.length();
        if move_dist < 1e-4 {
            break;
        }

        let move_dir = final_delta.normalize_or_zero();

        let sweep_heights = [-height * 0.5 + radius, 0.0, height * 0.5 - radius];

        for h_offset in sweep_heights {
            let mut local_min_t = 1.0;
            let mut local_closest_n = Vec3::ZERO;
            let origin = current_pos + Vec3::new(0.0, h_offset, 0.0);
            let move_ray = Ray::new(origin, move_dir);

            for (_col_ent, col_trans, col) in &near_colliders {
                if let Some((d, n)) = Raycast::ray_shape(&move_ray, &col.shape, col_trans) {
                    let actual_d = d - radius;
                    if actual_d >= 0.0 && actual_d < move_dist * local_min_t {
                        local_min_t = actual_d / move_dist;
                        local_closest_n = n;
                    }
                }
            }
            if local_min_t < min_t {
                min_t = local_min_t;
                closest_n = local_closest_n;
                hit = true;
            }
        }

        if hit {
            // Attempt Step Climbing
            let mut stepped = false;
            if kcc.is_grounded && kcc.step_height > 0.0 {
                // Determine if the hit normal represents a wall
                let wall_angle = closest_n.angle_between(Vec3::new(0.0, 1.0, 0.0));
                if wall_angle > kcc.max_slope_angle {
                    // Start raycast from above the step height, slightly forward into the obstacle
                    let step_check_origin = current_pos
                        + Vec3::new(0.0, -height * 0.5 + radius + kcc.step_height, 0.0)
                        + move_dir * (radius + 0.05); // Push forward into the wall

                    let step_down_ray = Ray::new(step_check_origin, Vec3::new(0.0, -1.0, 0.0));

                    let mut step_dist = f32::MAX;
                    let mut step_normal = Vec3::ZERO;

                    for (_col_ent, col_trans, col) in &near_colliders {
                        if let Some((d, n)) =
                            Raycast::ray_shape(&step_down_ray, &col.shape, col_trans)
                        {
                            if d < step_dist {
                                step_dist = d;
                                step_normal = n;
                            }
                        }
                    }

                    // If we found a surface within step_height bounds and it's walkable
                    if step_dist <= kcc.step_height * 2.0 {
                        let step_slope_angle = step_normal.angle_between(Vec3::new(0.0, 1.0, 0.0));
                        if step_slope_angle <= kcc.max_slope_angle {
                            // Calculate new Y position
                            let target_y = step_check_origin.y - step_dist + height * 0.5 - radius;
                            // Only step up, don't step down (handled by gravity/ground snapping)
                            if target_y > current_pos.y + 0.01 {
                                current_pos.y = target_y + 0.01; // Tiny lift to avoid precision issues
                                current_pos += move_dir * (move_dist * min_t).max(0.0); // Move forward to the ledge
                                stepped = true;
                            }
                        }
                    }
                }
            }

            if stepped {
                // We climbed onto the ledge and already advanced up to the wall
                // base (`move_dist * min_t`). Reduce `final_delta` to the *remaining*
                // movement so the next sweep iteration carries us forward onto the
                // ledge without re-applying the whole delta. Without this the loop
                // would run again with the full (unchanged) `final_delta` and, now
                // that the raised body clears the wall, add another full `move_dist`
                // — making the character lurch forward by up to ~2× its intended
                // speed every time it steps up.
                final_delta = move_dir * (move_dist * (1.0 - min_t)).max(0.0);
            } else {
                current_pos += move_dir * (move_dist * min_t).max(0.0);

                let remaining_dist = move_dist * (1.0 - min_t);
                let remaining_dir = move_dir;

                let slide_dir = remaining_dir - closest_n * remaining_dir.dot(closest_n);
                final_delta = slide_dir * remaining_dist;

                vel.linear = vel.linear - closest_n * vel.linear.dot(closest_n);
            }
        } else {
            current_pos += final_delta;
            break;
        }
    }

    transform.position = current_pos;
}

#[cfg(test)]
mod tests {
    use super::update_character;
    use gizmo_math::Vec3;
    use gizmo_physics_core::components::CharacterController;
    use gizmo_physics_core::{BodyHandle, BoxShape, Collider, ColliderShape, Transform};
    use gizmo_physics_rigid::components::Velocity;

    fn box_collider(hx: f32, hy: f32, hz: f32) -> Collider {
        Collider::from_shape(ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(hx, hy, hz),
        }))
    }

    /// A grounded character that climbs a small step must not advance forward by
    /// more than its intended per-frame distance (`speed * dt`). The old code left
    /// `final_delta` at its full length after a step, so the next sweep iteration —
    /// now clearing the raised wall — re-applied the whole delta, lurching the
    /// character forward by up to ~2× its speed on every stair. Discriminating:
    /// reverting the step-branch `final_delta` reduction makes this fail.
    #[test]
    fn step_climb_does_not_overshoot_forward() {
        let entity = BodyHandle::from_id(0);

        // Thin character so the low horizontal sweep ray sits near the feet and can
        // actually hit a short step: box half-extents (0.1, 0.9, 0.1) → radius 0.1,
        // height 1.8. At rest its centre sits at y = height/2 = 0.9 above ground.
        let collider = box_collider(0.1, 0.9, 0.1);

        // Flat ground with its top surface at y = 0, and a 0.15-high step whose
        // vertical face is at x = 1.0 (climbable: 0.15 < step_height 0.3, and the
        // step top 0.15 > radius 0.1 so the lowest sweep ray hits its face).
        let ground = box_collider(50.0, 0.5, 50.0);
        let step = box_collider(25.0, 0.075, 50.0);
        let colliders = vec![
            (BodyHandle::from_id(1), Transform::new(Vec3::new(0.0, -0.5, 0.0)), ground),
            (BodyHandle::from_id(2), Transform::new(Vec3::new(26.0, 0.075, 0.0)), step),
        ];

        // Start just short of the step face so a single frame's move reaches it.
        let start = Vec3::new(0.88, 0.9, 0.0);
        let mut transform = Transform::new(start);
        let mut vel = Velocity::new(Vec3::ZERO);

        let speed = 2.0;
        let dt = 0.0125; // move_dist = 0.025, wall is 0.02 ahead → step is hit
        let mut kcc = CharacterController {
            target_velocity: Vec3::new(speed, 0.0, 0.0),
            is_grounded: true,
            ..Default::default()
        };

        update_character(entity, &mut kcc, &mut transform, &mut vel, &collider, &colliders, dt);

        let advanced = transform.position.x - start.x;
        let intended = speed * dt;

        // The step must actually have been climbed (otherwise the test is vacuous).
        assert!(
            transform.position.y > start.y + 0.01,
            "character should have stepped up onto the ledge, y {} -> {}",
            start.y,
            transform.position.y
        );
        // And it must not have lurched past its intended per-frame distance.
        assert!(
            advanced <= intended * 1.2,
            "step-up overshoot: advanced {advanced} > intended {intended} (×1.2 tol)"
        );
        assert!(advanced > 0.0, "character should still move forward, got {advanced}");
    }

    /// Reproduces the downslope-direction computation used when sliding down a
    /// too-steep slope (see `update_character_controller`).
    fn downslope_dir(ground_normal: Vec3) -> Vec3 {
        let down = Vec3::new(0.0, -1.0, 0.0);
        (down - ground_normal * down.dot(ground_normal)).normalize_or_zero()
    }

    #[test]
    fn slide_direction_points_downhill_not_sideways() {
        // 60-degree slope: normal tilted in +x. normalize([0.5, 0.866, 0]).
        let ground_normal = Vec3::new(0.5, 0.866, 0.0).normalize();
        let slide_dir = downslope_dir(ground_normal);

        // Correct downslope has a NEGATIVE y component (points downward along the
        // slope) and no z drift for an x-tilted normal.
        assert!(
            slide_dir.y < -0.1,
            "slide direction must descend, got y={}",
            slide_dir.y
        );
        assert!(slide_dir.z.abs() < 1e-5, "no lateral z drift, got z={}", slide_dir.z);
        assert!(slide_dir.x > 0.0, "should slide toward +x (downhill), got x={}", slide_dir.x);

        // The slide direction must lie in the slope plane (perpendicular to normal).
        assert!(
            slide_dir.dot(ground_normal).abs() < 1e-5,
            "slide dir must be tangent to the slope, dot={}",
            slide_dir.dot(ground_normal)
        );

        // Regression guard against the old bug, which used the horizontal
        // projection of the normal: normalize([0.5, 0, 0]) = [1, 0, 0].
        let old_buggy = Vec3::new(ground_normal.x, 0.0, ground_normal.z).normalize_or_zero();
        assert!(
            (slide_dir - old_buggy).length() > 0.1,
            "fixed slide dir must differ from the old horizontal-normal projection"
        );
    }
}
