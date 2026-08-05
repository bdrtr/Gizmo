//! Ragdoll authoring and spawning: bone definitions, a builder for a stock humanoid
//! skeleton, and a spawner that turns them into jointed rigid bodies.
//!
//! The pipeline is [`RagdollBuilder`] → `Vec<`[`RagdollBoneDef`]`>` → [`spawn_ragdoll`]
//! → [`RagdollInstance`]. A bone is authored as a parent-relative offset in metres plus
//! a capsule size, a mass and a joint description; spawning turns each bone into one
//! dynamic capsule body and each non-root bone into one joint pushed into the
//! [`PhysicsWorld`] resource.
//!
//! Scope: this module only *builds* skeletons. There is no animation blending, no
//! get-up/recovery behaviour and no pose matching here. Bone-vs-bone contacts are
//! suppressed only between a bone and the bone it is directly jointed to (those joints
//! are created with collision disabled); any other pair of bones — e.g. the two thighs —
//! collides normally.

use std::collections::HashMap;

use gizmo_math::Vec3;

use gizmo_core::entity::Entity;
use gizmo_core::world::World;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::joints::{Joint, JointData, JointType};
use gizmo_physics_rigid::world::PhysicsWorld;

/// Identifies a bone within a humanoid ragdoll skeleton.
///
/// The variant is a *label* only — it carries no geometry, mass or joint data (all of
/// that lives in [`RagdollBoneDef`]). It is the identity key that links a child to its
/// parent during [`spawn_ragdoll`] and the key [`RagdollInstance::entity_of`] looks up
/// afterwards, so a bone type should appear at most once per skeleton: with duplicates,
/// a child attaches to the most recently spawned bone of that type while `entity_of`
/// keeps returning the first one.
///
/// The anatomical relationships named below describe the stock layout built by
/// [`RagdollBuilder::create_humanoid`] (Y-up, `Left*` bones at negative X and `Right*`
/// at positive X). A hand-authored skeleton may wire the same variants up any way it
/// likes — nothing in this module mirrors or validates geometry by side.
// `Hash` is derived so the type can be used as a `HashMap` key while resolving
// parent -> child bone world positions in [`spawn_ragdoll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RagdollBoneType {
    /// Skull. A leaf in the stock humanoid, hanging off [`Self::Torso`] through a
    /// ball-socket "neck".
    Head,
    /// Chest / upper spine. In the stock humanoid it is the parent of the head and of
    /// both upper arms, and is itself ball-socketed to [`Self::Pelvis`].
    Torso,
    /// Hips. The root of the stock humanoid (`parent_type: None`), so it is the bone
    /// that receives [`RagdollBuilder::new`]'s world position and the only one spawned
    /// without a joint back to a parent.
    Pelvis,
    /// Left upper arm (humerus): ball-socketed to the torso at the shoulder.
    LeftUpperArm,
    /// Left forearm: hinged to [`Self::LeftUpperArm`] at the elbow, limited in the
    /// stock humanoid to 0..0.8π rad so it bends one way only.
    LeftLowerArm,
    /// Right upper arm (humerus): ball-socketed to the torso at the shoulder.
    RightUpperArm,
    /// Right forearm: hinged to [`Self::RightUpperArm`] at the elbow, limited in the
    /// stock humanoid to 0..0.8π rad so it bends one way only.
    RightLowerArm,
    /// Left thigh (femur): ball-socketed to the pelvis at the hip.
    LeftUpperLeg,
    /// Left shin: hinged to [`Self::LeftUpperLeg`] at the knee, limited in the stock
    /// humanoid to −0.8π..0 rad — the opposite direction from the elbow.
    LeftLowerLeg,
    /// Right thigh (femur): ball-socketed to the pelvis at the hip.
    RightUpperLeg,
    /// Right shin: hinged to [`Self::RightUpperLeg`] at the knee, limited in the stock
    /// humanoid to −0.8π..0 rad — the opposite direction from the elbow.
    RightLowerLeg,
}

/// Definition of a single ragdoll bone: its shape, mass and the joint that
/// connects it to its parent.
#[derive(Debug, Clone)]
pub struct RagdollBoneDef {
    /// Identity of this bone, and the name children give in their
    /// [`Self::parent_type`]. Should be unique within one skeleton — see
    /// [`RagdollBoneType`] for what duplicates do.
    pub bone_type: RagdollBoneType,

