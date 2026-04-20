use crate::components::Transform;
use crate::shape::{Aabb, Capsule, Collider, ColliderShape, Sphere};
use gizmo_math::{Quat, Vec3};

/// Kinematic Character Controller
/// Fizik simülasyonuna TAM tabi olmadan çarpışma yapan özel bileşen.
/// Unity'deki CharacterController'a eşdeğer.
///
/// Çalışma prensibi:
/// 1. Oyuncu input'u → `desired_velocity` olarak ayarlanır
/// 2. `physics_character_system` çağrılır
/// 3. Sistem, capsule sweep ile hareketi dener
/// 4. Çarpışma varsa "slide" (kayma) vektörü hesaplar
/// 5. Basamak çıkma ve eğim kontrolü uygular
#[derive(Debug, Clone)]
pub struct CharacterController {
    /// Kapsül yarıçapı
    pub radius: f32,
    /// Kapsül yarı-yüksekliği (toplam yükseklik = 2*(half_height + radius))
    pub half_height: f32,
    /// Maksimum tırmanılabilir basamak yüksekliği (metre)
    pub step_height: f32,
    /// Maksimum tırmanılabilir eğim (derece)
    pub slope_limit: f32,
    /// Çarpışma zırhı kalınlığı (objelere bu kadar mesafede durur)
    pub skin_width: f32,
    /// Yerde mi?
    pub is_grounded: bool,
    /// Zemin normali
    pub ground_normal: Vec3,
    /// Yerçekimi hızı birikimi (düşme sırasında artar)
    pub vertical_velocity: f32,
    /// İstenen hareket yönü (her frame oyuncu input'undan ayarlanır)
    pub desired_velocity: Vec3,
}

impl CharacterController {
    pub fn new(radius: f32, half_height: f32) -> Self {
        Self {
            radius,
            half_height,
            step_height: 0.3,
            slope_limit: 45.0,
            skin_width: 0.08, // Daha kalın bir güvenlik tamponu (Unity Standardı) titremeyi önler
            is_grounded: false,
            ground_normal: Vec3::new(0.0, 1.0, 0.0),
            vertical_velocity: 0.0,
            desired_velocity: Vec3::ZERO,
        }
    }

    /// Zıplama impulse'u uygular (is_grounded ise)
    pub fn jump(&mut self, force: f32) {
        if self.is_grounded {
            self.vertical_velocity = force;
            self.is_grounded = false;
        }
    }
}

// ======================== ÇARPIŞMA SORGULARI ========================

/// Capsule vs AABB penetrasyon testi (world space)
/// true dönerse `normal` ve `depth` ile ne kadar gömüldüğünü verir
fn capsule_vs_aabb(
    cap_pos: Vec3,
    cap_rot: Quat,
    cap: &Capsule,
    box_pos: Vec3,
    aabb: &Aabb,
) -> Option<(Vec3, f32)> {
    let manifold =
        crate::collision::check_capsule_aabb_manifold(cap_pos, cap_rot, cap, box_pos, aabb);
    if manifold.is_colliding {
        Some((manifold.normal, manifold.penetration))
    } else {
        None
    }
}

/// Capsule vs Sphere penetrasyon testi
fn capsule_vs_sphere(
    cap_pos: Vec3,
    cap_rot: Quat,
    cap: &Capsule,
    sphere_pos: Vec3,
    sphere: &Sphere,
) -> Option<(Vec3, f32)> {
    let manifold =
        crate::collision::check_capsule_sphere_manifold(cap_pos, cap_rot, cap, sphere_pos, sphere);
    if manifold.is_colliding {
        Some((manifold.normal, manifold.penetration))
    } else {
        None
    }
}

/// Capsule vs Capsule penetrasyon testi
fn capsule_vs_capsule(
    cap_a_pos: Vec3,
    cap_a_rot: Quat,
    cap_a: &Capsule,
    cap_b_pos: Vec3,
    cap_b_rot: Quat,
    cap_b: &Capsule,
) -> Option<(Vec3, f32)> {
    let manifold = crate::collision::check_capsule_capsule_manifold(
        cap_a_pos, cap_a_rot, cap_a, cap_b_pos, cap_b_rot, cap_b,
    );
    if manifold.is_colliding {
        Some((manifold.normal, manifold.penetration))
    } else {
        None
    }
}

