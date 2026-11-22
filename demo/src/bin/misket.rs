//! # Misket — zikzak bir pistten kayan tepsiye
//!
//! Misketler en üstten birer birer bırakılır, dört rampadan zikzak iner ve en altta yavaşça
//! sağa sola kayan bir tepsiye düşer. Tepsinin içindeki bölge her girişi sayar.
//!
//! Motorun hangi yollarını yürüttüğü, ve onun için yazıldığı:
//!
//! - **Pist `RigidBody`'siz çarpıştırıcılardan kurulu**, her rampa bir **grup** (dönmüş bir
//!   `Transform`) ve parçaları onun çocukları, yerel dönüşümlerle. Yalnız çarpıştırıcı olan bir
//!   şey statik geometridir; bir grubun altındaki, grubun koyduğu yerdedir.
//! - **Tepsi kinematik bir gövde**, duvarları ve sayaç bölgesi onun çocukları: duvarlar gövdenin
//!   bileşik şeklidir ve onunla birlikte kayar; bölge bir tetiktir ve tepsiyle taşınır.
//! - **Sayaç tetik olaylarından**, `Events<TriggerEvent>`'ten okunur; kare başına birkaç fizik
//!   adımı (240 Hz) atılır ve her adımın olayları sayılmalıdır.
//!
//! Bu oyun yazılırken bulunan: tepsinin içindeki tetik bölgesi, tepsinin katı şekline
//! karışıyordu — misket bölgenin üstünde duruyor, sayaç hiç artmıyordu (`189fe42d`).
//!
//! `misket --test`: pencere açmadan bütün misketleri bırakır; sayaç, tepsideki misketlerin
//! sayısına eşit olmalı ve hiçbir misket dünyadan düşmemeli.
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::physics::components::KinematicTarget;
use gizmo::physics::{Collider, CollisionEventType, RigidBody, TriggerEvent, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};

const PHYSICS_HZ: u32 = 240;
const MARBLES: usize = 12;
const MARBLE_R: f32 = 0.15;
/// İki misketin bırakılışı arası, saniye.
const RELEASE_EVERY: f32 = 0.7;
/// Bir rampanın yarı boyu (x), yarı genişliği (z) ve eğimi.
const RAMP_HALF: f32 = 2.0;
const RAMP_WIDTH: f32 = 0.4;
const RAMP_TILT: f32 = 0.16;
/// Rampaların yükseklikleri, yukarıdan aşağı; tepsi en alttakinin altında.
const RAMP_Y: [f32; 4] = [8.0, 6.8, 5.6, 4.4];
const TRAY_Y: f32 = 2.6;
const TRAY_HALF: f32 = 1.1;
/// Tepsinin kayışı: genliği (z) ve açısal hızı.
const TRAY_SWAY: f32 = 0.35;
const TRAY_RATE: f32 = 0.8;

#[derive(Clone, Copy)]
struct Marble;
gizmo::core::impl_component!(Marble);

struct Game {
    physics: FixedPhysics,
    tray: Entity,
    zone: Entity,
    released: usize,
    next_release: f32,
    clock: f32,
    /// Bölgeye girmiş misketler (aynı misketin bir girişi bir kez sayılır).
    counted: std::collections::HashSet<u32>,
    hud: Entity,
}

/// Rampa `i`'nin merkezi: zikzak, alçak ucu sırayla +x ve −x'te; bir alttaki rampa, yukarıdakinin
/// alçak ucunun altına uzanacak kadar kaymış.
fn ramp_centre(i: usize) -> Vec3 {
    let x = if i.is_multiple_of(2) { 0.0 } else { 0.5 };
    Vec3::new(x, RAMP_Y[i], 0.0)
}

/// Rampa `i` hangi yöne iner: +1 → +x, −1 → −x.
fn ramp_down(i: usize) -> f32 {
    if i.is_multiple_of(2) { 1.0 } else { -1.0 }
}

/// Bir parça: `parent`'ın çocuğu, yerel `at`'te, `half` yarı boyda bir kutu — görünüşü ve
/// çarpıştırıcısı aynı kutu.
fn piece(world: &mut World, parent: Entity, at: Vec3, half: Vec3, colour: Vec4, trigger: bool) -> Entity {
    let collider = Collider::box_collider(half).with_trigger(trigger);
    let e = if trigger {
        world.spawn_bundle((Transform::new(at), GlobalTransform::default(), collider))
    } else {
        world.spawn_bundle((
            Transform::new(at).with_scale(half),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(colour, 0.6, 0.0),
            MeshRenderer::new(),
            collider,
        ))
    };
    world.add_child(parent, e);
    e
}

