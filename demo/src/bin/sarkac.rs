//! # Sarkaç — boyları seçilmiş on beş sarkacın dalgası
//!
//! On beş sarkaç yan yana bir kirişe asılı. Boyları, k'ıncısı bir dakikada 51 + k salınım yapacak
//! biçimde seçilmiş; hepsi aynı açıdan bırakılır ve bir yılan gibi kıvrılan, bölünen, yeniden
//! birleşen bir desen çizerler. Bir dakika sonunda hepsi yeniden aynı hizadadır.
//!
//! Motorun hangi yollarını yürüttüğü, ve onun için yazıldığı:
//!
//! - **Yalnız eklemler**: her sarkaç statik kirişe bir `Joint::ball_socket` ile asılı bir küre;
//!   hiçbir temas yok, çözüm baştan sona eklem çözücüsünün serbest geçişi.
//! - **Periyot**: bir sarkacın periyodu kolay ölçülür ve analitik değeri bilinir — burada bileşik
//!   sarkaç (kürenin kendi ataleti dahil) ve büyük genlik düzeltmesiyle. Sarkaçların periyotları
//!   birbirinden %1–2 farklı: desen bunu bir dakikada görünür kılar, test onu ölçer.
//! - **Enerji**: sönüm sıfır; genlik ne azalmalı ne artmalı. Eklemin konum düzeltmesi enerji
//!   katarsa ya da yiyerse, otuz saniyede görünür.
//!
//! Bu oyun yazılırken bulunan: eklemin ankrajı üç ayrı satırla tutuluyordu ve ankraj hafif bir
//! gövdenin merkezinden uzaktayken satırlar yakınsamıyordu. Buradaki sarkaçlar %0.1 yavaş
//! sallanıyor, otuz saniyede genliklerinin %0.7'sini yitiriyordu; 1 m ipteki 4 mm'lik bir küre
//! %324 yavaş. Ankraj artık tek bir 3×3 blok (`JointSolver::apply_point_constraint`, `pendulum.rs`).
//!
//! `sarkac --test`: pencere açmadan otuz saniye salınır. Her sarkacın periyodu analitik değerin
//! binde biri içinde, genliği bırakıldığı açının binde beşi içinde olmalı (60 Hz'te örneklenen
//! tepenin payı ~%0.16).
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::physics::joints::Joint;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{BodyHandle, Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};

const PHYSICS_HZ: u32 = 240;
const G: f32 = 9.81;
const PENDULUMS: usize = 15;
/// Desenin döngüsü: k'ıncı sarkaç bu kadar saniyede `FIRST_COUNT + k` salınım yapar.
const CYCLE: f32 = 60.0;
const FIRST_COUNT: f32 = 51.0;
/// Kirişin yüksekliği ve sarkaçların aralığı (x'te).
const BEAM_Y: f32 = 1.0;
const SPACING: f32 = 0.15;
const BOB_R: f32 = 0.04;
const BOB_MASS: f32 = 0.3;
/// Bırakılış açısı, radyan (z yönünde).
const THETA0: f32 = 0.3;

#[derive(Clone, Copy)]
struct Bob {
    k: usize,
    length: f32,
}
gizmo::core::impl_component!(Bob);

/// Sarkacın salınımının kaydı: açısı, sıfırdan yukarı geçişlerinin anları, en büyük açısı.
#[derive(Default, Clone)]
struct Swing {
    last_angle: f32,
    upward: Vec<f32>,
    /// Son yarım periyottaki en büyük |açı| — genliğin şimdiki değeri.
    peak: f32,
    /// Bir önceki tam yarım periyodunki.
    amplitude: f32,
}

struct Game {
    physics: FixedPhysics,
    clock: f32,
    swings: Vec<Swing>,
    hud: Entity,
}

/// Sarkaç `k`'nin tasarım periyodu ve boyu (basit sarkaç `T = 2π √(L/g)`'den).
fn design(k: usize) -> (f32, f32) {
    let period = CYCLE / (FIRST_COUNT + k as f32);
    (period, G * (period / std::f32::consts::TAU).powi(2))
}

/// Sarkacın analitik periyodu: bileşik sarkaç — kürenin kendi ataleti `0.4 m r²`, menteşeden
/// `L` uzakta — `T₀ = 2π √((L² + 0.4 r²) / (g L))`, ve büyük genlik düzeltmesi
/// `1 + θ₀²/16 + 11 θ₀⁴/3072`.
fn analytic_period(length: f32) -> f32 {
    let t0 = std::f32::consts::TAU * ((length * length + 0.4 * BOB_R * BOB_R) / (G * length)).sqrt();
    t0 * (1.0 + THETA0 * THETA0 / 16.0 + 11.0 * THETA0.powi(4) / 3072.0)
}

fn pivot(k: usize) -> Vec3 {
    Vec3::new((k as f32 - (PENDULUMS as f32 - 1.0) * 0.5) * SPACING, BEAM_Y, 0.0)
}

/// Sarkacın açısı: menteşeden küreye doğrultunun düşeyle yaptığı açı, z yönünde artı.
fn angle(bob: Vec3, k: usize) -> f32 {
    let d = bob - pivot(k);
    d.z.atan2(-d.y)
}

