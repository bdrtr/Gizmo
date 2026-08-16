/// One sample of an animated channel: a value pinned to a point in time.
///
/// `T` is the channel's value type — `Vec3` for translation/scale, `Quat` for rotation.
/// This type deliberately carries no interpolation maths of its own; blending is supplied
/// by the closure the caller hands to [`Track::get_interpolated`] / [`Track::sample_cubic`],
/// which is what lets one keyframe list serve both vectors and quaternions.
///
/// Produced by the glTF loader (`gizmo-renderer`'s `asset::loaders::animation`):
/// [`Keyframe::new`] for `LINEAR`/`STEP` samplers, [`Keyframe::with_tangents`] for
/// `CUBICSPLINE`.
#[derive(Clone, Copy, Debug)]
pub struct Keyframe<T> {
    /// Timestamp in **seconds**, measured from the start of the owning clip (the glTF
    /// sampler's input accessor value, copied verbatim — no rebasing to zero).
    ///
    /// [`Track::keyframes`] must be sorted ascending on this field: sampling binary-searches
    /// it, so an out-of-order entry does not error, it silently returns the wrong segment.
    /// Two keyframes may share a timestamp: in a sorted list the zero-length segment between
    /// them is never the one sampled, so nothing divides by zero.
    pub time: f32,

    /// The channel value at [`time`](Keyframe::time) — a pose translation in metres, a scale
    /// factor, or a rotation quaternion, depending on which of [`AnimationClip`]'s track
    /// lists this keyframe lives in.
    ///
    /// Authoritative at the sample points: at or past the first/last keyframe it is returned
    /// verbatim (no tangent maths), and the Hermite basis reproduces it exactly at the
    /// segment ends whatever the tangents are. For `CUBICSPLINE` tracks this is the middle
    /// element of glTF's `[inTangent, value, outTangent]` triple.
    ///
    /// [`AnimationClip`]: super::clip::AnimationClip
    pub value: T,
    /// Cubic-spline in-tangent (per-second). `None` for Linear/Step keyframes.
    /// For glTF `CUBICSPLINE` this is the first of the `[inTangent, value, outTangent]`
    /// triple stored for each sample.
    pub in_tangent: Option<T>,
    /// Cubic-spline out-tangent (per-second). `None` for Linear/Step keyframes.
    pub out_tangent: Option<T>,
}

impl<T> Keyframe<T> {
    /// A Linear/Step keyframe with no cubic tangents.
    pub fn new(time: f32, value: T) -> Self {
        Keyframe {
            time,
            value,
            in_tangent: None,
            out_tangent: None,
        }
    }

    /// A cubic-spline keyframe carrying its glTF in/out tangents (per-second).
    pub fn with_tangents(time: f32, value: T, in_tangent: T, out_tangent: T) -> Self {
        Keyframe {
            time,
            value,
            in_tangent: Some(in_tangent),
            out_tangent: Some(out_tangent),
        }
    }
}

/// Where a query time falls within a track's keyframe list.
enum SegmentPos {
    /// Time is at/before the first or at/after the last keyframe — return this index verbatim.
    Clamp(usize),
    /// Time is strictly inside a segment; `t` is the normalized `[0,1)` position and `dt`
    /// is the segment duration in seconds (needed to scale cubic tangents).
    Interp { i: usize, j: usize, t: f32, dt: f32 },
}

