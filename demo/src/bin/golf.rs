//! # Golf — tepeli bir sahada üç delik
//!
//! Üretilmiş, inişli çıkışlı bir çim saha; üç delik, her birinin bir başlangıcı ve bayraklı bir
//! çukuru var. Fareyle yönü seç, sol tuşu basılı tutup gücü doldur, bırak. Top durunca bir sonraki
//! vuruş. Motorun yirmi dördüncü oyunu ve **arazi** üstünde geçen ilki: saha diskten bir yükseklik
//! haritası okumadan, bellekte üretiliyor — `AssetManager::terrain_triangles` çiziyor,
//! `gizmo::systems::terrain_collider` çarpıştırıyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **fare** | nişan |
//! | **sol tuş (basılı tut, bırak)** | vuruş gücü, vuruş |
//! | **R** | baştan |
//!
//! `golf --test` pencere açmaz: bir yapay oyuncu üç deliği oynar, her adımda topun arazinin
//! içine düşüp düşmediğine bakar ve kaç vuruşta bitirdiğini yazar (`GOLF_DEBUG=1` her vuruşu).
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Arazi düz bir kutu olarak çarpışıyordu.** `Terrain`'in çarpıştırıcısı, en yüksek tepenin
//!   yarısında duran bir kutuydu: top çukurlarda havada, tepelerde toprağın içinde yuvarlanırdı.
//!   **Düzeltildi:** aynı örneklerden bir yükseklik alanı, ve mesh onun köşegeniyle bölünüyor
//!   (öbür köşegeni kullanıyordu; doğru çarpıştırıcıyla bile eğimli bir hücrede çizilen ve
//!   çarpışılan üçgenler farklıydı).
//! - **Üretilmiş bir arazi için diske bir resim yazmak gerekiyordu.** `Terrain` yalnız bir dosya
//!   yolu alıyor; mesh'i kuran kod GPU cihazı isteyen bir fonksiyonun içindeydi. **Düzeltildi:**
//!   `AssetManager::terrain_triangles` GPU'suz üçgen listesini veriyor, `terrain_collider` aynı
//!   örneklerden çarpıştırıcıyı.

