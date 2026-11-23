//! # Sabır — Klondike
//!
//! Elli iki kart: yedi sütun, bir deste, dört temel. Sütunlarda kartlar azalan sırayla ve renk
//! değiştirerek dizilir (siyah 9'un üstüne kırmızı 8); temellerde her tür Astan Papaza. Desteye
//! tıkla, bir kart açılsın; deste bitince açılanlar geri döner. Bir kartı (ve üstündekileri)
//! sürükle; çift tıkla, gidebiliyorsa temeline gitsin. Hepsi açıldığında kalanı kendisi bitiriyor.
//! Motorun ilk kart oyunu.
//!
//! | girdi | iş |
//! |---|---|
//! | **sürükle** (fare ya da parmak) | kartı / diziyi taşı |
//! | **tıkla** destede | bir kart aç |
//! | **çift tıkla** bir kartta | temeline gönder |
//! | **U** ya da **Geri** | son hamleyi geri al |
//! | **N** ya da **Yeni** | yeni el |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Çift tıklama yoktu.** `Clicked` bir tıklamanın tamamlandığı kareyi söylüyor; ikinci
//!   tıklamanın birinciye yetecek kadar yakın olduğunu bilmek için oyun zamanı ve hangi öğe
//!   olduğunu kendisi tutacaktı. **Kapatıldı:** `DoubleClicked` — `ButtonBundle`'ın parçası,
//!   aynı öğede yarım saniyeden kısa arayla tamamlanan ikinci tıklamanın karesinde `true`.
//!
//! ## Ortam değişkenleri
//!   * `SABIR_OTO=1` — kendi kendine birkaç hamle yapıyor (ekran görüntüsü için; `sabir --test`
//!     pencere açmadan kuralları, sürüklemeyi ve çift tıklamayı dener)
//!   * `GIZMO_FONT=<yol>` — font

use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::math::Ease;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, SimpleAppExt, SimpleSceneState};
use gizmo::ui::{
    BackgroundColor, ButtonBundle, Clicked, DoubleClicked, Drag, DropTarget, NodeBundle, PositionType, Style, UiPlugin, UiRect,
    UiScale, UiTransform, UiTween, Val, ZIndex,
};
use gizmo::winit::keyboard::KeyCode;

const W: f32 = 1280.0;
const H: f32 = 720.0;
const CARD_W: f32 = 96.0;
const CARD_H: f32 = 134.0;
const GAP: f32 = 16.0;
const LEFT: f32 = (W - 7.0 * CARD_W - 6.0 * GAP) * 0.5;
const TOP: f32 = 24.0;
const ROW: f32 = TOP + CARD_H + 28.0;
/// Bir sütunda kapalı ve açık kartların arası.
const DOWN_STEP: f32 = 14.0;
const UP_STEP: f32 = 26.0;
/// Bir sürüklemenin tıklama sayılmayacağı yol, düzen pikseli.
const SLOP: f32 = 6.0;
/// Otomatik bitirişte kartlar arası.
const AUTO_EVERY: f32 = 0.08;

const SUITS: [&str; 4] = ["♠", "♥", "♦", "♣"];
const RANKS: [&str; 13] = ["A", "2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K"];

const TABLE: Vec4 = Vec4::new(0.07, 0.3, 0.16, 1.0);
const SLOT: Vec4 = Vec4::new(0.05, 0.22, 0.12, 1.0);
const SLOT_LIT: Vec4 = Vec4::new(0.16, 0.42, 0.24, 1.0);
const FACE: Vec4 = Vec4::new(0.97, 0.96, 0.92, 1.0);
const BACK: Vec4 = Vec4::new(0.2, 0.3, 0.62, 1.0);
const RED: Vec4 = Vec4::new(0.78, 0.1, 0.12, 1.0);
const BLACK: Vec4 = Vec4::new(0.08, 0.08, 0.1, 1.0);

/// Bir kart: `0..52`, `kart / 13` türü (♠ ♥ ♦ ♣), `kart % 13 + 1` değeri.
type Card = u8;

fn suit(c: Card) -> usize {
    usize::from(c / 13)
}
fn rank(c: Card) -> u8 {
    c % 13 + 1
}
fn red(c: Card) -> bool {
    matches!(suit(c), 1 | 2)
}
fn name(c: Card) -> String {
    format!("{}{}", RANKS[usize::from(rank(c) - 1)], SUITS[suit(c)])
}

/// Bir kartın durduğu yer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pile {
    Stock,
    Waste,
    Foundation(usize),
    Tableau(usize),
}

/// Masanın tamamı: geri almak için bir kopyası saklanıyor.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Board {
    stock: Vec<Card>,
    waste: Vec<Card>,
    foundations: [Vec<Card>; 4],
    /// Her sütun alttan üste; `true` açık.
    tableau: [Vec<(Card, bool)>; 7],
}

