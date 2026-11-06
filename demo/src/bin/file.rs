//! # File — düşen meyveleri ağla yakala
//!
//! Yukarıdan meyveler düşüyor; iki direğe gerili bir file onları yakalıyor. Fileyi **A/D** (ya da
//! sol çubuk) ile sağa sola kaydır, meyve file içinde durunca sepete sayılır, yere düşerse kaçar.
//! Beş kaçırınca biter. Motorun yirmi sekizinci oyunu ve kumaşın bir şeyi **taşıdığı** ilki: file
//! bir `Cloth`, meyveler rijit gövdeler; file onları geri itiyor, sönümü sallanmayı dindiriyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **A / D · sol çubuk** | fileyi kaydır |
//! | **R** | baştan |
//!
//! `file --test` pencere açmaz: duran bir filenin üstüne on meyve bırakılır; hepsi filede durmalı
//! (`FILE_DEBUG=1` kaçanın nereye düştüğünü yazar).
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Kumaş hiçbir şeyi geri itmiyordu** — bir hamağa bırakılan top kumaşı yarıp yere düşüyordu.
//!   **Düzeltildi:** `Cloth::step_coupled`, sürücü onu çalıştırıyor.
//! - **Kumaşın sönümü sabitti ve neredeyse yoktu** (saniyede %1). Fileye düşen meyve geldiği
//!   hızla zıplamaya devam ediyordu. **Düzeltildi:** `Cloth::damping` (bu file `0.05`).
//!
//! *Açık sandığımız, açık olmayan:* filede yatan meyve dördüncü saniyede hâlâ ~5 cm inip kalkıyor
//! ve bu "bağlaşımın kendi döngüsü" diye yazılmıştı. On beş saniye ölçülünce (2026-11-04) sönüyor:
//! yavaş sönen bir ~2 Hz zıplama, onuncu saniyede 2 mm, on üçüncüde sıfır. Yakalama yine de konuma
//! bakıyor, çünkü meyve o birkaç saniye boyunca hareket ediyor.

use gizmo::core::input::Input;
use gizmo::physics::cloth::Cloth;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

const N: usize = 11;
const NET_W: f32 = 2.4;
const NET_Y: f32 = 1.6;
const SPEED: f32 = 4.0;
const FIELD: f32 = 6.0;
const FRUIT_R: f32 = 0.16;
const MISSES: u32 = 5;

#[derive(Clone, Copy)]
struct Fruit {
    /// Filede ne kadar süredir duruyor.
    resting: f32,
}
gizmo::core::impl_component!(Fruit);

/// File: kenarları sabit, `x` merkezli, yatay.
fn net(x: f32) -> Cloth {
    let spacing = NET_W / (N - 1) as f32;
    let mut cloth = Cloth::new(N, N, spacing, 0.03);
    for n in &mut cloth.nodes {
        let p = Vec3::new(n.position.x - NET_W / 2.0 + x, NET_Y, n.position.y - NET_W / 2.0);
        n.position = p;
        n.prev_position = p;
    }
    for i in 0..N {
        for j in [0, N - 1] {
            cloth.pin_node(j * N + i);
            cloth.pin_node(i * N + j);
        }
    }
    cloth.damping = 0.05;
    cloth
}

/// Filenin sabit kenarını `dx` kadar kaydırır.
fn slide(cloth: &mut Cloth, dx: f32) {
    for n in &mut cloth.nodes {
        if n.inv_mass == 0.0 {
            n.position.x += dx;
            n.prev_position.x += dx;
        }
    }
}

fn spawn_fruit(world: &mut World, at: Vec3, visual: bool) {
    let col = Collider::sphere(FRUIT_R);
    let mut rb = RigidBody::new(0.3, true);
    rb.update_inertia_from_collider(&col);
    let e = world.spawn();
    world.add_component(e, Transform::new(at).with_scale(Vec3::splat(FRUIT_R)));
    world.add_component(e, rb);
    world.add_component(e, Velocity::default());
    world.add_component(e, col);
    world.add_component(e, Fruit { resting: 0.0 });
    if visual {
        world.add_component(e, GlobalTransform::default());
        world.add_component(e, MeshSource("sphere".into()));
        world.add_component(e, MaterialDesc::new().with_pbr(Color::srgb_hex("#E67E22").to_vec4(), 0.5, 0.0));
        world.add_component(e, MeshRenderer::new());
    }
}

/// Meyvelerin hesabı: filenin bandında bir buçuk saniye kalan yakalanır, yere inen kaçar.
/// `(yakalanan, kaçan)`. Hıza değil konuma bakılır: fileye düşen meyve birkaç saniye zıplar
/// (yavaş sönen bir salınım) ve hızı o sürede sıfıra yakın değildir.
fn settle(world: &mut World, dt: f32) -> (u32, u32) {
    let fruits: Vec<(u32, Vec3, f32)> = {
        let ts = world.borrow::<Transform>();
        world.query::<(Entity, &Fruit)>().unwrap().iter().filter_map(|(entity, _)| Some((entity.id(), ts.get(entity)?.position, 0.0))).collect()
    };
    let (mut caught, mut missed) = (0, 0);
    for (id, p, speed) in fruits {
        let mut gone = false;
        if p.y < FRUIT_R + 0.1 {
            if std::env::var_os("FILE_DEBUG").is_some() {
                println!("  kaçtı: meyve {id} ({:.2}, {:.2}, {:.2})", p.x, p.y, p.z);
            }
            missed += 1;
            gone = true;
        } else if p.y > NET_Y - 1.2 && p.y < NET_Y + 0.3 {
            let _ = speed;
            let mut f = world.borrow_mut::<Fruit>();
            if let Some(mut f) = f.get_mut(id) {
                f.resting += dt;
                if f.resting > 1.5 {
                    caught += 1;
                    gone = true;
                }
            }
        }
        if gone {
            if let Some(e) = world.get_entity(id) {
                world.despawn(e);
            }
        }
    }
    (caught, missed)
}

