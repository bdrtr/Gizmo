use crate::components::{CharacterController, Collider, Transform, Velocity};
use gizmo_math::Vec3;
use gizmo_core::entity::Entity;
use crate::raycast::{Ray, Raycast};

pub fn update_character(
    _entity: Entity,
    kcc: &mut CharacterController,
    transform: &mut Transform,
    vel: &mut Velocity,
    collider: &Collider,
    colliders: &[(Entity, Transform, Collider)],
    dt: f32,
) {
    // Determine size from collider
    let (height, radius) = match &collider.shape {
        crate::components::ColliderShape::Capsule(c) => (c.half_height * 2.0 + c.radius * 2.0, c.radius),
        crate::components::ColliderShape::Box(b) => (b.half_extents.y * 2.0, b.half_extents.x.min(b.half_extents.z)),
        crate::components::ColliderShape::Sphere(s) => (s.radius * 2.0, s.radius),
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
    let max_move_dist = (kcc.target_velocity.length() + kcc.gravity * dt + kcc.jump_speed) * dt * 2.0;
    let char_aabb = gizmo_math::Aabb {
        min: (transform.position - Vec3::splat(height.max(radius) + max_move_dist)).into(),
        max: (transform.position + Vec3::splat(height.max(radius) + max_move_dist)).into(),
    };

    let mut near_colliders = Vec::new();
    for col_data in colliders {
        if col_data.0 == _entity { continue; }
        let aabb = col_data.2.compute_aabb(col_data.1.position, col_data.1.rotation);
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
    if kcc.is_grounded {
        if vel.linear.y <= 0.0 {
            transform.position.y -= ground_dist - radius;
            vel.linear.y = 0.0;
        }
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
            // Sliding down steep slope using new slope_slide_speed
            let slide_dir = Vec3::new(ground_normal.x, 0.0, ground_normal.z).normalize_or_zero();
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
        if move_dist < 1e-4 { break; }
        
        let move_dir = final_delta.normalize_or_zero();

        let sweep_heights = [
            -height * 0.5 + radius,
            0.0,
            height * 0.5 - radius,
        ];

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
                        if let Some((d, n)) = Raycast::ray_shape(&step_down_ray, &col.shape, col_trans) {
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

            if !stepped {
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
