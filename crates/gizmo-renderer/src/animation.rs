use gizmo_math::{Mat4, Quat, Vec3};

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

#[derive(Clone, Debug)]
pub struct AnimationClip {
    pub name: String,
    pub duration: f32,
    pub translations: Vec<Track<Vec3>>,
    pub rotations: Vec<Track<Quat>>,
    pub scales: Vec<Track<Vec3>>,
}

// Modelin GLTF parse anında kaydedilecek Orijinal Hiyerarşisi
#[derive(Clone, Debug)]
pub struct SkeletonJoint {
    pub name: String,
    pub node_index: usize, // GLTF node index'ini tutmaliyiz ki animasyon track'i dogru kemigi bulabilsin
    pub inverse_bind_matrix: Mat4,
    pub parent_index: Option<usize>,
    pub local_bind_transform: Mat4,
    pub bind_translation: Vec3,
    pub bind_rotation: Quat,
    pub bind_scale: Vec3,
}

#[derive(Clone, Debug)]
pub struct SkeletonHierarchy {
    pub joints: Vec<SkeletonJoint>,
    /// Armature (iskelet kök düğümü) transform'u.
    /// GLTF'te kemikler genellikle bir "Armature" düğümünün çocuklarıdır.
    /// inverse_bind_matrix bu Armature transform'unu içerir, bu yüzden
    /// global matris hesaplarken kök kemiklere bu transform'u uygulamalıyız.
    pub root_transform: Mat4,
}

impl SkeletonHierarchy {
    pub fn calculate_global_matrices(&self, local_poses: &[Mat4]) -> Vec<Mat4> {
        let mut globals: Vec<Option<Mat4>> = vec![None; self.joints.len()];

        // İteratif BFS / Topological Sıralama (Derin iskeletlerde Stack Overflow'u önler - O(N))
        let mut children_map = vec![vec![]; self.joints.len()];
        let mut roots = Vec::new();

        for (i, joint) in self.joints.iter().enumerate() {
            if let Some(parent_idx) = joint.parent_index {
                children_map[parent_idx].push(i);
            } else {
                roots.push(i);
            }
        }

        let mut queue = roots;
        while let Some(node) = queue.pop() {
            let local_mat = local_poses[node];
            let global_mat = if let Some(parent_idx) = self.joints[node].parent_index {
                globals[parent_idx].unwrap() * local_mat
            } else {
                // Kök kemikler için Armature transform'unu uygula
                self.root_transform * local_mat
            };
            globals[node] = Some(global_mat);

            for &child in &children_map[node] {
                queue.push(child);
            }
        }

        debug_assert!(
            globals.iter().all(|g| g.is_some()),
            "SkeletonHierarchy: Bazı joint'lere ulaşılamadı! Dairesel bağımlılık veya kopuk hiyerarşi olabilir."
        );
        globals.into_iter().map(|m| m.unwrap_or(Mat4::IDENTITY)).collect()
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

    // ── Skeleton Hierarchy Tests ──────────────────────────────────────

    fn make_joint(name: &str, idx: usize, parent: Option<usize>) -> SkeletonJoint {
        SkeletonJoint {
            name: name.into(),
            node_index: idx,
            inverse_bind_matrix: Mat4::IDENTITY,
            parent_index: parent,
            local_bind_transform: Mat4::IDENTITY,
            bind_translation: Vec3::ZERO,
            bind_rotation: Quat::IDENTITY,
            bind_scale: Vec3::ONE,
        }
    }

    #[test]
    fn test_skeleton_single_root() {
        let hierarchy = SkeletonHierarchy {
            joints: vec![make_joint("root", 0, None)],
            root_transform: Mat4::IDENTITY,
        };
        let local_poses = vec![Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))];
        let globals = hierarchy.calculate_global_matrices(&local_poses);
        assert_eq!(globals.len(), 1);
        let pos = Vec3::new(globals[0].w_axis.x, globals[0].w_axis.y, globals[0].w_axis.z);
        assert!((pos - Vec3::new(1.0, 2.0, 3.0)).length() < 0.001);
    }

    #[test]
    fn test_skeleton_chain_propagation() {
        let hierarchy = SkeletonHierarchy {
            joints: vec![
                make_joint("root", 0, None),
                make_joint("child", 1, Some(0)),
            ],
            root_transform: Mat4::IDENTITY,
        };
        let local_poses = vec![
            Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0)),
            Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        ];
        let globals = hierarchy.calculate_global_matrices(&local_poses);
        let child_pos = Vec3::new(globals[1].w_axis.x, globals[1].w_axis.y, globals[1].w_axis.z);
        assert!((child_pos - Vec3::new(1.0, 2.0, 0.0)).length() < 0.001,
            "Child global = root + child local");
    }

    #[test]
    fn test_skeleton_root_transform_applied() {
        let hierarchy = SkeletonHierarchy {
            joints: vec![make_joint("root", 0, None)],
            root_transform: Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)),
        };
        let local_poses = vec![Mat4::from_translation(Vec3::new(0.0, 5.0, 0.0))];
        let globals = hierarchy.calculate_global_matrices(&local_poses);
        let pos = Vec3::new(globals[0].w_axis.x, globals[0].w_axis.y, globals[0].w_axis.z);
        assert!((pos - Vec3::new(10.0, 5.0, 0.0)).length() < 0.001,
            "Root transform should be applied to root joints");
    }

    #[test]
    fn test_skeleton_branching() {
        let hierarchy = SkeletonHierarchy {
            joints: vec![
                make_joint("root", 0, None),
                make_joint("left", 1, Some(0)),
                make_joint("right", 2, Some(0)),
            ],
            root_transform: Mat4::IDENTITY,
        };
        let local_poses = vec![
            Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)),
            Mat4::from_translation(Vec3::new(-1.0, 0.0, 0.0)),
            Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0)),
        ];
        let globals = hierarchy.calculate_global_matrices(&local_poses);
        let left_pos = Vec3::new(globals[1].w_axis.x, globals[1].w_axis.y, globals[1].w_axis.z);
        let right_pos = Vec3::new(globals[2].w_axis.x, globals[2].w_axis.y, globals[2].w_axis.z);
        assert!((left_pos - Vec3::new(-1.0, 1.0, 0.0)).length() < 0.001);
        assert!((right_pos - Vec3::new(1.0, 1.0, 0.0)).length() < 0.001);
    }

    #[test]
    fn test_skeleton_deep_chain() {
        // 5-joint zinciri: her biri X'te +1
        let hierarchy = SkeletonHierarchy {
            joints: (0..5).map(|i| make_joint(&format!("j{i}"), i, if i == 0 { None } else { Some(i-1) })).collect(),
            root_transform: Mat4::IDENTITY,
        };
        let local_poses: Vec<Mat4> = (0..5).map(|_| Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0))).collect();
        let globals = hierarchy.calculate_global_matrices(&local_poses);
        let tip_x = globals[4].w_axis.x;
        assert!((tip_x - 5.0).abs() < 0.001, "5 joint zinciri, tip X=5.0 olmalı, got {tip_x}");
    }
}
