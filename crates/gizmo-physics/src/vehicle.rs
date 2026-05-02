use gizmo_math::{Vec3, Quat};
use gizmo_core::entity::Entity;
use crate::components::{RigidBody, Transform, Velocity, Collider};
use crate::raycast::{Ray, Raycast, RaycastHit};

#[derive(Clone, Debug)]
pub struct Wheel {
    pub attachment_local_pos: Vec3,
    pub direction_local: Vec3,
    pub axle_local: Vec3,
    
    pub radius: f32,
    pub suspension_rest_length: f32,
    pub suspension_max_travel: f32,
    pub suspension_stiffness: f32,
    pub suspension_damping: f32,
    pub friction_slip: f32,
    
    // State/Input
    pub steering_angle: f32,
    pub engine_force: f32,
    pub brake_force: f32,
    
    // Internal State computed during physics step
    pub is_grounded: bool,
    pub ground_hit: Option<RaycastHit>,
    pub suspension_length: f32,
    pub rotation_angle: f32,
    pub angular_velocity: f32,
}

impl Default for Wheel {
    fn default() -> Self {
        Self {
            attachment_local_pos: Vec3::ZERO,
            direction_local: Vec3::new(0.0, -1.0, 0.0),
            axle_local: Vec3::new(-1.0, 0.0, 0.0),
            radius: 0.3,
            suspension_rest_length: 0.5,
            suspension_max_travel: 0.2,
            suspension_stiffness: 50.0,
            suspension_damping: 3.0,
            friction_slip: 10.5,
            steering_angle: 0.0,
            engine_force: 0.0,
            brake_force: 0.0,
            is_grounded: false,
            ground_hit: None,
            suspension_length: 0.5,
            rotation_angle: 0.0,
            angular_velocity: 0.0,
        }
    }
}

#[derive(Clone)]
pub struct VehicleController {
    pub wheels: Vec<Wheel>,
    pub current_speed: f32,
}

impl gizmo_core::component::Component for VehicleController {}

impl Default for VehicleController {
    fn default() -> Self {
        Self {
            wheels: Vec::new(),
            current_speed: 0.0,
        }
    }
}

impl VehicleController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_wheel(&mut self, wheel: Wheel) {
        self.wheels.push(wheel);
    }
}

