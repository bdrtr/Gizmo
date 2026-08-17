//! First-person LOOK controller — mouse-look (+ optional WASD flight movement).
//!
//! A standard FP / free-flight camera is needed in almost every game and tool; but the
//! demos write it out BY HAND every frame: `mouse_delta → yaw/pitch`, pitch clamp, WASD →
//! position, then the `Camera` + `Transform` sync (see yikim_ustasi's hand-written camera
//! loop). Add the `FpsLook` component to the camera entity, run [`FpsLookPlugin`]; let the
//! engine do the looking with the mouse (and the moving with WASD if `move_speed>0`).
//! Aim/fire direction: [`FpsLook::forward`] — you don't need to copy the "front" maths.
//!
//! ```
//! use gizmo::prelude::*;
//! use gizmo::core::system::System;
//! use gizmo::systems::fps_look::FpsLookSystem;
//!
//! let mut world = World::new();
//! let cam = world.spawn();
//! world.add_component(cam, Transform::new(Vec3::ZERO));
//! world.add_component(cam, Camera::new(1.0, 0.1, 100.0, 0.0, 0.0, true));
//! world.add_component(cam, FpsLook::new().with_move_speed(8.0)); // mouse-look + WASD flight
//!
//! // In an application `app.add_plugin(FpsLookPlugin)` drives the system every frame; here
//! // we drive a single frame by hand: the mouse moves 100 pixels RIGHT.
//! let mut input = Input::new();
//! input.on_mouse_delta(100.0, 0.0);
//! world.insert_resource(input);
//! FpsLookSystem.run(&world, 1.0 / 60.0);
//!
//! let looks = world.borrow::<FpsLook>();
//! let look = looks.get_entity(cam).unwrap();
//! assert!((look.yaw - 100.0 * look.sensitivity).abs() < 1e-6); // mouse right -> yaw+
//! // The camera view stays in sync: the renderer uses `Camera.yaw/pitch`.
//! let cams = world.borrow::<Camera>();
//! assert_eq!(cams.get_entity(cam).unwrap().yaw, look.yaw);
//!
//! // when firing:
//! let dir = look.forward();
//! assert!((dir.length() - 1.0).abs() < 1e-5); // unit aim vector
//! ```

use gizmo_core::input::Input;
use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::Transform;
use gizmo_renderer::components::Camera;
use winit::keyboard::KeyCode;

/// The mouse-look controller added to a camera entity. `yaw`/`pitch` are HELD in this
/// component (the single source of truth); every frame the system processes the mouse/WASD
/// and writes to `Camera` + `Transform`. The aim direction is read with
/// [`forward`](Self::forward).
#[derive(Debug, Clone, Copy)]
pub struct FpsLook {
    /// Horizontal look angle (rad).
    pub yaw: f32,
    /// Vertical look angle (rad); clamped to `±pitch_limit`.
    pub pitch: f32,
    /// Mouse sensitivity, **radians per pixel**.
    ///
    /// Distinct from [`stick_sensitivity`](Self::stick_sensitivity) in units *and* in how it is
    /// applied — see [`apply_look`](Self::apply_look), where that difference is the whole point.
    pub sensitivity: f32,
    /// Right-stick look speed, **radians per second**. 0 → the stick does not look.
    ///
    /// A different unit from [`sensitivity`](Self::sensitivity) because it is a different kind of
    /// input: a mouse reports how far it *has moved*, a stick reports how far it *is held*.
    pub stick_sensitivity: f32,
    /// WASD movement speed (units/s). 0 → look only (the camera stays put).
    pub move_speed: f32,
    /// Pitch is clamped to ± this value (rad) (prevents going upside-down).
    pub pitch_limit: f32,
    /// false → the system SKIPS this camera (for menus, cutscenes, autoplay).
    pub enabled: bool,
    /// Mouse button that must be **held** for looking to apply; `None` looks always.
    ///
    /// Added 2026-08-18 because the controller had no callers and this was the first reason why:
    /// a tool or a demo with a cursor cannot have the camera swing every time the mouse moves.
    /// `gizmo-studio`'s editor camera and `cpu_physics` had both hand-rolled exactly this gate.
    /// Defaults to `None`, so a `FpsLook` written before this field behaves as it always did.
    ///
    /// It gates **looking only** — movement keys keep working, which is what a fly camera in a
    /// tool wants. It does not gate the stick, for the same reason the studio's does not: a pad
    /// has no cursor to fight with.
    pub look_button: Option<u32>,
    /// Key that multiplies movement speed by [`sprint_multiplier`](Self::sprint_multiplier) while
    /// held; `None` disables sprinting.
    ///
    /// `None` by default and NOT `ShiftLeft`, because ShiftLeft already means *descend* here —
    /// see [`down_key`](Self::down_key). A caller that wants shift-to-sprint should move the
    /// descend key first, and that collision is the reason both are fields rather than constants.
    pub sprint_key: Option<u32>,
    /// What [`sprint_key`](Self::sprint_key) multiplies the speed by.
    pub sprint_multiplier: f32,
    /// Key that moves straight up, in world space. Defaults to Space.
    pub up_key: u32,
    /// Key that moves straight down. Defaults to ShiftLeft — see [`sprint_key`](Self::sprint_key).
    pub down_key: u32,
}

