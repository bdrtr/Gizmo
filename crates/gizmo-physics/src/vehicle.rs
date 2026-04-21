use gizmo_math::Vec3;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WheelComponent {
    pub direction: Vec3,        // Süspansiyonun yere uzanma yönü (genelde 0, -1, 0)
    pub axle: Vec3,             // Tekerleğin dönme ekseni (genelde -1, 0, 0 veya 1, 0, 0)

    pub suspension_rest_length: f32, // Normal şartlardaki boşluk mesafesi
    pub suspension_stiffness: f32,   // Yay sertliği
    pub suspension_damping: f32,     // Sönümleme (Geri fırlamayı önler)
    pub wheel_radius: f32,           // Tekerleğin yarıçapı

    pub is_drive_wheel: bool, // Bu tekerlek motor gücü alıyor mu (FWD/RWD/4WD)
    #[serde(default)]
    pub is_steering_wheel: bool, // Bu tekerleğe direksiyon uygulanıyor mu?

    #[serde(skip)] pub is_grounded: bool,
    #[serde(skip)] pub compression: f32,    // Yerdeyse yayın ne kadar sıkıştığı
    #[serde(skip)] pub contact_point: Vec3, // Çarpışma noktası
    #[serde(skip)] pub rotation_angle: f32, // Animasyon için görsel dönüş açısı
}

impl Default for WheelComponent {
    fn default() -> Self {
        Self {
            direction: Vec3::new(0.0, -1.0, 0.0),
            axle: Vec3::new(-1.0, 0.0, 0.0),
            suspension_rest_length: 1.0,
            suspension_stiffness: 20000.0,
            suspension_damping: 2000.0,
            wheel_radius: 0.5,
            is_drive_wheel: false,
            is_steering_wheel: false,
            is_grounded: false,
            compression: 0.0,
            contact_point: Vec3::ZERO,
            rotation_angle: 0.0,
        }
    }
}

impl WheelComponent {
    pub fn new(
        rest_length: f32,
        stiffness: f32,
        damping: f32,
        radius: f32,
    ) -> Self {
        Self {
            suspension_rest_length: rest_length,
            suspension_stiffness: stiffness,
            suspension_damping: damping,
            wheel_radius: radius,
            ..Default::default()
        }
    }

    /// Motorlu tekerlek olarak ayarla
    pub fn with_drive(mut self) -> Self {
        self.is_drive_wheel = true;
        self
    }

    /// Direksiyon tekerleği olarak ayarla
    pub fn with_steering(mut self) -> Self {
        self.is_steering_wheel = true;
        self
    }
}

/// Raycast Vehicle Controller. Araç gövdesine (Chassis) RigidBody ile birlikte eklenmelidir.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VehicleController {
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
}

