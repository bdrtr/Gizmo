use gizmo_math::Vec3;

/// Represents a single particle in the rope or chain.
#[derive(Debug, Clone, Copy)]
pub struct RopeNode {
    pub position: Vec3,
    pub prev_position: Vec3,
    pub mass: f32,
    pub inv_mass: f32,
    pub is_fixed: bool,
}

/// A rope or chain simulated using Position Based Dynamics (PBD).
#[derive(Debug, Clone)]
pub struct Rope {
    pub nodes: Vec<RopeNode>,
    pub link_length: f32,
    pub iterations: usize,
    pub stiffness: f32,
    pub damping: f32,
}

impl Rope {
    pub fn new(
        start_pos: Vec3,
        direction: Vec3,
        num_segments: usize,
        segment_length: f32,
        node_mass: f32,
        fix_start: bool,
        fix_end: bool,
    ) -> Self {
        let mut nodes = Vec::with_capacity(num_segments + 1);
        let inv_mass = if node_mass > 0.0 {
            1.0 / node_mass
        } else {
            0.0
        };

        let dir_norm = direction.normalize_or_zero();

        for i in 0..=num_segments {
            let pos = start_pos + dir_norm * (i as f32 * segment_length);
            let is_fixed = (i == 0 && fix_start) || (i == num_segments && fix_end);
            nodes.push(RopeNode {
                position: pos,
                prev_position: pos,
                mass: if is_fixed { 0.0 } else { node_mass },
                inv_mass: if is_fixed { 0.0 } else { inv_mass },
                is_fixed,
            });
        }

        Self {
            nodes,
            link_length: segment_length,
            iterations: 10,
            stiffness: 1.0,
            damping: 0.98,
        }
    }

    pub fn step(&mut self, dt: f32, gravity: Vec3) {
        if dt <= 0.0 {
            return;
        }

        // 1. Explicit Euler integration for unconstrained motion
        for node in &mut self.nodes {
            if node.is_fixed {
                continue;
            }

            let velocity = (node.position - node.prev_position) / dt;
            node.prev_position = node.position;

            // Add gravity and damping
            let new_vel = (velocity + gravity * dt) * self.damping.powf(dt);
            node.position += new_vel * dt;
        }

        // 2. Position Based Dynamics constraint solving (XPBD)
        // Compliance = inverse stiffness; clamp stiffness to [0, 1] to avoid negative alpha
        let compliance = (1.0 - self.stiffness.min(1.0)).max(0.0);
        let alpha = compliance / (dt * dt);

        for _ in 0..self.iterations {
            for i in 0..(self.nodes.len() - 1) {
                let n1 = self.nodes[i];
                let n2 = self.nodes[i + 1];

                let w1 = n1.inv_mass;
                let w2 = n2.inv_mass;
                let w_sum = w1 + w2;

                if w_sum == 0.0 {
                    continue; // Both are fixed
                }

                let dir = n2.position - n1.position;
                let current_len = dir.length();
                if current_len < 1e-6 {
                    continue; // Prevent division by zero
                }

                let err = current_len - self.link_length;
                let correction_mag = err / (w_sum + alpha); // XPBD compliance
                let correction = dir.normalize() * correction_mag;

                if !n1.is_fixed {
                    self.nodes[i].position += correction * w1;
                }
                if !n2.is_fixed {
                    self.nodes[i + 1].position -= correction * w2;
                }
            }
        }

        // 3. Simple ground collision
        for node in &mut self.nodes {
            if node.position.y < 0.0 {
                node.position.y = 0.0;
                node.prev_position.y = node.position.y; // zero vertical velocity
            }
        }
    }
}
