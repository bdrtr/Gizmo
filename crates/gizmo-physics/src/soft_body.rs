use gizmo_math::{Mat3, Vec3};

/// Represents a single vertex/node in the FEM soft body mesh.
#[derive(Debug, Clone, Copy)]
pub struct SoftBodyNode {
    pub position: Vec3,
    pub velocity: Vec3,
    pub mass: f32,
    pub is_fixed: bool, // For pinning parts of the body (e.g. chassis mounts)
}

/// A 3D Tetrahedron (4 nodes) used as the fundamental finite element.
#[derive(Debug, Clone, Copy)]
pub struct Tetrahedron {
    /// Indices into the nodes array of the parent SoftBody
    pub node_indices: [u32; 4],
    
    /// Rest Volume (V0) - calculated once at initialization
    pub rest_volume: f32,
    
    /// The inverse of the reference shape matrix (Dm^-1)
    /// Used by the GPU to quickly calculate the Deformation Gradient (F = Ds * Dm^-1)
    pub inv_rest_matrix: Mat3,
}

impl Tetrahedron {
    /// Calculate the inverse rest matrix (Dm^-1) and rest volume for the tetrahedron.
    /// Needs the 4 rest positions (x0, x1, x2, x3) of the nodes.
    pub fn calculate_rest_data(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3) -> (Mat3, f32) {
        // Dm = [ (p1-p0) | (p2-p0) | (p3-p0) ]
        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let e3 = p3 - p0;
        
        let dm = Mat3::from_cols(e1, e2, e3);
        
        // V = 1/6 * det(Dm)
        let det = dm.determinant();
        let volume = (det / 6.0).abs();
        
        let inv_dm = dm.inverse();
        
        (inv_dm, volume)
    }
}

/// The main CPU-side structure for an FEM-based Soft Body.
/// It contains the geometry and material properties needed to upload to the GPU.
#[derive(Debug, Clone)]
pub struct SoftBodyMesh {
    pub nodes: Vec<SoftBodyNode>,
    pub elements: Vec<Tetrahedron>,
    
    // --- Material Properties (Elasto-Plasticity) ---
    /// Young's Modulus (E) - Stiffness of the material (e.g., higher = stiffer metal)
    pub youngs_modulus: f32,
    
    /// Poisson's Ratio (nu) - Incompressibility (0.0 to 0.499, e.g., 0.3 for steel, 0.49 for rubber)
    pub poissons_ratio: f32,
    
    /// Lame's first parameter (lambda) - derived
    pub lambda: f32,
    
    /// Shear Modulus (mu) - derived
    pub mu: f32,
    
    /// Damping factor to prevent infinite oscillation
    pub damping: f32,
}

pub fn resolve_node_collision(
    mut position: Vec3,
    mut velocity: Vec3,
    dt: f32,
    rigid_colliders: &[(gizmo_core::entity::Entity, crate::components::Transform, crate::components::Collider)]
) -> (Vec3, Vec3, bool) {
    let mut collided = false;
    let ray = crate::raycast::Ray::new(position, velocity.normalize_or_zero());
    let dist = velocity.length() * dt;
    
    if dist > 1e-5 {
        for (_, col_trans, col) in rigid_colliders {
            if let Some((d, n)) = crate::raycast::Raycast::ray_shape(&ray, &col.shape, col_trans) {
                if d <= dist + 0.1 {
                    let bounce = 0.5;
                    let friction = 0.8;
                    
                    let vn = velocity.dot(n);
                    if vn < 0.0 {
                        let vt = velocity - n * vn;
                        velocity = vt * (1.0 - friction) - n * (vn * bounce);
                    }
                    
                    position += ray.direction * (d - 0.1).max(0.0);
                    collided = true;
                }
            }
        }
    }
    
    (position, velocity, collided)
}

impl SoftBodyMesh {
    pub fn new(youngs_modulus: f32, poissons_ratio: f32) -> Self {
        // Calculate Lame Parameters
        let mu = youngs_modulus / (2.0 * (1.0 + poissons_ratio));
        let lambda = (youngs_modulus * poissons_ratio) / ((1.0 + poissons_ratio) * (1.0 - 2.0 * poissons_ratio));
        
        Self {
            nodes: Vec::new(),
            elements: Vec::new(),
            youngs_modulus,
            poissons_ratio,
            lambda,
            mu,
            damping: 0.99,
        }
    }
    
