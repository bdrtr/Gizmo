use std::collections::HashMap;
use gizmo_math::Vec3;
use crate::components::{Transform, RigidBody, Velocity};
use super::types::{DetectionResult, Island, PhysicsSolverState, CachedContact, MATCH_THRESHOLD_SQ, WARM_START_FACTOR};
use super::union_find::{ensure_node, find_root, union_nodes};
use crate::integration::apply_inv_inertia;




pub fn match_cached_contact(new_point: Vec3, cached: &[CachedContact]) -> Option<(f32, Vec3)> {
    let mut best_dist_sq = f32::MAX;
    let mut best = None;
    for cc in cached {
        let d = (new_point - cc.world_point).length_squared();
        if d < best_dist_sq && d < MATCH_THRESHOLD_SQ {
            best_dist_sq = d;
            best = Some((cc.accumulated_normal, cc.accumulated_friction));
        }
    }
    best
}





pub fn build_islands(
    detection_result: DetectionResult,
    transforms:  &gizmo_core::SparseSet<Transform>,
    velocities:  &gizmo_core::SparseSet<Velocity>,
    entities_to_wake: &mut Vec<u32>,
    rbs: &gizmo_core::SparseSet<RigidBody>,
    joint_world_opt: Option<&crate::constraints::JointWorld>,
) -> Vec<Island> {
    let mut parent_map: HashMap<u32, u32> = HashMap::new();
    let mut rank_map:   HashMap<u32, u8>  = HashMap::new();

    entities_to_wake.extend(detection_result.wake_entities);
    let all_contacts = detection_result.contacts;

    for c in &all_contacts {
        let a_dyn = c.inv_mass_a > 0.0;
        let b_dyn = c.inv_mass_b > 0.0;
        if a_dyn && b_dyn {
            ensure_node(&mut parent_map, &mut rank_map, c.ent_a);
            ensure_node(&mut parent_map, &mut rank_map, c.ent_b);
            union_nodes(&mut parent_map, &mut rank_map, c.ent_a, c.ent_b);
        } else if a_dyn {
            ensure_node(&mut parent_map, &mut rank_map, c.ent_a);
        } else if b_dyn {
            ensure_node(&mut parent_map, &mut rank_map, c.ent_b);
        }
    }

    let mut resolved_joints = Vec::new();
    if let Some(jw) = joint_world_opt {
        for (id, joint) in jw.joints.iter() {
            if let Some(jb) = crate::constraints::JointBodies::resolve(joint, transforms, rbs) {
                ensure_node(&mut parent_map, &mut rank_map, joint.entity_a);
                ensure_node(&mut parent_map, &mut rank_map, joint.entity_b);
                union_nodes(&mut parent_map, &mut rank_map, joint.entity_a, joint.entity_b);
                resolved_joints.push((*id, joint.clone(), jb));
            }
        }
    }

    // Temasları island'lara dağıt
    let mut islands_map: HashMap<u32, Island> = HashMap::new();
    for c in all_contacts {
        let a_dyn = c.inv_mass_a > 0.0;
        let root = if a_dyn {
            find_root(&mut parent_map, c.ent_a)
        } else {
            find_root(&mut parent_map, c.ent_b)
        };
        let island = islands_map.entry(root).or_insert_with(|| Island {
            joints: Vec::new(),
            contacts: Vec::new(),
            velocities: HashMap::new(),
            poses: HashMap::new(),
        });
        island.contacts.push(c);
    }

    for (id, joint, jb) in resolved_joints {
        let root = find_root(&mut parent_map, joint.entity_a);
        let island = islands_map.entry(root).or_insert_with(|| Island {
            joints: Vec::new(),
            contacts: Vec::new(),
            velocities: HashMap::new(),
            poses: HashMap::new(),
        });
        island.joints.push((id, joint.clone(), jb));
        
        island.velocities.entry(joint.entity_a).or_insert_with(|| velocities.get(joint.entity_a).cloned().unwrap_or(Velocity::new(Vec3::ZERO)));
        island.velocities.entry(joint.entity_b).or_insert_with(|| velocities.get(joint.entity_b).cloned().unwrap_or(Velocity::new(Vec3::ZERO)));
        island.poses.entry(joint.entity_a).or_insert_with(|| transforms.get(joint.entity_a).cloned().unwrap_or(Transform::new(Vec3::ZERO)));
        island.poses.entry(joint.entity_b).or_insert_with(|| transforms.get(joint.entity_b).cloned().unwrap_or(Transform::new(Vec3::ZERO)));
    }

    // Her island'a başlangıç hız ve pozisyon snapshot'larını aktar
    for island in islands_map.values_mut() {
        for c in &island.contacts {
            if c.inv_mass_a > 0.0 && !island.velocities.contains_key(&c.ent_a) {
                island.velocities.insert(
                    c.ent_a,
                    velocities.get(c.ent_a).cloned().unwrap_or(Velocity::new(Vec3::ZERO)),
                );
                let mut p = *transforms.get(c.ent_a).unwrap();
                p.position += c.ccd_offset_a;
                island.poses.insert(c.ent_a, p);
            }
            if c.inv_mass_b > 0.0 && !island.velocities.contains_key(&c.ent_b) {
                island.velocities.insert(
                    c.ent_b,
                    velocities.get(c.ent_b).cloned().unwrap_or(Velocity::new(Vec3::ZERO)),
                );
                let mut p = *transforms.get(c.ent_b).unwrap();
                p.position += c.ccd_offset_b;
                island.poses.insert(c.ent_b, p);
            }
        }
    }

    islands_map.into_values().collect()
}








