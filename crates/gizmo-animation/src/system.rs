use gizmo_core::component::{Children, EntityName};
use gizmo_core::entity::Entity;
use gizmo_core::World;
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

/// The transform-track animation driver: every [`AnimationPlayer`] advanced by `dt` while it plays,
/// and its clip's tracks sampled at its playhead onto the `Transform`s they target — **playing or
/// not**, so an editor scrubbing a paused player's `elapsed_time` moves what it animates.
///
/// A track targets the player's own entity when its `target_name` is empty, and otherwise the
/// first entity of that [`EntityName`] in the player's hierarchy (the player's entity and its
/// `Children`, walked once per clip and cached in `target_entities`, again whenever a cached target
/// has gone). The walk is guarded against a `Children` cycle — a scene file writes parent edges
/// verbatim — which until 2026-08-30 froze the frame.
///
/// The engine's frame runs it (`gizmo::systems::render::prepare_frame`, beside the tweens and the
/// skeletal drivers), on the game clock, in both the game's and the editor's draw paths.
pub fn transform_animation_system(world: &mut World, dt: f32) {
    let players: Vec<Entity> = world.query::<(Entity, &AnimationPlayer)>().iter().map(|(e, _)| e).collect();
    for root in players {
        let Some((clip, time, mut targets)) = ({
            let mut q = world.query_mut::<&mut AnimationPlayer>();
            q.get_mut(root).and_then(|mut player| {
                let clip = player.clip.clone()?;
                if player.playing {
                    player.advance(dt, clip.duration());
                }
                Some((clip, player.elapsed_time, std::mem::take(&mut player.target_entities)))
            })
        }) else {
            continue;
        };

        let stale = clip.tracks.iter().any(|t| {
            !t.target_name.is_empty() && targets.get(&t.target_name).is_none_or(|e| !world.is_alive(*e))
        });
        if stale {
            targets = resolve_targets(world, root, &clip);
        }

        let mut transforms = world.query_mut::<&mut Transform>();
        for track in &clip.tracks {
            let target = if track.target_name.is_empty() { Some(root) } else { targets.get(&track.target_name).copied() };
            if let Some(mut transform) = target.and_then(|e| transforms.get_mut(e)) {
                apply_interpolated(&mut transform, track.sample(time));
            }
        }
        drop(transforms);
        if let Some(mut player) = world.query_mut::<&mut AnimationPlayer>().get_mut(root) {
            player.target_entities = targets;
        }
    }
}

/// Each named track's target in `root`'s hierarchy: a walk of `Children` from `root`, every entity
/// visited once (a cycle cannot hold it), the first of each name winning.
fn resolve_targets(world: &World, root: Entity, clip: &crate::clip::AnimationClip) -> std::collections::HashMap<String, Entity> {
    let names = world.query::<&EntityName>();
    let children = world.query::<&Children>();
    let mut found = std::collections::HashMap::new();
    let mut visited = std::collections::HashSet::new();
    visited.insert(root);
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if let Some(name) = names.get(current) {
            if clip.tracks.iter().any(|t| t.target_name == name.0) {
                found.entry(name.0.clone()).or_insert(current);
            }
        }
        if let Some(c) = children.get(current) {
            for &child in &c.0 {
                if world.is_alive(child) && visited.insert(child) {
                    stack.push(child);
                }
            }
        }
    }
    tracing::debug!(resolved = found.len(), tracks = clip.tracks.len(), clip = %clip.name, "[Animation] resolved track targets");
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{AnimationClip, Interpolation, Keyframes, Track};
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
    ///
    /// The named bone sits outside the cycle and the walk enters the cycle first, so it reaches
    /// `bone` only if it gets out again — the evidence the guard works, not just that it ends.
    #[test]
    fn a_children_cycle_under_an_animated_root_does_not_hang_the_frame() {
        let resolved = within(10, || {
            let mut world = gizmo_core::world::World::new();

            let root = world.spawn();
            let bone = world.spawn();
            let loop_a = world.spawn();
            let loop_b = world.spawn();

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
            // `loop_a`/`loop_b` carry no `EntityName`, so no track can match them and the
            // pre-fix spin queues no commands. See the note above.

            // root -> {bone, loop_a}, and loop_a -> loop_b -> loop_a. Written directly,
            // bypassing `add_child`'s refusal. `loop_a` is listed last, so the walk — which
            // pops — enters the cycle FIRST and only reaches `bone` if it gets out again.
            world.add_component(root, Children(vec![bone, loop_a]));
            world.add_component(loop_a, Children(vec![loop_b]));
            world.add_component(loop_b, Children(vec![loop_a]));

            transform_animation_system(&mut world, 0.016);

            let players = world.query::<&AnimationPlayer>();
            let player = players.get(root).expect("the player is still there");
            (player.target_entities.len(), player.elapsed_time)
        });

        let (targets, elapsed) = resolved;
        // Terminating at all is the assertion. These two are the evidence that the frame did
        // the work rather than bailing out early: a system skipped for a missing resource would
        // leave both at zero and pass this test even with the guard deleted.
        assert_eq!(targets, 1, "the walk left the cycle and went on to resolve `bone`");
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
