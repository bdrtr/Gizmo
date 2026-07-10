use std::collections::HashMap;

use gizmo_math::Vec3;

use gizmo_core::entity::Entity;
use gizmo_core::world::World;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::joints::{Joint, JointData, JointType};
use gizmo_physics_rigid::world::PhysicsWorld;

/// Identifies a bone within a humanoid ragdoll skeleton.
// `Hash` is derived so the type can be used as a `HashMap` key while resolving
// parent -> child bone world positions in [`spawn_ragdoll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
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

/// Definition of a single ragdoll bone: its shape, mass and the joint that
/// connects it to its parent.
#[derive(Debug, Clone)]
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

/// Builder that accumulates [`RagdollBoneDef`]s and resolves root world
/// positions when [`RagdollBuilder::build`] is called.
#[derive(Debug, Clone)]
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

    /// Convenience: build the definitions and spawn them into `world` in one
    /// call. See [`spawn_ragdoll`].
    pub fn spawn(self, world: &mut World) -> RagdollInstance {
        spawn_ragdoll(world, self.build())
    }
}

/// A live, simulatable ragdoll: the spawned rigid-body entities (one per bone)
/// and the indices of the joints that were pushed into the [`PhysicsWorld`]
/// resource connecting them.
#[derive(Debug, Clone, Default)]
pub struct RagdollInstance {
    /// `(bone type, spawned entity)` for every bone, in spawn order.
    pub bones: Vec<(RagdollBoneType, Entity)>,
    /// Indices into `PhysicsWorld::joints` for the joints this ragdoll created.
    pub joint_indices: Vec<usize>,
}

impl RagdollInstance {
    /// Look up the entity that was spawned for a given bone type.
    pub fn entity_of(&self, bone_type: RagdollBoneType) -> Option<Entity> {
        self.bones
            .iter()
            .find(|(bt, _)| *bt == bone_type)
            .map(|(_, e)| *e)
    }

    /// Number of spawned bones.
    pub fn bone_count(&self) -> usize {
        self.bones.len()
    }

    /// Number of joints created for this ragdoll.
    pub fn joint_count(&self) -> usize {
        self.joint_indices.len()
    }
}

/// Spawn a runtime, physically-simulated ragdoll from a list of bone
/// definitions (typically produced by [`RagdollBuilder::build`]).
///
/// For every bone this:
/// 1. spawns an ECS entity carrying `RigidBody` + `Collider` (capsule) +
///    `Transform` + `Velocity`, so the standard `physics_step_system` picks it
///    up on the next step, and
/// 2. for every non-root bone, pushes a [`Joint`] (Fixed / BallSocket / Hinge /
///    Slider / Spring, per the bone def) into the [`PhysicsWorld`] resource so
///    the rigid-joint solver actually constrains the skeleton.
///
/// Bone world positions are resolved by walking the parent chain
/// (`world(child) = world(parent) + child.local_pos`); definitions are expected
/// in topological order (parent before child), which `build()` guarantees.
///
/// Joints connecting a parent and its child are created with collision
/// disabled (the default for [`Joint`]), so adjacent (overlapping) capsules do
/// not fight the joint constraint.
///
/// If no [`PhysicsWorld`] resource is present the bodies are still spawned but
/// no joints are created (the returned `joint_indices` is empty).
pub fn spawn_ragdoll(world: &mut World, bones: Vec<RagdollBoneDef>) -> RagdollInstance {
    let mut instance = RagdollInstance::default();

    // bone type -> (entity, resolved world position)
    let mut resolved: HashMap<RagdollBoneType, (Entity, Vec3)> = HashMap::new();
    // Pending joints, resolved after all bodies exist (parent handles are known
    // by the time a child is processed thanks to topological order).
    let mut pending_joints: Vec<Joint> = Vec::new();

    for bone in &bones {
        // Resolve world position by chaining onto the parent (root bones already
        // carry their absolute position from `build()`).
        let world_pos = match bone.parent_type {
            Some(parent) => {
                let parent_pos = resolved.get(&parent).map(|(_, p)| *p).unwrap_or(Vec3::ZERO);
                parent_pos + bone.local_pos
            }
            None => bone.local_pos,
        };

        // Capsule body sized from the bone def.
        let collider = Collider::capsule(bone.radius, (bone.length * 0.5).max(0.01));
        let mut rb = RigidBody::new(bone.mass.max(0.01), true);
        rb.update_inertia_from_collider(&collider);
        rb.wake_up();

        let entity = world.spawn();
        world.add_component(entity, rb);
        world.add_component(entity, Transform::new(world_pos));
        world.add_component(entity, Velocity::default());
        world.add_component(entity, collider);

        resolved.insert(bone.bone_type, (entity, world_pos));
        instance.bones.push((bone.bone_type, entity));

        // Build the joint back to the parent.
        if let Some(parent_type) = bone.parent_type {
            if let Some((parent_entity, _)) = resolved.get(&parent_type).copied() {
                pending_joints.push(build_bone_joint(
                    bone,
                    BodyHandle::from_id(parent_entity.id()),
                    BodyHandle::from_id(entity.id()),
                ));
            }
        }
    }

    // Push joints into the PhysicsWorld resource (persistent — not synced from
    // the ECS). Record their indices so callers can inspect/break them.
    if !pending_joints.is_empty() {
        if let Some(mut physics_world) = world.get_resource_mut::<PhysicsWorld>() {
            for joint in pending_joints {
                instance.joint_indices.push(physics_world.joints.len());
                physics_world.joints.push(joint);
            }
        }
    }

    instance
}

