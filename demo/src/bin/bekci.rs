//! # Bekçi — kuralları varlıklara bağlı Lua betiklerinde
//!
//! Hırsız avlunun güney ucundan çıkar, üç devriye hattını geçer ve kuzeydeki beş mücevheri toplar;
//! bir bekçiye bir metreden fazla yaklaşırsa yakalanır. Rust tarafı yalnız sahneyi kurar: her şeyin
//! davranışı, varlıklara `Script` bileşeniyle bağlı üç betikte (`demo/assets/scripts/`):
//!
//! - `bekci.lua` — üç bekçi aynı dosyayı paylaşır; her birinin uçları ve hızı kendi `props`'unda.
//! - `mucevher.lua` — beş mücevher; tetikleyicisine hırsız girince toplanır, kendini yok eder.
//! - `hirsiz.lua` — hırsız; otomatik pilotu (`props.oto`) bir hattı yalnız bekçi uzak kalacaksa
//!   geçer. Elle: ok tuşları ya da WASD.
//!
//! Motorun hangi yollarını yürüttüğü — `kirici` betiği genel olarak yüklüyordu; burada betikler
//! varlıkların, ve sahne bir **sahne dosyasından** gelir, dışa aktarılan bir oyunun açtığı gibi:
//!
//! - **`on_init(id, props)`**, varlık başına bir kez. Bu oyunla bulunan: `Script::initialized` bu
//!   kancayı başından beri belgeliyordu ve hiçbir şey onu çağırmıyordu — bir betiğin kurulumu ne
//!   editörün ▶'sinde ne dışa aktarılan oyunda çalışıyordu. Burada bekçiler `on_init`'te yön alır;
//!   o olmadan hiçbiri kıpırdamaz.
//! - **Varlık başına `props`**, sahne dosyasından geçerek: üç bekçinin üç hızı.
//! - **Paylaşılan ortam**: aynı dosyayı çalıştıran varlıkların globalleri ortak (`toplanan`),
//!   kendi durumları kimlikle anahtarlı.
//! - **Betikten okunan tetikleyici olaylar** (`physics.triggers`), `RigidBody`'siz tetikleyici
//!   collider'larda, ve `scene.find_by_name`, `scene.get_all_entities`, `entity.destroy`.
//!
//! `bekci --test`: sahne kurulur, bir sahne dosyasına yazılır ve boş bir dünyaya oradan yüklenir;
//! sonra otomatik pilot dikkatli 30, 60 ve 144 fps'te oynar — her birinde beş mücevher toplanmalı,
//! hırsız yakalanmamalı, her bekçi kendi hattında kendi hızıyla gezmeli — ve dalgın 60 fps'te
//! oynar: yakalanmalı.
use gizmo::core::component::EntityName;
use gizmo::core::input::Input;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::scripting::{Script, ScriptEngine, ScriptValue};
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::systems::{PlayLoop, PlayReport};

const BEKCI: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/scripts/bekci.lua");
const MUCEVHER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/scripts/mucevher.lua");
const HIRSIZ: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/scripts/hirsiz.lua");
/// Bekçiler: (z, hız m/s). Hepsi x'te −6 ile 6 arasında gezer.
const GUARDS: [(f32, f64); 3] = [(-4.0, 2.0), (0.0, 3.0), (4.0, 2.5)];
const GUARD_END: f64 = 6.0;
const GEMS: [f32; 5] = [-6.0, -3.0, 0.0, 3.0, 6.0];
const GEM_Z: f32 = 6.5;

fn script(path: &str, props: &[(&str, ScriptValue)]) -> Script {
    let mut s = Script::new(path);
    for (name, value) in props {
        s.properties.insert((*name).into(), value.clone());
    }
    s
}

fn material(r: f32, g: f32, b: f32) -> MaterialDesc {
    MaterialDesc::new().with_pbr(Vec4::new(r, g, b, 1.0), 0.6, 0.0)
}