impl Board {
    fn deal(rng: &mut Rng) -> Self {
        let mut deck: Vec<Card> = (0..52).collect();
        rng.shuffle(&mut deck);
        let mut tableau: [Vec<(Card, bool)>; 7] = Default::default();
        for (col, pile) in tableau.iter_mut().enumerate() {
            for row in 0..=col {
                pile.push((deck.pop().expect("elli iki kart"), row == col));
            }
        }
        Self { stock: deck, waste: Vec::new(), foundations: Default::default(), tableau }
    }

    fn won(&self) -> bool {
        self.foundations.iter().all(|f| f.len() == 13)
    }

    /// Her kart bir kez: elli iki kart, hiçbiri iki yerde.
    fn whole(&self) -> bool {
        let mut seen = [false; 52];
        let mut count = 0;
        let all = self
            .stock
            .iter()
            .chain(&self.waste)
            .chain(self.foundations.iter().flatten())
            .copied()
            .chain(self.tableau.iter().flatten().map(|(c, _)| *c));
        for c in all {
            if c >= 52 || seen[usize::from(c)] {
                return false;
            }
            seen[usize::from(c)] = true;
            count += 1;
        }
        count == 52
    }

    /// `card`'ın yeri, yığındaki sırası ve açık mı.
    fn find(&self, card: Card) -> (Pile, usize, bool) {
        if let Some(i) = self.stock.iter().position(|&c| c == card) {
            return (Pile::Stock, i, false);
        }
        if let Some(i) = self.waste.iter().position(|&c| c == card) {
            return (Pile::Waste, i, true);
        }
        for (f, pile) in self.foundations.iter().enumerate() {
            if let Some(i) = pile.iter().position(|&c| c == card) {
                return (Pile::Foundation(f), i, true);
            }
        }
        for (t, pile) in self.tableau.iter().enumerate() {
            if let Some(i) = pile.iter().position(|&(c, _)| c == card) {
                return (Pile::Tableau(t), i, pile[i].1);
            }
        }
        unreachable!("her kart bir yerde")
    }

    /// `card`'dan başlayıp birlikte taşınacak kartlar, ya da taşınamıyorsa boş: bir sütunda açık bir
    /// kart ve üstündekiler, açılmış destenin ve bir temelin en üstü.
    fn run_from(&self, card: Card) -> Vec<Card> {
        match self.find(card) {
            (Pile::Tableau(t), i, true) => self.tableau[t][i..].iter().map(|(c, _)| *c).collect(),
            (Pile::Waste, i, _) if i + 1 == self.waste.len() => vec![card],
            (Pile::Foundation(f), i, _) if i + 1 == self.foundations[f].len() => vec![card],
            _ => Vec::new(),
        }
    }

    /// `run` (`run_from`'un verdiği) `to`'ya konabilir mi.
    fn accepts(&self, to: Pile, run: &[Card]) -> bool {
        let Some(&first) = run.first() else { return false };
        match to {
            Pile::Foundation(f) => {
                run.len() == 1
                    && match self.foundations[f].last() {
                        None => rank(first) == 1,
                        Some(&top) => suit(top) == suit(first) && rank(first) == rank(top) + 1,
                    }
            }
            Pile::Tableau(t) => match self.tableau[t].last() {
                None => rank(first) == 13,
                Some(&(top, up)) => up && red(top) != red(first) && rank(top) == rank(first) + 1,
            },
            Pile::Stock | Pile::Waste => false,
        }
    }

    /// `card`'dan başlayan diziyi `to`'ya taşır; olmuyorsa hiçbir şey yapmaz ve `false` döner. Bir
    /// sütunun yeni en üstü kapalıysa açılır.
    fn play(&mut self, card: Card, to: Pile) -> bool {
        let run = self.run_from(card);
        let (from, at, _) = self.find(card);
        if run.is_empty() || from == to || !self.accepts(to, &run) {
            return false;
        }
        match from {
            Pile::Tableau(t) => {
                self.tableau[t].truncate(at);
                if let Some(top) = self.tableau[t].last_mut() {
                    top.1 = true;
                }
            }
            Pile::Waste => drop(self.waste.pop()),
            Pile::Foundation(f) => drop(self.foundations[f].pop()),
            Pile::Stock => return false,
        }
        match to {
            Pile::Foundation(f) => self.foundations[f].extend(&run),
            Pile::Tableau(t) => self.tableau[t].extend(run.iter().map(|&c| (c, true))),
            Pile::Stock | Pile::Waste => unreachable!("kabul edilmedi"),
        }
        true
    }

