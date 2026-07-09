use gizmo_math::{Quat, Vec3};

use crate::hermite::{hermite_quat, hermite_vec3};

/// Error returned when constructing a [`Track`] with inconsistent keyframe data.
///
/// A well-formed track requires its `keyframe_timestamps` to match the keyframe
/// values one-to-one, to be sorted ascending, and to contain only finite values.
/// Violating any of these invariants would otherwise lead to out-of-bounds
/// indexing or `NaN`-poisoned interpolation at sample time, so [`Track::new`]
/// rejects such data up front.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrackError {
    /// The number of keyframe timestamps did not match the number of keyframe
    /// values.
    LengthMismatch {
        /// Number of supplied timestamps.
        timestamps: usize,
        /// Number of supplied keyframe values.
        values: usize,
    },
    /// A timestamp was `NaN` or infinite.
    NonFiniteTimestamp {
        /// Index of the offending timestamp.
        index: usize,
    },
    /// Timestamps were not sorted in ascending order.
    UnsortedTimestamps {
        /// Index of the timestamp that was smaller than its predecessor.
        index: usize,
    },
}

impl std::fmt::Display for TrackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrackError::LengthMismatch { timestamps, values } => write!(
                f,
                "keyframe timestamp count ({timestamps}) does not match keyframe value count ({values})"
            ),
            TrackError::NonFiniteTimestamp { index } => {
                write!(f, "keyframe timestamp at index {index} is not finite")
            }
            TrackError::UnsortedTimestamps { index } => write!(
                f,
                "keyframe timestamp at index {index} is not in ascending order"
            ),
        }
    }
}

impl std::error::Error for TrackError {}

/// The keyframe data for a [`Track`], one variant per animated transform channel.
///
/// Each vector is parallel to [`Track::keyframe_timestamps`]. Scale is a
/// first-class channel: it is sampled and applied to the target's `Transform`
/// just like translation and rotation.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Keyframes {
    /// Position keyframes (linearly interpolated).
    Translation(Vec<Vec3>),
    /// Rotation keyframes (spherically interpolated).
    Rotation(Vec<Quat>),
    /// Scale keyframes (linearly interpolated).
    Scale(Vec<Vec3>),
}

impl Keyframes {
    /// Returns the number of keyframe values held by this channel.
    pub fn len(&self) -> usize {
        match self {
            Keyframes::Translation(v) => v.len(),
            Keyframes::Rotation(v) => v.len(),
            Keyframes::Scale(v) => v.len(),
        }
    }

    /// Returns `true` if this channel holds no keyframe values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// How values between two keyframes are blended.
///
/// Mirrors the glTF animation sampler interpolation modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interpolation {
    /// Linear (lerp for vectors, slerp for rotations).
    #[default]
    Linear,
    /// Hold the value of the previous keyframe until the next one.
    Step,
    /// Cubic Hermite spline using per-keyframe in/out tangents. Requires
    /// [`Track::tangents`] to be populated; falls back to `Linear` if absent.
    CubicSpline,
}

/// In/out tangents for [`Interpolation::CubicSpline`].
///
/// glTF stores cubic-spline output accessors as interleaved
/// `[in_tangent, value, out_tangent]` triples per keyframe; when loading, split
/// them into the track's `keyframes` (the values) and this struct. Both tangent
/// arrays must use the same [`Keyframes`] variant and length as the values.
#[derive(Clone, Debug)]
pub struct CubicTangents {
    /// In-tangent per keyframe (aligned with `keyframe_timestamps`).
    pub in_tangents: Keyframes,
    /// Out-tangent per keyframe (aligned with `keyframe_timestamps`).
    pub out_tangents: Keyframes,
}

/// A single animated channel targeting one named entity.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Track {
    /// Name of the entity this track animates (resolved at runtime).
    pub target_name: String,
    /// Keyframe times in seconds; must be sorted ascending and match `keyframes` in length.
    pub keyframe_timestamps: Vec<f32>,
    /// The keyframe values for this track.
    pub keyframes: Keyframes,
    /// How to blend between keyframes. Defaults to [`Interpolation::Linear`].
    pub interpolation: Interpolation,
    /// Cubic-spline tangents; only consulted when
    /// `interpolation == Interpolation::CubicSpline`.
    pub tangents: Option<CubicTangents>,
}

