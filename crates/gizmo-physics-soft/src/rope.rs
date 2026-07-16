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
#[non_exhaustive]
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
            // `saturating_sub` guards an empty `nodes` (a `pub` field): plain
            // `len() - 1` underflows to a panic in debug / `usize::MAX` in release.
            for i in 0..self.nodes.len().saturating_sub(1) {
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

    /// `Rope::new` lays `num_segments + 1` nodes along the (normalized) direction spaced
    /// `segment_length` apart, tags the requested endpoints fixed, and zeroes their mass.
    #[test]
    fn rope_new_builds_nodes_positions_and_fixed_flags() {
        let start = Vec3::new(1.0, 2.0, 3.0);
        let rope = Rope::new(start, Vec3::new(1.0, 0.0, 0.0), 4, 0.5, 2.0, true, false);

        assert_eq!(rope.nodes.len(), 5, "num_segments + 1 nodes");
        for (i, n) in rope.nodes.iter().enumerate() {
            let expected = start + Vec3::new(i as f32 * 0.5, 0.0, 0.0);
            assert!(n.position.abs_diff_eq(expected, 1e-6), "node {i} misplaced: {:?}", n.position);
            assert!(n.prev_position.abs_diff_eq(expected, 1e-6));
        }
        // fix_start pins node 0 only.
        assert!(rope.nodes[0].is_fixed && rope.nodes[0].inv_mass == 0.0 && rope.nodes[0].mass == 0.0);
        for n in &rope.nodes[1..] {
            assert!(!n.is_fixed);
            assert!((n.inv_mass - 0.5).abs() < 1e-6, "inv_mass = 1/mass");
            assert_eq!(n.mass, 2.0);
        }
        // Defaults.
        assert!((rope.link_length - 0.5).abs() < 1e-6);
        assert_eq!(rope.iterations, 10);
        assert!((rope.stiffness - 1.0).abs() < 1e-6);
        assert!((rope.damping - 0.98).abs() < 1e-6);
    }

    /// A non-unit direction is normalized: spacing is `segment_length`, not scaled by the
    /// direction's magnitude.
    #[test]
    fn rope_new_normalizes_direction() {
        // Direction magnitude 3, but spacing must stay at segment_length = 1.
        let rope = Rope::new(Vec3::ZERO, Vec3::new(0.0, 3.0, 0.0), 2, 1.0, 1.0, false, false);
        assert!(rope.nodes[1].position.abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-6));
        assert!(rope.nodes[2].position.abs_diff_eq(Vec3::new(0.0, 2.0, 0.0), 1e-6));
    }

    /// A zero direction has no defined orientation (`normalize_or_zero` → 0), collapsing every
    /// node onto the start position instead of producing NaN.
    #[test]
    fn rope_new_zero_direction_collapses_to_start() {
        let start = Vec3::new(4.0, 5.0, 6.0);
        let rope = Rope::new(start, Vec3::ZERO, 3, 1.0, 1.0, false, false);
        for n in &rope.nodes {
            assert!(n.position.abs_diff_eq(start, 1e-6), "all nodes collapse to start");
        }
    }

    /// `fix_end` pins the LAST node (and only it, when fix_start is false).
    #[test]
    fn rope_new_fix_end_pins_last_node() {
        let rope = Rope::new(Vec3::ZERO, Vec3::X, 3, 1.0, 1.0, false, true);
        let last = rope.nodes.len() - 1;
        assert!(rope.nodes[last].is_fixed && rope.nodes[last].inv_mass == 0.0);
        assert!(!rope.nodes[0].is_fixed, "start must remain free");
    }

    /// A non-positive `dt` is a guarded no-op: the rope must not integrate, drift, or divide
    /// by zero.
    #[test]
    fn rope_step_nonpositive_dt_is_noop() {
        let mut rope = Rope::new(Vec3::new(0.0, 5.0, 0.0), Vec3::X, 4, 0.5, 1.0, false, false);
        let before = rope.clone();
        rope.step(0.0, Vec3::new(0.0, -9.81, 0.0));
        assert_eq!(rope, before, "dt == 0 must change nothing");
        rope.step(-0.01, Vec3::new(0.0, -9.81, 0.0));
        assert_eq!(rope, before, "dt < 0 must change nothing");
    }

    /// The constraint loop uses `saturating_sub`, so stepping a rope whose (public) `nodes`
    /// vector has been emptied must not underflow-panic.
    #[test]
    fn rope_step_empty_nodes_does_not_panic() {
        let mut rope = Rope::new(Vec3::ZERO, Vec3::X, 2, 1.0, 1.0, false, false);
        rope.nodes.clear();
        rope.step(1.0 / 60.0, Vec3::new(0.0, -9.81, 0.0)); // must not panic
        assert!(rope.nodes.is_empty());
    }

    /// A free node below the floor is snapped up to `y = 0` with its vertical velocity zeroed
    /// (prev_position.y == 0).
    #[test]
    fn free_node_below_floor_is_clamped() {
        // Single-node rope (num_segments = 0) → no links, isolates the floor clamp.
        let mut rope = Rope::new(Vec3::new(0.0, -2.0, 0.0), Vec3::X, 0, 1.0, 1.0, false, false);
        assert_eq!(rope.nodes.len(), 1);
        rope.step(1.0 / 60.0, Vec3::ZERO);
        assert!((rope.nodes[0].position.y - 0.0).abs() < 1e-6, "must clamp to floor");
        assert!((rope.nodes[0].prev_position.y - 0.0).abs() < 1e-6, "vertical velocity zeroed");
    }

    /// An overstretched link is pulled back to `link_length`. With full stiffness (compliance
    /// 0) a symmetric free pair converges to the rest length essentially exactly.
    #[test]
    fn stretched_link_is_pulled_to_rest_length() {
        let mut rope = Rope::new(Vec3::new(0.0, 5.0, 0.0), Vec3::X, 1, 1.0, 1.0, false, false);
        // Pull the pair to 3 units apart (rest = 1), zero velocity, well above the floor.
        rope.nodes[0].position = Vec3::new(0.0, 5.0, 0.0);
        rope.nodes[0].prev_position = rope.nodes[0].position;
        rope.nodes[1].position = Vec3::new(3.0, 5.0, 0.0);
        rope.nodes[1].prev_position = rope.nodes[1].position;

        rope.step(1.0 / 60.0, Vec3::ZERO);
        let d = (rope.nodes[0].position - rope.nodes[1].position).length();
        assert!((d - 1.0).abs() < 1e-3, "link must converge to rest length, got {d}");
    }

    /// Two identical ropes stepped identically stay bit-identical (deterministic solver).
    #[test]
    fn rope_step_is_deterministic() {
        let mk = || Rope::new(Vec3::new(0.0, 5.0, 0.0), Vec3::X, 8, 0.3, 1.0, true, false);
        let (mut a, mut b) = (mk(), mk());
        let g = Vec3::new(0.1, -9.81, 0.0);
        for _ in 0..60 {
            a.step(1.0 / 60.0, g);
            b.step(1.0 / 60.0, g);
        }
        assert_eq!(a, b);
    }
}
