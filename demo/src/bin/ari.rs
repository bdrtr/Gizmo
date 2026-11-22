//! # Arı — davranış ağacıyla nektar taşıyan arılar, ve bir eşek arısı
//!
//! Beş arı kovandan çıkar, çevredeki sekiz çiçekten nektar toplar ve kovana taşır. Kovanın çevresinde
//! bir eşek arısı döner; yaklaşınca arılar kaçar, değerse sokar (arı yükünü düşürür ve kovana döner).
//! Bütün çiçekler boşalıp nektarın hepsi kovana varınca oyun biter.
//!
//! Motorun hangi yollarını yürüttüğü — oyunların hiçbirinin dokunmadığı **davranış ağacı** ve
//! **fayda yapay zekâsı**:
//!
//! - Her arı bir `BehaviorTree` bileşeni taşır; yapraklar bir `BtLibrary`'de adlandırılır, ağaç
//!   `ai_frame` ile her karede bir kez tick edilir.
//! - Kök bir **tepkili seçici** (`BtNode::reactive_selector`): her karede en öncelikli daldan
//!   (kaç) başlayarak yeniden sorulur; kaçış başlayınca çalışan toplama dalı kesilir. Bu oyunla
//!   eklendi — ağacın yalnız bellekli bileşkeleri vardı, ve bellekli bir seçici çalışan dalı
//!   bitmeden öncelikli dallara hiç bakmaz: aynı ağaç belleklisiyle kurulunca hiçbir arı bir kez
//!   bile kaçmaz, dört sokma olur ve düşen nektar yüzünden oyun iki dakikada bitmez (13 / 16).
//! - Çiçek seçimi bir `UtilityAction` ile: kalan nektar, uzaklık ve aynı çiçeğe giden başka arı
//!   sayısı, eğrilerle puanlanır; nektarı bitmiş çiçek veto edilir.
//!
//! `ari --test`: pencere açmadan oynar; nektarın hepsi kovana varmalı, hiçbir arı sokulmamalı ve
//! her arı en az bir kez kaçmış olmalı (yoksa tepkili dal hiç sınanmamıştır). Sonra aynı oyun
//! bellekli kökle oynanır ve **sokulmalı**: sokmayı önleyenin tepkili seçici olduğunun, senaryonun
//! onu gerçekten sınadığının kanıtı.
use gizmo::ai::utility_ai::{LinearCurve, UtilityAction, UtilityConsideration};
use gizmo::ai::{BehaviorTree, BtLibrary, BtNode, BtStatus};
use gizmo::core::input::Input;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, SimpleAppExt, SimpleSceneState};
use std::sync::Arc;

const BEES: usize = 5;
const FLOWERS: usize = 8;
const NECTAR_PER_FLOWER: u32 = 2;
const FLOWER_RING: f32 = 9.0;
const HIVE_R: f32 = 1.2;
const BEE_SPEED: f32 = 4.0;
const HORNET_RING: f32 = 5.5;
const HORNET_SPEED: f32 = 4.0;
/// Eşek arısı bu kadar yakındaysa arı kaçar; kaçış, bu uzaklığın ötesine geçince biter.
const DANGER: f32 = 3.5;
const SAFE: f32 = 5.0;
const STING: f32 = 1.2;
const GATHER_SECS: f32 = 0.6;

#[derive(Clone, Copy, Default)]
struct Bee {
    load: u32,
    target: Option<Entity>,
    stung: u32,
    /// Kaçış dalı kaç kez başladı (bir kaçış, art arda karelerde tek sayılır).
    flights: u32,
    fleeing: bool,
}
gizmo::core::impl_component!(Bee);

#[derive(Clone, Copy)]
struct Flower {
    nectar: u32,
}
gizmo::core::impl_component!(Flower);

#[derive(Clone, Copy)]
struct Hornet {
    angle: f32,
}
gizmo::core::impl_component!(Hornet);

struct Game {
    delivered: u32,
    hud: Entity,
}

fn position(world: &World, e: Entity) -> Vec3 {
    world.query::<&Transform>().get(e).map(|t| t.position).unwrap_or(Vec3::ZERO)
}

