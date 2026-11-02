//! # Menü — fareyle, klavyeyle ve gamepad'le kullanılan bir oyun menüsü
//!
//! Her oyunun ilk ekranı: Başla, Tam ekran, Ses, Çıkış. Bu demo bir oyun değil, oyunların hepsinin
//! ihtiyaç duyduğu ve motorun yapamadığı dört şeyi gösteriyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **fare** | üzerine gel, tıkla |
//! | **↑ ↓ / Tab · Enter / Space** | gezin · seç |
//! | **gamepad d-pad · A (güney)** | gezin · seç |
//! | **İsim alanı** | tıkla, yaz, Enter ile kaydet (Esc bırakır) |
//!
//! ## Motorda bulunan eksikler (bu menüyü yazarken)
//!
//! - **Klavye/gamepad ile gezinme yoktu.** `gizmo-ui`'nin odak kavramı yoktu; her menü yalnızca
//!   fareyle kullanılıyordu. **Kapatıldı:** `UiNavigation` kaynağı — açıldığında oklar/Tab/d-pad
//!   odağı okuma sırasında taşıyor, Enter/Space/güney düğmesi odaktakini tıklıyor (`Clicked`),
//!   fare bir düğmeye gelince odak onunla gidiyor, odaktaki düğme `Hovered` görünüyor. Oyunun
//!   tuşlarını çalmasın diye varsayılan kapalı.
//! - **Çalışırken tam ekran yoktu.** Pencere pencereli açılıyor ve öyle kalıyordu. **Kapatıldı:**
//!   `WindowMode::{Windowed, Fullscreen}` kaynağı; değiştiği kare pencereye uygulanıyor
//!   (kenarlıksız tam ekran).
//! - **Oyun çıkış isteyemiyordu.** Pencereli döngü yalnızca pencere kapanınca, başsız döngü hiç
//!   bitmiyordu. **Kapatıldı:** `AppExit` kaynağı; iki döngü de o karenin sonunda pencereyi
//!   kapatmakla aynı yoldan çıkıyor.
//!
//! - **Ayarlar kapanınca kayboluyordu.** **Kapatıldı:** `SaveData` — ses ve tam ekran
//!   `gizmo_menu/save.json`'da, menü onlarla açılıyor.
//!
//! - **Metin girişi yoktu.** Girdi katmanı yalnızca fiziksel tuş bildiriyordu: `KeyA`, ama
//!   `a` mı `A` mı bilinmiyordu, `ş` hiç okunamıyordu. **Kapatıldı:** `Input::typed_text()` (bu
//!   karede yazılan karakterler, klavye düzeninin ürettiği gibi) ve `gizmo::ui::TextInput` —
//!   tıklanınca odaklanan, Backspace/Enter/Esc bilen, değeri `Text`'in yer tutucusunun yerine
//!   imleçle çizilen bir alan. Odaktayken `UiNavigation` tuşları ona bırakıyor.
//!
//! ## Ortam değişkenleri
//!   * `GIZMO_FONT=<yol>` — font

use gizmo::audio::synth::{Blip, Wave};
use gizmo::app::save::SaveData;
use gizmo::audio::AudioManager;
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::core::window::{AppExit, WindowMode};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::ui::{
    AlignItems, BackgroundColor, ButtonBundle, Clicked, FlexDirection, Interaction, JustifyContent, NodeBundle, Style,
    TextInput, UiNavigation, UiPlugin, UiRect, Val,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Start,
    Fullscreen,
    Volume,
    Quit,
}

#[derive(Clone, Copy)]
struct MenuButton(Action);
gizmo::core::impl_component!(MenuButton);

struct Menu {
    volume: u32,
    banner: u32,
}

