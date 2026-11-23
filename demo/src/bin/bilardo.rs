//! # Bilardo — on beş top, altı cep
//!
//! Gerçek ölçülerde bir masa: 2.54 × 1.27 m'lik çuha, 28.6 mm yarıçaplı, 170 g'lık toplar. Beyaz
//! topu nişanla, vur; bantlar sektirir, cebe düşen top masadan alınır. Çuha topu yavaşlatıp
//! durdurur.
//!
//! Motorun hangi yollarını yürüttüğü, ve onun için yazıldığı:
//!
//! - **Top-top ve top-bant çarpışmaları**: küre-küre, ve bantlar kutu olduğu için küre-kutu temasları;
//!   kafa kafaya bir vuruşta hedef top çizgide gitmeli, sağa sola sapmamalı.
//! - **Yuvarlanma sürtünmesi** (`PhysicsMaterial::rolling_friction`): kayma sürtünmesi yuvarlanan
//!   bir topu durduramaz; durduran budur, ve iki yüzeye de verilmelidir.
//! - **Malzemelerin birleşmesi**: top-top, top-çuha ve top-bant çiftlerinin her biri kendi sekmesi ve
//!   sürtünmesiyle.
//! - **Cepler tetik**: bantların arasındaki boşluktan çuhanın kenarından düşen top, altındaki tetiğe
//!   girer; `Events<TriggerEvent>`.
//! - Küçük, hızlı gövdeler: 480 Hz'lik sabit adım, ve masanın ölçeğinde bir sekme eşiği
//!   (`ConstraintSolver::restitution_velocity_threshold`): motorun 1 m/s'si, metrelik cisimlerin
//!   dinlenmesi içindir; banda 0.6 m/s'de çarpan bir top seker.
//!
//! Bu oyun yazılırken bulunan: yuvarlanma direnci torku yarıçapsız sınırlanıyordu — her top bir
//! metre çapındaymış gibi. 28.6 mm'lik bir top katsayısının söylediğinin 35 katı frenleniyor,
//! dönüşü anında ölüyor, kayarak duruyordu: bir açılışta on altı topun hepsi üç saniyede durdu.
//!
//! `bilardo --test`: pencere açmadan üç vuruş. Kafa kafaya bir vuruş (hedef top çizgide), cebe bir
//! vuruş (top köşe cebine düşmeli) ve açılış (bütün toplar ya cepte ya çuhanın üstünde, hepsi
//! durmuş, hiçbiri beyazın hızını aşmamış).

use gizmo::core::input::Input;
use gizmo::physics::components::PhysicsMaterial;
use gizmo::physics::{Collider, CollisionEventType, RigidBody, TriggerEvent, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};

const PHYSICS_HZ: u32 = 480;
const BALL_R: f32 = 0.028_575;
const BALL_MASS: f32 = 0.17;
/// Çuhanın yarı boyu: oyun alanı 2.54 × 1.27 m.
const HALF_X: f32 = 1.27;
const HALF_Z: f32 = 0.635;
/// Bantların kalınlığı ve yüksekliği; köşe ve orta ceplerin ağzı.
const CUSHION: f32 = 0.05;
const CUSHION_HEIGHT: f32 = 0.045;
const CORNER_MOUTH: f32 = 0.11;
const MIDDLE_MOUTH: f32 = 0.12;
/// Açılış vuruşunun hızı, m/s.
const BREAK_SPEED: f32 = 6.0;

#[derive(Clone, Copy)]
struct Ball(u8);
gizmo::core::impl_component!(Ball);

#[derive(Clone, Copy)]
struct Pocket;
gizmo::core::impl_component!(Pocket);

struct Game {
    physics: FixedPhysics,
    cue: Entity,
    /// Nişanın açısı (XZ'de, +x'ten), radyan.
    aim: f32,
    potted: Vec<u8>,
    hud: Entity,
}

fn felt() -> PhysicsMaterial {
    PhysicsMaterial::default().with_friction(0.2).with_rolling_friction(0.02).with_restitution(0.5)
}

