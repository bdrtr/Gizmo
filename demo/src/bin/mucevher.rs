//! # Mücevher — bir eşleştirme bulmacası
//!
//! 8×8 bir tahta: komşu iki taşı yer değiştir, yatayda ya da dikeyde üç (ya da daha çok) aynı
//! taş dizilirse patlarlar, üsttekiler düşer, yukarıdan yenileri gelir — ve düşenler yeni
//! diziler kurarsa zincir. Doksan saniye. Motorun sekizinci oyunu; her şeyi **sırayla oynayan
//! animasyonlar** üzerine kurulu: değiştir → (eşleşme yoksa geri al) → patlat → düşür → doldur →
//! yeniden bak.
//!
//! | girdi | iş |
//! |---|---|
//! | **sürükle** (fare ya da parmak) | taşı komşusuyla değiştir |
//! | **tıkla, sonra komşuya tıkla** | aynı |
//! | **R** | yeniden başla |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Animasyon bekletilemiyordu.** Yeni taşlar sütun sütun, birer an arayla düşsün diye her
//!   `Tween`'in biraz geç başlaması gerekiyordu; tek yol başlamayı elle saymaktı. **Kapatıldı:**
//!   `Tween::with_delay` (ve `ColorTween::with_delay`): bekliyor, **başladığı an** varlığın
//!   neredeyse oradan başlıyor (eklendiği an değil — bekleme sırasında başka biri taşımış
//!   olabilir), beklemenin taştığı kare payını kendine harcıyor.
//!
//! Sırayla oynayan aşamalar (değiştir → geri al ya da patlat → düşür → yeniden bak) için başka
//! bir şey gerekmedi: her aşama `Tween::is_finished` bitene kadar bekleyen küçük bir durum
//! makinesi. Sürükleyerek değiştirme `gizmo::pointer::cursor_on_plane` ile; dokunmatikte de
//! aynen çalışıyor, ilk parmak fare olduğu için.
//!
//! ## Ortam değişkenleri
//!   * `MUCEVHER_OTO=1` — kendi kendine oynar (ekran görüntüsü için; `mucevher --test` pencere
//!     açmadan bir tur oynar). Otomatik oyunun puanı rekor sayılmaz.
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::app::save::SaveData;
use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::core::input::Input;
use gizmo::prelude::*;
use gizmo::renderer::components::{ParticleEmitter, ProjectionMode, Text, TextAlign, TextAnchor};
use gizmo::renderer::screen_fade::ScreenFade;
use gizmo::renderer::text::FontId;
use gizmo::renderer::tween::{Ease, Tween};
use gizmo::simple::{HeadlessFrames, SimpleAppExt, SimpleSceneState};

const N: usize = 8;
const KINDS: u8 = 6;
const ROUND: f32 = 90.0;

/// Altı taş: biçim ve renk birlikte değişiyor, renk körü bir oyuncu da biçimden ayırsın.
const SHAPES: [&str; KINDS as usize] = ["standard_cube", "sphere", "cone", "cylinder", "standard_cube", "sphere"];
const COLOURS: [Vec4; KINDS as usize] = [
    Vec4::new(0.9, 0.2, 0.25, 1.0),
    Vec4::new(0.2, 0.75, 0.3, 1.0),
    Vec4::new(0.25, 0.45, 0.95, 1.0),
    Vec4::new(0.95, 0.8, 0.2, 1.0),
    Vec4::new(0.7, 0.3, 0.9, 1.0),
    Vec4::new(0.95, 0.55, 0.15, 1.0),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Idle,
    /// İki taş yer değiştiriyor; `back` geri dönüşse.
    Swapping { a: (usize, usize), b: (usize, usize), back: bool },
    Clearing,
    Falling,
    Over,
}

#[derive(Clone, Copy)]
struct Gem;
gizmo::core::impl_component!(Gem);

struct Game {
    /// `cells[x][y]`: taşın türü ve varlığı; `y = 0` en alt satır.
    cells: [[Option<(u8, u32)>; N]; N],
    phase: Phase,
    selected: Option<(usize, usize)>,
    pressed: Option<(usize, usize)>,
    rng: Rng,
    score: u32,
    best: u32,
    combo: u32,
    clock: Timer,
    hud: u32,
    banner: u32,
    auto: bool,
}