    pub fn add_node(&mut self, position: Vec3, mass: f32) -> u32 {
        let idx = self.nodes.len() as u32;
        self.nodes.push(SoftBodyNode {
            position,
            velocity: Vec3::ZERO,
            mass,
            is_fixed: false,
        });
        idx
    }
    
    pub fn add_element(&mut self, i0: u32, i1: u32, i2: u32, i3: u32) {
        let p0 = self.nodes[i0 as usize].position;
        let p1 = self.nodes[i1 as usize].position;
        let p2 = self.nodes[i2 as usize].position;
        let p3 = self.nodes[i3 as usize].position;
        
        let (inv_rest_matrix, rest_volume) = Tetrahedron::calculate_rest_data(p0, p1, p2, p3);
        
        self.elements.push(Tetrahedron {
            node_indices: [i0, i1, i2, i3],
            rest_volume,
            inv_rest_matrix,
        });
    }

    /// Advances the FEM simulation by one timestep using a Neo-Hookean hyperelastic model.
    pub fn step(&mut self, dt: f32, gravity: Vec3, rigid_colliders: &[(gizmo_core::entity::Entity, crate::components::Transform, crate::components::Collider)]) {
        let num_nodes = self.nodes.len();
        let mut forces: Vec<Vec3> = self.nodes.iter().map(|n| gravity * n.mass).collect();

        // 1. Calculate and accumulate internal elastic forces from all tetrahedra in PARALLEL
        use rayon::prelude::*;
        
        let positions: Vec<Vec3> = self.nodes.iter().map(|n| n.position).collect();
        const MIN_JACOBIAN: f32 = 0.1;

        let elastic_forces = self.elements.par_iter().fold(
            || vec![Vec3::ZERO; num_nodes],
            |mut acc_forces, elem| {
                let i0 = elem.node_indices[0] as usize;
                let i1 = elem.node_indices[1] as usize;
                let i2 = elem.node_indices[2] as usize;
                let i3 = elem.node_indices[3] as usize;

                let x0 = positions[i0];
                let x1 = positions[i1];
                let x2 = positions[i2];
                let x3 = positions[i3];

                let ds = Mat3::from_cols(x1 - x0, x2 - x0, x3 - x0);
                let f = ds * elem.inv_rest_matrix;
                let j = f.determinant();

                if j < MIN_JACOBIAN {
                    return acc_forces;
                }

                let f_inv_t = f.inverse().transpose();
                let ln_j = j.ln();

                let p = f_inv_t.clone() * (-self.mu) + f * self.mu + f_inv_t * (self.lambda * ln_j);
                let h = p * elem.inv_rest_matrix.transpose() * elem.rest_volume;

                let f1 = -h.col(0);
                let f2 = -h.col(1);
                let f3 = -h.col(2);
                let f0 = -(f1 + f2 + f3);

                acc_forces[i0] += f0;
                acc_forces[i1] += f1;
                acc_forces[i2] += f2;
                acc_forces[i3] += f3;
                
                acc_forces
            }
        ).reduce(
            || vec![Vec3::ZERO; num_nodes],
            |mut a, b| {
                for i in 0..num_nodes {
                    a[i] += b[i];
                }
                a
            }
        );

        for i in 0..num_nodes {
            forces[i] += elastic_forces[i];
        }

        // 2. Integrate velocities and positions
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if node.is_fixed {
                continue;
            }

            // Explicit Euler Integration
            let acceleration = forces[i] / node.mass;
            node.velocity += acceleration * dt;
            
            // Apply damping normalized by dt
            node.velocity *= self.damping.powf(dt);

            let next_pos = node.position + node.velocity * dt;
            
            let (new_pos, new_vel, collided) = resolve_node_collision(node.position, node.velocity, dt, rigid_colliders);
            
            if collided {
                node.position = new_pos;
                node.velocity = new_vel;
            } else {
                node.position = next_pos;
            }
        }
    }
}
