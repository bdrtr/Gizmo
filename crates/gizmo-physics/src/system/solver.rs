use std::collections::HashMap;
use gizmo_math::Vec3;
use crate::components::{Transform, RigidBody, Velocity};
use super::types::{DetectionResult, Island, PhysicsSolverState, CachedContact, MATCH_THRESHOLD_SQ, WARM_START_FACTOR};
use super::union_find::UnionFind;
use crate::integration::apply_inv_inertia;




pub fn match_cached_contact(new_point: Vec3, cached: &[CachedContact]) -> Option<f32> {
    let mut best_dist_sq = f32::MAX;
    let mut best = None;
    for cc in cached {
        let d = (new_point - cc.world_point).length_squared();
        if d < best_dist_sq && d < MATCH_THRESHOLD_SQ {
            best_dist_sq = d;
            best = Some(cc.accumulated_normal);
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
    let mut uf = UnionFind::new();

    entities_to_wake.extend(detection_result.wake_entities);
    let all_contacts = detection_result.contacts;

    for c in &all_contacts {
        let a_dyn = c.inv_mass_a > 0.0;
        let b_dyn = c.inv_mass_b > 0.0;
        if a_dyn && b_dyn {
            uf.union_nodes(c.ent_a, c.ent_b);
        } else if a_dyn {
            uf.find_root(c.ent_a);
        } else if b_dyn {
            uf.find_root(c.ent_b);
        }
    }

    let mut resolved_joints = Vec::new();
    if let Some(jw) = joint_world_opt {
        for (id, joint) in jw.joints.iter() {
            if let Some(jb) = crate::constraints::JointBodies::resolve(joint, transforms, rbs) {
                uf.union_nodes(joint.entity_a, joint.entity_b);
                resolved_joints.push((*id, joint.clone(), jb));
            }
        }
    }

    // Temasları island'lara dağıt
    let mut islands_map: HashMap<u32, Island> = HashMap::new();
    for c in all_contacts {
        let a_dyn = c.inv_mass_a > 0.0;
        let root = if a_dyn {
            uf.find_root(c.ent_a)
        } else {
            uf.find_root(c.ent_b)
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
        let root = uf.find_root(joint.entity_a);
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

    // Warm-start uygulanmadan önce ECS'den gelen orijinal hızlarla Bias Bounce hedeflerini sabitle:
    for island in islands_map.values_mut() {
        for c in island.contacts.iter_mut() {
            let va_orig = island.velocities.get(&c.ent_a).cloned().unwrap_or(Velocity::new(Vec3::ZERO));
            let vb_orig = island.velocities.get(&c.ent_b).cloned().unwrap_or(Velocity::new(Vec3::ZERO));
            let rel_orig = (vb_orig.linear + vb_orig.angular.cross(c.r_b)) - (va_orig.linear + va_orig.angular.cross(c.r_a));
            let vn_orig = rel_orig.dot(c.normal);
            let e = if vn_orig.abs() < 0.01 { 0.0 } else { c.restitution };
            c.bias_bounce = if vn_orig < -0.01 { -e * vn_orig } else { 0.0 };
        }
    }

    islands_map.into_values().collect()
}








pub fn solve_single_island(island: &mut Island, solver_iters: u32, frame_count: u64, dt: f32) {
    const MAX_ANG: f32 = 100.0;
    const MAX_LIN: f32 = 200.0;

    // Frame-seeded yates-shuffle yerine standart LCG (Linear Congruential Generator)
    let contacts_len = island.contacts.len();
    if contacts_len > 1 {
        let seed = frame_count as u64;
        let mut rng = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        for i in (1..contacts_len).rev() {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = (rng >> 33) as usize % (i + 1);
            island.contacts.swap(i, j);
        }
    }

    // PGS (Gauss-Seidel) Iterasyonları
    // Not: bias_bounce hedefleri artık build_islands içerisinde warm-start'tan ÖNCE 
    // sabitlendiği için burada tekrardan mevcut değişmiş hızlara göre hesaplanmıyor.
    
    // Sıralı impulse döngüsü öncesi bu frame'in sürtünme birikimini sıfırlıyoruz.
    for c in island.contacts.iter_mut() {
        c.accumulated_friction = Vec3::ZERO;
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
            let mut j_new = (-vn + c.bias_bounce + bias) / eff_mass;
            j_new = j_new.min(10.0 / dt);
            
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
                        let kinetic_limit = max_friction * 0.7; // kinetic slip 30% reduction
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

    // Pseudo-Velocity / Position Projection (Penetrasyonları doğrudan çöz, domino kaymasını önle)
    for c in island.contacts.iter() {
        if c.penetration > 0.01 {
            let correction = c.normal * (c.penetration - 0.01) * 0.4;
            let total_inv_mass = c.inv_mass_a + c.inv_mass_b;
            if total_inv_mass > 0.0 {
                if let Some(p) = island.poses.get_mut(&c.ent_a) {
                    p.position -= correction * (c.inv_mass_a / total_inv_mass);
                }
                if let Some(p) = island.poses.get_mut(&c.ent_b) {
                    p.position += correction * (c.inv_mass_b / total_inv_mass);
                }
            }
        }
    }

    for (_, joint, jb) in island.joints.iter() {
        let mut pos_a = jb.pos_a;
        let mut pos_b = jb.pos_b;
        if let Some(p) = island.poses.get(&joint.entity_a) {
            pos_a = p.position + p.rotation.mul_vec3(joint.anchor_a);
        }
        if let Some(p) = island.poses.get(&joint.entity_b) {
            pos_b = p.position + p.rotation.mul_vec3(joint.anchor_b);
        }

        let mut pos_a_core = match island.poses.get(&joint.entity_a) {
            Some(p) => p.position,
            None => continue,
        };
        let mut pos_b_core = match island.poses.get(&joint.entity_b) {
            Some(p) => p.position,
            None => continue,
        };

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
                if let Some(cached_j) = match_cached_contact(c.world_point, cached) {
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
    // Delete invalid entity pairs from cache
    let active_entities: std::collections::HashSet<u32> = velocities.iter().map(|(e, _)| e).collect();
    solver_state.contact_cache.retain(|(a, b), _| {
        active_entities.contains(a) && active_entities.contains(b)
    });

    let frame = solver_state.frame_counter;

    for island in &islands {
        for c in &island.contacts {
            let key = if c.ent_a < c.ent_b { (c.ent_a, c.ent_b) } else { (c.ent_b, c.ent_a) };
            if let Some(entry) = solver_state.contact_cache.get_mut(&key) {
                entry.clear();
            }
        }
    }

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
                let pos_a = match island.poses.get(&c.ent_a)
                    .map(|t| t.position)
                    .or_else(|| transforms.get(c.ent_a).map(|t| t.position)) {
                        Some(p) => p,
                        None => continue,
                    };
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
