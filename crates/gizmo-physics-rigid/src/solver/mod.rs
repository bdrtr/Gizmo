use crate::components::{RigidBody, Velocity};
use gizmo_math::Vec3;
use gizmo_physics_core::components::Transform;
/// AAA Constraint Solver — Sequential Impulses (SI) with:
/// - Warm-starting (önceki frame'in impulslarını uygula)
/// - Accumulated-impulse clamping (negatif normal impulse engeli)
/// - Coulomb friction cone (statik + dinamik)
/// - Speculative contacts (penetrasyon öncesi temas)
/// - 2-boyutlu sürtünme (iki tangent yönü)
/// - Restitution threshold (micro-bounce önleme)
/// - Solver iteration sayısı konfigüre edilebilir
use gizmo_physics_core::ContactManifold;

mod standalone;
mod tgs;

// ─────────────────────────────────────────────────────────────────────────────
// Konfigürasyon
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
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
    /// (TGS Soft açıkken kullanılmaz; eski yol fallback olarak kalır.)
    pub split_impulse_enabled: bool,
    /// Split Impulse penetrasyon düzeltme oranı (0.1..0.4 arası ideal)
    pub split_impulse_erp: f32,

    // ── TGS Soft (modern çözücü) ──────────────────────────────────────────
    /// TGS Soft yolunu kullan (soft constraint + relax pass). Açıkken split-impulse
    /// devre dışı kalır; pozisyon düzeltmesi soft bias'ın velocity katkısından
    /// (biased−relaxed)·dt olarak `pos_corrections`'a yazılır. Yüksek-enerji çarpan
    /// uzun yığınları (n≥16) SI'nin çözemediği yerde kararlı tutar.
    pub use_tgs_soft: bool,
    /// Temas yumuşaklığı frekansı (Hz). Yüksek = daha sert/az gömülme; düşük = daha
    /// yumuşak/kararlı. Box2D v3 varsayılanı 30 Hz. Substep oranına da clamp'lenir.
    pub contact_hertz: f32,
    /// Temas sönümleme oranı (ζ). >1 aşırı-sönümlü (sekmesiz, kararlı yığın). ~10.
    pub contact_damping_ratio: f32,
    /// Relax (bias=0) iterasyon sayısı — soft bias'ın enjekte ettiği hızı temizler.
    pub relax_iterations: usize,
    /// Maksimum soft-bias hızı (m/s) — derin gömülmede pozisyon düzeltme hızını
    /// sınırlar (patlama yok). Box2D v3 ≈ 3·lengthUnits.
    pub max_bias_velocity: f32,
}

impl Default for ConstraintSolver {
    fn default() -> Self {
        Self {
            iterations: 20,
            baumgarte: 0.15,
            slop: 0.005,
            warm_start_factor: 0.85,
            restitution_velocity_threshold: 1.0,
            max_linear_correction: 0.02,
            split_impulse_enabled: true,
            split_impulse_erp: 0.1,
            use_tgs_soft: true,
            contact_hertz: 30.0,
            contact_damping_ratio: 10.0,
            relax_iterations: 4,
            max_bias_velocity: 4.0,
        }
    }
}

