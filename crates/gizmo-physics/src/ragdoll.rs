use gizmo_math::Vec3;

use crate::joints::JointType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RagdollBoneType {
    Head,
    Torso,
    Pelvis,
    LeftUpperArm,
    LeftLowerArm,
    RightUpperArm,
    RightLowerArm,
    LeftUpperLeg,
    LeftLowerLeg,
    RightUpperLeg,
    RightLowerLeg,
}

pub struct RagdollBoneDef {
    pub bone_type: RagdollBoneType,
    pub parent_type: Option<RagdollBoneType>,
    pub local_pos: Vec3, // Local position relative to the parent
    pub radius: f32,
    pub length: f32,
    pub mass: f32,
    pub joint_type: JointType, 
    pub local_anchor_parent: Vec3,
    pub local_anchor_child: Vec3,
    pub joint_axis: Vec3,
    pub limits: Option<(f32, f32)>,
}

pub struct RagdollBuilder {
    bones: Vec<RagdollBoneDef>,
    root_pos: Vec3,
}

impl Default for RagdollBuilder {
    fn default() -> Self {
        Self::new(Vec3::ZERO)
    }
}

impl RagdollBuilder {
    pub fn new(root_pos: Vec3) -> Self {
        Self {
            bones: Vec::new(),
            root_pos,
        }
    }

    pub fn add_bone(&mut self, bone: RagdollBoneDef) -> &mut Self {
        self.bones.push(bone);
        self
    }