/// How a [`Track`] blends between two neighbouring keyframes.
///
/// Mirrors the glTF animation-sampler modes, and like glTF it is a property of the whole
/// sampler — a single track cannot switch modes part-way through. The loader maps anything
/// it does not recognise to `Linear`.
///
/// This is the skeletal (GPU-skinning) counterpart of the transform-track
/// [`crate::clip::Interpolation`]; the two enums are separate only because the two
/// subsystems are (see [`crate::skeletal`]), and they must stay behaviourally identical.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InterpolationMode {
    /// Blend the two bracketing keyframes with the caller-supplied interpolator at the
    /// normalized segment position — `lerp` for translation/scale, `slerp` for rotation
    /// in [`crate::skeletal::sample`]. Also what the loader falls back to for any glTF
    /// sampler mode it does not recognise, so an exotic file animates rather than freezes.
    Linear,
    /// Hold the earlier keyframe's value for the whole segment; the value jumps at the
    /// next keyframe's timestamp. The interpolator closure is never called.
    Step,
    /// Cubic-Hermite through the per-keyframe in/out tangents (glTF Appendix C).
    ///
    /// Only [`Track::sample_cubic`] honours it; it needs *both* endpoints of the segment to
    /// carry tangents ([`Keyframe::with_tangents`]) and otherwise declines, and
    /// [`Track::get_interpolated`] treats this mode as `Linear` outright. So a
    /// `CubicSpline` track whose tangents were dropped degrades to a linear blend rather
    /// than failing — smoother curves are an upgrade, never a hard requirement.
    CubicSpline,
}

/// One animated channel — translation, rotation *or* scale — for a single glTF node.
///
/// Which channel a track drives is not stored here: it is implied by the list it sits in
/// ([`AnimationClip::translations`] / `rotations` / `scales`), which is also why `T` is
/// `Vec3` for two of them and `Quat` for the third. A node animated in all three channels
/// therefore owns three separate `Track`s, each with its own keyframe times and
/// interpolation mode, exactly as glTF stores them.
///
/// Sampling is implemented only for `T: Clone + Copy` (values are copied out of the
/// keyframe list rather than borrowed), and every sampling method takes `&self` — a track
/// is read-only data that any number of entities can play at different times at once.
///
/// [`AnimationClip::translations`]: super::clip::AnimationClip::translations
#[derive(Clone, Debug)]
pub struct Track<T> {
    /// The glTF **global node index** this channel targets
    /// (`channel.target().node().index()`) — *not* a bone/joint index into
    /// [`SkeletonHierarchy::joints`](super::skeleton::SkeletonHierarchy::joints).
    ///
    /// [`evaluate_clip`](super::sample::evaluate_clip) resolves it by finding the joint
    /// whose `node_index` equals this value; that is the fallback used when
    /// [`target_node_name`](Track::target_node_name) is absent or matches nothing.
    ///
    /// Historical bug worth knowing: this used to be indexed straight into the joint array
    /// (and only when it happened to be `< joints.len()`), which silently mis-targeted or
    /// dropped tracks on any file where the armature is not the first block of nodes in the
    /// document — i.e. most Blender exports.
    pub target_node: usize,

    /// The target glTF node's name, when the exporter wrote one. This is the *preferred*
    /// binding key, tried before [`target_node`](Track::target_node).
    ///
    /// [`evaluate_clip`](super::sample::evaluate_clip) first looks for an exact joint-name
    /// match, then a loose one that strips `/RootNode/`, `mixamorig:` and `mixamorig_` and
    /// lowercases both sides — which is what lets a Mixamo clip retarget onto a skeleton
    /// whose bones are named plainly.
    ///
    /// It also gates root motion: a translation track is applied only when this name
    /// contains `Hips`, every other translation track being discarded in favour of the bind
    /// pose. A `None` here consequently can never contribute root motion, however its node
    /// index resolves.
    pub target_node_name: Option<String>,

    /// How to blend inside a segment; a whole-track property, as in glTF (see
    /// [`InterpolationMode`]). Set once by the loader from the channel's sampler and read
    /// on every sample — nothing in the engine mutates it during playback.
    pub interpolation: InterpolationMode,

    /// The samples, which **must be sorted ascending by [`Keyframe::time`]** — sampling
    /// binary-searches this list (`partition_point`, O(log N); it was a linear scan once),
    /// so unsorted input yields wrong values rather than an error.
    ///
    /// May be empty, in which case every sampling method returns `None` and callers keep
    /// the joint's bind pose. A single keyframe is legal and makes the track a constant.
    /// Times before the first / after the last entry clamp to that entry's value, so a clip
    /// never extrapolates past its own keyframes.
    pub keyframes: Vec<Keyframe<T>>,
}

