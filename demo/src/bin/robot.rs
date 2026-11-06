//! # Robot — bir kolla topları deliğe it
//!
//! Masada iki kemikli bir robot kolu, beş top ve bir hedef alan. Kolun ucu imleci izler — omuz
//! ve dirsek açıları **ters kinematikle** (IK) bulunur — ve kolun parçaları topları iter. Beş topu
//! süre dolmadan hedef alana sok. Motorun otuz dördüncü oyunu ve IK'nın kullanıldığı ilki.
//!
//! | girdi | iş |
//! |---|---|
//! | **fare** · **WASD** | kolun ucu |
//! | **sol tık** · **Space** | ucu indir (bırakınca kalkar) |
//! | **R** | baştan |
//!
//! `robot --test` pencere açmaz: otomatik pilot her topun arkasının üstüne gider, iner, topu
//! hedefe iter, kalkar; doksan saniyede kaç topun girdiği ve kolun ucunun istenen yerden en çok ne
//! kadar saptığı yazılır. (`ROBOT_OTO=1` pencereli oyunda da otomatik pilotu açar.)
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! Yok. `solve_two_bone_ik` motorda vardı ama hiçbir oyun ya da sistem kullanmıyordu; ilk
//! kullanıldığı yerde, `KinematicTarget` ile sürülen iki kapsülle, kolun ucu istenen yerden hiç
//! sapmadı (0,000 m) ve beş top 24 saniyede hedefte. Yolda öğrenilen iki şey, ikisi de motorun
//! dediği gibi davranıyor: yuvarlanma sürtünmesi olmayan bir küre düz masada hiç durmuyor, ve iki
//! malzemenin sürtünmesi geometrik ortalamayla birleştiği için topa verilen yuvarlanma sürtünmesi
//! masanınki sıfırken sıfır — keçe ikisine de veriliyor. Kol da masa düzleminde gezince öteki
//! topları savuruyordu: dirsek yukarıda, uç yalnız iterken iniyor.

use gizmo::animation::ik::solve_two_bone_ik;
use gizmo::core::input::Input;
use gizmo::physics::components::{KinematicTarget, PhysicsMaterial};
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{MeshData, Text, TextAlign, TextAnchor};
use gizmo::simple::{set_text, sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

const TABLE: f32 = 3.0;
const UPPER: f32 = 1.4;
const LOWER: f32 = 1.2;
const ARM_RADIUS: f32 = 0.12;
const BALL: f32 = 0.18;
/// Kolun ucunun iterken yüksekliği: topların ortası hizası.
const ARM_Y: f32 = BALL;
/// Gezerken: topların üstünden geçecek kadar yukarıda.
const TRAVEL_Y: f32 = 0.75;
/// Omuz bir kaidenin tepesinde.
const ROOT: Vec3 = Vec3::new(0.0, 0.9, 0.0);
/// Hedef alanın ortası ve yarıçapı.
const GOAL: Vec3 = Vec3::new(1.6, 0.0, 1.5);
const GOAL_RADIUS: f32 = 0.6;
const BALLS: [(f32, f32); 5] = [(-1.2, 0.6), (0.8, -1.1), (-0.4, -1.4), (-1.3, -0.5), (1.4, -0.3)];
const TIME: f32 = 90.0;
/// Kolun dinlenme yeri.
const HOME: Vec3 = Vec3::new(-0.6, TRAVEL_Y, 1.4);

fn spawn_table(world: &mut World) {
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    // Keçe: iki malzemenin sürtünmesi geometrik ortalamayla birleşiyor, ve sıfırla ortalaması
    // sıfır — topun yuvarlanma sürtünmesi, masanınki de olmadan hiçbir şey yapmıyordu.
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(Vec3::new(TABLE, 0.5, TABLE)).with_material(PhysicsMaterial { rolling_friction: 0.08, ..Default::default() }),
    ));
    // Kenarlar: toplar masadan düşmesin.
    for (centre, half) in [
        (Vec3::new(TABLE + 0.1, 0.2, 0.0), Vec3::new(0.1, 0.2, TABLE + 0.2)),
        (Vec3::new(-TABLE - 0.1, 0.2, 0.0), Vec3::new(0.1, 0.2, TABLE + 0.2)),
        (Vec3::new(0.0, 0.2, TABLE + 0.1), Vec3::new(TABLE, 0.2, 0.1)),
        (Vec3::new(0.0, 0.2, -TABLE - 0.1), Vec3::new(TABLE, 0.2, 0.1)),
    ] {
        world.spawn_bundle((Transform::new(centre), RigidBody::new_static(), Velocity::default(), Collider::box_collider(half)));
    }
}

