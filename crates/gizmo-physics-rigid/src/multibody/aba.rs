use super::{ArticulatedTree, SpatialTransform};
use gizmo_math::spatial::SpatialVector;
use gizmo_math::Vec3;

/// Featherstone's Articulated Body Algorithm (ABA)
/// Computes the joint accelerations (q_ddot) and spatial accelerations (a) for an ArticulatedTree
/// O(N) complexity for an N-link robot.
pub fn compute_aba(tree: &mut ArticulatedTree, gravity: Vec3) {
    if tree.links.is_empty() {
        return;
    }

    let n = tree.links.len();

    // -------------------------------------------------------------------------
    // PASS 1: Forward Kinematics (Root to Leaves)
    // Compute spatial velocities (v) and bias accelerations (c).
    // -------------------------------------------------------------------------
    
    // Root base velocity and spatial gravity
    let a_grav = SpatialVector::new(Vec3::ZERO, -gravity);
    
    for i in 0..n {
        tree.links[i].S = tree.links[i].compute_motion_subspace();
        let x_i = tree.links[i].compute_spatial_transform(); // Transform from i to parent
        
        let v_J = tree.links[i].S * tree.links[i].q_dot;
        
        if tree.links[i].parent_index == usize::MAX {
            // Root link
            if tree.is_fixed_base {
                tree.links[i].v = v_J;
                tree.links[i].c = SpatialVector::ZERO;
            } else {
                let v_parent_transformed = x_i.inverse_transform_motion(tree.base_velocity);
                tree.links[i].v = v_parent_transformed + v_J;
                tree.links[i].c = tree.links[i].v.cross_motion(v_J);
            }
        } else {
            let p_idx = tree.links[i].parent_index;
            let v_parent_transformed = x_i.inverse_transform_motion(tree.links[p_idx].v);
            tree.links[i].v = v_parent_transformed + v_J;
            tree.links[i].c = tree.links[i].v.cross_motion(v_J);
        }

        // Initialize Articulated Body Inertia (i_a) and bias force (p_a) for Pass 2
        tree.links[i].i_a = tree.links[i].inertia.to_matrix();
        tree.links[i].p_a = tree.links[i].v.cross_force(tree.links[i].inertia.mul_vec(tree.links[i].v));
    }

    // -------------------------------------------------------------------------
    // PASS 2: Backward Dynamics (Leaves to Root)
    // Compute Articulated Body Inertia (i_a) and bias forces (p_a).
    // Propagate forces up to parents.
    // -------------------------------------------------------------------------
    for i in (0..n).rev() {
        let u = tree.links[i].joint_force - tree.links[i].S.dot(tree.links[i].p_a);
        let u_vec = tree.links[i].i_a.mul_vec(tree.links[i].S);
        let d_val = tree.links[i].S.dot(u_vec);
        
        tree.links[i].u = u;
        tree.links[i].u_vec = u_vec;
        tree.links[i].d_val = d_val;

        if tree.links[i].parent_index != usize::MAX {
            let p_idx = tree.links[i].parent_index;
            let x_i = tree.links[i].compute_spatial_transform();

            if d_val > 1e-6 {
                // I_a = I_a - (u_vec * u_vec^T) / d_val
                let U_outer = u_vec.outer_product(u_vec).mul_scalar(1.0 / d_val);
                let ia_minus = tree.links[i].i_a - U_outer;
                let pa_plus = tree.links[i].p_a + tree.links[i].i_a.mul_vec(tree.links[i].c) + u_vec * (u / d_val);
                
                // Propagate to parent: i_a_parent += X * I_a * X^T (Approximated by spatial inertia shift if we use full matrices, but we have SpatialMatrix)
                // For exact ABA we need full 6x6 spatial matrix transform.
                // X^* I_a X^{-1}. We will implement this efficiently soon, but for now we do naive approximation:
                // Actually, full 6x6 transform is required here.
                
                // --- KISA DEVRE: Tam Spatial Matrix Dönüşümü ---
                let rot = x_i.rotation;
                let t = x_i.translation;
                let tx = gizmo_math::Mat3::from_cols(
                    Vec3::new(0.0, t.z, -t.y),
                    Vec3::new(-t.z, 0.0, t.x),
                    Vec3::new(t.y, -t.x, 0.0)
                );
                
                let i_00 = rot * ia_minus.m00 * rot.transpose();
                let i_01 = rot * ia_minus.m01 * rot.transpose();
                let i_10 = rot * ia_minus.m10 * rot.transpose();
                let i_11 = rot * ia_minus.m11 * rot.transpose();
                
                let i_parent_add = gizmo_math::spatial::SpatialMatrix {
                    m00: i_00 + tx * i_10 + i_01 * tx.transpose() + tx * i_11 * tx.transpose(),
                    m01: i_01 + tx * i_11,
                    m10: i_10 + i_11 * tx.transpose(),
                    m11: i_11,
                };
                
                tree.links[p_idx].i_a = tree.links[p_idx].i_a + i_parent_add;
                tree.links[p_idx].p_a = tree.links[p_idx].p_a + x_i.transform_force(pa_plus);
            }
        }
    }

    // -------------------------------------------------------------------------
    // PASS 3: Forward Dynamics (Root to Leaves)
    // Compute joint accelerations (q_ddot) and spatial accelerations (a).
    // -------------------------------------------------------------------------
    for i in 0..n {
        let x_i = tree.links[i].compute_spatial_transform();
        
        let a_parent = if tree.links[i].parent_index == usize::MAX {
            if tree.is_fixed_base {
                x_i.inverse_transform_motion(a_grav) // Root attached to world, transformed gravity to local frame
            } else {
                x_i.inverse_transform_motion(tree.base_acceleration) // Free floating base
            }
        } else {
            let p_idx = tree.links[i].parent_index;
            x_i.inverse_transform_motion(tree.links[p_idx].a)
        };

        let a_prime = a_parent + tree.links[i].c;
        
        if tree.links[i].d_val > 1e-6 {
            tree.links[i].q_ddot = (tree.links[i].u - tree.links[i].u_vec.dot(a_prime)) / tree.links[i].d_val;
        } else {
            tree.links[i].q_ddot = 0.0;
        }
        
        tree.links[i].a = a_prime + tree.links[i].S * tree.links[i].q_ddot;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multibody::{ArticulatedLink, JointType};
    use gizmo_math::spatial::{SpatialInertia, SpatialMatrix, SpatialVector};
    use gizmo_math::{Mat3, Quat, Vec3};

    #[test]
    fn test_single_pendulum_aba() {
        // A simple 1-link pendulum attached to a fixed base.
        // Link has mass=1.0, length=1.0 along Y axis.
        // Revolute joint around Z axis.
        let mut tree = ArticulatedTree::default();
        tree.is_fixed_base = true;

        let inertia = SpatialInertia::new(
            1.0,
            Mat3::IDENTITY, // simple spherical inertia for test
            Vec3::new(0.0, -1.0, 0.0), // CoM is 1 unit down
        );

        let link = ArticulatedLink {
            parent_index: usize::MAX, // Root
            joint_type: JointType::Revolute(Vec3::Z),
            transform_to_parent: Vec3::ZERO,
            rotation_to_parent: Quat::IDENTITY,
            inertia,
            q: 0.0,      // horizontal? No, if joint is Z, rotation is in XY plane. q=0 means straight down? No, local transform.
            q_dot: 0.0,
            q_ddot: 0.0,
            joint_force: 0.0,
            // placeholders
            v: SpatialVector::ZERO,
            a: SpatialVector::ZERO,
            c: SpatialVector::ZERO,
            i_a: SpatialMatrix::ZERO,
            p_a: SpatialVector::ZERO,
            S: SpatialVector::ZERO,
            u: 0.0,
            d_val: 0.0,
            u_vec: SpatialVector::ZERO,
        };

        tree.links.push(link);

        // Apply gravity (-9.81 on Y)
        let gravity = Vec3::new(0.0, -9.81, 0.0);
        
        // Initial state: q = 90 degrees (pi/2). If CoM is at (0, -1, 0), rotating it by 90 degrees around Z
        // moves it to (1, 0, 0).
        tree.links[0].q = std::f32::consts::PI / 2.0; 
        
        compute_aba(&mut tree, gravity);

        println!("Pendulum test trace:");
        println!("  Gravity: {:?}", gravity);
        println!("  tree.links[0].d_val = {}", tree.links[0].d_val);
        println!("  tree.links[0].u_vec = {:?}", tree.links[0].u_vec);
        println!("  tree.links[0].u = {}", tree.links[0].u);
        println!("  tree.links[0].p_a = {:?}", tree.links[0].p_a);
        println!("  tree.links[0].i_a = {:?}", tree.links[0].i_a);
        println!("  tree.links[0].S = {:?}", tree.links[0].S);
        println!("  tree.links[0].a = {:?}", tree.links[0].a);

        assert!(tree.links[0].q_ddot.abs() > 0.1, "Pendulum should have angular acceleration under gravity. Got: {}", tree.links[0].q_ddot);
    }
}
