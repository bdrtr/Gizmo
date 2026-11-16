//! **Root motion:** a clip that carries its character across the ground — a lunge, a vault, a
//! walk authored to travel — moves the entity rather than drawing the body away from it.
//!
//! An entity with a [`RootMotion`] beside its [`AnimationPlayer`](super::AnimationPlayer) or
//! [`AnimationStateMachine`](super::AnimationStateMachine) has the horizontal motion of one joint —
//! [`bone`](RootMotion::bone), or the skeleton's first joint without a parent — taken out of its
//! pose each frame: that joint is drawn where it stands in the bind pose across the ground (its
//! height, the bob, is kept), and how far the clip moved it this frame is
//! [`delta`](RootMotion::delta), in the model's own space. When the clip wraps, the motion runs on
//! through its end and from its start, so a looping walk travels without a jump back.
//!
//! What moves the entity is the game's: [`apply`](RootMotion::apply) adds the delta to the entity's
//! `Transform` (turned and scaled by it), which suits a character with no physics; one driven by a
//! character controller reads `delta` and feeds it to the controller instead. An entity with root
//! motion is posed every frame, whatever its size on screen: its motion is the game's, not a picture's.

use gizmo_math::Vec3;

/// See the [module](self).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RootMotion {
    /// The joint whose motion is the character's, by name; `None`: the first joint without a
    /// parent.
    pub bone: Option<String>,
    /// Whether the engine adds [`delta`](Self::delta) to the entity's `Transform` each frame.
    pub apply: bool,
    /// How far the clip moved the joint across the ground this frame, in the model's space; zero
    /// the first frame.
    #[serde(skip)]
    pub delta: Vec3,
    /// Where the joint was in the last frame's pose — the animation driver's, for the next delta.
    #[serde(skip)]
    pub last: Option<Vec3>,
}

impl RootMotion {
    /// Root motion from the skeleton's root joint, applied to the entity's `Transform`.
    #[must_use]
    pub fn applied() -> Self {
        Self { apply: true, ..Self::default() }
    }

    /// Root motion from the skeleton's root joint, left in [`delta`](Self::delta) for the game.
    #[must_use]
    pub fn read() -> Self {
        Self::default()
    }

    /// The same, from joint `bone`.
    #[must_use]
    pub fn from_bone(mut self, bone: &str) -> Self {
        self.bone = Some(bone.to_string());
        self
    }

    /// Which joint of `joints` it follows: [`bone`](Self::bone) by name, else the first with no
    /// parent.
    #[must_use]
    pub fn joint(&self, hierarchy: &super::SkeletonHierarchy) -> Option<usize> {
        match &self.bone {
            Some(name) => hierarchy.joints.iter().position(|j| &j.name == name),
            None => hierarchy.joints.iter().position(|j| j.parent_index.is_none()),
        }
    }

    /// Takes this frame's motion out of `pose`: the joint's travel across the ground since the last
    /// frame becomes [`delta`](Self::delta) and the joint is put back over its bind position (its
    /// height kept). `wrap`: the joint where the clip ends and where it begins, when the playhead went
    /// round this frame — the travel runs on through both.
    pub fn take(&mut self, hierarchy: &super::SkeletonHierarchy, pose: &mut [super::layers::JointPose], wrap: Option<(Vec3, Vec3)>) {
        let Some(i) = self.joint(hierarchy).filter(|&i| i < pose.len()) else {
            self.delta = Vec3::ZERO;
            return;
        };
        let now = pose[i].0;
        let travel = match (self.last, wrap) {
            (None, _) => Vec3::ZERO,
            (Some(last), None) => now - last,
            (Some(last), Some((end, start))) => (end - last) + (now - start),
        };
        self.delta = Vec3::new(travel.x, 0.0, travel.z);
        self.last = Some(now);
        let bind = hierarchy.joints[i].bind_translation;
        pose[i].0 = Vec3::new(bind.x, now.y, bind.z);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeletal::{SkeletonHierarchy, SkeletonJoint};
    use gizmo_math::{Mat4, Quat};

    fn hierarchy() -> SkeletonHierarchy {
        let joint = |name: &str, parent: Option<usize>| SkeletonJoint {
            name: name.into(),
            node_index: 0,
            inverse_bind_matrix: Mat4::IDENTITY,
            parent_index: parent,
            local_bind_transform: Mat4::IDENTITY,
            bind_translation: Vec3::new(0.0, 1.0, 0.0),
            bind_rotation: Quat::IDENTITY,
            bind_scale: Vec3::ONE,
        };
        SkeletonHierarchy { joints: vec![joint("root", None), joint("hips", Some(0))], root_transform: Mat4::IDENTITY }
    }

    /// The joint's travel across the ground is the delta and the joint stays over its bind position,
    /// its height its own; through a wrap the travel runs on past the end and from the start.
    #[test]
    fn the_root_travels_and_the_pose_stays_put() {
        let h = hierarchy();
        let at = |z: f32, y: f32| vec![(Vec3::new(0.0, y, z), Quat::IDENTITY, Vec3::ONE); 2];
        let mut rm = RootMotion::applied();
        let mut pose = at(0.2, 1.1);
        rm.take(&h, &mut pose, None);
        assert_eq!(rm.delta, Vec3::ZERO, "nothing to compare with yet");
        assert_eq!(pose[0].0, Vec3::new(0.0, 1.1, 0.0), "back over its bind position, its height kept");
        let mut pose = at(0.5, 1.0);
        rm.take(&h, &mut pose, None);
        assert!((rm.delta - Vec3::new(0.0, 0.0, 0.3)).length() < 1e-6, "{:?}", rm.delta);
        // Round the loop: from 0.5 to the end at 1.0, then from the start at 0.0 to 0.1.
        let mut pose = at(0.1, 1.0);
        rm.take(&h, &mut pose, Some((Vec3::new(0.0, 1.0, 1.0), Vec3::new(0.0, 1.0, 0.0))));
        assert!((rm.delta - Vec3::new(0.0, 0.0, 0.6)).length() < 1e-6, "{:?}", rm.delta);
        assert_eq!(RootMotion::read().from_bone("hips").joint(&h), Some(1));
    }
}