    /// The bone this one hangs from, or `None` for a root.
    ///
    /// A root gets no joint at all (it is a free body) and its [`Self::local_pos`] is
    /// read as a world position rather than an offset. A named parent must appear
    /// *earlier* in the definition list: [`spawn_ragdoll`] resolves the skeleton in a
    /// single forward pass. An unresolved parent is substituted with `Vec3::ZERO`, so the
    /// bone's [`Self::local_pos`] ends up interpreted as an absolute world position and it
    /// gets no joint (a `tracing` warning is emitted; the call does not fail).
    pub parent_type: Option<RagdollBoneType>,

    /// Offset from the parent bone's origin, in metres.
    ///
    /// Pure translation in world axes: the parent's rotation is *not* applied (bones
    /// spawn with identity rotation, and positions are chained as
    /// `world(child) = world(parent) + local_pos`). For a root bone this field instead
    /// holds the world position — [`RagdollBuilder::build`] puts it there by adding the
    /// builder's root position to root bones only.
    pub local_pos: Vec3,

    /// Capsule radius in metres. It also caps the bone's ends, so the collider's total
    /// extent along its axis is `length + 2 * radius`.
    pub radius: f32,

    /// Length of the capsule's cylindrical section in metres, along the body's local
    /// +Y; the hemispherical caps add [`Self::radius`] beyond each end.
    ///
    /// [`spawn_ragdoll`] halves it into a half-height and clamps that to at least
    /// 0.01 m, so zero or negative lengths degenerate into a near-sphere of `radius`
    /// rather than an invalid collider.
    pub length: f32,

    /// Bone mass in kilograms, clamped to at least 0.01 kg at spawn. The rotational
    /// inertia is not authored here: it is derived from this mass together with the
    /// capsule's radius and half-height.
    pub mass: f32,

    /// Constraint attaching this bone to its parent; ignored for root bones.
    ///
    /// It also selects how [`Self::joint_axis`] and [`Self::limits`] are read. Variants
    /// this module does not handle explicitly (`JointType` is `#[non_exhaustive]`) fall
    /// back to a rigid fixed joint, so the skeleton stays connected — as of this version
    /// `D6` is the one variant that takes that path, i.e. a `D6` bone is silently rigid
    /// rather than 6-DOF.
    pub joint_type: JointType,

    /// Joint anchor expressed in the *parent* body's local frame, in metres.
    pub local_anchor_parent: Vec3,

    /// Joint anchor expressed in *this* bone's local frame, in metres.
    ///
    /// Together with the parent anchor and [`Self::local_pos`] it fixes the anchor
    /// separation at the authored rest pose,
    /// `|local_pos + local_anchor_child - local_anchor_parent|`, which `Spring` and
    /// `Distance` bones adopt as their rest length / rope length. That is deliberately
    /// *not* the bone centre-to-centre distance `|local_pos|`.
    pub local_anchor_child: Vec3,

    /// Axis for the joint types that need one: the rotation axis of a `Hinge`, the free
    /// axis of a `Slider`, and the twist axis of a `BallSocket` cone-twist. Ignored by
    /// `Fixed`, `Spring` and `Distance`.
    ///
    /// Since every bone spawns with identity rotation, the axis starts out coinciding
    /// with the world axes. It need not be unit length — hinge and slider normalise it
    /// and substitute +Y if it is ~zero, while a ~zero axis on a ball socket silently
    /// disables that joint's twist limit.
    pub joint_axis: Vec3,

