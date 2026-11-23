//! # Tahterevalli — bir ucuna kutu düşen, öbür ucundan top fırlatan bir tahta
//!
//! Bir tahta, statik bir takozun tepesinde bir menteşeyle döner. Sol ucunda, bir kenarın
//! önünde bir top durur ve tahtayı o yana yatırır; sağ uca yukarıdan bir kutu bırakılır ve top
//! havaya fırlar. Tur bitince her şey yerine konur ve yeniden oynanır.
//!
//! Motorun hangi yollarını yürüttüğü, ve onun için yazıldığı:
//!
//! - **Bir menteşe eklemi** (`Joint::hinge`), dinamik bir tahtayla statik bir takoz arasında; bir
//!   darbenin ve bir savrulmanın boyunca açılmamalı, tahta düzleminden çıkmamalı.
//! - **Eklemli bir bileşik gövde**: topu tutan kenar, tahtanın çocuğu olan bir çarpıştırıcıdır.
//! - **Eklem, temas ve uyku birlikte**: dinlenirken tahta bir ucuyla yere, ortasıyla eklemine
//!   dayanır ve uyumalıdır (uyumuyor: aşağıda); düşen kutu onu uyandırır.
//! - **Cisimleri yerine koymak**: tur sonunda tahta ve top `Transform` ve `Velocity`
//!   yazılarak ışınlanır, kutu silinir ve yenisi bırakılır; ikinci tur birincinin aynısı olmalı.
//!
//! Bu oyun yazılırken bulunan: dinlenen tahtanın üstündeki top saniyede 5–6 cm titriyor ve ikisi de
//! hiç uyumuyor. Adım temasları sonuna kadar, eklemleri ondan sonra, birbirini görmeden çözüyor
//! (docs/ENGINE.md §3 madde 8, `joints_with_contacts.rs`). Test uyuyanları yazar, şart koşmaz.
//!
//! `tahterevalli --test`: pencere açmadan iki tur oynar. Her tur dinlenişin geometrisini (tahta
//! ucuyla yerde, top köşesinde), atışı (top yükselir, ama düşen kutunun bıraktığı
//! enerjiden fazla değil) ve menteşeyi (açılmaz, düzlemden çıkmaz) ölçer; iki turun atışı aynı
//! yüksekliğe çıkmalı.
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::physics::joints::Joint;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{BodyHandle, Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};

const PHYSICS_HZ: u32 = 120;
/// Takozun tepesi, menteşenin yeri.
const PIVOT: Vec3 = Vec3::new(0.0, 0.6, 0.0);
/// Tahtanın yarı boyları ve kütlesi; alt yüzünün ortası menteşededir.
const PLANK_HALF: Vec3 = Vec3::new(2.0, 0.05, 0.35);
const PLANK_MASS: f32 = 4.0;
/// Topun yarıçapı ve kütlesi; tahtanın sol ucundaki kenarın önünde durur.
const BALL_R: f32 = 0.15;
const BALL_MASS: f32 = 0.5;
/// Kenar: tahtanın sol ucunda, üst yüzünden yukarı.
const LIP_HALF: Vec3 = Vec3::new(0.05, 0.12, 0.35);
/// Düşen kutu: yarı boyu, kütlesi, bırakıldığı yer.
const BOX_HALF: f32 = 0.25;
const BOX_MASS: f32 = 3.0;
const BOX_FROM: Vec3 = Vec3::new(1.7, 3.5, 0.0);
/// Bir turun takvimi, saniye: yerleşme, kutunun bırakılışı, turun sonu.
const DROP_AT: f32 = 3.0;
const ROUND: f32 = 9.0;

#[derive(Default, Clone, Copy, Debug)]
struct RoundLog {
    /// Kutu bırakılmadan hemen önce: tahtanın açısı (radyan, sol uç aşağı artı), topun tahtaya
    /// göre yeri, uyuyanların sayısı.
    rest_tilt: f32,
    rest_ball: Vec3,
    asleep: usize,
    /// Kutu bırakıldıktan sonra: topun en yükseği, kutunun en alçağı (merkezleri, y).
    ball_peak: f32,
    box_low: f32,
    /// Bütün tur boyunca: menteşenin iki ucunun en çok ayrılışı, tahtanın ekseninin z'den en
    /// çok sapışı.
    hinge_gap: f32,
    off_plane: f32,
}

struct Game {
    physics: FixedPhysics,
    plank: Entity,
    ball: Entity,
    falling: Option<Entity>,
    clock: f32,
    round: usize,
    logs: Vec<RoundLog>,
    hud: Entity,
}

fn plank_start() -> Transform {
    Transform::new(PIVOT + Vec3::Y * PLANK_HALF.y)
}

/// Topun yeri, tahtanın kendi çerçevesinde: kenarın iç yüzüne ve üst yüzüne değerek.
fn ball_on_plank() -> Vec3 {
    Vec3::new(-PLANK_HALF.x + 2.0 * LIP_HALF.x + BALL_R, PLANK_HALF.y + BALL_R, 0.0)
}

