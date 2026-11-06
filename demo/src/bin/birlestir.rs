//! # Birleştir — bir birleştirme bahçesi
//!
//! 6×5 bir tarla. **Üret** bir tohum ekiyor (enerji harcar, enerji kendiliğinden doluyor); iki
//! aynı şeyi üst üste sürükle, bir üstüne dönüşsünler: tohum → filiz → fidan → çalı → ağaç →
//! meyve ağacı → koru → orman. Sağdaki siparişler belli bir aşama istiyor; onu siparişin üstüne
//! bırak, altın kazan. Boş bir kareye bırakmak taşır, çöpe bırakmak siler. Tarla, altın ve enerji
//! kapanışta saklanıyor. Motorun on birinci oyunu; tamamen arayüz: sürükle-bırak, ızgara, ipucu.
//!
//! | girdi | iş |
//! |---|---|
//! | **sürükle** (fare ya da parmak) | taşı / birleştir / teslim et / çöpe at |
//! | **Üret** düğmesi ya da **Boşluk** | bir tohum |
//! | **R** | tarlayı sıfırla |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Bırakılan yer bulunamıyordu.** Sürüklenen şey imleci izliyor, yani imlecin altındaki o;
//!   altındaki kareler `Interaction::None` okuyor. Neyin üstüne bırakıldığını bulmak için her
//!   karenin kutusunu oyun kodunda tek tek sınamak, katmanı, kırpmayı ve sürüklenenin kendisini
//!   elle hesaba katmak gerekiyordu. **Kapatıldı:** `DropTarget` — bir öğeye ekle; sürükleme
//!   üstündeyken `hovered_by()`, bırakıldığı kare `dropped()` sürükleneni söylüyor, `Drag` da
//!   karşılığını (`over()`, `dropped_on()`). En üst katman kazanıyor, iç içe hedeflerde içteki,
//!   kırpılmış (kaydırılıp görünmeyen) hedef sayılmıyor, sürüklenen ve altındakiler hedef olamıyor.
//! - **Kareler içlerindeki şeyin üstüne çiziliyordu.** Arayüz kutuları aynı katmanda düğüm
//!   deposunun sırasıyla çiziliyordu, o da *arketip* sırası: kare (`NodeBundle` + `DropTarget`) ile
//!   içindeki öğe (`ButtonBundle` + `Drag`) ayrı tablolarda, ve önce gelen tablo önce çiziliyor —
//!   burada her kare kendi öğesini örtüyordu, yalnızca yazısı görünüyordu. **Kapatıldı:** bir
//!   katmanın içinde ağaç sırası — ebeveyn çocuklarının altında, önceki kardeş sonrakinin altında.
//! - **Arayüz pencereyle ölçeklenmiyordu.** 1280×720 için kurulmuş bir tarla, döşemeli bir
//!   pencere yöneticisinin verdiği 472×492'lik pencereye sığmıyordu; `Val::Px` her zaman pencere
//!   pikseliydi. **Kapatıldı:** `UiScale` kaynağı — `UiScale::fit(1280.0, 720.0)` düzeni her kare
//!   pencereye sığdırıyor (`fixed(k)` sabit bir çarpan); kutular, boşluklar ve kutuların üstündeki
//!   yazılar birlikte büyüyüp küçülüyor. `Node` pencere pikseli kalıyor, tıklama ve sürükleme
//!   hiçbir şey bilmeden çalışıyor.
//! - **Izgara düzeni yoktu.** Altı sütunlu tarla, genişliği altı kare ve beş boşluğun toplamına
//!   eşitlenmiş, *saran* bir satırdı — sütun sayısı bir genişlik hesabına gömülüydü ve bir kare
//!   boyu değişince tarla sessizce beş ya da yedi sütuna dönerdi. **Kapatıldı:** `Display::Grid`,
//!   `grid_template_columns: GridTracks::repeat(6, GridTrack::Px(84.0))`.
//! - **İpucu (tooltip) yoktu.** İlk sürüm kendisininkini kuruyordu: her kare imlecin yanına
//!   taşınan, kimse üstünde değilken `Display::None` olan mutlak bir düğüm — ve ölçekle bölmeyi
//!   unutmamak. **Kapatıldı:** `Tooltip::new(metin)` — üstünde yarım saniye durunca öğenin kendi
//!   fontuyla imlecin yanında çıkıyor, ayrılınca ya da tıklanınca kayboluyor, pencerenin içinde
//!   kalıyor; yalnızca klavye odağı onu açmıyor.
//!
//! Sürüklenen öğe ilk sürümde `Absolute`'a çevrilip `inset`'iyle taşınıyordu; `duello` için
//! eklenen `UiTransform` ile artık düzenden sonra kaydırılıyor, yanlış yere bırakılınca da
//! `UiTween` ile yerine kayarak dönüyor.
//!
//! ## Ortam değişkenleri
//!   * `BIRLESTIR_OTO=1` — kendi kendine birkaç tohum ekip birleştiriyor (ekran görüntüsü için;
//!     `birlestir --test` pencere açmadan oyunun kurallarını dener)
//!   * `GIZMO_FONT=<yol>` — font

