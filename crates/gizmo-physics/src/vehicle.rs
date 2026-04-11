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

    pub is_drive_wheel: bool, // Bu tekerlek motor gücü alıyor mu (FWD/RWD/4WD)
    pub is_grounded: bool,
    pub compression: f32,    // Yerdeyse yayın ne kadar sıkıştığı
    pub contact_point: Vec3, // Çarpışma noktası
    pub rotation_angle: f32, // Animasyon için görsel dönüş açısı
}

impl Wheel {
    pub fn new(
        connection_point: Vec3,
        rest_length: f32,
        stiffness: f32,
        damping: f32,
        radius: f32,
    ) -> Self {
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
            rotation_angle: 0.0,
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
    pub engine_force: f32,   // Motor gücü (Newton). Pozitif = ileri, Negatif = geri
    pub steering_angle: f32, // Direksiyon açısı (Radyan). Pozitif = sola, Negatif = sağa
    pub brake_force: f32,    // Fren kuvveti (Newton)
    // Konfigüre edilebilir fizik sabitleri
    pub steering_force_mult: f32, // Direksiyon kuvvet çarpanı
    pub lateral_grip: f32,        // Yanal tutuş kuvveti
    pub anti_slide_force: f32,    // Kayma önleme kuvveti
    /// Aerodinamik sürükleme katsayısı — F = ½ · Cd · ρA · v²
    /// Sadece Cd·ρA çarpımı saklanır (ρ=1.225 kg/m³ hava yoğunluğu dahil).
    /// Tipik değerler: spor araba ~0.15, SUV/kamyon ~0.40.
    pub drag_coefficient: f32,
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
            // 0.3 ≈ kötü aerodinamiği olan bir sedan/SUV için gerçekçi Cd·ρA
            drag_coefficient: 0.3,
        }
    }

    pub fn add_wheel(&mut self, wheel: Wheel) {
        self.wheels.push(wheel);
    }
}

