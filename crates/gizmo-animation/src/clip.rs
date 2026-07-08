use gizmo_math::{Quat, Vec3};

use crate::ik::{hermite_quat, hermite_vec3};

/// The animated channel of a [`Track`]. Translation and scale are `Vec3`
/// channels; rotation is a `Quat` channel. Scale is a first-class channel: it is
/// sampled and applied to the target's [`Transform::scale`](gizmo_physics_core::Transform)
/// just like translation and rotation.
#[derive(Clone, Debug)]
pub enum Keyframes {
    Translation(Vec<Vec3>),
    Rotation(Vec<Quat>),
    Scale(Vec<Vec3>),
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

#[derive(Clone, Debug)]
pub struct Track {
    pub target_name: String, // Which entity this track applies to (by name)
    pub keyframe_timestamps: Vec<f32>,
    pub keyframes: Keyframes,
    /// How to blend between keyframes. Defaults to [`Interpolation::Linear`].
    pub interpolation: Interpolation,
    /// Cubic-spline tangents; only consulted when
    /// `interpolation == Interpolation::CubicSpline`.
    pub tangents: Option<CubicTangents>,
}

impl Track {
    /// Create a linearly-interpolated track (the common case).
    pub fn new(target_name: impl Into<String>, keyframe_timestamps: Vec<f32>, keyframes: Keyframes) -> Self {
        Self {
            target_name: target_name.into(),
            keyframe_timestamps,
            keyframes,
            interpolation: Interpolation::Linear,
            tangents: None,
        }
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

        // Find the segment.
        let idx = self.keyframe_timestamps.partition_point(|&ts| ts <= t);
        let idx0 = idx - 1;
        let idx1 = idx;

        let t0 = self.keyframe_timestamps[idx0];
        let t1 = self.keyframe_timestamps[idx1];
        let segment = t1 - t0;
        let factor = if segment > 0.0 { (t - t0) / segment } else { 0.0 };

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
        match &self.keyframes {
            Keyframes::Translation(v) => InterpolatedValue::Translation(v[index]),
            Keyframes::Rotation(v) => InterpolatedValue::Rotation(v[index]),
            Keyframes::Scale(v) => InterpolatedValue::Scale(v[index]),
        }
    }

    fn interpolate_linear(&self, idx0: usize, idx1: usize, factor: f32) -> InterpolatedValue {
        match &self.keyframes {
            Keyframes::Translation(v) => InterpolatedValue::Translation(v[idx0].lerp(v[idx1], factor)),
            Keyframes::Rotation(v) => InterpolatedValue::Rotation(v[idx0].slerp(v[idx1], factor)),
            Keyframes::Scale(v) => InterpolatedValue::Scale(v[idx0].lerp(v[idx1], factor)),
        }
    }

    /// Cubic Hermite spline using the real in/out tangents from the animation
    /// format (glTF convention: `m0 = out_tangent[idx0] * dt`,
    /// `m1 = in_tangent[idx1] * dt`, where `dt` is the segment duration).
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
                let value = hermite_vec3(v[idx0], out_t[idx0] * segment, v[idx1], in_t[idx1] * segment, factor);
                InterpolatedValue::Translation(value)
            }
            (Keyframes::Scale(v), Keyframes::Scale(in_t), Keyframes::Scale(out_t)) => {
                let value = hermite_vec3(v[idx0], out_t[idx0] * segment, v[idx1], in_t[idx1] * segment, factor);
                InterpolatedValue::Scale(value)
            }
            (Keyframes::Rotation(v), Keyframes::Rotation(in_t), Keyframes::Rotation(out_t)) => {
                let scale = |q: Quat, s: f32| Quat::from_xyzw(q.x * s, q.y * s, q.z * s, q.w * s);
                let value = hermite_quat(
                    v[idx0],
                    scale(out_t[idx0], segment),
                    v[idx1],
                    scale(in_t[idx1], segment),
                    factor,
                );
                InterpolatedValue::Rotation(value)
            }
            // Mismatched value/tangent variants: fall back to linear rather than panic.
            _ => self.interpolate_linear(idx0, idx1, factor),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
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
        let track = Track::new("bone", vec![0.0, 1.0], values).with_cubic_tangents(in_t, out_t);

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
        let track = Track::new("bone", vec![], Keyframes::Scale(vec![]));
        assert_eq!(track.sample(0.5), InterpolatedValue::None);
    }
}
