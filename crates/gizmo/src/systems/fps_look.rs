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
    /// Mouse sensitivity (rad / pixel).
    pub sensitivity: f32,
    /// WASD movement speed (units/s). 0 → look only (the camera stays put).
    pub move_speed: f32,
    /// Pitch is clamped to ± this value (rad) (prevents going upside-down).
    pub pitch_limit: f32,
    /// false → the system SKIPS this camera (for menus, cutscenes, autoplay).
    pub enabled: bool,
}

impl Default for FpsLook {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            sensitivity: 0.0025,
            move_speed: 0.0,
            pitch_limit: std::f32::consts::FRAC_PI_2 - 0.05,
            enabled: true,
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

    /// Initial yaw/pitch (rad). Chainable.
    pub fn looking(mut self, yaw: f32, pitch: f32) -> Self {
        self.yaw = yaw;
        self.pitch = pitch;
        self
    }

    /// Apply a mouse delta (pixels) to yaw/pitch (mouse right → yaw+, mouse up →
    /// pitch+); pitch is clamped to `±pitch_limit`. Pure/testable (the system calls
    /// this with the real mouse delta).
    pub fn apply_look(&mut self, mouse_dx: f32, mouse_dy: f32) {
        self.yaw += mouse_dx * self.sensitivity;
        self.pitch -= mouse_dy * self.sensitivity;
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
        let key = |c: KeyCode| {
            world
                .get_resource::<Input>()
                .map(|i| i.is_key_pressed(c as u32))
                .unwrap_or(false)
        };

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
                    look.apply_look(mdx, mdy);

                    if look.move_speed > 0.0 {
                        let mut dir = Vec3::ZERO;
                        let (fwd, right) = (look.forward(), look.right());
                        if key(KeyCode::KeyW) {
                            dir += fwd;
                        }
                        if key(KeyCode::KeyS) {
                            dir -= fwd;
                        }
                        if key(KeyCode::KeyD) {
                            dir += right;
                        }
                        if key(KeyCode::KeyA) {
                            dir -= right;
                        }
                        if key(KeyCode::Space) {
                            dir += Vec3::Y;
                        }
                        if key(KeyCode::ShiftLeft) {
                            dir -= Vec3::Y;
                        }
                        if dir.length_squared() > 1e-9 {
                            t.position += dir.normalize() * look.move_speed * dt;
                        }
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
