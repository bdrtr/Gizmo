//! # Köprü — iki uçurum arasına kirişlerden bir köprü
//!
//! Düğümlere tıklayarak kiriş çek, boşlukla dene: kirişler gerçek cisimlere, düğümler kırılabilir
//! eklemlere dönüşüyor ve ağır bir top köprüden geçmeye çalışıyor. Her kiriş yükü kadar
//! kızarıyor; eşiğini aşan eklem kopuyor. Bütçe sınırlı. Motorun on yedinci oyunu ve ilk
//! **eklemlerden yapılmış** yapısı.
//!
//! | girdi | iş |
//! |---|---|
//! | **sol tık** düğüm, sonra başka düğüm | kiriş çek |
//! | **sağ tık** | son kirişi sil |
//! | **boşluk** | dene / kurmaya dön |
//! | **R** | köprüyü sil |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Bir eklemin taşıdığı yük okunamıyordu.** Kirişi yüküne göre kızartmak için eklemin
//!   kuvveti gerekiyor; kırılma eşiğiyle karşılaştırılan sayı `pub(crate)`'ti. **Kapatıldı:**
//!   `Joint::last_force` / `last_torque` / `load()` (eşiğin kesri; 1 kopar).
//! - **…ve onu ölçünce: sabit, menteşe ve bilye eklemleri eşiklerinin yarısında kopuyordu.**
//!   Ortak nokta kısıtı, çapalar 0,1 mm'den yakınken doğrusal satırlarını atlıyordu; cisim bir
//!   adım düşüp sonra hem düşüşe hem sarkmaya karşı itiliyordu. 10 kg'lık asılı bir ağırlık
//!   98 N yerine 196 N gösteriyordu. **Düzeltildi:** satırlar her hatada çalışıyor.
//! - **Silinen bir cismin eklemi, numarasını alan yeni cisme bağlanıyordu.** Köprüyü söküp
//!   yeniden kurmak tam bunu yapıyor. **Düzeltildi:** eklemler cisimleriyle gidiyor.
//!
//! *Açık, ölçülmüş:* kafesin eklem kuvvetleri elle hesabın üstünde. Kendi ağırlığı altında
//! ~5 kN (üst kuşak için kabaca 2,6 kN beklenir), 1,2 kN'luk top ortaya gelince ~16 kN. Eklem
//! iterasyonu (10 → 40) ve sönüm değiştirmiyor. Ayrıldı: statik olarak belirli iki çubuklu bir
//! konsol, statiğin verdiği kuvvetlerin %3–6 içinde ölçülüyor (bkz. `joint_load.rs`), yani
//! ölçüm de belirli bir yapının çözümü de doğru; fazlalık bu kafesin dört sabit düğümünden
//! (aşırı belirli yapı). Oyun eşiği buna göre ayarlı (30 kN).
//!
//! *Sonradan bulundu (2026-11-04, `golf`'tan sonra):* kirişlerin yük altında kızarması hiç
//! görünmüyordu. Renk `MaterialDesc` ile değişiyordu ve motor her kare açıklamayı malzemeden
//! yeniden yazıyordu. **Düzeltildi:** düzenlenen açıklama malzemeye geçiyor.
//!
//! ## Ortam değişkenleri
//!   * `KOPRU_OTO=1` — hazır bir kafes köprüyü kurup dener (ekran görüntüsü için; `kopru --test`
//!     pencere açmadan onu, bir döşemeyi ve ortası eksik bir döşemeyi dener)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::core::input::Input;
use gizmo::physics::components::CollisionLayer;
use gizmo::physics::joints::Joint;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{BodyHandle, Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

/// Düğüm ızgarası: x −6…6, y −4…2, 2 m aralık. Uçurum kenarındaki düğümler sabit.
const NODE_X: [f32; 7] = [-6.0, -4.0, -2.0, 0.0, 2.0, 4.0, 6.0];
const NODE_Y: [f32; 4] = [-4.0, -2.0, 0.0, 2.0];
const MAX_BEAM: f32 = 3.0;
const BUDGET: u32 = 24;
const BEAM_MASS: f32 = 20.0;
const BEAM_HALF_THICK: f32 = 0.12;
const BREAK_FORCE: f32 = 30000.0;
const BALL_R: f32 = 0.5;
const BALL_MASS: f32 = 120.0;
/// Kirişler birbiriyle çarpışmaz (bir düğümde buluşanlar üst üste biner); top ve uçurumla çarpışır.
const BEAM_LAYER: u32 = 1;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Build,
    Test,
}