    pub fn create_humanoid(&mut self) -> &mut Self {
        self.add_bone(RagdollBoneDef {
            bone_type: RagdollBoneType::Pelvis,
            parent_type: None,
            local_pos: Vec3::ZERO,
            radius: 0.15,
            length: 0.2,
            mass: 15.0,
            joint_type: JointType::Fixed,
            local_anchor_parent: Vec3::ZERO,
            local_anchor_child: Vec3::ZERO,
            joint_axis: Vec3::Y,
            limits: None,
        })
        .add_bone(RagdollBoneDef {
            bone_type: RagdollBoneType::Torso,
            parent_type: Some(RagdollBoneType::Pelvis),
            local_pos: Vec3::new(0.0, 0.4, 0.0),
            radius: 0.18,
            length: 0.4,
            mass: 25.0,
            joint_type: JointType::BallSocket,
            local_anchor_parent: Vec3::new(0.0, 0.2, 0.0),
            local_anchor_child: Vec3::new(0.0, -0.2, 0.0),
            joint_axis: Vec3::Y,
            limits: None,
        })
        .add_bone(RagdollBoneDef {
            bone_type: RagdollBoneType::Head,
            parent_type: Some(RagdollBoneType::Torso),
            local_pos: Vec3::new(0.0, 0.35, 0.0),
            radius: 0.12,
            length: 0.1,
            mass: 5.0,
            joint_type: JointType::BallSocket,
            local_anchor_parent: Vec3::new(0.0, 0.2, 0.0),
            local_anchor_child: Vec3::new(0.0, -0.1, 0.0),
            joint_axis: Vec3::Y,
            limits: None, // Head should have more freedom than just 1 axis
        })
        // Left Arm
        .add_bone(RagdollBoneDef {
            bone_type: RagdollBoneType::LeftUpperArm,
            parent_type: Some(RagdollBoneType::Torso),
            local_pos: Vec3::new(-0.3, 0.2, 0.0),
            radius: 0.08,
            length: 0.3,
            mass: 3.0,
            joint_type: JointType::BallSocket,
            local_anchor_parent: Vec3::new(-0.2, 0.2, 0.0),
            local_anchor_child: Vec3::new(0.0, 0.15, 0.0),
            joint_axis: Vec3::Y,
            limits: None,
        })
        .add_bone(RagdollBoneDef {
            bone_type: RagdollBoneType::LeftLowerArm,
            parent_type: Some(RagdollBoneType::LeftUpperArm),
            local_pos: Vec3::new(0.0, -0.3, 0.0),
            radius: 0.06,
            length: 0.25,
            mass: 2.0,
            joint_type: JointType::Hinge,
            local_anchor_parent: Vec3::new(0.0, -0.15, 0.0),
            local_anchor_child: Vec3::new(0.0, 0.125, 0.0),
            joint_axis: Vec3::new(1.0, 0.0, 0.0),
            limits: Some((0.0, std::f32::consts::PI * 0.8)), // Elbow can only bend one way
        })
        // Right Arm
        .add_bone(RagdollBoneDef {
            bone_type: RagdollBoneType::RightUpperArm,
            parent_type: Some(RagdollBoneType::Torso),
            local_pos: Vec3::new(0.3, 0.2, 0.0),
            radius: 0.08,
            length: 0.3,
            mass: 3.0,
            joint_type: JointType::BallSocket,
            local_anchor_parent: Vec3::new(0.2, 0.2, 0.0),
            local_anchor_child: Vec3::new(0.0, 0.15, 0.0),
            joint_axis: Vec3::Y,
            limits: None,
        })
        .add_bone(RagdollBoneDef {
            bone_type: RagdollBoneType::RightLowerArm,
            parent_type: Some(RagdollBoneType::RightUpperArm),
            local_pos: Vec3::new(0.0, -0.3, 0.0),
            radius: 0.06,
            length: 0.25,
            mass: 2.0,
            joint_type: JointType::Hinge,
            local_anchor_parent: Vec3::new(0.0, -0.15, 0.0),
            local_anchor_child: Vec3::new(0.0, 0.125, 0.0),
            joint_axis: Vec3::new(1.0, 0.0, 0.0),
            limits: Some((0.0, std::f32::consts::PI * 0.8)),
        })
        // Left Leg
        .add_bone(RagdollBoneDef {
            bone_type: RagdollBoneType::LeftUpperLeg,
            parent_type: Some(RagdollBoneType::Pelvis),
            local_pos: Vec3::new(-0.15, -0.25, 0.0),
            radius: 0.1,
            length: 0.4,
            mass: 6.0,
            joint_type: JointType::BallSocket,
            local_anchor_parent: Vec3::new(-0.15, -0.1, 0.0),
            local_anchor_child: Vec3::new(0.0, 0.2, 0.0),
            joint_axis: Vec3::Y,
            limits: None,
        })
        .add_bone(RagdollBoneDef {
            bone_type: RagdollBoneType::LeftLowerLeg,
            parent_type: Some(RagdollBoneType::LeftUpperLeg),
            local_pos: Vec3::new(0.0, -0.4, 0.0),
            radius: 0.08,
            length: 0.35,
            mass: 4.0,
            joint_type: JointType::Hinge,
            local_anchor_parent: Vec3::new(0.0, -0.2, 0.0),
            local_anchor_child: Vec3::new(0.0, 0.175, 0.0),
            joint_axis: Vec3::new(1.0, 0.0, 0.0),
            limits: Some((-std::f32::consts::PI * 0.8, 0.0)), // Knee bends backward
        })
        // Right Leg
        .add_bone(RagdollBoneDef {
            bone_type: RagdollBoneType::RightUpperLeg,
            parent_type: Some(RagdollBoneType::Pelvis),
            local_pos: Vec3::new(0.15, -0.25, 0.0),
            radius: 0.1,
            length: 0.4,
            mass: 6.0,
            joint_type: JointType::BallSocket,
            local_anchor_parent: Vec3::new(0.15, -0.1, 0.0),
            local_anchor_child: Vec3::new(0.0, 0.2, 0.0),
            joint_axis: Vec3::Y,
            limits: None,
        })
        .add_bone(RagdollBoneDef {
            bone_type: RagdollBoneType::RightLowerLeg,
            parent_type: Some(RagdollBoneType::RightUpperLeg),
            local_pos: Vec3::new(0.0, -0.4, 0.0),
            radius: 0.08,
            length: 0.35,
            mass: 4.0,
            joint_type: JointType::Hinge,
            local_anchor_parent: Vec3::new(0.0, -0.2, 0.0),
            local_anchor_child: Vec3::new(0.0, 0.175, 0.0),
            joint_axis: Vec3::new(1.0, 0.0, 0.0),
            limits: Some((-std::f32::consts::PI * 0.8, 0.0)),
        });
        
        self
    }

    /// Consumes the builder, computes initial world positions for root bones,
    /// and returns the list of bone definitions to be spawned.
    pub fn build(mut self) -> Vec<RagdollBoneDef> {
        for bone in &mut self.bones {
            if bone.parent_type.is_none() {
                bone.local_pos += self.root_pos;
            }
        }
        self.bones
    }
}
