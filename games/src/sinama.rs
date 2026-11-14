//! # Sınama — motoru zorlayan sahneler
//!
//! Dört ağır sahne, sağ üstteki düğmelerle (ya da 1–4) seçiliyor; ekranın sol üstü ne kadar
//! dayandığını söylüyor: kare hızı, kare süresi, en kötü kare, fizik adımının süresi, cisim sayısı.
//!
//! | sahne | ne zorluyor |
//! |---|---|
//! | **Kule** | 6×6×8 = 288 kutu üst üste: katı cisim çözücüsü, geniş/dar faz, uyuyan gövdeler |
//! | **Kumaş** | iki trambolin kumaş ve üstüne düşen toplar: kumaş ile katı cisimlerin iki yönlü etkileşimi |
//! | **Jöle** | on iki yumuşak blok üst üste: sonlu eleman yumuşak cisimleri, alt adımlar |
//! | **Işık** | 48 dönen renkli nokta ışığı, 120 küp, dört parçacık kaynağı: ışık kümeleme, parçacıklar |
//!
//! **Dokun / tıkla:** imlecin gösterdiği yere bir top fırlat (Jöle'de yeni bir blok düşür).
//! **R** ya da *Sıfırla*: sahneyi baştan kur. Fizik 60 Hz sabit adımla (`FixedPhysics`), yani
//! ekran daha hızlıysa adımlar arasında çiziliyor (`systems::interpolation`).
//!
//! `sinama --test` pencere açmaz: her sahneyi üç saniye oynatır; hiçbir cisim zeminden düşmemeli,
//! hiçbir konum sayı olmaktan çıkmamalı (NaN), ve her sahnenin fizik adımı süresi yazılır.

use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::physics::cloth::Cloth;
use gizmo::physics::soft_body::SoftBodyMesh;
use gizmo::physics::world::PhysicsWorld;
use gizmo::prelude::*;
use gizmo::renderer::components::{ParticleEmitter, Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};
use gizmo::ui::{BackgroundColor, ButtonBundle, Clicked, FlexDirection, Interaction, NodeBundle, PositionType, Style, UiPlugin, UiRect, Val};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scene {
    Tower,
    Cloth,
    Jelly,
    Lights,
}

impl Scene {
    const ALL: [Scene; 4] = [Scene::Tower, Scene::Cloth, Scene::Jelly, Scene::Lights];

    fn name(self) -> &'static str {
        match self {
            Scene::Tower => "Kule",
            Scene::Cloth => "Kumaş",
            Scene::Jelly => "Jöle",
            Scene::Lights => "Işık",
        }
    }
}

/// Bir sahneye ait olan, sahne değişince silinen her şey.
#[derive(Clone, Copy)]
struct SceneThing;
gizmo::core::impl_component!(SceneThing);

/// Işık sahnesinde dönen bir ışık: hangi yörüngede, hangi açıda.
#[derive(Clone, Copy)]
struct Orbit {
    radius: f32,
    height: f32,
    speed: f32,
    phase: f32,
}
gizmo::core::impl_component!(Orbit);

/// Sağ üstteki bir düğmenin işi: bir sahne ya da sıfırlama.
#[derive(Clone, Copy)]
struct ButtonAction(Option<Scene>);
gizmo::core::impl_component!(ButtonAction);

struct Bench {
    scene: Scene,
    physics: FixedPhysics,
    clock: f32,
    hud: Entity,
    /// Son yarım saniyenin kareleri: süreleri ve fizik adımlarının süreleri.
    window: Vec<(f32, f32)>,
    shown: String,
}

const FLOOR_HALF: f32 = 12.0;

