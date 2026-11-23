//! # İniş — ayın yüzüne yumuşak bir iniş
//!
//! Bir iniş aracı yüksekten, yana kaymış başlar; ana motoru ve iki yan iticisiyle pistin üstüne
//! gelip yavaşça inmelidir. Yakıt azdır. Sert inen ya da pistin dışına, kayalara inen araç
//! parçalanır; bacakları kopup düşer.
//!
//! Motorun hangi yollarını yürüttüğü, ve onun için yazıldığı:
//!
//! - **Araç bileşik, kırılabilir bir gövde**: gövdenin kendi kutusu ve iki bacağı onun çocukları,
//!   yerel dönüşümlerle. `Breakable` — sert bir inişte gövde kırıntılara ayrılır, bacaklar
//!   gövdenin onları tuttuğu yerden kendi başlarına düşer.
//! - **İtki `ExternalForce`'la**, gövdenin ekseni boyunca, yan iticiler bir tork; fizik kare
//!   başına birkaç adım (120 Hz) atar ve itki her adımda uygulanır.
//! - **Düzlemsel fizik** (`RigidBody::planar_xy`): araç XY düzleminde kalır, yalnız Z'de döner.
//! - **Yükseklik bir ışınla** ölçülür (`PhysicsWorld::raycast_excluding`), aracın kendisi hariç.
//! - Ayın yerçekimi, `PhysicsWorld::with_gravity`.
//!
//! Bu oyun yazılırken bulunan: kırılan bir gövde yalnız kendisi silinirdi, ve bir ebeveyni
//! silmek çocuklarını kökler — bacaklar, yerel konumları dünya konumu sayılarak, dünyanın
//! başlangıcının yanında, yere yarı gömülü statik engeller olarak kalırdı.
//!
//! `inis --test`: pencere açmadan iki iniş: otomatik pilotla piste yumuşak bir iniş (araç pistte,
//! dik, durmuş, parçalanmamış olmalı) ve kayalara sert bir düşüş (araç parçalanmalı, bacakları
//! kendi gövdeleri olarak, düzlemde, yere düşmüş olmalı).
//!
//! Bu oyun yazılırken bulunan ikinci şey: düzlemsel bir gövdenin kırıntıları ve kopan parçaları
//! onun eksen kilitlerini almıyordu — yandan bakılan bir oyunda bacaklar düzlemden 1.7 m kaçtı.

use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::physics::components::{Breakable, ExternalForce};
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{BodyHandle, Collider, Ray, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};

const PHYSICS_HZ: u32 = 120;
/// Ayın yerçekimi, m/s².
const GRAVITY: f32 = 1.62;
const LANDER_MASS: f32 = 600.0;
/// Ana motorun en çok itkisi: ayın yerçekiminin üç katı.
const MAX_THRUST: f32 = 3.0 * LANDER_MASS * GRAVITY;
/// Yan iticilerin en çok torku, N·m.
const MAX_TORQUE: f32 = 1400.0;
/// Yakıt, en çok itkide kaç saniye yeter.
const FUEL_SECONDS: f32 = 40.0;
/// Gövdenin yarı boyu.
const HULL: Vec3 = Vec3::new(1.0, 0.55, 0.7);
/// Bir bacağın yarı boyu ve gövdeye göre yeri (her iki yanda, x'in işareti değişir).
const LEG: Vec3 = Vec3::new(0.09, 0.45, 0.09);
const LEG_AT: Vec3 = Vec3::new(1.05, -0.85, 0.0);
/// Bacağın altından gövdenin merkezine: aracın yere oturduğundaki yüksekliği.
const STANDING_HEIGHT: f32 = -LEG_AT.y + LEG.y;
/// Pistin merkezi (x) ve yarı genişliği.
const PAD_X: f32 = 24.0;
const PAD_HALF: f32 = 3.0;
/// Güvenli bir iniş: eğiklik sınırı. Hız sınırını kırılma darbesi koyar (`BREAK_IMPULSE`).
const SAFE_TILT: f32 = 0.2;
/// Aracın kırıldığı darbe, N·s, bir temas noktasında: bir bacağın üstüne ~5 m/s'lik bir iniş.
const BREAK_IMPULSE: f32 = 1500.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Outcome {
    Flying,
    Landed,
    Crashed,
}

