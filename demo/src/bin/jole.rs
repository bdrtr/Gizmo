//! # Jöle — sallanan bir kule
//!
//! Bir vinç kulenin üstünde ileri geri gidip geliyor; **Space** ile taşıdığı jöle bloğunu bırak.
//! Bloklar yumuşak: düşünce ezilip sallanıyor, birbirinin üstünde duruyor, ortalanmazsa kayıp
//! düşüyor. Platformdan biri düşerse oyun biter. Motorun otuz ikinci oyunu ve **yumuşak
//! gövdelerin** (`SoftBodyMesh`) kullanıldığı ilki.
//!
//! | girdi | iş |
//! |---|---|
//! | **Space** | bloğu bırak |
//! | **R** | baştan |
//!
//! `jole --test` pencere açmaz: otomatik pilot vinç ortadayken dört blok bırakır, sonra beş
//! saniye bekler; kulenin boyu ve düşen blok olup olmadığı yazılır. (`JOLE_OTO=1` pencereli
//! oyunda da otomatik pilotu açar.)
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Bir jöle bloğu düşünce patlıyordu.** 1 m'lik, 10 kg'lık bir blok (E = 20 000 Pa) 1,5 m'den
//!   zemine bir saniyede dağılıyordu (düğümler 10¹⁵ m'de): tek bir 1/60 s'lik açık adım böyle bir
//!   malzemenin kaldırabileceğinin çok üstünde. **Düzeltildi:** adım, malzemenin kararlı kaldığı
//!   kadar alt adıma bölünüyor (`SoftBodyMesh::stable_substeps`).
//! - **Yerde duran blok zemine gömülüyordu** (kısa adımlarla on saniyede 0,33 m), **zeminden
//!   kalkan düğüm de yerde tutuluyordu.** **Düzeltildi:** içeri kaçan düğüm yüzeye geri konuyor;
//!   düğümün uzaklaştığı yüzey çarpma sayılmıyor.
//! - **Jöle jölenin içinden geçiyordu** — yumuşak gövdeler yalnız rijit olanlara çarpıyordu.
//!   **Düzeltildi:** `soft_contact`, sürtünmeyle (olmadan üstteki blok beş saniyede 0,8 m kayıp
//!   düşüyordu).
//! - **Blok kurmanın yolu yoktu,** elle kurulan beş-dörtyüzlü bölme de uyumsuzdu (iç yüzlerin
//!   yarısı yüzey sanılıyordu). **Düzeltildi:** `SoftBodyMesh::block`.
//! - **Yumuşak gövde hiç çizilmiyordu** — yalnız hata ayıklama çizgileri. **Düzeltildi:**
//!   `soft_body_mesh_system`, iki çizim yolu da çağırıyor.
//! - **Merkezden birkaç santim kaçan üç jöle kuleyi patlatıyordu** (iki saniyede düğümler
//!   3800 m/s): temas, iç içe geçmiş bir düğümü tek seferde dışarı itiyordu ve açık entegre edilen
//!   bir yumuşak gövdede bu, elemanları bir sonraki adımda dağıtacak kadar geriyordu.
//!   **Düzeltildi:** bir itme en çok 5 mm; kalanı sonraki alt adımlara.
//!
//! *Gözlem, açık değil:* jöleler yumuşak, ve kule de öyle — dört blok 0,6–1,2 m'den bırakılınca
//! dimdik duruyor, 1,6 m'den bırakılınca devriliyor; altı blok bir saniyelik eğilmeyle yıkılabiliyor.
//! Oyunun zorluğu bu. Bloklar 1 m'den düşüyor.

use gizmo::core::input::Input;
use gizmo::physics::soft_body::SoftBodyMesh;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::simple::{set_text, FixedPhysics, SimpleAppExt, SimpleSceneState};

