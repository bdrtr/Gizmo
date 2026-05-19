use gizmo_math::{Vec3, Quat};

#[derive(Clone, Debug)]
pub enum Keyframes {
    Translation(Vec<Vec3>),
    Rotation(Vec<Quat>),
    Scale(Vec<Vec3>),
}

#[derive(Clone, Debug)]
pub struct Track {
    pub target_name: String, // Which entity this track applies to (by name)
    pub keyframe_timestamps: Vec<f32>,
    pub keyframes: Keyframes,
}

impl Track {
    pub fn duration(&self) -> f32 {
        self.keyframe_timestamps.last().copied().unwrap_or(0.0)
    }

    /// Interpolates the track at a given time `t`.
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

pub enum InterpolatedValue {
    None,
    Translation(Vec3),
    Rotation(Quat),
    Scale(Vec3),
}

/// Repesents an animation sequence
#[derive(Clone, Debug, Default)]
pub struct AnimationClip {
    pub name: String,
    pub tracks: Vec<Track>,
}

impl AnimationClip {
    pub fn duration(&self) -> f32 {
        self.tracks
            .iter()
            .map(|t| t.duration())
            .fold(0.0, f32::max)
    }
}
