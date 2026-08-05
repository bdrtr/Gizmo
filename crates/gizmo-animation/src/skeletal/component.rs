use super::clip::AnimationClip;
use super::state_machine::AnimationStateMachine;
use std::sync::Arc;

/// ECS component that plays skeletal (GPU-skinning) clips on one skinned entity,
/// with a single-slot cross-fade.
///
/// Unrelated to the transform-track [`crate::player::AnimationPlayer`] of the same name —
/// see the [`crate::skeletal`] module docs for why the two are kept apart. This type is
/// pure playback *state*: it neither samples nor advances anything by itself.
/// `gizmo-renderer`'s `animation_update_system` is what steps [`current_time`], evaluates
/// the clip against the same entity's `Skeleton` hierarchy and uploads the skin matrices,
/// so without the renderer the component is inert data.
///
/// Cross-fading is deliberately one deep: [`prev_animation`] holds a single outgoing clip,
/// so switching again mid-fade abandons the clip that was already fading out instead of
/// stacking blends.
///
/// [`Default`] is an empty clip table, looping, 1× speed and no fade in flight — the shape
/// the glTF spawner fills in with `..Default::default()`. A player whose `animations` is
/// empty is skipped entirely by the renderer.
///
/// [`current_time`]: Self::current_time
/// [`prev_animation`]: Self::prev_animation
#[derive(Clone)]
pub struct AnimationPlayer {
    /// Playhead into the active clip, in **seconds**.
    ///
    /// The renderer normalizes it *after* advancing: a looping clip wraps with `rem_euclid`
    /// into `[0, clip.duration)` (which is what lets `speed < 0` run off zero and reappear at
    /// the clip's end), a non-looping one clamps into `[0, clip.duration]` — the end of the
    /// range included — and then sits on the final frame. A clip whose `duration` is `0.0` is
    /// only floored at zero, so there the playhead keeps growing without bound. Writing it
    /// directly is legitimate — the editor's timeline slider scrubs exactly this field — but
    /// note a clip switch through
    /// [`play_animation_by_name`](Self::play_animation_by_name) resets it to `0.0`.
    pub current_time: f32,

    /// Index into [`animations`](Self::animations) of the clip being sampled right now.
    ///
    /// A public field with no validation on assignment, so out-of-range is a supported
    /// state rather than a panic: it makes
    /// [`current_clip`](Self::current_clip) return `None`, and the renderer logs a warning
    /// and skips the entity for that frame — the skeleton holds its last uploaded pose
    /// instead of snapping back to bind.
    pub active_animation: usize,

    /// Whether the active clip wraps at its end (`true`) or holds its last frame (`false`).
    ///
    /// Overwritten by every [`play_animation_by_name`](Self::play_animation_by_name) call that
    /// actually changes clip — a call naming the already-active clip returns `true` and leaves
    /// this flag alone. There is only one such flag for the whole player, so during a
    /// cross-fade the *outgoing* clip is normalized with the incoming clip's looping choice —
    /// a non-looping clip fading into a looping one will wrap for the length of the fade.
    pub loop_anim: bool,

    /// Playback rate as a unitless multiplier on the frame delta
    /// (`current_time += dt * speed`), so `1.0` is authored speed.
    ///
    /// `0.0` freezes the pose without stopping the system — that is how the editor's
    /// pause button works, rather than by removing the component. Negative values play
    /// backwards and are explicitly supported by the wrap-around normalization. The
    /// outgoing clip of a cross-fade is advanced at this same rate so the two cannot
    /// desync. Untouched by clip switches.
    pub speed: f32,

