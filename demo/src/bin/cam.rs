//! # Cam — bir atış galerisi: cam levhalar ve kasalar
//!
//! Galeride altı cam levha durur, arkalarında üç kasa. Top fırlat, hepsini kır. Levhalar gövde
//! değil, yalnız çarpıştırıcı (statik geometri); kasalar gövde. Oyun motorun `PlayLoop`'u ile
//! oynar — editörün ▶'si ve dışa aktarılan her oyun bu döngüyle oynar — ve skoru motorun
//! söylediğinden sayar: her kırılma bir `BreakEvent`.
//!
//! Bu oyunu yazarken bulunan ve kapatılan, hepsi "oyunda hiçbir şey kırılmıyor" ailesinden:
//!
//! - **`PlayLoop` yalın katı-cisim adımını çalıştırıyordu**: kırılma, patlama, ip ve kumaş cephenin
//!   adımındaydı. Editörün ▶'sinde ve dışa aktarılan oyunda bir top camı kırmıyor, bir patlama
//!   hiçbir şey yapmıyor, bir ip havada donuk asılı kalıyordu. Patlama sistemini cephenin adımı da
//!   hiç çalıştırmıyordu.
//! - **Gövdesi olmayan bir `Breakable` kırılamıyordu**: kırılma sorgusu `Velocity` istiyordu; bir
//!   cam, bir duvar — statik geometri — her darbeye bağışıktı.
//! - **Parçalar küreydi**: hücresinin %42'si kadar bir küre. Çizilen hücre zemine gömülüyor, küre
//!   yuvarlanma sürtünmesi olmadığından sonsuza dek yuvarlanıyordu: bir kasanın parçaları sekiz
//!   saniye sonra hâlâ 1.7 m/s ile yuvarlanıyordu. Artık her parça kendi hücresinin dışbükey kabuğu.
//! - **Neyin kırıldığını hiçbir şey söylemiyordu** (gövde aynı adımda yok ediliyordu): artık
//!   `BreakEvent`. `Breakable::break_sound`'u hiçbir şey çalmıyordu, `piece_prefab`'ı ve
//!   `Explosion::damage_radius`'u hiçbir şey okumuyordu: ses artık çalınıyor, alan okunuyor,
//!   prefab silindi.
//!
//! `cam --test`: otomatik pilot her hedefe nişan alıp fırlatır. Her hedef kırılmalı ve her
//! kırılma bir kez söylenmeli; parçalar beş saniye içinde durulmalı, çizilen hücreleri zeminin
//! üstünde kalmalı, hiçbiri zeminden düşmemeli ve hepsi çizilmeli; ömürleri dolunca hepsi gitmeli.
use gizmo::core::component::EntityName;
use gizmo::core::input::Input;
use gizmo::physics::components::{BreakEvent, Breakable, Debris};
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{MeshRenderer, Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::systems::PlayLoop;

const PANES: usize = 6;
const CRATES: usize = 3;
const THROWER: Vec3 = Vec3::new(0.0, 1.5, -9.0);
const THROW_EVERY: f32 = 0.7;
const BALL_SPEED: f32 = 14.0;
const DEBRIS_LIFETIME: f32 = 8.0;
const GRAVITY: f32 = 9.81;

#[derive(Clone, Copy)]
struct Target;
gizmo::core::impl_component!(Target);

#[derive(Clone, Copy)]
struct Ball;
gizmo::core::impl_component!(Ball);

struct Game {
    play: PlayLoop,
    broken: usize,
    /// Kırılmalar, gövdenin kimliğiyle — her biri bir kez söylenmeli.
    told: Vec<u32>,
    next_throw: f32,
    clock: f32,
    hud: Entity,
}

fn material(r: f32, g: f32, b: f32, a: f32) -> MaterialDesc {
    MaterialDesc::new().with_pbr(Vec4::new(r, g, b, a), 0.2, 0.0)
}

fn setup(world: &mut World, font: FontId) {
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.8)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.92), 2.5, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: THROWER + Vec3::new(0.0, 1.5, -3.0),
        yaw: std::f32::consts::FRAC_PI_2,
        pitch: -0.15,
        primary: true,
        ..Default::default()
    });
    world.spawn_bundle((
        EntityName::new("zemin"),
        Transform::new(Vec3::new(0.0, -0.5, 0.0)).with_scale(Vec3::new(14.0, 0.5, 14.0)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        material(0.4, 0.38, 0.35, 1.0),
        MeshRenderer::new(),
        Collider::box_collider(Vec3::new(14.0, 0.5, 14.0)),
    ));
    // Galerinin duvarları, zeminin kenarlarında: ıskalayan toplar parçaları arkaya sürer, ve
    // duvarsız bir zeminden biri kenardan düşer (2026-11-23, top temasları doğrulanınca yollar
    // değişti ve bir parça z = 14.3'ten düştü). Ön duvar kameranın arkasında.
    for (at, half) in [
        (Vec3::new(0.0, 1.5, 13.75), Vec3::new(14.0, 1.5, 0.25)),
        (Vec3::new(0.0, 1.5, -13.75), Vec3::new(14.0, 1.5, 0.25)),
        (Vec3::new(13.75, 1.5, 0.0), Vec3::new(0.25, 1.5, 14.0)),
        (Vec3::new(-13.75, 1.5, 0.0), Vec3::new(0.25, 1.5, 14.0)),
    ] {
        world.spawn_bundle((
            EntityName::new("duvar"),
            Transform::new(at).with_scale(half),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            material(0.32, 0.3, 0.28, 1.0),
            MeshRenderer::new(),
            Collider::box_collider(half),
        ));
    }
    // Cam levhalar: yalnız çarpıştırıcı, gövde yok.
    for i in 0..PANES {
        let x = -6.25 + 2.5 * i as f32;
        let half = Vec3::new(1.0, 0.9, 0.04);
        world.spawn_bundle((
            EntityName::new("cam"),
            Transform::new(Vec3::new(x, 1.2, 0.0)).with_scale(half),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            material(0.6, 0.85, 0.95, 0.5),
            MeshRenderer::new(),
            Collider::box_collider(half),
            Breakable { max_pieces: 8, threshold: 2.0, current_health: 2.0, max_health: 2.0, debris_lifetime: DEBRIS_LIFETIME, break_sound: Some("cam".into()), ..Default::default() },
            Target,
        ));
    }
    // Kasalar: gövde, levhaların arkasında.
    for i in 0..CRATES {
        let x = -4.0 + 4.0 * i as f32;
        let half = Vec3::splat(0.5);
        world.spawn_bundle((
            EntityName::new("kasa"),
            Transform::new(Vec3::new(x, 0.5, 4.0)).with_scale(half),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            material(0.6, 0.42, 0.22, 1.0),
            MeshRenderer::new(),
            Collider::box_collider(half),
            RigidBody::new(20.0, true),
            Velocity::default(),
            Breakable { max_pieces: 8, threshold: 5.0, current_health: 5.0, max_health: 5.0, debris_lifetime: DEBRIS_LIFETIME, break_sound: Some("kasa".into()), ..Default::default() },
            Target,
        ));
    }
    world.insert_resource(gizmo::physics::world::PhysicsWorld::new().with_gravity(Vec3::new(0.0, -GRAVITY, 0.0)));
    world.init_events::<BreakEvent>();
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    world.insert_resource(Game { play: PlayLoop::new(), broken: 0, told: Vec::new(), next_throw: 0.5, clock: 0.0, hud });
}

