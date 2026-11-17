//! # Tırman — zaman çizelgesinde canlandırılmış platformlarla bir parkur
//!
//! Yandan bakan bir parkur: çukurun üstünde gidip gelen bir platform, yukarıdaki çıkıntıya çıkan
//! bir asansör, sonda bayrak. Platformlar elle yazılmış bir hareket kodu değil, motorun **keyed
//! animasyonu**: her biri bir `KeyedAnimationPlayer` ve bir konum track'i — editörün ANIMATION
//! sekmesinde ◆ ile anahtarlanan şeyin aynısı. Yolun kenarındaki lamba da öyle yanıp sönüyor, bir
//! property track'iyle (`PointLight.intensity`). Motorun kırk birinci oyunu ve keyed animasyonu
//! kullanan ilki.
//!
//! | girdi | iş |
//! |---|---|
//! | **A D · sol çubuk** | yürü |
//! | **R** | baştan |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Canlandırılmış platform üstündekini taşımıyordu.** Keyed animasyon platformun
//!   `Transform`'unu yazıyor; fizik ise kinematik bir gövdeyi yalnız `Velocity`'siyle hareket eden
//!   bir şey sayıyordu, ve karakteri taşıyan kod (`platform_carry`) zeminin hızına bakıyordu. Hızı
//!   olmayan platform her karede bir sonraki yerine atlıyor, karakter olduğu yerde kalıp çukura
//!   düşüyordu. **Düzeltildi:** dışarıdan `Transform`'u değiştirilen kinematik bir gövde, fizik
//!   adımında son bıraktığı yerden yeni yerine o adımın hızıyla taşınıyor (`KinematicTarget` gibi;
//!   `RigidBody::pose_driven`), varınca duruyor; `FixedPhysics` bir karedeki birden çok adıma
//!   bölüyor. Animasyon, script ya da oyun kodu — kim yazarsa yazsın.
//!
//! `--test` parkuru iki kez oynar: kurulduğu gibi, ve motorun sahne biçiminde kaydedilip temiz bir
//! dünyaya açılınca — klipler, kinematik platformlar ve karakter dosyadan gelir. İkisi aynı: 16,8 s,
//! 3,80 m taşınma, düşüş yok.
//!
//! ## Ortam değişkenleri
//!   * `TIRMAN_OTO=1` — oyunu kendisi oynar (`tirman --test` aynısını pencere açmadan yapar)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::core::input::Input;
use gizmo::physics::components::CharacterController;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{PointLight, Text, TextAlign, TextAnchor};
use gizmo::renderer::keyed_animation::{AnimationClip, Keyframes, KeyedAnimationPlayer, Track};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};
use std::sync::Arc;

/// Başlangıç zemini x ∈ [-3, 6], çukur (6, 12), karşı zemin [12, 16], asansör x ∈ [14.5, 16] (çıkıntıya
/// dayalı), çıkıntı x ∈ [16, 24], y = 4'te; bayrak x = 22'de.
const START: Vec3 = Vec3::new(-1.0, 1.0, 0.0);
const GOAL_X: f32 = 22.0;
const LEDGE_Y: f32 = 4.0;
/// Çukurun üstündeki platformun gidip geldiği iki uç (merkez x), ve bir tam turun süresi.
const FERRY: (f32, f32, f32) = (7.0, 11.0, 6.0);
/// Asansörün iki ucu (üst yüzeyinin y'si) ve bir tam turun süresi.
const LIFT: (f32, f32, f32) = (0.0, LEDGE_Y, 6.0);
const LIFT_X: f32 = 15.25;

