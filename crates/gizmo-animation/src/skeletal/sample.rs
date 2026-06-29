use super::clip::AnimationClip;
use super::skeleton::SkeletonHierarchy;
use gizmo_math::{Mat4, Quat, Vec3};

pub fn evaluate_clip(
    clip: &AnimationClip,
    time: f32,
    hierarchy: &SkeletonHierarchy,
) -> Vec<(Vec3, Quat, Vec3)> {
    // Start with all None
    let mut changes = vec![(None, None, None); hierarchy.joints.len()];

    let get_joint_idx = |target_node: usize, target_node_name: &Option<String>| -> Option<usize> {
        if let Some(name) = target_node_name {
            // Tam eşleşme
            if let Some(idx) = hierarchy.joints.iter().position(|j| &j.name == name) {
                return Some(idx);
            }
            // Gevşek eşleşme (mixamorig: vs mixamorig_ ve /RootNode/ gibi fbx2gltf eklentileri)
            let clean = |n: &str| {
                n.replace("/RootNode/", "")
                    .replace("mixamorig:", "")
                    .replace("mixamorig_", "")
                    .to_lowercase()
            };
            let clean_name = clean(name);
            if let Some(idx) = hierarchy
                .joints
                .iter()
                .position(|j| clean(&j.name) == clean_name)
            {
                return Some(idx);
            }
        }
        if target_node_name.is_none() && target_node < hierarchy.joints.len() {
            return Some(target_node);
        }
        None
    };

    for track in &clip.translations {
        if let Some(joint_idx) = get_joint_idx(track.target_node, &track.target_node_name) {
            if let Some(v) = track.get_interpolated(time, |a: Vec3, b: Vec3, t| a.lerp(b, t)) {
                // Sadece Hips (kök) kemiğinin hareketine izin ver, diğerlerini yoksay. Mixamo animasyonlarında root motion buradadır.
                let is_hips = track.target_node_name.as_deref().is_some_and(|n| n.contains("Hips"))
                    || track.target_node == 66;
                if is_hips {
                    changes[joint_idx].0 = Some(v);
                }
            }
        }
    }
    for track in &clip.rotations {
        if let Some(joint_idx) = get_joint_idx(track.target_node, &track.target_node_name) {
            if let Some(v) = track.get_interpolated(time, |a: Quat, b: Quat, t| a.slerp(b, t)) {
                changes[joint_idx].1 = Some(v.normalize());
            }
        }
    }
    for track in &clip.scales {
        if let Some(_joint_idx) = get_joint_idx(track.target_node, &track.target_node_name) {
            if let Some(_v) = track.get_interpolated(time, |a: Vec3, b: Vec3, t| a.lerp(b, t)) {
                // SCALE IZLERINI TAMAMEN YOK SAYIYORUZ!
            }
        }
    }

    let mut result_trs = Vec::with_capacity(hierarchy.joints.len());
    for (joint_idx, (t_opt, r_opt, s_opt)) in changes.into_iter().enumerate() {
        let joint = &hierarchy.joints[joint_idx];
        let pos = t_opt.unwrap_or(joint.bind_translation);
        let rot = r_opt.unwrap_or(joint.bind_rotation);
        let scale = s_opt.unwrap_or(joint.bind_scale);

        result_trs.push((pos, rot, scale));
    }
    result_trs
}

/// Linearly blend two pose arrays. Uses lerp for T/S, slerp for R.
pub fn blend_poses(
    a: &[(Vec3, Quat, Vec3)],
    b: &[(Vec3, Quat, Vec3)],
    alpha: f32,
) -> Vec<(Vec3, Quat, Vec3)> {
    debug_assert_eq!(
        a.len(), b.len(),
        "blend_poses: Pose dizileri farklı uzunlukta! a={}, b={}", a.len(), b.len()
    );
    a.iter()
        .zip(b.iter())
        .map(|((ta, ra, sa), (tb, rb, sb))| {
            let t = ta.lerp(*tb, alpha);
            let r = ra.slerp(*rb, alpha).normalize();
            let s = sa.lerp(*sb, alpha);
            (t, r, s)
        })
        .collect()
}

#[allow(dead_code)]
pub fn decompose_mat4(m: Mat4) -> (Vec3, Quat, Vec3) {
    let t = Vec3::new(m.w_axis.x, m.w_axis.y, m.w_axis.z);
    let sx = Vec3::new(m.x_axis.x, m.x_axis.y, m.x_axis.z).length().max(1e-6);
    let sy = Vec3::new(m.y_axis.x, m.y_axis.y, m.y_axis.z).length().max(1e-6);
    let sz = Vec3::new(m.z_axis.x, m.z_axis.y, m.z_axis.z).length().max(1e-6);
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
