//! # Sapan — kaidelerdeki sandıkları devir
//!
//! Ortadaki sapan, kaidelerin üstündeki sandıklara top atar; her dalgada sandıklar başka yerde.
//! Üç dalganın sandıklarının hepsi kaidesinden düşünce oyun biter.
//!
//! **Motorun Bevy biçiminde yazılmış ilk oyunu**: tek bir `update` kancası yok, oyunun tamamı bir
//! eklenti (`Sapan`) — sahneyi kuran bir `Startup` sistemi, nişan alan ve atan `Update` sistemleri,
//! devrilenleri sayıp sonraki dalgayı kuran bir **dışlayıcı sistem** (`fn(world: &mut World)`), topları
//! `LifetimePlugin`'in (o da dışlayıcı) temizlemesi, fiziği `PhysicsPlugin`'in sürmesi. Pencerede de
//! `--test`'te de aynı eklentiler: test, oyunu `HeadlessFrames`'e eklenti olarak ekleyip boş bir
//! kancayla oynatıyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **A D** | nişanı çevir |
//! | **SPACE** | at |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **`with_simple_scene` ile `PhysicsPlugin` birlikte fiziği iki kez adımlıyordu.** Basit sahnenin
//!   kancası fiziği karenin süresiyle kendisi adımlıyor, eklenti de sabit çizelgede: pencerede bir
//!   top bir saniyede 4,9 m yerine 19,5 m düşüyordu. `--test` bunu görmüyordu, çünkü kancayı değil
//!   eklentiyi çalıştırıyor. **Düzeltildi:** `PhysicsPlugin` dünyaya `PhysicsSteppedBySchedule`
//!   bırakıyor; kanca onu görünce fiziği adımlamıyor, `FixedPhysics` uyarıyor. `air_drag` ve `bounce`
//!   bu yüzden iki kat hızlı düşüyordu.
//!
//! ## Ortam değişkenleri
//!   * `SAPAN_OTO=1` — sapan kendisi nişan alıp atar (`sapan --test` aynısını pencere açmadan yapar)

use gizmo::core::input::Input;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::simple::{HeadlessFrames, SimpleAppExt, SimpleSceneState};

/// Topun yerçekimi (PhysicsPlugin'in varsayılanı) ve uçuş hızı: nişan, hedefe bu yatay hızla
/// giden balistik yol.
const GRAVITY: f32 = 9.81;
const FLIGHT_SPEED: f32 = 14.0;
const COOLDOWN: f32 = 0.7;
const WAVES: u32 = 3;
/// Bir sandık evinden bu kadar uzaklaşınca devrilmiş sayılır.
const KNOCKED: f32 = 0.6;

/// Oyunun tamamı: kaynakları ve sistemleri.
struct Sapan {
    auto: bool,
}

#[derive(Default)]
struct Launcher {
    yaw: f32,
    cooldown: f32,
    /// Otomatik pilotun nişan aldığı sandık.
    aim_at: Option<Vec3>,
}

#[derive(Default)]
struct Score {
    knocked: u32,
    shots: u32,
    wave: u32,
    won: bool,
}

struct Auto(bool);

#[derive(Clone, Copy)]
struct Target {
    home: Vec3,
}
gizmo::core::impl_component!(Target);

#[derive(Clone, Copy)]
struct Ball;
gizmo::core::impl_component!(Ball);

#[derive(Clone, Copy)]
struct Pedestal;
gizmo::core::impl_component!(Pedestal);

#[derive(Clone, Copy)]
struct Hud;
gizmo::core::impl_component!(Hud);

impl Plugin for Sapan {
    fn build(&self, app: &mut dyn AppLike) {
        app.insert_resource(Launcher::default())
            .insert_resource(Score::default())
            .insert_resource(Auto(self.auto))
            .add_systems(Startup, setup_level)
            .add_systems(Update, (aim, fire.run_if(playing), score_and_waves, hud).chain());
    }
}

fn playing(world: &World) -> bool {
    world.get_resource::<Score>().is_some_and(|s| !s.won)
}