struct Game {
    physics: FixedPhysics,
    player: Entity,
    ferry: Entity,
    lift: Entity,
    camera: Entity,
    hud: Entity,
    banner: Entity,
    won: bool,
    falls: u32,
    auto: bool,
    /// Otomatik pilotun bulunduğu adım.
    plan: Plan,
    /// Feribota bindiği x; feribotta yürümeden durduğu sürede taşındığı yol, ve süre.
    ride_from: f32,
    ridden: f32,
    ride_time: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Plan {
    ToEdge,
    WaitFerry,
    Board,
    Ride,
    ToLift,
    Lift,
    ToGoal,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Actor;
gizmo::core::impl_component!(Actor);

/// Bir kutu: görünüşü ve (`body` ise) çarpıştırıcısı. `centre`/`half` dünya biriminde.
fn block(world: &mut World, centre: Vec3, half: Vec3, colour: &str, body: Option<RigidBody>) -> Entity {
    let e = world.spawn_bundle((
        Transform::new(centre).with_scale(half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Color::srgb_hex(colour).to_vec4(), 0.8, 0.0),
        MeshRenderer::new(),
    ));
    if let Some(body) = body {
        world.add_component(e, body);
        world.add_component(e, Velocity::default());
        world.add_component(e, Collider::box_collider(half));
    }
    e
}

/// `entity`'yi `from`'dan `to`'ya ve geri, `period` saniyede bir tur götüren bir klip — keyed
/// animasyonun konum track'i, kendi varlığında (boş hedef adı). Her uçta turun beşte biri bekler
/// (aynı değerde iki anahtar): binecek ve inecek zaman.
fn shuttle(world: &mut World, entity: Entity, from: Vec3, to: Vec3, period: f32, name: &str) {
    let (dwell, travel) = (period * 0.2, period * 0.3);
    let times = vec![0.0, dwell, dwell + travel, 2.0 * dwell + travel, period];
    let track = Track::new("", times, Keyframes::Translation(vec![from, from, to, to, from])).expect("geçerli track");
    let mut clip = AnimationClip::default();
    clip.name = name.into();
    clip.tracks.push(track);
    let mut player = KeyedAnimationPlayer::new();
    player.play(Arc::new(clip));
    world.add_component(entity, player);
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
fn setup(world: &mut World, font: FontId) -> Game {
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.9)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.95, 0.88), 1.6, LightRole::Sun),
    ));
    let camera = world.spawn_bundle(CameraBundle {
        position: Vec3::new(START.x, 2.5, 11.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.12,
        primary: true,
        ..Default::default()
    });

    let ground = "#4A4E55";
    // Zeminler (üst yüzeyleri y = 0) ve çıkıntı (üst yüzeyi y = 4).
    block(world, Vec3::new(1.5, -0.5, 0.0), Vec3::new(4.5, 0.5, 1.5), ground, Some(RigidBody::new_static()));
    block(world, Vec3::new(14.0, -0.5, 0.0), Vec3::new(2.0, 0.5, 1.5), ground, Some(RigidBody::new_static()));
    block(world, Vec3::new(20.0, LEDGE_Y - 0.5, 0.0), Vec3::new(4.0, 0.5, 1.5), ground, Some(RigidBody::new_static()));
    // Çıkıntının altındaki duvar: yürüyerek değil, asansörle çıkılır.
    block(world, Vec3::new(16.5, 1.5, 0.0), Vec3::new(0.5, 2.0, 1.5), "#5C5048", Some(RigidBody::new_static()));
    // Çukurun dibi: oraya düşen baştan başlar (bkz. `update`).
    block(world, Vec3::new(9.0, -6.0, 0.0), Vec3::new(3.0, 0.2, 1.5), "#8B1A1A", None);

    // Çukurun üstündeki feribot ve asansör: kinematik gövdeler, keyed animasyonla.
    let ferry_y = -0.25;
    let ferry = block(world, Vec3::new(FERRY.0, ferry_y, 0.0), Vec3::new(1.0, 0.25, 1.2), "#D4A017", Some(RigidBody::new_kinematic()));
    world.add_component(ferry, gizmo::core::EntityName::new("feribot"));
    shuttle(world, ferry, Vec3::new(FERRY.0, ferry_y, 0.0), Vec3::new(FERRY.1, ferry_y, 0.0), FERRY.2, "feribot");
    let lift_at = |top: f32| Vec3::new(LIFT_X, top - 0.25, 0.0);
    let lift = block(world, lift_at(LIFT.0), Vec3::new(0.75, 0.25, 1.2), "#2E86C1", Some(RigidBody::new_kinematic()));
    world.add_component(lift, gizmo::core::EntityName::new("asansör"));
    shuttle(world, lift, lift_at(LIFT.0), lift_at(LIFT.1), LIFT.2, "asansör");

    // Bayrak ve yanıp sönen lamba (property track).
    block(world, Vec3::new(GOAL_X, LEDGE_Y + 0.75, 0.0), Vec3::new(0.06, 0.75, 0.06), "#DDDDDD", None);
    block(world, Vec3::new(GOAL_X + 0.35, LEDGE_Y + 1.25, 0.0), Vec3::new(0.3, 0.2, 0.03), "#3CB371", None);
    let lamp = world.spawn_bundle((
        Transform::new(Vec3::new(GOAL_X, LEDGE_Y + 2.0, 1.0)),
        GlobalTransform::default(),
        PointLight::new(Vec3::new(1.0, 0.8, 0.4), 2.0, 6.0),
    ));
    let pulse = Track::property("", "PointLight.intensity", vec![0.0, 0.5, 1.0], Keyframes::Scalar(vec![1.0, 8.0, 1.0])).expect("geçerli track");
    let mut clip = AnimationClip::default();
    clip.name = "lamba".into();
    clip.tracks.push(pulse);
    let mut player = KeyedAnimationPlayer::new();
    player.play(Arc::new(clip));
    world.add_component(lamp, player);

    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)), gizmo::core::EntityName::new("hud")));
    let banner = world.spawn_bundle((
        Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO).with_anchor(TextAnchor::Center).with_align(TextAlign::Center),
        gizmo::core::EntityName::new("pankart"),
    ));
    let mut game = Game {
        physics: FixedPhysics::new(60),
        player: Entity::new(0, 0),
        ferry,
        lift,
        camera,
        hud,
        banner,
        won: false,
        falls: 0,
        auto: std::env::var_os("TIRMAN_OTO").is_some(),
        plan: Plan::ToEdge,
        ride_from: 0.0,
        ridden: 0.0,
        ride_time: 0.0,
    };
    restart(world, &mut game);
    game
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Tırman", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// `--test`: pencere açmadan otomatik pilot parkuru bitirir — feribotu bekler, üstünde hiç
/// yürümeden karşıya taşınır, asansörle çıkıntıya çıkar, bayrağa varır. Oyunun kendi `setup`'ı ve
/// `update`'i, boş bir girdiyle, sabit adımla; motorun keyed animasyonu `HeadlessFrames`'in karesinde.
fn self_test() -> bool {
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font());
    game.auto = true;
    world.insert_resource(game);
    let played = play(world, "kurulduğu gibi");
    // The same course saved in the engine's scene format and opened in a fresh world: the clips,
    // the kinematic platforms and the character come back from the file, and play the same.
    let reopened = reopen().is_some_and(|world| play(world, "kaydedilip açılınca"));
    played && reopened
}

