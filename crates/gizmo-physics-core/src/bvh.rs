//! A static bounding-volume hierarchy over a triangle mesh.
//!
//! [`BvhTree::build`] partitions a triangle soup with a binned surface-area
//! heuristic and stores the result as one flat array of [`BvhNode`]s. It is
//! *static*: there is no insert or remove, so a mesh whose vertices move needs a
//! fresh build. The incremental counterpart, for whole bodies moving around a
//! scene, is [`DynamicAabbTree`](crate::broadphase::DynamicAabbTree).
//!
//! Two things to keep straight. Node bounds are in the same space as the vertex
//! slice handed to `build` — the mesh's own local space, since `build` applies no
//! transform. And triangle numbers reported out of this module address the index
//! slice **as `build` permuted it**, not the caller's original ordering.

use gizmo_math::{Aabb, Vec3, Vec3A};

/// One node of a [`BvhTree`]: either a leaf owning a contiguous run of triangles,
/// or an internal node owning exactly two children.
///
/// Which it is comes from [`is_leaf`](Self::is_leaf). The distinction is not
/// cosmetic: [`first_tri_index`](Self::first_tri_index) and
/// [`tri_count`](Self::tri_count) only describe a real triangle range on a leaf,
/// because subdivision zeroes `tri_count` when it turns a node into a parent.
#[derive(Debug, Clone, PartialEq)]
pub struct BvhNode {
    /// Bounds of every triangle beneath this node, in the same space as the
    /// `vertices` slice given to [`BvhTree::build`].
    ///
    /// Measured while the node still owned its whole triangle range, so it
    /// encloses the subtree on internal nodes as well as on leaves. It is a
    /// bound on the triangles themselves, not a fattened box — nothing is added
    /// for motion, which is why this tree suits static geometry.
    pub aabb: Aabb,
    /// Index of the left child in [`BvhTree::nodes`], or `-1` on a leaf.
    ///
    /// Children are appended as a pair, so on an internal node
    /// `right_child == left_child + 1` and both are greater than this node's own
    /// index — a top-down traversal never has to look backwards.
    pub left_child: i32,
    /// Index of the right child in [`BvhTree::nodes`], or `-1` on a leaf; see
    /// [`left_child`](Self::left_child).
    ///
    /// `-1` is the only negative value a well-formed tree ever stores here, and
    /// traversals treat any negative index as "no child" rather than trusting it.
    pub right_child: i32,
    /// Index of this node's first triangle, counted **in triangles** — multiply by
    /// 3 to reach the index slice.
    ///
    /// It addresses the slice after [`BvhTree::build`] permuted it. Internal nodes
    /// keep whatever value they had before they were split, so this is only
    /// meaningful together with a non-zero [`tri_count`](Self::tri_count).
    pub first_tri_index: u32,
    /// How many triangles this node owns — **zero on an internal node**, so it is
    /// not a subtree total.
    ///
    /// Leaves are not capped at any particular size: subdivision stops at two
    /// triangles or fewer, at a fixed depth limit, or as soon as the surface-area
    /// heuristic prefers leaving the node whole, so a leaf over a degenerate
    /// cluster of triangles can be large. Nor is a leaf guaranteed non-empty — an
    /// index slice shorter than one full triangle yields a single leaf owning
    /// zero.
    pub tri_count: u32,
}

impl BvhNode {
    /// Whether this node stores triangles rather than children — i.e. whether
    /// [`left_child`](Self::left_child) is `-1`.
    pub fn is_leaf(&self) -> bool {
        self.left_child == -1
    }
}

/// An immutable BVH over one triangle mesh: [`build`](Self::build) fills it and
/// nothing mutates it afterwards.
///
/// `Default` (and a build over zero indices) gives the empty tree, which answers
/// every query with nothing rather than failing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BvhTree {
    /// Every node, root first — a non-empty tree is always rooted at `nodes[0]`.
    ///
    /// Child links are indices into this same `Vec`, and a node's children always
    /// sit at a higher index than the node itself. The vector is empty for
    /// `Default` and for a [`build`](Self::build) over an empty index slice; any
    /// other input produces at least a root node.
    pub nodes: Vec<BvhNode>,
}

