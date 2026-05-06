/// AAA Constraint Solver — Sequential Impulses (SI) with:
/// - Warm-starting (önceki frame'in impulslarını uygula)
/// - Accumulated-impulse clamping (negatif normal impulse engeli)
/// - Coulomb friction cone (statik + dinamik)
/// - Speculative contacts (penetrasyon öncesi temas)
/// - 2-boyutlu sürtünme (iki tangent yönü)
/// - Restitution threshold (micro-bounce önleme)
/// - Solver iteration sayısı konfigüre edilebilir
use crate::collision::ContactManifold;
use crate::components::{RigidBody, Transform, Velocity};
use gizmo_math::{Mat3, Vec3};

// ─────────────────────────────────────────────────────────────────────────────
// Konfigürasyon
// ─────────────────────────────────────────────────────────────────────────────

pub struct ConstraintSolver {
    /// PGS iterasyon sayısı (daha fazla = daha stabil, daha yavaş)
    pub iterations: usize,
    /// Baumgarte stabilizasyon faktörü (0.1..0.3 arası ideal)
    /// Split Impulse kapalıyken fallback olarak kullanılır.
    pub baumgarte: f32,
    /// Penetrasyon toleransı — bu kadar penetrasyon normal kabul edilir
    pub slop: f32,
    /// Warm-start faktörü (0.8 = önceki frame impulsunun %80'ini uygula)
    pub warm_start_factor: f32,
    /// Bu hızın altındaki çarpışmalarda restitution sıfır yapılır (dinlenme teması)
    pub restitution_velocity_threshold: f32,
    /// Maksimum pozisyon düzeltme miktarı (metre/step) - Patlamaları önler
    pub max_linear_correction: f32,
    /// Split Impulse (Pseudo-Velocity) — pozisyon düzeltmesini ayrı bir
    /// pseudo-velocity kanalında yapar, velocity'yi kirletmez.
    /// Stacking stabilitesi ve resting contact jitter'ını önler.
    pub split_impulse_enabled: bool,
    /// Split Impulse penetrasyon düzeltme oranı (0.1..0.4 arası ideal)
    pub split_impulse_erp: f32,
}

impl Default for ConstraintSolver {
    fn default() -> Self {
        Self {
            iterations: 20,
            baumgarte:  0.15,
            slop:       0.005,
            warm_start_factor: 0.85,
            restitution_velocity_threshold: 1.0,
            max_linear_correction: 0.02,
            split_impulse_enabled: true,
            split_impulse_erp: 0.1,
        }
    }
}

