//! # Nişancı — ikiz çubuklu bir arena nişancısı
//!
//! Yukarıdan bakan bir arena: bir yönde yürü, başka yöne ateş et; düşmanlar dalga dalga geliyor
//! ve her dalga bir öncekinden kalabalık. Motorun yedinci oyunu ve şimdiye kadarki en çok
//! şey **doğurup öldüren**i — saniyede onlarca mermi, kıvılcım ve düşman.
//!
//! | girdi | iş |
//! |---|---|
//! | **WASD** · sol çubuk | yürü |
//! | **fare** · sağ çubuk | nişan al |
//! | **sol tık** (basılı) · sağ tetik | ateş |
//! | **R** · Start | yeniden başla |
//! | **dokunmatik**: sol çubuk · sağ çubuk | yürü · nişan al ve ateş et |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Yanlış yazılmış bir mesh anahtarı her varlıkta yeniden başarısız oluyordu.** Mermiyi
//!   önce `MeshSource("standard_sphere")` ile yazdım — anahtar `"sphere"`. Mermi görünmedi, ama
//!   asıl sorun başka yerdeydi: bilinmeyen anahtar dosya sanılıyor, okunamıyor ve başarısızlık
//!   önbelleğe girmiyordu, yani saniyede on bir mermi için on bir disk okuma denemesi, on bir hata
//!   satırı ve on bir boş GPU tamponu. Hata da yalnızca "OBJ load failed" diyordu. **Kapatıldı:**
//!   başarısızlık bir kez oluyor (önbellekte boş mesh; onu tutan kalmayınca toplanıyor, sonradan
//!   gelen dosya yine yükleniyor) ve uzantısız bir ad için hata geçerli ilkel anahtarlarını
//!   sayıyor.
//!
//! - **Telefonda oynanamıyordu.** Dokunma gelince ilk parmak fare oluyor; ama ikiz çubuklu bir
//!   oyun iki başparmak ister, ve ikinci parmağı izleyecek bir arayüz öğesi yoktu. **Kapatıldı:**
//!   `gizmo::ui::VirtualStick` ve `VirtualButton` — her biri kendi kutusuna inen parmağı sahipleniyor
//!   ve kalkana kadar onu izliyor (kutudan çıksa da); parmak yoksa fareye uyuyor. Ekrana ilk
//!   dokunuşta iki çubuk beliriyor; masaüstünde hiç görünmüyor.
//!
//! Gerisi motorda vardı ve ilk denemede çalıştı: `Input::move_axis` (WASD + sol çubuk),
//! `Gamepad::right_stick`/`right_trigger`, `pointer::cursor_on_plane`, `Timer` (ateş hızı ve doğma
//! temposu — uzun bir karede kaçan atış yok), `ParticleEmitter::one_shot` + `DespawnAfter`,
//! `CameraShake`, `ScreenFade::flash`, `Rng`, sentezlenmiş sesler.
//!
//! ## Ortam değişkenleri
//!   * `NISANCI_OTO=1` — kendi kendine oynar (ekran görüntüsü ve dayanıklılık için; `nisanci --test`
//!     iki dakikasını pencere açmadan oynar)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::core::input::{GamepadButton, Input};
use gizmo::prelude::*;
use gizmo::renderer::components::{ParticleEmitter, Text, TextAlign, TextAnchor};
use gizmo::renderer::screen_fade::ScreenFade;
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, SimpleAppExt, SimpleSceneState};
use gizmo::ui::{BackgroundColor, NodeBundle, PositionType, Style, UiPlugin, UiRect, Val, VirtualStick};

/// Arenanın yarı genişliği/derinliği.
const ARENA: f32 = 12.0;
const PLAYER_SPEED: f32 = 7.0;
const BULLET_SPEED: f32 = 22.0;
const FIRE_PERIOD: f32 = 0.09;
const ENEMY_RADIUS: f32 = 0.45;
const PLAYER_RADIUS: f32 = 0.45;
/// Kamera arenanın merkezinin bu kadar üstünde ve gerisinde; arenaya `CAMERA_PITCH` ile bakıyor.
const CAMERA_HEIGHT: f32 = 20.0;
const CAMERA_BACK: f32 = 11.0;
const CAMERA_PITCH: f32 = -1.07;

