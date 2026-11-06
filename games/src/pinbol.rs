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
//! | **dokunmatik**: ekranın sol / sağ yarısı | sol / sağ palet; kanaldaki topu da atar; oyun bitince yeniden başlatır |
//!
//! Masaüstünde `demo/src/bin/pinbol.rs`, tarayıcıda `demo-web` (`?oyun=pinbol`) çalıştırıyor — ikisi
//! de [`app`]'i. Dokunmatik düğmeler ekrana ilk dokunuşta beliriyor; masaüstünde hiç görünmüyor.
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
use gizmo::simple::{set_text, sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};
use gizmo::ui::{BackgroundColor, NodeBundle, PositionType, Style, UiPlugin, UiRect, Val, VirtualButton};

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
        let id = world.spawn_bundle((Transform::new(c), RigidBody::new_static(), Velocity::default(), Collider::box_collider(half).with_material(mat)));
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
            ;
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
            ;
        if visual {
            add_visual(world, id, MeshSource("cylinder".into()), Vec3::new(BUMPER_R, 0.08, BUMPER_R), "#E74C3C");
        }
    }
}

fn add_visual(world: &mut World, id: Entity, mesh: MeshSource, scale: Vec3, colour: &str) {
    if let Some(e) = world.live(id) {
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

fn spawn_flippers(world: &mut World, visual: bool) -> [Entity; 2] {
    let mut out = [Entity::new(0, 0); 2];
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
            ;
        if visual {
            add_visual(world, id, MeshSource("standard_cube".into()), half, "#ECF0F1");
        }
        out[k] = id;
    }
    out
}

fn spawn_ball(world: &mut World, visual: bool) -> Entity {
    let col = Collider::sphere(BALL).with_material(PhysicsMaterial { restitution: 0.3, ..Default::default() });
    let mut rb = RigidBody::new(0.08, true);
    rb.ccd_enabled = true;
    rb.update_inertia_from_collider(&col);
    let id = world.spawn_bundle((Transform::new(Vec3::new(LANE_X, BALL, HALF_L - 0.15)), rb, Velocity::default(), col));
    if visual {
        if let Some(e) = world.live(id) {
            world.add_component(e, GlobalTransform::default());
            world.add_component(e, MeshData::sphere(BALL, 10, 14));
            world.add_component(e, MaterialDesc::new().with_pbr(Color::srgb_hex("#BDC3C7").to_vec4(), 0.15, 1.0));
            world.add_component(e, MeshRenderer::new());
        }
    }
    id
}

fn ball_state(world: &World, ball: Entity) -> (Vec3, Vec3) {
    (
        world.borrow::<Transform>().get(ball).map(|t| t.position).unwrap_or(Vec3::ZERO),
        world.borrow::<Velocity>().get(ball).map(|v| v.linear).unwrap_or(Vec3::ZERO),
    )
}

fn launch(world: &mut World, ball: Entity) {
    if let Some(mut v) = world.borrow_mut::<Velocity>().get_mut(ball) {
        v.linear = Vec3::new(0.0, 0.0, -4.2);
    }
    if let Some(mut rb) = world.borrow_mut::<RigidBody>().get_mut(ball) {
        rb.wake_up();
    }
}

struct Table {
    physics: FixedPhysics,
    flippers: [Entity; 2],
    angles: [f32; 2],
    ball: Entity,
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
        // Kanala geri yuvarlanan top dibindeki durakta bekler: yeniden atılabilir. (Atılmış
        // sayıldığı için orada sonsuza kadar kalıyordu.)
        if !self.in_lane && p.x > LANE_X - 0.07 && p.z > HALF_L - 0.2 && v.length() < 0.05 {
            self.in_lane = true;
        }
        // Düştü: alttaki boşluktan çıktı.
        if p.z > HALF_L + 0.1 || p.y < -0.5 || !p.is_finite() {
            self.drains += 1;
            if let Some(e) = world.live(self.ball) {
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
pub fn self_test() -> bool {
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
    let touch_ok = touch_test();
    let ok = !table.escaped && table.hits >= 5 && touch_ok;
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
    hud: Entity,
    banner: Entity,
    /// Dokunmatik paletler (sol, sağ). Kurulumda, görünmez kuruluyorlar: bir düğme yalnızca
    /// üstüne inen parmağı sahipleniyor, yani ilk dokunuşta kurulan bir düğme o dokunuşu
    /// kaçırıyordu — telefonda ilk palet vuruşu boşa gidiyordu. Motor artık yeni kurulan bir
    /// kontrole altında duran parmağı veriyor (2026-11-05); düğmeler yine de kurulumda kuruluyor,
    /// ilk karede yerleşimleri hazır olsun diye.
    touch: [Entity; 2],
    /// Ekrana dokunuldu mu: düğmeler o zaman görünür, HUD dokunmatik ipucunu yazar.
    touched: bool,
}

/// Bütün oyun: sahne, girdi, güncelleme. Masaüstü ikilisi ve tarayıcı yapısı bunu çalıştırıyor.
pub fn app() -> App<SimpleSceneState> {
    App::<SimpleSceneState>::new("Gizmo Engine - Pinbol", 1280, 720)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, _state| {
            // The ball is chrome, and chrome shows only what it reflects: under the default studio
            // environment it is nearly black on a dark table. This is the pale sky behind the table.
            scene.renderer.sky = Some(gizmo::renderer::frame_uniforms::Sky::gradient(
                [0.55, 0.68, 0.85],
                [0.60, 0.68, 0.75],
                [0.20, 0.20, 0.22],
            ));
            let font = scene.renderer.default_font();
            let game = setup(scene.world, font);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
}

/// Masa, ışık, kamera, HUD (pencere de GPU da gerekmez).
fn setup(world: &mut World, font: gizmo::renderer::text::FontId) -> Game {
    let table = Table::new(world, true);
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-1.1)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 2.4, LightRole::Sun),
    ));
    // Masaya, oyuncunun ucundan (−z'ye) bakıyor. `yaw` verilmemişti: varsayılan 0 +x'e bakar, ve
    // masa görüşün solunda, yarısı kadrajın dışındaydı (pinbol yalnız başsız sınanmıştı; telefonda
    // ilk açılışta görüldü).
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 2.3, 2.3),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.85,
        primary: true,
        ..Default::default()
    });
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    let banner = world
        .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
            .with_anchor(TextAnchor::Center)
            .with_align(TextAlign::Center),))
        ;
    let touch = spawn_touch_buttons(world);
    Game { table, balls_left: BALLS, auto: Autopilot::default(), hud, banner, touch, touched: false }
}