impl Default for FpsLook {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            sensitivity: 0.0025,
            // ~143°/s at full tilt, the usual console default. Both axes turn at the same rate;
            // games that want a slower pitch scale it themselves.
            stick_sensitivity: 2.5,
            move_speed: 0.0,
            pitch_limit: std::f32::consts::FRAC_PI_2 - 0.05,
            enabled: true,
            look_button: None,
            sprint_key: None,
            sprint_multiplier: 3.0,
            up_key: KeyCode::Space as u32,
            down_key: KeyCode::ShiftLeft as u32,
        }
    }
}

impl FpsLook {
    /// Default mouse-look controller (no movement — look only).
    pub fn new() -> Self {
        Self::default()
    }

    /// Turn on WASD flight movement at `speed` units/s. Chainable.
    pub fn with_move_speed(mut self, speed: f32) -> Self {
        self.move_speed = speed;
        self
    }

    /// Set the mouse sensitivity (rad/pixel). Chainable.
    pub fn with_sensitivity(mut self, s: f32) -> Self {
        self.sensitivity = s;
        self
    }

    /// Set the right-stick look speed (rad/s). 0 turns stick look off. Chainable.
    pub fn with_stick_sensitivity(mut self, s: f32) -> Self {
        self.stick_sensitivity = s;
        self
    }

    /// Look only while `button` is held — `gizmo_core::input::mouse::RIGHT` is the usual choice.
    /// Chainable.
    pub fn with_look_button(mut self, button: u32) -> Self {
        self.look_button = Some(button);
        self
    }

    /// Hold `key` to move `multiplier` times faster. Chainable.
    ///
    /// Note the collision this exists to make visible: [`down_key`](Self::down_key) is ShiftLeft
    /// by default, so `with_sprint(KeyCode::ShiftLeft as u32, 3.0)` alone would make shift mean
    /// both. Set [`down_key`](Self::down_key) as well, or pick another sprint key.
    pub fn with_sprint(mut self, key: u32, multiplier: f32) -> Self {
        self.sprint_key = Some(key);
        self.sprint_multiplier = multiplier;
        self
    }

    /// The keys that move straight up and down in world space. Chainable.
    pub fn with_vertical_keys(mut self, up: u32, down: u32) -> Self {
        self.up_key = up;
        self.down_key = down;
        self
    }

    /// Initial yaw/pitch (rad). Chainable.
    pub fn looking(mut self, yaw: f32, pitch: f32) -> Self {
        self.yaw = yaw;
        self.pitch = pitch;
        self
    }

