use crate::components::{CharacterController, Collider, Transform, Velocity};
use gizmo_math::Vec3;
use gizmo_core::entity::Entity;
use crate::raycast::{Ray, Raycast};

pub fn update_character(
    _entity: Entity,
    kcc: &mut CharacterController,
    transform: &mut Transform,
    _vel: &mut Velocity,
    colliders: &[(Entity, Transform, Collider)],
    dt: f32,
) {
    // 1. Raycast downwards to check for ground
    let foot_pos = transform.position - Vec3::new(0.0, kcc.height * 0.5 - kcc.radius, 0.0);
    let ray = Ray::new(foot_pos, Vec3::new(0.0, -1.0, 0.0));
    
    let mut ground_dist = f32::MAX;
    let mut ground_normal = Vec3::ZERO;
    
    // Allow a small threshold for ground detection to account for numerical errors
    let grounded_threshold = kcc.radius + 0.1;

    for (col_ent, col_trans, col) in colliders {
        if *col_ent == _entity { continue; } // Skip self
        
        if let Some((d, n)) = Raycast::ray_shape(&ray, &col.shape, col_trans) {
            if d < ground_dist {
                ground_dist = d;
                ground_normal = n;
            }
        }
    }

    let _was_grounded = kcc.is_grounded;
    kcc.is_grounded = ground_dist <= grounded_threshold;
    
    // Snap to ground if close enough
    if kcc.is_grounded && ground_dist < grounded_threshold {
        // Prevent bouncing when going down slopes by snapping to ground
        if kcc.velocity.y <= 0.0 {
            transform.position.y -= ground_dist - kcc.radius;
            kcc.velocity.y = 0.0;
        }
    }

    // 2. Apply Gravity
    if !kcc.is_grounded {
        kcc.velocity.y -= kcc.gravity * dt;
    }

    // 3. Horizontal Movement (sliding along ground/slopes)
    let mut desired_move = kcc.target_velocity;
    
    if kcc.is_grounded {
        // Project desired move onto the ground plane
        let slope_angle = ground_normal.angle_between(Vec3::new(0.0, 1.0, 0.0));
        
        if slope_angle <= kcc.max_slope_angle {
            // Can walk on this slope
            desired_move = desired_move - ground_normal * desired_move.dot(ground_normal);
            if desired_move.length_squared() > 1e-6 {
                desired_move = desired_move.normalize() * kcc.target_velocity.length();
            }
        } else {
            // Sliding down steep slope
            let slide_dir = Vec3::new(ground_normal.x, 0.0, ground_normal.z).normalize_or_zero();
            desired_move += slide_dir * kcc.gravity * dt;
        }
    }

    // Assign Horizontal Velocity
    kcc.velocity.x = desired_move.x;
    kcc.velocity.z = desired_move.z;

    // 4. Collision Sliding against Walls
    let move_delta = kcc.velocity * dt;
    let mut final_delta = move_delta;

    // Simple multi-iteration slide algorithm
    let mut current_pos = transform.position;
    for _ in 0..3 { // 3 iterations for slide resolution
        let mut hit = false;
        let mut closest_n = Vec3::ZERO;
        let mut min_t = 1.0;
        let move_dist = final_delta.length();
        if move_dist < 1e-4 { break; }
        
        let move_dir = final_delta.normalize_or_zero();

        // Sweep from multiple heights (Foot, Center, Head) to approximate capsule collision
        let sweep_heights = [
            -kcc.height * 0.5 + kcc.radius, // Foot
            0.0,                             // Center
            kcc.height * 0.5 - kcc.radius,  // Head
        ];

        for h_offset in sweep_heights {
            let origin = current_pos + Vec3::new(0.0, h_offset, 0.0);
            let move_ray = Ray::new(origin, move_dir);
            
            for (col_ent, col_trans, col) in colliders {
                if *col_ent == _entity { continue; }
                
                if let Some((d, n)) = Raycast::ray_shape(&move_ray, &col.shape, col_trans) {
                    let actual_d = d - kcc.radius;
                    if actual_d >= 0.0 && actual_d < move_dist * min_t {
                        min_t = actual_d / move_dist;
                        closest_n = n;
                        hit = true;
                    }
                }
            }
        }

        if hit {
            // Move up to the wall
            current_pos += move_dir * (move_dist * min_t).max(0.0);
            
            // Slide along the wall
            let remaining_dist = move_dist * (1.0 - min_t);
            let remaining_dir = move_dir;
            
            let slide_dir = remaining_dir - closest_n * remaining_dir.dot(closest_n);
            final_delta = slide_dir * remaining_dist;
            
            // Also adjust actual velocity to kill speed against wall
            kcc.velocity = kcc.velocity - closest_n * kcc.velocity.dot(closest_n);
        } else {
            current_pos += final_delta;
            break;
        }
    }

    transform.position = current_pos;
}
