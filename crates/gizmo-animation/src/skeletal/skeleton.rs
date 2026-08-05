//! The skinned-mesh bone table: the immutable result of parsing a glTF skin, plus
//! the parent-chain composition that turns a per-bone *local* pose into model-space
//! matrices.
//!
//! Nothing here samples animation — [`crate::skeletal::sample::evaluate_clip`]
//! produces the local pose and this module only propagates it down the hierarchy.
//! The GPU side (uploading `global * inverse_bind_matrix` into the joint uniform)
//! stays in `gizmo-renderer`, which is why these types carry no `wgpu` dependency.

use gizmo_math::{Mat4, Quat, Vec3};

// Modelin GLTF parse anında kaydedilecek Orijinal Hiyerarşisi
/// One bone of a skinned mesh, built once when the glTF skin is parsed and never
/// mutated afterwards — the animated state lives in the caller's pose array, not
/// in here.
///
/// A bone is identified by its **position in [`SkeletonHierarchy::joints`]**, which
/// is the skin's joint order. That index is what [`Self::parent_index`], the local /
/// global pose arrays and the GPU joint slots all refer to; it is deliberately *not*
/// the same numbering as [`Self::node_index`].
#[derive(Clone, Debug)]
pub struct SkeletonJoint {
    /// Bone name taken from the source glTF node, or the literal `"bone"` when that
    /// node was unnamed — so this is **not** a unique key, and two joints can collide
    /// on it.
    ///
    /// Read by `evaluate_clip` to retarget a clip authored against another rig: an
    /// exact comparison first, then a loosened one that lowercases and strips the
    /// `/RootNode/`, `mixamorig:` and `mixamorig_` decorations exporters add.
    pub name: String,
    /// Index of the glTF *document node* this bone was built from — a document-global
    /// node number, unrelated to this joint's position in
    /// [`SkeletonHierarchy::joints`].
    ///
    /// Animation channels address their target by node, so this is how a nameless (or
    /// name-mismatched) track finds its bone. Storing it is load-bearing: the sampler
    /// once used a channel's node index directly as a bone index, which silently
    /// mis-targeted or dropped tracks in any file whose armature is not the first
    /// nodes of the document — the usual Blender export layout.
    pub node_index: usize, // GLTF node index'ini tutmaliyiz ki animasyon track'i dogru kemigi bulabilsin
    /// Maps model space into this bone's local bind space; read straight from the
    /// skin's `inverseBindMatrices`, or `IDENTITY` when the file supplies fewer
    /// matrices than joints (truncated data is tolerated rather than rejected).
    ///
    /// The skinning matrix handed to the GPU is
    /// `calculate_global_matrices()[i] * joints[i].inverse_bind_matrix`. Exporters
    /// bake the armature's own transform into this matrix, so the global side has to
    /// re-apply it to cancel it back out — that is what
    /// [`SkeletonHierarchy::root_transform`] exists for.
    pub inverse_bind_matrix: Mat4,
    /// Index into [`SkeletonHierarchy::joints`] of this bone's parent; `None` for a
    /// root bone. The loader also leaves it `None` when the parent *node* is not
    /// itself a joint of this skin.
    ///
    /// Not validated on construction: it is a plain `usize` any safe code can put out
    /// of range. [`SkeletonHierarchy::calculate_global_matrices`] therefore
    /// bounds-checks it and demotes an out-of-range parent to a root; before that
    /// check existed, a hand-built hierarchy panicked mid-frame.
    pub parent_index: Option<usize>,
    /// The bone's rest transform relative to its parent, precomposed as `T * R * S`
    /// from the three `bind_*` fields below (metres / unit quaternion / scale factors).
    ///
    /// Redundant on purpose. The renderer seeds a freshly created skeleton's
    /// local-pose array from this field so the mesh stands in bind pose before any
    /// clip plays, and pushing exactly these through
    /// [`SkeletonHierarchy::calculate_global_matrices`] is what makes the resulting
    /// skinning matrices come out identity.
    pub local_bind_transform: Mat4,
    /// Rest translation in metres, expressed in the parent bone's space.
    ///
    /// Also the per-frame fallback when a clip has no translation sample for this
    /// bone — which is nearly always: the sampler applies animated translation only to
    /// the root-motion bone (whose name contains `"Hips"`) and drops it for every
    /// other bone. For the rest of the rig this value therefore *is* the bone offset
    /// for the entire clip.
    pub bind_translation: Vec3,
    /// Rest orientation as a unit quaternion in the parent bone's space, and the
    /// fallback when no rotation sample lands on this bone.
    ///
    /// Rotation is the channel that actually carries skeletal animation, so a failed
    /// retarget surfaces here as a bone frozen at rest rather than as a visible
    /// glitch — check the `missed_targets` count `evaluate_clip` logs.
    pub bind_rotation: Quat,
    /// Rest scale factors per axis (dimensionless; `ONE` on a normal rig), and the
    /// fallback when no scale sample lands on this bone.
    ///
    /// This used to be the *only* reachable value: the sampler read scale tracks and
    /// discarded them, so squash/stretch and breathing animation never made it to the
    /// skeleton. Scale tracks now override it.
    pub bind_scale: Vec3,
}