    /// Apply a mouse delta (pixels) to yaw/pitch (mouse right → yaw+, mouse up →
    /// pitch+); pitch is clamped to `±pitch_limit`. Pure/testable (the system calls
    /// this with the real mouse delta).
    ///
    /// **There is no `dt` here, and that is not an oversight.** A mouse delta is how far the
    /// device moved *during the frame that just happened*: the time is already inside the number.
    /// Multiplying it by `dt` would make the same physical mouse movement turn the camera further
    /// at low frame rates than at high ones — the classic way an FPS camera ends up feeling
    /// different on every machine. Contrast [`apply_stick_look`](Self::apply_stick_look), which
    /// takes `dt` for the opposite reason.
    pub fn apply_look(&mut self, mouse_dx: f32, mouse_dy: f32) {
        self.yaw += mouse_dx * self.sensitivity;
        self.pitch -= mouse_dy * self.sensitivity;
        self.pitch = self.pitch.clamp(-self.pitch_limit, self.pitch_limit);
    }

    /// Apply a right-stick deflection to yaw/pitch, over `dt` seconds (stick right → yaw+,
    /// stick up → pitch+); pitch is clamped to `±pitch_limit`.
    ///
    /// **`dt` is required here for the exact reason it is absent from
    /// [`apply_look`](Self::apply_look).** A stick reports a *standing* deflection — it reads 0.8
    /// for as long as the player holds it there, however many frames that takes — so what it
    /// describes is a turn *rate*. Without `dt` the camera would turn further per second the
    /// faster the machine runs, which is the same bug as scaling the mouse by `dt`, arrived at
    /// from the other side.
    ///
    /// The deflection is expected to be deadzoned already, i.e. straight from
    /// [`Gamepad::right_stick`](gizmo_core::input::Gamepad::right_stick).
    pub fn apply_stick_look(&mut self, stick_x: f32, stick_y: f32, dt: f32) {
        let rate = self.stick_sensitivity * dt;
        self.yaw += stick_x * rate;
        self.pitch += stick_y * rate;
        self.pitch = self.pitch.clamp(-self.pitch_limit, self.pitch_limit);
    }

    /// World-space forward (aim) direction vector — the same as [`Camera::forward_from`].
    pub fn forward(&self) -> Vec3 {
        Camera::forward_from(self.yaw, self.pitch)
    }

    /// World-space right direction vector (horizontal).
    pub fn right(&self) -> Vec3 {
        Camera::right_from(self.yaw)
    }
}

gizmo_core::impl_component!(FpsLook);

/// Drives the [`FpsLook`] cameras with the mouse + WASD every frame and writes to
/// `Camera`/`Transform`. [`FpsLookPlugin`] adds this to the schedule.
pub struct FpsLookSystem;

