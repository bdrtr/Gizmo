use gizmo_math::Vec3;

#[derive(Debug, Clone)]
pub struct Wheel {
    pub connection_point: Vec3, // Gövde merkezine (Center of Mass) göre lokal pozisyonu
    pub direction: Vec3,        // Süspansiyonun yere uzanma yönü (genelde 0, -1, 0)
    pub axle: Vec3,             // Tekerleğin dönme ekseni (genelde -1, 0, 0 veya 1, 0, 0)
    
    pub suspension_rest_length: f32, // Normal şartlardaki boşluk mesafesi
    pub suspension_stiffness: f32,   // Yay sertliği
    pub suspension_damping: f32,     // Sönümleme (Geri fırlamayı önler)
    pub wheel_radius: f32,           // Tekerleğin yarıçapı
    
    pub is_drive_wheel: bool,        // Bu tekerlek motor gücü alıyor mu (FWD/RWD/4WD)
    // Geçici durumsal veriler (Sistem tarafından her frame güncellenir)
    pub is_grounded: bool,
    pub compression: f32,            // Yerdeyse yayın ne kadar sıkıştığı
    pub contact_point: Vec3,         // Çarpışma noktası
}

impl Wheel {
    pub fn new(connection_point: Vec3, rest_length: f32, stiffness: f32, damping: f32, radius: f32) -> Self {
        Self {
            connection_point,
            direction: Vec3::new(0.0, -1.0, 0.0),
            axle: Vec3::new(-1.0, 0.0, 0.0),
            suspension_rest_length: rest_length,
            suspension_stiffness: stiffness,
            suspension_damping: damping,
            wheel_radius: radius,
            is_drive_wheel: false,
            is_grounded: false,
            compression: 0.0,
            contact_point: Vec3::ZERO,
        }
    }

    /// Motorlu tekerlek olarak ayarla
    pub fn with_drive(mut self) -> Self {
        self.is_drive_wheel = true;
        self
    }
}

/// Raycast Vehicle Controller. Araç gövdesine (Chassis) RigidBody ile birlikte eklenmelidir.
#[derive(Debug, Clone)]
pub struct VehicleController {
    pub wheels: Vec<Wheel>,
    pub engine_force: f32,    // Motor gücü (Newton). Pozitif = ileri, Negatif = geri
    pub steering_angle: f32,  // Direksiyon açısı (Radyan). Pozitif = sola, Negatif = sağa
    pub brake_force: f32,     // Fren kuvveti (Newton)
    // Konfigüre edilebilir fizik sabitleri (artık hardcoded değil)
    pub steering_force_mult: f32,  // Direksiyon kuvvet çarpanı (eski: 8000.0)
    pub lateral_grip: f32,         // Yanal tutuş kuvveti (eski: 5000.0)
    pub anti_slide_force: f32,     // Kayma önleme kuvveti (eski: 3000.0)
}

impl Default for VehicleController {
    fn default() -> Self {
        Self::new()
    }
}

impl VehicleController {
    pub fn new() -> Self {
        Self {
            wheels: Vec::new(),
            engine_force: 0.0,
            steering_angle: 0.0,
            brake_force: 0.0,
            steering_force_mult: 8000.0,
            lateral_grip: 5000.0,
            anti_slide_force: 3000.0,
        }
    }

    pub fn add_wheel(&mut self, wheel: Wheel) {
        self.wheels.push(wheel);
    }
}