#[derive(Clone, Copy)]
struct Beam {
    a: usize,
    b: usize,
}

#[derive(Clone, Copy)]
struct BeamBody;
gizmo::core::impl_component!(BeamBody);

struct Game {
    physics: FixedPhysics,
    mode: Mode,
    beams: Vec<Beam>,
    /// Kurma: seçili ilk düğüm.
    picked: Option<usize>,
    /// Deneme: her kirişin cismi, ve her eklemin hangi kirişe ait olduğu.
    bodies: Vec<u32>,
    joint_beam: Vec<usize>,
    ball: u32,
    previews: Vec<u32>,
    nodes: Vec<u32>,
    hud: u32,
    banner: u32,
    clock: f32,
    result: Option<bool>,
    auto: bool,
}

fn nodes() -> Vec<Vec3> {
    NODE_Y.iter().flat_map(|&y| NODE_X.iter().map(move |&x| Vec3::new(x, y, 0.0))).collect()
}

fn anchored(p: Vec3) -> bool {
    p.x.abs() > 5.9 && p.y > -2.1
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
fn setup(world: &mut World, font: FontId) -> Game {

    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.8)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, -0.5, 22.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: 0.0,
        primary: true,
        ..Default::default()
    });
    // Uçurumlar: üstü y = 0, köprü boşluğu x −6…6.
    for x in [-11.0f32, 11.0] {
        let half = Vec3::new(5.0, 6.0, 2.0);
        let at = Vec3::new(x, -6.0, 0.0);
        world.spawn_bundle((
                Transform::new(at).with_scale(half),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Vec4::new(0.45, 0.38, 0.3, 1.0), 0.95, 0.0),
                MeshRenderer::new(),
            ));
        world.spawn_bundle((Transform::new(at), RigidBody::new_static(), Velocity::default(), Collider::box_collider(half)));
    }
    // Düğümler: küçük küreler; sabit olanlar koyu.
    let node_ids = nodes()
        .into_iter()
        .map(|p| {
            let colour = if anchored(p) { Vec4::new(0.2, 0.2, 0.25, 1.0) } else { Vec4::new(0.85, 0.85, 0.9, 1.0) };
            world
                .spawn_bundle((
                    Transform::new(p + Vec3::Z * 0.3).with_scale(Vec3::splat(0.18)),
                    GlobalTransform::default(),
                    MeshSource("sphere".into()),
                    MaterialDesc::new().with_unlit(colour),
                    MeshRenderer::new(),
                ))
                .id()
        })
        .collect();
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),)).id();
    let banner = world
        .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.2), Vec2::ZERO)
            .with_anchor(TextAnchor::Center)
            .with_align(TextAlign::Center),))
        .id();
    let auto = std::env::var_os("KOPRU_OTO").is_some();
    let mut game = Game {
        physics: FixedPhysics::new(120),
        mode: Mode::Build,
        beams: Vec::new(),
        picked: None,
        bodies: Vec::new(),
        joint_beam: Vec::new(),
        ball: 0,
        previews: Vec::new(),
        nodes: node_ids,
        hud,
        banner,
        clock: 0.0,
        result: None,
        auto,
    };
    if auto {
        game.beams = truss();
        rebuild_previews(world, &mut game);
        start_test(world, &mut game);
    }
    game
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Köprü", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Hazır bir kafes (Warren): üst kuşak y = 0, alt kuşak y = −2, aralarda çaprazlar.
/// Bir köprüyü kurup dener; topun karşıya geçip geçmediği, kaç eklemin koptuğu, en yüklü eklemin
/// eşiğin ne kadarını taşıdığı ve sürenin kaç saniye olduğu.
fn try_bridge(beams: Vec<Beam>) -> (Option<bool>, usize, f32, f32) {
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font());
    game.auto = false;
    game.beams = beams;
    rebuild_previews(&mut world, &mut game);
    start_test(&mut world, &mut game);
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let mut worst = 0.0f32;
    while run.elapsed < 20.0 && run.world.get_resource::<Game>().is_some_and(|g| g.result.is_none()) {
        run.frame(1.0 / 60.0, update);
        if let Some(p) = run.world.get_resource::<PhysicsWorld>() {
            worst = p.joints.iter().filter(|j| !j.is_broken).map(|j| j.load()).fold(worst, f32::max);
        }
    }
    let broken = run.world.get_resource::<PhysicsWorld>().map_or(0, |p| p.joints.iter().filter(|j| j.is_broken).count());
    (run.world.get_resource::<Game>().and_then(|g| g.result), broken, worst, run.elapsed)
}