impl gizmo_core::system::System for FpsLookSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        info.is_exclusive = true; // FpsLook + Camera + Transform'a mutable erişir
        info
    }

    fn run(&mut self, world: &World, dt: f32) {
        // Fare-delta'sını Input resource'undan al (yoksa 0).
        let (mdx, mdy) = world
            .get_resource::<Input>()
            .map(|i| i.mouse_delta())
            .unwrap_or((0.0, 0.0));
        let key_code = |code: u32| {
            world
                .get_resource::<Input>()
                .map(|i| i.is_key_pressed(code))
                .unwrap_or(false)
        };
        let mouse_held = |button: u32| {
            world
                .get_resource::<Input>()
                .map(|i| i.is_mouse_button_pressed(button))
                .unwrap_or(false)
        };
        // Keys and the left stick as one direction, and the right stick for looking. Read once
        // here rather than per camera: several `FpsLook` cameras in one world all see the same
        // frame of input, and the shared blend is a pure function of it.
        let move_axis = world
            .get_resource::<Input>()
            .map(|i| i.move_axis())
            .unwrap_or((0.0, 0.0));
        let stick_look = world
            .get_resource::<Input>()
            .and_then(|i| i.gamepad().map(gizmo_core::input::Gamepad::right_stick))
            .unwrap_or((0.0, 0.0));

        // SAFETY: exclusive sistem; scheduler disjoint mutable erişim garanti eder.
        if let Some(mut q) = unsafe {
            world.query_unchecked::<(
                gizmo_core::query::Mut<FpsLook>,
                gizmo_core::query::Mut<Camera>,
                gizmo_core::query::Mut<Transform>,
            )>()
        } {
            for (_id, (mut look, mut cam, mut t)) in q.iter_mut() {
                if look.enabled {
                    // The gate covers the MOUSE only. A stick has no cursor to fight with, so
                    // requiring a held button of it would be a restriction with nothing behind
                    // it — the same conclusion the studio's camera reached about its fly keys.
                    let looking = look.look_button.is_none_or(&mouse_held);
                    if looking {
                        look.apply_look(mdx, mdy);
                    }
                    look.apply_stick_look(stick_look.0, stick_look.1, dt);

                    if look.move_speed > 0.0 {
                        // Yatay yön tuş+sol çubuk; dikey (Space/Shift) ayrı, çubukta karşılığı
                        // yok. Toplam normalize DEĞİL kırpılıyor: normalize, yarım yatırılmış
                        // çubuğu tam hıza çıkarıp çubuğun kattığı tek şeyi yok ederdi.
                        let (fwd, right) = (look.forward(), look.right());
                        let mut dir = right * move_axis.0 + fwd * move_axis.1;
                        if key_code(look.up_key) {
                            dir += Vec3::Y;
                        }
                        if key_code(look.down_key) {
                            dir -= Vec3::Y;
                        }
                        let len = dir.length();
                        if len > 1.0 {
                            dir /= len;
                        }
                        let sprint = match look.sprint_key {
                            Some(k) if key_code(k) => look.sprint_multiplier,
                            _ => 1.0,
                        };
                        t.position += dir * look.move_speed * sprint * dt;
                    }
                }

                // Kamera görüşünün doğruluk kaynağı: Camera.yaw/pitch (renderer get_view
                // bunları kullanır). Transform.rotation görüşü ETKİLEMEZ ama tutarlılık +
                // çocuk-entity bağlama için yaw'a göre ayarlanır.
                cam.yaw = look.yaw;
                cam.pitch = look.pitch;
                t.rotation = Quat::from_rotation_y(-look.yaw);
                t.update_local_matrix();
            }
        }
    }
}

/// Adds [`FpsLookSystem`] to the application's schedule → cameras with a [`FpsLook`]
/// component look around with the mouse (and move around with WASD).
pub struct FpsLookPlugin;