fn hornet_position(world: &World) -> Option<Vec3> {
    world.query::<(&Transform, &Hornet)>().iter().next().map(|(t, _)| t.position)
}

/// `e`'yi `goal`'a doğru bir karelik yol yürütür; vardıysa `true`.
fn fly_towards(world: &mut World, e: Entity, goal: Vec3, dt: f32) -> bool {
    let mut transforms = world.query_mut::<&mut Transform>();
    let Some(mut t) = transforms.get_mut(e) else { return false };
    let to = Vec3::new(goal.x - t.position.x, 0.0, goal.z - t.position.z);
    let step = BEE_SPEED * dt;
    if to.length() <= step {
        t.position.x = goal.x;
        t.position.z = goal.z;
        true
    } else {
        t.position += to.normalize() * step;
        false
    }
}

fn bee(world: &World, e: Entity) -> Bee {
    world.query::<&Bee>().get(e).copied().unwrap_or_default()
}

fn edit_bee(world: &mut World, e: Entity, f: impl FnOnce(&mut Bee)) {
    if let Some(mut b) = world.query_mut::<&mut Bee>().get_mut(e) {
        f(&mut b);
    }
}

/// Bir çiçeğin bir arı için puanı: kalan nektar, yakınlık, kalabalık.
struct FlowerContext {
    nectar: f32,
    closeness: f32,
    crowd: f32,
}

fn flower_score() -> UtilityAction<FlowerContext> {
    UtilityAction::new("cicek", 1.0)
        // Nektarı bitmiş çiçek 0 alır: veto.
        .add_consideration(UtilityConsideration::new(Arc::new(|c: &FlowerContext| c.nectar), Box::new(LinearCurve::new(1.0, 0.0)), 1.0))
        .add_consideration(UtilityConsideration::new(Arc::new(|c: &FlowerContext| c.closeness), Box::new(LinearCurve::new(0.8, 0.2)), 1.0))
        .add_consideration(UtilityConsideration::new(Arc::new(|c: &FlowerContext| 1.0 - c.crowd), Box::new(LinearCurve::new(0.9, 0.1)), 1.0))
}

fn library() -> BtLibrary {
    let score = Arc::new(flower_score());
    BtLibrary::new()
        .with_condition("tehlike", |e, world| {
            let p = position(world, e);
            let outside = Vec2::new(p.x, p.z).length() > HIVE_R;
            outside && hornet_position(world).is_some_and(|h| h.distance(p) < DANGER)
        })
        .with_action("kac", |e, world, dt| {
            let p = position(world, e);
            let Some(h) = hornet_position(world) else { return BtStatus::Success };
            if !bee(world, e).fleeing {
                edit_bee(world, e, |b| {
                    b.fleeing = true;
                    b.flights += 1;
                });
            }
            if h.distance(p) > SAFE {
                edit_bee(world, e, |b| b.fleeing = false);
                return BtStatus::Success;
            }
            let away = Vec3::new(p.x - h.x, 0.0, p.z - h.z).normalize_or(Vec3::X);
            fly_towards(world, e, p + away * 10.0, dt);
            BtStatus::Running
        })
        .with_condition("dolu", |e, world| bee(world, e).load > 0)
        .with_action("kovana_git", |e, world, dt| {
            if fly_towards(world, e, Vec3::new(0.0, 0.5, 0.0), dt) { BtStatus::Success } else { BtStatus::Running }
        })
        .with_action("bosalt", |e, world, _| {
            let load = bee(world, e).load;
            edit_bee(world, e, |b| b.load = 0);
            if let Some(mut g) = world.get_resource_mut::<Game>() {
                g.delivered += load;
            }
            BtStatus::Success
        })
        .with_action("cicek_sec", move |e, world, _| {
            let p = position(world, e);
            let claims: Vec<Entity> = world.query::<(Entity, &Bee)>().iter().filter(|(other, _)| *other != e).filter_map(|(_, b)| b.target).collect();
            let flowers: Vec<(Entity, Vec3, u32)> = world.query::<(Entity, &Transform, &Flower)>().iter().map(|(f, t, fl)| (f, t.position, fl.nectar)).collect();
            let best = flowers
                .into_iter()
                .map(|(f, at, nectar)| {
                    let context = FlowerContext {
                        nectar: nectar as f32 / NECTAR_PER_FLOWER as f32,
                        closeness: 1.0 - at.distance(p) / (2.0 * FLOWER_RING),
                        crowd: claims.iter().filter(|c| **c == f).count() as f32 / BEES as f32,
                    };
                    (f, score.evaluate(&context))
                })
                .filter(|(_, s)| *s > 0.0)
                .max_by(|a, b| a.1.total_cmp(&b.1));
            let target = best.map(|(f, _)| f);
            edit_bee(world, e, |b| b.target = target);
            if target.is_some() { BtStatus::Success } else { BtStatus::Failure }
        })
        .with_action("cicege_git", |e, world, dt| {
            let Some(f) = bee(world, e).target else { return BtStatus::Failure };
            let goal = position(world, f);
            if fly_towards(world, e, goal, dt) { BtStatus::Success } else { BtStatus::Running }
        })
        .with_action("topla", |e, world, _| {
            let Some(f) = bee(world, e).target else { return BtStatus::Failure };
            let took = match world.query_mut::<&mut Flower>().get_mut(f) {
                Some(mut flower) if flower.nectar > 0 => {
                    flower.nectar -= 1;
                    true
                }
                _ => false,
            };
            edit_bee(world, e, |b| {
                b.target = None;
                if took {
                    b.load = 1;
                }
            });
            if took { BtStatus::Success } else { BtStatus::Failure }
        })
        .with_action("dinlen", |e, world, dt| {
            fly_towards(world, e, Vec3::new(0.0, 0.5, 0.0), dt);
            BtStatus::Running
        })
}