/// `--test`: pencere açmadan üç köprü. Hazır kafes topu taşımalı; menteşeli kirişlerden tek sıra
/// bir döşeme de taşır (sarkan bir zincir gibi: eğim birkaç derece, gerilme eşiğin çok altında);
/// ortasından bir kirişi eksik döşemede top düşmeli.
fn self_test() -> bool {
    let (truss_result, truss_broken, truss_load, truss_time) = try_bridge(truss());
    let all = nodes();
    let at = |x: f32| all.iter().position(|p| (p.x - x).abs() < 0.01 && p.y.abs() < 0.01).expect("a grid node");
    let deck: Vec<Beam> = (0..6).map(|i| Beam { a: at(NODE_X[i]), b: at(NODE_X[i + 1]) }).collect();
    let (deck_result, deck_broken, deck_load, deck_time) = try_bridge(deck.clone());
    let gapped: Vec<Beam> = deck.into_iter().enumerate().filter(|&(i, _)| i != 3).map(|(_, b)| b).collect();
    let (gap_result, _, _, gap_time) = try_bridge(gapped);
    let ok = truss_result == Some(true) && truss_broken == 0 && deck_result == Some(true) && gap_result == Some(false);
    println!(
        "kopru --test: kafes {:?} ({truss_time:.1} s, {truss_broken} eklem koptu, en yüklü eklem eşiğin %{:.0}'i); \
         döşeme {:?} ({deck_time:.1} s, {deck_broken} koptu, %{:.0}); bir kirişi eksik döşeme {:?} ({gap_time:.1} s) → {}",
        truss_result,
        truss_load * 100.0,
        deck_result,
        deck_load * 100.0,
        gap_result,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn truss() -> Vec<Beam> {
    let all = nodes();
    let at = |x: f32, y: f32| all.iter().position(|p| (p.x - x).abs() < 0.01 && (p.y - y).abs() < 0.01).expect("a grid node");
    let mut beams = Vec::new();
    for i in 0..6 {
        let (x0, x1) = (NODE_X[i], NODE_X[i + 1]);
        beams.push(Beam { a: at(x0, 0.0), b: at(x1, 0.0) });
        beams.push(Beam { a: at(x0, -2.0), b: at(x1, -2.0) });
        let (lo, hi) = if i % 2 == 0 { (at(x0, -2.0), at(x1, 0.0)) } else { (at(x0, 0.0), at(x1, -2.0)) };
        beams.push(Beam { a: lo, b: hi });
    }
    beams
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    if input.is_key_just_pressed(KeyCode::Space as u32) {
        match game.mode {
            Mode::Build => start_test(world, &mut game),
            Mode::Test => stop_test(world, &mut game),
        }
    }
    if game.mode == Mode::Build {
        build_input(world, &mut game, input);
        // Seçili düğüm büyür.
        for (i, &node) in game.nodes.iter().enumerate() {
            if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(node) {
                t.scale = Vec3::splat(if game.picked == Some(i) { 0.32 } else { 0.18 });
            }
        }
    } else {
        game.physics.advance(world, dt);
        sync_transforms(world);
        game.clock += dt;
        colour_beams(world, &game);
        judge(world, &mut game);
        if game.auto && game.clock > 12.0 {
            stop_test(world, &mut game);
            start_test(world, &mut game);
        }
    }
    hud(world, &game);
    world.insert_resource(game);
}

fn build_input(world: &mut World, game: &mut Game, input: &Input) {
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        game.beams.clear();
        game.picked = None;
        rebuild_previews(world, game);
    }
    if input.is_mouse_button_just_pressed(1) {
        game.beams.pop();
        game.picked = None;
        rebuild_previews(world, game);
    }
    if input.is_mouse_button_just_pressed(0) {
        let Some(at) = gizmo::pointer::cursor_on_plane(world, Vec3::ZERO, Vec3::Z) else { return };
        let all = nodes();
        let Some((node, d)) = all.iter().enumerate().map(|(i, p)| (i, p.distance(at))).min_by(|a, b| a.1.total_cmp(&b.1)) else { return };
        if d > 0.7 {
            game.picked = None;
            return;
        }
        match game.picked {
            None => game.picked = Some(node),
            Some(first) if first == node => game.picked = None,
            Some(first) => {
                let length = all[first].distance(all[node]);
                let exists = game.beams.iter().any(|b| (b.a == first && b.b == node) || (b.a == node && b.b == first));
                if length <= MAX_BEAM && !exists && (game.beams.len() as u32) < BUDGET {
                    game.beams.push(Beam { a: first, b: node });
                    rebuild_previews(world, game);
                }
                game.picked = Some(node);
            }
        }
    }
}

fn beam_transform(a: Vec3, b: Vec3) -> (Vec3, Quat, f32) {
    let d = b - a;
    (a + d * 0.5, Quat::from_rotation_z(d.y.atan2(d.x)), d.length() * 0.5)
}

/// Kurma kipinde kirişlerin yalnızca görünüşü.
fn rebuild_previews(world: &mut World, game: &mut Game) {
    for id in game.previews.drain(..) {
        if let Some(e) = world.get_entity(id) {
            world.despawn(e);
        }
    }
    let all = nodes();
    for beam in &game.beams {
        let (mid, rot, half) = beam_transform(all[beam.a], all[beam.b]);
        let id = world
            .spawn_bundle((
                Transform::new(mid).with_rotation(rot).with_scale(Vec3::new(half, BEAM_HALF_THICK, 0.25)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Vec4::new(0.6, 0.45, 0.25, 1.0), 0.8, 0.0),
                MeshRenderer::new(),
            ))
            .id();
        game.previews.push(id);
    }
}

/// Kirişleri cisme, düğümleri eklemlere çevirir ve topu bırakır.
fn start_test(world: &mut World, game: &mut Game) {
    for id in game.previews.drain(..) {
        if let Some(e) = world.get_entity(id) {
            world.despawn(e);
        }
    }
    let all = nodes();
    let cliff_body: Vec<(u32, Vec3)> = world
        .borrow::<RigidBody>()
        .iter()
        .filter(|(_, rb)| rb.is_static())
        .filter_map(|(id, _)| world.borrow::<Transform>().get(id).map(|t| (id, t.position)))
        .collect();
    let mut joints = Vec::new();
    game.bodies.clear();
    game.joint_beam.clear();
    // Her düğümde, oraya bağlanan kiriş uçları: (kiriş, yerel uç).
    let mut at_node: Vec<Vec<(usize, Vec3)>> = vec![Vec::new(); all.len()];
    for (i, beam) in game.beams.iter().enumerate() {
        let (mid, rot, half) = beam_transform(all[beam.a], all[beam.b]);
        let collider = Collider::box_collider(Vec3::new(half, BEAM_HALF_THICK, 0.25))
            .with_layer(CollisionLayer::new(BEAM_LAYER).with_mask(!(1 << BEAM_LAYER)));
        let mut body = RigidBody::new(BEAM_MASS, true);
        body.update_inertia_from_collider(&collider);
        body.lock_translation_z = true;
        body.lock_rotation_x = true;
        body.lock_rotation_y = true;
        let id = world
            .spawn_bundle((
                Transform::new(mid).with_rotation(rot),
                GlobalTransform::default(),
                body,
                Velocity::default(),
                collider,
                BeamBody,
            ))
            .id();
        let look = world.spawn_bundle((
            Transform::new(Vec3::ZERO).with_scale(Vec3::new(half, BEAM_HALF_THICK, 0.25)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(Vec4::new(0.3, 0.8, 0.3, 1.0), 0.8, 0.0),
            MeshRenderer::new(),
        ));
        {
            use gizmo::core::hierarchy::HierarchyExt;
            let parent = world.get_entity(id).expect("just spawned");
            world.add_child(parent, look);
        }
        game.bodies.push(id);
        at_node[beam.a].push((i, Vec3::new(-half, 0.0, 0.0)));
        at_node[beam.b].push((i, Vec3::new(half, 0.0, 0.0)));
    }
    for (node, ends) in at_node.iter().enumerate() {
        let Some(&(first, first_end)) = ends.first() else { continue };
        // A free node ties every beam to the first; an anchored one ties each to the cliff only —
        // both would be a loop of constraints fighting each other.
        if !anchored(all[node]) {
            for &(other, other_end) in &ends[1..] {
                joints.push((first, game.bodies[first], first_end, game.bodies[other], other_end));
            }
        }
        if anchored(all[node]) {
            // Uçuruma: en yakın statik cisme, düğümün onun içindeki yeri.
            let p = all[node];
            if let Some(&(cliff, centre)) = cliff_body.iter().min_by(|a, b| a.1.distance(p).total_cmp(&b.1.distance(p))) {
                for &(beam, end) in ends {
                    joints.push((beam, game.bodies[beam], end, cliff, p - centre));
                }
            }
        }
    }
    if let Some(mut physics) = world.get_resource_mut::<PhysicsWorld>() {
        physics.joints.clear();
        for (beam, a, end_a, b, end_b) in joints {
            let mut joint = Joint::ball_socket(BodyHandle::from_id(a), BodyHandle::from_id(b), end_a, end_b);
            joint.break_force = BREAK_FORCE;
            physics.joints.push(joint);
            game.joint_beam.push(beam);
        }
    }
    let mut ball = RigidBody::new(BALL_MASS, true);
    ball.lock_translation_z = true;
    game.ball = world
        .spawn_bundle((
            Transform::new(Vec3::new(-10.0, BALL_R + 0.05, 0.0)).with_scale(Vec3::splat(BALL_R)),
            GlobalTransform::default(),
            MeshSource("sphere".into()),
            MaterialDesc::new().with_pbr(Vec4::new(0.25, 0.3, 0.4, 1.0), 0.3, 0.8),
            MeshRenderer::new(),
            ball,
            Velocity { linear: Vec3::new(6.0, 0.0, 0.0), ..Default::default() },
            Collider::sphere(BALL_R),
        ))
        .id();
    game.physics.reset();
    game.mode = Mode::Test;
    game.clock = 0.0;
    game.result = None;
}

fn stop_test(world: &mut World, game: &mut Game) {
    for id in game.bodies.drain(..).chain(std::iter::once(game.ball)) {
        if let Some(e) = world.get_entity(id) {
            use gizmo::core::hierarchy::HierarchyExt;
            world.despawn_recursive(e);
        }
    }
    game.joint_beam.clear();
    game.mode = Mode::Build;
    game.picked = None;
    rebuild_previews(world, game);
}

/// Her kiriş, eklemlerinin en yüklüsü kadar kızarır; eklemi kopmuşsa griye döner.
fn colour_beams(world: &mut World, game: &Game) {
    let mut load = vec![0.0f32; game.bodies.len()];
    let mut broken = vec![false; game.bodies.len()];
    if let Some(physics) = world.get_resource::<PhysicsWorld>() {
        for (joint, &beam) in physics.joints.iter().zip(&game.joint_beam) {
            if joint.is_broken {
                broken[beam] = true;
            } else {
                load[beam] = load[beam].max(joint.load());
            }
        }
    }
    for (i, &body) in game.bodies.iter().enumerate() {
        let colour = if broken[i] {
            Vec4::new(0.35, 0.35, 0.35, 1.0)
        } else {
            let k = load[i].clamp(0.0, 1.0);
            Vec4::new(0.3 + 0.65 * k, 0.8 - 0.65 * k, 0.3 - 0.2 * k, 1.0)
        };
        let child = world.borrow::<gizmo::core::component::Children>().get(body).and_then(|c| c.0.first().copied());
        if let Some(child) = child {
            if let Some(mut m) = world.borrow_mut::<MaterialDesc>().get_mut(child.id()) {
                if m.albedo != colour {
                    *m = m.clone().with_pbr(colour, 0.8, 0.0);
                }
            }
        }
    }
}

fn judge(world: &mut World, game: &mut Game) {
    if game.result.is_some() {
        return;
    }
    let Some(p) = world.borrow::<Transform>().get(game.ball).map(|t| t.position) else { return };
    if p.x > 7.0 && p.y > -0.5 {
        game.result = Some(true);
    } else if p.y < -8.0 {
        game.result = Some(false);
    }
}

fn hud(world: &mut World, game: &Game) {
    let broken = world.get_resource::<PhysicsWorld>().map_or(0, |p| p.joints.iter().filter(|j| j.is_broken).count());
    let text = match game.mode {
        Mode::Build => format!(
            "Kurma — kiriş {}/{BUDGET}\nSol tık: düğümden düğüme kiriş   Sağ tık: geri al   Boşluk: dene   R: sil",
            game.beams.len()
        ),
        Mode::Test => format!("Deneme — {:.1} sn   kopan eklem {broken}\nBoşluk: kurmaya dön", game.clock),
    };
    set_text(world, game.hud, &text);
    let banner = match game.result {
        Some(true) => "Geçti!",
        Some(false) => "Köprü dayanmadı",
        None => "",
    };
    set_text(world, game.banner, banner);
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
