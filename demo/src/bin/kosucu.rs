//! # Koşucu — üç şeritte sonsuz bir koşu
//!
//! Yol hiç bitmiyor ve gittikçe hızlanıyor: şerit değiştir, alçak engelin üstünden atla, üst
//! kirişin altından kay, altınları topla. Bir engele çarpınca koşu biter; en uzun mesafe
//! saklanıyor. Motorun on dördüncü oyunu, ve ilk kez dünya **durmadan ileri** gidiyor: yol önde
//! parça parça doğuyor, arkada siliniyor; oyuncu başlangıçtan kilometrelerce uzaklaşıyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **A / D** · **← / →** · sol çubuk · sola/sağa kaydır | şerit değiştir |
//! | **W** · **↑** · **boşluk** · çubuk yukarı · güney tuşu · yukarı kaydır | zıpla |
//! | **S** · **↓** · çubuk aşağı · aşağı kaydır | kay |
//! | **R** · dokun (koşu bitince) | yeniden başla |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Sahneyi aydınlatan gök ayarlanamıyordu.** Arka planı (`Camera::clear_color`) ve sisi
//!   (`Renderer::fog`) açık maviye boyadım; altın paralar gri çıktı. Bir yüzeyin çevresinden aldığı
//!   ışık ve yansıttığı şey, shader'a gömülü dört hazır gökten geliyordu ve hiçbir ayar ona
//!   ulaşmıyordu — ileri çizim yolunun da kendi gömülü gün batımı vardı. **Kapatıldı:**
//!   `Renderer::sky = Some(Sky::gradient(tepe, ufuk, zemin))` — iki yolda da ortam ışığı normalin
//!   çevresindeki, yansıma ayna yönündeki gökten.
//! - **Bu karede doğan bir çocuğun dünyadaki yeri henüz yok.** `GlobalTransform`'u yayılma
//!   güncellemeden sonra koşana kadar varsayılan; yeni engellere karşı bir çarpışma sınaması
//!   onları başlangıçta — oyuncunun durduğu yerde — görürdü. Yazarken yakalandı: oyun yerel
//!   `Transform`'u okuyor (parça kökleri başlangıçta ve dönmüyor). *Açık:* bir varlığın dünya
//!   matrisini ebeveynleri üzerinden o anda soracak bir yol yok.
//!
//! Gerisi ilk denemede çalıştı: önde doğup arkada `despawn_recursive` ile silinen 30 metrelik yol
//! parçaları (otomatik koşu saniyede ~75 varlık doğurup öldürürken süreç üç dakika 89 MB'ta
//! kaldı), `Input::swipe`, `Fog`, `SaveData`, `ScreenFade`, sentezlenmiş sesler.
//!
//! ## Ortam değişkenleri
//!   * `KOSUCU_OTO=1` — kendi kendine koşar (ekran görüntüsü ve dayanıklılık için)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::app::save::SaveData;
use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::{GamepadButton, Input, MoveKeys};
use gizmo::math::Rng;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::{Fog, Sky};
use gizmo::renderer::screen_fade::ScreenFade;
use gizmo::renderer::text::FontId;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

const LANE_GAP: f32 = 2.2;
const CHUNK: f32 = 30.0;
/// Önde hazır tutulan parça sayısı (sis bunların sonunu gizliyor).
const AHEAD: usize = 5;
const START_SPEED: f32 = 12.0;
const MAX_SPEED: f32 = 34.0;
/// Hız artışı, m/s².
const SPEED_UP: f32 = 0.3;
const GRAVITY: f32 = 32.0;
const JUMP_SPEED: f32 = 11.0;
const SLIDE_TIME: f32 = 0.6;
/// Oyuncunun yarı boyutları: ayakta ve kayarken.
const STAND: Vec3 = Vec3::new(0.35, 0.8, 0.35);
const SLIDE: Vec3 = Vec3::new(0.35, 0.35, 0.35);
const SKY: [f32; 3] = [0.55, 0.7, 0.9];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    /// Alçak: üstünden atlanır.
    Low,
    /// Yüksek: yalnızca şerit değiştirerek geçilir.
    Wall,
    /// Üst kiriş: altından kayılır.
    Beam,
}

#[derive(Clone, Copy)]
struct Obstacle {
    kind: Kind,
}
gizmo::core::impl_component!(Obstacle);

