//! # Zindan — sıra tabanlı bir zindan oyunu
//!
//! Odalar ve koridorlardan üretilmiş bir zindanda adım adım ilerle: her adımın ardından düşmanlar
//! da bir adım atıyor. Altınları topla, merdivenden aşağı in — her kat bir öncekinden kalabalık.
//! Gördüğün yer aydınlık, gördüğün ama şimdi görmediğin yer soluk, hiç görmediğin yer karanlık.
//! Motorun onuncu oyunu ve **`Tilemap`'i bir oyunun asıl dünyası olarak** kullanan ilki.
//!
//! | girdi | iş |
//! |---|---|
//! | **WASD / oklar** · sol çubuk · d-pad | bir kare yürü (basılı tut: yürümeye devam) |
//! | düşmana doğru yürü | saldır |
//! | **R** | yeni zindan |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Bir karonun tonu ayarlanamıyordu.** Görüş sisi — görülen aydınlık, hatırlanan soluk —
//!   her karo oyununun ilk istediği şey; `Tilemap`'in tek bir rengi vardı, yani her karonun
//!   tileset'te bir de soluk kopyası çizilmek zorundaydı (bu oyunun ilk sürümü öyleydi).
//!   **Kapatıldı:** `Tilemap::set_tint(x, y, renk)` / `tint` / `clear_tints` — haritanın renginin
//!   üstüne çarpılıyor; hiç ton verilmeyen bir harita bunun için bellek ayırmıyor.
//!
//! Gerisi vardı: `Tilemap` (tek çizim çağrısı), `IsHidden` (sprite'lar da uyuyor — ilk sürüm
//! görünmeyenleri uzağa taşıyordu, gerek yokmuş), `move_axis` + `HoldRepeat` (basılı tutunca
//! adım adım yürüme), `ScreenFade` (kat geçişi ve vurulma), `ProgressBar` (can), `Rng`.
//!
//! ## Ortam değişkenleri
//!   * `ZINDAN_OTO=1` — kendi kendine oynar (en yakın altına, yoksa merdivene yürür; `zindan --test`
//!     pencere açmadan böyle oynar)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::core::component::IsHidden;
use gizmo::core::input::{HoldRepeat, Input};
use gizmo::prelude::*;
use gizmo::renderer::components::{ProjectionMode, Sprite, Text, TextAlign, TextAnchor, Tilemap};
use gizmo::renderer::screen_fade::ScreenFade;
use gizmo::renderer::sprite::{SpriteFilter, SpriteImageId};
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, SimpleAppExt, SimpleSceneState};
use gizmo::ui::{BackgroundColor, NodeBundle, PositionType, ProgressBar, Style, UiPlugin, UiRect, Val};
use std::collections::{HashSet, VecDeque};

const W: usize = 48;
const H: usize = 32;
const SIGHT: i32 = 7;
const MAX_HP: i32 = 10;