fn ball_start() -> Transform {
    Transform::new(plank_start().position + ball_on_plank()).with_scale(Vec3::splat(BALL_R))
}

/// Dinlenen tahtanın açısı: sol alt köşesi yerde. Menteşe tahtanın alt yüzündedir, köşe ondan
/// `PLANK_HALF.x` uzakta: `sin θ = PIVOT.y / PLANK_HALF.x`.
fn rest_tilt() -> f32 {
    (PIVOT.y / PLANK_HALF.x).asin()
}

fn setup(world: &mut World, font: FontId) -> Game {
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.9)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 3.0, 9.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.15,
        primary: true,
        ..Default::default()
    });
    let floor_half = Vec3::new(15.0, 0.1, 15.0);
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.1, 0.0)).with_scale(floor_half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.35, 0.45, 0.3, 1.0), 0.9, 0.0),
        MeshRenderer::new(),
        Collider::box_collider(floor_half),
    ));
    let block_half = Vec3::new(0.15, PIVOT.y * 0.5, 0.3);
    let block = world.spawn_bundle((
        Transform::new(Vec3::Y * block_half.y).with_scale(block_half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.4, 0.4, 0.45, 1.0), 0.7, 0.0),
        MeshRenderer::new(),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(block_half),
    ));
    let plank = world.spawn_bundle((
        plank_start(),
        GlobalTransform::default(),
        RigidBody::new(PLANK_MASS, true),
        Velocity::default(),
        Collider::box_collider(PLANK_HALF),
    ));
    // Tahtanın görünüşü ve kenarı onun çocukları: görünüş ölçekli bir küp, kenar bir çarpıştırıcı.
    let wood = world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_scale(PLANK_HALF),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.7, 0.5, 0.3, 1.0), 0.6, 0.0),
        MeshRenderer::new(),
    ));
    world.add_child(plank, wood);
    let lip = world.spawn_bundle((
        Transform::new(Vec3::new(-PLANK_HALF.x + LIP_HALF.x, PLANK_HALF.y + LIP_HALF.y, 0.0)).with_scale(LIP_HALF),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.5, 0.33, 0.2, 1.0), 0.6, 0.0),
        MeshRenderer::new(),
        // Kendi ölçeği yalnız görünüşüne: çarpıştırıcı kendi boyuyla yazılır.
        Collider::box_collider(LIP_HALF),
    ));
    world.add_child(plank, lip);
    if let Some(mut physics) = world.get_resource_mut::<PhysicsWorld>() {
        physics.joints.push(Joint::hinge(
            BodyHandle::from_id(plank.id()),
            BodyHandle::from_id(block.id()),
            Vec3::new(0.0, -PLANK_HALF.y, 0.0),
            Vec3::new(0.0, block_half.y, 0.0),
            Vec3::Z,
        ));
    }
    let ball = world.spawn_bundle((
        ball_start(),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.9, 0.25, 0.2, 1.0), 0.3, 0.1),
        MeshRenderer::new(),
        RigidBody::new(BALL_MASS, true),
        Velocity::default(),
        Collider::sphere(BALL_R),
    ));
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    Game {
        physics: FixedPhysics::new(PHYSICS_HZ),
        plank,
        ball,
        falling: None,
        clock: 0.0,
        round: 0,
        logs: vec![RoundLog::default()],
        hud,
    }
}

fn drop_box(world: &mut World) -> Entity {
    world.spawn_bundle((
        Transform::new(BOX_FROM).with_scale(Vec3::splat(BOX_HALF)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.25, 0.35, 0.8, 1.0), 0.5, 0.0),
        MeshRenderer::new(),
        RigidBody::new(BOX_MASS, true),
        Velocity::default(),
        Collider::box_collider(Vec3::splat(BOX_HALF)),
    ))
}

/// Turun sonu: tahta ve top ışınlanır, kutu silinir.
fn reset(world: &mut World, game: &mut Game) {
    for (e, start) in [(game.plank, plank_start()), (game.ball, ball_start())] {
        if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(e) {
            *t = start;
        }
        if let Some(mut v) = world.query_mut::<&mut Velocity>().get_mut(e) {
            *v = Velocity::default();
        }
    }
    if let Some(e) = game.falling.take() {
        world.despawn(e);
    }
}

