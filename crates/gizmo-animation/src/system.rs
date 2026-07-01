use gizmo_core::query::{Query, Mut};
use gizmo_core::system::Res;
use gizmo_core::Time;
use gizmo_core::component::{Children, EntityName};
use gizmo_core::entity::Entity;
use gizmo_core::world::Entities;
use gizmo_physics_core::Transform;
use crate::player::AnimationPlayer;
use crate::clip::InterpolatedValue;

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
                    match interpolated {
                        InterpolatedValue::Translation(v) => {
                            transform.position = v;
                        }
                        InterpolatedValue::Rotation(q) => {
                            transform.rotation = q;
                        }
                        InterpolatedValue::Scale(s) => {
                            transform.scale = s;
                        }
                        InterpolatedValue::None => {}
                    }
                }
            }
        }
    }
}
