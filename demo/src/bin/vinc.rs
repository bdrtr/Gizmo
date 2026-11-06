//! # Vinç — gülleyle binayı yık
//!
//! Yandan bir şantiye: tepede bir vinç kancası, ona 6 m'lik bir iple asılı 300 kg'lık bir gülle,
//! sağda kırılabilir bloklardan bir bina. Kancayı sağa sola sürüp gülleyi sallandır; sert vuran
//! gülle bloğu parçalara ayırır, sarsılan bloklar devrilir. 45 saniyede binanın beşte dördünü
//! indir. Motorun yirmi birinci oyunu ve **kırılmayı** kullanan ilki.
//!
//! | girdi | iş |
//! |---|---|
//! | **A / D · sol çubuk** | kancayı sür |
//! | **R** | baştan |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **`Breakable` bir oyunda hiç kırılmıyordu.** Kırılma sistemini ne `FixedPhysics` ne
//!   `PhysicsPlugin` çalıştırıyordu; oyun kendisi çağırmazsa duvar ölümsüzdü. **Düzeltildi:**
//!   facade'in fizik adımı kırılmayı da çalıştırıyor.
//! - **Parçalar görünmezdi, ölümsüzdü, iki kez ölçekleniyordu.** Kırılan blok yok oluyor, yerine
//!   küre çarpıştırıcılı, geometrisiz gövdeler geliyordu; bloğun `scale`'ini miras alıyorlardı ve
//!   `debris_lifetime` hiçbir şey tarafından okunmuyordu. **Düzeltildi:** her parça `Debris`
//!   taşıyor (kendi Voronoi hücresi), kırılan bloğun malzemesiyle çiziliyor, ölçeği bir, ömrü
//!   dolunca siliniyor.
//! - **İp görünmüyordu.** `Gizmos` basit sahnede yoktu; `draw_line` çağrıları sessizce boşa
//!   gidiyordu. **Düzeltildi:** basit sahne boş bir `Gizmos` kuruyor.
//!
//! Gerisi ilk denemede çalıştı: `Joint::rope` (kanca ile gülle), kanca `KinematicTarget` ile.
//! Blokların kırılma eşiği 90 N·s: 300 kg'lık güllenin 30 kg'lık bloğa vuruşu ~160 N·s, 250'lik bir
//! eşik hiçbirini kırmadı — yalnız devirdi.
//!
//! ## Ortam değişkenleri
//!   * `VINC_OTO=1` — kanca kendi kendine sallanır (ekran görüntüsü için; `vinc --test` aynısını
//!     pencere açmadan yapar)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::core::input::Input;
use gizmo::physics::components::{Breakable, KinematicTarget};
use gizmo::physics::joints::Joint;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{BodyHandle, Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

const HOOK_Y: f32 = 10.0;
const HOOK_RANGE: (f32, f32) = (-9.0, 6.0);
const HOOK_SPEED: f32 = 5.0;
const ROPE: f32 = 6.5;
const BALL_R: f32 = 0.7;
const BALL_MASS: f32 = 300.0;
const COLUMNS: usize = 4;
const FLOORS: usize = 8;
const BLOCK: Vec3 = Vec3::new(0.5, 0.35, 0.6);
const BUILDING_X: f32 = 4.0;
const TIME: f32 = 45.0;
const GOAL: f32 = 0.8;

#[derive(Clone, Copy)]
struct Actor;
gizmo::core::impl_component!(Actor);

/// Bir blok ve ilk yüksekliği.
#[derive(Clone, Copy)]
struct Block(f32);
gizmo::core::impl_component!(Block);

struct Game {
    physics: FixedPhysics,
    hook: Entity,
    hook_x: f32,
    ball: Entity,
    clock: f32,
    down: usize,
    broken: usize,
    hud: Entity,
    banner: Entity,
    over: Option<bool>,
    auto: bool,
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
fn setup(world: &mut World, font: FontId) -> Game {

    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(-0.5) * Quat::from_rotation_x(-0.7)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.92, 0.82), 1.8, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(-1.0, 5.5, 19.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.08,
        primary: true,
        ..Default::default()
    });
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.5, 0.0)).with_scale(Vec3::new(30.0, 0.5, 8.0)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Color::srgb_hex("#6B5B45").to_vec4(), 0.95, 0.0),
        MeshRenderer::new(),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(Vec3::new(30.0, 0.5, 8.0)),
    ));
    // Vincin kirişi: yalnız görüntü.
    world.spawn_bundle((
        Transform::new(Vec3::new(-1.5, HOOK_Y + 0.35, 0.0)).with_scale(Vec3::new(8.5, 0.15, 0.15)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Color::srgb_hex("#E1A73B").to_vec4(), 0.5, 0.6),
        MeshRenderer::new(),
    ));
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    let banner = world
        .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
            .with_anchor(TextAnchor::Center)
            .with_align(TextAlign::Center),))
        ;
    let mut game = Game {
        physics: FixedPhysics::new(60),
        hook: Entity::new(0, 0),
        hook_x: HOOK_RANGE.0,
        ball: Entity::new(0, 0),
        clock: 0.0,
        down: 0,
        broken: 0,
        hud,
        banner,
        over: None,
        auto: std::env::var_os("VINC_OTO").is_some(),
    };
    restart(world, &mut game);
    game
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Vinç", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// `--test`: pencere açmadan otomatik pilot kancayı ipin periyodunda sallar; süre bitmeden binanın
/// beşte dördü inmeli, en az biri parçalanarak. Oyunun kendi `setup`'ı ve `update`'i, boş bir girdiyle, sabit adımla.
fn self_test() -> bool {
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font());
    game.auto = true;
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    while run.world.get_resource::<Game>().is_some_and(|g| g.over.is_none()) && run.elapsed < 120.0 {
        run.frame(dt, update);
    }
    let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
    // Kırılma da işlemeli: yalnız devrilen bloklarla kazanılan oyun kırılmanın bozulduğunu saklar.
    let ok = game.over == Some(true) && game.broken > 0;
    println!(
        "vinc --test: {:.1} s; {} / {} blok yıkıldı, {} parçalandı → {}",
        game.clock,
        game.down,
        COLUMNS * FLOORS,
        game.broken,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn restart(world: &mut World, game: &mut Game) {
    let actors: Vec<Entity> = world.query::<(Entity, &Actor)>().iter().map(|(entity, _)| entity).collect();
    let debris: Vec<Entity> = world.query::<(Entity, &gizmo::physics::components::Debris)>().iter().map(|(entity, _)| entity).collect();
    for id in actors.into_iter().chain(debris) {
        if let Some(e) = world.live(id) {
            world.despawn(e);
        }
    }
    game.hook_x = HOOK_RANGE.0;
    let hook_at = Vec3::new(game.hook_x, HOOK_Y, 0.0);
    game.hook = world
        .spawn_bundle((
            Transform::new(hook_at).with_scale(Vec3::new(0.3, 0.25, 0.3)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(Color::srgb_hex("#333333").to_vec4(), 0.5, 0.8),
            MeshRenderer::new(),
            RigidBody::new_kinematic(),
            Velocity::default(),
            Collider::box_collider(Vec3::new(0.3, 0.25, 0.3)),
            KinematicTarget::new(hook_at, Quat::IDENTITY),
            Actor,
        ))
        ;
    let mut ball = RigidBody::new(BALL_MASS, true);
    ball.lock_translation_z = true;
    game.ball = world
        .spawn_bundle((
            Transform::new(hook_at - Vec3::Y * ROPE).with_scale(Vec3::splat(BALL_R)),
            GlobalTransform::default(),
            MeshSource("sphere".into()),
            MaterialDesc::new().with_pbr(Color::srgb_hex("#2C3E50").to_vec4(), 0.3, 0.9),
            MeshRenderer::new(),
            ball,
            Velocity::default(),
            Collider::sphere(BALL_R),
            Actor,
        ))
        ;
    let colours = ["#C0392B", "#B03A2E", "#A93226", "#CB4335"];
    for floor in 0..FLOORS {
        for col in 0..COLUMNS {
            let at = Vec3::new(
                BUILDING_X + col as f32 * BLOCK.x * 2.0,
                BLOCK.y + floor as f32 * BLOCK.y * 2.0,
                0.0,
            );
            world.spawn_bundle((
                Transform::new(at).with_scale(BLOCK),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex(colours[(floor + col) % 4]).to_vec4(), 0.85, 0.0),
                MeshRenderer::new(),
                RigidBody::new(30.0, true),
                Velocity::default(),
                Collider::box_collider(BLOCK),
                Breakable {
                    max_pieces: 6,
                    threshold: 90.0,
                    current_health: 90.0,
                    max_health: 90.0,
                    debris_lifetime: 4.0,
                    ..Default::default()
                },
                Block(at.y),
                Actor,
            ));
        }
    }
    if let Some(mut physics) = world.get_resource_mut::<PhysicsWorld>() {
        physics.joints.clear();
        physics
            .joints
            .push(Joint::rope(BodyHandle::from_id(game.hook.id()), BodyHandle::from_id(game.ball.id()), Vec3::ZERO, Vec3::ZERO, ROPE));
    }
    game.clock = 0.0;
    game.down = 0;
    game.broken = 0;
    game.over = None;
    game.physics.reset();
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        restart(world, &mut game);
    }
    if game.over.is_none() {
        game.clock += dt;
        let drive = if game.auto {
            // Salınımı besleyen bir ileri-geri: ipin kendi periyoduna yakın.
            let period = std::f32::consts::TAU * (ROPE / 9.81).sqrt();
            (game.clock * std::f32::consts::TAU / period).sin()
        } else {
            input.move_axis().0
        };
        game.hook_x = (game.hook_x + drive * HOOK_SPEED * dt).clamp(HOOK_RANGE.0, HOOK_RANGE.1);
        if let Some(mut k) = world.query_mut::<&mut KinematicTarget>().get_mut(game.hook) {
            *k = KinematicTarget::new(Vec3::new(game.hook_x, HOOK_Y, 0.0), Quat::IDENTITY);
        }
        game.physics.advance(world, dt);
        sync_transforms(world);
        count_down(world, &mut game);
        if game.down as f32 >= GOAL * (COLUMNS * FLOORS) as f32 {
            game.over = Some(true);
        } else if game.clock >= TIME {
            game.over = Some(false);
        }
    }
    hud(world, &game);
    draw_rope(world, &game);
    world.insert_resource(game);
}

