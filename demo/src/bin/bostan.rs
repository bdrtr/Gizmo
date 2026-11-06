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
//!   * `BOSTAN_OTO=1` — kendi kendine eker ve toplar (ekran görüntüsü için; `bostan --test` pencere
//!     açmadan bahçenin kurallarını sahte bir saatle dener)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::app::save::SaveData;
use gizmo::core::input::Input;
use gizmo::core::time::unix_time;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, SimpleAppExt, SimpleSceneState};
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
    plot_bars: Vec<Entity>,
    plot_labels: Vec<Entity>,
    seed_buttons: Vec<Entity>,
    status: Entity,
    note: Entity,
    note_time: f32,
    auto: bool,
    auto_clock: f32,
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
fn setup(world: &mut World, font: FontId) -> Game {
    world.spawn_bundle(CameraBundle { primary: true, ..Default::default() });
    world.insert_resource(UiScale::fit(1280.0, 720.0));
    let root = world.spawn_bundle(NodeBundle {
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
    let field = world.spawn_bundle(NodeBundle {
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
        world.add_child(root, field);
    }
    let mut plot_bars = Vec::new();
    let mut plot_labels = Vec::new();
    for i in 0..PLOTS {
        let plot = world.spawn_bundle(ButtonBundle {
            style: Style {
                flex_direction: FlexDirection::Column,
                justify_content: Some(JustifyContent::SpaceBetween),
                padding: UiRect::all(Val::Px(10.0)),
                ..Default::default()
            },
            background_color: BackgroundColor(hex(SOIL)),
            ..Default::default()
        });
        world.add_component(plot, PlotButton(i));
        world.add_component(plot, Text::screen("", font, 18.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
        let bar = world.spawn_bundle(NodeBundle {
            style: Style { width: Val::Percent(100.0), height: Val::Px(12.0), margin: UiRect::new(Val::ZERO, Val::ZERO, Val::Px(92.0), Val::ZERO), ..Default::default() },
            background_color: BackgroundColor(Vec4::new(0.0, 0.0, 0.0, 0.35)),
            ..Default::default()
        });
        world.add_component(bar, ProgressBar::new(0.0));
        {
            use gizmo::core::hierarchy::HierarchyExt;
            world.add_child(plot, bar);
            world.add_child(field, plot);
        }
        plot_bars.push(bar);
        plot_labels.push(plot);
    }
    let side = world.spawn_bundle(NodeBundle {
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
    let status = text_node(world, 60.0, 24.0);
    let mut seed_buttons = Vec::new();
    let mut children = vec![status];
    for (c, crop) in CROPS.iter().enumerate() {
        let b = world.spawn_bundle(ButtonBundle {
            style: Style { height: Val::Px(56.0), ..Default::default() },
            background_color: BackgroundColor(hex(SEED)),
            ..Default::default()
        });
        world.add_component(b, SeedButton(c));
        world.add_component(
            b,
            Text::screen(format!("{} — {} altın, {:.0} sn → {}", crop.0, crop.2, crop.1, crop.3), font, 17.0, Vec2::ZERO)
                .with_anchor(TextAnchor::Center),
        );
        seed_buttons.push(b);
        children.push(b);
    }
    let water = world.spawn_bundle(ButtonBundle {
        style: Style { height: Val::Px(56.0), ..Default::default() },
        background_color: BackgroundColor(hex("#2F4D73")),
        ..Default::default()
    });
    world.add_component(water, WaterButton);
    world.add_component(water, Text::screen(format!("Sulama — {WATER_PRICE} altın"), font, 17.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
    children.push(water);
    let note = text_node(world, 80.0, 17.0);
    children.push(note);
    {
        use gizmo::core::hierarchy::HierarchyExt;
        for c in children {
            world.add_child(side, c);
        }
        world.add_child(root, side);
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
        status,
        note,
        note_time: 0.0,
        auto,
        auto_clock: 0.0,
    };
    if ripened > 0 {
        set_text(world, game.note, &format!("Sen yokken {ripened} ürün olgunlaştı."));
        game.note_time = 5.0;
    }
    game
}

/// `--test`: pencere açmadan bahçenin kuralları, sahte bir saatle. Ekmek tohumun fiyatını alır;
/// olgunlaşmamış ürün toplanmaz, olgunu satış fiyatını verir; kaydedilip geri okunan bahçe oyun
/// kapalıyken geçen sürede büyümüş olur; geri alınmış bir saat ilerlemeyi sıfırın altına
/// indirmez; sulama büyümeyi dörtte üçe indirir.
fn self_test() -> bool {
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font());
    game.garden = Garden::default();
    game.seed = 0;
    let (_, grow, price, sell, _) = CROPS[0];
    let t0 = 1_000_000.0;
    let mut checks: Vec<(&str, bool)> = Vec::new();
    let coins = game.garden.coins;
    checks.push(("turp ekildi", tend(&mut game, 0, t0) && game.garden.coins == coins - price));
    checks.push(("yarı büyümüş toplanmadı", !tend(&mut game, 0, t0 + grow / 2.0)));
    let half = game.garden.progress(game.garden.plots[0], t0 + grow / 2.0);
    checks.push(("yarı yolda %50", (half - 0.5).abs() < 1e-4));
    checks.push(("olgun turp satıldı", tend(&mut game, 0, t0 + grow) && game.garden.coins == coins - price + sell));
    // Kapanış ve açılış: kaydedilen bahçe, aradaki sürede büyümüş.
    tend(&mut game, 1, t0);
    let mut save = SaveData::in_memory();
    let stored = save.set("garden", &game.garden).is_ok();
    let back: Option<Garden> = save.get("garden");
    let offline = back.as_ref().map(|g| g.progress(g.plots[1], t0 + grow * 3.0));
    checks.push(("kayıttan dönen bahçe kapalıyken büyümüş", stored && offline == Some(1.0)));
    let rewound = game.garden.progress(game.garden.plots[1], t0 - 500.0);
    checks.push(("geri alınan saat sıfırın altına inmedi", rewound == 0.0));
    game.garden.watered = true;
    checks.push(("sulama büyümeyi dörtte üçe indirdi", (game.garden.grow_time(0) - grow * 0.75).abs() < 1e-9));
    let failed: Vec<&str> = checks.iter().filter(|(_, ok)| !ok).map(|(name, _)| *name).collect();
    println!(
        "bostan --test: {} sınamadan {} geçti{} → {}",
        checks.len(),
        checks.len() - failed.len(),
        if failed.is_empty() { String::new() } else { format!("; geçmeyen: {}", failed.join(", ")) },
        if failed.is_empty() { "tamam" } else { "BAŞARISIZ" }
    );
    failed.is_empty()
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Bostan", 1280, 720)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let now = unix_time();
    let clicked: Vec<Entity> = world.query::<(Entity, &Clicked)>().iter().filter(|(_, c)| c.0).map(|(entity, _)| entity).collect();
    let mut changed = false;
    for id in clicked {
        if let Some(&SeedButton(c)) = world.query::<&SeedButton>().get(id) {
            game.seed = c;
        }
        if world.query::<&WaterButton>().get(id).is_some() && !game.garden.watered && game.garden.coins >= WATER_PRICE {
            game.garden.coins -= WATER_PRICE;
            game.garden.watered = true;
            changed = true;
        }
        if let Some(&PlotButton(i)) = world.query::<&PlotButton>().get(id) {
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
        if let Some(mut bg) = world.query_mut::<&mut BackgroundColor>().get_mut(game.plot_labels[i]) {
            if bg.0 != colour {
                bg.0 = colour;
            }
        }
        if let Some(mut bar) = world.query_mut::<&mut ProgressBar>().get_mut(game.plot_bars[i]) {
            if bar.value != p {
                bar.value = p;
            }
        }
    }
    for (c, &b) in game.seed_buttons.iter().enumerate() {
        let colour = hex(if c == game.seed { SEED_ON } else { SEED });
        if let Some(mut bg) = world.query_mut::<&mut BackgroundColor>().get_mut(b) {
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

fn set_text(world: &mut World, id: Entity, s: &str) {
    if let Some(mut t) = world.query_mut::<&mut Text>().get_mut(id) {
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