/// Karakter kapsülünü tüm collider'lara karşı test eder ve toplam itme vektörü döndürür
fn resolve_capsule_collisions<'a>(
    cap_pos: Vec3,
    cap_rot: Quat,
    cap: &Capsule,
    entity_id: u32,
    nearby_colliders: &[(u32, Transform, &'a Collider)],
    limit_cos: f32,
) -> (Vec3, Vec3, bool) {
    // Döndürür: (toplam_pozisyon_düzeltme, zemin_normali, yerde_mi)
    let mut correction = Vec3::ZERO;
    let mut ground_normal = Vec3::new(0.0, 1.0, 0.0);
    let mut is_grounded = false;

    for &(other_id, other_t, other_col) in nearby_colliders {
        if other_id == entity_id {
            continue;
        }

        // ================= BROAD-PHASE (KÜRESEL ÖN TEST) =================
        // Dar-faz (Narrow-phase) GJK/EPA veya benzeri detaylı hesaplara girmeden önce,
        // karakter ile obje arasındaki kaba küresel mesafeyi buluruz.
        // (Buradaki sabit 10000.0 eşiği çok geniş araziler/objeler için hataya yol açtığından kaldırıldı,
        // dinamik hesaplanan radius tabanlı check aşağıda yeterli görevi yapıyor.)

        let other_ext = other_col.shape.bounding_box_half_extents(other_t.rotation);
        let other_radius = other_ext.length();
        let cap_radius = cap.half_height + cap.radius;
        let max_dist = other_radius + cap_radius + 0.5; // 0.5m güvenlik payı
        if (cap_pos - other_t.position).length_squared() > max_dist * max_dist {
            continue; // Kapsül ve Obje birbirinden güvenle uzakta, atla!
        }

        let hit = match &other_col.shape {
            ColliderShape::Aabb(aabb) => {
                capsule_vs_aabb(cap_pos, cap_rot, cap, other_t.position, aabb)
            }
            ColliderShape::Sphere(sphere) => {
                capsule_vs_sphere(cap_pos, cap_rot, cap, other_t.position, sphere)
            }
            ColliderShape::Capsule(other_cap) => capsule_vs_capsule(
                cap_pos,
                cap_rot,
                cap,
                other_t.position,
                other_t.rotation,
                other_cap,
            ),
            ColliderShape::ConvexHull(_) => {
                let test_pos = cap_pos;
                let cap_shape = ColliderShape::Capsule(*cap);
                let (hit, sim) = crate::gjk::gjk_intersect(
                    &cap_shape,
                    test_pos,
                    cap_rot,
                    &other_col.shape,
                    other_t.position,
                    other_t.rotation,
                );
                if hit {
                    let manifold = crate::epa::epa_solve(
                        sim,
                        &cap_shape,
                        test_pos,
                        cap_rot,
                        &other_col.shape,
                        other_t.position,
                        other_t.rotation,
                    );
                    if manifold.is_colliding {
                        Some((manifold.normal, manifold.penetration))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            ColliderShape::Swept { .. } => {
                None // Swept shape should never be in ECS
            }
            ColliderShape::HeightField {
                heights,
                segments_x,
                segments_z,
                width,
                depth,
                max_height,
            } => {
                let test_pos = cap_pos;
                let position = other_t.position;
                let half_w = *width * 0.5;
                let half_d = *depth * 0.5;

                let bottom_y = test_pos.y - cap.half_height - cap.radius;
                let mut best_hit: Option<(gizmo_math::Vec3, f32)> = None;

                let offsets = [
                    (0.0, 0.0),
                    (cap.radius, 0.0),
                    (-cap.radius, 0.0),
                    (0.0, cap.radius),
                    (0.0, -cap.radius),
                ];

                for (ox, oz) in offsets.iter() {
                    let local_x = (test_pos.x + ox) - position.x;
                    let local_z = (test_pos.z + oz) - position.z;

                    if local_x >= -half_w && local_x <= half_w && local_z >= -half_d && local_z <= half_d {
                        let nx = (local_x + half_w) / *width;
                        let nz = (local_z + half_d) / *depth;
                        let sx = *segments_x as f32 - 1.0;
                        let sz_ = *segments_z as f32 - 1.0;
                        let fx = (nx * sx).max(0.0);
                        let fz = (nz * sz_).max(0.0);

                        let gx0 = (fx as u32).clamp(0, *segments_x - 2);
                        let gz0 = (fz as u32).clamp(0, *segments_z - 2);
                        let gx1 = gx0 + 1;
                        let gz1 = gz0 + 1;

                        let h = |gx: u32, gz: u32| -> f32 {
                            let idx = (gz * *segments_x + gx) as usize;
                            if idx < heights.len() { heights[idx] * *max_height } else { 0.0 }
                        };

                        let h00 = h(gx0, gz0);
                        let h10 = h(gx1, gz0);
                        let h01 = h(gx0, gz1);
                        let h11 = h(gx1, gz1);

                        let tx = fx - gx0 as f32;
                        let tz = fz - gz0 as f32;

                        let terrain_local_y = h00 * (1.0 - tx) * (1.0 - tz)
                            + h10 * tx * (1.0 - tz)
                            + h01 * (1.0 - tx) * tz
                            + h11 * tx * tz;

                        let terrain_y = position.y + terrain_local_y;

                        if bottom_y < terrain_y {
                            let raw_dx = (h10 - h00) * (1.0 - tz) + (h11 - h01) * tz;
                            let raw_dz = (h01 - h00) * (1.0 - tx) + (h11 - h10) * tx;

                            let cell_w = *width / (*segments_x as f32);
                            let cell_d = *depth / (*segments_z as f32);

                            let norm = gizmo_math::Vec3::new(-raw_dx / cell_w, 1.0, -raw_dz / cell_d).normalize();
                            let depth = terrain_y - bottom_y;

                            if let Some((_, max_depth)) = best_hit {
                                if depth > max_depth {
                                    best_hit = Some((norm, depth));
                                }
                            } else {
                                best_hit = Some((norm, depth));
                            }
                        }
                    }
                }
                best_hit
            }
        };

        if let Some((normal, depth)) = hit {
            // Kapsülü collider'dan dışarı it (+normal * depth)
            correction += normal * depth;

            // Zemin tespiti: yüzey normal açısı slope limitten küçük mü (zemin mi?)
            let n_y = normal.y;
            if n_y < -limit_cos {
                is_grounded = true;
                ground_normal += normal * -1.0;
            } else if n_y > limit_cos {
                is_grounded = true;
                ground_normal += normal;
            }
        }
    }

    if is_grounded && ground_normal.length_squared() > 0.001 {
        // Fix #22: Çoklu objelerden gelen normalleri toplayıp ortalamasını (bileşkeyi) alıyoruz
        ground_normal = ground_normal.normalize();
    } else {
        ground_normal = Vec3::new(0.0, 1.0, 0.0);
    }

    (correction, ground_normal, is_grounded)
}



// ======================== FİZİK SİSTEMİ ========================

/// Karakter Kontrolcüsü Fizik Sistemi
/// Move & Slide mekanizması:
/// 1. Yerçekimi uygula
/// 2. İstenen hareketi hesapla
/// 3. Capsule sweep ile çarpışma kontrol
/// 4. Slide vektörü hesapla (çarpışma normalinden hız bileşenini çıkar)
/// 5. Basamak çıkma kontrolü
/// 6. Eğim limiti kontrolü
pub fn physics_character_system(world: &gizmo_core::World, dt: f32) {
    // Collider'ları ayrıca borrow'lamak lazım
    let colliders = match world.borrow::<Collider>().unwrap_or(None) {
        Some(c) => c,
        None => return,
    };

    if let (Some(mut trans_storage), Some(mut controllers)) = (
        world.borrow_mut::<Transform>().unwrap_or(None),
        world.borrow_mut::<CharacterController>().unwrap_or(None),
    ) {
        let entities: Vec<u32> = controllers.iter().map(|(id, _)| id).collect();
        for entity in entities {
            let t = match trans_storage.get(entity) {
                Some(t) => *t,
                None => continue,
            };
            let cc = match controllers.get_mut(entity) {
                Some(c) => c,
                None => continue,
            };

            let cap = Capsule {
                radius: cc.radius,
                half_height: cc.half_height,
            };
            
            // ================= KCC KİŞİSEL BROAD-PHASE =================
            let mut nearby_colliders = Vec::new();
            let sweep_radius = cap.half_height + cap.radius + cc.desired_velocity.length() * dt + 5.0; 
            let broad_radius_sq = sweep_radius * sweep_radius;
            
            for (other_id, other_col) in colliders.iter() {
                if other_id == entity { continue; }
                if let Some(other_t) = trans_storage.get(other_id) {
                    if (t.position - other_t.position).length_squared() < broad_radius_sq {
                        nearby_colliders.push((other_id, *other_t, other_col));
                    }
                }
            }

            // === 1. Yerçekimi ===
            if !cc.is_grounded {
                cc.vertical_velocity -= 9.81 * dt;
                // Terminal hız limiti
                cc.vertical_velocity = cc.vertical_velocity.max(-50.0);
            } else {
                // Hafif aşağı kuvvet (yere yapışmak için, eğimden kayma amaçlı)
                if cc.vertical_velocity <= 0.0 {
                    cc.vertical_velocity = -9.81 * dt * 10.0;
                }
            }

            // === 2. Toplam hareketi hesapla ===
            let mut move_delta = cc.desired_velocity * dt;
            move_delta.y += cc.vertical_velocity * dt;

            // === 3. Hareketi uygula ve çarpışma çöz (3 iterasyon slide) ===
            let mut new_pos = t.position + move_delta;
            let mut current_delta = move_delta;

            let limit_cos = cc.slope_limit.to_radians().cos();

            for _slide_iter in 0..3 {
                let (correction, normal, grounded) = resolve_capsule_collisions(
                    new_pos,
                    t.rotation,
                    &cap,
                    entity,
                    &nearby_colliders,
                    limit_cos,
                );

                if correction.length_squared() < 0.00001 {
                    // Çarpışma yok, hareketi kabul et
                    break;
                }

                new_pos += correction;
                cc.is_grounded = grounded;
                cc.ground_normal = normal;

                // Slide: Kalan hızdan çarpışma normal bileşenini çıkar
                // normal, collider'dan dışarı (karaktere doğru) yönlü.
                // remaining'in normal üzerine projeksiyonu negatifse (collider'a doğru gidiyor),
                // o bileşeni çıkarıyoruz. Pozitifse (zaten uzaklaşıyor) dokunmuyoruz.
                let remaining = current_delta;
                let dot = remaining.dot(normal);
                if dot < 0.0 {
                    // Sadece collider'a doğru olan bileşeni çıkar
                    let normal_component = normal * dot;
                    let slide = remaining - normal_component;
                    // Önceki iterasyonların correction'ını kaybetmemek için güncel konumdan eski deltayı düşüp yenisini (slide) ekliyoruz
                    new_pos = new_pos - current_delta + slide;
                    current_delta = slide;
                }
            }

            // === 4. Zemin düzlemi kontrolü (fallback — collider yoksa) ===
            let ground_y = world
                .get_resource::<crate::components::PhysicsConfig>().expect("ECS Aliasing Error")
                .map(|c| c.ground_y)
                .unwrap_or(-1.0);
            let foot_y = new_pos.y - cc.half_height - cc.radius;

            if foot_y <= ground_y + cc.skin_width + 0.05 {
                let target_y = ground_y + cc.half_height + cc.radius + cc.skin_width;
                
                // Gerçekten zemini aşıp alta indiyse is_grounded = true ver
                if new_pos.y < target_y {
                    new_pos.y = target_y;
                    if cc.vertical_velocity < 0.0 {
                        cc.vertical_velocity = 0.0;
                    }
                    cc.is_grounded = true;
                    cc.ground_normal = Vec3::new(0.0, 1.0, 0.0);
                }
            }

            // === 5. Eğim kontrolü ===
            if cc.is_grounded {
                // Trigonometrik arc-cosine kullanmadan doğrudan cosinüs değerleri üzerinden test et:
                // normal.y = cos(açı). Eğer cos(açı) < limit_cos ise, açı slope_limit'ten BÜYÜKTÜR (aşılmıştır).
                if cc.ground_normal.y < limit_cos {
                    // Eğim çok dik — kayma uygula
                    let slide_dir = Vec3::new(cc.ground_normal.x, 0.0, cc.ground_normal.z);
                    if slide_dir.length_squared() > 0.001 {
                        // Eğimin yatay bileşeni ne kadar uzun olursa o kadar fazla kaydırır
                        let slide_force = slide_dir.normalize() * 9.81 * (1.0 - cc.ground_normal.y * cc.ground_normal.y).sqrt() * dt;
                        new_pos += slide_force;
                    }
                    cc.is_grounded = false; // Kontrol kaybı
                }
            }

            // === 6. Basamak çıkma (Step Climbing) ===
            // Yatay hareket varsa ve bir engelle karşılaşıldıysa:
            // step_height kadar yukarıdan tekrar dene
            if cc.is_grounded && cc.desired_velocity.length_squared() > 0.01 {
                let horizontal_move = Vec3::new(move_delta.x, 0.0, move_delta.z);
                if horizontal_move.length_squared() > 0.0001 {
                    let step_test_pos = Vec3::new(
                        new_pos.x + horizontal_move.x,
                        new_pos.y + cc.step_height,
                        new_pos.z + horizontal_move.z,
                    );

                    let (step_correction, _, _) = resolve_capsule_collisions(
                        step_test_pos,
                        t.rotation,
                        &cap,
                        entity,
                        &nearby_colliders,
                        limit_cos,
                    );

                    // Yukarıdan geçebildiyse (çarpışma düzeltmesi küçükse) → basamak çık
                    if step_correction.length_squared() < 0.01 {
                        // step_test_pos'tan aşağı doğru sanal capsule-sweep (zemin arama)
                        let mut best_y = step_test_pos.y;
                        let sweeps = 10;
                        let dy = cc.step_height / (sweeps as f32);
                        
                        for i in 1..=sweeps {
                            let test_y = step_test_pos.y - (i as f32) * dy;
                            let test_pos = Vec3::new(step_test_pos.x, test_y, step_test_pos.z);
                            let (down_corr, _, is_g) = resolve_capsule_collisions(
                                test_pos, t.rotation, &cap, entity, &nearby_colliders, limit_cos
                            );
                            
                            if down_corr.length_squared() > 0.0001 {
                                // Bulunan zemin y'si doğrudan kapsülümüzü basamağa oturttuğumuz ideal nokta
                                best_y = test_y + down_corr.y.max(0.0);
                                if is_g || down_corr.y > 0.01 {
                                    cc.is_grounded = true;
                                }
                                break;
                            } else {
                                best_y = test_y;
                            }
                        }

                        new_pos.x = step_test_pos.x;
                        new_pos.z = step_test_pos.z;
                        new_pos.y = best_y;
                    }
                }
            }

            // === 7. Son pozisyonu uygula ===
            if let Some(t_mut) = trans_storage.get_mut(entity) {
                t_mut.position = new_pos;
                t_mut.update_local_matrix();
            }

            // desired_velocity'yi sıfırla (her frame yeniden ayarlanmalı)
            // LÜTFEN DİKKAT: Bu durum Input sistemi ile KCC arasında "implicit contract" oluşturur.
            // Karakteri hareket ettiren sistemin HER FRAME cc.desired_velocity'yi ataması gerekir, 
            // aksi takdirde karakter derhal duracaktır.
            cc.desired_velocity = Vec3::ZERO;
        }
    }
}

gizmo_core::impl_component!(CharacterController);
