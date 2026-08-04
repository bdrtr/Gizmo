use super::data::*;
use gizmo_physics_core::components::Transform;
use crate::components::{RigidBody, Velocity};
use gizmo_math::Vec3;

/// Birikmiş-λ yuvaları ([`JointScratch`]).
///
/// Yuvalar DERLEME ZAMANI SABİTİ, ilerleyen bir imleç DEĞİL: satırların çoğu koşullu
/// atlanıyor (`fixed.rs`'te `err_len >= 1e-4`, `hinge.rs`'te `err_mag > 1e-6` ve limit
/// dalları, `slider.rs`'te `err.abs() > 1e-4`, `ball_socket.rs`'teki koni/twist/swing
/// kapıları, `d6.rs`'teki `continue` kolları), bir imleç atlanan her satırda sonraki
/// satırların kimliğini kaydırır ve λ'lar yanlış satıra yazılırdı.
///
/// Bir DOF'un ALT ve ÜST limiti aynı yuvayı paylaşır: iki bağ ama tek serbestlik derecesi,
/// ve bir geçiş boyunca `transforms` DEĞİŞMEDİĞİ için (çözücü `&[Transform]` alır,
/// entegrasyon geçişten sonra) hangi dalın seçildiği 10 iterasyon boyunca sabittir — ters
/// işaretli bayat bir λ miras alınamaz.
pub(crate) mod row {
    /// 0,1,2 — nokta kısıtının X/Y/Z'si, slider'ın iki dik ekseni, D6 lineer DOF'ları.
    pub const LIN: usize = 0;
    /// 3,4,5 — Fixed 3-eksen açısal kilidi, D6 açısal DOF'ları, hinge eksen hizalaması,
    /// slider açısal kilidi, ball-socket koni (3) ve twist (4).
    pub const ANG: usize = 3;
    /// Hinge/slider limiti, distance min|max — hepsi tek DOF'un iki yönlü sınırı.
    pub const LIMIT: usize = 6;
    /// 7,8 — ball-socket asimetrik swing limitleri (perp1, perp2).
    pub const SWING: usize = 7;
    /// 9 — motor / servo satırı. Kopma toplamına KATILMAZ: motor dış yük değil eyleyicidir.
    pub const MOTOR: usize = 9;
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct JointSolver {
    pub iterations: usize,
    pub max_correction_speed: f32,
    pub max_angular_speed: f32,
    pub position_bias: f32,
}

impl Default for JointSolver {
    fn default() -> Self {
        Self {
            iterations: 10,
            max_correction_speed: 5.0,
            max_angular_speed: 5.0,
            position_bias: 0.3,
        }
    }
}

impl JointSolver {
    pub fn new(iterations: usize) -> Self {
        Self {
            iterations,
            ..Default::default()
        }
    }