/// Editing, for a timeline that authors rather than only plays.
///
/// Separate from the sampling `impl` below and deliberately free of the `Clone + Copy` bound:
/// nothing here reads a keyframe's value, only its position in time, so a track of any payload
/// can be retimed.
///
/// # The invariant these exist to keep
///
/// [`keyframes`](Track::keyframes) **must stay sorted ascending by time** — sampling
/// binary-searches it, so an out-of-order entry does not error, it silently returns the wrong
/// segment. Mutating the list by hand from a UI is exactly how that happens: a drag past a
/// neighbour reorders two keys and nothing complains until the pose is wrong. These methods
/// restore the order themselves, which is why editing goes through them rather than through
/// `keyframes` directly.
impl<T> Track<T> {
    /// Move the keyframe at `index` to `new_time`, restoring the sort, and return the index it
    /// ended up at.
    ///
    /// Returns `None` — changing nothing — for an out-of-range index or a non-finite time. NaN
    /// is refused rather than clamped because every comparison against it is false: a single NaN
    /// timestamp makes the list unsortable and `partition_point` returns nonsense from then on,
    /// permanently, on a track that still looks fine.
    ///
    /// Negative times are clamped to zero. A clip's duration is measured from `t = 0`, so a
    /// keyframe before the start is not a shorter clip, it is a key that can never be the
    /// sampled segment.
    ///
    /// Equal timestamps are allowed and the moved key lands **after** its equals, which is where
    /// a drag leaves it visually. Two keys sharing a time are legal: the zero-length segment
    /// between them is never the one sampled.
    pub fn retime_keyframe(&mut self, index: usize, new_time: f32) -> Option<usize> {
        if index >= self.keyframes.len() || !new_time.is_finite() {
            return None;
        }
        let new_time = new_time.max(0.0);
        let mut moved = self.keyframes.remove(index);
        moved.time = new_time;
        let at = self.keyframes.partition_point(|k| k.time <= new_time);
        self.keyframes.insert(at, moved);
        Some(at)
    }

    /// Remove the keyframe at `index`, reporting whether there was one.
    ///
    /// Emptying a track is allowed: an empty track samples to `None` and every joint it targeted
    /// falls back to its bind pose, which is a legitimate thing to author.
    pub fn remove_keyframe(&mut self, index: usize) -> bool {
        if index >= self.keyframes.len() {
            return false;
        }
        self.keyframes.remove(index);
        true
    }

    /// The timestamp of the last keyframe, or `None` for an empty track.
    pub fn last_time(&self) -> Option<f32> {
        self.keyframes.last().map(|k| k.time)
    }

    /// Is the list still sorted ascending? For tests and debug assertions — the sampling path
    /// assumes this and cannot check it.
    pub fn is_sorted_by_time(&self) -> bool {
        self.keyframes.windows(2).all(|w| w[0].time <= w[1].time)
    }
}

impl<T: Clone + Copy> Track<T> {
    /// Locate where `time` falls in the keyframe list (shared by linear & cubic sampling).
    fn segment(&self, time: f32) -> Option<SegmentPos> {
        if self.keyframes.is_empty() {
            return None;
        }
        let last_idx = self.keyframes.len() - 1;
        if self.keyframes.len() == 1 || time <= self.keyframes[0].time {
            return Some(SegmentPos::Clamp(0));
        }
        if time >= self.keyframes[last_idx].time {
            return Some(SegmentPos::Clamp(last_idx));
        }

        // Binary search ile doğru aralığı bul (O(log N) — eskiden O(N) doğrusal arama)
        let idx = self.keyframes.partition_point(|k| k.time < time);
        if idx == 0 {
            return Some(SegmentPos::Clamp(0));
        }
        let i = idx - 1;
        let j = (i + 1).min(last_idx);
        let dt = self.keyframes[j].time - self.keyframes[i].time;
        let t = if dt > 0.0 {
            (time - self.keyframes[i].time) / dt
        } else {
            0.0
        };
        Some(SegmentPos::Interp { i, j, t, dt })
    }

