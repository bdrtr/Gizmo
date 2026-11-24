//! Slider/prismatic-joint (+ suspension spring) solver — extracted verbatim from the former 1236-line joint_types.rs.
//! One `impl JointSolver` block per joint kind; `pub(crate)` methods stay callable
//! from `solve_joints` regardless of file (inherent impls compose across modules).

use super::super::*;

impl JointSolver {

    pub(crate) fn solve_slider_joint(
        &self,
        joint: &mut Joint,
        rigid_bodies: &[RigidBody],
        transforms: Poses<'_>,
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let JointData::Slider(ref mut data) = joint.data else {
            return;
        };

        let anchor_a =
            transforms[idx_a].position + transforms[idx_a].rotation * joint.local_anchor_a;
        let anchor_b =
            transforms[idx_b].position + transforms[idx_b].rotation * joint.local_anchor_b;
        // `axis` is a public field and `SliderJointData::default()` leaves it at `Vec3::ZERO`
        // — only `Joint::slider` substitutes `Vec3::Y`, and `..Default::default()` walks
        // straight past that — so a zero axis is a reachable input, not a theoretical one.
        // A bare `.normalize()` on it returns a NaN vector (glam's `glam_assert!` is compiled
        // out; this workspace does not enable that feature), which then flowed into
        // `current_position` — a public field — on every pass. Nothing worse happened only
        // because NaN loses every `>` comparison downstream (`err.abs() > 1e-4`,
        // `along < lower_limit`, `k > 1e-10`), i.e. containment by accident of IEEE
        // semantics rather than by design; the first gate rewritten as `!(x <= eps)` would
        // have released it into every body this joint touches.
        //
        // Contract: no axis ⇒ the rows that NEED one are skipped (off-axis, travel limits,
        // motor) and the ones that do not keep running (the angular lock below, and the point
        // rows of whatever else is attached). That is the rule the siblings in this module
        // already follow — `solve_slider_spring` below, `solve_hinge_spring`,
        // `apply_angular_constraint_soft`'s direction gate, the ball-socket twist/swing rows.
        // Substituting `Vec3::Y` here was rejected: at CONSTRUCTION a fallback repairs an
        // argument the user meant to supply, but in the solver it would silently pin two axes
        // the caller never named — inventing a constraint is worse than applying none.
        //
        // `normalize_or_zero` computes the identical `self * self.length_recip()` product for
        // every axis that does normalise, so a valid slider is bit-for-bit unchanged.
        let axis_w = (transforms[idx_a].rotation * data.axis).normalize_or_zero();
        let has_axis = axis_w.length_squared() > 0.5; // unit or exactly zero — nothing between

        let delta = anchor_b - anchor_a;
        // With no axis this is `delta.dot(ZERO)` = exactly 0.0, not NaN: no axis, no travel
        // along it. (See `SliderJointData::current_position`.)
        let along = delta.dot(axis_w);
        let off_axis = anchor_a - (anchor_b - axis_w * along); // error = target - current

        data.current_position = along;

        let r_a = Self::lever_arm(rigid_bodies, transforms, idx_a, anchor_a);
        let r_b = Self::lever_arm(rigid_bodies, transforms, idx_b, anchor_b);
        let frame = SliderFrame { axis_w, off_axis, along, r_a, r_b };
        let scratch = &mut joint.scratch;

        // 1. Off-axis constraint: project onto two perpendicular directions
        // Gated: `perpendiculars` itself normalises a cross product, so calling it with a
        // zero axis is where the next NaN would be minted.
        if has_axis {
            self.slider_off_axis(rigid_bodies, transforms, velocities, idx_a, idx_b, &frame, scratch, dt);
        }
        self.slider_rotation_lock(rigid_bodies, transforms, velocities, idx_a, idx_b, data, scratch, dt);
        if data.use_limits && has_axis {
            self.slider_limits(rigid_bodies, transforms, velocities, idx_a, idx_b, data, &frame, scratch, dt);
        }
        if data.use_motor && has_axis {
            self.slider_motor(rigid_bodies, transforms, velocities, idx_a, idx_b, data, &frame, scratch, dt);
        }
    }

    /// The off-axis rows (1): the anchors held on the axis, along its two perpendiculars.
    #[allow(clippy::too_many_arguments)]
    fn slider_off_axis(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: Poses<'_>,
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        frame: &SliderFrame,
        scratch: &mut JointScratch,
        dt: f32,
    ) {
        let SliderFrame { axis_w, off_axis, r_a, r_b, .. } = *frame;
        let (perp1, perp2) = Self::perpendiculars(axis_w);

        let err1 = off_axis.dot(perp1);
        if err1.abs() > 1e-4 {
            self.apply_linear_constraint(
                rigid_bodies,
                transforms,
                velocities,
                idx_a,
                idx_b,
                perp1,
                r_a,
                r_b,
                err1,
                dt,
                f32::NEG_INFINITY,
                f32::INFINITY,
                scratch, row::LIN,
            );
        }

        let err2 = off_axis.dot(perp2);
        if err2.abs() > 1e-4 {
            self.apply_linear_constraint(
                rigid_bodies,
                transforms,
                velocities,
                idx_a,
                idx_b,
                perp2,
                r_a,
                r_b,
                err2,
                dt,
                f32::NEG_INFINITY,
                f32::INFINITY,
                scratch, row::LIN + 1,
            );
        }
    }

