//! # Pinbol — eğik masada bir top, iki palet, üç tampon
//!
//! Top eğik masadan aşağı yuvarlanır; iki paletle geri vur, tamponlara çarptır, puan topla.
//! Top alttaki boşluktan düşerse kaybolur; üç top. Motorun otuz beşinci oyunu: **hızlı küçük bir
//! top** ve onu vuran **hızla dönen kinematik paletler**.
//!
//! | girdi | iş |
//! |---|---|
//! | **A / ←** | sol palet |
//! | **D / →** | sağ palet |
//! | **Space** | topu at (atış kanalındayken) |
//! | **R** | baştan |
//!
//! `pinbol --test` pencere açmaz: otomatik palet bir dakika oynar; kaç kez vurduğu, topun kaç kez
//! düştüğü, ve masanın dışına (duvarın içinden) hiç çıkıp çıkmadığı yazılır. (`PINBOL_OTO=1`
//! pencereli oyunda da otomatik paleti açar.)

use gizmo::core::input::Input;
use gizmo::physics::components::{KinematicTarget, PhysicsMaterial};
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{MeshData, Text, TextAlign, TextAnchor};
use gizmo::simple::{set_text, sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

/// Masanın yarı genişliği (x) ve yarı boyu (z); oyuncu +z tarafında.
const HALF_W: f32 = 0.6;
const HALF_L: f32 = 1.2;
const TILT_DEG: f32 = 12.0;
const BALL: f32 = 0.05;
const WALL: f32 = 0.05;
/// Paletler: menteşe noktası, boyu, dinlenme ve kalkık açısı, dönüş hızı.
const FLIPPER_Z: f32 = 0.95;
const FLIPPER_X: f32 = 0.27;
const FLIPPER_LEN: f32 = 0.24;
const FLIPPER_DOWN: f32 = -0.45;
const FLIPPER_UP: f32 = 0.45;
const FLIPPER_SPEED: f32 = 25.0;
const BUMPERS: [(f32, f32); 3] = [(-0.22, -0.45), (0.22, -0.45), (0.0, -0.75)];
const BUMPER_R: f32 = 0.08;
const KICK: f32 = 1.8;
/// Atış kanalı: sağ kenar boyunca bir şerit.
const LANE_X: f32 = HALF_W - 0.08;
const BALLS: u32 = 3;

fn gravity() -> Vec3 {
    let t = TILT_DEG.to_radians();
    Vec3::new(0.0, -9.81 * t.cos(), 9.81 * t.sin())
}

fn spawn_table(world: &mut World, visual: bool) {
    world.insert_resource(PhysicsWorld::new().with_gravity(gravity()));
    let felt = PhysicsMaterial { static_friction: 0.2, dynamic_friction: 0.15, restitution: 0.2, ..Default::default() };
    let rail = PhysicsMaterial { static_friction: 0.1, dynamic_friction: 0.1, restitution: 0.6, ..Default::default() };
    let mut walls = vec![(Vec3::new(0.0, -0.05, 0.0), Vec3::new(HALF_W, 0.05, HALF_L), felt, "#1B4F72")];
    let h = 0.08;
    // Kenarlar; alttaki kenar ortada açık (boşluk paletlerin arasında).
    walls.push((Vec3::new(-HALF_W - WALL, h, 0.0), Vec3::new(WALL, h, HALF_L), rail, "#D4AC0D"));
    walls.push((Vec3::new(HALF_W + WALL, h, 0.0), Vec3::new(WALL, h, HALF_L), rail, "#D4AC0D"));
    walls.push((Vec3::new(0.0, h, -HALF_L - WALL), Vec3::new(HALF_W + 2.0 * WALL, h, WALL), rail, "#D4AC0D"));
    // Atış kanalını ayıran iç duvar, ve paletlere giden eğik kılavuzlar.
    walls.push((Vec3::new(LANE_X - 0.08, h, 0.35), Vec3::new(0.01, h, 0.75), rail, "#D4AC0D"));
    // Kanalın dibi: atılmayı bekleyen top burada durur.
    walls.push((Vec3::new((LANE_X - 0.07 + HALF_W) * 0.5, h, HALF_L + WALL), Vec3::new((HALF_W - LANE_X + 0.07) * 0.5, h, WALL), rail, "#D4AC0D"));
    for (c, half, mat, colour) in walls {
        let id = world.spawn_bundle((Transform::new(c), RigidBody::new_static(), Velocity::default(), Collider::box_collider(half).with_material(mat))).id();
        if visual {
            add_visual(world, id, MeshSource("standard_cube".into()), half, colour);
        }
    }
    let corner = 0.3;
    let slants = [
        // Paletin menteşesinden kenara uzanan eğik kılavuzlar.
        (Vec3::new(-HALF_W, h, FLIPPER_Z - 0.18), Vec3::new(-(FLIPPER_X + 0.02), h, FLIPPER_Z - 0.02)),
        (Vec3::new(LANE_X - 0.09, h, FLIPPER_Z - 0.18), Vec3::new(FLIPPER_X + 0.02, h, FLIPPER_Z - 0.02)),
        // Üst köşeler: kanaldan çıkan topu masanın içine çevirir.
        (Vec3::new(HALF_W, h, -HALF_L + corner), Vec3::new(HALF_W - corner, h, -HALF_L)),
        (Vec3::new(-HALF_W, h, -HALF_L + corner), Vec3::new(-HALF_W + corner, h, -HALF_L)),
    ];
    for (from, to) in slants {
        let mid = (from + to) * 0.5;
        let along = to - from;
        let half = Vec3::new(0.012, h, along.length() * 0.5);
        let rot = Quat::from_rotation_y(along.x.atan2(along.z));
        let id = world
            .spawn_bundle((Transform::new(mid).with_rotation(rot), RigidBody::new_static(), Velocity::default(), Collider::box_collider(half).with_material(rail)))
            .id();
        if visual {
            add_visual(world, id, MeshSource("standard_cube".into()), half, "#D4AC0D");
        }
    }
    for &(x, z) in &BUMPERS {
        let id = world
            .spawn_bundle((
                Transform::new(Vec3::new(x, 0.08, z)),
                RigidBody::new_static(),
                Velocity::default(),
                Collider::cylinder(BUMPER_R, 0.08).with_material(PhysicsMaterial { restitution: 0.9, ..Default::default() }),
            ))
            .id();
        if visual {
            add_visual(world, id, MeshSource("cylinder".into()), Vec3::new(BUMPER_R, 0.08, BUMPER_R), "#E74C3C");
        }
    }
}

fn add_visual(world: &mut World, id: u32, mesh: MeshSource, scale: Vec3, colour: &str) {
    if let Some(e) = world.get_entity(id) {
        world.add_component(e, GlobalTransform::default());
        world.add_component(e, mesh);
        world.add_component(e, MaterialDesc::new().with_pbr(Color::srgb_hex(colour).to_vec4(), 0.5, 0.2));
        world.add_component(e, MeshRenderer::new());
        if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(id) {
            t.scale = scale;
        }
    }
}

/// Paletin bu açıdaki pozu. `side` −1 sol, +1 sağ; açı, uç masanın ortasına doğru yukarı kalkarken
/// artar.
fn flipper_pose(side: f32, angle: f32) -> (Vec3, Quat) {
    let pivot = Vec3::new(side * FLIPPER_X, 0.04, FLIPPER_Z);
    // Dinlenmede uç içeri ve aşağı (oyuncuya doğru) bakar.
    let dir = Vec3::new(-side * angle.cos(), 0.0, -angle.sin());
    let rot = Quat::from_rotation_y((-dir.x).atan2(-dir.z));
    (pivot + dir * (FLIPPER_LEN * 0.5), rot)
}

fn spawn_flippers(world: &mut World, visual: bool) -> [u32; 2] {
    let mut out = [0u32; 2];
    for (k, side) in [-1.0f32, 1.0].into_iter().enumerate() {
        let (pos, rot) = flipper_pose(side, FLIPPER_DOWN);
        let half = Vec3::new(0.025, 0.035, FLIPPER_LEN * 0.5);
        let id = world
            .spawn_bundle((
                Transform::new(pos).with_rotation(rot),
                RigidBody::new_kinematic(),
                Velocity::default(),
                Collider::box_collider(half).with_material(PhysicsMaterial { restitution: 0.3, ..Default::default() }),
                KinematicTarget::new(pos, rot),
            ))
            .id();
        if visual {
            add_visual(world, id, MeshSource("standard_cube".into()), half, "#ECF0F1");
        }
        out[k] = id;
    }
    out
}

fn spawn_ball(world: &mut World, visual: bool) -> u32 {
    let col = Collider::sphere(BALL).with_material(PhysicsMaterial { restitution: 0.3, ..Default::default() });
    let mut rb = RigidBody::new(0.08, true);
    rb.ccd_enabled = true;
    rb.update_inertia_from_collider(&col);
    let id = world.spawn_bundle((Transform::new(Vec3::new(LANE_X, BALL, HALF_L - 0.15)), rb, Velocity::default(), col)).id();
    if visual {
        if let Some(e) = world.get_entity(id) {
            world.add_component(e, GlobalTransform::default());
            world.add_component(e, MeshData::sphere(BALL, 10, 14));
            world.add_component(e, MaterialDesc::new().with_pbr(Color::srgb_hex("#BDC3C7").to_vec4(), 0.15, 1.0));
            world.add_component(e, MeshRenderer::new());
        }
    }
    id
}

fn ball_state(world: &World, ball: u32) -> (Vec3, Vec3) {
    (
        world.borrow::<Transform>().get(ball).map(|t| t.position).unwrap_or(Vec3::ZERO),
        world.borrow::<Velocity>().get(ball).map(|v| v.linear).unwrap_or(Vec3::ZERO),
    )
}

fn launch(world: &mut World, ball: u32) {
    if let Some(mut v) = world.borrow_mut::<Velocity>().get_mut(ball) {
        v.linear = Vec3::new(0.0, 0.0, -4.2);
    }
    if let Some(mut rb) = world.borrow_mut::<RigidBody>().get_mut(ball) {
        rb.wake_up();
    }
}

struct Table {
    physics: FixedPhysics,
    flippers: [u32; 2],
    angles: [f32; 2],
    ball: u32,
    in_lane: bool,
    score: u32,
    hits: u32,
    drains: u32,
    escaped: bool,
}

impl Table {
    fn new(world: &mut World, visual: bool) -> Self {
        spawn_table(world, visual);
        let flippers = spawn_flippers(world, visual);
        let ball = spawn_ball(world, visual);
        Self { physics: FixedPhysics::new(120), flippers, angles: [FLIPPER_DOWN; 2], ball, in_lane: true, score: 0, hits: 0, drains: 0, escaped: false }
    }

    /// Bir kare: paletler `up`'a göre döner, fizik ilerler, tamponlar tekmeler, düşen top yenilenir.
    fn frame(&mut self, world: &mut World, dt: f32, up: [bool; 2], fire: bool, visual: bool) {
        for (k, &raised) in up.iter().enumerate() {
            let want = if raised { FLIPPER_UP } else { FLIPPER_DOWN };
            let a = &mut self.angles[k];
            *a += (want - *a).clamp(-FLIPPER_SPEED * dt, FLIPPER_SPEED * dt);
            let side = if k == 0 { -1.0 } else { 1.0 };
            let (pos, rot) = flipper_pose(side, *a);
            if let Some(mut t) = world.borrow_mut::<KinematicTarget>().get_mut(self.flippers[k]) {
                *t = KinematicTarget::new(pos, rot);
            }
        }
        if fire && self.in_lane {
            launch(world, self.ball);
            self.in_lane = false;
        }
        let before = ball_state(world, self.ball).1;
        self.physics.advance(world, dt);
        world.apply_commands();
        let (p, v) = ball_state(world, self.ball);
        // Palet vuruşu: top paletlerin hizasında yukarı (−z) hızlandıysa.
        if p.z > FLIPPER_Z - 0.2 && before.z > -0.5 && v.z < -2.0 {
            self.hits += 1;
        }
        // Tampon: değdiyse dışarı doğru bir tekme ve puan.
        for &(x, z) in &BUMPERS {
            let away = Vec3::new(p.x - x, 0.0, p.z - z);
            if away.length() < BUMPER_R + BALL + 0.006 {
                let n = away.normalize_or_zero();
                if let Some(mut bv) = world.borrow_mut::<Velocity>().get_mut(self.ball) {
                    let out = bv.linear.dot(n);
                    if out < KICK {
                        bv.linear += n * (KICK - out.max(0.0));
                        self.score += 100;
                    }
                }
            }
        }
        let outside = p.x.abs() > HALF_W + 0.02 || p.z < -HALF_L - 0.02 || p.y < -0.2;
        if !p.is_finite() || (outside && p.z < HALF_L + 0.05) {
            self.escaped = true;
        }
        // Düştü: alttaki boşluktan çıktı.
        if p.z > HALF_L + 0.1 || p.y < -0.5 || !p.is_finite() {
            self.drains += 1;
            if let Some(e) = world.get_entity(self.ball) {
                world.despawn(e);
            }
            world.apply_commands();
            self.ball = spawn_ball(world, visual);
            self.in_lane = true;
        }
    }
}

/// Otomatik palet: top bir paletin üstüne gelince o paleti kaldırır, kısa tutar, indirir ve
/// yeniden kurulmadan önce bekler (sürekli kalkık bir palet topu üstünde uyutur).
#[derive(Default)]
struct Autopilot {
    timer: [f32; 2],
}

impl Autopilot {
    const CYCLE: f32 = 0.45;
    const HOLD: f32 = 0.2;

    fn press(&mut self, world: &World, table: &Table, dt: f32) -> ([bool; 2], bool) {
        let (p, v) = ball_state(world, table.ball);
        let near = p.z > FLIPPER_Z - 0.12 && p.z < FLIPPER_Z + 0.08 && v.z > -0.5;
        let over = [near && p.x < 0.02, near && p.x > -0.02 && p.x < LANE_X - 0.1];
        let mut up = [false; 2];
        for k in 0..2 {
            self.timer[k] -= dt;
            if over[k] && self.timer[k] <= 0.0 {
                self.timer[k] = Self::CYCLE;
            }
            up[k] = self.timer[k] > Self::CYCLE - Self::HOLD;
        }
        (up, table.in_lane)
    }
}

/// `--test`: bir dakika otomatik palet.
fn self_test() -> bool {
    let mut world = World::new();
    let mut table = Table::new(&mut world, false);
    let mut auto = Autopilot::default();
    let dt = 1.0 / 60.0;
    let mut time = 0.0;
    while time < 60.0 {
        time += dt;
        let (up, fire) = auto.press(&world, &table, dt);
        table.frame(&mut world, dt, up, fire, false);
    }
    let ok = !table.escaped && table.hits >= 5;
    println!(
        "pinbol --test: 60 s; paletler {} kez vurdu, tamponlar {} puan, top {} kez düştü; masadan kaçtı mı: {} → {}",
        table.hits,
        table.score,
        table.drains,
        if table.escaped { "EVET" } else { "hayır" },
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

struct Game {
    table: Table,
    balls_left: u32,
    auto: Autopilot,
    hud: u32,
    banner: u32,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Pinbol", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = scene.renderer.default_font();
            let table = Table::new(scene.world, true);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-1.1)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 2.4, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle { position: Vec3::new(0.0, 2.3, 2.3), pitch: -0.85, primary: true, ..Default::default() });
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),)).id();
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();
            scene.world.insert_resource(Game { table, balls_left: BALLS, auto: Autopilot::default(), hud, banner });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.03);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        game.balls_left = BALLS;
        game.table.score = 0;
        game.table.drains = 0;
    }
    let (up, fire) = if std::env::var_os("PINBOL_OTO").is_some() {
        let Game { auto, table, .. } = &mut game;
        auto.press(world, table, dt)
    } else {
        (
            [
                input.is_key_pressed(KeyCode::KeyA as u32) || input.is_key_pressed(KeyCode::ArrowLeft as u32),
                input.is_key_pressed(KeyCode::KeyD as u32) || input.is_key_pressed(KeyCode::ArrowRight as u32),
            ],
            input.is_key_just_pressed(KeyCode::Space as u32),
        )
    };
    let over = game.table.drains >= game.balls_left;
    if !over {
        game.table.frame(world, dt, up, fire, true);
    }
    sync_transforms(world);
    let left = game.balls_left.saturating_sub(game.table.drains);
    set_text(
        world,
        game.hud,
        &format!(
            "puan {}   top {left}\nA/D: paletler   Space: at{}   R: baştan",
            game.table.score,
            if game.table.in_lane { "  (top kanalda)" } else { "" }
        ),
    );
    let banner = if over { format!("Oyun bitti: {} puan\nR ile yeniden", game.table.score) } else { String::new() };
    set_text(world, game.banner, &banner);
    world.insert_resource(game);
}