    /// Sample the track at `time` (seconds, same origin as [`Keyframe::time`]) using only
    /// keyframe values — the tangent-free path, and the one every mode can take.
    ///
    /// `interpolator` receives `(earlier_value, later_value, t)` with `t` the normalized
    /// position in the segment; callers pass `lerp` for vectors and `slerp` for
    /// quaternions, which is why this module needs no maths of its own. It is called at most
    /// once per sample, and not at all for `Step` or outside the keyframe range.
    ///
    /// Returns `None` **only** when the track has no keyframes; every other case yields a
    /// value. Times at or beyond the ends clamp to the first/last keyframe value, `Step`
    /// returns the earlier keyframe's value, and — deliberately — `CubicSpline` is treated
    /// exactly like `Linear` here. That last point is what makes this a safe fallback for
    /// [`sample_cubic`](Track::sample_cubic): try the Hermite path, and when it declines
    /// (no tangents), lerping through here still produces a usable pose.
    ///
    /// Duplicate timestamps are safe: the segment picked always starts strictly before the
    /// query time, so for a sorted list the zero-length segment between two keyframes sharing
    /// a timestamp is never sampled and nothing divides by zero.
    pub fn get_interpolated(
        &self,
        time: f32,
        mut interpolator: impl FnMut(T, T, f32) -> T,
    ) -> Option<T> {
        match self.segment(time)? {
            SegmentPos::Clamp(idx) => Some(self.keyframes[idx].value),
            SegmentPos::Interp { i, j, t, .. } => match self.interpolation {
                InterpolationMode::Step => Some(self.keyframes[i].value),
                // CubicSpline falls back to a linear blend here; callers that want true
                // cubic-Hermite use `sample_cubic` (which supplies the tangent math).
                InterpolationMode::Linear | InterpolationMode::CubicSpline => {
                    Some(interpolator(self.keyframes[i].value, self.keyframes[j].value, t))
                }
            },
        }
    }

    /// True cubic-Hermite sampling for `CubicSpline` tracks (glTF Appendix C).
    ///
    /// Returns `None` — so the caller can fall back to [`get_interpolated`] — when the track
    /// is not `CubicSpline` or a segment is missing its tangents. `cubic` receives
    /// `(p0, m0, p1, m1, s, dt)`: the segment endpoints, keyframe `k`'s out-tangent and
    /// keyframe `k+1`'s in-tangent (both per-second, scale by `dt`), the normalized position
    /// `s ∈ [0,1)` and the segment duration `dt`.
    pub fn sample_cubic(
        &self,
        time: f32,
        mut cubic: impl FnMut(T, T, T, T, f32, f32) -> T,
    ) -> Option<T> {
        if self.interpolation != InterpolationMode::CubicSpline {
            return None;
        }
        match self.segment(time)? {
            // At/beyond the ends the value is exact regardless of tangents.
            SegmentPos::Clamp(idx) => Some(self.keyframes[idx].value),
            SegmentPos::Interp { i, j, t, dt } => {
                let k1 = &self.keyframes[i];
                let k2 = &self.keyframes[j];
                match (k1.out_tangent, k2.in_tangent) {
                    (Some(m0), Some(m1)) => Some(cubic(k1.value, m0, k2.value, m1, t, dt)),
                    // Tangents were not preserved (e.g. author data) → let caller lerp.
                    _ => None,
                }
            }
        }
    }
}

#[cfg(test)]
mod editing_tests {
    use super::*;

    fn track(times: &[f32]) -> Track<f32> {
        Track {
            target_node: 0,
            target_node_name: None,
            interpolation: InterpolationMode::Linear,
            keyframes: times
                .iter()
                .map(|&t| Keyframe { time: t, value: t, in_tangent: None, out_tangent: None })
                .collect(),
        }
    }

    fn times(t: &Track<f32>) -> Vec<f32> {
        t.keyframes.iter().map(|k| k.time).collect()
    }

