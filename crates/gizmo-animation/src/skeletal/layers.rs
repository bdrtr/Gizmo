//! **Animation layers:** clips played over the pose an entity's player or state machine makes, on a
//! part of the skeleton — Unity's Animator layers, the filtered `Blend2`/`Add2` nodes of Godot's
//! `AnimationTree`.
//!
//! The base pose comes from the entity's [`AnimationPlayer`](super::AnimationPlayer) or
//! [`AnimationStateMachine`](super::AnimationStateMachine); an [`AnimationLayers`] on the same
//! entity then lays each of its layers over it, in order, before the skin is uploaded. A layer plays
//! one clip of the same clip table, on the joints its [`BoneMask`] names and everything below them,
//! at a weight that fades in and out:
//!
//! - [`LayerBlend::Override`] replaces the masked joints' pose with the layer's, by the weight: a
//!   character that waves while it walks, its arm and shoulder from the wave, its legs from the walk.
//! - [`LayerBlend::Additive`] adds how far the layer's clip has moved from its own first frame: a
//!   breath, a flinch, a lean over whatever the body is doing.
//!
//! The playhead, the clip and the fade are the layer's; the driver (`gizmo_renderer`'s animation
//! systems) advances them every frame, posed or not.

use super::clip::AnimationClip;
use super::skeleton::SkeletonHierarchy;
use gizmo_math::{Quat, Vec3};

/// A joint's pose: translation, rotation, scale — what the samplers return per joint.
pub type JointPose = (Vec3, Quat, Vec3);

/// How a layer is laid over the pose beneath it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum LayerBlend {
    /// The masked joints take the layer's pose, by its weight.
    #[default]
    Override,
    /// The layer's clip's motion away from its own first frame is added to the pose beneath, by
    /// its weight.
    Additive,
}

/// The joints a layer acts on: each named joint and every joint below it. Empty: the whole skeleton.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BoneMask {
    /// Joint names, matched exactly; each brings the joints below it.
    pub roots: Vec<String>,
}

impl BoneMask {
    /// The joints named and everything below them.
    #[must_use]
    pub fn below(roots: &[&str]) -> Self {
        Self { roots: roots.iter().map(|r| (*r).to_string()).collect() }
    }

    /// Each joint's share of the layer, 1 or 0, in `hierarchy`'s order. A name no joint has adds
    /// nothing; an empty mask is every joint.
    #[must_use]
    pub fn weights(&self, hierarchy: &SkeletonHierarchy) -> Vec<f32> {
        let joints = &hierarchy.joints;
        if self.roots.is_empty() {
            return vec![1.0; joints.len()];
        }
        // Parents come before their children in a glTF skin's joint list as the loader builds it,
        // but nothing promises it: walk up from each joint instead.
        (0..joints.len())
            .map(|i| {
                let mut at = Some(i);
                let mut steps = 0;
                while let Some(j) = at {
                    if self.roots.iter().any(|r| *r == joints[j].name) {
                        return 1.0;
                    }
                    at = joints[j].parent_index;
                    steps += 1;
                    if steps > joints.len() {
                        break;
                    }
                }
                0.0
            })
            .collect()
    }
}

/// One layer — see the [module](self).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnimationLayer {
    /// What [`AnimationLayers::play`] and [`stop`](AnimationLayers::stop) call it.
    pub name: String,
    /// The joints it acts on.
    pub mask: BoneMask,
    /// How it is laid over the pose beneath.
    pub blend: LayerBlend,
    /// The clip it plays, by name in the entity's clip table; `None`: nothing (its weight then
    /// counts for nothing).
    pub clip: Option<String>,
    /// Seconds into the clip.
    pub time: f32,
    /// Playback rate, as a player's.
    pub speed: f32,
    /// Whether the clip wraps at its end or holds its last frame.
    pub looped: bool,
    /// Its weight now, 0 to 1.
    pub weight: f32,
    /// The weight it fades to.
    pub target_weight: f32,
    /// How fast it fades, weight per second; 0 or less: at once.
    pub fade_rate: f32,
}

impl AnimationLayer {
    /// A layer over the joints of `mask`, overriding, playing nothing yet.
    #[must_use]
    pub fn new(name: &str, mask: BoneMask) -> Self {
        Self {
            name: name.into(),
            mask,
            blend: LayerBlend::Override,
            clip: None,
            time: 0.0,
            speed: 1.0,
            looped: true,
            weight: 0.0,
            target_weight: 0.0,
            fade_rate: 0.0,
        }
    }

