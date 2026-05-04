use gizmo_math::Vec3;

#[derive(Debug, Clone)]
pub struct ClothNode {
    pub position: Vec3,
    pub prev_position: Vec3,
    pub mass: f32,
    pub inv_mass: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct DistanceConstraint {
    pub node_a: usize,
    pub node_b: usize,
    pub rest_length: f32,
    pub compliance: f32, // Inverse stiffness
}

#[derive(Debug, Clone)]
pub struct Cloth {
    pub nodes: Vec<ClothNode>,
    pub constraints: Vec<DistanceConstraint>,
    pub thickness: f32,
    pub friction: f32,
}

impl Cloth {
    pub fn new(width: usize, height: usize, spacing: f32, mass_per_node: f32) -> Self {
        let mut nodes = Vec::with_capacity(width * height);
        let mut constraints = Vec::new();

        for y in 0..height {
            for x in 0..width {
                let position = Vec3::new(x as f32 * spacing, y as f32 * spacing, 0.0);
                nodes.push(ClothNode {
                    position,
                    prev_position: position,
                    mass: mass_per_node,
                    inv_mass: if mass_per_node > 0.0 { 1.0 / mass_per_node } else { 0.0 },
                });

                let idx = y * width + x;

                // Structural constraints
                if x > 0 {
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - 1,
                        rest_length: spacing,
                        compliance: 0.001,
                    });
                }
                if y > 0 {
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - width,
                        rest_length: spacing,
                        compliance: 0.001,
                    });
                }

                // Shear constraints
                if x > 0 && y > 0 {
                    let diag = (spacing * spacing * 2.0).sqrt();
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - width - 1,
                        rest_length: diag,
                        compliance: 0.005,
                    });
                    constraints.push(DistanceConstraint {
                        node_a: idx - 1,
                        node_b: idx - width,
                        rest_length: diag,
                        compliance: 0.005,
                    });
                }
            }
        }

        Self {
            nodes,
            constraints,
            thickness: 0.02,
            friction: 0.5,
        }
    }

    /// XPBD step
    pub fn step(&mut self, dt: f32, gravity: Vec3, sub_steps: usize) {
        let sub_dt = dt / (sub_steps as f32);
        let sub_dt2 = sub_dt * sub_dt;

        for _ in 0..sub_steps {
            // Predict
            for node in &mut self.nodes {
                if node.inv_mass == 0.0 { continue; }
                let velocity = (node.position - node.prev_position) / sub_dt;
                node.prev_position = node.position;
                
                // Add gravity
                let next_vel = velocity + gravity * sub_dt;
                node.position += next_vel * sub_dt;
            }

            // Solve Constraints
            for constraint in &self.constraints {
                let a = &self.nodes[constraint.node_a];
                let b = &self.nodes[constraint.node_b];
                
                let w_sum = a.inv_mass + b.inv_mass;
                if w_sum == 0.0 { continue; }

                let diff = a.position - b.position;
                let dist = diff.length();
                if dist < 1e-6 { continue; }

                let n = diff / dist;
                let c = dist - constraint.rest_length;
                let alpha = constraint.compliance / sub_dt2;

                let delta_lambda = -c / (w_sum + alpha);
                let p = n * delta_lambda;

                let inv_m_a = a.inv_mass;
                let inv_m_b = b.inv_mass;

                self.nodes[constraint.node_a].position += p * inv_m_a;
                self.nodes[constraint.node_b].position -= p * inv_m_b;
            }
        }
    }
}