    pub fn solve_joints(
        &self,
        joints: &mut [Joint],
        entity_index_map: &rustc_hash::FxHashMap<u32, usize>,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        dt: f32,
    ) {
        // Birikmiş λ'lar bir çözücü GEÇİŞİNE (= bir substep) aittir; aşağıdaki iterasyonlar
        // bu birikimi yakınsatır. Geçiş başında sıfırla — döngünün İÇİNDE sıfırlamak tüm
        // değişikliği no-op'a indirir.
        //
        // λ adımlar arasında TAŞINMADIĞI için `WorldSnapshot`'a girmesi gerekmez: rollback
        // restore'undan sonraki ilk `solve_joints` onu zaten sıfırdan kurar. Substep'ler
        // arası warm-start eklendiği gün bu tersine döner ve snapshot'a girmesi ZORUNLU olur
        // (bkz. `PhysicsWorld::WorldSnapshot`'taki contact_cache gerekçesi).
        for joint in joints.iter_mut() {
            joint.scratch = JointScratch::default();
        }

        for _ in 0..self.iterations {
            for joint in joints.iter_mut() {
                if joint.is_broken {
                    continue;
                }

                let idx_a = entity_index_map.get(&joint.entity_a.id()).copied();
                let idx_b = entity_index_map.get(&joint.entity_b.id()).copied();
                let (Some(idx_a), Some(idx_b)) = (idx_a, idx_b) else {
                    continue;
                };
                if idx_a == idx_b {
                    continue;
                }

                // Dispatch on the JointType enum (a Copy value derived from joint.data via
                // the compile-forced From impl), not the &str — so a new JointData variant
                // that forgot a solver case is a compile error, not a silent no-op.
                match JointType::from(&joint.data) {
                    JointType::Fixed => self.solve_fixed_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    JointType::Hinge => self.solve_hinge_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    JointType::BallSocket => self.solve_ball_socket_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    JointType::Slider => self.solve_slider_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    JointType::Distance => self.solve_distance_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    JointType::D6 => self.solve_d6_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    // Spring is force-based (depends on position, not velocity); running it
                    // inside the iteration loop would apply the force ~iterations times.
                    // It is applied once per step outside the loop (see below).
                    JointType::Spring => {}
                }
            }
        }

        // ── Kuvvet-tabanlı eklemler: step başına BİR kez ──────────────────
        // Yay kuvveti pozisyona bağlı olduğundan velocity-solver iterasyonları
        // boyunca sabittir; döngü dışında tek sefer uygulanmalıdır.
        for joint in joints.iter_mut() {
            if joint.is_broken {
                continue;
            }
            let (Some(idx_a), Some(idx_b)) = (
                entity_index_map.get(&joint.entity_a.id()).copied(),
                entity_index_map.get(&joint.entity_b.id()).copied(),
            ) else {
                continue;
            };
            if idx_a == idx_b {
                continue;
            }
            // Force-based contributions: Spring is always force-based; Slider/Hinge carry
            // optional suspension/torsional springs (the solve_*_spring fns no-op if off).
            match JointType::from(&joint.data) {
                JointType::Spring => {
                    self.solve_spring_joint(joint, rigid_bodies, transforms, velocities, idx_a, idx_b, dt)
                }
                JointType::Slider => {
                    self.solve_slider_spring(joint, rigid_bodies, transforms, velocities, idx_a, idx_b, dt)
                }
                JointType::Hinge => {
                    self.solve_hinge_spring(joint, rigid_bodies, transforms, velocities, idx_a, idx_b, dt)
                }
                JointType::D6 => {
                    self.solve_d6_drives(joint, rigid_bodies, transforms, velocities, idx_a, idx_b, dt)
                }
                _ => {}
            }
        }

        // ── Kopma kontrolü: geçiş başına BİR kez, NET tepki üzerinden ─────────
        //
        // Eskiden her joint türü kendi içinde, İTERASYON DÖNGÜSÜNÜN İÇİNDE kontrol
        // ediyordu (8 ayrı yer) ve ölçtüğü şey `Σ|λᵢ|` — satır büyüklüklerinin L1
        // toplamı — idi. Üç ayrı biçimde yanlıştı:
        //   * eş-doğrusal OLMAYAN satırların büyüklüklerini topluyordu: Fixed'in üç dik
        //     lineer satırında bu net tepkiyi √3'e kadar abartır, ball-socket'te
        //     (koni + twist + swing, dik bile değiller) daha da fazla;
        //   * `iterations` ile ölçekleniyordu — `world.joint_solver.iterations` public
        //     bir alan, yani onu değiştirmek sahnedeki HER eşiği sessizce yeniden
        //     ölçekliyordu;
        //   * `fixed.rs`'teki `err_len >= 1e-4` kapısı, kusursuz sabitlenmiş bir kaynağın
        //     lineer kontrolünü tamamen atlıyordu.
        //
        // Artık ölçülen şey geçişin NET impulse vektörü `‖Σ λᵢ·nᵢ‖ / dt` — yani eklemin
        // gerçekten taşıdığı kuvvet/tork. Kuvvet-tabanlı yaylar da (Spring, slider
        // süspansiyonu, hinge torsiyon yayı) bu toplama katkı verir; motorlar/sürücüler
        // VERMEZ, çünkü onlar dış yük değil eyleyicidir (bkz. docs/FIXPLAN.md B4 commit 4).
        for joint in joints.iter_mut() {
            if joint.is_broken {
                continue;
            }
            let force = joint.scratch.impulse_lin.length() / dt;
            let torque = joint.scratch.impulse_ang.length() / dt;
            if joint.check_break(force, torque) {
                tracing::debug!(
                    entity_a = ?joint.entity_a,
                    entity_b = ?joint.entity_b,
                    joint_type = joint.joint_type(),
                    applied_force = force,
                    break_force = joint.break_force,
                    applied_torque = torque,
                    break_torque = joint.break_torque,
                    "Joint broke (net reaction exceeded break threshold)"
                );
            }
        }
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Two unit vectors perpendicular to `v`.
    fn perpendiculars(v: Vec3) -> (Vec3, Vec3) {
        let p1 = if v.x.abs() < 0.9 {
            v.cross(Vec3::X).normalize()
        } else {
            v.cross(Vec3::Y).normalize()
        };
        (p1, v.cross(p1))
    }

    /// World-space lever arm from a body's **centre of mass** to a point.
    ///
    /// Every joint row needs this, and every one of them used to compute
    /// `anchor - transforms[idx].position` — the arm about the transform ORIGIN. The two
    /// agree only when `center_of_mass` is zero. They differ for every compound collider
    /// (`RigidBody::update_inertia_from_collider` derives a shifted COM automatically),
    /// every fracture chunk (`fracture.rs` sets one explicitly) and every vehicle chassis,
    /// so those bodies got the wrong torque and the wrong effective mass from every joint
    /// attached to them.
    ///
    /// This is the same expression `Integrator::apply_impulse_at_point` uses, which is the
    /// convention the contact solver has always followed — the joint path was the outlier.
    #[inline]
    pub(crate) fn lever_arm(
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        idx: usize,
        point: Vec3,
    ) -> Vec3 {
        let t = &transforms[idx];
        let global_com = t.position + t.rotation * rigid_bodies[idx].center_of_mass;
        point - global_com
    }

    /// Birikmiş-λ kırpma: `lambda_min/max` ARTIMA değil, geçiş boyunca birikmiş TOPLAMA
    /// uygulanır. Tek yönlü bir satır (limit / halat / koni) böylece NEGATİF artım da
    /// uygulayabilir — toplam doğru tarafta kaldığı sürece kendi önceki aşırı-düzeltmesini
    /// GERİ ALIR. Eskiden her artım ayrı kırpıldığından geri verme mümkün değildi.
    ///
    /// Uygulanan (geri döndürülen) değer artımdır, birikim değil; hızlar artımla güncellenir.
    #[inline]
    fn accumulate(accum: &mut f32, delta: f32, lambda_min: f32, lambda_max: f32) -> f32 {
        let total = *accum + delta;
        let clamped = total.clamp(lambda_min, lambda_max);
        if clamped == total {
            // Sınır ISIRMADI → artımı OLDUĞU GİBİ uygula. `clamped - *accum` yazsaydık
            // birikim büyüdükçe f32 yuvarlama farkı doğardı. Bu dal sayesinde ±∞ clamp'li
            // ve compliance = 0 olan EŞİTLİK satırları — Fixed, D6 Locked, slider'ın dik
            // eksenleri ve açısal kilidi, hinge eksen hizalaması — bugünküyle BİT-AYNI
            // kalır. Davranış değişimi yalnızca gerçekten bir sınıra dayanan satırlarda.
            *accum = total;
            delta
        } else {
            let applied = clamped - *accum;
            *accum = clamped;
            applied
        }
    }

    /// Apply a 1-DOF angular velocity constraint along `direction` (hard).
    /// `error` is the positional error in radians (positive = bodies need to rotate apart).
    #[allow(clippy::too_many_arguments)]
    fn apply_angular_constraint(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        error: f32,
        dt: f32,
        lambda_min: f32,
        lambda_max: f32,
        scratch: &mut JointScratch,
        slot: usize,
    ) -> f32 {
        self.apply_angular_constraint_soft(
            rigid_bodies, transforms, velocities, idx_a, idx_b, direction, error, dt, lambda_min,
            lambda_max, 0.0, scratch, slot,
        )
    }

    /// Soft (compliant) form of [`Self::apply_angular_constraint`]. `compliance` ≥ 0 is the
    /// inverse stiffness (CFM): the effective mass is regularised by `compliance/dt²`, so a
    /// larger value yields a springier constraint that gives under load (0 = fully rigid,
    /// identical to the hard path). Lets a specific limit/weld be soft without changing the
    /// global Baumgarte factor.
    #[allow(clippy::too_many_arguments)]
    fn apply_angular_constraint_soft(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        error: f32,
        dt: f32,
        lambda_min: f32,
        lambda_max: f32,
        compliance: f32,
        scratch: &mut JointScratch,
        slot: usize,
    ) -> f32 {
        if direction.length_squared() < 1e-10 {
            return 0.0;
        }

        let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
        let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
        let w_a = velocities[idx_a].angular;
        let w_b = velocities[idx_b].angular;
        let dyn_a = rigid_bodies[idx_a].is_dynamic();
        let dyn_b = rigid_bodies[idx_b].is_dynamic();

        let k = direction.dot(inv_i_a.mul_vec3(direction)) + direction.dot(inv_i_b.mul_vec3(direction));
        if k < 1e-10 {
            return 0.0;
        }
        let k = k + compliance / (dt * dt); // CFM regularisation (0 ⇒ rigid)

        let vel_err = (w_b - w_a).dot(direction);
        let position_bias = (self.position_bias * error / dt)
            .clamp(-self.max_angular_speed, self.max_angular_speed);
        // NOT: burada XPBD/CFM geri beslemesi (`- cfm * *accum`) KASITLI OLARAK YOK.
        // Doğru terim odur — yumuşak bir satırın denge noktası Jv + α̃·λ_toplam = bias'tır —
        // ama `position_bias` bu çözücüde `max_correction_speed`/`max_angular_speed` ile
        // HIZ-KIRPILI. Kırpma ısırdığı anda denge λ_toplam = bias_max/α̃ değerine tavanlanır;
        // bu, taşınması gereken yükün çok altında kalır ve kısıt sessizce boşalır. Ölçüldü:
        // compliance=0.03, 1 kg yük, dt=1/240 → 2 m'lik halat 600 adımda 27.4 m'ye uzuyor
        // (yani serbest düşüş), oysa doğru statik uzama α·m·g/β = 0.98 m. Kırpmayı 5000'e
        // çekince ölçüm 1.007 m — terim doğru, kırpmayla ETKİLEŞİMİ yanlış.
        // Bu yüzden compliance'ın iterasyon-sayısına bağımlılığı burada KAPANMIYOR; kırpma
        // rejimiyle birlikte ele alınacak (bkz. docs/FIXPLAN.md, B4 sonrası).
        let delta = (-vel_err + position_bias) / k;
        let lambda = Self::accumulate(scratch.row(slot), delta, lambda_min, lambda_max);
        // Geçişin NET açısal impulse'ı — `break_torque` bundan hesaplanır. Artımların
        // VEKTÖR toplamı: eş-doğrusal olmayan satırların büyüklüklerini toplamak (eski
        // `.abs()` yığını) taşınan torku Fixed'de √3'e kadar abartıyordu.
        scratch.impulse_ang += direction * lambda;

        let delta_a = inv_i_a.mul_vec3(direction) * lambda;
        let delta_b = inv_i_b.mul_vec3(direction) * lambda;

        if idx_a < idx_b {
            let (l, r) = velocities.split_at_mut(idx_b);
            if dyn_a {
                l[idx_a].angular -= delta_a;
            }
            if dyn_b {
                r[0].angular += delta_b;
            }
        } else {
            let (l, r) = velocities.split_at_mut(idx_a);
            if dyn_b {
                l[idx_b].angular += delta_b;
            }
            if dyn_a {
                r[0].angular -= delta_a;
            }
        }
        lambda
    }

    /// Apply a 1-DOF linear velocity constraint along `direction` at the anchor points (hard).
    #[allow(clippy::too_many_arguments)]
    fn apply_linear_constraint(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        r_a: Vec3,
        r_b: Vec3,
        error: f32,
        dt: f32,
        lambda_min: f32,
        lambda_max: f32,
        scratch: &mut JointScratch,
        slot: usize,
    ) -> f32 {
        self.apply_linear_constraint_soft(
            rigid_bodies, transforms, velocities, idx_a, idx_b, direction, r_a, r_b, error, dt,
            lambda_min, lambda_max, 0.0, scratch, slot,
        )
    }

    /// Soft (compliant) form of [`Self::apply_linear_constraint`]. See
    /// [`Self::apply_angular_constraint_soft`] — `compliance/dt²` regularises the effective
    /// mass (0 ⇒ rigid).
    #[allow(clippy::too_many_arguments)]
    fn apply_linear_constraint_soft(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        r_a: Vec3,
        r_b: Vec3,
        error: f32,
        dt: f32,
        lambda_min: f32,
        lambda_max: f32,
        compliance: f32,
        scratch: &mut JointScratch,
        slot: usize,
    ) -> f32 {
        let inv_m_a = rigid_bodies[idx_a].inv_mass();
        let inv_m_b = rigid_bodies[idx_b].inv_mass();
        let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
        let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
        let v_a = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
        let v_b = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
        let dyn_a = rigid_bodies[idx_a].is_dynamic();
        let dyn_b = rigid_bodies[idx_b].is_dynamic();

        // Efektif kütlenin açısal terimi: Jacobian açısal kısmı (r×n) olmak üzere
        // k_ang = (r×n)·I⁻¹·(r×n). (Eskiden ((I⁻¹ r)×n)×r·n hesaplanıyordu — farklı bir
        // nicelik; merkez-dışı ankor + anizotropik atalette yanlış impulse büyüklüğü.)
        let rxn_a = r_a.cross(direction);
        let rxn_b = r_b.cross(direction);
        let k = inv_m_a
            + inv_m_b
            + inv_i_a.mul_vec3(rxn_a).dot(rxn_a)
            + inv_i_b.mul_vec3(rxn_b).dot(rxn_b);
        if k < 1e-10 {
            return 0.0;
        }
        let k = k + compliance / (dt * dt); // CFM regularisation (0 ⇒ rigid)

        let rel_vel = (v_b - v_a).dot(direction);
        let position_bias = (self.position_bias * error / dt)
            .clamp(-self.max_correction_speed, self.max_correction_speed);
        // CFM geri beslemesi burada da yok — gerekçe apply_angular_constraint_soft'ta.
        let delta = (-rel_vel + position_bias) / k;
        let lambda = Self::accumulate(scratch.row(slot), delta, lambda_min, lambda_max);
        // Geçişin NET doğrusal impulse'ı — `break_force` bundan hesaplanır (bkz. açısal eş).
        scratch.impulse_lin += direction * lambda;

        let impulse = direction * lambda;

        if idx_a < idx_b {
            let (l, r) = velocities.split_at_mut(idx_b);
            if dyn_a {
                l[idx_a].linear -= impulse * inv_m_a;
                l[idx_a].angular -= inv_i_a.mul_vec3(r_a.cross(impulse));
            }
            if dyn_b {
                r[0].linear += impulse * inv_m_b;
                r[0].angular += inv_i_b.mul_vec3(r_b.cross(impulse));
            }
        } else {
            let (l, r) = velocities.split_at_mut(idx_a);
            if dyn_b {
                l[idx_b].linear += impulse * inv_m_b;
                l[idx_b].angular += inv_i_b.mul_vec3(r_b.cross(impulse));
            }
            if dyn_a {
                r[0].linear -= impulse * inv_m_a;
                r[0].angular -= inv_i_a.mul_vec3(r_a.cross(impulse));
            }
        }
        lambda
    }

    // ── joint solvers ─────────────────────────────────────────────────────────

}

// god-file Tier 3 round-2 bölmesi: per-joint çözücüler joint_types alt-modülünde
mod joint_types;

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_core::BodyHandle;

