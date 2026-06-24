use gizmo_math::Vec3;

/// Represents a single particle in the rope or chain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RopeNode {
    pub position: Vec3,
    pub prev_position: Vec3,
    pub mass: f32,
    pub inv_mass: f32,
    pub is_fixed: bool,
}

/// A rope or chain simulated using Position Based Dynamics (PBD).
#[derive(Debug, Clone, PartialEq)]
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

            // Mevcut hızı sönümle, korunumlu yerçekimini sönümsüz ekle.
            // (Eskiden taze yerçekimi de sönümleniyordu: `(v + g*dt) * damping`.)
            let new_vel = velocity * self.damping.powf(dt) + gravity * dt;
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

        // 3. Simple ground collision (sabit/pinned düğümleri TAŞIMA — bunlar her
        //    yere sabitlenebilir, zeminin altında bile; eskiden is_fixed guard yoktu).
        for node in &mut self.nodes {
            if node.is_fixed {
                continue;
            }
            if node.position.y < 0.0 {
                node.position.y = 0.0;
                node.prev_position.y = node.position.y; // zero vertical velocity
            }
        }
    }
}

impl gizmo_core::Component for Rope {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zeminin ALTINA sabitlenmiş bir düğüm, zemin çarpışması tarafından taşınmamalı
    /// (eskiden `is_fixed` guard yoktu → sabit düğüm y=0'a sıçrıyordu).
    #[test]
    fn fixed_node_below_floor_is_not_moved() {
        let mut rope = Rope::new(
            Vec3::new(0.0, -1.0, 0.0), // zeminin altında başla
            Vec3::new(1.0, 0.0, 0.0),
            3,
            0.5,
            1.0,
            true,  // fix_start
            false,
        );
        let fixed_pos = rope.nodes[0].position;
        let g = Vec3::new(0.0, -9.81, 0.0);
        for _ in 0..120 {
            rope.step(1.0 / 60.0, g);
        }
        assert_eq!(
            rope.nodes[0].position, fixed_pos,
            "sabit düğüm taşınmamalı (y=-1'de kalmalı)"
        );
        for n in &rope.nodes {
            assert!(n.position.is_finite(), "ip NaN/Inf üretmemeli");
        }
    }
}