/// Bütün sahnelerin ortak dekoru: güneş, zemin (ve onun çarpıştırıcısı), kamera.
fn setup(world: &mut World, font: FontId) -> Bench {
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.8)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.92), 1.8, LightRole::Sun),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.5, 0.0)).with_scale(Vec3::new(FLOOR_HALF, 0.5, FLOOR_HALF)),
        GlobalTransform::default(),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(Vec3::new(FLOOR_HALF, 0.5, FLOOR_HALF)),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.32, 0.33, 0.36, 1.0), 0.85, 0.0),
        MeshRenderer::new(),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 7.0, 15.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.38,
        primary: true,
        ..Default::default()
    });
    let hud = world.spawn_bundle((Text::pinned("", font, 24.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    spawn_buttons(world, font);
    let mut bench = Bench { scene: Scene::Tower, physics: FixedPhysics::new(60), clock: 0.0, hud, window: Vec::new(), shown: String::new() };
    load(world, &mut bench, Scene::Tower);
    bench
}

fn spawn_buttons(world: &mut World, font: FontId) {
    let column = world.spawn_bundle(NodeBundle {
        style: Style {
            position_type: PositionType::Absolute,
            inset: UiRect { left: Val::Auto, right: Val::Px(16.0), top: Val::Px(16.0), bottom: Val::Auto },
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            width: Val::Px(180.0),
            ..Default::default()
        },
        background_color: BackgroundColor(Vec4::ZERO),
        ..Default::default()
    });
    let entries = Scene::ALL.iter().map(|s| (Some(*s), s.name())).chain([(None, "Sıfırla")]);
    for (action, label) in entries {
        let button = world.spawn_bundle(ButtonBundle {
            style: Style { height: Val::Px(52.0), ..Default::default() },
            background_color: BackgroundColor(fill(Interaction::None)),
            ..Default::default()
        });
        world.add_component(button, ButtonAction(action));
        world.add_component(button, Text::screen(label, font, 24.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
        world.add_child(column, button);
    }
}

fn fill(i: Interaction) -> Vec4 {
    match i {
        Interaction::Pressed => Vec4::new(0.3, 0.55, 0.85, 0.95),
        Interaction::Hovered => Vec4::new(0.28, 0.3, 0.36, 0.9),
        _ => Vec4::new(0.15, 0.16, 0.2, 0.85),
    }
}

/// Bir sahneyi baştan kurar: öncekinin her şeyini silip yenisini dizer.
fn load(world: &mut World, bench: &mut Bench, scene: Scene) {
    let old: Vec<Entity> = world.query::<(Entity, &SceneThing)>().iter().map(|(e, _)| e).collect();
    for e in old {
        world.despawn_recursive(e);
    }
    bench.scene = scene;
    bench.physics = FixedPhysics::new(60);
    bench.clock = 0.0;
    bench.window.clear();
    match scene {
        Scene::Tower => {
            for x in 0..6 {
                for z in 0..6 {
                    for y in 0..8 {
                        let at = Vec3::new((x as f32 - 2.5) * 0.52, 0.25 + y as f32 * 0.5, (z as f32 - 2.5) * 0.52);
                        let t = y as f32 / 7.0;
                        spawn_box(world, at, Vec3::splat(0.25), Vec4::new(0.85 - 0.5 * t, 0.45 + 0.2 * t, 0.25 + 0.6 * t, 1.0), 1.0);
                    }
                }
            }
        }
        Scene::Cloth => {
            for (i, x0) in [-4.6f32, 1.0].into_iter().enumerate() {
                let n = 24;
                let spacing = 0.15;
                let mut cloth = Cloth::new(n, n, spacing, 0.05);
                // Yatay bir trambolin: düzlemin (x, y)'si dünyanın (x, z)'si, dört köşesi tutulu.
                for node in &mut cloth.nodes {
                    let p = Vec3::new(x0 + node.position.x, 2.5, 1.7 - node.position.y);
                    node.position = p;
                    node.prev_position = p;
                }
                for corner in [0, n - 1, n * (n - 1), n * n - 1] {
                    cloth.pin_node(corner);
                }
                let colour = if i == 0 { Vec4::new(0.25, 0.45, 0.8, 1.0) } else { Vec4::new(0.8, 0.3, 0.35, 1.0) };
                world.spawn_bundle((Transform::new(Vec3::ZERO), cloth, MaterialDesc::new().with_pbr(colour, 0.85, 0.0), SceneThing));
            }
            for k in 0..10 {
                let x = if k % 2 == 0 { -2.9 } else { 2.7 };
                spawn_ball(world, Vec3::new(x + (k as f32 * 0.37).sin() * 0.6, 5.0 + k as f32 * 0.8, (k as f32 * 0.71).cos() * 0.6), Vec3::ZERO);
            }
        }
        Scene::Jelly => {
            for k in 0..12 {
                drop_jelly(world, k);
            }
        }
        Scene::Lights => {
            for k in 0..120 {
                let (x, z) = ((k % 12) as f32 - 5.5, (k / 12) as f32 - 4.5);
                let h = 0.2 + ((k * 7) % 5) as f32 * 0.18;
                world.spawn_bundle((
                    Transform::new(Vec3::new(x * 1.6, h, z * 1.6)).with_scale(Vec3::new(0.35, h, 0.35)),
                    GlobalTransform::default(),
                    MeshSource("standard_cube".into()),
                    MaterialDesc::new().with_pbr(Vec4::new(0.8, 0.8, 0.82, 1.0), 0.4, 0.0),
                    MeshRenderer::new(),
                    SceneThing,
                ));
            }
            for k in 0..48 {
                let hue = k as f32 / 48.0 * std::f32::consts::TAU;
                let colour = Vec3::new(0.5 + 0.5 * hue.cos(), 0.5 + 0.5 * (hue + 2.1).cos(), 0.5 + 0.5 * (hue + 4.2).cos());
                let orbit = Orbit { radius: 2.0 + (k % 6) as f32 * 1.6, height: 1.0 + (k % 3) as f32 * 0.8, speed: 0.4 + (k % 5) as f32 * 0.15, phase: hue };
                let light = world.spawn_bundle(PointLightBundle {
                    position: Vec3::ZERO,
                    color: colour,
                    intensity: 6.0,
                    radius: 4.0,
                    casts_shadows: false,
                });
                world.add_component(light, orbit);
                world.add_component(light, SceneThing);
            }
            for k in 0..4 {
                let a = k as f32 * std::f32::consts::FRAC_PI_2 + 0.4;
                let mut sparks = ParticleEmitter::new();
                sparks.spawn_rate = 120.0;
                sparks.initial_velocity = Vec3::new(0.0, 4.0, 0.0);
                sparks.velocity_randomness = 1.5;
                sparks.lifespan = 1.6;
                sparks.size_start = 0.12;
                sparks.size_end = 0.0;
                sparks.color_start = Vec4::new(1.0, 0.7, 0.3, 1.0);
                sparks.color_end = Vec4::new(1.0, 0.2, 0.1, 0.0);
                world.spawn_bundle((Transform::new(Vec3::new(a.cos() * 6.0, 0.1, a.sin() * 6.0)), GlobalTransform::default(), sparks, SceneThing));
            }
        }
    }
}

fn spawn_box(world: &mut World, at: Vec3, half: Vec3, colour: Vec4, mass: f32) {
    world.spawn_bundle((
        Transform::new(at).with_scale(half),
        GlobalTransform::default(),
        RigidBody::new(mass, true),
        Velocity::default(),
        Collider::box_collider(half),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(colour, 0.5, 0.0),
        MeshRenderer::new(),
        SceneThing,
    ));
}

fn spawn_ball(world: &mut World, at: Vec3, velocity: Vec3) {
    world.spawn_bundle((
        Transform::new(at).with_scale(Vec3::splat(0.35)),
        GlobalTransform::default(),
        RigidBody::new(6.0, true),
        Velocity { linear: velocity, ..Default::default() },
        Collider::sphere(0.35),
        MeshSource("sphere".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.9, 0.85, 0.3, 1.0), 0.3, 0.6),
        MeshRenderer::new(),
        SceneThing,
    ));
}

fn drop_jelly(world: &mut World, k: usize) {
    let size = Vec3::new(1.2, 0.7, 1.2);
    let at = Vec3::new(((k % 3) as f32 - 1.0) * 1.5 - size.x * 0.5, 0.4 + (k / 3) as f32 * 1.0 + (k % 3) as f32 * 0.3, -size.z * 0.5);
    let Ok(mut body) = SoftBodyMesh::block(at, size, [2, 2, 2], 8.0, 15_000.0, 0.3) else { return };
    body.damping = 0.4;
    let hue = k as f32 * 0.9;
    let colour = Vec4::new(0.5 + 0.45 * hue.cos(), 0.5 + 0.45 * (hue + 2.1).cos(), 0.5 + 0.45 * (hue + 4.2).cos(), 1.0);
    world.spawn_bundle((body, MaterialDesc::new().with_pbr(colour, 0.25, 0.0), SceneThing));
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut bench) = world.remove_resource::<Bench>() else { return };
    // Düğmeler ve tuşlar.
    let mut chosen: Option<Option<Scene>> = None;
    {
        let mut buttons = world.query_mut::<(&ButtonAction, &Interaction, &Clicked, &mut BackgroundColor)>();
        for (action, interaction, clicked, mut bg) in buttons.iter_mut() {
            bg.0 = fill(*interaction);
            if clicked.0 {
                chosen = Some(action.0);
            }
        }
    }
    for (key, scene) in [(KeyCode::Digit1, Scene::Tower), (KeyCode::Digit2, Scene::Cloth), (KeyCode::Digit3, Scene::Jelly), (KeyCode::Digit4, Scene::Lights)] {
        if input.is_key_just_pressed(key as u32) {
            chosen = Some(Some(scene));
        }
    }
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        chosen = Some(None);
    }
    if let Some(choice) = chosen {
        let scene = choice.unwrap_or(bench.scene);
        load(world, &mut bench, scene);
    } else if input.is_mouse_button_just_pressed(0) && !gizmo::ui::pointer_over_ui(world) {
        throw(world, &bench);
    }

    // Işıklar dönüyor.
    bench.clock += dt;
    {
        let clock = bench.clock;
        let mut lights = world.query_mut::<(&Orbit, &mut Transform)>();
        for (o, mut t) in lights.iter_mut() {
            let a = o.phase + clock * o.speed;
            t.position = Vec3::new(a.cos() * o.radius, o.height + (a * 2.0).sin() * 0.4, a.sin() * o.radius);
        }
    }

    let started = std::time::Instant::now();
    bench.physics.advance(world, dt.min(0.1));
    world.apply_commands();
    sync_transforms(world);
    let physics_ms = started.elapsed().as_secs_f32() * 1000.0;

    bench.window.push((dt, physics_ms));
    let span: f32 = bench.window.iter().map(|(d, _)| d).sum();
    if span >= 0.5 {
        let frames = bench.window.len() as f32;
        let worst = bench.window.iter().map(|(d, _)| *d).fold(0.0, f32::max) * 1000.0;
        let physics = bench.window.iter().map(|(_, p)| *p).sum::<f32>() / frames;
        let (bodies, lights) = counts(world);
        bench.shown = format!(
            "Sınama: {}   {:.0} FPS   kare {:.1} ms (en kötü {:.1})   fizik {:.1} ms\ncisim {bodies}   ışık {lights}\n1–4 / düğmeler: sahne · tıkla: top fırlat · R: sıfırla",
            bench.scene.name(),
            frames / span,
            span / frames * 1000.0,
            worst,
            physics,
        );
        bench.window.clear();
    }
    gizmo::simple::set_text(world, bench.hud, &bench.shown);
    world.insert_resource(bench);
}

/// İmlecin gösterdiği yere, kameradan bir top; jöle sahnesinde yeni bir blok.
fn throw(world: &mut World, bench: &Bench) {
    if bench.scene == Scene::Jelly {
        let n = world.query::<&SoftBodyMesh>().iter().count();
        drop_jelly(world, n);
        return;
    }
    let Some(ray) = gizmo::pointer::cursor_ray(world) else { return };
    let origin = Vec3::from(ray.origin);
    let dir = Vec3::from(ray.direction).normalize_or_zero();
    spawn_ball(world, origin + dir * 1.5, dir * 22.0);
}

/// Sahnedeki gövdeler (katı, kumaş düğümü, yumuşak cisim düğümü) ve ışıklar.
fn counts(world: &World) -> (usize, usize) {
    let rigid = world.query::<(&RigidBody, &SceneThing)>().iter().count();
    let cloth: usize = world.query::<&Cloth>().iter().map(|c| c.nodes.len()).sum();
    let soft: usize = world.query::<&SoftBodyMesh>().iter().map(|b| b.nodes.len()).sum();
    let lights = world.query::<&PointLight>().iter().count();
    (rigid + cloth + soft, lights)
}

/// Oyunun tamamı bir `App` olarak.
pub fn app() -> App<SimpleSceneState> {
    App::<SimpleSceneState>::new("Gizmo Engine - Sınama", 1280, 800)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, _state| {
            let font = scene.renderer.default_font();
            let bench = setup(scene.world, font);
            scene.world.insert_resource(bench);
        })
        .replace_update(update)
}