/// Rampa `i`: dönmüş bir grup; altında zemin, iki korkuluk ve alçak uçta, misketin düşeceği
/// boşluğun ötesinde bir tampon.
fn build_ramp(world: &mut World, i: usize) {
    let down = ramp_down(i);
    let group = world.spawn_bundle((
        Transform::new(ramp_centre(i)).with_rotation(Quat::from_rotation_z(-down * RAMP_TILT)),
        GlobalTransform::default(),
    ));
    let wood = Vec4::new(0.62, 0.45, 0.28, 1.0);
    let rail = Vec4::new(0.35, 0.24, 0.15, 1.0);
    piece(world, group, Vec3::ZERO, Vec3::new(RAMP_HALF, 0.05, RAMP_WIDTH), wood, false);
    for side in [-1.0, 1.0] {
        piece(world, group, Vec3::new(0.0, 0.15, side * (RAMP_WIDTH + 0.05)), Vec3::new(RAMP_HALF, 0.15, 0.05), rail, false);
    }
    // Alçak ucun ötesinde, zeminle arasında misketin düşeceği bir boşluk bırakan tampon.
    piece(world, group, Vec3::new(down * (RAMP_HALF + 0.5), 0.1, 0.0), Vec3::new(0.05, 0.5, RAMP_WIDTH + 0.1), rail, false);
}

/// Tepsi: kinematik bir gövde (zemini), duvarları ve sayaç bölgesi çocukları.
fn build_tray(world: &mut World) -> (Entity, Entity) {
    let at = tray_at(0.0);
    let tray = world.spawn_bundle((
        Transform::new(at).with_scale(Vec3::new(TRAY_HALF, 0.05, TRAY_HALF)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.2, 0.45, 0.7, 1.0), 0.5, 0.0),
        MeshRenderer::new(),
        RigidBody::new_kinematic(),
        Velocity::default(),
        Collider::box_collider(Vec3::new(TRAY_HALF, 0.05, TRAY_HALF)),
        KinematicTarget::new(at, Quat::IDENTITY),
    ));
    let wall = Vec4::new(0.15, 0.3, 0.5, 1.0);
    // Tepsinin ölçeği çocuklarına geçer: yerel konumları da, meshleri de, çarpıştırıcıları da onunla
    // çarpılır. Duvarlar bu yüzden tepsi biriminde (±1) yazılır — meshi de çarpıştırıcısı da aynı
    // boyda, kendi ölçeği kadar.
    let s = Vec3::new(TRAY_HALF, 0.05, TRAY_HALF);
    for (offset, half) in [
        (Vec3::new(1.0, 0.25, 0.0), Vec3::new(0.05, 0.25, TRAY_HALF)),
        (Vec3::new(-1.0, 0.25, 0.0), Vec3::new(0.05, 0.25, TRAY_HALF)),
        (Vec3::new(0.0, 0.25, 1.0), Vec3::new(TRAY_HALF, 0.25, 0.05)),
        (Vec3::new(0.0, 0.25, -1.0), Vec3::new(TRAY_HALF, 0.25, 0.05)),
    ] {
        let local = Vec3::new(offset.x, offset.y / s.y, offset.z);
        let e = world.spawn_bundle((
            Transform::new(local).with_scale(half / s),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(wall, 0.5, 0.0),
            MeshRenderer::new(),
            Collider::box_collider(half / s),
        ));
        world.add_child(tray, e);
    }
    let zone = world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 0.3 / s.y, 0.0)),
        GlobalTransform::default(),
        Collider::box_collider(Vec3::new(TRAY_HALF - 0.1, 0.25, TRAY_HALF - 0.1) / s).with_trigger(true),
    ));
    world.add_child(tray, zone);
    (tray, zone)
}

/// Tepsinin `t` saniyedeki yeri: en alt rampanın alçak ucunun altında, z'de kayarak.
fn tray_at(t: f32) -> Vec3 {
    let last = RAMP_Y.len() - 1;
    let x = ramp_centre(last).x + ramp_down(last) * (RAMP_HALF + 0.2);
    Vec3::new(x, TRAY_Y, TRAY_SWAY * (t * TRAY_RATE).sin())
}

