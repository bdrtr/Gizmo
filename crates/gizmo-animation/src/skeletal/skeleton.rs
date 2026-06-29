use gizmo_math::{Mat4, Quat, Vec3};

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