use gizmo::app::save::SaveData;
use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, SimpleAppExt, SimpleSceneState};
use gizmo::ui::{
    AlignItems, BackgroundColor, ButtonBundle, Clicked, Display, Drag, DropTarget, FlexDirection, GridTrack, GridTracks, JustifyContent,
    NodeBundle, Style, Tooltip, UiPlugin, UiRect, UiScale, UiTransform, UiTween, Val, ZIndex,
};
use gizmo::math::Ease;

const COLS: usize = 6;
const ROWS: usize = 5;
const CELLS: usize = COLS * ROWS;
const CELL: f32 = 84.0;
const GAP: f32 = 6.0;
const PAD: f32 = 4.0;
const ORDERS: usize = 3;
const MAX_ENERGY: u32 = 10;
const ENERGY_EVERY: f32 = 3.0;

const NAMES: [&str; 8] = ["Tohum", "Filiz", "Fidan", "Çalı", "Ağaç", "Meyve", "Koru", "Orman"];
const COLOURS: [Vec4; 8] = [
    Vec4::new(0.62, 0.5, 0.32, 1.0),
    Vec4::new(0.55, 0.78, 0.35, 1.0),
    Vec4::new(0.35, 0.7, 0.3, 1.0),
    Vec4::new(0.25, 0.58, 0.32, 1.0),
    Vec4::new(0.18, 0.48, 0.25, 1.0),
    Vec4::new(0.85, 0.42, 0.3, 1.0),
    Vec4::new(0.3, 0.55, 0.55, 1.0),
    Vec4::new(0.95, 0.8, 0.3, 1.0),
];
const SLOT: Vec4 = Vec4::new(0.16, 0.13, 0.1, 1.0);
const SLOT_LIT: Vec4 = Vec4::new(0.32, 0.27, 0.18, 1.0);
const CARD: Vec4 = Vec4::new(0.14, 0.16, 0.21, 1.0);
const CARD_LIT: Vec4 = Vec4::new(0.25, 0.45, 0.3, 1.0);

/// Bir tarla karesi: kendisi bir bırakma hedefi, içindeki görünüm sürüklenen şey.
#[derive(Clone, Copy)]
struct Cell(usize);
gizmo::core::impl_component!(Cell);

/// Bir karenin içindeki görünüm — karenin `index`'i ne tutuyorsa onu çiziyor.
#[derive(Clone, Copy)]
struct View(usize);
gizmo::core::impl_component!(View);

#[derive(Clone, Copy)]
struct OrderCard(usize);
gizmo::core::impl_component!(OrderCard);

#[derive(Clone, Copy)]
struct Bin;
gizmo::core::impl_component!(Bin);

#[derive(Clone, Copy)]
struct Generate;
gizmo::core::impl_component!(Generate);