struct Game {
    physics: FixedPhysics,
    lander: Entity,
    legs: [Entity; 2],
    fuel: f32,
    /// Otomatik pilot: `--test`'te hep, oyunda `P` ile.
    autopilot: bool,
    outcome: Outcome,
    /// Pistte dik ve durgun geçen süre: bir an değmek iniş değildir.
    settled: f32,
    hud: Entity,
}

fn block(world: &mut World, centre: Vec3, half: Vec3, colour: Vec4) -> Entity {
    world.spawn_bundle((
        Transform::new(centre).with_scale(half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(colour, 0.9, 0.0),
        MeshRenderer::new(),
        Collider::box_collider(half),
    ))
}

/// Yer: kayalık bir zemin, birkaç tümsek, ve düz, parlak bir pist.
fn build_ground(world: &mut World) {
    let rock = Vec4::new(0.45, 0.44, 0.42, 1.0);
    block(world, Vec3::new(10.0, -1.0, 0.0), Vec3::new(40.0, 1.0, 4.0), rock);
    for (x, half) in [(-6.0, Vec3::new(2.0, 1.5, 3.0)), (6.0, Vec3::new(1.5, 2.5, 3.0)), (14.0, Vec3::new(2.5, 1.0, 3.0)), (34.0, Vec3::new(2.0, 3.0, 3.0))] {
        block(world, Vec3::new(x, half.y, 0.0), half, rock);
    }
    block(world, Vec3::new(PAD_X, 0.1, 0.0), Vec3::new(PAD_HALF, 0.1, 2.0), Vec4::new(0.85, 0.8, 0.3, 1.0));
}

/// Araç: gövde (kırılabilir, düzlemsel) ve iki bacağı, gövdenin bileşik şekli.
fn build_lander(world: &mut World, at: Vec3, velocity: Vec3) -> (Entity, [Entity; 2]) {
    let lander = world.spawn_bundle((
        Transform::new(at).with_scale(HULL),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.8, 0.8, 0.85, 1.0), 0.4, 0.6),
        MeshRenderer::new(),
        RigidBody::new(LANDER_MASS, true).planar_xy(),
        Velocity::new(velocity),
        Collider::box_collider(HULL),
        ExternalForce::default(),
        Breakable {
            max_pieces: 8,
            threshold: BREAK_IMPULSE,
            current_health: BREAK_IMPULSE,
            max_health: BREAK_IMPULSE,
            debris_lifetime: 0.0,
            ..Default::default()
        },
    ));
    let legs = [-1.0, 1.0].map(|side| {
        // Gövdenin ölçeği çocuklarına geçer: bacağın yeri ve boyu gövde biriminde.
        let leg = world.spawn_bundle((
            Transform::new(Vec3::new(side * LEG_AT.x, LEG_AT.y, 0.0) / HULL).with_scale(LEG / HULL),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(Vec4::new(0.3, 0.3, 0.32, 1.0), 0.6, 0.4),
            MeshRenderer::new(),
            Collider::box_collider(LEG / HULL),
        ));
        world.add_child(lander, leg);
        leg
    });
    (lander, legs)
}

fn setup(world: &mut World, font: FontId) -> Game {
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -GRAVITY, 0.0)));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.7)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.92), 3.5, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(12.0, 14.0, 42.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.2,
        primary: true,
        ..Default::default()
    });
    build_ground(world);
    let (lander, legs) = build_lander(world, Vec3::new(0.0, 30.0, 0.0), Vec3::new(3.0, 0.0, 0.0));
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    Game { physics: FixedPhysics::new(PHYSICS_HZ), lander, legs, fuel: FUEL_SECONDS, autopilot: false, outcome: Outcome::Flying, settled: 0.0, hud }
}

/// Aracın durumu: yeri, hızı, açısı (Z ekseninde, saat yönünün tersine artı) ve açısal hızı.
struct Pose {
    at: Vec3,
    velocity: Vec3,
    angle: f32,
    spin: f32,
}

