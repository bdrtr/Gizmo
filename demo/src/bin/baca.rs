//! # Baca — tek yönlü çıkıntılardan tepeye, tepeden içlerinden dibe
//!
//! Bir bacanın içinde üst üste sekiz çıkıntı var, her biri tek yönlü: alttan zıplayınca içinden
//! geçilir, üstüne inilir. Her çıkıntının üstünde bir jeton durur. Tepeye çık, jetonları topla,
//! sonra "aşağı in" ile çıkıntıların içinden dibe dön.
//!
//! Motorun hangi yollarını yürüttüğü — oyunların hiçbirinin kullanmadığı **tek yönlü
//! platformlar**:
//!
//! - **Çıkıntılar `Collider::one_way(Vec3::Y)`**, `RigidBody`'siz: karakter denetleyicisi alttan
//!   içlerinden geçer, üstlerine iner, `drop_through_timer` ile bırakır.
//! - **Karakter `CharacterController`**, gövdesiz; zıplama `jump_buffer_timer` ile istenir.
//! - **Jetonlar yalnız çarpıştırıcı tetikler**, `CharacterController::triggers`'tan okunur:
//!   gövdesi olmayan bir karakteri fizik dünyasının olayları anmaz.
//!
//! `baca --test`: otomatik pilot sekiz çıkıntıyı tırmanır, sekiz jetonu toplar, sonra her çıkıntının
//! içinden bir bir dibe iner; karakter düzlemden ayrılmamalı. Sonra bir top bacanın dibinden yukarı
//! atılır: rigid bir gövde de tek yönlü çıkıntıların içinden geçmeli ve tepesinin altında kalan en
//! yüksek çıkıntıya inmeli.
//!
//! **Tuşlar:** SPACE zıpla · S + SPACE aşağı in · A D yürü.
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::physics::components::CharacterController;
use gizmo::physics::{Collider, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};

const LEDGES: usize = 8;
const LEDGE_GAP: f32 = 2.0;
const LEDGE_HALF: Vec3 = Vec3::new(2.4, 0.15, 0.5);
const HALF_WIDTH: f32 = 2.5;
/// Karakterin kapsülü: yarıçap ve silindir yarı boyu; ayakları merkezin bu kadar altında.
const RADIUS: f32 = 0.3;
const HALF_HEIGHT: f32 = 0.5;
const FEET: f32 = RADIUS + HALF_HEIGHT;

#[derive(Clone, Copy)]
struct Coin;
gizmo::core::impl_component!(Coin);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Climb,
    Wait(f32),
    Descend,
    Done,
}

struct Game {
    physics: FixedPhysics,
    player: Entity,
    coins: usize,
    /// En yüksek, üstünde durulmuş çıkıntı (0 = zemin).
    highest: usize,
    phase: Phase,
    autopilot: bool,
    hud: Entity,
}

/// Çıkıntı `k`'nin üst yüzü (`k` 1'den; 0 zemin).
fn ledge_top(k: usize) -> f32 {
    k as f32 * LEDGE_GAP + LEDGE_HALF.y
}

/// Karakterin ayaklarının hangi çıkıntının üstünde olduğu, durduğu yükseklikten.
fn standing_on(y: f32) -> usize {
    (0..=LEDGES).min_by(|&a, &b| {
        let top = |k: usize| if k == 0 { 0.0 } else { ledge_top(k) };
        (top(a) - (y - FEET)).abs().total_cmp(&(top(b) - (y - FEET)).abs())
    })
    .unwrap_or(0)
}

