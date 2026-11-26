//! # Zar — bir tepsiye atılan beş zar
//!
//! Beş zar bir kenardan, dönerek, duvarlı bir tepsiye atılır; birbirine ve duvarlara çarpar,
//! yuvarlanır, bir yüzleri üstte durur ve uyurlar. Üstteki yüzlerin toplamı atışın puanıdır.
//! Boşluk tuşu yeniden atar; atılmazsa zarlar durduktan bir saniye sonra kendiliğinden atılır.
//!
//! Motorun hangi yollarını yürüttüğü, ve onun için yazıldığı:
//!
//! - **Kutunun yüzüne oturması**: yuvarlanan bir zar bir köşesinden, bir kenarından düşer ve bir
//!   yüzünün üstünde durmalıdır — yarı yolda, eğik uyumamalı. Bu oyun yazılırken bulunan: soft
//!   bias'ın yavaşça çıkardığı bir köşe derinliği uyku eşiğinin altında bir hızla çıkıyordu ve zar
//!   yolun ortasında uyuyup eğik kalıyordu (40 cm'lik bir kutu 1.2°, 1.6 cm'lik bir zar 3.3°);
//!   derinliğini çıkarmakta olan bir cisim artık uyumaz (`DIGGING`, `sleeping.rs`).
//! - **Adil yüzler**: zarın yüzleri eşit sıklıkta gelmeli. Temas noktalarını seçerken bir yöne,
//!   bir köşeye kayan bir manifold, ya da bir eksene göre farklı davranan bir eylemsizlik zarı
//!   hileli yapar; test yüzlerin χ²'sini ölçer.
//! - **Kutu–kutu ve kutu–duvar**: beş zar birbirine ve tepsinin duvarlarına çarpar.
//! - **Doğurulan ve silinen cisimler**: her atış eski zarları siler, yenilerini doğurur.
//!
//! `zar --test`: pencere açmadan altmış atış (300 zar). Her atışta bütün zarlar altı saniye içinde
//! uyumalı, tepsinin içinde ve — bir duvara ya da başka bir zara yaslanmamışsa — bir yüzünün üstünde
//! (0.06 rad'dan az eğik) durmalı; yaslanıp eğik kalan "kalkık" zarlar sayılmaz, yirmide birden azı
//! olabilir. Altı yüzün
//! sayılarının χ²'si 5 serbestlik derecesinde %1 sınırının (15.09) altında olmalı.
use gizmo::core::input::Input;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};

const PHYSICS_HZ: u32 = 240;
const DICE: usize = 5;
/// Zarın yarı kenarı, metre: 10 cm'lik bir zar.
const HALF: f32 = 0.05;
/// Tahta zar, ~700 kg/m³.
const DENSITY: f32 = 700.0;
/// Tepsinin iç yarı boyları (x, z) ve duvarlarının yüksekliği.
const TRAY: Vec2 = Vec2::new(1.0, 0.7);
const WALL_H: f32 = 0.25;
const WALL_T: f32 = 0.05;
/// Zarlar durduktan sonra yeniden atılana kadar geçen süre, ve bir atışın en uzun süresi.
const PAUSE: f32 = 1.0;
const MAX_ROLL: f32 = 6.0;
/// Yüzler: yerel eksenden zarın yüz değerine; karşılıklı yüzlerin toplamı yedidir.
const FACES: [(Vec3, u32); 6] = [
    (Vec3::Y, 1),
    (Vec3::NEG_Y, 6),
    (Vec3::X, 2),
    (Vec3::NEG_X, 5),
    (Vec3::Z, 3),
    (Vec3::NEG_Z, 4),
];

#[derive(Clone, Copy)]
struct Die;
gizmo::core::impl_component!(Die);

struct Game {
    physics: FixedPhysics,
    seed: u32,
    /// Bu atışın başladığı an, ve zarların hepsinin uyuduğu an.
    thrown_at: f32,
    settled_at: Option<f32>,
    clock: f32,
    dice: Vec<Entity>,
    /// Biten atışların her birinde: yüzler, uyumak için geçen süre (hiç uyumadıysa `None`).
    rounds: Vec<Round>,
    hud: Entity,
}

#[derive(Clone)]
struct Round {
    faces: Vec<u32>,
    /// Her zarın üstteki yüzünün düşeyden sapması, radyan.
    tilts: Vec<f32>,
    /// Her zar tepsinin içinde mi.
    inside: Vec<bool>,
    /// Her zar serbest mi: bir duvara ya da başka bir zara değebilecek kadar yakın değil. Bir duvara
    /// ya da zara yaslanmış bir zar eğik durabilir — masada "kalkık zar" yeniden atılır.
    free: Vec<bool>,
    settled_after: Option<f32>,
}

