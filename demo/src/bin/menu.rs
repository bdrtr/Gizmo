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
//! - **Müzik geçişi yoktu, ve ses ayarı çalan müziğe ulaşmıyordu.** Oyunun eklediği
//!   `AudioManager`'ı her kare kimse ilerletmiyordu: biten her ses yöneticide kalıyordu, ses
//!   ayarı yalnızca *sonra* başlayan seslere uygulanıyordu, ve bir sesi zamanla kısmanın yolu
//!   yoktu. **Kapatıldı:** kare döngüsü `AudioManager::tick(dt)`'yi kendisi çağırıyor;
//!   `fade_to` / `fade_out` / `crossfade`. Menünün bir müziği var; Başla öbür parçaya 1.5 saniyede
//!   geçiyor, Ses düğmesi çalan müziği o an kısıyor.
//!
//! - **Tek dil vardı.** Her demo menüsünü kaynakta Türkçe yazıyor; ikinci bir dile çıkacak bir
//!   oyunun dizgelerini koyacak yeri, çalışırken dil değiştirmenin yolu yoktu. **Kapatıldı:**
//!   `Locale` kaynağı (JSON tabloları, iç içe anahtarlar noktayla, `{isim}` yer tutucuları, eksik
//!   dizge önce yedek dile sonra anahtarın kendisine düşüyor ve listeleniyor, sistemin dili) ve
//!   `LocalizedText` — bir `Text`'in yanında, çizildiği karede o anki dildeki dizgeyi gösteriyor.
//!   Menünün her etiketi bir anahtar; "Dil" düğmesi hepsini aynı karede çeviriyor, seçim
//!   kaydediliyor.
//!
//! - **Metin harf harf yazılamıyordu.** Diyalog ve duyurular için bir `Text`'i parça parça
//!   göstermenin yolu yoktu. **Kapatıldı:** `Typewriter` — bir `Text`'in yanında, çizilecek metnin
//!   (çevrilmiş olsa da) ilk N harfini gösteriyor, yeni bir metin gelince baştan başlıyor,
//!   `skip()`/`just_finished()`. Alttaki duyuru satırı onunla yazılıyor.
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
    Difficulty,
    Language,
    Quit,
}

/// Menünün dizgeleri, iki dilde. Gerçek bir oyunda bunlar dosyada durur (`Locale::load_json`
/// dosyanın metnini alıyor); demoda kaynakta.
const STRINGS_TR: &str = r#"{
  "menu": {
    "start": "Başla", "quit": "Çıkış", "name": "İsminizi yazın…",
    "fullscreen": "Tam ekran",
    "easy": "Zorluk: Kolay", "normal": "Zorluk: Orta", "hard": "Zorluk: Zor",
    "volume": "Ses: %{v}", "language": "Dil: Türkçe",
    "hello": "Merhaba, {name}!", "started": "Oyun burada başlardı — müzik değişti.",
    "back": "Menüye dönüldü.",
    "hint": "↑ ↓ / Tab · Enter · ← → ses   |   d-pad · A   |   fare"
  }
}"#;
const STRINGS_EN: &str = r#"{
  "menu": {
    "start": "Start", "quit": "Quit", "name": "Type your name…",
    "fullscreen": "Fullscreen",
    "easy": "Difficulty: Easy", "normal": "Difficulty: Normal", "hard": "Difficulty: Hard",
    "volume": "Volume: {v}%", "language": "Language: English",
    "hello": "Hello, {name}!", "started": "The game would start here — the music changed.",
    "back": "Back to the menu.",
    "hint": "↑ ↓ / Tab · Enter · ← → volume   |   d-pad · A   |   mouse"
  }
}"#;

#[derive(Clone, Copy)]
struct MenuButton(Action);
gizmo::core::impl_component!(MenuButton);

struct Menu {
    volume: u32,
    banner: u32,
    /// Ses kaydırıcısı (`Slider` taşıyan düğme).
    volume_button: u32,
    /// Zorluk listesi (`Dropdown` taşıyan düğme).
    difficulty: u32,
    /// Çalan müzik ve hangisi olduğu (menü mü, "oyun" mu).
    music: Option<u64>,
    in_game: bool,
}