fn solid(world: &mut World, at: Vec3, half: Vec3, colour: Vec4, collider: Collider) -> Entity {
    world.spawn_bundle((
        Transform::new(at).with_scale(half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(colour, 0.7, 0.0),
        MeshRenderer::new(),
        collider,
    ))
}

fn build_chimney(world: &mut World) {
    let stone = Vec4::new(0.45, 0.42, 0.4, 1.0);
    let height = (LEDGES as f32 + 2.0) * LEDGE_GAP;
    solid(world, Vec3::new(0.0, -0.25, 0.0), Vec3::new(HALF_WIDTH + 0.3, 0.25, 0.6), stone, Collider::box_collider(Vec3::new(HALF_WIDTH + 0.3, 0.25, 0.6)));
    for side in [-1.0, 1.0] {
        let half = Vec3::new(0.15, height / 2.0, 0.6);
        solid(world, Vec3::new(side * (HALF_WIDTH + 0.15), height / 2.0, 0.0), half, stone, Collider::box_collider(half));
    }
    let wood = Vec4::new(0.6, 0.42, 0.25, 1.0);
    let gold = Vec4::new(0.95, 0.8, 0.2, 1.0);
    for k in 1..=LEDGES {
        let y = k as f32 * LEDGE_GAP;
        solid(world, Vec3::new(0.0, y, 0.0), LEDGE_HALF, wood, Collider::box_collider(LEDGE_HALF).one_way(Vec3::Y));
        world.spawn_bundle((
            Transform::new(Vec3::new(0.0, ledge_top(k) + FEET, 0.0)).with_scale(Vec3::splat(0.25)),
            GlobalTransform::default(),
            MeshSource("sphere".into()),
            MaterialDesc::new().with_pbr(gold, 0.3, 0.9),
            MeshRenderer::new(),
            Collider::sphere(0.25).with_trigger(true),
            Coin,
        ));
    }
}

fn setup(world: &mut World, font: FontId) -> Game {
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.3) * Quat::from_rotation_x(-0.7)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 9.0, 24.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: 0.0,
        primary: true,
        ..Default::default()
    });
    build_chimney(world);
    let kcc = CharacterController { gravity: 20.0, jump_speed: 10.0, ..Default::default() };
    let player = world.spawn_bundle((
        Transform::new(Vec3::new(0.0, FEET + 0.05, 0.0)),
        GlobalTransform::default(),
        Velocity::default(),
        Collider::capsule(RADIUS, HALF_HEIGHT),
        kcc,
    ));
    let body = world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_scale(Vec3::new(RADIUS, FEET, RADIUS)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.2, 0.55, 0.85, 1.0), 0.4, 0.0),
        MeshRenderer::new(),
    ));
    world.add_child(player, body);
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    Game {
        physics: FixedPhysics::new(120),
        player,
        coins: 0,
        highest: 0,
        phase: Phase::Climb,
        autopilot: std::env::var_os("BACA_OTO").is_some(),
        hud,
    }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    let (y, grounded) = {
        let t = world.query::<&Transform>().get(game.player).map_or(0.0, |t| t.position.y);
        (t, world.query::<&CharacterController>().get(game.player).is_some_and(|k| k.is_grounded))
    };
    let on = standing_on(y);
    if grounded {
        game.highest = game.highest.max(on);
    }

    // Ne istendiği: otomatik pilottan ya da tuşlardan.
    let (walk, jump, drop) = if game.autopilot {
        match game.phase {
            Phase::Climb if grounded && on == LEDGES => {
                game.phase = Phase::Wait(0.5);
                (0.0, false, false)
            }
            Phase::Climb => (0.0, grounded, false),
            Phase::Wait(t) => {
                game.phase = if t <= dt { Phase::Descend } else { Phase::Wait(t - dt) };
                (0.0, false, false)
            }
            Phase::Descend if grounded && on == 0 => {
                game.phase = Phase::Done;
                (0.0, false, false)
            }
            Phase::Descend => (0.0, false, grounded),
            Phase::Done => (0.0, false, false),
        }
    } else {
        let (mx, my) = input.move_axis();
        let space = input.is_key_just_pressed(KeyCode::Space as u32);
        (mx, space && my >= -0.5, space && my < -0.5)
    };
    if let Some(mut kcc) = world.query_mut::<&mut CharacterController>().get_mut(game.player) {
        kcc.target_velocity = Vec3::new(walk * kcc.speed, 0.0, 0.0);
        if jump {
            kcc.jump_buffer_timer = kcc.jump_buffer_time;
        }
        if drop {
            kcc.drop_through_timer = 0.1;
        }
    }

    game.physics.advance(world, dt);
    sync_transforms(world);

    // Jetonlar: karakterin içinde durduğu tetikler.
    let touched: Vec<u32> = world.query::<&CharacterController>().get(game.player).map(|k| k.triggers.iter().map(|h| h.id()).collect()).unwrap_or_default();
    let coins: Vec<Entity> = world.query::<(Entity, &Coin)>().iter().filter(|(e, _)| touched.contains(&e.id())).map(|(e, _)| e).collect();
    for coin in coins {
        world.despawn(coin);
        game.coins += 1;
    }

    if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        text.content = format!("Jeton {} / {LEDGES}   Kat {} / {LEDGES}", game.coins, game.highest);
    }
    world.insert_resource(game);
}

/// `--test`: otomatik pilot tırmanır, toplar, iner.
fn self_test() -> bool {
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font());
    game.autopilot = true;
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    let mut off_plane = 0.0f32;
    while run.elapsed < 90.0 {
        run.frame(dt, update);
        let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
        let z = run.world.query::<&Transform>().get(game.player).map_or(0.0, |t| t.position.z);
        off_plane = off_plane.max(z.abs());
        if game.phase == Phase::Done {
            break;
        }
    }
    let (highest, coins, done) = {
        let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
        (game.highest, game.coins, game.phase == Phase::Done)
    };
    let player = run.world.get_resource::<Game>().map(|g| g.player).expect("oyun kaynağı");
    let y = run.world.query::<&Transform>().get(player).map_or(f32::NAN, |t| t.position.y);
    let back_down = done && standing_on(y) == 0;

    // Bir de rigid bir gövde: bacanın dibinden yukarı atılan top, tek yönlü çıkıntıların içinden
    // geçer ve tepesinin altında kalan en yüksek çıkıntıya iner — dördüncüsüne (tepe ~9 m).
    const BALL_R: f32 = 0.2;
    let ball = run.world.spawn_bundle((
        Transform::new(Vec3::new(1.5, BALL_R + 0.05, 0.0)),
        GlobalTransform::default(),
        gizmo::physics::RigidBody::new(0.5, true).planar_xy(),
        Velocity::new(Vec3::new(0.0, (2.0 * 9.81 * 8.8f32).sqrt(), 0.0)),
        Collider::sphere(BALL_R),
    ));
    for _ in 0..240 {
        run.frame(dt, update);
    }
    let ball_y = run.world.query::<&Transform>().get(ball).map_or(f32::NAN, |t| t.position.y);
    let ball_on_fourth = (ball_y - (ledge_top(4) + BALL_R)).abs() < 0.05;

    let ok = highest == LEDGES && coins == LEDGES && back_down && off_plane < 1e-3 && ball_on_fourth;
    println!(
        "baca --test: en yüksek kat {highest} / {LEDGES}, jeton {coins} / {LEDGES}, dibe döndü {back_down} (y {y:.2}), düzlemden en çok {off_plane:.2e}; atılan top y {ball_y:.2} (4. çıkıntının üstü {:.2}), {:.0} s → {}",
        ledge_top(4) + BALL_R,
        run.elapsed,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Baca", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    // Makinenin bir yazı tipi (`GIZMO_FONT` ya da platformun olağan yüzleri); yoksa sentetik test
    // yüzü, ki oyun yine çalışsın.
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
