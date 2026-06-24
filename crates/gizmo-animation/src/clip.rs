use gizmo_math::{Vec3, Quat};

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
/// Each vector is parallel to [`Track::keyframe_timestamps`].
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
}

impl Track {
    /// Creates a new track targeting `target_name` with the given keyframe
    /// timestamps and values.
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
        })
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

        // Find the segment
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
        let span = t1 - t0;
        let factor = if span.abs() > f32::EPSILON {
            ((t - t0) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };

        self.interpolate_values(idx0, idx1, factor)
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

    fn interpolate_values(&self, idx0: usize, idx1: usize, factor: f32) -> InterpolatedValue {
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
        self.tracks
            .iter()
            .map(|t| t.duration())
            .fold(0.0, f32::max)
    }
}