impl crate::app::Plugin for FpsLookPlugin {
    fn build(&self, app: &mut dyn crate::app::AppLike) {
        let app = app.parts_mut();
        // Per-frame, not per fixed step. This system reads `Input::mouse_delta`, which is
        // accumulated from window events and cleared once per rendered frame — so on the
        // fixed schedule it saw a fraction of the mouse motion (or, on frames where the
        // accumulator did not fill, none of it) and the camera stuttered. Mouse-look is
        // also a presentation concern: it should track the display, not the simulation.
        app.update_schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(FpsLookSystem)).label("fps_look"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::system::System;

    #[test]
    fn apply_look_updates_and_clamps() {
        let mut look = FpsLook::new().with_sensitivity(0.01);
        look.apply_look(100.0, 0.0); // yaw += 100·0.01 = 1.0
        assert!((look.yaw - 1.0).abs() < 1e-5);

        // Aşağı fare-delta'sı pitch'i azaltır (dy>0 → pitch-=), aşırı clamp'lenir.
        look.apply_look(0.0, 100000.0);
        assert!(look.pitch >= -look.pitch_limit - 1e-6);
        assert!((look.pitch + look.pitch_limit).abs() < 1e-4, "aşağı-clamp: {}", look.pitch);

        // Yukarı çok → +limit'e clamp.
        look.apply_look(0.0, -1_000_000.0);
        assert!((look.pitch - look.pitch_limit).abs() < 1e-4, "yukarı-clamp: {}", look.pitch);
    }

    /// The rule the two look inputs differ by, checked from both sides. Getting it backwards is
    /// invisible on the machine it was written on and obvious on any other.
    #[test]
    fn the_mouse_ignores_dt_and_the_stick_does_not() {
        // The mouse half is held by the SIGNATURE, not by this assertion: `apply_look` takes no
        // `dt`, so it cannot depend on one. Stated here anyway, because the property is the point
        // of the pair and a future `dt` parameter should have to delete a line that says why.
        let mut fast = FpsLook::new().with_sensitivity(0.01);
        let mut slow = FpsLook::new().with_sensitivity(0.01);
        fast.apply_look(100.0, 0.0);
        slow.apply_look(100.0, 0.0);
        assert_eq!(fast.yaw, slow.yaw, "a mouse delta must not depend on the frame rate");

        // A stick deflection is a rate: held for twice as long, it must turn twice as far.
        let mut short = FpsLook::new().with_stick_sensitivity(2.0);
        let mut long = FpsLook::new().with_stick_sensitivity(2.0);
        short.apply_stick_look(1.0, 0.0, 0.01);
        long.apply_stick_look(1.0, 0.0, 0.02);
        assert!(
            (long.yaw - 2.0 * short.yaw).abs() < 1e-6,
            "a stick held over twice the time must turn twice as far: {} against {}",
            long.yaw,
            short.yaw
        );
        // …and the rate is what the field says it is: 2 rad/s for 0.5 s is 1 rad.
        let mut one = FpsLook::new().with_stick_sensitivity(2.0);
        one.apply_stick_look(1.0, 0.0, 0.5);
        assert!((one.yaw - 1.0).abs() < 1e-6, "yaw {}", one.yaw);
    }

    #[test]
    fn stick_look_clamps_pitch_like_the_mouse_does() {
        let mut look = FpsLook::new().with_stick_sensitivity(10.0);
        look.apply_stick_look(0.0, 1.0, 100.0);
        assert!((look.pitch - look.pitch_limit).abs() < 1e-4, "up: {}", look.pitch);
        look.apply_stick_look(0.0, -1.0, 100.0);
        assert!((look.pitch + look.pitch_limit).abs() < 1e-4, "down: {}", look.pitch);
    }

    #[test]
    fn a_zero_stick_sensitivity_turns_stick_look_off() {
        let mut look = FpsLook::new().with_stick_sensitivity(0.0);
        look.apply_stick_look(1.0, 1.0, 1.0);
        assert_eq!((look.yaw, look.pitch), (0.0, 0.0));
    }

    /// The movement half, through the system, with a live pad in the world.
    #[test]
    fn the_left_stick_moves_an_fps_camera_and_carries_its_amount() {
        use gizmo_core::input::{GamepadAxis, GamepadId};

        fn travel(stick_y: f32) -> f32 {
            let mut world = World::new();
            let cam = world.spawn();
            world.add_component(cam, Transform::new(Vec3::ZERO));
            world.add_component(cam, Camera::new(1.0, 0.1, 100.0, 0.0, 0.0, true));
            world.add_component(cam, FpsLook::new().with_move_speed(10.0));

            let mut input = Input::new();
            let id = GamepadId::new(0);
            input.on_gamepad_connected(id, "test pad");
            input.on_gamepad_axis(id, GamepadAxis::LeftStickY, stick_y);
            world.insert_resource(input);

            FpsLookSystem.run(&world, 1.0);
            let transforms = world.borrow::<Transform>();
            transforms.get_entity(cam).unwrap().position.length()
        }

        let full = travel(1.0);
        assert!(full > 9.0, "a full stick did not move the camera: {full}");
        let half = travel(0.5); // past the 0.15 deadzone, well short of the rim
        assert!(half > 0.1, "a half tilt must move it at all: {half}");
        assert!(
            half < full * 0.8,
            "half a tilt must walk, not run: {half} against {full}"
        );
    }

    /// The gate the controller had no callers for want of.
    #[test]
    fn a_look_button_gates_the_mouse_and_not_the_stick() {
        use gizmo_core::input::{mouse, GamepadAxis, GamepadId};

        fn yaw_after(look: FpsLook, hold_button: bool, stick: f32) -> f32 {
            let mut world = World::new();
            let cam = world.spawn();
            world.add_component(cam, Transform::new(Vec3::ZERO));
            world.add_component(cam, Camera::new(1.0, 0.1, 100.0, 0.0, 0.0, true));
            world.add_component(cam, look);

            let mut input = Input::new();
            input.on_mouse_delta(100.0, 0.0);
            if hold_button {
                input.on_mouse_button_pressed(mouse::RIGHT);
            }
            if stick != 0.0 {
                let id = GamepadId::new(0);
                input.on_gamepad_connected(id, "pad");
                input.on_gamepad_axis(id, GamepadAxis::RightStickX, stick);
            }
            world.insert_resource(input);
            FpsLookSystem.run(&world, 1.0);
            world.borrow::<FpsLook>().get_entity(cam).unwrap().yaw
        }

        let gated = FpsLook::new().with_sensitivity(0.01).with_look_button(mouse::RIGHT);
        assert_eq!(
            yaw_after(gated, false, 0.0),
            0.0,
            "a mouse drag with the button up must not turn a gated camera"
        );
        assert!(
            yaw_after(gated, true, 0.0) > 0.0,
            "…and must turn it while the button is held"
        );
        // The stick is NOT gated: it has no cursor to fight with.
        assert!(
            yaw_after(gated, false, 1.0) > 0.0,
            "the gate is for the mouse; requiring a held button of a stick is a restriction with \
             nothing behind it"
        );
        // Ungated is the default, so a `FpsLook` written before the field behaves as it did.
        assert!(yaw_after(FpsLook::new().with_sensitivity(0.01), false, 0.0) > 0.0);
    }

    #[test]
    fn sprint_multiplies_the_distance_travelled() {
        use gizmo_core::input::{code_from_name, MoveKeys};

        fn travel(look: FpsLook, sprinting: bool) -> f32 {
            let mut world = World::new();
            let cam = world.spawn();
            world.add_component(cam, Transform::new(Vec3::ZERO));
            world.add_component(cam, Camera::new(1.0, 0.1, 100.0, 0.0, 0.0, true));
            world.add_component(cam, look);
            let mut input = Input::new();
            input.on_key_pressed(MoveKeys::WASD.forward);
            if sprinting {
                input.on_key_pressed(code_from_name("q").unwrap());
            }
            world.insert_resource(input);
            FpsLookSystem.run(&world, 1.0);
            world.borrow::<Transform>().get_entity(cam).unwrap().position.length()
        }

        let look = FpsLook::new()
            .with_move_speed(10.0)
            .with_sprint(code_from_name("q").unwrap(), 3.0);
        let walk = travel(look, false);
        let run = travel(look, true);
        assert!((walk - 10.0).abs() < 1e-4, "walk {walk}");
        assert!((run - 30.0).abs() < 1e-3, "sprint should treble it: {run}");
    }

    /// The collision the sprint field exists to make visible: ShiftLeft already means *descend*.
    #[test]
    fn the_vertical_keys_are_configurable_because_shift_is_taken() {
        use gizmo_core::input::code_from_name;
        let look = FpsLook::new();
        assert_eq!(
            look.down_key,
            KeyCode::ShiftLeft as u32,
            "the default descend key is what a shift-to-sprint caller collides with"
        );
        let moved = FpsLook::new().with_vertical_keys(
            code_from_name("r").unwrap(),
            code_from_name("f").unwrap(),
        );
        assert_eq!(moved.up_key, code_from_name("r").unwrap());
        assert_eq!(moved.down_key, code_from_name("f").unwrap());
    }

    #[test]
    fn forward_matches_camera_helper() {
        let look = FpsLook::new().looking(0.7, 0.3);
        let f = look.forward();
        let c = Camera::forward_from(0.7, 0.3);
        assert!((f - c).length() < 1e-6);
        // İleri vektör birim uzunlukta.
        assert!((f.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn system_syncs_yaw_pitch_to_camera() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        world.add_component(e, Camera::new(1.0, 0.1, 100.0, 0.0, 0.0, true));
        world.add_component(e, FpsLook::new().looking(1.2, 0.4));

        // Input resource'u yok → mouse-delta 0; sistem yalnız senkron yapar.
        let mut sys = FpsLookSystem;
        sys.run(&world, 1.0 / 60.0);

        let cams = world.borrow::<Camera>();
        let cam = cams.get(e.id()).unwrap();
        assert!((cam.yaw - 1.2).abs() < 1e-5 && (cam.pitch - 0.4).abs() < 1e-5);
    }
}