fn ball_material() -> PhysicsMaterial {
    PhysicsMaterial::default().with_friction(0.05).with_rolling_friction(0.02).with_restitution(0.95)
}

fn slab(world: &mut World, centre: Vec3, half: Vec3, colour: Vec4, material: PhysicsMaterial) -> Entity {
    world.spawn_bundle((
        Transform::new(centre).with_scale(half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(colour, 0.85, 0.0),
        MeshRenderer::new(),
        Collider::box_collider(half).with_material(material),
    ))
}

/// Çuha, bantlar ve ceplerin altındaki tetikler. Çuha yalnız oyun alanıdır: bantların arasındaki
/// boşluktan geçen top çuhanın kenarından düşer.
fn build_table(world: &mut World) {
    let green = Vec4::new(0.05, 0.42, 0.2, 1.0);
    slab(world, Vec3::new(0.0, -0.05, 0.0), Vec3::new(HALF_X, 0.05, HALF_Z), green, felt());
    let rail = Vec4::new(0.35, 0.2, 0.1, 1.0);
    let cushion = PhysicsMaterial::default().with_friction(0.15).with_restitution(0.8);
    let y = CUSHION_HEIGHT * 0.5;
    // Uzun bantlar: her yanda iki parça, orta cebin iki yanında.
    let long = (HALF_X - CORNER_MOUTH - MIDDLE_MOUTH * 0.5) * 0.5;
    for side in [-1.0, 1.0] {
        let z = side * (HALF_Z + CUSHION * 0.5);
        for half in [-1.0, 1.0] {
            let x = half * (MIDDLE_MOUTH * 0.5 + long);
            slab(world, Vec3::new(x, y, z), Vec3::new(long, y, CUSHION * 0.5), rail, cushion);
        }
        // Kısa bantlar.
        let x = side * (HALF_X + CUSHION * 0.5);
        slab(world, Vec3::new(x, y, 0.0), Vec3::new(CUSHION * 0.5, y, HALF_Z - CORNER_MOUTH), rail, cushion);
    }
    // Cepler: çuhanın altında, düşen topu yakalayan tetikler; altlarında topları tutan bir taban.
    let corners = [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)].map(|(sx, sz)| Vec3::new(sx * HALF_X, -0.15, sz * HALF_Z));
    let middles = [-1.0, 1.0].map(|sz| Vec3::new(0.0, -0.15, sz * (HALF_Z + 0.03)));
    for at in corners.into_iter().chain(middles) {
        world.spawn_bundle((Transform::new(at), GlobalTransform::default(), Collider::box_collider(Vec3::new(0.14, 0.1, 0.14)).with_trigger(true), Pocket));
    }
    slab(world, Vec3::new(0.0, -0.4, 0.0), Vec3::new(2.0, 0.05, 1.5), Vec4::new(0.1, 0.1, 0.1, 1.0), PhysicsMaterial::default());
}

