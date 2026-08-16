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

impl AnimationClip {
    /// Bring [`duration`](Self::duration) back in step with the tracks after an edit.
    ///
    /// **Every editing entry point must end here.** `duration` is stored, not derived, and the
    /// field's own note spells out what a stale one does: shorter than the real last keyframe
    /// truncates the clip's tail, longer holds the final pose — *and neither is detected*. A
    /// timeline that retimes a key without this leaves a clip that plays wrong and looks fine.
    ///
    /// It only ever **grows**. Shrinking would be the more obvious rule and it is the wrong one:
    /// a glTF clip may declare a duration past its last keyframe on purpose, to hold the final
    /// pose, and recomputing exactly would silently throw that padding away the first time
    /// anybody nudged a key. Growing prevents the harmful case; the benign one is the author's
    /// business. To drop the padding deliberately, assign `duration` — that is a decision, not a
    /// side effect of an edit.
    pub fn grow_duration_to_fit(&mut self) {
        let last = self.last_keyframe_time();
        if last > self.duration {
            self.duration = last;
        }
    }

    /// The largest last-keyframe timestamp across all three channel lists, or `0.0` when every
    /// track is empty — the number [`duration`](Self::duration) was built from at load time.
    pub fn last_keyframe_time(&self) -> f32 {
        let t = self.translations.iter().filter_map(|t| t.last_time());
        let r = self.rotations.iter().filter_map(|t| t.last_time());
        let s = self.scales.iter().filter_map(|t| t.last_time());
        t.chain(r).chain(s).fold(0.0_f32, f32::max)
    }

    /// Is every track still sorted by time? The sampling path assumes it and cannot check it.
    pub fn tracks_are_sorted(&self) -> bool {
        self.translations.iter().all(|t| t.is_sorted_by_time())
            && self.rotations.iter().all(|t| t.is_sorted_by_time())
            && self.scales.iter().all(|t| t.is_sorted_by_time())
    }
}


#[cfg(test)]
mod editing_tests {
    use super::*;
    use crate::skeletal::keyframe::{InterpolationMode, Keyframe};

    fn rot_track(times: &[f32]) -> Track<Quat> {
        Track {
            target_node: 0,
            target_node_name: Some("Hips".to_string()),
            interpolation: InterpolationMode::Linear,
            keyframes: times
                .iter()
                .map(|&t| Keyframe {
                    time: t,
                    value: Quat::IDENTITY,
                    in_tangent: None,
                    out_tangent: None,
                })
                .collect(),
        }
    }

    fn clip(duration: f32, times: &[f32]) -> AnimationClip {
        AnimationClip {
            name: "test".to_string(),
            duration,
            translations: Vec::new(),
            rotations: vec![rot_track(times)],
            scales: Vec::new(),
        }
    }

    /// The invariant the whole thing turns on: a stored `duration` shorter than the real last
    /// keyframe truncates the clip's tail, and the field's own note says that is not detected
    /// anywhere. Dragging a key past the end is the ordinary way to produce it.
    #[test]
    fn dragging_a_key_past_the_end_grows_the_duration() {
        let mut c = clip(2.0, &[0.0, 1.0, 2.0]);
        c.rotations[0].retime_keyframe(2, 5.0).unwrap();
        assert_eq!(
            c.duration, 2.0,
            "the retime alone must not touch duration — that is why the caller has to grow it"
        );
        c.grow_duration_to_fit();
        assert_eq!(c.duration, 5.0);
        assert!(
            c.duration >= c.last_keyframe_time(),
            "a clip must never claim to be shorter than its own keyframes"
        );
    }

    /// It grows and never shrinks. A glTF clip is allowed to declare a duration past its last
    /// keyframe to hold the final pose; recomputing exactly would throw that away the first time
    /// anyone nudged a key, silently changing how long the animation runs.
    #[test]
    fn deliberate_padding_survives_an_edit() {
        let mut c = clip(10.0, &[0.0, 1.0, 2.0]);
        c.rotations[0].retime_keyframe(2, 1.5).unwrap();
        c.grow_duration_to_fit();
        assert_eq!(c.duration, 10.0, "padding past the last key is the author's, not ours to drop");
    }

    #[test]
    fn an_emptied_clip_reports_zero_and_keeps_its_duration() {
        let mut c = clip(3.0, &[0.0, 1.0]);
        assert!(c.rotations[0].remove_keyframe(1));
        assert!(c.rotations[0].remove_keyframe(0));
        assert_eq!(c.last_keyframe_time(), 0.0, "no keyframes anywhere");
        c.grow_duration_to_fit();
        assert_eq!(c.duration, 3.0);
    }

    /// `last_keyframe_time` has to look at all three lists, not just the one the test author
    /// happened to fill in — a translation track running past the rotations is the ordinary case
    /// for root motion.
    #[test]
    fn the_last_keyframe_is_the_latest_across_all_three_channels() {
        let mut c = clip(0.0, &[0.0, 1.0]);
        c.scales.push(Track {
            target_node: 1,
            target_node_name: None,
            interpolation: InterpolationMode::Linear,
            keyframes: vec![Keyframe {
                time: 7.0,
                value: Vec3::ONE,
                in_tangent: None,
                out_tangent: None,
            }],
        });
        assert_eq!(c.last_keyframe_time(), 7.0);
        c.grow_duration_to_fit();
        assert_eq!(c.duration, 7.0);
    }

    /// Sortedness is a whole-clip property because sampling is per track: one track left out of
    /// order is one joint animating wrongly while everything else looks right.
    #[test]
    fn a_retime_keeps_every_track_sorted() {
        let mut c = clip(2.0, &[0.0, 1.0, 2.0]);
        assert!(c.tracks_are_sorted());
        c.rotations[0].retime_keyframe(0, 1.5).unwrap();
        assert!(c.tracks_are_sorted(), "the retime must restore the order it broke");
    }
}