    /// Every clip this player can select, normally all animations of the source glTF file.
    ///
    /// `Arc<[_]>` because [`Component`](gizmo_core::component::Component) requires `Clone`
    /// and the player is cloned every frame by the update system — the `Arc` makes that a
    /// refcount bump instead of a deep copy of the keyframe data.
    ///
    /// It does NOT deduplicate across model instances: the glTF spawner builds a fresh `Arc`
    /// per spawn from a cloned `Vec<AnimationClip>`, so a hundred instances of one model hold
    /// a hundred independent copies. Sharing them is a caller-side decision — clone an
    /// existing player's `Arc` rather than spawning each instance from the asset.
    ///
    /// Treated as immutable — swap the whole `Arc` to change the set. Empty by default,
    /// which makes the renderer skip the entity outright.
    pub animations: Arc<[AnimationClip]>,
    // Blending support
    /// Seconds elapsed into the in-flight cross-fade; the blend weight is
    /// `blend_time / blend_duration` clamped to `[0, 1]`, where 0 is fully the outgoing clip.
    ///
    /// Advanced by **raw `dt`, not `dt * speed`** — a fade is wall-clock, so a slowed-down or
    /// paused player still completes it in [`blend_duration`](Self::blend_duration) seconds.
    /// Meaningless while [`prev_animation`](Self::prev_animation) is `None`.
    pub blend_time: f32,

    /// Length of the cross-fade armed by the last clip switch, in **seconds**.
    ///
    /// `0.0` is an instant cut and not a division hazard: `blend_time < blend_duration` is
    /// already false on the first frame, so the renderer drops
    /// [`prev_animation`](Self::prev_animation) before any weight is computed. A stale value
    /// is harmless — it is only read while a fade is in flight.
    pub blend_duration: f32,

    /// The outgoing clip of an in-flight cross-fade, or `None` when nothing is fading.
    ///
    /// Armed by [`play_animation_by_name`](Self::play_animation_by_name) (and by the editor's
    /// clip dropdown, which sets the fade fields by hand with a 0.25 s blend); cleared by the
    /// renderer on the first frame where `blend_time >= blend_duration`. While it is `Some`,
    /// two clips are evaluated and blended every frame, so this doubles the entity's sampling
    /// cost. An index that has fallen out of range is treated as "no fade": the incoming pose
    /// is used unblended.
    pub prev_animation: Option<usize>,

    /// Playhead into the outgoing clip, in **seconds**, seeded from `current_time` at the
    /// instant of the switch so the fade starts from the pose that was actually on screen.
    ///
    /// Keeps advancing by `dt * speed` for the duration of the fade. Unlike `current_time` the
    /// field itself is never normalized: it accumulates raw and may leave the outgoing clip's
    /// range (it goes negative under reverse playback). Only the *sample* time derived from it
    /// each frame is wrapped or clamped, the way `current_time` is, and that value is not
    /// written back. It used to advance by plain `dt`, which played the outgoing clip
    /// at 1× regardless of `speed` and let the two clips visibly desync mid-fade. Ignored
    /// while [`prev_animation`](Self::prev_animation) is `None`.
    pub prev_time: f32,
}

impl Default for AnimationPlayer {
    fn default() -> Self {
        Self {
            current_time: 0.0,
            active_animation: 0,
            loop_anim: true,
            speed: 1.0,
            animations: Arc::new([]),
            blend_time: 0.0,
            blend_duration: 0.0,
            prev_animation: None,
            prev_time: 0.0,
        }
    }
}

impl AnimationPlayer {
    /// The clip [`active_animation`](Self::active_animation) points at, or `None` when that
    /// index is out of range.
    ///
    /// Bounds-guarded on purpose: `active_animation` is a public `usize` that is never
    /// validated on assignment, and indexing would panic instead.
    /// During a cross-fade this reports the *incoming* clip — the outgoing one is reached
    /// through [`prev_animation`](Self::prev_animation).
    pub fn current_clip(&self) -> Option<&AnimationClip> {
        self.animations.get(self.active_animation)
    }