use gizmo::core::input::Input;
use gizmo::physics::components::PhysicsMaterial;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{MeshData, Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

const SAMPLES: u32 = 49;
const SIZE: f32 = 40.0;
const MAX_HEIGHT: f32 = 3.0;
const BALL_R: f32 = 0.2;
const BALL_MASS: f32 = 0.5;
const MAX_SPEED: f32 = 13.0;
const CUP_R: f32 = 0.45;
const DT: f32 = 1.0 / 60.0;
/// Her delik: başlangıç ve çukur, sahanın düzleminde (x, z).
const HOLES: [((f32, f32), (f32, f32)); 3] = [((-14.0, 12.0), (-4.0, 2.0)), ((-2.0, -2.0), (12.0, -12.0)), ((13.0, -9.0), (8.0, 13.0))];

/// Saha: birkaç yumuşak tepe ve çukur, `0..=1`.
fn heights() -> Vec<f32> {
    let bumps: [(f32, f32, f32, f32); 6] = [
        (-8.0, 8.0, 7.0, 0.9),
        (6.0, -4.0, 6.0, 1.0),
        (10.0, 8.0, 5.0, -0.6),
        (-10.0, -10.0, 8.0, 0.7),
        (2.0, 12.0, 4.0, 0.5),
        (-2.0, 3.0, 5.0, -0.4),
    ];
    let n = SAMPLES as usize;
    let mut h: Vec<f32> = (0..n * n)
        .map(|i| {
            let (col, row) = (i % n, i / n);
            let x = -SIZE / 2.0 + col as f32 / (n - 1) as f32 * SIZE;
            let z = -SIZE / 2.0 + row as f32 / (n - 1) as f32 * SIZE;
            0.5 + bumps.iter().map(|&(bx, bz, r, a)| a * (-((x - bx).powi(2) + (z - bz).powi(2)) / (r * r)).exp()).sum::<f32>() * 0.4
        })
        .collect();
    let (lo, hi) = h.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    for v in &mut h {
        *v = (*v - lo) / (hi - lo).max(1e-6);
    }
    h
}

fn terrain_collider() -> Collider {
    gizmo::systems::terrain_collider(heights(), SAMPLES, SAMPLES, SIZE, SIZE, MAX_HEIGHT)
        .with_material(PhysicsMaterial { rolling_friction: 0.08, restitution: 0.1, ..Default::default() })
}

/// Sahanın `(x, z)`'deki yüksekliği — aşağı bir ışınla, çarpıştırıcının kendisinden; `ball` (varsa)
/// ışının yolundan çıkarılır, yoksa topun tepesi zemin sanılır.
fn ground_at(physics: &PhysicsWorld, x: f32, z: f32, ball: Option<u32>) -> Option<f32> {
    let ray = gizmo::physics::Ray::new(Vec3::new(x, 50.0, z), Vec3::NEG_Y);
    match ball {
        Some(b) => physics.raycast_excluding(&ray, 100.0, gizmo::physics::BodyHandle::from_id(b)),
        None => physics.raycast(&ray, 100.0),
    }
    .map(|hit| 50.0 - hit.distance)
}

fn spawn_course(world: &mut World) -> u32 {
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world
        .spawn_bundle((Transform::new(Vec3::ZERO), RigidBody::new_static(), Velocity::default(), terrain_collider()))
        .id()
}

fn spawn_ball(world: &mut World, at: Vec3) -> u32 {
    let col = Collider::sphere(BALL_R).with_material(PhysicsMaterial { rolling_friction: 0.08, restitution: 0.1, ..Default::default() });
    let mut rb = RigidBody::new(BALL_MASS, true);
    rb.linear_damping = 0.35;
    rb.angular_damping = 0.6;
    rb.ccd_enabled = true;
    rb.update_inertia_from_collider(&col);
    world.spawn_bundle((Transform::new(at).with_scale(Vec3::splat(BALL_R)), rb, Velocity::default(), col)).id()
}

fn start_position(world: &World, hole: usize) -> Vec3 {
    let (x, z) = HOLES[hole].0;
    let y = world.get_resource::<PhysicsWorld>().and_then(|p| ground_at(&p, x, z, None)).unwrap_or(MAX_HEIGHT);
    Vec3::new(x, y + BALL_R + 0.05, z)
}

fn cup(hole: usize) -> Vec3 {
    let (x, z) = HOLES[hole].1;
    Vec3::new(x, 0.0, z)
}

/// Topu `dir` yönüne, `power` (`0..=1`) gücüyle vur.
fn hit(world: &mut World, ball: u32, dir: Vec3, power: f32) {
    let flat = Vec3::new(dir.x, 0.0, dir.z).normalize_or_zero();
    if let Some(mut v) = world.borrow_mut::<Velocity>().get_mut(ball) {
        v.linear = flat * power.clamp(0.0, 1.0) * MAX_SPEED;
        v.angular = Vec3::ZERO;
    }
    if let Some(mut rb) = world.borrow_mut::<RigidBody>().get_mut(ball) {
        rb.wake_up();
    }
}

fn ball_state(world: &World, ball: u32) -> (Vec3, f32) {
    let p = world.borrow::<Transform>().get(ball).map(|t| t.position).unwrap_or(Vec3::ZERO);
    let v = world.borrow::<Velocity>().get(ball).map(|v| v.linear.length()).unwrap_or(0.0);
    (p, v)
}

/// Çukurda mı: yeterince yakın ve yavaş.
fn in_cup(pos: Vec3, speed: f32, hole: usize) -> bool {
    let c = cup(hole);
    Vec2::new(pos.x - c.x, pos.z - c.z).length() < CUP_R && speed < 2.5
}

fn out_of_bounds(pos: Vec3) -> bool {
    pos.y < -3.0 || pos.x.abs() > SIZE / 2.0 + 1.0 || pos.z.abs() > SIZE / 2.0 + 1.0
}

/// Yapay oyuncu: çukura doğru, uzaklığın karekökü kadar güçle.
fn auto_shot(pos: Vec3, hole: usize) -> (Vec3, f32) {
    let to = cup(hole) - pos;
    let d = Vec2::new(to.x, to.z).length();
    (to, (0.17 * d.sqrt() + 0.05).min(1.0))
}

/// `--test`: pencere yok; yapay oyuncu üç deliği oynar.
fn self_test() -> bool {
    let mut world = World::new();
    spawn_course(&mut world);
    // İlk adım fizik dünyasını kurar; yükseklik ışını ondan sonra cevap verir.
    gizmo::physics::physics_step_system(&world, DT);
    let mut ok = true;
    let mut total = 0;
    for hole in 0..HOLES.len() {
        let at = start_position(&world, hole);
        let ball = spawn_ball(&mut world, at);
        let (mut strokes, mut sunk, mut deepest) = (0, false, f32::MAX);
        let mut last_rest = start_position(&world, hole);
        // Uyarlanır güç: sahadan çıkınca azalır, yaklaşamayınca (yokuştan geri yuvarlanınca) artar.
        let mut boost = 1.0f32;
        while strokes < 15 && !sunk {
            let (pos, _) = ball_state(&world, ball);
            let before = Vec2::new(cup(hole).x - pos.x, cup(hole).z - pos.z).length();
            let (dir, power) = auto_shot(pos, hole);
            let power = (power * boost).min(1.0);
            if std::env::var_os("GOLF_DEBUG").is_some() {
                let c = cup(hole);
                println!("  vuruş {}: top ({:.2}, {:.2}, {:.2}), çukura {:.2} m, güç {power:.2}", strokes + 1, pos.x, pos.y, pos.z, Vec2::new(c.x - pos.x, c.z - pos.z).length());
            }
            hit(&mut world, ball, dir, power);
            strokes += 1;
            let mut still = 0;
            for _ in 0..60 * 20 {
                gizmo::physics::physics_step_system(&world, DT);
                world.apply_commands();
                let (p, v) = ball_state(&world, ball);
                if let Some(g) = world.get_resource::<PhysicsWorld>().and_then(|pw| ground_at(&pw, p.x, p.z, Some(ball))) {
                    deepest = deepest.min(p.y - BALL_R - g);
                }
                if in_cup(p, v, hole) {
                    sunk = true;
                    break;
                }
                if out_of_bounds(p) {
                    // Sahadan çıktı: son durduğu yere, bir ceza vuruşuyla.
                    strokes += 1;
                    boost *= 0.75;
                    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(ball) {
                        t.position = last_rest;
                    }
                    hit(&mut world, ball, Vec3::X, 0.0);
                    break;
                }
                still = if v < 0.05 { still + 1 } else { 0 };
                if still > 30 {
                    last_rest = p;
                    let after = Vec2::new(cup(hole).x - p.x, cup(hole).z - p.z).length();
                    if after > before * 0.85 {
                        boost *= 1.3;
                    }
                    break;
                }
            }
        }
        total += strokes;
        let through = deepest < -0.08;
        println!(
            "golf --test: delik {} — {} vuruş, {}; topun yere en çok girdiği {:.3} m{}",
            hole + 1,
            strokes,
            if sunk { "girdi" } else { "GİRMEDİ" },
            -deepest.min(0.0),
            if through { " — ARAZİNİN İÇİNE DÜŞTÜ" } else { "" }
        );
        ok &= sunk && !through;
        if let Some(e) = world.get_entity(ball) {
            world.despawn(e);
        }
    }
    println!("golf --test: toplam {total} vuruş → {}", if ok { "tamam" } else { "BAŞARISIZ" });
    ok
}

struct Game {
    physics: FixedPhysics,
    hole: usize,
    ball: u32,
    strokes: Vec<u32>,
    power: Option<f32>,
    still: f32,
    last_rest: Vec3,
    camera: u32,
    flag: u32,
    cup_disc: u32,
    aim: u32,
    hud: u32,
    banner: u32,
    done: bool,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Golf", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            spawn_course(scene.world);
            let grass = AssetManager::terrain_triangles(&heights(), SAMPLES, SAMPLES, SIZE, SIZE, MAX_HEIGHT);
            // Saha mesh'i çarpıştırıcıyla aynı varlıkta değil; aynı yerde ve aynı örneklerden.
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                MeshData::new(grass),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#4C9A2A").to_vec4(), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.9)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.96, 0.88), 1.9, LightRole::Sun),
            ));
            let camera = scene
                .world
                .spawn_bundle(CameraBundle { position: Vec3::new(0.0, 14.0, 20.0), pitch: -0.6, primary: true, ..Default::default() })
                .id();
            let flag = scene
                .world
                .spawn_bundle((
                    Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.04, 1.0, 0.04)),
                    GlobalTransform::default(),
                    MeshSource("cylinder".into()),
                    MaterialDesc::new().with_unlit(Color::srgb_hex("#E74C3C").to_vec4()),
                    MeshRenderer::new(),
                ))
                .id();
            let cup_disc = scene
                .world
                .spawn_bundle((
                    Transform::new(Vec3::ZERO).with_scale(Vec3::new(CUP_R, 0.02, CUP_R)),
                    GlobalTransform::default(),
                    MeshSource("cylinder".into()),
                    MaterialDesc::new().with_unlit(Color::srgb_hex("#111111").to_vec4()),
                    MeshRenderer::new(),
                ))
                .id();
            let aim = scene
                .world
                .spawn_bundle((
                    Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.05, 0.02, 0.6)),
                    GlobalTransform::default(),
                    MeshSource("standard_cube".into()),
                    MaterialDesc::new().with_unlit(Vec4::new(1.0, 1.0, 1.0, 0.8)),
                    MeshRenderer::new(),
                ))
                .id();
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),)).id();
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();
            let mut game = Game {
                physics: FixedPhysics::new(60),
                hole: 0,
                ball: 0,
                strokes: vec![0; HOLES.len()],
                power: None,
                still: 0.0,
                last_rest: Vec3::ZERO,
                camera,
                flag,
                cup_disc,
                aim,
                hud,
                banner,
                done: false,
            };
            // Fizik dünyası ilk adımda kurulur; başlangıç yüksekliği ondan sonra okunabilir.
            gizmo::physics::physics_step_system(scene.world, DT);
            start_hole(scene.world, &mut game, 0);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

