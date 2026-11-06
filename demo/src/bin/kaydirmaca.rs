//! # Kaydırmaca — dörde dört kaydırmalı karo bulmacası
//!
//! On beş numaralı karo, bir boşluk. Boşluğun yanındaki bir karoya tıkla, boşluğa kaysın.
//! Karoları 1'den 15'e sıraya diz. Karıştırma rastgele ama her zaman çözülebilir: geçerli
//! hamlelerden yapılıyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **sol tık** | karo kaydır |
//! | **R** | yeniden karıştır |
//!
//! ## Neden var
//!
//! Hareket eden şeylerin **yumuşakça** hareket etmesi gereken bir oyun. Motorda bunun yolu
//! yoktu: ne tween ne easing. Bu oyun ilk kullanıcısı.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Tween yoktu.** Bir karoyu 0.15 saniyede yavaşlayarak kaydırmak için oyun kendi
//!   interpolasyonunu, süresini ve eğrisini tutacaktı. **Kapatıldı:** `Tween` + `Ease` —
//!   `Tween::to_position(hedef, 0.15).with_ease(Ease::OutCubic)` karoya takılıyor, motorun kare
//!   sürücüsü animasyon saatinde ilerletiyor (duraklatma ve zaman ölçeği geçerli), iki çizim
//!   yolunda da. Başlangıç noktası karonun o anki yeri, yani kayan bir karoya yeni bir tween
//!   takmak sıçratmıyor.
//!
//! ## Ortam değişkenleri
//!   * `GIZMO_FONT=<yol>` — font
//!   * `KAYDIRMACA_OTO=1` — birkaç hamleyi kendiliğinden yap (ekran görüntüsü için; `kaydirmaca
//!     --test` pencere açmadan hamle kurallarını ve çözülme sınamasını dener)

use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, sync_transforms, SimpleAppExt, SimpleSceneState};
use gizmo::winit::keyboard::KeyCode;

const N: usize = 4;
const CELL: f32 = 1.05;
const TILE_Y: f32 = 0.1;

#[derive(Clone, Copy)]
struct Tile;
gizmo::core::impl_component!(Tile);

struct Game {
    /// `board[row * N + col]` = o hücredeki karonun numarası (1..15) ya da 0 (boşluk).
    board: [u8; N * N],
    /// Numara → karonun varlığı.
    tiles: [Entity; N * N],
    moves: u32,
    solved: bool,
    rng: Rng,
    hud: Entity,
    banner: Entity,
    auto: u32,
}

fn cell_position(index: usize) -> Vec3 {
    let (row, col) = (index / N, index % N);
    let offset = (N as f32 - 1.0) * 0.5;
    Vec3::new((col as f32 - offset) * CELL, TILE_Y, (row as f32 - offset) * CELL)
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
/// `sound` kapalıyken ses aygıtı açılmaz.
fn setup(world: &mut World, font: FontId, sound: bool) -> Game {

    // Tabla.
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.05, 0.0)).with_scale(Vec3::new(2.3, 0.05, 2.3)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.18, 0.2, 0.26, 1.0), 0.8, 0.0),
        MeshRenderer::new(),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-1.0)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 6.0, 3.6),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -1.03,
        primary: true,
        ..Default::default()
    });

    // On beş karo, her birinin üstünde numarası.
    let mut tiles = [Entity::new(0, 0); N * N];
    for (n, slot) in tiles.iter_mut().enumerate().skip(1) {
        let hue = n as f32 / 15.0;
        let tile = world.spawn_bundle((
            Transform::new(cell_position(n - 1)).with_scale(Vec3::new(0.48, 0.1, 0.48)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(Vec4::new(0.9 - hue * 0.5, 0.45 + hue * 0.3, 0.3 + hue * 0.5, 1.0), 0.5, 0.0),
            MeshRenderer::new(),
            Tile,
        ));
        // Numara: karonun bir çocuğu, üst yüzünün biraz üstünde (yerel ölçek karonun ölçeğini
        // geri alıyor ki yazı ezilmesin).
        let label = world.spawn_bundle((
            Transform::new(Vec3::new(0.0, 3.2, 0.0)).with_scale(Vec3::new(1.0 / 0.48, 10.0, 1.0 / 0.48)),
            GlobalTransform::default(),
            Text::world(n.to_string(), font, 64.0, 110.0).with_color(Vec4::new(1.0, 1.0, 1.0, 1.0)),
        ));
        world.add_child(tile, label);
        *slot = tile;
    }

    let hud = world
        .spawn_bundle((Text::pinned("", font, 24.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
        ;
    let banner = world
        .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.12), Vec2::ZERO)
            .with_anchor(TextAnchor::Center)
            .with_align(TextAlign::Center),))
        ;

    if let Some(mut audio) = sound.then(AudioManager::new).and_then(Result::ok) {
        audio.load_sound_bytes("kay", Blip::new(Wave::Triangle, 260.0, 0.07).with_slide(200.0).with_volume(0.35).wav());
        audio.load_sound_bytes("cozuldu", Blip::new(Wave::Square, 523.0, 0.6).with_slide(1046.0).with_volume(0.25).wav());
        world.insert_resource(audio);
    }

    let mut game = Game {
        board: [0; N * N],
        tiles,
        moves: 0,
        solved: false,
        rng: Rng::new(2026),
        hud,
        banner,
        auto: if std::env::var_os("KAYDIRMACA_OTO").is_some() { 3 } else { 0 },
    };
    shuffle(world, &mut game);
    game
}