impl ConstraintSolver {
    pub fn new(iterations: usize) -> Self {
        Self {
            iterations,
            ..Default::default()
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ANA SOLVER: Manifold listesi üzerinde PGS (Projected Gauss-Seidel)
    // ─────────────────────────────────────────────────────────────────────────

    /// `pos_corrections` (uzunluğu `velocities` ile aynı), split-impulse pozisyon
    /// düzeltmesini gövde başına (lineer Δkonum, açısal Δ-scaled-axis) olarak DIŞARI
    /// yazar. Eskiden bu düzeltme doğrudan `velocities`'e ekleniyordu; bu, pozisyon
    /// düzeltme hızının kalıcı hıza sızmasına (resting jitter / cisimlerin uyumaması)
    /// yol açıyordu. Çağıran bu deltaları pozisyona uygulamalıdır.
    // İndeks-tabanlı döngüler kasıtlı: `mid`/`cid` aynı anda paralel dizileri
    // (manifolds + rigid_bodies/transforms/velocities/pseudo_vel, hepsi entity
    // indeksiyle hizalı) okuyup `manifolds[mid].contacts[cid]` impulslarını geri
    // yazıyor. iter_mut'a çevirmek split-borrow gymnastics gerektirir ve bu
    // determinizm-kritik PGS yolunda fayda sağlamaz.
    #[allow(clippy::needless_range_loop)]
    pub fn solve_contacts(
        &self,
        manifolds: &mut [ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        pos_corrections: &mut [(Vec3, Vec3)],
        entity_index_map: &std::collections::HashMap<u32, usize>,
        dt: f32,
    ) {
        // Pozisyon düzeltme buffer'ını sıfırla (çağıran tarafından yeniden kullanılabilir).
        for pc in pos_corrections.iter_mut() {
            *pc = (Vec3::ZERO, Vec3::ZERO);
        }
        if manifolds.is_empty() {
            return;
        }

        // TGS Soft yolu (modern çözücü): soft constraint + relax pass.
        // İSTİSNA: CCD-etkin gövde içeren island'lar eski (split-impulse) yolu kullanır.
        // CCD speculative temasları ince ayarlıdır; TGS'in dp/relax akışı yüksek-hızlı
        // açılı çarpmalarda speculative clamp'le çatışıp tünellemeye yol açabiliyor.
        // CCD cisimleri (mermiler) nadir ve genelde izole; yığın kararlılığı TGS'te kalır.
        let has_ccd = manifolds.iter().any(|m| {
            [m.entity_a, m.entity_b].iter().any(|e| {
                entity_index_map
                    .get(&e.id())
                    .is_some_and(|&i| rigid_bodies[i].ccd_enabled)
            })
        });
        if self.use_tgs_soft && !has_ccd {
            self.solve_contacts_tgs(
                manifolds,
                rigid_bodies,
                transforms,
                velocities,
                pos_corrections,
                entity_index_map,
                dt,
            );
            return;
        }

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
            let dyn_a = rigid_bodies[idx_a].is_dynamic();
            let dyn_b = rigid_bodies[idx_b].is_dynamic();

            let com_a = transforms[idx_a].position
                + transforms[idx_a]
                    .rotation
                    .mul_vec3(rigid_bodies[idx_a].center_of_mass);
            let com_b = transforms[idx_b].position
                + transforms[idx_b]
                    .rotation
                    .mul_vec3(rigid_bodies[idx_b].center_of_mass);

            for contact in &manifolds[mid].contacts {
                let r_a = contact.point - com_a;
                let r_b = contact.point - com_b;

                let wn = contact.normal * (contact.normal_impulse * self.warm_start_factor);
                let wt = contact.tangent_impulse * self.warm_start_factor;

                if dyn_a {
                    velocities[idx_a].linear -= wn * inv_m_a;
                    velocities[idx_a].linear -= wt * inv_m_a;
                    velocities[idx_a].angular -= inv_i_a * (r_a.cross(wn) + r_a.cross(wt));
                }
                if dyn_b {
                    velocities[idx_b].linear += wn * inv_m_b;
                    velocities[idx_b].linear += wt * inv_m_b;
                    velocities[idx_b].angular += inv_i_b * (r_b.cross(wn) + r_b.cross(wt));
                }
            }
        }

        // ── İteratif PGS ─────────────────────────────────────────────────────
        let inv_dt = if dt > 0.0 { 1.0 / dt } else { 0.0 };

        let n_manifolds = manifolds.len();
        for iter in 0..self.iterations {
            // Symmetric Gauss-Seidel: alternate the sweep direction every
            // iteration. Plain forward-only PGS applies the manifold's contact
            // points in a fixed order; each point's impulse is off-centre, so the
            // transient bias never fully cancels and a *perfectly symmetric* impact
            // (e.g. an axis-aligned box stack) picks up spurious angular velocity
            // that tips and collapses tall stacks. Reversing on odd iterations
            // cancels the directional bias and keeps such stacks upright. The
            // order is a deterministic function of `iter`, so determinism holds.
            let reverse = iter % 2 == 1;
            for mi in 0..n_manifolds {
                let mid = if reverse { n_manifolds - 1 - mi } else { mi };
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

                let friction = manifolds[mid].friction;
                let restitution = manifolds[mid].restitution;

                let n_contacts = manifolds[mid].contacts.len();
                for ci in 0..n_contacts {
                    let cid = if reverse { n_contacts - 1 - ci } else { ci };
                    let contact_pt = manifolds[mid].contacts[cid].point;
                    let normal = manifolds[mid].contacts[cid].normal;
                    let penetration = manifolds[mid].contacts[cid].penetration;
                    let acc_n = manifolds[mid].contacts[cid].normal_impulse;
                    let acc_t = manifolds[mid].contacts[cid].tangent_impulse;

                    let com_a = transforms[idx_a].position
                        + transforms[idx_a]
                            .rotation
                            .mul_vec3(rigid_bodies[idx_a].center_of_mass);
                    let com_b = transforms[idx_b].position
                        + transforms[idx_b]
                            .rotation
                            .mul_vec3(rigid_bodies[idx_b].center_of_mass);
                    let r_a = contact_pt - com_a;
                    let r_b = contact_pt - com_b;

                    let inv_m_a = rigid_bodies[idx_a].inv_mass();
                    let inv_m_b = rigid_bodies[idx_b].inv_mass();
                    let inv_i_a =
                        rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
                    let inv_i_b =
                        rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
                    let dyn_a = rigid_bodies[idx_a].is_dynamic();
                    let dyn_b = rigid_bodies[idx_b].is_dynamic();

                    if !dyn_a && !dyn_b {
                        continue;
                    }

                    // Temas noktasındaki göreli hız
                    let va = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
                    let vb = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
                    let rel_vel = vb - va;
                    let vel_norm = rel_vel.dot(normal);

                    // ── Normal İmpuls ────────────────────────────────────────
                    let r_a_x_n = r_a.cross(normal);
                    let r_b_x_n = r_b.cross(normal);
                    let k_n = inv_m_a
                        + inv_m_b
                        + (inv_i_a.mul_vec3(r_a_x_n)).dot(r_a_x_n)
                        + (inv_i_b.mul_vec3(r_b_x_n)).dot(r_b_x_n);

                    if k_n < 1e-8 {
                        continue;
                    }

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
                        let correction = (penetration - self.slop)
                            .max(0.0)
                            .min(self.max_linear_correction);
                        self.baumgarte * inv_dt * correction
                    };

                    // Restitution: sadece yüksek hızlı GERÇEK çarpışmalarda. Speculative
                    // temas (penetration < 0) bir boşluk-kapatma LİMİTİdir; ona restitution
                    // uygulamak bias'ı bozar (cisim substep'ler arası tutarsız yavaşlar ve
                    // son substep'te yüzeyi aşıp girer). Sekme, cisim gerçekten değdiğinde
                    // (penetration ≥ 0) doğal olarak uygulanır.
                    let e = if penetration < 0.0 {
                        0.0
                    } else if -vel_norm > self.restitution_velocity_threshold {
                        restitution
                    } else {
                        0.0
                    };

                    let delta_n = (-(1.0 + e) * vel_norm + bias) / k_n;
                    let new_acc_n = (acc_n + delta_n).max(0.0); // Clamp: çekme yok
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

                    // ── Sürtünme İmpulsu (2-tangent Coulomb cone) ───────────
                    // Normalden türetilen SABİT iki ortonormal tangent (t1,t2); birikim
                    // her eksende skaler ve birlikte dairesel koniye clamp'lenir.
                    // (Eski tek-tangent yöntemi tangenti her iterasyonda döndürüp birikmiş
                    // impulsun dik bileşenini kaybediyordu → kayıplı/yön kayan sürtünme.)
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

                    // Eksen başına efektif kütle: k = inv_m + (r×t)·I⁻¹·(r×t).
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

                    // Birikmiş tangent impulsu sabit baza ayrıştır, her eksende çöz.
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

                    // Dairesel Coulomb koni: |(new1,new2)| ≤ μ_s·N ise statik; aşarsa μ_d·N'e ölçekle.
                    let max_static = manifolds[mid].static_friction * new_acc_n.abs();
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

        // ── Split Impulse: Pozisyon Düzeltme Pass ────────────────────────────
        // Asıl velocity'den bağımsız olarak pseudo-velocity hesaplar.
        // Bu pass penetrasyon düzeltmesini velocity kanalından ayırır.
        // Birikimli pseudo-impulse takibi ile over-correction engellenir.
        if self.split_impulse_enabled {
            // Per-contact birikimli pseudo-impulse (PGS clamping için)
            let mut acc_pseudo: Vec<Vec<f32>> = manifolds
                .iter()
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
                    let inv_i_a =
                        rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
                    let inv_i_b =
                        rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
                    let dyn_a = rigid_bodies[idx_a].is_dynamic();
                    let dyn_b = rigid_bodies[idx_b].is_dynamic();

                    if !dyn_a && !dyn_b {
                        continue;
                    }

                    let com_a = transforms[idx_a].position
                        + transforms[idx_a]
                            .rotation
                            .mul_vec3(rigid_bodies[idx_a].center_of_mass);
                    let com_b = transforms[idx_b].position
                        + transforms[idx_b]
                            .rotation
                            .mul_vec3(rigid_bodies[idx_b].center_of_mass);

                    for cid in 0..manifolds[mid].contacts.len() {
                        let contact_pt = manifolds[mid].contacts[cid].point;
                        let normal = manifolds[mid].contacts[cid].normal;
                        let penetration = manifolds[mid].contacts[cid].penetration;

                        let correction = (penetration - self.slop)
                            .max(0.0)
                            .min(self.max_linear_correction);
                        if correction < 1e-6 {
                            continue;
                        }

                        let r_a = contact_pt - com_a;
                        let r_b = contact_pt - com_b;

                        let r_a_x_n = r_a.cross(normal);
                        let r_b_x_n = r_b.cross(normal);
                        let k_n = inv_m_a
                            + inv_m_b
                            + (inv_i_a.mul_vec3(r_a_x_n)).dot(r_a_x_n)
                            + (inv_i_b.mul_vec3(r_b_x_n)).dot(r_b_x_n);
                        if k_n < 1e-8 {
                            continue;
                        }

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
                            pseudo_vel[idx_a].1 -= inv_i_a.mul_vec3(r_a.cross(imp_p));
                        }
                        if dyn_b {
                            pseudo_vel[idx_b].0 += imp_p * inv_m_b;
                            pseudo_vel[idx_b].1 += inv_i_b.mul_vec3(r_b.cross(imp_p));
                        }
                    }
                }
            }

            // Pseudo-velocity'yi HIZA EKLEME (eski hata buydu). Bunun yerine pozisyon
            // düzeltmesi olarak dışarı yaz: Δkonum = pseudo_vel * dt. Çağıran bunu
            // doğrudan transform'a uygular; hız kanalı temiz kalır.
            for i in 0..velocities.len() {
                pos_corrections[i] = (pseudo_vel[i].0 * dt, pseudo_vel[i].1 * dt);
            }
        }
    }
}