/// Platformun yarı genişliği; üstü y = 0'da.
const PLATFORM: f32 = 1.3;
const BLOCK: Vec3 = Vec3::new(1.0, 0.7, 1.0);
const SWING: f32 = 1.6;
const COOLDOWN: f32 = 1.4;
/// Kaç blok `--test`.
const TEST_BLOCKS: usize = 4;
const COLOURS: [&str; 6] = ["#E74C3C", "#F39C12", "#F1C40F", "#2ECC71", "#3498DB", "#9B59B6"];

fn spawn_world(world: &mut World) {
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    // Platform, ve çok aşağıda düşenleri tutan zemin.
    for (centre, half) in [
        (Vec3::new(0.0, -0.5, 0.0), Vec3::new(PLATFORM, 0.5, PLATFORM)),
        (Vec3::new(0.0, -8.5, 0.0), Vec3::new(30.0, 0.5, 30.0)),
    ] {
        world.spawn_bundle((Transform::new(centre), RigidBody::new_static(), Velocity::default(), Collider::box_collider(half)));
    }
}

/// Vincin bu andaki yatay konumu.
fn crane_x(clock: f32) -> f32 {
    (clock * 1.3).sin() * SWING
}

fn tower_top(world: &World) -> f32 {
    world
        .borrow::<SoftBodyMesh>()
        .iter()
        .map(|(_, b)| b.nodes.iter().map(|n| n.position.y).fold(f32::MIN, f32::max))
        .fold(0.0, f32::max)
}

fn drop_height(world: &World) -> f32 {
    tower_top(world) + 1.0
}

fn drop_block(world: &mut World, x: f32, k: usize, visual: bool) {
    let min = Vec3::new(x - BLOCK.x * 0.5, drop_height(world), -BLOCK.z * 0.5);
    let mut body = SoftBodyMesh::block(min, BLOCK, [2, 2, 2], 8.0, 15_000.0, 0.3).expect("geçerli malzeme");
    body.damping = 0.4;
    let id = world.spawn_bundle((body,)).id();
    if visual {
        if let Some(e) = world.get_entity(id) {
            let colour = Color::srgb_hex(COLOURS[k % COLOURS.len()]).to_vec4();
            world.add_component(e, MaterialDesc::new().with_pbr(colour, 0.25, 0.0));
        }
    }
}

/// Platformdan düşen blok var mı.
fn fallen(world: &World) -> bool {
    world.borrow::<SoftBodyMesh>().iter().any(|(_, b)| b.nodes.iter().map(|n| n.position.y).fold(f32::MAX, f32::min) < -2.0)
}

