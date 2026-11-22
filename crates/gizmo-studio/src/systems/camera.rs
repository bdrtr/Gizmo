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
    let prefs = CameraPrefs::read(world);
    let mut camera_focus_distance = prefs.focus_distance;
    let is_playing = prefs.is_playing;

    // Editor Camera WASD Controller
    // SAFETY: exclusive `&mut World`; Transform and Camera are distinct component types.
    let mut transforms = unsafe { world.query_unchecked::<&mut Transform>() };
    // SAFETY: as above — Camera is a distinct component type from Transform.
    let mut cameras = unsafe { world.query_unchecked::<&mut gizmo::renderer::components::Camera>() };
    let (Some(mut t), Some(_)) = (transforms.get_mut(state.editor_camera), cameras.get_mut(state.editor_camera)) else {
        return;
    };
    // The editor camera thinks in yaw/pitch; its aim is the rotation, read here and written back at
    // the end.
    let (mut yaw, mut pitch) = gizmo::renderer::components::Camera::yaw_pitch_of(t.rotation);
    // 1. Mouse Look (Egui üzerinden gelen delta okuması)
    if !is_playing {
        if let Some(delta) = look_delta {
            let sensitivity = 0.003;

            yaw += delta.x * sensitivity;
            pitch -= delta.y * sensitivity;
            // Pitch sınırlaması fonksiyonun sonunda yapılıyor
        }
    }

    // 2. Serbest Uçuş (WASD + Q/E)
    let speed = if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftLeft as u32) {
        prefs.speed * 2.5
    } else {
        prefs.speed
    };

    let forward = gizmo::renderer::components::Camera::forward_from(yaw, pitch);
    let right = forward
        .cross(gizmo::math::Vec3::new(0.0, 1.0, 0.0))
        .normalize();
    let up = gizmo::math::Vec3::new(0.0, 1.0, 0.0);

    let move_dir = if is_playing { gizmo::math::Vec3::ZERO } else { fly_direction(input, prefs.fly_active, right, forward, up) };

    // Eğer kullanıcı manuel olarak kamerayı hareket ettirirse, odaklanmayı iptal et
    let mut focus_target = prefs.focus_target;
    if move_dir.length_squared() > 0.0 || look_delta.is_some() || pan_delta.is_some() || orbit_delta.is_some() || scroll_delta != 0.0 {
        focus_target = None;
        if let Some(mut es) = world.get_resource_mut::<EditorState>() {
            es.camera.focus_target = None;
        }
    }

    if let Some(target) = focus_target {
        if approach_focus(&mut t, &mut yaw, &mut pitch, target, forward, camera_focus_distance, dt) {
            if let Some(mut es) = world.get_resource_mut::<EditorState>() {
                es.camera.focus_target = None;
            }
        }
    } else {
        t.position += clipped(move_dir) * (speed * dt);
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
        orbit_about_pivot(&mut t, &mut yaw, &mut pitch, orbit, forward, camera_focus_distance);
    }

    // 4.5 Viewport eksen gizmo'sundan gelen bakış isteği.
    if let Some(dir) = prefs.view_request {
        look_along(&mut t, &mut yaw, &mut pitch, dir, camera_focus_distance);
        if let Some(mut es) = world.get_resource_mut::<EditorState>() {
            es.camera.view_request = None;
        }
    }

    // 5. Scroll Zoom, 6. the fixed views, 7. the bookmarks — not in play. The game reads those keys
    // for itself (1–9 to change weapon), and moving the editor's camera behind the game's picture
    // meant ⏹ opened the scene somewhere else.
    if !is_playing {
        if scroll_delta.abs() > 0.0001 {
            let moved = zoom(&mut camera_focus_distance, scroll_delta);
            t.position += forward * moved;
        }
        fixed_views(input, &mut yaw, &mut pitch);
        camera_bookmarks(world, input, &mut t, &mut yaw, &mut pitch);
    }

    // Gimbal Lock sınırlaması ve yansıtması
    pitch = clamp_pitch(pitch);

    t.rotation = gizmo::renderer::components::Camera::rotation_from(yaw, pitch);

    if let Some(mut es) = world.get_resource_mut::<EditorState>() {
        es.prefs.camera_focus_distance = camera_focus_distance;
    }
}