fn setup(world: &mut World, font: FontId) -> Game {
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0)));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.7) * Quat::from_rotation_x(-0.8)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
    ));
    // Yandan ve biraz önden: desen, sarkaçların z'deki sapmalarının dizisidir.
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(-2.2, 1.4, 1.6),
        yaw: -0.65,
        pitch: -0.35,
        primary: true,
        ..Default::default()
    });
    let beam_half = Vec3::new(PENDULUMS as f32 * SPACING * 0.5 + 0.1, 0.02, 0.02);
    let beam = world.spawn_bundle((
        Transform::new(Vec3::new(0.0, BEAM_Y + beam_half.y, 0.0)).with_scale(beam_half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.3, 0.3, 0.35, 1.0), 0.5, 0.6),
        MeshRenderer::new(),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(beam_half),
    ));
    let mut joints = Vec::new();
    for k in 0..PENDULUMS {
        let (_, length) = design(k);
        // Küre, menteşeden `length` uzakta, `THETA0` kadar z'ye çekilmiş; yerel +y menteşeyi gösterir.
        let rotation = Quat::from_rotation_x(-THETA0);
        let at = pivot(k) - rotation * Vec3::Y * length;
        let shade = k as f32 / (PENDULUMS as f32 - 1.0);
        let mut body = RigidBody::new(BOB_MASS, true);
        body.linear_damping = 0.0;
        body.angular_damping = 0.0;
        let bob = world.spawn_bundle((
            Transform::new(at).with_rotation(rotation),
            GlobalTransform::default(),
            body,
            Velocity::default(),
            Collider::sphere(BOB_R),
            Bob { k, length },
        ));
        let ball = world.spawn_bundle((
            Transform::new(Vec3::ZERO).with_scale(Vec3::splat(BOB_R)),
            GlobalTransform::default(),
            MeshSource("sphere".into()),
            MaterialDesc::new().with_pbr(Vec4::new(0.9 - 0.6 * shade, 0.3 + 0.4 * shade, 0.2 + 0.7 * shade, 1.0), 0.3, 0.4),
            MeshRenderer::new(),
        ));
        world.add_child(bob, ball);
        let rod = world.spawn_bundle((
            Transform::new(Vec3::Y * length * 0.5).with_scale(Vec3::new(0.003, length * 0.5, 0.003)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.4, 0.9),
            MeshRenderer::new(),
        ));
        world.add_child(bob, rod);
        joints.push(Joint::ball_socket(
            BodyHandle::from_id(bob.id()),
            BodyHandle::from_id(beam.id()),
            Vec3::Y * length,
            pivot(k) - Vec3::new(0.0, BEAM_Y + beam_half.y, 0.0),
        ));
    }
    if let Some(mut physics) = world.get_resource_mut::<PhysicsWorld>() {
        physics.joints.extend(joints);
    }
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    let swings = (0..PENDULUMS).map(|_| Swing { last_angle: THETA0, ..Swing::default() }).collect();
    Game { physics: FixedPhysics::new(PHYSICS_HZ), clock: 0.0, swings, hud }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, _input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.physics.advance(world, dt);
    sync_transforms(world);
    game.clock += dt;
    let clock = game.clock;
    for (t, bob) in world.query::<(&Transform, &Bob)>().iter() {
        let swing = &mut game.swings[bob.k];
        let a = angle(t.position, bob.k);
        swing.peak = swing.peak.max(a.abs());
        // Sıfırdan geçiş: anı, iki örnek arasında doğrusal ara değerle. Her geçişte yarım periyot
        // biter; genlik o yarımın en büyük açısıdır.
        if (swing.last_angle < 0.0) != (a < 0.0) {
            if swing.last_angle < 0.0 {
                let f = swing.last_angle / (swing.last_angle - a);
                swing.upward.push(clock - dt + f * dt);
            }
            swing.amplitude = swing.peak;
            swing.peak = 0.0;
        }
        swing.last_angle = a;
    }
    if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        text.content = format!("{:.1} s / {CYCLE:.0} s", clock % CYCLE);
    }
    world.insert_resource(game);
}

/// `--test`: otuz saniye. Her sarkacın ölçülen periyodu (yukarı geçişlerinin ortalama aralığı)
/// analitik değerinin binde biri içinde, son genliği bırakıldığı açının binde beşi içinde olmalı.
fn self_test() -> bool {
    let mut world = World::new();
    let game = setup(&mut world, HeadlessFrames::font());
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    while run.elapsed < 30.0 {
        run.frame(dt, update);
    }
    let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
    let lengths: Vec<f32> = {
        let mut l = vec![0.0; PENDULUMS];
        for (_, bob) in run.world.query::<(&Transform, &Bob)>().iter() {
            l[bob.k] = bob.length;
        }
        l
    };
    let mut worst_period = 0.0f32;
    let mut worst_amplitude = 0.0f32;
    for (k, swing) in game.swings.iter().enumerate() {
        let (Some(first), Some(last)) = (swing.upward.first(), swing.upward.last()) else {
            println!("sarkac --test: sarkaç {k} hiç geçmedi → BAŞARISIZ");
            return false;
        };
        let measured = (last - first) / (swing.upward.len() - 1) as f32;
        let expected = analytic_period(lengths[k]);
        worst_period = worst_period.max((measured / expected - 1.0).abs());
        worst_amplitude = worst_amplitude.max((swing.amplitude / THETA0 - 1.0).abs());
        if k == 0 || k == PENDULUMS - 1 {
            println!(
                "  sarkaç {k}: L {:.3} m, periyot {measured:.4} s (analitik {expected:.4}), genlik {:.4} rad ({THETA0} bırakıldı)",
                lengths[k], swing.amplitude
            );
        }
    }
    let ok = worst_period < 1e-3 && worst_amplitude < 0.005;
    println!(
        "sarkac --test: {PENDULUMS} sarkaç, 30 s; periyot analitikten en çok %{:.3}, genlik en çok %{:.2} saptı → {}",
        worst_period * 100.0,
        worst_amplitude * 100.0,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Sarkaç", 1280, 720)
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
