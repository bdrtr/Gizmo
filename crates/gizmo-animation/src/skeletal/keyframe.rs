#[derive(Clone, Copy, Debug)]
pub struct Keyframe<T> {
    pub time: f32,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InterpolationMode {
    Linear,
    Step,
    CubicSpline,
}

#[derive(Clone, Debug)]
pub struct Track<T> {
    pub target_node: usize,
    pub target_node_name: Option<String>,
    pub interpolation: InterpolationMode,
    pub keyframes: Vec<Keyframe<T>>,
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
