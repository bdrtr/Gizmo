use gizmo_audio::{AudioManager, AudioSource};
use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics_core::Transform;
use gizmo_physics_rigid::components::Velocity;

/// Should this 3D source be auto-started this frame? `true` only for 3D sources that have not
/// yet been auto-started (`!has_played`) and have no live sink.
///
/// The `has_played` latch is CRITICAL: when a one-shot sound finishes the system sets
/// `_internal_sink_id` back to `None`; without this latch the guard re-triggers the next frame →
/// infinite repetition.
fn should_autostart(source: &AudioSource) -> bool {
    source.is_3d && !source.has_played && source._internal_sink_id.is_none()
}

/// Where the game is listening from: the primary camera's position, its velocity (for Doppler)
/// and its right vector (for the two ears).
///
/// Pulled out of [`audio_spatial_system`] so that the script command path can ask the same
/// question — a `audio.play_3d` from Lua has to place its sound against the same listener the
/// spatial system will update it against, or the sound jumps on its second frame.
#[derive(Debug, Clone, Copy)]
pub struct Listener {
    /// Where the listener is, in world space.
    pub position: Vec3,
    /// How fast it is moving — the Doppler shift's other half.
    pub velocity: Vec3,
    /// Its right vector, which is what separates the ears.
    pub right: Vec3,
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            right: Vec3::new(1.0, 0.0, 0.0),
        }
    }
}

impl Listener {
    /// Half the distance between a pair of human ears, in metres.
    const EAR_DISTANCE: f32 = 0.2;

    /// The two ear positions rodio's spatial player wants, left first.
    pub fn ears(&self) -> ([f32; 3], [f32; 3]) {
        let offset = self.right * (Self::EAR_DISTANCE / 2.0);
        let left = self.position - offset;
        let right = self.position + offset;
        ([left.x, left.y, left.z], [right.x, right.y, right.z])
    }
}

/// Finds the primary camera and reads the listener off it. Falls back to a listener at the origin
/// facing +X when there is no primary camera — a scene with no camera has no listening position,
/// and silence is a worse answer than a sound placed at the origin.
pub fn listener(world: &World) -> Listener {
    let mut listener = Listener::default();
    if let Some(mut query) = world.query::<(&gizmo_renderer::Camera, &Transform)>() {
        for (e, (cam, t)) in query.iter_mut() {
            if cam.primary {
                listener.position = t.position;
                listener.right = t.rotation.mul_vec3(Vec3::new(1.0, 0.0, 0.0)).normalize();
                if let Some(v) = world.borrow::<Velocity>().get(e) {
                    listener.velocity = v.linear;
                }
                break;
            }
        }
    }
    listener
}