    /// Optional joint limits, read according to [`Self::joint_type`]:
    ///
    /// * `Hinge` — `(lower, upper)` rotation about [`Self::joint_axis`] in radians;
    ///   `None` leaves the hinge free.
    /// * `BallSocket` — a single cone half-angle `max(|lower|, |upper|)`, i.e. the
    ///   largest swing away from the bone's rest orientation, in radians; the pair is
    ///   *not* used asymmetrically. `None` yields a 1.2 rad (≈69°) default cone. The
    ///   ±0.6 rad twist limit and the soft-limit compliance applied to ball sockets are
    ///   fixed and cannot be authored through this field.
    /// * `Distance` — `(min, max)` anchor separation in metres; `None` yields a rope
    ///   from 0 to the rest-pose anchor separation.
    /// * `Fixed`, `Slider`, `Spring` — ignored.
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
    /// Starts an empty builder whose root bones will be placed at `root_pos`
    /// (world-space metres).
    ///
    /// The offset is applied only to bones with `parent_type == None`, and only when
    /// [`Self::build`] runs; child bones keep their parent-relative offsets and inherit
    /// the translation through the parent chain. `Default` uses `Vec3::ZERO`.
    pub fn new(root_pos: Vec3) -> Self {
        Self {
            bones: Vec::new(),
            root_pos,
        }
    }

    /// Appends a bone definition and returns `&mut self` for chaining.
    ///
    /// Order matters: a bone must be added *after* the bone named by its
    /// `parent_type`, because [`spawn_ragdoll`] resolves world positions in a single
    /// forward pass. Nothing is validated here — a dangling or out-of-order parent is
    /// only diagnosed at spawn time (a `tracing` warning; the bone's `local_pos` is then
    /// treated as a world position and it is left unjointed), and duplicate bone types are
    /// not diagnosed at all.
    pub fn add_bone(&mut self, bone: RagdollBoneDef) -> &mut Self {
        self.bones.push(bone);
        self
    }

    /// Appends the stock 11-bone humanoid: a `Pelvis` root carrying a `Torso` → `Head`
    /// spine, two arms and two legs. It measures ≈1.83 m from the bottom of a lower-leg
    /// capsule to the top of the head capsule and weighs 75 kg in total, and is
    /// authored standing upright with its origin at the pelvis.
    ///
    /// Spine, neck, shoulders and hips are ball sockets with no authored limits, which
    /// [`spawn_ragdoll`] turns into cone-twist joints with the default 1.2 rad cone and
    /// ±0.6 rad twist. Elbows and knees are hinges about local X, limited to
    /// `0..0.8π` and `-0.8π..0` rad respectively so they fold in opposite directions.
    ///
    /// Bones are *appended*, not replaced: calling this twice — or after adding a bone
    /// of one of the same types — produces duplicate bone types, which
    /// [`RagdollBoneType`] describes the consequences of. Returns `&mut self` for
    /// chaining.
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

    /// Consumes the builder and returns the bone definitions in the order they were
    /// added.
    ///
    /// The only transformation applied is the root offset from [`Self::new`]: it is added
    /// to the `local_pos` of every bone with `parent_type == None`, promoting that field
    /// from an offset to a world position in metres. Child bones are returned untouched —
    /// their `local_pos` stays parent-relative and is chained onto the parent later, by
    /// [`spawn_ragdoll`].
    ///
    /// Nothing is validated or reordered here: dangling parents, bones listed before their
    /// parent, and duplicate bone types all pass straight through. A definition list
    /// containing no root bone at all is likewise returned as authored, which silently
    /// discards the root position.
    pub fn build(mut self) -> Vec<RagdollBoneDef> {
        for bone in &mut self.bones {
            if bone.parent_type.is_none() {
                bone.local_pos += self.root_pos;
            }
        }
        self.bones
    }

    /// Builds the definitions and spawns them into `world` in one call — exactly
    /// `spawn_ragdoll(world, self.build())`. See [`spawn_ragdoll`] for what is created and
    /// for what happens when `world` holds no [`PhysicsWorld`] resource.
    ///
    /// Takes the builder by value, while [`Self::add_bone`] and [`Self::create_humanoid`]
    /// return `&mut Self`; a chain ending in `.spawn(..)` therefore cannot compile — bind
    /// the builder to a `let mut` binding first.
    pub fn spawn(self, world: &mut World) -> RagdollInstance {
        spawn_ragdoll(world, self.build())
    }
}