/// Translate a [`RagdollBoneDef`] joint description into a rigid-solver
/// [`Joint`] connecting `parent` -> `child`.
fn build_bone_joint(bone: &RagdollBoneDef, parent: BodyHandle, child: BodyHandle) -> Joint {
    match bone.joint_type {
        JointType::Fixed => Joint::fixed(
            parent,
            child,
            bone.local_anchor_parent,
            bone.local_anchor_child,
        ),
        JointType::BallSocket => {
            let mut joint = Joint::ball_socket(
                parent,
                child,
                bone.local_anchor_parent,
                bone.local_anchor_child,
            );
            if let JointData::BallSocket(data) = &mut joint.data {
                // Swing cone: use the authored limit if given, else a moderate default so
                // shoulders/hips/torso/neck cannot hyperextend (they were previously free).
                data.use_cone_limit = true;
                data.cone_limit_angle = bone
                    .limits
                    .map(|(lo, hi)| lo.abs().max(hi.abs()))
                    .unwrap_or(1.2);
                // Twist limit about the bone axis — stops a limb spinning freely about
                // itself (the classic cone-twist ragdoll joint).
                data.use_twist_limit = true;
                data.twist_axis = bone.joint_axis;
                data.twist_lower = -0.6;
                data.twist_upper = 0.6;
                // Slightly soft limits (CFM) for a natural, springy joint feel.
                data.compliance = 0.001;
            }
            joint
        }
        JointType::Hinge => {
            let mut joint = Joint::hinge(
                parent,
                child,
                bone.local_anchor_parent,
                bone.local_anchor_child,
                bone.joint_axis,
            );
            if let (JointData::Hinge(data), Some((lo, hi))) = (&mut joint.data, bone.limits) {
                data.use_limits = true;
                data.lower_limit = lo;
                data.upper_limit = hi;
            }
            joint
        }
        JointType::Slider => Joint::slider(
            parent,
            child,
            bone.local_anchor_parent,
            bone.local_anchor_child,
            bone.joint_axis,
        ),
        JointType::Spring => {
            // Rest length is the ANCHOR-to-anchor separation at the initial pose
            // (what the spring solver compares `|anchor_b - anchor_a|` against),
            // NOT the bone centre-to-centre distance. Since
            // `child_world - parent_world == local_pos`, the anchor gap is
            // `local_pos + anchor_child - anchor_parent`. Using `local_pos.length()`
            // made an offset-anchor spring rest at the wrong length (fighting its
            // own initial pose).
            let rest_length =
                (bone.local_pos + bone.local_anchor_child - bone.local_anchor_parent).length();
            Joint::spring(
                parent,
                child,
                bone.local_anchor_parent,
                bone.local_anchor_child,
                rest_length,
                1000.0,
                50.0,
            )
        }
        JointType::Distance => {
            // Rope/distance bone: `limits` = (min, max) separation; default to a rope
            // whose max is the initial anchor gap so it hangs taut at the rest pose.
            let rest = (bone.local_pos + bone.local_anchor_child - bone.local_anchor_parent).length();
            let (min, max) = bone.limits.unwrap_or((0.0, rest));
            Joint::distance(
                parent,
                child,
                bone.local_anchor_parent,
                bone.local_anchor_child,
                min,
                max,
            )
        }
        // `JointType` is `#[non_exhaustive]`; fall back to a rigid fixed joint
        // for any future variant so the skeleton stays connected.
        _ => Joint::fixed(
            parent,
            child,
            bone.local_anchor_parent,
            bone.local_anchor_child,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{RagdollBoneType, RagdollBuilder};
    use gizmo_core::world::World;
    use gizmo_math::Vec3;
    use gizmo_physics_core::Transform;
    use gizmo_physics_rigid::physics_step_system;
    use gizmo_physics_rigid::world::PhysicsWorld;

    /// A humanoid ragdoll spawns one body per bone (11) and one joint per
    /// non-root bone (10), and the joints land in the `PhysicsWorld` resource.
    #[test]
    fn ragdoll_spawns_expected_body_and_joint_counts() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());

        let mut builder = RagdollBuilder::new(Vec3::new(0.0, 5.0, 0.0));
        builder.create_humanoid();
        let instance = builder.spawn(&mut world);

        assert_eq!(instance.bone_count(), 11, "humanoid has 11 bones");
        assert_eq!(instance.joint_count(), 10, "one joint per non-root bone");
        assert!(instance.entity_of(RagdollBoneType::Pelvis).is_some());
        assert!(instance.entity_of(RagdollBoneType::Head).is_some());

        // Joints were actually pushed into the resource.
        let pw = world.get_resource_mut::<PhysicsWorld>().unwrap();
        assert_eq!(pw.joints.len(), 10);
    }

    /// The humanoid's ball-socket joints (shoulders/hips/torso/neck/head) now carry a
    /// swing cone AND a twist limit (cone-twist) with soft compliance — so limbs can't
    /// hyperextend or spin freely about their own axis.
    #[test]
    fn humanoid_ballsocket_joints_are_cone_twist_limited() {
        use gizmo_physics_rigid::JointData;
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());
        let mut builder = RagdollBuilder::new(Vec3::new(0.0, 5.0, 0.0));
        builder.create_humanoid();
        builder.spawn(&mut world);

        let pw = world.get_resource_mut::<PhysicsWorld>().unwrap();
        let ball_sockets: Vec<_> = pw
            .joints
            .iter()
            .filter_map(|j| match j.data {
                JointData::BallSocket(d) => Some(d),
                _ => None,
            })
            .collect();
        assert!(!ball_sockets.is_empty(), "humanoid should have ball-socket joints");
        for d in &ball_sockets {
            assert!(d.use_cone_limit, "ragdoll ball-socket must have a swing cone limit");
            assert!(d.use_twist_limit, "ragdoll ball-socket must have a twist limit (no free spin)");
            assert!(d.compliance > 0.0, "ragdoll ball-socket limits should be soft");
        }
    }

    /// Without a `PhysicsWorld` resource the bodies still spawn but no joints
    /// are recorded.
    #[test]
    fn ragdoll_spawns_bodies_without_physics_world() {
        let mut world = World::new();
        let mut builder = RagdollBuilder::new(Vec3::ZERO);
        builder.create_humanoid();
        let instance = builder.spawn(&mut world);
        assert_eq!(instance.bone_count(), 11);
        assert_eq!(instance.joint_count(), 0);
    }

    /// A Spring-jointed bone must rest at the ANCHOR-to-anchor separation, not
    /// the bone centre-to-centre distance (which is what the spring solver
    /// actually compares against). Regression for the old `local_pos.length()`.
    #[test]
    fn spring_bone_rest_length_is_anchor_to_anchor() {
        use super::{build_bone_joint, RagdollBoneDef};
        use gizmo_physics_core::BodyHandle;
        use gizmo_physics_rigid::joints::{JointData, JointType};

        let bone = RagdollBoneDef {
            bone_type: RagdollBoneType::Head,
            parent_type: Some(RagdollBoneType::Torso),
            local_pos: Vec3::new(0.0, 0.4, 0.0),
            radius: 0.1,
            length: 0.2,
            mass: 3.0,
            joint_type: JointType::Spring,
            local_anchor_parent: Vec3::new(0.0, 0.1, 0.0),
            local_anchor_child: Vec3::new(0.0, -0.1, 0.0),
            joint_axis: Vec3::Y,
            limits: None,
        };
        let joint = build_bone_joint(&bone, BodyHandle::from_id(0), BodyHandle::from_id(1));
        let JointData::Spring(data) = joint.data else {
            panic!("expected a spring joint");
        };
        // Anchor gap = local_pos + anchor_child - anchor_parent
        //            = (0,0.4,0) + (0,-0.1,0) - (0,0.1,0) = (0,0.2,0) → 0.2.
        // The old centre-to-centre value would have been |local_pos| = 0.4.
        assert!(
            (data.rest_length - 0.2).abs() < 1e-6,
            "spring rest length must be the 0.2 anchor separation, got {} (0.4 = old centre-to-centre bug)",
            data.rest_length
        );
    }

    /// A freshly spawned ragdoll falls under gravity without producing NaN/Inf.
    #[test]
    fn ragdoll_falls_under_gravity_without_nan() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());

        let mut builder = RagdollBuilder::new(Vec3::new(0.0, 5.0, 0.0));
        builder.create_humanoid();
        let instance = builder.spawn(&mut world);

        let pelvis = instance.entity_of(RagdollBoneType::Pelvis).unwrap();
        let start_y = world.query::<&Transform>().unwrap().get(pelvis.id()).unwrap().position.y;

        let dt = 1.0 / 120.0;
        for _ in 0..60 {
            physics_step_system(&world, dt);
        }

        // Every bone must remain finite.
        for (_, entity) in &instance.bones {
            let p = world.query::<&Transform>().unwrap().get(entity.id()).unwrap().position;
            assert!(p.is_finite(), "ragdoll bone position went non-finite: {p:?}");
        }

        let end_y = world.query::<&Transform>().unwrap().get(pelvis.id()).unwrap().position.y;
        assert!(
            end_y < start_y - 0.05,
            "ragdoll should fall under gravity: pelvis y {start_y} -> {end_y}"
        );
    }
}
