//! # Lobut — gerçek rigid body'lerle küçük bir bowling
//!
//! Topu kulvarın başında hizala, nişan al, gücü topla, bırak. On lobut gerçek fizik cisimleri:
//! devrilenler sayılıyor. Beş çerçeve, çerçeve başına iki atış.
//!
//! | tuş | iş |
//! |---|---|
//! | **A / D** | topu sağa sola kaydır |
//! | **← / →** | nişan açısı |
//! | **Space** (basılı tut, bırak) | güç topla, at |
//! | **R** | oyunu baştan başlat |
//!
//! ## Neden var
//!
//! `kristal_avcisi` motorun 3D yolunu fiziksiz sınadı (çarpışma mesafe ölçümüydü). Bu oyun
//! fiziği oyunun merkezine koyuyor: ince, uzun, devrilmesi gereken cisimler (lobutlar), hızlı
//! küçük bir top (CCD), çarpışma olaylarından ses, fizik durulunca sayım. Yazarken
//! karşılaşılan her eksik aşağıda.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **`Velocity`'si olmayan bir cisim simülasyonda yoktu — sessizce.** Zemin
//!   `RigidBody::new_static()` + çarpıştırıcıydı; hızı yok, çünkü zemin hareket etmez. Fizik
//!   köprüsü `(RigidBody, Transform, Velocity)` sorguluyordu, zemin hiç girmedi ve ilk kareden
//!   itibaren on lobut ve top serbest düşüşteydi. Uyarı yoktu. **Kapatıldı:** statik cisim sıfır
//!   hızla hemen katılıyor, hareketli olana komut kuyruğundan bir `Velocity` ekleniyor.
//! - **Atalet çarpıştırıcıdan gelmiyordu.** `RigidBody::new` şekli bilemez, `(1,1,1)` yer tutucu
//!   koyuyordu ve kural "`update_inertia_from_collider`'ı kendin çağır"dı — `demo/`'daki 36
//!   `RigidBody::new`'in 28'i çağırmıyordu. 20 cm'lik top 10 kat zor dönüyordu: hiç
//!   yuvarlanmadı, 1.5 saniyede hızının %80'ini sürtünmeye verdi (5/7'de yuvarlanmaya geçmesi
//!   gerekirken). **Kapatıldı:** ECS fizik adımı yer tutucuyu ilk adımda çarpıştırıcının
//!   tensörüyle değiştiriyor; ölçülen yuvarlanma hızı 6.77 m/s, ders kitabı 6.80.
//! - **Silindir ilkeli yoktu.** `MeshSource` küp, küre ve düzlem biliyordu; lobutlar görünmezdi.
//!   **Kapatıldı:** `"cylinder"` ve `"cone"` (yarıçap 1, yarı yükseklik 1 — çarpıştırıcının
//!   argümanlarıyla aynı sayılar), ve ilkel adlarının tek listesi
//!   (`AssetManager::PRIMITIVE_MESH_KEYS`; stüdyo ikinci bir kopya tutuyordu).
//! - **SimpleApp kurulumdan sonra kendi `PhysicsWorld`'ünü yazıyordu.** Kurulumda yerçekimini
//!   değiştiren bir oyunun ayarı sessizce eziliyordu. **Kapatıldı:** kurulum bir tane koyduysa o
//!   kalıyor.
//!
//! ## Ortam değişkenleri
//!   * `GIZMO_FONT=<yol>` — HUD fontu
//!   * `LOBUT_AT=<güç 0..1>` — açılışta bir atışı kendiliğinden yap (ekran görüntüsü için)
//!   * `LOBUT_LOG=1` — her kare top ve ilk üç lobutun konumu stderr'e (fizik teşhisi için)

use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::core::event::EventReader;
use gizmo::core::input::Input;
use gizmo::core::query::{Query, With};
use gizmo::core::system::{Phase, ResMut};
use gizmo::physics::{Collider, CollisionEvent, CollisionEventType, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};
use gizmo::winit::keyboard::KeyCode;

const PHYSICS_HZ: u32 = 240;
const FRAMES: u32 = 5;
/// Kulvarın yarı genişliği ve topun başladığı / lobutların durduğu z.
const LANE_HALF: f32 = 0.9;
const BALL_Z: f32 = 0.0;
const PIN_Z: f32 = -16.0;
const BALL_R: f32 = 0.2;
const PIN_R: f32 = 0.07;
const PIN_HALF_H: f32 = 0.19;

#[derive(Clone, Copy)]
struct Pin;
gizmo::core::impl_component!(Pin);

