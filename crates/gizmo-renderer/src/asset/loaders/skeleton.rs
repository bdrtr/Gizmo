//! glTF skeleton/skin parsing — joint hierarchy + armature-root resolution.
//! Extracted verbatim from `loaders.rs` (pure move). Called from `load_gltf_from_import`.

use super::*;

pub(super) fn parse_skeletons(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    node_parents: &std::collections::HashMap<usize, usize>,
    nodes_by_index: &[gltf::Node],
) -> Vec<SkeletonHierarchy> {
    document
        .skins()
        .map(|skin| {
            let reader = skin.reader(|b| Some(&buffers[b.index()]));

            let identity_mat = [
                [1.0, 0., 0., 0.],
                [0., 1., 0., 0.],
                [0., 0., 1., 0.],
                [0., 0., 0., 1.],
            ];
            let ibm: Vec<[[f32; 4]; 4]> = reader
                .read_inverse_bind_matrices()
                .map(|v| v.collect())
                .unwrap_or_else(|| vec![identity_mat; skin.joints().count()]);

            // Map node_index → bone_index for O(1) parent lookups.
            let node_to_bone: std::collections::HashMap<usize, usize> = skin
                .joints()
                .enumerate()
                .map(|(bone_idx, node)| (node.index(), bone_idx))
                .collect();

            let joints: Vec<SkeletonJoint> = skin
                .joints()
                .enumerate()
                .map(|(bone_idx, joint_node)| {
                    // Fall back to IDENTITY when the glTF file has fewer
                    // inverse_bind_matrices than joints (malformed/truncated data),
                    // rather than panicking on an out-of-bounds index.
                    let inverse_bind_matrix = ibm
                        .get(bone_idx)
                        .map(gizmo_math::Mat4::from_cols_array_2d)
                        .unwrap_or(gizmo_math::Mat4::IDENTITY);

                    let parent_index = node_parents
                        .get(&joint_node.index())
                        .and_then(|p| node_to_bone.get(p).copied());

                    let (t, r, s) = joint_node.transform().decomposed();
                    let bind_translation = Vec3::new(t[0], t[1], t[2]);
                    let bind_rotation = Quat::from_array(r);
                    let bind_scale = Vec3::new(s[0], s[1], s[2]);

                    let local_bind_transform = gizmo_math::Mat4::from_translation(bind_translation)
                        * gizmo_math::Mat4::from_quat(bind_rotation)
                        * gizmo_math::Mat4::from_scale(bind_scale);

                    SkeletonJoint {
                        name: joint_node.name().unwrap_or("bone").to_string(),
                        node_index: joint_node.index(),
                        inverse_bind_matrix,
                        parent_index,
                        local_bind_transform,
                        bind_translation,
                        bind_rotation,
                        bind_scale,
                    }
                })
                .collect();

            // Compute the combined transform of all non-joint ancestor nodes
            // (the "armature" transform).  `calculate_global_matrices` relies
            // on this so that joint matrices are identity in the bind pose.
            //
            // We use `nodes_by_index` for O(1) node lookup instead of O(n) `.nth()`.
            let root_transform =
                compute_armature_root_transform(&skin, node_parents, &node_to_bone, nodes_by_index);

            SkeletonHierarchy {
                joints,
                root_transform,
            }
        })
        .collect()
}

/// Walk the parent chain of the first joint upward until we hit a joint or the
/// root, accumulating the transforms of all non-joint ancestors.
fn compute_armature_root_transform(
    skin: &gltf::Skin,
    node_parents: &std::collections::HashMap<usize, usize>,
    node_to_bone: &std::collections::HashMap<usize, usize>,
    nodes_by_index: &[gltf::Node],
) -> gizmo_math::Mat4 {
    let mut root_transform = gizmo_math::Mat4::IDENTITY;

    let first_joint = match skin.joints().next() {
        Some(j) => j,
        None => return root_transform,
    };

    let mut current_idx = first_joint.index();
    let mut ancestor_transforms: Vec<gizmo_math::Mat4> = Vec::new();

    while let Some(&parent_idx) = node_parents.get(&current_idx) {
        // Stop when we reach another bone — its transform is already baked
        // into the skeleton hierarchy.
        if node_to_bone.contains_key(&parent_idx) {
            break;
        }

        if let Some(parent_node) = nodes_by_index.get(parent_idx) {
            let (t, r, s) = parent_node.transform().decomposed();
            let mat = gizmo_math::Mat4::from_translation(Vec3::new(t[0], t[1], t[2]))
                * gizmo_math::Mat4::from_quat(Quat::from_array(r))
                * gizmo_math::Mat4::from_scale(Vec3::new(s[0], s[1], s[2]));
            ancestor_transforms.push(mat);
        }

        current_idx = parent_idx;
    }

    // Apply transforms from root downward (reverse of collection order).
    for mat in ancestor_transforms.into_iter().rev() {
        root_transform *= mat;
    }

    root_transform
}
