use gizmo_math::{Aabb, Vec3, Vec3A};

#[derive(Debug, Clone, PartialEq)]
pub struct BvhNode {
    pub aabb: Aabb,
    pub left_child: i32,
    pub right_child: i32,
    pub first_tri_index: u32,
    pub tri_count: u32,
}

impl BvhNode {
    pub fn is_leaf(&self) -> bool {
        self.tri_count > 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BvhTree {
    pub nodes: Vec<BvhNode>,
}

impl Default for BvhTree {
    fn default() -> Self {
        Self { nodes: Vec::new() }
    }
}

impl BvhTree {
    pub fn build(vertices: &[Vec3], indices: &mut [u32]) -> Self {
        if indices.is_empty() {
            return Self::default();
        }

        let mut nodes = Vec::new();
        // Root node
        nodes.push(BvhNode {
            aabb: Aabb::empty(),
            left_child: -1,
            right_child: -1,
            first_tri_index: 0,
            tri_count: (indices.len() / 3) as u32,
        });
        
        let mut tree = Self { nodes };
        tree.update_node_bounds(0, vertices, indices);
        tree.subdivide(0, vertices, indices);
        tree
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

    fn subdivide(&mut self, node_idx: usize, vertices: &[Vec3], indices: &mut [u32]) {
        let first_tri_index;
        let tri_count;
        let aabb;
        {
            let node = &self.nodes[node_idx];
            first_tri_index = node.first_tri_index;
            tri_count = node.tri_count;
            aabb = node.aabb;
        }
        
        // Stop subdividing if we have very few triangles
        if tri_count <= 2 {
            return;
        }

        // Find longest axis of the AABB
        let extent = aabb.size();
        let axis = if extent.x > extent.y && extent.x > extent.z {
            0
        } else if extent.y > extent.z {
            1
        } else {
            2
        };

        let split_pos = aabb.center()[axis];
        
        let mut i = first_tri_index as usize;
        let mut j = (first_tri_index + tri_count - 1) as usize;
        
        while i <= j {
            let i_idx = i * 3;
            let v0 = vertices[indices[i_idx] as usize];
            let v1 = vertices[indices[i_idx + 1] as usize];
            let v2 = vertices[indices[i_idx + 2] as usize];
            let centroid = (v0 + v1 + v2) / 3.0;
            
            if centroid[axis] < split_pos {
                i += 1;
            } else {
                // swap triangle i and j
                let j_idx = j * 3;
                indices.swap(i_idx, j_idx);
                indices.swap(i_idx + 1, j_idx + 1);
                indices.swap(i_idx + 2, j_idx + 2);
                if j == 0 { break; }
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
        
        self.subdivide(left_child_idx, vertices, indices);
        self.subdivide(right_child_idx, vertices, indices);
    }
}
