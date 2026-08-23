//! # Bevy'nin `input/gamepad_rumble` örneğinin Gizmo Engine portu
//!
//! Kolu titretmek. Bevy'de `GamepadRumbleRequest::Add { duration, intensity, gamepad }` diye bir
//! olay yazılıyor. Gizmo'da karşılığı [`Input::rumble`] / [`Input::rumble_pad`] /
//! [`Input::stop_rumble`], ve arkasında bir istek kuyruğu var.
//!
//! ## İki motor, tek şiddet değil
//!
//! Bevy'nin `intensity`'si tek sayı. Gizmo [`RumbleRequest`] iki motoru ayrı istiyor:
//! **`weak`** (küçük, yüksek frekanslı — vızıltı) ve **`strong`** (büyük, alçak frekanslı —
//! gümbürtü). Motorun kendi yorumu adlandırmanın gerekçesini de yazıyor: `left`/`right` demek
//! *bir kolun plastiği* hakkında bir şey söylerdi, oysa istenen şey **hissin kendisi**.
//!
//! ## Kuyruğu oyun değil platform katmanı boşaltıyor
//!
//! [`Input::take_rumble_requests`] kuyruğu **boşaltarak** okuyor, ve onu çağıran
//! `gizmo-app`'in kol arka ucu (`gamepad.rs`). Yani pencereli bir oyunda kendi isteklerinizi
//! geri okuyamazsınız — sıra size geldiğinde kuyruk çoktan boşalmış olur. Demo bunu HUD'da
//! ölçüyor: istek yazıldıktan sonra `has_rumble_requests()` ne diyor.
//!
//! ## Ölçülen: `RumbleRequest::new`'in kırpma sözleşmesi
//!
//! Alanlar `pub`, yani kırpmanın yeri yapıcı. Motorun yorumu neden orada olduğunu da söylüyor:
//! 1'in üstündeki bir büyüklük "daha gürültülü titreşim" değil, iki katman aşağıda değerin `u16`
//! olduğu yerde bir **taşma**.
//!
//! `GIZMO_RUMBLE_SELFTEST=1` bilerek aralık dışı değerler verip sonucu günlüğe basıyor
//! (2026-08-23):
//!
//! | istenen (zayıf, güçlü, süre) | kırpılmış | `is_stop()` |
//! |------------------------------|-----------|-------------|
//! | (+5,00 · −2,00 · −1,00) | (1,00 · 0,00 · 0,00) | false |
//! | (+0,50 · +1,50 · +0,25) | (0,50 · 1,00 · 0,25) | false |
//! | (+0,00 · +0,00 · +0,40) | (0,00 · 0,00 · 0,40) | **true** |
//!
//! İlk satır dikkate değer: yarısı geçersiz bir istek **reddedilmiyor**, kırpılıp tek motorlu bir
//! titreşime dönüşüyor — güçlü motor için istenen −2,00 sessizce 0'a iniyor ve isteğin kendisi
//! ayakta kalıyor. Üçüncü satır da sözleşmenin öbür ucu: iki motor da sıfırsa bu "hiçbir şey
//! çalma" değil **"durdur"** demek, ve `is_stop()` bunu ayırt ediyor.
//!
//! ## Ölçülen: kuyruğu gerçekten platform katmanı boşaltıyor
//!
//! Demo isteği yazdıktan hemen sonra ve bir sonraki karede `has_rumble_requests()` soruyor:
//!
//! | an | kuyrukta var mı |
//! |----|-----------------|
//! | yazdıktan hemen sonra | **true** |
//! | bir sonraki karede | **false** |
//!
//! Yani istek araya giren kare sınırında tüketiliyor. Bu, kendi titreşim isteklerinizi geri
//! okuyup denetleyemeyeceğiniz anlamına geliyor — kuyruk sizin için değil, arka uç için.
//!
//! ## Kontroller
//!   * **Boşluk** — elle titret · **X** — durdur (S kameranın: WASDQE)
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::{GamepadId, Input, RumbleRequest};
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Zıplayan küpün işareti ve düşey hızı.
#[derive(Clone, Copy)]
struct Bouncer {
    velocity: f32,
}
gizmo::core::impl_component!(Bouncer);

/// HUD'un defteri.
#[derive(Default, Clone)]
struct RumbleLog {
    /// Kaç kez titreşim istendi.
    requests: u32,
    /// En son istenen değerler (kırpmadan sonra).
    last: Option<RumbleRequest>,
    /// İstek yazıldıktan hemen sonra kuyrukta bir şey var mıydı.
    queue_after_write: bool,
    /// Bir sonraki karede kuyruk hâlâ dolu muydu — platform katmanı boşalttı mı.
    queue_next_frame: bool,
    selftest_done: bool,
}
gizmo::core::impl_component!(RumbleLog);