    /// Desteden bir kart açar; deste bittiyse açılanları ters çevirip desteye geri koyar. Bir şey
    /// değiştiyse `true`.
    fn draw(&mut self) -> bool {
        if let Some(c) = self.stock.pop() {
            self.waste.push(c);
            true
        } else if !self.waste.is_empty() {
            self.stock = self.waste.drain(..).rev().collect();
            true
        } else {
            false
        }
    }

    /// `card` tek başına bir temele gidebiliyorsa o temel.
    fn foundation_for(&self, card: Card) -> Option<Pile> {
        let run = self.run_from(card);
        (run.len() == 1).then_some(())?;
        (0..4).map(Pile::Foundation).find(|&p| self.accepts(p, &run))
    }

    /// Deste ve açılanlar bitti, sütunlarda kapalı kart yok: gerisi kendiliğinden gider.
    fn settled(&self) -> bool {
        self.stock.is_empty() && self.waste.is_empty() && self.tableau.iter().flatten().all(|(_, up)| *up)
    }

    /// Temele gidebilen en küçük açık kart, otomatik bitiriş için.
    fn next_home(&self) -> Option<Card> {
        let tops = self.tableau.iter().filter_map(|t| t.last().map(|(c, _)| *c)).chain(self.waste.last().copied());
        tops.filter(|&c| self.foundation_for(c).is_some()).min_by_key(|&c| rank(c))
    }
}

/// Bir kartın görünümü.
#[derive(Clone, Copy)]
struct CardView(Card);
gizmo::core::impl_component!(CardView);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    Undo,
    New,
}

#[derive(Clone, Copy)]
struct Button(Command);
gizmo::core::impl_component!(Button);