/// Tileset hücreleri: zemin, duvar, merdiven.
const FLOOR: u32 = 0;
const WALL: u32 = 1;
const STAIRS: u32 = 2;
/// Görülmüş ama şu an görülmeyen bir karonun tonu.
const REMEMBERED: Vec4 = Vec4::new(0.32, 0.32, 0.4, 1.0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cell {
    Wall,
    Floor,
    Stairs,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct P(i32, i32);

struct Monster {
    at: P,
    hp: i32,
    entity: u32,
}

struct Game {
    cells: Vec<Cell>,
    seen: Vec<bool>,
    visible: HashSet<P>,
    player: P,
    hp: i32,
    gold: u32,
    depth: u32,
    monsters: Vec<Monster>,
    coins: Vec<(P, u32)>,
    rng: Rng,
    map: u32,
    player_e: u32,
    camera: u32,
    hud: u32,
    hp_bar: u32,
    banner: u32,
    walk: HoldRepeat,
    descending: bool,
    auto: bool,
    images: Images,
}

#[derive(Clone, Copy)]
struct Images {
    monster: SpriteImageId,
    coin: SpriteImageId,
}

fn idx(p: P) -> Option<usize> {
    (p.0 >= 0 && p.1 >= 0 && (p.0 as usize) < W && (p.1 as usize) < H).then(|| p.1 as usize * W + p.0 as usize)
}

/// Bir hücrenin merkezi, dünyada: harita sol üstten aşağı iniyor, dünya `y`'si yukarı.
fn world_of(p: P) -> Vec3 {
    Vec3::new(p.0 as f32 + 0.5, -(p.1 as f32 + 0.5), 0.5)
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır):
/// resimler `[karolar, kahraman, canavar, altın]`, kamera hazır.
fn setup(world: &mut World, font: FontId, [tiles, hero, monster, coin]: [SpriteImageId; 4], camera: u32) -> Game {
    let map = world
        .spawn_bundle((Transform::new(Vec3::ZERO), GlobalTransform::default(), Tilemap::new(tiles, 3, 1, W as u32, H as u32, Vec2::ONE)))
        .id();
    let player_e = world
        .spawn_bundle((Transform::new(Vec3::ZERO), GlobalTransform::default(), Sprite::world(hero, Vec2::splat(0.9)).with_layer(3)))
        .id();
    world.insert_resource(ScreenFade::default());

    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 40.0)),)).id();
    let banner = world
        .spawn_bundle((Text::pinned_at("", font, 40.0, Vec2::new(0.5, 0.4), Vec2::ZERO).with_anchor(TextAnchor::Center).with_align(TextAlign::Center),))
        .id();
    let hp_bar = world.spawn_bundle(NodeBundle {
        style: Style {
            position_type: PositionType::Absolute,
            inset: UiRect { left: Val::Px(16.0), top: Val::Px(14.0), right: Val::Auto, bottom: Val::Auto },
            width: Val::Px(220.0),
            height: Val::Px(14.0),
            ..Default::default()
        },
        background_color: BackgroundColor(Vec4::new(0.2, 0.05, 0.05, 0.8)),
        ..Default::default()
    });
    world.add_component(hp_bar, ProgressBar::new(1.0).with_fill(Vec4::new(0.85, 0.2, 0.2, 1.0)));

    let mut game = Game {
        cells: vec![Cell::Wall; W * H],
        seen: vec![false; W * H],
        visible: HashSet::new(),
        player: P(0, 0),
        hp: MAX_HP,
        gold: 0,
        depth: 1,
        monsters: Vec::new(),
        coins: Vec::new(),
        rng: Rng::new(7),
        map,
        player_e,
        camera,
        hud,
        hp_bar: hp_bar.id(),
        banner,
        walk: HoldRepeat::new(0.25, 0.11),
        descending: false,
        auto: std::env::var_os("ZINDAN_OTO").is_some(),
        images: Images { monster, coin },
    };
    new_level(world, &mut game);
    game
}

