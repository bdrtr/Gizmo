//! # Labirent — eğilen tahtada bir bilye
//!
//! Tahtayı eğ, bilye yuvarlansın: duvarların arasından, deliklere düşmeden yeşil kareye. Üç
//! bölüm; her bölümün en iyi süresi saklanıyor. Düşersen başa dönüyorsun, saat işlemeye devam
//! ediyor. Motorun on üçüncü oyunu; fizik merkezde: tahta **kinematik** bir cisim, bilye onun
//! üstünde gerçek bir dinamik cisim — tahtanın dönüşü bilyeyi taşıyor, itiyor, duvarlar
//! durduruyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **WASD / oklar / sol çubuk** | tahtayı eğ |
//! | **R** | bölümü baştan |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Kinematik bir cisim bir duruşa sürülemiyordu.** Kinematik cisim `Velocity`'siyle hareket
//!   ediyor; oyun ise tahtanın *nerede* olmasını istediğini biliyor. O duruşu hıza çevirmek oyuna
//!   kalıyordu, ve sabit adımlı fizikte bu yanlış: bir kare sıfır, bir ya da birkaç adım koşuyor,
//!   kare süresinden hesaplanan açısal hız tahtayı yanlış kadar döndürüyor. **Kapatıldı:**
//!   `KinematicTarget::new(konum, dönüş)` — her adımdan önce cismin hızı hedefe *o adımda*
//!   varacak şekilde kuruluyor (üstündeki bilye bu hareketle taşınıyor), adımdan sonra cisim
//!   hedefe tam oturtuluyor; hedef değişmedikçe orada sıfır hızla duruyor.
//! - **Çok parçalı bir çarpışma şekli yazılamıyordu.** Tahta: yer karoları, delikler (karo yok),
//!   duvarlar — tek cisim. `ColliderShape::Compound`'u elle kurmak her şekli kutulamak demekti.
//!   **Kapatıldı:** `Collider::compound([(Transform, Collider), …])`.
//! - **…ve çok parçalı bir şekil CCD'li bir cisme yaklaşınca fizik çöküyordu.** Kinematik cisimler
//!   CCD'yi kendiliğinden açıyor; CCD'nin "öngörülü temas" yolu bileşik şekli parçalara ayırmadan
//!   GJK'ya veriyordu: debug'da panik, release'de sıfır destek noktası (yanlış bir temas). Kalıcı
//!   bir motor hatasıydı, bu oyunun ilk adımında çıktı. **Düzeltildi:** bileşik şekil parçalarına
//!   ayrılıyor, en yakın parçanın teması tutuluyor.
//!
//! - **Karolu zeminde bilye dikişlere takılıyordu.** Tahtanın zemini yan yana kutular; bir karodan
//!   ötekine geçen bilye, yanındaki kutunun üst *kenarına* da değiyor ve o kenarın normali onu geri
//!   itiyor. Ölçüldü (iki saniye, 9° eğim, CCD'li bilye): tek parça zeminde 2,21 m (analitik
//!   değer), on yedi karoda 1,01 m. **Düzeltildi:** bileşik bir şeklin kutu parçasından gelen
//!   kenar teması, başka bir parça aynı düzlemde yüz temasıyla cismi tutuyorsa atılıyor (bir
//!   basamağın kenarı o düzlemde değil, kalıyor): 2,03 m. Kalan %8 dikişlerde değilmiş: bilye
//!   5 cm yukarıdan bırakılıyor ve inişteki tek öngörülü temas "kapatılacak boşluk yok"u `-0.0`
//!   diye yazıyordu; çözücünün `< 0.0` sınaması onu gerçek temas sandı ve bilyenin *merkezine*
//!   sürtünme uyguladı — bilye dönmedi, eğim boyunca taşıdığı hızı bir adımda kaybetti.
//!   **Düzeltildi:** boşluk kesin negatif, öngörülü temasın sürtünmesi yok: 2,23 m. Bu oyun yine
//!   de CCD'yi kapatıyor; hızları ona gerek bırakmıyor.
//!
//! ## Ortam değişkenleri
//!   * `LABIRENT_OTO=1` — labirenti kendisi çözer: duvarlardan ve deliklerden kaçan en kısa yolun
//!     bir sonraki karesine doğru eğer (ekran görüntüsü için; `labirent --test` üç bölümü de
//!     pencere açmadan böyle oynar). Otomatik pilotun süreleri rekor sayılmaz.
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::app::save::SaveData;
use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::physics::components::KinematicTarget;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::screen_fade::ScreenFade;
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