/// What the editor's state says about the camera this frame.
struct CameraPrefs {
    speed: f32,
    focus_distance: f32,
    is_playing: bool,
    fly_active: bool,
    focus_target: Option<gizmo::math::Vec3>,
    view_request: Option<gizmo::math::Vec3>,
}

impl CameraPrefs {
    /// Read from `EditorState`; the defaults without one.
    fn read(world: &World) -> Self {
        let mut prefs = CameraPrefs {
            speed: 8.0,
            focus_distance: 10.0,
            is_playing: false,
            fly_active: false,
            focus_target: None,
            view_request: None,
        };
        if let Some(es) = world.get_resource::<EditorState>() {
            prefs.speed = es.prefs.camera_speed;
            prefs.focus_distance = es.prefs.camera_focus_distance;
            prefs.is_playing = es.is_playing();
            prefs.fly_active = es.camera.fly_active;
            prefs.focus_target = es.camera.focus_target;
            prefs.view_request = es.camera.view_request;
        }
        prefs
    }
}

/// The fly direction from the keys and the left stick, unclipped (`clipped` limits it).
///
/// Fly keys only while the right button is held on the viewport — the same gesture
/// that already gates looking around, and what the Game panel's own help text
/// describes (right-click and drag: turn the camera).
///
/// Ungated, these keys collided head-on with the editor's tool shortcuts, which are
/// bound globally in `gizmo_editor::draw_editor`:
///
/// ```text
/// Q → down / Select      W → forward / Translate
/// E → up   / Rotate      R →    —    / Scale
/// ```
///
/// So flying forward with W silently switched the active tool to Translate, and there
/// was no way to avoid it: movement asked for no modifier at all. Every editor in this
/// class solves it the same way — the right button is what puts the viewport in camera
/// mode, and the letter keys mean tools outside it.
///
/// THE STICK IS OUTSIDE THAT GATE. The gate exists because the LETTERS collide with the tool
/// shortcuts; the stick has no tool shortcut, so asking it for the same gesture would be a
/// restriction with no reason behind it. `!is_playing` holds for both: in Play the stick is the
/// game's.
///
/// That is why this calls `blend_move_axis` and not `Input::move_axis`: to mute the keys' half on
/// a condition. The blend's rules (the keys' direction is brought to unit length first, so it
/// compares with the stick, and the sum is clipped to the unit disc) stay in one place.
fn fly_direction(
    input: &Input,
    fly_active: bool,
    right: gizmo::math::Vec3,
    forward: gizmo::math::Vec3,
    up: gizmo::math::Vec3,
) -> gizmo::math::Vec3 {
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
    let mut move_dir = right * mx + forward * my;

    // Dünyaya göre yukarı/aşağı tırmanış — çubukta karşılığı yok, harflerde kalıyor.
    if fly_active {
        if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyE as u32) {
            move_dir += up;
        }
        if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyQ as u32) {
            move_dir -= up;
        }
    }
    move_dir
}

/// The fly direction at most unit length.
///
/// CLIPPED, not normalised: for keys the two are the same (any key combination that moves at all
/// is at least unit length already), but normalising would take a half-tilted stick to full
/// speed and remove the one thing the stick adds — how much.
fn clipped(mut move_dir: gizmo::math::Vec3) -> gizmo::math::Vec3 {
    let len = move_dir.length();
    if len > 1.0 {
        move_dir /= len;
    }
    move_dir
}

/// The shortest turn from `current` to `desired`, in `(-PI, PI]`: the long way round is never taken.
fn shortest_yaw_diff(desired: f32, current: f32) -> f32 {
    let mut yaw_diff = desired - current;
    while yaw_diff > std::f32::consts::PI {
        yaw_diff -= std::f32::consts::TAU;
    }
    while yaw_diff < -std::f32::consts::PI {
        yaw_diff += std::f32::consts::TAU;
    }
    yaw_diff
}

