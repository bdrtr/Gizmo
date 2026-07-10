use super::data::*;
use gizmo_physics_core::components::Transform;
use crate::components::{RigidBody, Velocity};
use gizmo_math::Vec3;

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
        entity_index_map: &std::collections::HashMap<u32, usize>,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        dt: f32,
    ) {
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
                _ => {}
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

    /// Apply a 1-DOF angular velocity constraint along `direction`.
    /// `error` is the positional error in radians (positive = bodies need to rotate apart).
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

        let vel_err = (w_b - w_a).dot(direction);
        let position_bias = (self.position_bias * error / dt)
            .clamp(-self.max_angular_speed, self.max_angular_speed);
        let lambda = ((-vel_err + position_bias) / k).clamp(lambda_min, lambda_max);

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

    /// Apply a 1-DOF linear velocity constraint along `direction` at the anchor points.
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

        let rel_vel = (v_b - v_a).dot(direction);
        let position_bias = (self.position_bias * error / dt)
            .clamp(-self.max_correction_speed, self.max_correction_speed);
        let lambda = ((-rel_vel + position_bias) / k).clamp(lambda_min, lambda_max);

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
        );

        let v_a = vels[0].linear + vels[0].angular.cross(r_a);
        let v_b = vels[1].linear + vels[1].angular.cross(r_b);
        let rel_n = (v_b - v_a).dot(direction);
        assert!(
            rel_n.abs() < 1e-5,
            "tek uygulamada bağıl hız sıfırlanmalı; kalan = {rel_n} (yanlış efektif kütle?)"
        );
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