fn rnd(s: &mut u32) -> f32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    (*s >> 8) as f32 / (1u32 << 24) as f32
}

/// Üstteki yüz ve onun düşeyden sapması.
fn top_face(rotation: Quat) -> (u32, f32) {
    let (value, up) = FACES
        .iter()
        .map(|&(axis, value)| (value, (rotation * axis).dot(Vec3::Y)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .expect("altı yüz");
    (value, up.clamp(-1.0, 1.0).acos())
}

fn setup(world: &mut World, font: FontId) -> Game {
    world.insert_resource(gizmo::physics::world::PhysicsWorld::new());
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.9)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 1.9, 1.7),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.8,
        primary: true,
        ..Default::default()
    });
    let felt = MaterialDesc::new().with_pbr(Vec4::new(0.08, 0.35, 0.15, 1.0), 0.0, 0.9);
    let wood = MaterialDesc::new().with_pbr(Vec4::new(0.45, 0.28, 0.15, 1.0), 0.0, 0.6);
    let slab = |world: &mut World, at: Vec3, half: Vec3, look: &MaterialDesc| {
        world.spawn_bundle((
            Transform::new(at).with_scale(half),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            look.clone(),
            MeshRenderer::new(),
            RigidBody::new_static(),
            Velocity::default(),
            Collider::box_collider(half),
        ));
    };
    slab(world, Vec3::new(0.0, -0.05, 0.0), Vec3::new(TRAY.x + WALL_T, 0.05, TRAY.y + WALL_T), &felt);
    for sx in [-1.0f32, 1.0] {
        slab(world, Vec3::new(sx * (TRAY.x + WALL_T * 0.5), WALL_H * 0.5, 0.0), Vec3::new(WALL_T * 0.5, WALL_H * 0.5, TRAY.y + WALL_T), &wood);
    }
    for sz in [-1.0f32, 1.0] {
        slab(world, Vec3::new(0.0, WALL_H * 0.5, sz * (TRAY.y + WALL_T * 0.5)), Vec3::new(TRAY.x, WALL_H * 0.5, WALL_T * 0.5), &wood);
    }
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    let mut game = Game {
        physics: FixedPhysics::new(PHYSICS_HZ),
        seed: 0x2545_F491,
        thrown_at: 0.0,
        settled_at: None,
        clock: 0.0,
        dice: Vec::new(),
        rounds: Vec::new(),
        hud,
    };
    throw(world, &mut game);
    game
}

