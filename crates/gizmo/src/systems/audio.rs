use gizmo_audio::{AudioManager, AudioSource};
use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics::components::{Transform, Velocity};

/// Gelişmiş 3D Uzamsal Ses (Spatial Audio) ve Doppler Etkisi Sistemi
///
/// Bu sistem her frame çalışır ve:
/// 1. `AudioSource` bileşenine sahip tüm objelerin 3D pozisyonlarını ses motoruna yollar.
/// 2. Mesafe tabanlı ses zayıflamasını (Distance Attenuation) uygular.
/// 3. Hızları (`Velocity`) hesaba katarak Doppler Etkisi (Pitch Shift) hesaplar.
pub fn audio_spatial_system(world: &mut World, _dt: f32) {
    let audio_opt = world.get_resource_mut::<AudioManager>();
    let mut audio = match audio_opt {
        Some(m) => m,
        None => return,
    };

    audio.update(); // Biten sesleri temizler

    // Kamerayı (Listener/Dinleyici) bul
    let transforms = world.borrow::<Transform>();

    // Aktif kamerayı bulmak için (Sahnede gizmo_scene::Camera bileşenine sahip objeyi arıyoruz)
    let mut listener_pos = Vec3::ZERO;
    let mut listener_vel = Vec3::ZERO;
    let mut listener_right = Vec3::new(1.0, 0.0, 0.0);

    if let Some(mut query) = world.query::<(&gizmo_renderer::Camera, &Transform)>() {
        for (e, (cam, t)) in query.iter_mut() {
            if cam.primary {
                listener_pos = t.position;
                listener_right = t.rotation.mul_vec3(Vec3::new(1.0, 0.0, 0.0)).normalize();

                // Eğer kameranın bir hızı varsa al
                if let Some(v) = world.borrow::<Velocity>().get(e) {
                    listener_vel = v.linear;
                }
                break;
            }
        }
    }

    // İnsan başı (kulaklar arası mesafe genelde ~0.2 metredir)
    let ear_distance = 0.2;
    let left_ear = listener_pos - (listener_right * (ear_distance / 2.0));
    let right_ear = listener_pos + (listener_right * (ear_distance / 2.0));

    let left_ear_arr = [left_ear.x, left_ear.y, left_ear.z];
    let right_ear_arr = [right_ear.x, right_ear.y, right_ear.z];

    let mut sources = world.borrow_mut::<AudioSource>();
    let velocities = world.borrow::<Velocity>();

    let mut source_ids = Vec::new();
    for (id, _) in sources.iter() {
        source_ids.push(id);
    }

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
        if source._internal_sink_id.is_none() && source.is_3d {
            let sink_id = if source.loop_sound {
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
            source._internal_sink_id = sink_id;
            if let Some(s) = sources.get_mut(id) {
                s._internal_sink_id = sink_id;
            }
        }

        // Eğer ses çalıyorsa güncelle (Mesafe ve Doppler)
        if let Some(sink_id) = source._internal_sink_id {
            if !audio.is_playing(sink_id) {
                if !source.loop_sound {
                    // Tek seferlik ses bittiyse ID'yi temizle
                    if let Some(s) = sources.get_mut(id) {
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