/// Dörde dörtlük bir dizilişin çözülebilirliği: ters sıralı çiftlerin sayısı ile boşluğun alttan
/// satır numarasının toplamı tek olmalı.
fn solvable(board: &[u8; N * N]) -> bool {
    let tiles: Vec<u8> = board.iter().copied().filter(|&v| v != 0).collect();
    let inversions = (0..tiles.len()).map(|i| tiles[i + 1..].iter().filter(|&&t| t < tiles[i]).count()).sum::<usize>();
    let blank_row_from_bottom = N - board.iter().position(|&v| v == 0).expect("boşluk") / N;
    (inversions + blank_row_from_bottom) % 2 == 1
}

/// `--test`: pencere açmadan dört sınama. Karıştırılan tahta çözülebilir olmalı; boşluğa komşu
/// olmayan bir kareye tıklamak hiçbir şey yapmamalı; komşu karo kaymalı ve kayması bitince
/// boşluğun eski yerinde durmalı; çözülmüş tahtada bir karoyu kaydırıp geri almak "çözüldü" demeli.
fn self_test() -> bool {
    let mut world = World::new();
    let game = setup(&mut world, HeadlessFrames::font(), false);
    let shuffled_ok = solvable(&game.board) && game.board != solved_board();
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let settle = |run: &mut HeadlessFrames| {
        for _ in 0..30 {
            run.frame(1.0 / 60.0, update);
        }
    };
    let with_game = |run: &mut HeadlessFrames, f: &mut dyn FnMut(&mut World, &mut Game)| {
        let mut game = run.world.remove_resource::<Game>().expect("oyun kaynağı");
        f(&mut run.world, &mut game);
        run.world.insert_resource(game);
    };
    // Boşluğa komşu olmayan ve komşu olan birer kare.
    let (mut far_ignored, mut slid_home) = (false, false);
    with_game(&mut run, &mut |world, game| {
        let empty = game.board.iter().position(|&v| v == 0).expect("boşluk");
        let far = (0..N * N).find(|c| *c != empty && !neighbours(empty).contains(c)).expect("uzak kare");
        let before = (game.board, game.moves);
        try_move(world, game, far);
        far_ignored = (game.board, game.moves) == before;
        let near = neighbours(empty)[0];
        let tile = game.board[near] as usize;
        try_move(world, game, near);
        slid_home = game.board[empty] as usize == tile && game.moves == 1;
    });
    settle(&mut run);
    let landed = {
        let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
        // Her karo, tahtada durduğu hücrenin yerinde çiziliyor mu.
        (0..N * N).filter(|&c| game.board[c] != 0).all(|c| {
            run.world.borrow::<Transform>().get(game.tiles[game.board[c] as usize]).is_some_and(|t| (t.position - cell_position(c)).length() < 0.01)
        })
    };
    // Çözülmüş tahta: son karo boşluğa kayar, geri gelir.
    let mut solved_after = false;
    with_game(&mut run, &mut |world, game| {
        game.board = solved_board();
        game.solved = false;
        try_move(world, game, N * N - 2);
        let unsolved = !game.solved;
        try_move(world, game, N * N - 1);
        solved_after = unsolved && game.solved;
    });
    let ok = shuffled_ok && far_ignored && slid_home && landed && solved_after;
    println!(
        "kaydirmaca --test: karışık tahta çözülebilir {shuffled_ok}; uzak kare yok sayıldı {far_ignored}; komşu karo kaydı {slid_home}, \
         kayma bitince her karo yerinde {landed}; kaydırıp geri almak çözdü {solved_after} → {}",
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Kaydirmaca", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let r = &mut *scene.renderer;
            r.deband_dither = true;
            let font = load_font(r);
            let game = setup(scene.world, font, true);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn solved_board() -> [u8; N * N] {
    let mut b = [0u8; N * N];
    for (i, v) in b.iter_mut().enumerate().take(N * N - 1) {
        *v = i as u8 + 1;
    }
    b
}

/// Çözülmüş dizilişten 200 rastgele geçerli hamle — çözülebilirliği garanti eden karıştırma.
/// Karolar yerlerine animasyonsuz konuyor.
fn shuffle(world: &mut World, game: &mut Game) {
    game.board = solved_board();
    let mut empty = N * N - 1;
    let mut last = usize::MAX;
    for _ in 0..200 {
        let options: Vec<usize> = neighbours(empty).into_iter().filter(|&c| c != last).collect();
        let pick = *game.rng.pick(&options).expect("boşluğun her zaman komşusu var");
        game.board.swap(empty, pick);
        last = empty;
        empty = pick;
    }
    game.moves = 0;
    game.solved = false;
    let mut tfs = world.borrow_mut::<Transform>();
    for (cell, &n) in game.board.iter().enumerate() {
        if n != 0 {
            if let Some(mut t) = tfs.get_mut(game.tiles[n as usize]) {
                t.position = cell_position(cell);
            }
        }
    }
    drop(tfs);
    // Yarım kalmış bir kayma yeni dizilişin üstüne yazmasın: tween'leri (karoları değil) kaldır.
    for &id in &game.tiles[1..] {
        if let Some(e) = world.live(id) {
            world.remove_component::<Tween>(e);
        }
    }
}

fn neighbours(cell: usize) -> Vec<usize> {
    let (row, col) = (cell / N, cell % N);
    let mut out = Vec::new();
    if row > 0 {
        out.push(cell - N);
    }
    if row + 1 < N {
        out.push(cell + N);
    }
    if col > 0 {
        out.push(cell - 1);
    }
    if col + 1 < N {
        out.push(cell + 1);
    }
    out
}

/// Tıklanan hücredeki karo boşluğa komşuysa kayar.
fn try_move(world: &mut World, game: &mut Game, cell: usize) {
    let Some(empty) = game.board.iter().position(|&v| v == 0) else { return };
    if game.board[cell] == 0 || !neighbours(empty).contains(&cell) {
        return;
    }
    let n = game.board[cell] as usize;
    game.board.swap(empty, cell);
    game.moves += 1;
    world.add_component(
        game.tiles[n],
        Tween::to_position(cell_position(empty), 0.15).with_ease(Ease::OutCubic),
    );
    play(world, "kay");
    if game.board == solved_board() {
        game.solved = true;
        play(world, "cozuldu");
    }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, _dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };

    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        shuffle(world, &mut game);
    }
    if game.auto > 0 {
        game.auto -= 1;
        if let Some(empty) = game.board.iter().position(|&v| v == 0) {
            let cell = neighbours(empty)[0];
            try_move(world, &mut game, cell);
        }
    } else if input.is_mouse_button_just_pressed(0) && !game.solved {
        if let Some(p) = gizmo::pointer::cursor_on_plane(world, Vec3::new(0.0, TILE_Y, 0.0), Vec3::Y) {
            let offset = (N as f32 - 1.0) * 0.5;
            let col = (p.x / CELL + offset).round();
            let row = (p.z / CELL + offset).round();
            if (0.0..N as f32).contains(&col) && (0.0..N as f32).contains(&row) {
                try_move(world, &mut game, row as usize * N + col as usize);
            }
        }
    }

    sync_transforms(world);
    draw_hud(world, &game);
    world.insert_resource(game);
}

fn play(world: &mut World, name: &str) {
    if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
        let _ = audio.play(name);
    }
}

fn draw_hud(world: &mut World, game: &Game) {
    let mut texts = world.borrow_mut::<Text>();
    if let Some(mut t) = texts.get_mut(game.hud) {
        t.content = format!("Hamle {}\nR: yeniden karıştır", game.moves);
    }
    if let Some(mut t) = texts.get_mut(game.banner) {
        t.content = if game.solved { format!("Çözüldü! {} hamle", game.moves) } else { String::new() };
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
