use crate::components::{RigidBody, Transform, Velocity};
use gizmo_core::entity::Entity;
use gizmo_math::Vec3;
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

                match joint.joint_type {
                    JointType::Fixed => {
                        self.solve_fixed_joint(joint, bodies, idx_a, idx_b, dt);
                    }
                    JointType::Hinge => {
                        self.solve_hinge_joint(joint, bodies, idx_a, idx_b, dt);
                    }
                    JointType::BallSocket => {
                        self.solve_ball_socket_joint(joint, bodies, idx_a, idx_b, dt);
                    }
                    JointType::Slider => {
                        self.solve_slider_joint(joint, bodies, idx_a, idx_b, dt);
                    }
                    JointType::Spring => {
                        self.solve_spring_joint(joint, bodies, idx_a, idx_b, dt);
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
        // Extract data we need before mutable borrow
        let anchor_a = bodies[idx_a].1.position + bodies[idx_a].1.rotation * joint.local_anchor_a;
        let anchor_b = bodies[idx_b].1.position + bodies[idx_b].1.rotation * joint.local_anchor_b;

        // Position error
        let error = anchor_b - anchor_a;
        let error_length = error.length();

        if error_length < 0.001 {
            return;
        }

        let direction = error / error_length;

        // Calculate effective mass
        let r_a = anchor_a - bodies[idx_a].1.position;
        let r_b = anchor_b - bodies[idx_b].1.position;

        let inv_mass_a = bodies[idx_a].0.inv_mass();
        let inv_mass_b = bodies[idx_b].0.inv_mass();

        let inv_inertia_a = bodies[idx_a].0.inv_world_inertia_tensor(bodies[idx_a].1.rotation);
        let inv_inertia_b = bodies[idx_b].0.inv_world_inertia_tensor(bodies[idx_b].1.rotation);

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

        let is_dynamic_a = bodies[idx_a].0.is_dynamic();
        let is_dynamic_b = bodies[idx_b].0.is_dynamic();

        // Apply impulse - split mutable borrows
        if idx_a < idx_b {
            let (left, right) = bodies.split_at_mut(idx_b);
            if is_dynamic_a {
                left[idx_a].2.linear -= impulse * inv_mass_a;
                left[idx_a].2.angular -= inv_inertia_a * r_a.cross(impulse);
            }
            if is_dynamic_b {
                right[0].2.linear += impulse * inv_mass_b;
                right[0].2.angular += inv_inertia_b * r_b.cross(impulse);
            }
        } else {
            let (left, right) = bodies.split_at_mut(idx_a);
            if is_dynamic_b {
                left[idx_b].2.linear += impulse * inv_mass_b;
                left[idx_b].2.angular += inv_inertia_b * r_b.cross(impulse);
            }
            if is_dynamic_a {
                right[0].2.linear -= impulse * inv_mass_a;
                right[0].2.angular -= inv_inertia_a * r_a.cross(impulse);
            }
        }

        // Check for breaking
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
        // First solve position constraint (like fixed joint)
        self.solve_fixed_joint(joint, bodies, idx_a, idx_b, dt);

        // Extract hinge data
        if let JointData::Hinge(ref mut data) = joint.data {
            let axis_world = bodies[idx_a].1.rotation * data.axis;
            let ref_a = bodies[idx_a].1.rotation * Vec3::X;
            let ref_b = bodies[idx_b].1.rotation * Vec3::X;

            let projected_a = (ref_a - axis_world * ref_a.dot(axis_world)).normalize();
            let projected_b = (ref_b - axis_world * ref_b.dot(axis_world)).normalize();
            
            data.current_angle = projected_a.dot(projected_b).acos();
            if projected_a.cross(projected_b).dot(axis_world) < 0.0 {
                data.current_angle = -data.current_angle;
            }
        }

        // TODO: Implement full angle limits and motor
    }

    fn solve_ball_socket_joint(
        &self,
        joint: &mut Joint,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        // Ball-socket is just a fixed position constraint
        self.solve_fixed_joint(joint, bodies, idx_a, idx_b, dt);
        
        // TODO: Implement cone limit
    }

    fn solve_slider_joint(
        &self,
        joint: &mut Joint,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        // For now, just solve as fixed joint
        self.solve_fixed_joint(joint, bodies, idx_a, idx_b, dt);
        // TODO: Implement slider joint solver properly
    }

    fn solve_spring_joint(
        &self,
        joint: &Joint,
        bodies: &mut [(RigidBody, Transform, Velocity)],
        idx_a: usize,
        idx_b: usize,
        dt: f32,
    ) {
        // Clone joint to avoid borrow issues
        let mut joint_copy = joint.clone();
        self.solve_fixed_joint(&mut joint_copy, bodies, idx_a, idx_b, dt);
        // TODO: Implement spring joint solver properly
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
            panic!("Expected hinge joint data");
        }
    }

    #[test]
    fn test_spring_joint() {
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(2, 0);

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