/// A live, simulatable ragdoll: the spawned rigid-body entities (one per bone)
/// and the indices of the joints that were pushed into the [`PhysicsWorld`]
/// resource connecting them.
///
/// It is a record of what [`spawn_ragdoll`] created, not a handle that owns it: dropping
/// it leaves the bodies and joints in place, and it has no despawn or teardown method.
/// The derived `Default` is the empty record (no bones, no joints) that spawning starts
/// from, and describes no ragdoll at all.
#[derive(Debug, Clone, Default)]
pub struct RagdollInstance {
    /// `(bone type, spawned entity)` for every bone, in spawn order — one entry per
    /// [`RagdollBoneDef`] handed to [`spawn_ragdoll`], so this runs parallel to the
    /// definition list even where a bone failed to attach to its parent.
    ///
    /// Recorded once at spawn and never updated afterwards, so an entry outlives its bone
    /// being despawned.
    pub bones: Vec<(RagdollBoneType, Entity)>,
    /// Indices into `PhysicsWorld::joints` for the joints this ragdoll created, in bone
    /// order — a contiguous ascending run appended to the end of that vector at spawn
    /// time, and empty when no [`PhysicsWorld`] resource was present.
    pub joint_indices: Vec<usize>,
}

impl RagdollInstance {
    /// The entity spawned for `bone_type`, or `None` if this skeleton has no bone of that
    /// type.
    ///
    /// A linear scan of [`Self::bones`] in spawn order, so with duplicate bone types it
    /// resolves to the *first* one — whereas joints naming that type as a parent were
    /// wired to the last (see [`RagdollBoneType`]). Cost is O(bones); for a stock humanoid
    /// that is a scan of 11 entries.
    pub fn entity_of(&self, bone_type: RagdollBoneType) -> Option<Entity> {
        self.bones
            .iter()
            .find(|(bt, _)| *bt == bone_type)
            .map(|(_, e)| *e)
    }

    /// Number of rigid bodies spawned, which is always exactly the number of
    /// [`RagdollBoneDef`]s given to [`spawn_ragdoll`]: every definition yields a body even
    /// when its parent could not be resolved (such a bone is placed at its own `local_pos`,
    /// read as a world position, and left unjointed). 11 for the stock humanoid.
    pub fn bone_count(&self) -> usize {
        self.bones.len()
    }