fn pose(world: &World, lander: Entity) -> Option<Pose> {
    let t = *world.query::<&Transform>().get(lander)?;
    let v = *world.query::<&Velocity>().get(lander)?;
    let up = t.rotation * Vec3::Y;
    Some(Pose { at: t.position, velocity: v.linear, angle: (-up.x).atan2(up.y), spin: v.angular.z })
}

/// Aracın altındaki yerin yüksekliği: aşağı bir ışın, aracın kendisi hariç.
fn ground_below(world: &World, lander: Entity, at: Vec3) -> Option<f32> {
    let physics = world.get_resource::<PhysicsWorld>()?;
    let hit = physics.raycast_excluding(&Ray::new(at, Vec3::NEG_Y), 200.0, BodyHandle::from_id(lander.id()))?;
    Some(hit.point.y)
}

/// Otomatik pilot: pistin üstüne gel, eğilerek yatayda dur, sonra yavaşça in. İtki (0..1) ve
/// tork (-1..1).
fn autopilot(pose: &Pose, height: f32) -> (f32, f32) {
    let off = PAD_X - pose.at.x;
    // Yatay: piste doğru, yaklaştıkça yavaşlayarak.
    let want_vx = (0.35 * off).clamp(-5.0, 5.0);
    // İleri itmek için öne eğil: gövdenin yukarısı (−sin θ, cos θ), +x'e itki için θ < 0.
    let want_angle = (-0.12 * (want_vx - pose.velocity.x)).clamp(-0.4, 0.4);
    let torque = (6.0 * (want_angle - pose.angle) - 3.0 * pose.spin).clamp(-1.0, 1.0);
    // Dikey: pistin üstünde değilken yüksekliği tut, üstündeyken yere yaklaştıkça yavaşla.
    let above_pad = off.abs() < PAD_HALF * 0.5 && pose.velocity.x.abs() < 0.5;
    let want_vy = if above_pad { -(0.2 * height).clamp(0.6, 4.0) } else { (0.4 * (10.0 - height)).clamp(-2.0, 2.0) };
    let thrust = LANDER_MASS * (GRAVITY + 1.5 * (want_vy - pose.velocity.y)) / pose.angle.cos().max(0.5);
    ((thrust / MAX_THRUST).clamp(0.0, 1.0), torque)
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    if input.is_key_just_pressed(KeyCode::KeyP as u32) {
        game.autopilot = !game.autopilot;
    }
    if game.outcome == Outcome::Flying {
        if let Some(p) = pose(world, game.lander) {
            let height = ground_below(world, game.lander, p.at).map_or(100.0, |g| p.at.y - g - STANDING_HEIGHT);
            let (throttle, turn) = if game.autopilot {
                autopilot(&p, height)
            } else {
                // W ya da çubuğu ileri: itki, ne kadar ileriyse o kadar; A/D ya da çubuğu yana: dön
                // (A sola, saat yönünün tersine).
                let (sideways, forward) = input.move_axis();
                (forward.max(0.0), -sideways)
            };
            let throttle = if game.fuel > 0.0 { throttle } else { 0.0 };
            game.fuel = (game.fuel - throttle * dt).max(0.0);
            let up = Vec3::new(-p.angle.sin(), p.angle.cos(), 0.0);
            if let Some(mut push) = world.query_mut::<&mut ExternalForce>().get_mut(game.lander) {
                *push = ExternalForce::new(up * throttle * MAX_THRUST).with_torque(Vec3::new(0.0, 0.0, turn * MAX_TORQUE));
            }
            if let Some(mut body) = world.query_mut::<&mut RigidBody>().get_mut(game.lander) {
                if throttle > 0.0 || turn != 0.0 {
                    body.wake_up();
                }
            }
        }
    }

    game.physics.advance(world, dt);
    sync_transforms(world);

    if game.outcome == Outcome::Flying {
        match pose(world, game.lander) {
            None => game.outcome = Outcome::Crashed,
            Some(p) => {
                let on_pad = (p.at.x - PAD_X).abs() < PAD_HALF - LEG_AT.x * 0.5 && (p.at.y - (0.2 + STANDING_HEIGHT)).abs() < 0.15;
                let still = p.velocity.length() < 0.05 && p.spin.abs() < 0.05;
                if on_pad && still && p.angle.abs() < SAFE_TILT {
                    game.settled += dt;
                    if game.settled > 1.0 {
                        game.outcome = Outcome::Landed;
                    }
                } else {
                    game.settled = 0.0;
                }
            }
        }
    }

    if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        let state = match game.outcome {
            Outcome::Flying => if game.autopilot { "otomatik pilot" } else { "W itki · A/D dön · P otomatik pilot (ya da çubuk)" },
            Outcome::Landed => "İNDİ",
            Outcome::Crashed => "PARÇALANDI",
        };
        text.content = format!("Yakıt: {:.0} s   {state}", game.fuel);
    }
    world.insert_resource(game);
}