/// Bir arının ağacı. `reactive`: kök tepkili mi (oyun), yoksa bellekli mi (karşılaştırma).
fn bee_tree(reactive: bool) -> BtNode {
    let children = [
        BtNode::sequence([BtNode::condition("tehlike"), BtNode::action("kac")]),
        BtNode::sequence([BtNode::condition("dolu"), BtNode::action("kovana_git"), BtNode::action("bosalt")]),
        BtNode::sequence([BtNode::action("cicek_sec"), BtNode::action("cicege_git"), BtNode::wait(GATHER_SECS), BtNode::action("topla")]),
        BtNode::action("dinlen"),
    ];
    if reactive { BtNode::reactive_selector(children) } else { BtNode::selector(children) }
}

fn material(r: f32, g: f32, b: f32) -> MaterialDesc {
    MaterialDesc::new().with_pbr(Vec4::new(r, g, b, 1.0), 0.6, 0.0)
}

fn setup(world: &mut World, font: FontId, reactive: bool) {
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-1.0)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.92), 2.5, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 22.0, 12.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -1.05,
        primary: true,
        ..Default::default()
    });
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.1, 0.0)).with_scale(Vec3::new(12.0, 0.1, 12.0)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        material(0.3, 0.55, 0.25),
        MeshRenderer::new(),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 0.3, 0.0)).with_scale(Vec3::new(HIVE_R, 0.3, HIVE_R)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        material(0.85, 0.65, 0.2),
        MeshRenderer::new(),
    ));
    for i in 0..FLOWERS {
        let a = i as f32 / FLOWERS as f32 * std::f32::consts::TAU;
        world.spawn_bundle((
            Transform::new(Vec3::new(a.cos() * FLOWER_RING, 0.3, a.sin() * FLOWER_RING)).with_scale(Vec3::splat(0.4)),
            GlobalTransform::default(),
            MeshSource("sphere".into()),
            material(0.9, 0.3, 0.6),
            MeshRenderer::new(),
            Flower { nectar: NECTAR_PER_FLOWER },
        ));
    }
    for i in 0..BEES {
        let a = i as f32 / BEES as f32 * std::f32::consts::TAU;
        world.spawn_bundle((
            Transform::new(Vec3::new(a.cos() * 0.5, 0.5, a.sin() * 0.5)).with_scale(Vec3::splat(0.2)),
            GlobalTransform::default(),
            MeshSource("sphere".into()),
            material(0.95, 0.85, 0.1),
            MeshRenderer::new(),
            Bee::default(),
            BehaviorTree::new(bee_tree(reactive)),
        ));
    }
    world.spawn_bundle((
        Transform::new(Vec3::new(HORNET_RING, 0.5, 0.0)).with_scale(Vec3::splat(0.3)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        material(0.15, 0.1, 0.05),
        MeshRenderer::new(),
        Hornet { angle: 0.0 },
    ));
    world.insert_resource(library());
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    world.insert_resource(Game { delivered: 0, hud });
}

