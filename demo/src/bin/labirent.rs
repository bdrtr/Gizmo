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
//!   * `LABIRENT_OTO=1` — tahtayı kendi kendine sağa eğer (ekran görüntüsü için)
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
use gizmo::simple::{sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

const PHYSICS_HZ: u32 = 240;
const MAX_TILT: f32 = 0.16; // ~9°
const TILT_SPEED: f32 = 0.6; // radyan/sn
const BALL_R: f32 = 0.3;

/// `#` duvar, `.` zemin, `O` delik, `S` başlangıç, `G` hedef. Satırlar +z yönünde (ekranda aşağı).
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
        "#...O...#.#",
        "#####.###.#",
        "#G....O...#",
        "###########",
    ],
    &[
        "#############",
        "#S.O.....O..#",
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
    board: u32,
    ball: u32,
    /// Başlangıç ve hedef, tahtanın kendi uzayında.
    start: Vec3,
    goal: Vec3,
    tilt: Vec2,
    clock: f32,
    falls: u32,
    best: Vec<Option<f32>>,
    /// Bölüm bitti: bir sonrakine geçmeden önce kalan süre.
    won: Option<f32>,
    hud: u32,
    banner: u32,
    auto: bool,
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Labirent", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-1.1)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 15.0, 8.5),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -1.05,
                primary: true,
                ..Default::default()
            });
            scene.world.insert_resource(ScreenFade::default());
            let hud = scene
                .world
                .spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
                .id();
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 40.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();
            if let Ok(mut audio) = AudioManager::new() {
                audio.load_sound_bytes("dus", Blip::new(Wave::Sine, 420.0, 0.35).with_slide(90.0).with_volume(0.3).wav());
                audio.load_sound_bytes("hedef", Blip::new(Wave::Square, 523.0, 0.4).with_slide(1046.0).with_volume(0.2).wav());
                scene.world.insert_resource(audio);
            }
            let save = SaveData::open("labirent");
            let best = (0..LEVELS.len()).map(|i| save.get::<f32>(&format!("best{i}"))).collect();
            let mut game = Game {
                physics: FixedPhysics::new(PHYSICS_HZ),
                level: 0,
                board: 0,
                ball: 0,
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
            build_level(scene.world, &mut game);
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
        if let Some(e) = world.get_entity(id).filter(|_| id != 0) {
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
    game.board = board.id();
    game.tilt = Vec2::ZERO;
    game.clock = 0.0;
    game.falls = 0;
    game.won = None;
    spawn_ball(world, game);
}

fn spawn_ball(world: &mut World, game: &mut Game) {
    if let Some(e) = world.get_entity(game.ball).filter(|_| game.ball != 0) {
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
    game.ball = ball.id();
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };

    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        build_level(world, &mut game);
    }

    // Eğim: girdinin istediğine doğru, saniyede TILT_SPEED kadar.
    let (mx, my) = if game.auto { (1.0, 0.0) } else { input.move_axis() };
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
                let best = game.best[game.level].map_or(game.clock, |b| b.min(game.clock));
                game.best[game.level] = Some(best);
                let mut save = SaveData::open("labirent");
                if save.set(&format!("best{}", game.level), &best).and_then(|()| save.save()).is_err() {
                    gizmo::gizmo_log!(Warning, "süre kaydedilemedi");
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