/// Eski zarları siler, beş yenisini tepsinin sol kenarının üstünden, dönerek, sağa doğru atar.
fn throw(world: &mut World, game: &mut Game) {
    for e in game.dice.drain(..) {
        world.despawn(e);
    }
    let collider = Collider::box_collider(Vec3::splat(HALF));
    let ivory = MaterialDesc::new().with_pbr(Vec4::new(0.92, 0.9, 0.82, 1.0), 0.0, 0.35);
    let s = &mut game.seed;
    for k in 0..DICE {
        let at = Vec3::new(-TRAY.x + 0.2 + 0.15 * rnd(s), 0.35 + 0.1 * k as f32, (k as f32 - 2.0) * 0.2 + 0.05 * (rnd(s) - 0.5));
        let rotation = Quat::from_euler(EulerRot::XYZ, 6.3 * rnd(s), 6.3 * rnd(s), 6.3 * rnd(s));
        let mut v = Velocity::new(Vec3::new(1.5 + 1.5 * rnd(s), 0.5 * rnd(s), 0.8 * (rnd(s) - 0.5)));
        v.angular = Vec3::new(rnd(s) - 0.5, rnd(s) - 0.5, rnd(s) - 0.5) * 40.0;
        let mut body = RigidBody::new(DENSITY * (2.0 * HALF).powi(3), true);
        body.update_inertia_from_collider(&collider);
        let die = world.spawn_bundle((
            Transform::new(at).with_rotation(rotation).with_scale(Vec3::splat(HALF)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            ivory.clone(),
            MeshRenderer::new(),
            body,
            v,
            collider.clone(),
            Die,
        ));
        game.dice.push(die);
    }
    game.thrown_at = game.clock;
    game.settled_at = None;
}

/// Atışın sonucu, zarların şimdiki hâlinden.
fn read_round(world: &World, game: &Game) -> Round {
    let mut round = Round { faces: Vec::new(), tilts: Vec::new(), inside: Vec::new(), free: Vec::new(), settled_after: game.settled_at.map(|t| t - game.thrown_at) };
    let centres: Vec<Vec3> = world.query::<(&Transform, &Die)>().iter().map(|(t, _)| t.position).collect();
    // Bir zarın merkezinden en uzak noktası `HALF·√3`; payıyla.
    let reach = HALF * 1.8;
    for (t, _) in world.query::<(&Transform, &Die)>().iter() {
        let (face, tilt) = top_face(t.rotation);
        let p = t.position;
        let off_walls = p.x.abs() < TRAY.x - reach && p.z.abs() < TRAY.y - reach;
        let apart = centres.iter().all(|&c| c == p || c.distance(p) > 2.0 * reach);
        round.free.push(off_walls && apart);
        round.faces.push(face);
        round.tilts.push(tilt);
        round.inside.push(t.position.x.abs() < TRAY.x && t.position.z.abs() < TRAY.y && t.position.y > 0.0 && t.position.y < WALL_H + HALF);
    }
    round
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.physics.advance(world, dt);
    sync_transforms(world);
    game.clock += dt;
    let asleep = world.query::<(&RigidBody, &Die)>().iter().filter(|(rb, _)| rb.is_sleeping).count();
    if game.settled_at.is_none() && asleep == DICE {
        game.settled_at = Some(game.clock);
    }
    let done = match game.settled_at {
        Some(t) => game.clock - t >= PAUSE,
        None => game.clock - game.thrown_at >= MAX_ROLL,
    };
    let asked = input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32);
    let shown = read_round(world, &game);
    if done || asked {
        game.rounds.push(shown.clone());
        throw(world, &mut game);
    }
    if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        let sum: u32 = shown.faces.iter().sum();
        text.content = if game.settled_at.is_some() {
            format!("Atış {}: {:?} = {sum}   (boşluk: yeniden at)", game.rounds.len() + 1, shown.faces)
        } else {
            format!("Atış {}: yuvarlanıyor…", game.rounds.len() + 1)
        };
    }
    world.insert_resource(game);
}

/// `--test`: altmış atış. Her atışta zarlar altı saniye içinde uyumalı, tepsinin içinde ve bir
/// yüzlerinin üstünde durmalı; yüzlerin dağılımı adil olmalı.
fn self_test() -> bool {
    const ROUNDS: usize = 60;
    let mut world = World::new();
    let game = setup(&mut world, HeadlessFrames::font());
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    while run.world.get_resource::<Game>().expect("oyun kaynağı").rounds.len() < ROUNDS && run.elapsed < ROUNDS as f32 * (MAX_ROLL + PAUSE) {
        run.frame(dt, update);
    }
    let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
    let mut ok = game.rounds.len() >= ROUNDS;
    let mut counts = [0u32; 6];
    let (mut worst_tilt, mut slowest, mut cocked) = (0.0f32, 0.0f32, 0);
    for (i, round) in game.rounds.iter().take(ROUNDS).enumerate() {
        let Some(after) = round.settled_after else {
            println!("  atış {i}: zarlar {MAX_ROLL} s içinde uyumadı");
            ok = false;
            continue;
        };
        slowest = slowest.max(after);
        if round.faces.len() != DICE || round.inside.iter().any(|&inside| !inside) {
            println!("  atış {i}: bir zar tepsiden çıktı ({:?})", round.inside);
            ok = false;
        }
        for ((&face, &tilt), &free) in round.faces.iter().zip(&round.tilts).zip(&round.free) {
            // Yaslanmış ve eğik: kalkık zar, sayılmaz (masada yeniden atılır).
            if !free && tilt > 0.06 {
                cocked += 1;
                continue;
            }
            counts[face as usize - 1] += 1;
            worst_tilt = worst_tilt.max(tilt);
            if tilt > 0.06 {
                println!("  atış {i}: bir zar {tilt:.3} rad eğik durdu");
                ok = false;
            }
        }
    }
    let n: u32 = counts.iter().sum();
    let expected = n as f32 / 6.0;
    let chi: f32 = counts.iter().map(|&c| (c as f32 - expected).powi(2) / expected).sum();
    ok &= chi < 15.09 && cocked * 20 < n + cocked;
    println!(
        "zar --test: {} atış, {n} zar; yüzler 1–6 {counts:?}, χ² {chi:.1} (sınır 15.09); en eğik {worst_tilt:.4} rad, en geç {slowest:.2} s'de uyudu, {cocked} kalkık → {}",
        game.rounds.len().min(ROUNDS),
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Zar", 1280, 720)
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
