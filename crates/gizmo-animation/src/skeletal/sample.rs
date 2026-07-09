use super::clip::AnimationClip;
use super::keyframe::Track;
use super::skeleton::SkeletonHierarchy;
use gizmo_math::{Mat4, Quat, Vec3};

/// glTF Appendix C cubic-Hermite basis coefficients for normalized position `s ∈ [0,1]`.
/// Tangents `m0`/`m1` are per-second and are scaled by the segment duration `dt`.
#[inline]
fn hermite_coeffs(s: f32, dt: f32) -> (f32, f32, f32, f32) {
    let s2 = s * s;
    let s3 = s2 * s;
    (
        2.0 * s3 - 3.0 * s2 + 1.0, // h00 · p0
        (s3 - 2.0 * s2 + s) * dt,  // h10 · m0 (out-tangent of k)
        -2.0 * s3 + 3.0 * s2,      // h01 · p1
        (s3 - s2) * dt,            // h11 · m1 (in-tangent of k+1)
    )
}

fn hermite_vec3(p0: Vec3, m0: Vec3, p1: Vec3, m1: Vec3, s: f32, dt: f32) -> Vec3 {
    let (a, b, c, d) = hermite_coeffs(s, dt);
    p0 * a + m0 * b + p1 * c + m1 * d
}

fn hermite_quat(p0: Quat, m0: Quat, p1: Quat, m1: Quat, s: f32, dt: f32) -> Quat {
    let (a, b, c, d) = hermite_coeffs(s, dt);
    // glTF cubic rotations are Hermite-interpolated component-wise, then renormalized.
    (p0 * a + m0 * b + p1 * c + m1 * d).normalize()
}

/// Sample a Vec3 track: true cubic-Hermite for `CubicSpline`, otherwise (or when tangents
/// are absent) linear.
fn sample_vec3(track: &Track<Vec3>, time: f32) -> Option<Vec3> {
    track
        .sample_cubic(time, hermite_vec3)
        .or_else(|| track.get_interpolated(time, |a: Vec3, b: Vec3, t| a.lerp(b, t)))
}