#[derive(Clone, Copy)]
struct Coin;
gizmo::core::impl_component!(Coin);

/// Bir yol parçası: başlangıç z'si (parça `z0 - CHUNK`'a kadar uzanıyor).
#[derive(Clone, Copy)]
struct Chunk {
    z0: f32,
}
gizmo::core::impl_component!(Chunk);

struct Game {
    player: u32,
    camera: u32,
    lane: i32,
    /// Oyuncunun yanal konumu (şeride doğru yumuşakça kayıyor).
    x: f32,
    y: f32,
    vy: f32,
    z: f32,
    slide: f32,
    speed: f32,
    coins: u32,
    best: f32,
    over: bool,
    /// Bir sonraki parçanın başlangıç z'si.
    next_chunk: f32,
    rng: Rng,
    hud: u32,
    banner: u32,
    auto: bool,
    spin: f32,
    /// Geçen karenin yön girdisi — bir yöne geçişi (hamleyi) basılı tutmaktan ayırmak için.
    last_axis: Vec2,
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Koşucu", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let r = &mut *scene.renderer;
            // Sis gökle aynı renkte: uzakta doğan parça sisin içinden çıkıyor, birden belirmiyor.
            r.fog = Some(Fog::new(SKY, 0.008).with_height(0.0, 0.0));
            // Sahneyi aydınlatan gök de aynı: altınlar bu mavi-beyazı yansıtıyor.
            r.sky = Some(Sky::gradient([0.3, 0.5, 0.95], SKY, [0.3, 0.3, 0.32]).with_intensity(0.4));
            let font = load_font(r);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.9)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.95, 0.85), 3.2, LightRole::Sun),
            ));
            let camera = scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 4.0, 7.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -0.28,
                primary: true,
                ..Default::default()
            });
            if let Some(mut c) = scene.world.borrow_mut::<Camera>().get_mut(camera.id()) {
                c.clear_color = Some(Vec3::from(SKY));
            }
            let hud = scene
                .world
                .spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
                .id();
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 40.0, Vec2::new(0.5, 0.35), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();
            let player = scene
                .world
                .spawn_bundle((
                    Transform::new(Vec3::new(0.0, STAND.y, 0.0)).with_scale(STAND),
                    GlobalTransform::default(),
                    MeshSource("standard_cube".into()),
                    MaterialDesc::new().with_pbr(Vec4::new(0.9, 0.3, 0.2, 1.0), 0.5, 0.0),
                    MeshRenderer::new(),
                ))
                .id();
            if let Ok(mut audio) = AudioManager::new() {
                audio.load_sound_bytes("altin", Blip::new(Wave::Square, 988.0, 0.08).with_slide(1318.0).with_volume(0.15).wav());
                audio.load_sound_bytes("zipla", Blip::new(Wave::Triangle, 330.0, 0.15).with_slide(660.0).with_volume(0.25).wav());
                audio.load_sound_bytes("carp", Blip::new(Wave::Noise, 200.0, 0.4).with_slide(60.0).with_volume(0.4).wav());
                scene.world.insert_resource(audio);
            }
            scene.world.insert_resource(ScreenFade::default());
            let best = SaveData::open("kosucu").get::<f32>("best").unwrap_or(0.0);
            let mut game = Game {
                player,
                camera: camera.id(),
                lane: 0,
                x: 0.0,
                y: 0.0,
                vy: 0.0,
                z: 0.0,
                slide: 0.0,
                speed: START_SPEED,
                coins: 0,
                best,
                over: false,
                next_chunk: 10.0,
                rng: Rng::new(0x5EED),
                hud,
                banner,
                auto: std::env::var_os("KOSUCU_OTO").is_some(),
                spin: 0.0,
                last_axis: Vec2::ZERO,
            };
            restart(scene.world, &mut game);
            scene.world.insert_resource(game);
        })
        .set_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn lane_x(lane: i32) -> f32 {
    lane as f32 * LANE_GAP
}