/// `--test`: vinç ortadayken altı blok, sonra beş saniye.
fn self_test() -> bool {
    let mut world = World::new();
    spawn_world(&mut world);
    let mut physics = FixedPhysics::new(60);
    let dt = 1.0 / 60.0;
    let (mut clock, mut since_drop, mut dropped, mut wait) = (0.0f32, COOLDOWN, 0usize, 0.0f32);
    let mut worst_speed = 0.0f32;
    while wait < 5.0 && clock < 120.0 {
        clock += dt;
        since_drop += dt;
        if dropped < TEST_BLOCKS {
            if since_drop >= COOLDOWN && crane_x(clock).abs() < 0.06 {
                drop_block(&mut world, crane_x(clock), dropped, false);
                dropped += 1;
                since_drop = 0.0;
            }
        } else {
            wait += dt;
        }
        physics.advance(&mut world, dt);
        world.apply_commands();
        if wait > 4.0 {
            let fastest = world
                .borrow::<SoftBodyMesh>()
                .iter()
                .flat_map(|(_, b)| b.nodes.iter().map(|n| n.velocity.length()).collect::<Vec<_>>())
                .fold(0.0f32, f32::max);
            worst_speed = worst_speed.max(fastest);
        }
    }
    let top = tower_top(&world);
    let lost = fallen(&world);
    let ok = dropped == TEST_BLOCKS && !lost && top > TEST_BLOCKS as f32 * BLOCK.y * 0.8 && worst_speed < 1.0;
    println!(
        "jole --test: {dropped} blok bırakıldı; kule {top:.2} m ({:.2} m olurdu dimdik); düşen {}; son saniyede en hızlı düğüm {worst_speed:.2} m/s → {}",
        TEST_BLOCKS as f32 * BLOCK.y,
        if lost { "VAR" } else { "yok" },
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

struct Game {
    physics: FixedPhysics,
    clock: f32,
    since_drop: f32,
    dropped: usize,
    over: bool,
    crane: u32,
    camera: u32,
    hud: u32,
    banner: u32,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Jöle", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = scene.renderer.default_font();
            spawn_world(scene.world);
            for (centre, half, colour) in [
                (Vec3::new(0.0, -0.5, 0.0), Vec3::new(PLATFORM, 0.5, PLATFORM), "#7F8C8D"),
                (Vec3::new(0.0, -8.5, 0.0), Vec3::new(30.0, 0.5, 30.0), "#2C3E50"),
            ] {
                scene.world.spawn_bundle((
                    Transform::new(centre).with_scale(half),
                    GlobalTransform::default(),
                    MeshSource("standard_cube".into()),
                    MaterialDesc::new().with_pbr(Color::srgb_hex(colour).to_vec4(), 0.8, 0.0),
                    MeshRenderer::new(),
                ));
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.8)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 2.4, LightRole::Sun),
            ));
            let crane = scene
                .world
                .spawn_bundle((
                    Transform::new(Vec3::new(0.0, 2.0, 0.0)).with_scale(Vec3::new(0.15, 0.15, 0.6)),
                    GlobalTransform::default(),
                    MeshSource("standard_cube".into()),
                    MaterialDesc::new().with_unlit(Color::srgb_hex("#ECF0F1").to_vec4()),
                    MeshRenderer::new(),
                ))
                .id();
            let camera = scene
                .world
                .spawn_bundle(CameraBundle { position: Vec3::new(0.0, 3.0, 9.0), pitch: -0.15, primary: true, ..Default::default() })
                .id();
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),)).id();
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();
            scene.world.insert_resource(Game {
                physics: FixedPhysics::new(60),
                clock: 0.0,
                since_drop: COOLDOWN,
                dropped: 0,
                over: false,
                crane,
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
        let blocks: Vec<u32> = world.borrow::<SoftBodyMesh>().iter().map(|(id, _)| id).collect();
        for id in blocks {
            if let Some(e) = world.get_entity(id) {
                world.despawn(e);
            }
        }
        game.dropped = 0;
        game.over = false;
        game.since_drop = COOLDOWN;
        game.physics.reset();
    }
    game.clock += dt;
    game.since_drop += dt;
    let x = crane_x(game.clock);
    let auto = std::env::var_os("JOLE_OTO").is_some();
    let wants = if auto { x.abs() < 0.06 } else { input.is_key_just_pressed(KeyCode::Space as u32) };
    if !game.over && wants && game.since_drop >= COOLDOWN {
        drop_block(world, x, game.dropped, true);
        game.dropped += 1;
        game.since_drop = 0.0;
    }
    game.physics.advance(world, dt);
    if fallen(world) {
        game.over = true;
    }
    let top = tower_top(world);
    let hook = drop_height(world) + BLOCK.y + 0.3;
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.crane) {
        t.position = Vec3::new(x, hook, 0.0);
    }
    if let Some(mut c) = world.borrow_mut::<Transform>().get_mut(game.camera) {
        let want = Vec3::new(0.0, top + 2.0, 9.0);
        c.position = c.position.lerp(want, (dt * 2.0).min(1.0));
        let look = (Vec3::new(0.0, top * 0.7, 0.0) - c.position).normalize_or_zero();
        c.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, look);
    }
    set_text(
        world,
        game.hud,
        &format!(
            "kule {top:.2} m   blok {}\nSpace: bırak   R: baştan{}",
            game.dropped,
            if game.since_drop < COOLDOWN { "   (vinç doluyor…)" } else { "" }
        ),
    );
    let banner = if game.over { format!("Bir blok düştü!\nKule {top:.2} m — R ile yeniden") } else { String::new() };
    set_text(world, game.banner, &banner);
    world.insert_resource(game);
}