/// İki kol parçası — kinematik kapsüller, `KinematicTarget` ile sürülen.
fn spawn_arm(world: &mut World, visual: bool) -> [Entity; 2] {
    let pose = arm_pose(HOME);
    let mut ids = [Entity::new(0, 0); 2];
    for (k, (len, (pos, rot))) in [(UPPER, pose[0]), (LOWER, pose[1])].into_iter().enumerate() {
        let col = Collider::capsule(ARM_RADIUS, len * 0.5);
        let id = world
            .spawn_bundle((Transform::new(pos).with_rotation(rot), RigidBody::new_kinematic(), Velocity::default(), col, KinematicTarget::new(pos, rot)))
            ;
        if visual {
            if let Some(e) = world.live(id) {
                world.add_component(e, GlobalTransform::default());
                world.add_component(e, MeshData::capsule(ARM_RADIUS, len, 6, 12));
                world.add_component(e, MaterialDesc::new().with_pbr(Color::srgb_hex(if k == 0 { "#F39C12" } else { "#E67E22" }).to_vec4(), 0.4, 0.6));
                world.add_component(e, MeshRenderer::new());
            }
        }
        ids[k] = id;
    }
    ids
}

/// Kolun ucu `target`'a uzanırken iki parçanın (orta noktası, dönüşü). Dirsek yukarı bükülür:
/// kol topların üstünden geçer, yalnız ucu iner.
fn arm_pose(target: Vec3) -> [(Vec3, Quat); 2] {
    let pole = ROOT + Vec3::Y * 3.0;
    let ik = solve_two_bone_ik(ROOT, UPPER, LOWER, target, pole);
    let segment = |a: Vec3, b: Vec3| ((a + b) * 0.5, Quat::from_rotation_arc(Vec3::Y, (b - a).normalize_or(Vec3::Y)));
    [segment(ik.root, ik.mid), segment(ik.mid, ik.end)]
}

/// Kolun uzanabileceği yere çeker.
fn clamp_reach(tip: Vec3) -> Vec3 {
    // İki kemik kökten ne en çok UPPER + LOWER uzağa uzanır, ne de |UPPER − LOWER|'den yakına
    // katlanır: kökün tam üstünden geçen bir uç ikisinin arasındaki ölü bölgeye düşer (orada
    // 3 cm kaçırıyordu).
    let d = tip - ROOT;
    let len = d.length().clamp((UPPER - LOWER).abs() + 0.02, UPPER + LOWER - 0.02);
    ROOT + d.try_normalize().unwrap_or(Vec3::X) * len
}

fn drive_arm(world: &mut World, arm: [Entity; 2], target: Vec3) {
    let pose = arm_pose(target);
    for (k, &id) in arm.iter().enumerate() {
        if let Some(mut t) = world.query_mut::<&mut KinematicTarget>().get_mut(id) {
            *t = KinematicTarget::new(pose[k].0, pose[k].1);
        }
    }
}

fn spawn_balls(world: &mut World, visual: bool) -> Vec<Entity> {
    BALLS
        .iter()
        .map(|&(x, z)| {
            // Keçe kaplı masa: yuvarlanan top yavaşlar (yuvarlanma sürtünmesi olmadan bir küre
            // düz bir yüzeyde hiç durmuyordu — hedefi geçip masanın köşesine kadar gidiyordu).
            let col = Collider::sphere(BALL).with_material(PhysicsMaterial { rolling_friction: 0.08, ..Default::default() });
            let mut rb = RigidBody::new(0.6, true);
            rb.update_inertia_from_collider(&col);
            let id = world.spawn_bundle((Transform::new(Vec3::new(x, BALL, z)), rb, Velocity::default(), col));
            if visual {
                if let Some(e) = world.live(id) {
                    world.add_component(e, GlobalTransform::default());
                    world.add_component(e, MeshData::sphere(BALL, 12, 16));
                    world.add_component(e, MaterialDesc::new().with_pbr(Color::srgb_hex("#3498DB").to_vec4(), 0.3, 0.0));
                    world.add_component(e, MeshRenderer::new());
                }
            }
            id
        })
        .collect()
}

fn position(world: &World, id: Entity) -> Vec3 {
    world.query::<&Transform>().get(id).map(|t| t.position).unwrap_or(Vec3::ZERO)
}

fn in_goal(p: Vec3) -> bool {
    Vec2::new(p.x - GOAL.x, p.z - GOAL.z).length() < GOAL_RADIUS
}