    /// The same layer, adding its clip's motion instead of overriding.
    #[must_use]
    pub fn additive(mut self) -> Self {
        self.blend = LayerBlend::Additive;
        self
    }

    /// Moves the playhead and the fade by `dt`: the playhead wraps or holds at `duration`, the
    /// weight steps toward its target.
    pub fn advance(&mut self, dt: f32, duration: f32) {
        self.time += dt * self.speed;
        if duration > 0.0 {
            self.time = if self.looped { self.time.rem_euclid(duration) } else { self.time.clamp(0.0, duration) };
        }
        let step = if self.fade_rate > 0.0 { self.fade_rate * dt } else { f32::INFINITY };
        let gap = self.target_weight - self.weight;
        self.weight = if gap.abs() <= step { self.target_weight } else { self.weight + step.copysign(gap) };
    }
}

/// ECS component: the layers laid over an entity's animated pose, in order — see the [module](self).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnimationLayers {
    /// Bottom first: each is laid over the result of those before it.
    pub layers: Vec<AnimationLayer>,
}

impl AnimationLayers {
    /// The layers given, in order.
    #[must_use]
    pub fn new(layers: Vec<AnimationLayer>) -> Self {
        Self { layers }
    }

    /// The layer called `name`.
    pub fn layer_mut(&mut self, name: &str) -> Option<&mut AnimationLayer> {
        self.layers.iter_mut().find(|l| l.name == name)
    }

    /// Plays `clip` on layer `name` from its start, fading in over `fade` seconds; `looped` whether
    /// it wraps. Whether the layer exists.
    pub fn play(&mut self, name: &str, clip: &str, fade: f32, looped: bool) -> bool {
        let Some(layer) = self.layer_mut(name) else { return false };
        if layer.clip.as_deref() != Some(clip) || layer.weight <= 0.0 {
            layer.time = 0.0;
        }
        layer.clip = Some(clip.to_string());
        layer.looped = looped;
        layer.target_weight = 1.0;
        layer.fade_rate = if fade > 0.0 { 1.0 / fade } else { 0.0 };
        true
    }

    /// Fades layer `name` out over `fade` seconds. Whether the layer exists.
    pub fn stop(&mut self, name: &str, fade: f32) -> bool {
        let Some(layer) = self.layer_mut(name) else { return false };
        layer.target_weight = 0.0;
        layer.fade_rate = if fade > 0.0 { 1.0 / fade } else { 0.0 };
        true
    }

    /// Advances every layer's playhead and fade by `dt`, each clip's length found in `clips`.
    pub fn advance(&mut self, dt: f32, clips: &[AnimationClip]) {
        for layer in &mut self.layers {
            let duration = layer.clip.as_deref().and_then(|c| clips.iter().find(|a| a.name == c)).map_or(0.0, |c| c.duration);
            layer.advance(dt, duration);
        }
    }

    /// What the layers would draw differently from one frame to the next other than by their own
    /// clocks: each layer's clip and the weight it is heading to — for a driver telling a change made
    /// by the game from its own advance.
    #[must_use]
    pub fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for layer in &self.layers {
            layer.clip.hash(&mut h);
            layer.target_weight.to_bits().hash(&mut h);
            (layer.weight > 0.0).hash(&mut h);
        }
        h.finish()
    }
}