    #[test]
    fn test_joint_creation() {
        let e1 = BodyHandle::from_id(1);
        let e2 = BodyHandle::from_id(2);
        let joint = Joint::fixed(e1, e2, Vec3::ZERO, Vec3::ZERO);
        assert_eq!(joint.joint_type(), "Fixed");
        assert!(!joint.is_broken);
    }

    #[test]
    fn test_hinge_joint() {
        let e1 = BodyHandle::from_id(1);
        let e2 = BodyHandle::from_id(2);
        let joint = Joint::hinge(e1, e2, Vec3::ZERO, Vec3::ZERO, Vec3::Y);
        assert_eq!(joint.joint_type(), "Hinge");
        if let JointData::Hinge(data) = joint.data {
            assert_eq!(data.axis, Vec3::Y);
        } else {
            panic!("expected hinge data");
        }
    }

    #[test]
    fn test_spring_joint() {
        let e1 = BodyHandle::from_id(1);
        let e2 = BodyHandle::from_id(2);
        let joint = Joint::spring(e1, e2, Vec3::ZERO, Vec3::ZERO, 1.0, 100.0, 10.0);
        if let JointData::Spring(data) = joint.data {
            assert_eq!(data.stiffness, 100.0);
            assert_eq!(data.damping, 10.0);
        } else {
            panic!("expected spring data");
        }
    }