    /// Switches to the clip called `name`, arming a `blend`-second cross-fade from wherever
    /// the current clip's playhead is.
    ///
    /// Matching is exact and case-sensitive against `AnimationClip::name`, and the first
    /// match wins — a glTF file with duplicate clip names can only ever reach the first one.
    ///
    /// Returns `false` and changes nothing when no clip carries that name. That is almost
    /// always a typo, and callers routinely discard the result (the Lua `PlayAnimation`
    /// command does), so the failure is logged at `warn!` rather than vanishing into an
    /// ignored return value and a character that simply never animates.
    ///
    /// Re-selecting the clip that is already active returns `true` but is a **no-op**: the
    /// playhead is deliberately not rewound and no fade is armed, so gameplay code may call
    /// this every frame ("keep running") without pinning the animation to frame 0. Restart a
    /// clip explicitly by assigning `current_time = 0.0`.
    ///
    /// `blend` is in seconds and `0.0` cuts instantly; on a real switch `loop_anim` overwrites
    /// [`Self::loop_anim`] for the whole player. [`Self::speed`] is left alone.
    pub fn play_animation_by_name(&mut self, name: &str, blend: f32, loop_anim: bool) -> bool {
        if let Some(idx) = self.animations.iter().position(|a| a.name == name) {
            if self.active_animation != idx {
                // Gameplay-driven clip switch (arms a cross-fade). Frequent enough
                // in normal play that it belongs at debug!, not info!.
                tracing::debug!(
                    from = self.active_animation,
                    to = idx,
                    name = %name,
                    blend,
                    loop_anim,
                    "[Animation] skeletal clip switch (cross-fade armed)"
                );
                self.prev_animation = Some(self.active_animation);
                self.prev_time = self.current_time;
                self.active_animation = idx;
                self.current_time = 0.0;
                self.blend_duration = blend;
                self.blend_time = 0.0;
                self.loop_anim = loop_anim;
            } else {
                tracing::trace!(name = %name, "[Animation] skeletal clip already active; play is a no-op");
            }
            true
        } else {
            // Previously a silent `false`: a caller that ignores the return value
            // (a typo'd clip name) would just see nothing animate. Surface it.
            tracing::warn!(
                requested = %name,
                available = self.animations.len(),
                "[Animation] play_animation_by_name: no clip with that name; request ignored"
            );
            false
        }
    }
}

/// Pins this entity to a single bone of *another* entity's skeleton — sword in hand,
/// hat on head.
///
/// Driven by `gizmo`'s `BoneAttachmentSystem` (behind the facade's `render` feature, since
/// the `Skeleton` it reads is a renderer component). Every frame that system **overwrites**
/// this entity's `Transform` — position, rotation *and* scale — with
/// `skeleton.global_poses[bone_index] * offset`, decomposed back into TRS.
///
/// Ordering matters: the studio's render pipeline runs the attachment system immediately
/// after the skeletal animation update, so it consumes poses computed this frame. Scheduled
/// before it, attachments would trail the skeleton by one frame.
///
/// The poses it samples are in the **skinned model's own space** (they include the glTF
/// armature root transform but not the skinned entity's world matrix), so the attachment
/// entity must be parented under the skeleton owner for hierarchy propagation to place it
/// correctly in the world.
///
/// This is the one animation component that is serializable, hence the `serde` derives; the
/// studio registers it under the name `"BoneAttachment"`, which is what puts it in the
/// inspector's add/remove menu and the reason it needs a [`Default`]. (The editor crate only
/// reads that registry to build the menu — it does not register anything itself.)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BoneAttachment {
    /// The entity carrying the `Skeleton` component to follow — the skinned model, *not*
    /// this entity.
    ///
    /// Only the id half is used: the driving system looks the skeleton up by
    /// `target_entity.id()` and never checks the generation, so a handle left over from a
    /// despawned model silently follows whatever now occupies that slot. Defaults to
    /// `Entity::new(0, 0)`, a placeholder for the editor's "add component" path rather than a
    /// meaningful target — every real attachment must overwrite it, ideally with a handle
    /// from `World::entity(id)` so the generation is current.
    pub target_entity: gizmo_core::entity::Entity,

    /// Which bone to follow, as an index into `SkeletonHierarchy::joints` (equivalently into
    /// the renderer's `Skeleton::global_poses`, which is parallel to it).
    ///
    /// A **bone** index, not the glTF node index that animation channels target — those are
    /// recorded separately in
    /// [`SkeletonJoint::node_index`](super::skeleton::SkeletonJoint::node_index) and the two
    /// routinely differ; confusing them is a documented past bug in the clip evaluator.
    /// Out of range is not an error: the attachment is skipped for that frame and the entity
    /// keeps its previous transform.
    pub bone_index: usize,

    /// Extra transform applied in **bone space**, right-multiplied onto the bone's global
    /// matrix (`global_pose * offset`).
    ///
    /// This is where a weapon's grip correction lives: translation in metres, plus any
    /// rotation and scale the matrix carries — all three survive, because the product is
    /// decomposed into TRS and written to the entity's `Transform`. `Mat4::IDENTITY` (the
    /// default) seats the entity exactly at the bone's origin and orientation.
    pub offset: gizmo_math::Mat4,
}

