//! # Bostan — oyun kapalıyken de büyüyen bir bahçe
//!
//! Tohum seç, boş tarlaya tıkla, büyümesini bekle, olgunlaşınca topla ve sat; parayla daha pahalı
//! tohumlar ve sulama al. Oyunu kapatsan da bahçe büyümeye devam ediyor: ekim anları gerçek saatle
//! kaydediliyor, geri dönünce aradaki süre sayılıyor. Motorun on sekizinci oyunu ve ilk **gerçek
//! saatle** işleyeni.
//!
//! | girdi | iş |
//! |---|---|
//! | **tohum düğmesi** | seç |
//! | **tarlaya tıkla** | ek / topla |
//! | **Sulama** | büyümeyi %25 hızlandır (bir kez) |
//! | **R** | bahçeyi sıfırla |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Gerçek saat yoktu.** `Time` her açılışta sıfırdan başlayan kare saati; oyun kapalıyken
//!   geçen süre için takvim saati gerekiyor. `std::time::SystemTime::now()` yerelde çalışıyor ama
//!   tarayıcıda (`wasm32-unknown-unknown`) panikliyor — oyun tarayıcıda açılışta çökerdi.
//!   **Kapatıldı:** `gizmo::core::time::unix_time()`, tarayıcıda tarayıcının saatinden.
//! - **Bir renk kodu yazdığımdan soluk çıkıyordu.** Motorun renk alanları (UI arka planı, malzeme)
//!   doğrusal okuyor; bir boya programından ya da CSS'ten alınan renk sRGB. `Color::hex` ve
//!   `rgb8` sayıları olduğu gibi aldığı için koyu yeşil zemin gri-yeşil göründü. **Kapatıldı:**
//!   `Color::srgb` / `srgb_u8` / `srgb_hex` (eskilerin belgesi de artık bunu söylüyor).
//!
//! Gerisi ilk denemede çalıştı: `SaveData`'ya bir `serde` yapısı (tarlalar bir enum vektörü),
//! `Display::Grid` ile tarla, `ProgressBar`, `Clicked`.
//!
//! ## Ortam değişkenleri
//!   * `BOSTAN_OTO=1` — kendi kendine eker ve toplar (ekran görüntüsü için)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::app::save::SaveData;
use gizmo::core::input::Input;
use gizmo::core::time::unix_time;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::ui::{
    AlignItems, BackgroundColor, ButtonBundle, Clicked, Display, FlexDirection, GridTrack, GridTracks, JustifyContent,
    NodeBundle, ProgressBar, Style, UiPlugin, UiRect, UiScale, Val,
};

const COLS: usize = 4;
const PLOTS: usize = 12;
const CELL: f32 = 130.0;

/// (ad, büyüme süresi sn, tohum fiyatı, satış fiyatı, renk — sRGB hex)
const CROPS: [(&str, f64, u32, u32, &str); 3] = [
    ("Turp", 10.0, 2, 5, "#C23B5A"),
    ("Domates", 40.0, 6, 18, "#D93B2B"),
    ("Kabak", 120.0, 15, 50, "#E8A23A"),
];
const SOIL: &str = "#5A3D24";
const SEED: &str = "#3F4D33";
const SEED_ON: &str = "#5E8A3A";

/// Bir renk kodu, sRGB olarak — motorun renk alanları doğrusal okur.
fn hex(code: &str) -> Vec4 {
    Color::srgb_hex(code).to_vec4()
}
const WATER_PRICE: u32 = 60;

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq)]
enum Plot {
    Empty,
    /// Ürün ve ekildiği an (Unix saniyesi).
    Growing { crop: usize, planted: f64 },
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Garden {
    plots: Vec<Plot>,
    coins: u32,
    watered: bool,
}

impl Default for Garden {
    fn default() -> Self {
        Self { plots: vec![Plot::Empty; PLOTS], coins: 10, watered: false }
    }
}

impl Garden {
    fn grow_time(&self, crop: usize) -> f64 {
        CROPS[crop].1 * if self.watered { 0.75 } else { 1.0 }
    }