/// A standalone function that calculates and applies suspension and tire forces
/// to a single vehicle based on raycasts against all static/dynamic colliders.
pub fn update_vehicle(
    vehicle_entity: Entity,
    vehicle: &mut VehicleController,
    vehicle_rb: &mut RigidBody,
    vehicle_transform: &Transform,
    vehicle_vel: &mut Velocity,
    all_colliders: &[(Entity, Transform, Collider)],
    dt: f32,
) {
    if vehicle_rb.is_static() {
        return;
    }

    let up = vehicle_transform.rotation.mul_vec3(Vec3::new(0.0, 1.0, 0.0));
    let forward = vehicle_transform.rotation.mul_vec3(Vec3::new(0.0, 0.0, -1.0));
    let right = vehicle_transform.rotation.mul_vec3(Vec3::new(1.0, 0.0, 0.0));

    // Vehicle COM velocity approximation
    let v_com = vehicle_vel.linear;
    vehicle.current_speed = v_com.dot(forward);
    
    let mass_per_wheel = vehicle_rb.mass / vehicle.wheels.len().max(1) as f32;

    for wheel in &mut vehicle.wheels {
        // Compute world-space properties of the wheel
        let attach_world = vehicle_transform.position + vehicle_transform.rotation.mul_vec3(wheel.attachment_local_pos);
        let ray_dir = vehicle_transform.rotation.mul_vec3(wheel.direction_local).normalize();
        
        let ray_length = wheel.suspension_rest_length + wheel.radius;
        let ray = Ray::new(attach_world, ray_dir);

        // Raycast against the world
        let mut closest_hit: Option<RaycastHit> = None;
        let mut closest_dist = ray_length;

        for (other_ent, other_trans, other_col) in all_colliders {
            if *other_ent == vehicle_entity || other_col.is_trigger {
                continue; // Skip self and triggers
            }

            let aabb = other_col.compute_aabb(other_trans.position, other_trans.rotation);
            if Raycast::ray_aabb(&ray, &aabb).is_none() {
                continue;
            }

            if let Some((distance, normal)) = Raycast::ray_shape(&ray, &other_col.shape, other_trans) {
                if distance < closest_dist {
                    closest_dist = distance;
                    closest_hit = Some(RaycastHit {
                        entity: *other_ent,
                        point: ray.point_at(distance),
                        normal,
                        distance,
                    });
                }
            }
        }

        // Update wheel state
        if let Some(hit) = closest_hit {
            wheel.is_grounded = true;
            wheel.ground_hit = Some(hit.clone());
            wheel.suspension_length = hit.distance - wheel.radius;
            wheel.suspension_length = wheel.suspension_length.clamp(
                wheel.suspension_rest_length - wheel.suspension_max_travel,
                wheel.suspension_rest_length + wheel.suspension_max_travel,
            );
        } else {
            wheel.is_grounded = false;
            wheel.ground_hit = None;
            wheel.suspension_length = wheel.suspension_rest_length;
        }

        // Calculate forces
        if wheel.is_grounded {
            let hit = wheel.ground_hit.as_ref().unwrap();

            // 1. Suspension Force
            let suspension_compression = wheel.suspension_rest_length - wheel.suspension_length;
            
            // Point velocity at attachment
            let point_rel = attach_world - vehicle_transform.position;
            let point_vel = vehicle_vel.linear + vehicle_vel.angular.cross(point_rel);
            
            // Velocity along the suspension direction (usually up)
            let susp_vel = point_vel.dot(ray_dir); 
            // Damping opposes velocity. Note: ray_dir points DOWN, so moving UP means susp_vel is negative.
            // So if susp_vel < 0, suspension is compressing.
            let spring_force = wheel.suspension_stiffness * suspension_compression;
            let damping_force = -wheel.suspension_damping * susp_vel;
            
            let total_susp_force = (spring_force + damping_force).max(0.0); // Suspension only pushes up
            let susp_impulse = (-ray_dir) * total_susp_force;

            // Apply suspension force to chassis
            apply_force_at_point(vehicle_rb, vehicle_vel, vehicle_transform.position, vehicle_transform.rotation, susp_impulse, attach_world, dt);

            // 2. Friction & Drive Forces (Tire model)
            let steering_rot = Quat::from_axis_angle(up, wheel.steering_angle);
            let wheel_forward = steering_rot.mul_vec3(forward).normalize();
            let wheel_right = steering_rot.mul_vec3(right).normalize();

            // Forward speed at the wheel
            let forward_speed = point_vel.dot(wheel_forward);
            let lateral_speed = point_vel.dot(wheel_right);

            // Compute the target angular velocity based on forward speed to prevent slipping
            let target_angular_vel = forward_speed / wheel.radius;
            
            // Integrate angular velocity
            let slip = target_angular_vel - wheel.angular_velocity;
            wheel.angular_velocity += (wheel.engine_force * 0.1 - wheel.brake_force * wheel.angular_velocity.signum() + slip * 0.5) * dt;
            
            // Calculate longitudinal force (Engine/Brake)
            let drive_force = wheel.engine_force;
            let brake_force = if wheel.brake_force > 0.0 && forward_speed.abs() > 0.1 {
                -forward_speed.signum() * wheel.brake_force
            } else {
                0.0
            };
            
            // Simple friction model
            let mut long_force = drive_force + brake_force;
            
            // Lateral force (prevent sliding sideways)
            let lat_force = -lateral_speed * mass_per_wheel * wheel.friction_slip / dt;

            // Cap the combined force to the friction circle
            let max_friction = total_susp_force * 1.5; // coefficient of friction
            let combined_force = (long_force.powi(2) + lat_force.powi(2)).sqrt();
            if combined_force > max_friction {
                let scale = max_friction / combined_force;
                long_force *= scale;
                //lat_force *= scale;
            }

            // Apply longitudinal and lateral forces
            let tire_force_vec = wheel_forward * long_force + wheel_right * lat_force;
            // Apply tire force at the contact point on the ground!
            let contact_point = attach_world + ray_dir * hit.distance;
            apply_force_at_point(vehicle_rb, vehicle_vel, vehicle_transform.position, vehicle_transform.rotation, tire_force_vec, contact_point, dt);
        } else {
            // Gradually slow down free-spinning wheels
            wheel.angular_velocity *= 0.95;
            wheel.angular_velocity += wheel.engine_force * 0.01 * dt;
        }

        // Update visual rotation
        wheel.rotation_angle += wheel.angular_velocity * dt;
        wheel.rotation_angle %= std::f32::consts::TAU;
    }
}

fn apply_force_at_point(
    rb: &RigidBody,
    vel: &mut Velocity,
    center_of_mass: Vec3,
    rotation: Quat,
    force: Vec3,
    point: Vec3,
    dt: f32
) {
    if rb.is_static() {
        return;
    }
    
    // Linear acceleration
    vel.linear += (force * rb.inv_mass()) * dt;

    // Angular acceleration
    let torque = (point - center_of_mass).cross(force);
    vel.angular += (rb.inv_world_inertia_tensor(rotation) * torque) * dt;
}
