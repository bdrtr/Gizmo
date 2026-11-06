//! # Arazi — tepelerde kontrol noktası yarışı
//!
//! Engebeli, üretilmiş bir arazi; beş bayraklı kontrol noktası. Arabayla sırayla hepsinden geç,
//! süre işliyor. Motorun otuzuncu oyunu ve **bir aracın arazide** sürüldüğü ilki: tekerlekler
//! zemini ışınla buluyor, zemin bir yükseklik alanı.
//!
//! | girdi | iş |
//! |---|---|
//! | **W / S · sağ-sol tetik** | gaz / fren-geri |
//! | **A / D · sol çubuk** | direksiyon |
//! | **R** | baştan |
//!
//! `arazi --test` pencere açmaz: otomatik pilot iki dakika sürer; her adımda arabanın zeminin
//! içine düşüp düşmediği ölçülür, kaç kontrol noktasından geçtiği yazılır. (`ARAZI_OTO=1`
//! pencereli oyunda da otomatik pilotu açar.)
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! Yok — ilk denemede çalıştı, ve bu da bir ölçüm: araç modeli (`VehicleController`, tekerlek
//! ışınları geniş faz üzerinden) ile `golf` için düzeltilen yükseklik alanı ilk kez birlikte
//! sürüldü; beş noktanın beşi, gövde merkezi zeminin 0,66–1,50 m üstünde. Kullanılanlar:
//! `terrain_triangles` + `terrain_collider`, `FixedPhysics`, ve bu oyunda ilk kez
//! `Renderer::default_font` ile `gizmo::simple::set_text`.

use gizmo::core::input::Input;
use gizmo::physics::components::PhysicsMaterial;
use gizmo::physics::vehicle::VehicleController;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{BodyHandle, Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{MeshData, Text, TextAlign, TextAnchor};
use gizmo::simple::{set_text, sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

const SAMPLES: u32 = 61;
const SIZE: f32 = 160.0;
const MAX_HEIGHT: f32 = 9.0;
const CHECKPOINTS: [(f32, f32); 5] = [(30.0, -40.0), (55.0, 20.0), (5.0, 55.0), (-50.0, 25.0), (-35.0, -45.0)];
const RADIUS: f32 = 7.0;

fn heights() -> Vec<f32> {
    let n = SAMPLES as usize;
    let mut h: Vec<f32> = (0..n * n)
        .map(|i| {
            let (col, row) = (i % n, i / n);
            let x = -SIZE / 2.0 + col as f32 / (n - 1) as f32 * SIZE;
            let z = -SIZE / 2.0 + row as f32 / (n - 1) as f32 * SIZE;
            (x * 0.045).sin() * 0.35 + (z * 0.06 + 1.0).cos() * 0.3 + ((x + z) * 0.025).sin() * 0.25
        })
        .collect();
    let (lo, hi) = h.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    for v in &mut h {
        *v = (*v - lo) / (hi - lo).max(1e-6);
    }
    h
}

fn ground_at(physics: &PhysicsWorld, x: f32, z: f32, skip: u32) -> Option<f32> {
    let ray = gizmo::physics::Ray::new(Vec3::new(x, 60.0, z), Vec3::NEG_Y);
    physics.raycast_excluding(&ray, 120.0, BodyHandle::from_id(skip)).map(|hit| 60.0 - hit.distance)
}

fn spawn_world(world: &mut World) {
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        RigidBody::new_static(),
        Velocity::default(),
        gizmo::systems::terrain_collider(heights(), SAMPLES, SAMPLES, SIZE, SIZE, MAX_HEIGHT)
            .with_material(PhysicsMaterial { static_friction: 0.9, dynamic_friction: 0.8, ..Default::default() }),
    ));
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
    car.max_steering_angle = 0.55;
    let id = world.spawn_bundle((Transform::new(at), body, Velocity::default(), collider, car));
    if visual {
        if let Some(e) = world.live(id) {
            world.add_component(e, GlobalTransform::default());
            for (offset, half, colour) in [
                (Vec3::ZERO, Vec3::new(0.9, 0.35, 2.0), Color::srgb_hex("#D35400").to_vec4()),
                (Vec3::new(0.0, 0.55, 0.3), Vec3::new(0.75, 0.25, 1.0), Vec4::new(0.15, 0.17, 0.22, 1.0)),
            ] {
                let part = world.spawn_bundle((
                    Transform::new(offset).with_scale(half),
                    GlobalTransform::default(),
                    MeshSource("standard_cube".into()),
                    MaterialDesc::new().with_pbr(colour, 0.5, 0.2),
                    MeshRenderer::new(),
                ));
                use gizmo::core::hierarchy::HierarchyExt;
                world.add_child(e, part);
            }
        }
    }
    id
}

fn start_position(world: &World) -> Vec3 {
    let y = world.get_resource::<PhysicsWorld>().and_then(|p| ground_at(&p, 0.0, 0.0, u32::MAX)).unwrap_or(MAX_HEIGHT);
    Vec3::new(0.0, y + 1.5, 0.0)
}

/// Otomatik pilot: sıradaki noktaya dön, hız sınırının altında gaz.
fn autopilot(world: &World, car: Entity, target: Vec3) -> (f32, f32) {
    let (t, speed) = {
        let ts = world.query::<&Transform>();
        let cs = world.query::<&VehicleController>();
        (ts.get(car).copied().unwrap_or_default(), cs.get(car).map_or(0.0, |c| c.current_speed_kmh))
    };
    let forward = t.rotation * Vec3::NEG_Z;
    let to = Vec3::new(target.x - t.position.x, 0.0, target.z - t.position.z).normalize_or_zero();
    let f = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let angle = f.cross(to).y.atan2(f.dot(to));
    let throttle = if speed < 45.0 { 0.8 } else { 0.0 };
    (throttle, (angle * 1.6).clamp(-1.0, 1.0))
}

fn drive(world: &mut World, car: Entity, throttle: f32, steer: f32) {
    if let Some(mut c) = world.query_mut::<&mut VehicleController>().get_mut(car) {
        c.throttle_input = throttle.max(0.0);
        c.brake_input = (-throttle).max(0.0);
        c.steering_input = steer;
    }
}

fn checkpoint(world: &World, k: usize) -> Vec3 {
    let (x, z) = CHECKPOINTS[k % CHECKPOINTS.len()];
    let y = world.get_resource::<PhysicsWorld>().and_then(|p| ground_at(&p, x, z, u32::MAX)).unwrap_or(0.0);
    Vec3::new(x, y, z)
}

/// `--test`: iki dakika otomatik pilot.
fn self_test() -> bool {
    let mut world = World::new();
    spawn_world(&mut world);
    gizmo::physics::physics_step_system(&world, 1.0 / 60.0);
    let start = start_position(&world);
    let car = spawn_car(&mut world, start, false);
    let mut physics = FixedPhysics::new(60);
    let (mut next, mut deepest, mut airborne_max) = (0usize, f32::MAX, 0.0f32);
    let dt = 1.0 / 60.0;
    for frame in 0..60 * 120 {
        let target = checkpoint(&world, next);
        let (throttle, steer) = if frame < 60 { (0.0, 0.0) } else { autopilot(&world, car, target) };
        drive(&mut world, car, throttle, steer);
        physics.advance(&mut world, dt);
        world.apply_commands();
        let p = world.query::<&Transform>().get(car).map(|t| t.position).unwrap_or(Vec3::ZERO);
        if let Some(g) = world.get_resource::<PhysicsWorld>().and_then(|pw| ground_at(&pw, p.x, p.z, car.id())) {
            deepest = deepest.min(p.y - g);
            airborne_max = airborne_max.max(p.y - g);
        }
        if Vec2::new(p.x - target.x, p.z - target.z).length() < RADIUS {
            next += 1;
            if next == CHECKPOINTS.len() {
                break;
            }
        }
        if !p.is_finite() {
            break;
        }
    }
    let ok = next >= 3 && deepest > -0.3;
    println!(
        "arazi --test: {} / {} kontrol noktası; gövde merkezi zeminin en az {:.2} m, en çok {:.2} m üstünde → {}",
        next,
        CHECKPOINTS.len(),
        deepest,
        airborne_max,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

struct Game {
    physics: FixedPhysics,
    car: Entity,
    next: usize,
    clock: f32,
    finished: Option<f32>,
    flags: Vec<Entity>,
    camera: Entity,
    hud: Entity,
    banner: Entity,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Arazi", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = scene.renderer.default_font();
            spawn_world(scene.world);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                MeshData::new(AssetManager::terrain_triangles(&heights(), SAMPLES, SAMPLES, SIZE, SIZE, MAX_HEIGHT)),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#7A8F3B").to_vec4(), 0.95, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.7) * Quat::from_rotation_x(-0.8)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.95, 0.85), 2.0, LightRole::Sun),
            ));
            gizmo::physics::physics_step_system(scene.world, 1.0 / 60.0);
            let flags: Vec<Entity> = (0..CHECKPOINTS.len())
                .map(|k| {
                    let at = checkpoint(scene.world, k);
                    scene
                        .world
                        .spawn_bundle((
                            Transform::new(at + Vec3::Y * 2.5).with_scale(Vec3::new(0.25, 2.5, 0.25)),
                            GlobalTransform::default(),
                            MeshSource("cylinder".into()),
                            MaterialDesc::new().with_unlit(Color::srgb_hex("#BDC3C7").to_vec4()),
                            MeshRenderer::new(),
                        ))
                })
                .collect();
            let start = start_position(scene.world);
            let car = spawn_car(scene.world, start, true);
            let camera = scene
                .world
                .spawn_bundle(CameraBundle { position: Vec3::new(0.0, 12.0, 18.0), pitch: -0.4, primary: true, ..Default::default() })
                ;
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                ;
            scene.world.insert_resource(Game { physics: FixedPhysics::new(60), car, next: 0, clock: 0.0, finished: None, flags, camera, hud, banner });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        if let Some(e) = world.live(game.car) {
            use gizmo::core::hierarchy::HierarchyExt;
            world.despawn_recursive(e);
        }
        let start = start_position(world);
        game.car = spawn_car(world, start, true);
        game.next = 0;
        game.clock = 0.0;
        game.finished = None;
        game.physics.reset();
    }
    let (mx, my) = input.move_axis();
    let auto = std::env::var_os("ARAZI_OTO").is_some();
    let (throttle, steer) = if auto { autopilot(world, game.car, checkpoint(world, game.next)) } else { (my, mx) };
    drive(world, game.car, if game.finished.is_some() { 0.0 } else { throttle }, steer);
    game.physics.advance(world, dt);
    sync_transforms(world);
    if game.finished.is_none() {
        game.clock += dt;
    }
    let t = world.query::<&Transform>().get(game.car).copied().unwrap_or_default();
    let target = checkpoint(world, game.next);
    if game.finished.is_none() && Vec2::new(t.position.x - target.x, t.position.z - target.z).length() < RADIUS {
        game.next += 1;
        if game.next == CHECKPOINTS.len() {
            game.finished = Some(game.clock);
        }
    }
    for (k, &flag) in game.flags.iter().enumerate() {
        if let Some(mut m) = world.query_mut::<&mut MaterialDesc>().get_mut(flag) {
            m.albedo = match k.cmp(&game.next) {
                std::cmp::Ordering::Less => Color::srgb_hex("#27AE60").to_vec4(),
                std::cmp::Ordering::Equal => Color::srgb_hex("#F1C40F").to_vec4(),
                std::cmp::Ordering::Greater => Color::srgb_hex("#BDC3C7").to_vec4(),
            };
        }
    }
    // Takip kamerası.
    if let Some(mut c) = world.query_mut::<&mut Transform>().get_mut(game.camera) {
        let back = t.rotation * Vec3::new(0.0, 0.0, 1.0);
        let want = t.position + Vec3::new(back.x, 0.0, back.z).normalize_or_zero() * 11.0 + Vec3::Y * 5.0;
        c.position = c.position.lerp(want, (dt * 3.0).min(1.0));
        let look = (t.position - c.position).normalize_or_zero();
        c.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, look);
    }
    let speed = world.query::<&VehicleController>().get(game.car).map_or(0.0, |c| c.current_speed_kmh);
    let dist = Vec2::new(t.position.x - target.x, t.position.z - target.z).length();
    set_text(
        world,
        game.hud,
        &format!(
            "kontrol {}/{}   sıradakine {:.0} m\n{:.0} km/s   süre {:.1} s\nW/S: gaz/fren   A/D: direksiyon   R: baştan",
            game.next.min(CHECKPOINTS.len()),
            CHECKPOINTS.len(),
            dist,
            speed,
            game.finished.unwrap_or(game.clock)
        ),
    );
    let banner = game.finished.map(|s| format!("Bitti: {s:.1} s\nR ile yeniden")).unwrap_or_default();
    set_text(world, game.banner, &banner);
    world.insert_resource(game);
}
