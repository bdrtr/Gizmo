//! # Balon — yükselen balonları patlat
//!
//! Renkli balonlar aşağıdan yükseliyor; tıkla, patlat. Kaçan her balon bir can götürür; beş can.
//! Patlayan balon büyüyüp soluyor, parçaları saçılıyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **sol tık** | patlat |
//! | **R** | baştan başlat |
//!
//! ## Neden var
//!
//! 2D yolu (sprite, ortografik kamera) tween, parçacık ve fareyle birlikte kullanan ilk oyun.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Tween yalnızca `Transform` biliyordu.** Patlayan balon büyüyebiliyordu ama
//!   solamıyordu; ilk sürüm sprite'ın saydamlığını kendi zamanlayıcısıyla elle indiriyordu.
//!   **Kapatıldı:** `ColorTween` (`to(renk, s)`, `fade_to(alfa, s)`) — `Sprite`, `Text` ve
//!   `Material` renklerini aynı eğri ve tekrarlarla sürüyor, iki çizim yolunda da.
//! - *Eksik değil:* `Sprite::color` doğrusal (linear) renk; `0.15` sRGB'de `~0.4` görünüyor.
//!   İlk renk seçimi soluk çıktı, ölçünce tutuyordu.
//!
//! ## Ortam değişkenleri
//!   * `GIZMO_FONT=<yol>` — font
//!   * `BALON_OTO=1` — kendiliğinden birkaç balon patlat (ekran görüntüsü için)

use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::core::input::Input;
use gizmo::prelude::*;
use gizmo::renderer::components::{ColorTween, ParticleEmitter, ProjectionMode, Sprite, Text, TextAlign, TextAnchor};
use gizmo::renderer::sprite::{SpriteFilter, SpriteImageId};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, SimpleAppExt, SimpleSceneState};
use gizmo::winit::keyboard::KeyCode;

const VIEW_W: f32 = 16.0;
const VIEW_H: f32 = 10.0;
const RADIUS: f32 = 0.55;

#[derive(Clone, Copy)]
struct Balloon {
    speed: f32,
    sway: f32,
    phase: f32,
}
gizmo::core::impl_component!(Balloon);

/// Patlamış, büyüyüp sönen bir balon — solması bitince silinir.
#[derive(Clone, Copy)]
struct Popping;
gizmo::core::impl_component!(Popping);

struct Game {
    disc: SpriteImageId,
    rng: Rng,
    spawn: Timer,
    score: u32,
    lives: u32,
    hud: Entity,
    banner: Entity,
    auto: u32,
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Balon", 1280, 720)
        .with_simple_scene(|scene, state| {
            let r = &mut *scene.renderer;
            let font = load_font(r);
            let disc = r.load_sprite_rgba(&disc_image(32), 32, 32, SpriteFilter::Linear).expect("disk yüklenir");
            let sky = r.load_sprite_rgba(&sky_image(), 1, 32, SpriteFilter::Linear).expect("gök yüklenir");

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, -5.0)),
                GlobalTransform::default(),
                Sprite::world(sky, Vec2::new(60.0, 40.0)).with_layer(-10),
            ));
            scene.spawn_camera(state, Vec3::new(0.0, 0.0, 20.0), Vec3::ZERO);
            if let Some(mut q) = scene.world.query_mut::<Mut<Camera>>() {
                for mut cam in q.iter_mut() {
                    cam.projection = ProjectionMode::OrthographicFit { width: VIEW_W, height: VIEW_H };
                }
            }

            let hud = scene
                .world
                .spawn_bundle((Text::pinned("", font, 26.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
                ;
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.4), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                ;

            if let Ok(mut audio) = AudioManager::new() {
                audio.load_sound_bytes("pat", Blip::new(Wave::Noise, 2600.0, 0.12).with_slide(300.0).with_volume(0.45).wav());
                audio.load_sound_bytes("kac", Blip::new(Wave::Sine, 500.0, 0.3).with_slide(180.0).with_volume(0.3).wav());
                scene.world.insert_resource(audio);
            }
            scene.world.insert_resource(Game {
                disc,
                rng: Rng::new(99),
                spawn: Timer::repeating(1.1),
                score: 0,
                lives: 5,
                hud,
                banner,
                auto: if std::env::var_os("BALON_OTO").is_some() { 4 } else { 0 },
            });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        world.despawn_all_with::<Balloon>();
        game.score = 0;
        game.lives = 5;
    }
    if game.lives > 0 {
        spawn_balloons(world, &mut game, dt);
        rise(world, &mut game, dt);
        // Tık: imlecin altındaki dünya noktası — ortografik kamerada da `cursor_on_plane`.
        let target = if let Some(p) = (game.auto > 0).then(|| first_balloon(world)).flatten() {
            game.auto -= 1;
            Some(p)
        } else if input.is_mouse_button_just_pressed(0) {
            gizmo::pointer::cursor_on_plane(world, Vec3::ZERO, Vec3::Z)
        } else {
            None
        };
        if let Some(p) = target {
            pop_at(world, &mut game, p);
        }
    }
    clear_pops(world);
    sync_transforms(world);
    draw_hud(world, &game);
    world.insert_resource(game);
}