impl Default for BoneAttachment {
    fn default() -> Self {
        Self {
            target_entity: gizmo_core::entity::Entity::new(0, 0),
            bone_index: 0,
            offset: gizmo_math::Mat4::IDENTITY,
        }
    }
}

gizmo_core::impl_component!(AnimationPlayer, AnimationStateMachine, BoneAttachment);

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(name: &str) -> AnimationClip {
        AnimationClip {
            name: name.into(),
            duration: 1.0,
            translations: vec![],
            rotations: vec![],
            scales: vec![],
        }
    }

    fn player_with(names: &[&str]) -> AnimationPlayer {
        let anims: Vec<AnimationClip> = names.iter().map(|n| clip(n)).collect();
        AnimationPlayer {
            animations: Arc::from(anims),
            ..Default::default()
        }
    }

    #[test]
    fn play_by_name_switches_and_arms_the_crossfade() {
        let mut p = player_with(&["idle", "run"]);
        p.current_time = 5.0; // pretend we were mid-idle
        let ok = p.play_animation_by_name("run", 0.3, false);
        assert!(ok, "known animation must report success");
        assert_eq!(p.active_animation, 1, "active switches to run");
        assert_eq!(p.prev_animation, Some(0), "previous animation captured for cross-fade");
        assert_eq!(p.prev_time, 5.0, "previous playhead captured");
        assert_eq!(p.current_time, 0.0, "new animation restarts from 0");
        assert_eq!(p.blend_duration, 0.3);
        assert_eq!(p.blend_time, 0.0);
        assert!(!p.loop_anim, "loop flag taken from the call");
    }

    #[test]
    fn play_by_name_same_animation_is_noop_but_succeeds() {
        let mut p = player_with(&["idle", "run"]);
        p.current_time = 5.0;
        let ok = p.play_animation_by_name("idle", 0.5, true);
        assert!(ok, "re-selecting the active clip still reports success");
        assert_eq!(p.active_animation, 0, "no switch");
        assert_eq!(p.prev_animation, None, "no cross-fade armed");
        assert_eq!(p.current_time, 5.0, "playhead must NOT be reset when already playing it");
        assert_eq!(p.blend_duration, 0.0, "no blend armed for a no-op");
    }

    #[test]
    fn play_by_name_unknown_returns_false_and_changes_nothing() {
        let mut p = player_with(&["idle", "run"]);
        p.current_time = 2.0;
        let ok = p.play_animation_by_name("fly", 0.1, false);
        assert!(!ok, "unknown animation must fail");
        assert_eq!(p.active_animation, 0, "state untouched on failure");
        assert_eq!(p.prev_animation, None);
        assert_eq!(p.current_time, 2.0);
    }

    #[test]
    fn current_clip_indexes_active_and_guards_bounds() {
        let mut p = player_with(&["idle", "run"]);
        p.active_animation = 1;
        assert_eq!(p.current_clip().map(|c| c.name.as_str()), Some("run"));
        // An out-of-range active index must yield None, not panic.
        p.active_animation = 99;
        assert!(p.current_clip().is_none());
    }
}
