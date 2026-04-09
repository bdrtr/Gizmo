use gizmo_math::{Vec3, Quat, Mat4};

#[derive(Clone, Copy, Debug)]
pub struct Keyframe<T> {
    pub time: f32,
    pub value: T,
}

#[derive(Clone, Debug)]
pub struct Track<T> {
    pub target_node: usize,
    pub keyframes: Vec<Keyframe<T>>,
}

impl<T: Clone + Copy> Track<T> {
    pub fn get_interpolated(&self, time: f32, mut interpolator: impl FnMut(T, T, f32) -> T) -> Option<T> {
        if self.keyframes.is_empty() { return None; }
        if self.keyframes.len() == 1 || time <= self.keyframes[0].time { return Some(self.keyframes[0].value); }
        let last_idx = self.keyframes.len() - 1;
        if time >= self.keyframes[last_idx].time { return Some(self.keyframes[last_idx].value); }

        // Binary search ile doğru aralığı bul (O(log N) — eskiden O(N) doğrusal arama)
        let idx = self.keyframes.partition_point(|k| k.time < time);
        if idx == 0 { return Some(self.keyframes[0].value); }
        let i = idx - 1;
        let k1 = &self.keyframes[i];
        let k2 = &self.keyframes[i + 1.min(last_idx)];
        let dt = k2.time - k1.time;
        let t = if dt > 0.0 { (time - k1.time) / dt } else { 0.0 };
        Some(interpolator(k1.value, k2.value, t))
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
        
        // Recursive & Memoized fonksiyon ile derinlik öncelikli ağaç taraması
        fn compute_global(i: usize, joints: &[SkeletonJoint], local_poses: &[Mat4], globals: &mut [Option<Mat4>]) -> Mat4 {
            if let Some(mat) = globals[i] {
                return mat;
            }
            let local_mat = local_poses[i];
            let global_mat = if let Some(parent) = joints[i].parent_index {
                compute_global(parent, joints, local_poses, globals) * local_mat
            } else {
                local_mat
            };
            globals[i] = Some(global_mat);
            global_mat
        }

        for i in 0..self.joints.len() {
            compute_global(i, &self.joints, local_poses, &mut globals);
        }
        
        globals.into_iter().map(|m| m.unwrap()).collect()
    }
}