#[derive(Clone, Copy)]
struct Ball;
gizmo::core::impl_component!(Ball);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase2 {
    /// Nişan alınıyor; top kulvarın başında.
    Aiming,
    /// Top yuvarlanıyor; bu kadar saniye geçti.
    Rolling(f32),
    /// Top gitti, lobutlar duruluyor; bu kadar saniye geçti.
    Settling(f32),
    Over,
}

struct Game {
    physics: FixedPhysics,
    phase: Phase2,
    /// Topun kulvardaki yanal konumu ve nişan açısı (radyan, sola pozitif).
    offset: f32,
    angle: f32,
    power: f32,
    frame: u32,
    roll: u32,
    /// Her çerçevenin devrilen lobut sayısı.
    scores: Vec<u32>,
    /// Bu çerçevede ilk atışta devrilenler.
    first_roll: u32,
    hud: u32,
    banner: u32,
    auto_shot: Option<f32>,
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Lobut", 1280, 720)
        .add_event::<CollisionEvent>()
        .with_simple_scene(|scene, _state| {
            let r = &mut *scene.renderer;
            r.bloom_intensity = 0.6;
            r.deband_dither = true;
            let font = load_font(r);

            // Kulvar: zemin, iki yan ray, arka çukurun duvarı. Hepsi statik kutu.
            let wood = MaterialDesc::new().with_pbr(Vec4::new(0.78, 0.6, 0.38, 1.0), 0.35, 0.0);
            let rail = MaterialDesc::new().with_pbr(Vec4::new(0.25, 0.25, 0.3, 1.0), 0.6, 0.3);
            spawn_static(scene.world, Vec3::new(0.0, -0.1, -9.5), Vec3::new(LANE_HALF, 0.1, 10.5), wood.clone());
            spawn_static(scene.world, Vec3::new(0.0, -0.3, -21.0), Vec3::new(LANE_HALF + 0.6, 0.1, 1.0), rail.clone());
            spawn_static(scene.world, Vec3::new(0.0, 0.4, -22.0), Vec3::new(LANE_HALF + 0.6, 0.6, 0.1), rail.clone());
            for x in [-(LANE_HALF + 0.05), LANE_HALF + 0.05] {
                spawn_static(scene.world, Vec3::new(x, 0.1, -9.5), Vec3::new(0.05, 0.2, 10.5), rail.clone());
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.9)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.95, 0.85), 2.5, LightRole::Sun),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 2.5, PIN_Z - 0.4)),
                GlobalTransform::default(),
                PointLight::new(Vec3::new(1.0, 0.9, 0.7), 8.0, 6.0),
            ));

            scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 1.3, 3.2),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -0.22,
                primary: true,
                ..Default::default()
            });

            let hud = scene
                .world
                .spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
                .id();
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 40.0, Vec2::new(0.5, 0.35), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();

            if let Ok(mut audio) = AudioManager::new() {
                audio.load_sound_bytes("tok", Blip::new(Wave::Noise, 2400.0, 0.09).with_slide(600.0).with_volume(0.6).wav());
                audio.load_sound_bytes("yuvarlan", Blip::new(Wave::Noise, 300.0, 0.6).with_slide(120.0).with_volume(0.15).wav());
                audio.load_sound_bytes("strike", Blip::new(Wave::Square, 523.0, 0.5).with_slide(1046.0).with_volume(0.25).wav());
                scene.world.insert_resource(audio);
            }

            let auto_shot = std::env::var("LOBUT_AT").ok().and_then(|v| v.parse::<f32>().ok());
            let mut game = Game {
                physics: FixedPhysics::new(PHYSICS_HZ),
                phase: Phase2::Aiming,
                offset: 0.0,
                angle: 0.0,
                power: 0.0,
                frame: 1,
                roll: 1,
                scores: Vec::new(),
                first_roll: 0,
                hud,
                banner,
                auto_shot,
            };
            reset_pins(scene.world, &mut game);
            scene.world.insert_resource(game);
        })
        .add_update_system(clatter.in_phase(Phase::Update))
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn spawn_static(world: &mut World, at: Vec3, half: Vec3, material: MaterialDesc) {
    world.spawn_bundle((
        Transform::new(at).with_scale(half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        material,
        MeshRenderer::new(),
        RigidBody::new_static(),
        Collider::box_collider(half),
    ));
}

/// On lobut, üçgen dizilişte; eskileri ve top silinir.
fn reset_pins(world: &mut World, game: &mut Game) {
    world.despawn_all_with::<Pin>();
    world.despawn_all_with::<Ball>();
    for row in 0..4 {
        for i in 0..=row {
            let x = (i as f32 - row as f32 * 0.5) * 0.3;
            let z = PIN_Z - row as f32 * 0.26;
            spawn_pin(world, Vec3::new(x, PIN_HALF_H, z));
        }
    }
    game.physics.reset();
}

fn spawn_pin(world: &mut World, at: Vec3) {
    world.spawn_bundle((
        Transform::new(at).with_scale(Vec3::new(PIN_R, PIN_HALF_H, PIN_R)),
        GlobalTransform::default(),
        // Silindir: "standard_cube" gibi birim boyutta bir ilkel, ölçekle boyutlanıyor.
        MeshSource("cylinder".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.95, 0.95, 0.93, 1.0), 0.25, 0.0),
        MeshRenderer::new(),
        RigidBody::new(1.5, true),
        Velocity::default(),
        Collider::cylinder(PIN_R, PIN_HALF_H),
        Pin,
    ));
}