    /// The angular lock (2): the relative rotation held where the first pass found it, all three
    /// degrees.
    #[allow(clippy::too_many_arguments)]
    fn slider_rotation_lock(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: Poses<'_>,
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        data: &mut SliderJointData,
        scratch: &mut JointScratch,
        dt: f32,
    ) {
        // 2. Angular lock — full 3-DOF rotation constraint using quaternion error
        let relative_rot = transforms[idx_a].rotation.inverse() * transforms[idx_b].rotation;
        if let Some(initial_rot) = data.initial_relative_rotation {
            let err_quat = initial_rot.inverse() * relative_rot;
            let ang_err_local = if err_quat.w >= 0.0 {
                Vec3::new(err_quat.x, err_quat.y, err_quat.z) * 2.0
            } else {
                -Vec3::new(err_quat.x, err_quat.y, err_quat.z) * 2.0
            };

            let err_world = transforms[idx_a].rotation * ang_err_local;
            let err_mag = err_world.length();
            if err_mag > 1e-6 {
                self.apply_angular_constraint(
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        err_world / err_mag,
                        -err_mag,
                        dt,
                        f32::NEG_INFINITY,
                        f32::INFINITY,
                        scratch, row::ANG,
                    );
            }
        } else {
            data.initial_relative_rotation = Some(relative_rot);
        }
    }

    /// The travel limits along the axis (3).
    #[allow(clippy::too_many_arguments)]
    fn slider_limits(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: Poses<'_>,
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        data: &SliderJointData,
        frame: &SliderFrame,
        scratch: &mut JointScratch,
        dt: f32,
    ) {
        let SliderFrame { axis_w, along, r_a, r_b, .. } = *frame;
        // 3. Along-axis limits.
        // Impulse-clamp yönü `apply_linear_constraint` konvansiyonuyla uyumlu olmalı
        // (bkz. çalışan hinge limiti): alt-limit ihlali (err > 0) cismi +eksene İTER →
        // pozitif lambda → clamp (0, +∞); üst-limit ihlali (err < 0) −eksene iter →
        // negatif lambda → clamp (−∞, 0). (Eskiden ikisi de TERSTİ → limit hiç tutmuyordu;
        // 5 m/s'lik cisim 1 m'lik üst limiti delip 19 m'ye gidiyordu.)
        if along < data.lower_limit {
            let err = data.lower_limit - along; // positive
            self.apply_linear_constraint(
                    rigid_bodies,
                    transforms,
                    velocities,
                    idx_a,
                    idx_b,
                    axis_w,
                    r_a,
                    r_b,
                    err,
                    dt,
                    0.0,
                    f32::INFINITY,
                    scratch, row::LIMIT,
                );
        } else if along > data.upper_limit {
            let err = data.upper_limit - along; // negative
            self.apply_linear_constraint(
                    rigid_bodies,
                    transforms,
                    velocities,
                    idx_a,
                    idx_b,
                    axis_w,
                    r_a,
                    r_b,
                    err,
                    dt,
                    f32::NEG_INFINITY,
                    0.0,
                    scratch, row::LIMIT,
                );
        }
    }

