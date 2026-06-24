use gizmo_math::{Vec3, Quat};

/// The keyframe data for a [`Track`], one variant per animated transform channel.
///
/// Each vector is parallel to [`Track::keyframe_timestamps`].
#[derive(Clone, Debug)]
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
pub struct Track {
    /// Name of the entity this track animates (resolved at runtime).
    pub target_name: String,
    /// Keyframe times in seconds; must be sorted ascending and match `keyframes` in length.
    pub keyframe_timestamps: Vec<f32>,
    /// The keyframe values for this track.
    pub keyframes: Keyframes,
}

impl Track {
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
        let idx0 = idx - 1;
        let idx1 = idx;

        let t0 = self.keyframe_timestamps[idx0];
        let t1 = self.keyframe_timestamps[idx1];
        let factor = (t - t0) / (t1 - t0);

        self.interpolate_values(idx0, idx1, factor)
    }

    fn get_value(&self, index: usize) -> InterpolatedValue {
        match &self.keyframes {
            Keyframes::Translation(v) => InterpolatedValue::Translation(v[index]),
            Keyframes::Rotation(v) => InterpolatedValue::Rotation(v[index]),
            Keyframes::Scale(v) => InterpolatedValue::Scale(v[index]),
        }
    }

    fn interpolate_values(&self, idx0: usize, idx1: usize, factor: f32) -> InterpolatedValue {
        match &self.keyframes {
            Keyframes::Translation(v) => {
                let v0 = v[idx0];
                let v1 = v[idx1];
                InterpolatedValue::Translation(v0.lerp(v1, factor))
            }
            Keyframes::Rotation(v) => {
                let v0 = v[idx0];
                let v1 = v[idx1];
                InterpolatedValue::Rotation(v0.slerp(v1, factor))
            }
            Keyframes::Scale(v) => {
                let v0 = v[idx0];
                let v1 = v[idx1];
                InterpolatedValue::Scale(v0.lerp(v1, factor))
            }
        }
    }
}

/// A value sampled from a [`Track`] at a specific time.
///
/// The variant indicates which transform channel the value belongs to.
#[derive(Clone, Copy, Debug, PartialEq)]
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