/// Pitch held off vertical (±89°): at ±90° yaw is undefined and the camera flips.
fn clamp_pitch(pitch: f32) -> f32 {
    let max_pitch = 89.0_f32.to_radians();
    pitch.clamp(-max_pitch, max_pitch)
}

/// One frame of the focus (`F`): the camera turns toward `target` and moves to `focus_distance` in
/// front of it, smoothly. Returns whether it has arrived.
///
/// The move is clamped to a whole step like the turn: unclamped, a frame longer than 1/8 s (a
/// stall, a shader rebuild) carried the camera past its spot and back.
fn approach_focus(
    t: &mut Transform,
    yaw: &mut f32,
    pitch: &mut f32,
    target: gizmo::math::Vec3,
    forward: gizmo::math::Vec3,
    focus_distance: f32,
    dt: f32,
) -> bool {
    let diff = target - t.position;
    let dist_to_target = diff.length();
    let dir = if dist_to_target > 0.001 { diff / dist_to_target } else { forward };

    // Hedef tam tepedeyse yaw belirsiz; mevcut yaw devralınıyor, ki odaklanma nesneyi
    // ortalarken sahneyi bir de kendi etrafında döndürmesin.
    let (desired_yaw, desired_pitch) =
        gizmo::renderer::components::Camera::yaw_pitch_from_forward(dir, *yaw).unwrap_or((*yaw, *pitch));

    let yaw_diff = shortest_yaw_diff(desired_yaw, *yaw);

    // Yumuşak kamera dönüşü
    let step = (8.0 * dt).clamp(0.0, 1.0);
    *yaw += yaw_diff * step;
    *pitch += (desired_pitch - *pitch) * step;

    // Güncel bakış açısına göre hedef noktayı belirle
    let current_forward = gizmo::renderer::components::Camera::forward_from(*yaw, *pitch);
    let desired_pos = target - current_forward * focus_distance;

    // Pozisyonu yumuşakça lerple
    t.position = t.position.lerp(desired_pos, step);

    t.position.distance(desired_pos) < 0.1 && yaw_diff.abs() < 0.05 && (desired_pitch - *pitch).abs() < 0.05
}

/// Alt + left drag: the camera orbits the point `focus_distance` in front of it.
fn orbit_about_pivot(
    t: &mut Transform,
    yaw: &mut f32,
    pitch: &mut f32,
    orbit: gizmo::math::Vec2,
    forward: gizmo::math::Vec3,
    focus_distance: f32,
) {
    let orbit_speed = 0.005;

    // Pivot noktasını dinamik odak mesafesinden bul
    let pivot = t.position + forward * focus_distance;

    *yaw += orbit.x * orbit_speed;
    *pitch -= orbit.y * orbit_speed;
    // Pitch sınırlaması fonksiyonun sonunda yapılıyor

    // Yeni pozisyon: pivotun, yeni bakış yönünde odak mesafesi gerisi. (Bu,
    // `R_y(yaw)·R_x(pitch)·Z` ile kuruluyordu — kameranın bakışı değil — ve yörünge
    // pivotun etrafında dönmüyordu.)
    t.position = pivot
        - gizmo::renderer::components::Camera::forward_from(*yaw, pitch.clamp(-89.0_f32.to_radians(), 89.0_f32.to_radians()))
            * focus_distance;
}

/// The viewport axis gizmo's request to look along `dir`.
///
/// The camera does not turn IN PLACE: the point at the centre of the screen stays there and only
/// the direction changes — what the orbit does, with a target direction instead of a delta.
/// Turned in place, the thing you were looking at would leave the screen, and a view cube's one
/// job is to look at it.
fn look_along(t: &mut Transform, yaw: &mut f32, pitch: &mut f32, dir: gizmo::math::Vec3, focus_distance: f32) {
    if let Some((new_yaw, new_pitch)) = gizmo::renderer::components::Camera::yaw_pitch_from_forward(dir, *yaw) {
        let max_pitch = 89.0_f32.to_radians();
        let new_pitch = new_pitch.clamp(-max_pitch, max_pitch);
        // Pivot güncel yaw/pitch'ten kuruluyor, fonksiyonun başındaki `forward`dan
        // değil: orbit bu kareyi çoktan döndürmüş olabilir.
        let pivot = t.position + gizmo::renderer::components::Camera::forward_from(*yaw, *pitch) * focus_distance;
        *yaw = new_yaw;
        *pitch = new_pitch;
        t.position = pivot - gizmo::renderer::components::Camera::forward_from(*yaw, *pitch) * focus_distance;
    }
}