const PHYSICS_HZ: u32 = 240;
const MAX_TILT: f32 = 0.16; // ~9°
const TILT_SPEED: f32 = 0.6; // radyan/sn
const BALL_R: f32 = 0.3;

/// `#` duvar, `.` zemin, `O` delik, `S` başlangıç, `G` hedef. Satırlar +z yönünde (ekranda aşağı).
///
/// Bir delik koca bir karonun yokluğu: 30 cm'lik bilye 1 m'lik boşluğu ancak 4 m/s'nin üstünde
/// aşar, eğimle o hıza çıkılmaz — yani delik geçit değil, tuzak. İkinci ve üçüncü bölümde birer
/// delik hedefe giden her yolu kesiyordu, ikisi de hiç kazanılamazdı (`--test`'in yol bulucusu
/// buldu, 2026-11-04); `--test` artık her bölümde bir yol olduğunu da sınıyor.
const LEVELS: [&[&str]; 3] = [
    &[
        "#########",
        "#S......#",
        "#######.#",
        "#.......#",
        "#.#######",
        "#......G#",
        "#########",
    ],
    &[
        "###########",
        "#S..#.....#",
        "#.#.#.###.#",
        "#.#...#O#.#",
        "#.#####.#.#",
        "#.......#.#",
        "#####.###.#",
        "#G....O...#",
        "###########",
    ],
    &[
        "#############",
        "#S.......O..#",
        "#.#.#####.#.#",
        "#.#...O...#.#",
        "#.###.#.###.#",
        "#O....#....O#",
        "###.#####.###",
        "#...O...O...#",
        "#.#########.#",
        "#.....G.....#",
        "#############",
    ],
];

#[derive(Clone, Copy)]
struct Part;
gizmo::core::impl_component!(Part);

struct Game {
    physics: FixedPhysics,
    level: usize,
    board: Entity,
    ball: Entity,
    /// Başlangıç ve hedef, tahtanın kendi uzayında.
    start: Vec3,
    goal: Vec3,
    tilt: Vec2,
    clock: f32,
    falls: u32,
    best: Vec<Option<f32>>,
    /// Bölüm bitti: bir sonrakine geçmeden önce kalan süre.
    won: Option<f32>,
    hud: Entity,
    banner: Entity,
    auto: bool,
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
/// `sound` kapalıyken ses aygıtı açılmaz.
fn setup(world: &mut World, font: FontId, sound: bool) -> Game {

    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-1.1)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 15.0, 8.5),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -1.05,
        primary: true,
        ..Default::default()
    });
    world.insert_resource(ScreenFade::default());
    let hud = world
        .spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
        ;
    let banner = world
        .spawn_bundle((Text::pinned_at("", font, 40.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
            .with_anchor(TextAnchor::Center)
            .with_align(TextAlign::Center),))
        ;
    if let Some(mut audio) = sound.then(AudioManager::new).and_then(Result::ok) {
        audio.load_sound_bytes("dus", Blip::new(Wave::Sine, 420.0, 0.35).with_slide(90.0).with_volume(0.3).wav());
        audio.load_sound_bytes("hedef", Blip::new(Wave::Square, 523.0, 0.4).with_slide(1046.0).with_volume(0.2).wav());
        world.insert_resource(audio);
    }
    let save = SaveData::open("labirent");
    let best = (0..LEVELS.len()).map(|i| save.get::<f32>(&format!("best{i}"))).collect();
    let mut game = Game {
        physics: FixedPhysics::new(PHYSICS_HZ),
        level: 0,
        board: Entity::new(0, 0),
        ball: Entity::new(0, 0),
        start: Vec3::ZERO,
        goal: Vec3::ZERO,
        tilt: Vec2::ZERO,
        clock: 0.0,
        falls: 0,
        best,
        won: None,
        hud,
        banner,
        auto: std::env::var_os("LABIRENT_OTO").is_some(),
    };
    build_level(world, &mut game);
    game
}