fn ball(world: &mut World, number: u8, at: Vec3) -> Entity {
    let colour = if number == 0 {
        Vec4::new(0.95, 0.95, 0.9, 1.0)
    } else {
        let hue = number as f32 / 15.0;
        Vec4::new(0.5 + 0.5 * (hue * std::f32::consts::TAU).sin(), 0.5 + 0.5 * (hue * std::f32::consts::TAU + 2.1).sin(), 0.5 + 0.5 * (hue * std::f32::consts::TAU + 4.2).sin(), 1.0)
    };
    world.spawn_bundle((
        Transform::new(Vec3::new(at.x, BALL_R, at.z)).with_scale(Vec3::splat(BALL_R)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        MaterialDesc::new().with_pbr(colour, 0.15, 0.0),
        MeshRenderer::new(),
        RigidBody::new(BALL_MASS, true),
        Velocity::default(),
        Collider::sphere(BALL_R).with_material(ball_material()),
        Ball(number),
    ))
}

/// On beş top üçgende, tepesi ayak noktasında (x = +L/4), sıra sıra +x'e doğru.
fn rack(world: &mut World) {
    let mut n = 1;
    for row in 0..5 {
        for k in 0..=row {
            let x = HALF_X * 0.5 + row as f32 * BALL_R * 3f32.sqrt() * 1.0005;
            let z = (k as f32 - row as f32 * 0.5) * 2.0 * BALL_R * 1.0005;
            ball(world, n, Vec3::new(x, 0.0, z));
            n += 1;
        }
    }
}

fn setup(world: &mut World, font: FontId, balls: impl FnOnce(&mut World) -> Entity) -> Game {
    world.init_events::<TriggerEvent>();
    let mut physics = gizmo::physics::world::PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    // Masanın ölçeğinde: çuhada duran bir topun bir adımdaki düşüşü (g·dt) 2 cm/s'dir.
    physics.solver.restitution_velocity_threshold = 0.1;
    world.insert_resource(physics);
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-1.3)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.9), 3.0, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(-2.4, 1.6, 0.0),
        yaw: 0.0,
        pitch: -0.55,
        primary: true,
        ..Default::default()
    });
    build_table(world);
    let cue = balls(world);
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    Game { physics: FixedPhysics::new(PHYSICS_HZ), cue, aim: 0.0, potted: Vec::new(), hud }
}

/// Beyaz topa `speed` m/s'lik bir vuruş, nişanın yönünde.
fn strike(world: &mut World, game: &Game, speed: f32) {
    let dir = Vec3::new(game.aim.cos(), 0.0, game.aim.sin());
    if let Some(mut v) = world.query_mut::<&mut Velocity>().get_mut(game.cue) {
        v.linear = dir * speed;
        v.angular = Vec3::ZERO;
    }
    if let Some(mut body) = world.query_mut::<&mut RigidBody>().get_mut(game.cue) {
        body.wake_up();
    }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let (turn, _) = input.move_axis();
    game.aim += turn * dt * 1.5;
    if input.is_key_just_pressed(KeyCode::Space as u32) && world.is_alive(game.cue) {
        strike(world, &game, 3.0);
    }
    game.physics.advance(world, dt);
    sync_transforms(world);

    // Cebe düşenler: kuyruk karenin bütün adımlarının girişlerini tutuyor.
    let entered: Vec<u32> = match world.get_resource_mut::<gizmo::core::event::Events<TriggerEvent>>() {
        Some(mut events) => events
            .drain()
            .filter(|e| e.event_type == CollisionEventType::Started)
            .filter(|e| world.query::<&Pocket>().get(world.entity(e.trigger_entity.id()).unwrap_or(game.cue)).is_some())
            .map(|e| e.other_entity.id())
            .collect(),
        None => Vec::new(),
    };
    for id in entered {
        let Some(e) = world.entity(id) else { continue };
        let Some(&Ball(n)) = world.query::<&Ball>().get(e) else { continue };
        if !game.potted.contains(&n) {
            game.potted.push(n);
        }
        world.despawn(e);
    }

    if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        text.content = format!("Cepte: {}   A/D nişan · Boşluk vur", game.potted.len());
    }
    world.insert_resource(game);
}

/// Bir vuruş, pencere açmadan: `balls` masayı kurar ve beyazı döndürür, `aim` nişanlar, en çok
/// `seconds` koşar — bütün toplar durunca daha önce biter. Her karede en hızlı topun hızı izlenir.
fn shot(balls: impl FnOnce(&mut World) -> Entity, aim: f32, speed: f32, seconds: f32) -> (World, Game, f32) {
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font(), balls);
    game.aim = aim;
    strike(&mut world, &game, speed);
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let mut fastest = 0.0f32;
    while run.elapsed < seconds {
        run.frame(1.0 / 60.0, update);
        let mut moving = false;
        for (v, _) in run.world.query::<(&Velocity, &Ball)>().iter() {
            fastest = fastest.max(v.linear.length());
            moving |= v.linear.length() > 0.005;
        }
        if !moving && run.elapsed > 1.0 {
            break;
        }
    }
    let mut world = run.world;
    let game = world.remove_resource::<Game>().expect("oyun kaynağı");
    (world, game, fastest)
}

