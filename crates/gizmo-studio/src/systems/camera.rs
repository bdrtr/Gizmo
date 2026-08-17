use crate::state::StudioState;
use gizmo::editor::EditorState;
use gizmo::physics::components::Transform;
use gizmo::prelude::*;

/// Drives the editor camera for this frame: fly controls while the viewport has the mouse, plus
/// any focus or orbit the UI asked for.
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
    let mut fly_active = false;
    let mut focus_target = None;
    let mut view_request = None;
    if let Some(es) = world.get_resource::<EditorState>() {
        camera_speed = es.prefs.camera_speed;
        camera_focus_distance = es.prefs.camera_focus_distance;
        is_playing = es.is_playing();
        fly_active = es.camera.fly_active;
        focus_target = es.camera.focus_target;
        view_request = es.camera.view_request;
    }

    // Editor Camera WASD Controller
    // SAFETY: exclusive `&mut World`; Transform and Camera are distinct component types.
    let mut transforms = unsafe { world.borrow_mut_unchecked::<Transform>() };
    // SAFETY: as above — Camera is a distinct component type from Transform.
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

            // Fly keys only while the right button is held on the viewport — the same gesture
            // that already gates looking around, and what the Game panel's own help text
            // describes ("Sağ Tık + Sürükle — Kamerayı döndür").
            //
            // Ungated, these keys collided head-on with the editor's tool shortcuts, which are
            // bound globally in `gizmo_editor::draw_editor`:
            //
            //     Q → aşağı  / Select      W → ileri  / Translate
            //     E → yukarı / Rotate      R →   —    / Scale
            //
            // So flying forward with W silently switched the active tool to Translate, and there
            // was no way to avoid it: movement asked for no modifier at all. Every editor in this
            // class solves it the same way — the right button is what puts the viewport in camera
            // mode, and the letter keys mean tools outside it.
            //
            // ÇUBUK O KAPININ DIŞINDA. Kapı, HARFLERİN araç kısayollarıyla çakışması yüzünden
            // var; kolun araç kısayolu yok, yani aynı jesti ondan istemek gerekçesi olmayan bir
            // kısıtlama olurdu. `!is_playing` ise ikisi için de geçerli: Play sırasında kol
            // oyunun.
            //
            // Bu yüzden burada `Input::move_axis` değil `blend_move_axis` çağrılıyor — tuş
            // yarısını koşullu susturabilmek için. Harmanın kuralları (tuş yönü çubukla
            // karşılaştırılabilir olsun diye önce birim boya gelir, toplam birim diske kırpılır)
            // yine tek yerde.
            if !is_playing {
                let keys = if fly_active {
                    let axis = |neg: u32, pos: u32| {
                        f32::from(input.is_key_pressed(pos)) - f32::from(input.is_key_pressed(neg))
                    };
                    (
                        axis(
                            gizmo::winit::keyboard::KeyCode::KeyA as u32,
                            gizmo::winit::keyboard::KeyCode::KeyD as u32,
                        ),
                        axis(
                            gizmo::winit::keyboard::KeyCode::KeyS as u32,
                            gizmo::winit::keyboard::KeyCode::KeyW as u32,
                        ),
                    )
                } else {
                    (0.0, 0.0)
                };
                let stick = input
                    .gamepad()
                    .map(gizmo::core::input::Gamepad::left_stick)
                    .unwrap_or((0.0, 0.0));
                let (mx, my) = gizmo::core::input::blend_move_axis(keys, stick);
                move_dir += right * mx + forward * my;

                // Dünyaya göre yukarı/aşağı tırmanış — çubukta karşılığı yok, harflerde kalıyor.
                if fly_active {
                    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyE as u32) {
                        move_dir += up;
                    }
                    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyQ as u32) {
                        move_dir -= up;
                    }
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
                
                // Hedef tam tepedeyse yaw belirsiz; mevcut yaw devralınıyor, ki odaklanma nesneyi
                // ortalarken sahneyi bir de kendi etrafında döndürmesin.
                let (desired_yaw, desired_pitch) =
                    gizmo::renderer::components::Camera::yaw_pitch_from_forward(dir, cam.yaw)
                        .unwrap_or((cam.yaw, cam.pitch));


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
                // Normalize değil KIRPMA: tuşlarda ikisi aynı (boş olmayan her tuş bileşimi
                // zaten en az birim boyda), ama normalize yarım yatırılmış çubuğu tam hıza
                // çıkarıp çubuğun kattığı tek şeyi — miktarı — yok ederdi.
                let len = move_dir.length();
                if len > 1.0 {
                    move_dir /= len;
                }
                t.position += move_dir * (speed * dt);
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

            // 4.5 Viewport eksen gizmo'sundan gelen bakış isteği.
            //
            // Kamera YERİNDE dönmüyor: ekranın ortasındaki nokta ortada kalıyor, yalnız yön
            // değişiyor — orbit'in yaptığının aynısı, deltayla değil hedef yönle. Yerinde dönseydi
            // bakmakta olduğun nesne ekrandan çıkardı, ve bir bakış küpünün tek işi ona bakmaktır.
            if let Some(dir) = view_request {
                if let Some((yaw, pitch)) =
                    gizmo::renderer::components::Camera::yaw_pitch_from_forward(dir, cam.yaw)
                {
                    let max_pitch = 89.0_f32.to_radians();
                    let pitch = pitch.clamp(-max_pitch, max_pitch);
                    // Pivot güncel yaw/pitch'ten kuruluyor, fonksiyonun başındaki `forward`dan
                    // değil: orbit bu kareyi çoktan döndürmüş olabilir.
                    let pivot = t.position
                        + gizmo::renderer::components::Camera::forward_from(cam.yaw, cam.pitch)
                            * camera_focus_distance;
                    cam.yaw = yaw;
                    cam.pitch = pitch;
                    t.position = pivot
                        - gizmo::renderer::components::Camera::forward_from(yaw, pitch)
                            * camera_focus_distance;
                }
                if let Some(mut es) = world.get_resource_mut::<EditorState>() {
                    es.camera.view_request = None;
                }
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

    use gizmo::math::Vec3;
    use gizmo::renderer::components::Camera;

    /// The six axes the viewport gizmo can be clicked on.
    const AXES: [Vec3; 6] = [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ];

    /// `yaw_pitch_from_forward` is claimed to be the inverse of `forward_from`, so measure it against
    /// the real function rather than against a second copy of the formula. A swapped `atan2`
    /// argument order or a sign lost on pitch survives any test that only reads the numbers back
    /// out of the inverse itself.
    #[test]
    fn yaw_pitch_from_forward_round_trips_through_forward_from() {
        for dir in AXES {
            let (yaw, pitch) = Camera::yaw_pitch_from_forward(dir, 0.0).expect("an axis is a direction");
            let back = Camera::forward_from(yaw, pitch);
            // `forward_from` holds pitch just off vertical, so ±Y cannot come back exact; a dot
            // product of 1 - 5e-7 is that clamp and nothing else.
            assert!(
                back.dot(dir) > 0.999,
                "yaw_pitch_from_forward({dir}) → yaw {yaw}, pitch {pitch} → {back}, which is not {dir}"
            );
        }
    }

    /// Yaw is undetermined when looking straight up or down, and `atan2(0.0, 0.0)` returns `0.0`
    /// without complaining — so the top view would quietly swing the scene round to face world +X.
    #[test]
    fn yaw_pitch_from_forward_inherits_yaw_when_it_is_undetermined() {
        for dir in [Vec3::Y, Vec3::NEG_Y] {
            let (yaw, _) = Camera::yaw_pitch_from_forward(dir, 1.234).expect("a direction");
            assert!(
                (yaw - 1.234).abs() < 1e-6,
                "vertical look must keep the yaw it had, got {yaw}"
            );
        }
        // ...and a direction that *does* determine yaw must ignore the hint entirely.
        let (yaw, _) = Camera::yaw_pitch_from_forward(Vec3::X, 1.234).expect("a direction");
        assert!(yaw.abs() < 1e-6, "+X is yaw 0 regardless of where we were, got {yaw}");
    }

    #[test]
    fn yaw_pitch_from_forward_rejects_a_zero_direction() {
        assert!(Camera::yaw_pitch_from_forward(Vec3::ZERO, 0.0).is_none());
    }

    /// The handle for `+X` sends `look_dir = -X`, and the camera must end up standing on the **+X**
    /// side looking back at the pivot — not on the -X side looking away. This mirrors the
    /// repositioning in `handle_camera`, including its 89° pitch clamp.
    #[test]
    fn a_handle_puts_the_camera_on_that_axis_side() {
        const DIST: f32 = 10.0;
        let pivot = Vec3::new(2.0, 3.0, -4.0);
        for axis in AXES {
            let (yaw, pitch) = Camera::yaw_pitch_from_forward(-axis, 0.0).expect("a direction");
            let max_pitch = 89.0_f32.to_radians();
            let front = Camera::forward_from(yaw, pitch.clamp(-max_pitch, max_pitch));
            let pos = pivot - front * DIST;

            let offset = pos - pivot;
            assert!(
                offset.dot(axis) > DIST * 0.99,
                "clicking {axis} must put the camera on the {axis} side of the pivot, got {offset}"
            );
            assert!(
                (offset.length() - DIST).abs() < 1e-3,
                "the turn must not change the distance to the pivot, got {}",
                offset.length()
            );
        }
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