impl Track {
    /// Creates a new linearly-interpolated track targeting `target_name` with the
    /// given keyframe timestamps and values.
    ///
    /// # Errors
    ///
    /// Returns a [`TrackError`] if `keyframe_timestamps` does not match the
    /// length of the data inside `keyframes`, if any timestamp is non-finite,
    /// or if the timestamps are not sorted in ascending order. Enforcing these
    /// invariants here guarantees that [`Track::sample`] cannot panic or emit
    /// `NaN` values for a track built through this constructor.
    pub fn new(
        target_name: impl Into<String>,
        keyframe_timestamps: Vec<f32>,
        keyframes: Keyframes,
    ) -> Result<Self, TrackError> {
        let values = keyframes.len();
        let timestamps = keyframe_timestamps.len();
        if timestamps != values {
            return Err(TrackError::LengthMismatch { timestamps, values });
        }

        let mut prev: Option<f32> = None;
        for (index, &ts) in keyframe_timestamps.iter().enumerate() {
            if !ts.is_finite() {
                return Err(TrackError::NonFiniteTimestamp { index });
            }
            if let Some(p) = prev {
                if ts < p {
                    return Err(TrackError::UnsortedTimestamps { index });
                }
            }
            prev = Some(ts);
        }

        Ok(Self {
            target_name: target_name.into(),
            keyframe_timestamps,
            keyframes,
            interpolation: Interpolation::Linear,
            tangents: None,
        })
    }

    /// Set the interpolation mode (builder style).
    pub fn with_interpolation(mut self, interpolation: Interpolation) -> Self {
        self.interpolation = interpolation;
        self
    }

    /// Attach cubic-spline tangents and switch to [`Interpolation::CubicSpline`].
    pub fn with_cubic_tangents(mut self, in_tangents: Keyframes, out_tangents: Keyframes) -> Self {
        self.interpolation = Interpolation::CubicSpline;
        self.tangents = Some(CubicTangents { in_tangents, out_tangents });
        self
    }

    /// Returns the time of the last keyframe, or `0.0` if the track is empty.
    pub fn duration(&self) -> f32 {
        self.keyframe_timestamps.last().copied().unwrap_or(0.0)
    }

    /// Interpolates the track at a given time `t`.
    ///
    /// Times before the first or after the last keyframe are clamped to the
    /// endpoint value. An empty track returns [`InterpolatedValue::None`].
    pub fn sample(&self, t: f32) -> InterpolatedValue {
        if self.keyframe_timestamps.is_empty() {
            return InterpolatedValue::None;
        }

        if t <= *self.keyframe_timestamps.first().unwrap() {
            return self.get_value(0);
        }

        if t >= *self.keyframe_timestamps.last().unwrap() {
            return self.get_value(self.keyframe_timestamps.len() - 1);
        }

        // Find the segment.
        let idx = self.keyframe_timestamps.partition_point(|&ts| ts <= t);
        // `idx` is in `1..len` here (the t-before-first / t-after-last cases
        // returned above), but guard the subtraction defensively in case of
        // NaN timestamps that break the ordering assumptions of partition_point.
        let idx1 = idx.clamp(1, self.keyframe_timestamps.len() - 1);
        let idx0 = idx1 - 1;

        let t0 = self.keyframe_timestamps[idx0];
        let t1 = self.keyframe_timestamps[idx1];
        // Guard against a zero/degenerate (or non-finite) segment span: two
        // identical or out-of-order timestamps would produce a division by zero
        // (NaN/Inf). Fall back to the start value rather than emitting NaN.
        let segment = t1 - t0;
        let factor = if segment.abs() > f32::EPSILON {
            ((t - t0) / segment).clamp(0.0, 1.0)
        } else {
            0.0
        };

        match self.effective_interpolation() {
            Interpolation::Step => self.get_value(idx0),
            Interpolation::Linear => self.interpolate_linear(idx0, idx1, factor),
            Interpolation::CubicSpline => self.interpolate_cubic(idx0, idx1, factor, segment),
        }
    }

    /// The interpolation actually used: falls back to `Linear` if `CubicSpline`
    /// was requested without tangents.
    fn effective_interpolation(&self) -> Interpolation {
        match self.interpolation {
            Interpolation::CubicSpline if self.tangents.is_none() => Interpolation::Linear,
            other => other,
        }
    }

    fn get_value(&self, index: usize) -> InterpolatedValue {
        // Bounds-checked access: the keyframe vectors are public and may be
        // shorter than `keyframe_timestamps`, so a mismatched-length Track must
        // not panic with an out-of-bounds index.
        match &self.keyframes {
            Keyframes::Translation(v) => match v.get(index) {
                Some(&val) => InterpolatedValue::Translation(val),
                None => InterpolatedValue::None,
            },
            Keyframes::Rotation(v) => match v.get(index) {
                Some(&val) => InterpolatedValue::Rotation(val),
                None => InterpolatedValue::None,
            },
            Keyframes::Scale(v) => match v.get(index) {
                Some(&val) => InterpolatedValue::Scale(val),
                None => InterpolatedValue::None,
            },
        }
    }