/// `--test`: pencere açmadan otomatik oyun (en yakın düşmana ya da altına, yoksa merdivene),
/// ölene ya da dört dakika dolana dek. Altın toplamalı ve en az bir kat inmeli — iniş ekranın
/// kararmasını bekliyor, yani çizilmeyen bir karede de kararma ilerlemeli. (Otomatik oyuncu kaba:
/// gördüğü her düşmana saldırıyor ve ikinci katta ölüyor; zar tohumlu, sonuç her seferinde aynı.)
fn self_test() -> bool {
    let mut world = World::new();
    let camera = world.spawn_bundle(CameraBundle::default()).id();
    let images = [0, 1, 2, 3].map(SpriteImageId::placeholder);
    let mut game = setup(&mut world, HeadlessFrames::font(), images, camera);
    game.auto = true;
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world).add_plugin(UiPlugin);
    let state = |w: &World| w.get_resource::<Game>().map(|g| (g.depth, g.gold, g.hp)).expect("oyun kaynağı");
    while run.elapsed < 240.0 && state(&run.world).2 > 0 {
        run.frame(1.0 / 60.0, update);
    }
    let (depth, gold, hp) = state(&run.world);
    let ok = depth >= 2 && gold > 0;
    println!(
        "zindan --test: {:.0} s, {}. kata indi, {gold} altın, can {hp}/{MAX_HP} → {}",
        run.elapsed,
        depth,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Zindan", 1280, 720)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, state| {
            let font = load_font(scene.renderer);
            let r = &mut *scene.renderer;
            let tiles = r.load_sprite_rgba(&tileset(), 24, 8, SpriteFilter::Nearest).expect("tileset");
            let hero = r.load_sprite_rgba(&glyph([90, 170, 255]), 8, 8, SpriteFilter::Nearest).expect("hero");
            let monster = r.load_sprite_rgba(&glyph([235, 70, 70]), 8, 8, SpriteFilter::Nearest).expect("monster");
            let coin = r.load_sprite_rgba(&glyph([255, 210, 60]), 8, 8, SpriteFilter::Nearest).expect("coin");

            scene.spawn_camera(state, Vec3::new(W as f32 / 2.0, -(H as f32) / 2.0, 20.0), Vec3::new(W as f32 / 2.0, -(H as f32) / 2.0, 0.0));
            let camera = scene.world.query::<(Entity, &Camera)>().unwrap().iter().next().map(|(entity, _)| entity.id()).expect("camera");
            if let Some(mut c) = scene.world.borrow_mut::<Camera>().get_mut(camera) {
                c.projection = ProjectionMode::OrthographicFit { width: 22.0, height: 14.0 };
                // Hiç görülmeyen yer siyah: harita orada boş, arkası kameranın rengi.
                c.clear_color = Some(Vec3::ZERO);
            }

            let game = setup(scene.world, font, [tiles, hero, monster, coin], camera);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn cell(game: &Game, p: P) -> Cell {
    idx(p).map_or(Cell::Wall, |i| game.cells[i])
}

/// Odalar ve onları sırayla bağlayan L biçimli koridorlar; son odanın ortasında merdiven.
fn new_level(world: &mut World, game: &mut Game) {
    for m in game.monsters.drain(..) {
        world.despawn_by_id(m.entity);
    }
    for (_, e) in game.coins.drain(..) {
        world.despawn_by_id(e);
    }
    game.cells = vec![Cell::Wall; W * H];
    game.seen = vec![false; W * H];
    let mut rooms: Vec<(i32, i32, i32, i32)> = Vec::new();
    for _ in 0..200 {
        if rooms.len() >= 9 {
            break;
        }
        let (w, h) = (game.rng.range(4..10), game.rng.range(3..7));
        let (x, y) = (game.rng.range(1..(W as i32 - w - 1)), game.rng.range(1..(H as i32 - h - 1)));
        if rooms.iter().any(|&(rx, ry, rw, rh)| x <= rx + rw && rx <= x + w && y <= ry + rh && ry <= y + h) {
            continue;
        }
        rooms.push((x, y, w, h));
    }
    let centre = |r: (i32, i32, i32, i32)| P(r.0 + r.2 / 2, r.1 + r.3 / 2);
    for &(x, y, w, h) in &rooms {
        for yy in y..y + h {
            for xx in x..x + w {
                game.cells[idx(P(xx, yy)).unwrap()] = Cell::Floor;
            }
        }
    }
    for pair in rooms.windows(2) {
        let (a, b) = (centre(pair[0]), centre(pair[1]));
        for x in a.0.min(b.0)..=a.0.max(b.0) {
            game.cells[idx(P(x, a.1)).unwrap()] = Cell::Floor;
        }
        for y in a.1.min(b.1)..=a.1.max(b.1) {
            game.cells[idx(P(b.0, y)).unwrap()] = Cell::Floor;
        }
    }
    game.player = centre(rooms[0]);
    let stairs = centre(*rooms.last().unwrap());
    game.cells[idx(stairs).unwrap()] = Cell::Stairs;
    // Düşmanlar ve altınlar: ilk oda dışındaki odalara.
    for &room in &rooms[1..] {
        let n = game.rng.range(0..2 + game.depth.min(3));
        for _ in 0..n {
            let p = P(game.rng.range(room.0..room.0 + room.2), game.rng.range(room.1..room.1 + room.3));
            if p != stairs && !game.monsters.iter().any(|m| m.at == p) {
                let entity = spawn_sprite(world, game.images.monster, p, 2);
                game.monsters.push(Monster { at: p, hp: 2 + game.depth as i32 / 2, entity });
            }
        }
        let p = P(game.rng.range(room.0..room.0 + room.2), game.rng.range(room.1..room.1 + room.3));
        if p != stairs {
            let e = spawn_sprite(world, game.images.coin, p, 1);
            game.coins.push((p, e));
        }
    }
    look(game);
    redraw(world, game);
}

fn spawn_sprite(world: &mut World, image: SpriteImageId, p: P, layer: i32) -> u32 {
    world
        .spawn_bundle((Transform::new(world_of(p)), GlobalTransform::default(), Sprite::world(image, Vec2::splat(0.8)).with_layer(layer)))
        .id()
}

/// Görüş: yarıçap içindeki her hücreye bir doğru çiz; duvar ötesini görmez.
fn look(game: &mut Game) {
    game.visible.clear();
    let o = game.player;
    for dy in -SIGHT..=SIGHT {
        for dx in -SIGHT..=SIGHT {
            if dx * dx + dy * dy > SIGHT * SIGHT {
                continue;
            }
            let target = P(o.0 + dx, o.1 + dy);
            if line_of_sight(game, o, target) {
                game.visible.insert(target);
                if let Some(i) = idx(target) {
                    game.seen[i] = true;
                }
            }
        }
    }
}

fn line_of_sight(game: &Game, a: P, b: P) -> bool {
    let (mut x, mut y) = (a.0, a.1);
    let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
    let (sx, sy) = (if a.0 < b.0 { 1 } else { -1 }, if a.1 < b.1 { 1 } else { -1 });
    let mut err = dx + dy;
    loop {
        if P(x, y) == b {
            return true;
        }
        if P(x, y) != a && cell(game, P(x, y)) == Cell::Wall {
            return false;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// Haritayı ve varlıkları görüşe göre yeniden boya.
fn redraw(world: &mut World, game: &Game) {
    if let Some(mut map) = world.borrow_mut::<Tilemap>().get_mut(game.map) {
        for y in 0..H {
            for x in 0..W {
                let p = P(x as i32, y as i32);
                let i = y * W + x;
                let tile = match game.cells[i] {
                    Cell::Wall => WALL,
                    Cell::Floor => FLOOR,
                    Cell::Stairs => STAIRS,
                };
                // Görülen aydınlık, hatırlanan soluk (karo tonu), hiç görülmeyen boş.
                let (shown, tint) = if game.visible.contains(&p) {
                    (Some(tile), Vec4::ONE)
                } else if game.seen[i] {
                    (Some(tile), REMEMBERED)
                } else {
                    (None, Vec4::ONE)
                };
                map.set(x as u32, y as u32, shown);
                map.set_tint(x as u32, y as u32, tint);
            }
        }
    }
    {
        let mut ts = world.borrow_mut::<Transform>();
        if let Some(mut t) = ts.get_mut(game.player_e) {
            t.position = world_of(game.player);
        }
        for m in &game.monsters {
            if let Some(mut t) = ts.get_mut(m.entity) {
                t.position = world_of(m.at);
            }
        }
    }
    // Düşman yalnız görülürken, altın görüldüyse: `IsHidden` (sprite'lar da ona uyuyor).
    let shows: Vec<(u32, bool)> = game
        .monsters
        .iter()
        .map(|m| (m.entity, game.visible.contains(&m.at)))
        .chain(game.coins.iter().map(|(p, e)| (*e, idx(*p).is_some_and(|i| game.seen[i]))))
        .collect();
    for (id, show) in shows {
        let Some(e) = world.get_entity(id) else { continue };
        let hidden = world.borrow::<IsHidden>().get(id).is_some();
        if show && hidden {
            world.remove_component::<IsHidden>(e);
        } else if !show && !hidden {
            world.add_component(e, IsHidden);
        }
    }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        game.hp = MAX_HP;
        game.gold = 0;
        game.depth = 1;
        new_level(world, &mut game);
    }

    // Merdivenden iniş: ekran kararınca yeni kat, sonra aydınlanıyor.
    if game.descending {
        let dark = world.get_resource::<ScreenFade>().is_some_and(|f| f.just_finished() && f.is_opaque());
        if dark {
            game.depth += 1;
            new_level(world, &mut game);
            game.descending = false;
            if let Some(mut fade) = world.get_resource_mut::<ScreenFade>() {
                fade.fade_in(0.35);
            }
        }
    } else if game.hp > 0 {
        let step = if game.auto {
            // Oto da insan hızında: basılı tutulan bir tuş gibi.
            (game.walk.tick(true, dt) > 0).then(|| auto_step(&game)).flatten()
        } else {
            read_step(&mut game, input, dt)
        };
        if let Some(d) = step {
            act(world, &mut game, d);
        }
    }

    // Kamera oyuncuyu yumuşakça izliyor.
    let target = world_of(game.player);
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.camera) {
        let k = 1.0 - (-dt * 8.0).exp();
        t.position.x += (target.x - t.position.x) * k;
        t.position.y += (target.y - t.position.y) * k;
    }
    draw_hud(world, &game);
    world.insert_resource(game);
}

/// Yön: `move_axis` (WASD, oklar, sol çubuk); basılı tutulursa `HoldRepeat` tekrar ettiriyor.
fn read_step(game: &mut Game, input: &Input, dt: f32) -> Option<P> {
    let (mx, my) = input.move_axis();
    let pad = input.gamepad().map(|p| p.dpad()).unwrap_or((0.0, 0.0));
    let (mx, my) = if mx.abs() + my.abs() > 0.3 { (mx, my) } else { pad };
    let held = mx.abs() > 0.3 || my.abs() > 0.3;
    if game.walk.tick(held, dt) == 0 {
        return None;
    }
    // Baskın eksen; ekranda yukarı = haritada y−1.
    Some(if mx.abs() >= my.abs() { P(mx.signum() as i32, 0) } else { P(0, -(my.signum() as i32)) })
}

/// Bir sıra: yürü ya da saldır, sonra düşmanlar.
fn act(world: &mut World, game: &mut Game, d: P) {
    let to = P(game.player.0 + d.0, game.player.1 + d.1);
    if let Some(i) = game.monsters.iter().position(|m| m.at == to) {
        game.monsters[i].hp -= 1;
        if game.monsters[i].hp <= 0 {
            let m = game.monsters.remove(i);
            world.despawn_by_id(m.entity);
        }
    } else if cell(game, to) != Cell::Wall {
        game.player = to;
        if let Some(i) = game.coins.iter().position(|(p, _)| *p == to) {
            let (_, e) = game.coins.remove(i);
            world.despawn_by_id(e);
            game.gold += 1;
        }
        if cell(game, to) == Cell::Stairs {
            game.descending = true;
            if let Some(mut fade) = world.get_resource_mut::<ScreenFade>() {
                fade.set_color(Vec3::ZERO);
                fade.fade_out(0.35);
            }
        }
    } else {
        return; // duvara yürümek bir sıra harcamıyor
    }
    look(game);
    monsters_turn(world, game);
    redraw(world, game);
}

/// Oyuncuyu gören düşman ona bir adım yaklaşıyor; yanındaysa vuruyor.
fn monsters_turn(world: &mut World, game: &mut Game) {
    let me = game.player;
    for i in 0..game.monsters.len() {
        let at = game.monsters[i].at;
        if (at.0 - me.0).abs() + (at.1 - me.1).abs() == 1 {
            game.hp -= 1;
            if let Some(mut fade) = world.get_resource_mut::<ScreenFade>() {
                fade.flash(Vec3::new(0.8, 0.05, 0.05), 0.3, 0.2);
            }
            continue;
        }
        if !game.visible.contains(&at) {
            continue;
        }
        let options = [P(1, 0), P(-1, 0), P(0, 1), P(0, -1)];
        let best = options
            .iter()
            .map(|d| P(at.0 + d.0, at.1 + d.1))
            .filter(|p| cell(game, *p) != Cell::Wall && *p != me && !game.monsters.iter().any(|m| m.at == *p))
            .min_by_key(|p| (p.0 - me.0).abs() + (p.1 - me.1).abs());
        if let Some(p) = best {
            if (p.0 - me.0).abs() + (p.1 - me.1).abs() < (at.0 - me.0).abs() + (at.1 - me.1).abs() {
                game.monsters[i].at = p;
            }
        }
    }
    game.hp = game.hp.max(0);
}

/// Oto: önce görülen düşmana, sonra en yakın bilinen altına, yoksa merdivene — genişlik öncelikli.
fn auto_step(game: &Game) -> Option<P> {
    let goals: HashSet<P> = if let Some(m) = game.monsters.iter().find(|m| game.visible.contains(&m.at)) {
        [m.at].into()
    } else if !game.coins.is_empty() {
        game.coins.iter().map(|(p, _)| *p).collect()
    } else {
        (0..W * H).filter(|&i| game.cells[i] == Cell::Stairs).map(|i| P((i % W) as i32, (i / W) as i32)).collect()
    };
    let mut prev = vec![None; W * H];
    let mut queue = VecDeque::from([game.player]);
    prev[idx(game.player)?] = Some(game.player);
    while let Some(p) = queue.pop_front() {
        if goals.contains(&p) {
            let mut step = p;
            while let Some(back) = prev[idx(step)?] {
                if back == game.player {
                    return Some(P(step.0 - back.0, step.1 - back.1));
                }
                step = back;
            }
            return None;
        }
        for d in [P(1, 0), P(-1, 0), P(0, 1), P(0, -1)] {
            let n = P(p.0 + d.0, p.1 + d.1);
            if let Some(i) = idx(n) {
                if prev[i].is_none() && game.cells[i] != Cell::Wall {
                    prev[i] = Some(p);
                    queue.push_back(n);
                }
            }
        }
    }
    None
}

fn draw_hud(world: &mut World, game: &Game) {
    if let Some(mut bar) = world.borrow_mut::<ProgressBar>().get_mut(game.hp_bar) {
        bar.value = game.hp as f32 / MAX_HP as f32;
    }
    let mut texts = world.borrow_mut::<Text>();
    if let Some(mut t) = texts.get_mut(game.hud) {
        t.content = format!("Kat {}   Altın {}   Can {}/{}   Düşman {}", game.depth, game.gold, game.hp, MAX_HP, game.monsters.len());
    }
    if let Some(mut t) = texts.get_mut(game.banner) {
        t.content = if game.hp == 0 { format!("Öldün — kat {}, {} altın\nR: yeni zindan", game.depth, game.gold) } else { String::new() };
    }
}

/// 3 hücrelik tileset, 8×8: zemin, duvar, merdiven. Soluk halleri karo tonuyla (`set_tint`).
fn tileset() -> Vec<u8> {
    let mut px = vec![0u8; 24 * 8 * 4];
    for cell in 0..3 {
        for y in 0..8 {
            for x in 0..8 {
                let c: [u8; 3] = match cell {
                    0 => if (x + y) % 4 == 0 { [70, 64, 58] } else { [52, 48, 44] },
                    1 => if y == 0 || x == 0 { [120, 110, 100] } else { [92, 84, 76] },
                    _ => if x > y { [200, 170, 60] } else { [52, 48, 44] },
                };
                let i = ((y * 24) + cell * 8 + x) * 4;
                px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
    }
    px
}

/// 8×8 bir yuvarlak, verilen renkte.
fn glyph(rgb: [u8; 3]) -> Vec<u8> {
    let mut px = vec![0u8; 8 * 8 * 4];
    for y in 0..8 {
        for x in 0..8 {
            let (dx, dy) = (x as f32 - 3.5, y as f32 - 3.5);
            if dx * dx + dy * dy <= 12.0 {
                let i = (y * 8 + x) * 4;
                px[i..i + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
    }
    px
}

fn load_font(renderer: &mut Renderer) -> FontId {
    // A font the machine has (`GIZMO_FONT`, or the platform's usual faces); without one, the
    // synthetic test face, so the game still runs.
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
