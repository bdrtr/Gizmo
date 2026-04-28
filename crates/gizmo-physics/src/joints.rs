use crate::components::{RigidBody, Transform, Velocity};
use gizmo_core::entity::Entity;
use gizmo_math::{Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Joint types supported by the physics engine
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum JointType {
    Fixed,
    Hinge,
    BallSocket,
    Slider,
    Spring,
}

/// Base joint constraint between two bodies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Joint {
    pub entity_a: Entity,
    pub entity_b: Entity,
    pub joint_type: JointType,
    pub local_anchor_a: Vec3,
    pub local_anchor_b: Vec3,
    pub break_force: f32,      // Force threshold to break joint (0 = unbreakable)
    pub break_torque: f32,     // Torque threshold to break joint
    pub is_broken: bool,
    pub collision_enabled: bool, // Allow collision between connected bodies
    pub data: JointData,
}

/// Type-specific joint data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JointData {
    Fixed,
    Hinge(HingeJointData),
    BallSocket(BallSocketJointData),
    Slider(SliderJointData),
    Spring(SpringJointData),
}

/// Hinge joint - rotation around a single axis
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HingeJointData {
    pub axis: Vec3,           // Rotation axis in local space of body A
    pub use_limits: bool,
    pub lower_limit: f32,     // Radians
    pub upper_limit: f32,     // Radians
    pub use_motor: bool,
    pub motor_target_velocity: f32,
    pub motor_max_force: f32,
    pub current_angle: f32,
}

/// Ball-socket joint - free rotation around a point
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BallSocketJointData {
    pub use_cone_limit: bool,
    pub cone_limit_angle: f32, // Maximum angle from initial orientation
}

/// Slider joint - translation along a single axis
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SliderJointData {
    pub axis: Vec3,           // Slide axis in local space of body A
    pub use_limits: bool,
    pub lower_limit: f32,
    pub upper_limit: f32,
    pub use_motor: bool,
    pub motor_target_velocity: f32,
    pub motor_max_force: f32,
    pub current_position: f32,
}

/// Spring joint - elastic connection
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpringJointData {
    pub rest_length: f32,
    pub stiffness: f32,       // Spring constant (k)
    pub damping: f32,         // Damping coefficient
    pub min_length: f32,
    pub max_length: f32,
}

impl Joint {
    /// Create a fixed joint (welds two bodies together)
    pub fn fixed(
        entity_a: Entity,
        entity_b: Entity,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
    ) -> Self {
        Self {
            entity_a,
            entity_b,
            joint_type: JointType::Fixed,
            local_anchor_a,
            local_anchor_b,
            break_force: 0.0,
            break_torque: 0.0,
            is_broken: false,
            collision_enabled: false,
            data: JointData::Fixed,
        }
    }

