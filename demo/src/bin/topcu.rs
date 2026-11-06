//! # Topçu — bir ragdoll'u hedefe at
//!
//! Bir top, bir ragdoll, yerde iç içe halkalar. Açıyı ve gücü ayarla, ateşle; ragdoll uçar, yere
//! çarpar, yuvarlanır, durur. Durduğu yer halkaların ortasına ne kadar yakınsa o kadar puan. Üç
//! atış. Motorun otuz üçüncü oyunu ve **ragdoll'un** kullanıldığı ilki.
//!
//! | girdi | iş |
//! |---|---|
//! | **A / D** | açı |
//! | **W / S** | güç |
//! | **Space** | ateş |
//! | **R** | baştan |
//!
//! `topcu --test` pencere açmaz: otomatik pilot balistik hesapla 25 m'deki hedefe bir atış
//! yapar; ragdoll'un nereye indiği, nerede durduğu ve eklemlerinin en çok ne kadar ayrıldığı
//! yazılır. (`TOPCU_OTO=1` pencereli oyunda da otomatik pilotu açar.)
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Hazır insan iskeleti doğduğunda dağınıktı.** On eklemin dokuzu açıktı (omuzlar 18 cm) ve
//!   alt kollar gövdenin içinde doğuyordu: her ragdoll ilk adımında kendini toplayıp tekmeliyordu.
//!   Bu oyunda top ağzındaki ragdoll fırlamadan önce silkiniyordu. **Düzeltildi:**
//!   `RagdollBuilder::create_humanoid` kemikleri eklemlerinin buluştuğu yere koyuyor.
//!
//! Gerisi ilk denemede çalıştı: eklemler uçuşta ve çarpmada en çok 6 mm açılıyor, ragdoll zemini
//! delmiyor, yerde birkaç saniyede duruyor. 40°'de 14,9 m/s ile atılan ragdoll hedefin 1,3 m
//! önüne iniyor ve yuvarlanıp 8,6 m daha gidiyor — onu hesaba katmak oyunun kendisi.

use gizmo::core::input::Input;
use gizmo::physics::ragdoll::{RagdollBuilder, RagdollInstance};
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{MeshData, Text, TextAlign, TextAnchor};
use gizmo::simple::{set_text, sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

const G: f32 = 9.81;
/// Topun ağzı.
const MUZZLE: Vec3 = Vec3::new(0.0, 2.5, 0.0);
/// Hedefin ortası, +X yönünde.
const TARGET: f32 = 25.0;
const RINGS: [f32; 3] = [1.5, 3.0, 4.5];
const SHOTS: usize = 3;

fn spawn_ground(world: &mut World) {
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0)));
    world.spawn_bundle((
        Transform::new(Vec3::new(20.0, -0.5, 0.0)),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(Vec3::new(60.0, 0.5, 30.0)),
    ));
}

/// Açı (radyan, yataydan) ve hız (m/s) için fırlatma yönü.
fn launch_velocity(angle: f32, speed: f32) -> Vec3 {
    Vec3::new(angle.cos(), angle.sin(), 0.0) * speed
}

/// Hava direnci olmadan, ağız yüksekliğinden `distance` uzağa düşüren hız.
fn ballistic_speed(angle: f32, distance: f32) -> f32 {
    // y(x) = h + x·tanθ − g·x² / (2 v² cos²θ) = 0
    let (c, t) = (angle.cos(), angle.tan());
    (G * distance * distance / (2.0 * c * c * (MUZZLE.y + distance * t))).sqrt()
}

fn fire(world: &mut World, angle: f32, speed: f32, visual: bool) -> RagdollInstance {
    // Kemik ağzın biraz önünde, başı yukarıda; pelvis ağızda.
    let mut builder = RagdollBuilder::new(MUZZLE);
    builder.create_humanoid();
    let defs = builder.build();
    let ragdoll = gizmo::physics::ragdoll::spawn_ragdoll(world, defs.clone());
    let v = launch_velocity(angle, speed);
    for ((_, e), def) in ragdoll.bones.iter().zip(&defs) {
        if let Some(mut vel) = world.query_mut::<&mut Velocity>().get_mut(*e) {
            vel.linear = v;
            vel.angular = Vec3::new(0.0, 0.0, -1.5);
        }
        if visual {
            world.add_component(*e, GlobalTransform::default());
            world.add_component(*e, MeshData::capsule(def.radius, def.length, 6, 10));
            world.add_component(*e, MaterialDesc::new().with_pbr(Color::srgb_hex("#E67E22").to_vec4(), 0.6, 0.0));
            world.add_component(*e, MeshRenderer::new());
        }
    }
    ragdoll
}