/// Bilyenin bulunduğu kare (tahtanın kendi uzayında), ızgara sırasıyla.
fn cell_of(rows: &[&str], local: Vec3) -> (i32, i32) {
    let (w, h) = (rows[0].len() as f32, rows.len() as f32);
    ((local.x + (w - 1.0) / 2.0).round() as i32, (local.z + (h - 1.0) / 2.0).round() as i32)
}

/// Kareden hedefe, duvarlardan ve deliklerden geçmeyen en kısa yol (kendisi dahil).
fn path_to_goal(rows: &[&str], from: (i32, i32)) -> Vec<(i32, i32)> {
    let at = |x: i32, z: i32| rows.get(z as usize).and_then(|r| r.as_bytes().get(x as usize)).copied().unwrap_or(b'#');
    let mut came = std::collections::HashMap::new();
    let mut queue = std::collections::VecDeque::from([from]);
    came.insert(from, from);
    while let Some((x, z)) = queue.pop_front() {
        if at(x, z) == b'G' {
            let mut path = vec![(x, z)];
            while *path.last().unwrap() != from {
                path.push(came[path.last().unwrap()]);
            }
            path.reverse();
            return path;
        }
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let next = (x + dx, z + dz);
            if !matches!(at(next.0, next.1), b'#' | b'O') && !came.contains_key(&next) {
                came.insert(next, (x, z));
                queue.push_back(next);
            }
        }
    }
    vec![from]
}

/// `LABIRENT_OTO`: yolun bir sonraki karesinin ortasına doğru eğ; bilyenin hızı frenler.
fn auto_tilt(world: &World, game: &Game) -> (f32, f32) {
    let rows = LEVELS[game.level];
    let rotation = world.borrow::<Transform>().get(game.board).map_or(Quat::IDENTITY, |t| t.rotation);
    let (Some(p), Some(v)) = (
        world.borrow::<Transform>().get(game.ball).map(|t| t.position),
        world.borrow::<Velocity>().get(game.ball).map(|v| v.linear),
    ) else {
        return (0.0, 0.0);
    };
    let (local, v) = (rotation.inverse() * p, rotation.inverse() * v);
    let path = path_to_goal(rows, cell_of(rows, local));
    let (w, h) = (rows[0].len() as f32, rows.len() as f32);
    let (x, z) = path.get(1).copied().unwrap_or(path[0]);
    let target = Vec3::new(x as f32 - (w - 1.0) / 2.0, 0.0, z as f32 - (h - 1.0) / 2.0);
    let push = (target - local) * 2.5 - v * 1.8;
    // +x eğimi bilyeyi +x'e, +y eğimi −z'ye yuvarlar.
    let input = Vec2::new(push.x, -push.z).clamp_length_max(1.0);
    (input.x, input.y)
}