/// Avlu, bekçiler, mücevherler ve hırsız. `oto`: hırsızın otomatik pilotu ("dikkatli", "dalgin"),
/// ya da elle oynanacaksa `None`.
fn build_scene(world: &mut World, oto: Option<&str>) {
    world.spawn_bundle((
        EntityName::new("avlu"),
        Transform::new(Vec3::new(0.0, -0.5, 0.0)).with_scale(Vec3::new(8.0, 0.5, 10.0)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        material(0.35, 0.33, 0.3),
        MeshRenderer::new(),
        Collider::box_collider(Vec3::new(8.0, 0.5, 10.0)),
    ));
    for (i, (z, speed)) in GUARDS.iter().enumerate() {
        world.spawn_bundle((
            EntityName::new(&format!("bekci{}", i + 1)),
            Transform::new(Vec3::new(-GUARD_END as f32, 0.6, *z)).with_scale(Vec3::splat(0.5)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            material(0.8, 0.2, 0.15),
            MeshRenderer::new(),
            script(
                BEKCI,
                &[
                    ("ax", ScriptValue::Num(-GUARD_END)),
                    ("bx", ScriptValue::Num(GUARD_END)),
                    ("z", ScriptValue::Num(*z as f64)),
                    ("hiz", ScriptValue::Num(*speed)),
                ],
            ),
        ));
    }
    for x in GEMS {
        world.spawn_bundle((
            EntityName::new("mucevher"),
            Transform::new(Vec3::new(x, 0.5, GEM_Z)).with_scale(Vec3::splat(0.3)),
            GlobalTransform::default(),
            MeshSource("sphere".into()),
            material(0.2, 0.9, 0.5),
            MeshRenderer::new(),
            Collider::sphere(0.5).with_trigger(true),
            script(MUCEVHER, &[]),
        ));
    }
    let props: Vec<(&str, ScriptValue)> = oto.map(|o| ("oto", ScriptValue::Text(o.into()))).into_iter().collect();
    world.spawn_bundle((
        EntityName::new("hirsiz"),
        Transform::new(Vec3::new(0.0, 0.4, -8.0)).with_scale(Vec3::splat(0.4)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        material(0.2, 0.4, 0.95),
        MeshRenderer::new(),
        RigidBody::new(1.0, true),
        Velocity::default(),
        Collider::sphere(0.4),
        script(HIRSIZ, &props),
    ));
}

fn report(r: PlayReport<'_>) {
    match r {
        PlayReport::ScriptError { error } | PlayReport::EntityScriptError { error, .. } => eprintln!("bekci: betik hatası: {error}"),
        PlayReport::ScriptBroke { path, error } => eprintln!("bekci: {path} yüklenemedi: {error}"),
        PlayReport::ScriptLog { level, message } if level != "info" => eprintln!("bekci: {message}"),
        _ => {}
    }
}

fn global(world: &mut World, path: &str, name: &str) -> Option<ScriptValue> {
    world.get_resource_mut::<ScriptEngine>().and_then(|mut e| e.get_global(path, name))
}

fn text(world: &mut World, path: &str, name: &str) -> String {
    match global(world, path, name) {
        Some(ScriptValue::Text(t)) => t,
        other => format!("{other:?}"),
    }
}

fn number(world: &mut World, path: &str, name: &str) -> f64 {
    match global(world, path, name) {
        Some(ScriptValue::Num(n)) => n,
        _ => -1.0,
    }
}

fn named(world: &World, name: &str) -> Vec<Entity> {
    world.query::<(Entity, &EntityName)>().iter().filter(|(_, n)| n.0 == name).map(|(e, _)| e).collect()
}

/// Sahne kurulur, dosyaya yazılır ve boş bir dünyaya oradan yüklenir — dışa aktarılan oyunun yolu.
fn world_from_scene_file(oto: &str) -> World {
    let mut source = World::new();
    build_scene(&mut source, Some(oto));
    let registry = gizmo::full_scene_registry();
    let file = std::env::temp_dir().join(format!("gizmo_bekci_{}_{oto}.scene", std::process::id()));
    let path = file.to_string_lossy().to_string();
    gizmo::scene::SceneData::save(&source, &path, &registry).expect("sahne yazılamadı");
    let mut world = World::new();
    world.insert_resource(gizmo::physics::world::PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world.insert_resource(ScriptEngine::new().expect("Lua başlatılamadı"));
    gizmo::scene::SceneData::load_into(&path, &mut world, &registry).expect("sahne okunamadı");
    let _ = std::fs::remove_file(&file);
    world
}

/// Bir oyunun sonucu.
struct Outcome {
    state: String,
    gems: f64,
    gems_left: usize,
    inits: f64,
    /// Bekçi başına: hattından en çok sapması (m) ve ortalama hızı (yol / süre, m/s).
    guards: Vec<(f32, f64)>,
    /// Dikkatli pilotun bir hattın önünde beklediği toplam süre.
    waited: f64,
    seconds: f32,
}

fn play(oto: &str, fps: f32) -> Outcome {
    let mut world = world_from_scene_file(oto);
    let guards: Vec<Entity> = (1..=GUARDS.len()).map(|i| named(&world, &format!("bekci{i}"))[0]).collect();
    let mut off_lane = vec![0.0f32; guards.len()];
    let mut travelled = vec![0.0f64; guards.len()];
    let mut last_x: Vec<f32> = vec![-GUARD_END as f32; guards.len()];
    let mut loop_ = PlayLoop::new();
    let input = Input::default();
    let mut frames = 0;
    while (frames as f32) < 90.0 * fps {
        loop_.step(&mut world, 1.0 / fps, &input, &mut report);
        frames += 1;
        for (i, g) in guards.iter().enumerate() {
            let p = world.query::<&Transform>().get(*g).map(|t| t.position).expect("bekçinin Transform'u");
            let z = GUARDS[i].0;
            let past_end = (p.x.abs() - GUARD_END as f32).max(0.0);
            off_lane[i] = off_lane[i].max((p.z - z).abs()).max(past_end);
            travelled[i] += (p.x - last_x[i]).abs() as f64;
            last_x[i] = p.x;
        }
        if text(&mut world, HIRSIZ, "durum") != "oynuyor" {
            break;
        }
    }
    // Son karenin komutları uygulanmış olsun (yok edilen mücevher).
    loop_.step(&mut world, 1.0 / fps, &input, &mut report);
    let seconds = frames as f32 / fps;
    Outcome {
        state: text(&mut world, HIRSIZ, "durum"),
        gems: number(&mut world, MUCEVHER, "toplanan"),
        gems_left: named(&world, "mucevher").len(),
        inits: number(&mut world, BEKCI, "bekciler"),
        guards: off_lane.into_iter().zip(travelled.into_iter().map(|d| d / seconds as f64)).collect(),
        waited: number(&mut world, HIRSIZ, "bekleme"),
        seconds,
    }
}

/// `--test`: bkz. modül belgesi.
fn self_test() -> bool {
    let mut ok = true;
    let mut lines = Vec::new();
    for fps in [30.0, 60.0, 144.0] {
        let o = play("dikkatli", fps);
        // Her bekçi kendi hızıyla, yüzde birden yakın: üçünün `props`'u ayrı ve ayrı okunuyor.
        let paced = o.guards.iter().zip(GUARDS).all(|((_, measured), (_, speed))| (measured / speed - 1.0).abs() < 0.01);
        let on_lane = o.guards.iter().all(|(off, _)| *off < 1e-3);
        let good = o.state == "kazandi" && o.gems == GEMS.len() as f64 && o.gems_left == 0 && o.inits == GUARDS.len() as f64 && paced && on_lane && o.waited > 0.0;
        ok &= good;
        lines.push(format!(
            "dikkatli {fps:.0} fps: {} ({:.0} s, {:.1} s bekledi), {} mücevher, sahnede {} kaldı, {} on_init, bekçi hızları {:?}, hattan sapma {:.1e}",
            o.state,
            o.seconds,
            o.waited,
            o.gems,
            o.gems_left,
            o.inits,
            o.guards.iter().map(|(_, v)| (v * 100.0).round() / 100.0).collect::<Vec<_>>(),
            o.guards.iter().map(|(off, _)| *off).fold(0.0f32, f32::max)
        ));
    }
    let careless = play("dalgin", 60.0);
    let caught = careless.state == "yakalandi";
    ok &= caught;
    lines.push(format!("dalgın 60 fps: {} ({:.0} s)", careless.state, careless.seconds));
    println!("bekci --test: {} → {}", lines.join("; "), if ok { "tamam" } else { "BAŞARISIZ" });
    ok
}

struct Game {
    play: PlayLoop,
    hud: Entity,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Bekçi", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-1.0)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.97, 0.92), 2.5, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 16.0, -14.0),
                yaw: std::f32::consts::FRAC_PI_2,
                pitch: -0.85,
                primary: true,
                ..Default::default()
            });
            let oto = std::env::var_os("BEKCI_OTO").map(|_| "dikkatli");
            build_scene(scene.world, oto);
            scene.world.insert_resource(gizmo::physics::world::PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
            scene.world.insert_resource(ScriptEngine::new().expect("Lua başlatılamadı"));
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
            scene.world.insert_resource(Game { play: PlayLoop::new(), hud });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.play.step(world, dt, input, &mut report);
    let line = format!(
        "Mücevher {} / {}   {}",
        number(world, MUCEVHER, "toplanan").max(0.0),
        GEMS.len(),
        text(world, HIRSIZ, "durum")
    );
    if let Some(mut t) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        t.content = line;
    }
    world.insert_resource(game);
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