fn restart(world: &mut World, game: &mut Game) {
    let chunks: Vec<u32> = world.borrow::<Chunk>().iter().map(|(id, _)| id).collect();
    for id in chunks {
        if let Some(e) = world.get_entity(id) {
            world.despawn_recursive(e);
        }
    }
    game.lane = 0;
    game.x = 0.0;
    game.y = 0.0;
    game.vy = 0.0;
    game.z = 0.0;
    game.slide = 0.0;
    game.speed = START_SPEED;
    game.coins = 0;
    game.over = false;
    game.next_chunk = 10.0;
    // İlk parça boş: oyuncu ısınsın.
    spawn_chunk(world, game, true);
    while game.next_chunk > game.z - CHUNK * AHEAD as f32 {
        spawn_chunk(world, game, false);
    }
}

fn block(at: Vec3, half: Vec3, colour: Vec4) -> impl gizmo::core::component::Bundle + Send + Sync + 'static {
    (
        Transform::new(at).with_scale(half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(colour, 0.8, 0.0),
        MeshRenderer::new(),
    )
}

/// Bir parça: zemin, iki yan duvar, her 10 metrede bir sıra engel (en az bir şerit hep açık) ve
/// açık şeritlerde altınlar.
fn spawn_chunk(world: &mut World, game: &mut Game, empty: bool) {
    let z0 = game.next_chunk;
    game.next_chunk -= CHUNK;
    let mid = z0 - CHUNK / 2.0;
    let root = world.spawn_bundle((Transform::new(Vec3::ZERO), GlobalTransform::default(), Chunk { z0 }));
    let shade = if ((z0 / CHUNK) as i64).rem_euclid(2) == 0 { 0.36 } else { 0.32 };
    let mut parts = vec![
        world.spawn_bundle(block(Vec3::new(0.0, -0.1, mid), Vec3::new(3.6, 0.1, CHUNK / 2.0), Vec4::new(shade, shade, shade + 0.04, 1.0))),
        world.spawn_bundle(block(Vec3::new(-3.8, 0.4, mid), Vec3::new(0.2, 0.5, CHUNK / 2.0), Vec4::new(0.6, 0.55, 0.45, 1.0))),
        world.spawn_bundle(block(Vec3::new(3.8, 0.4, mid), Vec3::new(0.2, 0.5, CHUNK / 2.0), Vec4::new(0.6, 0.55, 0.45, 1.0))),
    ];
    if !empty {
        for row in 0..3 {
            let z = z0 - 5.0 - row as f32 * 10.0;
            let mut lanes = [-1, 0, 1];
            game.rng.shuffle(&mut lanes);
            let blocked = game.rng.range(1..3usize);
            for &lane in &lanes[..blocked] {
                let kind = *game.rng.pick(&[Kind::Low, Kind::Wall, Kind::Beam, Kind::Low]).unwrap_or(&Kind::Low);
                let (at, half, colour) = obstacle_box(kind, lane, z);
                let e = world.spawn_bundle(block(at, half, colour));
                world.add_component(e, Obstacle { kind });
                parts.push(e);
            }
            // Açık bir şeritte bir sıra altın.
            let open = lanes[blocked];
            for k in 0..4 {
                let at = Vec3::new(lane_x(open), 0.7, z + 3.0 - k as f32 * 2.0);
                let e = world.spawn_bundle((
                    Transform::new(at).with_scale(Vec3::new(0.3, 0.05, 0.3)),
                    GlobalTransform::default(),
                    MeshSource("cylinder".into()),
                    MaterialDesc::new().with_pbr(Vec4::new(1.0, 0.8, 0.2, 1.0), 0.3, 1.0),
                    MeshRenderer::new(),
                    Coin,
                ));
                parts.push(e);
            }
        }
    }
    for p in parts {
        world.add_child(root, p);
    }
}