    fn interpolate_linear(&self, idx0: usize, idx1: usize, factor: f32) -> InterpolatedValue {
        // Bounds-checked: a Track whose `keyframes` vector is shorter than its
        // `keyframe_timestamps` must degrade gracefully instead of panicking.
        match &self.keyframes {
            Keyframes::Translation(v) => match (v.get(idx0), v.get(idx1)) {
                (Some(&v0), Some(&v1)) => InterpolatedValue::Translation(v0.lerp(v1, factor)),
                _ => InterpolatedValue::None,
            },
            Keyframes::Rotation(v) => match (v.get(idx0), v.get(idx1)) {
                (Some(&v0), Some(&v1)) => InterpolatedValue::Rotation(v0.slerp(v1, factor)),
                _ => InterpolatedValue::None,
            },
            Keyframes::Scale(v) => match (v.get(idx0), v.get(idx1)) {
                (Some(&v0), Some(&v1)) => InterpolatedValue::Scale(v0.lerp(v1, factor)),
                _ => InterpolatedValue::None,
            },
        }
    }

    /// Cubic Hermite spline using the real in/out tangents from the animation
    /// format (glTF convention: `m0 = out_tangent[idx0] * dt`,
    /// `m1 = in_tangent[idx1] * dt`, where `dt` is the segment duration).
    ///
    /// Falls back to [`Self::interpolate_linear`] if the value/tangent variants
    /// disagree or the tangent arrays are shorter than the indexed keyframes, so
    /// a malformed cubic track degrades gracefully instead of panicking.
    fn interpolate_cubic(
        &self,
        idx0: usize,
        idx1: usize,
        factor: f32,
        segment: f32,
    ) -> InterpolatedValue {
        // `effective_interpolation` guarantees tangents are present here.
        let tangents = self.tangents.as_ref().expect("cubic requires tangents");
        match (&self.keyframes, &tangents.in_tangents, &tangents.out_tangents) {
            (Keyframes::Translation(v), Keyframes::Translation(in_t), Keyframes::Translation(out_t)) => {
                match (v.get(idx0), out_t.get(idx0), v.get(idx1), in_t.get(idx1)) {
                    (Some(&p0), Some(&m0), Some(&p1), Some(&m1)) => InterpolatedValue::Translation(
                        hermite_vec3(p0, m0 * segment, p1, m1 * segment, factor),
                    ),
                    _ => self.interpolate_linear(idx0, idx1, factor),
                }
            }
            (Keyframes::Scale(v), Keyframes::Scale(in_t), Keyframes::Scale(out_t)) => {
                match (v.get(idx0), out_t.get(idx0), v.get(idx1), in_t.get(idx1)) {
                    (Some(&p0), Some(&m0), Some(&p1), Some(&m1)) => InterpolatedValue::Scale(
                        hermite_vec3(p0, m0 * segment, p1, m1 * segment, factor),
                    ),
                    _ => self.interpolate_linear(idx0, idx1, factor),
                }
            }
            (Keyframes::Rotation(v), Keyframes::Rotation(in_t), Keyframes::Rotation(out_t)) => {
                let scale = |q: Quat, s: f32| Quat::from_xyzw(q.x * s, q.y * s, q.z * s, q.w * s);
                match (v.get(idx0), out_t.get(idx0), v.get(idx1), in_t.get(idx1)) {
                    (Some(&p0), Some(&m0), Some(&p1), Some(&m1)) => InterpolatedValue::Rotation(
                        hermite_quat(p0, scale(m0, segment), p1, scale(m1, segment), factor),
                    ),
                    _ => self.interpolate_linear(idx0, idx1, factor),
                }
            }
            // Mismatched value/tangent variants: fall back to linear rather than panic.
            _ => self.interpolate_linear(idx0, idx1, factor),
        }
    }
}

/// A value sampled from a [`Track`] at a specific time.
///
/// The variant indicates which transform channel the value belongs to.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum InterpolatedValue {
    /// The track produced no value (e.g. it had no keyframes).
    None,
    /// A sampled position.
    Translation(Vec3),
    /// A sampled rotation.
    Rotation(Quat),
    /// A sampled scale.
    Scale(Vec3),
}

/// Represents an animation sequence: a named collection of [`Track`]s.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AnimationClip {
    /// Human-readable clip name.
    pub name: String,
    /// The tracks that make up this clip.
    pub tracks: Vec<Track>,
}