    /// The whole reason these methods exist: sampling binary-searches the list, so a drag past a
    /// neighbour must reorder it rather than leave it unsorted and silently wrong.
    #[test]
    fn dragging_a_key_past_its_neighbour_reorders_the_list() {
        let mut t = track(&[0.0, 1.0, 2.0]);
        // Drag the FIRST key to the far end.
        let at = t.retime_keyframe(0, 5.0).expect("a valid index and time");
        assert_eq!(at, 2, "it must report where it landed, not where it was");
        assert_eq!(times(&t), vec![1.0, 2.0, 5.0]);
        assert!(t.is_sorted_by_time());

        // ...and back past the others the other way.
        let at = t.retime_keyframe(2, 0.5).expect("valid");
        assert_eq!(at, 0);
        assert_eq!(times(&t), vec![0.5, 1.0, 2.0]);
        assert!(t.is_sorted_by_time());
    }

    /// The value travels with the key. A retime that reordered the timestamps but left the
    /// values where they were would rewrite the animation instead of moving one key.
    #[test]
    fn the_value_moves_with_its_timestamp() {
        let mut t = track(&[0.0, 1.0, 2.0]); // value == time, so the pairing is visible
        t.retime_keyframe(0, 5.0).unwrap();
        let pairs: Vec<(f32, f32)> = t.keyframes.iter().map(|k| (k.time, k.value)).collect();
        assert_eq!(pairs, vec![(1.0, 1.0), (2.0, 2.0), (5.0, 0.0)]);
    }

    /// NaN is refused, not clamped. Every comparison against it is false, so one NaN timestamp
    /// makes the list permanently unsortable and `partition_point` nonsense from then on — on a
    /// track that still looks perfectly fine.
    #[test]
    fn a_nan_time_is_refused_and_changes_nothing() {
        let mut t = track(&[0.0, 1.0]);
        assert!(t.retime_keyframe(0, f32::NAN).is_none());
        assert!(t.retime_keyframe(0, f32::INFINITY).is_none());
        assert_eq!(times(&t), vec![0.0, 1.0], "a refused edit must not have moved anything");
        assert!(t.is_sorted_by_time());
    }

    /// Negative times clamp to zero: duration is measured from `t = 0`, so a key before the
    /// start is not a shorter clip, it is a key that can never be the sampled segment.
    #[test]
    fn a_negative_time_clamps_to_the_start() {
        let mut t = track(&[1.0, 2.0]);
        let at = t.retime_keyframe(1, -3.0).unwrap();
        assert_eq!(at, 0);
        assert_eq!(times(&t), vec![0.0, 1.0]);
    }

    #[test]
    fn an_out_of_range_index_changes_nothing() {
        let mut t = track(&[0.0, 1.0]);
        assert!(t.retime_keyframe(9, 0.5).is_none());
        assert!(!t.remove_keyframe(9));
        assert_eq!(times(&t), vec![0.0, 1.0]);
    }

    /// Equal timestamps are legal; the moved key lands after its equals, which is where a drag
    /// leaves it on screen.
    #[test]
    fn a_key_dropped_onto_another_lands_after_it() {
        let mut t = track(&[0.0, 1.0, 2.0]);
        let at = t.retime_keyframe(2, 1.0).unwrap();
        assert_eq!(at, 2, "after the key it was dropped onto");
        assert_eq!(times(&t), vec![0.0, 1.0, 1.0]);
        assert!(t.is_sorted_by_time());
    }