/// Triangle count for `index_count` indices, or `None` when it exceeds `u32::MAX`.
///
/// `tri_count` / `first_tri_index` are `u32`; an index count above `u32::MAX`
/// would silently truncate when cast and corrupt the BVH, so the build must
/// reject it instead.
fn checked_tri_count(index_count: usize) -> Option<u32> {
    if index_count > u32::MAX as usize {
        None
    } else {
        Some((index_count / 3) as u32)
    }
}

impl BvhTree {
    /// Build a hierarchy over `indices`, read as consecutive triples of positions
    /// into `vertices`.
    ///
    /// `indices` is **reordered in place**: after the call the caller's array is a
    /// permutation of what it passed, arranged so each node's triangles are
    /// contiguous. Every triangle number this crate reports afterwards
    /// ([`BvhNode::first_tri_index`], [`BvhTree::query_aabb`]) refers to the
    /// permuted array, so a caller that keeps per-triangle side data must permute
    /// it the same way or key it by vertex instead. `vertices` is only read, and
    /// never transformed, so all node bounds come out in the mesh's own space.
    ///
    /// Splits are chosen by a surface-area heuristic over 8 centroid bins per axis.
    /// A node stops splitting at two triangles or fewer, at a fixed depth limit,
    /// or when no candidate split scores better than leaving the node intact.
    ///
    /// # Errors
    ///
    /// [`GizmoError::BvhBuildFailed`](crate::error::GizmoError::BvhBuildFailed) if any
    /// entry of `indices` is not a valid position in `vertices`, or if
    /// `indices.len()` exceeds `u32::MAX` (the node counters are `u32` and would
    /// truncate silently). Both are checked before any node is allocated, so bad
    /// input costs an error rather than a panic or a corrupt tree.
    ///
    /// # Degenerate input
    ///
    /// Empty `indices` is not an error: it returns the empty tree. A length that is
    /// not a multiple of three is not an error either — the trailing one or two
    /// indices simply fall outside every leaf and are never visited again.
    ///
    /// The build hashes nothing, reads no clock and runs on one thread, so the same
    /// inputs yield the same tree and the same permutation of `indices` on the same
    /// platform (this crate makes no cross-platform bit-equality claim).
    #[tracing::instrument(
        skip_all,
        name = "bvh_build",
        fields(vertex_count = vertices.len(), index_count = indices.len())
    )]
    pub fn build(vertices: &[Vec3], indices: &mut [u32]) -> Result<Self, crate::error::GizmoError> {
        if indices.is_empty() {
            return Ok(Self::default());
        }

        // Reject meshes so large that the triangle count would overflow u32 (the type used
        // for `tri_count` / `first_tri_index`), which would silently truncate and corrupt the
        // BVH allocation and bounds computation.
        let tri_count =
            checked_tri_count(indices.len()).ok_or(crate::error::GizmoError::BvhBuildFailed)?;

        // Validate indices to prevent out of bounds panics
        for &idx in indices.iter() {
            if idx as usize >= vertices.len() {
                return Err(crate::error::GizmoError::BvhBuildFailed);
            }
        }
        let mut nodes = Vec::with_capacity((tri_count * 2) as usize);

        // Root node
        nodes.push(BvhNode {
            aabb: Aabb::empty(),
            left_child: -1,
            right_child: -1,
            first_tri_index: 0,
            tri_count,
        });

        let mut tree = Self { nodes };
        tree.update_node_bounds(0, vertices, indices);
        tree.subdivide(0, vertices, indices, 0);
        tracing::debug!(
            triangle_count = tri_count,
            node_count = tree.nodes.len(),
            "BVH built"
        );
        Ok(tree)
    }

    fn update_node_bounds(&mut self, node_idx: usize, vertices: &[Vec3], indices: &[u32]) {
        let node = &mut self.nodes[node_idx];
        let mut aabb = Aabb::empty();
        let start = (node.first_tri_index * 3) as usize;
        let end = start + (node.tri_count * 3) as usize;

        for i in start..end {
            aabb.extend(Vec3A::from(vertices[indices[i] as usize]));
        }
        node.aabb = aabb;
    }

    // Helper functions removed, using native Aabb methods.

    fn subdivide(&mut self, node_idx: usize, vertices: &[Vec3], indices: &mut [u32], depth: u32) {
        let first_tri_index;
        let tri_count;
        let aabb;
        {
            let node = &self.nodes[node_idx];
            first_tri_index = node.first_tri_index;
            tri_count = node.tri_count;
            aabb = node.aabb;
        }

        // Stop subdividing if we have very few triangles or reached max depth
        if tri_count <= 2 || depth > 64 {
            return;
        }

        let mut best_axis = -1;
        let mut best_split_pos = 0.0;
        let mut best_cost = f32::MAX;

        const BINS: usize = 8;

        let start = (first_tri_index * 3) as usize;
        let end = start + (tri_count * 3) as usize;

        let parent_area = aabb.surface_area();
        const C_TRAV: f32 = 1.0;
        const C_ISECT: f32 = 1.0;
        let current_cost = (tri_count as f32) * parent_area * C_ISECT;

        for axis in 0..3 {
            let mut bin_count = [0; BINS];
            let mut bin_bounds = [Aabb::empty(); BINS];

            // Find min/max centroid on this axis
            let mut min_centroid = f32::MAX;
            let mut max_centroid = f32::MIN;

            for i in (start..end).step_by(3) {
                let v0 = vertices[indices[i] as usize];
                let v1 = vertices[indices[i + 1] as usize];
                let v2 = vertices[indices[i + 2] as usize];
                let centroid = (v0[axis] + v1[axis] + v2[axis]) / 3.0;
                min_centroid = min_centroid.min(centroid);
                max_centroid = max_centroid.max(centroid);
            }

            if min_centroid == max_centroid {
                continue;
            }

            let scale = BINS as f32 / (max_centroid - min_centroid);

            for i in (start..end).step_by(3) {
                let v0 = vertices[indices[i] as usize];
                let v1 = vertices[indices[i + 1] as usize];
                let v2 = vertices[indices[i + 2] as usize];

                let centroid = (v0[axis] + v1[axis] + v2[axis]) / 3.0;
                let mut bin_idx = ((centroid - min_centroid) * scale) as usize;
                if bin_idx == BINS {
                    bin_idx = BINS - 1;
                }

                bin_count[bin_idx] += 1;
                bin_bounds[bin_idx].extend(Vec3A::from(v0));
                bin_bounds[bin_idx].extend(Vec3A::from(v1));
                bin_bounds[bin_idx].extend(Vec3A::from(v2));
            }

            // Evaluate split costs
            let mut left_area = [0.0; BINS - 1];
            let mut left_count = [0; BINS - 1];

            let mut left_box = Aabb::empty();
            let mut left_sum = 0;
            for i in 0..BINS - 1 {
                left_sum += bin_count[i];
                left_count[i] = left_sum;
                left_box = left_box.merge(bin_bounds[i]);
                left_area[i] = left_box.surface_area();
            }

            let mut right_box = Aabb::empty();
            let mut right_sum = 0;
            for i in (1..BINS).rev() {
                right_sum += bin_count[i];
                right_box = right_box.merge(bin_bounds[i]);
                let right_area = right_box.surface_area();

                let cost = C_TRAV * parent_area
                    + (left_count[i - 1] as f32 * left_area[i - 1] + right_sum as f32 * right_area)
                        * C_ISECT;
                if cost < best_cost {
                    best_cost = cost;
                    best_axis = axis as i32;
                    best_split_pos =
                        min_centroid + (i as f32) * (max_centroid - min_centroid) / (BINS as f32);
                }
            }
        }

        if best_axis == -1 || best_cost >= current_cost {
            return;
        }

        let best_axis = best_axis as usize;

        let mut i = first_tri_index as usize;
        let mut j = (first_tri_index + tri_count - 1) as usize;

        while i <= j {
            let i_idx = i * 3;
            let v0 = vertices[indices[i_idx] as usize];
            let v1 = vertices[indices[i_idx + 1] as usize];
            let v2 = vertices[indices[i_idx + 2] as usize];
            let centroid = (v0[best_axis] + v1[best_axis] + v2[best_axis]) / 3.0;

            if centroid < best_split_pos {
                i += 1;
            } else {
                // swap triangle i and j
                let j_idx = j * 3;
                indices.swap(i_idx, j_idx);
                indices.swap(i_idx + 1, j_idx + 1);
                indices.swap(i_idx + 2, j_idx + 2);
                if j == 0 {
                    break;
                }
                j -= 1;
            }
        }

        let left_count = (i as u32) - first_tri_index;
        if left_count == 0 || left_count == tri_count {
            return;
        }

        let left_child_idx = self.nodes.len();
        let right_child_idx = left_child_idx + 1;

        self.nodes.push(BvhNode {
            aabb: Aabb::empty(),
            left_child: -1,
            right_child: -1,
            first_tri_index,
            tri_count: left_count,
        });

        self.nodes.push(BvhNode {
            aabb: Aabb::empty(),
            left_child: -1,
            right_child: -1,
            first_tri_index: i as u32,
            tri_count: tri_count - left_count,
        });

        self.nodes[node_idx].left_child = left_child_idx as i32;
        self.nodes[node_idx].right_child = right_child_idx as i32;
        self.nodes[node_idx].tri_count = 0; // Internal node

        self.update_node_bounds(left_child_idx, vertices, indices);
        self.update_node_bounds(right_child_idx, vertices, indices);

        self.subdivide(left_child_idx, vertices, indices, depth + 1);
        self.subdivide(right_child_idx, vertices, indices, depth + 1);
    }

    /// Every triangle whose leaf bounds overlap `query`, as **triangle indices** — multiply by 3
    /// to reach the mesh's index array.
    ///
    /// Appends rather than returning, so a caller running this per pair per frame reuses one
    /// buffer. Results are in tree order, not sorted, and a triangle appears once.
    ///
    /// This is the query a *concave* mesh needs. Support-point algorithms (GJK/EPA) walk the same
    /// tree to find the farthest vertex in a direction, which is a **convex hull** support
    /// function — correct for a hull and wrong for a mesh with a dent in it. Collecting the
    /// candidate triangles and testing each one separately is what makes a valley a valley
    /// instead of the lid over it.
    pub fn query_aabb(&self, query: Aabb, out: &mut Vec<u32>) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = Vec::with_capacity(32);
        stack.push(0usize);
        while let Some(i) = stack.pop() {
            let Some(node) = self.nodes.get(i) else { continue };
            if !node.aabb.intersects(query) {
                continue;
            }
            if node.is_leaf() {
                out.extend(node.first_tri_index..node.first_tri_index + node.tri_count);
            } else {
                // `-1` is the leaf marker; anything else is an index, and a negative one that is
                // not `-1` would be a corrupt tree rather than a leaf.
                if node.left_child >= 0 {
                    stack.push(node.left_child as usize);
                }
                if node.right_child >= 0 {
                    stack.push(node.right_child as usize);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_tri_count_rejects_u32_overflow() {
        // One index past u32::MAX must be rejected rather than truncating.
        assert_eq!(checked_tri_count(u32::MAX as usize + 1), None);
        // The boundary itself and normal sizes are accepted.
        assert_eq!(checked_tri_count(u32::MAX as usize), Some(u32::MAX / 3));
        assert_eq!(checked_tri_count(9), Some(3));
        assert_eq!(checked_tri_count(0), Some(0));
    }

    #[test]
    fn build_rejects_out_of_range_index() {
        // Index references a vertex that does not exist -> error, not a panic.
        let verts = [Vec3::ZERO, Vec3::X, Vec3::Y];
        let mut indices = [0u32, 1, 5]; // 5 is out of range
        assert!(BvhTree::build(&verts, &mut indices).is_err());
    }

    #[test]
    fn build_empty_is_ok() {
        let verts: [Vec3; 0] = [];
        let mut indices: [u32; 0] = [];
        assert!(BvhTree::build(&verts, &mut indices).is_ok());
    }
}