impl AnimationClip {
    /// Returns the length of the longest track, in seconds.
    pub fn duration(&self) -> f32 {
        self.tracks.iter().map(|t| t.duration()).fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f32 = 1e-4;

    fn scale_track(interp: Interpolation) -> Track {
        Track::new(
            "bone",
            vec![0.0, 1.0],
            Keyframes::Scale(vec![Vec3::new(1.0, 1.0, 1.0), Vec3::new(2.0, 4.0, 8.0)]),
        )
        .expect("valid track")
        .with_interpolation(interp)
    }

    #[test]
    fn scale_track_linear_non_uniform() {
        // A non-uniform scale keyed 1..(2,4,8) sampled at t=0.5 must lerp each axis
        // independently. This is the regression guard for scale reaching the pose.
        let track = scale_track(Interpolation::Linear);
        match track.sample(0.5) {
            InterpolatedValue::Scale(s) => {
                assert!((s - Vec3::new(1.5, 2.5, 4.5)).length() < TOL, "got {s:?}");
            }
            other => panic!("expected Scale, got {other:?}"),
        }
    }

    #[test]
    fn scale_track_endpoints_and_clamp() {
        let track = scale_track(Interpolation::Linear);
        assert_eq!(track.sample(0.0), InterpolatedValue::Scale(Vec3::new(1.0, 1.0, 1.0)));
        assert_eq!(track.sample(1.0), InterpolatedValue::Scale(Vec3::new(2.0, 4.0, 8.0)));
        // Clamp beyond the ends.
        assert_eq!(track.sample(-5.0), InterpolatedValue::Scale(Vec3::new(1.0, 1.0, 1.0)));
        assert_eq!(track.sample(9.0), InterpolatedValue::Scale(Vec3::new(2.0, 4.0, 8.0)));
    }

    #[test]
    fn scale_track_step_holds_previous() {
        // Step interpolation did not exist before; a linear-only sampler would
        // return (1.5,2.5,4.5) here instead of holding the first keyframe.
        let track = scale_track(Interpolation::Step);
        match track.sample(0.5) {
            InterpolatedValue::Scale(s) => {
                assert!((s - Vec3::new(1.0, 1.0, 1.0)).length() < TOL, "step should hold prev, got {s:?}");
            }
            other => panic!("expected Scale, got {other:?}"),
        }
    }

    #[test]
    fn scale_track_cubic_uses_real_tangents() {
        // Cubic spline with real (non-zero) tangents must differ from the linear
        // midpoint. A sampler that ignored tangents (or lacked cubic support)
        // would return the linear value (1.5,2.5,4.5) and fail this test.
        let values = Keyframes::Scale(vec![Vec3::new(1.0, 1.0, 1.0), Vec3::new(2.0, 4.0, 8.0)]);
        // Steep out-tangent at k0, flat in-tangent at k1 -> curve overshoots the
        // linear line in the first half.
        let in_t = Keyframes::Scale(vec![Vec3::ZERO, Vec3::ZERO]);
        let out_t = Keyframes::Scale(vec![Vec3::new(4.0, 4.0, 4.0), Vec3::ZERO]);
        let track = Track::new("bone", vec![0.0, 1.0], values)
            .unwrap()
            .with_cubic_tangents(in_t, out_t);

        let cubic = match track.sample(0.5) {
            InterpolatedValue::Scale(s) => s,
            other => panic!("expected Scale, got {other:?}"),
        };
        let linear = Vec3::new(1.5, 2.5, 4.5);
        assert!((cubic - linear).length() > 0.1, "cubic must differ from linear, got {cubic:?}");

        // Verify against the hand-computed Hermite value for the X axis.
        // p0=1, m0=4*1 (dt=1), p1=2, m1=0, t=0.5:
        // h00=0.5, h10=0.125, h01=0.5, h11=-0.125 -> 0.5*1 + 0.125*4 + 0.5*2 = 2.0
        assert!((cubic.x - 2.0).abs() < TOL, "cubic X expected 2.0, got {}", cubic.x);
    }

    #[test]
    fn cubic_without_tangents_falls_back_to_linear() {
        let mut track = scale_track(Interpolation::CubicSpline);
        track.tangents = None; // request cubic but provide nothing
        match track.sample(0.5) {
            InterpolatedValue::Scale(s) => assert!((s - Vec3::new(1.5, 2.5, 4.5)).length() < TOL),
            other => panic!("expected Scale, got {other:?}"),
        }
    }

    #[test]
    fn empty_track_samples_none() {
        let track = Track::new("bone", vec![], Keyframes::Scale(vec![])).unwrap();
        assert_eq!(track.sample(0.5), InterpolatedValue::None);
    }
}