/// `at`'a balistik bir atış: `BALL_SPEED` ile yatay uzaklığı geçecek kadar sürede varan hız.
fn aim(at: Vec3) -> Vec3 {
    let d = at - THROWER;
    let t = Vec3::new(d.x, 0.0, d.z).length() / BALL_SPEED;
    Vec3::new(d.x / t, d.y / t + 0.5 * GRAVITY * t, d.z / t)
}

fn throw(world: &mut World, velocity: Vec3) {
    world.spawn_bundle((
        EntityName::new("top"),
        Transform::new(THROWER).with_scale(Vec3::splat(0.15)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        material(0.9, 0.2, 0.15, 1.0),
        MeshRenderer::new(),
        RigidBody::new(1.0, true),
        Velocity::new(velocity),
        Collider::sphere(0.15),
        Ball,
    ));
}

/// Otomatik pilotun sıradaki hedefi: kırılmamış ilk hedef.
fn next_target(world: &World) -> Option<Vec3> {
    let mut targets: Vec<(u32, Vec3)> = world.query::<(Entity, &Transform, &Target)>().iter().map(|(e, t, _)| (e.id(), t.position)).collect();
    targets.sort_by_key(|(id, _)| *id);
    targets.first().map(|(_, p)| *p)
}

fn frame(world: &mut World, dt: f32, input: &Input, auto: bool) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.clock += dt;
    if game.clock >= game.next_throw {
        let shot = if auto { next_target(world).map(aim) } else if input.is_mouse_button_just_pressed(gizmo::core::input::mouse::LEFT) { Some(Vec3::new(0.0, 2.0, BALL_SPEED)) } else { None };
        if let Some(v) = shot {
            throw(world, v);
            game.next_throw = game.clock + THROW_EVERY;
        }
    }
    game.play.step(world, dt, input, &mut |_| {});
    world.apply_commands();
    if let Some(mut events) = world.get_resource_mut::<gizmo::core::event::Events<BreakEvent>>() {
        for e in events.drain() {
            game.told.push(e.body.id());
            game.broken += 1;
        }
    }
    let left = world.query::<&Target>().iter().count();
    if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        text.content = format!("Kırılan: {}   Kalan: {left}", game.broken);
    }
    world.insert_resource(game);
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let auto = std::env::var_os("CAM_OTO").is_some();
    frame(world, dt, input, auto);
}

