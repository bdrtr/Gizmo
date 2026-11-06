//! # Sarmaşık — iplerden ipe sallanarak uçurumu geç
//!
//! Uçurumun üstünde ilk ipte asılı başlarsın; karşı kayalığa tavandan sarkan iplerle geçilir:
//! sallan, bırak, uçarken sonrakini yakala. Düşen başa döner. Motorun otuz altıncı oyunu: **oyun
//! sırasında takılıp sökülen ip eklemleri** (`Joint::rope`).
//!
//! | girdi | iş |
//! |---|---|
//! | **A / D** (veya ← / →) | ipteyken sallan (hareket yönüne it); kayalıkta yuvarlan |
//! | **Space** | ipteysen bırak; havadaysan uzandığın ipi yakala |
//! | **R** | baştan |
//!
//! `sarmasik --test` pencere açmaz: otomatik pilot sallanıp bırakarak karşıya geçer; kaç kez
//! tutunduğu, ipin boyundan en çok ne kadar uzadığı, kaç kez düştüğü yazılır.
//! (`SARMASIK_OTO=1` pencereli oyunda da otomatik pilotu açar.)
//!
//! Bulduğu eksik: düşen oyuncu silinip yenisi doğunca yeni gövde eskisinin numarasını alır, ve
//! fizik gövdeleri numarayla tanıdığı için eskisinin ip eklemi yeni gövdeye geçiyordu (uzakta
//! doğan bir top bir saniyede 5 m çekildi). **Düzeltildi:** ECS köprüsü numaranın kuşağını da
//! izler; kuşağı değişen numara yeni bir gövdedir, eskisi eklemleriyle birlikte silinir.