/// Misketin bırakıldığı yer: ilk rampanın yüksek ucunun üstü.
fn release_at(n: usize) -> Vec3 {
    let first = ramp_centre(0);
    Vec3::new(first.x - ramp_down(0) * (RAMP_HALF - 0.4), first.y + 0.6, 0.06 * (n as f32 % 3.0 - 1.0))
}

fn setup(world: &mut World, font: FontId) -> Game {
    world.init_events::<TriggerEvent>();
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.9)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 5.5, 12.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.12,
        primary: true,
        ..Default::default()
    });
    for i in 0..RAMP_Y.len() {
        build_ramp(world, i);
    }
    let (tray, zone) = build_tray(world);
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    Game {
        physics: FixedPhysics::new(PHYSICS_HZ),
        tray,
        zone,
        released: 0,
        next_release: 0.0,
        clock: 0.0,
        counted: std::collections::HashSet::new(),
        hud,
    }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, _input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.clock += dt;

    if game.released < MARBLES && game.clock >= game.next_release {
        world.spawn_bundle((
            Transform::new(release_at(game.released)).with_scale(Vec3::splat(MARBLE_R)),
            GlobalTransform::default(),
            MeshSource("sphere".into()),
            MaterialDesc::new().with_pbr(Vec4::new(0.85, 0.2 + 0.05 * game.released as f32, 0.25, 1.0), 0.2, 0.3),
            MeshRenderer::new(),
            RigidBody::new(0.05, true),
            Velocity::default(),
            Collider::sphere(MARBLE_R),
            Marble,
        ));
        game.released += 1;
        game.next_release = game.clock + RELEASE_EVERY;
    }

    if let Some(mut target) = world.query_mut::<&mut KinematicTarget>().get_mut(game.tray) {
        *target = KinematicTarget::new(tray_at(game.clock), Quat::IDENTITY);
    }
    game.physics.advance(world, dt);
    sync_transforms(world);

    // Her adımın girişleri: kuyruk karenin bütün adımlarınınkini tutuyor.
    if let Some(mut events) = world.get_resource_mut::<gizmo::core::event::Events<TriggerEvent>>() {
        for e in events.drain() {
            if e.event_type == CollisionEventType::Started && e.trigger_entity.id() == game.zone.id() {
                game.counted.insert(e.other_entity.id());
            }
        }
    }

    if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        text.content = format!("Misket: {} / {}   Tepside: {}", game.released, MARBLES, game.counted.len());
    }
    world.insert_resource(game);
}

/// Tepsinin içindeki misketler — bölgenin değil, yerlerinin söylediği: tepsinin üstünde, duvarlarının
/// arasında.
fn marbles_in_tray(world: &World, game: &Game) -> (usize, usize) {
    let tray = world.query::<&Transform>().get(game.tray).map(|t| t.position).unwrap_or(Vec3::ZERO);
    let marbles = world.query::<(&Transform, &Marble)>();
    let (mut inside, mut lost) = (0, 0);
    for (t, _) in marbles.iter() {
        let d = t.position - tray;
        if d.x.abs() < TRAY_HALF && d.z.abs() < TRAY_HALF && d.y > 0.0 && d.y < 0.6 {
            inside += 1;
        }
        if t.position.y < 0.0 {
            lost += 1;
        }
    }
    (inside, lost)
}

/// `--test`: bütün misketler bırakılır, tepsiye inmeleri beklenir. Sayaç tepsidekilere eşit, hepsi tepside,
/// hiçbiri dünyadan düşmemiş olmalı.
fn self_test() -> bool {
    let mut world = World::new();
    let game = setup(&mut world, HeadlessFrames::font());
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    // Bütün misketler tepsiye inene kadar, en çok bir dakika: bir misket tamponlarda yavaşlar,
    // pistin süresi misketten miskete değişir.
    while run.elapsed < 60.0 {
        run.frame(dt, update);
        let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
        if game.released == MARBLES && marbles_in_tray(&run.world, &game).0 == MARBLES {
            break;
        }
    }
    // Biraz daha: girişin olayı, girişin karesinden bir kare sonra okunur.
    for _ in 0..30 {
        run.frame(dt, update);
    }
    let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
    let (inside, lost) = marbles_in_tray(&run.world, &game);
    let counted = game.counted.len();
    let ok = counted == MARBLES && inside == MARBLES && lost == 0;
    println!(
        "misket --test: {MARBLES} misket bırakıldı; sayaç {counted}, tepside {inside}, düşen {lost}, {:.0} s → {}",
        run.elapsed,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Misket", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font);
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
