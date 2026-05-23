use gizmo_core::query::{Query, Mut};
use gizmo_core::system::Res;
use gizmo_core::Time;
use gizmo_core::component::{Children, EntityName};
use gizmo_physics_core::Transform;
use crate::player::AnimationPlayer;
use crate::clip::InterpolatedValue;

pub fn animation_system(
    time: Res<Time>,
    mut players: Query<Mut<AnimationPlayer>>,
    names: Query<&EntityName>,
    children: Query<&Children>,
    transforms: Query<Mut<Transform>>,
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

        // Advance time
        player.elapsed_time += dt * player.speed;
        let duration = clip.duration();

        if player.looping {
            if duration > 0.0 {
                player.elapsed_time %= duration;
            }
        } else if player.elapsed_time > duration {
            player.elapsed_time = duration;
            player.playing = false;
        }

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
                            player.target_entities.insert(name.0.clone(), current);
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
            if let Some(&target_id) = player.target_entities.get(&track.target_name) {
                let interpolated = track.sample(player.elapsed_time);
                
                if let Some(mut transform) = transforms.get_mut(target_id) {
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