fn fill(i: Interaction) -> Vec4 {
    match i {
        Interaction::Pressed => Vec4::new(0.25, 0.55, 0.9, 1.0),
        Interaction::Hovered => Vec4::new(0.3, 0.34, 0.44, 1.0),
        _ => Vec4::new(0.14, 0.16, 0.21, 1.0),
    }
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Menu", 1280, 720)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);

            // Arkada yavaşça dönen bir şey olsun.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(2.5, 0.0, 0.0)),
                GlobalTransform::default(),
                MeshSource("cone".into()),
                MaterialDesc::new().with_pbr(Vec4::new(0.85, 0.5, 0.25, 1.0), 0.4, 0.2),
                MeshRenderer::new(),
                Tween::to_rotation(Quat::from_rotation_y(std::f32::consts::PI), 4.0).repeating(gizmo::renderer::tween::Repeat::PingPong),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-0.8)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::ONE, 3.0, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle { position: Vec3::new(0.0, 0.5, 6.0), yaw: -std::f32::consts::FRAC_PI_2, primary: true, ..Default::default() });

            scene.world.spawn_bundle((Text::pinned_at("GIZMO", font, 72.0, Vec2::new(0.25, 0.18), Vec2::ZERO).with_anchor(TextAnchor::Center),));

            // Sütun: dört düğme.
            let column = scene.world.spawn_bundle(NodeBundle {
                style: Style {
                    width: Val::Px(320.0),
                    height: Val::Percent(100.0),
                    padding: UiRect::all(Val::Px(48.0)),
                    row_gap: Val::Px(14.0),
                    flex_direction: FlexDirection::Column,
                    justify_content: Some(JustifyContent::Center),
                    align_items: Some(AlignItems::Stretch),
                    ..Default::default()
                },
                background_color: BackgroundColor(Vec4::new(0.0, 0.0, 0.0, 0.35)),
                ..Default::default()
            });
            // Ayarlar bir önceki oturumdan.
            let save = SaveData::open("gizmo_menu");
            let volume = save.get::<u32>("volume").unwrap_or(50).min(100);
            let mode = if save.get::<bool>("fullscreen").unwrap_or(false) { WindowMode::Fullscreen } else { WindowMode::Windowed };
            scene.world.insert_resource(mode);

            // İsim alanı: `Text`'in kendi içeriği yer tutucu; yazılınca değer gösteriliyor.
            let mut field = TextInput::new(16);
            field.value = save.get::<String>("name").unwrap_or_default();
            let name_field = scene.world.spawn_bundle(ButtonBundle {
                style: Style { height: Val::Px(52.0), ..Default::default() },
                background_color: BackgroundColor(fill(Interaction::None)),
                ..Default::default()
            });
            scene.world.add_component(
                name_field,
                Text::screen("İsminizi yazın…", font, 22.0, Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_color(Vec4::new(0.8, 0.82, 0.88, 1.0)),
            );
            scene.world.add_component(name_field, field);
            scene.world.add_component(name_field, gizmo::ui::ClipContent);
            scene.world.add_child(column, name_field);

            let mut first = None;
            for (action, label) in [
                (Action::Start, "Başla".to_string()),
                (Action::Fullscreen, fullscreen_label(mode)),
                (Action::Volume, volume_label(volume)),
                (Action::Quit, "Çıkış".to_string()),
            ] {
                let button = scene.world.spawn_bundle(ButtonBundle {
                    style: Style { height: Val::Px(52.0), ..Default::default() },
                    background_color: BackgroundColor(fill(Interaction::None)),
                    ..Default::default()
                });
                scene.world.add_component(button, Text::screen(label, font, 24.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
                scene.world.add_component(button, MenuButton(action));
                scene.world.add_child(column, button);
                first.get_or_insert(button.id());
            }

            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 32.0, Vec2::new(0.62, 0.85), Vec2::ZERO).with_anchor(TextAnchor::Center).with_align(TextAlign::Center),))
                .id();
            scene.world.spawn_bundle((Text::pinned("↑ ↓ / Tab · Enter   |   d-pad · A   |   fare", font, 18.0, TextAnchor::BottomRight, Vec2::new(-16.0, -16.0))
                .with_color(Vec4::new(0.7, 0.72, 0.78, 1.0)),));

            // Menü açık: klavye ve gamepad gezsin, ilk düğme odakta başlasın.
            if let Some(mut nav) = scene.world.get_resource_mut::<UiNavigation>() {
                nav.enabled = true;
                nav.focus(first);
            }

            if let Ok(mut audio) = AudioManager::new() {
                audio.load_sound_bytes("tik", Blip::new(Wave::Sine, 880.0, 0.05).with_volume(0.5).wav());
                audio.mixer_mut().set_master_gain(volume as f32 / 100.0);
                scene.world.insert_resource(audio);
            }
            scene.world.insert_resource(Menu { volume, banner });
        })
        .set_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn fullscreen_label(mode: WindowMode) -> String {
    format!("Tam ekran: {}", if mode == WindowMode::Fullscreen { "Açık" } else { "Kapalı" })
}

