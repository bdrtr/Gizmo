use super::ArticulatedTree;
use super::aba::compute_aba;
use gizmo_math::Vec3;

/// Entagrates the joints of an articulated tree using Heun's Method (RK2) or Semi-Implicit Euler.
pub fn step_articulated_trees(trees: &mut [ArticulatedTree], dt: f32, gravity: Vec3) {
    if dt <= 0.0 {
        return;
    }

    for tree in trees.iter_mut() {
        // 1. Compute joint accelerations (q_ddot) via ABA
        compute_aba(tree, gravity);

        // 2. Integrate q_ddot into q_dot, and q_dot into q
        // Using Semi-Implicit Euler for stability in joints
        for link in tree.links.iter_mut() {
            link.q_dot += link.q_ddot * dt;
            link.q += link.q_dot * dt;
        }

        // Eğer tree serbest uçan (free-floating) bir base'e sahipse, 
        // root'un base_acceleration değerini de v'ye entegre etmeliyiz.
        if !tree.is_fixed_base {
            // (Base ivmesi şu an pass 3'ten çıkarılmıyor, ancak gelecekte floating-base ABA
            // tamamlandığında burada base_velocity += base_acceleration * dt yapılacak)
            // tree.base_velocity += tree.base_acceleration * dt;
            // tree.base_position += tree.base_velocity.v * dt;
            // (Rotasyon için Quaternion integrasyonu)
        }
    }
}