    /// 0…1; gerçek saat geri alınmış olsa da 0'ın altına inmez.
    fn progress(&self, plot: Plot, now: f64) -> f32 {
        match plot {
            Plot::Empty => 0.0,
            Plot::Growing { crop, planted } => ((now - planted).max(0.0) / self.grow_time(crop)).min(1.0) as f32,
        }
    }
}

#[derive(Clone, Copy)]
struct PlotButton(usize);
gizmo::core::impl_component!(PlotButton);
#[derive(Clone, Copy)]
struct SeedButton(usize);
gizmo::core::impl_component!(SeedButton);
#[derive(Clone, Copy)]
struct WaterButton;
gizmo::core::impl_component!(WaterButton);

struct Game {
    garden: Garden,
    seed: usize,
    plot_bars: Vec<u32>,
    plot_labels: Vec<u32>,
    seed_buttons: Vec<u32>,
    status: u32,
    note: u32,
    note_time: f32,
    auto: bool,
    auto_clock: f32,
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bostan", 1280, 720)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.spawn_bundle(CameraBundle { primary: true, ..Default::default() });
            scene.world.insert_resource(UiScale::fit(1280.0, 720.0));
            let root = scene.world.spawn_bundle(NodeBundle {
                style: Style {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    justify_content: Some(JustifyContent::Center),
                    align_items: Some(AlignItems::Center),
                    column_gap: Val::Px(40.0),
                    ..Default::default()
                },
                background_color: BackgroundColor(hex("#24301C")),
                ..Default::default()
            });
            let field = scene.world.spawn_bundle(NodeBundle {
                style: Style {
                    display: Display::Grid,
                    grid_template_columns: GridTracks::repeat(COLS, GridTrack::Px(CELL)),
                    grid_auto_rows: GridTrack::Px(CELL),
                    row_gap: Val::Px(10.0),
                    column_gap: Val::Px(10.0),
                    ..Default::default()
                },
                background_color: BackgroundColor(Vec4::ZERO),
                ..Default::default()
            });
            {
                use gizmo::core::hierarchy::HierarchyExt;
                scene.world.add_child(root, field);
            }
            let mut plot_bars = Vec::new();
            let mut plot_labels = Vec::new();
            for i in 0..PLOTS {
                let plot = scene.world.spawn_bundle(ButtonBundle {
                    style: Style {
                        flex_direction: FlexDirection::Column,
                        justify_content: Some(JustifyContent::SpaceBetween),
                        padding: UiRect::all(Val::Px(10.0)),
                        ..Default::default()
                    },
                    background_color: BackgroundColor(hex(SOIL)),
                    ..Default::default()
                });
                scene.world.add_component(plot, PlotButton(i));
                scene.world.add_component(plot, Text::screen("", font, 18.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
                let bar = scene.world.spawn_bundle(NodeBundle {
                    style: Style { width: Val::Percent(100.0), height: Val::Px(12.0), margin: UiRect::new(Val::ZERO, Val::ZERO, Val::Px(92.0), Val::ZERO), ..Default::default() },
                    background_color: BackgroundColor(Vec4::new(0.0, 0.0, 0.0, 0.35)),
                    ..Default::default()
                });
                scene.world.add_component(bar, ProgressBar::new(0.0));
                {
                    use gizmo::core::hierarchy::HierarchyExt;
                    scene.world.add_child(plot, bar);
                    scene.world.add_child(field, plot);
                }
                plot_bars.push(bar.id());
                plot_labels.push(plot.id());
            }
            let side = scene.world.spawn_bundle(NodeBundle {
                style: Style { width: Val::Px(280.0), flex_direction: FlexDirection::Column, row_gap: Val::Px(12.0), ..Default::default() },
                background_color: BackgroundColor(Vec4::ZERO),
                ..Default::default()
            });
            let text_node = |world: &mut World, h: f32, size: f32| {
                let n = world.spawn_bundle(NodeBundle {
                    style: Style { height: Val::Px(h), ..Default::default() },
                    background_color: BackgroundColor(Vec4::ZERO),
                    ..Default::default()
                });
                world.add_component(n, Text::screen("", font, size, Vec2::ZERO).with_anchor(TextAnchor::Center));
                n
            };
            let status = text_node(scene.world, 60.0, 24.0);
            let mut seed_buttons = Vec::new();
            let mut children = vec![status];
            for (c, crop) in CROPS.iter().enumerate() {
                let b = scene.world.spawn_bundle(ButtonBundle {
                    style: Style { height: Val::Px(56.0), ..Default::default() },
                    background_color: BackgroundColor(hex(SEED)),
                    ..Default::default()
                });
                scene.world.add_component(b, SeedButton(c));
                scene.world.add_component(
                    b,
                    Text::screen(format!("{} — {} altın, {:.0} sn → {}", crop.0, crop.2, crop.1, crop.3), font, 17.0, Vec2::ZERO)
                        .with_anchor(TextAnchor::Center),
                );
                seed_buttons.push(b.id());
                children.push(b);
            }
            let water = scene.world.spawn_bundle(ButtonBundle {
                style: Style { height: Val::Px(56.0), ..Default::default() },
                background_color: BackgroundColor(hex("#2F4D73")),
                ..Default::default()
            });
            scene.world.add_component(water, WaterButton);
            scene.world.add_component(water, Text::screen(format!("Sulama — {WATER_PRICE} altın"), font, 17.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
            children.push(water);
            let note = text_node(scene.world, 80.0, 17.0);
            children.push(note);
            {
                use gizmo::core::hierarchy::HierarchyExt;
                for c in children {
                    scene.world.add_child(side, c);
                }
                scene.world.add_child(root, side);
            }

            // Kayıt: bahçe ve kapalıyken olgunlaşanlar.
            let garden: Garden = SaveData::open("bostan").get("garden").unwrap_or_default();
            let now = unix_time();
            let ripened = garden.plots.iter().filter(|&&p| p != Plot::Empty && garden.progress(p, now) >= 1.0).count();
            let auto = std::env::var_os("BOSTAN_OTO").is_some();
            let mut game = Game {
                garden,
                seed: 0,
                plot_bars,
                plot_labels,
                seed_buttons,
                status: status.id(),
                note: note.id(),
                note_time: 0.0,
                auto,
                auto_clock: 0.0,
            };
            if ripened > 0 {
                set_text(scene.world, game.note, &format!("Sen yokken {ripened} ürün olgunlaştı."));
                game.note_time = 5.0;
            }
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let now = unix_time();
    let clicked: Vec<u32> = world.borrow::<Clicked>().iter().filter(|(_, c)| c.0).map(|(id, _)| id).collect();
    let mut changed = false;
    for id in clicked {
        if let Some(&SeedButton(c)) = world.borrow::<SeedButton>().get(id) {
            game.seed = c;
        }
        if world.borrow::<WaterButton>().get(id).is_some() && !game.garden.watered && game.garden.coins >= WATER_PRICE {
            game.garden.coins -= WATER_PRICE;
            game.garden.watered = true;
            changed = true;
        }
        if let Some(&PlotButton(i)) = world.borrow::<PlotButton>().get(id) {
            changed |= tend(&mut game, i, now);
        }
    }
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        game.garden = Garden::default();
        changed = true;
    }
    if game.auto {
        game.auto_clock += dt;
        if game.auto_clock > 0.5 {
            game.auto_clock = 0.0;
            game.seed = if game.garden.coins >= CROPS[1].2 * 2 { 1 } else { 0 };
            for i in 0..PLOTS {
                changed |= tend(&mut game, i, now);
            }
        }
    }
    if changed {
        let mut save = SaveData::open("bostan");
        if save.set("garden", &game.garden).and_then(|()| save.save()).is_err() {
            gizmo::gizmo_log!(Warning, "bahçe kaydedilemedi");
        }
    }
    draw(world, &mut game, now, dt);
    world.insert_resource(game);
}

/// Bir tarlaya tıklamak: boşsa seçili tohumu ek, olgunsa topla ve sat. Bir şey değiştiyse `true`.
fn tend(game: &mut Game, i: usize, now: f64) -> bool {
    let plot = game.garden.plots[i];
    match plot {
        Plot::Empty => {
            let cost = CROPS[game.seed].2;
            if game.garden.coins < cost {
                return false;
            }
            game.garden.coins -= cost;
            game.garden.plots[i] = Plot::Growing { crop: game.seed, planted: now };
            true
        }
        Plot::Growing { crop, .. } if game.garden.progress(plot, now) >= 1.0 => {
            game.garden.coins += CROPS[crop].3;
            game.garden.plots[i] = Plot::Empty;
            true
        }
        Plot::Growing { .. } => false,
    }
}

fn draw(world: &mut World, game: &mut Game, now: f64, dt: f32) {
    for i in 0..PLOTS {
        let plot = game.garden.plots[i];
        let p = game.garden.progress(plot, now);
        let (label, colour) = match plot {
            Plot::Empty => ("boş".to_string(), hex(SOIL)),
            Plot::Growing { crop, planted } => {
                let (soil, ripe) = (hex(SOIL), hex(CROPS[crop].4));
                let left = (game.garden.grow_time(crop) - (now - planted).max(0.0)).max(0.0);
                let text = if p >= 1.0 { format!("{} — topla!", CROPS[crop].0) } else { format!("{}\n{:.0} sn", CROPS[crop].0, left.ceil()) };
                (text, soil.lerp(ripe, 0.35 + 0.65 * p))
            }
        };
        set_text(world, game.plot_labels[i], &label);
        if let Some(mut bg) = world.borrow_mut::<BackgroundColor>().get_mut(game.plot_labels[i]) {
            if bg.0 != colour {
                bg.0 = colour;
            }
        }
        if let Some(mut bar) = world.borrow_mut::<ProgressBar>().get_mut(game.plot_bars[i]) {
            if bar.value != p {
                bar.value = p;
            }
        }
    }
    for (c, &b) in game.seed_buttons.iter().enumerate() {
        let colour = hex(if c == game.seed { SEED_ON } else { SEED });
        if let Some(mut bg) = world.borrow_mut::<BackgroundColor>().get_mut(b) {
            bg.0 = colour;
        }
    }
    let water = if game.garden.watered { "  ·  sulandı" } else { "" };
    set_text(world, game.status, &format!("{} altın{water}\nTohum: {}", game.garden.coins, CROPS[game.seed].0));
    if game.note_time > 0.0 {
        game.note_time -= dt;
        if game.note_time <= 0.0 {
            set_text(world, game.note, "");
        }
    }
}

fn set_text(world: &mut World, id: u32, s: &str) {
    if let Some(mut t) = world.borrow_mut::<Text>().get_mut(id) {
        if t.content != s {
            t.content = s.to_string();
        }
    }
}

fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