use gizmo::core::input::{Input, MoveKeys};
use gizmo::physics::components::ExternalForce;
use gizmo::physics::joints::Joint;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{BodyHandle, Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::simple::{set_text, sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

const G: f32 = 9.81;
const ROPE: f32 = 4.2;
/// Tavandaki tutunma noktaları; ipler buradan sarkar.
const ANCHORS: [(f32, f32); 4] = [(1.6, 6.0), (6.8, 6.4), (12.2, 6.0), (17.4, 6.6)];
/// Uzanılabilen mesafe: ipin boyu ve biraz el payı.
const REACH: f32 = ROPE + 0.3;
const PLAYER_R: f32 = 0.3;
const MASS: f32 = 50.0;
/// İpteyken sallanma kuvveti (ağırlığın altıda biri).
const PUMP: f32 = MASS * G / 6.0;
/// Kayalıklar: (sol x, sağ x, üst y).
const START: (f32, f32, f32) = (-8.0, -3.2, 2.0);
const GOAL: (f32, f32, f32) = (21.6, 27.0, 2.0);
/// İlk ipin ucunda, uçurumun üstünde asılı başlar (kayalık salınımın erişiminin dışında: yakın
/// bir kayalık her geri salınımda topu duvarından sektiriyordu).
const SPAWN: Vec3 = Vec3::new(ANCHORS[0].0, ANCHORS[0].1 - ROPE, 0.0);
const FALL_Y: f32 = -3.0;

fn spawn_rock(world: &mut World, (x0, x1, top): (f32, f32, f32), visual: bool) {
    let half = Vec3::new((x1 - x0) * 0.5, 2.5, 1.0);
    let id = world
        .spawn_bundle((Transform::new(Vec3::new((x0 + x1) * 0.5, top - half.y, 0.0)), RigidBody::new_static(), Velocity::default(), Collider::box_collider(half)))
        ;
    if visual {
        add_visual(world, id, MeshSource("standard_cube".into()), half, "#7E5109");
    }
}

fn add_visual(world: &mut World, id: Entity, mesh: MeshSource, scale: Vec3, colour: &str) {
    if let Some(e) = world.live(id) {
        world.add_component(e, GlobalTransform::default());
        world.add_component(e, mesh);
        world.add_component(e, MaterialDesc::new().with_pbr(Color::srgb_hex(colour).to_vec4(), 0.7, 0.0));
        world.add_component(e, MeshRenderer::new());
        if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(id) {
            t.scale = scale;
        }
    }
}

fn anchor_pos(k: usize) -> Vec3 {
    Vec3::new(ANCHORS[k].0, ANCHORS[k].1, 0.0)
}

struct Course {
    physics: FixedPhysics,
    player: Entity,
    anchors: Vec<Entity>,
    /// Tutunulan ipin sırası; `None` havada ya da kayalıkta.
    holding: Option<usize>,
    /// Tutulan ipin boyu.
    length: f32,
    grabs: u32,
    falls: u32,
    worst_stretch: f32,
    won: bool,
}

impl Course {
    fn new(world: &mut World, visual: bool) -> Self {
        world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0)));
        spawn_rock(world, START, visual);
        spawn_rock(world, GOAL, visual);
        let anchors = (0..ANCHORS.len())
            .map(|k| {
                let id = world.spawn_bundle((Transform::new(anchor_pos(k)), RigidBody::new_static(), Velocity::default(), Collider::sphere(0.08)));
                if visual {
                    add_visual(world, id, MeshSource("sphere".into()), Vec3::splat(0.08), "#27AE60");
                }
                id
            })
            .collect();
        let player = spawn_player(world, visual);
        let mut course = Self { physics: FixedPhysics::new(120), player, anchors, holding: None, length: ROPE, grabs: 0, falls: 0, worst_stretch: 0.0, won: false };
        // Başta ilk ipte asılı.
        course.grab(world, 0);
        course.grabs = 0;
        course
    }

    fn grab(&mut self, world: &mut World, k: usize) {
        // Uzanarak tutulan ip o anki boyuyla takılır: daha kısa bir ip, eklemin farkı bir adımda
        // kapatmasıyla oyuncuyu ipin ucuna fırlatırdı (0.2 m'lik fark 3 m/s'lik bir sıçrama).
        let length = anchor_pos(k).distance(position(world, self.player)).max(ROPE);
        if let Some(mut pw) = world.get_resource_mut::<PhysicsWorld>() {
            pw.joints.push(Joint::rope(BodyHandle::from_id(self.anchors[k].id()), BodyHandle::from_id(self.player.id()), Vec3::ZERO, Vec3::ZERO, length));
        }
        self.length = length;
        self.holding = Some(k);
        self.grabs += 1;
    }

    fn release(&mut self, world: &mut World) {
        let player = BodyHandle::from_id(self.player.id());
        if let Some(mut pw) = world.get_resource_mut::<PhysicsWorld>() {
            pw.joints.retain(|j| j.entity_b != player);
        }
        self.holding = None;
    }

    /// Uzanılabilen en yakın ip (tutulandan başka).
    fn in_reach(&self, world: &World) -> Option<usize> {
        let p = position(world, self.player);
        (0..ANCHORS.len())
            .filter(|&k| Some(k) != self.holding)
            .map(|k| (k, anchor_pos(k).distance(p)))
            .filter(|&(_, d)| d < REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(k, _)| k)
    }

    /// Bir kare: `push` −1/0/+1 yön, `space` bırak/yakala.
    fn frame(&mut self, world: &mut World, dt: f32, push: f32, space: bool, visual: bool) {
        if space {
            if self.holding.is_some() {
                self.release(world);
            } else if let Some(k) = self.in_reach(world) {
                self.grab(world, k);
            }
        }
        let (p, _) = state(world, self.player);
        let force = match self.holding {
            // İpe dik yönde, ipin düzleminde it: sallanmayı büyütür.
            Some(k) => {
                let rope = (p - anchor_pos(k)).normalize_or_zero();
                let tangent = Vec3::new(-rope.y, rope.x, 0.0);
                let tangent = if tangent.x < 0.0 { -tangent } else { tangent };
                tangent * push * PUMP
            }
            None => Vec3::X * push * PUMP * 0.3,
        };
        if let Some(mut f) = world.borrow_mut::<ExternalForce>().get_mut(self.player) {
            *f = ExternalForce::new(force);
        }
        self.physics.advance(world, dt);
        world.apply_commands();
        let (p, _) = state(world, self.player);
        if let Some(k) = self.holding {
            self.worst_stretch = self.worst_stretch.max(anchor_pos(k).distance(p) - self.length);
        }
        if p.x > GOAL.0 + PLAYER_R && p.y > GOAL.2 && p.y < GOAL.2 + 0.5 && self.holding.is_none() {
            self.won = true;
        }
        if p.y < FALL_Y || !p.is_finite() {
            self.falls += 1;
            self.restart(world, visual);
        }
    }

    fn restart(&mut self, world: &mut World, visual: bool) {
        self.release(world);
        if let Some(e) = world.live(self.player) {
            world.despawn(e);
        }
        world.apply_commands();
        self.player = spawn_player(world, visual);
        self.grab(world, 0);
        self.won = false;
    }
}