/// `--test`: pencere açmadan otomatik pilot üç bölümü sırayla çözer.
fn self_test() -> bool {
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font(), false);
    game.auto = true;
    world.insert_resource(game);
    // Her bölüm kazanılabilir mi: başlangıçtan hedefe, deliklerden geçmeyen bir yol.
    let blocked: Vec<usize> = (0..LEVELS.len())
        .filter(|&l| {
            let rows = LEVELS[l];
            let start = rows.iter().enumerate().find_map(|(z, r)| r.find('S').map(|x| (x as i32, z as i32))).expect("S");
            path_to_goal(rows, start).len() < 2
        })
        .collect();
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    let (mut solved, mut falls, mut times) = (0usize, 0u32, Vec::new());
    let mut was_won = false;
    while run.elapsed < 240.0 && solved < LEVELS.len() {
        run.frame(dt, update);
        let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
        let won = game.won.is_some();
        if won && !was_won {
            solved += 1;
            falls += game.falls;
            times.push(game.clock);
        }
        was_won = won;
    }
    let ok = blocked.is_empty() && solved == LEVELS.len();
    let times: Vec<String> = times.iter().map(|t| format!("{t:.1}")).collect();
    println!(
        "labirent --test: yolu olmayan bölüm {blocked:?}; {solved} / {} bölüm çözüldü ({} sn), {falls} düşüş, {:.0} s → {}",
        LEVELS.len(),
        times.join(", "),
        run.elapsed,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Labirent", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font, true);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Bölümü kurar: tahta tek bir kinematik cisim (karoların ve duvarların bileşik şekli), görünüşü
/// çocukları; bilye başlangıçta.
fn build_level(world: &mut World, game: &mut Game) {
    for id in [game.board, game.ball] {
        if let Some(e) = world.live(id).filter(|_| id != Entity::new(0, 0)) {
            world.despawn_recursive(e);
        }
    }
    let rows = LEVELS[game.level];
    let (w, h) = (rows[0].len() as f32, rows.len() as f32);
    let cell = |x: usize, z: usize| Vec3::new(x as f32 - (w - 1.0) / 2.0, 0.0, z as f32 - (h - 1.0) / 2.0);

    let mut parts = Vec::new();
    let mut looks: Vec<(Vec3, Vec3, Vec4)> = Vec::new();
    let floor = Vec4::new(0.55, 0.42, 0.28, 1.0);
    let wall = Vec4::new(0.32, 0.22, 0.14, 1.0);
    for (z, row) in rows.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            let at = cell(x, z);
            if c != 'O' {
                let half = Vec3::new(0.5, 0.1, 0.5);
                parts.push((Transform::new(at - Vec3::Y * 0.1), Collider::box_collider(half)));
                let colour = if c == 'G' { Vec4::new(0.2, 0.7, 0.3, 1.0) } else { floor };
                looks.push((at - Vec3::Y * 0.1, half, colour));
            }
            if c == '#' {
                let half = Vec3::new(0.5, 0.35, 0.5);
                parts.push((Transform::new(at + Vec3::Y * 0.35), Collider::box_collider(half)));
                looks.push((at + Vec3::Y * 0.35, half, wall));
            }
            if c == 'S' {
                game.start = at + Vec3::Y * (BALL_R + 0.05);
            }
            if c == 'G' {
                game.goal = at;
            }
        }
    }

    // Tahta: bir kinematik cisim, hedefi düz duruş. CCD kapalı (aşağıya bakın).
    let mut body = RigidBody::new_kinematic();
    body.ccd_enabled = false;
    let board = world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        GlobalTransform::default(),
        body,
        Velocity::default(),
        Collider::compound(parts),
        KinematicTarget::new(Vec3::ZERO, Quat::IDENTITY),
    ));
    for (at, half, colour) in looks {
        let part = world.spawn_bundle((
            Transform::new(at).with_scale(half),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(colour, 0.8, 0.0),
            MeshRenderer::new(),
            Part,
        ));
        world.add_child(board, part);
    }
    game.board = board;
    game.tilt = Vec2::ZERO;
    game.clock = 0.0;
    game.falls = 0;
    game.won = None;
    spawn_ball(world, game);
}