/// Bir engelin kutusu (merkez, yarı boyut, renk).
fn obstacle_box(kind: Kind, lane: i32, z: f32) -> (Vec3, Vec3, Vec4) {
    let x = lane_x(lane);
    match kind {
        Kind::Low => (Vec3::new(x, 0.35, z), Vec3::new(0.9, 0.35, 0.25), Vec4::new(0.9, 0.75, 0.2, 1.0)),
        Kind::Wall => (Vec3::new(x, 1.3, z), Vec3::new(0.9, 1.3, 0.4), Vec4::new(0.7, 0.2, 0.2, 1.0)),
        Kind::Beam => (Vec3::new(x, 1.45, z), Vec3::new(0.9, 0.35, 0.2), Vec4::new(0.25, 0.45, 0.8, 1.0)),
    }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(1.0 / 20.0);

    if game.over {
        let tapped = input.touches().iter().next().is_some() && input.swipe(1.0e9).is_none();
        if input.is_key_just_pressed(KeyCode::KeyR as u32) || tapped || game.auto {
            restart(world, &mut game);
        }
    } else {
        run(world, &mut game, dt, input);
    }

    // Kamera: arkada ve yukarıda, şeridi yumuşakça izliyor.
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.camera) {
        let want = Vec3::new(game.x * 0.6, 3.6 + game.y * 0.3, game.z + 7.0);
        t.position = t.position.lerp(want, (dt * 8.0).min(1.0));
        t.position.z = game.z + 7.0;
    }

    let hud = format!(
        "{:.0} m   Altın {}   Hız {:.0} km/sa   En iyi {:.0} m\nA/D: şerit   W: zıpla   S: kay",
        -game.z,
        game.coins,
        game.speed * 3.6,
        game.best
    );
    set_text(world, game.hud, &hud);
    let banner = if game.over { format!("Çarptın! {:.0} m\nR ile yeniden", -game.z) } else { String::new() };
    set_text(world, game.banner, &banner);
    world.insert_resource(game);
}

fn run(world: &mut World, game: &mut Game, dt: f32, input: &Input) {
    // Girdi: `move_axis` (WASD, sol çubuk) ve oklar; bir yöne *geçildiği* kare bir hamle — basılı
    // tutmak tekrar etmiyor. Çubuk yarıdan fazla itilince sayılıyor.
    let (wx, wy) = input.move_axis();
    let (ax, ay) = input.move_axis_with(MoveKeys::ARROWS);
    let axis = Vec2::new(if wx.abs() > ax.abs() { wx } else { ax }, if wy.abs() > ay.abs() { wy } else { ay });
    let dir = |v: Vec2| -> (i32, i32) {
        let pick = |c: f32| if c > 0.5 { 1 } else if c < -0.5 { -1 } else { 0 };
        (pick(v.x), pick(v.y))
    };
    let (now, before) = (dir(axis), dir(game.last_axis));
    game.last_axis = axis;
    let edge = |now: i32, before: i32, want: i32| now == want && before != want;
    let pad_jump = input.gamepad().is_some_and(|p| p.is_just_pressed(GamepadButton::South));
    let (mut left, mut right, mut jump, mut slide) = (
        edge(now.0, before.0, -1),
        edge(now.0, before.0, 1),
        edge(now.1, before.1, 1) || input.is_key_just_pressed(KeyCode::Space as u32) || pad_jump,
        edge(now.1, before.1, -1),
    );
    let swipe = input.swipe(60.0);
    if let Some((sx, sy)) = swipe {
        if sx.abs() > sy.abs() {
            left |= sx < 0.0;
            right |= sx > 0.0;
        } else {
            jump |= sy < 0.0;
            slide |= sy > 0.0;
        }
    }
    if game.auto {
        (left, right, jump, slide) = autopilot(world, game);
    }
    if left {
        game.lane = (game.lane - 1).max(-1);
    }
    if right {
        game.lane = (game.lane + 1).min(1);
    }
    let grounded = game.y <= 0.0;
    if jump && grounded {
        game.vy = JUMP_SPEED;
        game.slide = 0.0;
        play(world, "zipla");
    }
    if slide {
        game.slide = SLIDE_TIME;
        // Havadayken kaymak: hızla in.
        if !grounded {
            game.vy = game.vy.min(-JUMP_SPEED);
        }
    }

    // Hareket.
    game.speed = (game.speed + SPEED_UP * dt).min(MAX_SPEED);
    game.z -= game.speed * dt;
    let target = lane_x(game.lane);
    game.x += (target - game.x) * (dt * 14.0).min(1.0);
    game.vy -= GRAVITY * dt;
    game.y = (game.y + game.vy * dt).max(0.0);
    if game.y == 0.0 {
        game.vy = game.vy.max(0.0);
    }
    game.slide = (game.slide - dt).max(0.0);
    let half = if game.slide > 0.0 { SLIDE } else { STAND };
    let centre = Vec3::new(game.x, game.y + half.y, game.z);
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.player) {
        t.position = centre;
        t.scale = half;
    }

    // Çarpışma: oyuncunun kutusu engellerin ve altınların kutularıyla.
    let mut hit = false;
    let mut taken = Vec::new();
    {
        // Parçanın kökü başlangıçta ve dönmüyor: çocukların yerel `Transform`'u dünyadaki yerleri.
        // (Bu karede doğanların `GlobalTransform`'u henüz yayılmadı.)
        let transforms = world.borrow::<Transform>();
        for (id, _) in world.borrow::<Obstacle>().iter() {
            let Some(t) = transforms.get(id) else { continue };
            if overlap(centre, half * 0.9, t.position, t.scale) {
                hit = true;
            }
        }
        for (id, _) in world.borrow::<Coin>().iter() {
            let Some(t) = transforms.get(id) else { continue };
            if (t.position - centre).abs().cmple(half + Vec3::splat(0.35)).all() {
                taken.push(id);
            }
        }
    }
    for id in taken {
        if let Some(e) = world.get_entity(id) {
            world.despawn(e);
            game.coins += 1;
            play(world, "altin");
        }
    }
    // Altınlar dönüyor.
    game.spin += dt * 4.0;
    let spin = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_x(game.spin);
    let coins: Vec<u32> = world.borrow::<Coin>().iter().map(|(id, _)| id).collect();
    for id in coins {
        if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(id) {
            t.rotation = spin;
        }
    }

    if hit {
        game.over = true;
        play(world, "carp");
        if let Some(mut fade) = world.get_resource_mut::<ScreenFade>() {
            fade.flash(Vec3::new(1.0, 0.3, 0.2), 0.6, 0.5);
        }
        let distance = -game.z;
        if distance > game.best {
            game.best = distance;
            let mut save = SaveData::open("kosucu");
            if save.set("best", &distance).and_then(|()| save.save()).is_err() {
                gizmo::gizmo_log!(Warning, "en iyi mesafe kaydedilemedi");
            }
        }
    }

    // Yol: önde yeni parçalar, arkada kalanlar siliniyor.
    while game.next_chunk > game.z - CHUNK * AHEAD as f32 {
        spawn_chunk(world, game, false);
    }
    let behind: Vec<u32> = world
        .borrow::<Chunk>()
        .iter()
        .filter(|(_, c)| c.z0 - CHUNK > game.z + 10.0)
        .map(|(id, _)| id)
        .collect();
    for id in behind {
        if let Some(e) = world.get_entity(id) {
            world.despawn_recursive(e);
        }
    }
}