    /// 1-DOF doğrusal hız kısıtı, DOĞRU efektif kütleyle tek uygulamada ankor
    /// noktalarındaki bağıl hızı tam olarak sıfırlar (λ = -Jv/k, yeni Jv = Jv + kλ = 0).
    /// Yanlış `k` ile (eski `((I⁻¹r)×n)×r·n`) over/undershoot olur ve bağıl hız ≠ 0 kalır;
    /// bu test bu yüzden doğru çapraz-çarpım sırasını ayırt eder.
    #[test]
    fn linear_constraint_zeroes_relative_velocity_with_correct_effective_mass() {
        let solver = JointSolver::default();

        let body = || {
            let mut rb = RigidBody::new(1.0, false);
            rb.local_inertia = Vec3::new(2.0, 5.0, 8.0); // anizotropik atalet
            rb
        };
        let bodies = [body(), body()];
        let transforms = [Transform::new(Vec3::ZERO), Transform::new(Vec3::ZERO)];
        let mut vels = [
            Velocity::default(),
            Velocity::new(Vec3::new(0.0, 1.0, 0.0)), // B ankora göre Y'de bağıl hız
        ];

        // Merkez-dışı ankorlar (bug bu durumda ortaya çıkar).
        let r_a = Vec3::new(0.3, 0.0, 0.0);
        let r_b = Vec3::new(-0.2, 0.1, 0.0);
        let direction = Vec3::Y;

        solver.apply_linear_constraint(
            &bodies,
            &transforms,
            &mut vels,
            0,
            1,
            direction,
            r_a,
            r_b,
            0.0, // pozisyon hatası yok → saf hız kısıtı
            1.0 / 60.0,
            f32::NEG_INFINITY,
            f32::INFINITY,
            &mut JointScratch::default(),
            row::LIN,
        );

        let v_a = vels[0].linear + vels[0].angular.cross(r_a);
        let v_b = vels[1].linear + vels[1].angular.cross(r_b);
        let rel_n = (v_b - v_a).dot(direction);
        assert!(
            rel_n.abs() < 1e-5,
            "tek uygulamada bağıl hız sıfırlanmalı; kalan = {rel_n} (yanlış efektif kütle?)"
        );
    }

