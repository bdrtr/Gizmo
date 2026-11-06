//! # Atölye — blok blok kur, kaydet, geri yükle
//!
//! Yukarıdan bakan 12×12 bir tezgâh. Tıkladığın sütuna bir blok koyarsın, sağ tık en üsttekini
//! alır; boşluk imlecin üstünden bir top bırakır, yaptığın şeyin sağlam olup olmadığını görürsün.
//! **F5** kurduğunu diske yazar, **F9** geri getirir. Motorun yirmi üçüncü oyunu ve **sahne
//! dosyası** kullanan ilki: kayıt, motorun sahne biçimi (RON) — yalnız oyunun blokları, kamera ve
//! HUD değil.
//!
//! | girdi | iş |
//! |---|---|
//! | **sol tık** | blok koy |
//! | **sağ tık** | en üstteki bloğu al |
//! | **1 – 4** | renk |
//! | **boşluk** | top bırak |
//! | **F5 / F9** | kaydet / yükle |
//! | **C** | tezgâhı boşalt |
//!
//! `atolye --test` pencere açmaz: bir yapı kurar, kaydeder, siler, geri yükler ve her bloğun yeri,
//! rengi, çarpıştırıcısı ve mesh'i geri geldi mi diye bakar.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Dünyanın bir kısmını kaydetmenin yolu yoktu.** `SceneData::save` bütün dünyayı yazıyor —
//!   kamerayı, ışığı, HUD'u da; yükleyen her seferinde bir kamera daha kazanırdı. Seçilen
//!   varlıkları yazan `serialize_entities` vardı ama `SceneData` `#[non_exhaustive]`, oyun onu
//!   kuramıyordu: `default()` alıp alanlarını tek tek atamak gerekiyordu, eklemler de dışarıda
//!   kalıyordu. **Düzeltildi:** `SceneData::of_entities` (aradaki eklemlerle) ve
//!   `SceneData::instantiate` (eklemleri yeni kimliklere bağlayarak).
//!
//! Gerisi ilk denemede çalıştı: `full_scene_registry` + oyunun kendi `Block`'u, `SaveData`,
//! `cursor_on_plane`. Bir gözlem, eksik değil: bir blok RON'da ~1,7 KB — her bileşen bütün
//! alanlarını yazıyor (varsayılanlar ve `RigidBody`'nin kuvvet biriktiricileri dahil).
//!
//! ## Ortam değişkenleri
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::app::save::SaveData;
use gizmo::core::input::Input;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::scene::registry::SceneRegistry;
use gizmo::scene::scene::SceneData;
use gizmo::simple::{sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};
use serde::{Deserialize, Serialize};

const GRID: i32 = 12;
const HALF: f32 = 0.5;
const COLOURS: [&str; 4] = ["#C0392B", "#2980B9", "#27AE60", "#F1C40F"];

/// Oyunun bloğu: hangi sütunda, kaçıncı katta, hangi renk. Sahne dosyasına bu da yazılır, yükleyen
/// taraf sütun yüksekliklerini bundan kurar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Block {
    x: i32,
    z: i32,
    level: i32,
    colour: u8,
}
gizmo::core::impl_component!(Block);

/// Bırakılan toplar: kaydedilmez.
#[derive(Clone, Copy)]
struct Ball;
gizmo::core::impl_component!(Ball);

fn registry() -> SceneRegistry {
    let mut reg = gizmo::full_scene_registry();
    reg.register_serializable::<Block>("Block").expect("'Block' motorun bir adıyla çakışmamalı");
    reg
}

fn block_position(x: i32, z: i32, level: i32) -> Vec3 {
    Vec3::new(x as f32 + HALF - GRID as f32 * 0.5, HALF + level as f32, z as f32 + HALF - GRID as f32 * 0.5)
}

fn colour(i: u8) -> Vec4 {
    Color::srgb_hex(COLOURS[i as usize % COLOURS.len()]).to_vec4()
}

fn blocks(world: &World) -> Vec<(Entity, Block)> {
    world.query::<(Entity, &Block)>().iter().map(|(entity, b)| (entity, *b)).collect()
}

/// Sütunun bir sonraki boş katı.
fn height(world: &World, x: i32, z: i32) -> i32 {
    blocks(world).iter().filter(|(_, b)| b.x == x && b.z == z).map(|(_, b)| b.level + 1).max().unwrap_or(0)
}