fn ground(world: &mut World) {
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)),
    ));
}

/// `--test`: duran bir filenin üstüne on meyve, iki saniye arayla — biri yakalanıp kalkmadan öbürü
/// gelmesin (saniyede bir düşünce, gelen meyve yatanı fileden dışarı seken oldu).
fn self_test() -> bool {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    ground(&mut world);
    world.spawn_bundle((Transform::new(Vec3::ZERO), net(0.0)));
    let mut physics = FixedPhysics::new(60);
    let (mut caught, mut missed) = (0, 0);
    let dt = 1.0 / 60.0;
    for frame in 0..60 * 24 {
        if frame % 120 == 0 && frame < 60 * 20 {
            let k = (frame / 120) as f32;
            spawn_fruit(&mut world, Vec3::new((k * 0.37).sin() * 0.6, NET_Y + 2.5, (k * 0.71).cos() * 0.5), false);
        }
        physics.advance(&mut world, dt);
        world.apply_commands();
        let (c, m) = settle(&mut world, dt);
        caught += c;
        missed += m;
    }
    let ok = caught == 10 && missed == 0;
    println!("file --test: 10 meyve, {caught} yakalandı, {missed} kaçtı → {}", if ok { "tamam" } else { "BAŞARISIZ" });
    ok
}

struct Game {
    physics: FixedPhysics,
    net: u32,
    net_x: f32,
    caught: u32,
    missed: u32,
    clock: f32,
    next_drop: f32,
    seed: u64,
    hud: u32,
    banner: u32,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - File", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.8)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.96, 0.88), 1.8, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 4.5, 8.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -0.4,
                primary: true,
                ..Default::default()
            });
            ground(scene.world);
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.05, 0.0)).with_scale(Vec3::new(FIELD + 2.0, 0.05, 3.0)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#6A994E").to_vec4(), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            let net_id = scene
                .world
                .spawn_bundle((Transform::new(Vec3::ZERO), net(0.0), MaterialDesc::new().with_pbr(Color::srgb_hex("#F2E8CF").to_vec4(), 0.9, 0.0)))
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
                net: net_id,
                net_x: 0.0,
                caught: 0,
                missed: 0,
                clock: 0.0,
                next_drop: 1.0,
                seed: 0x9E37_79B9_7F4A_7C15,
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
    let over = game.missed >= MISSES;
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        let fruits: Vec<u32> = world.query::<(Entity, &Fruit)>().unwrap().iter().map(|(entity, _)| entity.id()).collect();
        for id in fruits {
            if let Some(e) = world.get_entity(id) {
                world.despawn(e);
            }
        }
        if let Some(mut c) = world.borrow_mut::<Cloth>().get_mut(game.net) {
            *c = net(0.0);
        }
        game.net_x = 0.0;
        game.caught = 0;
        game.missed = 0;
        game.clock = 0.0;
        game.next_drop = 1.0;
    }
    if !over {
        game.clock += dt;
        let (mx, _) = input.move_axis();
        let target = (game.net_x + mx * SPEED * dt).clamp(-FIELD / 2.0 + NET_W / 2.0, FIELD / 2.0 - NET_W / 2.0);
        let dx = target - game.net_x;
        game.net_x = target;
        if let Some(mut c) = world.borrow_mut::<Cloth>().get_mut(game.net) {
            slide(&mut c, dx);
        }
        if game.clock >= game.next_drop {
            game.seed ^= game.seed << 13;
            game.seed ^= game.seed >> 7;
            game.seed ^= game.seed << 17;
            let x = (game.seed % 1000) as f32 / 1000.0 * (FIELD - 0.6) - (FIELD - 0.6) / 2.0;
            spawn_fruit(world, Vec3::new(x, 5.0, 0.0), true);
            // Zamanla sıklaşır.
            game.next_drop = game.clock + (2.2 - game.clock * 0.02).max(0.8);
        }
        game.physics.advance(world, dt);
        let (c, m) = settle(world, dt);
        game.caught += c;
        game.missed += m;
    }
    sync_transforms(world);
    set_text(
        world,
        game.hud,
        &format!("yakalanan {}   kaçan {}/{MISSES}\nA/D: fileyi kaydır   R: baştan", game.caught, game.missed),
    );
    set_text(world, game.banner, if game.missed >= MISSES { "Bitti!\nR ile yeniden" } else { "" });
    world.insert_resource(game);
}

fn set_text(world: &mut World, id: u32, s: &str) {
    if let Some(mut t) = world.borrow_mut::<Text>().get_mut(id) {
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
