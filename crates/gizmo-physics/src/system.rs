use gizmo_core::World;
use crate::components::{Transform, Velocity, RigidBody};
use crate::shape::{Collider, ColliderShape};
use gizmo_math::{Vec3, Quat};
use crate::vehicle::VehicleController;
use std::collections::HashMap;

/// Kalıcı Çözücü Durumu (Warm-Starting Cache için)
pub struct PhysicsSolverState {
    pub cached_impulses: HashMap<(u32, u32), (f32, Vec3)>, // Accumulated Normal Impulse, Accumulated Friction Impulse
}

impl PhysicsSolverState {
    pub fn new() -> Self {
        Self { cached_impulses: HashMap::new() }
    }
}

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
                    if rbs.get(e).map_or(false, |rb| rb.mass == 0.0) {
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
            let t = match trans_storage.get(entity) { Some(t) => t.clone(), None => continue };
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

            for (i, wheel) in vehicle.wheels.iter_mut().enumerate() {
                // Lokal bağlantı noktasını dünya haritasına çevir
                let r_ws = t.rotation.mul_vec3(wheel.connection_point);
                let origin = t.position + r_ws;
                let dir = t.rotation.mul_vec3(wheel.direction).normalize();

                // === RAYCASTING (AABB & Ground Plane) ===
                let mut hit_t = f32::MAX;
                
                // 1. Zemin Yüzeyi (Y = -1.0) fallback olarak
                let ground_y = -1.0_f32;
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
                        total_angular_impulse += Vec3::new(torque.x * inv_inertia.x, torque.y * inv_inertia.y, torque.z * inv_inertia.z);
                    }
                    
                    // === MOTOR GÜCÜ (Arka tekerlekler) ===
                    if i >= 2 && engine_force.abs() > 0.01 {
                        let drive_impulse = forward * (engine_force / 2.0) * dt;
                        total_linear_impulse += drive_impulse;
                        let drive_torque = r_ws.cross(drive_impulse);
                        total_angular_impulse += Vec3::new(drive_torque.x * inv_inertia.x, drive_torque.y * inv_inertia.y, drive_torque.z * inv_inertia.z);
                    }
                    
                    // === FREN ===
                    if brake_force > 0.01 {
                        let forward_speed = v.linear.dot(forward);
                        let brake_impulse = forward * (-forward_speed.signum() * brake_force / num_wheels) * dt;
                        total_linear_impulse += brake_impulse;
                    }
                    
                    // === DİREKSİYON (Ön tekerlekler) ===
                    if i < 2 && steering_angle.abs() > 0.001 {
                        let wheel_vel = v.linear + v.angular.cross(r_ws);
                        let lateral_vel = wheel_vel.dot(right);
                        let steer_force = steering_angle * 8000.0; 
                        let grip_force = -lateral_vel * 5000.0;
                        let lateral_impulse = right * (steer_force + grip_force) * dt / num_wheels;
                        total_linear_impulse += lateral_impulse;
                        
                        let steer_torque = r_ws.cross(lateral_impulse);
                        total_angular_impulse += Vec3::new(
                            steer_torque.x * inv_inertia.x,
                            steer_torque.y * inv_inertia.y, 
                            steer_torque.z * inv_inertia.z
                        );
                    }
                    
                    // Yanal Sürtünme / Tutuş (Araca viraj dönerken drift engelleme)
                    let wheel_vel = v.linear + v.angular.cross(r_ws);
                    let lateral_vel = wheel_vel.dot(right);
                    let anti_slide = right * (-lateral_vel * 3000.0 / num_wheels) * dt;
                    total_linear_impulse += anti_slide;
                    let slide_torque = r_ws.cross(anti_slide);
                    total_angular_impulse += Vec3::new(slide_torque.x * inv_inertia.x, slide_torque.y * inv_inertia.y, slide_torque.z * inv_inertia.z);
                    
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
pub fn physics_movement_system(world: &World, dt: f32) {
    // CCD için collider'ları ayrıca borrow'la (sadece immutable okuma)
    let colliders_storage = world.borrow::<Collider>();
    
    // CCD: Hangi entity'ler statik? Kesişim testi için AABB'leri önceden topla (borrow conflict önleme)
    struct StaticAabb {
        entity: u32,
        position: Vec3,
        half_extents: Vec3,
    }
    let static_aabbs: Vec<StaticAabb> = {
        if let (Some(rbs), Some(ref cols), Some(ts)) = (world.borrow::<RigidBody>(), &colliders_storage, world.borrow::<Transform>()) {
            cols.entity_dense.iter()
                .filter_map(|&e| {
                    if rbs.get(e).map_or(false, |rb| rb.mass == 0.0) {
                        let t = ts.get(e)?;
                        let col = cols.get(e)?;
                        if let ColliderShape::Aabb(aabb) = &col.shape {
                            return Some(StaticAabb {
                                entity: e,
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
    
    if let (Some(mut trans_storage), Some(mut vel_storage), Some(mut rbs)) = (world.borrow_mut::<Transform>(), world.borrow_mut::<Velocity>(), world.borrow_mut::<RigidBody>()) {
        use wide::f32x8;

        // BATCH 1: Aktif objeleri SIMD formatında belleğe al (Structure of Arrays)
        let entities = trans_storage.entity_dense.clone();
        let mut active_ents = Vec::with_capacity(entities.len());
        for &entity in &entities {
            if let Some(rb) = rbs.get_mut(entity) {
                if let Some(v) = vel_storage.get_mut(entity) {
                    if rb.mass > 0.0 {
                        let speed_sq = v.linear.length_squared() + v.angular.length_squared();
                        if speed_sq < 0.05 {
                            rb.sleep_timer += dt;
                            if rb.sleep_timer > 1.0 {
                                rb.is_sleeping = true;
                                v.linear = Vec3::ZERO;
                                v.angular = Vec3::ZERO;
                            }
                        } else {
                            rb.wake_up();
                        }
                    }
                    if !rb.is_sleeping {
                        active_ents.push(entity);
                    }
                }
            }
        }

        // BATCH 2: 8'li Register (Lane) paketlerinde f32x8 AVX operasyonları
        let mut index = 0;
        while index < active_ents.len() {
            let mut chunk_ents = [0u32; 8];
            let mut vx = [0.0; 8]; let mut vy = [0.0; 8]; let mut vz = [0.0; 8];
            let mut ax = [0.0; 8]; let mut ay = [0.0; 8]; let mut az = [0.0; 8];
            let mut grav = [0.0; 8];
            
            let end = (index + 8).min(active_ents.len());
            let valid_count = end - index;
            for i in 0..valid_count {
                let e = active_ents[index + i];
                chunk_ents[i] = e;
                if let Some(v) = vel_storage.get(e) {
                    vx[i] = v.linear.x; vy[i] = v.linear.y; vz[i] = v.linear.z;
                    ax[i] = v.angular.x; ay[i] = v.angular.y; az[i] = v.angular.z;
                }
                if let Some(rb) = rbs.get(e) {
                    grav[i] = if rb.use_gravity && rb.mass > 0.0 { 9.81 } else { 0.0 };
                }
            }

            // SIMD YÜKLEMESİ (AVX Registers)
            let mut x_v = f32x8::new(vx); let mut y_v = f32x8::new(vy); let mut z_v = f32x8::new(vz);
            let mut x_a = f32x8::new(ax); let mut y_a = f32x8::new(ay); let mut z_a = f32x8::new(az);
            let g_v = f32x8::new(grav);
            let wf_dt = f32x8::splat(dt);

            // 1. YERÇEKİMİ UYGULANMASI (Tek CPU komutuyla 8 objenin Y velocity'si güncellenir)
            y_v -= g_v * wf_dt;

            // 2. GÜVENLİK SINIRI (Safety Clamp)
            let max_lin = f32x8::splat(200.0);
            let min_lin = f32x8::splat(-200.0);
            let max_ang = f32x8::splat(100.0);
            let min_ang = f32x8::splat(-100.0);
            x_v = x_v.max(min_lin).min(max_lin);
            y_v = y_v.max(min_lin).min(max_lin);
            z_v = z_v.max(min_lin).min(max_lin);
            x_a = x_a.max(min_ang).min(max_ang);
            y_a = y_a.max(min_ang).min(max_ang);
            z_a = z_a.max(min_ang).min(max_ang);

            // 3. HAVA DİRENCİ (Air Drag - 0.95/sn)
            let drag = f32x8::splat(1.0 - dt * 0.05);
            x_v *= drag; y_v *= drag; z_v *= drag;
            x_a *= drag; y_a *= drag; z_a *= drag;

            // SONUÇLARI ECS'YE GERİ YAZ (SIMD Store)
            let xv_arr = x_v.to_array(); let yv_arr = y_v.to_array(); let zv_arr = z_v.to_array();
            let xa_arr = x_a.to_array(); let ya_arr = y_a.to_array(); let za_arr = z_a.to_array();

            for i in 0..valid_count {
                let e = chunk_ents[i];
                if let Some(v) = vel_storage.get_mut(e) {
                    v.linear = Vec3::new(xv_arr[i], yv_arr[i], zv_arr[i]);
                    v.angular = Vec3::new(xa_arr[i], ya_arr[i], za_arr[i]);
                }
            }
            index += 8;
        }

        // BATCH 3: Pozisyon Entegrasyonu & CCD (Continuous Collision Detection) - Skalar Loop
        for &e in &active_ents {
            let rb = rbs.get(e).unwrap();
            let v = vel_storage.get(e).unwrap().clone();
            let t = match trans_storage.get_mut(e) { Some(t) => t, None => continue };

            // === CCD (Continuous Collision Detection) ===
            // Hızlı objeler için: genişletilmiş AABB üzerinden sphere-sweep
            if rb.ccd_enabled && rb.mass > 0.0 {
                let displacement = v.linear * dt;
                let speed = displacement.length();
                
                if speed > 0.3 { // 0.3m/frame eşik
                    let ray_dir = displacement / speed;
                    let ray_origin = t.position;
                    
                    // Objenin collider yarıçapı
                    let col = colliders_storage.as_ref().and_then(|c| c.get(e));
                    let sweep_radius = match col.map(|c| &c.shape) {
                        Some(crate::shape::ColliderShape::Sphere(s)) => s.radius,
                        Some(crate::shape::ColliderShape::Aabb(a)) => a.half_extents.x.max(a.half_extents.y).max(a.half_extents.z),
                        Some(crate::shape::ColliderShape::Capsule(c)) => c.radius,
                        _ => 0.5,
                    };
                    
                    let mut closest_t = speed;
                    let mut hit_normal = Vec3::ZERO;
                    let mut had_hit = false;
                    
                    let ground_y = -1.0_f32;
                    if ray_dir.y < -0.001 && ray_origin.y > ground_y + sweep_radius {
                        let t_hit = (ground_y + sweep_radius - ray_origin.y) / ray_dir.y;
                        if t_hit > 0.0 && t_hit < closest_t {
                            closest_t = t_hit;
                            hit_normal = Vec3::new(0.0, 1.0, 0.0);
                            had_hit = true;
                        }
                    }
                    
                    let up = Vec3::new(0.0, 1.0, 0.0);
                    let rot_axis = up.cross(ray_dir);
                    let rot_angle = up.dot(ray_dir).acos();
                    let swept_rot = if rot_axis.length_squared() > 1e-6 { Quat::from_axis_angle(rot_axis.normalize(), rot_angle) } else { Quat::IDENTITY };
                    
                    for other in &static_aabbs {
                        if other.entity == e { continue; }
                        
                        let expanded_half = other.half_extents + Vec3::new(sweep_radius, sweep_radius, sweep_radius);
                        let min_b = other.position - expanded_half;
                        let max_b = other.position + expanded_half;
                        
                        let inv_dir = Vec3::new(
                            if ray_dir.x.abs() > 1e-8 { 1.0 / ray_dir.x } else { f32::MAX },
                            if ray_dir.y.abs() > 1e-8 { 1.0 / ray_dir.y } else { f32::MAX },
                            if ray_dir.z.abs() > 1e-8 { 1.0 / ray_dir.z } else { f32::MAX },
                        );
                        let t1x = (min_b.x - ray_origin.x) * inv_dir.x;
                        let t2x = (max_b.x - ray_origin.x) * inv_dir.x;
                        let t1y = (min_b.y - ray_origin.y) * inv_dir.y;
                        let t2y = (max_b.y - ray_origin.y) * inv_dir.y;
                        let t1z = (min_b.z - ray_origin.z) * inv_dir.z;
                        let t2z = (max_b.z - ray_origin.z) * inv_dir.z;
                        
                        let t_near = t1x.min(t2x).max(t1y.min(t2y)).max(t1z.min(t2z));
                        let t_far = t1x.max(t2x).min(t1y.max(t2y)).min(t1z.max(t2z));
                        
                        if t_near <= t_far && t_far > 0.0 && t_near < closest_t {
                            let aabb_col = crate::shape::ColliderShape::Aabb(crate::shape::Aabb { half_extents: other.half_extents });
                            let full_capsule = crate::shape::Capsule { radius: sweep_radius, half_height: speed / 2.0 };
                            let swept_pos = ray_origin + ray_dir * (speed / 2.0);
                            
                            let (collides, _) = crate::gjk::gjk_intersect(&crate::shape::ColliderShape::Capsule(full_capsule), swept_pos, swept_rot, &aabb_col, other.position, Quat::IDENTITY);
                            
                            if collides {
                                let mut t_low = if t_near > 0.0 { t_near } else { 0.0 };
                                let mut t_high = t_far.min(speed);
                                
                                for _ in 0..6 {
                                    let t_mid = (t_low + t_high) * 0.5;
                                    let mid_capsule = crate::shape::Capsule { radius: sweep_radius, half_height: t_mid / 2.0 };
                                    let mid_pos = ray_origin + ray_dir * (t_mid / 2.0);
                                    let (hit, _) = crate::gjk::gjk_intersect(&crate::shape::ColliderShape::Capsule(mid_capsule), mid_pos, swept_rot, &aabb_col, other.position, Quat::IDENTITY);
                                    
                                    if hit { t_high = t_mid; } else { t_low = t_mid; }
                                }
                                
                                let t_hit = t_high;
                                if t_hit < closest_t {
                                    closest_t = t_hit;
                                    let hit_point = ray_origin + ray_dir * t_hit;
                                    let diff = hit_point - other.position;
                                    let abs_diff = Vec3::new(diff.x.abs() / other.half_extents.x, diff.y.abs() / other.half_extents.y, diff.z.abs() / other.half_extents.z);
                                    
                                    if abs_diff.x > abs_diff.y && abs_diff.x > abs_diff.z {
                                        hit_normal = Vec3::new(if diff.x > 0.0 { 1.0 } else { -1.0 }, 0.0, 0.0);
                                    } else if abs_diff.y > abs_diff.z {
                                        hit_normal = Vec3::new(0.0, if diff.y > 0.0 { 1.0 } else { -1.0 }, 0.0);
                                    } else {
                                        hit_normal = Vec3::new(0.0, 0.0, if diff.z > 0.0 { 1.0 } else { -1.0 });
                                    }
                                    had_hit = true;
                                }
                            }
                        }
                    }
                    
                    if had_hit {
                        let safe_t = (closest_t - 0.01).max(0.0);
                        t.position += ray_dir * safe_t;
                        
                        let vel_along_normal = hit_normal * v.linear.dot(hit_normal);
                        if let Some(mut_v) = vel_storage.get_mut(e) {
                            mut_v.linear -= vel_along_normal;
                        }
                        t.update_local_matrix();
                        continue; // CCD triggered, skip normal integration
                    }
                }
            }

            t.position += v.linear * dt;
            
            if v.angular.length_squared() > 0.0001 {
                let w_quat = Quat::from_xyzw(v.angular.x, v.angular.y, v.angular.z, 0.0);
                let q = t.rotation;
                let dq = w_quat * q; 
                t.rotation = Quat::from_xyzw(
                    q.x + 0.5 * dt * dq.x,
                    q.y + 0.5 * dt * dq.y,
                    q.z + 0.5 * dt * dq.z,
                    q.w + 0.5 * dt * dq.w,
                ).normalize();
            }
            
            t.update_local_matrix();
    }
}
}


// O(N^2) Çarpışma Tespit ve Fizik (Impulse/Sekme/Tork) Çözümleyici Sistem
pub fn physics_collision_system(world: &World) {
    // Wake-up listesi — collision scope dışında tanımlanıyor
    let mut entities_to_wake: Vec<u32> = Vec::new();

    { // --- Borrow Scope Başlangıcı (immutable rigidbodies + mutable transforms/velocities) ---
    let mut transforms = match world.borrow_mut::<Transform>() { Some(t) => t, None => { return; } };
    let mut velocities = match world.borrow_mut::<Velocity>() { Some(v) => v, None => { return; } };
    let colliders = match world.borrow::<Collider>() { Some(c) => c, None => { return; } };
    let rigidbodies = match world.borrow::<RigidBody>() { Some(r) => r, None => { return; } };
    
    // Collision Layer: VehicleController olan entity'ler statik objelerle çarpışmaz
    let vehicles = world.borrow::<VehicleController>();

    // 1. BROAD-PHASE: Sweep and Prune (3D Bounding Box / AABB Filtrelemesi)
    struct Interval {
        entity: u32,
        min: Vec3,
        max: Vec3,
    }
    let entities = transforms.entity_dense.clone();

    let mut intervals = Vec::with_capacity(entities.len());
    for &e in &entities {
        let t = match transforms.get(e) { Some(t) => t, None => continue };
        let col = match colliders.get(e) { Some(c) => c, None => continue };

        use crate::shape::ColliderShape;
        let (min, max) = match &col.shape {
            ColliderShape::Aabb(a) => {
                (t.position - a.half_extents, t.position + a.half_extents)
            },
            ColliderShape::Sphere(s) => {
                let radius_vec = Vec3::new(s.radius, s.radius, s.radius);
                (t.position - radius_vec, t.position + radius_vec)
            },
            ColliderShape::Capsule(c) => {
                let ext = c.radius + c.half_height;
                let ext_vec = Vec3::new(ext, ext, ext);
                (t.position - ext_vec, t.position + ext_vec)
            },
            ColliderShape::ConvexHull(hull) => {
                let mut mn = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
                let mut mx = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
                for v in &hull.vertices {
                    let wv = t.position + t.rotation.mul_vec3(*v);
                    mn.x = mn.x.min(wv.x); mn.y = mn.y.min(wv.y); mn.z = mn.z.min(wv.z);
                    mx.x = mx.x.max(wv.x); mx.y = mx.y.max(wv.y); mx.z = mx.z.max(wv.z);
                }
                (mn, mx)
            }
        };
        intervals.push(Interval { entity: e, min, max });
    }

    // =========================================================================
    // Profesyonel Spatial Hash Grid (Uzamsal Karma İzgarası) Broadphase - O(N)
    // Standart Sweep & Prune algoritmasının O(N^2) patlama (stacking) sorununu tamamen 
    // engelleyerek fizik motorlarına özgü AAA kalitesinde izolasyon sağlar.
    // =========================================================================
    const CELL_SIZE: f32 = 4.0; // Çoğu standart oyun objesi için ideal blok boyutu
    let mut grid: std::collections::HashMap<[i32; 3], Vec<usize>> = std::collections::HashMap::new();

    // 1. Tüm objeleri uzay kafesindeki (Grid) 3B indeks hücrelerine O(1) sürede kaydet
    for (i, a) in intervals.iter().enumerate() {
        let min_x = (a.min.x / CELL_SIZE).floor() as i32;
        let max_x = (a.max.x / CELL_SIZE).floor() as i32;
        let min_y = (a.min.y / CELL_SIZE).floor() as i32;
        let max_y = (a.max.y / CELL_SIZE).floor() as i32;
        let min_z = (a.min.z / CELL_SIZE).floor() as i32;
        let max_z = (a.max.z / CELL_SIZE).floor() as i32;

        for x in min_x..=max_x {
            for y in min_y..=max_y {
                for z in min_z..=max_z {
                    grid.entry([x, y, z]).or_insert_with(Vec::new).push(i);
                }
            }
        }
    }

    // 2. Hücre dışı çakışma testlerini ekarte ederek sadece "Aynı Hücredeki" N objeyi sorgula
    let mut collision_pairs_set = std::collections::HashSet::new();

    for cell_entities in grid.values() {
        let len = cell_entities.len();
        if len < 2 { continue; } // Hücrede tek obje varsa çarpışma ihtimali yoktur

        for i in 0..len {
            let idx_a = cell_entities[i];
            let a = &intervals[idx_a];
            for j in (i + 1)..len {
                let idx_b = cell_entities[j];
                let b = &intervals[idx_b];

                // Benzersiz Çift (Tuple) oluştur: (küçük_id, büyük_id)
                let pair = if a.entity < b.entity { (a.entity, b.entity) } else { (b.entity, a.entity) };

                if collision_pairs_set.contains(&pair) { continue; }

                // Daraltılmış hücre içi hassas AABB Testi
                if a.min.x <= b.max.x && a.max.x >= b.min.x &&
                   a.min.y <= b.max.y && a.max.y >= b.min.y &&
                   a.min.z <= b.max.z && a.max.z >= b.min.z {
                    collision_pairs_set.insert(pair);
                }
            }
        }
    }

    // HashSet'ten Rayon Paralel İşleyiciye beslemek üzere Vector'e indirgeme
    let collision_pairs: Vec<(u32, u32)> = collision_pairs_set.into_iter().collect();

    // =========================================================================
    // 2. NARROW-PHASE: Sequential Impulse (SI) Çözücü + Rayon Paralelleştirme
    //    Mimari: Erin Catto (Box2D) SI + Paralel Algılama
    // =========================================================================

    // ---- FAZ 1a: PARALEL ÇARPIŞMA ALGILAMA ----
    // Her çarpışma çiftinin GJK/EPA hesabı bağımsızdır → çekirdekler arası dağıtılır.
    // Immutable (paylaşılan) referanslar thread-safe: &[T] ve &HashMap<K,V> → Sync ✓
    
    struct StoredContact {
        ent_a: u32,
        ent_b: u32,
        normal: Vec3,
        inv_mass_a: f32,
        inv_mass_b: f32,
        inv_inertia_a: Vec3,
        inv_inertia_b: Vec3,
        restitution: f32,
        friction: f32,
        r_a: Vec3,
        r_b: Vec3,
        accumulated_j: f32,
        accumulated_friction: Vec3,
    }

    // Paralel algılama sonucu — her iş parçacığı kendi sonuçlarını üretir
    struct DetectionResult {
        contacts: Vec<StoredContact>,
        wake_entities: Vec<u32>,
        // Pozisyon düzeltmeleri: (entity, direction_sign, correction_vec, inv_mass)
        corrections: Vec<(u32, f32, Vec3, f32)>,
    }

    // Paylaşılan immutable referanslar — SparseSet.dense (&[T]) ve SparseSet.sparse (&HashMap) Sync ✓
    let t_dense = &transforms.dense;
    let t_sparse = &transforms.sparse;
    let c_dense = &colliders.dense;
    let c_sparse = &colliders.sparse;
    let rb_dense = &rigidbodies.dense;
    let rb_sparse = &rigidbodies.sparse;
    
    // Vehicle entity ID'lerini thread-safe HashSet'e çıkar
    // Ref<SparseSet> = !Sync (Cell içerir), ama entity_dense (&[u32]) = Sync
    let vehicle_entities: std::collections::HashSet<u32> = match &vehicles {
        Some(v) => v.entity_dense.iter().cloned().collect(),
        None => std::collections::HashSet::new(),
    };
    let has_vehicles = vehicles.is_some();
    let v_set = &vehicle_entities;

    use rayon::prelude::*;
    use crate::shape::ColliderShape;

    // Paralel algılama: her çarpışma çifti bağımsız iş parçacığında işlenir
    let detection_results: Vec<DetectionResult> = collision_pairs.par_iter().filter_map(|&(ent_a, ent_b)| {
        // Bileşen lookup (hash tabanlı O(1) — immutable, thread-safe)
        let rb_a = rb_sparse.get(&ent_a).map(|&i| &rb_dense[i])?;
        let rb_b = rb_sparse.get(&ent_b).map(|&i| &rb_dense[i])?;

        if (rb_a.mass == 0.0 && rb_b.mass == 0.0) || (rb_a.is_sleeping && rb_b.is_sleeping) { return None; }

        if has_vehicles {
            if (v_set.contains(&ent_a) && rb_b.mass == 0.0) || (v_set.contains(&ent_b) && rb_a.mass == 0.0) { return None; }
        }

        let col_a = c_sparse.get(&ent_a).map(|&i| &c_dense[i])?;
        let col_b = c_sparse.get(&ent_b).map(|&i| &c_dense[i])?;
        let t_a = t_sparse.get(&ent_a).map(|&i| &t_dense[i])?;
        let t_b = t_sparse.get(&ent_b).map(|&i| &t_dense[i])?;
        let (pos_a, rot_a) = (t_a.position, t_a.rotation);
        let (pos_b, rot_b) = (t_b.position, t_b.rotation);

        // Çarpışma algılama (analitik veya GJK/EPA) — saf hesaplama, yan etkisiz
        let mut is_fast_path = false;
        let mut manifold = crate::collision::CollisionManifold {
            is_colliding: false, normal: Vec3::ZERO, penetration: 0.0, contact_points: vec![]
        };

        if let (ColliderShape::Sphere(s), ColliderShape::Aabb(a)) = (&col_a.shape, &col_b.shape) {
            is_fast_path = true;
            let min_box = pos_b - a.half_extents;
            let max_box = pos_b + a.half_extents;
            let cp = Vec3::new(pos_a.x.clamp(min_box.x, max_box.x), pos_a.y.clamp(min_box.y, max_box.y), pos_a.z.clamp(min_box.z, max_box.z));
            let diff = cp - pos_a;
            let dist_sq = diff.length_squared();
            if dist_sq < s.radius * s.radius {
                let dist = dist_sq.sqrt();
                manifold.is_colliding = true;
                if dist > 0.0001 { manifold.normal = diff / dist; manifold.penetration = s.radius - dist; }
                else { manifold.normal = Vec3::new(0.0, -1.0, 0.0); manifold.penetration = s.radius; }
                manifold.contact_points.push(cp);
            }
        } else if let (ColliderShape::Aabb(a), ColliderShape::Sphere(s)) = (&col_a.shape, &col_b.shape) {
            is_fast_path = true;
            let min_box = pos_a - a.half_extents;
            let max_box = pos_a + a.half_extents;
            let cp = Vec3::new(pos_b.x.clamp(min_box.x, max_box.x), pos_b.y.clamp(min_box.y, max_box.y), pos_b.z.clamp(min_box.z, max_box.z));
            let diff = pos_b - cp;
            let dist_sq = diff.length_squared();
            if dist_sq < s.radius * s.radius {
                let dist = dist_sq.sqrt();
                manifold.is_colliding = true;
                if dist > 0.0001 { manifold.normal = diff / dist; manifold.penetration = s.radius - dist; }
                else { manifold.normal = Vec3::new(0.0, 1.0, 0.0); manifold.penetration = s.radius; }
                manifold.contact_points.push(cp);
            }
        } else if let (ColliderShape::Aabb(a1), ColliderShape::Aabb(a2)) = (&col_a.shape, &col_b.shape) {
            is_fast_path = true;
            manifold = crate::collision::check_aabb_aabb_manifold(pos_a, a1, pos_b, a2);
        } else if let (ColliderShape::Capsule(c1), ColliderShape::Capsule(c2)) = (&col_a.shape, &col_b.shape) {
            is_fast_path = true;
            manifold = crate::collision::check_capsule_capsule_manifold(pos_a, rot_a, c1, pos_b, rot_b, c2);
        } else if let (ColliderShape::Capsule(c), ColliderShape::Sphere(s)) = (&col_a.shape, &col_b.shape) {
            is_fast_path = true;
            manifold = crate::collision::check_capsule_sphere_manifold(pos_a, rot_a, c, pos_b, s);
        } else if let (ColliderShape::Sphere(s), ColliderShape::Capsule(c)) = (&col_a.shape, &col_b.shape) {
            is_fast_path = true;
            manifold = crate::collision::check_capsule_sphere_manifold(pos_b, rot_b, c, pos_a, s);
            manifold.normal = manifold.normal * -1.0;
        } else if let (ColliderShape::Capsule(c), ColliderShape::Aabb(a)) = (&col_a.shape, &col_b.shape) {
            is_fast_path = true;
            manifold = crate::collision::check_capsule_aabb_manifold(pos_a, rot_a, c, pos_b, a);
        } else if let (ColliderShape::Aabb(a), ColliderShape::Capsule(c)) = (&col_a.shape, &col_b.shape) {
            is_fast_path = true;
            manifold = crate::collision::check_capsule_aabb_manifold(pos_b, rot_b, c, pos_a, a);
            manifold.normal = manifold.normal * -1.0;
        } else if let (ColliderShape::Sphere(s1), ColliderShape::Sphere(s2)) = (&col_a.shape, &col_b.shape) {
            is_fast_path = true;
            manifold = crate::collision::check_sphere_sphere_manifold(pos_a, s1, pos_b, s2);
        }

        if !is_fast_path {
            let (is_colliding, simplex) = crate::gjk::gjk_intersect(&col_a.shape, pos_a, rot_a, &col_b.shape, pos_b, rot_b);
            if is_colliding {
                manifold = crate::epa::epa_solve(simplex, &col_a.shape, pos_a, rot_a, &col_b.shape, pos_b, rot_b);
            }
        }

        if !manifold.is_colliding || manifold.contact_points.is_empty() {
            return None;
        }

        // Sonuç yapıları oluştur (heap alloc yok — tüm veriler stack'te)
        let inv_mass_a = if rb_a.mass == 0.0 { 0.0 } else { 1.0 / rb_a.mass };
        let inv_mass_b = if rb_b.mass == 0.0 { 0.0 } else { 1.0 / rb_b.mass };
        let sum_inv = inv_mass_a + inv_mass_b;

        // SADECE ZATEN UYUYAN OBJELERİ UYANDIR!
        // Eğer iki obje de zaten uyanıksa birbirlerini dürtüp sleep_timer'larını SIFIRLAMAMALILAR.
        // Yoksa üst üste binen objeler sonsuza dek titreşir ve asla uyumazlar.
        let mut wakes = Vec::new();
        if rb_a.is_sleeping { wakes.push(ent_a); }
        if rb_b.is_sleeping { wakes.push(ent_b); }

        let mut result = DetectionResult {
            contacts: Vec::new(),
            wake_entities: wakes,
            corrections: Vec::new(),
        };

        // Pozisyon düzeltmesi verilerini topla (uygulanması sıralı fazda olacak)
        if sum_inv > 0.0 {
            let correction = (manifold.penetration - 0.01).max(0.0) / sum_inv * 0.2;
            let cv = manifold.normal * correction;
            result.corrections.push((ent_a, -1.0, cv, inv_mass_a)); // -= cv * inv_mass_a
            result.corrections.push((ent_b, 1.0, cv, inv_mass_b));  // += cv * inv_mass_b
        }

        // Temas noktaları
        for contact_point in &manifold.contact_points {
            let mut r_a = *contact_point - pos_a;
            let mut r_b = *contact_point - pos_b;
            if let ColliderShape::Sphere(s) = &col_a.shape { r_a = manifold.normal * s.radius; }
            if let ColliderShape::Sphere(s) = &col_b.shape { r_b = manifold.normal * -s.radius; }

            result.contacts.push(StoredContact {
                ent_a, ent_b,
                normal: manifold.normal,
                inv_mass_a, inv_mass_b,
                inv_inertia_a: rb_a.inverse_inertia,
                inv_inertia_b: rb_b.inverse_inertia,
                restitution: rb_a.restitution.max(rb_b.restitution),
                friction: (rb_a.friction + rb_b.friction) * 0.5,
                r_a, r_b,
                accumulated_j: 0.0,
                accumulated_friction: Vec3::ZERO,
            });
        }

        Some(result)
    }).collect();

    // ---- FAZ 1b: ISLAND GENERATION & POZİSYON DÜZELTMELERİ ----
    let mut parent_map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();

    fn find_root(parent: &mut std::collections::HashMap<u32, u32>, mut i: u32) -> u32 {
        while i != *parent.entry(i).or_insert(i) {
            i = *parent.get(&i).unwrap();
        }
        i
    }

    fn union_nodes(parent: &mut std::collections::HashMap<u32, u32>, i: u32, j: u32) {
        let root_i = find_root(parent, i);
        let root_j = find_root(parent, j);
        if root_i != root_j {
            parent.insert(root_j, root_i);
        }
    }

    struct Island {
        contacts: Vec<StoredContact>,
        velocities: std::collections::HashMap<u32, Velocity>,
    }

    let mut all_contacts = Vec::new();
    for result in detection_results {
        for &(entity, sign, ref cv, inv_mass) in &result.corrections {
            if let Some(t) = transforms.get_mut(entity) {
                t.position += *cv * (sign * inv_mass);
            }
        }
        entities_to_wake.extend(result.wake_entities);
        for c in result.contacts {
            let a_dyn = c.inv_mass_a > 0.0;
            let b_dyn = c.inv_mass_b > 0.0;
            if a_dyn && b_dyn { union_nodes(&mut parent_map, c.ent_a, c.ent_b); }
            else if a_dyn { find_root(&mut parent_map, c.ent_a); }
            else if b_dyn { find_root(&mut parent_map, c.ent_b); }
            all_contacts.push(c);
        }
    }

    let mut islands_map: std::collections::HashMap<u32, Island> = std::collections::HashMap::new();
    for c in all_contacts {
        let a_dyn = c.inv_mass_a > 0.0;
        let root = if a_dyn { find_root(&mut parent_map, c.ent_a) } else { find_root(&mut parent_map, c.ent_b) };
        let island = islands_map.entry(root).or_insert_with(|| Island {
            contacts: Vec::new(),
            velocities: std::collections::HashMap::new(),
        });
        island.contacts.push(c);
    }

    for island in islands_map.values_mut() {
        for c in &island.contacts {
            if c.inv_mass_a > 0.0 && !island.velocities.contains_key(&c.ent_a) {
                island.velocities.insert(c.ent_a, velocities.get(c.ent_a).cloned().unwrap_or(Velocity::new(Vec3::ZERO)));
            }
            if c.inv_mass_b > 0.0 && !island.velocities.contains_key(&c.ent_b) {
                island.velocities.insert(c.ent_b, velocities.get(c.ent_b).cloned().unwrap_or(Velocity::new(Vec3::ZERO)));
            }
        }
    }

    let mut islands: Vec<Island> = islands_map.into_values().collect();

    if let Some(mut state) = world.get_resource_mut::<PhysicsSolverState>() {
        let mut old_cache = std::mem::take(&mut state.cached_impulses);
        for island in &mut islands {
            for c in &mut island.contacts {
                let pair = if c.ent_a < c.ent_b { (c.ent_a, c.ent_b) } else { (c.ent_b, c.ent_a) };
                if let Some((acc_n, acc_f)) = old_cache.remove(&pair) {
                    c.accumulated_j = acc_n;
                    c.accumulated_friction = acc_f;
                    let impulse = c.normal * acc_n + acc_f;
                    if let Some(v_a) = island.velocities.get_mut(&c.ent_a) {
                        v_a.linear -= impulse * c.inv_mass_a;
                        let t = c.r_a.cross(impulse * -1.0);
                        v_a.angular += Vec3::new(t.x * c.inv_inertia_a.x, t.y * c.inv_inertia_a.y, t.z * c.inv_inertia_a.z);
                    }
                    if let Some(v_b) = island.velocities.get_mut(&c.ent_b) {
                        v_b.linear += impulse * c.inv_mass_b;
                        let t = c.r_b.cross(impulse);
                        v_b.angular += Vec3::new(t.x * c.inv_inertia_b.x, t.y * c.inv_inertia_b.y, t.z * c.inv_inertia_b.z);
                    }
                }
            }
        }
    }

    const MAX_ANG: f32 = 100.0;
    const MAX_LIN: f32 = 200.0;

    // ---- FAZ 2: PARALEL ADA ÇÖZÜMÜ ----
    islands.par_iter_mut().for_each(|island| {
        for _iter in 0..8 {
            for c in island.contacts.iter_mut() {
                let va = island.velocities.get(&c.ent_a).cloned().unwrap_or(Velocity::new(Vec3::ZERO));
                let vb = island.velocities.get(&c.ent_b).cloned().unwrap_or(Velocity::new(Vec3::ZERO));

                let vpa = va.linear + va.angular.cross(c.r_a);
                let vpb = vb.linear + vb.angular.cross(c.r_b);
                let rel = vpb - vpa;
                let vn = rel.dot(c.normal);

                let mut e = c.restitution;
                if vn.abs() < 0.2 { e = 0.0; }

                let ra_x_n = c.r_a.cross(c.normal);
                let rb_x_n = c.r_b.cross(c.normal);
                let it_a = Vec3::new(ra_x_n.x * c.inv_inertia_a.x, ra_x_n.y * c.inv_inertia_a.y, ra_x_n.z * c.inv_inertia_a.z);
                let it_b = Vec3::new(rb_x_n.x * c.inv_inertia_b.x, rb_x_n.y * c.inv_inertia_b.y, rb_x_n.z * c.inv_inertia_b.z);
                let ang_a = it_a.cross(c.r_a).dot(c.normal);
                let ang_b = it_b.cross(c.r_b).dot(c.normal);
                let eff_mass = c.inv_mass_a + c.inv_mass_b + ang_a + ang_b;
                if eff_mass == 0.0 { continue; }

                let j_new = -(1.0 + e) * vn / eff_mass;
                let old_acc = c.accumulated_j;
                c.accumulated_j = (c.accumulated_j + j_new).max(0.0);
                let j = c.accumulated_j - old_acc;
                if j.abs() > 1e-8 {
                    let impulse = c.normal * j;

                    if let Some(v_a) = island.velocities.get_mut(&c.ent_a) {
                        v_a.linear -= impulse * c.inv_mass_a;
                        v_a.linear.x = v_a.linear.x.clamp(-MAX_LIN, MAX_LIN);
                        v_a.linear.y = v_a.linear.y.clamp(-MAX_LIN, MAX_LIN);
                        v_a.linear.z = v_a.linear.z.clamp(-MAX_LIN, MAX_LIN);
                        let t = c.r_a.cross(impulse * -1.0); 
                        v_a.angular += Vec3::new(t.x * c.inv_inertia_a.x, t.y * c.inv_inertia_a.y, t.z * c.inv_inertia_a.z);
                        v_a.angular.x = v_a.angular.x.clamp(-MAX_ANG, MAX_ANG);
                        v_a.angular.y = v_a.angular.y.clamp(-MAX_ANG, MAX_ANG);
                        v_a.angular.z = v_a.angular.z.clamp(-MAX_ANG, MAX_ANG);
                    }
                    if let Some(v_b) = island.velocities.get_mut(&c.ent_b) {
                        v_b.linear += impulse * c.inv_mass_b;
                        v_b.linear.x = v_b.linear.x.clamp(-MAX_LIN, MAX_LIN);
                        v_b.linear.y = v_b.linear.y.clamp(-MAX_LIN, MAX_LIN);
                        v_b.linear.z = v_b.linear.z.clamp(-MAX_LIN, MAX_LIN);
                        let t = c.r_b.cross(impulse);
                        v_b.angular += Vec3::new(t.x * c.inv_inertia_b.x, t.y * c.inv_inertia_b.y, t.z * c.inv_inertia_b.z);
                        v_b.angular.x = v_b.angular.x.clamp(-MAX_ANG, MAX_ANG);
                        v_b.angular.y = v_b.angular.y.clamp(-MAX_ANG, MAX_ANG);
                        v_b.angular.z = v_b.angular.z.clamp(-MAX_ANG, MAX_ANG);
                    }
                }

                // === COULOMB SÜRTÜNME ===
                let va2 = island.velocities.get(&c.ent_a).cloned().unwrap_or(Velocity::new(Vec3::ZERO));
                let vb2 = island.velocities.get(&c.ent_b).cloned().unwrap_or(Velocity::new(Vec3::ZERO));
                let rel2 = (vb2.linear + vb2.angular.cross(c.r_b)) - (va2.linear + va2.angular.cross(c.r_a));
                let tangent_vel = rel2 - c.normal * rel2.dot(c.normal);
                let ts = tangent_vel.length();

                if ts > 0.001 {
                    let tangent_dir = tangent_vel / ts;
                    let mu_s = c.friction;
                    
                    let ra_cross_t = c.r_a.cross(tangent_dir);
                    let rb_cross_t = c.r_b.cross(tangent_dir);
                    let ita = Vec3::new(ra_cross_t.x * c.inv_inertia_a.x, ra_cross_t.y * c.inv_inertia_a.y, ra_cross_t.z * c.inv_inertia_a.z);
                    let itb = Vec3::new(rb_cross_t.x * c.inv_inertia_b.x, rb_cross_t.y * c.inv_inertia_b.y, rb_cross_t.z * c.inv_inertia_b.z);
                    
                    let tangent_eff_mass = c.inv_mass_a + c.inv_mass_b + ita.cross(c.r_a).dot(tangent_dir) + itb.cross(c.r_b).dot(tangent_dir);
                    
                    if tangent_eff_mass > 0.0 {
                        let jt = -ts / tangent_eff_mass;
                        let max_friction = c.accumulated_j * mu_s;
                        let old_friction = c.accumulated_friction;
                        
                        let mut new_friction = old_friction + tangent_dir * jt;
                        let friction_len = new_friction.length();
                        
                        if friction_len > max_friction {
                            new_friction = new_friction * (max_friction / friction_len);
                        }
                        
                        let fi = new_friction - old_friction;
                        c.accumulated_friction = new_friction;

                        if let Some(v) = island.velocities.get_mut(&c.ent_a) {
                            v.linear -= fi * c.inv_mass_a;
                            let ft = c.r_a.cross(fi * -1.0);
                            v.angular += Vec3::new(ft.x * c.inv_inertia_a.x, ft.y * c.inv_inertia_a.y, ft.z * c.inv_inertia_a.z);
                            v.angular.x = v.angular.x.clamp(-MAX_ANG, MAX_ANG);
                            v.angular.y = v.angular.y.clamp(-MAX_ANG, MAX_ANG);
                            v.angular.z = v.angular.z.clamp(-MAX_ANG, MAX_ANG);
                        }
                        if let Some(v) = island.velocities.get_mut(&c.ent_b) {
                            v.linear += fi * c.inv_mass_b;
                            let ft = c.r_b.cross(fi);
                            v.angular += Vec3::new(ft.x * c.inv_inertia_b.x, ft.y * c.inv_inertia_b.y, ft.z * c.inv_inertia_b.z);
                            v.angular.x = v.angular.x.clamp(-MAX_ANG, MAX_ANG);
                            v.angular.y = v.angular.y.clamp(-MAX_ANG, MAX_ANG);
                            v.angular.z = v.angular.z.clamp(-MAX_ANG, MAX_ANG);
                        }
                    }
                }
            }
        }
    });

    // Yazımları ana array'e geri aktar (Sync phase)
    let mut sync_cache = Vec::new();
    for island in islands {
        for (ent, vel) in island.velocities {
            if let Some(v) = velocities.get_mut(ent) {
                *v = vel;
            }
        }
        for c in island.contacts {
            sync_cache.push((c.ent_a, c.ent_b, c.accumulated_j, c.accumulated_friction));
        }
    }

    // === FAZ 3: WARM STARTING CACHE KAYDI ===
    // Çözülen kısıtlayıcıları (constraints) bir sonraki karede (frame) esnemeden kullanmak üzere kaydet.
    if let Some(mut state) = world.get_resource_mut::<PhysicsSolverState>() {
        for (ent_a, ent_b, acc_n, acc_f) in sync_cache {
            let pair = if ent_a < ent_b { (ent_a, ent_b) } else { (ent_b, ent_a) };
            state.cached_impulses.insert(pair, (acc_n, acc_f));
        }
    } // --- Borrow Scope Sonu (state) ---

    } // --- Borrow Scope Sonu (transforms, velocities, colliders, rigidbodies drop ediliyor) ---

    // Uyuyan ve dokunulan objeleri UYANDIR!
    // Tüm immutable borrow'lar scope dışına çıktı, güvenle borrow_mut yapabiliriz
    if !entities_to_wake.is_empty() {
        if let Some(mut rbs) = world.borrow_mut::<RigidBody>() {
            for e in entities_to_wake {
                if let Some(rb) = rbs.get_mut(e) {
                    rb.wake_up();
                }
            }
        }
    }
} // closes physics_collision_system
