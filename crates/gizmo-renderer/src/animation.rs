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
}

#[derive(Clone, Debug)]
pub struct SkeletonHierarchy {
    pub joints: Vec<SkeletonJoint>,
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
                local_mat
            };
            globals[node] = Some(global_mat);

            for &child in &children_map[node] {
                queue.push(child);
            }
        }

        globals.into_iter().map(|m| m.unwrap()).collect()
    }
}
