//! # Düello — bir kart düellosu
//!
//! Sen ve rakip, yirmişer canla. Her tur mana bir artıyor (en çok on), elin beşe tamamlanıyor;
//! bir kartı oynamak için onu hedefine sürükle — saldırılar rakibin portresine, kalkan ve
//! iyileşme kendi portrene. **Turu bitir**, rakip oynasın. Kalkan bir sonraki turuna kadar
//! hasarı emer. Desten bitince atılanlar karılıp yeniden deste oluyor. Motorun on ikinci oyunu;
//! tamamen arayüz ve hareket: kartlar desteden ele kayıyor, oynanınca hedefe uçup sönüyor, yanlış
//! yere bırakılınca yerine dönüyor, el boşalınca kalanlar yana kayıyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **sürükle** bir kartı bir portreye | oyna |
//! | **Turu bitir** ya da **Enter** | sıra rakipte |
//! | **R** | yeni düello |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Arayüz öğeleri canlandırılamıyordu.** `Tween` bir `Transform`'u yürütüyor; bir arayüz
//!   kutusunun yeri ise `Style`'dan, her kare düzenle hesaplanıyor. Bir kartı desteden ele
//!   kaydırmak, sürüklerken imleci izletmek, bırakınca geri yollamak için her kare `inset`
//!   hesaplamak gerekiyordu; `birlestir` sürüklenen öğeyi bunun için `Absolute`'a çeviriyordu.
//!   **Kapatıldı:** `UiTransform { translation, opacity }` — düzenden *sonra* uygulanan bir
//!   kaydırma (çocuklarıyla birlikte, tıklama da onu izliyor) ve bir saydamlık (çocuklarına,
//!   arka planlara ve yazılara geçiyor) — ve onu yürüten `UiTween`
//!   (`UiTween::translation(hedef, sn)`, `opacity`, `with_ease`, `with_delay`, `is_finished`).
//!   Yumuşatma eğrileri `Tween`'inkiler (`Ease`, artık `gizmo-math`'te).
//!
//! ## Ortam değişkenleri
//!   * `DUELLO_OTO=1` — kendi kendine bir kart oynar (ekran görüntüsü için)
//!   * `GIZMO_FONT=<yol>` — font

use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::math::Ease;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::ui::{
    BackgroundColor, ButtonBundle, Clicked, Drag, DropTarget, NodeBundle, PositionType, Style, Tooltip, UiPlugin,
    UiRect, UiScale, UiTransform, UiTween, Val, ZIndex,
};

const HAND_MAX: usize = 5;
const CARD: Vec2 = Vec2::new(120.0, 160.0);
const HAND_Y: f32 = 530.0;
const DECK_AT: Vec2 = Vec2::new(60.0, 530.0);
const START_HP: i32 = 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Strike,
    Heavy,
    Shield,
    Heal,
    Fireball,
    Draw,
}

