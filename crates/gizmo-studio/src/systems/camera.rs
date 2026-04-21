use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::physics::components::Transform;
use gizmo::prelude::*;

pub fn handle_camera(world: &mut World, state: &mut StudioState, dt: f32, input: &Input, 
    look_delta: Option<gizmo::math::Vec2>,
    pan_delta: Option<gizmo::math::Vec2>,
    orbit_delta: Option<gizmo::math::Vec2>,
    scroll_delta: f32) {
    // Editör kamera değişkenlerini world'dan oku
    let mut camera_speed = 8.0;
    let mut camera_focus_distance = 10.0;
    let mut is_playing = false;
    if let Some(es) = world.get_resource::<EditorState>() {
        camera_speed = es.prefs.camera_speed;
        camera_focus_distance = es.prefs.camera_focus_distance;
        is_playing = es.is_playing();
    }

    // Editor Camera WASD Controller
    let mut transforms = world.borrow_mut::<Transform>(); let mut cameras = world.borrow_mut::<gizmo::renderer::components::Camera>(); {
        if let (Some(t), Some(cam)) = (
            transforms.get_mut(state.editor_camera),
            cameras.get_mut(state.editor_camera),
        ) {
            // 1. Mouse Look (Egui üzerinden gelen delta okuması)
            if let Some(delta) = look_delta {
                let sensitivity = 0.003;

                cam.yaw += delta.x * sensitivity;
                cam.pitch -= delta.y * sensitivity;

                // Gimbal Lock'u (tepetaklak olmayı) önle
                let max_pitch = 89.0_f32.to_radians();
                if cam.pitch > max_pitch {
                    cam.pitch = max_pitch;
                }
                if cam.pitch < -max_pitch {
                    cam.pitch = -max_pitch;
                }

                // Transform rotasyonunu kameraya uydur (motor içi tutarlılık için)
                let q_yaw = gizmo::math::Quat::from_axis_angle(
                    gizmo::math::Vec3::new(0.0, 1.0, 0.0),
                    cam.yaw,
                );
                let q_pitch = gizmo::math::Quat::from_axis_angle(
                    gizmo::math::Vec3::new(1.0, 0.0, 0.0),
                    cam.pitch,
                );
                t.rotation = q_yaw * q_pitch;
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

            t.position += move_dir.normalize_or_zero() * (speed * dt);

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

                let max_pitch = 89.0_f32.to_radians();
                if cam.pitch > max_pitch {
                    cam.pitch = max_pitch;
                }
                if cam.pitch < -max_pitch {
                    cam.pitch = -max_pitch;
                }

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
                t.position = pivot - (t.rotation * gizmo::math::Vec3::new(0.0, 0.0, 1.0)) * camera_focus_distance;
            }

            // 5. Scroll Zoom (İleri / Geri)
            if scroll_delta.abs() > 0.0001 {
                let scroll = scroll_delta;
                // Zoom hızı da odak noktasına yaklaştıkça yavaşlayıp hassaslaşacak
                let zoom_amount = scroll * camera_focus_distance * 0.1;
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
                gizmo::winit::keyboard::KeyCode::Digit0, gizmo::winit::keyboard::KeyCode::Digit1,
                gizmo::winit::keyboard::KeyCode::Digit2, gizmo::winit::keyboard::KeyCode::Digit3,
                gizmo::winit::keyboard::KeyCode::Digit4, gizmo::winit::keyboard::KeyCode::Digit5,
                gizmo::winit::keyboard::KeyCode::Digit6, gizmo::winit::keyboard::KeyCode::Digit7,
                gizmo::winit::keyboard::KeyCode::Digit8, gizmo::winit::keyboard::KeyCode::Digit9,
            ];
            let ctrl = input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlLeft as u32) || input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ControlRight as u32);
            for (i, &key) in digits.iter().enumerate() {
                if input.is_key_just_pressed(key as u32) {
                    if ctrl { // Bookmark Save
                        if let Some(mut es) = world.get_resource_mut::<EditorState>() {
                            es.camera.bookmarks[i] = Some((t.position, cam.yaw, cam.pitch));
                            es.log_info(&format!("Kamera #{} kaydedildi.", i));
                        }
                    } else { // Bookmark Load
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

            let q_yaw = gizmo::math::Quat::from_axis_angle(
                gizmo::math::Vec3::new(0.0, 1.0, 0.0),
                cam.yaw,
            );
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