#[derive(Clone, Copy)]
struct Ball;
gizmo::core::impl_component!(Ball);

fn start_hole(world: &mut World, game: &mut Game, hole: usize) {
    let balls: Vec<u32> = world.borrow::<Ball>().iter().map(|(id, _)| id).collect();
    for id in balls {
        if let Some(e) = world.get_entity(id) {
            world.despawn(e);
        }
    }
    game.hole = hole;
    let at = start_position(world, hole);
    game.ball = spawn_ball(world, at);
    if let Some(e) = world.get_entity(game.ball) {
        world.add_component(e, GlobalTransform::default());
        world.add_component(e, MeshSource("sphere".into()));
        world.add_component(e, MaterialDesc::new().with_pbr(Color::srgb_hex("#F8F8F8").to_vec4(), 0.3, 0.0));
        world.add_component(e, MeshRenderer::new());
        world.add_component(e, Ball);
    }
    game.last_rest = at;
    game.power = None;
    let c = cup(hole);
    let g = world.get_resource::<PhysicsWorld>().and_then(|p| ground_at(&p, c.x, c.z, None)).unwrap_or(0.0);
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.cup_disc) {
        t.position = Vec3::new(c.x, g + 0.01, c.z);
    }
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.flag) {
        t.position = Vec3::new(c.x, g + 1.0, c.z);
    }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        game.strokes = vec![0; HOLES.len()];
        game.done = false;
        start_hole(world, &mut game, 0);
    }
    let (pos, speed) = ball_state(world, game.ball);
    let resting = speed < 0.05 && game.still > 0.4;
    let target = gizmo::pointer::cursor_on_plane(world, pos, Vec3::Y);
    let dir = target.map(|t| Vec3::new(t.x - pos.x, 0.0, t.z - pos.z).normalize_or_zero()).unwrap_or(Vec3::NEG_Z);
    if !game.done && resting {
        if input.is_mouse_button_pressed(0) {
            let p = game.power.unwrap_or(0.0) + dt / 1.2;
            game.power = Some(if p > 1.0 { 0.0 } else { p });
        } else if let Some(p) = game.power.take() {
            game.last_rest = pos;
            game.strokes[game.hole] += 1;
            hit(world, game.ball, dir, p);
            game.still = 0.0;
        }
    }
    game.physics.advance(world, dt);
    sync_transforms(world);
    let (pos, speed) = ball_state(world, game.ball);
    game.still = if speed < 0.05 { game.still + dt } else { 0.0 };
    if !game.done && in_cup(pos, speed, game.hole) {
        if game.hole + 1 < HOLES.len() {
            let next = game.hole + 1;
            start_hole(world, &mut game, next);
        } else {
            game.done = true;
        }
    } else if out_of_bounds(pos) {
        game.strokes[game.hole] += 1;
        if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.ball) {
            t.position = game.last_rest;
        }
        hit(world, game.ball, Vec3::X, 0.0);
    }
    // Nişan çizgisi, kamera.
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.aim) {
        let len = 0.6 + game.power.unwrap_or(0.0) * 2.0;
        t.position = pos + dir * (BALL_R + len) + Vec3::Y * 0.05;
        t.rotation = Quat::from_rotation_y(dir.x.atan2(dir.z));
        t.scale = Vec3::new(0.05, 0.02, len);
    }
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.camera) {
        let want = pos + Vec3::new(0.0, 9.0, 12.0);
        t.position = t.position.lerp(want, (dt * 2.5).min(1.0));
        t.rotation = Quat::from_rotation_x(-0.62);
    }
    let total: u32 = game.strokes.iter().sum();
    let card: Vec<String> = game.strokes.iter().map(|s| s.to_string()).collect();
    set_text(
        world,
        game.hud,
        &format!(
            "delik {}/{}   vuruş {}   (kart: {}, toplam {total})\nfare: nişan   sol tuş basılı: güç {:.0}%   R: baştan",
            game.hole + 1,
            HOLES.len(),
            game.strokes[game.hole],
            card.join(" · "),
            game.power.unwrap_or(0.0) * 100.0
        ),
    );
    set_text(world, game.banner, if game.done { "Bitti!\nR ile yeniden" } else { "" });
    world.insert_resource(game);
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