fn despawn(world: &mut World, ragdoll: &RagdollInstance) {
    for (_, e) in &ragdoll.bones {
        if world.is_alive(*e) {
            world.despawn(*e);
        }
    }
}

fn pelvis(world: &World, ragdoll: &RagdollInstance) -> Vec3 {
    ragdoll.bones.first().and_then(|(_, e)| world.query::<&Transform>().get(*e).map(|t| t.position)).unwrap_or(Vec3::ZERO)
}

fn fastest(world: &World, ragdoll: &RagdollInstance) -> f32 {
    let vs = world.query::<&Velocity>();
    ragdoll.bones.iter().filter_map(|(_, e)| vs.get(*e).map(|v| v.linear.length())).fold(0.0, f32::max)
}

fn lowest(world: &World, ragdoll: &RagdollInstance) -> f32 {
    let ts = world.query::<&Transform>();
    ragdoll.bones.iter().filter_map(|(_, e)| ts.get(*e).map(|t| t.position.y)).fold(f32::MAX, f32::min)
}

/// Eklemlerin iki ucu arasındaki en büyük açıklık.
fn widest_joint(world: &World, ragdoll: &RagdollInstance) -> f32 {
    let Some(pw) = world.get_resource::<PhysicsWorld>() else { return 0.0 };
    let ts = world.query::<&Transform>();
    ragdoll
        .joint_indices
        .iter()
        .filter_map(|&j| {
            let joint = pw.joints.get(j)?;
            let (a, b) = (ts.get(world.entity(joint.entity_a.id())?)?, ts.get(world.entity(joint.entity_b.id())?)?);
            Some(((a.position + a.rotation * joint.local_anchor_a) - (b.position + b.rotation * joint.local_anchor_b)).length())
        })
        .fold(0.0, f32::max)
}

/// Durduğu yerin hedefe uzaklığından puan: içteki halka 3, sonra 2, 1.
fn score(at: Vec3) -> u32 {
    let d = Vec2::new(at.x - TARGET, at.z).length();
    RINGS.iter().filter(|&&r| d <= r).count() as u32
}