/// The course as `setup` builds it, saved to a scene file and loaded into a fresh world, with the
/// game's state found again by the entities' names.
fn reopen() -> Option<World> {
    let mut registry = gizmo::full_scene_registry();
    registry.register_serializable::<Actor>("Actor").expect("'Actor' motorun bir adıyla çakışmamalı");
    let mut world = World::new();
    let game = setup(&mut world, HeadlessFrames::font());
    let path = std::env::temp_dir().join(format!("tirman-{}.scene", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    if let Err(e) = gizmo::scene::SceneData::save(&world, &path, &registry) {
        println!("tirman --test: kaydedilemedi: {e}");
        return None;
    }
    let mut world = World::new();
    let loaded = gizmo::scene::SceneData::load_into(&path, &mut world, &registry);
    let _ = std::fs::remove_file(&path);
    if let Err(e) = loaded {
        println!("tirman --test: açılamadı: {e}");
        return None;
    }
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let named = |world: &World, name: &str| {
        world.query::<(Entity, &gizmo::core::EntityName)>().iter().find(|(_, n)| n.0 == name).map(|(e, _)| e)
    };
    let found = (named(&world, "oyuncu"), named(&world, "feribot"), named(&world, "asansör"), named(&world, "hud"), named(&world, "pankart"));
    let (Some(player), Some(ferry), Some(lift), Some(hud), Some(banner)) = found else {
        println!("tirman --test: açılan sahnede bir varlık eksik: {found:?}");
        return None;
    };
    let camera = world.query::<(Entity, &gizmo::renderer::components::Camera)>().iter().next().map(|(e, _)| e)?;
    world.insert_resource(Game { player, ferry, lift, camera, hud, banner, auto: true, physics: FixedPhysics::new(60), ..game });
    Some(world)
}

/// Plays `world`'s course with the autopilot until the flag or a minute, and says how it went.
fn play(world: World, how: &str) -> bool {
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    let mut time = 0.0f32;
    while time < 60.0 {
        time += dt;
        run.frame(dt, update);
        if run.world.get_resource::<Game>().is_some_and(|g| g.won) {
            break;
        }
    }
    let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
    // Feribotta yürümeden durduğu sürede karakterin gittiği yol: taşındıysa feribotun gittiği kadar.
    let carried = game.ridden > 2.0;
    let ok = game.won && game.falls == 0 && carried;
    println!(
        "tirman --test ({how}): {time:.1} s; feribotta {:.1} s durup {:.2} m taşındı; düşüş {}; bayrağa vardı mı: {} → {}",
        game.ride_time,
        game.ridden,
        game.falls,
        if game.won { "evet" } else { "HAYIR" },
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn restart(world: &mut World, game: &mut Game) {
    let actors: Vec<Entity> = world.query::<(Entity, &Actor)>().iter().map(|(entity, _)| entity).collect();
    for id in actors {
        if let Some(e) = world.live(id) {
            world.despawn(e);
        }
    }
    game.player = world.spawn_bundle((
        Transform::new(START).with_scale(Vec3::new(0.3, 0.5, 0.3)),
        GlobalTransform::default(),
        MeshSource("cylinder".into()),
        MaterialDesc::new().with_pbr(Color::srgb_hex("#E74C3C").to_vec4(), 0.5, 0.0),
        MeshRenderer::new(),
        CharacterController { speed: 3.5, ..Default::default() },
        Velocity::default(),
        Collider::capsule(0.3, 0.2),
        Actor,
        gizmo::core::EntityName::new("oyuncu"),
    ));
    game.won = false;
    game.plan = Plan::ToEdge;
    game.physics.reset();
}

fn position(world: &World, e: Entity) -> Vec3 {
    world.query::<&Transform>().get(e).map_or(Vec3::ZERO, |t| t.position)
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        restart(world, &mut game);
    }
    if !game.won {
        let walk = if game.auto { auto_walk(world, &mut game, dt) } else { input.move_axis().0 };
        if let Some(mut c) = world.query_mut::<&mut CharacterController>().get_mut(game.player) {
            c.target_velocity = Vec3::new(walk * c.speed, 0.0, 0.0);
        }
        game.physics.advance(world, dt);
        sync_transforms(world);
        let p = position(world, game.player);
        if p.y < -3.0 {
            game.falls += 1;
            restart(world, &mut game);
        } else if p.x > GOAL_X - 0.5 && p.y > LEDGE_Y - 0.5 {
            game.won = true;
        }
    }
    // Kamera karakteri izler.
    let p = position(world, game.player);
    if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(game.camera) {
        t.position = Vec3::new(p.x, p.y.max(0.0) + 2.5, 11.0);
    }
    hud(world, &game);
    world.insert_resource(game);
}

/// `TIRMAN_OTO`: kenara yürür, feribot yanaşınca biner ve **hiç yürümeden** karşıya geçer, karşıda
/// asansöre yürür, asansör tepedeyken çıkıntıya geçer, bayrağa yürür. Yürüme yönü (-1…1).
fn auto_walk(world: &World, game: &mut Game, dt: f32) -> f32 {
    let p = position(world, game.player);
    let ferry = position(world, game.ferry);
    let lift_top = position(world, game.lift).y + 0.25;
    let grounded = world.query::<&CharacterController>().get(game.player).is_some_and(|c| c.is_grounded);
    match game.plan {
        Plan::ToEdge => {
            if p.x >= 5.3 {
                game.plan = Plan::WaitFerry;
            }
            1.0
        }
        // Feribot başlangıç ucundayken (sol kenarı zemine dayalı) biner.
        Plan::WaitFerry => {
            if ferry.x < FERRY.0 + 0.2 {
                game.plan = Plan::Board;
            }
            0.0
        }
        Plan::Board => {
            if p.x >= ferry.x - 0.3 && grounded {
                game.plan = Plan::Ride;
                game.ride_from = p.x;
                game.ridden = 0.0;
            }
            1.0
        }
        // Yürümeden durur: ilerlediği her santim feribotun taşıdığı.
        Plan::Ride => {
            game.ride_time += dt;
            game.ridden = p.x - game.ride_from;
            if ferry.x > FERRY.1 - 0.2 || p.x > 11.5 {
                game.plan = Plan::ToLift;
            }
            0.0
        }
        Plan::ToLift => {
            // Asansör aşağıdayken ortasına yürür.
            if (p.x - LIFT_X).abs() < 0.2 && lift_top < 0.3 {
                game.plan = Plan::Lift;
                return 0.0;
            }
            if lift_top > 0.3 && p.x > 12.5 {
                return 0.0; // asansör aşağı inene dek bekle
            }
            ((LIFT_X - p.x) / 0.4).clamp(-1.0, 1.0)
        }
        Plan::Lift => {
            if lift_top > LEDGE_Y - 0.05 {
                game.plan = Plan::ToGoal;
            }
            0.0
        }
        Plan::ToGoal => 1.0,
    }
}

fn hud(world: &mut World, game: &Game) {
    let p = position(world, game.player);
    set_text(world, game.hud, &format!("düşüş: {}\nx {:.1}  y {:.1}\nA D: yürü   R: baştan", game.falls, p.x, p.y));
    set_text(world, game.banner, if game.won { "Bayrak!\nR ile yeniden" } else { "" });
}

fn set_text(world: &mut World, id: Entity, s: &str) {
    if let Some(mut t) = world.query_mut::<&mut Text>().get_mut(id) {
        if t.content != s {
            t.content = s.to_string();
        }
    }
}

fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
