use gizmo_core::World;
use crate::components::{Transform, Velocity, RigidBody};
use crate::shape::{Collider, ColliderShape};
use gizmo_math::{Vec3, Quat};

pub fn apply_inv_inertia(torque: Vec3, inv_inertia: Vec3, rot: Quat) -> Vec3 {
    let local_t = rot.inverse().mul_vec3(torque);
    let local_ang = Vec3::new(local_t.x * inv_inertia.x, local_t.y * inv_inertia.y, local_t.z * inv_inertia.z);
    rot.mul_vec3(local_ang)
}

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
                    if rbs.get(e).is_some_and(|rb| rb.mass == 0.0) {
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

            // 3. HAVA VE YER SÜRTÜNMESİ DAMPING (Dengesiz Jitter'ı durdurur)
            let linear_drag = f32x8::splat((1.0 - dt * 2.0).max(0.0));
            let angular_drag = f32x8::splat((1.0 - dt * 15.0).max(0.0));
            x_v *= linear_drag; y_v *= linear_drag; z_v *= linear_drag;
            x_a *= angular_drag; y_a *= angular_drag; z_a *= angular_drag;

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
            let v = *vel_storage.get(e).unwrap();
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