/// İki saniyelik, kendine dikişsiz bağlanan yumuşak bir akor: her frekans iki saniyede tam sayıda
/// devir yapıyor, nefes alan genlik de bir devir. Döngüde tık yok.
fn pad(freqs: [f32; 3]) -> Vec<f32> {
    const RATE: f32 = 44_100.0;
    (0..(2.0 * RATE) as usize)
        .map(|i| {
            let t = i as f32 / RATE;
            let tone: f32 = freqs.iter().map(|f| (std::f32::consts::TAU * f * t).sin()).sum();
            tone / 3.0 * 0.18 * (0.75 + 0.25 * (std::f32::consts::TAU * 0.5 * t).sin())
        })
        .collect()
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
            // Dil: kayıtlıysa o, değilse sistemin dili, o da yoksa Türkçe.
            let mut locale = Locale::new("tr");
            locale.load_json("tr", STRINGS_TR).expect("the Turkish table is valid JSON");
            locale.load_json("en", STRINGS_EN).expect("the English table is valid JSON");
            let language = save
                .get::<String>("language")
                .or_else(Locale::system_language)
                .filter(|l| locale.languages().contains(&l.as_str()))
                .unwrap_or_else(|| "tr".to_string());
            locale.set_language(&language);
            scene.world.insert_resource(locale);

            // Başlık sütunun ilk öğesi: pencereye iğnelenmiş bir başlık, kısa bir pencerede
            // dikeyde ortalanan düğmelerin üstüne biniyordu.
            let title = scene.world.spawn_bundle(NodeBundle {
                style: Style { height: Val::Px(90.0), ..Default::default() },
                background_color: BackgroundColor(Vec4::ZERO),
                ..Default::default()
            });
            scene.world.add_component(title, Text::screen("GIZMO", font, 72.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
            scene.world.add_child(column, title);

            let mut field = TextInput::new(16);
            field.value = save.get::<String>("name").unwrap_or_default();
            let name_field = scene.world.spawn_bundle(ButtonBundle {
                style: Style { height: Val::Px(52.0), ..Default::default() },
                background_color: BackgroundColor(fill(Interaction::None)),
                ..Default::default()
            });
            scene.world.add_component(
                name_field,
                Text::screen("", font, 22.0, Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_color(Vec4::new(0.8, 0.82, 0.88, 1.0)),
            );
            scene.world.add_component(name_field, LocalizedText::new("menu.name"));
            scene.world.add_component(name_field, field);
            scene.world.add_component(name_field, gizmo::ui::ClipContent);
            scene.world.add_child(column, name_field);

            let mut first = None;
            let mut slider = None;
            let mut difficulty = None;
            // Etiketler anahtar: ekranda `Locale`'in o anki dilindeki dizge çiziliyor.
            for (action, label) in [
                (Action::Start, LocalizedText::new("menu.start")),
                (Action::Fullscreen, LocalizedText::new("menu.fullscreen")),
                (Action::Volume, volume_label(volume)),
                (Action::Difficulty, LocalizedText::new("menu.normal")),
                (Action::Language, LocalizedText::new("menu.language")),
                (Action::Quit, LocalizedText::new("menu.quit")),
            ] {
                let button = scene.world.spawn_bundle(ButtonBundle {
                    style: Style { height: Val::Px(52.0), ..Default::default() },
                    background_color: BackgroundColor(fill(Interaction::None)),
                    ..Default::default()
                });
                scene.world.add_component(button, MenuButton(action));
                scene.world.add_component(button, Text::screen("", font, 24.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
                scene.world.add_component(button, label);
                scene.world.add_child(column, button);
                if action == Action::Fullscreen {
                    // Açık mı kapalı mı, solundaki onay kutusu söylüyor (`Toggle`); her tık onu da
                    // çeviriyor, aşağıdaki işleyici pencereyi.
                    scene.world.add_component(button, gizmo::ui::Toggle::new(mode == WindowMode::Fullscreen));
                }
                if action == Action::Difficulty {
                    // Bir açılır liste: tıklayınca altında seçenekler açılıyor; odaktayken ←/→
                    // seçenekler arasında dönüyor. Seçenekler dilin dizgeleri — dil değişince
                    // yenileniyor (`difficulty_options`). Etiketi seçili seçenek, `LocalizedText`
                    // değil.
                    scene.world.remove_component::<LocalizedText>(button);
                    let chosen = save.get::<usize>("difficulty").unwrap_or(1).min(2);
                    let options = difficulty_options(scene.world.get_resource::<Locale>().as_deref());
                    let mut dropdown = gizmo::ui::Dropdown::new(options, chosen);
                    if std::env::var_os("MENU_LISTE_ACIK").is_some() {
                        dropdown.set_open(true); // ekran görüntüsü için
                    }
                    scene.world.add_component(button, dropdown);
                    difficulty = Some(button.id());
                }
                if action == Action::Volume {
                    // Sürüklenen ya da odaktayken ←/→ ile adım adım değişen bir kaydırıcı; dolgusunu
                    // motor çiziyor, etiketin altında.
                    scene.world.add_component(button, gizmo::ui::Slider::new(volume as f32 / 100.0));
                    slider = Some(button.id());
                }
                first.get_or_insert(button.id());
            }

            let banner = scene
                .world
                .spawn_bundle((
                    Text::pinned_at("", font, 32.0, Vec2::new(0.62, 0.85), Vec2::ZERO).with_anchor(TextAnchor::Center).with_align(TextAlign::Center),
                    // Her yeni duyuru harf harf yazılıyor.
                    Typewriter::new(40.0),
                ))
                .id();
            scene.world.spawn_bundle((
                Text::pinned("", font, 18.0, TextAnchor::BottomRight, Vec2::new(-16.0, -16.0))
                    .with_color(Vec4::new(0.7, 0.72, 0.78, 1.0)),
                LocalizedText::new("menu.hint"),
            ));

            // Menü açık: klavye ve gamepad gezsin, ilk düğme odakta başlasın.
            if let Some(mut nav) = scene.world.get_resource_mut::<UiNavigation>() {
                nav.enabled = true;
                nav.focus(first);
            }

            let mut music = None;
            if let Ok(mut audio) = AudioManager::new() {
                audio.load_sound_bytes("tik", Blip::new(Wave::Sine, 880.0, 0.05).with_volume(0.5).wav());
                audio.mixer_mut().set_master_gain(volume as f32 / 100.0);
                audio.load_samples("menu", &pad([220.0, 261.5, 330.0]), 44_100);
                audio.load_samples("oyun", &pad([196.0, 247.0, 293.5]), 44_100);
                music = audio.crossfade(None, "menu", 2.0).ok();
                scene.world.insert_resource(audio);
            }
            let volume_button = slider.expect("the volume button was built");
            let difficulty = difficulty.expect("the difficulty list was built");
            scene.world.insert_resource(Menu { volume, banner, volume_button, difficulty, music, in_game: false });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn volume_label(v: u32) -> LocalizedText {
    LocalizedText::new("menu.volume").with_arg("v", v)
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
        set_banner(world, menu.banner, LocalizedText::new("menu.hello").with_arg("name", name));
    }

    // Düğme renkleri: odak `Hovered` olarak geliyor, yani üzerine gelme rengi odak rengi de.
    let states: Vec<(u32, Interaction)> = world.borrow::<Interaction>().iter().map(|(id, i)| (id, *i)).collect();
    for (id, i) in states {
        if let Some(mut bg) = world.borrow_mut::<BackgroundColor>().get_mut(id) {
            bg.0 = fill(i);
        }
    }

    // Ses kaydırıcısı: değiştikçe uygula; bir sürükleme bitince ya da klavyeyle bir adımda kaydet.
    if let Some(slider) = world.borrow::<gizmo::ui::Slider>().get(menu.volume_button).copied() {
        if slider.just_changed() {
            set_volume(world, &mut menu, (slider.value * 100.0).round() as u32);
        }
        if slider.just_released() || (slider.just_changed() && !slider.is_dragging()) {
            remember("volume", &menu.volume);
        }
    }

    // Zorluk: seçim değiştiyse kaydet.
    let picked = world
        .borrow::<gizmo::ui::Dropdown>()
        .get(menu.difficulty)
        .filter(|d| d.just_changed())
        .map(|d| d.selected);
    if let Some(i) = picked {
        remember("difficulty", &i);
    }

    let clicked: Vec<(u32, Action)> = {
        let clicks = world.borrow::<Clicked>();
        let buttons = world.borrow::<MenuButton>();
        buttons.iter().filter(|(id, _)| clicks.get(*id).is_some_and(|c| c.0)).map(|(id, b)| (id, b.0)).collect()
    };
    for (id, action) in clicked {
        let label = match action {
            Action::Start => {
                // Öbür parçaya geç: yenisi sessizden açılıyor, eskisi kısılıp duruyor.
                menu.in_game = !menu.in_game;
                let (track, text) = if menu.in_game { ("oyun", "menu.started") } else { ("menu", "menu.back") };
                if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
                    menu.music = audio.crossfade(menu.music, track, 1.5).ok().or(menu.music);
                }
                set_banner(world, menu.banner, LocalizedText::new(text));
                None
            }
            Action::Fullscreen => {
                let next = match world.get_resource::<WindowMode>().map(|m| *m).unwrap_or_default() {
                    WindowMode::Windowed => WindowMode::Fullscreen,
                    _ => WindowMode::Windowed,
                };
                world.insert_resource(next);
                remember("fullscreen", &(next == WindowMode::Fullscreen));
                None
            }
            // Kaydırıcının ve listenin işi: sürükleme, açılan liste, ya da odaktayken ←/→.
            Action::Volume | Action::Difficulty => None,
            // Dili çevir: her `LocalizedText` bir sonraki karede yeni dilde.
            Action::Language => {
                let options = world.get_resource_mut::<Locale>().map(|mut locale| {
                    let next = if locale.language() == "tr" { "en" } else { "tr" };
                    locale.set_language(next);
                    remember("language", &next);
                    difficulty_options(Some(&locale))
                });
                // Listenin seçenekleri de yeni dilde.
                if let Some(options) = options {
                    if let Some(mut d) = world.borrow_mut::<gizmo::ui::Dropdown>().get_mut(menu.difficulty) {
                        d.options = options;
                    }
                }
                None
            }
            Action::Quit => {
                world.insert_resource(AppExit);
                None
            }
        };
        if let Some(label) = label {
            if let Some(mut t) = world.borrow_mut::<LocalizedText>().get_mut(id) {
                *t = label;
            }
        }
        if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
            let _ = audio.play("tik");
        }
    }
    world.insert_resource(menu);
}

/// Zorluğun seçenekleri, o anki dilde.
fn difficulty_options(locale: Option<&Locale>) -> Vec<String> {
    ["menu.easy", "menu.normal", "menu.hard"]
        .iter()
        .map(|k| locale.map_or_else(|| (*k).to_string(), |l| l.get(k).to_string()))
        .collect()
}

/// Sesi uygular: ana kazanç ve etiket.
fn set_volume(world: &mut World, menu: &mut Menu, v: u32) {
    menu.volume = v.min(100);
    if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
        audio.mixer_mut().set_master_gain(menu.volume as f32 / 100.0);
    }
    if let Some(mut t) = world.borrow_mut::<LocalizedText>().get_mut(menu.volume_button) {
        *t = volume_label(menu.volume);
    }
}

/// Bir ayarı kalıcı yapar — bir sonraki açılışta menü onunla açılıyor.
fn remember<T: serde::Serialize>(key: &str, value: &T) {
    let mut save = SaveData::open("gizmo_menu");
    if save.set(key, value).and_then(|()| save.save()).is_err() {
        gizmo::gizmo_log!(Warning, "ayar kaydedilemedi: {key}");
    }
}

/// Alttaki duyuruyu değiştirir — bir anahtar, dil değişince o da değişsin diye.
fn set_banner(world: &mut World, id: u32, text: LocalizedText) {
    if let Some(e) = world.get_entity(id) {
        world.add_component(e, text);
    }
}


fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    // A font the machine has (`GIZMO_FONT`, or the platform's usual faces); without one, the
    // synthetic test face, so the game still runs.
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