/// `--test`: her sahne üç saniye; hiçbir cisim zeminden düşmemeli ya da sayı olmaktan çıkmamalı.
pub fn self_test() -> bool {
    let mut ok = true;
    for scene in Scene::ALL {
        let mut world = World::new();
        let mut bench = setup(&mut world, HeadlessFrames::font());
        load(&mut world, &mut bench, scene);
        world.insert_resource(bench);
        let mut run = HeadlessFrames::new(world);
        let started = std::time::Instant::now();
        while run.elapsed < 3.0 {
            run.frame(1.0 / 60.0, update);
        }
        let wall = started.elapsed().as_secs_f32();
        let w = &run.world;
        let mut bad = 0usize;
        let mut lowest = f32::MAX;
        for (t, _) in w.query::<(&Transform, &RigidBody)>().iter() {
            lowest = lowest.min(t.position.y);
            bad += usize::from(!t.position.is_finite() || t.position.y < -2.0);
        }
        for c in w.query::<&Cloth>().iter() {
            bad += c.nodes.iter().filter(|n| !n.position.is_finite() || n.position.y < -2.0).count();
        }
        for b in w.query::<&SoftBodyMesh>().iter() {
            bad += b.nodes.iter().filter(|n| !n.position.is_finite() || n.position.y < -2.0).count();
        }
        let (bodies, lights) = counts(w);
        let pass = bad == 0;
        println!(
            "sinama --test: {:<6} cisim {bodies:>4}, ışık {lights:>2}; 3 s'lik oyun {:.2} s sürdü (kare başına {:.1} ms); düşen ya da NaN {bad} → {}",
            scene.name(),
            wall,
            wall / 180.0 * 1000.0,
            if pass { "tamam" } else { "BAŞARISIZ" }
        );
        ok &= pass;
    }
    ok
}