struct Pieces {
    count: usize,
    drawn: usize,
    fastest: f32,
    deepest: f32,
}

fn pieces(world: &World) -> Pieces {
    let drawn = world.query::<&MeshRenderer>();
    let all: Vec<(f32, f32, bool)> = world
        .query::<(Entity, &Transform, &Velocity, &Debris)>()
        .iter()
        .map(|(e, t, v, d)| {
            let lowest = d.vertices.iter().map(|p| (t.position + t.rotation * *p).y).fold(f32::MAX, f32::min);
            (v.linear.length(), lowest, drawn.get(e).is_some())
        })
        .collect();
    Pieces {
        count: all.len(),
        drawn: all.iter().filter(|p| p.2).count(),
        fastest: all.iter().map(|p| p.0).fold(0.0, f32::max),
        deepest: all.iter().map(|p| p.1).fold(f32::MAX, f32::min),
    }
}

/// `--test`: bkz. modül belgesi.
fn self_test() -> bool {
    let mut world = World::new();
    setup(&mut world, gizmo::simple::HeadlessFrames::font());
    let dt = 1.0 / 60.0;
    let input = Input::default();
    let mut t = 0.0;
    // Çerçeve döngüsünün yaptığı gibi: bir karenin olayları ertesi karede okunur.
    let run = |world: &mut World, t: &mut f32| {
        world.update_events();
        frame(world, dt, &input, true);
        *t += dt;
    };
    while t < 60.0 && world.query::<&Target>().iter().count() > 0 {
        run(&mut world, &mut t);
    }
    let all_broken_at = t;
    let targets_left = world.query::<&Target>().iter().count();
    // Topları kaldır, ki parçaları itmesinler; sonra beş saniye.
    let balls: Vec<Entity> = world.query::<(Entity, &Ball)>().iter().map(|(e, _)| e).collect();
    for b in balls {
        world.despawn(b);
    }
    while t < all_broken_at + 5.0 {
        run(&mut world, &mut t);
    }
    let settled = pieces(&world);
    while t < all_broken_at + DEBRIS_LIFETIME + 1.0 {
        run(&mut world, &mut t);
    }
    let after_lifetime = pieces(&world).count;
    let game = world.get_resource::<Game>().expect("oyun");
    let mut told = game.told.clone();
    told.sort_unstable();
    let once = told.windows(2).all(|w| w[0] != w[1]);
    let targets = PANES + CRATES;
    let ok = targets_left == 0
        && game.broken == targets
        && once
        && settled.count > targets
        && settled.drawn == settled.count
        && settled.fastest < 0.1
        && settled.deepest > -0.03
        && after_lifetime == 0;
    println!(
        "cam --test: {} hedeften {} kırıldı ({all_broken_at:.1} s), {} kırılma söylendi{}; beş saniye sonra {} parça ({} çizili), en hızlısı {:.3} m/s, en derin çizili nokta {:.3} m; ömür dolunca {after_lifetime} parça → {}",
        targets,
        targets - targets_left,
        game.broken,
        if once { "" } else { " (YİNELENEN)" },
        settled.count,
        settled.drawn,
        settled.fastest,
        settled.deepest,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Cam", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            setup(scene.world, font);
            // Kırılma sesleri: sentetik, kısa birer gürültü patlaması.
            if let Ok(mut audio) = gizmo::audio::AudioManager::new() {
                let burst = |pitch: f32| -> Vec<f32> {
                    let mut seed = 7u32;
                    (0..11_025)
                        .map(|i| {
                            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            let noise = (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
                            let fade = (-(i as f32) / 2_000.0).exp();
                            0.5 * fade * (noise * 0.6 + 0.4 * (i as f32 * pitch).sin())
                        })
                        .collect()
                };
                audio.load_samples("cam", &burst(0.9), 44_100);
                audio.load_samples("kasa", &burst(0.15), 44_100);
                scene.world.insert_resource(audio);
            }
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