fn cell_pos(x: usize, y: usize) -> Vec3 {
    Vec3::new(x as f32 - (N as f32 - 1.0) / 2.0, y as f32 - (N as f32 - 1.0) / 2.0, 0.0)
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
/// `sound` kapalıyken ses aygıtı açılmaz.
fn setup(world: &mut World, font: FontId, sound: bool) -> Game {
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-0.6) * Quat::from_rotation_y(0.4)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::ONE, 3.0, LightRole::Sun),
    ));
    // Tahtanın zemini.
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 0.0, -0.8)).with_scale(Vec3::new(N as f32 / 2.0 + 0.2, N as f32 / 2.0 + 0.2, 0.1)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.08, 0.09, 0.12, 1.0), 0.9, 0.0),
        MeshRenderer::new(),
    ));
    world.insert_resource(ScreenFade::default());

    let hud = world
        .spawn_bundle((Text::pinned("", font, 24.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
        .id();
    let banner = world
        .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.45), Vec2::ZERO)
            .with_anchor(TextAnchor::Center)
            .with_align(TextAlign::Center),))
        .id();

    if let Some(mut audio) = sound.then(AudioManager::new).and_then(Result::ok) {
        audio.load_sound_bytes("tak", Blip::new(Wave::Triangle, 520.0, 0.06).with_volume(0.3).wav());
        audio.load_sound_bytes("pat", Blip::new(Wave::Sine, 880.0, 0.12).with_slide(1400.0).with_volume(0.3).wav());
        audio.load_sound_bytes("yok", Blip::new(Wave::Square, 200.0, 0.1).with_slide(150.0).with_volume(0.15).wav());
        world.insert_resource(audio);
    }

    let best = SaveData::open("mucevher").get::<u32>("best").unwrap_or(0);
    let mut game = Game {
        cells: [[None; N]; N],
        phase: Phase::Idle,
        selected: None,
        pressed: None,
        rng: Rng::new(42),
        score: 0,
        best,
        combo: 1,
        clock: Timer::once(ROUND),
        hud,
        banner,
        auto: std::env::var_os("MUCEVHER_OTO").is_some(),
    };
    fill_board(world, &mut game);
    game
}