/// Sample a Quat track: cubic-Hermite for `CubicSpline`, otherwise slerp.
fn sample_quat(track: &Track<Quat>, time: f32) -> Option<Quat> {
    track
        .sample_cubic(time, hermite_quat)
        .map(|q| q.normalize())
        .or_else(|| track.get_interpolated(time, |a: Quat, b: Quat, t| a.slerp(b, t)))
}

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
        // Fallback: `target_node` is the GLTF *global node index* (channel.target().
        // node().index()), NOT a bone index. Map it to the bone that was built from that
        // node — joints record their source `node_index` exactly for this. The old code
        // used `target_node` directly as a bone index (and only if it happened to be <
        // joint count), which silently mis-targeted or dropped the track whenever the
        // armature wasn't the first nodes in the document (common in Blender exports).
        // This also now recovers a track whose name was present but didn't match a joint.
        hierarchy
            .joints
            .iter()
            .position(|j| j.node_index == target_node)
    };

    for track in &clip.translations {
        if let Some(joint_idx) = get_joint_idx(track.target_node, &track.target_node_name) {
            if let Some(v) = sample_vec3(track, time) {
                // Sadece Hips (kök) kemiğinin hareketine izin ver, diğerlerini yoksay. Mixamo animasyonlarında root motion buradadır.
                // Kök tespiti yalnızca isim tabanlıdır: hard-coded bir düğüm indeksi (eskiden 66),
                // Hips'i farklı indekste olan iskeletlerde root motion'ı sessizce bozuyordu.
                let is_hips = track
                    .target_node_name
                    .as_deref()
                    .is_some_and(|n| n.contains("Hips"));
                if is_hips {
                    changes[joint_idx].0 = Some(v);
                }
            }
        }
    }
    for track in &clip.rotations {
        if let Some(joint_idx) = get_joint_idx(track.target_node, &track.target_node_name) {
            if let Some(v) = sample_quat(track, time) {
                changes[joint_idx].1 = Some(v.normalize());
            }
        }
    }
    for track in &clip.scales {
        if let Some(joint_idx) = get_joint_idx(track.target_node, &track.target_node_name) {
            if let Some(v) = sample_vec3(track, time) {
                // Scale izleri artık uygulanıyor (eskiden tamamen atılıyordu). Squash/stretch,
                // nefes-alma ve büyüme animasyonları için gerekli; render iskelete ulaşır.
                changes[joint_idx].2 = Some(v);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeletal::keyframe::{InterpolationMode, Keyframe, Track};
    use crate::skeletal::skeleton::{SkeletonHierarchy, SkeletonJoint};

    fn make_joint(name: &str, node_index: usize) -> SkeletonJoint {
        SkeletonJoint {
            name: name.into(),
            node_index,
            inverse_bind_matrix: Mat4::IDENTITY,
            parent_index: None,
            local_bind_transform: Mat4::IDENTITY,
            bind_translation: Vec3::ZERO,
            bind_rotation: Quat::IDENTITY,
            bind_scale: Vec3::ONE,
        }
    }

    fn translation_track(target_node: usize, name: Option<&str>, value: Vec3) -> Track<Vec3> {
        Track {
            target_node,
            target_node_name: name.map(|s| s.to_string()),
            interpolation: InterpolationMode::Linear,
            keyframes: vec![Keyframe::new(0.0, value)],
        }
    }

    fn scale_track(target_node: usize, name: Option<&str>, value: Vec3) -> Track<Vec3> {
        Track {
            target_node,
            target_node_name: name.map(|s| s.to_string()),
            interpolation: InterpolationMode::Linear,
            keyframes: vec![Keyframe::new(0.0, value)],
        }
    }

    #[test]
    fn root_motion_applies_to_hips_regardless_of_node_index() {
        // Hips is NOT at the old hard-coded index 66; root motion must still apply.
        let hierarchy = SkeletonHierarchy {
            joints: vec![make_joint("mixamorig:Hips", 15)],
            root_transform: Mat4::IDENTITY,
        };
        let motion = Vec3::new(0.0, 0.0, 5.0);
        let clip = AnimationClip {
            name: "walk".into(),
            duration: 1.0,
            translations: vec![translation_track(15, Some("mixamorig:Hips"), motion)],
            rotations: vec![],
            scales: vec![],
        };
        let poses = evaluate_clip(&clip, 0.0, &hierarchy);
        assert!(
            (poses[0].0 - motion).length() < 0.001,
            "Hips root motion should apply based on the name, got {:?}",
            poses[0].0
        );
    }

    #[test]
    fn root_motion_ignored_for_non_hips_bones() {
        // A non-Hips translation track must be ignored (bind translation kept),
        // and it must NOT be resurrected just because target_node == 66.
        let bind = Vec3::new(1.0, 2.0, 3.0);
        let mut joint = make_joint("LeftFoot", 66);
        joint.bind_translation = bind;
        let hierarchy = SkeletonHierarchy {
            joints: vec![joint],
            root_transform: Mat4::IDENTITY,
        };
        let clip = AnimationClip {
            name: "walk".into(),
            duration: 1.0,
            translations: vec![translation_track(66, Some("LeftFoot"), Vec3::new(9.0, 9.0, 9.0))],
            rotations: vec![],
            scales: vec![],
        };
        let poses = evaluate_clip(&clip, 0.0, &hierarchy);
        assert!(
            (poses[0].0 - bind).length() < 0.001,
            "Non-Hips translation must be ignored even at node index 66, got {:?}",
            poses[0].0
        );
    }

    // Regression: a nameless animation channel carries the GLTF *global node index*,
    // which is NOT a bone index. When the armature isn't the first nodes in the
    // document (typical Blender export) the old fallback (`target_node < joint_count`)
    // dropped or mis-targeted the track. The fix maps the node index to the bone built
    // from that node via `joint.node_index`.
    #[test]
    fn nameless_track_maps_gltf_node_index_to_the_right_bone() {
        // Bone 0 was built from node 20, bone 1 from node 22 (node indices ≠ bone indices).
        let hierarchy = SkeletonHierarchy {
            joints: vec![make_joint("Root", 20), make_joint("Arm", 22)],
            root_transform: Mat4::IDENTITY,
        };
        let rot = Quat::from_rotation_y(1.0);
        let clip = AnimationClip {
            name: "a".into(),
            duration: 1.0,
            translations: vec![],
            rotations: vec![Track {
                target_node: 22,       // GLTF node index of "Arm"
                target_node_name: None, // nameless → exercises the node-index fallback
                interpolation: InterpolationMode::Linear,
                keyframes: vec![Keyframe::new(0.0, rot)],
            }],
            scales: vec![],
        };
        let poses = evaluate_clip(&clip, 0.0, &hierarchy);
        // Old fallback: `target_node (22) < joints.len() (2)` == false → track dropped →
        // bone 1 would stay at bind (identity). The fix applies it to bone 1 (node 22).
        assert!(
            poses[1].1.dot(rot).abs() > 0.999,
            "nameless track must rotate the bone built from node 22, got {:?}",
            poses[1].1
        );
        // Bone 0 (node 20) was not targeted → stays at bind rotation (identity).
        assert!(
            poses[0].1.dot(Quat::IDENTITY).abs() > 0.999,
            "untargeted bone must keep its bind rotation, got {:?}",
            poses[0].1
        );
    }

    // Regression: scale tracks were previously read-and-discarded ("SCALE IZLERINI TAMAMEN
    // YOK SAYIYORUZ!"), so squash/stretch/breathing animation never reached the skeleton.
    // A scale track must now drive the joint's scale instead of falling back to bind_scale.
    #[test]
    fn scale_track_is_applied_to_joint() {
        let hierarchy = SkeletonHierarchy {
            joints: vec![make_joint("Bone", 3)],
            root_transform: Mat4::IDENTITY,
        };
        let s = Vec3::new(2.0, 0.5, 1.5);
        let clip = AnimationClip {
            name: "squash".into(),
            duration: 1.0,
            translations: vec![],
            rotations: vec![],
            scales: vec![scale_track(3, Some("Bone"), s)],
        };
        let poses = evaluate_clip(&clip, 0.0, &hierarchy);
        assert!(
            (poses[0].2 - s).length() < 1e-5,
            "scale track must be applied, got {:?} (bind_scale ONE would mean it was dropped)",
            poses[0].2
        );
    }

    // A joint with no scale track keeps its bind scale (ONE here).
    #[test]
    fn untargeted_joint_keeps_bind_scale() {
        let hierarchy = SkeletonHierarchy {
            joints: vec![make_joint("Bone", 3)],
            root_transform: Mat4::IDENTITY,
        };
        let clip = AnimationClip {
            name: "empty".into(),
            duration: 1.0,
            translations: vec![],
            rotations: vec![],
            scales: vec![],
        };
        let poses = evaluate_clip(&clip, 0.0, &hierarchy);
        assert!((poses[0].2 - Vec3::ONE).length() < 1e-5, "should keep bind_scale ONE");
    }

    // A CubicSpline scale track with tangents must sample via true Hermite, not lerp.
    #[test]
    fn cubic_scale_track_uses_hermite_not_lerp() {
        use crate::skeletal::keyframe::Keyframe;
        let hierarchy = SkeletonHierarchy {
            joints: vec![make_joint("Bone", 3)],
            root_transform: Mat4::IDENTITY,
        };
        // Flat (zero) tangents at both ends → smooth ease; at s=0.25 the X channel is
        // Hermite(0,10)=1.5625, distinctly below the linear value 2.5.
        let track = Track {
            target_node: 3,
            target_node_name: Some("Bone".into()),
            interpolation: InterpolationMode::CubicSpline,
            keyframes: vec![
                Keyframe::with_tangents(0.0, Vec3::new(0.0, 0.0, 0.0), Vec3::ZERO, Vec3::ZERO),
                Keyframe::with_tangents(1.0, Vec3::new(10.0, 10.0, 10.0), Vec3::ZERO, Vec3::ZERO),
            ],
        };
        let clip = AnimationClip {
            name: "grow".into(),
            duration: 1.0,
            translations: vec![],
            rotations: vec![],
            scales: vec![track],
        };
        let poses = evaluate_clip(&clip, 0.25, &hierarchy);
        assert!(
            (poses[0].2.x - 1.5625).abs() < 1e-4,
            "cubic scale at s=0.25 should be 1.5625, got {} (2.5 would mean lerp)",
            poses[0].2.x
        );
    }
}
