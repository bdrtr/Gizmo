use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::physics::components::Transform;
use gizmo::prelude::*;

pub fn handle_camera(
    world: &mut World,
    state: &mut StudioState,
    dt: f32,
    input: &Input,
    look_delta: Option<gizmo::math::Vec2>,
    pan_delta: Option<gizmo::math::Vec2>,
    orbit_delta: Option<gizmo::math::Vec2>,
    scroll_delta: f32,
) {
    // Editör kamera değişkenlerini world'dan oku
    let mut camera_speed = 8.0;
    let mut camera_focus_distance = 10.0;
    let mut is_playing = false;
    let mut focus_target = None;
    if let Some(es) = world.get_resource::<EditorState>() {
        camera_speed = es.prefs.camera_speed;
        camera_focus_distance = es.prefs.camera_focus_distance;
        is_playing = es.is_playing();
        focus_target = es.camera.focus_target;
    }

    // Editor Camera WASD Controller
    // SAFETY: exclusive `&mut World`; Transform and Camera are distinct component types.
    let mut transforms = unsafe { world.borrow_mut_unchecked::<Transform>() };
    let mut cameras = unsafe { world.borrow_mut_unchecked::<gizmo::renderer::components::Camera>() };
    {
        if let (Some(mut t), Some(mut cam)) = (
            transforms.get_mut(state.editor_camera),
            cameras.get_mut(state.editor_camera),
        ) {
            // 1. Mouse Look (Egui üzerinden gelen delta okuması)
            if !is_playing {
                if let Some(delta) = look_delta {
                    let sensitivity = 0.003;

                    cam.yaw += delta.x * sensitivity;
                    cam.pitch -= delta.y * sensitivity;
                    // Pitch sınırlaması fonksiyonun sonunda yapılıyor
                }
            }

            // 2. Serbest Uçuş (WASD + Q/E)
            let speed = if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftLeft as u32) {
                camera_speed * 2.5
            } else {
                camera_speed
            };

            let forward = cam.get_front();
            let right = forward
                .cross(gizmo::math::Vec3::new(0.0, 1.0, 0.0))
                .normalize();
            let up = gizmo::math::Vec3::new(0.0, 1.0, 0.0);

            let mut move_dir = gizmo::math::Vec3::ZERO;

            if !is_playing {
                // Kamera nereye bakıyorsa ORAYA ileri git
                if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyW as u32) {
                    move_dir += forward;
                }
                if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyS as u32) {
                    move_dir -= forward;
                }
                if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyA as u32) {
                    move_dir -= right;
                }
                if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyD as u32) {
                    move_dir += right;
                }
                // Dünyaya göre yukarı/aşağı tırmanış
                if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyE as u32) {
                    move_dir += up;
                }
                if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyQ as u32) {
                    move_dir -= up;
                }
            }

            // Eğer kullanıcı manuel olarak kamerayı hareket ettirirse, odaklanmayı iptal et
            if move_dir.length_squared() > 0.0 || look_delta.is_some() || pan_delta.is_some() || orbit_delta.is_some() || scroll_delta != 0.0 {
                focus_target = None;
                if let Some(mut es) = world.get_resource_mut::<EditorState>() {
                    es.camera.focus_target = None;
                }
            }

            if let Some(target) = focus_target {
                let diff = target - t.position;
                let dist_to_target = diff.length();
                let dir = if dist_to_target > 0.001 { diff / dist_to_target } else { forward };
                
                let desired_pitch = dir.y.asin();
                let desired_yaw = dir.z.atan2(dir.x);
                
                let mut yaw_diff = desired_yaw - cam.yaw;
                while yaw_diff > std::f32::consts::PI { yaw_diff -= std::f32::consts::TAU; }
                while yaw_diff < -std::f32::consts::PI { yaw_diff += std::f32::consts::TAU; }
                
                // Yumuşak kamera dönüşü
                cam.yaw += yaw_diff * (8.0 * dt).clamp(0.0, 1.0);
                cam.pitch += (desired_pitch - cam.pitch) * (8.0 * dt).clamp(0.0, 1.0);
                
                // Güncel bakış açısına göre hedef noktayı belirle
                let current_forward = cam.get_front();
                let desired_pos = target - current_forward * camera_focus_distance;
                
                // Pozisyonu yumuşakça lerple
                t.position = t.position.lerp(desired_pos, 8.0 * dt);
                
                if t.position.distance(desired_pos) < 0.1 && yaw_diff.abs() < 0.05 && (desired_pitch - cam.pitch).abs() < 0.05 {
                    if let Some(mut es) = world.get_resource_mut::<EditorState>() {
                        es.camera.focus_target = None;
                    }
                }
            } else {
                t.position += move_dir.normalize_or_zero() * (speed * dt);
            }

            // 3. Orta Tık Pan (Kaydırma)
            if let Some(pan) = pan_delta {
                // Pan hızı sabit değere (0.01) tıkalı olmak yerine odak mesafesiyle dinamik
                let pan_speed = camera_focus_distance * 0.0015;
                t.position += right * (-pan.x * pan_speed);
                t.position += up * (pan.y * pan_speed);
            }

            // 4. Alt + Sol Tık Orbit (Etrafında Dönme)
            if let Some(orbit) = orbit_delta {
                let orbit_speed = 0.005;

                // Pivot noktasını dinamik odak mesafesinden bul
                let pivot = t.position + forward * camera_focus_distance;

                cam.yaw += orbit.x * orbit_speed;
                cam.pitch -= orbit.y * orbit_speed;
                // Pitch sınırlaması fonksiyonun sonunda yapılıyor

                // Quaternion'u güncelle (orbit hesaplaması için gerekli)
                let q_yaw = gizmo::math::Quat::from_axis_angle(
                    gizmo::math::Vec3::new(0.0, 1.0, 0.0),
                    cam.yaw,
                );
                let q_pitch = gizmo::math::Quat::from_axis_angle(
                    gizmo::math::Vec3::new(1.0, 0.0, 0.0),
                    cam.pitch,
                );
                t.rotation = q_yaw * q_pitch;

                // Yeni pozisyonu pivota göre konumlandır
                t.position = pivot
                    - (t.rotation * gizmo::math::Vec3::new(0.0, 0.0, 1.0)) * camera_focus_distance;
            }

            // 5. Scroll Zoom (İleri / Geri) — Play modunda devre dışı
            if !is_playing && scroll_delta.abs() > 0.0001 {
                let scroll = scroll_delta;
                // Zoom hızı da odak noktasına yaklaştıkça yavaşlayıp hassaslaşacak
                // Egui'den gelen scroll_delta piksel cinsinden olduğu için çarpanı çok düşük tutmalıyız
                let zoom_amount = scroll * camera_focus_distance * 0.003;
                camera_focus_distance -= zoom_amount;
                if camera_focus_distance < 0.1 {
                    camera_focus_distance = 0.1;
                }
                t.position += forward * zoom_amount;
            }

            // 6. Ortografik / Sabit Bakış Açıları (Numpad 1, 3, 7)
            if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Numpad1 as u32) {
                cam.yaw = 0.0;
                cam.pitch = 0.0;
            }
            if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Numpad3 as u32) {
                cam.yaw = -std::f32::consts::FRAC_PI_2;
                cam.pitch = 0.0;
            }
            if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Numpad7 as u32) {
                cam.yaw = 0.0;
                cam.pitch = -std::f32::consts::FRAC_PI_2;
            }

            // 7. Bookmark Kaydet / Yükle (Ctrl + 0..9)
            let digits = [
                gizmo::winit::keyboard::KeyCode::Digit0,
                gizmo::winit::keyboard::KeyCode::Digit1,
                gizmo::winit::keyboard::KeyCode::Digit2,
                gizmo::winit::keyboard::KeyCode::Digit3,
                gizmo::winit::keyboard::KeyCode::Digit4,
                gizmo::winit::keyboard::KeyCode::Digit5,
                gizmo::winit::keyboard::KeyCode::Digit6,
                gizmo::winit::keyboard::KeyCode::Digit7,
                gizmo::winit::keyboard::KeyCode::Digit8,
                gizmo::winit::keyboard::KeyCode::Digit9,
            ];
            let ctrl = input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlLeft as u32)
                || input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlRight as u32);
            for (i, &key) in digits.iter().enumerate() {
                if input.is_key_just_pressed(key as u32) {
                    if ctrl {
                        // Bookmark Save
                        if let Some(mut es) = world.get_resource_mut::<EditorState>() {
                            es.camera.bookmarks[i] = Some((t.position, cam.yaw, cam.pitch));
                            es.log_info(&format!("Kamera #{} kaydedildi.", i));
                        }
                    } else {
                        // Bookmark Load
                        if let Some(mut es) = world.get_resource_mut::<EditorState>() {
                            if let Some((pos, yaw, pitch)) = es.camera.bookmarks[i] {
                                t.position = pos;
                                cam.yaw = yaw;
                                cam.pitch = pitch;
                                es.log_info(&format!("Kamera #{} yüklendi.", i));
                            }
                        }
                    }
                }
            }

            // Gimbal Lock sınırlaması ve yansıtması
            let max_pitch = 89.0_f32.to_radians();
            cam.pitch = cam.pitch.clamp(-max_pitch, max_pitch);

            let q_yaw =
                gizmo::math::Quat::from_axis_angle(gizmo::math::Vec3::new(0.0, 1.0, 0.0), cam.yaw);
            let q_pitch = gizmo::math::Quat::from_axis_angle(
                gizmo::math::Vec3::new(1.0, 0.0, 0.0),
                cam.pitch,
            );
            t.rotation = q_yaw * q_pitch;
        }
    }

    if let Some(mut es) = world.get_resource_mut::<EditorState>() {
        es.prefs.camera_focus_distance = camera_focus_distance;
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{PI, TAU};

    // Mirror of the focus-target yaw wrapping inside `handle_camera`: the delta
    // between the desired and current yaw is folded into (-PI, PI] so the camera
    // always rotates along the SHORTEST arc instead of unwinding the long way
    // around. Kept as a formula mirror because the real code is welded to World +
    // Camera + EditorState; the angle arithmetic is the part that silently breaks.
    fn shortest_yaw_diff(desired: f32, current: f32) -> f32 {
        let mut yaw_diff = desired - current;
        while yaw_diff > PI {
            yaw_diff -= TAU;
        }
        while yaw_diff < -PI {
            yaw_diff += TAU;
        }
        yaw_diff
    }

    #[test]
    fn yaw_diff_within_range_is_unchanged() {
        assert!((shortest_yaw_diff(0.5, 0.2) - 0.3).abs() < 1e-5);
        assert!((shortest_yaw_diff(-0.4, 0.4) - (-0.8)).abs() < 1e-5);
    }

    /// Crossing the ±PI seam must take the short way. Desired just past +PI relative
    /// to current should become a small NEGATIVE delta, not ~+2PI.
    #[test]
    fn yaw_diff_wraps_to_shortest_arc() {
        // current near +PI, desired just past -PI (i.e. wrapped) → tiny step, sign flips.
        let d = shortest_yaw_diff(-PI + 0.1, PI - 0.1);
        assert!(d.abs() < PI, "must be the short arc, got {d}");
        assert!((d - 0.2).abs() < 1e-4, "expected ~+0.2 shortest step, got {d}");

        // A near-full-turn desired collapses to a near-zero move.
        let d2 = shortest_yaw_diff(TAU - 0.05, 0.0);
        assert!((d2 - (-0.05)).abs() < 1e-4, "full turn should be ~-0.05, got {d2}");
    }

    /// The wrapped delta is always in (-PI, PI], regardless of how many turns apart
    /// the two raw angles are (bounded-output invariant).
    #[test]
    fn yaw_diff_is_always_bounded() {
        let samples = [-10.0f32, -3.3, -1.0, 0.0, 0.7, 2.9, 5.5, 9.99, 100.0];
        for &a in &samples {
            for &b in &samples {
                let d = shortest_yaw_diff(a, b);
                assert!(d > -PI - 1e-4 && d <= PI + 1e-4, "diff({a},{b})={d} out of range");
            }
        }
    }

    // Mirror of the gimbal-lock pitch clamp at the end of `handle_camera`.
    fn clamp_pitch(pitch: f32) -> f32 {
        let max_pitch = 89.0_f32.to_radians();
        pitch.clamp(-max_pitch, max_pitch)
    }

    #[test]
    fn pitch_is_clamped_to_avoid_gimbal_flip() {
        let max_pitch = 89.0_f32.to_radians();
        // Looking straight up (PI/2) is clamped just below vertical.
        assert!((clamp_pitch(PI / 2.0) - max_pitch).abs() < 1e-6);
        // And straight down.
        assert!((clamp_pitch(-PI / 2.0) + max_pitch).abs() < 1e-6);
        // A gentle pitch is untouched.
        assert!((clamp_pitch(0.3) - 0.3).abs() < 1e-6);
    }
}
