use crate::animation_state_machine::{ActiveBlend, AnimationStateMachine};
use crate::components::{AnimationPlayer, Skeleton};
use gizmo_core::World;
use gizmo_math::{Mat4, Quat, Vec3};
use std::sync::Arc;

// ── Simple AnimationPlayer update ────────────────────────────────────────────

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

            let animations = Arc::clone(&player.animations);
            let anim = match animations.get(player.active_animation) {
                Some(c) => c,
                None => {
                    eprintln!("Warning: Invalid active_animation index for entity.");
                    continue;
                }
            };

            player.current_time = player.current_time.max(0.0) + dt * player.speed;
            if player.current_time > anim.duration {
                if player.loop_anim && anim.duration > 0.0 {
                    player.current_time %= anim.duration;
                } else {
                    player.current_time = anim.duration;
                }
            }

            let poses_trs = evaluate_clip(anim, player.current_time, &skeleton.hierarchy);
            
            let final_trs = if let Some(prev_idx) = player.prev_animation {
                if player.blend_time < player.blend_duration {
                    player.blend_time += dt;
                    player.prev_time += dt;
                    
                    let mut prev_time_clamped = player.prev_time;
                    if let Some(prev_anim) = animations.get(prev_idx) {
                        if prev_time_clamped > prev_anim.duration {
                            if player.loop_anim && prev_anim.duration > 0.0 {
                                prev_time_clamped %= prev_anim.duration;
                            } else {
                                prev_time_clamped = prev_anim.duration;
                            }
                        }
                        
                        let prev_poses_trs = evaluate_clip(prev_anim, prev_time_clamped, &skeleton.hierarchy);
                        let alpha = (player.blend_time / player.blend_duration).clamp(0.0, 1.0);
                        blend_poses(&prev_poses_trs, &poses_trs, alpha)
                    } else {
                        poses_trs
                    }
                } else {
                    player.prev_animation = None;
                    poses_trs
                }
            } else {
                poses_trs
            };

            let poses = final_trs.into_iter().map(|(t, r, s)| {
                Mat4::from_scale_rotation_translation(s, r, t)
            }).collect();

            skeleton.local_poses = poses;

            upload_skin_matrices(skeleton, queue);
        }
    }
}

// ── AnimationStateMachine update ─────────────────────────────────────────────