/// Advanced 3D Spatial Audio and Doppler Effect System
///
/// This system runs every frame and:
/// 1. Sends the 3D positions of all objects with an `AudioSource` component to the audio engine.
/// 2. Applies distance-based sound attenuation (Distance Attenuation).
/// 3. Computes the Doppler Effect (Pitch Shift) taking velocities (`Velocity`) into account.
///
/// **Opt-in:** `DefaultPlugins` does not register this automatically — running it requires an
/// `AudioManager` resource + an audio output device; not every game wants spatial audio. To use
/// it, add it to your game's schedule by hand, e.g.
/// `schedule.add_system(Phase::Update, audio_spatial_system)`.
pub fn audio_spatial_system(world: &mut World, _dt: f32) {
    let audio_opt = world.get_resource_mut::<AudioManager>();
    let mut audio = match audio_opt {
        Some(m) => m,
        None => return,
    };

    audio.update(); // Biten sesleri temizler

    // Kamerayı (Listener/Dinleyici) bul — script komut yolu da aynı fonksiyonu çağırıyor.
    let transforms = world.borrow::<Transform>();
    let ear = listener(world);
    let listener_pos = ear.position;
    let listener_vel = ear.velocity;
    let (left_ear_arr, right_ear_arr) = ear.ears();

    let source_ids: Vec<u32> = world.borrow::<AudioSource>().entities().collect();
    // SAFETY: exclusive `&mut World`; AudioSource is a distinct component type from the
    // read-only Velocity/Transform queries, so this mutable query never aliases them.
    let mut sources = unsafe { world.borrow_mut_unchecked::<AudioSource>() };
    let velocities = world.borrow::<Velocity>();

    // Tüm ses kaynaklarını güncelle
    for id in source_ids {
        let mut source = if let Some(s) = sources.get_mut(id) {
            s.clone()
        } else {
            continue;
        };

        let t = if let Some(t) = transforms.get(id) {
            t
        } else {
            continue;
        };

        // Eğer ses henüz çalmıyorsa ve otomatik başlatılacaksa
        if should_autostart(&source) {
            let sink_result = if source.loop_sound {
                audio.play_3d_looped(
                    &source.sound_name,
                    [t.position.x, t.position.y, t.position.z],
                    left_ear_arr,
                    right_ear_arr,
                )
            } else {
                audio.play_3d(
                    &source.sound_name,
                    [t.position.x, t.position.y, t.position.z],
                    left_ear_arr,
                    right_ear_arr,
                )
            };
            let sink_id = match sink_result {
                Ok(id) => {
                    // The scene's choice of mixer bus, applied before the sound is a frame old.
                    // Without this line a scene-authored `AudioSource::bus` would be a field the
                    // engine reads back and never acts on.
                    audio.set_sink_bus(id, &source.bus);
                    Some(id)
                }
                Err(e) => {
                    tracing::warn!("3D ses çalınamadı '{}': {}", source.sound_name, e);
                    None
                }
            };
            // Denemeyi bir kez yaptık: hem başarıda hem hatada mandalı kaldır. Böylece
            // biten tek-atış yeniden başlamaz ve eksik/başarısız ses her frame yeniden
            // denenip log'u doldurmaz.
            source._internal_sink_id = sink_id;
            source.has_played = true;
            if let Some(mut s) = sources.get_mut(id) {
                s._internal_sink_id = sink_id;
                s.has_played = true;
            }
        }

        // Eğer ses çalıyorsa güncelle (Mesafe ve Doppler)
        if let Some(sink_id) = source._internal_sink_id {
            if !audio.is_playing(sink_id) {
                if !source.loop_sound {
                    // Tek seferlik ses bittiyse ID'yi temizle
                    if let Some(mut s) = sources.get_mut(id) {
                        s._internal_sink_id = None;
                    }
                }
                continue;
            }

            // 1. Mesafe bazlı Volume
            audio.update_spatial_sink(
                sink_id,
                [t.position.x, t.position.y, t.position.z],
                left_ear_arr,
                right_ear_arr,
                source.max_distance,
                source.volume,
            );

            // 2. Doppler Etkisi (Pitch Shift)
            let emitter_vel = if let Some(v) = velocities.get(id) {
                v.linear
            } else {
                Vec3::ZERO
            };

            let speed_of_sound = 343.0; // m/s havada ses hızı

            // Dinleyici ile kaynak arasındaki yön vektörü
            let diff = t.position - listener_pos;
            let dist = diff.length();

            if dist > 0.1 {
                // Sıfıra bölünmeyi önle
                let dir = diff / dist;

                // Göreceli hızları hesapla (Birbirlerine doğru hızlar pozitiftir)
                let listener_speed_towards_emitter = listener_vel.dot(dir);
                let emitter_speed_towards_listener = emitter_vel.dot(-dir); // Emitter dinleyiciye gidiyorsa negatif yön

                // Doppler formülü: f' = f * (v + v_r) / (v - v_s)
                let mut doppler_factor: f32 = (speed_of_sound + listener_speed_towards_emitter)
                    / (speed_of_sound - emitter_speed_towards_listener).max(1.0);

                // Mantık hatalarını önlemek için kelepçele (Aşırı hızlarda pitch bozulmasını engeller)
                doppler_factor = doppler_factor.clamp(0.5, 2.0);

                let final_pitch = source.pitch * doppler_factor;
                audio.set_pitch(sink_id, final_pitch);
            } else {
                audio.set_pitch(sink_id, source.pitch);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: a finished one-shot 3D sound used to auto-restart every frame because the
    // guard only checked `_internal_sink_id.is_none() && is_3d`, and finishing the sound
    // cleared the sink id — re-satisfying the guard. `has_played` must latch that off.
    #[test]
    fn finished_one_shot_does_not_restart() {
        let mut s = AudioSource::new("boom"); // is_3d=true, has_played=false, sink=None
        assert!(should_autostart(&s), "fresh 3D source should auto-start once");

        // Simulate the system starting playback.
        s.has_played = true;
        s._internal_sink_id = Some(42);
        assert!(!should_autostart(&s), "must not restart while playing");

        // One-shot finishes → system clears the sink id.
        s._internal_sink_id = None;
        assert!(
            !should_autostart(&s),
            "finished one-shot must stay stopped (was the infinite-repeat bug)"
        );
    }

    #[test]
    fn non_3d_source_never_autostarts() {
        let mut s = AudioSource::new("ui_click");
        s.is_3d = false;
        assert!(!should_autostart(&s), "2D sources are not handled by the spatial system");
    }

    /// The system function itself, which had no test at all — the two above cover
    /// `should_autostart`, its helper.
    ///
    /// Its documented precondition is an `AudioManager` resource, and its first act is to return
    /// when there is none. A game without audio — a headless server, a test, a build with the
    /// feature off — must therefore run it harmlessly rather than panic on the missing resource,
    /// and nothing checked that. Found by the sweep in
    /// `crates/gizmo/tests/unmentioned_api.rs`, which reported the system as mentioned nowhere
    /// and covered by nothing; being opt-in explains the first half and not the second.
    #[test]
    fn the_system_is_inert_without_an_audio_manager() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        world.add_component(e, AudioSource::new("boom"));
        audio_spatial_system(&mut world, 1.0 / 60.0); // must not panic
        assert!(
            !world
                .borrow::<AudioSource>()
                .get(e.id())
                .expect("the source is still there")
                .has_played,
            "with no AudioManager nothing may be started"
        );
    }
}