use crate::components::{RigidBody, Transform, Velocity};
use crate::integration::apply_inv_inertia;
use crate::shape::{Collider, ColliderShape};
use gizmo_core::World;
pub fn physics_vehicle_system(world: &World, dt: f32) {
    // Statik objeleri topla (Raycast testleri için)
    enum StaticCol<'a> {
        Aabb {
            position: Vec3,
            half_extents: Vec3,
        },
        HeightField {
            position: Vec3,
            heights: &'a [f32],
            segments_x: u32,
            segments_z: u32,
            width: f32,
            depth: f32,
            max_height: f32,
        },
    }

    let colliders_storage = world.borrow::<Collider>();
    let rbs = world.borrow::<RigidBody>();
    let ts = world.borrow::<Transform>();

    let static_cols: Vec<StaticCol> = colliders_storage.entities()
        .filter_map(|e| {
            if rbs.get(e).is_some_and(|rb| rb.mass < 1e-6) {
                let t = ts.get(e)?;
                let col = colliders_storage.get(e)?;
                match &col.shape {
                    ColliderShape::Aabb(aabb) => {
                        Some(StaticCol::Aabb {
                            position: t.position,
                            half_extents: Vec3::new(
                                aabb.half_extents.x * t.scale.x,
                                aabb.half_extents.y * t.scale.y,
                                aabb.half_extents.z * t.scale.z,
                            ),
                        })
                    }
                    ColliderShape::HeightField {
                        heights,
                        segments_x,
                        segments_z,
                        width,
                        depth,
                        max_height,
                    } => {
                        Some(StaticCol::HeightField {
                            position: t.position,
                            heights: heights.as_slice(),
                            segments_x: *segments_x,
                            segments_z: *segments_z,
                            width: *width * t.scale.x,
                            depth: *depth * t.scale.z,
                            max_height: *max_height * t.scale.y,
                        })
                    }
                    _ => None,
                }
            } else {
                None
            }
        })
        .collect();

    let mut trans_storage = world.borrow_mut::<Transform>();
    let mut vel_storage = world.borrow_mut::<Velocity>();
    let mut rb_storage = world.borrow_mut::<RigidBody>();
    let vehicles = world.borrow::<VehicleController>();
    let children_storage = world.borrow::<gizmo_core::component::Children>();
    let mut wheel_storage = world.borrow_mut::<WheelComponent>();

    let entities: Vec<u32> = vehicles.entities().collect();
    for entity in entities {
            let t = match trans_storage.get(entity) {
                Some(t) => *t,
                None => continue,
            };
            let v = match vel_storage.get_mut(entity) {
                Some(v) => v,
                None => continue,
            };
            let rb = match rb_storage.get_mut(entity) {
                Some(r) => r,
                None => continue,
            };
            let vehicle = vehicles.get(entity).unwrap();

            rb.wake_up();

            let inv_mass = if rb.mass > 0.0 { 1.0 / rb.mass } else { 0.0 };
            let inv_inertia = rb.inverse_inertia_local;

            let mut total_linear_impulse = Vec3::ZERO;
            let mut total_angular_impulse = Vec3::ZERO;

            let forward = t.rotation.mul_vec3(Vec3::new(0.0, 0.0, 1.0)).normalize();
            let right = t.rotation.mul_vec3(Vec3::new(1.0, 0.0, 0.0)).normalize();

            let mut wheel_entities = Vec::new();
            if let Some(children) = children_storage.get(entity) {
                for &child_id in &children.0 {
                    if wheel_storage.contains(child_id) {
                        wheel_entities.push(child_id);
                    }
                }
            }

            let num_wheels = wheel_entities.len() as f32;
            if num_wheels < 1.0 { continue; }

            let engine_force = vehicle.engine_force;
            let steering_angle = vehicle.steering_angle;
            let brake_force = vehicle.brake_force;
            let steer_mult = vehicle.steering_force_mult;
            let lat_grip = vehicle.lateral_grip;
            let anti_slide_k = vehicle.anti_slide_force;
            
            let mut drive_wheel_count: f32 = 0.0;
            for &c in &wheel_entities {
                if let Some(w) = wheel_storage.get(c) {
                    if w.is_drive_wheel {
                        drive_wheel_count += 1.0;
                    }
                }
            }
            let drive_wheel_count = drive_wheel_count.max(1.0);

            for &wheel_entity in &wheel_entities {
                let wt = match trans_storage.get_mut(wheel_entity) {
                    Some(wt) => wt, // get mutable reference once
                    None => continue,
                };
                let wheel = wheel_storage.get_mut(wheel_entity).unwrap();
                
                let wheel_mat = wt.global_matrix;
                let origin = Vec3::new(wheel_mat.w_axis.x, wheel_mat.w_axis.y, wheel_mat.w_axis.z);
                
                let r_ws = origin - t.position;
                let dir = t.rotation.mul_vec3(wheel.direction).normalize();

                // Süspansiyon uzunluklarını vehicle scale ile çarp (child'ın da scale'i olabilir ama parent'ını kullanalım)
                let scaled_rest_length = wheel.suspension_rest_length * t.scale.y;
                let scaled_radius = wheel.wheel_radius * t.scale.y;

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

                            if t_near > 0.0 && t_near <= t_far && t_far > 0.0 && t_near < hit_t {
                                hit_t = t_near;

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
                            let half_w = width * 0.5;
                            let half_d = depth * 0.5;

                            if local_x >= -half_w
                                && local_x <= half_w
                                && local_z >= -half_d
                                && local_z <= half_d
                            {
                                // Normalize [0, 1] aralığına taşı
                                let nx = (local_x + half_w) / width;
                                let nz = (local_z + half_d) / depth;

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
                                    if idx < heights.len() { heights[idx] * max_height } else { 0.0 }
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
                                        let cell_w = width / sx;
                                        let cell_d = depth / sz_;
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
                if hit_t <= scaled_rest_length + scaled_radius {
                    wheel.is_grounded = true;
                    // X = Dinlenme uzunluğu (rest) eksi, ulaşılan ray uzaklığı.
                    // Tekerlek lastiği de pay içerdiği için çıkarılır.
                    let tire_margin = scaled_radius;
                    // Tam Hooke Sıkıştırması:
                    wheel.compression = (scaled_rest_length + tire_margin) - hit_t;

                    if wheel.compression > 0.0 {
                        let spring_force = wheel.suspension_stiffness * wheel.compression;

                        let wheel_vel = v.linear + v.angular.cross(r_ws);
                        // Sönümleme: hız bileşenini gerçek terrein normaline göre ölç.
                        // Damping hıza zıt yönde etki etmelidir! eksi işareti hayati önem taşır.
                        let vel_along_normal = wheel_vel.dot(contact_normal);
                        let damping_force = -wheel.suspension_damping * vel_along_normal;
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
                        if forward_speed.abs() > 0.01 {
                            let brake_impulse =
                                forward * (-forward_speed.signum() * brake_force / num_wheels) * dt;
                            total_linear_impulse += brake_impulse;
                        }
                    }

                    // === DİREKSİYON (is_steering_wheel) ===
                    if wheel.is_steering_wheel {
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

                    // Yanal Sürtünme / Tutuş (Arka tekerlek drift algısı, direksiyon OLMAYANLAR)
                    if !wheel.is_steering_wheel {
                        let wheel_vel = v.linear + v.angular.cross(r_ws);
                        let lateral_vel = wheel_vel.dot(right);
                        let anti_slide = right * (-lateral_vel * anti_slide_k / num_wheels) * dt;
                        total_linear_impulse += anti_slide;

                        let torque = r_ws.cross(anti_slide);
                        total_angular_impulse += apply_inv_inertia(torque, inv_inertia, t.rotation);
                    }

                    // Görsel tekerlek dönmesi (hıza göre tekerlek çevresini hesaplayarak döndür)
                    let speed = v.linear.dot(forward);
                    wheel.rotation_angle += (speed / scaled_radius) * dt;

                    let base_rot = gizmo_math::Quat::from_axis_angle(wheel.axle, wheel.rotation_angle);
                    let steer_rot = if wheel.is_steering_wheel {
                        gizmo_math::Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), steering_angle)
                    } else {
                        gizmo_math::Quat::IDENTITY
                    };
                    wt.rotation = steer_rot * base_rot;
                    wt.update_local_matrix();
                } else {
                    wheel.is_grounded = false;
                    wheel.compression = 0.0;
                }
            }

            v.linear += total_linear_impulse * inv_mass;
            v.angular += total_angular_impulse;

            // === HAVA DİRENCI (Kuadratik Sürükleme) ===
            // Geleneksel simülasyonlarda F_drag = -½ · Cd·ρA · |v|² formülü kullanılır ve mass'e bölünür.
            // Fakat oyun motorlarında mass genelde 1.0 (veya keyfi) bırakıldığı için bu, birim tutarsızlığına
            // (1 tonluk araç ile 1 kg'lık aracın 1000 kat farklı yavaşlamasına) yol açar.
            // Bunun yerine hava direncini kütleden bağımsız saf hıza etki eden bir "yavaşlama (deceleration)" faktörü olarak uygularız.
            let speed_sq = v.linear.length_squared();
            if speed_sq > 0.01 {
                let cd = vehicle.drag_coefficient;
                // Δv = -½ * Cd * |v| * v * dt
                let drag_dv = v.linear * (-0.5 * cd * speed_sq.sqrt() * dt);
                v.linear += drag_dv;
            }
    }
}

// Varlıkların fiziksel hareketlerini, yerçekimi ve sürtünme etkileriyle uygulayan sistem

gizmo_core::impl_component!(VehicleController, WheelComponent);