/// Bir iniş, pencere açmadan: `prepare` aracı yerleştirir, `seconds` boyunca koşar.
fn fly(prepare: impl FnOnce(&mut World, &mut Game), seconds: f32) -> (World, Game) {
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font());
    prepare(&mut world, &mut game);
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    while run.elapsed < seconds {
        run.frame(dt, update);
        if run.world.get_resource::<Game>().is_some_and(|g| g.outcome != Outcome::Flying && run.elapsed > 2.0 && g.outcome == Outcome::Landed) {
            break;
        }
    }
    // Biraz daha: inişten sonra araç yerinde kalmalı, düşüşten sonra parçalar yere inmeli.
    for _ in 0..120 {
        run.frame(dt, update);
    }
    let mut world = run.world;
    let game = world.remove_resource::<Game>().expect("oyun kaynağı");
    (world, game)
}

/// `--test`: otomatik pilotla piste yumuşak bir iniş, ve kayalara sert bir düşüş.
fn self_test() -> bool {
    // 1. Yumuşak iniş.
    let (world, game) = fly(|_, game| game.autopilot = true, 90.0);
    let landed = pose(&world, game.lander);
    let soft = game.outcome == Outcome::Landed && landed.as_ref().is_some_and(|p| p.angle.abs() < SAFE_TILT && p.velocity.length() < 0.05);
    match &landed {
        Some(p) => println!(
            "inis --test: otomatik pilot {:?}; x {:.2} (pist {PAD_X}), açı {:.3}, hız {:.3}, yakıt {:.1} s",
            game.outcome, p.at.x, p.angle, p.velocity.length(), game.fuel
        ),
        None => println!("inis --test: otomatik pilot — araç yok ({:?})", game.outcome),
    }

    // 2. Sert düşüş: kayaların üstünden, motor kapalı, 14 m/s.
    let (world, game) = fly(
        |world, game| {
            if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(game.lander) {
                t.position = Vec3::new(6.0, 12.0, 0.0);
            }
            if let Some(mut v) = world.query_mut::<&mut Velocity>().get_mut(game.lander) {
                v.linear = Vec3::new(0.0, -14.0, 0.0);
            }
        },
        8.0,
    );
    let broke = !world.is_alive(game.lander) && game.outcome == Outcome::Crashed;
    let legs: Vec<Option<Vec3>> = game.legs.iter().map(|&l| world.query::<&Transform>().get(l).map(|t| t.position)).collect();
    // Her bacak kendi gövdesi olarak kopmuş, düzlemde, yere düşmüş olmalı: yerel konumunda,
    // yerin içinde (y −1.5) bir statik engel olarak değil.
    let legs_fell = game.legs.iter().all(|&l| {
        let free = world.query::<&RigidBody>().get(l).is_some() && world.query::<&gizmo::core::component::Parent>().get(l).is_none();
        let at = world.query::<&Transform>().get(l).map(|t| t.position);
        free && at.is_some_and(|p| p.y > -0.2 && p.z.abs() < 1e-3)
    });
    println!("inis --test: sert düşüş — araç {}, bacaklar {legs:?}", if broke { "parçalandı" } else { "SAĞLAM" });

    let ok = soft && broke && legs_fell;
    println!("inis --test: {}", if ok { "tamam" } else { "BAŞARISIZ" });
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - İniş", 1280, 720)
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