    /// Create a hinge joint
    pub fn hinge(
        entity_a: Entity,
        entity_b: Entity,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        axis: Vec3,
    ) -> Self {
        Self {
            entity_a,
            entity_b,
            joint_type: JointType::Hinge,
            local_anchor_a,
            local_anchor_b,
            break_force: 0.0,
            break_torque: 0.0,
            is_broken: false,
            collision_enabled: false,
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

    /// Create a ball-socket joint
    pub fn ball_socket(
        entity_a: Entity,
        entity_b: Entity,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
    ) -> Self {
        Self {
            entity_a,
            entity_b,
            joint_type: JointType::BallSocket,
            local_anchor_a,
            local_anchor_b,
            break_force: 0.0,
            break_torque: 0.0,
            is_broken: false,
            collision_enabled: false,
            data: JointData::BallSocket(BallSocketJointData {
                use_cone_limit: false,
                cone_limit_angle: std::f32::consts::PI,
            }),
        }
    }

    /// Create a slider joint
    pub fn slider(
        entity_a: Entity,
        entity_b: Entity,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        axis: Vec3,
    ) -> Self {
        Self {
            entity_a,
            entity_b,
            joint_type: JointType::Slider,
            local_anchor_a,
            local_anchor_b,
            break_force: 0.0,
            break_torque: 0.0,
            is_broken: false,
            collision_enabled: false,
            data: JointData::Slider(SliderJointData {
                axis: axis.normalize(),
                use_limits: false,
                lower_limit: -10.0,
                upper_limit: 10.0,
                use_motor: false,
                motor_target_velocity: 0.0,
                motor_max_force: 0.0,
                current_position: 0.0,
            }),
        }
    }

    /// Create a spring joint
    pub fn spring(
        entity_a: Entity,
        entity_b: Entity,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        rest_length: f32,
        stiffness: f32,
        damping: f32,
    ) -> Self {
        Self {
            entity_a,
            entity_b,
            joint_type: JointType::Spring,
            local_anchor_a,
            local_anchor_b,
            break_force: 0.0,
            break_torque: 0.0,
            is_broken: false,
            collision_enabled: false,
            data: JointData::Spring(SpringJointData {
                rest_length,
                stiffness,
                damping,
                min_length: 0.0,
                max_length: f32::INFINITY,
            }),
        }
    }

    /// Set break thresholds
    pub fn with_break_force(mut self, force: f32, torque: f32) -> Self {
        self.break_force = force;
        self.break_torque = torque;
        self
    }

    /// Enable collision between connected bodies
    pub fn with_collision(mut self, enabled: bool) -> Self {
        self.collision_enabled = enabled;
        self
    }

    /// Check if joint should break based on applied forces
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

/// Joint constraint solver
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

    /// Solve all joint constraints
    pub fn solve_joints(
        &self,
        joints: &mut [Joint],
        bodies: &mut [(RigidBody, Transform, Velocity)],
        dt: f32,
    ) {
        for _ in 0..self.iterations {
            for joint in joints.iter_mut() {
                if joint.is_broken {
                    continue;
                }

                // Find body indices (in real implementation, use entity lookup)
                let idx_a = joint.entity_a.id() as usize;
                let idx_b = joint.entity_b.id() as usize;

                if idx_a >= bodies.len() || idx_b >= bodies.len() {
                    continue;
                }

                match &mut joint.data {
                    JointData::Fixed => {
                        self.solve_fixed_joint(joint, bodies, idx_a, idx_b, dt);
                    }
                    JointData::Hinge(data) => {
                        self.solve_hinge_joint(joint, data, bodies, idx_a, idx_b, dt);
                    }
                    JointData::BallSocket(data) => {
                        self.solve_ball_socket_joint(joint, data, bodies, idx_a, idx_b, dt);
                    }
                    JointData::Slider(data) => {
                        self.solve_slider_joint(joint, data, bodies, idx_a, idx_b, dt);
                    }
                    JointData::Spring(data) => {
                        self.solve_spring_joint(joint, data, bodies, idx_a, idx_b, dt);
                    }
                }
            }
        }
    }

    fn solve_fixed_joint(
        &self,
        joint: &mut Joint,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let (rb_a, transform_a, vel_a) = &bodies[idx_a];
        let (rb_b, transform_b, vel_b) = &bodies[idx_b];

        // World-space anchor points
        let anchor_a = transform_a.position + transform_a.rotation * joint.local_anchor_a;
        let anchor_b = transform_b.position + transform_b.rotation * joint.local_anchor_b;

        // Position error
        let error = anchor_b - anchor_a;
        let error_length = error.length();

        if error_length < 0.001 {
            return;
        }

        let direction = error / error_length;

        // Calculate effective mass
        let r_a = anchor_a - transform_a.position;
        let r_b = anchor_b - transform_b.position;

        let inv_mass_a = rb_a.inv_mass();
        let inv_mass_b = rb_b.inv_mass();

        let inv_inertia_a = rb_a.inv_world_inertia_tensor(transform_a.rotation);
        let inv_inertia_b = rb_b.inv_world_inertia_tensor(transform_b.rotation);

        let r_a_cross = r_a.cross(direction);
        let r_b_cross = r_b.cross(direction);

        let angular_factor_a = (inv_inertia_a * r_a_cross).cross(r_a);
        let angular_factor_b = (inv_inertia_b * r_b_cross).cross(r_b);

        let inv_mass_sum = inv_mass_a + inv_mass_b 
            + angular_factor_a.dot(direction) 
            + angular_factor_b.dot(direction);

        if inv_mass_sum < 1e-6 {
            return;
        }

        // Calculate impulse (with position correction)
        let bias = 0.2 * error_length / dt;
        let lambda = bias / inv_mass_sum;

        let impulse = direction * lambda;

        // Apply impulse
        let (_, _, vel_a) = &mut bodies[idx_a];
        let (_, _, vel_b) = &mut bodies[idx_b];

        if rb_a.is_dynamic() {
            vel_a.linear -= impulse * inv_mass_a;
            vel_a.angular -= inv_inertia_a * r_a.cross(impulse);
        }

        if rb_b.is_dynamic() {
            vel_b.linear += impulse * inv_mass_b;
            vel_b.angular += inv_inertia_b * r_b.cross(impulse);
        }

        // Check for breaking
        joint.check_break(lambda, 0.0);
    }

    fn solve_hinge_joint(
        &self,
        joint: &mut Joint,
        data: &mut HingeJointData,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        // First solve position constraint (like fixed joint)
        self.solve_fixed_joint(joint, bodies, idx_a, idx_b, dt);

        let (rb_a, transform_a, vel_a) = &bodies[idx_a];
        let (rb_b, transform_b, vel_b) = &bodies[idx_b];

        // World-space hinge axis
        let axis_world = transform_a.rotation * data.axis;

        // Calculate current angle
        let ref_a = transform_a.rotation * Vec3::X;
        let ref_b = transform_b.rotation * Vec3::X;
        let projected_a = (ref_a - axis_world * ref_a.dot(axis_world)).normalize();
        let projected_b = (ref_b - axis_world * ref_b.dot(axis_world)).normalize();
        
        data.current_angle = projected_a.dot(projected_b).acos();
        if projected_a.cross(projected_b).dot(axis_world) < 0.0 {
            data.current_angle = -data.current_angle;
        }

        // Apply angle limits
        if data.use_limits {
            if data.current_angle < data.lower_limit || data.current_angle > data.upper_limit {
                let target_angle = data.current_angle.clamp(data.lower_limit, data.upper_limit);
                let angle_error = target_angle - data.current_angle;
                
                let inv_inertia_a = rb_a.inv_world_inertia_tensor(transform_a.rotation);
                let inv_inertia_b = rb_b.inv_world_inertia_tensor(transform_b.rotation);
                
                let inv_inertia_sum = (inv_inertia_a * axis_world).dot(axis_world)
                    + (inv_inertia_b * axis_world).dot(axis_world);
                
                if inv_inertia_sum > 1e-6 {
                    let correction_impulse = angle_error * 0.2 / (dt * inv_inertia_sum);
                    let angular_impulse = axis_world * correction_impulse;
                    
                    let (_, _, vel_a) = &mut bodies[idx_a];
                    let (_, _, vel_b) = &mut bodies[idx_b];
                    
                    if rb_a.is_dynamic() {
                        vel_a.angular += inv_inertia_a * angular_impulse;
                    }
                    if rb_b.is_dynamic() {
                        vel_b.angular -= inv_inertia_b * angular_impulse;
                    }
                }
            }
        }

        // Apply motor
        if data.use_motor {
            let (rb_a, transform_a, _) = &bodies[idx_a];
            let (rb_b, transform_b, _) = &bodies[idx_b];
            
            let inv_inertia_a = rb_a.inv_world_inertia_tensor(transform_a.rotation);
            let inv_inertia_b = rb_b.inv_world_inertia_tensor(transform_b.rotation);
            
            let (_, _, vel_a) = &mut bodies[idx_a];
            let (_, _, vel_b) = &mut bodies[idx_b];
            
            let current_velocity = vel_b.angular.dot(axis_world) - vel_a.angular.dot(axis_world);
            let velocity_error = data.motor_target_velocity - current_velocity;
            
            let inv_inertia_sum = (inv_inertia_a * axis_world).dot(axis_world)
                + (inv_inertia_b * axis_world).dot(axis_world);
            
            if inv_inertia_sum > 1e-6 {
                let motor_impulse = (velocity_error / inv_inertia_sum).clamp(
                    -data.motor_max_force * dt,
                    data.motor_max_force * dt,
                );
                let angular_impulse = axis_world * motor_impulse;
                
                if rb_a.is_dynamic() {
                    vel_a.angular -= inv_inertia_a * angular_impulse;
                }
                if rb_b.is_dynamic() {
                    vel_b.angular += inv_inertia_b * angular_impulse;
                }
            }
        }
    }

    fn solve_ball_socket_joint(
        &self,
        joint: &mut Joint,
        data: &BallSocketJointData,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        // Ball-socket is just a fixed position constraint
        self.solve_fixed_joint(joint, bodies, idx_a, idx_b, dt);

        // Apply cone limit if enabled
        if data.use_cone_limit {
            let (rb_a, transform_a, _) = &bodies[idx_a];
            let (rb_b, transform_b, _) = &bodies[idx_b];

            let initial_dir = Vec3::Y; // Assume initial direction is Y-up
            let dir_a = transform_a.rotation * initial_dir;
            let dir_b = transform_b.rotation * initial_dir;

            let angle = dir_a.dot(dir_b).acos();

            if angle > data.cone_limit_angle {
                let correction_axis = dir_a.cross(dir_b).normalize();
                let correction_angle = angle - data.cone_limit_angle;

                let inv_inertia_a = rb_a.inv_world_inertia_tensor(transform_a.rotation);
                let inv_inertia_b = rb_b.inv_world_inertia_tensor(transform_b.rotation);

                let inv_inertia_sum = (inv_inertia_a * correction_axis).dot(correction_axis)
                    + (inv_inertia_b * correction_axis).dot(correction_axis);

                if inv_inertia_sum > 1e-6 {
                    let correction_impulse = correction_angle * 0.2 / (dt * inv_inertia_sum);
                    let angular_impulse = correction_axis * correction_impulse;

                    let (_, _, vel_a) = &mut bodies[idx_a];
                    let (_, _, vel_b) = &mut bodies[idx_b];

                    if rb_a.is_dynamic() {
                        vel_a.angular += inv_inertia_a * angular_impulse;
                    }
                    if rb_b.is_dynamic() {
                        vel_b.angular -= inv_inertia_b * angular_impulse;
                    }
                }
            }
        }
    }

    fn solve_slider_joint(
        &self,
        joint: &mut Joint,
        data: &mut SliderJointData,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let (rb_a, transform_a, vel_a) = &bodies[idx_a];
        let (rb_b, transform_b, vel_b) = &bodies[idx_b];

        let anchor_a = transform_a.position + transform_a.rotation * joint.local_anchor_a;
        let anchor_b = transform_b.position + transform_b.rotation * joint.local_anchor_b;

        let axis_world = transform_a.rotation * data.axis;

        // Calculate current position along axis
        let delta = anchor_b - anchor_a;
        data.current_position = delta.dot(axis_world);

        // Constrain perpendicular motion
        let perpendicular_error = delta - axis_world * data.current_position;
        let error_length = perpendicular_error.length();

        if error_length > 0.001 {
            let direction = perpendicular_error / error_length;

            let r_a = anchor_a - transform_a.position;
            let r_b = anchor_b - transform_b.position;

            let inv_mass_a = rb_a.inv_mass();
            let inv_mass_b = rb_b.inv_mass();

            let inv_inertia_a = rb_a.inv_world_inertia_tensor(transform_a.rotation);
            let inv_inertia_b = rb_b.inv_world_inertia_tensor(transform_b.rotation);

            let r_a_cross = r_a.cross(direction);
            let r_b_cross = r_b.cross(direction);

            let angular_factor_a = (inv_inertia_a * r_a_cross).cross(r_a);
            let angular_factor_b = (inv_inertia_b * r_b_cross).cross(r_b);

            let inv_mass_sum = inv_mass_a + inv_mass_b 
                + angular_factor_a.dot(direction) 
                + angular_factor_b.dot(direction);

            if inv_mass_sum > 1e-6 {
                let bias = 0.2 * error_length / dt;
                let lambda = bias / inv_mass_sum;
                let impulse = direction * lambda;

                let (_, _, vel_a) = &mut bodies[idx_a];
                let (_, _, vel_b) = &mut bodies[idx_b];

                if rb_a.is_dynamic() {
                    vel_a.linear -= impulse * inv_mass_a;
                    vel_a.angular -= inv_inertia_a * r_a.cross(impulse);
                }
                if rb_b.is_dynamic() {
                    vel_b.linear += impulse * inv_mass_b;
                    vel_b.angular += inv_inertia_b * r_b.cross(impulse);
                }
            }
        }

        // Apply position limits
        if data.use_limits {
            if data.current_position < data.lower_limit || data.current_position > data.upper_limit {
                let target_pos = data.current_position.clamp(data.lower_limit, data.upper_limit);
                let pos_error = target_pos - data.current_position;

                let inv_mass_a = rb_a.inv_mass();
                let inv_mass_b = rb_b.inv_mass();
                let inv_mass_sum = inv_mass_a + inv_mass_b;

                if inv_mass_sum > 1e-6 {
                    let correction = pos_error * 0.2 / dt;
                    let impulse = axis_world * (correction / inv_mass_sum);

                    let (_, _, vel_a) = &mut bodies[idx_a];
                    let (_, _, vel_b) = &mut bodies[idx_b];

                    if rb_a.is_dynamic() {
                        vel_a.linear -= impulse * inv_mass_a;
                    }
                    if rb_b.is_dynamic() {
                        vel_b.linear += impulse * inv_mass_b;
                    }
                }
            }
        }

        // Apply motor
        if data.use_motor {
            let (rb_a, _, _) = &bodies[idx_a];
            let (rb_b, _, _) = &bodies[idx_b];
            
            let (_, _, vel_a) = &mut bodies[idx_a];
            let (_, _, vel_b) = &mut bodies[idx_b];

            let current_velocity = (vel_b.linear - vel_a.linear).dot(axis_world);
            let velocity_error = data.motor_target_velocity - current_velocity;

            let inv_mass_a = rb_a.inv_mass();
            let inv_mass_b = rb_b.inv_mass();
            let inv_mass_sum = inv_mass_a + inv_mass_b;

            if inv_mass_sum > 1e-6 {
                let motor_impulse = (velocity_error / inv_mass_sum).clamp(
                    -data.motor_max_force * dt,
                    data.motor_max_force * dt,
                );
                let impulse = axis_world * motor_impulse;

                if rb_a.is_dynamic() {
                    vel_a.linear -= impulse * inv_mass_a;
                }
                if rb_b.is_dynamic() {
                    vel_b.linear += impulse * inv_mass_b;
                }
            }
        }
    }

    fn solve_spring_joint(
        &self,
        joint: &Joint,
        data: &SpringJointData,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        let (rb_a, transform_a, vel_a) = &bodies[idx_a];
        let (rb_b, transform_b, vel_b) = &bodies[idx_b];

        let anchor_a = transform_a.position + transform_a.rotation * joint.local_anchor_a;
        let anchor_b = transform_b.position + transform_b.rotation * joint.local_anchor_b;

        let delta = anchor_b - anchor_a;
        let current_length = delta.length();

        if current_length < 0.001 {
            return;
        }

        let direction = delta / current_length;

        // Clamp length
        let clamped_length = current_length.clamp(data.min_length, data.max_length);
        
        // Spring force: F = -k * (x - rest_length)
        let displacement = clamped_length - data.rest_length;
        let spring_force = -data.stiffness * displacement;

        // Damping force: F = -c * v
        let r_a = anchor_a - transform_a.position;
        let r_b = anchor_b - transform_b.position;
        
        let vel_a_at_anchor = vel_a.linear + vel_a.angular.cross(r_a);
        let vel_b_at_anchor = vel_b.linear + vel_b.angular.cross(r_b);
        let relative_velocity = vel_b_at_anchor - vel_a_at_anchor;
        let velocity_along_spring = relative_velocity.dot(direction);
        
        let damping_force = -data.damping * velocity_along_spring;

        let total_force = spring_force + damping_force;
        let impulse = direction * total_force * dt;

        let inv_mass_a = rb_a.inv_mass();
        let inv_mass_b = rb_b.inv_mass();

        let inv_inertia_a = rb_a.inv_world_inertia_tensor(transform_a.rotation);
        let inv_inertia_b = rb_b.inv_world_inertia_tensor(transform_b.rotation);

        let (_, _, vel_a) = &mut bodies[idx_a];
        let (_, _, vel_b) = &mut bodies[idx_b];

        if rb_a.is_dynamic() {
            vel_a.linear -= impulse * inv_mass_a;
            vel_a.angular -= inv_inertia_a * r_a.cross(impulse);
        }

        if rb_b.is_dynamic() {
            vel_b.linear += impulse * inv_mass_b;
            vel_b.angular += inv_inertia_b * r_b.cross(impulse);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_joint_creation() {
        let e1 = Entity::from_raw(1);
        let e2 = Entity::from_raw(2);

        let joint = Joint::fixed(e1, e2, Vec3::ZERO, Vec3::ZERO);
        assert_eq!(joint.joint_type, JointType::Fixed);
        assert!(!joint.is_broken);
    }

    #[test]
    fn test_hinge_joint() {
        let e1 = Entity::from_raw(1);
        let e2 = Entity::from_raw(2);

        let joint = Joint::hinge(e1, e2, Vec3::ZERO, Vec3::ZERO, Vec3::Y);
        assert_eq!(joint.joint_type, JointType::Hinge);
        
        if let JointData::Hinge(data) = joint.data {
            assert_eq!(data.axis, Vec3::Y);
        } else {
            panic!("Expected hinge joint data");
        }
    }

    #[test]
    fn test_spring_joint() {
        let e1 = Entity::from_raw(1);
        let e2 = Entity::from_raw(2);

        let joint = Joint::spring(e1, e2, Vec3::ZERO, Vec3::ZERO, 1.0, 100.0, 10.0);
        assert_eq!(joint.joint_type, JointType::Spring);
        
        if let JointData::Spring(data) = joint.data {
            assert_eq!(data.rest_length, 1.0);
            assert_eq!(data.stiffness, 100.0);
            assert_eq!(data.damping, 10.0);
        } else {
            panic!("Expected spring joint data");
        }
    }
}