fn spawn_ball(world: &mut World, game: &Game) {
    let dir = Vec3::new(-game.angle.sin(), 0.0, -game.angle.cos());
    let speed = 4.0 + game.power * 8.0;
    let mut body = RigidBody::new(6.0, true);
    body.ccd_enabled = true;
    world.spawn_bundle((
        Transform::new(Vec3::new(game.offset, BALL_R, BALL_Z)).with_scale(Vec3::splat(BALL_R)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.15, 0.25, 0.75, 1.0), 0.15, 0.4),
        MeshRenderer::new(),
        body,
        Velocity::new(dir * speed),
        Collider::sphere(BALL_R),
        Ball,
    ));
}

/// Devrilmiş: dik eksen 35°'den fazla yatmış, ya da kulvardan düşmüş.
fn knocked(tf: &Transform) -> bool {
    (tf.rotation * Vec3::Y).y < 0.82 || tf.position.y < -0.05 || tf.position.x.abs() > LANE_HALF
}

fn count_knocked(world: &World) -> u32 {
    let pins = world.borrow::<Pin>();
    let tfs = world.borrow::<Transform>();
    pins.iter().filter(|(id, _)| tfs.get(*id).is_some_and(knocked)).count() as u32
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };

    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        game.frame = 1;
        game.roll = 1;
        game.scores.clear();
        game.phase = Phase2::Aiming;
        reset_pins(world, &mut game);
    }

    match game.phase {
        Phase2::Aiming => {
            let (x, _) = input.move_axis();
            game.offset = (game.offset + x * 1.2 * dt).clamp(-(LANE_HALF - BALL_R - 0.05), LANE_HALF - BALL_R - 0.05);
            let left = input.is_key_pressed(KeyCode::ArrowLeft as u32) as i32 as f32;
            let right = input.is_key_pressed(KeyCode::ArrowRight as u32) as i32 as f32;
            game.angle = (game.angle + (left - right) * 0.25 * dt).clamp(-0.08, 0.08);
            let release = if let Some(p) = game.auto_shot.take() {
                game.power = p.clamp(0.0, 1.0);
                true
            } else if input.is_key_pressed(KeyCode::Space as u32) {
                game.power = (game.power + dt * 0.8).min(1.0);
                false
            } else {
                game.power > 0.0
            };
            if release {
                spawn_ball(world, &game);
                play(world, "yuvarlan");
                game.power = 0.0;
                game.phase = Phase2::Rolling(0.0);
            }
        }
        Phase2::Rolling(t) => {
            let ball_z = {
                let balls = world.borrow::<Ball>();
                let tfs = world.borrow::<Transform>();
                balls.iter().find_map(|(id, _)| tfs.get(id).map(|t| t.position.z))
            };
            let gone = ball_z.is_none_or(|z| z < PIN_Z - 2.0);
            game.phase = if gone || t > 7.0 { Phase2::Settling(0.0) } else { Phase2::Rolling(t + dt) };
        }
        Phase2::Settling(t) => {
            if t < 2.0 {
                game.phase = Phase2::Settling(t + dt);
            } else {
                finish_roll(world, &mut game);
            }
        }
        Phase2::Over => {}
    }

    game.physics.advance(world, dt);
    sync_transforms(world);
    if std::env::var_os("LOBUT_LOG").is_some() {
        let pins = world.borrow::<Pin>();
        let balls = world.borrow::<Ball>();
        let tfs = world.borrow::<Transform>();
        let p: Vec<String> = pins.iter().take(3).filter_map(|(id, _)| tfs.get(id)).map(|t| format!("({:.2},{:.2},{:.2} up{:.2})", t.position.x, t.position.y, t.position.z, (t.rotation * Vec3::Y).y)).collect();
        let b: Vec<String> = balls.iter().filter_map(|(id, _)| tfs.get(id)).map(|t| format!("({:.2},{:.2},{:.2})", t.position.x, t.position.y, t.position.z)).collect();
        eprintln!("LOG {:?} pins {} ball {}", game.phase, p.join(" "), b.join(" "));
    }
    follow_ball(world, &game);
    draw_hud(world, &game);
    world.insert_resource(game);
}