fn spawn_balloons(world: &mut World, game: &mut Game, dt: f32) {
    // `Timer` keeps the time past each period, so the pace does not depend on the frame rate;
    // the hand-rolled `t -= dt; if t <= 0 { t = …; }` this replaced lost a frame each balloon.
    if game.spawn.tick(dt) == 0 {
        return;
    }
    game.spawn.set_duration((1.1 - game.score as f32 * 0.02).max(0.35));
    const COLOURS: [Vec4; 5] = [
        Vec4::new(0.95, 0.15, 0.2, 1.0),
        Vec4::new(1.0, 0.6, 0.05, 1.0),
        Vec4::new(0.2, 0.75, 0.25, 1.0),
        Vec4::new(0.15, 0.4, 0.95, 1.0),
        Vec4::new(0.7, 0.2, 0.85, 1.0),
    ];
    let colour = *game.rng.pick(&COLOURS).expect("beş renk var");
    let x = game.rng.range(-VIEW_W * 0.45..VIEW_W * 0.45);
    world.spawn_bundle((
        Transform::new(Vec3::new(x, -VIEW_H * 0.5 - RADIUS, 0.0)),
        GlobalTransform::default(),
        Sprite::world(game.disc, Vec2::splat(RADIUS * 2.0)).with_color(colour),
        Balloon { speed: 1.2 + game.score as f32 * 0.05 + game.rng.f32(), sway: game.rng.range(0.2..0.6), phase: game.rng.range(0.0..std::f32::consts::TAU) },
    ));
}

fn rise(world: &mut World, game: &mut Game, dt: f32) {
    let mut escaped = Vec::new();
    if let Some(mut q) = world.query_mut::<(Entity, Mut<Transform>, Mut<Balloon>)>() {
        for (entity, mut t, mut b) in q.iter_mut() {
            b.phase += dt * 2.0;
            t.position.y += b.speed * dt;
            t.position.x += b.phase.sin() * b.sway * dt;
            if t.position.y > VIEW_H * 0.5 + RADIUS {
                escaped.push(entity);
            }
        }
    }
    for id in escaped {
        world.despawn(id);
        game.lives = game.lives.saturating_sub(1);
        play(world, "kac");
    }
}

fn first_balloon(world: &World) -> Option<Vec3> {
    let tfs = world.borrow::<Transform>();
    world.query::<(Entity, &Balloon)>().unwrap().iter().find_map(|(entity, _)| tfs.get(entity).map(|t| t.position).filter(|p| p.y > -VIEW_H * 0.1))
}

/// İmlecin altındaki balon patlar: büyüyüp sönüyor, parçaları saçılıyor.
fn pop_at(world: &mut World, game: &mut Game, p: Vec3) {
    let hit = {
        let tfs = world.borrow::<Transform>();
        world.query::<(Entity, &Balloon)>().unwrap().iter()
            .filter_map(|(entity, _)| tfs.get(entity).map(|t| (entity, t.position)))
            .filter(|(_, at)| Vec2::new(at.x - p.x, at.y - p.y).length() < RADIUS)
            .map(|(id, _)| id)
            .next()
    };
    let Some(id) = hit else { return };
    let Some(e) = world.live(id) else { return };
    let colour = world.borrow::<Sprite>().get(id).map(|s| s.color).unwrap_or(Vec4::ONE);
    world.remove_component::<Balloon>(e);
    world.add_component(e, Tween::to_scale(Vec3::splat(1.8), 0.18).with_ease(Ease::OutQuad));
    world.add_component(e, ColorTween::fade_to(0.0, 0.18));
    world.add_component(e, Popping);
    let mut burst = ParticleEmitter::one_shot(40);
    burst.initial_velocity = Vec3::ZERO;
    burst.velocity_randomness = 4.0;
    burst.lifespan = 0.5;
    burst.lifespan_randomness = 0.2;
    burst.size_start = 0.18;
    burst.size_end = 0.0;
    burst.color_start = colour;
    burst.color_end = Vec4::new(colour.x, colour.y, colour.z, 0.0);
    world.add_component(e, burst);
    game.score += 1;
    play(world, "pat");
}

/// Solması biten patlamış balonları siler.
fn clear_pops(world: &mut World) {
    let done: Vec<Entity> = {
        let fades = world.borrow::<ColorTween>();
        world.query::<(Entity, &Popping)>().unwrap().iter().filter(|(entity, _)| fades.get(*entity).is_some_and(|f| f.is_finished())).map(|(entity, _)| entity).collect()
    };
    for id in done {
        world.despawn(id);
    }
}

fn play(world: &mut World, name: &str) {
    if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
        let _ = audio.play(name);
    }
}

fn draw_hud(world: &mut World, game: &Game) {
    let mut texts = world.borrow_mut::<Text>();
    if let Some(mut t) = texts.get_mut(game.hud) {
        t.content = format!("Patlayan {}   Can {}", game.score, game.lives);
    }
    if let Some(mut t) = texts.get_mut(game.banner) {
        t.content = if game.lives == 0 { format!("Bitti — {} balon\nR ile yeniden", game.score) } else { String::new() };
    }
}

fn disc_image(n: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    let c = (n as f32 - 1.0) * 0.5;
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt() / (n as f32 * 0.5);
            // Beyaz disk, sol üstte bir parlama: renk sprite'ın tonundan geliyor.
            let shine = ((x as f32 - c * 0.6).powi(2) + (y as f32 - c * 0.6).powi(2)).sqrt() < n as f32 * 0.12;
            let a = if d <= 1.0 { 255 } else { 0 };
            let v = if shine { 255 } else { 225 };
            px.extend_from_slice(&[v, v, v, a]);
        }
    }
    px
}

fn sky_image() -> Vec<u8> {
    (0..32)
        .flat_map(|y| {
            let t = y as f32 / 31.0;
            let l = |a: f32, b: f32| (a + (b - a) * t) as u8;
            [l(90.0, 210.0), l(150.0, 230.0), l(220.0, 245.0), 255]
        })
        .collect()
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