/// Hedef bir delik: içine giren top orada kalır (kinematik olur, biraz batar).
fn capture(world: &mut World, balls: &[Entity]) {
    for &b in balls {
        let p = position(world, b);
        let sunk = world.query::<&RigidBody>().get(b).is_some_and(|rb| rb.is_kinematic());
        if in_goal(p) && !sunk {
            if let Some(mut rb) = world.query_mut::<&mut RigidBody>().get_mut(b) {
                *rb = RigidBody::new_kinematic();
            }
            if let Some(mut v) = world.query_mut::<&mut Velocity>().get_mut(b) {
                *v = Velocity::default();
            }
            if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(b) {
                t.position.y = -BALL * 0.6;
            }
        }
    }
}

/// Kolun ucu o noktaya (itme yüksekliğinde) uzanabiliyor mu.
fn reachable(p: Vec3) -> bool {
    (Vec3::new(p.x, ARM_Y, p.z) - ROOT).length() < UPPER + LOWER - 0.05
}

/// Otomatik pilot. `phase` 0: sıradaki topun hedefe göre arkasının üstüne git; 1: in; 2: topu
/// hedefe doğru it; 3: kalk.
fn autopilot(world: &World, balls: &[Entity], tip: Vec3, phase: &mut f32, dt: f32) -> Vec3 {
    let step = |from: Vec3, to: Vec3, speed: f32| from + (to - from).clamp_length_max(speed * dt);
    let behind_of = |p: Vec3| {
        let to_goal = Vec3::new(GOAL.x - p.x, 0.0, GOAL.z - p.z).normalize_or_zero();
        Vec3::new(p.x, ARM_Y, p.z) - to_goal * (BALL + ARM_RADIUS + 0.2)
    };
    let Some(&ball) = balls.iter().find(|&&b| !in_goal(position(world, b)) && reachable(behind_of(position(world, b)))) else {
        return step(tip, HOME, 2.0);
    };
    let p = position(world, ball);
    let to_goal = Vec3::new(GOAL.x - p.x, 0.0, GOAL.z - p.z).normalize_or_zero();
    let behind = behind_of(p);
    let above = Vec3::new(behind.x, TRAVEL_Y, behind.z);
    match *phase as i32 {
        0 => {
            if (tip - above).length() < 0.05 {
                *phase = 1.0;
            }
            step(tip, above, 2.5)
        }
        1 => {
            if (tip - behind).length() < 0.03 {
                *phase = 2.0;
            }
            step(tip, behind, 1.5)
        }
        2 => {
            let target = Vec3::new(GOAL.x, ARM_Y, GOAL.z) - to_goal * (BALL + ARM_RADIUS);
            if in_goal(p) || (tip - target).length() < 0.05 || (tip - p).length() > 0.9 {
                *phase = 3.0;
            }
            step(tip, target, 1.0)
        }
        _ => {
            if tip.y > TRAVEL_Y - 0.05 {
                *phase = 0.0;
            }
            step(tip, Vec3::new(tip.x, TRAVEL_Y, tip.z), 1.5)
        }
    }
}

