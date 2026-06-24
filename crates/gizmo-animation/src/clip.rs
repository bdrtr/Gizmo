use gizmo_math::{Vec3, Quat};

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
    /// `keyframe_timestamps` must be sorted ascending and match the length of
    /// the data inside `keyframes`.
    pub fn new(
        target_name: impl Into<String>,
        keyframe_timestamps: Vec<f32>,
        keyframes: Keyframes,
    ) -> Self {
        Self {
            target_name: target_name.into(),
            keyframe_timestamps,
            keyframes,
        }
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