struct Game {
    /// Her karede ne var: `0` boş, `n` → `NAMES[n - 1]`.
    board: [u8; CELLS],
    /// Her siparişin istediği aşama.
    orders: [u8; ORDERS],
    coins: u32,
    energy: u32,
    refill: Timer,
    rng: Rng,
    cells: [Entity; CELLS],
    views: [Entity; CELLS],
    cards: [Entity; ORDERS],
    bin: Entity,
    status: Entity,
    /// Kendi kendine oynarken kaçıncı karede olduğu.
    auto: Option<u32>,
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
/// `sound` kapalıyken ses aygıtı açılmaz.
fn setup(world: &mut World, font: FontId, sound: bool) -> Game {
    world.spawn_bundle(CameraBundle { primary: true, ..Default::default() });
    // Tarla 1280×720 için kuruldu; pencere ne olursa olsun ona sığdırılıyor.
    world.insert_resource(UiScale::fit(1280.0, 720.0));

    let root = world.spawn_bundle(NodeBundle {
        style: Style {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Row,
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            column_gap: Val::Px(32.0),
            ..Default::default()
        },
        background_color: BackgroundColor(Vec4::new(0.08, 0.1, 0.07, 1.0)),
        ..Default::default()
    });

    // Tarla: altı sütunlu bir ızgara; satırlar kareler geldikçe.
    let field = world.spawn_bundle(NodeBundle {
        style: Style {
            display: Display::Grid,
            grid_template_columns: GridTracks::repeat(COLS, GridTrack::Px(CELL)),
            row_gap: Val::Px(GAP),
            column_gap: Val::Px(GAP),
            ..Default::default()
        },
        background_color: BackgroundColor(Vec4::ZERO),
        ..Default::default()
    });
    world.add_child(root, field);

    let mut cells = [Entity::new(0, 0); CELLS];
    let mut views = [Entity::new(0, 0); CELLS];
    for i in 0..CELLS {
        let cell = world.spawn_bundle(NodeBundle {
            style: Style {
                width: Val::Px(CELL),
                height: Val::Px(CELL),
                padding: UiRect::all(Val::Px(PAD)),
                ..Default::default()
            },
            background_color: BackgroundColor(SLOT),
            ..Default::default()
        });
        world.add_component(cell, Cell(i));
        world.add_component(cell, DropTarget::default());
        world.add_child(field, cell);
        let view = world.spawn_bundle(ButtonBundle {
            style: view_style(),
            background_color: BackgroundColor(Vec4::ZERO),
            ..Default::default()
        });
        world.add_component(view, View(i));
        world.add_component(view, Drag::default());
        // Ne olduğu ve birleşince ne olacağı — metni her kare karenin içeriğinden.
        world.add_component(view, Tooltip::new(""));
        world.add_component(
            view,
            Text::screen("", font, 15.0, Vec2::ZERO)
                .with_anchor(TextAnchor::Center)
                .with_color(Vec4::new(0.05, 0.06, 0.04, 1.0)),
        );
        world.add_child(cell, view);
        cells[i] = cell;
        views[i] = view;
    }

    // Sağ sütun: altın, siparişler, Üret, çöp.
    let side = world.spawn_bundle(NodeBundle {
        style: Style {
            width: Val::Px(260.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(12.0),
            align_items: Some(AlignItems::Stretch),
            ..Default::default()
        },
        background_color: BackgroundColor(Vec4::ZERO),
        ..Default::default()
    });
    world.add_child(root, side);
    let label = |world: &mut World, parent: Entity, h: f32, size: f32| {
        let n = world.spawn_bundle(NodeBundle {
            style: Style { height: Val::Px(h), ..Default::default() },
            background_color: BackgroundColor(Vec4::ZERO),
            ..Default::default()
        });
        world.add_component(n, Text::screen("", font, size, Vec2::ZERO).with_anchor(TextAnchor::Center));
        world.add_child(parent, n);
        n
    };
    let status = label(world, side, 64.0, 22.0);
    let mut cards = [Entity::new(0, 0); ORDERS];
    for (o, card_id) in cards.iter_mut().enumerate() {
        let card = world.spawn_bundle(NodeBundle {
            style: Style { height: Val::Px(70.0), ..Default::default() },
            background_color: BackgroundColor(CARD),
            ..Default::default()
        });
        world.add_component(card, OrderCard(o));
        world.add_component(card, DropTarget::default());
        world.add_component(card, Text::screen("", font, 19.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
        world.add_child(side, card);
        *card_id = card;
    }
    let generate = world.spawn_bundle(ButtonBundle {
        style: Style { height: Val::Px(56.0), ..Default::default() },
        background_color: BackgroundColor(CARD),
        ..Default::default()
    });
    world.add_component(generate, Generate);
    world.add_component(generate, Text::screen("Üret", font, 22.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
    world.add_child(side, generate);
    let bin = world.spawn_bundle(NodeBundle {
        style: Style { height: Val::Px(56.0), ..Default::default() },
        background_color: BackgroundColor(Vec4::new(0.25, 0.1, 0.1, 1.0)),
        ..Default::default()
    });
    world.add_component(bin, Bin);
    world.add_component(bin, DropTarget::default());
    world.add_component(bin, Text::screen("Çöp", font, 20.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
    world.add_child(side, bin);

    if let Some(mut audio) = sound.then(AudioManager::new).and_then(Result::ok) {
        audio.load_sound_bytes("tak", Blip::new(Wave::Triangle, 440.0, 0.05).with_volume(0.3).wav());
        audio.load_sound_bytes("birles", Blip::new(Wave::Sine, 660.0, 0.12).with_slide(990.0).with_volume(0.3).wav());
        audio.load_sound_bytes("altin", Blip::new(Wave::Square, 880.0, 0.1).with_slide(1320.0).with_volume(0.15).wav());
        audio.load_sound_bytes("yok", Blip::new(Wave::Square, 180.0, 0.08).with_volume(0.15).wav());
        world.insert_resource(audio);
    }

    let save = SaveData::open("birlestir");
    let mut board = [0u8; CELLS];
    if let Some(saved) = save.get::<Vec<u8>>("board").filter(|b| b.len() == CELLS) {
        board.copy_from_slice(&saved);
        board.iter_mut().for_each(|t| *t = (*t).min(NAMES.len() as u8));
    }
    let mut rng = Rng::new(0xB1_2025);
    let mut orders = [0; ORDERS];
    orders.iter_mut().for_each(|o| *o = new_order(&mut rng));
    let game = Game {
        board,
        orders,
        coins: save.get("coins").unwrap_or(0),
        energy: save.get::<u32>("energy").unwrap_or(MAX_ENERGY).min(MAX_ENERGY),
        refill: Timer::repeating(ENERGY_EVERY),
        rng,
        cells,
        views,
        cards,
        bin,
        status,
        auto: std::env::var_os("BIRLESTIR_OTO").is_some().then_some(0),
    };
    let _ = generate;
    game
}

/// `--test`: pencere açmadan kuralları dener. Üret bir tohum eker ve enerji harcar; iki aynı şey
/// bir üstüne birleşir, ayrı şeyler yer değiştirir, boş kareye bırakılan taşınır; istenen aşama
/// siparişe teslim edilince altın gelir; enerji zamanla dolar.
fn self_test() -> bool {
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font(), false);
    // Kayıttan ne geldiyse: boş bir tarla, ve oyuncunun kaydına yazmayan, ekran görüntüsü
    // hamlelerini (ilk karelerde) çoktan geçmiş bir otomatik sayaç.
    game.board = [0; CELLS];
    game.coins = 0;
    game.energy = MAX_ENERGY;
    game.auto = Some(1_000_000);
    let mut checks: Vec<(&str, bool)> = Vec::new();
    generate(&mut world, &mut game);
    generate(&mut world, &mut game);
    let seeds: Vec<usize> = (0..CELLS).filter(|&i| game.board[i] == 1).collect();
    checks.push(("iki tohum, iki enerji", seeds.len() == 2 && game.energy == MAX_ENERGY - 2));
    drop_on_cell(&mut world, &mut game, seeds[0], seeds[1]);
    checks.push(("tohum + tohum = filiz", game.board[seeds[1]] == 2 && game.board[seeds[0]] == 0));
    let empty = (0..CELLS).find(|&i| game.board[i] == 0).expect("boş kare");
    drop_on_cell(&mut world, &mut game, seeds[1], empty);
    checks.push(("boş kareye taşındı", game.board[empty] == 2 && game.board[seeds[1]] == 0));
    generate(&mut world, &mut game);
    let seed = (0..CELLS).find(|&i| game.board[i] == 1).expect("tohum");
    drop_on_cell(&mut world, &mut game, seed, empty);
    checks.push(("tohum ile filiz yer değiştirdi", game.board[empty] == 1 && game.board[seed] == 2));
    game.orders[0] = 2;
    let sprout = seed;
    deliver(&mut world, &mut game, sprout, 0);
    checks.push(("filiz siparişe teslim, 9 altın", game.coins == reward(2) && game.board[sprout] == 0));
    let before = game.energy;
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world).add_plugin(UiPlugin);
    while run.elapsed < ENERGY_EVERY * 2.5 {
        run.frame(1.0 / 60.0, update);
    }
    let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
    checks.push(("enerji zamanla doldu", game.energy == (before + 2).min(MAX_ENERGY)));
    let failed: Vec<&str> = checks.iter().filter(|(_, ok)| !ok).map(|(name, _)| *name).collect();
    println!(
        "birlestir --test: {} sınamadan {} geçti{} → {}",
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
    App::<SimpleSceneState>::new("Gizmo Engine - Birleştir", 1280, 720)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font, true);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn new_order(rng: &mut Rng) -> u8 {
    rng.range(2u32..=5) as u8
}

fn reward(tier: u8) -> u32 {
    3u32.pow(u32::from(tier))
}

/// Bir görünümün biçimi: karenin içini dolduruyor. Sürüklenirken yerinden oynamıyor — onu
/// `UiTransform` kaydırıyor.
fn view_style() -> Style {
    let side = Val::Px(CELL - 2.0 * PAD);
    Style { width: side, height: side, ..Default::default() }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };

    // Enerji kendiliğinden doluyor.
    if game.energy < MAX_ENERGY {
        let n = game.refill.tick(dt);
        game.energy = (game.energy + n).min(MAX_ENERGY);
    } else {
        game.refill.reset();
    }

    let clicked = world.borrow::<Clicked>();
    let generate_clicked = world.query::<(Entity, &Generate)>().unwrap().iter().any(|(entity, _)| clicked.get(entity).is_some_and(|c| c.0));
    drop(clicked);
    if generate_clicked || input.is_key_just_pressed(KeyCode::Space as u32) {
        generate(world, &mut game);
    }
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        game.board = [0; CELLS];
        game.coins = 0;
        game.energy = MAX_ENERGY;
        persist(&game);
    }
    if let Some(f) = game.auto.map(|f| f + 1) {
        game.auto = Some(f);
        // Ekran görüntüsü için, kare sayısıyla (hızlı bir makinede saniyeler çok kısa): önce
        // yedi tohum, sonra ikişer birleştirme — bir fidan ve bir filiz.
        if f == 20 {
            for _ in 0..7 {
                generate(world, &mut game);
            }
        }
        if f == 40 || f == 50 || f == 60 {
            let seeds: Vec<usize> = (0..CELLS).filter(|&i| game.board[i] == 1).take(2).collect();
            if let [a, b] = seeds[..] {
                drop_on_cell(world, &mut game, a, b);
            }
        }
        if f == 70 {
            let sprouts: Vec<usize> = (0..CELLS).filter(|&i| game.board[i] == 2).take(2).collect();
            if let [a, b] = sprouts[..] {
                drop_on_cell(world, &mut game, a, b);
            }
        }
    }

    // Sürükleme: görünüm imleci izliyor; bırakıldığı yer `Drag::dropped_on`.
    let mut moves: Vec<(usize, Option<Entity>)> = Vec::new();
    {
        let drags = world.borrow::<Drag>();
        for (entity, v) in world.query::<(Entity, &View)>().unwrap().iter() {
            let Some(d) = drags.get(entity) else { continue };
            if d.just_ended() {
                moves.push((v.0, d.dropped_on()));
            }
        }
    }
    for (from, target) in moves {
        let before = game.board;
        if let Some(target) = target {
            drop_into(world, &mut game, from, target);
        }
        // Bir yere gitti: görünüm boşaldı (ya da yeni içeriğiyle) yerinde. Gitmediyse yerine kayarak
        // dönüyor.
        let view = Some(game.views[from]);
        if let Some(e) = view {
            world.remove_component::<ZIndex>(e);
            if game.board[from] == before[from] && before[from] > 0 {
                world.add_component(e, UiTween::translation(Vec2::ZERO, 0.25).with_ease(Ease::OutBack));
            } else {
                world.add_component(e, UiTransform::default());
                world.remove_component::<UiTween>(e);
            }
        }
    }

    draw(world, &game);
    world.insert_resource(game);
}

fn generate(world: &mut World, game: &mut Game) {
    let empty: Vec<usize> = (0..CELLS).filter(|&i| game.board[i] == 0).collect();
    if game.energy == 0 || empty.is_empty() {
        play(world, "yok");
        return;
    }
    let at = empty[game.rng.range(0..empty.len() as u32) as usize];
    game.board[at] = 1;
    game.energy -= 1;
    play(world, "tak");
    persist(game);
}

/// `from` karesindeki şey `target`'a bırakıldı: bir kare, bir sipariş ya da çöp.
fn drop_into(world: &mut World, game: &mut Game, from: usize, target: Entity) {
    if let Some(c) = world.borrow::<Cell>().get(target).copied() {
        drop_on_cell(world, game, from, c.0);
    } else if let Some(o) = world.borrow::<OrderCard>().get(target).copied() {
        deliver(world, game, from, o.0);
    } else if target == game.bin && game.board[from] > 0 {
        game.board[from] = 0;
        play(world, "tak");
        persist(game);
    }
}

fn drop_on_cell(world: &mut World, game: &mut Game, from: usize, to: usize) {
    let (a, b) = (game.board[from], game.board[to]);
    if from == to || a == 0 {
        return;
    }
    if b == 0 {
        game.board[to] = a;
        game.board[from] = 0;
        play(world, "tak");
    } else if a == b && (a as usize) < NAMES.len() {
        game.board[to] = a + 1;
        game.board[from] = 0;
        play(world, "birles");
    } else {
        // Başka bir şeyin üstü: yer değiştirsinler.
        game.board.swap(from, to);
        play(world, "tak");
    }
    persist(game);
}

fn deliver(world: &mut World, game: &mut Game, from: usize, order: usize) {
    if game.board[from] == game.orders[order] {
        game.coins += reward(game.orders[order]);
        game.board[from] = 0;
        game.orders[order] = new_order(&mut game.rng);
        play(world, "altin");
        persist(game);
    } else {
        play(world, "yok");
    }
}

fn persist(game: &Game) {
    // Kendi kendine oynarken oyuncunun kaydına dokunma.
    if game.auto.is_some() {
        return;
    }
    let mut save = SaveData::open("birlestir");
    let ok = save.set("board", &game.board.to_vec()).is_ok()
        && save.set("coins", &game.coins).is_ok()
        && save.set("energy", &game.energy).is_ok()
        && save.save().is_ok();
    if !ok {
        gizmo::gizmo_log!(Warning, "tarla kaydedilemedi");
    }
}

fn play(world: &mut World, name: &str) {
    if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
        let _ = audio.play(name);
    }
}

fn set_text(world: &mut World, id: Entity, s: &str) {
    if let Some(mut t) = world.borrow_mut::<Text>().get_mut(id) {
        if t.content != s {
            t.content = s.to_string();
        }
    }
}

fn set_fill(world: &mut World, id: Entity, c: Vec4) {
    if let Some(mut b) = world.borrow_mut::<BackgroundColor>().get_mut(id) {
        if b.0 != c {
            b.0 = c;
        }
    }
}

fn draw(world: &mut World, game: &Game) {
    // Kareler ve içleri.
    for i in 0..CELLS {
        let (view, cell) = (game.views[i], game.cells[i]);
        let tier = game.board[i];
        // Sürüklenen şey imleci izliyor (düzen pikseliyle: ölçeğe böl), öbürlerinin önünde.
        let drag = world.borrow::<Drag>().get(view).copied().unwrap_or_default();
        if drag.is_dragging() && tier > 0 {
            let k = world.get_resource::<UiScale>().map_or(1.0, |s| s.factor());
            if let Some(e) = world.live(view) {
                if drag.just_started() {
                    world.add_component(e, ZIndex(50));
                    world.remove_component::<UiTween>(e);
                }
                world.add_component(e, UiTransform { translation: drag.delta() / k, opacity: 1.0 });
            }
        }
        let colour = if tier == 0 { Vec4::ZERO } else { COLOURS[tier as usize - 1] };
        set_fill(world, view, colour);
        set_text(world, view, if tier == 0 { "" } else { NAMES[tier as usize - 1] });
        let lit = world.borrow::<DropTarget>().get(cell).is_some_and(|t| t.hovered_by().is_some());
        set_fill(world, cell, if lit { SLOT_LIT } else { SLOT });
        let tip = match tier {
            0 => String::new(),
            t if (t as usize) < NAMES.len() => format!("{}\nİki tane → {}", NAMES[t as usize - 1], NAMES[t as usize]),
            t => format!("{}\nEn üst aşama", NAMES[t as usize - 1]),
        };
        if let Some(mut t) = world.borrow_mut::<Tooltip>().get_mut(view) {
            if t.text != tip {
                t.text = tip;
            }
        }
    }
    for (o, &card) in game.cards.iter().enumerate() {
        let want = game.orders[o];
        let s = format!("{} iste · {} altın", NAMES[want as usize - 1], reward(want));
        set_text(world, card, &s);
        let lit = world.borrow::<DropTarget>().get(card).is_some_and(|t| t.hovered_by().is_some());
        set_fill(world, card, if lit { CARD_LIT } else { CARD });
    }
    let refill = if game.energy < MAX_ENERGY { format!(" (+1 {:.0} sn)", game.refill.remaining().ceil()) } else { String::new() };
    let s = format!("Altın {}   ·   Enerji {}/{}{}", game.coins, game.energy, MAX_ENERGY, refill);
    set_text(world, game.status, &s);

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
