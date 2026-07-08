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
            let mut stack = vec![root_id];
            
            while let Some(current) = stack.pop() {
                // If it has a name, check if it matches any track
                if let Some(name) = names.get(current) {
                    for track in &clip.tracks {
                        if track.target_name == name.0 {
                            // Recover from a poisoned mutex instead of panicking: a
                            // poisoned lock here would otherwise abort the whole frame.
                            let gen = {
                                let state =
                                    entities.state.lock().unwrap_or_else(|e| e.into_inner());
                                // Bounds-check the id: a stale/out-of-range id must skip
                                // gracefully rather than panic on an out-of-bounds index.
                                match state.generations.get(current as usize).copied() {
                                    Some(gen) => gen,
                                    None => continue,
                                }
                            };
                            let entity = Entity::new(current, gen);
                            player.target_entities.insert(name.0.clone(), entity);

                            // Insert the Animated marker component onto the target entity.
                            commands.entity(entity).insert(crate::player::Animated);
                        }
                    }
                }
                
                // Add children to stack
                if let Some(child_comp) = children.get(current) {
                    for &child in &child_comp.0 {
                        stack.push(child);
                    }
                }
            }
        }

        // Apply animations
        for track in &clip.tracks {
            if let Some(&target_entity) = player.target_entities.get(&track.target_name) {
                // Check if the target entity is still alive and matches the generation in the world
                if !entities.is_alive(target_entity) {
                    continue;
                }
                let target_id = target_entity.id();
                let interpolated = track.sample(player.elapsed_time);
                
                if let Some((mut transform, _)) = transforms.get_mut(target_id) {
                    apply_interpolated(&mut transform, interpolated);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{Interpolation, Keyframes, Track};
    use gizmo_math::Vec3;

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