/// The wheel's zoom: the focus distance shrinks by a share of itself (slower as the pivot nears),
/// never below 0.1. Returns how far the camera moves forward — what the distance lost. It moved
/// the whole step even when the distance stopped at 0.1, and so went through the pivot.
fn zoom(focus_distance: &mut f32, scroll_delta: f32) -> f32 {
    // Egui'den gelen scroll_delta piksel cinsinden olduğu için çarpanı çok düşük tutmalıyız
    let zoom_amount = scroll_delta * *focus_distance * 0.003;
    if *focus_distance - zoom_amount < 0.1 {
        let moved = *focus_distance - 0.1;
        *focus_distance = 0.1;
        moved
    } else {
        *focus_distance -= zoom_amount;
        zoom_amount
    }
}

/// Numpad 1, 3 and 7: the front, side and top views.
fn fixed_views(input: &Input, yaw: &mut f32, pitch: &mut f32) {
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Numpad1 as u32) {
        *yaw = 0.0;
        *pitch = 0.0;
    }
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Numpad3 as u32) {
        *yaw = -std::f32::consts::FRAC_PI_2;
        *pitch = 0.0;
    }
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Numpad7 as u32) {
        *yaw = 0.0;
        *pitch = -std::f32::consts::FRAC_PI_2;
    }
}

/// Ctrl + 0..9 saves the camera as that bookmark; 0..9 alone goes back to it.
fn camera_bookmarks(world: &World, input: &Input, t: &mut Transform, yaw: &mut f32, pitch: &mut f32) {
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
                    es.camera.bookmarks[i] = Some((t.position, *yaw, *pitch));
                    es.log_info(&format!("Kamera #{} kaydedildi.", i));
                }
            } else {
                // Bookmark Load
                if let Some(mut es) = world.get_resource_mut::<EditorState>() {
                    if let Some((pos, saved_yaw, saved_pitch)) = es.camera.bookmarks[i] {
                        t.position = pos;
                        *yaw = saved_yaw;
                        *pitch = saved_pitch;
                        es.log_info(&format!("Kamera #{} yüklendi.", i));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{PI, TAU};
    // The real functions: these tests tested copies of their formulas while the code was one
    // function welded to the World, the camera and the editor's state.
    use super::{approach_focus, clamp_pitch, shortest_yaw_diff, zoom};

    /// The wheel never takes the camera through its pivot: at the 0.1 floor it moves only what the
    /// distance lost. It moved the whole step and passed the point it was zooming at.
    #[test]
    fn zooming_stops_at_the_pivot() {
        let mut distance = 0.2;
        let moved = zoom(&mut distance, 5000.0);
        assert_eq!(distance, 0.1);
        assert!((moved - 0.1).abs() < 1e-6, "moved {moved}, the distance lost 0.1");
        let mut far = 10.0;
        let step = zoom(&mut far, 10.0);
        assert!((step - 0.3).abs() < 1e-5 && (far - 9.7).abs() < 1e-5);
    }

    /// A long frame does not carry the focus past its spot: the move is clamped to a whole step, as
    /// the turn already was.
    #[test]
    fn a_long_frame_does_not_overshoot_the_focus() {
        let target = gizmo::math::Vec3::new(0.0, 0.0, -20.0);
        let mut t = gizmo::physics::components::Transform::new(gizmo::math::Vec3::ZERO);
        let (mut yaw, mut pitch) = (0.0, 0.0);
        let forward = Camera::forward_from(yaw, pitch);
        approach_focus(&mut t, &mut yaw, &mut pitch, target, forward, 5.0, 0.5);
        let spot = target - Camera::forward_from(yaw, pitch) * 5.0;
        assert!(t.position.distance(spot) < 1e-3, "the camera is at {:?}, its spot is {spot:?}", t.position);
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