fn overlap(a: Vec3, ha: Vec3, b: Vec3, hb: Vec3) -> bool {
    (a - b).abs().cmple(ha + hb).all()
}

/// Kendi kendine koşu: önündeki en yakın engele bakar; şeridinde yüksek bir duvar varsa açık bir
/// şeride geçer, alçaksa zıplar, kirişse kayar.
fn autopilot(world: &World, game: &Game) -> (bool, bool, bool, bool) {
    let mut ahead: Vec<(f32, i32, Kind)> = Vec::new();
    let transforms = world.borrow::<Transform>();
    for (id, o) in world.borrow::<Obstacle>().iter() {
        let Some(t) = transforms.get(id) else { continue };
        let at = t.position;
        let d = game.z - at.z;
        if d > -0.5 && d < 14.0 {
            ahead.push((d, (at.x / LANE_GAP).round() as i32, o.kind));
        }
    }
    ahead.sort_by(|a, b| a.0.total_cmp(&b.0));
    let blocked = |lane: i32, within: f32| ahead.iter().any(|&(d, l, k)| l == lane && d < within && k == Kind::Wall);
    if blocked(game.lane, 12.0) {
        for to in [game.lane - 1, game.lane + 1] {
            if (-1..=1).contains(&to) && !blocked(to, 12.0) {
                return (to < game.lane, to > game.lane, false, false);
            }
        }
    }
    let reach = game.speed * 0.28;
    let next = ahead.iter().find(|&&(d, l, _)| l == game.lane && d > 0.0 && d < reach);
    match next.map(|n| n.2) {
        Some(Kind::Low) => (false, false, true, false),
        Some(Kind::Beam) => (false, false, false, game.slide <= 0.05),
        _ => (false, false, false, false),
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