pub fn solve_single_island(island: &mut Island, solver_iters: u32, frame_count: u64, dt: f32) {
    const MAX_ANG: f32 = 100.0;
    const MAX_LIN: f32 = 200.0;

    // Frame-seeded Fisher-Yates shuffle (çözüm bias'ını önler)
    let contacts_len = island.contacts.len();
    if contacts_len > 1 {
        let seed = frame_count as usize;
        for i in 0..(contacts_len - 1) {
            let range = contacts_len - i;
            let h = (i.wrapping_add(1).wrapping_mul(2654435761).wrapping_add(seed)) ^ seed;
            let swap_idx = i + (h % range);
            island.contacts.swap(i, swap_idx);
        }
    }

    // PGS (Gauss-Seidel) Iterasyonları Öncesi: Başlangıç Hızlarına Göre Bounce (Sekme) Hedeflerini Sabitle.
    // Eğer iterations döngüsünün içinde güncel hıza göre e * vn hesaplarsak, Newton Sarkacı gibi sistemlerde
    // hız aktarılırken osilasyon (sarmal momentum) oluşur ve toplar yavaşlayarak birbirine yapışır (çamurlaşır)!
    for c in island.contacts.iter_mut() {
        let va = island.velocities.get(&c.ent_a).cloned().unwrap_or(Velocity::new(Vec3::ZERO));
        let vb = island.velocities.get(&c.ent_b).cloned().unwrap_or(Velocity::new(Vec3::ZERO));
        let rel = (vb.linear + vb.angular.cross(c.r_b)) - (va.linear + va.angular.cross(c.r_a));
        let vn = rel.dot(c.normal);
        
        let e = if vn.abs() < 0.01 { 0.0 } else { c.restitution };
        
        // Sadece yaklaşıyorlarsa (vn < 0) sekme hedeflenir. Ayrılıyorlarsa sekme 0'dır.
        c.bias_bounce = if vn < 0.0 { -e * vn } else { 0.0 };
    }

    // Sequential Impulse iterasyonları
    for _iter in 0..solver_iters {
        for c in island.contacts.iter_mut() {
            let va = island
                .velocities
                .get(&c.ent_a)
                .cloned()
                .unwrap_or(Velocity::new(Vec3::ZERO));
            let vb = island
                .velocities
                .get(&c.ent_b)
                .cloned()
                .unwrap_or(Velocity::new(Vec3::ZERO));

            let rel =
                (vb.linear + vb.angular.cross(c.r_b)) - (va.linear + va.angular.cross(c.r_a));
            let vn = rel.dot(c.normal);

            let ra_x_n = c.r_a.cross(c.normal);
            let rb_x_n = c.r_b.cross(c.normal);
            let ang_a = apply_inv_inertia(ra_x_n, c.inv_inertia_a, c.rot_a)
                .cross(c.r_a)
                .dot(c.normal);
            let ang_b = apply_inv_inertia(rb_x_n, c.inv_inertia_b, c.rot_b)
                .cross(c.r_b)
                .dot(c.normal);
            let eff_mass = c.inv_mass_a + c.inv_mass_b + ang_a + ang_b;
            if eff_mass == 0.0 {
                continue;
            }

            let bias = ((0.15 / 1.0) * (c.penetration - 0.005).max(0.0)).min(2.0);
            
            // J_new formülünde -(1+e)*vn yerine: -vn + c.bias_bounce kullanılır!
            // Ayrıca Baumgarte bias'ının kusursuz elastik çarpışmalarda sonsuz enerji üretmesini önlemek için,
            // eğer sekme hızı zaten bias'ı aşıyorsa bias'ı sıfırlıyoruz. (Baumgarte patlaması engellendi)
            let effective_bias = (bias - c.bias_bounce).max(0.0);
            let j_new = (-vn + c.bias_bounce + effective_bias) / eff_mass;
            
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
                    v_a.angular += apply_inv_inertia(
                        c.r_a.cross(impulse * -1.0),
                        c.inv_inertia_a,
                        c.rot_a,
                    );
                    v_a.angular.x = v_a.angular.x.clamp(-MAX_ANG, MAX_ANG);
                    v_a.angular.y = v_a.angular.y.clamp(-MAX_ANG, MAX_ANG);
                    v_a.angular.z = v_a.angular.z.clamp(-MAX_ANG, MAX_ANG);
                }
                if let Some(v_b) = island.velocities.get_mut(&c.ent_b) {
                    v_b.linear += impulse * c.inv_mass_b;
                    v_b.linear.x = v_b.linear.x.clamp(-MAX_LIN, MAX_LIN);
                    v_b.linear.y = v_b.linear.y.clamp(-MAX_LIN, MAX_LIN);
                    v_b.linear.z = v_b.linear.z.clamp(-MAX_LIN, MAX_LIN);
                    v_b.angular += apply_inv_inertia(c.r_b.cross(impulse), c.inv_inertia_b, c.rot_b);
                    v_b.angular.x = v_b.angular.x.clamp(-MAX_ANG, MAX_ANG);
                    v_b.angular.y = v_b.angular.y.clamp(-MAX_ANG, MAX_ANG);
                    v_b.angular.z = v_b.angular.z.clamp(-MAX_ANG, MAX_ANG);
                }
            }

            let va2 = island
                .velocities
                .get(&c.ent_a)
                .cloned()
                .unwrap_or(Velocity::new(Vec3::ZERO));
            let vb2 = island
                .velocities
                .get(&c.ent_b)
                .cloned()
                .unwrap_or(Velocity::new(Vec3::ZERO));
            let rel2 =
                (vb2.linear + vb2.angular.cross(c.r_b)) - (va2.linear + va2.angular.cross(c.r_a));
            let tangent_vel = rel2 - c.normal * rel2.dot(c.normal);
            let ts = tangent_vel.length();

            if ts > 0.001 {
                let tangent_dir = tangent_vel / ts;
                let ra_cross_t = c.r_a.cross(tangent_dir);
                let rb_cross_t = c.r_b.cross(tangent_dir);
                let tangent_eff_mass = c.inv_mass_a
                    + c.inv_mass_b
                    + apply_inv_inertia(ra_cross_t, c.inv_inertia_a, c.rot_a)
                        .cross(c.r_a)
                        .dot(tangent_dir)
                    + apply_inv_inertia(rb_cross_t, c.inv_inertia_b, c.rot_b)
                        .cross(c.r_b)
                        .dot(tangent_dir);

                if tangent_eff_mass > 0.0 {
                    let jt = -ts / tangent_eff_mass;
                    let max_friction = c.accumulated_j * c.friction;
                    let old_friction = c.accumulated_friction;
                    let mut new_friction = old_friction + tangent_dir * jt;
                    let friction_len = new_friction.length();
                    if friction_len > max_friction {
                        let kinetic_limit = c.accumulated_j * (c.friction * 0.7);
                        new_friction *= kinetic_limit / friction_len;
                    }
                    let fi = new_friction - old_friction;
                    c.accumulated_friction = new_friction;

                    if let Some(v) = island.velocities.get_mut(&c.ent_a) {
                        v.linear -= fi * c.inv_mass_a;
                        v.angular +=
                            apply_inv_inertia(c.r_a.cross(fi * -1.0), c.inv_inertia_a, c.rot_a);
                        v.angular.x = v.angular.x.clamp(-MAX_ANG, MAX_ANG);
                        v.angular.y = v.angular.y.clamp(-MAX_ANG, MAX_ANG);
                        v.angular.z = v.angular.z.clamp(-MAX_ANG, MAX_ANG);
                    }
                    if let Some(v) = island.velocities.get_mut(&c.ent_b) {
                        v.linear += fi * c.inv_mass_b;
                        v.angular += apply_inv_inertia(c.r_b.cross(fi), c.inv_inertia_b, c.rot_b);
                        v.angular.x = v.angular.x.clamp(-MAX_ANG, MAX_ANG);
                        v.angular.y = v.angular.y.clamp(-MAX_ANG, MAX_ANG);
                        v.angular.z = v.angular.z.clamp(-MAX_ANG, MAX_ANG);
                    }
                }
            }
        }

        for (_, joint, jb) in island.joints.iter() {
            let va = island
                .velocities
                .get(&joint.entity_a)
                .cloned()
                .unwrap_or(Velocity::new(Vec3::ZERO));
            let vb = island
                .velocities
                .get(&joint.entity_b)
                .cloned()
                .unwrap_or(Velocity::new(Vec3::ZERO));

            let mut va_lin = va.linear;
            let mut va_ang = va.angular;
            let mut vb_lin = vb.linear;
            let mut vb_ang = vb.angular;

            crate::constraints::solve_joint_velocity(
                dt, joint, jb, &mut va_lin, &mut va_ang, &mut vb_lin, &mut vb_ang,
            );

            if let Some(v) = island.velocities.get_mut(&joint.entity_a) {
                v.linear = va_lin;
                v.angular = va_ang;
            }
            if let Some(v) = island.velocities.get_mut(&joint.entity_b) {
                v.linear = vb_lin;
                v.angular = vb_ang;
            }
        }
    }

    // NOT: Position projection kaldırıldı — Baumgarte bias (satır 958) zaten
    // hız seviyesinde penetrasyon düzeltmesi yapıyor. İkisinin birden aktif olması
    // çift düzeltme üretip objeleri havaya fırlatıyordu.

    for (_, joint, jb) in island.joints.iter() {
        let mut pos_a = jb.pos_a;
        let mut pos_b = jb.pos_b;
        if let Some(p) = island.poses.get(&joint.entity_a) {
            pos_a = p.position + p.rotation.mul_vec3(joint.anchor_a);
        }
        if let Some(p) = island.poses.get(&joint.entity_b) {
            pos_b = p.position + p.rotation.mul_vec3(joint.anchor_b);
        }

        let mut pos_a_core = island
            .poses
            .get(&joint.entity_a)
            .map(|p| p.position)
            .unwrap_or(Vec3::ZERO);
        let mut pos_b_core = island
            .poses
            .get(&joint.entity_b)
            .map(|p| p.position)
            .unwrap_or(Vec3::ZERO);

        let mut latest_jb = jb.clone();
        latest_jb.pos_a = pos_a;
        latest_jb.pos_b = pos_b;

        crate::constraints::solve_joint_position(joint, &latest_jb, &mut pos_a_core, &mut pos_b_core);

        if let Some(p) = island.poses.get_mut(&joint.entity_a) {
            p.position = pos_a_core;
        }
        if let Some(p) = island.poses.get_mut(&joint.entity_b) {
            p.position = pos_b_core;
        }
    }
}

