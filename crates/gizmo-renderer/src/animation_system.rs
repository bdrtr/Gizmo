use crate::components::{AnimationPlayer, Skeleton};
use gizmo_core::World;
use gizmo_math::{Mat4, Quat, Vec3};

pub fn animation_update_system(world: &mut World, dt: f32, queue: &wgpu::Queue) {
    let mut players = world.borrow_mut::<AnimationPlayer>();
    let mut skeletons = world.borrow_mut::<Skeleton>();
    {
        let entities: Vec<u32> = players.entities().collect();
        for entity in entities {
            let player = match players.get_mut(entity) {
                Some(p) => p,
                None => continue,
            };
            let skeleton = match skeletons.get_mut(entity) {
                Some(s) => s,
                None => continue,
            };

            if player.animations.is_empty() {
                continue;
            }

            // Güvenli klip indeksi
            if player.active_animation >= player.animations.len() {
                player.active_animation = 0;
            }

            let anim = &player.animations[player.active_animation];

            // Süre ilerletme
            player.current_time += dt;
            if player.current_time > anim.duration {
                if player.loop_anim && anim.duration > 0.0 {
                    player.current_time %= anim.duration;
                } else {
                    player.current_time = anim.duration;
                }
            }

            let t = player.current_time;

            // Her track'in değerini Skeleton joint'ine işle.
            // Orijinal local matrisin bileşenlerini saklamak yerine T, R, S'yi parçalayıp
            // manipüle ederek yeniden matris inşa edeceğiz.
            let mut local_poses = skeleton.local_poses.clone();

            let mut node_changes: std::collections::HashMap<usize, (Option<Vec3>, Option<Quat>, Option<Vec3>)> =
                std::collections::HashMap::new();

            // Hangi joint'in gltf node index'ine göre track edildiğini bul (O(N^2) yerine hashing de yapılabilir ama N <= 64)
            // Track'leri işle
            for track in &anim.translations {
                if let Some(val) = track.get_interpolated(t, |a: Vec3, b: Vec3, frac: f32| a.lerp(b, frac)) {
                    node_changes.entry(track.target_node).or_default().0 = Some(val);
                }
            }
            for track in &anim.rotations {
                if let Some(val) = track.get_interpolated(t, |a: Quat, b: Quat, frac: f32| a.slerp(b, frac)) {
                    node_changes.entry(track.target_node).or_default().1 = Some(val.normalize());
                }
            }
            for track in &anim.scales {
                if let Some(val) = track.get_interpolated(t, |a: Vec3, b: Vec3, frac: f32| a.lerp(b, frac)) {
                    node_changes.entry(track.target_node).or_default().2 = Some(val);
                }
            }

            // Uygula
            for (node_idx, (t_opt, r_opt, s_opt)) in node_changes {
                if let Some(joint_idx) = find_joint_index(&skeleton.hierarchy.joints, node_idx) {
                    let old_mat = local_poses[joint_idx];
                    let (mut pos, mut rot, mut scale) = decompose_mat4(old_mat);

                    if let Some(new_t) = t_opt { pos = new_t; }
                    if let Some(new_r) = r_opt { rot = new_r; }
                    if let Some(new_s) = s_opt { scale = new_s; }

                    local_poses[joint_idx] = Mat4::from_scale_rotation_translation(scale, rot, pos);
                }
            }

            skeleton.local_poses = local_poses;

            // Global matrisleri hesapla (Local * Parent Global)
            let global_matrices = skeleton.hierarchy.calculate_global_matrices(&skeleton.local_poses);

            // Skin matrix = GlobalMatrix * InverseBindMatrix
            let mut joint_matrices = vec![Mat4::IDENTITY; 64]; 
            for (i, joint) in skeleton.hierarchy.joints.iter().enumerate() {
                if i < 64 {
                    joint_matrices[i] = global_matrices[i] * joint.inverse_bind_matrix;
                }
            }

            // GPU'ya yükle
            let byte_data: Vec<u8> = bytemuck::cast_slice(&joint_matrices).to_vec();
            queue.write_buffer(&skeleton.buffer, 0, &byte_data);
        }
    }
}

fn find_joint_index(joints: &[crate::animation::SkeletonJoint], node_index: usize) -> Option<usize> {
    joints.iter().position(|j| j.node_index == node_index)
}

fn decompose_mat4(m: Mat4) -> (Vec3, Quat, Vec3) {
    let t = Vec3::new(m.w_axis.x, m.w_axis.y, m.w_axis.z);
    
    let sx = Vec3::new(m.x_axis.x, m.x_axis.y, m.x_axis.z).length();
    let sy = Vec3::new(m.y_axis.x, m.y_axis.y, m.y_axis.z).length();
    let sz = Vec3::new(m.z_axis.x, m.z_axis.y, m.z_axis.z).length();
    
    let scale = Vec3::new(sx, sy, sz);
    
    let r_mat = Mat4::from_cols(
        gizmo_math::Vec4::new(m.x_axis.x / sx, m.x_axis.y / sx, m.x_axis.z / sx, 0.0),
        gizmo_math::Vec4::new(m.y_axis.x / sy, m.y_axis.y / sy, m.y_axis.z / sy, 0.0),
        gizmo_math::Vec4::new(m.z_axis.x / sz, m.z_axis.y / sz, m.z_axis.z / sz, 0.0),
        gizmo_math::Vec4::new(0.0, 0.0, 0.0, 1.0),
    );
    let r = Quat::from_mat4(&r_mat).normalize();
    
    (t, r, scale)
}
