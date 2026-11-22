//! # Rampa — yokuştan tepeye, tepeden inişe, üç kontrol noktası
//!
//! Bir araba düz bir yoldan bir yokuşa tırmanır, düz bir tepeyi geçer, inişten iner ve bitişe
//! varır. Yolun üstünde üç kontrol noktası vardır; sırayla geçilmeleri gerekir.
//!
//! Motorun hangi yollarını yürüttüğü:
//!
//! - **Yol `RigidBody`'siz çarpıştırıcılardan kurulu**, her parça bir **grup** (eğik bir
//!   `Transform`) ve zemini onun çocuğu. Tekerleklerin ışınları fizik dünyasının geniş evresinden
//!   geçer; yalnız çarpıştırıcı olan bir zemini o dünya tutmuyordu ve araba böyle bir yolun içinden
//!   düşüyordu (`c6ade21e`).
//! - **Kontrol noktaları da yalnız çarpıştırıcı**, tetik, bir grubun altında: grubun koyduğu yerde
//!   ve eğiminde. Geçişler `Events<TriggerEvent>`'ten, aracın gövdesinin girişi olarak okunur.
//!
//! `rampa --test`: pencere açmadan otomatik pilot bitişe sürer; üç kontrol noktası sırayla
//! geçilmeli, araba tepede gerçekten tepede olmalı (yokuşun içinden geçmemiş) ve hiç yolun altına
//! düşmemeli.
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::physics::vehicle::VehicleController;
use gizmo::physics::{Collider, CollisionEventType, RigidBody, TriggerEvent, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};

/// Yolun parçaları, baştan sona: (boy, eğim). Eğim yukarı doğru artı.
const SEGMENTS: [(f32, f32); 5] = [(40.0, 0.0), (16.0, 0.15), (16.0, 0.0), (16.0, -0.15), (60.0, 0.0)];
/// Kontrol noktası olan parçalar, sırayla: düz başlangıç, tepe, bitiş.
const CHECKPOINTS: [usize; 3] = [0, 2, 4];
const ROAD_HALF_WIDTH: f32 = 5.0;
const TOP_SPEED_KMH: f32 = 40.0;

struct Game {
    physics: FixedPhysics,
    car: Entity,
    camera: Entity,
    /// Kontrol noktalarının tetikleri, sırayla.
    gates: Vec<Entity>,
    /// Sıradaki kontrol noktası.
    next: usize,
    clock: f32,
    finished: Option<f32>,
    /// Arabanın tepedeki kontrol noktasını geçerken yüksekliği.
    height_at_top: Option<f32>,
    /// Otomatik pilot sürer (`--test`, ya da `RAMPA_OTO`); yoksa klavye.
    autopilot: bool,
    hud: Entity,
}

/// Parça `i`'nin başladığı yer ve yönü: her parça bir öncekinin bittiği yerden.
fn segment_start(i: usize) -> Vec3 {
    let mut at = Vec3::ZERO;
    for &(len, angle) in &SEGMENTS[..i] {
        at += direction(angle) * len;
    }
    at
}

/// `angle` eğimli bir parçanın yönü: araba −z'ye gider.
fn direction(angle: f32) -> Vec3 {
    Vec3::new(0.0, angle.sin(), -angle.cos())
}

/// Yol: her parça bir grup, zemini çocuğu; kontrol noktası olan parçada bir de tetik.
fn build_road(world: &mut World) -> Vec<Entity> {
    let mut gates = Vec::new();
    for (i, &(len, angle)) in SEGMENTS.iter().enumerate() {
        let centre = segment_start(i) + direction(angle) * (len / 2.0);
        let group = world.spawn_bundle((Transform::new(centre).with_rotation(Quat::from_rotation_x(angle)), GlobalTransform::default()));
        // Zemin biraz uzun: parçalar birbirinin içine girer, eklem yerinde basamak kalmaz.
        let half = Vec3::new(ROAD_HALF_WIDTH, 0.25, len / 2.0 + 0.4);
        let colour = if i % 2 == 0 { Vec4::new(0.32, 0.33, 0.36, 1.0) } else { Vec4::new(0.45, 0.4, 0.3, 1.0) };
        let floor = world.spawn_bundle((
            Transform::new(Vec3::new(0.0, -0.25, 0.0)).with_scale(half),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(colour, 0.85, 0.0),
            MeshRenderer::new(),
            Collider::box_collider(half),
        ));
        world.add_child(group, floor);
        if let Some(k) = CHECKPOINTS.iter().position(|&s| s == i) {
            let gate = world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 1.5, len / 2.0 - 4.0 - 2.0 * k as f32)),
                GlobalTransform::default(),
                Collider::box_collider(Vec3::new(ROAD_HALF_WIDTH, 1.5, 0.5)).with_trigger(true),
            ));
            world.add_child(group, gate);
            gates.push(gate);
        }
    }
    gates
}