struct Game {
    board: Board,
    history: Vec<Board>,
    views: [Entity; 52],
    slots: Vec<(Entity, Pile)>,
    stock_slot: Entity,
    status: Entity,
    banner: Entity,
    rng: Rng,
    moves: u32,
    /// Sürüklenen kart ve birlikte gidenler.
    dragging: Option<Vec<Card>>,
    /// Kartların son çizildiği yer, düzen pikseli: bir hamlede yeni yerine buradan kayıyor.
    shown: [Vec2; 52],
    auto_timer: f32,
    /// Kendi kendine oynarken kaçıncı karede olduğu.
    auto: Option<u32>,
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
/// `sound` kapalıyken ses aygıtı açılmaz.
fn setup(world: &mut World, font: FontId, sound: bool, seed: u64) -> Game {
    world.spawn_bundle(CameraBundle { primary: true, ..Default::default() });
    world.insert_resource(UiScale::fit(W, H));

    let table = world.spawn_bundle(NodeBundle {
        style: Style { width: Val::Px(W), height: Val::Px(H), ..Default::default() },
        background_color: BackgroundColor(TABLE),
        ..Default::default()
    });
    let place = |left: f32, top: f32, width: f32, height: f32| Style {
        position_type: PositionType::Absolute,
        inset: UiRect { left: Val::Px(left), top: Val::Px(top), ..UiRect::default() },
        width: Val::Px(width),
        height: Val::Px(height),
        ..Default::default()
    };

    // Yerler: deste, dört temel, yedi sütun. Sütunun bırakma alanı masanın dibine kadar.
    let mut slots = Vec::new();
    let stock_slot = world.spawn_bundle(ButtonBundle {
        style: place(LEFT, TOP, CARD_W, CARD_H),
        background_color: BackgroundColor(SLOT),
        ..Default::default()
    });
    world.add_child(table, stock_slot);
    for f in 0..4 {
        let e = world.spawn_bundle(NodeBundle {
            style: place(column_x(3 + f), TOP, CARD_W, CARD_H),
            background_color: BackgroundColor(SLOT),
            ..Default::default()
        });
        world.add_component(e, DropTarget::default());
        // Her tür boş bir temele gidebilir; temel ilk gelen Asın türünün oluyor.
        world.add_component(e, Text::screen("A", font, 40.0, Vec2::ZERO).with_anchor(TextAnchor::Center).with_color(SLOT_LIT));
        world.add_child(table, e);
        slots.push((e, Pile::Foundation(f)));
    }
    for t in 0..7 {
        let e = world.spawn_bundle(NodeBundle {
            style: place(column_x(t), ROW, CARD_W, H - ROW),
            background_color: BackgroundColor(Vec4::ZERO),
            ..Default::default()
        });
        world.add_component(e, DropTarget::default());
        world.add_child(table, e);
        // Boş bir sütunun yeri görünsün.
        let mark = world.spawn_bundle(NodeBundle {
            style: Style { width: Val::Px(CARD_W), height: Val::Px(CARD_H), ..Default::default() },
            background_color: BackgroundColor(SLOT),
            ..Default::default()
        });
        world.add_child(e, mark);
        slots.push((e, Pile::Tableau(t)));
    }

    // Kartlar: hepsi masanın çocukları, yerleri ve katmanları her kare tahtadan.
    let mut views = [Entity::new(0, 0); 52];
    for (c, view) in views.iter_mut().enumerate() {
        // Etiket kartın sol üstünde, dolgunun içinde: üst üste binen kartlarda görünen şerit.
        let e = world.spawn_bundle(ButtonBundle {
            style: Style { padding: UiRect { left: Val::Px(7.0), top: Val::Px(2.0), ..UiRect::default() }, ..place(LEFT, TOP, CARD_W, CARD_H) },
            background_color: BackgroundColor(BACK),
            ..Default::default()
        });
        world.add_component(e, CardView(c as Card));
        world.add_component(e, Drag::default());
        world.add_component(e, Text::screen("", font, 22.0, Vec2::ZERO).with_anchor(TextAnchor::TopLeft));
        world.add_child(table, e);
        *view = e;
    }

    // Düğmeler ve yazılar.
    let mut button = |label: &str, command: Command, x: f32| {
        let e = world.spawn_bundle(ButtonBundle {
            style: place(x, TOP + CARD_H - 40.0, 92.0, 40.0),
            background_color: BackgroundColor(SLOT),
            ..Default::default()
        });
        world.add_component(e, Button(command));
        world.add_component(e, Text::screen(label, font, 20.0, Vec2::ZERO).with_anchor(TextAnchor::Center).with_color(FACE));
        world.add_child(table, e);
    };
    button("Geri", Command::Undo, 24.0);
    button("Yeni", Command::New, 124.0);
    let status = world.spawn_bundle(NodeBundle {
        style: place(24.0, TOP, 200.0, 60.0),
        background_color: BackgroundColor(Vec4::ZERO),
        ..Default::default()
    });
    world.add_component(status, Text::screen("", font, 20.0, Vec2::ZERO).with_color(FACE));
    world.add_child(table, status);
    let banner = world.spawn_bundle(NodeBundle {
        style: place(W * 0.5 - 220.0, H * 0.5 - 40.0, 440.0, 80.0),
        background_color: BackgroundColor(Vec4::ZERO),
        ..Default::default()
    });
    world.add_component(banner, ZIndex(500));
    world.add_component(banner, Text::screen("", font, 44.0, Vec2::ZERO).with_anchor(TextAnchor::Center).with_color(FACE));
    world.add_child(table, banner);

    if let Some(mut audio) = sound.then(AudioManager::new).and_then(Result::ok) {
        audio.load_sound_bytes("tak", Blip::new(Wave::Triangle, 520.0, 0.04).with_volume(0.25).wav());
        audio.load_sound_bytes("temel", Blip::new(Wave::Sine, 660.0, 0.1).with_slide(990.0).with_volume(0.3).wav());
        audio.load_sound_bytes("yok", Blip::new(Wave::Square, 180.0, 0.08).with_volume(0.15).wav());
        world.insert_resource(audio);
    }

    let mut rng = Rng::new(seed);
    let board = Board::deal(&mut rng);
    let mut game = Game {
        board,
        history: Vec::new(),
        views,
        slots,
        stock_slot,
        status,
        banner,
        rng,
        moves: 0,
        dragging: None,
        shown: [Vec2::new(LEFT, TOP); 52],
        auto_timer: 0.0,
        auto: std::env::var_os("SABIR_OTO").is_some().then_some(0),
    };
    // İlk dağıtım desteden kayarak geliyor.
    lay_out(world, &mut game, true);
    game
}

fn column_x(i: usize) -> f32 {
    LEFT + i as f32 * (CARD_W + GAP)
}

/// Bir kartın tahtadaki yeri ve katmanı, düzen pikseli.
fn place_of(board: &Board, card: Card) -> (Vec2, i32) {
    let (pile, i, _) = board.find(card);
    match pile {
        Pile::Stock => (Vec2::new(LEFT, TOP), 1 + i as i32),
        // Açılanların en üstteki üçü yelpaze gibi.
        Pile::Waste => {
            let shown = board.waste.len().saturating_sub(3);
            let fan = i.saturating_sub(shown) as f32 * 32.0;
            (Vec2::new(column_x(1) + fan, TOP), 1 + i as i32)
        }
        Pile::Foundation(f) => (Vec2::new(column_x(3 + f), TOP), 1 + i as i32),
        Pile::Tableau(t) => {
            let y = board.tableau[t][..i].iter().map(|(_, up)| if *up { UP_STEP } else { DOWN_STEP }).sum::<f32>();
            (Vec2::new(column_x(t), ROW + y), 1 + i as i32)
        }
    }
}

/// Her kartı tahtadaki yerine koyar. Yeri değişen kart eski yerinden kayarak gelir (düzende yeni
/// yerinde, `UiTransform` eski yerinin farkıyla başlayıp sıfıra iniyor).
fn lay_out(world: &mut World, game: &mut Game, slide: bool) {
    for c in 0..52u8 {
        let e = game.views[usize::from(c)];
        let (at, layer) = place_of(&game.board, c);
        let (_, _, up) = game.board.find(c);
        let before = game.shown[usize::from(c)];
        if let Some(mut style) = world.query_mut::<&mut Style>().get_mut(e) {
            let want = UiRect { left: Val::Px(at.x), top: Val::Px(at.y), ..UiRect::default() };
            if style.inset != want {
                style.inset = want;
            }
        }
        world.add_component(e, ZIndex(layer));
        if slide && before != at {
            let from = before - at;
            world.add_component(e, UiTransform { translation: from, opacity: 1.0 });
            world.add_component(e, UiTween::translation(Vec2::ZERO, 0.18).with_ease(Ease::OutCubic));
        }
        game.shown[usize::from(c)] = at;
        set_fill(world, e, if up { FACE } else { BACK });
        let (label, colour) = if up { (name(c), if red(c) { RED } else { BLACK }) } else { (String::new(), BLACK) };
        if let Some(mut t) = world.query_mut::<&mut Text>().get_mut(e) {
            if t.content != label {
                t.content = label;
            }
            t.color = colour;
        }
    }
    let status = format!("Hamle {}\nDeste {}", game.moves, game.board.stock.len());
    set_text(world, game.status, &status);
    set_text(world, game.banner, if game.board.won() { "Bitti — tebrikler!" } else { "" });
}

/// `card`'ı `to`'ya oynar: olduysa geri almak için önceki tahtayı saklar, ses ve yerleşim.
fn play(world: &mut World, game: &mut Game, card: Card, to: Pile) -> bool {
    let before = game.board.clone();
    if !game.board.play(card, to) {
        return false;
    }
    game.history.push(before);
    game.moves += 1;
    sound(world, if matches!(to, Pile::Foundation(_)) { "temel" } else { "tak" });
    lay_out(world, game, true);
    true
}

fn draw_card(world: &mut World, game: &mut Game) {
    let before = game.board.clone();
    if game.board.draw() {
        game.history.push(before);
        game.moves += 1;
        sound(world, "tak");
        lay_out(world, game, true);
    }
}

fn undo(world: &mut World, game: &mut Game) {
    if let Some(board) = game.history.pop() {
        game.board = board;
        game.moves += 1;
        lay_out(world, game, true);
    } else {
        sound(world, "yok");
    }
}

fn new_game(world: &mut World, game: &mut Game) {
    game.board = Board::deal(&mut game.rng);
    game.history.clear();
    game.moves = 0;
    for p in game.shown.iter_mut() {
        *p = Vec2::new(LEFT, TOP);
    }
    lay_out(world, game, true);
}

/// `--test`: pencere açmadan dener. Kurallar: dağıtım, renk ve sıra, temel, Papaz boş sütuna,
/// geri alma; sonra gerçek fareyle: bir kartı sütundan sütuna sürüklemek, temele çift tıklamak,
/// desteye tıklamak, yanlış yere bırakılanın yerinde kalması.
fn self_test() -> bool {
    let mut checks: Vec<(&str, bool)> = Vec::new();

    // Kurallar, elle kurulmuş bir tahtada.
    let mut rng = Rng::new(7);
    let dealt = Board::deal(&mut rng);
    checks.push((
        "dağıtım: 28 sütunda, 24 destede, her sütunun yalnız üstü açık",
        dealt.whole()
            && dealt.stock.len() == 24
            && dealt.tableau.iter().enumerate().all(|(i, t)| t.len() == i + 1 && t.iter().rev().skip(1).all(|(_, up)| !up) && t.last().unwrap().1),
    ));
    let c = |r: u8, s: usize| (s * 13) as u8 + r - 1;
    let mut b = Board { stock: vec![], waste: vec![], foundations: Default::default(), tableau: Default::default() };
    b.tableau[0] = vec![(c(5, 0), false), (c(9, 0), true)]; // kapalı 5♠, açık 9♠
    b.tableau[1] = vec![(c(8, 1), true), (c(7, 3), true)]; // 8♥ ve üstünde 7♣
    b.tableau[2] = vec![(c(1, 2), true)]; // A♦
    b.tableau[3] = vec![(c(13, 1), true)]; // K♥
    b.waste = vec![c(2, 2)]; // 2♦
    checks.push(("aynı renge konmuyor (7♣ → 9♠ değil, 8♥ → 9♠ değil değil)", !b.clone().play(c(7, 3), Pile::Tableau(0))));
    let mut moved = b.clone();
    checks.push(("8♥ ile 7♣ birlikte 9♠'nin üstüne", moved.play(c(8, 1), Pile::Tableau(0)) && moved.tableau[0].len() == 4 && moved.tableau[1].is_empty()));
    checks.push(("A♦ temele, 2♦ üstüne", {
        let mut f = b.clone();
        f.play(c(1, 2), Pile::Foundation(0)) && f.play(c(2, 2), Pile::Foundation(0)) && f.foundations[0].len() == 2 && f.waste.is_empty()
    }));
    checks.push(("2♦ boş temele gitmiyor", !b.clone().play(c(2, 2), Pile::Foundation(1))));
    checks.push(("Papaz boş sütuna gidiyor, 9 gitmiyor", {
        let mut k = moved.clone();
        k.play(c(13, 1), Pile::Tableau(1)) && !moved.clone().play(c(9, 0), Pile::Tableau(1))
    }));
    checks.push(("üstü giden kapalı kart açılıyor", {
        // 9♠ kırmızı bir 10'un üstüne gidiyor; altındaki kapalı 5♠ açılmalı.
        let mut e = b.clone();
        e.tableau[4] = vec![(c(10, 1), true)];
        e.play(c(9, 0), Pile::Tableau(4)) && e.tableau[0] == vec![(c(5, 0), true)]
    }));
    let mut cycle = Board { stock: vec![1, 2, 3], ..b.clone() };
    cycle.waste.clear();
    let opened = cycle.draw() && cycle.draw() && cycle.draw();
    checks.push(("deste bitince açılanlar sırasıyla geri dönüyor", opened && cycle.draw() && cycle.stock == vec![1, 2, 3] && cycle.waste.is_empty()));

    // Gerçek fareyle, pencere olmadan: 1280×720'lik bir pencere.
    let mut world = World::new();
    world.insert_resource(gizmo::core::window::WindowInfo { width: W, height: H });
    let mut game = setup(&mut world, HeadlessFrames::font(), false, 11);
    game.auto = Some(1_000_000);
    // Elle kurulmuş bir masa: 9♠ sütun 0'da, 8♥ sütun 1'de, A♣ açılanlarda.
    game.board = Board { stock: vec![], waste: vec![], foundations: Default::default(), tableau: Default::default() };
    let rest: Vec<Card> = (0..52).filter(|&k| ![c(9, 0), c(8, 1), c(1, 3)].contains(&k)).collect();
    game.board.stock = rest;
    game.board.tableau[0] = vec![(c(9, 0), true)];
    game.board.tableau[1] = vec![(c(8, 1), true)];
    game.board.waste = vec![c(1, 3)];
    lay_out(&mut world, &mut game, false);
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world).add_plugin(UiPlugin);
    let dt = 1.0 / 60.0;
    let frames = |run: &mut HeadlessFrames, n: u32| {
        for _ in 0..n {
            run.frame(dt, update);
        }
    };
    frames(&mut run, 3);
    // 8♥'yi tut, 9♠'nin üstüne sürükle, bırak.
    let at = |run: &HeadlessFrames, card: Card| {
        let game = run.world.get_resource::<Game>().expect("oyun");
        let (p, _) = place_of(&game.board, card);
        p + Vec2::new(CARD_W * 0.5, 10.0)
    };
    let (from, to) = (at(&run, c(8, 1)), at(&run, c(9, 0)) + Vec2::new(0.0, UP_STEP));
    run.input.set_mouse_position(from.x, from.y);
    frames(&mut run, 1);
    run.input.on_mouse_button_pressed(0);
    frames(&mut run, 1);
    for k in 1..=10 {
        let p = from + (to - from) * (k as f32 / 10.0);
        run.input.on_mouse_moved(p.x, p.y);
        frames(&mut run, 1);
    }
    run.input.on_mouse_button_released(0);
    frames(&mut run, 2);
    let board = |run: &HeadlessFrames| run.world.get_resource::<Game>().expect("oyun").board.clone();
    checks.push(("fareyle 8♥ sürüklendi, 9♠'nin üstünde", board(&run).tableau[0] == vec![(c(9, 0), true), (c(8, 1), true)]));
    // A♣'ye çift tıkla: temele gider.
    let ace = at(&run, c(1, 3));
    run.input.set_mouse_position(ace.x, ace.y);
    frames(&mut run, 1);
    for _ in 0..2 {
        run.input.on_mouse_button_pressed(0);
        frames(&mut run, 1);
        run.input.on_mouse_button_released(0);
        frames(&mut run, 3);
    }
    checks.push(("çift tıklanan A♣ temelde", board(&run).foundations.iter().any(|f| f == &vec![c(1, 3)])));
    // Desteye tıkla: bir kart açılır.
    let deck = Vec2::new(LEFT + CARD_W * 0.5, TOP + CARD_H * 0.5);
    let before = board(&run).stock.len();
    run.input.set_mouse_position(deck.x, deck.y);
    frames(&mut run, 1);
    run.input.on_mouse_button_pressed(0);
    frames(&mut run, 1);
    run.input.on_mouse_button_released(0);
    frames(&mut run, 2);
    checks.push(("desteye tıklanınca bir kart açıldı", board(&run).stock.len() + 1 == before && board(&run).waste.len() == 1));
    // Açılan kartı boş bir sütunun ortasına sürükle (Papaz değilse yerinde kalmalı).
    let opened = *board(&run).waste.last().expect("açılan kart");
    let start = at(&run, opened);
    let end = Vec2::new(column_x(5) + CARD_W * 0.5, ROW + 200.0);
    run.input.set_mouse_position(start.x, start.y);
    frames(&mut run, 1);
    run.input.on_mouse_button_pressed(0);
    frames(&mut run, 1);
    run.input.on_mouse_moved(end.x, end.y);
    frames(&mut run, 2);
    run.input.on_mouse_button_released(0);
    frames(&mut run, 2);
    let after = board(&run);
    let kept = if rank(opened) == 13 { after.tableau[5] == vec![(opened, true)] } else { after.waste.last() == Some(&opened) };
    checks.push(("boş sütuna yalnız Papaz gidiyor", kept));
    // Geri al: son hamle geri geliyor.
    let undone_from = board(&run);
    run.input.on_key_pressed(KeyCode::KeyU as u32);
    frames(&mut run, 1);
    run.input.on_key_released(KeyCode::KeyU as u32);
    frames(&mut run, 1);
    checks.push(("U son hamleyi geri aldı", board(&run) != undone_from && board(&run).whole()));