fn spawn_player(world: &mut World, visual: bool) -> Entity {
    let col = Collider::sphere(PLAYER_R);
    let mut rb = RigidBody::new(MASS, true);
    rb.update_inertia_from_collider(&col);
    rb.lock_translation_z = true;
    let id = world.spawn_bundle((Transform::new(SPAWN), rb, Velocity::default(), col, ExternalForce::default()));
    if visual {
        add_visual(world, id, MeshSource("sphere".into()), Vec3::splat(PLAYER_R), "#E67E22");
    }
    id
}

fn position(world: &World, id: Entity) -> Vec3 {
    world.borrow::<Transform>().get(id).map(|t| t.position).unwrap_or(Vec3::ZERO)
}

fn state(world: &World, id: Entity) -> (Vec3, Vec3) {
    (position(world, id), world.borrow::<Velocity>().get(id).map(|v| v.linear).unwrap_or(Vec3::ZERO))
}

/// Şu an bırakılırsa top nereye düşer: sonraki ipin erişimine ya da hedef kayalığa varıyor mu.
fn release_lands(p: Vec3, v: Vec3, next: Option<usize>) -> bool {
    let mut t = 0.0;
    while t < 2.5 {
        t += 0.01;
        let q = p + v * t + Vec3::new(0.0, -0.5 * G * t * t, 0.0);
        match next {
            // İpi kaçırmamak için erişimin içine iyice girmeli.
            Some(k) => {
                if q.distance(anchor_pos(k)) < ROPE - 0.2 && q.y < anchor_pos(k).y - 1.0 {
                    return true;
                }
            }
            None => {
                if q.y < GOAL.2 + PLAYER_R + 0.2 {
                    return q.x > GOAL.0 + 0.8 && q.x < GOAL.1 - 1.0;
                }
            }
        }
        if q.y < FALL_Y {
            return false;
        }
    }
    false
}

/// Otomatik pilot: hareket yönüne iterek sallan; bırakılınca sonraki ipe (ya da hedefe) varılacaksa
/// bırak; uçarken sonraki ip erişime girince yakala.
#[derive(Default)]
struct Autopilot {
    /// Bırakılan ipin sırası: aynı ipi uçarken yeniden yakalamasın.
    last: Option<usize>,
}

impl Autopilot {
    fn press(&mut self, world: &World, course: &Course) -> (f32, bool) {
        let (p, v) = state(world, course.player);
        match course.holding {
            Some(k) => {
                self.last = Some(k);
                let next = (k + 1 < ANCHORS.len()).then_some(k + 1);
                // Yalnız ileri ve yukarı giderken bırak.
                if v.x > 1.0 && v.y > -0.5 && release_lands(p, v, next) {
                    return (0.0, true);
                }
                let along = if v.x.abs() < 0.05 { 1.0 } else { v.x.signum() };
                (along, false)
            }
            None => {
                let want = self.last.map_or(0, |k| k + 1);
                let grab = want < ANCHORS.len() && course.in_reach(world) == Some(want);
                (0.0, grab)
            }
        }
    }
}

