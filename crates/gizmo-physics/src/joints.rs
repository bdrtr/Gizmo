use crate::components::{RigidBody, Transform, Velocity};
use gizmo_core::entity::Entity;
use gizmo_math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum JointType {
    Fixed,
    Hinge,
    BallSocket,
    Slider,
    Spring,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Joint {
    pub entity_a: Entity,
    pub entity_b: Entity,
    pub joint_type: JointType,
    pub local_anchor_a: Vec3,
    pub local_anchor_b: Vec3,
    pub break_force: f32,
    pub break_torque: f32,
    pub is_broken: bool,
    pub collision_enabled: bool,
    pub data: JointData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JointData {
    Fixed,
    Hinge(HingeJointData),
    BallSocket(BallSocketJointData),
    Slider(SliderJointData),
    Spring(SpringJointData),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HingeJointData {
    pub axis: Vec3,
    pub use_limits: bool,
    pub lower_limit: f32,
    pub upper_limit: f32,
    pub use_motor: bool,
    pub motor_target_velocity: f32,
    pub motor_max_force: f32,
    pub current_angle: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BallSocketJointData {
    pub use_cone_limit: bool,
    pub cone_limit_angle: f32,
    #[serde(default)]
    pub initial_relative_rotation: Option<Quat>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SliderJointData {
    pub axis: Vec3,
    pub use_limits: bool,
    pub lower_limit: f32,
    pub upper_limit: f32,
    pub use_motor: bool,
    pub motor_target_velocity: f32,
    pub motor_max_force: f32,
    pub current_position: f32,
    #[serde(default)]
    pub initial_relative_rotation: Option<Quat>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpringJointData {
    pub rest_length: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub min_length: f32,
    pub max_length: f32,
}

impl Joint {
    pub fn fixed(entity_a: Entity, entity_b: Entity, local_anchor_a: Vec3, local_anchor_b: Vec3) -> Self {
        Self {
            entity_a, entity_b,
            joint_type: JointType::Fixed,
            local_anchor_a, local_anchor_b,
            break_force: 0.0, break_torque: 0.0,
            is_broken: false, collision_enabled: false,
            data: JointData::Fixed,
        }
    }

    pub fn hinge(entity_a: Entity, entity_b: Entity, local_anchor_a: Vec3, local_anchor_b: Vec3, axis: Vec3) -> Self {
        Self {
            entity_a, entity_b,
            joint_type: JointType::Hinge,
            local_anchor_a, local_anchor_b,
            break_force: 0.0, break_torque: 0.0,
            is_broken: false, collision_enabled: false,
            data: JointData::Hinge(HingeJointData {
                axis: axis.normalize(),
                use_limits: false,
                lower_limit: -std::f32::consts::PI,
                upper_limit: std::f32::consts::PI,
                use_motor: false,
                motor_target_velocity: 0.0,
                motor_max_force: 0.0,
                current_angle: 0.0,
            }),
        }
    }

    pub fn ball_socket(entity_a: Entity, entity_b: Entity, local_anchor_a: Vec3, local_anchor_b: Vec3) -> Self {
        Self {
            entity_a, entity_b,
            joint_type: JointType::BallSocket,
            local_anchor_a, local_anchor_b,
            break_force: 0.0, break_torque: 0.0,
            is_broken: false, collision_enabled: false,
            data: JointData::BallSocket(BallSocketJointData {
                use_cone_limit: false,
                cone_limit_angle: std::f32::consts::PI,
                initial_relative_rotation: None,
            }),
        }
    }

    pub fn slider(entity_a: Entity, entity_b: Entity, local_anchor_a: Vec3, local_anchor_b: Vec3, axis: Vec3) -> Self {
        Self {
            entity_a, entity_b,
            joint_type: JointType::Slider,
            local_anchor_a, local_anchor_b,
            break_force: 0.0, break_torque: 0.0,
            is_broken: false, collision_enabled: false,
            data: JointData::Slider(SliderJointData {
                axis: axis.normalize(),
                use_limits: false,
                lower_limit: -10.0, upper_limit: 10.0,
                use_motor: false,
                motor_target_velocity: 0.0, motor_max_force: 0.0,
                current_position: 0.0,
                initial_relative_rotation: None,
            }),
        }
    }

    pub fn spring(entity_a: Entity, entity_b: Entity, local_anchor_a: Vec3, local_anchor_b: Vec3, rest_length: f32, stiffness: f32, damping: f32) -> Self {
        Self {
            entity_a, entity_b,
            joint_type: JointType::Spring,
            local_anchor_a, local_anchor_b,
            break_force: 0.0, break_torque: 0.0,
            is_broken: false, collision_enabled: false,
            data: JointData::Spring(SpringJointData {
                rest_length, stiffness, damping,
                min_length: 0.0, max_length: f32::INFINITY,
            }),
        }
    }

    pub fn with_break_force(mut self, force: f32, torque: f32) -> Self {
        self.break_force = force;
        self.break_torque = torque;
        self
    }

    pub fn with_collision(mut self, enabled: bool) -> Self {
        self.collision_enabled = enabled;
        self
    }

    pub fn check_break(&mut self, applied_force: f32, applied_torque: f32) -> bool {
        if self.break_force > 0.0 && applied_force > self.break_force {
            self.is_broken = true;
            return true;
        }
        if self.break_torque > 0.0 && applied_torque > self.break_torque {
            self.is_broken = true;
            return true;
        }
        false
    }
}

pub struct JointSolver {
    pub iterations: usize,
}

impl Default for JointSolver {
    fn default() -> Self {
        Self { iterations: 10 }
    }
}

impl JointSolver {
    pub fn new(iterations: usize) -> Self {
        Self { iterations }
    }

    pub fn solve_joints(&self, joints: &mut [Joint], bodies: &mut [(RigidBody, Transform, Velocity)], dt: f32) {
        for _ in 0..self.iterations {
            for joint in joints.iter_mut() {
                if joint.is_broken { continue; }

                let idx_a = joint.entity_a.id() as usize;
                let idx_b = joint.entity_b.id() as usize;

                if idx_a >= bodies.len() || idx_b >= bodies.len() || idx_a == idx_b { continue; }

                match joint.joint_type {
                    JointType::Fixed    => self.solve_fixed_joint(joint, bodies, idx_a, idx_b, dt),
                    JointType::Hinge    => self.solve_hinge_joint(joint, bodies, idx_a, idx_b, dt),
                    JointType::BallSocket => self.solve_ball_socket_joint(joint, bodies, idx_a, idx_b, dt),
                    JointType::Slider   => self.solve_slider_joint(joint, bodies, idx_a, idx_b, dt),
                    JointType::Spring   => self.solve_spring_joint(joint, bodies, idx_a, idx_b, dt),
                }
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
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        error: f32,
        dt: f32,
        bias: f32,
    ) {
        if direction.length_squared() < 1e-10 { return; }

        let inv_i_a = bodies[idx_a].0.inv_world_inertia_tensor(bodies[idx_a].1.rotation);
        let inv_i_b = bodies[idx_b].0.inv_world_inertia_tensor(bodies[idx_b].1.rotation);
        let w_a    = bodies[idx_a].2.angular;
        let w_b    = bodies[idx_b].2.angular;
        let dyn_a  = bodies[idx_a].0.is_dynamic();
        let dyn_b  = bodies[idx_b].0.is_dynamic();

        let k = direction.dot(inv_i_a * direction) + direction.dot(inv_i_b * direction);
        if k < 1e-10 { return; }

        let vel_err   = (w_b - w_a).dot(direction);
        let position_bias = bias * error / dt;
        let lambda = (-vel_err + position_bias) / k;

        let delta_a = inv_i_a * direction * lambda;
        let delta_b = inv_i_b * direction * lambda;

        if idx_a < idx_b {
            let (l, r) = bodies.split_at_mut(idx_b);
            if dyn_a { l[idx_a].2.angular -= delta_a; }
            if dyn_b { r[0].2.angular     += delta_b; }
        } else {
            let (l, r) = bodies.split_at_mut(idx_a);
            if dyn_b { l[idx_b].2.angular += delta_b; }
            if dyn_a { r[0].2.angular     -= delta_a; }
        }
    }

    /// Same as `apply_angular_constraint` but clamps lambda to [min, max].
    fn apply_angular_constraint_clamped(
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        error: f32,
        dt: f32,
        bias: f32,
        lambda_min: f32,
        lambda_max: f32,
    ) {
        if direction.length_squared() < 1e-10 { return; }

        let inv_i_a = bodies[idx_a].0.inv_world_inertia_tensor(bodies[idx_a].1.rotation);
        let inv_i_b = bodies[idx_b].0.inv_world_inertia_tensor(bodies[idx_b].1.rotation);
        let w_a    = bodies[idx_a].2.angular;
        let w_b    = bodies[idx_b].2.angular;
        let dyn_a  = bodies[idx_a].0.is_dynamic();
        let dyn_b  = bodies[idx_b].0.is_dynamic();

        let k = direction.dot(inv_i_a * direction) + direction.dot(inv_i_b * direction);
        if k < 1e-10 { return; }

        let vel_err = (w_b - w_a).dot(direction);
        let position_bias = bias * error / dt;
        let lambda = ((-vel_err + position_bias) / k).clamp(lambda_min, lambda_max);

        let delta_a = inv_i_a * direction * lambda;
        let delta_b = inv_i_b * direction * lambda;

        if idx_a < idx_b {
            let (l, r) = bodies.split_at_mut(idx_b);
            if dyn_a { l[idx_a].2.angular -= delta_a; }
            if dyn_b { r[0].2.angular     += delta_b; }
        } else {
            let (l, r) = bodies.split_at_mut(idx_a);
            if dyn_b { l[idx_b].2.angular += delta_b; }
            if dyn_a { r[0].2.angular     -= delta_a; }
        }
    }

    /// Apply a 1-DOF linear velocity constraint along `direction` at the anchor points.
    fn apply_linear_constraint(
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        r_a: Vec3,
        r_b: Vec3,
        error: f32,
        dt: f32,
        bias: f32,
        lambda_min: f32,
        lambda_max: f32,
    ) {
        let inv_m_a  = bodies[idx_a].0.inv_mass();
        let inv_m_b  = bodies[idx_b].0.inv_mass();
        let inv_i_a  = bodies[idx_a].0.inv_world_inertia_tensor(bodies[idx_a].1.rotation);
        let inv_i_b  = bodies[idx_b].0.inv_world_inertia_tensor(bodies[idx_b].1.rotation);
        let v_a = bodies[idx_a].2.linear + bodies[idx_a].2.angular.cross(r_a);
        let v_b = bodies[idx_b].2.linear + bodies[idx_b].2.angular.cross(r_b);
        let dyn_a = bodies[idx_a].0.is_dynamic();
        let dyn_b = bodies[idx_b].0.is_dynamic();

        let ang_a = (inv_i_a * r_a.cross(direction)).cross(r_a);
        let ang_b = (inv_i_b * r_b.cross(direction)).cross(r_b);
        let k = inv_m_a + inv_m_b + ang_a.dot(direction) + ang_b.dot(direction);
        if k < 1e-10 { return; }

        let rel_vel = (v_b - v_a).dot(direction);
        let position_bias = bias * error / dt;
        let lambda = ((-rel_vel + position_bias) / k).clamp(lambda_min, lambda_max);

        let impulse = direction * lambda;

        if idx_a < idx_b {
            let (l, r) = bodies.split_at_mut(idx_b);
            if dyn_a {
                l[idx_a].2.linear  -= impulse * inv_m_a;
                l[idx_a].2.angular -= inv_i_a * r_a.cross(impulse);
            }
            if dyn_b {
                r[0].2.linear  += impulse * inv_m_b;
                r[0].2.angular += inv_i_b * r_b.cross(impulse);
            }
        } else {
            let (l, r) = bodies.split_at_mut(idx_a);
            if dyn_b {
                l[idx_b].2.linear  += impulse * inv_m_b;
                l[idx_b].2.angular += inv_i_b * r_b.cross(impulse);
            }
            if dyn_a {
                r[0].2.linear  -= impulse * inv_m_a;
                r[0].2.angular -= inv_i_a * r_a.cross(impulse);
            }
        }
    }

    // ── joint solvers ─────────────────────────────────────────────────────────

    fn solve_fixed_joint(
        &self,
        joint: &mut Joint,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let anchor_a = bodies[idx_a].1.position + bodies[idx_a].1.rotation * joint.local_anchor_a;
        let anchor_b = bodies[idx_b].1.position + bodies[idx_b].1.rotation * joint.local_anchor_b;
        let error    = anchor_b - anchor_a;
        let err_len  = error.length();

        if err_len < 0.001 { return; }

        let direction = error / err_len;
        let r_a = anchor_a - bodies[idx_a].1.position;
        let r_b = anchor_b - bodies[idx_b].1.position;

        let inv_m_a = bodies[idx_a].0.inv_mass();
        let inv_m_b = bodies[idx_b].0.inv_mass();
        let inv_i_a = bodies[idx_a].0.inv_world_inertia_tensor(bodies[idx_a].1.rotation);
        let inv_i_b = bodies[idx_b].0.inv_world_inertia_tensor(bodies[idx_b].1.rotation);
        let dyn_a   = bodies[idx_a].0.is_dynamic();
        let dyn_b   = bodies[idx_b].0.is_dynamic();

        let ang_a = (inv_i_a * r_a.cross(direction)).cross(r_a);
        let ang_b = (inv_i_b * r_b.cross(direction)).cross(r_b);
        let k = inv_m_a + inv_m_b + ang_a.dot(direction) + ang_b.dot(direction);

        if k < 1e-6 { return; }

        let bias   = 0.2 * err_len / dt;
        let lambda = bias / k;
        let impulse = direction * lambda;

        if idx_a < idx_b {
            let (l, r) = bodies.split_at_mut(idx_b);
            if dyn_a {
                l[idx_a].2.linear  -= impulse * inv_m_a;
                l[idx_a].2.angular -= inv_i_a * r_a.cross(impulse);
            }
            if dyn_b {
                r[0].2.linear  += impulse * inv_m_b;
                r[0].2.angular += inv_i_b * r_b.cross(impulse);
            }
        } else {
            let (l, r) = bodies.split_at_mut(idx_a);
            if dyn_b {
                l[idx_b].2.linear  += impulse * inv_m_b;
                l[idx_b].2.angular += inv_i_b * r_b.cross(impulse);
            }
            if dyn_a {
                r[0].2.linear  -= impulse * inv_m_a;
                r[0].2.angular -= inv_i_a * r_a.cross(impulse);
            }
        }

        joint.check_break(lambda, 0.0);
    }

    fn solve_hinge_joint(
        &self,
        joint: &mut Joint,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        // 1. Position constraint — keep anchor points together
        self.solve_fixed_joint(joint, bodies, idx_a, idx_b, dt);

        let JointData::Hinge(ref mut data) = joint.data else { return };

        let rot_a    = bodies[idx_a].1.rotation;
        let rot_b    = bodies[idx_b].1.rotation;
        let axis_a   = rot_a * data.axis;
        let axis_b   = rot_b * data.axis;

        // 2. Angular constraint — keep hinge axes aligned (2 DOF)
        let ang_err = axis_a.cross(axis_b);
        let err_mag = ang_err.length();
        if err_mag > 1e-6 {
            let err_dir = ang_err / err_mag;
            Self::apply_angular_constraint(bodies, idx_a, idx_b, err_dir, -err_mag, dt, 0.3);
        }

        // 3. Track current angle
        let ref_local = if data.axis.cross(Vec3::X).length() > 0.1 {
            data.axis.cross(Vec3::X).normalize()
        } else {
            data.axis.cross(Vec3::Y).normalize()
        };

        let rot_a = bodies[idx_a].1.rotation;
        let rot_b = bodies[idx_b].1.rotation;
        let axis_w  = rot_a * data.axis;
        let ref_a_w = rot_a * ref_local;
        let ref_b_w = rot_b * ref_local;

        let proj_a = (ref_a_w - axis_w * ref_a_w.dot(axis_w)).normalize_or_zero();
        let proj_b = (ref_b_w - axis_w * ref_b_w.dot(axis_w)).normalize_or_zero();

        if proj_a.length_squared() > 0.01 && proj_b.length_squared() > 0.01 {
            let cos_a  = proj_a.dot(proj_b).clamp(-1.0, 1.0);
            let sign   = if proj_a.cross(proj_b).dot(axis_w) >= 0.0 { 1.0_f32 } else { -1.0 };
            data.current_angle = sign * cos_a.acos();

            // 4. Angle limits
            if data.use_limits {
                if data.current_angle < data.lower_limit {
                    let err = data.lower_limit - data.current_angle;
                    // axis_w points from A to B; positive lambda increases angle
                    Self::apply_angular_constraint_clamped(
                        bodies, idx_a, idx_b, axis_w, err, dt, 0.3,
                        f32::NEG_INFINITY, 0.0,
                    );
                } else if data.current_angle > data.upper_limit {
                    let err = data.upper_limit - data.current_angle; // negative
                    Self::apply_angular_constraint_clamped(
                        bodies, idx_a, idx_b, axis_w, err, dt, 0.3,
                        0.0, f32::INFINITY,
                    );
                }
            }
        }

        // 5. Motor — velocity constraint along hinge axis
        if data.use_motor {
            let axis_w    = bodies[idx_a].1.rotation * data.axis;
            let inv_i_a   = bodies[idx_a].0.inv_world_inertia_tensor(bodies[idx_a].1.rotation);
            let inv_i_b   = bodies[idx_b].0.inv_world_inertia_tensor(bodies[idx_b].1.rotation);
            let w_a       = bodies[idx_a].2.angular;
            let w_b       = bodies[idx_b].2.angular;
            let dyn_a     = bodies[idx_a].0.is_dynamic();
            let dyn_b     = bodies[idx_b].0.is_dynamic();

            let k = axis_w.dot(inv_i_a * axis_w) + axis_w.dot(inv_i_b * axis_w);
            if k > 1e-10 {
                let rel_vel      = (w_b - w_a).dot(axis_w);
                let vel_err      = data.motor_target_velocity - rel_vel;
                let max_impulse  = data.motor_max_force * dt;
                let lambda       = (vel_err / k).clamp(-max_impulse, max_impulse);

                let delta_a = inv_i_a * axis_w * lambda;
                let delta_b = inv_i_b * axis_w * lambda;

                if idx_a < idx_b {
                    let (l, r) = bodies.split_at_mut(idx_b);
                    if dyn_a { l[idx_a].2.angular -= delta_a; }
                    if dyn_b { r[0].2.angular     += delta_b; }
                } else {
                    let (l, r) = bodies.split_at_mut(idx_a);
                    if dyn_b { l[idx_b].2.angular += delta_b; }
                    if dyn_a { r[0].2.angular     -= delta_a; }
                }
            }
        }
    }

    fn solve_ball_socket_joint(
        &self,
        joint: &mut Joint,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        // 1. Position constraint
        self.solve_fixed_joint(joint, bodies, idx_a, idx_b, dt);

        let JointData::BallSocket(ref mut data) = joint.data else { return };
        if !data.use_cone_limit { return; }

        // 2. Initialise reference rotation on first solve
        let relative_rot = bodies[idx_a].1.rotation.inverse() * bodies[idx_b].1.rotation;
        if data.initial_relative_rotation.is_none() {
            data.initial_relative_rotation = Some(relative_rot);
            return;
        }
        let initial_rot = data.initial_relative_rotation.unwrap();

        // Compute the "swing" rotation of B away from its initial orientation (in A's frame)
        let swing_quat = initial_rot.inverse() * relative_rot;

        // Small-angle: angular error ≈ 2 * quat.xyz (when w ≥ 0)
        let swing_err_local = if swing_quat.w >= 0.0 {
            Vec3::new(swing_quat.x, swing_quat.y, swing_quat.z) * 2.0
        } else {
            -Vec3::new(swing_quat.x, swing_quat.y, swing_quat.z) * 2.0
        };

        let swing_angle = swing_err_local.length();
        if swing_angle <= data.cone_limit_angle || swing_angle < 1e-6 { return; }

        let excess = swing_angle - data.cone_limit_angle;
        let swing_dir_local = swing_err_local / swing_angle;

        // Convert error direction to world space
        let swing_dir_world = bodies[idx_a].1.rotation * swing_dir_local;

        Self::apply_angular_constraint_clamped(
            bodies, idx_a, idx_b,
            swing_dir_world,
            -excess, dt, 0.3,
            f32::NEG_INFINITY, 0.0,
        );
    }

    fn solve_slider_joint(
        &self,
        joint: &mut Joint,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let JointData::Slider(ref mut data) = joint.data else { return };

        let anchor_a  = bodies[idx_a].1.position + bodies[idx_a].1.rotation * joint.local_anchor_a;
        let anchor_b  = bodies[idx_b].1.position + bodies[idx_b].1.rotation * joint.local_anchor_b;
        let axis_w    = (bodies[idx_a].1.rotation * data.axis).normalize();

        let delta    = anchor_b - anchor_a;
        let along    = delta.dot(axis_w);
        let off_axis = delta - axis_w * along;

        data.current_position = along;

        let r_a = anchor_a - bodies[idx_a].1.position;
        let r_b = anchor_b - bodies[idx_b].1.position;

        // 1. Off-axis constraint: project onto two perpendicular directions
        let (perp1, perp2) = Self::perpendiculars(axis_w);

        let err1 = off_axis.dot(perp1);
        if err1.abs() > 1e-4 {
            Self::apply_linear_constraint(
                bodies, idx_a, idx_b, perp1, r_a, r_b,
                err1, dt, 0.3, f32::NEG_INFINITY, f32::INFINITY,
            );
        }

        let err2 = off_axis.dot(perp2);
        if err2.abs() > 1e-4 {
            Self::apply_linear_constraint(
                bodies, idx_a, idx_b, perp2, r_a, r_b,
                err2, dt, 0.3, f32::NEG_INFINITY, f32::INFINITY,
            );
        }

        // 2. Angular lock — full 3-DOF rotation constraint using quaternion error
        let relative_rot = bodies[idx_a].1.rotation.inverse() * bodies[idx_b].1.rotation;
        if data.initial_relative_rotation.is_none() {
            data.initial_relative_rotation = Some(relative_rot);
        } else {
            let initial_rot = data.initial_relative_rotation.unwrap();
            let err_quat = initial_rot.inverse() * relative_rot;
            let ang_err_local = if err_quat.w >= 0.0 {
                Vec3::new(err_quat.x, err_quat.y, err_quat.z) * 2.0
            } else {
                -Vec3::new(err_quat.x, err_quat.y, err_quat.z) * 2.0
            };

            let err_world = bodies[idx_a].1.rotation * ang_err_local;
            let err_mag   = err_world.length();
            if err_mag > 1e-6 {
                Self::apply_angular_constraint(
                    bodies, idx_a, idx_b,
                    err_world / err_mag, -err_mag, dt, 0.3,
                );
            }
        }

        // 3. Along-axis limits
        if data.use_limits {
            let axis_w = bodies[idx_a].1.rotation * data.axis;
            let anchor_a = bodies[idx_a].1.position + bodies[idx_a].1.rotation * joint.local_anchor_a;
            let anchor_b = bodies[idx_b].1.position + bodies[idx_b].1.rotation * joint.local_anchor_b;
            let r_a = anchor_a - bodies[idx_a].1.position;
            let r_b = anchor_b - bodies[idx_b].1.position;

            if along < data.lower_limit {
                let err = data.lower_limit - along;
                Self::apply_linear_constraint(
                    bodies, idx_a, idx_b, axis_w, r_a, r_b,
                    err, dt, 0.3, f32::NEG_INFINITY, 0.0,
                );
            } else if along > data.upper_limit {
                let err = data.upper_limit - along; // negative
                Self::apply_linear_constraint(
                    bodies, idx_a, idx_b, axis_w, r_a, r_b,
                    err, dt, 0.3, 0.0, f32::INFINITY,
                );
            }
        }

        // 4. Motor — velocity along axis
        if data.use_motor {
            let axis_w  = bodies[idx_a].1.rotation * data.axis;
            let anchor_a = bodies[idx_a].1.position + bodies[idx_a].1.rotation * joint.local_anchor_a;
            let anchor_b = bodies[idx_b].1.position + bodies[idx_b].1.rotation * joint.local_anchor_b;
            let r_a = anchor_a - bodies[idx_a].1.position;
            let r_b = anchor_b - bodies[idx_b].1.position;
            let max_impulse = data.motor_max_force * dt;

            let v_a = bodies[idx_a].2.linear + bodies[idx_a].2.angular.cross(r_a);
            let v_b = bodies[idx_b].2.linear + bodies[idx_b].2.angular.cross(r_b);
            let rel_vel = (v_b - v_a).dot(axis_w);
            let vel_err = data.motor_target_velocity - rel_vel;

            let inv_m_a = bodies[idx_a].0.inv_mass();
            let inv_m_b = bodies[idx_b].0.inv_mass();
            let inv_i_a = bodies[idx_a].0.inv_world_inertia_tensor(bodies[idx_a].1.rotation);
            let inv_i_b = bodies[idx_b].0.inv_world_inertia_tensor(bodies[idx_b].1.rotation);
            let dyn_a   = bodies[idx_a].0.is_dynamic();
            let dyn_b   = bodies[idx_b].0.is_dynamic();

            let ang_a = (inv_i_a * r_a.cross(axis_w)).cross(r_a);
            let ang_b = (inv_i_b * r_b.cross(axis_w)).cross(r_b);
            let k = inv_m_a + inv_m_b + ang_a.dot(axis_w) + ang_b.dot(axis_w);
            if k > 1e-10 {
                let lambda  = (vel_err / k).clamp(-max_impulse, max_impulse);
                let impulse = axis_w * lambda;

                if idx_a < idx_b {
                    let (l, r) = bodies.split_at_mut(idx_b);
                    if dyn_a {
                        l[idx_a].2.linear  -= impulse * inv_m_a;
                        l[idx_a].2.angular -= inv_i_a * r_a.cross(impulse);
                    }
                    if dyn_b {
                        r[0].2.linear  += impulse * inv_m_b;
                        r[0].2.angular += inv_i_b * r_b.cross(impulse);
                    }
                } else {
                    let (l, r) = bodies.split_at_mut(idx_a);
                    if dyn_b {
                        l[idx_b].2.linear  += impulse * inv_m_b;
                        l[idx_b].2.angular += inv_i_b * r_b.cross(impulse);
                    }
                    if dyn_a {
                        r[0].2.linear  -= impulse * inv_m_a;
                        r[0].2.angular -= inv_i_a * r_a.cross(impulse);
                    }
                }
            }
        }
    }

    fn solve_spring_joint(
        &self,
        joint: &Joint,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let JointData::Spring(data) = joint.data else { return };

        let anchor_a = bodies[idx_a].1.position + bodies[idx_a].1.rotation * joint.local_anchor_a;
        let anchor_b = bodies[idx_b].1.position + bodies[idx_b].1.rotation * joint.local_anchor_b;

        let diff   = anchor_b - anchor_a;
        let length = diff.length();
        if length < 1e-6 { return; }

        let direction = diff / length;
        let r_a = anchor_a - bodies[idx_a].1.position;
        let r_b = anchor_b - bodies[idx_b].1.position;

        let v_a = bodies[idx_a].2.linear + bodies[idx_a].2.angular.cross(r_a);
        let v_b = bodies[idx_b].2.linear + bodies[idx_b].2.angular.cross(r_b);

        // Hooke's law + damping
        let spring_force  = -data.stiffness * (length - data.rest_length);
        let relative_vel  = (v_b - v_a).dot(direction);
        let damping_force = -data.damping * relative_vel;
        let total_impulse = (spring_force + damping_force) * dt;

        // Hard limits
        let clamped_impulse = if length <= data.min_length && total_impulse < 0.0 {
            0.0 // already at min, don't push further
        } else if length >= data.max_length && total_impulse > 0.0 {
            0.0 // already at max, don't push further
        } else {
            total_impulse
        };

        if clamped_impulse.abs() < 1e-10 { return; }

        let impulse  = direction * clamped_impulse;
        let inv_m_a  = bodies[idx_a].0.inv_mass();
        let inv_m_b  = bodies[idx_b].0.inv_mass();
        let inv_i_a  = bodies[idx_a].0.inv_world_inertia_tensor(bodies[idx_a].1.rotation);
        let inv_i_b  = bodies[idx_b].0.inv_world_inertia_tensor(bodies[idx_b].1.rotation);
        let dyn_a    = bodies[idx_a].0.is_dynamic();
        let dyn_b    = bodies[idx_b].0.is_dynamic();

        if idx_a < idx_b {
            let (l, r) = bodies.split_at_mut(idx_b);
            if dyn_a {
                l[idx_a].2.linear  -= impulse * inv_m_a;
                l[idx_a].2.angular -= inv_i_a * r_a.cross(impulse);
            }
            if dyn_b {
                r[0].2.linear  += impulse * inv_m_b;
                r[0].2.angular += inv_i_b * r_b.cross(impulse);
            }
        } else {
            let (l, r) = bodies.split_at_mut(idx_a);
            if dyn_b {
                l[idx_b].2.linear  += impulse * inv_m_b;
                l[idx_b].2.angular += inv_i_b * r_b.cross(impulse);
            }
            if dyn_a {
                r[0].2.linear  -= impulse * inv_m_a;
                r[0].2.angular -= inv_i_a * r_a.cross(impulse);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_joint_creation() {
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(2, 0);
        let joint = Joint::fixed(e1, e2, Vec3::ZERO, Vec3::ZERO);
        assert_eq!(joint.joint_type, JointType::Fixed);
        assert!(!joint.is_broken);
    }

    #[test]
    fn test_hinge_joint() {
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(2, 0);
        let joint = Joint::hinge(e1, e2, Vec3::ZERO, Vec3::ZERO, Vec3::Y);
        assert_eq!(joint.joint_type, JointType::Hinge);
        if let JointData::Hinge(data) = joint.data {
            assert_eq!(data.axis, Vec3::Y);
        } else {
            panic!("expected hinge data");
        }
    }

    #[test]
    fn test_spring_joint() {
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(2, 0);
        let joint = Joint::spring(e1, e2, Vec3::ZERO, Vec3::ZERO, 1.0, 100.0, 10.0);
        if let JointData::Spring(data) = joint.data {
            assert_eq!(data.stiffness, 100.0);
            assert_eq!(data.damping, 10.0);
        } else {
            panic!("expected spring data");
        }
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