fn place(world: &mut World, x: i32, z: i32, colour_index: u8) {
    let level = height(world, x, z);
    world.spawn_bundle((
        Transform::new(block_position(x, z, level)).with_scale(Vec3::splat(HALF)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(colour(colour_index), 0.8, 0.0),
        MeshRenderer::new(),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(Vec3::splat(HALF)),
        Block { x, z, level, colour: colour_index },
    ));
}

fn remove_top(world: &mut World, x: i32, z: i32) {
    let top = blocks(world).into_iter().filter(|(_, b)| b.x == x && b.z == z).max_by_key(|(_, b)| b.level);
    if let Some(e) = top.and_then(|(id, _)| world.live(id)) {
        world.despawn(e);
    }
}

fn clear(world: &mut World) {
    let ids: Vec<Entity> = blocks(world).into_iter().map(|(id, _)| id).chain(world.query::<(Entity, &Ball)>().iter().map(|(entity, _)| entity)).collect();
    for id in ids {
        if let Some(e) = world.live(id) {
            world.despawn(e);
        }
    }
}

/// Yalnız blokları, motorun sahne biçiminde.
fn save_level(world: &World) -> Result<String, String> {
    let mut ids: Vec<Entity> = blocks(world).into_iter().map(|(id, _)| id).collect();
    ids.sort_unstable();
    SceneData::of_entities(world, ids, &registry()).to_ron_string().map_err(|e| e.to_string())
}

/// Tezgâhı boşaltır ve `ron`'daki blokları kurar.
fn load_level(world: &mut World, ron: &str) -> Result<usize, String> {
    let scene = SceneData::from_ron_str(ron).map_err(|e| e.to_string())?;
    clear(world);
    Ok(scene.instantiate(world, &registry()).len())
}

/// `--test`: kur, kaydet, sil, yükle, karşılaştır.
fn self_test() -> bool {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new());
    for i in 0..30 {
        place(&mut world, (i * 7) % GRID, (i * 5) % GRID, (i % 4) as u8);
    }
    let before = snapshot(&world);
    let ron = match save_level(&world) {
        Ok(r) => r,
        Err(e) => {
            println!("atolye --test: kaydedilemedi: {e}");
            return false;
        }
    };
    clear(&mut world);
    world.apply_commands();
    let loaded = load_level(&mut world, &ron);
    world.apply_commands();
    let after = snapshot(&world);
    let same = before == after;
    println!(
        "atolye --test: {} blok kuruldu, dosya {} bayt, {:?} yüklendi, {} blok geri geldi → {}",
        before.len(),
        ron.len(),
        loaded,
        after.len(),
        if same { "aynı" } else { "FARKLI" }
    );
    if !same {
        for (b, a) in before.iter().zip(&after) {
            if b != a {
                println!("  önce {b:?}\n  sonra {a:?}");
                break;
            }
        }
    }
    same
}

/// Her bloğun karşılaştırılacak hâli: bloğun kendisi, yeri, rengi, çarpıştırıcısı var mı, mesh'i.
type Look = (Block, [i32; 3], [i32; 4], bool, Option<String>);
fn snapshot(world: &World) -> Vec<Look> {
    let ts = world.borrow::<Transform>();
    let ms = world.borrow::<MaterialDesc>();
    let cs = world.borrow::<Collider>();
    let ss = world.borrow::<MeshSource>();
    let q = |v: f32| (v * 1000.0).round() as i32;
    let mut out: Vec<Look> = blocks(world)
        .into_iter()
        .map(|(id, b)| {
            let p = ts.get(id).map(|t| t.position).unwrap_or(Vec3::NAN);
            let c = ms.get(id).map(|m| m.albedo).unwrap_or(Vec4::ZERO);
            (b, [q(p.x), q(p.y), q(p.z)], [q(c.x), q(c.y), q(c.z), q(c.w)], cs.get(id).is_some(), ss.get(id).map(|s| s.0.clone()))
        })
        .collect();
    out.sort_by_key(|l| (l.0.x, l.0.z, l.0.level));
    out
}