fn measure(world: &World, game: &mut Game) {
    let (Some(plank), Some(ball)) = (
        world.query::<&Transform>().get(game.plank).copied(),
        world.query::<&Transform>().get(game.ball).copied(),
    ) else {
        return;
    };
    let log = game.logs.last_mut().expect("bu turun kaydı");
    let anchor = plank.position + plank.rotation * Vec3::new(0.0, -PLANK_HALF.y, 0.0);
    log.hinge_gap = log.hinge_gap.max(anchor.distance(PIVOT));
    log.off_plane = log.off_plane.max(1.0 - (plank.rotation * Vec3::Z).z).max(plank.position.z.abs());
    if game.falling.is_none() {
        let (axis, angle) = plank.rotation.to_axis_angle();
        log.rest_tilt = angle * axis.z.signum();
        log.rest_ball = plank.rotation.inverse() * (ball.position - plank.position);
        log.asleep = [game.plank, game.ball].iter().filter(|&&e| world.query::<&RigidBody>().get(e).is_some_and(|rb| rb.is_sleeping)).count();
    } else {
        log.ball_peak = log.ball_peak.max(ball.position.y);
        if let Some(b) = game.falling.and_then(|e| world.query::<&Transform>().get(e).copied()) {
            log.box_low = log.box_low.min(b.position.y);
        }
    }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, _input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.clock += dt;
    if game.falling.is_none() && game.clock >= DROP_AT {
        measure(world, &mut game);
        game.falling = Some(drop_box(world));
        if let Some(log) = game.logs.last_mut() {
            log.box_low = BOX_FROM.y;
        }
    }
    if game.clock >= ROUND {
        reset(world, &mut game);
        game.clock = 0.0;
        game.round += 1;
        game.logs.push(RoundLog::default());
    }
    game.physics.advance(world, dt);
    sync_transforms(world);
    measure(world, &mut game);
    let peak = game.logs.last().map_or(0.0, |l| l.ball_peak - ball_start().position.y);
    if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        text.content = format!("Tur {}   Topun yükselişi: {:.2} m", game.round + 1, peak.max(0.0));
    }
    world.insert_resource(game);
}

/// Bir turun kaydını sınar; neyin tuttuğunu ve tutmadığını yazar.
fn check(n: usize, log: &RoundLog) -> bool {
    // Dinlenirken: tahtanın sol ucu yerde; top, kenarla üst yüzün köşesinde. Uyumaları gerekirdi ama
    // uyumuyorlar — eklemle temas ayrı geçişlerde çözülüyor, top tahtanın üstünde titriyor
    // (docs/ENGINE.md §3 madde 8). Sayıları yazılır, şart koşulmaz; o madde kapanınca koşulur.
    let tilt_ok = (log.rest_tilt - rest_tilt()).abs() < 0.5f32.to_radians();
    let ball_off = (log.rest_ball - ball_on_plank()).length();
    let rest_ok = tilt_ok && ball_off < 0.02;
    // Atış: top bir metreden fazla yükselir, ama kutunun bıraktığı potansiyel enerjiden fazla değil.
    // Tahtanın kendi enerjisi tur başında ve kutu dinlendiğinde aynıdır (simetrik, ucu yerde).
    let ball_rest_y = PIVOT.y + Quat::from_rotation_z(rest_tilt()).mul_vec3(ball_on_plank() + Vec3::Y * PLANK_HALF.y).y;
    let gained = BALL_MASS * (log.ball_peak - ball_rest_y);
    let given = BOX_MASS * (BOX_FROM.y - log.box_low);
    let launch_ok = log.ball_peak - ball_rest_y > 1.0 && gained <= given;
    let hinge_ok = log.hinge_gap < 0.01 && log.off_plane < 1e-3;
    println!(
        "  tur {n}: tahta {:.2}° (geometri {:.2}°), top köşesinden {:.3} m, {}/2 uyuyor (ENGINE §3 madde 8); top {:.2} m yükseldi \
         ({gained:.1} ≤ {given:.1} kg·m); menteşe en çok {:.4} m açıldı, düzlemden {:.1e} saptı",
        log.rest_tilt.to_degrees(),
        rest_tilt().to_degrees(),
        ball_off,
        log.asleep,
        log.ball_peak - ball_rest_y,
        log.hinge_gap,
        log.off_plane,
    );
    rest_ok && launch_ok && hinge_ok
}

/// `--test`: iki tur. Her biri `check`'ten geçmeli, ve ikinci turun atışı birincininkiyle aynı
/// yüksekliğe (2 cm içinde) çıkmalı.
fn self_test() -> bool {
    let mut world = World::new();
    let game = setup(&mut world, HeadlessFrames::font());
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    while run.world.get_resource::<Game>().is_some_and(|g| g.round < 2) {
        run.frame(dt, update);
    }
    let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
    println!("tahterevalli --test:");
    let rounds_ok = game.logs[..2].iter().enumerate().map(|(i, l)| check(i + 1, l)).fold(true, |a, b| a & b);
    let same = (game.logs[0].ball_peak - game.logs[1].ball_peak).abs();
    let ok = rounds_ok && same < 0.02;
    println!("  iki turun atışı {same:.4} m farklı → {}", if ok { "tamam" } else { "BAŞARISIZ" });
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Tahterevalli", 1280, 720)
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