fn spawn_ball(world: &mut World, game: &mut Game) {
    if let Some(e) = world.live(game.ball).filter(|_| game.ball != Entity::new(0, 0)) {
        world.despawn(e);
    }
    // Tahta eğikse başlangıç noktası da onunla birlikte.
    let rotation = world.borrow::<Transform>().get(game.board).map_or(Quat::IDENTITY, |t| t.rotation);
    // CCD yok: bilye 240 Hz'de adım başına birkaç santim gidiyor, 20 cm'lik zemini delemez.
    let body = RigidBody::new(0.2, true);
    let ball = world.spawn_bundle((
        Transform::new(rotation * game.start).with_scale(Vec3::splat(BALL_R)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.8, 0.82, 0.88, 1.0), 0.15, 0.9),
        MeshRenderer::new(),
        body,
        Velocity::default(),
        Collider::sphere(BALL_R),
    ));
    game.ball = ball;
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };

    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        build_level(world, &mut game);
    }

    // Eğim: girdinin istediğine doğru, saniyede TILT_SPEED kadar.
    let (mx, my) = if game.auto { auto_tilt(world, &game) } else { input.move_axis() };
    let want = Vec2::new(mx, my).clamp_length_max(1.0) * MAX_TILT;
    let step = TILT_SPEED * dt;
    game.tilt += (want - game.tilt).clamp_length_max(step);
    // +x eğimi sağ kenarı indirir (z ekseni etrafında), +y (ileri) uzak kenarı indirir.
    let rotation = Quat::from_rotation_x(-game.tilt.y) * Quat::from_rotation_z(-game.tilt.x);
    if let Some(mut target) = world.borrow_mut::<KinematicTarget>().get_mut(game.board) {
        *target = KinematicTarget::new(Vec3::ZERO, rotation);
    }

    game.physics.advance(world, dt);
    sync_transforms(world);

    let ball = world.borrow::<Transform>().get(game.ball).map(|t| t.position);
    match (ball, game.won) {
        (_, Some(t)) => {
            let t = t - dt;
            if t <= 0.0 {
                game.level = (game.level + 1) % LEVELS.len();
                build_level(world, &mut game);
            } else {
                game.won = Some(t);
            }
        }
        (Some(p), None) => {
            game.clock += dt;
            // Hedef: bilye, tahtanın kendi uzayında hedef karenin üstünde.
            let local = rotation.inverse() * p;
            if (local - game.goal).truncate_y().length() < 0.4 && local.y < 0.6 {
                game.won = Some(2.0);
                // Otomatik pilotun süresi rekor sayılmaz (ve sınama oyuncunun kaydına yazmaz).
                if !game.auto {
                    let best = game.best[game.level].map_or(game.clock, |b| b.min(game.clock));
                    game.best[game.level] = Some(best);
                    let mut save = SaveData::open("labirent");
                    if save.set(&format!("best{}", game.level), &best).and_then(|()| save.save()).is_err() {
                        gizmo::gizmo_log!(Warning, "süre kaydedilemedi");
                    }
                }
                play(world, "hedef");
                if let Some(mut fade) = world.get_resource_mut::<ScreenFade>() {
                    fade.flash(Vec3::new(0.3, 0.9, 0.4), 0.5, 0.6);
                }
            } else if p.y < -4.0 {
                game.falls += 1;
                play(world, "dus");
                spawn_ball(world, &mut game);
            }
        }
        (None, None) => spawn_ball(world, &mut game),
    }

    let best = game.best[game.level].map_or("—".to_string(), |b| format!("{b:.1} sn"));
    let hud = format!(
        "Bölüm {}/{}   Süre {:.1} sn   Düşüş {}   En iyi {}\nWASD / oklar: eğ   R: baştan",
        game.level + 1,
        LEVELS.len(),
        game.clock,
        game.falls,
        best
    );
    set_text(world, game.hud, &hud);
    let banner = if game.won.is_some() { format!("Hedef! {:.1} sn", game.clock) } else { String::new() };
    set_text(world, game.banner, &banner);
    world.insert_resource(game);
}

trait TruncateY {
    fn truncate_y(self) -> Vec2;
}
impl TruncateY for Vec3 {
    fn truncate_y(self) -> Vec2 {
        Vec2::new(self.x, self.z)
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


fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    // A font the machine has (`GIZMO_FONT`, or the platform's usual faces); without one, the
    // synthetic test face, so the game still runs.
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
