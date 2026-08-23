//! # Oyun kolu girdisi
//!
//! Oyun kolu: çubuklar, tetikler, düğmeler. Demo bağlı kolun sol çubuğunu ve tetiklerini okuyup
//! hem HUD'a basıyor hem de bir küpü sürüyor.
//!
//! **Seride oyun kolunu okuyan ilk demo.**
//!
//! ## Motorun yüzeyi geniş
//!
//! Gizmo'nun [`Gamepad`]'i doğrudan adlandırılmış erişim veriyor:
//! [`Gamepad::left_stick`], [`right_stick`](Gamepad::right_stick),
//! [`left_trigger`](Gamepad::left_trigger), [`right_trigger`](Gamepad::right_trigger),
//! [`dpad`](Gamepad::dpad), artı düğme kenarları
//! ([`is_just_pressed`](Gamepad::is_just_pressed) / `is_just_released`) ve
//! [`pressed_buttons`](Gamepad::pressed_buttons).
//!
//! Ve **ölü bölge motorun içinde**, uygulamanın değil: [`GamepadDeadzone`] çubuk için 0,15,
//! tetik için 0,05 varsayılanıyla geliyor, ve çubuğunki **radyal** — eşik her eksende ayrı değil,
//! çubuğun merkezden uzaklığında.
//!
//! Gerçek kollar `gilrs` üzerinden geliyor (`gizmo-app/src/gamepad.rs`), ve `gamepad` özelliği
//! varsayılan olarak açık — tarayıcı dahil her hedefte.
//!
//! ## Kol yoksa da doğrulanabiliyor
//!
//! Bu makinede fiziksel kol yok. Ama motorun olay girişleri **açık**:
//! `Input::on_gamepad_connected`, `on_gamepad_button_pressed`, `on_gamepad_axis`. Motorun kendi
//! testleri de sanal kolu böyle kuruyor. `GIZMO_PAD_SELFTEST=1` demoya bir sanal kol taktırıyor
//! ve sol çubuğu bir tam tur döndürüyor — yani okuma yolunun tamamı, kol olmadan, ölçülebiliyor.
//!
//! ## Ölçülen: radyal ölü bölgenin eğrisi
//!
//! Sanal çubuk merkezden dışa doğru süpürülüp okunan değer kaydedildiğinde motorun
//! [`apply_stick_deadzone`]'unun ne yaptığı doğrudan görünüyor (2026-08-23, çubuk ölü bölgesi
//! 0,15):
//!
//! | ham büyüklük | okunan | `(m − 0,15) / 0,85` |
//! |--------------|--------|---------------------|
//! | 0,000 | 0,000 | — |
//! | 0,100 | **0,000** | eşiğin altı |
//! | 0,200 | 0,059 | 0,059 |
//! | 0,300 | 0,176 | 0,176 |
//! | 0,500 | 0,412 | 0,412 |
//! | 0,700 | 0,647 | 0,647 |
//! | 0,900 | 0,882 | 0,882 |
//! | 1,000 | 1,000 | 1,000 |
//!
//! Eşiğin altı tam sıfır, üstü ise kalan yolu **yeniden ölçekliyor** — yani ölü bölge menzil
//! yemiyor, sadece başlangıcı kaydırıyor. Ve **yön birebir korunuyor**: ham (−0,433 · +0,250)
//! okunduğunda (−0,357 · +0,206) çıkıyor, iki bileşenin oranı da 0,824 — aynı çarpan, aynı yön.
//! Motorun kendi yorumu bunu "kırpılmış büyüklüğe değil ORİJİNAL büyüklüğe böl" diye anlatıyor.
//!
//! ## Tuzak: kolu bir kez duyurmak yetmiyor
//!
//! `on_gamepad_connected` bir kez çağrıldığında kolun **adı bir süre sonra boşalıyor**: eksen
//! olayı bilinmeyen bir kimlik için kolu yeniden yaratıyor ve o yol adı boş bırakıyor
//! (`Gamepads::entry`, `Gamepad::new(id, "", …)`). Demo bu yüzden **her kare** duyuruyor — ki
//! motorun belgesi de bunu güvenli ve beklenen kullanım diye yazıyor ("aynı kimlik için iki kez
//! çağırmak güvenlidir … yalnız adını tazeler"), ve platform katmanı da böyle yapıyor.
//!
//! ## Kontroller
//!   * **Sol çubuk** — küpü taşı · **Sağ çubuk** — döndür · **Tetikler** — büyüt/küçült
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::{
    apply_stick_deadzone, GamepadAxis, GamepadButton, GamepadDeadzone, GamepadId, Input,
};
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::TAU;

