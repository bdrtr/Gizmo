use crate::components::{RigidBody, Vehicle, Velocity};
use crate::world::PhysicsWorld;
use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::raycast::Ray;
use gizmo_physics_core::Transform;
use gizmo_physics_core::BodyHandle;

#[tracing::instrument(skip_all, name = "physics_vehicle_system")]
pub fn physics_vehicle_system(world: &World, dt: f32) {
    if dt <= 0.0 {
        return;
    }

    let physics_world = match world.try_get_resource_mut::<PhysicsWorld>() {
        Ok(res) => res,
        Err(_) => return,
    };

    let materials = world.borrow::<gizmo_physics_core::components::PhysicsMaterial>();

    // SAFETY: scheduled system; scheduler guarantees disjoint mutable access.
    if let Some(mut query) = unsafe {
        world.query_unchecked::<(
            &Transform,
            gizmo_core::query::Mut<RigidBody>,
            gizmo_core::query::Mut<Velocity>,
            gizmo_core::query::Mut<Vehicle>,
            gizmo_core::query::Without<gizmo_core::component::IsDeleted>,
        )>()
    } {
        for (id, (transform, mut rb, mut vel, mut vehicle, _)) in query.iter_mut() {
            // Wake up rigid body if vehicle inputs are active
            if vehicle.current_throttle.abs() > 0.01 || vehicle.current_steer.abs() > 0.01 {
                rb.is_sleeping = false;
            }

            if rb.is_sleeping {
                continue;
            }

            let chassis_pos = transform.position;
            let chassis_rot = transform.rotation;
            
            // Calculate velocity at Center of Mass
            let com_world = chassis_pos + chassis_rot * rb.center_of_mass;

            // max_steer_angle ile sınırla (eskiden ham açı doğrudan kullanılıyordu, sınır yoktu).
            let current_steer = vehicle
                .current_steer
                .clamp(-vehicle.max_steer_angle, vehicle.max_steer_angle);
            let current_throttle = vehicle.current_throttle;
            let current_brake = vehicle.current_brake;
            let brake_force = vehicle.brake_force;

            // --- Gearbox Logic ---
            let forward_dir = chassis_rot * Vec3::new(0.0, 0.0, 1.0);
            let forward_speed = vel.linear.dot(forward_dir);
            
            if vehicle.gearbox.is_automatic {
                // Auto-detect reverse
                if vehicle.current_throttle < -0.01 && forward_speed < 0.1 {
                    vehicle.gearbox.is_reversing = true;
                } else if vehicle.current_throttle > 0.01 && forward_speed > -0.1 {
                    vehicle.gearbox.is_reversing = false;
                }
                vehicle.gearbox.update_gear(forward_speed);
            }

            let gear_ratio = vehicle.gearbox.current_ratio();
            
            let engine_power = vehicle.engine_power * gear_ratio * vehicle.gearbox.final_drive;

            // --- Aerodynamics (Air Drag & Downforce) ---
            let velocity_mag = vel.linear.length();
            if velocity_mag > 0.1 {
                let velocity_dir = vel.linear / velocity_mag;
                let speed_sq = velocity_mag * velocity_mag;
                let air_density = 1.225; // kg/m^3

                // Air Drag
                let drag_force_mag = 0.5 * air_density * speed_sq * vehicle.aerodynamic_drag * vehicle.frontal_area;
                let drag_impulse = -velocity_dir * drag_force_mag * dt;
                vel.linear += drag_impulse * rb.inv_mass();

                // Downforce (pushes car down into the ground based on speed)
                let down_dir = chassis_rot * Vec3::new(0.0, -1.0, 0.0);
                let downforce_mag = 0.5 * air_density * speed_sq * vehicle.downforce_coefficient * vehicle.frontal_area;
                let downforce_impulse = down_dir * downforce_mag * dt;
                vel.linear += downforce_impulse * rb.inv_mass();
            }

            // Tekerlek başına kütle payı (eskiden 4 tekerleğe sabit `mass*0.25` varsayılıyordu).
            let per_wheel_mass = rb.mass / (vehicle.wheels.len().max(1) as f32);

            // `engine_power` is the TOTAL engine output; the drive block runs per wheel, so
            // it must be split across the driven wheels (mirrors per_wheel_mass and the arcade
            // update_vehicle's drive_torque/rear_count). Applying full output on every drive
            // wheel multiplied total traction by the drive-wheel count (a 4WD car accelerated
            // ~4× a 1WD car with identical engine_power).
            let drive_count = vehicle.wheels.iter().filter(|w| w.is_drive).count().max(1) as f32;

            for wheel in &mut vehicle.wheels {
                // Determine wheel attachment point in world space
                let wheel_world_pos = chassis_pos + chassis_rot * wheel.local_position;
                
                // Direction of suspension (usually local -Y)
                let ray_dir = (chassis_rot * wheel.direction).normalize();

                // Raycast downwards
                let max_dist = wheel.suspension_rest_length + wheel.radius;
                let ray = Ray::new(wheel_world_pos, ray_dir);

                // Exclude self from raycast
                let hit_opt = physics_world.raycast(&ray, max_dist);

                wheel.is_grounded = false;

                if let Some(hit) = hit_opt {
                    // Only consider it a ground hit if it's not the chassis itself
                    if hit.entity != BodyHandle::from_id(id) {
                        wheel.is_grounded = true;
                        wheel.contact_point = hit.point;
                        wheel.contact_normal = hit.normal;

                        // 1. Suspension Force
                        let compression = max_dist - hit.distance;
                        wheel.suspension_compression = compression;

                        // Calculate velocity at the contact point
                        let r = wheel_world_pos - com_world;
                        let point_vel = vel.linear + vel.angular.cross(r);
                        
                        let susp_vel = point_vel.dot(ray_dir);
                        
                        // F = kx + cv
                        let spring_force = wheel.suspension_stiffness * compression;
                        let damping_force = wheel.suspension_damping * susp_vel;
                        
                        let total_susp_force = (spring_force + damping_force).max(0.0);
                        let suspension_impulse = -ray_dir * total_susp_force * dt;

                        // Apply suspension impulse (yay kuvveti strut boyunca, bağlantı noktasında).
                        vel.linear += suspension_impulse * rb.inv_mass();
                        vel.angular += rb.inv_world_inertia_tensor(chassis_rot) * r.cross(suspension_impulse);

                        // Lastik kuvvetleri TEMAS NOKTASINDA etki eder (bağlantı noktasında değil);
                        // doğru kaldıraç kolu için temas yamasından ölçülen r_contact kullanılır.
                        let r_contact = wheel.contact_point - com_world;

                        // 2. Friction / Steering / Drive
                        let mut right = chassis_rot * Vec3::new(1.0, 0.0, 0.0);
                        let mut forward = chassis_rot * Vec3::new(0.0, 0.0, 1.0);
                        
                        // Project vectors onto the ground plane so the car doesn't fly like an airplane.
                        // `normalize_or_zero`: if an axis is (nearly) parallel to the contact normal
                        // the projection collapses to ~zero and bare `normalize()` would yield NaN,
                        // poisoning velocity for the rest of the sim. Zero = no force this frame instead.
                        right = (right - wheel.contact_normal * right.dot(wheel.contact_normal)).normalize_or_zero();
                        forward = (forward - wheel.contact_normal * forward.dot(wheel.contact_normal)).normalize_or_zero();
                        
                        if wheel.is_steering {
                            let steer_rot = gizmo_math::Quat::from_axis_angle(wheel.contact_normal, current_steer);
                            forward = steer_rot * forward;
                        }

                        // Temas noktasındaki hız (lastik kuvvetleri için).
                        let point_vel = vel.linear + vel.angular.cross(r_contact);

                        // Surface Material (Grip & Rolling Resistance)
                        let mat = materials.get(hit.entity.id())
                            .copied()
                            .unwrap_or(gizmo_physics_core::components::PhysicsMaterial::ASPHALT);

                        let grip_mult = mat.dynamic_friction / gizmo_physics_core::components::PhysicsMaterial::ASPHALT.dynamic_friction;
                        let current_base_grip = wheel.base_grip * grip_mult;
                        let current_drift_grip = wheel.drift_grip * grip_mult;

                        // Lateral Friction (prevent sideways sliding)
                        let lat_vel = point_vel.dot(right);
                        let base_grip = if lat_vel.abs() > wheel.slip_threshold {
                            current_drift_grip
                        } else {
                            current_base_grip
                        };
                        
                        // --- Weather Modifiers ---
                        let (grip_multiplier, rr_multiplier) = match physics_world.weather {
                            crate::world::Weather::Sunny => (1.0, 1.0),
                            crate::world::Weather::Rain => {
                                // Aquaplaning effect: Grip drops as speed increases
                                let speed = vel.linear.length();
                                let mut wet_grip = 0.5; // Base 50% penalty
                                if speed > 20.0 {
                                    wet_grip = (0.5 - (speed - 20.0) * 0.01).max(0.1);
                                }
                                (wet_grip, 1.2)
                            },
                            crate::world::Weather::Snow => {
                                // Low grip, massive resistance from snow packing
                                (0.3, 5.0) 
                            }
                        };
                        
                        let grip = base_grip * grip_multiplier;
                        
                        // Calculate max possible lateral friction based on normal load
                        let max_lat_force = grip * total_susp_force;
                        let max_lat_impulse = max_lat_force * dt;
                        
                        // Yanal kaymayı durduracak istenen impuls (tekerlek başına kütle payı).
                        let desired_lat_impulse = -lat_vel * per_wheel_mass;
                        let actual_lat_impulse_mag = desired_lat_impulse.clamp(-max_lat_impulse, max_lat_impulse);
                        let lat_impulse = right * actual_lat_impulse_mag;

                        vel.linear += lat_impulse * rb.inv_mass();
                        vel.angular += rb.inv_world_inertia_tensor(chassis_rot) * r_contact.cross(lat_impulse);

                        // Recompute point vel after lateral impulse
                        let point_vel = vel.linear + vel.angular.cross(r_contact);
                        let long_vel = point_vel.dot(forward);

                        // Rolling Resistance
                        let rolling_force_mag = wheel.rolling_resistance_coefficient * rr_multiplier * total_susp_force;
                        let max_rolling_impulse = rolling_force_mag * dt;
                        let desired_rolling_impulse = -long_vel * per_wheel_mass;
                        let actual_rolling_impulse_mag = desired_rolling_impulse.clamp(-max_rolling_impulse, max_rolling_impulse);
                        let rolling_impulse = forward * actual_rolling_impulse_mag;

                        vel.linear += rolling_impulse * rb.inv_mass();
                        vel.angular += rb.inv_world_inertia_tensor(chassis_rot) * r_contact.cross(rolling_impulse);

                        // Longitudinal Force (Drive) — sürtünme dairesine (friction circle) clamp'lenir:
                        // mevcut boyuna grip = sqrt((μ·N)² − kullanılan_yanal²). Eskiden tahrik kuvveti
                        // hiç sınırlanmıyordu → sonsuz çekiş, yanal+boyuna birleşik kayma yoktu.
                        if wheel.is_drive {
                            let max_tire_impulse = grip * total_susp_force * dt;
                            let lat_used = actual_lat_impulse_mag.abs();
                            let max_long_impulse =
                                (max_tire_impulse * max_tire_impulse - lat_used * lat_used).max(0.0).sqrt();
                            let desired_drive_impulse = current_throttle * engine_power * dt / drive_count;
                            let drive_mag = desired_drive_impulse.clamp(-max_long_impulse, max_long_impulse);
                            let drive_impulse = forward * drive_mag;
                            vel.linear += drive_impulse * rb.inv_mass();
                            vel.angular += rb.inv_world_inertia_tensor(chassis_rot) * r_contact.cross(drive_impulse);
                        }

                        // Brake Force
                        if current_brake > 0.0 {
                            let point_vel = vel.linear + vel.angular.cross(r_contact);
                            let long_vel = point_vel.dot(forward);
                            let max_brake_impulse = current_brake * brake_force * dt;
                            let desired_brake_impulse = -long_vel * per_wheel_mass;

                            let actual_brake_impulse_mag = if desired_brake_impulse > 0.0 {
                                desired_brake_impulse.clamp(0.0, max_brake_impulse)
                            } else {
                                desired_brake_impulse.clamp(-max_brake_impulse, 0.0)
                            };

                            let brake_impulse = forward * actual_brake_impulse_mag;
                            vel.linear += brake_impulse * rb.inv_mass();
                            vel.angular += rb.inv_world_inertia_tensor(chassis_rot) * r_contact.cross(brake_impulse);
                        }
                    }
                } else {
                    wheel.suspension_compression = 0.0;
                }
            }
        }
    }
}