/// Zemin, sapanın kaidesi, ışık, kamera ve ilk dalga.
fn setup_level(mut commands: Commands) {
    commands.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.5, 0.0)).with_scale(Vec3::new(30.0, 0.5, 30.0)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Color::srgb_hex("#55603F").to_vec4(), 0.9, 0.0),
        MeshRenderer::new(),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(Vec3::new(30.0, 0.5, 30.0)),
    ));
    commands.spawn_bundle((
        Transform::new(Vec3::new(0.0, 0.4, 0.0)).with_scale(Vec3::new(0.3, 0.4, 0.3)),
        GlobalTransform::default(),
        MeshSource("cylinder".into()),
        MaterialDesc::new().with_pbr(Color::srgb_hex("#7B4A2A").to_vec4(), 0.7, 0.0),
        MeshRenderer::new(),
    ));
    commands.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.9)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.95, 0.88), 1.7, LightRole::Sun),
    ));
    commands.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 7.0, 13.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.45,
        primary: true,
        ..Default::default()
    });
}

/// Bir dalganın kaideleri ve sandıkları: dalgaya göre başka bir yay üstünde, üç kaide, her birinde
/// iki sandık üst üste.
fn spawn_wave(world: &mut World, wave: u32) {
    let radius = 6.0 + wave as f32 * 1.5;
    for k in 0..3 {
        let angle = -0.7 + k as f32 * 0.7 + wave as f32 * 0.25;
        let at = Vec3::new(angle.sin() * radius, 0.0, -angle.cos() * radius);
        world.spawn_bundle((
            Transform::new(at + Vec3::Y * 0.5).with_scale(Vec3::new(0.5, 0.5, 0.5)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(Color::srgb_hex("#6E6A64").to_vec4(), 0.9, 0.0),
            MeshRenderer::new(),
            RigidBody::new_static(),
            Velocity::default(),
            Collider::box_collider(Vec3::new(0.5, 0.5, 0.5)),
            Pedestal,
        ));
        for level in 0..2 {
            let home = at + Vec3::Y * (1.25 + level as f32 * 0.5);
            world.spawn_bundle((
                Transform::new(home).with_scale(Vec3::splat(0.25)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#C0392B").to_vec4(), 0.7, 0.0),
                MeshRenderer::new(),
                RigidBody::new(1.0, true),
                Velocity::default(),
                Collider::box_collider(Vec3::splat(0.25)),
                Target { home },
            ));
        }
    }
}

/// Nişan: elle A/D ile çevrilir; otomatik pilot en yakın ayaktaki sandığı seçer.
fn aim(input: Res<Input>, auto: Res<Auto>, mut launcher: ResMut<Launcher>, targets: Query<(&Transform, &Target)>, time: Res<Time>) {
    let dt = time.dt();
    if auto.0 {
        let standing = targets.iter().filter(|(t, target)| t.position.distance(target.home) < KNOCKED).map(|(t, _)| t.position);
        launcher.aim_at = standing.min_by(|a, b| a.length().total_cmp(&b.length()));
        if let Some(at) = launcher.aim_at {
            launcher.yaw = at.x.atan2(-at.z);
        }
    } else {
        let turn = input.move_axis().0;
        launcher.yaw += turn * 1.5 * dt;
        launcher.aim_at = None;
    }
}

/// Atış: bekleme süresi dolunca (otomatik pilot kendi, oyuncu SPACE ile) nişanın yönüne bir top.
fn fire(mut commands: Commands, input: Res<Input>, auto: Res<Auto>, time: Res<Time>, mut launcher: ResMut<Launcher>, mut score: ResMut<Score>) {
    launcher.cooldown -= time.dt();
    let wants = if auto.0 { launcher.aim_at.is_some() } else { input.is_key_pressed(KeyCode::Space as u32) };
    if launcher.cooldown > 0.0 || !wants {
        return;
    }
    launcher.cooldown = COOLDOWN;
    let from = Vec3::new(0.0, 1.0, 0.0);
    // Hedef: otomatik pilotunki, ya da nişan yönünde 8 m ötede bir sandık yüksekliği.
    let to = launcher.aim_at.unwrap_or(Vec3::new(launcher.yaw.sin() * 8.0, 1.5, -launcher.yaw.cos() * 8.0));
    let flat = Vec3::new(to.x - from.x, 0.0, to.z - from.z);
    let time_of_flight = (flat.length() / FLIGHT_SPEED).max(0.05);
    let rise = (to.y - from.y + 0.5 * GRAVITY * time_of_flight * time_of_flight) / time_of_flight;
    let velocity = flat / time_of_flight + Vec3::Y * rise;
    commands.spawn_bundle((
        Transform::new(from).with_scale(Vec3::splat(0.2)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        MaterialDesc::new().with_pbr(Color::srgb_hex("#2C3E50").to_vec4(), 0.4, 0.3),
        MeshRenderer::new(),
        RigidBody::new(3.0, true),
        Velocity::new(velocity),
        Collider::sphere(0.2),
        DespawnAfter::secs(3.0),
        DespawnBelowY::new(-5.0),
        Ball,
    ));
    score.shots += 1;
}

/// Devrilen sandıkları siler ve sayar; dalga bitince sonrakini kurar, son dalgadan sonra oyunu
/// bitirir. **Dışlayıcı sistem:** silip doğurmak dünyanın kendisini istiyor, ve çizelge onu böyle bir
/// sisteme batch'inde yalnızken veriyor.
fn score_and_waves(world: &mut World) {
    let knocked: Vec<Entity> = world
        .query::<(Entity, &Transform, &Target)>()
        .iter()
        .filter(|(_, t, target)| t.position.distance(target.home) >= KNOCKED)
        .map(|(e, _, _)| e)
        .collect();
    let standing = world.query::<&Target>().iter().count() - knocked.len();
    for e in &knocked {
        world.despawn(*e);
    }
    let Some(mut score) = world.remove_resource::<Score>() else { return };
    score.knocked += knocked.len() as u32;
    if standing == 0 && !score.won {
        // Kaideler de gider: sonraki dalga başka yerde.
        let pedestals: Vec<Entity> = world.query::<(Entity, &Pedestal)>().iter().map(|(e, _)| e).collect();
        for e in pedestals {
            world.despawn(e);
        }
        if score.wave >= WAVES {
            score.won = true;
        } else {
            score.wave += 1;
            spawn_wave(world, score.wave);
        }
    }
    world.insert_resource(score);
}

fn hud(score: Res<Score>, mut texts: Query<&mut Text, With<Hud>>) {
    let line = if score.won {
        format!("Bitti! {} atışta {} sandık", score.shots, score.knocked)
    } else {
        format!("dalga {}/{}   devrilen {}   atış {}\nA D: nişan   SPACE: at", score.wave, WAVES, score.knocked, score.shots)
    };
    for mut t in texts.iter_mut() {
        if t.content != line {
            t.content = line.clone();
        }
    }
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Sapan", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = scene.renderer.load_system_font().unwrap_or_else(|_| {
                scene.renderer.load_font(gizmo::renderer::text::synthetic::synthetic_face()).expect("sentetik yüz")
            });
            scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)), Hud));
        })
        .add_plugin(Sapan { auto: std::env::var_os("SAPAN_OTO").is_some() })
        .add_plugin(PhysicsPlugin::default())
        .add_plugin(LifetimePlugin)
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// `--test`: oyunun eklentisi, `PhysicsPlugin` ve `LifetimePlugin` bir `HeadlessFrames`'te, boş bir
/// kancayla — pencerede koşan aynı sistemler. Otomatik pilot üç dalgayı bitirmeli; topların hepsi
/// ömürleri dolunca gitmiş olmalı.
fn self_test() -> bool {
    let mut run = HeadlessFrames::new(World::new())
        .add_plugin(Sapan { auto: true })
        .add_plugin(PhysicsPlugin::default())
        .add_plugin(LifetimePlugin);
    let dt = 1.0 / 60.0;
    let mut most_balls = 0;
    while run.elapsed < 120.0 {
        run.frame(dt, |_, _, _, _| {});
        most_balls = most_balls.max(run.world.query::<&Ball>().iter().count());
        if run.world.get_resource::<Score>().is_some_and(|s| s.won) {
            break;
        }
    }
    // Biten oyunda ateş kesilir; havadaki toplar ömürleri dolunca silinir.
    for _ in 0..240 {
        run.frame(dt, |_, _, _, _| {});
    }
    let balls_left = run.world.query::<&Ball>().iter().count();
    let score = run.world.get_resource::<Score>().expect("skor");
    let ok = score.won && score.knocked == WAVES * 6 && balls_left == 0 && most_balls < 8;
    println!(
        "sapan --test: {:.1} s; {} dalga, {} sandık devrildi, {} atış; havada en çok {} top, sonunda {} → {}",
        run.elapsed,
        score.wave,
        score.knocked,
        score.shots,
        most_balls,
        balls_left,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}