    // Kendi kendine oynayan bir el: her hamleden sonra kartlar eksiksiz.
    let mut rng = Rng::new(3);
    let mut b = Board::deal(&mut rng);
    let mut whole = true;
    for _ in 0..600 {
        let moved = greedy_move(&mut b);
        whole &= b.whole();
        if !moved {
            break;
        }
    }
    checks.push(("açgözlü bir oyun boyunca 52 kart, her biri bir kez", whole));

    let failed: Vec<&str> = checks.iter().filter(|(_, ok)| !ok).map(|(name, _)| *name).collect();
    println!(
        "sabir --test: {} sınamadan {} geçti{} → {}",
        checks.len(),
        checks.len() - failed.len(),
        if failed.is_empty() { String::new() } else { format!("; geçmeyen: {}", failed.join(", ")) },
        if failed.is_empty() { "tamam" } else { "BAŞARISIZ" }
    );
    failed.is_empty()
}

/// Bir hamle: temele gidebilen bir kart, yoksa sütundan sütuna bir dizi, yoksa desteden bir kart.
/// Bir şey yaptıysa `true`.
fn greedy_move(b: &mut Board) -> bool {
    if let Some(c) = b.next_home() {
        let to = b.foundation_for(c).expect("gidebiliyor");
        return b.play(c, to);
    }
    for from in 0..7 {
        let Some(i) = b.tableau[from].iter().position(|(_, up)| *up) else { continue };
        let card = b.tableau[from][i].0;
        // Kapalı kartı açmayan bir Papaz taşıması boşuna.
        if i == 0 && rank(card) == 13 {
            continue;
        }
        for to in 0..7 {
            if to != from && b.play(card, Pile::Tableau(to)) {
                return true;
            }
        }
    }
    if let Some(&top) = b.waste.last() {
        for to in 0..7 {
            if b.play(top, Pile::Tableau(to)) {
                return true;
            }
        }
    }
    // Deste döngüsü sonsuz olmasın: dönebilecek bir şey yoksa dur.
    if b.stock.is_empty() && b.waste.len() <= 1 {
        return false;
    }
    b.draw()
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Sabır", 1280, 720)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_secs());
            let game = setup(scene.world, font, true, seed);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let k = world.get_resource::<UiScale>().map_or(1.0, |s| s.factor());

    // Düğmeler ve tuşlar.
    let pressed: Vec<Command> = {
        let clicked = world.query::<&Clicked>();
        world.query::<(Entity, &Button)>().iter().filter(|(e, _)| clicked.get(*e).is_some_and(|c| c.0)).map(|(_, b)| b.0).collect()
    };
    if pressed.contains(&Command::Undo) || input.is_key_just_pressed(KeyCode::KeyU as u32) {
        undo(world, &mut game);
    }
    if pressed.contains(&Command::New) || input.is_key_just_pressed(KeyCode::KeyN as u32) {
        new_game(world, &mut game);
    }

    // Sürükleme: bir kart tutulunca birlikte gidenler belli oluyor, imleci izliyorlar.
    let drags: Vec<(Card, Drag, bool, bool)> = {
        let drags = world.query::<&Drag>();
        let clicks = world.query::<&Clicked>();
        let doubles = world.query::<&DoubleClicked>();
        world
            .query::<(Entity, &CardView)>()
            .iter()
            .filter_map(|(e, v)| {
                let d = drags.get(e).copied()?;
                Some((v.0, d, clicks.get(e).is_some_and(|c| c.0), doubles.get(e).is_some_and(DoubleClicked::get)))
            })
            .collect()
    };
    let mut clicked_deck = world.query::<&Clicked>().get(game.stock_slot).is_some_and(|c| c.0);
    for (card, drag, click, double) in drags {
        let (pile, _, _) = game.board.find(card);
        if drag.just_started() {
            let run = game.board.run_from(card);
            game.dragging = (!run.is_empty()).then_some(run);
        }
        let moved = drag.delta().length() / k > SLOP;
        if drag.is_dragging() && !drag.just_ended() {
            if let Some(run) = game.dragging.as_ref().filter(|r| r.first() == Some(&card)) {
                for (j, &c) in run.iter().enumerate() {
                    let e = game.views[usize::from(c)];
                    world.remove_component::<UiTween>(e);
                    world.add_component(e, ZIndex(200 + j as i32));
                    world.add_component(e, UiTransform { translation: drag.delta() / k, opacity: 1.0 });
                }
            }
        }
        if drag.just_ended() {
            let run = game.dragging.take().filter(|r| r.first() == Some(&card));
            if let Some(run) = run {
                let target = drag.dropped_on().and_then(|t| game.slots.iter().find(|(e, _)| *e == t).map(|(_, p)| *p));
                // Kart sürüklenmeden bırakıldıysa tıklamadır; sürüklendiyse bıraktığı yer.
                let played = moved && target.is_some_and(|to| play(world, &mut game, card, to));
                if !played {
                    // Yerine kayarak dönüyor.
                    for &c in &run {
                        let e = game.views[usize::from(c)];
                        world.add_component(e, UiTween::translation(Vec2::ZERO, 0.2).with_ease(Ease::OutBack));
                    }
                    if moved {
                        sound(world, "yok");
                    }
                    lay_out(world, &mut game, false);
                }
            }
        }
        if click && pile == Pile::Stock {
            clicked_deck = true;
        }
        if double {
            if let Some(to) = game.board.foundation_for(card) {
                play(world, &mut game, card, to);
            }
        }
    }
    if clicked_deck {
        draw_card(world, &mut game);
    }

    // Hepsi açıldı: kalanı temellere kendisi götürüyor.
    if game.board.settled() && !game.board.won() && game.dragging.is_none() {
        game.auto_timer += dt;
        if game.auto_timer >= AUTO_EVERY {
            game.auto_timer = 0.0;
            if let Some(c) = game.board.next_home() {
                let to = game.board.foundation_for(c).expect("gidebiliyor");
                play(world, &mut game, c, to);
            }
        }
    }

    // Ekran görüntüsü için birkaç hamle.
    if let Some(f) = game.auto.map(|f| f + 1) {
        game.auto = Some(f);
        if f % 20 == 0 && f <= 400 {
            let before = game.board.clone();
            if greedy_move(&mut game.board) {
                game.history.push(before);
                game.moves += 1;
                lay_out(world, &mut game, true);
            }
        }
    }

    // Bırakma yerleri, üstlerinde bir şey varken yanıyor.
    for &(e, pile) in &game.slots {
        let lit = world.query::<&DropTarget>().get(e).is_some_and(|t| t.hovered_by().is_some());
        if let Pile::Foundation(_) = pile {
            set_fill(world, e, if lit { SLOT_LIT } else { SLOT });
        } else {
            set_fill(world, e, if lit { SLOT_LIT.truncate().extend(0.35) } else { Vec4::ZERO });
        }
    }
    world.insert_resource(game);
}

fn sound(world: &mut World, name: &str) {
    if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
        let _ = audio.play(name);
    }
}

fn set_text(world: &mut World, id: Entity, s: &str) {
    if let Some(mut t) = world.query_mut::<&mut Text>().get_mut(id) {
        if t.content != s {
            t.content = s.to_string();
        }
    }
}

fn set_fill(world: &mut World, id: Entity, c: Vec4) {
    if let Some(mut b) = world.query_mut::<&mut BackgroundColor>().get_mut(id) {
        if b.0 != c {
            b.0 = c;
        }
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
