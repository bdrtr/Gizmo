#[derive(Clone, Copy, Debug)]
pub struct Keyframe<T> {
    pub time: f32,
    pub value: T,
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
    pub fn get_interpolated(
        &self,
        time: f32,
        mut interpolator: impl FnMut(T, T, f32) -> T,
    ) -> Option<T> {
        if self.keyframes.is_empty() {
            return None;
        }
        if self.keyframes.len() == 1 || time <= self.keyframes[0].time {
            return Some(self.keyframes[0].value);
        }
        let last_idx = self.keyframes.len() - 1;
        if time >= self.keyframes[last_idx].time {
            return Some(self.keyframes[last_idx].value);
        }

        // Binary search ile doğru aralığı bul (O(log N) — eskiden O(N) doğrusal arama)
        let idx = self.keyframes.partition_point(|k| k.time < time);
        if idx == 0 {
            return Some(self.keyframes[0].value);
        }
        let i = idx - 1;
        let k1 = &self.keyframes[i];
        let k2 = &self.keyframes[(i + 1).min(last_idx)];
        let dt = k2.time - k1.time;
        let t = if dt > 0.0 { (time - k1.time) / dt } else { 0.0 };

        match self.interpolation {
            InterpolationMode::Step => Some(k1.value),
            InterpolationMode::Linear | InterpolationMode::CubicSpline => {
                // Fallback CubicSpline to Linear if tangents are unavailable in simple T values
                Some(interpolator(k1.value, k2.value, t))
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
            keyframes: keyframes.into_iter().map(|(t, v)| Keyframe { time: t, value: v }).collect(),
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
}