/// Bir atış bitti: say, gerekirse ikinci atışa ya da sonraki çerçeveye geç.
fn finish_roll(world: &mut World, game: &mut Game) {
    let down = count_knocked(world);
    if game.roll == 1 && down < 10 {
        game.first_roll = down;
        game.roll = 2;
        // Devrilenleri kaldır; ayaktakiler yerinde kalır.
        let fallen: Vec<u32> = {
            let pins = world.borrow::<Pin>();
            let tfs = world.borrow::<Transform>();
            pins.iter().filter(|(id, _)| tfs.get(*id).is_some_and(knocked)).map(|(id, _)| id).collect()
        };
        for id in fallen {
            world.despawn_by_id(id);
        }
        world.despawn_all_with::<Ball>();
        game.phase = Phase2::Aiming;
        return;
    }
    let total = if game.roll == 1 { down } else { game.first_roll + down };
    if game.roll == 1 && down == 10 {
        play(world, "strike");
    }
    game.scores.push(total);
    if game.frame >= FRAMES {
        game.phase = Phase2::Over;
        world.despawn_all_with::<Ball>();
        return;
    }
    game.frame += 1;
    game.roll = 1;
    game.first_roll = 0;
    reset_pins(world, game);
    game.phase = Phase2::Aiming;
}

/// Kamera topun arkasından gelir, lobutlara yaklaşınca durur.
fn follow_ball(world: &mut World, game: &Game) {
    let ball = {
        let balls = world.borrow::<Ball>();
        let tfs = world.borrow::<Transform>();
        balls.iter().find_map(|(id, _)| tfs.get(id).map(|t| t.position))
    };
    let target_z = match (game.phase, ball) {
        (Phase2::Rolling(_), Some(p)) => (p.z + 3.2).max(PIN_Z + 4.5),
        (Phase2::Settling(_), _) => PIN_Z + 4.5,
        _ => BALL_Z + 3.2,
    };
    let cams: Vec<u32> = world.borrow::<Camera>().iter().map(|(id, _)| id).collect();
    let mut tfs = world.borrow_mut::<Transform>();
    for id in cams {
        if let Some(mut t) = tfs.get_mut(id) {
            t.position.z += (target_z - t.position.z) * 0.08;
            t.position.x = game.offset * 0.5;
        }
    }
}

/// Top bir lobuta, ya da lobut lobuta yeni çarptığında bir "tok".
fn clatter(events: EventReader<CollisionEvent>, pins: Query<With<Pin>>, audio: Option<ResMut<AudioManager>>) {
    let Some(mut audio) = audio else { return };
    let is_pin = |id: u32| pins.get(id).is_some();
    let hits = events
        .iter()
        .filter(|e| e.event_type == CollisionEventType::Started)
        .filter(|e| is_pin(e.entity_a.id()) || is_pin(e.entity_b.id()))
        .filter(|e| e.contact_points.iter().any(|p| p.normal_impulse > 0.4))
        .count();
    for _ in 0..hits.min(3) {
        let _ = audio.play("tok");
    }
}

fn play(world: &mut World, name: &str) {
    if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
        let _ = audio.play(name);
    }
}

fn draw_hud(world: &mut World, game: &Game) {
    let standing_down = count_knocked(world);
    let total: u32 = game.scores.iter().sum();
    let frames: String = game.scores.iter().map(|s| format!("{s:>2} ")).collect();
    let mut texts = world.borrow_mut::<Text>();
    if let Some(mut t) = texts.get_mut(game.hud) {
        t.content = format!(
            "Çerçeve {}/{}   Atış {}   Toplam {}\n{}\nGüç {:.0}%   Açı {:+.1}°",
            game.frame.min(FRAMES),
            FRAMES,
            game.roll,
            total,
            frames,
            game.power * 100.0,
            game.angle.to_degrees()
        );
    }
    if let Some(mut t) = texts.get_mut(game.banner) {
        t.content = match game.phase {
            Phase2::Over => format!("Oyun bitti — {total} lobut\nR ile yeniden"),
            Phase2::Settling(_) if game.roll == 1 && standing_down == 10 => "STRIKE!".into(),
            _ => String::new(),
        };
    }
}


fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    // A font the machine has (`GIZMO_FONT`, or the platform's usual faces); without one, the
    // synthetic test face, so the game still runs.
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