use gizmo_core::World;
use crate::components::{Transform, Velocity, RigidBody};
use crate::shape::{Collider, ColliderShape};
use crate::integration::apply_inv_inertia;
pub fn physics_vehicle_system(world: &World, dt: f32) {
    // Statik objeleri topla (Raycast testleri için)
    struct StaticAabb {
        position: Vec3,
        half_extents: Vec3,
    }
    let colliders_storage = world.borrow::<Collider>();
    let static_aabbs: Vec<StaticAabb> = {
        if let (Some(rbs), Some(ref cols), Some(ts)) = (world.borrow::<RigidBody>(), &colliders_storage, world.borrow::<Transform>()) {
            cols.entity_dense.iter()
                .filter_map(|&e| {
                    if rbs.get(e).is_some_and(|rb| rb.mass == 0.0) {
                        let t = ts.get(e)?;
                        let col = cols.get(e)?;
                        if let ColliderShape::Aabb(aabb) = &col.shape {
                            return Some(StaticAabb {
                                position: t.position,
                                half_extents: Vec3::new(
                                    aabb.half_extents.x * t.scale.x,
                                    aabb.half_extents.y * t.scale.y,
                                    aabb.half_extents.z * t.scale.z,
                                ),
                            });
                        }
                    }
                    None
                })
                .collect()
        } else {
            Vec::new()
        }
    };

    if let (Some(trans_storage), Some(mut vel_storage), Some(mut rbs), Some(mut vehicles)) = 
        (world.borrow_mut::<Transform>(), world.borrow_mut::<Velocity>(), world.borrow_mut::<RigidBody>(), world.borrow_mut::<VehicleController>()) 
    {
        let entities = vehicles.entity_dense.clone();
        for entity in entities {
            let t = match trans_storage.get(entity) { Some(t) => *t, None => continue };
            let v = match vel_storage.get_mut(entity) { Some(v) => v, None => continue };
            let rb = match rbs.get_mut(entity) { Some(r) => r, None => continue };
            let vehicle = vehicles.get_mut(entity).unwrap();

            rb.wake_up();

            let inv_mass = if rb.mass > 0.0 { 1.0 / rb.mass } else { 0.0 };
            let inv_inertia = rb.inverse_inertia;

            let mut total_linear_impulse = Vec3::ZERO;
            let mut total_angular_impulse = Vec3::ZERO;
            
            let forward = t.rotation.mul_vec3(Vec3::new(0.0, 0.0, 1.0)).normalize();
            let right = t.rotation.mul_vec3(Vec3::new(1.0, 0.0, 0.0)).normalize();
            
            let num_wheels = vehicle.wheels.len() as f32;
            let engine_force = vehicle.engine_force;
            let steering_angle = vehicle.steering_angle;
            let brake_force = vehicle.brake_force;
            let steer_mult = vehicle.steering_force_mult;
            let lat_grip = vehicle.lateral_grip;
            let anti_slide_k = vehicle.anti_slide_force;
            let drive_wheel_count = vehicle.wheels.iter().filter(|w| w.is_drive_wheel).count().max(1) as f32;

            for (_i, wheel) in vehicle.wheels.iter_mut().enumerate() {
                // Lokal bağlantı noktasını dünya haritasına çevir
                let r_ws = t.rotation.mul_vec3(wheel.connection_point);
                let origin = t.position + r_ws;
                let dir = t.rotation.mul_vec3(wheel.direction).normalize();

                // === RAYCASTING (AABB & Ground Plane) ===
                let mut hit_t = f32::MAX;
                
                // 1. Zemin Yüzeyi fallback olarak (PhysicsConfig'den oku)
                let ground_y = world.get_resource::<crate::components::PhysicsConfig>()
                    .map(|c| c.ground_y)
                    .unwrap_or(-1.0);
                if dir.y < -0.001 && origin.y > ground_y {
                    let t_y = (ground_y - origin.y) / dir.y;
                    if t_y > 0.0 && t_y < hit_t { hit_t = t_y; }
                }

                // 2. Statik AABB'lere Raycast Testi
                for static_col in &static_aabbs {
                    let min_b = static_col.position - static_col.half_extents;
                    let max_b = static_col.position + static_col.half_extents;
                    
                    let inv_dir = Vec3::new(
                        if dir.x.abs() > 1e-8 { 1.0 / dir.x } else { f32::MAX },
                        if dir.y.abs() > 1e-8 { 1.0 / dir.y } else { f32::MAX },
                        if dir.z.abs() > 1e-8 { 1.0 / dir.z } else { f32::MAX },
                    );
                    
                    let t1x = (min_b.x - origin.x) * inv_dir.x;
                    let t2x = (max_b.x - origin.x) * inv_dir.x;
                    let t1y = (min_b.y - origin.y) * inv_dir.y;
                    let t2y = (max_b.y - origin.y) * inv_dir.y;
                    let t1z = (min_b.z - origin.z) * inv_dir.z;
                    let t2z = (max_b.z - origin.z) * inv_dir.z;
                    
                    let t_near = t1x.min(t2x).max(t1y.min(t2y)).max(t1z.min(t2z));
                    let t_far = t1x.max(t2x).min(t1y.max(t2y)).min(t1z.max(t2z));
                    
                    if t_near <= t_far && t_far > 0.0 && t_near < hit_t {
                        hit_t = if t_near > 0.0 { t_near } else { 0.0 };
                    }
                }
                
                // === SÜSPANSİYON YAYLANMASI (Hooke Yasası + Damper) ===
                if hit_t <= wheel.suspension_rest_length {
                    wheel.is_grounded = true;
                    // X = Dinlenme uzunluğu (rest) eksi, ulaşılan ray uzaklığı. 
                    // Tekerlek lastiği de pay içerdiği için çıkarılır.
                    let tire_margin = wheel.wheel_radius;
                    // Tam Hooke Sıkıştırması: 
                    wheel.compression = (wheel.suspension_rest_length + tire_margin) - hit_t;
                    
                    if wheel.compression > 0.0 {
                        let spring_force = wheel.suspension_stiffness * wheel.compression;
                        
                        let wheel_vel = v.linear + v.angular.cross(r_ws);
                        let vel_along_dir = wheel_vel.dot(dir);
                        
                        let damping_force = wheel.suspension_damping * vel_along_dir;
                        let total_suspension_force = (spring_force + damping_force).max(0.0);
                        
                        let suspension_impulse = dir * -total_suspension_force * dt;
                        total_linear_impulse += suspension_impulse;
                        
                        // Direk Tork oluştur (Merkezi olmayan kuvvet)
                        let torque = r_ws.cross(suspension_impulse);
                        total_angular_impulse += apply_inv_inertia(torque, inv_inertia, t.rotation);
                    }
                    
                    // === MOTOR GÜCÜ (is_drive_wheel ile belirlenen tekerlekler) ===
                    if wheel.is_drive_wheel && engine_force.abs() > 0.01 {
                        let drive_impulse = forward * (engine_force / drive_wheel_count) * dt;
                        total_linear_impulse += drive_impulse;
                        let drive_torque = r_ws.cross(drive_impulse);
                        total_angular_impulse += apply_inv_inertia(drive_torque, inv_inertia, t.rotation);
                    }
                    
                    // === FREN ===
                    if brake_force > 0.01 {
                        let forward_speed = v.linear.dot(forward);
                        let brake_impulse = forward * (-forward_speed.signum() * brake_force / num_wheels) * dt;
                        total_linear_impulse += brake_impulse;
                    }
                    
                    // === DİREKSİYON (Ön tekerlekler — drive olmayan) ===
                    if !wheel.is_drive_wheel && steering_angle.abs() > 0.001 {
                        let wheel_vel = v.linear + v.angular.cross(r_ws);
                        let lateral_vel = wheel_vel.dot(right);
                        let steer_force = steering_angle * steer_mult;
                        let grip_force = -lateral_vel * lat_grip;
                        let lateral_impulse = right * (steer_force + grip_force) * dt / num_wheels;
                        total_linear_impulse += lateral_impulse;
                        
                        let steer_torque = r_ws.cross(lateral_impulse);
                        total_angular_impulse += apply_inv_inertia(steer_torque, inv_inertia, t.rotation);
                    }
                    
                    // Yanal Sürtünme / Tutuş (Araca viraj dönerken drift engelleme)
                    let wheel_vel = v.linear + v.angular.cross(r_ws);
                    let lateral_vel = wheel_vel.dot(right);
                    let anti_slide = right * (-lateral_vel * anti_slide_k / num_wheels) * dt;
                    total_linear_impulse += anti_slide;
                    let slide_torque = r_ws.cross(anti_slide);
                    total_angular_impulse += apply_inv_inertia(slide_torque, inv_inertia, t.rotation);
                    
                } else {
                    wheel.is_grounded = false;
                    wheel.compression = 0.0;
                }
            }
            
            // Hava Direnci
            let speed_sq = v.linear.length_squared();
            if speed_sq > 0.1 {
                let drag = v.linear * (-0.5 * speed_sq.sqrt() * 0.3 * dt);
                total_linear_impulse += drag;
            }

            v.linear += total_linear_impulse * inv_mass;
            v.angular += total_angular_impulse;
        }
    }
}

// Varlıkların fiziksel hareketlerini, yerçekimi ve sürtünme etkileriyle uygulayan sistem
