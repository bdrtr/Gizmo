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
    // The cycle guard. This walks `node_parents` UPWARD, and its only other exits are reaching a
    // bone of this skin or a node with no parent entry — so a parent loop among non-joint nodes
    // never left it. Unlike the descending walks elsewhere in the engine this one also GROWS:
    // `ancestor_transforms` gains a `Mat4` every step, so the failure is 64 bytes per iteration
    // until the allocator gives up, not a spin at flat memory. `node_parents` is built from every
    // node's child list (`loaders/mod.rs`), and nothing on that path rejects a cyclic node graph
    // — `gltf` 1.4.1 validates index bounds and vocabulary only. `gizmo-animation`'s
    // `SkeletonHierarchy::calculate_global_matrices` already models the right answer for the
    // equivalent bone-level walk: bound it, and degrade a loop to identity rather than spin.
    // Unguarded until 2026-08-31.
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::from([current_idx]);

    while let Some(&parent_idx) = node_parents.get(&current_idx) {
        // Stop when we reach another bone — its transform is already baked
        // into the skeleton hierarchy.
        if node_to_bone.contains_key(&parent_idx) {
            break;
        }
        // Back where we started: the chain loops, so there is no root above it. Stopping here
        // keeps the ancestors gathered so far, which is the same degradation the animation
        // crate's walk chose — a partial transform, not a refusal to load the file.
        if !seen.insert(parent_idx) {
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


#[cfg(test)]
mod tests {
    use super::*;

    /// The armature-root walk climbs `node_parents` and must not follow a loop.
    ///
    /// Unlike the descending walks elsewhere in the engine this one GROWS as it spins —
    /// `ancestor_transforms` gains a `Mat4` per step — so the failure is unbounded allocation,
    /// not a spin at flat memory. `node_parents` is built from every node's child list, and
    /// nothing on that path rejects a cyclic node graph.
    ///
    /// A loop is degraded to the ancestors gathered so far rather than refused, which is the
    /// same choice `gizmo-animation`'s `SkeletonHierarchy::calculate_global_matrices` documents
    /// for the equivalent bone-level walk: a partial transform, not a file that will not load.
    #[test]
    fn a_node_parent_cycle_above_a_joint_terminates() {
        // Node 0 is the skin's joint; 1 and 2 are non-joint ancestors that name each other, so
        // climbing from 0 reaches 1, then 2, then 1 again.
        let json = r#"{
          "asset": { "version": "2.0" },
          "nodes": [
            { "translation": [1.0, 0.0, 0.0] },
            { "translation": [0.0, 1.0, 0.0], "children": [0] },
            { "translation": [0.0, 0.0, 1.0], "children": [1] }
          ],
          "skins": [ { "joints": [0] } ]
        }"#;
        let doc = gltf::Gltf::from_slice(json.as_bytes()).expect("the fixture parses");
        let skin = doc.skins().next().expect("one skin");
        let nodes_by_index: Vec<gltf::Node> = doc.nodes().collect();

        // Built the way `load_gltf_from_import` builds it, plus the edge that closes the loop:
        // 1's parent is 2, and 2's parent is 1.
        let node_parents: std::collections::HashMap<usize, usize> =
            [(0, 1), (1, 2), (2, 1)].into_iter().collect();
        // No node is a bone of this skin except the joint itself, so the "reached a bone" exit
        // cannot fire and the loop is the only thing left to stop the walk.
        let node_to_bone: std::collections::HashMap<usize, usize> =
            [(0usize, 0usize)].into_iter().collect();

        let m = compute_armature_root_transform(&skin, &node_parents, &node_to_bone, &nodes_by_index);

        // Terminating at all is the assertion. The value is the evidence that it gathered the
        // two real ancestors before the back-edge stopped it rather than bailing out at once:
        // node 2's translation applied above node 1's.
        let expected = gizmo_math::Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0))
            * gizmo_math::Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0));
        assert!(
            (m.to_cols_array()[13] - expected.to_cols_array()[13]).abs() < 1e-6
                && (m.to_cols_array()[14] - expected.to_cols_array()[14]).abs() < 1e-6,
            "expected the two ancestors composed, got {m:?}"
        );
    }
}