const POOL: [Kind; 6] = [Kind::Strike, Kind::Heavy, Kind::Shield, Kind::Heal, Kind::Fireball, Kind::Draw];

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Strike => "Vuruş",
            Kind::Heavy => "Ağır Darbe",
            Kind::Shield => "Kalkan",
            Kind::Heal => "İyileş",
            Kind::Fireball => "Ateş Topu",
            Kind::Draw => "Hazırlık",
        }
    }
    fn cost(self) -> u32 {
        match self {
            Kind::Strike | Kind::Shield => 1,
            Kind::Heavy | Kind::Heal => 2,
            Kind::Fireball => 3,
            Kind::Draw => 0,
        }
    }
    fn hostile(self) -> bool {
        matches!(self, Kind::Strike | Kind::Heavy | Kind::Fireball)
    }
    fn text(self) -> String {
        match self {
            Kind::Strike => "6 hasar".into(),
            Kind::Heavy => "12 hasar".into(),
            Kind::Fireball => "15 hasar".into(),
            Kind::Shield => "5 kalkan".into(),
            Kind::Heal => "6 can".into(),
            Kind::Draw => "2 kart çek".into(),
        }
    }
    fn colour(self) -> Vec4 {
        if self.hostile() {
            Vec4::new(0.55, 0.18, 0.16, 1.0)
        } else if self == Kind::Draw {
            Vec4::new(0.25, 0.3, 0.55, 1.0)
        } else {
            Vec4::new(0.18, 0.45, 0.3, 1.0)
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Fighter {
    hp: i32,
    block: i32,
}

impl Fighter {
    fn hit(&mut self, damage: i32) {
        let absorbed = damage.min(self.block);
        self.block -= absorbed;
        self.hp -= damage - absorbed;
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Player,
    /// Rakip oynuyor: bir sonraki hamlesine kalan süre.
    Enemy { wait: f32 },
    Over,
}

/// Oynanıp uçan (ya da rakibin oynadığı) bir kart: sönünce siliniyor.
#[derive(Clone, Copy)]
struct Leaving;
gizmo::core::impl_component!(Leaving);

struct Game {
    me: Fighter,
    foe: Fighter,
    deck: Vec<Kind>,
    discard: Vec<Kind>,
    /// Eldeki kartlar, soldan sağa: türü ve varlığı.
    hand: Vec<(Kind, u32)>,
    mana: u32,
    /// Rakibin bu turda kalan manası.
    foe_mana: u32,
    turn: u32,
    phase: Phase,
    rng: Rng,
    font: FontId,
    table: u32,
    me_box: u32,
    foe_box: u32,
    end_turn: u32,
    deck_box: u32,
    log: u32,
    auto: Option<u32>,
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Düello", 1280, 720)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.spawn_bundle(CameraBundle { primary: true, ..Default::default() });
            scene.world.insert_resource(UiScale::fit(1280.0, 720.0));

            let world = &mut *scene.world;
            let table = world.spawn_bundle(NodeBundle {
                // Pencerenin tamamı: oran 16:9 değilse kalan yer de masa.
                style: Style { width: Val::Percent(100.0), height: Val::Percent(100.0), ..Default::default() },
                background_color: BackgroundColor(Vec4::new(0.06, 0.12, 0.09, 1.0)),
                ..Default::default()
            });
            let table_id = table.id();
            let foe_box = panel(world, table, Vec2::new(520.0, 40.0), Vec2::new(240.0, 110.0), Vec4::new(0.3, 0.12, 0.12, 1.0), font, 22.0);
            world.add_component(world.entity(foe_box).unwrap(), DropTarget::default());
            let me_box = panel(world, table, Vec2::new(520.0, 380.0), Vec2::new(240.0, 110.0), Vec4::new(0.12, 0.22, 0.3, 1.0), font, 22.0);
            world.add_component(world.entity(me_box).unwrap(), DropTarget::default());
            let deck_box = panel(world, table, DECK_AT, CARD, Vec4::new(0.2, 0.17, 0.12, 1.0), font, 20.0);
            let log = panel(world, table, Vec2::new(40.0, 230.0), Vec2::new(420.0, 90.0), Vec4::ZERO, font, 20.0);
            let end_turn = world.spawn_bundle(ButtonBundle {
                style: absolute(Vec2::new(1060.0, 300.0), Vec2::new(180.0, 64.0)),
                background_color: BackgroundColor(Vec4::new(0.35, 0.3, 0.15, 1.0)),
                ..Default::default()
            });
            world.add_component(end_turn, Text::screen("Turu bitir", font, 22.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
            world.add_child(table, end_turn);

            let mut game = Game {
                me: Fighter::default(),
                foe: Fighter::default(),
                deck: Vec::new(),
                discard: Vec::new(),
                hand: Vec::new(),
                mana: 0,
                foe_mana: 0,
                turn: 0,
                phase: Phase::Player,
                rng: Rng::new(0xD0E1),
                font,
                table: table_id,
                me_box,
                foe_box,
                end_turn: end_turn.id(),
                deck_box,
                log,
                auto: std::env::var_os("DUELLO_OTO").is_some().then_some(0),
            };
            new_duel(world, &mut game);
            world.insert_resource(game);
        })
        .set_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn absolute(at: Vec2, size: Vec2) -> Style {
    Style {
        position_type: PositionType::Absolute,
        inset: UiRect { left: Val::Px(at.x), top: Val::Px(at.y), right: Val::Auto, bottom: Val::Auto },
        width: Val::Px(size.x),
        height: Val::Px(size.y),
        ..Default::default()
    }
}

fn panel(world: &mut World, parent: Entity, at: Vec2, size: Vec2, colour: Vec4, font: FontId, px: f32) -> u32 {
    let e = world.spawn_bundle(NodeBundle {
        style: absolute(at, size),
        background_color: BackgroundColor(colour),
        ..Default::default()
    });
    world.add_component(e, Text::screen("", font, px, Vec2::ZERO).with_anchor(TextAnchor::Center).with_align(TextAlign::Center));
    world.add_child(parent, e);
    e.id()
}

fn slot(i: usize, n: usize) -> Vec2 {
    let width = n as f32 * CARD.x + (n.saturating_sub(1)) as f32 * 16.0;
    Vec2::new(640.0 - width / 2.0 + i as f32 * (CARD.x + 16.0), HAND_Y)
}

fn new_duel(world: &mut World, game: &mut Game) {
    for (_, id) in game.hand.drain(..) {
        if let Some(e) = world.get_entity(id) {
            world.despawn_recursive(e);
        }
    }
    game.me = Fighter { hp: START_HP, block: 0 };
    game.foe = Fighter { hp: START_HP, block: 0 };
    game.discard.clear();
    game.deck = (0..20).map(|i| POOL[i % POOL.len()]).collect();
    shuffle(&mut game.deck, &mut game.rng);
    game.turn = 0;
    start_turn(world, game);
}

fn shuffle(v: &mut [Kind], rng: &mut Rng) {
    for i in (1..v.len()).rev() {
        let j = rng.range(0..=i as u32) as usize;
        v.swap(i, j);
    }
}

fn start_turn(world: &mut World, game: &mut Game) {
    game.turn += 1;
    game.mana = game.turn.min(10);
    game.me.block = 0;
    game.phase = Phase::Player;
    let want = HAND_MAX.saturating_sub(game.hand.len());
    draw(world, game, want);
    say(world, game, &format!("Tur {} — sıra sende.", game.turn));
}

/// `n` kart çeker: her biri desteden kendi yerine kayıyor, biri ötekinin biraz ardından.
fn draw(world: &mut World, game: &mut Game, n: usize) {
    let mut drawn = 0;
    for _ in 0..n {
        if game.hand.len() >= HAND_MAX + 2 {
            break;
        }
        if game.deck.is_empty() {
            game.deck.append(&mut game.discard);
            shuffle(&mut game.deck, &mut game.rng);
        }
        let Some(kind) = game.deck.pop() else { break };
        let id = spawn_card(world, game, kind);
        game.hand.push((kind, id));
        drawn += 1;
    }
    // Herkes yeni yerine; yeni gelenler desteden, ötekiler eski yerlerinden kayarak.
    relayout(world, game, drawn);
}

fn spawn_card(world: &mut World, game: &Game, kind: Kind) -> u32 {
    let card = world.spawn_bundle(ButtonBundle {
        style: absolute(DECK_AT, CARD),
        background_color: BackgroundColor(kind.colour()),
        ..Default::default()
    });
    world.add_component(
        card,
        Text::screen(format!("{}\n\n{}\n\n◆ {}", kind.name(), kind.text(), kind.cost()), game.font, 18.0, Vec2::ZERO)
            .with_anchor(TextAnchor::Center)
            .with_align(TextAlign::Center),
    );
    world.add_component(card, Drag::default());
    world.add_component(card, UiTransform::default());
    let target = if kind.hostile() { "rakibe" } else { "kendine" };
    world.add_component(card, Tooltip::new(format!("{} — {}, {} mana.\nOynamak için {} sürükle.", kind.name(), kind.text(), kind.cost(), target)));
    if let Some(table) = world.get_entity(game.table) {
        world.add_child(table, card);
    }
    card.id()
}

/// Kartları yeni yerlerine koyar; ekranda oldukları yerden oraya kayarak (`fresh` kadar son
/// kart desteden geliyor, biri ötekinin ardından).
fn relayout(world: &mut World, game: &Game, fresh: usize) {
    let n = game.hand.len();
    for (i, &(_, id)) in game.hand.iter().enumerate() {
        let to = slot(i, n);
        // Ekranda şu an nerede: eski yeri artı kayması.
        let (from, shift) = {
            let styles = world.borrow::<Style>();
            let ts = world.borrow::<UiTransform>();
            let s = styles.get(id).copied().unwrap_or_default();
            let at = match (s.inset.left, s.inset.top) {
                (Val::Px(x), Val::Px(y)) => Vec2::new(x, y),
                _ => to,
            };
            (at, ts.get(id).map_or(Vec2::ZERO, |t| t.translation))
        };
        let delay = if i + fresh >= n { (i + fresh - n) as f32 * 0.08 } else { 0.0 };
        if let Some(mut s) = world.borrow_mut::<Style>().get_mut(id) {
            *s = absolute(to, CARD);
        }
        if let Some(mut t) = world.borrow_mut::<UiTransform>().get_mut(id) {
            t.translation = from + shift - to;
        }
        if let Some(e) = world.get_entity(id) {
            world.add_component(e, UiTween::translation(Vec2::ZERO, 0.35).with_ease(Ease::OutCubic).with_delay(delay));
        }
    }
}

fn say(world: &mut World, game: &Game, s: &str) {
    set_text(world, game.log, s);
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };

    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        new_duel(world, &mut game);
    }

    // Sönen kartlar bitince siliniyor.
    let gone: Vec<u32> = {
        let leaving = world.borrow::<Leaving>();
        let tweens = world.borrow::<UiTween>();
        leaving.iter().filter(|(id, _)| tweens.get(*id).is_none_or(|t| t.is_finished())).map(|(id, _)| id).collect()
    };
    for id in gone {
        if let Some(e) = world.get_entity(id) {
            world.despawn_recursive(e);
        }
    }

    match game.phase {
        Phase::Player => player_turn(world, &mut game, input),
        Phase::Enemy { wait } => {
            let wait = wait - dt;
            if wait > 0.0 {
                game.phase = Phase::Enemy { wait };
            } else {
                enemy_move(world, &mut game);
            }
        }
        Phase::Over => {}
    }
    if game.phase != Phase::Over && (game.me.hp <= 0 || game.foe.hp <= 0) {
        game.phase = Phase::Over;
        let s = if game.foe.hp <= 0 { "Kazandın! (R: yeni düello)" } else { "Kaybettin. (R: yeni düello)" };
        say(world, &game, s);
    }
    hud(world, &game);
    world.insert_resource(game);
}

fn player_turn(world: &mut World, game: &mut Game, input: &Input) {
    if let Some(f) = game.auto.map(|f| f + 1) {
        game.auto = Some(f);
        // Ekran görüntüsü için: bir saldırı kartını rakibe oyna.
        if f == 90 {
            if let Some(i) = game.hand.iter().position(|(k, _)| k.hostile() && k.cost() <= game.mana) {
                play(world, game, i, true);
            }
        }
    }
    let clicked = world.borrow::<Clicked>().get(game.end_turn).is_some_and(|c| c.0);
    if clicked || input.is_key_just_pressed(KeyCode::Enter as u32) {
        game.phase = Phase::Enemy { wait: 0.6 };
        game.foe.block = 0;
        game.foe_mana = game.turn.min(10);
        say(world, game, "Sıra rakipte…");
        return;
    }
    // Sürüklenen kart imleci izliyor; bırakılınca ya oynanıyor ya yerine dönüyor.
    let mut dropped: Option<(usize, Option<u32>)> = None;
    for (i, &(_, id)) in game.hand.iter().enumerate() {
        let Some(d) = world.borrow::<Drag>().get(id).copied() else { continue };
        if d.is_dragging() {
            let follow = d.delta() / ui_scale(world);
            if let Some(mut t) = world.borrow_mut::<UiTransform>().get_mut(id) {
                t.translation = follow;
            }
            // Kaldırıldığı an: kayıyorsa dursun, öbürlerinin önüne geçsin.
            if d.just_started() {
                if let Some(e) = world.get_entity(id) {
                    world.remove_component::<UiTween>(e);
                    world.add_component(e, ZIndex(10));
                }
            }
        } else if d.just_ended() {
            dropped = Some((i, d.dropped_on()));
        }
    }
    if let Some((i, on)) = dropped {
        let (kind, id) = game.hand[i];
        let target_ok = match on {
            Some(t) if t == game.foe_box => kind.hostile(),
            Some(t) if t == game.me_box => !kind.hostile(),
            _ => false,
        };
        if let Some(e) = world.get_entity(id) {
            world.remove_component::<ZIndex>(e);
        }
        if target_ok && kind.cost() <= game.mana {
            play(world, game, i, kind.hostile());
        } else {
            if target_ok {
                say(world, game, "Mana yetmiyor.");
            }
            // Yerine dön.
            if let Some(e) = world.get_entity(id) {
                world.add_component(e, UiTween::translation(Vec2::ZERO, 0.3).with_ease(Ease::OutBack));
            }
        }
    }
}

fn ui_scale(world: &World) -> f32 {
    world.get_resource::<UiScale>().map_or(1.0, |s| s.factor())
}

/// Eldeki `i`. kartı oynar: etkisi, hedefe uçup sönmesi, kalanların yana kayması.
fn play(world: &mut World, game: &mut Game, i: usize, at_foe: bool) {
    let (kind, id) = game.hand.remove(i);
    game.mana -= kind.cost();
    game.discard.push(kind);
    let target = if at_foe { game.foe_box } else { game.me_box };
    fly_away(world, id, target);
    apply(world, game, kind, true);
    relayout(world, game, 0);
}

/// Bir kartı `target` kutusunun üstüne uçurur ve söndürür; sönünce siliniyor.
fn fly_away(world: &mut World, card: u32, target: u32) {
    let (from, to) = {
        let nodes = world.borrow::<gizmo::ui::Node>();
        let k = ui_scale(world);
        let c = nodes.get(card).map(|n| n.position + n.size * 0.5).unwrap_or_default();
        let t = nodes.get(target).map(|n| n.position + n.size * 0.5).unwrap_or_default();
        (c / k, t / k)
    };
    let shift = world.borrow::<UiTransform>().get(card).map_or(Vec2::ZERO, |t| t.translation);
    if let Some(e) = world.get_entity(card) {
        world.add_component(e, Leaving);
        world.add_component(e, ZIndex(20));
        world.add_component(
            e,
            UiTween::translation(shift + to - from, 0.35).and_opacity(0.0).with_ease(Ease::InQuad),
        );
    }
}

fn apply(world: &mut World, game: &mut Game, kind: Kind, mine: bool) {
    let (user, other) = if mine { (&mut game.me, &mut game.foe) } else { (&mut game.foe, &mut game.me) };
    match kind {
        Kind::Strike => other.hit(6),
        Kind::Heavy => other.hit(12),
        Kind::Fireball => other.hit(15),
        Kind::Shield => user.block += 5,
        Kind::Heal => user.hp = (user.hp + 6).min(START_HP),
        Kind::Draw => {}
    }
    if kind.hostile() {
        // Vurulan sarsılsın.
        let hit = if mine { game.foe_box } else { game.me_box };
        if let Some(e) = world.get_entity(hit) {
            world.add_component(e, UiTransform { translation: Vec2::new(14.0, 0.0), ..Default::default() });
            world.add_component(e, UiTween::translation(Vec2::ZERO, 0.4).with_ease(Ease::OutElastic));
        }
    }
    if kind == Kind::Draw && mine {
        draw(world, game, 2);
    }
    let who = if mine { "Sen" } else { "Rakip" };
    say(world, game, &format!("{who}: {} ({}).", kind.name(), kind.text()));
}

/// Rakip, manası yettikçe bir kart oynuyor — canı azsa iyileşmeyi, yoksa gücünün yettiği en
/// sert saldırıyı, ara sıra kalkanı — her hamle arasında bir an. Manası bitince sıra sende.
fn enemy_move(world: &mut World, game: &mut Game) {
    let budget = game.foe_mana;
    let choice = if game.foe.hp < 10 && budget >= 2 && game.rng.bool() {
        Some(Kind::Heal)
    } else if budget >= 1 && game.rng.range(0u32..4) == 0 {
        Some(Kind::Shield)
    } else {
        [Kind::Fireball, Kind::Heavy, Kind::Strike].into_iter().find(|k| k.cost() <= budget)
    };
    let Some(kind) = choice.filter(|_| game.foe.hp > 0) else {
        start_turn(world, game);
        return;
    };
    game.foe_mana -= kind.cost();
    // Rakibin kartı onun portresinden çıkıp hedefe uçuyor.
    let card = spawn_card(world, game, kind);
    let at = Vec2::new(580.0, 40.0);
    if let Some(mut s) = world.borrow_mut::<Style>().get_mut(card) {
        *s = absolute(at, CARD);
    }
    let to = if kind.hostile() { Vec2::new(580.0, 380.0) } else { at };
    if let Some(e) = world.get_entity(card) {
        world.remove_component::<Drag>(e);
        world.remove_component::<Tooltip>(e);
        world.add_component(e, Leaving);
        world.add_component(e, ZIndex(20));
        world.add_component(e, UiTween::translation(to - at, 0.45).and_opacity(0.0).with_ease(Ease::InOutQuad));
    }
    apply(world, game, kind, false);
    game.phase = Phase::Enemy { wait: 0.7 };
}

fn hud(world: &mut World, game: &Game) {
    let foe = format!("Rakip\n♥ {}   Kalkan {}", game.foe.hp.max(0), game.foe.block);
    set_text(world, game.foe_box, &foe);
    let me = format!("Sen\n♥ {}   Kalkan {}\nMana {}/{}", game.me.hp.max(0), game.me.block, game.mana, game.turn.min(10));
    set_text(world, game.me_box, &me);
    let deck = format!("Deste\n{}\n\nAtılan\n{}", game.deck.len(), game.discard.len());
    set_text(world, game.deck_box, &deck);
    // Oynanabilen kartlar tam renkli, manası yetmeyenler soluk.
    for &(kind, id) in &game.hand {
        let playable = game.phase == Phase::Player && kind.cost() <= game.mana;
        let a = if playable { 1.0 } else { 0.45 };
        if let Some(mut b) = world.borrow_mut::<BackgroundColor>().get_mut(id) {
            let c = kind.colour().truncate().extend(a);
            if b.0 != c {
                b.0 = c;
            }
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