/// One complete skin: every bone of a skeleton plus the armature transform sitting
/// above its root bones.
///
/// Immutable after load and shared behind an `Arc` — the renderer's `Skeleton`
/// component clones the handle, never the table, so one hierarchy can drive many
/// instances. Pose data (one local matrix per bone) belongs to the caller and is
/// passed in per frame; nothing about the current animation is stored here.
#[derive(Clone, Debug)]
pub struct SkeletonHierarchy {
    /// Bones in glTF skin order. An index into this vector is *the* bone index — the
    /// same numbering used by [`SkeletonJoint::parent_index`], by the local/global
    /// pose arrays, and by the GPU joint slots.
    ///
    /// The order is **not** topologically sorted: a parent may appear after its own
    /// child, which is exactly why [`Self::calculate_global_matrices`] runs a
    /// traversal instead of a single forward sweep. The renderer's joint uniform
    /// holds 128 matrices, so bones from index 128 up are parsed and posed here but
    /// never skinned on the GPU.
    pub joints: Vec<SkeletonJoint>,
    /// Transform of the armature — the skeleton's root node — applied to every root
    /// bone by [`Self::calculate_global_matrices`].
    ///
    /// In glTF the bones are usually children of an "Armature" node, and the exporter
    /// bakes that node's transform into every
    /// [`SkeletonJoint::inverse_bind_matrix`]. Re-applying it on the global side is
    /// what cancels it back out, so a skeleton posed at its bind transforms yields
    /// identity skinning matrices. Leaving this at `IDENTITY` for a file that does
    /// have an armature transform shows up as a mesh that jumps to the wrong
    /// scale/orientation the instant skinning takes over.
    ///
    /// The loader builds it by walking up from the first joint and accumulating the
    /// transforms of every non-joint ancestor node.
    pub root_transform: Mat4,
}

impl SkeletonHierarchy {
    /// Composes each bone's local pose up its parent chain, returning one model-space
    /// matrix per bone, indexed exactly like [`Self::joints`].
    ///
    /// `local_poses` is indexed by **bone index** and must hold at least one entry per
    /// joint; a shorter slice panics (the renderer's `Skeleton::new` asserts equal
    /// lengths up front), and surplus entries are ignored. Root bones are
    /// pre-multiplied by [`Self::root_transform`], every other bone by its parent's
    /// already-computed global matrix — so feeding each joint's
    /// [`SkeletonJoint::local_bind_transform`] reproduces the bind pose. Callers turn
    /// the result into skinning matrices with
    /// `global[i] * joints[i].inverse_bind_matrix`.
    ///
    /// The walk is iterative over an explicit stack rather than recursive: deep rigs
    /// (long spine, tail or hair chains) would otherwise risk overflowing the native
    /// stack. It is O(N) and cannot spin forever — a bone has at most one parent, so
    /// it lands in at most one child list and is visited at most once. Visit order is
    /// LIFO, i.e. depth-first; only the parent-before-child guarantee matters, and
    /// that holds because a bone is pushed solely after its parent has been popped
    /// and written.
    ///
    /// Malformed hierarchies degrade instead of aborting the frame:
    ///
    /// * A `parent_index` past the end of `joints` is treated as a root. This was an
    ///   out-of-bounds panic until the bounds check was added, and it is reachable
    ///   from safe code since the field is a plain `usize`.
    /// * A bone unreachable from any root — a parent cycle, e.g. two bones naming
    ///   each other — is never visited and comes back as `Mat4::IDENTITY`, i.e. posed
    ///   as though it sat unrotated at the model origin, ignoring both its parents and
    ///   `root_transform`. A `debug_assert!` names that case in debug builds; release
    ///   builds render the deformed result rather than failing.
    ///
    /// Allocates a fresh child-index map and result vector on every call, and the
    /// renderer calls it once per skinned entity per frame.
    pub fn calculate_global_matrices(&self, local_poses: &[Mat4]) -> Vec<Mat4> {
        let mut globals: Vec<Option<Mat4>> = vec![None; self.joints.len()];

        // İteratif BFS / Topological Sıralama (Derin iskeletlerde Stack Overflow'u önler - O(N))
        let mut children_map = vec![vec![]; self.joints.len()];
        let mut roots = Vec::new();

        for (i, joint) in self.joints.iter().enumerate() {
            match joint.parent_index {
                // Bounds-check the parent index: a bogus parent_index (>= joints.len())
                // must not panic on the children_map index. Treat such joints as roots
                // so the pose is still produced instead of aborting the frame.
                Some(parent_idx) if parent_idx < self.joints.len() => {
                    children_map[parent_idx].push(i);
                }
                Some(_parent_idx) => {
                    // Out-of-range parent: treat as a root rather than panicking.
                    roots.push(i);
                }
                None => {
                    roots.push(i);
                }
            }
        }

        let mut queue = roots;
        while let Some(node) = queue.pop() {
            let local_mat = local_poses[node];
            let global_mat = if let Some(parent_idx) = self.joints[node]
                .parent_index
                .filter(|&p| p < self.joints.len())
            {
                // Parent is guaranteed processed before its child by the BFS order;
                // fall back to identity if that invariant is somehow violated.
                globals[parent_idx].unwrap_or(Mat4::IDENTITY) * local_mat
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
    fn test_skeleton_invalid_parent_index_does_not_panic() {
        // A joint with an out-of-range parent_index (safe-code constructible) must
        // not cause an out-of-bounds panic; it is treated as a root instead.
        let hierarchy = SkeletonHierarchy {
            joints: vec![
                make_joint("root", 0, None),
                make_joint("bad", 1, Some(99)), // 99 >= joints.len()
            ],
            root_transform: Mat4::IDENTITY,
        };
        let local_poses = vec![
            Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0)),
            Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        ];
        // Would panic before the bounds check was added.
        let globals = hierarchy.calculate_global_matrices(&local_poses);
        assert_eq!(globals.len(), 2);
        // The bad joint is treated as a root => root_transform (identity) * local.
        let bad_pos = Vec3::new(globals[1].w_axis.x, globals[1].w_axis.y, globals[1].w_axis.z);
        assert!((bad_pos - Vec3::new(0.0, 2.0, 0.0)).length() < 0.001);
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
