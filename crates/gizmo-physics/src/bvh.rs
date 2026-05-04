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

    fn aabb_surface_area(aabb: &Aabb) -> f32 {
        if aabb.min.x > aabb.max.x {
            return 0.0;
        }
        let ext = Vec3::new(
            aabb.max.x - aabb.min.x,
            aabb.max.y - aabb.min.y,
            aabb.max.z - aabb.min.z,
        );
        2.0 * (ext.x * ext.y + ext.y * ext.z + ext.z * ext.x)
    }

    fn aabb_union(a: &Aabb, b: &Aabb) -> Aabb {
        if a.min.x > a.max.x { return *b; }
        if b.min.x > b.max.x { return *a; }
        Aabb {
            min: Vec3A::from(Vec3::new(
                a.min.x.min(b.min.x),
                a.min.y.min(b.min.y),
                a.min.z.min(b.min.z),
            )),
            max: Vec3A::from(Vec3::new(
                a.max.x.max(b.max.x),
                a.max.y.max(b.max.y),
                a.max.z.max(b.max.z),
            )),
        }
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

        let mut best_axis = -1;
        let mut best_split_pos = 0.0;
        let mut best_cost = f32::MAX;
        
        const BINS: usize = 8;
        
        let start = (first_tri_index * 3) as usize;
        let end = start + (tri_count * 3) as usize;
        
        let current_cost = (tri_count as f32) * Self::aabb_surface_area(&aabb);

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
                if bin_idx == BINS { bin_idx = BINS - 1; }
                
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
                left_box = Self::aabb_union(&left_box, &bin_bounds[i]);
                left_area[i] = Self::aabb_surface_area(&left_box);
            }
            
            let mut right_box = Aabb::empty();
            let mut right_sum = 0;
            for i in (1..BINS).rev() {
                right_sum += bin_count[i];
                right_box = Self::aabb_union(&right_box, &bin_bounds[i]);
                let right_area = Self::aabb_surface_area(&right_box);
                
                let cost = left_count[i - 1] as f32 * left_area[i - 1] + right_sum as f32 * right_area;
                if cost < best_cost {
                    best_cost = cost;
                    best_axis = axis as i32;
                    best_split_pos = min_centroid + (i as f32) * (max_centroid - min_centroid) / (BINS as f32);
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
