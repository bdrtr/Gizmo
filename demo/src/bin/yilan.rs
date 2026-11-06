//! # Yılan — kuralları Lua'da yazılmış bir oyun
//!
//! 16 × 16 bir tahta, bir yılan, bir yem; yedikçe uzar, kendine ya da kenara çarparsa biter.
//! Motorun yirmi beşinci oyunu ve kuralları baştan sona **Lua**'da olan ilki
//! (`demo/assets/scripts/yilan.lua`, ikiliye `include_str!` ile gömülü). Rust tarafı yalnız
//! tahtayı, kamerayı ve HUD'u kuruyor; her kare facade'in oyun döngüsünü (`PlayLoop::step`,
//! editörün ▶'u ve dışa aktarılan oyunun da çalıştırdığı adım) çağırıyor ve skoru betikten okuyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **oklar · WASD** | dön |
//! | **R · boşluk** | bitince yeniden |
//!
//! `yilan --test` pencere açmaz: betik kendi kendine oynar (`OTO = true`), sonra skor, durum ve
//! tahtadaki parça sayısı denetlenir.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **`entity.spawn` hiçbir şey döndürmüyordu.** Bir betik yarattığı şeyi sonra adıyla bile
//!   anamıyordu — yılanın parçalarını taşıyamaz, kuyruğunu silemezdi. **Düzeltildi:**
//!   `on_update` sırasında `spawn` ve `spawn_prefab` yeni varlığın kimliğini döndürüyor; aynı
//!   karede o kimliğe verilen komutlar ona ulaşıyor.
//! - **Betikten görünen bir şey yaratılamıyordu.** `spawn` yalnız bir `Transform` veriyordu, mesh
//!   ya da renk verecek bir çağrı yoktu. **Düzeltildi:** `entity.set_mesh`, `entity.set_color`.
//! - **Oyun betiğin değerini okuyamıyordu, ona değer veremiyordu** — skor HUD'a yazılamazdı.
//!   **Düzeltildi:** `ScriptEngine::get_global` / `set_global`.
//! - **Betik yalnız bir dosyadan yüklenebiliyordu.** **Düzeltildi:** `load_script_source`.

use gizmo::core::input::Input;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::scripting::{ScriptEngine, ScriptValue};
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::systems::{PlayLoop, PlayReport};

const SCRIPT: &str = include_str!("../../assets/scripts/yilan.lua");
const NAME: &str = "yilan.lua";
const N: f32 = 16.0;

fn engine(auto: bool) -> ScriptEngine {
    let mut engine = ScriptEngine::new().expect("Lua başlatılamadı");
    engine.load_script_source(NAME, SCRIPT.to_string()).expect("yilan.lua yüklenemedi");
    if auto {
        engine.set_global(NAME, "OTO", ScriptValue::Bool(true)).expect("betik yüklü");
    }
    engine
}

fn report(r: PlayReport<'_>) {
    match r {
        PlayReport::ScriptError { error } => eprintln!("yilan: betik hatası: {error}"),
        PlayReport::ScriptLog { level, message } if level != "info" => eprintln!("yilan: {message}"),
        _ => {}
    }
}

fn global(world: &mut World, name: &str) -> Option<ScriptValue> {
    world.get_resource_mut::<ScriptEngine>().and_then(|mut e| e.get_global(NAME, name))
}

/// Tahtadaki yılan parçaları: adı `yilan` olan, mesh'i ve rengi olan varlıklar.
fn segments(world: &World) -> (usize, usize) {
    let meshes = world.borrow::<gizmo::core::component::MeshSource>();
    let colours = world.borrow::<MaterialDesc>();
    let ids: Vec<u32> = world.query::<(Entity, &gizmo::core::EntityName)>().unwrap().iter().filter(|(_, n)| n.0 == "yilan").map(|(entity, _)| entity.id()).collect();
    let drawn = ids.iter().filter(|id| meshes.get(**id).is_some() && colours.get(**id).is_some()).count();
    (ids.len(), drawn)
}

/// `--test`: kendi kendine 60 saniye oynar.
fn self_test() -> bool {
    let mut world = World::new();
    world.insert_resource(engine(true));
    let mut play = PlayLoop::new();
    let input = Input::default();
    let mut most = 0.0f64;
    for _ in 0..60 * 60 {
        play.step(&mut world, 1.0 / 60.0, &input, &mut report);
        if let Some(ScriptValue::Num(s)) = global(&mut world, "skor") {
            most = most.max(s);
        }
        if let Some(ScriptValue::Text(t)) = global(&mut world, "durum") {
            if t == "bitti" {
                break;
            }
        }
    }
    let score = match global(&mut world, "skor") {
        Some(ScriptValue::Num(s)) => s,
        _ => -1.0,
    };
    let state = match global(&mut world, "durum") {
        Some(ScriptValue::Text(t)) => t,
        other => format!("{other:?}"),
    };
    let (count, drawn) = segments(&world);
    // Yılan üç parçayla başlar, her yemde bir uzar: tahtada tam o kadar parça olmalı.
    let expected = 3 + score as usize;
    let ok = score >= 10.0 && count == expected && drawn == count;
    println!(
        "yilan --test: skor {score} ({state}), tahtada {count} parça (beklenen {expected}), {drawn} tanesi mesh'li ve renkli → {}",
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

struct Game {
    play: PlayLoop,
    hud: u32,
    banner: u32,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Yılan", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-1.1)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.97, 0.92), 1.8, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 17.0, 7.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -1.18,
                primary: true,
                ..Default::default()
            });
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.05, 0.0)).with_scale(Vec3::new(N / 2.0, 0.05, N / 2.0)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#2C3E50").to_vec4(), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),)).id();
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();
            scene.world.insert_resource(engine(std::env::var_os("YILAN_OTO").is_some()));
            scene.world.insert_resource(Game { play: PlayLoop::new(), hud, banner });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.play.step(world, dt.min(0.05), input, &mut report);
    let score = match global(world, "skor") {
        Some(ScriptValue::Num(s)) => s as u32,
        _ => 0,
    };
    let over = matches!(global(world, "durum"), Some(ScriptValue::Text(t)) if t == "bitti");
    set_text(world, game.hud, &format!("skor {score}\noklar / WASD: dön"));
    set_text(world, game.banner, if over { "Bitti!\nR ya da boşluk: yeniden" } else { "" });
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