#[derive(Clone, Copy)]
struct Player;
gizmo::core::impl_component!(Player);

#[derive(Clone, Copy)]
struct Enemy {
    hp: i32,
    speed: f32,
}
gizmo::core::impl_component!(Enemy);

#[derive(Clone, Copy)]
struct Bullet {
    velocity: Vec3,
    life: f32,
}
gizmo::core::impl_component!(Bullet);

struct Game {
    player: u32,
    camera: u32,
    hud: u32,
    banner: u32,
    rng: Rng,
    fire: Timer,
    spawn: Timer,
    wave: u32,
    to_spawn: u32,
    score: u32,
    lives: u32,
    grace: f32,
    aim: Vec3,
    auto: bool,
    t: f32,
    /// Dokunmatik çubuklar: ilk dokunuşta kuruluyor (hareket, nişan).
    sticks: Option<(u32, u32)>,
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
/// `sound` kapalıyken ses aygıtı açılmaz.
fn setup(world: &mut World, font: FontId, sound: bool) -> Game {

    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.1, 0.0)).with_scale(Vec3::new(ARENA, 0.1, ARENA)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.16, 0.17, 0.2, 1.0), 0.95, 0.0),
        MeshRenderer::new(),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-1.1)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 3.0, LightRole::Sun),
    ));
    let player = world
        .spawn_bundle((
            Transform::new(Vec3::new(0.0, 0.4, 0.0)).with_scale(Vec3::splat(0.4)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_unlit(Vec4::new(0.3, 0.8, 1.0, 1.0)).with_emissive(Vec3::new(0.2, 0.6, 1.0)),
            MeshRenderer::new(),
            Player,
        ))
        .id();
    let camera = world
        .spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, CAMERA_HEIGHT, CAMERA_BACK),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: CAMERA_PITCH,
            primary: true,
            ..Default::default()
        })
        .id();
    let mut shake = CameraShake::default();
    shake.max_offset = 0.25;
    shake.max_angle = 0.0;
    shake.decay = 1.6;
    if let Some(e) = world.get_entity(camera) {
        world.add_component(e, shake);
    }
    world.insert_resource(ScreenFade::default());

    let hud = world
        .spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
        .id();
    let banner = world
        .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.4), Vec2::ZERO)
            .with_anchor(TextAnchor::Center)
            .with_align(TextAlign::Center),))
        .id();

    if let Some(mut audio) = sound.then(AudioManager::new).and_then(Result::ok) {
        audio.load_sound_bytes("ates", Blip::new(Wave::Square, 900.0, 0.04).with_slide(500.0).with_volume(0.06).wav());
        audio.load_sound_bytes("vur", Blip::new(Wave::Noise, 1400.0, 0.08).with_slide(300.0).with_volume(0.2).wav());
        audio.load_sound_bytes("acı", Blip::new(Wave::Saw, 220.0, 0.35).with_slide(70.0).with_volume(0.35).wav());
        world.insert_resource(audio);
    }

    Game {
        player,
        camera,
        hud,
        banner,
        rng: Rng::new(2026),
        fire: Timer::repeating(FIRE_PERIOD),
        spawn: Timer::repeating(0.6),
        wave: 0,
        to_spawn: 0,
        score: 0,
        lives: 3,
        grace: 0.0,
        aim: Vec3::X,
        auto: std::env::var_os("NISANCI_OTO").is_some(),
        t: 0.0,
        sticks: None,
    }
}