/// Titreşimin gönderileceği kol.
const PAD: u32 = 0;
/// Zemin yüksekliği ve yerçekimi.
const FLOOR: f32 = 0.5;
const GRAVITY: f32 = -14.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Gamepad Rumble", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 16.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 4.0, 0.0)).with_scale(Vec3::splat(0.5)),
                GlobalTransform::default(),
                AssetManager::create_cube(device),
                Material::new(white).with_pbr(Vec4::new(0.90, 0.45, 0.25, 1.0), 0.4, 0.0),
                MeshRenderer::new(),
                Bouncer { velocity: 0.0 },
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.7),
                intensity: 2.6,
                ..Default::default()
            });

            scene.world.insert_resource(RumbleLog::default());
            scene.spawn_camera(state, Vec3::new(0.0, 3.0, 8.0), Vec3::new(0.0, 1.2, 0.0));
        })
        .add_update_system(bounce_and_rumble.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(log) = world.get_resource::<RumbleLog>().map(|l| l.clone()) else {
                return;
            };
            gizmo::egui::Area::new("rumble".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(440.0);
                        ui.heading("Kol titreşimi");
                        ui.label(format!("istek sayısı: {}", log.requests));
                        match log.last {
                            Some(r) => {
                                ui.label(format!(
                                    "son istek: zayıf {:.2} · güçlü {:.2} · süre {:.2} s",
                                    r.weak, r.strong, r.duration_secs
                                ));
                                ui.label(format!("  durdurma isteği mi: {}", r.is_stop()));
                            }
                            None => {
                                ui.label("henüz istek yok");
                            }
                        }
                        ui.separator();
                        ui.label(format!(
                            "yazdıktan hemen sonra kuyrukta var mı: {}",
                            log.queue_after_write
                        ));
                        ui.label(format!(
                            "bir sonraki karede hâlâ var mı: {}",
                            log.queue_next_frame
                        ));
                        ui.label("  kuyruğu platform katmanı BOŞALTARAK okuyor");
                        ui.separator();
                        ui.label("iki motor ayrı: zayıf = vızıltı, güçlü = gümbürtü");
                        ui.label("Bevy'de tek 'intensity' var");
                        ui.separator();
                        ui.label("boşluk — elle titret · X — durdur");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Küpü zıplatır, her çarpışmada çarpma şiddetiyle orantılı titreşim ister.
fn bounce_and_rumble(
    mut bouncers: Query<(Mut<Transform>, Mut<Bouncer>, With<Bouncer>)>,
    mut log: ResMut<RumbleLog>,
    mut input: ResMut<Input>,
    time: Res<Time>,
) {
    use gizmo::winit::keyboard::KeyCode;
    let dt = time.dt().min(0.05);
    let pad = GamepadId::new(PAD);

    // Geçen karede yazılan istek hâlâ duruyor mu — platform katmanı araya girdi mi.
    log.queue_next_frame = input.has_rumble_requests();

    // Aralık dışı değerlerin ne olduğunu bir kez ölç.
    if std::env::var("GIZMO_RUMBLE_SELFTEST").is_ok() && !log.selftest_done {
        log.selftest_done = true;
        for (weak, strong, secs) in [
            (5.0f32, -2.0f32, -1.0f32),
            (0.5, 1.5, 0.25),
            (0.0, 0.0, 0.4),
        ] {
            let r = RumbleRequest::new(pad, weak, strong, secs);
            gizmo::gizmo_log!(
                Info,
                "istenen ({:+.2}, {:+.2}, {:+.2}) -> kırpılmış ({:.2}, {:.2}, {:.2}) · durdurma: {}",
                weak,
                strong,
                secs,
                r.weak,
                r.strong,
                r.duration_secs,
                r.is_stop()
            );
        }
    }

    let mut fired: Option<(f32, f32, f32)> = None;

    for (_entity, (mut transform, mut bouncer, _)) in bouncers.iter_mut() {
        bouncer.velocity += GRAVITY * dt;
        transform.position.y += bouncer.velocity * dt;
        if transform.position.y <= FLOOR {
            transform.position.y = FLOOR;
            // Çarpma şiddeti düşüş hızından: sert iniş güçlü motoru, yumuşak iniş vızıltıyı.
            let impact = (-bouncer.velocity / 12.0).clamp(0.0, 1.0);
            bouncer.velocity = 7.5;
            if impact > 0.05 {
                fired = Some((impact * 0.4, impact, 0.12 + impact * 0.15));
            }
        }
    }

    if input.is_key_just_pressed(KeyCode::Space as u32) {
        fired = Some((0.8, 0.3, 0.3));
    }
    if input.is_key_just_pressed(KeyCode::KeyX as u32) {
        input.stop_rumble();
        log.requests += 1;
        log.last = Some(RumbleRequest::new(pad, 0.0, 0.0, 0.0));
        log.queue_after_write = input.has_rumble_requests();
        return;
    }

    if let Some((weak, strong, secs)) = fired {
        // Belirli bir kola: `rumble()` etkin kola gider, `rumble_pad()` seçilene.
        input.rumble_pad(pad, weak, strong, secs);
        log.requests += 1;
        log.last = Some(RumbleRequest::new(pad, weak, strong, secs));
        log.queue_after_write = input.has_rumble_requests();
    }
}
