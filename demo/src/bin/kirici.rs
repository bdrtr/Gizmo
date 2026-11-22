//! # Kırıcı — kuralları Lua'da yazılmış bir tuğla kırma oyunu
//!
//! Raketle topu karşıla, tuğlaları kır. Oyunun bütünü `demo/assets/scripts/kirici.lua`'da: duvarlar,
//! tuğlalar, raket ve top oradan doğar; tuğla, betiğin okuduğu çarpışmalarla kırılır. Rust tarafı
//! yalnız kamerayı ve ışığı kurar, motorun `PlayLoop`'u ile oynatır — editörün ▶'si ve dışa aktarılan
//! her oyun bu döngüyle oynar — ve skoru HUD'a yazar.
//!
//! Motorun hangi yollarını yürüttüğü:
//!
//! - **Lua'nın fizik olayları** (`physics.collisions`), her karede, adımları ne olursa olsun. Bu oyun
//!   tasarlanırken bulunan: betik yalnız son fizik adımının olaylarını görüyordu; iki adımlı bir
//!   karede birincisini kaybediyor, adımsız bir karede sonuncuyu yeniden görüyordu (`5ce6ee57`).
//!   Ölçüldü, o düzeltme geri alınınca bu `--test` 30 fps'te 29/32 tuğlada kalıyor (kaybolan
//!   vuruşlar: top tuğladan kırmadan seker) ve 144 fps'te 23/32'de (raket vuruşu ikinci kez
//!   işlenir, raket o arada yer değiştirmiştir, topun açısı bozulur).
//! - **Lua'dan collider malzemesi** (`physics.set_material`): top ve duvarlar sürtünmesiz ve tam
//!   sekmeli. Bu oyunla eklendi — sürtünme ve sekme `add_rigidbody`'den collider'a taşındığında
//!   hiçbir Lua çağrısı onları almamıştı; bir betiğin yaptığı her collider 0.3 sekiyordu.
//! - **Betikten doğan ve yok edilen varlıklar**: `entity.spawn`'ın döndürdüğü kimlik, collider'lar,
//!   gövde, `entity.destroy`.
//! - **Yalnız çarpıştırıcı olan tuğlalar, duvarlar ve raket**: statik geometri; raket her karede
//!   `entity.set_position` ile taşınır.
//!
//! Topun yönü iki sınırın içinde tutulur: yatay bileşen hızın en az dörtte biri, çünkü çözücü
//! 1 m/s'nin altında çarpan bir teması sektirmez (`restitution_velocity_threshold`) ve duvara
//! yavaş değen top duvar boyunca dikey bir döngüye girer — ilk sürümde 144 fps'te tam bu oldu.
//!
//! `kirici --test`: otomatik pilotla (raket topu izler) 30, 60 ve 144 fps'te oynar; her birinde
//! bütün tuğlalar kırılmalı, skor kırılan tuğla sayısına eşit olmalı ve sahnede tuğla kalmamalı.
//!
//! **Tuşlar:** ← → (ya da A D) raketi sürer.
use gizmo::core::component::EntityName;
use gizmo::core::input::Input;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::scripting::{ScriptEngine, ScriptValue};
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::systems::{PlayLoop, PlayReport};

const SCRIPT: &str = include_str!("../../assets/scripts/kirici.lua");
const NAME: &str = "kirici.lua";
const BRICKS: usize = 32;

fn engine(auto: bool) -> ScriptEngine {
    let mut engine = ScriptEngine::new().expect("Lua başlatılamadı");
    engine.load_script_source(NAME, SCRIPT.to_string()).expect("kirici.lua yüklenemedi");
    if auto {
        engine.set_global(NAME, "OTO", ScriptValue::Bool(true)).expect("betik yüklü");
    }
    engine
}

fn report(r: PlayReport<'_>) {
    match r {
        PlayReport::ScriptError { error } => eprintln!("kirici: betik hatası: {error}"),
        PlayReport::ScriptLog { level, message } if level != "info" => eprintln!("kirici: {message}"),
        _ => {}
    }
}

fn global(world: &mut World, name: &str) -> Option<ScriptValue> {
    world.get_resource_mut::<ScriptEngine>().and_then(|mut e| e.get_global(NAME, name))
}

fn number(world: &mut World, name: &str) -> f64 {
    match global(world, name) {
        Some(ScriptValue::Num(n)) => n,
        _ => -1.0,
    }
}

fn text(world: &mut World, name: &str) -> String {
    match global(world, name) {
        Some(ScriptValue::Text(t)) => t,
        other => format!("{other:?}"),
    }
}

/// Sahnede kalan tuğlalar: adı `tugla` olan varlıklar.
fn bricks_left(world: &World) -> usize {
    world.query::<&EntityName>().iter().filter(|n| n.0 == "tugla").count()
}

/// Bir oyun, `fps` kare hızında, en çok iki dakika: (skor, durum, kalan tuğla).
fn play_at(fps: f32) -> (f64, String, usize) {
    let mut world = World::new();
    world.insert_resource(engine(true));
    let mut play = PlayLoop::new();
    let input = Input::default();
    let frames = (120.0 * fps) as usize;
    for _ in 0..frames {
        play.step(&mut world, 1.0 / fps, &input, &mut report);
        world.apply_commands();
        if text(&mut world, "durum") != "oynuyor" {
            break;
        }
    }
    // Son karenin komutları (yok edilen tuğla) uygulanmış olsun.
    play.step(&mut world, 1.0 / fps, &input, &mut report);
    world.apply_commands();
    (number(&mut world, "skor"), text(&mut world, "durum"), bricks_left(&world))
}

/// `--test`: üç kare hızında, otomatik pilotla.
fn self_test() -> bool {
    let mut ok = true;
    let mut lines = Vec::new();
    for fps in [30.0, 60.0, 144.0] {
        let (score, state, left) = play_at(fps);
        let good = state == "kazandi" && score == BRICKS as f64 && left == 0;
        ok &= good;
        lines.push(format!("{fps:.0} fps: skor {score} / {BRICKS} ({state}), sahnede {left} tuğla"));
    }
    println!("kirici --test: {} → {}", lines.join("; "), if ok { "tamam" } else { "BAŞARISIZ" });
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
    App::<SimpleSceneState>::new("Gizmo Engine - Kırıcı", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.3) * Quat::from_rotation_x(-0.8)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.97, 0.92), 2.5, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 6.0, 17.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: 0.0,
                primary: true,
                ..Default::default()
            });
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
            scene.world.insert_resource(engine(std::env::var_os("KIRICI_OTO").is_some()));
            scene.world.insert_resource(Game { play: PlayLoop::new(), hud });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.play.step(world, dt, input, &mut report);
    let line = format!("Skor {}   Kalan {}   {}", number(world, "skor"), number(world, "kalan"), text(world, "durum"));
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
