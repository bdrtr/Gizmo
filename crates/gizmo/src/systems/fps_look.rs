//! Birinci-şahıs BAKIŞ denetleyici — fare-look (+ opsiyonel WASD uçuş hareketi).
//!
//! Standart bir FP / serbest-uçuş kamerası neredeyse her oyunda ve araçta gerekir; ama
//! demolar bunu her frame ELLE yazar: `mouse_delta → yaw/pitch`, pitch clamp, WASD →
//! konum, sonra `Camera` + `Transform` senkronu (bkz. yikim_ustasi'nin elle kamera
//! döngüsü). `FpsLook` komponentini kamera entity'sine ekle, [`FpsLookPlugin`]'i çalıştır;
//! motor fareyle baktırsın (ve `move_speed>0` ise WASD ile gezdirsin). Nişan/atış yönü:
//! [`FpsLook::forward`] — "front" matematiğini kopyalamana gerek yok.
//!
//! ```ignore
//! world.add_component(cam, FpsLook::new().with_move_speed(8.0)); // fare-look + WASD uçuş
//! app.add_plugin(FpsLookPlugin);
//! // ateş ederken:  let dir = look.forward();
//! ```

use gizmo_core::input::Input;
use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::Transform;
use gizmo_renderer::components::Camera;
use winit::keyboard::KeyCode;

/// Bir kamera entity'sine eklenen fare-look denetleyicisi. `yaw`/`pitch` bu komponentte
/// TUTULUR (tek doğruluk kaynağı); sistem her frame fare/WASD'yi işleyip `Camera` +
/// `Transform`'a yazar. Nişan yönü [`forward`](Self::forward) ile okunur.
#[derive(Debug, Clone, Copy)]
pub struct FpsLook {
    /// Yatay bakış açısı (rad).
    pub yaw: f32,
    /// Dikey bakış açısı (rad); `±pitch_limit`'e clamp'lenir.
    pub pitch: f32,
    /// Fare duyarlılığı (rad / piksel).
    pub sensitivity: f32,
    /// WASD hareket hızı (birim/sn). 0 → yalnız bakış (kamera sabit durur).
    pub move_speed: f32,
    /// Pitch bu değere (rad) ± clamp'lenir (tepetaklak olmayı önler).
    pub pitch_limit: f32,
    /// false → sistem bu kamerayı ATLAR (menü, ara sahne, autoplay için).
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
    /// Varsayılan fare-look denetleyici (hareketsiz — yalnız bakış).
    pub fn new() -> Self {
        Self::default()
    }

    /// WASD uçuş hareketini `speed` birim/sn ile aç. Zincirlenebilir.
    pub fn with_move_speed(mut self, speed: f32) -> Self {
        self.move_speed = speed;
        self
    }

    /// Fare duyarlılığını ayarla (rad/piksel). Zincirlenebilir.
    pub fn with_sensitivity(mut self, s: f32) -> Self {
        self.sensitivity = s;
        self
    }

    /// Başlangıç yaw/pitch'i (rad). Zincirlenebilir.
    pub fn looking(mut self, yaw: f32, pitch: f32) -> Self {
        self.yaw = yaw;
        self.pitch = pitch;
        self
    }

    /// Bir fare-delta'sını (piksel) yaw/pitch'e uygula (fare sağ → yaw+, fare yukarı →
    /// pitch+); pitch `±pitch_limit`'e clamp'lenir. Saf/test edilebilir (sistem bunu
    /// gerçek fare-delta'sıyla çağırır).
    pub fn apply_look(&mut self, mouse_dx: f32, mouse_dy: f32) {
        self.yaw += mouse_dx * self.sensitivity;
        self.pitch -= mouse_dy * self.sensitivity;
        self.pitch = self.pitch.clamp(-self.pitch_limit, self.pitch_limit);
    }

    /// Dünya-uzayı ileri (nişan) yön vektörü — [`Camera::forward_from`] ile aynı.
    pub fn forward(&self) -> Vec3 {
        Camera::forward_from(self.yaw, self.pitch)
    }

    /// Dünya-uzayı sağ yön vektörü (yatay).
    pub fn right(&self) -> Vec3 {
        Camera::right_from(self.yaw)
    }
}

gizmo_core::impl_component!(FpsLook);

/// Her frame [`FpsLook`] kameralarını fare + WASD ile sürer ve `Camera`/`Transform`'a
/// yazar. [`FpsLookPlugin`] bunu schedule'a ekler.
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

/// [`FpsLookSystem`]'i uygulamanın schedule'ına ekler → [`FpsLook`] komponentli kameralar
/// fareyle bakar (ve WASD ile gezer).
pub struct FpsLookPlugin;

impl<State: 'static> crate::app::Plugin<State> for FpsLookPlugin {
    fn build(&self, app: &mut crate::app::App<State>) {
        app.schedule.add_di_system(
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