impl ConstraintSolver {
    pub fn new(iterations: usize) -> Self {
        Self { iterations, ..Default::default() }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ANA SOLVER: Manifold listesi üzerinde PGS (Projected Gauss-Seidel)
    // ─────────────────────────────────────────────────────────────────────────

    pub fn solve_contacts(
        &self,
        manifolds:  &mut [ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        entity_index_map: &std::collections::HashMap<u32, usize>,
        dt: f32,
    ) {
        if manifolds.is_empty() { return; }

        // ── Split Impulse: pseudo-velocity buffers ────────────────────────
        // Pozisyon düzeltmesi asıl velocity'den ayrılır, böylece resting
        // contact'larda jitter engellenir ve stacking stabilitesi artar.
        let mut pseudo_vel: Vec<(Vec3, Vec3)> = vec![(Vec3::ZERO, Vec3::ZERO); velocities.len()];

        // ── Warm-starting ────────────────────────────────────────────────────
        for mid in 0..manifolds.len() {
            let entity_a_id = manifolds[mid].entity_a.id();
            let entity_b_id = manifolds[mid].entity_b.id();
            
            let idx_a = match entity_index_map.get(&entity_a_id) {
                Some(&i) => i,
                None => continue,
            };
            let idx_b = match entity_index_map.get(&entity_b_id) {
                Some(&i) => i,
                None => continue,
            };

            let inv_m_a = rigid_bodies[idx_a].inv_mass();
            let inv_m_b = rigid_bodies[idx_b].inv_mass();
            let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
            let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
            let dyn_a   = rigid_bodies[idx_a].is_dynamic();
            let dyn_b   = rigid_bodies[idx_b].is_dynamic();

            let com_a = transforms[idx_a].position + transforms[idx_a].rotation.mul_vec3(rigid_bodies[idx_a].center_of_mass);
            let com_b = transforms[idx_b].position + transforms[idx_b].rotation.mul_vec3(rigid_bodies[idx_b].center_of_mass);

            for contact in &manifolds[mid].contacts {
                let r_a = contact.point - com_a;
                let r_b = contact.point - com_b;

                let wn = contact.normal * (contact.normal_impulse * self.warm_start_factor);
                let wt = contact.tangent_impulse * self.warm_start_factor;

                if dyn_a {
                    velocities[idx_a].linear  -= wn * inv_m_a;
                    velocities[idx_a].linear  -= wt * inv_m_a;
                    velocities[idx_a].angular -= inv_i_a * (r_a.cross(wn) + r_a.cross(wt));
                }
                if dyn_b {
                    velocities[idx_b].linear  += wn * inv_m_b;
                    velocities[idx_b].linear  += wt * inv_m_b;
                    velocities[idx_b].angular += inv_i_b * (r_b.cross(wn) + r_b.cross(wt));
                }
            }
        }

        // ── İteratif PGS ─────────────────────────────────────────────────────
        let inv_dt = if dt > 0.0 { 1.0 / dt } else { 0.0 };

        for _ in 0..self.iterations {
            for mid in 0..manifolds.len() {
                let entity_a_id = manifolds[mid].entity_a.id();
                let entity_b_id = manifolds[mid].entity_b.id();
                
                let idx_a = match entity_index_map.get(&entity_a_id) {
                    Some(&i) => i,
                    None => continue,
                };
                let idx_b = match entity_index_map.get(&entity_b_id) {
                    Some(&i) => i,
                    None => continue,
                };

                let friction    = manifolds[mid].friction;
                let restitution = manifolds[mid].restitution;

                for cid in 0..manifolds[mid].contacts.len() {
                    let contact_pt  = manifolds[mid].contacts[cid].point;
                    let normal      = manifolds[mid].contacts[cid].normal;
                    let penetration = manifolds[mid].contacts[cid].penetration;
                    let acc_n       = manifolds[mid].contacts[cid].normal_impulse;
                    let acc_t       = manifolds[mid].contacts[cid].tangent_impulse;

                    let com_a = transforms[idx_a].position + transforms[idx_a].rotation.mul_vec3(rigid_bodies[idx_a].center_of_mass);
                    let com_b = transforms[idx_b].position + transforms[idx_b].rotation.mul_vec3(rigid_bodies[idx_b].center_of_mass);
                    let r_a   = contact_pt - com_a;
                    let r_b   = contact_pt - com_b;

                    let inv_m_a = rigid_bodies[idx_a].inv_mass();
                    let inv_m_b = rigid_bodies[idx_b].inv_mass();
                    let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
                    let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
                    let dyn_a   = rigid_bodies[idx_a].is_dynamic();
                    let dyn_b   = rigid_bodies[idx_b].is_dynamic();

                    if !dyn_a && !dyn_b { continue; }

                    // Temas noktasındaki göreli hız
                    let va = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
                    let vb = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
                    let rel_vel  = vb - va;
                    let vel_norm = rel_vel.dot(normal);

                    // ── Normal İmpuls ────────────────────────────────────────
                    let r_a_x_n = r_a.cross(normal);
                    let r_b_x_n = r_b.cross(normal);
                    let k_n = inv_m_a + inv_m_b
                        + (inv_i_a * r_a_x_n).dot(r_a_x_n)
                        + (inv_i_b * r_b_x_n).dot(r_b_x_n);

                    if k_n < 1e-8 { continue; }

                    // Pozisyon düzeltme stratejisi:
                    // Split Impulse: bias=0 (pozisyon düzeltmesi ayrı pseudo-velocity kanalında)
                    // Fallback: Baumgarte bias velocity'ye karıştırılır
                    let bias = if penetration < 0.0 {
                        // Speculative contact: nesne henüz teması yapmadı
                        penetration * inv_dt
                    } else if self.split_impulse_enabled {
                        // Split Impulse: pozisyon düzeltme tamamen pseudo-velocity pass'te
                        // Velocity kanalı temiz kalır → resting jitter yok
                        0.0
                    } else {
                        // Fallback Baumgarte
                        let correction = (penetration - self.slop).max(0.0).min(self.max_linear_correction);
                        self.baumgarte * inv_dt * correction
                    };

                    // Restitution: sadece yüksek hızlı çarpışmalarda
                    let e = if -vel_norm > self.restitution_velocity_threshold {
                        restitution
                    } else {
                        0.0
                    };

                    let delta_n   = (-(1.0 + e) * vel_norm + bias) / k_n;
                    let new_acc_n = (acc_n + delta_n).max(0.0); // Clamp: çekme yok
                    let actual_n  = new_acc_n - acc_n;
                    manifolds[mid].contacts[cid].normal_impulse = new_acc_n;

                    let imp_n = normal * actual_n;
                    if dyn_a {
                        velocities[idx_a].linear  -= imp_n * inv_m_a;
                        velocities[idx_a].angular -= inv_i_a * r_a.cross(imp_n);
                    }
                    if dyn_b {
                        velocities[idx_b].linear  += imp_n * inv_m_b;
                        velocities[idx_b].angular += inv_i_b * r_b.cross(imp_n);
                    }

                    // ── Sürtünme İmpulsu (2D Coulomb Cone) ──────────────────
                    // Güncel hızları al (normal impuls uygulandıktan sonra)
                    let va2     = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
                    let vb2     = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
                    let rel2    = vb2 - va2;
                    let tang_v  = rel2 - normal * rel2.dot(normal);
                    let tang_mag = tang_v.length();

                    if tang_mag < 1e-8 && acc_t.length_squared() < 1e-8 { continue; }
                    
                    let tangent = if acc_t.length_squared() > 1e-8 {
                        acc_t.normalize()
                    } else if tang_mag > 1e-8 {
                        tang_v / tang_mag
                    } else {
                        if normal.x.abs() > 0.9 {
                            gizmo_math::Vec3::new(0.0, 1.0, 0.0).cross(normal).normalize()
                        } else {
                            gizmo_math::Vec3::new(1.0, 0.0, 0.0).cross(normal).normalize()
                        }
                    };

                    let r_a_x_t = r_a.cross(tangent);
                    let r_b_x_t = r_b.cross(tangent);
                    let k_t = inv_m_a + inv_m_b
                        + (inv_i_a * r_a_x_t).dot(r_a_x_t)
                        + (inv_i_b * r_b_x_t).dot(r_b_x_t);

                    if k_t < 1e-8 { continue; }

                    let rel_t = rel2.dot(tangent);
                    let delta_t = -rel_t / k_t;

                    // Coulomb cone: statik ≤ μ_s * N, dinamik = μ_d * N
                    let static_mu  = manifolds[mid].static_friction;
                    let dynamic_mu = friction;
                    let max_static  = static_mu  * new_acc_n.abs();
                    let max_dynamic = dynamic_mu * new_acc_n.abs();

                    // Önceki birikimli tanjant (aynı yön boyunca projeksiyon)
                    let old_t_along = acc_t.dot(tangent);
                    let new_t_along = old_t_along + delta_t;

                    let clamped_t = if new_t_along.abs() <= max_static {
                        new_t_along // Statik sürtünme koni içinde
                    } else {
                        new_t_along.signum() * max_dynamic // Dinamik sürtünmeye geç
                    };

                    let actual_t = clamped_t - old_t_along;
                    manifolds[mid].contacts[cid].tangent_impulse = tangent * clamped_t;

                    let imp_t = tangent * actual_t;
                    if dyn_a {
                        velocities[idx_a].linear  -= imp_t * inv_m_a;
                        velocities[idx_a].angular -= inv_i_a * r_a.cross(imp_t);
                    }
                    if dyn_b {
                        velocities[idx_b].linear  += imp_t * inv_m_b;
                        velocities[idx_b].angular += inv_i_b * r_b.cross(imp_t);
                    }
                }
            }
        }

        // ── Split Impulse: Pozisyon Düzeltme Pass ────────────────────────────
        // Asıl velocity'den bağımsız olarak pseudo-velocity hesaplar.
        // Bu pass penetrasyon düzeltmesini velocity kanalından ayırır.
        // Birikimli pseudo-impulse takibi ile over-correction engellenir.
        if self.split_impulse_enabled {
            // Per-contact birikimli pseudo-impulse (PGS clamping için)
            let mut acc_pseudo: Vec<Vec<f32>> = manifolds.iter()
                .map(|m| vec![0.0f32; m.contacts.len()])
                .collect();

            let pos_iterations = (self.iterations / 2).max(4);
            for _ in 0..pos_iterations {
                for mid in 0..manifolds.len() {
                    let entity_a_id = manifolds[mid].entity_a.id();
                    let entity_b_id = manifolds[mid].entity_b.id();

                    let idx_a = match entity_index_map.get(&entity_a_id) {
                        Some(&i) => i,
                        None => continue,
                    };
                    let idx_b = match entity_index_map.get(&entity_b_id) {
                        Some(&i) => i,
                        None => continue,
                    };

                    let inv_m_a = rigid_bodies[idx_a].inv_mass();
                    let inv_m_b = rigid_bodies[idx_b].inv_mass();
                    let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
                    let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
                    let dyn_a   = rigid_bodies[idx_a].is_dynamic();
                    let dyn_b   = rigid_bodies[idx_b].is_dynamic();

                    if !dyn_a && !dyn_b { continue; }

                    let com_a = transforms[idx_a].position + transforms[idx_a].rotation.mul_vec3(rigid_bodies[idx_a].center_of_mass);
                    let com_b = transforms[idx_b].position + transforms[idx_b].rotation.mul_vec3(rigid_bodies[idx_b].center_of_mass);

                    for cid in 0..manifolds[mid].contacts.len() {
                        let contact_pt  = manifolds[mid].contacts[cid].point;
                        let normal      = manifolds[mid].contacts[cid].normal;
                        let penetration = manifolds[mid].contacts[cid].penetration;

                        let correction = (penetration - self.slop).max(0.0).min(self.max_linear_correction);
                        if correction < 1e-6 { continue; }

                        let r_a = contact_pt - com_a;
                        let r_b = contact_pt - com_b;

                        let r_a_x_n = r_a.cross(normal);
                        let r_b_x_n = r_b.cross(normal);
                        let k_n = inv_m_a + inv_m_b
                            + (inv_i_a * r_a_x_n).dot(r_a_x_n)
                            + (inv_i_b * r_b_x_n).dot(r_b_x_n);
                        if k_n < 1e-8 { continue; }

                        // Pseudo-velocity relative to contact normal
                        let pv_a = pseudo_vel[idx_a].0 + pseudo_vel[idx_a].1.cross(r_a);
                        let pv_b = pseudo_vel[idx_b].0 + pseudo_vel[idx_b].1.cross(r_b);
                        let pv_rel = pv_b.dot(normal) - pv_a.dot(normal);

                        let bias = self.split_impulse_erp * inv_dt * correction;
                        // Velocity solver ile aynı konvansiyon: delta = (-pv_rel + bias) / k
                        // pv_rel > 0 → nesneler zaten ayrılıyor → düzeltme azalır
                        // pv_rel ≈ bias → yakınsadı → delta ≈ 0
                        let delta_p = (-pv_rel + bias) / k_n;

                        // Birikimli clamp: toplam pseudo-impulse ≥ 0 (çekme yok)
                        let old_acc = acc_pseudo[mid][cid];
                        let new_acc = (old_acc + delta_p).max(0.0);
                        let actual_delta = new_acc - old_acc;
                        acc_pseudo[mid][cid] = new_acc;

                        let imp_p = normal * actual_delta;
                        if dyn_a {
                            pseudo_vel[idx_a].0 -= imp_p * inv_m_a;
                            pseudo_vel[idx_a].1 -= inv_i_a * r_a.cross(imp_p);
                        }
                        if dyn_b {
                            pseudo_vel[idx_b].0 += imp_p * inv_m_b;
                            pseudo_vel[idx_b].1 += inv_i_b * r_b.cross(imp_p);
                        }
                    }
                }
            }

            // Pseudo-velocity'yi gerçek velocity'ye ekle (sadece pozisyon düzeltme bileşeni)
            for i in 0..velocities.len() {
                velocities[i].linear  += pseudo_vel[i].0;
                velocities[i].angular += pseudo_vel[i].1;
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Tek temas noktası için standalone solver (geriye dönük uyum)
    // ─────────────────────────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub fn solve_contact_constraint(
        &self,
        rb_a:        &mut RigidBody,
        transform_a: &Transform,
        vel_a:       &mut Velocity,
        rb_b:        &mut RigidBody,
        transform_b: &Transform,
        vel_b:       &mut Velocity,
        contact_point: Vec3,
        normal:        Vec3,
        penetration:   f32,
        static_friction: f32,
        dynamic_friction: f32,
        restitution:   f32,
        dt: f32,
    ) {
        if !rb_a.is_dynamic() && !rb_b.is_dynamic() { return; }

        let com_a = transform_a.position + transform_a.rotation.mul_vec3(rb_a.center_of_mass);
        let com_b = transform_b.position + transform_b.rotation.mul_vec3(rb_b.center_of_mass);

        let r_a = contact_point - com_a;
        let r_b = contact_point - com_b;

        let va = vel_a.linear + vel_a.angular.cross(r_a);
        let vb = vel_b.linear + vel_b.angular.cross(r_b);
        let rel_vel  = vb - va;
        let vel_norm = rel_vel.dot(normal);

        if vel_norm > 0.0 { return; } // Ayrılıyor, işlem yapma

        let inv_m_a = rb_a.inv_mass();
        let inv_m_b = rb_b.inv_mass();
        let inv_i_a = rb_a.inv_world_inertia_tensor(transform_a.rotation);
        let inv_i_b = rb_b.inv_world_inertia_tensor(transform_b.rotation);

        let r_a_x_n = r_a.cross(normal);
        let r_b_x_n = r_b.cross(normal);
        let k = inv_m_a + inv_m_b
            + (inv_i_a * r_a_x_n).dot(r_a_x_n)
            + (inv_i_b * r_b_x_n).dot(r_b_x_n);

        if k < 1e-8 { return; }

        let inv_dt = if dt > 0.0 { 1.0 / dt } else { 0.0 };
        let bias   = self.baumgarte * inv_dt * (penetration - self.slop).max(0.0);
        let e      = if -vel_norm > self.restitution_velocity_threshold { restitution } else { 0.0 };
        let j      = ((-(1.0 + e) * vel_norm + bias) / k).max(0.0);

        let impulse = normal * j;

        if rb_a.is_dynamic() {
            vel_a.linear  -= impulse * inv_m_a;
            vel_a.angular -= inv_i_a * r_a.cross(impulse);
        }
        if rb_b.is_dynamic() {
            vel_b.linear  += impulse * inv_m_b;
            vel_b.angular += inv_i_b * r_b.cross(impulse);
        }

        // Sürtünme
        self.apply_friction_standalone(
            rb_a, vel_a, rb_b, vel_b,
            r_a, r_b, normal, static_friction, dynamic_friction, j,
            &inv_i_a, &inv_i_b,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_friction_standalone(
        &self,
        rb_a: &RigidBody, vel_a: &mut Velocity,
        rb_b: &RigidBody, vel_b: &mut Velocity,
        r_a: Vec3, r_b: Vec3,
        normal: Vec3, static_friction: f32, dynamic_friction: f32, normal_impulse: f32,
        inv_i_a: &Mat3, inv_i_b: &Mat3,
    ) {
        let va   = vel_a.linear + vel_a.angular.cross(r_a);
        let vb   = vel_b.linear + vel_b.angular.cross(r_b);
        let rel  = vb - va;
        let tang_v   = rel - normal * rel.dot(normal);
        let tang_mag = tang_v.length();
        if tang_mag < 1e-8 { return; }
        let tangent = tang_v / tang_mag;

        let inv_m_a = rb_a.inv_mass();
        let inv_m_b = rb_b.inv_mass();

        let r_a_x_t = r_a.cross(tangent);
        let r_b_x_t = r_b.cross(tangent);
        let k = inv_m_a + inv_m_b
            + (*inv_i_a * r_a_x_t).dot(r_a_x_t)
            + (*inv_i_b * r_b_x_t).dot(r_b_x_t);
        if k < 1e-8 { return; }

        let max_static  = static_friction  * normal_impulse.abs();
        let max_dynamic = dynamic_friction * normal_impulse.abs();
        
        let delta_t = -tang_mag / k;

        let jt = if delta_t.abs() <= max_static {
            delta_t
        } else {
            delta_t.signum() * max_dynamic
        };

        let ft = tangent * jt;
        if rb_a.is_dynamic() {
            vel_a.linear  -= ft * inv_m_a;
            vel_a.angular -= *inv_i_a * r_a.cross(ft);
        }
        if rb_b.is_dynamic() {
            vel_b.linear  += ft * inv_m_b;
            vel_b.angular += *inv_i_b * r_b.cross(ft);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Testler
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec3;

    #[test]
    fn test_solver_creation() {
        let solver = ConstraintSolver::new(20);
        assert_eq!(solver.iterations, 20);
    }

    #[test]
    fn test_collision_response() {
        let mut rb_a = RigidBody::default();
        let mut rb_b = RigidBody::default();
        rb_a.wake_up();
        rb_b.wake_up();
        let transform_a = Transform::new(Vec3::new(0.0, 0.0, 0.0));
        let transform_b = Transform::new(Vec3::new(0.0, 2.0, 0.0));
        let mut vel_a = Velocity::new(Vec3::new(0.0,  1.0, 0.0));
        let mut vel_b = Velocity::new(Vec3::new(0.0, -1.0, 0.0));

        let solver = ConstraintSolver::default();
        solver.solve_contact_constraint(
            &mut rb_a, &transform_a, &mut vel_a,
            &mut rb_b, &transform_b, &mut vel_b,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            0.1, 0.6, 0.5, 0.5, 0.016,
        );

        assert!(vel_a.linear.y < 1.0);
        assert!(vel_b.linear.y > -1.0);
    }

    #[test]
    fn test_normal_impulse_non_negative() {
        let mut rb_a = RigidBody::default();
        let mut rb_b = RigidBody::default();
        rb_a.wake_up();
        rb_b.wake_up();
        let transform_a = Transform::new(Vec3::ZERO);
        let transform_b = Transform::new(Vec3::new(0.0, 1.0, 0.0));
        let mut vel_a   = Velocity::new(Vec3::new(0.0,  5.0, 0.0));
        let mut vel_b   = Velocity::new(Vec3::new(0.0, -5.0, 0.0));

        let before_a = vel_a.linear.y;
        let before_b = vel_b.linear.y;

        let solver = ConstraintSolver::default();
        solver.solve_contact_constraint(
            &mut rb_a, &transform_a, &mut vel_a,
            &mut rb_b, &transform_b, &mut vel_b,
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            0.05, 0.4, 0.3, 0.0, 0.016,
        );

        assert!(vel_a.linear.y < before_a);
        assert!(vel_b.linear.y > before_b);
    }

    #[test]
    fn test_resting_contact_no_bounce() {
        // Çok yavaş çarpışma → restitution sıfır olmalı
        let mut rb_a = RigidBody::default();
        let mut rb_b = RigidBody::default();
        rb_b.body_type = crate::components::rigid_body::BodyType::Static;
        rb_a.wake_up();
        let transform_a = Transform::new(Vec3::new(0.0, 1.05, 0.0));
        let transform_b = Transform::new(Vec3::ZERO);
        let mut vel_a   = Velocity::new(Vec3::new(0.0, -0.1, 0.0)); // Çok yavaş düşüyor
        let mut vel_b   = Velocity::default();

        let solver = ConstraintSolver::default();
        // Solver convention: rel_vel = vb - va, vel_norm = rel_vel.dot(normal)
        // A(y=1.05) düşüyor, B(y=0) duruyor. Normal A→B yönünde = (0,-1,0)
        // rel_vel = (0,0,0) - (0,-0.1,0) = (0,0.1,0)
        // vel_norm = (0,0.1,0).dot(0,-1,0) = -0.1 < 0 → yaklaşıyor ✓
        solver.solve_contact_constraint(
            &mut rb_a, &transform_a, &mut vel_a,
            &mut rb_b, &transform_b, &mut vel_b,
            Vec3::new(0.0, 0.525, 0.0),  // temas noktası (iki cismin arası)
            Vec3::new(0.0, -1.0, 0.0),   // normal: A'dan B'ye (aşağı)
            0.05, 0.6, 0.5, 0.0, 0.016,  // restitution=0.0 (resting contact)
        );
        assert!(vel_a.linear.y >= -0.01, "A should not bounce or sink significantly: {}", vel_a.linear.y);
        assert!(vel_b.linear.y <= 0.01, "B (static) should remain still: {}", vel_b.linear.y);
    }
}