fn total_nectar() -> u32 {
    FLOWERS as u32 * NECTAR_PER_FLOWER
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, _input: &Input) {
    // Eşek arısı halkasında döner.
    let hornet = {
        let mut q = world.query_mut::<(&mut Transform, &mut Hornet)>();
        let mut at = None;
        for (mut t, mut h) in q.iter_mut() {
            h.angle += HORNET_SPEED / HORNET_RING * dt;
            t.position = Vec3::new(h.angle.cos() * HORNET_RING, 0.5, h.angle.sin() * HORNET_RING);
            at = Some(t.position);
        }
        at
    };
    gizmo::ai::system::ai_frame(world, dt);
    // Sokma: kovanın dışında, eşek arısına değen arı yükünü düşürür ve kovana döner.
    if let Some(h) = hornet {
        let stung: Vec<Entity> = world
            .query::<(Entity, &Transform, &Bee)>()
            .iter()
            .filter(|(_, t, _)| Vec2::new(t.position.x, t.position.z).length() > HIVE_R && t.position.distance(h) < STING)
            .map(|(e, _, _)| e)
            .collect();
        for e in stung {
            edit_bee(world, e, |b| {
                b.stung += 1;
                b.load = 0;
                b.target = None;
                b.fleeing = false;
            });
            if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(e) {
                t.position = Vec3::new(0.0, 0.5, 0.0);
            }
            if let Some(mut tree) = world.query_mut::<&mut BehaviorTree>().get_mut(e) {
                if let Some(root) = tree.root.as_mut() {
                    root.reset();
                }
            }
        }
    }
    let delivered = world.get_resource::<Game>().map_or(0, |g| g.delivered);
    let stings: u32 = world.query::<&Bee>().iter().map(|b| b.stung).sum();
    if let Some(hud) = world.get_resource::<Game>().map(|g| g.hud) {
        if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(hud) {
            text.content = format!("Kovanda nektar: {delivered} / {}   Sokma: {stings}", total_nectar());
        }
    }
}

/// Bir oyun, en çok iki dakika: (teslim edilen nektar, sokma, arı başına kaçış, süre).
fn play(reactive: bool) -> (u32, u32, Vec<u32>, f32) {
    let mut world = World::new();
    setup(&mut world, HeadlessFrames::font(), reactive);
    let mut run = HeadlessFrames::new(world);
    while run.elapsed < 120.0 {
        run.frame(1.0 / 60.0, update);
        if run.world.get_resource::<Game>().is_some_and(|g| g.delivered == total_nectar()) {
            break;
        }
    }
    let delivered = run.world.get_resource::<Game>().map_or(0, |g| g.delivered);
    let bees: Vec<Bee> = run.world.query::<&Bee>().iter().copied().collect();
    (delivered, bees.iter().map(|b| b.stung).sum(), bees.iter().map(|b| b.flights).collect(), run.elapsed)
}

/// `--test`: bkz. modül belgesi.
fn self_test() -> bool {
    let (delivered, stings, flights, secs) = play(true);
    let (m_delivered, m_stings, m_flights, m_secs) = play(false);
    let ok = delivered == total_nectar() && stings == 0 && flights.iter().all(|f| *f > 0) && m_stings > 0;
    println!(
        "ari --test: tepkili kök {delivered} / {} nektar kovanda, {stings} sokma, arı başına kaçış {flights:?}, {secs:.0} s; \
         bellekli kök {m_delivered} / {} nektar, {m_stings} sokma, kaçış {m_flights:?}, {m_secs:.0} s → {}",
        total_nectar(),
        total_nectar(),
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Arı", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            setup(scene.world, font, true);
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