    /// Number of joints registered for this ragdoll — one per non-root bone whose parent
    /// had already been spawned.
    ///
    /// It is therefore `0` for a single-bone skeleton, and also `0` when the world held no
    /// [`PhysicsWorld`] resource at spawn time — in which case the bodies do exist but
    /// nothing constrains them into a skeleton. 10 for the stock humanoid.
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
#[tracing::instrument(skip_all, name = "spawn_ragdoll", fields(bone_count = bones.len()))]
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
                // Parent topolojik sırayla ÖNCE gelmiş olmalı (build() garanti eder). El ile
                // sırasız bir liste verilirse parent çözülmez → sessizce ZERO'ya düşüp kemiği
                // yanlış konuma koyardık; artık görünür bir uyarı ver (davranış korunur).
                let parent_pos = match resolved.get(&parent) {
                    Some((_, p)) => *p,
                    None => {
                        tracing::warn!(
                            bone = ?bone.bone_type,
                            parent = ?parent,
                            "[Ragdoll] parent bone not resolved (bones out of topological order?) — placing child at origin"
                        );
                        Vec3::ZERO
                    }
                };
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
            } else {
                // Non-root kemik ama parent'ı spawn edilmemiş → eklem OLUŞTURULMAZ (kemik
                // iskeleeten kopuk kalır). Eskiden sessizdi; artık uyar.
                tracing::warn!(
                    bone = ?bone.bone_type,
                    parent = ?parent_type,
                    "[Ragdoll] parent not spawned — bone left unconnected (no joint created)"
                );
            }
        }
    }

    // Push joints into the PhysicsWorld resource (persistent — not synced from
    // the ECS). Record their indices so callers can inspect/break them.
    if !pending_joints.is_empty() {
        if let Some(mut physics_world) = world.get_resource_mut::<PhysicsWorld>() {
            let joint_count = pending_joints.len();
            for joint in pending_joints {
                instance.joint_indices.push(physics_world.joints.len());
                physics_world.joints.push(joint);
            }
            tracing::debug!(joint_count, "[Ragdoll] joints registered in PhysicsWorld");
        } else {
            // Kemikler spawn edildi ama eklem solver'ı yok → iskelet KISITSIZ (dağılır). Doküman
            // bunu destekli sayar, yine de neredeyse her zaman yanlış kurulum → uyar.
            tracing::warn!(
                pending_joints = pending_joints.len(),
                "[Ragdoll] no PhysicsWorld resource — skeleton spawned WITHOUT joints (bones unconstrained)"
            );
        }
    }

    tracing::debug!(
        bones = instance.bones.len(),
        joints = instance.joint_indices.len(),
        "[Ragdoll] spawn complete"
    );

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
        _ => {
            tracing::debug!(
                bone = ?bone.bone_type,
                joint_type = ?bone.joint_type,
                "[Ragdoll] unhandled joint type — falling back to a rigid Fixed joint"
            );
            Joint::fixed(
                parent,
                child,
                bone.local_anchor_parent,
                bone.local_anchor_child,
            )
        }
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

    /// `build()` must add the root world position ONLY to root bones (`parent_type ==
    /// None`); child bones keep their parent-local offset (they are resolved onto the
    /// parent later, in `spawn_ragdoll`). Feeding the offset to children too would
    /// double-count the root translation for the whole skeleton.
    #[test]
    fn build_offsets_only_root_bones() {
        let root = Vec3::new(1.0, 5.0, -2.0);
        let mut builder = RagdollBuilder::new(root);
        builder.create_humanoid();
        let defs = builder.build();

        let pelvis = defs.iter().find(|d| d.bone_type == RagdollBoneType::Pelvis).unwrap();
        assert!(pelvis.parent_type.is_none(), "pelvis is the root");
        assert_eq!(pelvis.local_pos, root, "root bone gets the world offset");

        let torso = defs.iter().find(|d| d.bone_type == RagdollBoneType::Torso).unwrap();
        assert_eq!(
            torso.local_pos,
            Vec3::new(0.0, 0.4, 0.0),
            "child bone keeps its parent-local offset, unshifted"
        );
    }

    /// `RagdollInstance::entity_of` is a lookup, not a bijection: it returns the spawned
    /// entity for a present bone and `None` for a bone type that was never spawned.
    #[test]
    fn entity_of_returns_none_for_unspawned_bone() {
        use super::RagdollBoneDef;
        use gizmo_physics_rigid::joints::JointType;
        let mut world = World::new();
        let mut builder = RagdollBuilder::new(Vec3::ZERO);
        // Spawn a lone pelvis (no head).
        builder.add_bone(RagdollBoneDef {
            bone_type: RagdollBoneType::Pelvis,
            parent_type: None,
            local_pos: Vec3::ZERO,
            radius: 0.15,
            length: 0.2,
            mass: 10.0,
            joint_type: JointType::Fixed,
            local_anchor_parent: Vec3::ZERO,
            local_anchor_child: Vec3::ZERO,
            joint_axis: Vec3::Y,
            limits: None,
        });
        let instance = builder.spawn(&mut world);
        assert!(instance.entity_of(RagdollBoneType::Pelvis).is_some(), "spawned bone resolves");
        assert!(
            instance.entity_of(RagdollBoneType::Head).is_none(),
            "an unspawned bone type must resolve to None"
        );
    }

    /// A `Distance` bone joint rests taut at the ANCHOR-to-anchor separation (min 0 →
    /// rope) by default, and honours explicit `(min, max)` limits when given. The
    /// anchor gap is `|local_pos + anchor_child - anchor_parent|`, matching the Spring
    /// convention (not the centre-to-centre `local_pos.length()`).
    #[test]
    fn distance_bone_joint_uses_anchor_gap_and_honours_limits() {
        use super::{build_bone_joint, RagdollBoneDef};
        use gizmo_physics_core::BodyHandle;
        use gizmo_physics_rigid::joints::{JointData, JointType};

        let mk = |limits| RagdollBoneDef {
            bone_type: RagdollBoneType::LeftLowerArm,
            parent_type: Some(RagdollBoneType::LeftUpperArm),
            local_pos: Vec3::new(0.0, 0.5, 0.0),
            radius: 0.06,
            length: 0.25,
            mass: 2.0,
            joint_type: JointType::Distance,
            local_anchor_parent: Vec3::new(0.0, 0.1, 0.0),
            local_anchor_child: Vec3::new(0.0, -0.1, 0.0),
            joint_axis: Vec3::Y,
            limits,
        };

        // Default: rope from 0 to the anchor gap = |(0,0.5,0)+(0,-0.1,0)-(0,0.1,0)| = 0.3.
        let joint = build_bone_joint(&mk(None), BodyHandle::from_id(0), BodyHandle::from_id(1));
        let JointData::Distance(d) = joint.data else { panic!("expected a distance joint") };
        assert_eq!(d.min_length, 0.0, "default distance bone is a rope (min 0)");
        assert!(
            (d.max_length - 0.3).abs() < 1e-6,
            "max must be the 0.3 anchor gap, got {} (0.5 = centre-to-centre bug)",
            d.max_length
        );

        // Explicit limits pass through (clamped to be sane by Joint::distance).
        let joint = build_bone_joint(&mk(Some((0.2, 1.5))), BodyHandle::from_id(0), BodyHandle::from_id(1));
        let JointData::Distance(d) = joint.data else { panic!("expected a distance joint") };
        assert!((d.min_length - 0.2).abs() < 1e-6, "explicit min honoured, got {}", d.min_length);
        assert!((d.max_length - 1.5).abs() < 1e-6, "explicit max honoured, got {}", d.max_length);
    }

    /// A `Hinge` bone joint maps `limits` → `use_limits` + `lower/upper_limit`, and
    /// leaves `use_limits` false when no limits are authored. The joint axis is
    /// normalised into the hinge data.
    #[test]
    fn hinge_bone_joint_maps_limits_and_axis() {
        use super::{build_bone_joint, RagdollBoneDef};
        use gizmo_physics_core::BodyHandle;
        use gizmo_physics_rigid::joints::{JointData, JointType};

        let mk = |limits| RagdollBoneDef {
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
            limits,
        };

        let joint = build_bone_joint(&mk(Some((-0.5, 2.0))), BodyHandle::from_id(0), BodyHandle::from_id(1));
        let JointData::Hinge(d) = joint.data else { panic!("expected a hinge joint") };
        assert!(d.use_limits, "authored limits must enable the hinge limit");
        assert!((d.lower_limit - -0.5).abs() < 1e-6, "lower limit mapped, got {}", d.lower_limit);
        assert!((d.upper_limit - 2.0).abs() < 1e-6, "upper limit mapped, got {}", d.upper_limit);
        assert!((d.axis - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-6, "axis normalised, got {:?}", d.axis);

        let joint = build_bone_joint(&mk(None), BodyHandle::from_id(0), BodyHandle::from_id(1));
        let JointData::Hinge(d) = joint.data else { panic!("expected a hinge joint") };
        assert!(!d.use_limits, "no authored limits → free hinge (use_limits false)");
    }

    /// A `Fixed` bone joint carries the parent/child anchors straight through and is
    /// created with collision disabled (adjacent overlapping capsules must not fight
    /// the joint) — the documented default for ragdoll joints.
    #[test]
    fn fixed_bone_joint_carries_anchors_and_disables_collision() {
        use super::{build_bone_joint, RagdollBoneDef};
        use gizmo_physics_core::BodyHandle;
        use gizmo_physics_rigid::joints::{JointData, JointType};

        let bone = RagdollBoneDef {
            bone_type: RagdollBoneType::Torso,
            parent_type: Some(RagdollBoneType::Pelvis),
            local_pos: Vec3::new(0.0, 0.4, 0.0),
            radius: 0.18,
            length: 0.4,
            mass: 25.0,
            joint_type: JointType::Fixed,
            local_anchor_parent: Vec3::new(0.0, 0.2, 0.0),
            local_anchor_child: Vec3::new(0.0, -0.2, 0.0),
            joint_axis: Vec3::Y,
            limits: None,
        };
        let joint = build_bone_joint(&bone, BodyHandle::from_id(3), BodyHandle::from_id(4));
        assert!(matches!(joint.data, JointData::Fixed), "joint type must be Fixed");
        assert_eq!(joint.local_anchor_a, bone.local_anchor_parent, "parent anchor carried through");
        assert_eq!(joint.local_anchor_b, bone.local_anchor_child, "child anchor carried through");
        assert!(!joint.collision_enabled, "ragdoll joints disable adjacent-body collision");
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