/// Kolla sürülen küpün işareti.
#[derive(Clone, Copy)]
struct Pawn;
gizmo::core::impl_component!(Pawn);

/// HUD'un ve kendi kendine sınamanın defteri.
#[derive(Default, Clone)]
struct PadState {
    connected: usize,
    name: String,
    left: (f32, f32),
    right: (f32, f32),
    triggers: (f32, f32),
    dpad: (f32, f32),
    buttons: Vec<String>,
    frame: u32,
    /// Sanal kol kuruldu mu.
    synthetic: bool,
}
gizmo::core::impl_component!(PadState);

/// Sanal kolun kimliği — gerçek bir kolla çakışmasın diye yüksek.
const SYNTHETIC_PAD: u32 = 9001;
/// Küpün çubukla gidebileceği en uzak nokta.
const REACH: f32 = 4.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Gamepad Input", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 20.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_scale(Vec3::splat(0.6)),
                GlobalTransform::default(),
                AssetManager::create_cube(device),
                Material::new(white).with_pbr(Vec4::new(0.85, 0.55, 0.25, 1.0), 0.4, 0.0),
                MeshRenderer::new(),
                Pawn,
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.8),
                intensity: 2.6,
                ..Default::default()
            });

            scene.world.insert_resource(PadState::default());
            scene.spawn_camera(state, Vec3::new(0.0, 5.0, 9.0), Vec3::ZERO);
        })
        .add_update_system(drive_selftest.in_phase(Phase::Update).label("pad_selftest"))
        .add_update_system(read_pad.in_phase(Phase::Update).after("pad_selftest"))
        .set_ui(|world, _state, ctx| {
            let Some(pad) = world.get_resource::<PadState>().map(|p| p.clone()) else {
                return;
            };
            gizmo::egui::Area::new("pad".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(430.0);
                        ui.heading("Oyun kolu");
                        if pad.connected == 0 {
                            ui.label("bağlı kol yok");
                            ui.label("GIZMO_PAD_SELFTEST=1 sanal kol takar");
                        } else {
                            ui.label(format!("{} kol · aktif: {}", pad.connected, pad.name));
                            if pad.synthetic {
                                ui.label("(sanal kol — olay girişlerinden besleniyor)");
                            }
                        }
                        ui.separator();
                        ui.label(format!(
                            "sol çubuk : ({:+.3}, {:+.3})",
                            pad.left.0, pad.left.1
                        ));
                        ui.label(format!(
                            "sağ çubuk : ({:+.3}, {:+.3})",
                            pad.right.0, pad.right.1
                        ));
                        ui.label(format!(
                            "tetikler  : sol {:.3} · sağ {:.3}",
                            pad.triggers.0, pad.triggers.1
                        ));
                        ui.label(format!("dpad      : ({:+.0}, {:+.0})", pad.dpad.0, pad.dpad.1));
                        ui.label(format!("basılı    : {}", pad.buttons.join(" ")));
                        ui.separator();
                        ui.label(format!(
                            "ölü bölge motorda: çubuk {:.2} (RADYAL) · tetik {:.2}",
                            GamepadDeadzone::DEFAULT_STICK,
                            GamepadDeadzone::DEFAULT_TRIGGER
                        ));
                        ui.label("ölü bölge oyunun değil motorun işi");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// `GIZMO_PAD_SELFTEST=1` iken bir sanal kol takar ve sol çubuğu süpürür.
///
/// Motorun olay girişleri açık olduğu için okuma yolunun tamamı fiziksel kol olmadan
/// çalıştırılabiliyor — motorun kendi testleri de sanal kolu böyle kuruyor.
fn drive_selftest(mut input: ResMut<Input>, mut pad: ResMut<PadState>) {
    if std::env::var("GIZMO_PAD_SELFTEST").is_err() {
        return;
    }
    let id = GamepadId::new(SYNTHETIC_PAD);
    // HER KARE duyuruluyor, bir kez değil. Motorun kendi belgesi bunu güvenli ve beklenen
    // kullanım diye yazıyor ("aynı kimlik için iki kez çağırmak güvenlidir … yalnız adını
    // tazeler"), ve platform katmanı da tam olarak böyle yapıyor. Tek seferlik duyuruda kolun
    // ADI bir süre sonra boşalıyordu: eksen olayı bilinmeyen bir kimlik için kolu yeniden
    // yaratıyor ve o yol adı boş bırakıyor.
    input.on_gamepad_connected(id, "sanal kol");
    pad.synthetic = true;

    // Çubuğu bir tam tur döndür, yarıçapı da yavaşça büyüt: hem yön hem büyüklük süpürülüyor.
    let t = pad.frame as f32 / 240.0;
    let angle = t * TAU;
    let radius = (t * 1.2).min(1.0);
    input.on_gamepad_axis(id, GamepadAxis::LeftStickX, angle.cos() * radius);
    input.on_gamepad_axis(id, GamepadAxis::LeftStickY, angle.sin() * radius);
    input.on_gamepad_axis(id, GamepadAxis::RightStickX, (t * 2.0).sin());
    input.on_gamepad_axis(id, GamepadAxis::RightTrigger, (t * 0.7).min(1.0));

    // Dönüşümlü olarak bir düğmeye bas: kenar okuması da çalışsın.
    if pad.frame.is_multiple_of(120) {
        input.on_gamepad_button_pressed(id, GamepadButton::South);
    } else if pad.frame % 120 == 30 {
        input.on_gamepad_button_released(id, GamepadButton::South);
    }

    // Ham çubuk ile okunan değeri yan yana günlüğe bas: ölü bölgenin eğrisi bu.
    if pad.frame.is_multiple_of(20) {
        let (dx, dy) = apply_stick_deadzone(
            angle.cos() * radius,
            angle.sin() * radius,
            GamepadDeadzone::DEFAULT_STICK,
        );
        gizmo::gizmo_log!(
            Info,
            "ham |{:.3}| ({:+.3}, {:+.3}) -> okunan |{:.3}| ({:+.3}, {:+.3})",
            radius,
            angle.cos() * radius,
            angle.sin() * radius,
            (dx * dx + dy * dy).sqrt(),
            dx,
            dy
        );
    }
}

/// Kolu okur, küpü sürer, defteri doldurur.
fn read_pad(
    mut pawns: Query<(Mut<Transform>, With<Pawn>)>,
    mut pad: ResMut<PadState>,
    input: Res<Input>,
) {
    pad.frame += 1;
    pad.connected = input.gamepads().iter().count();

    let Some(gamepad) = input.gamepad() else {
        pad.name.clear();
        return;
    };
    if std::env::var("GIZMO_PAD_SELFTEST").is_ok() && pad.frame.is_multiple_of(60) {
        let names: Vec<String> = input
            .gamepads()
            .iter()
            .map(|g| format!("#{:?}='{}'", g.id(), g.name()))
            .collect();
        gizmo::gizmo_log!(Info, "kare {} · kollar: {}", pad.frame, names.join(" "));
    }
    pad.name = gamepad.name().to_string();
    pad.left = gamepad.left_stick();
    pad.right = gamepad.right_stick();
    pad.triggers = (gamepad.left_trigger(), gamepad.right_trigger());
    pad.dpad = gamepad.dpad();
    pad.buttons = gamepad
        .pressed_buttons()
        .map(|b| format!("{b:?}"))
        .collect();

    let (lx, ly) = pad.left;
    let (rx, _) = pad.right;
    let scale = 0.6 + pad.triggers.1 * 0.8 - pad.triggers.0 * 0.4;
    for (_entity, (mut transform, _)) in pawns.iter_mut() {
        // Çubuğun Y'si ileri: ekranda −Z.
        transform.position = Vec3::new(lx * REACH, 0.0, -ly * REACH);
        transform.rotation = Quat::from_rotation_y(rx * TAU * 0.25);
        transform.scale = Vec3::splat(scale.clamp(0.2, 1.6));
    }
}