fn spawn_car(world: &mut World, at: Vec3, visual: bool) -> Entity {
    let collider = Collider::box_collider(Vec3::new(0.9, 0.35, 2.0));
    let mut body = RigidBody::new(900.0, true);
    body.update_inertia_from_collider(&collider);
    let mut car = VehicleController::four_wheeled(1.7, 2.8, 0.38);
    for wheel in &mut car.wheels {
        wheel.suspension_rest_length = 0.45;
        wheel.suspension_max_travel = 0.35;
    }
    car.max_steering_angle = 0.5;
    let id = world.spawn_bundle((Transform::new(at), GlobalTransform::default(), body, Velocity::default(), collider, car));
    if visual {
        let part = world.spawn_bundle((
            Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.9, 0.35, 2.0)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(Color::srgb_hex("#C0392B").to_vec4(), 0.4, 0.3),
            MeshRenderer::new(),
        ));
        world.add_child(id, part);
    }
    id
}

/// Otomatik pilot: yolun ortasına, ileriye doğru; hız sınırının altında gaz.
fn autopilot(world: &World, car: Entity) -> (f32, f32) {
    let t = world.query::<&Transform>().get(car).copied().unwrap_or_default();
    let speed = world.query::<&VehicleController>().get(car).map_or(0.0, |c| c.current_speed_kmh);
    let forward = t.rotation * Vec3::NEG_Z;
    let f = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let to = Vec3::new(-t.position.x, 0.0, -12.0).normalize_or_zero();
    let angle = f.cross(to).y.atan2(f.dot(to));
    let throttle = if speed < TOP_SPEED_KMH { 0.7 } else { 0.0 };
    // `angle` solda artı; direksiyonun artısı sağa.
    (throttle, (-angle * 1.6).clamp(-1.0, 1.0))
}

fn setup(world: &mut World, font: FontId, visual: bool) -> Game {
    world.init_events::<TriggerEvent>();
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-1.0)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
    ));
    let camera = world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 5.0, 8.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.25,
        primary: true,
        ..Default::default()
    });
    let gates = build_road(world);
    let car = spawn_car(world, Vec3::new(0.0, 1.2, -5.0), visual);
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    Game {
        physics: FixedPhysics::new(120),
        car,
        camera,
        gates,
        next: 0,
        clock: 0.0,
        finished: None,
        height_at_top: None,
        autopilot: std::env::var_os("RAMPA_OTO").is_some(),
        hud,
    }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    let (mx, my) = input.move_axis();
    let (throttle, steer) = if game.autopilot { autopilot(world, game.car) } else { (my, mx) };
    if let Some(mut c) = world.query_mut::<&mut VehicleController>().get_mut(game.car) {
        c.drive(if game.finished.is_some() { 0.0 } else { throttle });
        c.steering_input = steer;
    }
    game.physics.advance(world, dt);
    sync_transforms(world);
    if game.finished.is_none() {
        game.clock += dt;
    }

    let car_at = world.query::<&Transform>().get(game.car).map(|t| t.position).unwrap_or(Vec3::ZERO);
    if let Some(mut events) = world.get_resource_mut::<gizmo::core::event::Events<TriggerEvent>>() {
        for e in events.drain() {
            let entered = e.event_type == CollisionEventType::Started && e.other_entity.id() == game.car.id();
            if entered && game.gates.get(game.next).is_some_and(|g| g.id() == e.trigger_entity.id()) {
                if CHECKPOINTS[game.next] == 2 {
                    game.height_at_top = Some(car_at.y);
                }
                game.next += 1;
                if game.next == game.gates.len() {
                    game.finished = Some(game.clock);
                }
            }
        }
    }

    // Takip kamerası.
    if let Some(mut c) = world.query_mut::<&mut Transform>().get_mut(game.camera) {
        let want = car_at + Vec3::new(0.0, 4.5, 10.0);
        c.position = c.position.lerp(want, (dt * 3.0).min(1.0));
        if let Some(aim) = Camera::looking_at(c.position, car_at) {
            c.rotation = aim;
        }
    }
    if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        text.content = match game.finished {
            Some(t) => format!("Bitti: {t:.1} s"),
            None => format!("Kontrol noktası {} / {}   {:.1} s", game.next, game.gates.len(), game.clock),
        };
    }
    world.insert_resource(game);
}

/// `--test`: otomatik pilot bitişe sürer.
fn self_test() -> bool {
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font(), false);
    game.autopilot = true;
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    let mut lowest = f32::MAX;
    while run.elapsed < 60.0 {
        run.frame(dt, update);
        let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
        let y = run.world.query::<&Transform>().get(game.car).map_or(f32::MIN, |t| t.position.y);
        lowest = lowest.min(y);
        if game.finished.is_some() {
            break;
        }
    }
    let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
    // Tepenin yüzeyi yokuşun boyu kadar yukarıda; arabanın merkezi onun üstünde olmalı.
    let top = SEGMENTS[1].0 * SEGMENTS[1].1.sin();
    let on_top = game.height_at_top.is_some_and(|y| y > top);
    let ok = game.finished.is_some() && on_top && lowest > -0.5;
    println!(
        "rampa --test: {} / {} kontrol noktası, {}; tepede yükseklik {} (yüzey {top:.2}), en alçak {lowest:.2} → {}",
        game.next,
        game.gates.len(),
        game.finished.map_or("bitmedi".to_string(), |t| format!("{t:.1} s")),
        game.height_at_top.map_or("—".to_string(), |y| format!("{y:.2}")),
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Rampa", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font, true);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
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