use crate::components::{RigidBody, Transform, Velocity};
use crate::integration::apply_inv_inertia;
use crate::shape::{Collider, ColliderShape};
use gizmo_core::World;
pub fn physics_vehicle_system(world: &World, dt: f32) {
    // Statik objeleri topla (Raycast testleri için)
    enum StaticCol {
        Aabb {
            position: Vec3,
            half_extents: Vec3,
        },
        HeightField {
            position: Vec3,
            heights: std::sync::Arc<Vec<f32>>,
            segments_x: u32,
            segments_z: u32,
            width: f32,
            depth: f32,
            max_height: f32,
        },
    }

    let colliders_storage = world.borrow::<Collider>();
    let static_cols: Vec<StaticCol> = {
        if let (Some(rbs), Some(ref cols), Some(ts)) = (
            world.borrow::<RigidBody>(),
            &colliders_storage,
            world.borrow::<Transform>(),
        ) {
            cols.dense
                .iter()
                .map(|e| &e.entity)
                .filter_map(|&e| {
                    if rbs.get(e).is_some_and(|rb| rb.mass == 0.0) {
                        let t = ts.get(e)?;
                        let col = cols.get(e)?;
                        match &col.shape {
                            ColliderShape::Aabb(aabb) => {
                                return Some(StaticCol::Aabb {
                                    position: t.position,
                                    half_extents: Vec3::new(
                                        aabb.half_extents.x * t.scale.x,
                                        aabb.half_extents.y * t.scale.y,
                                        aabb.half_extents.z * t.scale.z,
                                    ),
                                });
                            }
                            ColliderShape::HeightField {
                                heights,
                                segments_x,
                                segments_z,
                                width,
                                depth,
                                max_height,
                            } => {
                                return Some(StaticCol::HeightField {
                                    position: t.position,
                                    heights: std::sync::Arc::new(heights.clone()),
                                    segments_x: *segments_x,
                                    segments_z: *segments_z,
                                    width: *width * t.scale.x,
                                    depth: *depth * t.scale.z,
                                    max_height: *max_height * t.scale.y,
                                });
                            }
                            _ => return None,
                        }
                    }
                    None
                })
                .collect()
        } else {
            Vec::new()
        }
    };

    if let (Some(trans_storage), Some(mut vel_storage), Some(mut rbs), Some(mut vehicles)) = (
        world.borrow_mut::<Transform>(),
        world.borrow_mut::<Velocity>(),
        world.borrow_mut::<RigidBody>(),
        world.borrow_mut::<VehicleController>(),
    ) {
        let entities: Vec<u32> = vehicles.dense.iter().map(|e| e.entity).collect();
        for entity in entities {
            let t = match trans_storage.get(entity) {
                Some(t) => *t,
                None => continue,
            };
            let v = match vel_storage.get_mut(entity) {
                Some(v) => v,
                None => continue,
            };
            let rb = match rbs.get_mut(entity) {
                Some(r) => r,
                None => continue,
            };
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
            let drive_wheel_count = vehicle
                .wheels
                .iter()
                .filter(|w| w.is_drive_wheel)
                .count()
                .max(1) as f32;

            for (_i, wheel) in vehicle.wheels.iter_mut().enumerate() {
                // Lokal bağlantı noktasını dünya haritasına çevir
                let r_ws = t.rotation.mul_vec3(wheel.connection_point);
                let origin = t.position + r_ws;
                let dir = t.rotation.mul_vec3(wheel.direction).normalize();

                // === RAYCASTING — hit_t (mesafe) + contact_normal (yüzey normali) ===
                //
                // contact_normal; süspansiyon kuvvetinin yönünü belirler.
                // HeightField için bilinear türev ile hesaplanır — düz global-Y değil.
                // Bu, eğimli zemin üzerinde titreşimi ve yan kaymayı önler.
                let mut hit_t = f32::MAX;
                let mut contact_normal = Vec3::new(0.0, 1.0, 0.0); // fallback: düz zemin

                // 1. Zemin Yüzeyi fallback (PhysicsConfig'den oku)
                let ground_y = world
                    .get_resource::<crate::components::PhysicsConfig>()
                    .map(|c| c.ground_y)
                    .unwrap_or(-1.0);
                if dir.y < -0.001 && origin.y > ground_y {
                    let t_y = (ground_y - origin.y) / dir.y;
                    if t_y > 0.0 && t_y < hit_t {
                        hit_t = t_y;
                        contact_normal = Vec3::new(0.0, 1.0, 0.0);
                    }
                }

                // 2. Statik Objelerle Raycast Test (AABB & HeightField)
                for static_col in &static_cols {
                    match static_col {
                        StaticCol::Aabb {
                            position,
                            half_extents,
                        } => {
                            let min_b = *position - *half_extents;
                            let max_b = *position + *half_extents;

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
                            let t_far  = t1x.max(t2x).min(t1y.max(t2y)).min(t1z.max(t2z));

                            if t_near <= t_far && t_far > 0.0 && t_near < hit_t {
                                let ht = if t_near > 0.0 { t_near } else { 0.0 };
                                hit_t = ht;

                                // Hangi slab'dan girildi? — o slab'ın normali = contact_normal
                                // t_near'ın katkısını sağlayan eksen en büyük t_near'ı veriyor.
                                let tx_near = t1x.min(t2x);
                                let ty_near = t1y.min(t2y);
                                let tz_near = t1z.min(t2z);
                                contact_normal = if tx_near >= ty_near && tx_near >= tz_near {
                                    Vec3::new(if dir.x > 0.0 { -1.0 } else { 1.0 }, 0.0, 0.0)
                                } else if ty_near >= tz_near {
                                    Vec3::new(0.0, if dir.y > 0.0 { -1.0 } else { 1.0 }, 0.0)
                                } else {
                                    Vec3::new(0.0, 0.0, if dir.z > 0.0 { -1.0 } else { 1.0 })
                                };
                            }
                        }
                        StaticCol::HeightField {
                            position,
                            heights,
                            segments_x,
                            segments_z,
                            width,
                            depth,
                            max_height,
                        } => {
                            if dir.y > -0.001 {
                                continue;
                            }

                            let local_x = origin.x - position.x;
                            let local_z = origin.z - position.z;
                            let half_w = *width * 0.5;
                            let half_d = *depth * 0.5;

                            if local_x >= -half_w
                                && local_x <= half_w
                                && local_z >= -half_d
                                && local_z <= half_d
                            {
                                // Normalize [0, 1] aralığına taşı
                                let nx = (local_x + half_w) / *width;
                                let nz = (local_z + half_d) / *depth;

                                // Grid hücresi sol-alt köşesi (floor)
                                let sx = *segments_x as f32 - 1.0;
                                let sz_ = *segments_z as f32 - 1.0;
                                let fx = (nx * sx).max(0.0);
                                let fz = (nz * sz_).max(0.0);
                                let gx0 = (fx as u32).clamp(0, *segments_x - 2);
                                let gz0 = (fz as u32).clamp(0, *segments_z - 2);
                                let gx1 = gx0 + 1;
                                let gz1 = gz0 + 1;

                                // Dört köşe yükseklikleri
                                let h = |gx: u32, gz: u32| -> f32 {
                                    let idx = (gz * *segments_x + gx) as usize;
                                    if idx < heights.len() { heights[idx] * *max_height } else { 0.0 }
                                };
                                let h00 = h(gx0, gz0);
                                let h10 = h(gx1, gz0);
                                let h01 = h(gx0, gz1);
                                let h11 = h(gx1, gz1);

                                // Hücre içi interpolasyon ağırlıkları
                                let tx = fx - gx0 as f32;
                                let tz = fz - gz0 as f32;

                                // Bilinear yükseklik interpolasyonu — snap değil
                                let terrain_height = h00 * (1.0 - tx) * (1.0 - tz)
                                    + h10 * tx * (1.0 - tz)
                                    + h01 * (1.0 - tx) * tz
                                    + h11 * tx * tz;

                                let terrain_y = position.y + terrain_height;
                                if origin.y > terrain_y {
                                    let t_y = (terrain_y - origin.y) / dir.y;
                                    if t_y > 0.0 && t_y < hit_t {
                                        hit_t = t_y;

                                        // Bilinear türev ile terrein normali
                                        // ∂h/∂x ≈ bilinear türev x yönünde
                                        // ∂h/∂z ≈ bilinear türev z yönünde
                                        let cell_w = *width / sx;
                                        let cell_d = *depth / sz_;
                                        let dh_dx = ((h10 - h00) * (1.0 - tz)
                                            + (h11 - h01) * tz)
                                            / cell_w;
                                        let dh_dz = ((h01 - h00) * (1.0 - tx)
                                            + (h11 - h10) * tx)
                                            / cell_d;
                                        // Normal = (-∂h/∂x, 1, -∂h/∂z).normalize()
                                        contact_normal = Vec3::new(-dh_dx, 1.0, -dh_dz)
                                            .normalize();
                                    }
                                }
                            }
                        }
                    }
                }

                // === SÜSPANSİYON YAYLANMASI (Hooke Yasası + Damper) ===
                if hit_t <= wheel.suspension_rest_length + wheel.wheel_radius {
                    wheel.is_grounded = true;
                    // X = Dinlenme uzunluğu (rest) eksi, ulaşılan ray uzaklığı.
                    // Tekerlek lastiği de pay içerdiği için çıkarılır.
                    let tire_margin = wheel.wheel_radius;
                    // Tam Hooke Sıkıştırması:
                    wheel.compression = (wheel.suspension_rest_length + tire_margin) - hit_t;

                    if wheel.compression > 0.0 {
                        let spring_force = wheel.suspension_stiffness * wheel.compression;

                        let wheel_vel = v.linear + v.angular.cross(r_ws);
                        // Sönümleme: hız bileşenini gerçek terrein normaline göre ölç.
                        // Düz zeminde contact_normal ≈ Vec3::Y olduğundan davranış değişmez.
                        let vel_along_normal = wheel_vel.dot(contact_normal);

                        let damping_force = wheel.suspension_damping * vel_along_normal;
                        let total_suspension_force = (spring_force + damping_force).max(0.0);

                        // Kuvveti terrein normaline göre uygula — eğimde titreşim önlenir.
                        let suspension_impulse = contact_normal * total_suspension_force * dt;
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
                        total_angular_impulse +=
                            apply_inv_inertia(drive_torque, inv_inertia, t.rotation);
                    }

                    // === FREN ===
                    if brake_force > 0.01 {
                        let forward_speed = v.linear.dot(forward);
                        let brake_impulse =
                            forward * (-forward_speed.signum() * brake_force / num_wheels) * dt;
                        total_linear_impulse += brake_impulse;
                    }

                    // === DİREKSİYON (Ön tekerlekler — drive olmayan) ===
                    if !wheel.is_drive_wheel {
                        let wheel_vel = v.linear + v.angular.cross(r_ws);
                        let lateral_vel = wheel_vel.dot(right);
                        let steer_force = steering_angle * steer_mult;
                        let grip_force = -lateral_vel * lat_grip;
                        let lateral_impulse = right * (steer_force + grip_force) * dt / num_wheels;
                        total_linear_impulse += lateral_impulse;

                        let steer_torque = r_ws.cross(lateral_impulse);
                        total_angular_impulse +=
                            apply_inv_inertia(steer_torque, inv_inertia, t.rotation);
                    }

                    // Yanal Sürtünme / Tutuş (Arka tekerlek drift algısı)
                    if wheel.is_drive_wheel {
                        let wheel_vel = v.linear + v.angular.cross(r_ws);
                        let lateral_vel = wheel_vel.dot(right);
                        let anti_slide = right * (-lateral_vel * anti_slide_k / num_wheels) * dt;
                        total_linear_impulse += anti_slide;

                        let torque = r_ws.cross(anti_slide);
                        total_angular_impulse += apply_inv_inertia(torque, inv_inertia, t.rotation);
                    }

                    // Görsel tekerlek dönmesi (hıza göre tekerlek çevresini hesaplayarak döndür)
                    let speed = v.linear.dot(forward);
                    wheel.rotation_angle += (speed / wheel.wheel_radius) * dt;
                } else {
                    wheel.is_grounded = false;
                    wheel.compression = 0.0;
                }
            }

            // === HAVA DİRENCI (Kuadratik Sürükleme) ===
            // Gerçek aerodinamik formül: F_drag = -½ · Cd·ρA · |v|² · v̂
            // Impulse = F_drag · dt,  Δv = impulse · (1/m)
            // Önceki hatalar:
            //   1) speed_sq.sqrt() * v.linear → net v² etkisi tesadüfen doğruydu, ama okunaksız.
            //   2) inv_mass ile **çarpılmıyordu** → kütle bağımsız direnç (50 kg = 5000 kg aynı etki).
            //   3) Katsayı hardcoded magic number'dı; artık VehicleController::drag_coefficient'ten okunuyor.
            let speed_sq = v.linear.length_squared();
            if speed_sq > 0.01 {
                let cd = vehicle.drag_coefficient;
                // F_drag yönü: hıza zıt → v̂ = v / |v|, |v|² · v̂ = |v| · v
                // impulse = -½·Cd·|v|²·v̂ · dt = -½·Cd·|v|·v · dt
                let drag_impulse = v.linear * (-0.5 * cd * speed_sq.sqrt() * dt);
                // Δv = impulse / m — kütle-bağımlı hava direnci
                total_linear_impulse += drag_impulse * inv_mass;
            }

            v.linear += total_linear_impulse * inv_mass;
            v.angular += total_angular_impulse;
        }
    }
}

// Varlıkların fiziksel hareketlerini, yerçekimi ve sürtünme etkileriyle uygulayan sistem
