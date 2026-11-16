use gizmo_core::component::{Children, EntityName};
use gizmo_core::entity::Entity;
use gizmo_core::World;
use gizmo_physics_core::Transform;
use std::sync::Arc;

use crate::clip::{AnimationClip, InterpolatedValue};
use crate::player::AnimationPlayer;
use crate::property::PropertyRegistry;

/// Write a sampled [`InterpolatedValue`] onto a [`Transform`]. Each channel maps
/// to its own TRS field; scale is applied just like translation and rotation
/// (dropping it here is the classic "scale animation does nothing" bug).
pub fn apply_interpolated(transform: &mut Transform, value: InterpolatedValue) {
    match value {
        InterpolatedValue::Translation(v) => transform.position = v,
        InterpolatedValue::Rotation(q) => transform.rotation = q,
        InterpolatedValue::Scale(s) => transform.scale = s,
        // A value channel belongs to a property track, which the driver hands to the
        // `PropertyRegistry`, never to a `Transform`.
        InterpolatedValue::None
        | InterpolatedValue::Scalar(_)
        | InterpolatedValue::Vector2(_)
        | InterpolatedValue::Vector3(_)
        | InterpolatedValue::Vector4(_) => {}
    }
}

/// One frame of the animation clock, as every frame driver runs it: the [`Tween`](crate::tween::Tween)s
/// ([`tween_system`](crate::tween::tween_system)), then the keyed players
/// ([`keyed_animation_system`]) — so a keyed `Transform` wins over a tween on the same entity.
///
/// It is the part of the frame's clocks that needs no renderer, kept in one function so the drivers
/// cannot drift apart: `gizmo`'s `advance_frame_clocks` (the windowed frame and `HeadlessFrames`)
/// calls it after the colour tweens, and `gizmo_app`'s headless loop — a server, a build without
/// `render` — calls it after each frame's commands. Until 2026-11-16 only the first existed, and a
/// headless build moved no tween and played no clip: a door keyed to open stayed shut on a server.
///
/// `dt` is the game clock's (`Time::dt`), so time scale and pause apply.
pub fn advance_animation(world: &mut World, dt: f32) {
    crate::tween::tween_system(world, dt);
    keyed_animation_system(world, dt);
}

