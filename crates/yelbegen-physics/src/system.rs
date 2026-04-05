use yelbegen_core::World;
use crate::components::{Transform, Velocity, RigidBody};
use crate::shape::Collider;
use yelbegen_math::{Vec3, Quat};
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

            for wheel in &mut vehicle.wheels {
                // Tekerleğin gövdeye bağlanma ofseti (Rotasyona göre global offset)
                let r_ws = t.rotation.mul_vec3(wheel.connection_point);
                // Tekerleğin süspansiyon başlangıç noktası (Gövde üzerinde lokal tavanı)
                let origin = t.position + r_ws;
                // Aşağı doğru yön
                let dir = t.rotation.mul_vec3(wheel.direction).normalize();

                // Basit Y=0.0 düzlem ışın kesişimi (Sadece devasa düz zemin için optimize edilmiştir)
                // P.y = origin.y + t * dir.y = 0.0 (veya tekerlek yarıçapı kadar üstü)
                // Zemini y=0 sayalım. Zeminle temas tekerleğin alt noktasından olmalı.
                let target_y = 0.0;
                
                // Eğer dir.y dümdüz değilse bile bir hesaplama yapılır.
                if dir.y.abs() > 0.001 {
                    // Tekerlek yarıçapı çıkarılmış zemin teması (Tekerlek çapı kadar havada kalmalı)
                    let hit_t = (target_y + wheel.wheel_radius - origin.y) / dir.y;
                    
                    if hit_t > 0.0 && hit_t < wheel.suspension_rest_length {
                        wheel.is_grounded = true;
                        wheel.compression = wheel.suspension_rest_length - hit_t;
                        
                        // Hooke Yasası: F = k * x (Yay Sıkışma Kuvveti)
                        let force = wheel.suspension_stiffness * wheel.compression;
                        
                        // Sönümleme (Damping)
                        let wheel_vel = v.linear + v.angular.cross(r_ws);
                        let vel_along_dir = wheel_vel.dot(dir);
                        let damping_force = -wheel.suspension_damping * vel_along_dir;
                        
                        let total_suspension_force = (force + damping_force).max(0.0); // Çekme yapamaz, sadece itebilir
                        
                        let impulse_vec = dir * -total_suspension_force * dt;
                        
                        total_linear_impulse += impulse_vec;
                        // Tork (Angular Impulse)
                        let torque = r_ws.cross(impulse_vec);
                        total_angular_impulse += Vec3::new(torque.x * inv_inertia.x, torque.y * inv_inertia.y, torque.z * inv_inertia.z);
                    } else {
                        wheel.is_grounded = false;
                        wheel.compression = 0.0;
                    }
                }
            }

            v.linear += total_linear_impulse * inv_mass;
            v.angular += total_angular_impulse;
        }
    }
}

