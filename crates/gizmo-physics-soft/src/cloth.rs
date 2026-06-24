use gizmo_math::Vec3;

/// A single particle (mass point) in the cloth simulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClothNode {
    /// Current world-space position.
    pub position: Vec3,
    /// Position at the previous sub-step (used for implicit velocity in XPBD).
    pub prev_position: Vec3,
    /// Mass of this node; `0.0` means the node is pinned (immovable).
    pub mass: f32,
    /// Inverse mass (`1/mass`); `0.0` for pinned nodes.
    pub inv_mass: f32,
}

/// A distance (stretch/bend/shear) constraint between two cloth nodes, solved with XPBD.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[non_exhaustive]
pub struct DistanceConstraint {
    pub node_a: usize,
    pub node_b: usize,
    pub rest_length: f32,
    pub compliance: f32, // Inverse stiffness
    pub lambda: f32,     // Accumulated XPBD multiplier
}

/// A cloth sheet simulated with Extended Position Based Dynamics (XPBD).
///
/// Built as a regular grid of [`ClothNode`]s linked by structural, bend and shear
/// [`DistanceConstraint`]s. Step the simulation with [`Cloth::step`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Cloth {
    /// All particles of the cloth, laid out row-major (`idx = y * width + x`).
    pub nodes: Vec<ClothNode>,
    /// Distance constraints connecting the nodes.
    pub constraints: Vec<DistanceConstraint>,
    /// Collision thickness against the floor plane (`y = thickness`).
    pub thickness: f32,
    /// Horizontal friction applied on floor contact, in `[0, 1]`.
    pub friction: f32,
}

impl Cloth {
    /// Builds a `width` x `height` grid of nodes spaced `spacing` units apart in the
    /// XY plane, each with mass `mass_per_node` (`0.0` makes a node pinned).
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
                    inv_mass: if mass_per_node > 0.0 {
                        1.0 / mass_per_node
                    } else {
                        0.0
                    },
                });

                let idx = y * width + x;

                // Structural constraints
                if x > 0 {
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - 1,
                        rest_length: spacing,
                        compliance: 0.001,
                        lambda: 0.0,
                    });
                }
                if y > 0 {
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - width,
                        rest_length: spacing,
                        compliance: 0.001,
                        lambda: 0.0,
                    });
                }

                // Bend constraints
                if x > 1 {
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - 2,
                        rest_length: spacing * 2.0,
                        compliance: 0.1,
                        lambda: 0.0,
                    });
                }
                if y > 1 {
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - width * 2,
                        rest_length: spacing * 2.0,
                        compliance: 0.1,
                        lambda: 0.0,
                    });
                }

                // Shear constraints
                if x > 0 && y > 0 {
                    let diag = spacing * std::f32::consts::SQRT_2;
                    constraints.push(DistanceConstraint {
                        node_a: idx,
                        node_b: idx - width - 1,
                        rest_length: diag,
                        compliance: 0.005,
                        lambda: 0.0,
                    });
                    constraints.push(DistanceConstraint {
                        node_a: idx - 1,
                        node_b: idx - width,
                        rest_length: diag,
                        compliance: 0.005,
                        lambda: 0.0,
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

    /// Pins the node at `idx` so it becomes immovable (no-op if out of range).
    pub fn pin_node(&mut self, idx: usize) {
        if idx < self.nodes.len() {
            self.nodes[idx].inv_mass = 0.0;
            self.nodes[idx].mass = 0.0;
        }
    }

    /// Advances the cloth by one XPBD timestep of length `dt` (seconds), applying
    /// `gravity` (units/s²) and dividing the step into `sub_steps` solver iterations.
    pub fn step(&mut self, dt: f32, gravity: Vec3, sub_steps: usize) {
        let sub_dt = dt / (sub_steps as f32);
        let sub_dt2 = sub_dt * sub_dt;

        for _ in 0..sub_steps {
            for c in &mut self.constraints {
                c.lambda = 0.0;
            }

            // Predict
            for node in &mut self.nodes {
                if node.inv_mass == 0.0 {
                    continue;
                }
                let velocity = (node.position - node.prev_position) / sub_dt;
                node.prev_position = node.position;

                // Add gravity and damping (frame-rate independent)
                let damping = 0.99f32;
                let next_vel = velocity * damping.powf(sub_dt) + gravity * sub_dt;
                node.position += next_vel * sub_dt;
            }

            // Solve Constraints
            for constraint in &mut self.constraints {
                let (pos_a, pos_b, inv_m_a, inv_m_b) = {
                    let a = &self.nodes[constraint.node_a];
                    let b = &self.nodes[constraint.node_b];
                    (a.position, b.position, a.inv_mass, b.inv_mass)
                };

                let w_sum = inv_m_a + inv_m_b;
                if w_sum == 0.0 {
                    continue;
                }

                let diff = pos_a - pos_b;
                let dist = diff.length();
                if dist < 1e-6 {
                    continue;
                }

                let n = diff / dist;
                let c = dist - constraint.rest_length;
                let alpha = constraint.compliance / sub_dt2;

                let delta_lambda = (-c - alpha * constraint.lambda) / (w_sum + alpha);
                constraint.lambda += delta_lambda;

                let p = n * delta_lambda;

                self.nodes[constraint.node_a].position += p * inv_m_a;
                self.nodes[constraint.node_b].position -= p * inv_m_b;
            }

            // Floor Collision
            for node in &mut self.nodes {
                if node.inv_mass == 0.0 {
                    continue;
                }
                if node.position.y < self.thickness {
                    node.position.y = self.thickness;

                    // Simple friction: damp horizontal velocity when touching ground.
                    let mut vel = (node.position - node.prev_position) / sub_dt;
                    vel.x *= 1.0 - self.friction;
                    vel.z *= 1.0 - self.friction;
                    // Aşağı yönlü hızı koru-MA: zemine doğru momentum biriktirmek
                    // titreme/enerji enjeksiyonuna yol açıyordu (yukarı serbest).
                    vel.y = vel.y.max(0.0);
                    node.prev_position = node.position - vel * sub_dt;
                }
            }
        }
    }
}

impl gizmo_core::Component for Cloth {}
