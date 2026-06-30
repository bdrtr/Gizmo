use crate::components::Skeleton;
use gizmo_animation::skeletal::{
    blend_poses, evaluate_clip, ActiveBlend, AnimationPlayer, AnimationStateMachine,
};
use gizmo_core::World;
use gizmo_math::Mat4;
use std::sync::Arc;

// ── Simple AnimationPlayer update ────────────────────────────────────────────

pub fn animation_update_system(world: &mut World, dt: f32, queue: &wgpu::Queue) {
    let entities: Vec<u32> = world.borrow::<AnimationPlayer>().entities().collect();
    // SAFETY: exclusive `&mut World`; AnimationPlayer and Skeleton are distinct component
    // types, so these two mutable queries never alias the same storage.
    let mut players = unsafe { world.borrow_mut_unchecked::<AnimationPlayer>() };
    let mut skeletons = unsafe { world.borrow_mut_unchecked::<Skeleton>() };
    {
        for entity in entities {
            let mut player = match players.get_mut(entity) {
                Some(p) => p,
                None => continue,
            };
            let mut skeleton = match skeletons.get_mut(entity) {
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
                    tracing::error!("Warning: Invalid active_animation index for entity.");
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

                        let prev_poses_trs =
                            evaluate_clip(prev_anim, prev_time_clamped, &skeleton.hierarchy);
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

            let poses = final_trs
                .into_iter()
                .map(|(t, r, s)| Mat4::from_scale_rotation_translation(s, r, t))
                .collect();

            skeleton.local_poses = poses;

            upload_skin_matrices(&mut skeleton, queue);
        }
    }
}

// ── AnimationStateMachine update ─────────────────────────────────────────────

pub fn animation_state_machine_update_system(world: &mut World, dt: f32, queue: &wgpu::Queue) {
    let entities: Vec<u32> = world.borrow::<AnimationStateMachine>().entities().collect();
    // SAFETY: exclusive `&mut World`; AnimationStateMachine and Skeleton are distinct
    // component types, so these two mutable queries never alias the same storage.
    let mut machines = unsafe { world.borrow_mut_unchecked::<AnimationStateMachine>() };
    let mut skeletons = unsafe { world.borrow_mut_unchecked::<Skeleton>() };
    for entity in entities {
        let mut machine_mut = match machines.get_mut(entity) {
            Some(m) => m,
            None => continue,
        };
        let machine = &mut *machine_mut;
        
        let mut skeleton_mut = match skeletons.get_mut(entity) {
            Some(s) => s,
            None => continue,
        };
        let skeleton = &mut *skeleton_mut;

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

            let to_clip_duration = machine
                .clips
                .get(blend.to_clip)
                .map(|c| c.duration)
                .unwrap_or(1.0);
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
                if let Some(tr) =
                    machine.find_transition(&current_name, Some(trigger), clip_finished)
                {
                    chosen_transition = Some((tr.to.clone(), tr.blend_duration));
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
                        to_clip: to_state.clip_index,
                        from_time: machine.current_time,
                        to_time: 0.0,
                        elapsed: 0.0,
                        duration: blend_dur,
                        to_state: to_state_name,
                        to_looped: to_state.looped,
                        to_speed: to_state.speed,
                    });
                }
            }
        }

        // --- Check blend completion ---
        if machine
            .active_blend
            .as_ref()
            .map(|b| b.alpha() >= 1.0)
            .unwrap_or(false)
        {
            let blend = machine.active_blend.take().unwrap();
            machine.current_state = blend.to_state;
            machine.current_time = blend.to_time;
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
            let poses_b = evaluate_clip(clip_b, blend.to_time, &skeleton.hierarchy);
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
        let poses = poses_trs
            .into_iter()
            .map(|(t, r, s)| Mat4::from_scale_rotation_translation(s, r, t))
            .collect();

        skeleton.local_poses = poses;
        upload_skin_matrices(&mut *skeleton, queue);
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────
//
// `evaluate_clip`, `blend_poses`, and `decompose_mat4` now live in
// `gizmo_animation::skeletal::sample` and are imported above.

fn upload_skin_matrices(skeleton: &mut Skeleton, queue: &wgpu::Queue) {
    let global_matrices = skeleton
        .hierarchy
        .calculate_global_matrices(&skeleton.local_poses);
    skeleton.global_poses = global_matrices.clone();


    let mut joint_matrices = vec![Mat4::IDENTITY; 128];
    for (i, joint) in skeleton.hierarchy.joints.iter().enumerate() {
        if i < 128 {
            joint_matrices[i] = global_matrices[i] * joint.inverse_bind_matrix;
        }
    }
    queue.write_buffer(&skeleton.buffer, 0, bytemuck::cast_slice(&joint_matrices));
}

// find_joint_for_node artik kullanilmiyor