/// Lays `layer` over `pose`: `layer_pose` is its clip sampled now, `reference` its clip at its first
/// frame (read only by an additive layer), `mask` each joint's share, `weight` the layer's.
pub fn lay_over(
    pose: &mut [JointPose],
    layer_pose: &[JointPose],
    reference: &[JointPose],
    mask: &[f32],
    blend: LayerBlend,
    weight: f32,
) {
    for (i, joint) in pose.iter_mut().enumerate() {
        let w = weight * mask.get(i).copied().unwrap_or(0.0);
        if w <= 0.0 {
            continue;
        }
        let Some(&(lt, lr, ls)) = layer_pose.get(i) else { continue };
        let (t, r, s) = *joint;
        *joint = match blend {
            LayerBlend::Override => (t.lerp(lt, w), r.slerp(lr, w).normalize(), s.lerp(ls, w)),
            LayerBlend::Additive => {
                let Some(&(rt, rr, rs)) = reference.get(i) else { continue };
                let delta_r = (rr.inverse() * lr).normalize();
                let delta_s = ls / rs.max(Vec3::splat(1.0e-6));
                (t + (lt - rt) * w, (r * Quat::IDENTITY.slerp(delta_r, w)).normalize(), s * Vec3::ONE.lerp(delta_s, w))
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeletal::skeleton::SkeletonJoint;
    use gizmo_math::Mat4;

    fn hierarchy(joints: &[(&str, Option<usize>)]) -> SkeletonHierarchy {
        SkeletonHierarchy {
            joints: joints
                .iter()
                .enumerate()
                .map(|(i, (name, parent))| SkeletonJoint {
                    name: (*name).into(),
                    node_index: i,
                    inverse_bind_matrix: Mat4::IDENTITY,
                    parent_index: *parent,
                    local_bind_transform: Mat4::IDENTITY,
                    bind_translation: Vec3::ZERO,
                    bind_rotation: Quat::IDENTITY,
                    bind_scale: Vec3::ONE,
                })
                .collect(),
            root_transform: Mat4::IDENTITY,
        }
    }

    /// A mask is its joints and everything below them: the spine brings the arms, not the legs.
    #[test]
    fn a_mask_brings_the_joints_below_its_roots() {
        let h = hierarchy(&[("hips", None), ("spine", Some(0)), ("arm", Some(1)), ("hand", Some(2)), ("leg", Some(0))]);
        assert_eq!(BoneMask::below(&["spine"]).weights(&h), vec![0.0, 1.0, 1.0, 1.0, 0.0]);
        assert_eq!(BoneMask::default().weights(&h), vec![1.0; 5], "empty: every joint");
        assert_eq!(BoneMask::below(&["tail"]).weights(&h), vec![0.0; 5], "a name no joint has");
    }

    /// Override moves the masked joints to the layer's pose by the weight and leaves the rest;
    /// additive adds the clip's motion away from its first frame.
    #[test]
    fn a_layer_overrides_or_adds_on_its_joints_alone() {
        let base = vec![(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE); 2];
        let layer = vec![(Vec3::new(2.0, 0.0, 0.0), Quat::from_rotation_y(1.0), Vec3::splat(3.0)); 2];
        let mut over = base.clone();
        lay_over(&mut over, &layer, &[], &[0.0, 1.0], LayerBlend::Override, 0.5);
        assert_eq!(over[0], base[0], "an unmasked joint is left alone");
        assert!((over[1].0.x - 1.0).abs() < 1e-5 && (over[1].2.x - 2.0).abs() < 1e-5, "{:?}", over[1]);
        assert!((over[1].1.angle_between(Quat::from_rotation_y(0.5))) < 1e-4);

        let start = vec![(Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE); 2];
        let mut added = vec![(Vec3::new(0.0, 5.0, 0.0), Quat::from_rotation_x(0.3), Vec3::splat(2.0)); 2];
        lay_over(&mut added, &layer, &start, &[1.0, 1.0], LayerBlend::Additive, 1.0);
        // +1 in x (2 − 1), the y rotation on top of the x one, the scale tripled.
        assert!((added[0].0 - Vec3::new(1.0, 5.0, 0.0)).length() < 1e-5, "{:?}", added[0]);
        assert!(added[0].1.angle_between(Quat::from_rotation_x(0.3) * Quat::from_rotation_y(1.0)) < 1e-4);
        assert!((added[0].2 - Vec3::splat(6.0)).length() < 1e-5);
    }

    /// The fade steps toward its target at its rate, the playhead wraps, and `play` / `stop` set
    /// them; a layer playing nothing has nothing to say.
    #[test]
    fn a_layer_fades_in_plays_and_fades_out() {
        let clips = vec![AnimationClip { name: "wave".into(), duration: 1.0, translations: vec![], rotations: vec![], scales: vec![] }];
        let mut layers = AnimationLayers::new(vec![AnimationLayer::new("arms", BoneMask::below(&["spine"]))]);
        assert!(layers.play("arms", "wave", 0.5, true));
        assert!(!layers.play("legs", "wave", 0.5, true), "no such layer");
        layers.advance(0.25, &clips);
        assert!((layers.layers[0].weight - 0.5).abs() < 1e-5, "half faded in after half the fade");
        layers.advance(1.0, &clips);
        assert_eq!(layers.layers[0].weight, 1.0);
        assert!((layers.layers[0].time - 0.25).abs() < 1e-5, "wrapped: {}", layers.layers[0].time);
        let before = layers.key();
        layers.stop("arms", 0.0);
        assert_ne!(layers.key(), before, "a stop is a change the driver sees");
        layers.advance(0.01, &clips);
        assert_eq!(layers.layers[0].weight, 0.0, "a stop without a fade is at once");
    }
}