/// The keyed animation driver: every [`AnimationPlayer`] advanced by `dt` while it plays, and its
/// clip's tracks sampled at its playhead onto what they target — **playing or not**, so an editor
/// scrubbing a paused player's `elapsed_time` moves what it animates.
///
/// A track targets the player's own entity when its `target_name` is empty, and otherwise the
/// first entity of that [`EntityName`] in the player's hierarchy (the player's entity and its
/// `Children`, walked once per clip and cached in `target_entities`, again whenever a cached target
/// has gone). The walk is guarded against a `Children` cycle — a scene file writes parent edges
/// verbatim — which until 2026-08-30 froze the frame.
///
/// A transform track ([`Track::property`](crate::clip::Track::property) `None`) writes the target's
/// `Transform`. A property track writes the component field its name registers in the world's
/// [`PropertyRegistry`]: those are gathered across **all** players and written after the
/// transforms, one query per property, a field written only when its value moves. Two players
/// keying the same field: the later one wins, as with transforms. A name the registry does not
/// know, or keyframes of another kind than the property, is warned about once per clip and
/// property and skipped; a target without the component is skipped silently, as one without a
/// `Transform` is. With no registry in the world an empty one is inserted, so such a track warns
/// rather than vanishing.
///
/// Every frame driver runs it through [`advance_animation`], on the game clock: the game's and the
/// editor's draw paths (`gizmo::systems::render::advance_frame_clocks`) **after
/// `color_tween_system`** — a keyed track wins over a `ColorTween` on the same field, deliberately —
/// and `gizmo_app`'s headless loop.
pub fn keyed_animation_system(world: &mut World, dt: f32) {
    let players: Vec<Entity> = world.query::<(Entity, &AnimationPlayer)>().iter().map(|(e, _)| e).collect();
    // Property samples, in player order then track order: (clip, track index, target, value).
    let mut properties: Vec<(Arc<AnimationClip>, usize, Entity, InterpolatedValue)> = Vec::new();
    // Transform tracks holding value keyframes (a hand-edited or deserialised clip).
    let mut misfiled: Vec<(Arc<AnimationClip>, usize)> = Vec::new();
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
        for (i, track) in clip.tracks.iter().enumerate() {
            let target = if track.target_name.is_empty() { Some(root) } else { targets.get(&track.target_name).copied() };
            let Some(target) = target else { continue };
            if track.property.is_some() {
                match track.sample(time) {
                    InterpolatedValue::None => {}
                    value => properties.push((clip.clone(), i, target, value)),
                }
            } else if track.keyframes.kind().is_some() {
                misfiled.push((clip.clone(), i));
            } else if let Some(mut transform) = transforms.get_mut(target) {
                apply_interpolated(&mut transform, track.sample(time));
            }
        }
        drop(transforms);
        if let Some(mut player) = world.query_mut::<&mut AnimationPlayer>().get_mut(root) {
            player.target_entities = targets;
        }
    }
    if properties.is_empty() && misfiled.is_empty() {
        return;
    }
    if world.get_resource::<PropertyRegistry>().is_none() {
        world.insert_resource(PropertyRegistry::default());
    }
    world.resource_scope::<PropertyRegistry, _, _>(|world, registry| {
        for (clip, i) in &misfiled {
            registry.warn_once(
                &format!("{}/#{i}", clip.name),
                format_args!("clip `{}`: transform track {i} holds value keyframes; skipped", clip.name),
            );
        }
        // Each sample to its registry entry; what does not resolve warns once and is dropped.
        let mut resolved: Vec<(usize, Entity, InterpolatedValue)> = Vec::with_capacity(properties.len());
        for (clip, i, target, value) in &properties {
            let track = &clip.tracks[*i];
            let name = track.property.as_deref().unwrap_or_default();
            match registry.lookup(name) {
                Some((entry, kind)) if track.keyframes.kind() == Some(kind) => resolved.push((entry, *target, *value)),
                Some((_, kind)) => {
                    registry.warn_once(
                        &format!("{}/{name}", clip.name),
                        format_args!("clip `{}`: `{name}` is a {kind} property, keyed with other keyframes; skipped", clip.name),
                    );
                }
                None => {
                    registry.warn_once(
                        &format!("{}/{name}", clip.name),
                        format_args!("clip `{}`: no animatable property `{name}` is registered; skipped", clip.name),
                    );
                }
            }
        }
        // Grouped by field and target, keeping player order within a group (a stable sort). A lane
        // track (`Sprite.color.w`) and a whole-vector track (`Sprite.color`) are one field: their
        // samples fold, in player order, into one value of the whole vector — so the later player
        // wins whichever of the two it keys, and the field is written, and stamped as a change,
        // once. Of plain samples only the last is kept, for the same reason.
        let mut fields: Vec<(usize, Entity, Option<usize>, InterpolatedValue)> = resolved
            .iter()
            .map(|&(entry, target, value)| {
                let (field, lane) = registry.field_of(entry);
                (field, target, lane, value)
            })
            .collect();
        fields.sort_by_key(|&(field, target, _, _)| (field, target));
        let mut entries: Vec<usize> = Vec::with_capacity(fields.len());
        let mut items: Vec<(Entity, InterpolatedValue)> = Vec::with_capacity(fields.len());
        let mut start = 0;
        while start < fields.len() {
            let (field, target, _, _) = fields[start];
            let end = start + fields[start..].iter().take_while(|s| s.0 == field && s.1 == target).count();
            let group = &fields[start..end];
            start = end;
            let value = if group.iter().all(|s| s.2.is_none()) {
                Some(group[group.len() - 1].3)
            } else {
                // Only what follows the last whole-vector sample matters; lanes before it are
                // overwritten by it.
                let from = group.iter().rposition(|s| s.2.is_none());
                let mut value = match from {
                    Some(k) => Some(group[k].3),
                    None => registry.get_at(field, world, target),
                };
                for s in &group[from.map_or(0, |k| k + 1)..] {
                    if let (Some(lane), InterpolatedValue::Scalar(v)) = (s.2, s.3) {
                        value = value.and_then(|whole| crate::property::with_lane(whole, lane, v));
                    }
                }
                value
            };
            if let Some(value) = value {
                entries.push(field);
                items.push((target, value));
            }
        }
        let mut start = 0;
        while start < entries.len() {
            let end = start + entries[start..].iter().take_while(|&&e| e == entries[start]).count();
            registry.apply_at(entries[start], world, &items[start..end]);
            start = end;
        }
    });
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

            keyed_animation_system(&mut world, 0.016);

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

    #[derive(Clone, Debug, PartialEq)]
    struct Glow {
        power: f32,
        tint: gizmo_math::Vec4,
    }
    gizmo_core::impl_component!(Glow);

    fn glow_registry() -> PropertyRegistry {
        let mut r = PropertyRegistry::default();
        r.register::<Glow, f32>("Glow.power", |g| g.power, |g, v| g.power = v).unwrap();
        r.register_lanes::<Glow, gizmo_math::Vec4>("Glow.tint", |g| g.tint, |g, v| g.tint = v).unwrap();
        r
    }

    fn glow() -> Glow {
        Glow { power: 0.0, tint: gizmo_math::Vec4::ONE }
    }

    fn player_of(tracks: Vec<Track>, playing: bool) -> AnimationPlayer {
        AnimationPlayer {
            clip: Some(std::sync::Arc::new(AnimationClip { name: "clip".into(), tracks })),
            playing,
            looping: false,
            ..Default::default()
        }
    }

    fn power_track(target: &str, from: f32, to: f32) -> Track {
        Track::property(target, "Glow.power", vec![0.0, 1.0], Keyframes::Scalar(vec![from, to])).expect("valid")
    }

    fn power(world: &World, e: Entity) -> f32 {
        world.get::<Glow>(e).expect("has Glow").power
    }

    /// A property track writes its field while the player plays, and — paused — wherever the
    /// playhead is put, as a transform track does: what an editor's scrub relies on.
    #[test]
    fn property_track_drives_component_while_playing_and_paused_scrub() {
        let mut world = World::new();
        world.insert_resource(glow_registry());
        let e = world.spawn();
        world.add_component(e, glow());
        let alpha = Track::property("", "Glow.tint.w", vec![0.0, 1.0], Keyframes::Scalar(vec![1.0, 0.0])).unwrap();
        world.add_component(e, player_of(vec![power_track("", 0.0, 10.0), alpha], true));

        keyed_animation_system(&mut world, 0.25);
        assert_eq!(power(&world, e), 2.5);
        assert_eq!(world.get::<Glow>(e).unwrap().tint, gizmo_math::Vec4::new(1.0, 1.0, 1.0, 0.75), "the lane alone");

        if let Some(mut p) = world.query_mut::<&mut AnimationPlayer>().get_mut(e) {
            p.playing = false;
            p.elapsed_time = 0.8;
        }
        keyed_animation_system(&mut world, 0.25);
        assert_eq!(power(&world, e), 8.0, "paused, the scrubbed playhead is sampled");
        keyed_animation_system(&mut world, 0.25);
        assert_eq!(power(&world, e), 8.0, "and it stays");
    }

    #[test]
    fn named_child_property_target_resolves() {
        let mut world = World::new();
        world.insert_resource(glow_registry());
        let root = world.spawn();
        let lamp = world.spawn();
        world.add_component(lamp, EntityName("lamp".into()));
        world.add_component(lamp, glow());
        world.add_component(root, glow());
        world.add_component(root, Children(vec![lamp]));
        world.add_component(root, player_of(vec![power_track("lamp", 2.0, 4.0)], false));
        keyed_animation_system(&mut world, 0.0);
        assert_eq!(power(&world, lamp), 2.0);
        assert_eq!(power(&world, root), 0.0, "the root is not the target");
    }

    /// A property nothing registered is skipped (warned once) and the clip's transform tracks still
    /// play — with a registry, and with none in the world at all.
    #[test]
    fn unknown_property_skipped_and_transform_tracks_still_apply() {
        for with_registry in [true, false] {
            let mut world = World::new();
            if with_registry {
                world.insert_resource(glow_registry());
            }
            let e = world.spawn();
            world.add_component(e, glow());
            world.add_component(e, Transform::default());
            let unknown = Track::property("", "Glow.nope", vec![0.0, 1.0], Keyframes::Scalar(vec![1.0, 2.0])).unwrap();
            // A property track keyed with another kind than the property's.
            let wrong_kind = Track::property("", "Glow.power", vec![0.0, 1.0], Keyframes::Vector2(vec![gizmo_math::Vec2::ONE; 2])).unwrap();
            // A transform track holding value keys, as only a hand-edited file can build.
            let mut misfiled = Track::new("", vec![0.0, 1.0], Keyframes::Scale(vec![Vec3::ONE; 2])).unwrap();
            misfiled.keyframes = Keyframes::Scalar(vec![3.0, 3.0]);
            let moved = Track::new("", vec![0.0, 1.0], Keyframes::Translation(vec![Vec3::ZERO, Vec3::X])).unwrap();
            world.add_component(e, player_of(vec![unknown, wrong_kind, misfiled, moved], true));
            keyed_animation_system(&mut world, 0.5);
            keyed_animation_system(&mut world, 0.25);
            assert_eq!(world.get::<Transform>(e).unwrap().position, Vec3::X * 0.75, "registry: {with_registry}");
            assert_eq!(power(&world, e), 0.0, "nothing wrote the field");
            let mut registry = world.get_resource_mut::<PropertyRegistry>().expect("there is a registry afterwards");
            assert!(!registry.warn_once("clip/Glow.nope", ""), "the unknown name was warned about already");
            assert!(!registry.warn_once("clip/#2", ""), "the misfiled track too");
            if with_registry {
                assert!(!registry.warn_once("clip/Glow.power", ""), "and the kind mismatch");
            }
        }
    }

    /// Two players keying one field: the later one in the players' query order wins, as it does for
    /// transforms, and the earlier one's value is never written.
    #[test]
    fn two_players_same_field_later_wins() {
        let mut world = World::new();
        world.insert_resource(glow_registry());
        let lamp = world.spawn();
        world.add_component(lamp, EntityName("lamp".into()));
        world.add_component(lamp, glow());
        let a = world.spawn();
        let b = world.spawn();
        world.add_component(a, Children(vec![lamp]));
        world.add_component(b, Children(vec![lamp]));
        world.add_component(a, player_of(vec![power_track("lamp", 1.0, 1.0)], false));
        world.add_component(b, player_of(vec![power_track("lamp", 7.0, 7.0)], false));
        let order: Vec<Entity> = world.query::<(Entity, &AnimationPlayer)>().iter().map(|(e, _)| e).collect();
        let last = *order.last().unwrap();
        let t1 = world.begin_change_frame(0);
        world.begin_change_frame(t1);
        keyed_animation_system(&mut world, 0.0);
        assert_eq!(power(&world, lamp), if last == b { 7.0 } else { 1.0 });
        world.begin_change_frame(t1 + 1);
        keyed_animation_system(&mut world, 0.0);
        let changed = world.query_filtered::<&Glow, gizmo_core::query::Changed<Glow>>().iter().count();
        assert_eq!(changed, 0, "held at the winner's value, the field is not rewritten every frame");
    }

    fn tint_track(target: &str, tint: gizmo_math::Vec4) -> Track {
        Track::property(target, "Glow.tint", vec![0.0], Keyframes::Vector4(vec![tint])).expect("valid")
    }

    fn alpha_track(target: &str, a: f32) -> Track {
        Track::property(target, "Glow.tint.w", vec![0.0], Keyframes::Scalar(vec![a])).expect("valid")
    }

    /// A whole-vector track and a lane track on one field are one field: the later player wins
    /// whichever of the two it keys — not the one registered last — and a held value is written
    /// once, not once by each and stamped as a change every frame.
    #[test]
    fn vector_and_lane_tracks_on_one_field_compose_in_player_order() {
        use gizmo_math::Vec4;
        for lane_is_later in [false, true] {
            let mut world = World::new();
            world.insert_resource(glow_registry());
            let lamp = world.spawn();
            world.add_component(lamp, EntityName("lamp".into()));
            world.add_component(lamp, glow());
            let a = world.spawn();
            let b = world.spawn();
            world.add_component(a, Children(vec![lamp]));
            world.add_component(b, Children(vec![lamp]));
            world.add_component(a, player_of(vec![], false));
            world.add_component(b, player_of(vec![], false));
            let queried: Vec<Entity> = world.query::<(Entity, &AnimationPlayer)>().iter().map(|(e, _)| e).collect();
            assert_eq!(queried.len(), 2);
            let (first, last) = (queried[0], queried[1]);
            let whole = tint_track("lamp", Vec4::new(0.5, 0.5, 0.5, 1.0));
            let lane = alpha_track("lamp", 0.2);
            let (early, late) = if lane_is_later { (whole, lane) } else { (lane, whole) };
            world.add_component(first, player_of(vec![early], false));
            world.add_component(last, player_of(vec![late], false));

            let t1 = world.begin_change_frame(0);
            world.begin_change_frame(t1);
            keyed_animation_system(&mut world, 0.0);
            let tint = world.get::<Glow>(lamp).unwrap().tint;
            let expected = if lane_is_later { Vec4::new(0.5, 0.5, 0.5, 0.2) } else { Vec4::new(0.5, 0.5, 0.5, 1.0) };
            assert_eq!(tint, expected, "the later player wins (lane later: {lane_is_later})");

            world.begin_change_frame(t1 + 1);
            keyed_animation_system(&mut world, 0.0);
            let changed = world.query_filtered::<&Glow, gizmo_core::query::Changed<Glow>>().iter().count();
            assert_eq!(changed, 0, "a held composition is not rewritten every frame (lane later: {lane_is_later})");
        }
    }

    /// Lane tracks alone on one field keep the other lanes and both reach it.
    #[test]
    fn two_lane_tracks_on_one_field_both_apply() {
        use gizmo_math::Vec4;
        let mut world = World::new();
        world.insert_resource(glow_registry());
        let e = world.spawn();
        world.add_component(e, glow());
        let red = Track::property("", "Glow.tint.x", vec![0.0], Keyframes::Scalar(vec![0.25])).unwrap();
        world.add_component(e, player_of(vec![red, alpha_track("", 0.5)], false));
        keyed_animation_system(&mut world, 0.0);
        assert_eq!(world.get::<Glow>(e).unwrap().tint, Vec4::new(0.25, 1.0, 1.0, 0.5));
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