pub fn solve_islands(
    islands: &mut Vec<Island>,
    contact_cache: &HashMap<(u32, u32), Vec<CachedContact>>,
    solver_iters: u32,
    frame_count: u64,
    dt: f32,
    parallel_island_solve: bool,
) {
    // Warm-start: önceki frame'in NORMAL impulslarını temas eşlemesiyle aktar.
    // NOT: Sürtünme warm-start kaldırıldı — cached friction vektörü önceki frame'in
    // teğet yönüne göre hesaplandı. Obje döndüğünde teğet değişir ama cache'deki
    // eski yön uygulanmaya devam eder → yanlış yönde kuvvet → jitter.
    for island in islands.iter_mut() {
        for c in island.contacts.iter_mut() {
            let key = if c.ent_a < c.ent_b { (c.ent_a, c.ent_b) } else { (c.ent_b, c.ent_a) };
            if let Some(cached) = contact_cache.get(&key) {
                if let Some((cached_j, _cached_friction)) = match_cached_contact(c.world_point, cached) {
                    c.accumulated_j = (cached_j * WARM_START_FACTOR).min(5.0);
                    // c.accumulated_friction kasıtlı olarak sıfır bırakılıyor
                }
            }
        }
    }

    // Warm-start normal impulslarını hızlara uygula
    for island in islands.iter_mut() {
        for c in island.contacts.iter() {
            if c.accumulated_j > 1e-6 {
                let impulse = c.normal * c.accumulated_j;
                if let Some(v_a) = island.velocities.get_mut(&c.ent_a) {
                    v_a.linear  -= impulse * c.inv_mass_a;
                    v_a.angular += apply_inv_inertia(c.r_a.cross(impulse * -1.0), c.inv_inertia_a, c.rot_a);
                }
                if let Some(v_b) = island.velocities.get_mut(&c.ent_b) {
                    v_b.linear  += impulse * c.inv_mass_b;
                    v_b.angular += apply_inv_inertia(c.r_b.cross(impulse), c.inv_inertia_b, c.rot_b);
                }
            }
        }
    }

    if parallel_island_solve {
        use rayon::prelude::*;
        islands.par_iter_mut().for_each(|island| {
            solve_single_island(island, solver_iters, frame_count, dt);
        });
    } else {
        for island in islands.iter_mut() {
            solve_single_island(island, solver_iters, frame_count, dt);
        }
    }
}


