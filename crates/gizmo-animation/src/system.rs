use gizmo_core::query::{Query, Mut};
use gizmo_core::system::Res;
use gizmo_core::Time;
use gizmo_core::component::{Children, EntityName};
use gizmo_core::entity::Entity;
use gizmo_core::world::Entities;
use gizmo_physics_core::Transform;
use crate::player::AnimationPlayer;
use crate::clip::InterpolatedValue;

/// Write a sampled [`InterpolatedValue`] onto a [`Transform`]. Each channel maps
/// to its own TRS field; scale is applied just like translation and rotation
/// (dropping it here is the classic "scale animation does nothing" bug).
pub fn apply_interpolated(transform: &mut Transform, value: InterpolatedValue) {
    match value {
        InterpolatedValue::Translation(v) => transform.position = v,
        InterpolatedValue::Rotation(q) => transform.rotation = q,
        InterpolatedValue::Scale(s) => transform.scale = s,
        InterpolatedValue::None => {}
    }
}

/// ECS system that advances every [`AnimationPlayer`], resolves track targets by
/// name within each player's hierarchy, and applies sampled values to the
/// targeted [`Transform`]s.
#[tracing::instrument(skip_all, name = "animation_system")]
pub fn animation_system(
    time: Res<Time>,
    entities: Res<Entities>,
    mut commands: gizmo_core::Commands,
    mut players: Query<Mut<AnimationPlayer>>,
    names: Query<&EntityName>,
    children: Query<&Children>,
    mut transforms: Query<(Mut<Transform>, gizmo_core::query::With<crate::player::Animated>)>,
) {
    let dt = time.dt();

    for (root_id, mut player) in players.iter_mut() {
        if !player.playing {
            continue;
        }

        let clip = match &player.clip {
            Some(c) => c.clone(),
            None => continue,
        };

        // Advance time. `advance` guards against a non-finite speed (NaN/Inf)
        // poisoning elapsed_time, wraps when looping, and stops exactly at the
        // clip end (`>=`) when not. Unit-tested in `player.rs`.
        player.advance(dt, clip.duration());

        // Resolve targets if necessary
        // A simple heuristic: if cached map is empty but there are tracks, we need to resolve.
        // Or if we haven't found all of them, but we only do this once to avoid performance hit.
        if player.target_entities.is_empty() && !clip.tracks.is_empty() {
            // `visited` is the cycle guard. This resolution re-runs on EVERY FRAME for as long
            // as `target_entities` stays empty — and a clip whose track names match nothing
            // keeps it empty forever — so a `Children` loop under an animated root was not a
            // one-shot risk: the first frame in which that player is playing never completes.
            //
            // The symptom is a FROZEN frame at flat memory, not an out-of-memory kill. This
            // walk pops before it pushes, so on the simple cycle A→B→C→A the stack holds one
            // id for the whole infinite run; it grows only when an entity inside the cycle has
            // more than one child. Unguarded until 2026-08-30.
            let mut visited = std::collections::HashSet::new();
            visited.insert(root_id);
            let mut stack = vec![root_id];
            
            while let Some(current) = stack.pop() {
                // If it has a name, check if it matches any track
                if let Some(name) = names.get(current) {
                    for track in &clip.tracks {
                        if track.target_name == name.0 {
                            // Recover from a poisoned mutex instead of panicking: a
                            // poisoned lock here would otherwise abort the whole frame.
                            let gen = {
                                let state = entities.state.lock().unwrap_or_else(|e| {
                                    // Surface the silently-recovered lock poisoning: it
                                    // means another thread panicked while holding it.
                                    tracing::warn!(
                                        "[Animation] entities state lock was poisoned; recovering in place"
                                    );
                                    e.into_inner()
                                });
                                // Bounds-check the id: a stale/out-of-range id must skip
                                // gracefully rather than panic on an out-of-bounds index.
                                match state.generations.get(current as usize).copied() {
                                    Some(gen) => gen,
                                    None => {
                                        tracing::trace!(
                                            id = current,
                                            target = %track.target_name,
                                            "[Animation] target resolution: entity id out of range, skipping track"
                                        );
                                        continue;
                                    }
                                }
                            };
                            let entity = Entity::new(current, gen);
                            player.target_entities.insert(name.0.clone(), entity);

                            // Insert the Animated marker component onto the target entity.
                            commands.entity(entity).insert(crate::player::Animated);
                        }
                    }
                }
                
                // Add children to stack — only the ones this walk has not already queued.
                if let Some(child_comp) = children.get(current) {
                    for &child in &child_comp.0 {
                        if visited.insert(child) {
                            stack.push(child);
                        }
                    }
                }
            }

            // One-time-per-clip resolution result. A successful resolve is a useful
            // debug! landmark; a resolve that found nothing re-runs every frame (the
            // cache stays empty), so it is kept at trace! to avoid per-frame spam.
            let resolved = player.target_entities.len();
            if resolved > 0 {
                tracing::debug!(
                    resolved,
                    tracks = clip.tracks.len(),
                    clip = %clip.name,
                    "[Animation] resolved track targets in hierarchy"
                );
            } else {
                tracing::trace!(
                    tracks = clip.tracks.len(),
                    clip = %clip.name,
                    "[Animation] no track targets resolved this frame"
                );
            }
        }

        // Apply animations. Aggregate per-track outcomes and emit a single
        // per-player trace! rather than logging inside the hot per-track loop.
        let mut applied = 0usize;
        let mut skipped_dead = 0usize;
        let mut missing_transform = 0usize;
        for track in &clip.tracks {
            if let Some(&target_entity) = player.target_entities.get(&track.target_name) {
                // Check if the target entity is still alive and matches the generation in the world
                if !entities.is_alive(target_entity) {
                    skipped_dead += 1;
                    continue;
                }
                let target_id = target_entity.id();
                let interpolated = track.sample(player.elapsed_time);

                if let Some((mut transform, _)) = transforms.get_mut(target_id) {
                    apply_interpolated(&mut transform, interpolated);
                    applied += 1;
                } else {
                    missing_transform += 1;
                }
            }
        }
        tracing::trace!(
            applied,
            skipped_dead,
            missing_transform,
            elapsed = player.elapsed_time,
            "[Animation] applied sampled tracks"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{AnimationClip, Interpolation, Keyframes, Track};
    use gizmo_core::component::Children;
    use gizmo_core::system::Schedule;
    use gizmo_math::Vec3;
    use std::sync::mpsc::RecvTimeoutError;

    /// Runs `f` on a worker thread and fails if it has not finished within `secs`.
    ///
    /// The guard below protects against a walk that never ends, and a walk that never ends has
    /// no assertion to disagree with: the natural test does not fail, it hangs, and a suite
    /// that hangs covers less than one that goes red — the same argument `CLAUDE.md` makes for
    /// `--no-fail-fast`. The `World` is built inside the closure so nothing crosses a thread
    /// boundary.
    ///
    /// A leftover worker is not free: on a timeout it keeps resolving targets for the rest of
    /// the binary's run, so a red here wants investigating rather than ignoring.
    fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        match rx.recv_timeout(std::time::Duration::from_secs(secs)) {
            Ok(value) => value,
            // Told apart on purpose: `expect` here would report a panicking worker — whose own
            // message has already been printed — as a walk that looped for `secs` seconds.
            Err(RecvTimeoutError::Timeout) => {
                panic!("the animation frame did not finish within {secs}s — a `Children` cycle looped")
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!("the worker panicked before answering; its own message is above")
            }
        }
    }

    /// A `Children` cycle under an animated root must not cost the frame.
    ///
    /// Resolution re-runs every frame while `target_entities` is empty, so this walk is inside
    /// the per-frame budget rather than on a load path — a cyclic scene froze the engine on the
    /// first tick after it loaded, not at some later user action.
    ///
    /// The cycle is written straight into `Children`: `add_child` refuses to build one, but
    /// `SceneData::instantiate_entities` writes a file's parent edges verbatim, and an animated
    /// rig is exactly the kind of thing that arrives out of a file.
    #[test]
    fn a_children_cycle_under_an_animated_root_does_not_hang_the_frame() {
        let resolved = within(10, || {
            let mut world = gizmo_core::world::World::new();
            let mut schedule = Schedule::new();
            crate::register(&mut world, &mut schedule);
            world.register_component_type::<Children>();
            world.register_component_type::<EntityName>();
            world.register_component_type::<Transform>();
            // The schedule's `dt` argument and the `Time` resource are separate: the system
            // reads the resource, so a default `Time` would hand it dt = 0 and the player
            // would not advance at all.
            let mut time = Time::new();
            time.update(0.016);
            world.insert_resource(time);

            let root = world.spawn();
            let bone = world.spawn();

            let track = Track::new(
                "bone",
                vec![0.0, 1.0],
                Keyframes::Scale(vec![Vec3::ONE, Vec3::splat(2.0)]),
            )
            .expect("valid track");
            world.add_component(
                root,
                AnimationPlayer {
                    clip: Some(std::sync::Arc::new(AnimationClip {
                        name: "cyclic".into(),
                        tracks: vec![track],
                    })),
                    ..Default::default()
                },
            );
            world.add_component(bone, EntityName("bone".into()));
            world.add_component(bone, Transform::default());

            // root -> bone -> root. Written directly, bypassing `add_child`'s refusal.
            world.add_component(root, Children(vec![bone.id()]));
            world.add_component(bone, Children(vec![root.id()]));

            schedule.run(&mut world, 0.016);

            let players = world.borrow::<AnimationPlayer>();
            let player = players.get(root.id()).expect("the player is still there");
            (player.target_entities.len(), player.elapsed_time)
        });

        let (targets, elapsed) = resolved;
        // Terminating at all is the assertion. These two are the evidence that the frame did
        // the work rather than bailing out early: a system skipped for a missing resource would
        // leave both at zero and pass this test even with the guard deleted.
        assert_eq!(targets, 1, "the walk reached `bone` through the cycle and resolved it");
        assert!(elapsed > 0.0, "the player advanced, so the system really ran");
    }

    /// End-to-end (sample -> apply) proof that a non-uniform scale track reaches
    /// the output pose. This FAILS if `apply_interpolated` drops the `Scale`
    /// channel (the historical "scale tracks ignored" bug).
    #[test]
    fn sampled_scale_reaches_transform() {
        let track = Track::new(
            "bone",
            vec![0.0, 1.0],
            Keyframes::Scale(vec![Vec3::new(1.0, 1.0, 1.0), Vec3::new(2.0, 4.0, 8.0)]),
        )
        .expect("valid track")
        .with_interpolation(Interpolation::Linear);

        let mut transform = Transform::default();
        assert_eq!(transform.scale, Vec3::ONE, "sanity: starts at unit scale");

        apply_interpolated(&mut transform, track.sample(0.5));

        assert!(
            (transform.scale - Vec3::new(1.5, 2.5, 4.5)).length() < 1e-4,
            "non-uniform scale must reach the transform, got {:?}",
            transform.scale
        );
    }

    #[test]
    fn apply_translation_and_rotation_channels() {
        let mut t = Transform::default();
        apply_interpolated(&mut t, InterpolatedValue::Translation(Vec3::new(5.0, 6.0, 7.0)));
        assert_eq!(t.position, Vec3::new(5.0, 6.0, 7.0));
        apply_interpolated(&mut t, InterpolatedValue::None);
        assert_eq!(t.position, Vec3::new(5.0, 6.0, 7.0), "None is a no-op");
    }
}