/// `--test`: pencere açmadan iki dakika otomatik oyun, oyunun eklentileriyle. Düşmanlar vurulmalı,
/// dalgalar ilerlemeli; kıvılcımlar ömürleri dolunca silinmeli.
pub fn self_test() -> bool {
    use gizmo::systems::lifetime::{DespawnAfter, LifetimePlugin};
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font(), false);
    game.auto = true;
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world).add_plugin(UiPlugin).add_plugin(LifetimePlugin);
    let (mut sparks_most, mut deaths, mut best) = (0usize, 0u32, (0u32, 0u32));
    let mut was_alive = true;
    while run.elapsed < 120.0 {
        run.frame(1.0 / 60.0, update);
        let (lives, score, wave) = run.world.get_resource::<Game>().map(|g| (g.lives, g.score, g.wave)).expect("oyun kaynağı");
        if lives == 0 && was_alive {
            deaths += 1;
        }
        was_alive = lives > 0;
        best = best.max((score, wave));
        sparks_most = sparks_most.max(run.world.borrow::<DespawnAfter>().iter().count());
        if lives == 0 {
            // Otomatik oyun ölünce yeniden başlar (R'ye basmış gibi).
            let mut game = run.world.remove_resource::<Game>().expect("oyun kaynağı");
            restart(&mut run.world, &mut game);
            run.world.insert_resource(game);
        }
    }
    let touch_ok = touch_test();
    let camera_ok = camera_test();
    let ok = best.0 > 0 && best.1 >= 2 && sparks_most < 50 && touch_ok && camera_ok;
    println!(
        "nisanci --test: 120 s, en iyi {} puan, {}. dalga, {deaths} kez öldü; aynı anda en çok {sparks_most} kıvılcım → {}",
        best.0,
        best.1,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

/// Kamera dar ekranda oyuncuyu izliyor: dikey bir telefonda (9 : 19,5) arenanın iki ucundaki
/// oyuncu görünen alanın içinde kalmalı; geniş bir pencerede kamera hiç kıpırdamamalı.
fn camera_test() -> bool {
    let fov = std::f32::consts::FRAC_PI_3;
    let distance = (CAMERA_HEIGHT * CAMERA_HEIGHT + CAMERA_BACK * CAMERA_BACK).sqrt();
    let phone = 9.0 / 19.5;
    let half_width = distance * (fov * 0.5).tan() * phone;
    let edge = ARENA - 0.5;
    let seen = [-edge, edge].iter().all(|&x| (x - camera_x(x, phone, fov)).abs() + PLAYER_RADIUS < half_width);
    let still = [-edge, 0.0, edge].iter().all(|&x| camera_x(x, 16.0 / 9.0, fov) == 0.0);
    let ok = seen && still;
    println!(
        "    kamera: dikey telefonda arenanın uçları görünüyor mu {seen}, geniş pencerede kımıldamıyor mu {still} → {}",
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

/// İlk dokunuş, pencere açmadan: sol alttaki çubuk alanına inen ilk parmak sağa kayınca oyuncu
/// sağa gitmeli. Çubuklar o dokunuşa cevap olarak kuruluyor; motor yeni bir kontrole altında
/// duran parmağı vermeden önce bu ilk dokunuş boşa gidiyordu, başparmağı kaldırıp yeniden
/// koymak gerekiyordu.
fn touch_test() -> bool {
    use gizmo::core::input::touch::TouchPhase;
    let mut world = World::new();
    let game = setup(&mut world, HeadlessFrames::font(), false);
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world).add_plugin(UiPlugin);
    let x = |run: &HeadlessFrames| run.world.get_resource::<Game>().map(|g| player_position(&run.world, &g).x).unwrap_or(0.0);
    let size = gizmo::core::window::WindowInfo::default();
    let (fx, fy) = (size.width * 0.15, size.height * 0.8);
    let start = x(&run);
    run.input.on_touch(1, TouchPhase::Started, fx, fy);
    for i in 0..40 {
        if i == 5 {
            run.input.on_touch(1, TouchPhase::Moved, fx + 60.0, fy);
        }
        run.frame(1.0 / 60.0, update);
    }
    let moved = x(&run) - start;
    let ok = moved > 0.5;
    println!(
        "    dokunma: ilk parmak sol çubukta sağa → oyuncu {moved:+.2} birim → {}",
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

/// Bütün oyun: sahne, girdi, güncelleme. Masaüstü ikilisi ve tarayıcı yapısı (`?oyun=nisanci`)
/// bunu çalıştırıyor.
pub fn app() -> App<SimpleSceneState> {
    App::<SimpleSceneState>::new("Gizmo Engine - Nişancı", 1280, 720)
        .add_plugin(UiPlugin)
        // Kıvılcım varlıkları `DespawnAfter` ile kendini siliyor.
        .add_plugin(gizmo::systems::lifetime::LifetimePlugin)
        .with_simple_scene(|scene, _state| {
            // Makinenin bir fontu (`GIZMO_FONT` ya da platformun olağan yüzleri); tarayıcıda yok,
            // orada motorla gelen Noto Sans.
            let font = scene.renderer.default_font();
            let game = setup(scene.world, font, true);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.t += dt;
    let pad = input.gamepad();
    let restart_pressed =
        input.is_key_just_pressed(KeyCode::KeyR as u32) || pad.is_some_and(|p| p.is_just_pressed(GamepadButton::Start));
    if restart_pressed && game.lives == 0 {
        restart(world, &mut game);
    }

    // Oto modda da kuruluyor: ekran görüntüsünde yerleşimleri görünsün (oto oynayış onları okumuyor).
    if game.sticks.is_none() && (!input.touches().is_empty() || game.auto) {
        game.sticks = Some(spawn_sticks(world));
    }
    if game.lives > 0 {
        move_player(world, &mut game, input, dt);
        follow_with_camera(world, &game, dt);
        aim_and_fire(world, &mut game, input, dt);
        spawn_enemies(world, &mut game, dt);
        chase(world, &game, dt);
        get_hit(world, &mut game, dt);
    }
    fly_bullets(world, &mut game, dt);
    draw_hud(world, &game);
    world.insert_resource(game);
}

/// Dar bir ekranda — dikey tutulan bir telefon — arena yanlara sığmıyor: kamera geniş bir pencere
/// için kurulu, dikey telefonda yatay görüş 34° kadar ve 24 birimlik arenanın yarısından azı
/// görünüyor; oyuncu kenara yürüyünce ekrandan çıkıyordu. Görünen yarı genişlik arenadan darsa
/// kamera yanlara oyuncuyu izliyor, arenanın kenarında duruyor; geniş bir pencerede yerinden
/// kıpırdamıyor.
fn follow_with_camera(world: &mut World, game: &Game, dt: f32) {
    let aspect = world.get_resource::<gizmo::core::window::WindowInfo>().map_or(16.0 / 9.0, |w| w.aspect_ratio());
    let fov = world.borrow::<Camera>().get(game.camera).map_or(std::f32::consts::FRAC_PI_3, |c| c.fov);
    let target = camera_x(player_position(world, game).x, aspect, fov);
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.camera) {
        // Yumuşak: oyuncu dönünce görüntü sarsılmasın.
        let x = t.position.x + (target - t.position.x) * (1.0 - (-8.0 * dt).exp());
        t.position.x = x;
    }
}

/// Kameranın x'i: oyuncu `player_x`'teyken, en-boy oranı `aspect` olan bir ekranda, dikey görüş
/// açısı `fov`. Görünen alan arenayı (bir birim payla) kaplıyorsa 0.
fn camera_x(player_x: f32, aspect: f32, fov: f32) -> f32 {
    let distance = (CAMERA_HEIGHT * CAMERA_HEIGHT + CAMERA_BACK * CAMERA_BACK).sqrt();
    let half_width = distance * (fov * 0.5).tan() * aspect;
    let room = ARENA + 1.0 - half_width;
    if room > 0.0 { player_x.clamp(-room, room) } else { 0.0 }
}

fn player_position(world: &World, game: &Game) -> Vec3 {
    world.borrow::<Transform>().get(game.player).map(|t| t.position).unwrap_or(Vec3::ZERO)
}

fn move_player(world: &mut World, game: &mut Game, input: &Input, dt: f32) {
    let (mx, my) = if game.auto {
        // Arenada yavaş bir daire.
        ((game.t * 0.6).cos(), (game.t * 0.6).sin())
    } else if let Some(v) = stick(world, game.sticks.map(|s| s.0)).filter(|v| *v != Vec2::ZERO) {
        (v.x, v.y)
    } else {
        input.move_axis()
    };
    // Ekranda yukarı = dünyada -Z (kamera kuzeye bakıyor).
    let step = Vec3::new(mx, 0.0, -my);
    let step = if step.length() > 1.0 { step.normalize() } else { step };
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.player) {
        let p = t.position + step * PLAYER_SPEED * dt;
        t.position = Vec3::new(p.x.clamp(-ARENA + 0.5, ARENA - 0.5), p.y, p.z.clamp(-ARENA + 0.5, ARENA - 0.5));
    }
}

fn aim_and_fire(world: &mut World, game: &mut Game, input: &Input, dt: f32) {
    let me = player_position(world, game);
    let pad = input.gamepad();
    let stick = pad.map(|p| p.right_stick()).unwrap_or((0.0, 0.0));
    let firing;
    if game.auto {
        // En yakın düşmana.
        let nearest = {
            let es = world.borrow::<Enemy>();
            let ts = world.borrow::<Transform>();
            es.iter()
                .filter_map(|(id, _)| ts.get(id).map(|t| t.position))
                .min_by(|a, b| a.distance(me).total_cmp(&b.distance(me)))
        };
        firing = nearest.is_some();
        if let Some(n) = nearest {
            game.aim = (n - me).with_y(0.0).normalize_or(game.aim);
        }
    } else if let Some(v) = self::stick(world, game.sticks.map(|s| s.1)).filter(|v| v.length() > 0.3) {
        // Sağ çubuk: ittiğin yöne nişan al ve ateş et.
        game.aim = Vec3::new(v.x, 0.0, -v.y).normalize_or(game.aim);
        firing = true;
    } else if game.sticks.is_some() {
        // Dokunmatik oyunda ilk parmak fareyi de sürüyor; sol çubuğu tutan parmak ateş etmesin.
        firing = false;
    } else if stick.0.hypot(stick.1) > 0.3 {
        game.aim = Vec3::new(stick.0, 0.0, -stick.1).normalize_or(game.aim);
        firing = pad.is_some_and(|p| p.right_trigger() > 0.3) || stick.0.hypot(stick.1) > 0.85;
    } else {
        if let Some(p) = gizmo::pointer::cursor_on_plane(world, Vec3::new(0.0, 0.4, 0.0), Vec3::Y) {
            game.aim = (p - me).with_y(0.0).normalize_or(game.aim);
        }
        firing = input.is_mouse_button_pressed(0) || pad.is_some_and(|p| p.right_trigger() > 0.3);
    }

    let shots = game.fire.tick(dt);
    if !firing {
        return;
    }
    for _ in 0..shots.min(3) {
        let spread = game.rng.range(-0.05..0.05);
        let dir = Quat::from_rotation_y(spread) * game.aim;
        world.spawn_bundle((
            Transform::new(me + dir * 0.5).with_scale(Vec3::new(0.08, 0.08, 0.08)),
            GlobalTransform::default(),
            MeshSource("sphere".into()),
            MaterialDesc::new().with_unlit(Vec4::new(1.0, 0.85, 0.3, 1.0)).with_emissive(Vec3::new(3.0, 2.0, 0.5)),
            MeshRenderer::new(),
            Bullet { velocity: dir * BULLET_SPEED, life: 1.2 },
        ));
    }
    if shots > 0 {
        play(world, "ates");
    }
}

/// Dokunmatik çubuklar: ekranın sol alt ve sağ alt köşelerinde iki yarı saydam kutu.
fn spawn_sticks(world: &mut World) -> (u32, u32) {
    let mut spawn = |left: Val, right: Val| {
        let e = world.spawn_bundle(NodeBundle {
            style: Style {
                position_type: PositionType::Absolute,
                inset: UiRect { left, right, top: Val::Auto, bottom: Val::Percent(3.0) },
                // Pencereye oranlı: telefonda yatay ekranın iki alt köşesi, masaüstünde küçük.
                width: Val::Percent(30.0),
                height: Val::Percent(40.0),
                ..Default::default()
            },
            background_color: BackgroundColor(Vec4::new(1.0, 1.0, 1.0, 0.08)),
            ..Default::default()
        });
        world.add_component(e, VirtualStick::new(70.0));
        e.id()
    };
    (spawn(Val::Percent(2.0), Val::Auto), spawn(Val::Auto, Val::Percent(2.0)))
}

/// Bir dokunmatik çubuğun değeri (yoksa `None`).
fn stick(world: &World, id: Option<u32>) -> Option<Vec2> {
    world.borrow::<VirtualStick>().get(id?).map(|s| s.value())
}

fn spawn_enemies(world: &mut World, game: &mut Game, dt: f32) {
    let alive = world.borrow::<Enemy>().iter().count();
    if game.to_spawn == 0 && alive == 0 {
        game.wave += 1;
        game.to_spawn = 6 + game.wave * 4;
        game.spawn = Timer::repeating((0.6 - game.wave as f32 * 0.04).max(0.15));
    }
    if game.to_spawn == 0 || game.spawn.tick(dt) == 0 {
        return;
    }
    game.to_spawn -= 1;
    // Kenarın rastgele bir noktasından.
    let s = game.rng.range(-ARENA..ARENA);
    let at = match game.rng.range(0..4) {
        0 => Vec3::new(s, 0.4, -ARENA),
        1 => Vec3::new(s, 0.4, ARENA),
        2 => Vec3::new(-ARENA, 0.4, s),
        _ => Vec3::new(ARENA, 0.4, s),
    };
    let tough = game.rng.chance(0.15 + game.wave as f32 * 0.03);
    let (hp, speed, size, colour) = if tough {
        (5, 2.2, 0.55, Vec4::new(0.9, 0.35, 0.1, 1.0))
    } else {
        (2, 3.2 + game.rng.range(0.0..0.8), 0.38, Vec4::new(0.85, 0.15, 0.3, 1.0))
    };
    world.spawn_bundle((
        Transform::new(at).with_scale(Vec3::splat(size)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(colour, 0.5, 0.0),
        MeshRenderer::new(),
        Enemy { hp, speed },
    ));
}

/// Düşmanlar oyuncuya yürüyor, birbirlerinden biraz itilerek (yoksa tek bir küpe dönüşürler).
fn chase(world: &mut World, game: &Game, dt: f32) {
    let me = player_position(world, game);
    let enemies: Vec<(u32, Vec3, f32)> = {
        let es = world.borrow::<Enemy>();
        let ts = world.borrow::<Transform>();
        es.iter().filter_map(|(id, e)| ts.get(id).map(|t| (id, t.position, e.speed))).collect()
    };
    let mut ts = world.borrow_mut::<Transform>();
    for (i, &(id, p, speed)) in enemies.iter().enumerate() {
        let mut v = (me - p).with_y(0.0).normalize_or_zero() * speed;
        for (j, &(_, q, _)) in enemies.iter().enumerate() {
            let d = (p - q).with_y(0.0);
            let l = d.length();
            if i != j && l < 0.9 && l > 1e-4 {
                v += d / l * (0.9 - l) * 6.0;
            }
        }
        if let Some(mut t) = ts.get_mut(id) {
            t.position += v * dt;
        }
    }
}

fn fly_bullets(world: &mut World, game: &mut Game, dt: f32) {
    let targets: Vec<(u32, Vec3)> = {
        let es = world.borrow::<Enemy>();
        let ts = world.borrow::<Transform>();
        es.iter().filter_map(|(id, _)| ts.get(id).map(|t| (id, t.position))).collect()
    };
    let mut dead_bullets = Vec::new();
    let mut hits: Vec<(u32, Vec3)> = Vec::new();
    if let Some(mut q) = world.query_mut::<(Mut<Bullet>, Mut<Transform>)>() {
        for (id, (mut b, mut t)) in q.iter_mut() {
            b.life -= dt;
            t.position += b.velocity * dt;
            let p = t.position;
            if b.life <= 0.0 || p.x.abs() > ARENA + 1.0 || p.z.abs() > ARENA + 1.0 {
                dead_bullets.push(id);
                continue;
            }
            if let Some(&(enemy, _)) = targets.iter().find(|(_, q)| q.distance(p) < ENEMY_RADIUS) {
                dead_bullets.push(id);
                hits.push((enemy, p));
            }
        }
    }
    for id in dead_bullets {
        world.despawn_by_id(id);
    }
    for (enemy, at) in hits {
        let killed = {
            let mut es = world.borrow_mut::<Enemy>();
            let Some(mut e) = es.get_mut(enemy) else { continue };
            e.hp -= 1;
            e.hp == 0
        };
        if killed {
            let colour = Vec4::new(1.0, 0.5, 0.2, 1.0);
            world.despawn_by_id(enemy);
            sparks(world, at, colour, 30);
            game.score += 10;
            kick(world, game, 0.12);
            play(world, "vur");
        } else {
            sparks(world, at, Vec4::new(1.0, 0.9, 0.6, 1.0), 6);
        }
    }
}

fn get_hit(world: &mut World, game: &mut Game, dt: f32) {
    game.grace = (game.grace - dt).max(0.0);
    if game.grace > 0.0 {
        return;
    }
    let me = player_position(world, game);
    let touching: Vec<u32> = {
        let es = world.borrow::<Enemy>();
        let ts = world.borrow::<Transform>();
        es.iter()
            .filter(|(id, _)| ts.get(*id).is_some_and(|t| t.position.distance(me) < ENEMY_RADIUS + PLAYER_RADIUS))
            .map(|(id, _)| id)
            .collect()
    };
    if touching.is_empty() {
        return;
    }
    game.lives -= 1;
    game.grace = 1.5;
    // Değenleri it: aynı kalabalık bir sonraki karede yine vurmasın.
    for id in touching {
        world.despawn_by_id(id);
        sparks(world, me, Vec4::new(0.3, 0.8, 1.0, 1.0), 20);
    }
    kick(world, game, 0.6);
    if let Some(mut fade) = world.get_resource_mut::<ScreenFade>() {
        fade.flash(Vec3::new(0.9, 0.05, 0.05), 0.4, 0.35);
    }
    play(world, "acı");
    if game.lives == 0 {
        if let Some(mut fade) = world.get_resource_mut::<ScreenFade>() {
            fade.set_color(Vec3::ZERO);
            fade.fade_to(0.6, 1.0);
        }
    }
}

fn restart(world: &mut World, game: &mut Game) {
    world.despawn_all_with::<Enemy>();
    world.despawn_all_with::<Bullet>();
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.player) {
        t.position = Vec3::new(0.0, 0.4, 0.0);
    }
    if let Some(mut fade) = world.get_resource_mut::<ScreenFade>() {
        fade.fade_in(0.4);
    }
    game.wave = 0;
    game.to_spawn = 0;
    game.score = 0;
    game.lives = 3;
    game.grace = 1.0;
}

fn sparks(world: &mut World, at: Vec3, colour: Vec4, count: u32) {
    let mut burst = ParticleEmitter::one_shot(count);
    burst.initial_velocity = Vec3::new(0.0, 2.0, 0.0);
    burst.velocity_randomness = 5.0;
    burst.lifespan = 0.35;
    burst.lifespan_randomness = 0.15;
    burst.size_start = 0.12;
    burst.size_end = 0.0;
    burst.color_start = colour;
    burst.color_end = colour.truncate().extend(0.0);
    world.spawn_bundle((
        Transform::new(at),
        GlobalTransform::default(),
        burst,
        gizmo::systems::lifetime::DespawnAfter::secs(0.6),
    ));
}

fn kick(world: &mut World, game: &Game, trauma: f32) {
    if let Some(mut s) = world.borrow_mut::<CameraShake>().get_mut(game.camera) {
        s.add(trauma);
    }
}

fn play(world: &mut World, name: &str) {
    if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
        let _ = audio.play(name);
    }
}

fn draw_hud(world: &mut World, game: &Game) {
    let enemies = world.borrow::<Enemy>().iter().count();
    let bullets = world.borrow::<Bullet>().iter().count();
    let effects = world.borrow::<ParticleEmitter>().iter().count();
    let fps = world.get_resource::<Time>().map(|t| t.fps()).unwrap_or(0.0);
    let mut texts = world.borrow_mut::<Text>();
    if let Some(mut t) = texts.get_mut(game.hud) {
        t.content = format!(
            "Skor {}   Can {}   Dalga {}\nDüşman {}   Mermi {}   Efekt {}   {:.0} fps",
            game.score, game.lives, game.wave, enemies, bullets, effects, fps
        );
    }
    if let Some(mut t) = texts.get_mut(game.banner) {
        t.content = if game.lives == 0 { format!("Bitti — {} puan\nR / Start: yeniden", game.score) } else { String::new() };
    }
}