struct Game {
    physics: FixedPhysics,
    colour: u8,
    hud: Entity,
    message: String,
    cursor: Entity,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Atölye", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.7) * Quat::from_rotation_x(-0.95)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 1.8, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 13.0, 11.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -0.85,
                primary: true,
                ..Default::default()
            });
            let half = GRID as f32 * 0.5;
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.1, 0.0)).with_scale(Vec3::new(half, 0.1, half)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#8B7355").to_vec4(), 0.9, 0.0),
                MeshRenderer::new(),
                RigidBody::new_static(),
                Velocity::default(),
                Collider::box_collider(Vec3::new(half, 0.1, half)),
            ));
            // İmlecin altındaki sütunu gösteren ince bir çerçeve.
            let cursor = scene
                .world
                .spawn_bundle((
                    Transform::new(Vec3::new(0.0, 0.02, 0.0)).with_scale(Vec3::new(HALF, 0.02, HALF)),
                    GlobalTransform::default(),
                    MeshSource("standard_cube".into()),
                    MaterialDesc::new().with_unlit(Vec4::new(1.0, 1.0, 1.0, 0.35)),
                    MeshRenderer::new(),
                ))
                ;
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
            scene.world.insert_resource(Game { physics: FixedPhysics::new(60), colour: 0, hud, message: String::new(), cursor });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    for (i, key) in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4].into_iter().enumerate() {
        if input.is_key_just_pressed(key as u32) {
            game.colour = i as u8;
        }
    }
    let cell = gizmo::pointer::cursor_on_plane(world, Vec3::ZERO, Vec3::Y).and_then(|p| {
        let (x, z) = ((p.x + GRID as f32 * 0.5).floor() as i32, (p.z + GRID as f32 * 0.5).floor() as i32);
        ((0..GRID).contains(&x) && (0..GRID).contains(&z)).then_some((x, z))
    });
    if let Some((x, z)) = cell {
        if input.is_mouse_button_just_pressed(0) {
            place(world, x, z, game.colour);
        }
        if input.is_mouse_button_just_pressed(1) {
            remove_top(world, x, z);
        }
        if input.is_key_just_pressed(KeyCode::Space as u32) {
            let at = block_position(x, z, height(world, x, z) + 3);
            world.spawn_bundle((
                Transform::new(at).with_scale(Vec3::splat(0.35)),
                GlobalTransform::default(),
                MeshSource("sphere".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#ECF0F1").to_vec4(), 0.3, 0.2),
                MeshRenderer::new(),
                RigidBody::new(2.0, true),
                Velocity::default(),
                Collider::sphere(0.35),
                Ball,
            ));
        }
        let top = height(world, x, z);
        if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.cursor) {
            t.position = block_position(x, z, top) - Vec3::Y * (HALF - 0.02);
        }
    }
    if input.is_key_just_pressed(KeyCode::KeyC as u32) {
        clear(world);
        game.message = "tezgâh boşaltıldı".into();
    }
    if input.is_key_just_pressed(KeyCode::F5 as u32) {
        let mut save = SaveData::open("atolye");
        game.message = match save_level(world).and_then(|ron| save.set("level", &ron).and_then(|()| save.save()).map_err(|e| e.to_string())) {
            Ok(()) => format!("{} blok kaydedildi", blocks(world).len()),
            Err(e) => format!("kaydedilemedi: {e}"),
        };
    }
    if input.is_key_just_pressed(KeyCode::F9 as u32) {
        let save = SaveData::open("atolye");
        game.message = match save.get::<String>("level") {
            Some(ron) => match load_level(world, &ron) {
                Ok(n) => format!("{n} blok yüklendi"),
                Err(e) => format!("yüklenemedi: {e}"),
            },
            None => "kayıt yok".into(),
        };
    }
    game.physics.advance(world, dt.min(0.05));
    sync_transforms(world);
    let text = format!(
        "{} blok   renk {}\nsol tık: koy   sağ tık: al   1-4: renk   boşluk: top\nF5: kaydet   F9: yükle   C: boşalt\n{}",
        blocks(world).len(),
        game.colour + 1,
        game.message
    );
    if let Some(mut t) = world.borrow_mut::<Text>().get_mut(game.hud) {
        if t.content != text {
            t.content = text;
        }
    }
    world.insert_resource(game);
}

fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