/// `--test`: bir dakika otomatik pilot.
fn self_test() -> bool {
    let mut world = World::new();
    let mut course = Course::new(&mut world, false);
    let mut auto = Autopilot::default();
    let dt = 1.0 / 60.0;
    let mut time = 0.0f32;
    while time < 60.0 && !course.won {
        time += dt;
        let (push, space) = auto.press(&world, &course);
        let falls = course.falls;
        course.frame(&mut world, dt, push, space, false);
        if course.falls != falls {
            auto = Autopilot::default();
        }
    }
    let ok = course.won && course.worst_stretch < 0.05;
    println!(
        "sarmasik --test: {:.1} s; karşıya geçti mi: {}; {} kez tutundu, {} kez düştü; ip boyundan en çok {:.3} m uzadı → {}",
        time,
        if course.won { "evet" } else { "HAYIR" },
        course.grabs,
        course.falls,
        course.worst_stretch,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

struct Game {
    course: Course,
    auto: Autopilot,
    rope_vis: Entity,
    hud: Entity,
    banner: Entity,
    camera: Entity,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Sarmaşık", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = scene.renderer.default_font();
            let course = Course::new(scene.world, true);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.9)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.95, 0.85), 2.2, LightRole::Sun),
            ));
            let camera = scene
                .world
                .spawn_bundle(CameraBundle { position: Vec3::new(0.0, 3.5, 13.0), yaw: -std::f32::consts::FRAC_PI_2, pitch: -0.08, primary: true, ..Default::default() })
                ;
            let rope_vis = scene.world.spawn_bundle((Transform::new(Vec3::ZERO),));
            add_visual(scene.world, rope_vis, MeshSource("standard_cube".into()), Vec3::ZERO, "#A0522D");
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                ;
            scene.world.insert_resource(Game { course, auto: Autopilot::default(), rope_vis, hud, banner, camera });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.03);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        game.course.restart(world, true);
        game.course.falls = 0;
        game.course.grabs = 0;
        game.auto = Autopilot::default();
    }
    let (push, space) = if std::env::var_os("SARMASIK_OTO").is_some() {
        let Game { auto, course, .. } = &mut game;
        auto.press(world, course)
    } else {
        let push = input.move_axis().0 + input.move_axis_with(MoveKeys::ARROWS).0;
        (push.clamp(-1.0, 1.0), input.is_key_just_pressed(KeyCode::Space as u32))
    };
    if !game.course.won {
        game.course.frame(world, dt, push, space, true);
    }
    let p = position(world, game.course.player);
    // İp: tutunulan noktadan oyuncuya gerili ince bir kutu; havadayken görünmez.
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.rope_vis) {
        match game.course.holding {
            Some(k) => {
                let a = anchor_pos(k);
                let seg = p - a;
                t.position = a + seg * 0.5;
                t.rotation = Quat::from_rotation_arc(Vec3::Y, seg.normalize_or(Vec3::NEG_Y));
                t.scale = Vec3::new(0.03, seg.length() * 0.5, 0.03);
            }
            None => t.scale = Vec3::ZERO,
        }
    }
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.camera) {
        t.position.x += (p.x - t.position.x) * (dt * 3.0).min(1.0);
    }
    sync_transforms(world);
    set_text(
        world,
        game.hud,
        &format!(
            "düşüş {}   tutunma {}\nA/D: sallan   Space: bırak / yakala   R: baştan{}",
            game.course.falls,
            game.course.grabs,
            if game.course.holding.is_none() && game.course.in_reach(world).is_some() { "\n(uzanabileceğin bir ip var)" } else { "" }
        ),
    );
    set_text(world, game.banner, if game.course.won { "Karşıya geçtin!\nR ile yeniden" } else { "" });
    world.insert_resource(game);
}
