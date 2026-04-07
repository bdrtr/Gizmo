use gizmo_math::{Vec3, Quat};
use crate::shape::{ColliderShape, Collider, Aabb, Sphere, Capsule};
use crate::components::Transform;

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
            skin_width: 0.01,
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
    cap_pos: Vec3, cap_rot: Quat, cap: &Capsule,
    box_pos: Vec3, aabb: &Aabb,
) -> Option<(Vec3, f32)> {
    let manifold = crate::collision::check_capsule_aabb_manifold(
        cap_pos, cap_rot, cap, box_pos, aabb,
    );
    if manifold.is_colliding {
        Some((manifold.normal, manifold.penetration))
    } else {
        None
    }
}

/// Capsule vs Sphere penetrasyon testi
fn capsule_vs_sphere(
    cap_pos: Vec3, cap_rot: Quat, cap: &Capsule,
    sphere_pos: Vec3, sphere: &Sphere,
) -> Option<(Vec3, f32)> {
    let manifold = crate::collision::check_capsule_sphere_manifold(
        cap_pos, cap_rot, cap, sphere_pos, sphere,
    );
    if manifold.is_colliding {
        Some((manifold.normal, manifold.penetration))
    } else {
        None
    }
}

/// Capsule vs Capsule penetrasyon testi
fn capsule_vs_capsule(
    cap_a_pos: Vec3, cap_a_rot: Quat, cap_a: &Capsule,
    cap_b_pos: Vec3, cap_b_rot: Quat, cap_b: &Capsule,
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
fn resolve_capsule_collisions(
    cap_pos: Vec3, cap_rot: Quat, cap: &Capsule,
    entity_id: u32,
    transforms: &gizmo_core::SparseSet<Transform>,
    colliders: &gizmo_core::SparseSet<Collider>,
) -> (Vec3, Vec3, bool) {
    // Döndürür: (toplam_pozisyon_düzeltme, zemin_normali, yerde_mi)
    let mut correction = Vec3::ZERO;
    let mut ground_normal = Vec3::new(0.0, 1.0, 0.0);
    let mut is_grounded = false;

    for other_entity in colliders.entity_dense.iter() {
        let other_id = *other_entity;
        if other_id == entity_id { continue; }

        let other_t = match transforms.get(other_id) { Some(t) => t, None => continue };
        let other_col = match colliders.get(other_id) { Some(c) => c, None => continue };

        let hit = match &other_col.shape {
            ColliderShape::Aabb(aabb) => {
                capsule_vs_aabb(cap_pos + correction, cap_rot, cap, other_t.position, aabb)
            },
            ColliderShape::Sphere(sphere) => {
                capsule_vs_sphere(cap_pos + correction, cap_rot, cap, other_t.position, sphere)
            },
            ColliderShape::Capsule(other_cap) => {
                capsule_vs_capsule(cap_pos + correction, cap_rot, cap, other_t.position, other_t.rotation, other_cap)
            },
            ColliderShape::ConvexHull(_) => {
                // ConvexHull karakter çarpışması GJK/EPA ile çözülür (ileri adım)
                None
            },
            ColliderShape::Swept { .. } => {
                None  // Swept shape should never be in ECS
            }
        };

        if let Some((normal, depth)) = hit {
            // Kapsülü collider'dan dışarı it
            correction -= normal * depth;

            // Zemin tespiti: normal yukarı bakıyorsa (Y > 0.7 ≈ 45°) yerdeyiz
            if normal.y < -0.7 {
                // Normal A'dan B'ye bakıyor, karakter A olduğu için ters
                is_grounded = true;
                ground_normal = normal * -1.0;
            } else if normal.y > 0.7 {
                is_grounded = true;
                ground_normal = normal;
            }
        }
    }

    (correction, ground_normal, is_grounded)
}

/// Eğim açısını hesaplar (derece cinsinden)
fn slope_angle(normal: Vec3) -> f32 {
    let up = Vec3::new(0.0, 1.0, 0.0);
    let cos_angle = normal.dot(up).abs();
    cos_angle.acos().to_degrees()
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
    let colliders = match world.borrow::<Collider>() { Some(c) => c, None => return };
    
    if let (Some(mut trans_storage), Some(mut controllers)) = 
        (world.borrow_mut::<Transform>(), world.borrow_mut::<CharacterController>())
    {
        let entities = controllers.entity_dense.clone();
        for entity in entities {
            let t = match trans_storage.get(entity) { Some(t) => *t, None => continue };
            let cc = match controllers.get_mut(entity) { Some(c) => c, None => continue };
            
            let cap = Capsule { radius: cc.radius, half_height: cc.half_height };
            
            // === 1. Yerçekimi ===
            if !cc.is_grounded {
                cc.vertical_velocity -= 9.81 * dt;
                // Terminal hız limiti
                cc.vertical_velocity = cc.vertical_velocity.max(-50.0);
            } else {
                // Hafif aşağı kuvvet (yere yapışmak için, eğimden kayma amaçlı)
                cc.vertical_velocity = -0.5;
            }
            
            // === 2. Toplam hareketi hesapla ===
            let mut move_delta = cc.desired_velocity * dt;
            move_delta.y += cc.vertical_velocity * dt;
            
            // === 3. Hareketi uygula ve çarpışma çöz (3 iterasyon slide) ===
            let mut new_pos = t.position + move_delta;
            
            for _slide_iter in 0..3 {
                let (correction, normal, grounded) = resolve_capsule_collisions(
                    new_pos, t.rotation, &cap, entity, &trans_storage, &colliders,
                );
                
                if correction.length_squared() < 0.00001 {
                    // Çarpışma yok, hareketi kabul et
                    break;
                }
                
                new_pos += correction;
                cc.is_grounded = grounded;
                cc.ground_normal = normal;
                
                // Slide: Kalan hızdan çarpışma normal bileşenini çıkar
                let remaining = new_pos - t.position;
                let normal_component = normal * remaining.dot(normal);
                let slide = remaining - normal_component;
                new_pos = t.position + slide;
            }
            
            // === 4. Zemin düzlemi kontrolü (fallback — collider yoksa) ===
            let ground_y = world.get_resource::<crate::components::PhysicsConfig>()
                .map(|c| c.ground_y)
                .unwrap_or(-1.0);
            let foot_y = new_pos.y - cc.half_height - cc.radius;
            
            if foot_y <= ground_y + cc.skin_width + 0.05 {
                cc.is_grounded = true;
                cc.ground_normal = Vec3::new(0.0, 1.0, 0.0);
                
                let target_y = ground_y + cc.half_height + cc.radius + cc.skin_width;
                if new_pos.y < target_y {
                    new_pos.y = target_y;
                }
                
                if cc.vertical_velocity < 0.0 {
                    cc.vertical_velocity = 0.0;
                }
            } else if !cc.is_grounded {
                cc.is_grounded = false;
            }
            
            // === 5. Eğim kontrolü ===
            if cc.is_grounded {
                let angle = slope_angle(cc.ground_normal);
                if angle > cc.slope_limit {
                    // Eğim çok dik — kayma uygula
                    let slide_dir = Vec3::new(cc.ground_normal.x, 0.0, cc.ground_normal.z);
                    if slide_dir.length_squared() > 0.001 {
                        let slide_force = slide_dir.normalize() * 9.81 * angle.to_radians().sin() * dt;
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
                        t.position.x + horizontal_move.x,
                        t.position.y + cc.step_height,
                        t.position.z + horizontal_move.z,
                    );
                    
                    let (step_correction, _, _) = resolve_capsule_collisions(
                        step_test_pos, t.rotation, &cap, entity, &trans_storage, &colliders,
                    );
                    
                    // Yukarıdan geçebildiyse (çarpışma düzeltmesi küçükse) → basamak çık
                    if step_correction.length_squared() < 0.01 {
                        // step_test_pos'tan aşağı doğru yerçekimi ile dengeye otur
                        new_pos.x = step_test_pos.x;
                        new_pos.z = step_test_pos.z;
                        new_pos.y = step_test_pos.y; // Bir sonraki frame'de yerçekimi indirir
                    }
                }
            }
            
            // === 7. Son pozisyonu uygula ===
            if let Some(t_mut) = trans_storage.get_mut(entity) {
                t_mut.position = new_pos;
                t_mut.update_local_matrix();
            }
            
            // desired_velocity'yi sıfırla (her frame yeniden ayarlanmalı)
            cc.desired_velocity = Vec3::ZERO;
        }
    }
}
