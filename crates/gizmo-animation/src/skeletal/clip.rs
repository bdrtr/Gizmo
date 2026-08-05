//! The skeletal [`AnimationClip`] — one glTF animation stored as flat per-node
//! keyframe tracks.
//!
//! Pure data: nothing here samples, advances time or touches a skeleton. Loading
//! happens in `gizmo-renderer`'s glTF importer, evaluation in
//! [`super::sample::evaluate_clip`]. Kept deliberately separate from the
//! transform-track [`crate::clip`] module, which has a same-named type for a
//! different subsystem.

use super::keyframe::Track;
use gizmo_math::{Quat, Vec3};

/// One glTF animation, as three independent lists of per-node TRS tracks.
///
/// A clip carries no playhead — time lives in whatever drives it
/// ([`super::AnimationPlayer::current_time`] or
/// [`super::AnimationStateMachine::current_time`]) and
/// [`super::sample::evaluate_clip`] turns `(clip, time, skeleton)` into one
/// `(translation, rotation, scale)` triple per joint. Clips are produced once by the
/// glTF loader and then shared behind an `Arc<[AnimationClip]>`, so treat a loaded
/// clip as read-only: [`Self::duration`] is a stored value rather than a derived one,
/// and nothing revalidates the tracks after construction.
///
/// **Track → joint resolution** is done per evaluation, not at load, and is
/// deliberately forgiving so a clip can be retargeted onto a skeleton it was not
/// authored against. Each [`Track`] carries a glTF *global node index* plus an
/// optional node name, and `evaluate_clip` tries, in order: exact name match; a
/// loosened match that strips `mixamorig:` / `mixamorig_` / `/RootNode/` and compares
/// case-insensitively; then the node index against `SkeletonJoint::node_index`.
/// Tracks that resolve to nothing are dropped silently and only counted into a
/// `debug!` aggregate — a clip paired with the wrong skeleton animates nothing rather
/// than failing loudly.
///
/// The three lists are independent: a node may be targeted by all of them, one, or
/// none, and a joint with no track on a channel keeps that channel's bind value.
/// Order within a list is glTF channel order and means nothing, with one exception —
/// where two tracks in the same list resolve to the *same* joint, the last one that
/// actually writes wins (of the `translations`, only Hips-named tracks write at all).
/// glTF morph-target (weight) channels have no representation here; the loader
/// discards them.
#[derive(Clone, Debug)]
pub struct AnimationClip {
    /// The glTF animation's name, or `"unnamed"` when the asset supplied none.
    ///
    /// A lookup key, not an identifier:
    /// [`super::AnimationPlayer::play_animation_by_name`] takes the *first* clip whose
    /// name matches exactly, so several unnamed animations in one file (all
    /// `"unnamed"`) collapse onto the first of them and the rest become unreachable by
    /// name. Also echoed in the retarget-mismatch diagnostic from
    /// [`super::sample::evaluate_clip`], which is usually how a bad pairing is found.
    pub name: String,

    /// Playback length in seconds — the largest last-keyframe timestamp across
    /// `translations`, `rotations` and `scales` at load time.
    ///
    /// Measured from `t = 0`, not from the first keyframe, and `0.0` when every track
    /// is empty. This is the value the drivers wrap or clamp against: a looping clip
    /// does `current_time.rem_euclid(duration)`, a one-shot clamps into
    /// `[0, duration]`. A value shorter than the real last keyframe therefore truncates
    /// the clip's tail, and a longer one holds the final pose for the remainder —
    /// neither is detected. When it is `0.0` there is nothing to wrap against and the two
    /// drivers part ways: the [`super::AnimationPlayer`] one only floors the playhead at
    /// zero, so it keeps growing, while the [`super::AnimationStateMachine`] one clamps it
    /// to exactly `0.0` (and then reports the clip finished on every frame).
    ///
    /// Stored, not derived — editing the tracks afterwards does not update it. (The
    /// transform-track [`crate::clip::AnimationClip::duration`] is the opposite: a
    /// method recomputed from the tracks on every call.) A state machine whose state
    /// does not resolve to a clip substitutes `1.0` s instead of reading this field;
    /// see [`super::AnimationStateMachine::current_clip_duration`].
    pub duration: f32,

    /// Translation channels in metres, one [`Track`] per animated glTF node.
    ///
    /// Only *partially* applied. [`super::sample::evaluate_clip`] writes a sampled
    /// translation onto a joint **only when the track's node name contains `"Hips"`** —
    /// that is, root motion, where Mixamo-style clips put it. Every other translation
    /// track is sampled and thrown away, and the joint keeps its `bind_translation`;
    /// nameless tracks are always discarded too, since the test is name-based and a
    /// name-less track cannot pass it. The consequence worth remembering: bone offsets
    /// (and hence limb lengths) always come from the bind pose, never from the clip.
    ///
    /// Historically the root was detected by a hard-coded node index (`66`), which
    /// silently dropped root motion on any skeleton whose Hips sat elsewhere; the
    /// name test replaced it.
    pub translations: Vec<Track<Vec3>>,

    /// Rotation channels, one [`Track`] per animated glTF node — the channel that does
    /// nearly all the posing.
    ///
    /// Sampled with slerp, or true cubic-Hermite when the track is `CubicSpline` and
    /// its keyframes kept their glTF tangents, then **renormalized** before it reaches
    /// the pose so interpolation or export drift off the unit sphere cannot leak a
    /// scale factor into the skinning matrices. Untargeted joints keep
    /// `bind_rotation`.
    pub rotations: Vec<Track<Quat>>,

    /// Scale channels, one [`Track`] per animated glTF node: unitless per-axis scale
    /// factors in the joint's local space.
    ///
    /// Unlike `translations` these apply to *every* joint they resolve to — squash and
    /// stretch, breathing and grow animations all live here. A sampled value **replaces**
    /// the joint's `bind_scale` rather than multiplying onto it. They were once parsed and
    /// then discarded outright, so scale animation never reached the skeleton at all;
    /// `scale_track_is_applied_to_joint` in `sample.rs` is the regression test guarding
    /// that. Untargeted joints keep `bind_scale`.
    pub scales: Vec<Track<Vec3>>,
}