    /// İki eşit kütleli cisim, ankorları kütle merkezinde (r = 0 → k = 1/m + 1/m = 2),
    /// tek yönlü (yalnız ÇEKEN) bir satır. Aradaki tek fark clamp'in nereye uygulandığı.
    fn one_sided_pair() -> ([RigidBody; 2], [Transform; 2], [Velocity; 2]) {
        let body = || RigidBody::new(1.0, false);
        (
            [body(), body()],
            [Transform::new(Vec3::ZERO), Transform::new(Vec3::ZERO)],
            [Velocity::default(), Velocity::default()],
        )
    }

    /// Tek yönlü bir satır kendi ÖNCEKİ impulse'ını GERİ VEREBİLMELİ.
    ///
    /// Eskiden clamp her iterasyonun kendi artımına uygulanıyordu: yalnız-çeken bir satırda
    /// (`lambda_max = 0`) pozitif bir artım her seferinde 0'a kırpıldığından satır, kendi
    /// aşırı-düzeltmesini geri alamıyordu — tek yönlü bir cırcır. Clamp artık geçiş boyunca
    /// birikmiş TOPLAMA uygulanıyor, dolayısıyla toplam doğru tarafta kaldığı sürece
    /// negatif/pozitif her artım uygulanabilir.
    ///
    /// Ayırt edici: eski kodda ikinci çağrı hiçbir şey uygulamaz ve bağıl hız −1.0'da kalır.
    #[test]
    fn a_one_sided_row_can_return_the_impulse_it_applied() {
        let solver = JointSolver::default();
        let (bodies, transforms, mut vels) = one_sided_pair();
        let mut scratch = JointScratch::default();

        // 1) Cisimler ayrılıyor (bağıl hız +1) → yalnız-çeken satır onları yakalar.
        vels[1].linear = Vec3::Y;
        let first = solver.apply_linear_constraint(
            &bodies, &transforms, &mut vels, 0, 1, Vec3::Y, Vec3::ZERO, Vec3::ZERO,
            0.0, // pozisyon hatası yok → saf hız kısıtı
            1.0 / 60.0,
            f32::NEG_INFINITY,
            0.0, // yalnız çek
            &mut scratch,
            row::LIMIT,
        );
        assert!(first < 0.0, "çeken satır negatif λ uygulamalı, uyguladığı = {first}");
        let rel = |v: &[Velocity; 2]| (v[1].linear - v[0].linear).dot(Vec3::Y);
        assert!(rel(&vels).abs() < 1e-6, "ilk çağrı bağıl hızı sıfırlamalı: {}", rel(&vels));

        // 2) Başka bir satır (burada elle) cisimleri BİRBİRİNE yaklaştırıyor. Satırın artık
        //    daha az çekmesi gerekiyor: doğru davranış, uyguladığının bir kısmını geri vermek.
        vels[0].linear = Vec3::ZERO;
        vels[1].linear = -Vec3::Y;
        assert!((rel(&vels) - (-1.0)).abs() < 1e-6);

        let second = solver.apply_linear_constraint(
            &bodies, &transforms, &mut vels, 0, 1, Vec3::Y, Vec3::ZERO, Vec3::ZERO, 0.0,
            1.0 / 60.0,
            f32::NEG_INFINITY,
            0.0,
            &mut scratch,
            row::LIMIT,
        );
        assert!(
            second > 0.0,
            "satır kendi impulse'ını geri vermeli (pozitif artım); uyguladığı = {second} \
             — iterasyon-başına clamp'te bu 0'a kırpılırdı"
        );
        assert!(
            rel(&vels).abs() < 1e-6,
            "geri verdikten sonra bağıl hız yine sıfır olmalı, kalan = {}",
            rel(&vels)
        );
    }