    #[test]
    fn removing_takes_that_key_and_can_empty_the_track() {
        let mut t = track(&[0.0, 1.0, 2.0]);
        assert!(t.remove_keyframe(1));
        assert_eq!(times(&t), vec![0.0, 2.0]);
        assert!(t.remove_keyframe(0));
        assert!(t.remove_keyframe(0));
        assert!(t.keyframes.is_empty(), "emptying a track is allowed");
        assert_eq!(t.last_time(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_track(keyframes: Vec<(f32, f32)>, interp: InterpolationMode) -> Track<f32> {
        Track {
            target_node: 0,
            target_node_name: None,
            interpolation: interp,
            keyframes: keyframes.into_iter().map(|(t, v)| Keyframe::new(t, v)).collect(),
        }
    }

    // ── Track Interpolation Tests ──────────────────────────────────────

    #[test]
    fn test_track_empty() {
        let track = make_track(vec![], InterpolationMode::Linear);
        assert!(track.get_interpolated(0.5, |a, b, t| a + (b - a) * t).is_none());
    }

    #[test]
    fn test_track_single_keyframe() {
        let track = make_track(vec![(1.0, 42.0)], InterpolationMode::Linear);
        assert_eq!(track.get_interpolated(0.0, |a, b, t| a + (b - a) * t), Some(42.0));
        assert_eq!(track.get_interpolated(5.0, |a, b, t| a + (b - a) * t), Some(42.0));
    }

    #[test]
    fn test_track_linear_interpolation() {
        let track = make_track(vec![(0.0, 0.0), (1.0, 10.0)], InterpolationMode::Linear);
        let v = track.get_interpolated(0.5, |a, b, t| a + (b - a) * t).unwrap();
        assert!((v - 5.0).abs() < 0.001, "Expected 5.0, got {v}");
    }

    #[test]
    fn test_track_step_interpolation() {
        let track = make_track(vec![(0.0, 0.0), (1.0, 10.0)], InterpolationMode::Step);
        let v = track.get_interpolated(0.5, |a, b, t| a + (b - a) * t).unwrap();
        assert_eq!(v, 0.0, "Step mode should hold the first keyframe value");
    }

    #[test]
    fn test_track_clamp_before_first() {
        let track = make_track(vec![(1.0, 5.0), (2.0, 10.0)], InterpolationMode::Linear);
        assert_eq!(track.get_interpolated(0.0, |a, b, t| a + (b - a) * t), Some(5.0));
    }

    #[test]
    fn test_track_clamp_after_last() {
        let track = make_track(vec![(1.0, 5.0), (2.0, 10.0)], InterpolationMode::Linear);
        assert_eq!(track.get_interpolated(100.0, |a, b, t| a + (b - a) * t), Some(10.0));
    }

    #[test]
    fn test_track_many_keyframes_binary_search() {
        let keyframes: Vec<(f32, f32)> = (0..100).map(|i| (i as f32, i as f32 * 2.0)).collect();
        let track = make_track(keyframes, InterpolationMode::Linear);
        let v = track.get_interpolated(50.5, |a, b, t| a + (b - a) * t).unwrap();
        assert!((v - 101.0).abs() < 0.001, "Expected 101.0, got {v}");
    }

    #[test]
    fn test_track_zero_duration_keyframe() {
        // İki keyframe aynı zamanda → dt=0, t=0 olmalı, bölme hatası olmamalı
        let track = make_track(vec![(1.0, 5.0), (1.0, 10.0)], InterpolationMode::Linear);
        let v = track.get_interpolated(1.0, |a, b, t| a + (b - a) * t).unwrap();
        assert_eq!(v, 5.0, "dt=0 durumunda ilk keyframe değeri döndürülmeli");
    }

    // ── Cubic-Hermite Tests ────────────────────────────────────────────

    /// glTF Appendix C Hermite basis for scalar values.
    fn hermite_f32(p0: f32, m0: f32, p1: f32, m1: f32, s: f32, dt: f32) -> f32 {
        let s2 = s * s;
        let s3 = s2 * s;
        let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
        let h10 = s3 - 2.0 * s2 + s;
        let h01 = -2.0 * s3 + 3.0 * s2;
        let h11 = s3 - s2;
        h00 * p0 + h10 * (dt * m0) + h01 * p1 + h11 * (dt * m1)
    }

    #[test]
    fn sample_cubic_returns_none_for_non_cubic_track() {
        // A Linear track must decline cubic sampling so the caller keeps lerping.
        let track = make_track(vec![(0.0, 0.0), (1.0, 10.0)], InterpolationMode::Linear);
        assert!(track.sample_cubic(0.5, hermite_f32).is_none());
    }

    #[test]
    fn sample_cubic_falls_back_when_tangents_missing() {
        // CubicSpline mode but keyframes carry no tangents → None (caller lerps).
        let track = make_track(vec![(0.0, 0.0), (1.0, 10.0)], InterpolationMode::CubicSpline);
        assert!(track.sample_cubic(0.5, hermite_f32).is_none());
    }

    #[test]
    fn sample_cubic_interpolates_with_tangents() {
        // Flat tangents (m=0) at both ends → a smooth ease that at s=0.5 gives the
        // Hermite midpoint 0.5*(p0+p1) = 5.0, but with a zero first-derivative shape it is
        // NOT the same as an arbitrary lerp elsewhere. Value must match the basis exactly.
        let track = Track {
            target_node: 0,
            target_node_name: None,
            interpolation: InterpolationMode::CubicSpline,
            keyframes: vec![
                Keyframe::with_tangents(0.0, 0.0, 0.0, 0.0),
                Keyframe::with_tangents(1.0, 10.0, 0.0, 0.0),
            ],
        };
        let v = track.sample_cubic(0.25, hermite_f32).unwrap();
        // Analytic: h00(.25)*0 + 0 + h01(.25)*10 + 0 = (-2*.015625+3*.0625)*10 = 1.5625
        assert!((v - 1.5625).abs() < 1e-5, "cubic ease at s=0.25 should be 1.5625, got {v}");
        // Distinct from linear (which would be 2.5): proves cubic actually ran.
        let lin = track.get_interpolated(0.25, |a, b, t| a + (b - a) * t).unwrap();
        assert!((lin - 2.5).abs() < 1e-5 && (v - lin).abs() > 0.5, "cubic must differ from lerp");
    }

    #[test]
    fn sample_cubic_clamps_at_ends() {
        let track = Track {
            target_node: 0,
            target_node_name: None,
            interpolation: InterpolationMode::CubicSpline,
            keyframes: vec![
                Keyframe::with_tangents(1.0, 5.0, 2.0, 2.0),
                Keyframe::with_tangents(2.0, 9.0, 2.0, 2.0),
            ],
        };
        assert_eq!(track.sample_cubic(0.0, hermite_f32), Some(5.0));
        assert_eq!(track.sample_cubic(100.0, hermite_f32), Some(9.0));
    }

    #[test]
    fn sample_cubic_partial_segment_tangents_return_none() {
        // Only the FIRST keyframe carries tangents; the segment's second endpoint is
        // missing its in-tangent, so cubic sampling must decline (caller lerps) rather
        // than fabricate a curve from half the data.
        let track = Track {
            target_node: 0,
            target_node_name: None,
            interpolation: InterpolationMode::CubicSpline,
            keyframes: vec![
                Keyframe::with_tangents(0.0, 0.0, 1.0, 1.0),
                Keyframe::new(1.0, 10.0), // no tangents
            ],
        };
        assert!(track.sample_cubic(0.5, hermite_f32).is_none());
    }

    #[test]
    fn sample_cubic_clamps_end_even_without_tangents() {
        // At/after the last keyframe the exact value is returned regardless of whether
        // tangents were preserved — the Clamp arm never touches the tangent math.
        let track = make_track(vec![(0.0, 0.0), (1.0, 7.0)], InterpolationMode::CubicSpline);
        assert_eq!(track.sample_cubic(5.0, hermite_f32), Some(7.0));
        assert_eq!(track.sample_cubic(-5.0, hermite_f32), Some(0.0));
    }

    #[test]
    fn get_interpolated_treats_cubic_as_linear_blend() {
        // `get_interpolated` is the tangent-free fallback path: a CubicSpline track
        // sampled through it must produce the plain linear blend, not a Hermite curve.
        let track = make_track(vec![(0.0, 0.0), (1.0, 10.0)], InterpolationMode::CubicSpline);
        let v = track.get_interpolated(0.25, |a, b, t| a + (b - a) * t).unwrap();
        assert!((v - 2.5).abs() < 1e-5, "cubic-via-get_interpolated should lerp to 2.5, got {v}");
    }
}
