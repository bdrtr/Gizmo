//! # Kapı — sandığı plakaya it, kapı açılsın
//!
//! Yukarıdan bakan üç oda. Her odanın kapısı yerdeki basınç plakalarına bağlı: plakaların hepsi
//! basılıyken kapı yere gömülür, biri boşalınca geri yükselir. Plakaya sen de basabilirsin ama
//! üstünden inince kapı kapanır; kalıcı olarak açık tutmak için sandıkları üstlerine itmen gerek.
//! Motorun yirminci oyunu ve karakterin bir şeyi **ittiği** ilki.
//!
//! | girdi | iş |
//! |---|---|
//! | **WASD · sol çubuk** | yürü, sandığı it |
//! | **R** | baştan |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Karakter sandığı itemiyordu.** `CharacterController` her çarpıştırıcıyı duvar sayıyordu:
//!   10 kg'lık sandığa yürüyen karakter önünde duruyordu. **Düzeltildi:**
//!   `CharacterController::push_force` (400 N) — çarptığı dinamik gövde yürüme yönünde, en fazla
//!   karakterin hızıyla itiliyor; ağır olan yerinden kıpırdamıyor.
//! - **Karakter tetiğe duvar gibi çarpıyor, tetik de karakteri hiç görmüyordu.** Plaka bir
//!   `with_trigger(true)` kutusu; karakter önünde durdu, `RigidBody`'si olmadığı için fizik
//!   dünyasının `trigger_events()`'i onu hiç anmadı. **Düzeltildi:** tetikler süpürmeye, yer
//!   ışınına ve platform taşımaya girmiyor; karakterin içinde olduğu tetikler
//!   `CharacterController::triggers`'ta.
//!
//! Gerisi ilk denemede çalıştı: kapılar `KinematicTarget` ile inip çıkıyor, plakadaki sandıkları
//! `PhysicsWorld::overlap_shape` buluyor, sandıklar `lock_rotation` ile dönmüyor.
//!
//! *Sonradan bulundu (2026-11-04):* plaka lambası basılınca yanmıyordu — renk `MaterialDesc` ile
//! değişiyordu ve motor her kare açıklamayı malzemeden yeniden yazıyordu. **Düzeltildi.**
//!
//! ## Ortam değişkenleri
//!   * `KAPI_OTO=1` — oyunu kendisi çözer (ekran görüntüsü için; `kapi --test` aynısını pencere
//!     açmadan yapar)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::core::input::Input;
use gizmo::physics::components::{CharacterController, KinematicTarget};
use gizmo::physics::world::{PhysicsWorld, QueryFilter};
use gizmo::physics::{BodyHandle, Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

/// `#` duvar, `.` zemin, `S` başlangıç, `E` çıkış, `x` sandık, `1`…`2` plaka,
/// `A`…`B` kapı (`A` plaka `1`'lere, `B` plaka `2`'lere bağlı).
const MAP: [&str; 7] = [
    "#################",
    "#.....#.....#...#",
    "#.S...#.x.x.#...#",
    "#..x..A.....B.E.#",
    "#.....#.....#...#",
    "#..1..#.2.2.#...#",
    "#################",
];
const DOOR_CLOSED_Y: f32 = 0.6;
const DOOR_OPEN_Y: f32 = -0.65;
const DOOR_SPEED: f32 = 2.5;

/// `KAPI_OTO`'nun yürüdüğü yol, hücre koordinatlarında (x, z). Her nokta ya varılınca ya da
/// oyuncu bir süre ilerleyemeyince geçilir — sandık duvara dayanınca itiş orada biter.
const AUTO_PATH: [(f32, f32); 16] = [
    (3.5, 1.5),
    (3.5, 4.75),
    (4.5, 4.5),
    (5.5, 3.5),
    (7.5, 3.5),
    (7.5, 1.5),
    (8.5, 1.5),
    (8.5, 4.75),
    (9.5, 4.5),
    (9.5, 1.5),
    (10.5, 1.5),
    (10.5, 4.75),
    (11.5, 4.0),
    (11.5, 3.5),
    (12.5, 3.5),
    (14.5, 3.5),
];

struct Door {
    id: u32,
    group: char,
    at: Vec3,
    y: f32,
}

struct Plate {
    group: char,
    at: Vec3,
    body: u32,
    lamp: u32,
}

struct Game {
    physics: FixedPhysics,
    player: u32,
    start: Vec3,
    exit: Vec3,
    crate_starts: Vec<Vec3>,
    doors: Vec<Door>,
    plates: Vec<Plate>,
    hud: u32,
    banner: u32,
    won: bool,
    auto: bool,
    auto_step: usize,
    auto_stuck: f32,
}

#[derive(Clone, Copy)]
struct Actor;
gizmo::core::impl_component!(Actor);

#[derive(Clone, Copy)]
struct Crate;
gizmo::core::impl_component!(Crate);

fn cell(x: usize, z: usize) -> Vec3 {
    Vec3::new(x as f32 + 0.5, 0.0, z as f32 + 0.5)
}

fn plate_collider() -> Collider {
    // Zeminin 5 cm üstünde: zemin kutusuyla örtüşmesin, üstündeki sandık örtüşsün.
    Collider::box_collider(Vec3::new(0.4, 0.25, 0.4)).with_trigger(true)
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
fn setup(world: &mut World, font: FontId) -> Game {

    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-1.0)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.95, 0.88), 1.7, LightRole::Sun),
    ));
    let (w, h) = (MAP[0].len() as f32, MAP.len() as f32);
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(w * 0.5, 13.0, h * 0.5 + 6.5),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -1.05,
        primary: true,
        ..Default::default()
    });
    world.spawn_bundle((
        Transform::new(Vec3::new(w * 0.5, -0.5, h * 0.5)),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(Vec3::new(w * 0.5, 0.5, h * 0.5)),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::new(w * 0.5, -0.05, h * 0.5)).with_scale(Vec3::new(w * 0.5, 0.05, h * 0.5)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Color::srgb_hex("#4A4E55").to_vec4(), 0.9, 0.0),
        MeshRenderer::new(),
    ));
    let (mut start, mut exit) = (Vec3::ZERO, Vec3::ZERO);
    let (mut crate_starts, mut doors, mut plates) = (Vec::new(), Vec::new(), Vec::new());
    for (z, row) in MAP.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            let at = cell(x, z);
            match c {
                '#' => {
                    world.spawn_bundle((
                        Transform::new(at + Vec3::Y * 0.6).with_scale(Vec3::new(0.5, 0.6, 0.5)),
                        GlobalTransform::default(),
                        MeshSource("standard_cube".into()),
                        MaterialDesc::new().with_pbr(Color::srgb_hex("#7A7268").to_vec4(), 0.85, 0.0),
                        MeshRenderer::new(),
                        RigidBody::new_static(),
                        Velocity::default(),
                        Collider::box_collider(Vec3::new(0.5, 0.6, 0.5)),
                    ));
                }
                'S' => start = at,
                'E' => exit = at,
                'x' => crate_starts.push(at),
                'A' | 'B' => {
                    let group = if c == 'A' { '1' } else { '2' };
                    doors.push(spawn_door(world, at, group));
                }
                '1' | '2' => {
                    let body = world
                        .spawn_bundle((
                            Transform::new(at + Vec3::Y * 0.3),
                            RigidBody::new_static(),
                            Velocity::default(),
                            plate_collider(),
                        ))
                        .id();
                    let lamp = world
                        .spawn_bundle((
                            Transform::new(at + Vec3::Y * 0.02).with_scale(Vec3::new(0.4, 0.02, 0.4)),
                            GlobalTransform::default(),
                            MeshSource("standard_cube".into()),
                            MaterialDesc::new().with_unlit(plate_colour(false)),
                            MeshRenderer::new(),
                        ))
                        .id();
                    plates.push(Plate { group: c, at, body, lamp });
                }
                _ => {}
            }
        }
    }
    world.spawn_bundle((
        Transform::new(exit + Vec3::Y * 0.02).with_scale(Vec3::new(0.45, 0.02, 0.45)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_unlit(Color::srgb_hex("#3CB371").to_vec4()),
        MeshRenderer::new(),
    ));
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),)).id();
    let banner = world
        .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
            .with_anchor(TextAnchor::Center)
            .with_align(TextAlign::Center),))
        .id();
    let mut game = Game {
        physics: FixedPhysics::new(60),
        player: 0,
        start,
        exit,
        crate_starts,
        doors,
        plates,
        hud,
        banner,
        won: false,
        auto: std::env::var_os("KAPI_OTO").is_some(),
        auto_step: 0,
        auto_stuck: 0.0,
    };
    restart(world, &mut game);
    game
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Kapı", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// `--test`: pencere açmadan otomatik pilot oyunu çözer — iki sandığı plakalara iter, iki kapıyı
/// açar, çıkışa yürür. Oyunun kendi `setup`'ı ve `update`'i, boş bir girdiyle, sabit adımla.
fn self_test() -> bool {
    let mut world = World::new();
    let font = gizmo::renderer::text::FontLibrary::new()
        .load(gizmo::renderer::text::synthetic::synthetic_face())
        .expect("sentetik yüz her zaman ayrıştırılır");
    let mut game = setup(&mut world, font);
    game.auto = true;
    world.insert_resource(game);
    let mut state = SimpleSceneState { camera_speed: 0.0, camera_pitch: 0.0, camera_yaw: 0.0, camera_pos: Vec3::ZERO };
    let input = Input::default();
    let dt = 1.0 / 60.0;
    let (mut time, mut opened) = (0.0f32, [false; 2]);
    while time < 60.0 {
        time += dt;
        update(&mut world, &mut state, dt, &input);
        // Pencereli döngü de her `update`'ten sonra ertelenmiş komutları uygular.
        world.apply_commands();
        let game = world.get_resource::<Game>().expect("oyun kaynağı");
        for (k, g) in ['1', '2'].into_iter().enumerate() {
            opened[k] |= game.doors.iter().filter(|d| d.group == g).all(|d| d.y < 0.0);
        }
        if game.won {
            break;
        }
    }
    let won = world.get_resource::<Game>().is_some_and(|g| g.won);
    let ok = won && opened.iter().all(|&o| o);
    println!(
        "kapi --test: {time:.1} s; kapı 1 {}, kapı 2 {}; çıkışa vardı mı: {} → {}",
        if opened[0] { "açıldı" } else { "AÇILMADI" },
        if opened[1] { "açıldı" } else { "AÇILMADI" },
        if won { "evet" } else { "HAYIR" },
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn spawn_door(world: &mut World, at: Vec3, group: char) -> Door {
    let position = at + Vec3::Y * DOOR_CLOSED_Y;
    let id = world
        .spawn_bundle((
            Transform::new(position).with_scale(Vec3::new(0.5, 0.6, 0.45)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(Color::srgb_hex("#B5651D").to_vec4(), 0.6, 0.2),
            MeshRenderer::new(),
            RigidBody::new_kinematic(),
            Velocity::default(),
            Collider::box_collider(Vec3::new(0.5, 0.6, 0.45)),
            KinematicTarget::new(position, Quat::IDENTITY),
        ))
        .id();
    Door { id, group, at, y: DOOR_CLOSED_Y }
}

fn plate_colour(pressed: bool) -> Vec4 {
    if pressed { Color::srgb_hex("#F1C40F").to_vec4() } else { Color::srgb_hex("#5D4E1F").to_vec4() }
}

fn restart(world: &mut World, game: &mut Game) {
    let actors: Vec<u32> = world.borrow::<Actor>().iter().map(|(id, _)| id).collect();
    for id in actors {
        if let Some(e) = world.get_entity(id) {
            world.despawn(e);
        }
    }
    game.player = world
        .spawn_bundle((
            Transform::new(game.start + Vec3::Y * 0.5).with_scale(Vec3::new(0.3, 0.5, 0.3)),
            GlobalTransform::default(),
            MeshSource("cylinder".into()),
            MaterialDesc::new().with_pbr(Color::srgb_hex("#3498DB").to_vec4(), 0.5, 0.0),
            MeshRenderer::new(),
            CharacterController { speed: 3.0, ..Default::default() },
            Velocity::default(),
            Collider::capsule(0.3, 0.2),
            Actor,
        ))
        .id();
    for &at in &game.crate_starts {
        world.spawn_bundle((
            Transform::new(at + Vec3::Y * 0.4).with_scale(Vec3::splat(0.4)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(Color::srgb_hex("#A0522D").to_vec4(), 0.8, 0.0),
            MeshRenderer::new(),
            RigidBody::new(10.0, true).lock_rotation(),
            Velocity::default(),
            Collider::box_collider(Vec3::splat(0.4)),
            Crate,
            Actor,
        ));
    }
    for door in &mut game.doors {
        door.y = DOOR_CLOSED_Y;
        let position = door.at + Vec3::Y * door.y;
        if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(door.id) {
            t.position = position;
        }
        if let Some(mut k) = world.borrow_mut::<KinematicTarget>().get_mut(door.id) {
            *k = KinematicTarget::new(position, Quat::IDENTITY);
        }
    }
    game.won = false;
    game.auto_step = 0;
    game.auto_stuck = 0.0;
    game.physics.reset();
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        restart(world, &mut game);
    }
    if !game.won {
        let wish = if game.auto { auto_walk(world, &mut game, dt) } else {
            let (mx, my) = input.move_axis();
            Vec3::new(mx, 0.0, -my)
        };
        if let Some(mut c) = world.borrow_mut::<CharacterController>().get_mut(game.player) {
            c.target_velocity = wish * c.speed;
        }
        game.physics.advance(world, dt);
        sync_transforms(world);
        plates_and_doors(world, &mut game, dt);
        let p = world.borrow::<Transform>().get(game.player).map(|t| t.position).unwrap_or(game.start);
        if Vec2::new(p.x - game.exit.x, p.z - game.exit.z).length() < 0.5 {
            game.won = true;
        }
    }
    hud(world, &game);
    world.insert_resource(game);
}

/// Hangi plakalar basılı (üstünde sandık ya da oyuncu var); her kapı, grubundaki plakaların
/// hepsi basılıysa iner, değilse çıkar.
fn plates_and_doors(world: &mut World, game: &mut Game, dt: f32) {
    let stood_on: Vec<BodyHandle> = world
        .borrow::<CharacterController>()
        .get(game.player)
        .map(|c| c.triggers.clone())
        .unwrap_or_default();
    let crates: Vec<u32> = world.borrow::<Crate>().iter().map(|(id, _)| id).collect();
    let mut pressed = Vec::with_capacity(game.plates.len());
    {
        let Some(physics) = world.get_resource::<PhysicsWorld>() else { return };
        let shape = plate_collider().shape;
        for plate in &game.plates {
            let by_player = stood_on.contains(&BodyHandle::from_id(plate.body));
            let by_crate = physics
                .overlap_shape(&shape, plate.at + Vec3::Y * 0.3, Quat::IDENTITY, QueryFilter::default())
                .iter()
                .any(|h| crates.contains(&h.id()));
            pressed.push(by_player || by_crate);
        }
    }
    for (plate, &down) in game.plates.iter().zip(&pressed) {
        if let Some(mut m) = world.borrow_mut::<MaterialDesc>().get_mut(plate.lamp) {
            let want = MaterialDesc::new().with_unlit(plate_colour(down));
            if *m != want {
                *m = want;
            }
        }
    }
    for door in &mut game.doors {
        let open = game.plates.iter().zip(&pressed).filter(|(p, _)| p.group == door.group).all(|(_, &d)| d);
        let goal = if open { DOOR_OPEN_Y } else { DOOR_CLOSED_Y };
        let step = DOOR_SPEED * dt;
        door.y += (goal - door.y).clamp(-step, step);
        if let Some(mut k) = world.borrow_mut::<KinematicTarget>().get_mut(door.id) {
            *k = KinematicTarget::new(door.at + Vec3::Y * door.y, Quat::IDENTITY);
        }
    }
}

/// `KAPI_OTO`: yoldaki bir sonraki noktaya yürür; varınca ya da yarım saniye ilerleyemeyince
/// bir sonrakine geçer.
fn auto_walk(world: &World, game: &mut Game, dt: f32) -> Vec3 {
    let Some(&(x, z)) = AUTO_PATH.get(game.auto_step) else { return Vec3::ZERO };
    let p = world.borrow::<Transform>().get(game.player).map(|t| t.position).unwrap_or(game.start);
    let to = Vec3::new(x - p.x, 0.0, z - p.z);
    let moving = world.borrow::<Velocity>().get(game.player).is_some_and(|v| Vec2::new(v.linear.x, v.linear.z).length() > 0.3);
    game.auto_stuck = if moving { 0.0 } else { game.auto_stuck + dt };
    if to.length() < 0.1 || game.auto_stuck > 0.5 {
        game.auto_step += 1;
        game.auto_stuck = 0.0;
    }
    (to / 0.4).clamp_length_max(1.0)
}

fn hud(world: &mut World, game: &Game) {
    let open = |g: char| game.doors.iter().filter(|d| d.group == g).all(|d| d.y < 0.0);
    let line = |g: char, name: &str| format!("kapı {name}: {}", if open(g) { "açık" } else { "kapalı" });
    set_text(world, game.hud, &format!("{}\n{}\nWASD: yürü, it   R: baştan", line('1', "1"), line('2', "2")));
    set_text(world, game.banner, if game.won { "Çıktın!\nR ile yeniden" } else { "" });
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