    /// …ama uyguladığından FAZLASINI geri veremez: biriken toplam sınırı geçemez, yani
    /// yalnız-çeken bir satır hiçbir koşulda İTMEZ.
    #[test]
    fn a_one_sided_row_never_pushes_past_its_bound() {
        let solver = JointSolver::default();
        let (bodies, transforms, mut vels) = one_sided_pair();
        let mut scratch = JointScratch::default();
        let dt = 1.0 / 60.0;
        let args = (Vec3::Y, Vec3::ZERO, Vec3::ZERO, 0.0f32);

        vels[1].linear = Vec3::Y;
        solver.apply_linear_constraint(
            &bodies, &transforms, &mut vels, 0, 1, args.0, args.1, args.2, args.3, dt,
            f32::NEG_INFINITY, 0.0, &mut scratch, row::LIMIT,
        );
        let applied_total = *scratch.row(row::LIMIT);
        assert!(applied_total < 0.0);

        // Cisimler artık HIZLA yaklaşıyor: satır bunu düzeltmeye çalışsa iterek yapardı.
        vels[0].linear = Vec3::ZERO;
        vels[1].linear = Vec3::Y * -3.0;
        solver.apply_linear_constraint(
            &bodies, &transforms, &mut vels, 0, 1, args.0, args.1, args.2, args.3, dt,
            f32::NEG_INFINITY, 0.0, &mut scratch, row::LIMIT,
        );

        let total = *scratch.row(row::LIMIT);
        assert_eq!(total, 0.0, "biriken toplam üst sınırda durmalı, durduğu = {total}");
        assert!(
            (vels[1].linear - vels[0].linear).dot(Vec3::Y) < 0.0,
            "yalnız-çeken satır cisimleri AYIRMAMALI; bağıl hız = {}",
            (vels[1].linear - vels[0].linear).dot(Vec3::Y)
        );
    }