/// `--test`: pencere açmadan bir tur otomatik oyun. Puan toplanmalı; tur bitip taşlar durulunca
/// tahtanın her karesinde bir taş olmalı, her taş canlı ve kendi karesinde durmalı (taşları
/// tween'ler yürütüyor — çizilmeyen karede de ilerlemeleri gerek).
fn self_test() -> bool {
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font(), false);
    game.auto = true;
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let over = |w: &World| w.get_resource::<Game>().is_some_and(|g| g.phase == Phase::Over);
    while run.elapsed < ROUND + 30.0 && !(over(&run.world) && !animating(&run.world)) {
        run.frame(1.0 / 60.0, update);
    }
    let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
    let (mut filled, mut placed) = (0, 0);
    for x in 0..N {
        for y in 0..N {
            if let Some((_, id)) = game.cells[x][y] {
                filled += 1;
                let at = run.world.borrow::<Transform>().get(id).map(|t| t.position);
                if at.is_some_and(|p| (p - cell_pos(x, y)).length() < 0.01) {
                    placed += 1;
                }
            }
        }
    }
    let ok = game.score > 0 && filled == N * N && placed == N * N;
    println!(
        "mucevher --test: {:.0} s'lik tur, {} puan; tahtada {filled}/{} taş, {placed}'i kendi karesinde → {}",
        ROUND,
        game.score,
        N * N,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Mücevher", 1280, 720)
        .with_simple_scene(|scene, state| {
            let font = load_font(scene.renderer);
            scene.spawn_camera(state, Vec3::new(0.0, 0.0, 20.0), Vec3::ZERO);
            if let Some(mut q) = scene.world.query_mut::<Mut<Camera>>() {
                for mut cam in q.iter_mut() {
                    cam.projection = ProjectionMode::OrthographicFit { width: 10.0, height: 10.0 };
                }
            }
            let game = setup(scene.world, font, true);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Eşleşmesiz yeni bir tahta.
fn fill_board(world: &mut World, game: &mut Game) {
    for x in 0..N {
        for y in 0..N {
            if let Some((_, id)) = game.cells[x][y].take() {
                world.despawn_by_id(id);
            }
        }
    }
    for x in 0..N {
        for y in 0..N {
            let kind = loop {
                let k = game.rng.range(0..KINDS as u32) as u8;
                let row = x >= 2 && kind_at(game, x - 1, y) == Some(k) && kind_at(game, x - 2, y) == Some(k);
                let col = y >= 2 && kind_at(game, x, y - 1) == Some(k) && kind_at(game, x, y - 2) == Some(k);
                if !row && !col {
                    break k;
                }
            };
            let id = spawn_gem(world, kind, cell_pos(x, y));
            game.cells[x][y] = Some((kind, id));
        }
    }
}

fn kind_at(game: &Game, x: usize, y: usize) -> Option<u8> {
    game.cells[x][y].map(|(k, _)| k)
}

fn spawn_gem(world: &mut World, kind: u8, at: Vec3) -> u32 {
    let colour = COLOURS[kind as usize];
    world
        .spawn_bundle((
            Transform::new(at).with_scale(Vec3::splat(0.36)).with_rotation(Quat::from_rotation_x(0.5) * Quat::from_rotation_y(0.6)),
            GlobalTransform::default(),
            MeshSource(SHAPES[kind as usize].into()),
            MaterialDesc::new().with_pbr(colour, 0.35, 0.1).with_emissive(colour.truncate() * 0.15),
            MeshRenderer::new(),
            Gem,
        ))
        .id()
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        restart(world, &mut game);
    }
    if game.phase != Phase::Over && game.clock.tick(dt) > 0 {
        game.phase = Phase::Over;
        game.best = game.best.max(game.score);
        // Otomatik oyunun puanı rekor sayılmaz (ve sınama oyuncunun kaydına yazmaz).
        if !game.auto {
            let mut save = SaveData::open("mucevher");
            let _ = save.set("best", &game.best).and_then(|()| save.save());
        }
        if let Some(mut fade) = world.get_resource_mut::<ScreenFade>() {
            fade.fade_to(0.55, 0.6);
        }
    }
    // Her aşama, taşların animasyonları bitene kadar bekliyor.
    if !animating(world) {
        step(world, &mut game, input);
    }
    draw_hud(world, &game);
    world.insert_resource(game);
}

/// Bitmemiş bir `Tween` var mı.
fn animating(world: &World) -> bool {
    world.borrow::<Tween>().iter().any(|t| !t.is_finished())
}

fn step(world: &mut World, game: &mut Game, input: &Input) {
    match game.phase {
        Phase::Idle => {
            if let Some((a, b)) = if game.auto { auto_move(game) } else { read_swap(world, game, input) } {
                start_swap(world, game, a, b, false);
            }
        }
        Phase::Swapping { a, b, back } => {
            if back {
                game.phase = Phase::Idle;
            } else if matches(game).is_empty() {
                // Eşleşme yok: geri al.
                play(world, "yok");
                start_swap(world, game, a, b, true);
            } else {
                game.combo = 1;
                clear(world, game);
            }
        }
        Phase::Clearing => fall(world, game),
        Phase::Falling => {
            if matches(game).is_empty() {
                game.phase = Phase::Idle;
                if auto_move(game).is_none() {
                    // Hamle kalmadı: yeni tahta.
                    fill_board(world, game);
                }
            } else {
                game.combo += 1;
                clear(world, game);
            }
        }
        Phase::Over => {}
    }
}

/// Fareyle (ya da parmakla) bir değiştirme: bir taşa basıp komşusuna sürüklemek, ya da seçili bir
/// taştan sonra komşusuna tıklamak.
fn read_swap(world: &mut World, game: &mut Game, input: &Input) -> Option<((usize, usize), (usize, usize))> {
    let cell = cursor_cell(world);
    if input.is_mouse_button_just_pressed(0) {
        if let (Some(sel), Some(c)) = (game.selected, cell) {
            if adjacent(sel, c) {
                set_selected(world, game, None);
                return Some((sel, c));
            }
        }
        game.pressed = cell;
        set_selected(world, game, cell);
    } else if input.is_mouse_button_pressed(0) {
        if let (Some(p), Some(c)) = (game.pressed, cell) {
            if adjacent(p, c) {
                game.pressed = None;
                set_selected(world, game, None);
                return Some((p, c));
            }
        }
    } else {
        game.pressed = None;
    }
    None
}

fn cursor_cell(world: &World) -> Option<(usize, usize)> {
    let p = gizmo::pointer::cursor_on_plane(world, Vec3::ZERO, Vec3::Z)?;
    let h = N as f32 / 2.0;
    let (x, y) = ((p.x + h).floor(), (p.y + h).floor());
    (x >= 0.0 && y >= 0.0 && x < N as f32 && y < N as f32).then_some((x as usize, y as usize))
}

fn adjacent(a: (usize, usize), b: (usize, usize)) -> bool {
    a.0.abs_diff(b.0) + a.1.abs_diff(b.1) == 1
}

/// Seçili taş biraz büyük.
fn set_selected(world: &mut World, game: &mut Game, cell: Option<(usize, usize)>) {
    for (c, s) in [(game.selected, 0.36), (cell, 0.46)] {
        if let Some((_, id)) = c.and_then(|(x, y)| game.cells[x][y]) {
            if let Some(e) = world.get_entity(id) {
                world.add_component(e, Tween::to_scale(Vec3::splat(s), 0.08));
            }
        }
    }
    game.selected = cell;
}

fn start_swap(world: &mut World, game: &mut Game, a: (usize, usize), b: (usize, usize), back: bool) {
    let (ga, gb) = (game.cells[a.0][a.1], game.cells[b.0][b.1]);
    game.cells[a.0][a.1] = gb;
    game.cells[b.0][b.1] = ga;
    for (cell, gem) in [(b, ga), (a, gb)] {
        if let Some((_, id)) = gem {
            if let Some(e) = world.get_entity(id) {
                world.add_component(e, Tween::to_position(cell_pos(cell.0, cell.1), 0.16).with_ease(Ease::InOutQuad).and_scale(Vec3::splat(0.36)));
            }
        }
    }
    play(world, "tak");
    game.phase = Phase::Swapping { a, b, back };
}

/// Üç ya da daha uzun dizilerdeki hücreler — her satır ve her sütun aynı taramadan geçiyor.
fn matches(game: &Game) -> Vec<(usize, usize)> {
    let rows = (0..N).map(|y| (0..N).map(|x| (x, y)).collect::<Vec<_>>());
    let columns = (0..N).map(|x| (0..N).map(|y| (x, y)).collect::<Vec<_>>());
    let mut hit = Vec::new();
    for line in rows.chain(columns) {
        let kind = |c: (usize, usize)| kind_at(game, c.0, c.1);
        let mut start = 0;
        for end in 1..=line.len() {
            let same = end < line.len() && kind(line[end]).is_some() && kind(line[end]) == kind(line[start]);
            if !same {
                if end - start >= 3 {
                    hit.extend_from_slice(&line[start..end]);
                }
                start = end;
            }
        }
    }
    hit.sort_unstable();
    hit.dedup();
    hit
}

/// Eşleşenleri küçültüp kıvılcım saçarak boşalt.
fn clear(world: &mut World, game: &mut Game) {
    let cells = matches(game);
    game.score += cells.len() as u32 * 10 * game.combo;
    for (x, y) in cells {
        if let Some((kind, id)) = game.cells[x][y].take() {
            if let Some(e) = world.get_entity(id) {
                world.add_component(e, Tween::to_scale(Vec3::ZERO, 0.14).with_ease(Ease::InQuad));
                world.add_component(e, Dying);
            }
            sparkle(world, cell_pos(x, y), COLOURS[kind as usize]);
        }
    }
    play(world, "pat");
    game.phase = Phase::Clearing;
}

#[derive(Clone, Copy)]
struct Dying;
gizmo::core::impl_component!(Dying);

/// Küçülenleri sil, üsttekileri düşür, boşlukları yukarıdan doldur.
fn fall(world: &mut World, game: &mut Game) {
    world.despawn_all_with::<Dying>();
    for x in 0..N {
        let mut write = 0;
        for y in 0..N {
            if let Some(g) = game.cells[x][y].take() {
                game.cells[x][write] = Some(g);
                if write != y {
                    move_to(world, g.1, cell_pos(x, write), y - write, x);
                }
                write += 1;
            }
        }
        // Yenileri tahtanın üstünden düşür.
        for (i, y) in (write..N).enumerate() {
            let kind = game.rng.range(0..KINDS as u32) as u8;
            let above = cell_pos(x, N + i);
            let id = spawn_gem(world, kind, above);
            game.cells[x][y] = Some((kind, id));
            move_to(world, id, cell_pos(x, y), N + i - y, x);
        }
    }
    game.phase = Phase::Falling;
}

/// Düşüş: uzağa düşen uzun sürüyor, ve sütunlar soldan sağa birer an arayla başlıyor — tahta
/// dalga gibi doluyor.
fn move_to(world: &mut World, id: u32, to: Vec3, rows: usize, column: usize) {
    if let Some(e) = world.get_entity(id) {
        world.add_component(
            e,
            Tween::to_position(to, 0.1 + rows as f32 * 0.05).with_ease(Ease::OutBounce).with_delay(column as f32 * 0.03),
        );
    }
}

/// Eşleşme kuran ilk hamle — oto mod ve "hamle kaldı mı" sorusu için.
fn auto_move(game: &Game) -> Option<((usize, usize), (usize, usize))> {
    let mut g = Game { cells: game.cells, ..placeholder() };
    for x in 0..N {
        for y in 0..N {
            for (dx, dy) in [(1, 0), (0, 1)] {
                let (bx, by) = (x + dx, y + dy);
                if bx >= N || by >= N {
                    continue;
                }
                let t = g.cells[x][y];
                g.cells[x][y] = g.cells[bx][by];
                g.cells[bx][by] = t;
                let ok = !matches(&g).is_empty();
                g.cells[bx][by] = g.cells[x][y];
                g.cells[x][y] = t;
                if ok {
                    return Some(((x, y), (bx, by)));
                }
            }
        }
    }
    None
}

fn placeholder() -> Game {
    Game {
        cells: [[None; N]; N],
        phase: Phase::Idle,
        selected: None,
        pressed: None,
        rng: Rng::new(0),
        score: 0,
        best: 0,
        combo: 1,
        clock: Timer::once(ROUND),
        hud: 0,
        banner: 0,
        auto: false,
    }
}

fn restart(world: &mut World, game: &mut Game) {
    world.despawn_all_with::<Dying>();
    fill_board(world, game);
    game.phase = Phase::Idle;
    game.score = 0;
    game.combo = 1;
    game.selected = None;
    game.clock = Timer::once(ROUND);
    if let Some(mut fade) = world.get_resource_mut::<ScreenFade>() {
        fade.fade_in(0.3);
    }
}

fn sparkle(world: &mut World, at: Vec3, colour: Vec4) {
    let mut burst = ParticleEmitter::one_shot(18);
    burst.initial_velocity = Vec3::ZERO;
    burst.velocity_randomness = 3.0;
    burst.lifespan = 0.4;
    burst.size_start = 0.12;
    burst.size_end = 0.0;
    burst.color_start = colour;
    burst.color_end = colour.truncate().extend(0.0);
    world.spawn_bundle((Transform::new(at + Vec3::Z * 0.5), GlobalTransform::default(), burst, Dying));
}

fn play(world: &mut World, name: &str) {
    if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
        let _ = audio.play(name);
    }
}

fn draw_hud(world: &mut World, game: &Game) {
    let mut texts = world.borrow_mut::<Text>();
    if let Some(mut t) = texts.get_mut(game.hud) {
        let combo = if game.combo > 1 { format!("   ×{}", game.combo) } else { String::new() };
        t.content = format!(
            "Skor {}{}   Süre {:.0}   En iyi {}",
            game.score,
            combo,
            game.clock.remaining().ceil(),
            game.best.max(game.score)
        );
    }
    if let Some(mut t) = texts.get_mut(game.banner) {
        t.content = if game.phase == Phase::Over { format!("Süre doldu — {} puan\nR: yeniden", game.score) } else { String::new() };
    }
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
