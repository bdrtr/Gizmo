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

            // Advance FIRST, then normalize. Clamping to `.max(0.0)` *before* adding
            // `dt*speed` pinned reverse playback (speed < 0) at frame 0 forever. rem_euclid
            // wraps negative times back into [0, duration) so a looping clip plays backward;
            // non-looping clips clamp into range. Forward playback is unchanged (t % dur).
            player.current_time += dt * player.speed;
            if anim.duration > 0.0 {
                if player.loop_anim {
                    player.current_time = player.current_time.rem_euclid(anim.duration);
                } else {
                    player.current_time = player.current_time.clamp(0.0, anim.duration);
                }
            } else {
                player.current_time = player.current_time.max(0.0);
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

/// Normalize an animation clip time into `[0, duration)`.
///
/// Looping clips use `rem_euclid` so negative times (reverse playback, speed < 0)
/// wrap back into range — plain `%` preserves the dividend's sign and would leave
/// the sampler evaluating at negative times. Non-looping clips clamp into range.
fn normalize_anim_time(time: f32, duration: f32, looped: bool) -> f32 {
    if looped && duration > 0.0 {
        time.rem_euclid(duration)
    } else {
        time.clamp(0.0, duration.max(0.0))
    }
}

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
        // Capture completion BEFORE normalization wraps the time back into range,
        // so exit-time transitions still fire on the frame the clip finishes.
        let clip_finished = machine.current_time >= clip_duration;

        machine.current_time = normalize_anim_time(machine.current_time, clip_duration, looped);

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
            blend.to_time = normalize_anim_time(blend.to_time, to_clip_duration, to_looped);
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

#[cfg(test)]
mod tests {
    use super::normalize_anim_time;

    #[test]
    fn looped_negative_time_wraps_forward() {
        // Reverse playback drove time just below 0; rem_euclid wraps it near the
        // clip end instead of leaving it negative (which `%` would do).
        let wrapped = normalize_anim_time(-0.1, 5.0, true);
        assert!(
            (wrapped - 4.9).abs() < 1e-5,
            "expected ~4.9, got {wrapped}"
        );
        assert!(wrapped >= 0.0, "wrapped time must be non-negative");
    }

    #[test]
    fn looped_forward_overflow_wraps() {
        let wrapped = normalize_anim_time(5.2, 5.0, true);
        assert!((wrapped - 0.2).abs() < 1e-5, "expected ~0.2, got {wrapped}");
    }

    #[test]
    fn non_looped_clamps_both_ends() {
        assert_eq!(normalize_anim_time(-1.0, 5.0, false), 0.0);
        assert_eq!(normalize_anim_time(7.0, 5.0, false), 5.0);
        assert_eq!(normalize_anim_time(2.0, 5.0, false), 2.0);
    }

    #[test]
    fn zero_duration_is_safe() {
        // rem_euclid path is skipped for duration == 0; clamp keeps it at 0.
        assert_eq!(normalize_anim_time(1.0, 0.0, true), 0.0);
        assert_eq!(normalize_anim_time(-1.0, 0.0, false), 0.0);
    }
}