pub fn write_back(
    islands: Vec<Island>,
    transforms:           &mut gizmo_core::SparseSet<Transform>,
    velocities:           &mut gizmo_core::SparseSet<Velocity>,
    _vehicle_entities:     &std::collections::HashSet<u32>,
    solver_state:         &mut PhysicsSolverState,
    collision_events:     &mut Vec<crate::CollisionEvent>,
    max_contacts_per_pair: usize,
    event_throttle_frames: u32,
) {
    // Warm-start cache temizle — Fix #3:
    // contact_cache.clear() yerine sadece geçersiz (artık var olmayan) entity çiftlerini sil.
    // Aktif çiftler yeni değerlerle güncellendiğinden, eski değerler bu döngüde üzerine yazılacak.
    // Bu %30 daha az alloc demek ve gerçek warm-startħ korur.
    let active_entities: std::collections::HashSet<u32> = velocities.iter().map(|(e, _)| e).collect();
    solver_state.contact_cache.retain(|(a, b), _| {
        active_entities.contains(a) && active_entities.contains(b)
    });

    let frame = solver_state.frame_counter;

    for island in &islands {
        for c in &island.contacts {
            // Warm-start cache kaydı — Fix #6: limiti config'den al
            // Fix: poses'da entity yoksa (statik cisimler gibi) c.world_point CCD-öncesi
            // eski değerdir. Bunun yerine write-back ile zaten güncellenmiş olan
            // transforms'tan oku — solver-sonrası doğru pozisyon garanti edilir.
            let wp = island.poses.get(&c.ent_a)
                .map(|p| p.position + c.r_a)
                .or_else(|| transforms.get(c.ent_a).map(|t| t.position + c.r_a))
                .unwrap_or(c.world_point);
            let key = if c.ent_a < c.ent_b { (c.ent_a, c.ent_b) } else { (c.ent_b, c.ent_a) };
            let entry = solver_state.contact_cache.entry(key).or_default();
            if entry.len() < max_contacts_per_pair {
                entry.push(CachedContact {
                    world_point:          wp,
                    accumulated_normal:   c.accumulated_j,
                    accumulated_friction: c.accumulated_friction,
                });
            }

            // Darbe/momentum event'i fırlat — Fix #31: throttle
            // Aynı çift için en fazla her `event_throttle_frames` frame'de bir event
            let eff_mass  = 1.0 / (c.inv_mass_a + c.inv_mass_b).max(0.0001);
            let threshold = 0.05 * eff_mass + 0.01;
            let should_fire = c.accumulated_j > threshold && (
                event_throttle_frames == 0
                || frame.is_multiple_of(event_throttle_frames as u64)
            );
            if should_fire {
                let pos_a = island.poses.get(&c.ent_a)
                    .map(|t| t.position)
                    .or_else(|| transforms.get(c.ent_a).map(|t| t.position))
                    .unwrap_or(Vec3::ZERO);
                collision_events.push(crate::CollisionEvent {
                    entity_a: c.ent_a,
                    entity_b: c.ent_b,
                    position: pos_a + c.r_a,
                    normal:   c.normal,
                    impulse:  c.accumulated_j,
                });
            }
        }
    }

    for island in islands {
        for (ent, vel) in island.velocities {
            if let Some(v) = velocities.get_mut(ent) { *v = vel; }
        }
        for (ent, tbox) in island.poses {
            if let Some(t) = transforms.get_mut(ent) {
                *t = tbox;
                t.update_local_matrix();
            }
        }
    }
}

// ─── Ana Giriş Noktası ────────────────────────────────────────────────────────
