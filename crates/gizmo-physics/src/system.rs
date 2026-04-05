use gizmo_core::World;
use crate::components::{Transform, Velocity, RigidBody};
use crate::shape::{Collider, ColliderShape};
use gizmo_math::{Vec3, Quat};
use crate::vehicle::VehicleController;

pub fn physics_vehicle_system(world: &World, dt: f32) {
    if let (Some(mut trans_storage), Some(mut vel_storage), Some(mut rbs), Some(mut vehicles)) = 
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
            
            // Aracın ileri yönü (lokal Z+ → dünya koordinatı)
            let forward = t.rotation.mul_vec3(Vec3::new(0.0, 0.0, 1.0)).normalize();
            let right = t.rotation.mul_vec3(Vec3::new(1.0, 0.0, 0.0)).normalize();
            
            let num_wheels = vehicle.wheels.len() as f32;
            let engine_force = vehicle.engine_force;
            let steering_angle = vehicle.steering_angle;
            let brake_force = vehicle.brake_force;

            for (i, wheel) in vehicle.wheels.iter_mut().enumerate() {
                // Tekerleğin gövdeye bağlanma ofseti (Rotasyona göre global offset)
                let r_ws = t.rotation.mul_vec3(wheel.connection_point);
                let origin = t.position + r_ws;
                let dir = t.rotation.mul_vec3(wheel.direction).normalize();

                // Zemin yüzeyi Y=-1.0 (ground plane transform)
                let target_y = -1.0_f32;
                
                if dir.y.abs() > 0.001 {
                    let hit_t = (target_y + wheel.wheel_radius - origin.y) / dir.y;
                    
                    if hit_t > 0.0 && hit_t < wheel.suspension_rest_length {
                        wheel.is_grounded = true;
                        wheel.compression = wheel.suspension_rest_length - hit_t;
                        
                        // === SÜSPANSIYON (Hooke Yasası) ===
                        let spring_force = wheel.suspension_stiffness * wheel.compression;
                        let wheel_vel = v.linear + v.angular.cross(r_ws);
                        let vel_along_dir = wheel_vel.dot(dir);
                        let damping_force = wheel.suspension_damping * vel_along_dir;
                        let total_suspension_force = (spring_force + damping_force).max(0.0);
                        
                        let suspension_impulse = dir * -total_suspension_force * dt;
                        total_linear_impulse += suspension_impulse;
                        let torque = r_ws.cross(suspension_impulse);
                        total_angular_impulse += Vec3::new(torque.x * inv_inertia.x, torque.y * inv_inertia.y, torque.z * inv_inertia.z);
                        
                        // === MOTOR GÜCÜ (Arka tekerlekler = indeks 2, 3) ===
                        if i >= 2 && engine_force.abs() > 0.01 {
                            let drive_impulse = forward * (engine_force / 2.0) * dt;
                            total_linear_impulse += drive_impulse;
                        }
                        
                        // === FREN ===
                        if brake_force > 0.01 {
                            let forward_speed = v.linear.dot(forward);
                            let brake_impulse = forward * (-forward_speed.signum() * brake_force / num_wheels) * dt;
                            total_linear_impulse += brake_impulse;
                        }
                        
                        // === DİREKSİYON (Ön tekerlekler = indeks 0, 1) ===
                        if i < 2 && steering_angle.abs() > 0.001 {
                            // Yanal hız (kayma)
                            let lateral_vel = wheel_vel.dot(right);
                            // Direksiyon açısına göre yanal kuvvet
                            let steer_force = steering_angle * 8000.0; // Direksiyon hassasiyeti
                            let grip_force = -lateral_vel * 5000.0; // Yanal tutuş (grip)
                            let lateral_impulse = right * (steer_force + grip_force) * dt / num_wheels;
                            total_linear_impulse += lateral_impulse;
                            
                            // Direksiyon torku (Aracı döndür)
                            let steer_torque = r_ws.cross(lateral_impulse);
                            total_angular_impulse += Vec3::new(
                                steer_torque.x * inv_inertia.x,
                                steer_torque.y * inv_inertia.y, 
                                steer_torque.z * inv_inertia.z
                            );
                        }
                        
                        // === YANAL SÜRTÜNMe (tüm tekerlekler, kayma önleyici) ===
                        let lateral_vel = wheel_vel.dot(right);
                        let anti_slide = right * (-lateral_vel * 3000.0 / num_wheels) * dt;
                        total_linear_impulse += anti_slide;
                        
                    } else {
                        wheel.is_grounded = false;
                        wheel.compression = 0.0;
                    }
                }
            }
            
            // === HAVA DİRENCİ (Drag) ===
            let speed_sq = v.linear.length_squared();
            if speed_sq > 0.1 {
                let drag = v.linear * (-0.5 * speed_sq.sqrt() * 0.3 * dt); // Cd=0.3
                total_linear_impulse += drag;
            }

            v.linear += total_linear_impulse * inv_mass;
            // Açısal impulsu azalt (süspansiyon torku arabayı devirmesin)
            v.angular += total_angular_impulse * 0.2;
            
            // Güçlü açısal sönümleme (takla atmayı engelle)
            v.angular *= 0.85;
            
            // Sert açısal hız limiti (2 rad/s max — takla imkansız)
            let max_angular = 2.0;
            v.angular.x = v.angular.x.clamp(-max_angular, max_angular);
            v.angular.y = v.angular.y.clamp(-max_angular, max_angular);
            v.angular.z = v.angular.z.clamp(-max_angular, max_angular);
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
        let entities = trans_storage.entity_dense.clone();
        for entity in entities {
            let e = entity;
            let rb = match rbs.get_mut(entity) { Some(r) => r, None => continue };
            let v = match vel_storage.get_mut(entity) { Some(v) => v, None => continue };
            let t = match trans_storage.get_mut(entity) { Some(t) => t, None => continue };
            
            // CCD için bu entity'nin collider'ını al
            let col = colliders_storage.as_ref().and_then(|c| c.get(entity));

            if rb.mass > 0.0 {
                let speed_sq = v.linear.length_squared() + v.angular.length_squared();
                if speed_sq < 0.05 { // Yerçekimi ivmesi (0.15/frame) sebebiyle kare hızı 0.025'e çıkıyor. Uyuması için 0.05 yaptık.
                    rb.sleep_timer += dt;
                    if rb.sleep_timer > 1.0 { // 1 sn boyunca durağansa uyut
                        rb.is_sleeping = true;
                        v.linear = Vec3::ZERO;
                        v.angular = Vec3::ZERO;
                    }
                } else {
                    rb.wake_up(); // Hareket etti, uyandır
                }
            }

            if rb.is_sleeping {
                continue; // Uyuyan objeler hareket etmez, yerçekimine yenilmez, kaynak tüketmez!
            }

            // Kuvvetleri Uygula (Eğer Katı Cisim ise)
            if rb.use_gravity && rb.mass > 0.0 {
                v.linear.y -= 9.81 * dt; // Yerçekimi ivmesi
            }
            
            if rb.friction > 0.0 && rb.mass > 0.0 {
                v.linear.x *= 1.0 - (rb.friction * dt);
                v.linear.z *= 1.0 - (rb.friction * dt);
                v.angular.x *= 1.0 - (rb.friction * dt * 0.5); // Açısal sürtünme
                v.angular.y *= 1.0 - (rb.friction * dt * 0.5);
                v.angular.z *= 1.0 - (rb.friction * dt * 0.5);
            }
            
            // === CCD (Continuous Collision Detection) ===
            // Hızlı objeler için: genişletilmiş AABB üzerinden sphere-sweep
            if rb.ccd_enabled && rb.mass > 0.0 {
                let displacement = v.linear * dt;
                let speed = displacement.length();
                
                if speed > 0.3 { // 0.3m/frame eşik
                    let ray_dir = displacement / speed;
                    let ray_origin = t.position;
                    
                    // Objenin collider yarıçapı (sphere-sweep genişliği)
                    let sweep_radius = match col.map(|c| &c.shape) {
                        Some(ColliderShape::Sphere(s)) => s.radius,
                        Some(ColliderShape::Aabb(a)) => a.half_extents.x.max(a.half_extents.y).max(a.half_extents.z),
                        Some(ColliderShape::Capsule(c)) => c.radius,
                        _ => 0.5,
                    };
                    
                    // Tüm statik AABB'lere karşı sphere-sweep (Minkowski genişletilmiş AABB)
                    let mut closest_t = speed;
                    let mut hit_normal = Vec3::ZERO;
                    let mut had_hit = false;
                    
                    // Zemin düzlemi (fallback)
                    let ground_y = -1.0_f32;
                    if ray_dir.y < -0.001 && ray_origin.y > ground_y + sweep_radius {
                        let t_hit = (ground_y + sweep_radius - ray_origin.y) / ray_dir.y;
                        if t_hit > 0.0 && t_hit < closest_t {
                            closest_t = t_hit;
                            hit_normal = Vec3::new(0.0, 1.0, 0.0);
                            had_hit = true;
                        }
                    }
                    
                    // Diğer statik collider'lara karşı test
                    for other in &static_aabbs {
                        if other.entity == e { continue; }
                        
                        // Minkowski genişletilmiş AABB (AABB + sweep_radius)
                        let expanded_half = other.half_extents + Vec3::new(sweep_radius, sweep_radius, sweep_radius);
                        
                        // Ray-AABB kesişim testi (Slab method)
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
                                    let t_hit = if t_near > 0.0 { t_near } else { 0.0 };
                                    closest_t = t_hit;
                                    
                                    // Hit normal (hangi yüze çarptık)
                                    let hit_point = ray_origin + ray_dir * t_hit;
                                    let diff = hit_point - other.position;
                                    let abs_diff = Vec3::new(diff.x.abs(), diff.y.abs(), diff.z.abs());
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
                    
                    if had_hit {
                        let safe_t = (closest_t - 0.01).max(0.0);
                        t.position += ray_dir * safe_t;
                        // Çarpışma normaline dik hız bileşenini koru, normal yönündekini sıfırla
                        let vel_along_normal = hit_normal * v.linear.dot(hit_normal);
                        v.linear -= vel_along_normal;
                        t.update_local_matrix();
                        continue;
                    }
                }
            }
            
            // Hızı pozisyona uygula
            t.position += v.linear * dt;
            
            // Açısal Hızı (Angular Velocity) Quat dönüşümüne entegre et: q = q + 0.5 * w * q * dt
            if v.angular.length_squared() > 0.0001 {
                let w_quat = Quat::new(v.angular.x, v.angular.y, v.angular.z, 0.0);
                let q = t.rotation;
                let dq = w_quat * q; 
                t.rotation = Quat::new(
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
    use rayon::prelude::*;
    // Sweep için objeleri rastgele bir eksende (Örn. Y) diziyoruz
    intervals.sort_by(|a, b| a.min.y.partial_cmp(&b.min.y).unwrap_or(std::cmp::Ordering::Equal));

    let mut collision_pairs = Vec::new();
    for i in 0..intervals.len() {
        let a = &intervals[i];
        for j in (i + 1)..intervals.len() {
            let b = &intervals[j];
            if b.min.y > a.max.y {
                break; // PRUNE! Y eksenini aştık, geri kalanların tümü imkansız. (O(n^2) engellendi)
            }
            
            // X ve Z eksenlerinde de tam kesişim varsa bu gerçekçi bir adaydır!
            // Bu ekstra AABB filtresi yüz binlerce gereksiz Dar-Aşama (Narrow-phase) iterasyonunu yutar.
            if a.min.x <= b.max.x && a.max.x >= b.min.x &&
               a.min.z <= b.max.z && a.max.z >= b.min.z {
                collision_pairs.push((a.entity, b.entity));
            }
        }
    }

    // 2. NARROW-PHASE: GJK/EPA ve Çözümleyici
    let solver_iterations = 8; // Alt adımlama (sub-stepping) ile birlikte daha güçlü çözümler
    for _iter in 0..solver_iterations {
        for &(ent_a, ent_b) in &collision_pairs {
            let (rb_a, rb_b) = match (rigidbodies.get(ent_a), rigidbodies.get(ent_b)) {
                (Some(a), Some(b)) => (a, b),
                _ => continue,
            };

            // İkisinin de kütlesi yoksa veya İKİSİ DE UYUYORSA çarpışma çözümüne gerek yok
            if (rb_a.mass == 0.0 && rb_b.mass == 0.0) || (rb_a.is_sleeping && rb_b.is_sleeping) { 
                continue; 
            }
            
            // === COLLISION LAYER: Araç (VehicleController) + Statik Obje = ATLA ===
            // Süspansiyon raycast'leri zemin temasını yönetiyor, AABB çakışması istemiyoruz.
            if let Some(ref v) = vehicles {
                let a_is_vehicle = v.contains(ent_a);
                let b_is_vehicle = v.contains(ent_b);
                if (a_is_vehicle && rb_b.mass == 0.0) || (b_is_vehicle && rb_a.mass == 0.0) {
                    continue;
                }
            }

            if let (Some(col_a), Some(col_b)) = (colliders.get(ent_a), colliders.get(ent_b)) {
                // Tek seferde Transform lookup (4 ayrı HashMap hit yerine 2)
                let (pos_a, rot_a) = match transforms.get(ent_a) {
                    Some(t) => (t.position, t.rotation),
                    None => continue,
                };
                let (pos_b, rot_b) = match transforms.get(ent_b) {
                    Some(t) => (t.position, t.rotation),
                    None => continue,
                };

                // Evrensel GJK-EPA Çarpışma Testi (Öncesi Özel Optimizasyonlar)
                let mut is_fast_path = false;
                let mut manifold = crate::collision::CollisionManifold { 
                    is_colliding: false, normal: Vec3::ZERO, penetration: 0.0, contact_points: vec![] 
                };

                // ---- HIZLI VE KESİN ANALİTİK ÇÖZÜCÜ (SPHERE vs AABB) ----
                // O(1) matematik, GJK'nın devasa ölçekli düzlemsel objelerde girdiği sonsuz döngüyü önler.
                use crate::shape::ColliderShape;
                if let (ColliderShape::Sphere(s), ColliderShape::Aabb(a)) = (&col_a.shape, &col_b.shape) {
                    is_fast_path = true;
                    // AABB rotasyon desteklemiyor varsayımıyla lokal koordinatlar
                    let min_box = pos_b - a.half_extents;
                    let max_box = pos_b + a.half_extents;
                    
                    // Sphere'nin AABB üzerindeki en yakın noktası
                    let closest_x = pos_a.x.clamp(min_box.x, max_box.x);
                    let closest_y = pos_a.y.clamp(min_box.y, max_box.y);
                    let closest_z = pos_a.z.clamp(min_box.z, max_box.z);
                    let closest_point = Vec3::new(closest_x, closest_y, closest_z);
                    
                    let diff = closest_point - pos_a; // A'dan B'ye doğru vektör (B'nin noktasından A'nın merkezini çıkar)
                    let dist_sq = diff.length_squared();
                    
                    if dist_sq < s.radius * s.radius {
                        let dist = dist_sq.sqrt();
                        manifold.is_colliding = true;
                        if dist > 0.0001 {
                            // A (Sphere), B (AABB). Normal A'dan B'ye bakmalı.
                            manifold.normal = diff / dist; 
                            manifold.penetration = s.radius - dist;
                        } else {
                            // İç içelerse B'yi (AABB) nereye doğru iteceğiz? Kürenin altındaysa aşağı vs. Varsayılan (0, -1, 0)
                            manifold.normal = Vec3::new(0.0, -1.0, 0.0); 
                            manifold.penetration = s.radius;
                        }
                        manifold.contact_points.push(closest_point);
                    }
                } else if let (ColliderShape::Aabb(a), ColliderShape::Sphere(s)) = (&col_a.shape, &col_b.shape) {
                    is_fast_path = true;
                    let min_box = pos_a - a.half_extents;
                    let max_box = pos_a + a.half_extents;
                    
                    let closest_x = pos_b.x.clamp(min_box.x, max_box.x);
                    let closest_y = pos_b.y.clamp(min_box.y, max_box.y);
                    let closest_z = pos_b.z.clamp(min_box.z, max_box.z);
                    let closest_point = Vec3::new(closest_x, closest_y, closest_z);
                    
                    let diff = pos_b - closest_point; // A'nın noktasından B'nin (Kürenin) merkezine (A'dan B'ye vektör)
                    let dist_sq = diff.length_squared();
                    
                    if dist_sq < s.radius * s.radius {
                        let dist = dist_sq.sqrt();
                        manifold.is_colliding = true;
                        if dist > 0.0001 {
                            // A (AABB), B (Sphere). Normal A'dan B'ye bakmalı. (Örn: A zeminse, B yukarıdaysa normal UP olmalı)
                            manifold.normal = diff / dist; 
                            manifold.penetration = s.radius - dist;
                        } else {
                            manifold.normal = Vec3::new(0.0, 1.0, 0.0); // Zemin içindeyse havaya it
                            manifold.penetration = s.radius;
                        }
                        manifold.contact_points.push(closest_point);
                    }
                } else if let (ColliderShape::Aabb(a1), ColliderShape::Aabb(a2)) = (&col_a.shape, &col_b.shape) {
                    is_fast_path = true;
                    manifold = crate::collision::check_aabb_aabb_manifold(pos_a, a1, pos_b, a2);
                }
                // ---- KAPSÜL ANALİTİK ÇÖZÜCÜLER ----
                else if let (ColliderShape::Capsule(c1), ColliderShape::Capsule(c2)) = (&col_a.shape, &col_b.shape) {
                    is_fast_path = true;
                    manifold = crate::collision::check_capsule_capsule_manifold(pos_a, rot_a, c1, pos_b, rot_b, c2);
                } else if let (ColliderShape::Capsule(c), ColliderShape::Sphere(s)) = (&col_a.shape, &col_b.shape) {
                    is_fast_path = true;
                    manifold = crate::collision::check_capsule_sphere_manifold(pos_a, rot_a, c, pos_b, s);
                } else if let (ColliderShape::Sphere(s), ColliderShape::Capsule(c)) = (&col_a.shape, &col_b.shape) {
                    is_fast_path = true;
                    manifold = crate::collision::check_capsule_sphere_manifold(pos_b, rot_b, c, pos_a, s);
                    manifold.normal = manifold.normal * -1.0; // Normal yönünü ters çevir (A→B)
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
                
                if manifold.is_colliding {
                    entities_to_wake.push(ent_a);
                    entities_to_wake.push(ent_b);
                }

                // Eğer objeler birbirine geçiyorsa:
                if manifold.is_colliding && !manifold.contact_points.is_empty() {
                    // Çarpışma tespit edildi
                    let point_count = manifold.contact_points.len() as f32;
                    
                    // -- 1. POZİSYON DÜZELTMESİ (Positional Correction) Sadece 1 kere uygulanır --
                    let inv_mass_a = if rb_a.mass == 0.0 { 0.0 } else { 1.0 / rb_a.mass };
                    let inv_mass_b = if rb_b.mass == 0.0 { 0.0 } else { 1.0 / rb_b.mass };
                    let sum_inv_mass_pos = inv_mass_a + inv_mass_b;

                    if sum_inv_mass_pos > 0.0 {
                        // Iterasyonlar arası patlamayı engellemek için kuvveti yumuşatıyoruz (%15)
                        // Kule ne kadar yüksekse (100 kutu vs) percent o kadar düşük olmalı ki titreşmesin (Jitter)
                        let percent = 0.15; 
                        let slop = 0.015;  // Titreşimleri yutacak minimal pay
                        let correction = (manifold.penetration - slop).max(0.0) / sum_inv_mass_pos * percent;
                        let correction_vec = manifold.normal * correction;

                        if let Some(t_a) = transforms.get_mut(ent_a) {
                            t_a.position -= correction_vec * inv_mass_a;
                        }
                        if let Some(t_b) = transforms.get_mut(ent_b) {
                            t_b.position += correction_vec * inv_mass_b;
                        }
                    }

                    // -- 2. KATI CİSİM MOMENTUM, TORK VE İTME (Her Temas Noktası İçin) --
                    for contact_point in &manifold.contact_points {
                        let r_a = *contact_point - pos_a;
                        let r_b = *contact_point - pos_b;

                        let vel_a = velocities.get(ent_a).cloned().unwrap_or(Velocity::new(Vec3::ZERO));
                        let vel_b = velocities.get(ent_b).cloned().unwrap_or(Velocity::new(Vec3::ZERO));

                        let v_point_a = vel_a.linear + vel_a.angular.cross(r_a);
                        let v_point_b = vel_b.linear + vel_b.angular.cross(r_b);

                        let relative_vel = v_point_b - v_point_a;
                        let vel_along_normal = relative_vel.dot(manifold.normal);

                        if vel_along_normal > 0.0 { continue; }

                        // Çarpışan iki cisimden Zıplama kapasitesi YOKSEK olanın değerini kullan.
                        let mut e = rb_a.restitution.max(rb_b.restitution);
                        // Jitterı Önle: Hız yer çekimi ivmesinden kaynaklı ufak bir düşüş hızından ibaretse sekmeyi yoksay
                        // 1.0 limiti zıplamayı kısıtlıyordu, sonsuz top testi için daha hassas yaptık (0.2)
                        if vel_along_normal.abs() < 0.2 { 
                            e = 0.0;
                        }

                        // Eylemsizlik Temsiline (Inertia) Göre Açısal Etki Hesabı
                        let ra_cross_n = r_a.cross(manifold.normal);
                        let rb_cross_n = r_b.cross(manifold.normal);

                        let inv_inertia_a_vec = rb_a.inverse_inertia;
                        let inv_t_a = Vec3::new(ra_cross_n.x * inv_inertia_a_vec.x, ra_cross_n.y * inv_inertia_a_vec.y, ra_cross_n.z * inv_inertia_a_vec.z);
                        let angular_effect_a = inv_t_a.cross(r_a).dot(manifold.normal);

                        let inv_inertia_b_vec = rb_b.inverse_inertia;
                        let inv_t_b = Vec3::new(rb_cross_n.x * inv_inertia_b_vec.x, rb_cross_n.y * inv_inertia_b_vec.y, rb_cross_n.z * inv_inertia_b_vec.z);
                        let angular_effect_b = inv_t_b.cross(r_b).dot(manifold.normal);

                        let sum_inv_mass_impulse = inv_mass_a + inv_mass_b + angular_effect_a + angular_effect_b;
                        if sum_inv_mass_impulse == 0.0 { continue; }

                        let j = (-(1.0 + e) * vel_along_normal / sum_inv_mass_impulse) / point_count;
                        let impulse = manifold.normal * j;

                        // Hızlara ve Açısal Hızlara (Angular Velocity) Yansıtma
                        if let Some(v_a) = velocities.get_mut(ent_a) {
                            v_a.linear -= impulse * inv_mass_a;
                            let t_a = r_a.cross(impulse * -1.0); 
                            v_a.angular += Vec3::new(t_a.x * inv_inertia_a_vec.x, t_a.y * inv_inertia_a_vec.y, t_a.z * inv_inertia_a_vec.z);
                        }

                        if let Some(v_b) = velocities.get_mut(ent_b) {
                            v_b.linear += impulse * inv_mass_b;
                            let t_b = r_b.cross(impulse);
                            v_b.angular += Vec3::new(t_b.x * inv_inertia_b_vec.x, t_b.y * inv_inertia_b_vec.y, t_b.z * inv_inertia_b_vec.z);
                        }

                        // -- 3. COULOMB SÜRTÜNME MODELİ (Tangential Friction Impulse) --
                        let tangent_vel = relative_vel - manifold.normal * vel_along_normal;
                        let tangent_speed = tangent_vel.length();

                        if tangent_speed > 0.001 {
                            let tangent_dir = tangent_vel / tangent_speed; // Normalize

                            let mu_static = (rb_a.friction + rb_b.friction) * 0.5;
                            let mu_kinetic = mu_static * 0.7; 

                            let jt = (-tangent_speed / sum_inv_mass_impulse) / point_count;

                            let friction_impulse = if jt.abs() < j.abs() * mu_static {
                                tangent_dir * jt
                            } else {
                                tangent_dir * (-j.abs() * mu_kinetic)
                            };

                            if let Some(v_a) = velocities.get_mut(ent_a) {
                                v_a.linear -= friction_impulse * inv_mass_a;
                                let ft_a = r_a.cross(friction_impulse * -1.0);
                                v_a.angular += Vec3::new(ft_a.x * inv_inertia_a_vec.x, ft_a.y * inv_inertia_a_vec.y, ft_a.z * inv_inertia_a_vec.z);
                            }
                            if let Some(v_b) = velocities.get_mut(ent_b) {
                                v_b.linear += friction_impulse * inv_mass_b;
                                let ft_b = r_b.cross(friction_impulse);
                                v_b.angular += Vec3::new(ft_b.x * inv_inertia_b_vec.x, ft_b.y * inv_inertia_b_vec.y, ft_b.z * inv_inertia_b_vec.z);
                            } // closes if let v_b
                        } // closes tangent_speed
                    } // closes contact_points
                } // closes manifold.is_colliding
            } // closes colliders
        } // closes for collision_pairs
    } // closes iter loop

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