/// `--test`: doksan saniye otomatik pilot.
fn self_test() -> bool {
    let mut world = World::new();
    spawn_table(&mut world);
    let arm = spawn_arm(&mut world, false);
    let balls = spawn_balls(&mut world, false);
    let mut physics = FixedPhysics::new(60);
    let dt = 1.0 / 60.0;
    let (mut tip, mut phase, mut time) = (HOME, 0.0f32, 0.0f32);
    let mut worst_reach_error = 0.0f32;
    while time < TIME {
        time += dt;
        tip = clamp_reach(autopilot(&world, &balls, tip, &mut phase, dt));
        drive_arm(&mut world, arm, tip);
        physics.advance(&mut world, dt);
        world.apply_commands();
        capture(&mut world, &balls);
        // Kolun ucu gerçekten istenen yerde mi (IK + kinematik sürüş).
        let lower = world.query::<&Transform>().get(arm[1]).copied().unwrap_or_default();
        let end = lower.position + lower.rotation * Vec3::Y * (LOWER * 0.5);
        worst_reach_error = worst_reach_error.max((end - tip).length());
        if balls.iter().all(|&b| in_goal(position(&world, b))) {
            break;
        }
    }
    let scored = balls.iter().filter(|&&b| in_goal(position(&world, b))).count();
    let ok = scored >= 3 && worst_reach_error < 0.05;
    println!(
        "robot --test: {scored}/{} top hedefte, {time:.0} s; kolun ucu istenen yerden en çok {worst_reach_error:.3} m saptı → {}",
        balls.len(),
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

struct Game {
    physics: FixedPhysics,
    arm: [Entity; 2],
    balls: Vec<Entity>,
    tip: Vec3,
    phase: f32,
    time: f32,
    done: Option<f32>,
    hud: Entity,
    banner: Entity,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Robot", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = scene.renderer.default_font();
            spawn_table(scene.world);
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.5, 0.0)).with_scale(Vec3::new(TABLE, 0.5, TABLE)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#7F8C8D").to_vec4(), 0.8, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(GOAL + Vec3::Y * 0.005).with_scale(Vec3::new(GOAL_RADIUS, 0.003, GOAL_RADIUS)),
                GlobalTransform::default(),
                MeshSource("cylinder".into()),
                MaterialDesc::new().with_unlit(Color::srgb_hex("#27AE60").to_vec4()),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(ROOT.x, ROOT.y * 0.5, ROOT.z)).with_scale(Vec3::new(0.25, ROOT.y * 0.5, 0.25)),
                GlobalTransform::default(),
                MeshSource("cylinder".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#2C3E50").to_vec4(), 0.5, 0.7),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-1.0)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 2.2, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle { position: Vec3::new(0.0, 7.5, 6.0), pitch: -0.9, primary: true, ..Default::default() });
            let arm = spawn_arm(scene.world, true);
            let balls = spawn_balls(scene.world, true);
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                ;
            scene.world.insert_resource(Game {
                physics: FixedPhysics::new(60),
                arm,
                balls,
                tip: HOME,
                phase: 0.0,
                time: 0.0,
                done: None,
                hud,
                banner,
            });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        for &b in &game.balls {
            if let Some(e) = world.live(b) {
                world.despawn(e);
            }
        }
        game.balls = spawn_balls(world, true);
        game.time = 0.0;
        game.done = None;
        game.physics.reset();
    }
    let auto = std::env::var_os("ROBOT_OTO").is_some();
    if auto {
        let (tip, mut phase) = (game.tip, game.phase);
        game.tip = autopilot(world, &game.balls, tip, &mut phase, dt);
        game.phase = phase;
    } else {
        // İmlecin masadaki yeri; fare yoksa WASD. Sol tık ya da Space basılıyken uç iner.
        let down = input.is_mouse_button_pressed(gizmo::core::input::mouse::LEFT) || input.is_key_pressed(KeyCode::Space as u32);
        let height = if down { ARM_Y } else { TRAVEL_Y };
        let on_table = gizmo::pointer::cursor_ray(world).and_then(|ray| {
            let t = (ARM_Y - ray.origin.y) / ray.direction.y;
            (t > 0.0).then(|| Vec3::from(ray.origin + ray.direction * t))
        });
        let (mx, my) = input.move_axis();
        let flat = match on_table {
            Some(p) if mx == 0.0 && my == 0.0 => p,
            _ => game.tip + Vec3::new(mx, 0.0, -my) * 2.5 * dt,
        };
        let want = Vec3::new(flat.x, height, flat.z);
        game.tip += (want - game.tip).clamp_length_max(3.0 * dt);
    }
    game.tip = clamp_reach(game.tip);
    drive_arm(world, game.arm, game.tip);
    game.physics.advance(world, dt);
    capture(world, &game.balls);
    sync_transforms(world);
    let scored = game.balls.iter().filter(|&&b| in_goal(position(world, b))).count();
    if game.done.is_none() {
        game.time += dt;
        if scored == game.balls.len() || game.time >= TIME {
            game.done = Some(game.time);
        }
    }
    for &b in &game.balls {
        let colour = if in_goal(position(world, b)) { "#27AE60" } else { "#3498DB" };
        if let Some(mut m) = world.query_mut::<&mut MaterialDesc>().get_mut(b) {
            m.albedo = Color::srgb_hex(colour).to_vec4();
        }
    }
    set_text(
        world,
        game.hud,
        &format!("hedefte {scored}/{}   süre {:.0} s\nfare / WASD: kolun ucu   sol tık / Space: in   R: baştan", game.balls.len(), (TIME - game.time).max(0.0)),
    );
    let banner = game.done.map(|t| if scored == game.balls.len() { format!("Hepsi içeride: {t:.0} s\nR ile yeniden") } else { format!("Süre bitti: {scored} top\nR ile yeniden") }).unwrap_or_default();
    set_text(world, game.banner, &banner);
    world.insert_resource(game);
}