/// Dokunmatik paletler, pencere açmadan: ekranın sol yarısına inen bir parmak sol paleti kaldırıyor,
/// sağ palet inik kalıyor; parmak kalkınca palet iniyor. Oyunun kendi kurulumu ve güncellemesi,
/// arayüz eklentisiyle.
fn touch_test() -> bool {
    use gizmo::core::input::touch::TouchPhase;
    let mut run = HeadlessFrames::new(World::new()).add_plugin(UiPlugin);
    let game = setup(&mut run.world, HeadlessFrames::font());
    run.world.insert_resource(game);
    let angles = |run: &HeadlessFrames| run.world.get_resource::<Game>().map(|g| g.table.angles).unwrap_or_default();
    let frames = |run: &mut HeadlessFrames, n: usize| {
        for _ in 0..n {
            run.frame(1.0 / 60.0, update);
        }
    };
    frames(&mut run, 5);
    let size = gizmo::core::window::WindowInfo::default();
    // Bu ilk dokunuş: palet hemen kalkmalı.
    run.input.on_touch(1, TouchPhase::Started, size.width * 0.25, size.height * 0.6);
    frames(&mut run, 20);
    let held = angles(&run);
    run.input.on_touch(1, TouchPhase::Ended, size.width * 0.25, size.height * 0.6);
    frames(&mut run, 20);
    let released = angles(&run);
    let up = |a: f32| (a - FLIPPER_UP).abs() < 0.01;
    let down = |a: f32| (a - FLIPPER_DOWN).abs() < 0.01;
    let ok = up(held[0]) && down(held[1]) && down(released[0]);
    println!(
        "    dokunma: sol yarıda parmak → paletler {:.2} / {:.2} (kalkık {FLIPPER_UP}, inik {FLIPPER_DOWN}), parmak kalkınca sol {:.2} → {}",
        held[0],
        held[1],
        released[0],
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

/// Ekranın iki yarısını kaplayan iki dokunmatik palet düğmesi (sol, sağ), görünmez. Her biri kendi
/// parmağını tutuyor: iki palet aynı anda kaldırılabiliyor.
fn spawn_touch_buttons(world: &mut World) -> [Entity; 2] {
    let mut half = |left: Val, right: Val| {
        let e = world.spawn_bundle(NodeBundle {
            style: Style {
                position_type: PositionType::Absolute,
                inset: UiRect { left, right, top: Val::Percent(0.0), bottom: Val::Percent(0.0) },
                width: Val::Percent(50.0),
                ..Default::default()
            },
            background_color: BackgroundColor(Vec4::ZERO),
            ..Default::default()
        });
        world.add_component(e, VirtualButton::default());
        e
    };
    [half(Val::Percent(0.0), Val::Auto), half(Val::Auto, Val::Percent(0.0))]
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.03);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        game.balls_left = BALLS;
        game.table.score = 0;
        game.table.drains = 0;
    }
    if !game.touched && !input.touches().is_empty() {
        game.touched = true;
        for id in game.touch {
            if let Some(mut bg) = world.borrow_mut::<BackgroundColor>().get_mut(id) {
                bg.0 = Vec4::new(1.0, 1.0, 1.0, 0.03);
            }
        }
    }
    // Dokunmatik paletler: basılı mı, bu karede mi basıldı.
    let touch: [(bool, bool); 2] = {
        let buttons = world.borrow::<VirtualButton>();
        game.touch.map(|id| buttons.get(id).map_or((false, false), |b| (b.is_pressed(), b.just_pressed())))
    };
    let tapped = touch.iter().any(|t| t.1);
    let (up, fire) = if std::env::var_os("PINBOL_OTO").is_some() {
        let Game { auto, table, .. } = &mut game;
        auto.press(world, table, dt)
    } else {
        (
            [
                input.is_key_pressed(KeyCode::KeyA as u32) || input.is_key_pressed(KeyCode::ArrowLeft as u32) || touch[0].0,
                input.is_key_pressed(KeyCode::KeyD as u32) || input.is_key_pressed(KeyCode::ArrowRight as u32) || touch[1].0,
            ],
            input.is_key_just_pressed(KeyCode::Space as u32) || tapped,
        )
    };
    let over = game.table.drains >= game.balls_left;
    // Oyun bitince bir dokunuş da baştan başlatıyor.
    if over && tapped {
        game.balls_left = BALLS;
        game.table.score = 0;
        game.table.drains = 0;
    }
    if !over {
        game.table.frame(world, dt, up, fire, true);
    }
    sync_transforms(world);
    let left = game.balls_left.saturating_sub(game.table.drains);
    set_text(
        world,
        game.hud,
        &format!(
            "puan {}   top {left}\n{}{}",
            game.table.score,
            if game.touched { "sol / sağ yarı: paletler, dokun: at" } else { "A/D: paletler   Space: at   R: baştan" },
            if game.table.in_lane { "  (top kanalda)" } else { "" }
        ),
    );
    let banner = if over { format!("Oyun bitti: {} puan\nR ile yeniden", game.table.score) } else { String::new() };
    set_text(world, game.banner, &banner);
    world.insert_resource(game);
}