/// `--test`: tek atış, balistik hesapla.
fn self_test() -> bool {
    let mut world = World::new();
    spawn_ground(&mut world);
    let angle = 40f32.to_radians();
    let speed = ballistic_speed(angle, TARGET);
    let ragdoll = fire(&mut world, angle, speed, false);
    let mut physics = FixedPhysics::new(60);
    let (mut landed, mut widest, mut finite) = (None, 0.0f32, true);
    let first_gap = widest_joint(&world, &ragdoll);
    for _ in 0..60 * 8 {
        physics.advance(&mut world, 1.0 / 60.0);
        world.apply_commands();
        widest = widest.max(widest_joint(&world, &ragdoll));
        if landed.is_none() && lowest(&world, &ragdoll) < 0.25 {
            landed = Some(pelvis(&world, &ragdoll).x);
        }
        finite &= pelvis(&world, &ragdoll).is_finite();
    }
    let rest = pelvis(&world, &ragdoll);
    let still = fastest(&world, &ragdoll);
    let landed = landed.unwrap_or(f32::NAN);
    let ok = finite && first_gap < 1e-3 && (landed - TARGET).abs() < 2.5 && still < 0.3 && widest < 0.05 && rest.y < 0.6;
    println!(
        "topcu --test: {speed:.1} m/s, 40°; {landed:.1} m'ye indi (hedef {TARGET:.0} m), {:.1} m'de durdu (puan {}); doğarken eklem açıklığı {first_gap:.4} m, uçuşta en çok {widest:.3} m; sonda en hızlı kemik {still:.2} m/s → {}",
        rest.x,
        score(rest),
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

struct Game {
    physics: FixedPhysics,
    angle: f32,
    speed: f32,
    ragdoll: Option<RagdollInstance>,
    flying: f32,
    shots: usize,
    total: u32,
    last: Option<u32>,
    barrel: Entity,
    camera: Entity,
    hud: Entity,
    banner: Entity,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Topçu", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = scene.renderer.default_font();
            spawn_ground(scene.world);
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(20.0, -0.5, 0.0)).with_scale(Vec3::new(60.0, 0.5, 30.0)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#6A8F3C").to_vec4(), 0.95, 0.0),
                MeshRenderer::new(),
            ));
            for (k, &r) in RINGS.iter().enumerate().rev() {
                let colour = ["#C0392B", "#ECF0F1", "#2980B9"][k];
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(TARGET, 0.005 + (2 - k) as f32 * 0.004, 0.0)).with_scale(Vec3::new(r, 0.002, r)),
                    GlobalTransform::default(),
                    MeshSource("cylinder".into()),
                    MaterialDesc::new().with_unlit(Color::srgb_hex(colour).to_vec4()),
                    MeshRenderer::new(),
                ));
            }
            let barrel = scene
                .world
                .spawn_bundle((
                    Transform::new(MUZZLE).with_scale(Vec3::new(0.35, 1.2, 0.35)),
                    GlobalTransform::default(),
                    MeshSource("cylinder".into()),
                    MaterialDesc::new().with_pbr(Color::srgb_hex("#2C3E50").to_vec4(), 0.4, 0.8),
                    MeshRenderer::new(),
                ))
                ;
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.9)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 2.2, LightRole::Sun),
            ));
            let camera = scene
                .world
                .spawn_bundle(CameraBundle { position: Vec3::new(10.0, 8.0, 26.0), pitch: -0.25, primary: true, ..Default::default() })
                ;
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                ;
            scene.world.insert_resource(Game {
                physics: FixedPhysics::new(60),
                angle: 40f32.to_radians(),
                speed: 15.0,
                ragdoll: None,
                flying: 0.0,
                shots: 0,
                total: 0,
                last: None,
                barrel,
                camera,
                hud,
                banner,
            });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        if let Some(r) = game.ragdoll.take() {
            despawn(world, &r);
        }
        game.shots = 0;
        game.total = 0;
        game.last = None;
        game.physics.reset();
    }
    let auto = std::env::var_os("TOPCU_OTO").is_some();
    if auto {
        game.angle = 40f32.to_radians();
        game.speed = ballistic_speed(game.angle, TARGET);
    } else {
        let (mx, my) = input.move_axis();
        game.angle = (game.angle + mx * dt * 0.8).clamp(10f32.to_radians(), 75f32.to_radians());
        game.speed = (game.speed + my * dt * 6.0).clamp(5.0, 30.0);
    }
    let ready = game.ragdoll.is_none() || game.flying > 2.0 && game.ragdoll.as_ref().is_some_and(|r| fastest(world, r) < 0.3);
    if ready {
        if let Some(r) = &game.ragdoll {
            if game.last.is_none() {
                let s = score(pelvis(world, r));
                game.last = Some(s);
                game.total += s;
            }
        }
    }
    let fire_now = if auto { ready } else { input.is_key_just_pressed(KeyCode::Space as u32) };
    if fire_now && ready && game.shots < SHOTS {
        if let Some(r) = game.ragdoll.take() {
            despawn(world, &r);
        }
        game.ragdoll = Some(fire(world, game.angle, game.speed, true));
        game.flying = 0.0;
        game.shots += 1;
        game.last = None;
    }
    game.flying += dt;
    game.physics.advance(world, dt);
    sync_transforms(world);
    if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(game.barrel) {
        t.rotation = Quat::from_rotation_z(game.angle - std::f32::consts::FRAC_PI_2);
    }
    let follow = game.ragdoll.as_ref().map(|r| pelvis(world, r)).unwrap_or(MUZZLE);
    if let Some(mut c) = world.query_mut::<&mut Transform>().get_mut(game.camera) {
        let want = Vec3::new(follow.x * 0.6 + 4.0, 7.0, 24.0);
        c.position = c.position.lerp(want, (dt * 1.5).min(1.0));
        let look = (Vec3::new(follow.x * 0.7 + 4.0, 1.0, 0.0) - c.position).normalize_or_zero();
        c.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, look);
    }
    set_text(
        world,
        game.hud,
        &format!(
            "açı {:.0}°   güç {:.1} m/s   atış {}/{SHOTS}   puan {}{}\nA/D: açı   W/S: güç   Space: ateş   R: baştan",
            game.angle.to_degrees(),
            game.speed,
            game.shots,
            game.total,
            game.last.map(|s| format!("   (son atış {s})")).unwrap_or_default()
        ),
    );
    let banner = if game.shots == SHOTS && game.last.is_some() { format!("Bitti: {} puan\nR ile yeniden", game.total) } else { String::new() };
    set_text(world, game.banner, &banner);
    world.insert_resource(game);
}