    /// Birikim bir GEÇİŞE ait: `solve_joints` her çağrıda sıfırdan başlamalı. Sıfırlama
    /// olmasa ikinci geçiş, birincinin doymuş λ'sını miras alır ve aynı girdiye farklı
    /// cevap verir — adımlar arası sessiz bir durum sızıntısı.
    #[test]
    fn accumulated_lambda_does_not_leak_between_passes() {
        use crate::joints::data::Joint;
        use gizmo_physics_core::BodyHandle;

        let solver = JointSolver::default();
        let mut bodies = [RigidBody::new(1.0, false), RigidBody::new(1.0, false)];
        for rb in &mut bodies {
            rb.local_inertia = Vec3::splat(1.0);
        }
        let transforms = [
            Transform::new(Vec3::ZERO),
            Transform::new(Vec3::new(0.0, -3.0, 0.0)), // halat gergin (max 2.0)
        ];
        let map: rustc_hash::FxHashMap<u32, usize> =
            [(1u32, 0usize), (2u32, 1usize)].into_iter().collect();
        let fresh = || {
            vec![Joint::rope(
                BodyHandle::from_id(1),
                BodyHandle::from_id(2),
                Vec3::ZERO,
                Vec3::ZERO,
                2.0,
            )]
        };
        let start = [Velocity::default(), Velocity::new(Vec3::new(0.0, -4.0, 0.0))];

        // Tek geçiş.
        let mut joints = fresh();
        let mut v1 = start;
        solver.solve_joints(&mut joints, &map, &bodies, &transforms, &mut v1, 1.0 / 60.0);
        let lambda_after_one = joints[0].scratch;

        // Aynı eklem üzerinde İKİ geçiş, ikincisi aynı başlangıç hızlarıyla.
        let mut v2 = start;
        solver.solve_joints(&mut joints, &map, &bodies, &transforms, &mut v2, 1.0 / 60.0);
        v2 = start;
        solver.solve_joints(&mut joints, &map, &bodies, &transforms, &mut v2, 1.0 / 60.0);

        assert_eq!(
            joints[0].scratch, lambda_after_one,
            "aynı girdiyle ikinci geçiş aynı λ'yı üretmeli; birikim geçişler arasında taşınmış"
        );
        assert_eq!(v2, v1, "…ve dolayısıyla aynı hızları");
    }

    #[test]
    fn test_perpendiculars_orthogonality() {
        let v = Vec3::new(1.0, 0.0, 0.0);
        let (p1, p2) = JointSolver::perpendiculars(v);
        assert!(p1.dot(v).abs() < 1e-5);
        assert!(p2.dot(v).abs() < 1e-5);
        assert!(p1.dot(p2).abs() < 1e-5);
    }
}