// Varlıkların fiziksel hareketlerini, yerçekimi ve sürtünme etkileriyle uygulayan sistem
pub fn physics_movement_system(world: &World, dt: f32) {
    if let (Some(mut trans_storage), Some(mut vel_storage), Some(mut rbs)) = (world.borrow_mut::<Transform>(), world.borrow_mut::<Velocity>(), world.borrow_mut::<RigidBody>()) {
        let entities = trans_storage.entity_dense.clone();
        for entity in entities {
            let rb = match rbs.get_mut(entity) { Some(r) => r, None => continue };
            let v = match vel_storage.get_mut(entity) { Some(v) => v, None => continue };
            let t = match trans_storage.get_mut(entity) { Some(t) => t, None => continue };

            if rb.mass > 0.0 {
                let speed_sq = v.linear.length_squared() + v.angular.length_squared();
                if speed_sq < 0.005 { // Hız uykuda sayılabilecek kadar düşük mü?
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
    let mut transforms = match world.borrow_mut::<Transform>() { Some(t) => t, None => { println!("ERROR: Transforms yok!"); return; } };
    let mut velocities = match world.borrow_mut::<Velocity>() { Some(v) => v, None => { println!("ERROR: Velocities yok!"); return; } };
    let colliders = match world.borrow::<Collider>() { Some(c) => c, None => { println!("ERROR: Colliders yok!"); return; } };
    let rigidbodies = match world.borrow::<RigidBody>() { Some(r) => r, None => { println!("ERROR: Rigidbodies yok!"); return; } };

    // 1. BROAD-PHASE: Sweep and Prune (1D X-Ekseni)
    struct Interval {
        entity: u32,
        min_x: f32,
        max_x: f32,
    }
    let entities = transforms.entity_dense.clone();

    let mut intervals = Vec::with_capacity(entities.len());
    for &e in &entities {
        let t = match transforms.get(e) { Some(t) => t, None => continue };
        let col = match colliders.get(e) { Some(c) => c, None => continue };

        use crate::shape::ColliderShape;
        let (min_x, max_x) = match &col.shape {
            ColliderShape::Aabb(a) => {
                let half_x = a.half_extents.x * t.scale.x;
                (t.position.x - half_x, t.position.x + half_x)
            },
            ColliderShape::Sphere(s) => {
                let r = s.radius * t.scale.x.max(t.scale.y).max(t.scale.z);
                (t.position.x - r, t.position.x + r)
            }
        };
        intervals.push(Interval { entity: e, min_x, max_x });
        println!("INTERVAL: Ent={} Min={} Max={}", e, min_x, max_x);
    }
    use rayon::prelude::*;
    intervals.sort_by(|a, b| a.min_x.partial_cmp(&b.min_x).unwrap_or(std::cmp::Ordering::Equal));

    let mut collision_pairs = Vec::new();
    for i in 0..intervals.len() {
        let a = &intervals[i];
        for j in (i + 1)..intervals.len() {
            let b = &intervals[j];
            if b.min_x > a.max_x {
                break; // PRUNE! Geri kalan hiçbirinin a objesiyle çarpışma ihtimali yok. (O(n^2) engellendi)
            }
            collision_pairs.push((a.entity, b.entity));
        }
    }

    // 2. NARROW-PHASE: GJK/EPA ile gerçek kesişim testi (Sadece filtreden geçen çiftler)
    for (ent_a, ent_b) in collision_pairs {
        println!("Kesisme Testi => Ent {} ve {}", ent_a, ent_b);
            let (rb_a, rb_b) = match (rigidbodies.get(ent_a), rigidbodies.get(ent_b)) {
                (Some(a), Some(b)) => (a, b),
                _ => continue, // Rigidbody'si olmayan çarpışıp güç aktaramaz
            };

            // İkisinin de kütlesi yoksa veya İKİSİ DE UYUYORSA çarpışma çözümüne gerek yok
            if (rb_a.mass == 0.0 && rb_b.mass == 0.0) || (rb_a.is_sleeping && rb_b.is_sleeping) { 
                continue; 
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
                    println!("CARPISMA OLDU! GUC: {}", manifold.penetration);
                    let point_count = manifold.contact_points.len() as f32;
                    
                    // -- 1. POZİSYON DÜZELTMESİ (Positional Correction) Sadece 1 kere uygulanır --
                    let inv_mass_a = if rb_a.mass == 0.0 { 0.0 } else { 1.0 / rb_a.mass };
                    let inv_mass_b = if rb_b.mass == 0.0 { 0.0 } else { 1.0 / rb_b.mass };
                    let sum_inv_mass_pos = inv_mass_a + inv_mass_b;

                    if sum_inv_mass_pos > 0.0 {
                        let percent = 0.4; // Yüzde kaç kadar düzelt (-yılanlama önleyici)
                        let slop = 0.01;   // İzin verilen sapma payı (ufak titreşimleri yutar)
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

                        let mut e = rb_a.restitution.min(rb_b.restitution);
                        // Jitterı Önle: Hız yer çekimi ivmesinden kaynaklı ufak bir düşüş hızından ibaretse sekmeyi yoksay
                        if vel_along_normal.abs() < 1.0 { 
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