fn position(world: &World, e: Entity) -> Option<Vec3> {
    world.query::<&Transform>().get(e).map(|t| t.position)
}

/// `--test`: kafa kafaya bir vuruş, cebe bir vuruş, ve açılış.
fn self_test() -> bool {
    // 1. Kafa kafaya: beyaz x = −0.6'dan hedef topa (x = 0) 2 m/s; yarım saniye sonra, hedef top
    //    banda varmadan.
    let mut target = None;
    let (world, game, _) = shot(
        |world| {
            target = Some(ball(world, 1, Vec3::new(0.0, 0.0, 0.0)));
            ball(world, 0, Vec3::new(-0.6, 0.0, 0.0))
        },
        0.0,
        2.0,
        0.5,
    );
    let target = target.expect("hedef");
    let t = position(&world, target);
    let c = position(&world, game.cue);
    let straight = t.is_some_and(|p| p.x > 0.3 && p.x < 1.2 && p.z.abs() < 1e-3 && (p.y - BALL_R).abs() < 0.002) && c.is_some_and(|p| p.z.abs() < 1e-3);
    println!("bilardo --test: kafa kafaya — hedef {t:?}, beyaz {c:?} → {}", if straight { "çizgide" } else { "SAPTI" });

    // 2. Cebe: hedef top köşe cebinin 0.4 m önünde, beyaz onun arkasında, ikisi cebe bakan çizgide.
    let pocket = Vec3::new(HALF_X, 0.0, HALF_Z);
    let line = Vec3::new(1.0, 0.0, 0.5).normalize();
    let (_, game_pot, _) = shot(
        |world| {
            ball(world, 1, pocket - line * 0.4);
            ball(world, 0, pocket - line * 0.9)
        },
        line.z.atan2(line.x),
        1.5,
        6.0,
    );
    let potted = game_pot.potted.contains(&1);
    println!("bilardo --test: cebe vuruş — cepte {:?} → {}", game_pot.potted, if potted { "girdi" } else { "GİRMEDİ" });

    // 3. Açılış.
    let (world, game, fastest) = shot(
        |world| {
            rack(world);
            ball(world, 0, Vec3::new(-HALF_X * 0.5, 0.0, 0.0))
        },
        0.0,
        BREAK_SPEED,
        90.0,
    );
    let mut on_table = 0;
    let mut astray = Vec::new();
    let mut moving = 0;
    for (t, v, b) in world.query::<(&Transform, &Velocity, &Ball)>().iter() {
        let p = t.position;
        if p.x.abs() < HALF_X && p.z.abs() < HALF_Z && (p.y - BALL_R).abs() < 0.003 {
            on_table += 1;
        } else {
            astray.push((b.0, p));
        }
        if v.linear.length() > 0.005 {
            moving += 1;
        }
    }
    let total = on_table + game.potted.len();
    // Toplar yuvarlanır: yarıçapsız yuvarlanma direnciyle bu masa 2.9 saniyede duruyordu, şimdi 7.2.
    let lasted = world.get_resource::<gizmo::core::time::Time>().map_or(0.0, |t| t.elapsed() as f32);
    let broke_clean = astray.is_empty() && total == 16 && moving == 0 && fastest <= BREAK_SPEED * 1.02 && lasted > 5.0;
    println!(
        "bilardo --test: açılış — {lasted:.1} s'de durdu; çuhada {on_table}, cepte {} {:?}, kaçan {astray:?}, hâlâ yuvarlanan {moving}, en hızlı {fastest:.2} m/s → {}",
        game.potted.len(),
        game.potted,
        if broke_clean { "tamam" } else { "BAŞARISIZ" }
    );

    let ok = straight && potted && broke_clean;
    println!("bilardo --test: {}", if ok { "tamam" } else { "BAŞARISIZ" });
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Bilardo", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font, |world| {
                rack(world);
                ball(world, 0, Vec3::new(-HALF_X * 0.5, 0.0, 0.0))
            });
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