fn volume_label(v: u32) -> String {
    format!("Ses: %{v}")
}

// Esc bir metin alanını bırakıyor; çıkış Çıkış düğmesinde.
fn update(world: &mut World, _state: &mut SimpleSceneState, _dt: f32, _input: &Input) {
    let Some(mut menu) = world.remove_resource::<Menu>() else { return };

    // İsim alanı: Enter ile gönderildiyse kaydet ve selamla.
    let submitted: Option<String> = world
        .borrow::<TextInput>()
        .iter()
        .find(|(_, f)| f.is_submitted())
        .map(|(_, f)| f.value.clone());
    if let Some(name) = submitted {
        remember("name", &name);
        set_banner(world, menu.banner, &format!("Merhaba, {name}!"));
    }

    // Düğme renkleri: odak `Hovered` olarak geliyor, yani üzerine gelme rengi odak rengi de.
    let states: Vec<(u32, Interaction)> = world.borrow::<Interaction>().iter().map(|(id, i)| (id, *i)).collect();
    for (id, i) in states {
        if let Some(mut bg) = world.borrow_mut::<BackgroundColor>().get_mut(id) {
            bg.0 = fill(i);
        }
    }

    let clicked: Vec<(u32, Action)> = {
        let clicks = world.borrow::<Clicked>();
        let buttons = world.borrow::<MenuButton>();
        buttons.iter().filter(|(id, _)| clicks.get(*id).is_some_and(|c| c.0)).map(|(id, b)| (id, b.0)).collect()
    };
    for (id, action) in clicked {
        let label = match action {
            Action::Start => {
                set_banner(world, menu.banner, "Oyun burada başlardı.");
                None
            }
            Action::Fullscreen => {
                let next = match world.get_resource::<WindowMode>().map(|m| *m).unwrap_or_default() {
                    WindowMode::Windowed => WindowMode::Fullscreen,
                    _ => WindowMode::Windowed,
                };
                world.insert_resource(next);
                remember("fullscreen", &(next == WindowMode::Fullscreen));
                Some(fullscreen_label(next))
            }
            Action::Volume => {
                menu.volume = (menu.volume + 25) % 125;
                if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
                    audio.mixer_mut().set_master_gain(menu.volume as f32 / 100.0);
                }
                remember("volume", &menu.volume);
                Some(volume_label(menu.volume))
            }
            Action::Quit => {
                world.insert_resource(AppExit);
                None
            }
        };
        if let Some(label) = label {
            if let Some(mut t) = world.borrow_mut::<Text>().get_mut(id) {
                t.content = label;
            }
        }
        if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
            let _ = audio.play("tik");
        }
    }
    world.insert_resource(menu);
}

/// Bir ayarı kalıcı yapar — bir sonraki açılışta menü onunla açılıyor.
fn remember<T: serde::Serialize>(key: &str, value: &T) {
    let mut save = SaveData::open("gizmo_menu");
    if save.set(key, value).and_then(|()| save.save()).is_err() {
        gizmo::gizmo_log!(Warning, "ayar kaydedilemedi: {key}");
    }
}

fn set_banner(world: &mut World, id: u32, text: &str) {
    if let Some(mut t) = world.borrow_mut::<Text>().get_mut(id) {
        t.content = text.to_string();
    }
}

const FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    "C:/Windows/Fonts/arial.ttf",
];

fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    let env = std::env::var("GIZMO_FONT").ok();
    for path in env.iter().map(String::as_str).chain(FONT_CANDIDATES.iter().copied()) {
        if let Ok(id) = renderer.load_font_file(path) {
            return id;
        }
    }
    renderer
        .load_font(gizmo::renderer::text::synthetic::synthetic_face())
        .expect("sentetik yüz her zaman ayrıştırılır")
}