pub fn animation_state_machine_update_system(world: &mut World, dt: f32, queue: &wgpu::Queue) {
    let mut machines = world.borrow_mut::<AnimationStateMachine>();
    let mut skeletons = world.borrow_mut::<Skeleton>();

    let entities: Vec<u32> = machines.entities().collect();
    for entity in entities {
        let machine = match machines.get_mut(entity) {
            Some(m) => m,
            None => continue,
        };
        let skeleton = match skeletons.get_mut(entity) {
            Some(s) => s,
            None => continue,
        };

        if machine.clips.is_empty() || machine.states.is_empty() {
            continue;
        }

        // --- Advance times ---
        let speed = machine.current_speed();
        machine.current_time += dt * speed;

        let clip_duration = machine.current_clip_duration();
        let looped = machine.is_current_looped();
        let clip_finished = machine.current_time >= clip_duration;

        if clip_finished {
            if looped && clip_duration > 0.0 {
                machine.current_time %= clip_duration;
            } else {
                machine.current_time = clip_duration;
            }
        }

        if let Some(ref mut blend) = machine.active_blend {
            let to_speed = blend.to_speed;
            blend.elapsed += dt;
            blend.to_time += dt * to_speed;

            let to_clip_duration = machine.clips.get(blend.to_clip).map(|c| c.duration).unwrap_or(1.0);
            let to_looped = blend.to_looped;
            if blend.to_time >= to_clip_duration {
                if to_looped && to_clip_duration > 0.0 {
                    blend.to_time %= to_clip_duration;
                } else {
                    blend.to_time = to_clip_duration;
                }
            }
        }

        // --- Evaluate pending triggers and exit-time transitions ---
        let triggers: Vec<String> = machine.drain_triggers();
        let current_name = machine.current_state.clone();

        // Only start a new blend if not already mid-transition
        if machine.active_blend.is_none() {
            let mut chosen_transition = None;

            // Trigger-based first
            'outer: for trigger in &triggers {
                if let Some(tr) = machine.find_transition(&current_name, Some(trigger), clip_finished) {
                    chosen_transition = Some((
                        tr.to.clone(),
                        tr.blend_duration,
                    ));
                    break 'outer;
                }
            }

            // Exit-time if no trigger matched
            if chosen_transition.is_none() && clip_finished {
                if let Some(tr) = machine.find_transition(&current_name, None, true) {
                    chosen_transition = Some((tr.to.clone(), tr.blend_duration));
                }
            }

            if let Some((to_state_name, blend_dur)) = chosen_transition {
                if let Some(to_state) = machine.find_state(&to_state_name).cloned() {
                    let from_clip = machine.current_clip_index().unwrap_or(0);
                    machine.active_blend = Some(ActiveBlend {
                        from_clip,
                        to_clip:   to_state.clip_index,
                        from_time: machine.current_time,
                        to_time:   0.0,
                        elapsed:   0.0,
                        duration:  blend_dur,
                        to_state:  to_state_name,
                        to_looped: to_state.looped,
                        to_speed:  to_state.speed,
                    });
                }
            }
        }

        // --- Check blend completion ---
        if machine.active_blend.as_ref().map(|b| b.alpha() >= 1.0).unwrap_or(false) {
            let blend = machine.active_blend.take().unwrap();
            machine.current_state = blend.to_state;
            machine.current_time  = blend.to_time;
        }

        // --- Compute blended poses ---
        let poses_trs = if let Some(ref blend) = machine.active_blend {
            let clip_a = match machine.clips.get(blend.from_clip) {
                Some(c) => c,
                None => continue,
            };
            let clip_b = match machine.clips.get(blend.to_clip) {
                Some(c) => c,
                None => continue,
            };
            let poses_a = evaluate_clip(clip_a, blend.from_time, &skeleton.hierarchy);
            let poses_b = evaluate_clip(clip_b, blend.to_time,   &skeleton.hierarchy);
            blend_poses(&poses_a, &poses_b, blend.alpha())
        } else {
            let clip_idx = match machine.current_clip_index() {
                Some(i) => i,
                None => continue,
            };
            let clip = match machine.clips.get(clip_idx) {
                Some(c) => c,
                None => continue,
            };
            evaluate_clip(clip, machine.current_time, &skeleton.hierarchy)
        };

        // Convert TRS to Mat4
        let poses = poses_trs.into_iter().map(|(t, r, s)| {
            Mat4::from_scale_rotation_translation(s, r, t)
        }).collect();

        skeleton.local_poses = poses;
        upload_skin_matrices(skeleton, queue);
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn evaluate_clip(
    clip: &crate::animation::AnimationClip,
    time: f32,
    hierarchy: &crate::animation::SkeletonHierarchy,
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
            if let Some(idx) = hierarchy.joints.iter().position(|j| clean(&j.name) == clean_name) {
                return Some(idx);
            }
        }
        if target_node_name.is_none() {
            if target_node < hierarchy.joints.len() {
                return Some(target_node);
            }
        }
        None
    };



    for track in &clip.translations {
        if let Some(joint_idx) = get_joint_idx(track.target_node, &track.target_node_name) {
            if let Some(v) = track.get_interpolated(time, |a: Vec3, b: Vec3, t| a.lerp(b, t)) {
                // Sadece Hips (kök) kemiğinin hareketine izin ver, diğerlerini yoksay. Mixamo animasyonlarında root motion buradadır.
                if track.target_node_name.as_deref() == Some("mixamorig:Hips") || track.target_node == 66 {
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
        let pos   = t_opt.unwrap_or(joint.bind_translation);
        let rot   = r_opt.unwrap_or(joint.bind_rotation);
        let scale = s_opt.unwrap_or(joint.bind_scale);
        
        result_trs.push((pos, rot, scale));
    }
    result_trs
}

/// Linearly blend two pose arrays. Uses lerp for T/S, slerp for R.
fn blend_poses(a: &[(Vec3, Quat, Vec3)], b: &[(Vec3, Quat, Vec3)], alpha: f32) -> Vec<(Vec3, Quat, Vec3)> {
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

fn upload_skin_matrices(skeleton: &Skeleton, queue: &wgpu::Queue) {
    let global_matrices = skeleton
        .hierarchy
        .calculate_global_matrices(&skeleton.local_poses);

    let mut joint_matrices = vec![Mat4::IDENTITY; 128];
    for (i, joint) in skeleton.hierarchy.joints.iter().enumerate() {
        if i < 128 {
            joint_matrices[i] = global_matrices[i] * joint.inverse_bind_matrix;
        }
    }
    queue.write_buffer(&skeleton.buffer, 0, bytemuck::cast_slice(&joint_matrices));
}

// find_joint_for_node artik kullanilmiyor

#[allow(dead_code)]
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