/// Yıkılmış sayılan bloklar: kırılmış (artık yok), yan yatmış ya da ilk yerinden bir kattan
/// fazla aşağı düşmüş olanlar.
fn count_down(world: &World, game: &mut Game) {
    game.broken = COLUMNS * FLOORS - world.query::<&Block>().iter().count();
    let standing = {
        let ts = world.query::<&Transform>();
        world.query::<(Entity, &Block)>().iter()
            .filter(|(entity, b)| {
                ts.get(*entity).is_some_and(|t| {
                    let upright = (t.rotation * Vec3::Y).y > 0.9;
                    upright && t.position.y > b.0 - BLOCK.y * 2.0
                })
            })
            .count()
    };
    game.down = COLUMNS * FLOORS - standing;
}

fn draw_rope(world: &mut World, game: &Game) {
    let ts = world.query::<&Transform>();
    let (Some(a), Some(b)) = (ts.get(game.hook).map(|t| t.position), ts.get(game.ball).map(|t| t.position)) else { return };
    drop(ts);
    if let Some(mut g) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
        g.draw_line(a, b, [0.15, 0.15, 0.15, 1.0]);
    }
}

fn hud(world: &mut World, game: &Game) {
    let total = COLUMNS * FLOORS;
    let left = (TIME - game.clock).max(0.0);
    set_text(
        world,
        game.hud,
        &format!(
            "yıkılan {}/{total} (hedef {}), {} parçalandı\nsüre {left:.0} sn\nA/D: kancayı sür   R: baştan",
            game.down,
            (GOAL * total as f32).ceil(),
            game.broken
        ),
    );
    let banner = match game.over {
        Some(true) => "Bina yıkıldı!\nR ile yeniden",
        Some(false) => "Süre bitti.\nR ile yeniden",
        None => "",
    };
    set_text(world, game.banner, banner);
}

fn set_text(world: &mut World, id: Entity, s: &str) {
    if let Some(mut t) = world.query_mut::<&mut Text>().get_mut(id) {
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