    /// The motor (4): a velocity along the axis — or, as a servo, toward a position — within its
    /// force.
    #[allow(clippy::too_many_arguments)]
    fn slider_motor(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: Poses<'_>,
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        data: &SliderJointData,
        frame: &SliderFrame,
        scratch: &mut JointScratch,
        dt: f32,
    ) {
        let SliderFrame { axis_w, r_a, r_b, .. } = *frame;
        // 4. Motor — velocity along axis
        // Bütçe geçişin TOPLAMINA uygulanır — bkz. hinge motoru.
        let max_impulse = data.motor_max_force * dt;

        let v_a = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
        let v_b = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
        let rel_vel = (v_b - v_a).dot(axis_w);
        // Servo: drive toward the target position along the axis (P-control via
        // position_bias), force still capped by motor_max_force.
        let target_vel = if data.motor_is_servo {
            self.position_bias * (data.motor_target_position - data.current_position) / dt
        } else {
            data.motor_target_velocity
        };
        let vel_err = target_vel - rel_vel;

        let inv_m_a = rigid_bodies[idx_a].inv_mass();
        let inv_m_b = rigid_bodies[idx_b].inv_mass();
        let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
        let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
        let dyn_a = rigid_bodies[idx_a].is_dynamic();
        let dyn_b = rigid_bodies[idx_b].is_dynamic();

        // k_ang = (r×axis)·I⁻¹·(r×axis) — bkz. apply_linear_constraint düzeltmesi.
        let rxa_a = r_a.cross(axis_w);
        let rxa_b = r_b.cross(axis_w);
        let k = inv_m_a
            + inv_m_b
            + inv_i_a.mul_vec3(rxa_a).dot(rxa_a)
            + inv_i_b.mul_vec3(rxa_b).dot(rxa_b);
        if k > 1e-10 {
            let lambda = Self::accumulate(
                scratch.row(row::MOTOR),
                vel_err / k,
                -max_impulse,
                max_impulse,
            );
            let impulse = axis_w * lambda;

            let (va, vb) = pair_mut(velocities, idx_a, idx_b);
            if dyn_a {
                va.linear -= impulse * inv_m_a;
                va.angular -= inv_i_a.mul_vec3(r_a.cross(impulse));
            }
            if dyn_b {
                vb.linear += impulse * inv_m_b;
                vb.angular += inv_i_b.mul_vec3(r_b.cross(impulse));
            }
    }
    }

    /// Suspension spring on a Slider: a soft PD force along the free axis pulling the
    /// along-axis offset toward `spring_rest_position`. Force-based (runs once per step,
    /// like Spring); same sign convention — positive impulse reduces the along-axis offset.
    /// No-op unless `use_spring`. This is the springy-prismatic a shock/suspension needs,
    /// which the hard-limit + velocity-motor Slider could not express.
    pub(crate) fn solve_slider_spring(
        &self,
        joint: &mut Joint,
        rigid_bodies: &[RigidBody],
        transforms: Poses<'_>,
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let JointData::Slider(data) = joint.data else {
            return;
        };
        if !data.use_spring {
            return;
        }

        let anchor_a =
            transforms[idx_a].position + transforms[idx_a].rotation * joint.local_anchor_a;
        let anchor_b =
            transforms[idx_b].position + transforms[idx_b].rotation * joint.local_anchor_b;
        let axis_w = (transforms[idx_a].rotation * data.axis).normalize_or_zero();
        if axis_w.length_squared() < 1e-6 {
            return;
        }
        let along = (anchor_b - anchor_a).dot(axis_w);
        let r_a = Self::lever_arm(rigid_bodies, transforms, idx_a, anchor_a);
        let r_b = Self::lever_arm(rigid_bodies, transforms, idx_b, anchor_b);
        let v_a = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
        let v_b = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
        let rel_vel = (v_b - v_a).dot(axis_w);

        let k_inv = Self::linear_row_k_inv(rigid_bodies, transforms, (idx_a, idx_b), (r_a, r_b), axis_w);
        let impulse_mag = Self::implicit_spring_impulse(
            data.spring_stiffness,
            data.spring_damping,
            along - data.spring_rest_position,
            rel_vel,
            0.0,
            k_inv,
            dt,
        );
        if impulse_mag.abs() < 1e-10 {
            return;
        }
        let impulse = axis_w * impulse_mag;
        // Süspansiyon yayı gerçek yük taşır → `break_force` toplamına girer. (Daha önce
        // hiç break kontrolü görmüyordu: "breakable" bir slider'ın yay yükü görünmezdi.)
        // İşaret ters, çünkü aşağıda impulse A'ya ARTI, B'ye EKSİ uygulanıyor — satır
        // yardımcılarının konvansiyonunun tersi; büyüklük aynı.
        joint.scratch.impulse_lin -= impulse;
        let inv_m_a = rigid_bodies[idx_a].inv_mass();
        let inv_m_b = rigid_bodies[idx_b].inv_mass();
        let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
        let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
        let dyn_a = rigid_bodies[idx_a].is_dynamic();
        let dyn_b = rigid_bodies[idx_b].is_dynamic();
        let (va, vb) = pair_mut(velocities, idx_a, idx_b);
        if dyn_a {
            va.linear += impulse * inv_m_a;
            va.angular += inv_i_a.mul_vec3(r_a.cross(impulse));
        }
        if dyn_b {
            vb.linear -= impulse * inv_m_b;
            vb.angular -= inv_i_b.mul_vec3(r_b.cross(impulse));
        }
    }
}

/// Where a slider stands this pass: its axis in the world (zero when it has none), the anchors'
/// error off it and their travel along it, and the two lever arms.
#[derive(Clone, Copy)]
struct SliderFrame {
    axis_w: Vec3,
    off_axis: Vec3,
    along: f32,
    r_a: Vec3,
    r_b: Vec3,
}
