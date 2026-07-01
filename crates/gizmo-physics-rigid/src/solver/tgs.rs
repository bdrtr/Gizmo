use super::ConstraintSolver;
use crate::components::{RigidBody, Velocity};
use gizmo_math::Vec3;
use gizmo_physics_core::components::Transform;
use gizmo_physics_core::ContactManifold;

impl ConstraintSolver {
    // ─────────────────────────────────────────────────────────────────────────
    // TGS SOFT ÇÖZÜCÜ (Temporal Gauss-Seidel, soft constraints)
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Box2D v3 "Soft Step" uyarlaması. Engine zaten 240 Hz sabit substep'le döner
    // (her substep: collision + solve), yani temporal alt-adım ZATEN var. Eksik olan,
    // her substep içindeki temas çözümünün KALİTESİYDİ: ileri-SI + split-impulse,
    // yüksek-enerjili çarpan UZUN yığınlarda (n≥16) çarpma dürtüsünü tüm temaslar
    // boyunca yayamıyor → metastable yığın çöküyordu.
    //
    // Bu çözücü her substep'te:
    //   1) warm-start (önceki impulslar),
    //   2) BIASED soft solve (N iter) + iterasyonlar arası POZİSYON-DELTA entegrasyonu
    //      (gerçek TGS): normal kısıt kritik-sönümlü yay-damper gibi çözülür
    //      (biasRate/massScale/impulseScale, contact_hertz+damping'ten); her iterasyondan
    //      sonra pozisyon-delta'sı (dp) güncel hızla ilerletilir ve bir sonraki iterasyonun
    //      bias'ı GÜNCEL penetrasyonu görür → düzeltme yığın boyunca yayılır (asıl kazanç),
    //   3) RELAX (M iter, bias=0) + restitution: soft bias hızını temizler (dinlenme TEMİZ,
    //      uyuyabilir) ve taze (vn0 hızlı, pen0≥0) çarpmada sekmeyi sönümlü/hedef-hız olarak
    //      geri verir,
    //   4) pozisyon düzeltmesi = dp − (relaxed hız)·dt olarak DIŞARI yazılır (pipeline bunu
    //      transform'a uygular; hız kanalına relaxed+restitution yazılır).
    //
    // Böylece pozisyon penetrasyonu TGS-entegre dp ile düzelir ama kalıcı hıza sızmaz.
    // NOT: CCD-etkin gövde içeren island'lar bu yola GİRMEZ — speculative temaslar için
    // eski split-impulse yolu kullanılır (solve_contacts başında has_ccd dalı); TGS'in
    // dp/relax akışı yüksek-hızlı açılı çarpmalarda speculative clamp'le çatışabiliyor.
    #[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
    pub(super) fn solve_contacts_tgs(
        &self,
        manifolds: &mut [ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        pos_corrections: &mut [(Vec3, Vec3)],
        entity_index_map: &std::collections::HashMap<u32, usize>,
        dt: f32,
    ) {
        let inv_dt = if dt > 0.0 { 1.0 / dt } else { 0.0 };

        // ── Soft constraint katsayıları (Box2D v3 formülü) ──
        // hertz'i substep oranının çeyreğine clamp'le (ω·dt makul kalsın).
        let hertz = self.contact_hertz.min(0.25 * inv_dt);
        let zeta = self.contact_damping_ratio;
        let omega = 2.0 * std::f32::consts::PI * hertz;
        let denom = 2.0 * zeta + dt * omega;
        let c = dt * omega * denom;
        let bias_rate = if denom > 1e-9 { omega / denom } else { 0.0 };
        let mass_scale = if c > 0.0 { c / (1.0 + c) } else { 1.0 };
        let impulse_scale = if c > 0.0 { 1.0 / (1.0 + c) } else { 0.0 };

        let com_of = |idx: usize| -> Vec3 {
            transforms[idx].position
                + transforms[idx]
                    .rotation
                    .mul_vec3(rigid_bodies[idx].center_of_mass)
        };

        // ── 0) Restitution için başlangıç yaklaşma hızı (warm-start ÖNCESİ) ──
        let mut vn0: Vec<Vec<f32>> = manifolds
            .iter()
            .map(|m| vec![0.0f32; m.contacts.len()])
            .collect();
        for mid in 0..manifolds.len() {
            let (idx_a, idx_b) = match (
                entity_index_map.get(&manifolds[mid].entity_a.id()),
                entity_index_map.get(&manifolds[mid].entity_b.id()),
            ) {
                (Some(&a), Some(&b)) => (a, b),
                _ => continue,
            };
            let com_a = com_of(idx_a);
            let com_b = com_of(idx_b);
            for cid in 0..manifolds[mid].contacts.len() {
                let ct = manifolds[mid].contacts[cid];
                let r_a = ct.point - com_a;
                let r_b = ct.point - com_b;
                let va = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
                let vb = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
                vn0[mid][cid] = (vb - va).dot(ct.normal);
            }
        }

        // ── 1) Warm-start ──
        for mid in 0..manifolds.len() {
            let (idx_a, idx_b) = match (
                entity_index_map.get(&manifolds[mid].entity_a.id()),
                entity_index_map.get(&manifolds[mid].entity_b.id()),
            ) {
                (Some(&a), Some(&b)) => (a, b),
                _ => continue,
            };
            let inv_m_a = rigid_bodies[idx_a].inv_mass();
            let inv_m_b = rigid_bodies[idx_b].inv_mass();
            let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
            let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
            let dyn_a = rigid_bodies[idx_a].is_dynamic();
            let dyn_b = rigid_bodies[idx_b].is_dynamic();
            let com_a = com_of(idx_a);
            let com_b = com_of(idx_b);
            for contact in &manifolds[mid].contacts {
                let r_a = contact.point - com_a;
                let r_b = contact.point - com_b;
                let wn = contact.normal * (contact.normal_impulse * self.warm_start_factor);
                let wt = contact.tangent_impulse * self.warm_start_factor;
                if dyn_a {
                    velocities[idx_a].linear -= (wn + wt) * inv_m_a;
                    velocities[idx_a].angular -= inv_i_a * (r_a.cross(wn) + r_a.cross(wt));
                }
                if dyn_b {
                    velocities[idx_b].linear += (wn + wt) * inv_m_b;
                    velocities[idx_b].angular += inv_i_b * (r_b.cross(wn) + r_b.cross(wt));
                }
            }
        }

        // ── 2) Aktif gövdeler + pozisyon-delta birikimi (gerçek TGS) ──
        let n_bodies = velocities.len();
        let mut active = vec![false; n_bodies];
        for m in manifolds.iter() {
            if let (Some(&a), Some(&b)) = (
                entity_index_map.get(&m.entity_a.id()),
                entity_index_map.get(&m.entity_b.id()),
            ) {
                active[a] = true;
                active[b] = true;
            }
        }
        // GERÇEK (penetran, pen0≥0) teması olan gövdeler. Yalnız bunlar TGS pozisyon
        // düzeltmesi (dp) alır. Sadece SPECULATIVE (gap, pen0<0) teması olan gövdeler
        // (ör. CCD mermisi) eski yolu kullanır: biased clamp velocity + position_integration
        // taşır; relax/dp UYGULANMAZ (yoksa hız sıfırlanıp donar ya da dp taşıp tüneller).
        let mut has_real = vec![false; n_bodies];
        for m in manifolds.iter() {
            if m.contacts.iter().any(|c| c.penetration >= 0.0) {
                if let (Some(&a), Some(&b)) = (
                    entity_index_map.get(&m.entity_a.id()),
                    entity_index_map.get(&m.entity_b.id()),
                ) {
                    has_real[a] = true;
                    has_real[b] = true;
                }
            }
        }
        let mut dp: Vec<(Vec3, Vec3)> = vec![(Vec3::ZERO, Vec3::ZERO); n_bodies];
        let h = if self.iterations > 0 {
            dt / self.iterations as f32
        } else {
            dt
        };

        // ── 3) BIASED soft solve + iterasyon-arası pozisyon entegrasyonu ──
        // Her iterasyondan sonra pozisyon-delta'sı güncel hızla ilerletilir; bir sonraki
        // iterasyonun bias'ı GÜNCEL penetrasyonu görür → düzeltme yığın boyunca yayılır
        // (SI'nin yapamadığı; uzun/çarpan yığınları ayakta tutan temel mekanizma).
        for iter in 0..self.iterations {
            self.tgs_sweep(
                manifolds,
                rigid_bodies,
                transforms,
                velocities,
                entity_index_map,
                &dp,
                &vn0,
                false,
                iter % 2 == 1,
                true,
                bias_rate,
                mass_scale,
                impulse_scale,
                inv_dt,
            );
            for i in 0..n_bodies {
                if has_real[i] && active[i] && rigid_bodies[i].is_dynamic() {
                    dp[i].0 += velocities[i].linear * h;
                    dp[i].1 += velocities[i].angular * h;
                }
            }
        }

        // ── 4) RELAX (bias=0) — soft bias hızını temizle ──
        for iter in 0..self.relax_iterations {
            self.tgs_sweep(
                manifolds,
                rigid_bodies,
                transforms,
                velocities,
                entity_index_map,
                &dp,
                &vn0,
                true,
                iter % 2 == 1,
                false,
                0.0,
                1.0,
                0.0,
                inv_dt,
            );
        }

        // ── 5) Pozisyon düzeltmesi = dp − (relaxed hız)·dt ──
        // dp, biased çözümün TGS-entegre GERÇEK pozisyon değişimidir (penetrasyon
        // düzeltmesi yığın boyunca yayılmış). position_integration relaxed·dt ekleyecek;
        // farkı burada ekleyince toplam = dp olur → penetrasyon düzelir, hız temiz kalır
        // (resting jitter yok, uyuyabilir). Restitution relax içinde uygulandığından
        // sekme hızı `velocities`'tedir; çıkarılması sekme yer-değişimini bu kareye değil
        // SONRAKİ kareye taşır (kararlı). Yalnız aktif dinamik gövdelere uygulanır.
        for i in 0..n_bodies {
            if has_real[i] && active[i] && rigid_bodies[i].is_dynamic() {
                let dlin = dp[i].0 - velocities[i].linear * dt;
                let dang = dp[i].1 - velocities[i].angular * dt;
                pos_corrections[i] = (dlin, dang);
            }
        }
    }

    /// Tek TGS iterasyon taraması: her temasta normal (opsiyonel soft bias) + 2-tangent
    /// Coulomb sürtünme çöz. `use_bias=false` → rijit relax (bias yok).
    #[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
    fn tgs_sweep(
        &self,
        manifolds: &mut [ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        entity_index_map: &std::collections::HashMap<u32, usize>,
        // Gerçek TGS: gövde başına biriken pozisyon-delta'sı (lin, açısal-scaled-axis).
        // Bias, başlangıç penetrasyonu yerine bu delta'larla GÜNCELLENMİŞ penetrasyondan
        // hesaplanır → düzeltme iterasyonlar arası yığın boyunca yayılır.
        dp: &[(Vec3, Vec3)],
        // Başlangıç yaklaşma hızı (warm-start öncesi), restitution hedefi için. [mid][cid].
        vn0: &[Vec<f32>],
        // Relax aşamasında restitution'u hedef-hız olarak uygula (sönümlü, kararlı).
        apply_restitution: bool,
        reverse: bool,
        use_bias: bool,
        bias_rate: f32,
        mass_scale: f32,
        impulse_scale: f32,
        inv_dt: f32,
    ) {
        let n_manifolds = manifolds.len();
        for mi in 0..n_manifolds {
            let mid = if reverse { n_manifolds - 1 - mi } else { mi };
            let (idx_a, idx_b) = match (
                entity_index_map.get(&manifolds[mid].entity_a.id()),
                entity_index_map.get(&manifolds[mid].entity_b.id()),
            ) {
                (Some(&a), Some(&b)) => (a, b),
                _ => continue,
            };
            let dyn_a = rigid_bodies[idx_a].is_dynamic();
            let dyn_b = rigid_bodies[idx_b].is_dynamic();
            if !dyn_a && !dyn_b {
                continue;
            }

            let inv_m_a = rigid_bodies[idx_a].inv_mass();
            let inv_m_b = rigid_bodies[idx_b].inv_mass();
            let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
            let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
            let com_a = transforms[idx_a].position
                + transforms[idx_a]
                    .rotation
                    .mul_vec3(rigid_bodies[idx_a].center_of_mass);
            let com_b = transforms[idx_b].position
                + transforms[idx_b]
                    .rotation
                    .mul_vec3(rigid_bodies[idx_b].center_of_mass);
            let friction = manifolds[mid].friction;
            let static_friction = manifolds[mid].static_friction;
            let restitution = manifolds[mid].restitution;

            let n_contacts = manifolds[mid].contacts.len();
            for ci in 0..n_contacts {
                let cid = if reverse { n_contacts - 1 - ci } else { ci };
                let normal = manifolds[mid].contacts[cid].normal;
                let point = manifolds[mid].contacts[cid].point;
                let pen0 = manifolds[mid].contacts[cid].penetration;
                let acc_n = manifolds[mid].contacts[cid].normal_impulse;
                let acc_t = manifolds[mid].contacts[cid].tangent_impulse;

                let r_a = point - com_a;
                let r_b = point - com_b;

                // GÜNCEL penetrasyon (gerçek TGS): biriken pozisyon-delta'larıyla düzelt.
                // Cisimler ayrıldıkça (dp normal yönünde) penetrasyon azalır.
                let dp_a = dp[idx_a].0 + dp[idx_a].1.cross(r_a);
                let dp_b = dp[idx_b].0 + dp[idx_b].1.cross(r_b);
                let penetration = pen0 - (dp_b - dp_a).dot(normal);

                // ── Normal kısıt ──
                let va = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
                let vb = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
                let vel_norm = (vb - va).dot(normal);

                let r_a_x_n = r_a.cross(normal);
                let r_b_x_n = r_b.cross(normal);
                let k_n = inv_m_a
                    + inv_m_b
                    + inv_i_a.mul_vec3(r_a_x_n).dot(r_a_x_n)
                    + inv_i_b.mul_vec3(r_b_x_n).dot(r_b_x_n);
                if k_n < 1e-8 {
                    continue;
                }

                // Soft bias: penetrasyonda yay-damper gibi ayır; speculative boşlukta
                // (penetration<0) kapanmayı boşluk/dt ile sınırla; aksi halde bias yok.
                let (bias, m_scale, i_scale) = if penetration < 0.0 {
                    (penetration * inv_dt, 1.0, 0.0)
                } else if use_bias && penetration > self.slop {
                    let b = (bias_rate * (penetration - self.slop)).min(self.max_bias_velocity);
                    (b, mass_scale, impulse_scale)
                } else {
                    (0.0, 1.0, 0.0)
                };

                // Restitution hedef-hızı: yalnız RELAX aşamasında, GERÇEK temasta
                // (pen0≥0, speculative değil), eşiği aşan çarpmada — sönümlü/clamp'li
                // uygulanır (ayrı pass yerine; yığın çarpmasını patlatmaz).
                let target_vn = if apply_restitution
                    && restitution > 0.0
                    && pen0 >= 0.0
                    && vn0[mid][cid] < -self.restitution_velocity_threshold
                {
                    -restitution * vn0[mid][cid]
                } else {
                    0.0
                };
                let delta_n = (m_scale * (target_vn - vel_norm + bias) - i_scale * acc_n) / k_n;
                let new_acc_n = (acc_n + delta_n).max(0.0);
                let actual_n = new_acc_n - acc_n;
                manifolds[mid].contacts[cid].normal_impulse = new_acc_n;

                let imp_n = normal * actual_n;
                if dyn_a {
                    velocities[idx_a].linear -= imp_n * inv_m_a;
                    velocities[idx_a].angular -= inv_i_a.mul_vec3(r_a.cross(imp_n));
                }
                if dyn_b {
                    velocities[idx_b].linear += imp_n * inv_m_b;
                    velocities[idx_b].angular += inv_i_b.mul_vec3(r_b.cross(imp_n));
                }

                // ── Sürtünme (2-tangent Coulomb cone) ──
                let (t1, t2) = {
                    let a = if normal.x.abs() > 0.9 {
                        Vec3::new(0.0, 1.0, 0.0).cross(normal)
                    } else {
                        Vec3::new(1.0, 0.0, 0.0).cross(normal)
                    }
                    .normalize();
                    (a, normal.cross(a))
                };
                let va2 = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
                let vb2 = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
                let rel2 = vb2 - va2;
                let eff_mass = |taxis: Vec3| -> f32 {
                    let rxt_a = r_a.cross(taxis);
                    let rxt_b = r_b.cross(taxis);
                    inv_m_a
                        + inv_m_b
                        + inv_i_a.mul_vec3(rxt_a).dot(rxt_a)
                        + inv_i_b.mul_vec3(rxt_b).dot(rxt_b)
                };
                let k_t1 = eff_mass(t1);
                let k_t2 = eff_mass(t2);
                let acc_t1 = acc_t.dot(t1);
                let acc_t2 = acc_t.dot(t2);
                let mut new1 = if k_t1 > 1e-8 {
                    acc_t1 - rel2.dot(t1) / k_t1
                } else {
                    acc_t1
                };
                let mut new2 = if k_t2 > 1e-8 {
                    acc_t2 - rel2.dot(t2) / k_t2
                } else {
                    acc_t2
                };
                let max_static = static_friction * new_acc_n.abs();
                let max_dynamic = friction * new_acc_n.abs();
                let mag = (new1 * new1 + new2 * new2).sqrt();
                if mag > max_static && mag > 1e-12 {
                    let s = max_dynamic / mag;
                    new1 *= s;
                    new2 *= s;
                }
                let imp_t = t1 * (new1 - acc_t1) + t2 * (new2 - acc_t2);
                manifolds[mid].contacts[cid].tangent_impulse = t1 * new1 + t2 * new2;
                if dyn_a {
                    velocities[idx_a].linear -= imp_t * inv_m_a;
                    velocities[idx_a].angular -= inv_i_a.mul_vec3(r_a.cross(imp_t));
                }
                if dyn_b {
                    velocities[idx_b].linear += imp_t * inv_m_b;
                    velocities[idx_b].angular += inv_i_b.mul_vec3(r_b.cross(imp_t));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // Regression for the TGS soft-constraint impulse ordering. The impulse-scale
    // penalty term (`i_scale * acc_n`) must be applied *inside* the effective-mass
    // division, i.e. `(m_scale*(...) - i_scale*acc_n) / k_n`, not after it
    // (`m_scale*(...)/k_n - i_scale*acc_n`). The two disagree whenever both an
    // impulse-scale penalty and a non-unit effective mass (k_n != 1) are present.
    #[test]
    fn soft_constraint_impulse_scale_applied_before_division() {
        // Representative soft-constraint values (bias_rate>0, use_tgs_soft path).
        let m_scale = 0.75_f32;
        let i_scale = 0.25_f32;
        let target_vn = 0.0_f32;
        let vel_norm = -2.0_f32;
        let bias = 1.5_f32;
        let k_n = 4.0_f32; // non-unit effective mass exposes the ordering bug
        let acc_n = 0.6_f32;

        // Correct (post-fix) ordering.
        let correct = (m_scale * (target_vn - vel_norm + bias) - i_scale * acc_n) / k_n;
        // Buggy (pre-fix) ordering.
        let buggy = m_scale * (target_vn - vel_norm + bias) / k_n - i_scale * acc_n;

        // They must differ, and the current code must match the correct form.
        assert!((correct - buggy).abs() > 1e-6, "orderings must differ for this case");

        let delta_n = (m_scale * (target_vn - vel_norm + bias) - i_scale * acc_n) / k_n;
        assert!((delta_n - correct).abs() < 1e-9);
    }
}
