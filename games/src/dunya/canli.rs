//! Canlılar ve köyler: otlayan, yaklaşınca kaçan hayvanlar; köylerde dolaşan, iş isteyen insanlar.
//!
//! **Hayvanlar** — koyun ve geyik — çimenli hücrelerde, hücrenin tohumuyla. Her biri motorun
//! davranış ağacıyla (`gizmo::ai::BehaviorTree`) yaşıyor: oyuncu yakınsa kaç; değilse evinin
//! çevresinde bir yere yürü, sonra bir süre otla. Koyun 6 m'de ürküyor, geyik 12 m'de ve daha hızlı
//! kaçıyor. Yürüyüşleri oyuncununki gibi bir `CharacterController`: tepeye tırmanıyor, ağaca
//! çarpıyor.
//!
//! **Köyler** bazı hücrelerin düz bir yerinde: üç barınak, ortada bir kamp ateşi, iki köylü. Köylü
//! köyün içinde dolaşıyor; oyuncu yanına gelince durup ona dönüyor. **T** ile konuşulur: ilk
//! konuşmada bir iş ister ("altı odun getirir misin?"), istenen envanterde varsa ikinci konuşmada
//! alır ve karşılığını verir. İşlerin durumu kayıtta.
//!
//! Bir ağacın eylemi, ağacı taşıyan varlık için çalışan bir fonksiyon (`BtLibrary`); bir hayvanın
//! aklında tuttuğu (gideceği yer, ne kadar otladığı) kendi `Wanderer` bileşeninde.

use super::*;
use gizmo::ai::{behavior_tree_system, BehaviorTree, BtLibrary, BtNode, BtStatus};

/// Bir canlının yürüyüşü ve aklında tuttukları.
#[derive(Clone, Copy)]
pub(super) struct Wanderer {
    /// Dolaştığı yerin ortası ve yarıçapı.
    home: Vec3,
    radius: f32,
    speed: f32,
    /// Bu uzaklıkta oyuncudan ürküyor (köylü için: duruyor ve dönüyor).
    wary: f32,
    flee_speed: f32,
    goal: Option<Vec3>,
    /// Bu hedefe ya da bu otlamaya ne kadar zamandır.
    timer: f32,
    seed: u32,
}
gizmo::core::impl_component!(Wanderer);

/// Bir köylü: kimliği (hücre ve sırası) ve adı.
#[derive(Clone, Copy)]
pub(super) struct Villager {
    pub(super) id: (i32, i32, u8),
    pub(super) name: &'static str,
}
gizmo::core::impl_component!(Villager);

/// Bir hayvan; kaç tanesinin kaçtığını sınama sayıyor.
#[derive(Clone, Copy)]
pub(super) struct Animal;
gizmo::core::impl_component!(Animal);

/// Oyuncunun bu karedeki yeri — ağaçların koşulları okuyor.
#[derive(Clone, Copy)]
struct Seen(Vec3);

/// Bir köylünün isteği ve karşılığı.
pub(super) struct Quest {
    pub(super) want: (&'static str, u32),
    pub(super) reward: (&'static str, u32),
}

/// Köylünün kimliğinden, hep aynı iş.
pub(super) fn quest_of(id: (i32, i32, u8)) -> Quest {
    match hash(id.0, id.1, 300 + id.2 as u32) % 4 {
        0 => Quest { want: ("odun", 6), reward: ("kristal", 1) },
        1 => Quest { want: ("tas", 4), reward: ("kamp_atesi", 1) },
        2 => Quest { want: ("odun", 10), reward: ("cit", 4) },
        _ => Quest { want: ("tas", 6), reward: ("kristal", 1) },
    }
}

const NAMES: [&str; 12] = ["Ayşe", "Mehmet", "Zeynep", "Ali", "Elif", "Hasan", "Fatma", "Emre", "Selin", "Kemal", "Derya", "Murat"];

/// Bu hücrede bir köy var mı, varsa ortası: dört hücreden birinde, başlangıçtan uzakta, düz ve
/// karsız bir yerde.
pub(super) fn village_at(coord: ChunkCoord) -> Option<Vec3> {
    if !hash(coord.0, coord.1, 77).is_multiple_of(4) || (coord.0 == 0 && coord.1 == 0) {
        return None;
    }
    let centre = ChunkManager::chunk_to_world_pos(coord);
    // Hücrenin ortasına yakın birkaç aday: en düzü.
    let mut best: Option<(f32, Vec3)> = None;
    for k in 0..9 {
        let (dx, dz) = (((k % 3) as f32 - 1.0) * 22.0, ((k / 3) as f32 - 1.0) * 22.0);
        let (x, z) = (centre.x + dx, centre.z + dz);
        let s = slope(x, z) + (slope(x + 6.0, z) + slope(x - 6.0, z) + slope(x, z + 6.0) + slope(x, z - 6.0)) * 0.25;
        if best.is_none_or(|(b, _)| s < b) {
            best = Some((s, Vec3::new(x, height(x, z), z)));
        }
    }
    let (s, at) = best?;
    (s < 0.18 && at.y < 24.0 && at.y > super::su::WATER + 1.0).then_some(at)
}

fn material(c: Vec4) -> MaterialDesc {
    MaterialDesc::new().with_pbr(c, 0.85, 0.0)
}

/// Bir görünen parçayı `parent`'ın çocuğu yapar (motorun küpü ±1, silindiri yarıçap 1 ve yarı boyu 1).
fn piece(world: &mut World, parent: Entity, at: Vec3, scale: Vec3, turn: Quat, mesh: &str, look: MaterialDesc) {
    let e = world.spawn_bundle((
        Transform::new(at).with_rotation(turn).with_scale(scale),
        GlobalTransform::default(),
        MeshSource(mesh.into()),
        look,
        MeshRenderer::new(),
    ));
    world.add_child(parent, e);
}

/// Bir canlının gövdesi: kapsül çarpıştırıcı, oyuncununki gibi yürüyüş.
fn walker(world: &mut World, at: Vec3, radius: f32, half_height: f32, speed: f32) -> Entity {
    world.spawn_bundle((
        Transform::new(at + Vec3::Y * (half_height + radius + 0.2)),
        GlobalTransform::default(),
        CharacterController { speed, jump_speed: 0.0, ..Default::default() },
        Velocity::default(),
        Collider::capsule(radius, half_height),
    ))
}

fn spawn_sheep(world: &mut World, at: Vec3, seed: u32, visual: bool) -> Entity {
    let e = walker(world, at, 0.3, 0.15, 1.2);
    world.add_component(e, Wanderer { home: at, radius: 14.0, speed: 1.2, wary: 6.0, flee_speed: 3.8, goal: None, timer: 0.0, seed });
    world.add_component(e, Animal);
    world.add_component(e, BehaviorTree::new(animal_tree()));
    if visual {
        let wool = material(Vec4::new(0.92, 0.9, 0.86, 1.0));
        let dark = material(Vec4::new(0.18, 0.16, 0.15, 1.0));
        piece(world, e, Vec3::new(0.0, 0.05, 0.0), Vec3::new(0.38, 0.32, 0.52), Quat::IDENTITY, "sphere", wool);
        piece(world, e, Vec3::new(0.0, 0.2, -0.55), Vec3::new(0.16, 0.16, 0.2), Quat::IDENTITY, "sphere", dark.clone());
        for (x, z) in [(-0.18f32, -0.28f32), (0.18, -0.28), (-0.18, 0.28), (0.18, 0.28)] {
            piece(world, e, Vec3::new(x, -0.3, z), Vec3::new(0.05, 0.18, 0.05), Quat::IDENTITY, "cylinder", dark.clone());
        }
    }
    e
}

fn spawn_deer(world: &mut World, at: Vec3, seed: u32, visual: bool) -> Entity {
    let e = walker(world, at, 0.35, 0.3, 1.8);
    world.add_component(e, Wanderer { home: at, radius: 22.0, speed: 1.8, wary: 12.0, flee_speed: 6.5, goal: None, timer: 0.0, seed });
    world.add_component(e, Animal);
    world.add_component(e, BehaviorTree::new(animal_tree()));
    if visual {
        let coat = material(Vec4::new(0.55, 0.36, 0.2, 1.0));
        let dark = material(Vec4::new(0.25, 0.17, 0.1, 1.0));
        piece(world, e, Vec3::new(0.0, 0.15, 0.0), Vec3::new(0.24, 0.26, 0.62), Quat::IDENTITY, "standard_cube", coat.clone());
        piece(world, e, Vec3::new(0.0, 0.62, -0.62), Vec3::new(0.09, 0.32, 0.09), Quat::from_rotation_x(0.45), "standard_cube", coat.clone());
        piece(world, e, Vec3::new(0.0, 0.95, -0.82), Vec3::new(0.12, 0.12, 0.2), Quat::IDENTITY, "standard_cube", coat);
        for x in [-0.12f32, 0.12] {
            piece(world, e, Vec3::new(x, 1.22, -0.78), Vec3::new(0.025, 0.2, 0.025), Quat::from_rotation_z(x * 3.0), "cylinder", dark.clone());
        }
        for (x, z) in [(-0.16f32, -0.48f32), (0.16, -0.48), (-0.16, 0.48), (0.16, 0.48)] {
            piece(world, e, Vec3::new(x, -0.3, z), Vec3::new(0.05, 0.35, 0.05), Quat::IDENTITY, "cylinder", dark.clone());
        }
    }
    e
}

fn spawn_villager(world: &mut World, at: Vec3, id: (i32, i32, u8), home: Vec3, visual: bool) -> Entity {
    let name = NAMES[(hash(id.0, id.1, 400 + id.2 as u32) % NAMES.len() as u32) as usize];
    let e = walker(world, at, 0.33, 0.5, 1.4);
    world.add_component(e, Wanderer { home, radius: 9.0, speed: 1.4, wary: 4.0, flee_speed: 0.0, goal: None, timer: 0.0, seed: hash(id.0, id.1, id.2 as u32) });
    world.add_component(e, Villager { id, name });
    world.add_component(e, BehaviorTree::new(villager_tree()));
    if visual {
        let h = (hash(id.0, id.1, 500 + id.2 as u32) % 1000) as f32 / 1000.0 * std::f32::consts::TAU;
        let shirt = material(Vec4::new(0.5 + 0.4 * h.cos(), 0.5 + 0.4 * (h + 2.1).cos(), 0.5 + 0.4 * (h + 4.2).cos(), 1.0));
        piece(world, e, Vec3::ZERO, Vec3::ONE, Quat::IDENTITY, "capsule_0.33_1.0_6_16", shirt);
        piece(world, e, Vec3::new(0.0, 0.95, 0.0), Vec3::splat(0.22), Quat::IDENTITY, "sphere", material(Vec4::new(0.88, 0.7, 0.55, 1.0)));
        piece(world, e, Vec3::new(0.0, 1.22, 0.0), Vec3::new(0.3, 0.14, 0.3), Quat::IDENTITY, "cone", material(Vec4::new(0.75, 0.62, 0.3, 1.0)));
        piece(world, e, Vec3::new(0.0, 0.95, -0.2), Vec3::new(0.14, 0.04, 0.03), Quat::IDENTITY, "standard_cube", MaterialDesc::new().with_unlit(Vec4::new(0.1, 0.1, 0.12, 1.0)));
    }
    e
}

/// Bir hücrenin canlıları ve köyü; hücreyle birlikte silinecek varlıkları `spawned`'a ekler.
pub(super) fn populate_chunk(world: &mut World, coord: ChunkCoord, visual: bool, spawned: &mut Vec<Entity>) {
    let centre = ChunkManager::chunk_to_world_pos(coord);
    let mut rng = ChunkRng(hash(coord.0, coord.1, 202));
    // Hayvanlar: iki küçük sürü — çimende (alçak, düz).
    for herd in 0..2u32 {
        let (x, z) = (centre.x + (rng.next() - 0.5) * 70.0, centre.z + (rng.next() - 0.5) * 70.0);
        let deer = rng.next() < 0.35;
        let count = if deer { 2 } else { 3 + (rng.next() * 2.0) as u32 };
        if slope(x, z) > 0.35 || height(x, z) > 20.0 || super::su::wet(x, z, 0.5) || Vec2::new(x, z).length() < 15.0 {
            continue;
        }
        for k in 0..count {
            let (ax, az) = (x + (rng.next() - 0.5) * 6.0, z + (rng.next() - 0.5) * 6.0);
            let at = Vec3::new(ax, height(ax, az), az);
            let seed = hash(coord.0, coord.1, 600 + herd * 10 + k);
            spawned.push(if deer { spawn_deer(world, at, seed, visual) } else { spawn_sheep(world, at, seed, visual) });
        }
    }
    // Köy.
    let Some(at) = village_at(coord) else { return };
    for k in 0..3 {
        let a = k as f32 / 3.0 * std::f32::consts::TAU + 0.4;
        let house = at + Vec3::new(a.cos(), 0.0, a.sin()) * 8.0;
        // Kapısı (açık önü, +Z) köyün ortasına baksın.
        let yaw = (house.x - at.x).atan2(house.z - at.z) + std::f32::consts::PI;
        spawned.extend(super::insa::spawn_structure(world, super::insa::Build::Hut, house, yaw, visual, false));
    }
    spawned.extend(super::insa::spawn_structure(world, super::insa::Build::Fire, at, 0.0, visual, false));
    for k in 0..2u8 {
        let p = at + Vec3::new(if k == 0 { 3.0 } else { -3.0 }, 0.0, 2.0);
        spawned.push(spawn_villager(world, Vec3::new(p.x, height(p.x, p.z), p.z), (coord.0, coord.1, k), at, visual));
    }
}

fn animal_tree() -> BtNode {
    BtNode::selector([BtNode::sequence([BtNode::condition("korkmus"), BtNode::action("kac")]), BtNode::sequence([BtNode::action("dolas"), BtNode::action("otla")])])
}

fn villager_tree() -> BtNode {
    BtNode::selector([BtNode::sequence([BtNode::condition("korkmus"), BtNode::action("bak")]), BtNode::sequence([BtNode::action("dolas"), BtNode::action("otla")])])
}

fn position(world: &World, e: Entity) -> Option<Vec3> {
    world.get::<Transform>(e).map(|t| t.position)
}

fn scared(e: Entity, world: &World) -> bool {
    let (Some(w), Some(p), Some(seen)) = (world.get::<Wanderer>(e), position(world, e), world.get_resource::<Seen>()) else { return false };
    Vec2::new(p.x - seen.0.x, p.z - seen.0.z).length() < w.wary
}

/// Yürüt: `dir` yönünde `speed` hızla (sıfır: dur), ve o yöne dön.
fn walk(world: &mut World, e: Entity, dir: Vec3, speed: f32, dt: f32) {
    let dir = Vec3::new(dir.x, 0.0, dir.z).normalize_or_zero();
    if let Some(mut k) = world.query_mut::<&mut CharacterController>().get_mut(e) {
        k.target_velocity = dir * speed;
    }
    if dir != Vec3::ZERO {
        if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(e) {
            let want = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
            t.rotation = t.rotation.slerp(want, (dt * 6.0).min(1.0));
        }
    }
}

fn library() -> BtLibrary {
    BtLibrary::new()
        .with_condition("korkmus", scared)
        // Oyuncudan uzağa, ürktüğü uzaklığın iki katına dek.
        .with_action("kac", |e, world, dt| {
            let (Some(w), Some(p), Some(seen)) = (world.get::<Wanderer>(e).copied(), position(world, e), world.get_resource::<Seen>().map(|s| *s)) else {
                return BtStatus::Failure;
            };
            let away = Vec3::new(p.x - seen.0.x, 0.0, p.z - seen.0.z);
            if away.length() > w.wary * 2.0 {
                walk(world, e, Vec3::ZERO, 0.0, dt);
                if let Some(mut w) = world.query_mut::<&mut Wanderer>().get_mut(e) {
                    // Kaçtığı yer yeni evi: geri dönüp oyuncunun dibinde otlamasın.
                    w.home = p;
                    w.goal = None;
                }
                return BtStatus::Success;
            }
            walk(world, e, away, w.flee_speed, dt);
            BtStatus::Running
        })
        // Köylü: dur, oyuncuya dön.
        .with_action("bak", |e, world, dt| {
            let (Some(p), Some(seen)) = (position(world, e), world.get_resource::<Seen>().map(|s| *s)) else { return BtStatus::Failure };
            walk(world, e, Vec3::ZERO, 0.0, dt);
            let to = Vec3::new(seen.0.x - p.x, 0.0, seen.0.z - p.z).normalize_or_zero();
            if to != Vec3::ZERO {
                if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(e) {
                    t.rotation = t.rotation.slerp(Quat::from_rotation_arc(Vec3::NEG_Z, to), (dt * 5.0).min(1.0));
                }
            }
            BtStatus::Success
        })
        // Evinin çevresinde bir yere yürü; ürkerse bırak (ağaç kaçmaya dönsün).
        .with_action("dolas", |e, world, dt| {
            if scared(e, world) {
                return BtStatus::Failure;
            }
            let (Some(mut w), Some(p)) = (world.get::<Wanderer>(e).copied(), position(world, e)) else { return BtStatus::Failure };
            let goal = *w.goal.get_or_insert_with(|| {
                let mut r = ChunkRng(w.seed);
                let (a, d) = (r.next() * std::f32::consts::TAU, r.next().sqrt() * w.radius);
                w.seed = r.0;
                w.timer = 0.0;
                w.home + Vec3::new(a.cos() * d, 0.0, a.sin() * d)
            });
            w.timer += dt;
            let to = Vec3::new(goal.x - p.x, 0.0, goal.z - p.z);
            let done = to.length() < 1.0 || w.timer > 12.0;
            if done {
                w.goal = None;
                w.timer = 0.0;
            }
            if let Some(mut stored) = world.query_mut::<&mut Wanderer>().get_mut(e) {
                *stored = w;
            }
            if done {
                walk(world, e, Vec3::ZERO, 0.0, dt);
                return BtStatus::Success;
            }
            walk(world, e, to, w.speed, dt);
            BtStatus::Running
        })
        // Bir süre dur (otla, dinlen); ürkerse bırak.
        .with_action("otla", |e, world, dt| {
            if scared(e, world) {
                return BtStatus::Failure;
            }
            walk(world, e, Vec3::ZERO, 0.0, dt);
            let mut wanderers = world.query_mut::<&mut Wanderer>();
            let Some(mut w) = wanderers.get_mut(e) else { return BtStatus::Failure };
            w.timer += dt;
            let rest = 2.0 + (w.seed % 400) as f32 / 100.0;
            if w.timer >= rest {
                w.timer = 0.0;
                return BtStatus::Success;
            }
            BtStatus::Running
        })
}

/// Oyun kurulurken: ağaçların fonksiyonları.
pub(super) fn install(world: &mut World) {
    world.insert_resource(library());
    world.insert_resource(Seen(Vec3::splat(f32::MAX)));
}

/// Her kare, fizikten önce: oyuncunun yeri, sonra her canlının ağacı.
pub(super) fn tick(world: &mut World, me: Vec3, dt: f32) {
    world.insert_resource(Seen(me));
    behavior_tree_system(world, dt);
}

/// Konuşulabilecek en yakın köylü (3,5 m içinde).
pub(super) fn nearest_villager(world: &World, me: Vec3) -> Option<(Entity, Villager)> {
    world
        .query::<(Entity, &Villager, &Transform)>()
        .iter()
        .filter(|(_, _, t)| t.position.distance(me) < 3.5)
        .min_by(|a, b| a.2.position.distance(me).total_cmp(&b.2.position.distance(me)))
        .map(|(e, v, _)| (e, *v))
}

fn item_name(game: &Game, item: &str, count: u32) -> String {
    format!("{count} {}", game.catalog.name(item).to_lowercase())
}

/// T: en yakın köylüyle konuş — iş al, getirdiğini ver, ya da selam.
pub(super) fn talk(world: &mut World, game: &mut Game) -> bool {
    let me = focus(world, game);
    let Some((_, villager)) = nearest_villager(world, me) else { return false };
    let quest = quest_of(villager.id);
    let state = game.quests.get(&villager.id).copied().unwrap_or(0);
    let (item, need) = quest.want;
    let text = match state {
        0 => {
            game.quests.insert(villager.id, 1);
            game.dirty = true;
            format!("{}: {} getirir misin? Karşılığında {} veririm.", villager.name, item_name(game, item, need), item_name(game, quest.reward.0, quest.reward.1))
        }
        1 => {
            let have = world.get::<Inventory>(game.player).map_or(0, |b| b.count(item));
            if have < need {
                format!("{}: {} bekliyorum — {have} tane var.", villager.name, item_name(game, item, need))
            } else {
                let taken = {
                    let mut bags = world.query_mut::<&mut Inventory>();
                    bags.get_mut(game.player).is_some_and(|mut b| b.remove(item, need))
                };
                if taken && give(world, game, quest.reward.0, quest.reward.1) {
                    game.quests.insert(villager.id, 2);
                    game.dirty = true;
                    format!("{}: Sağ ol! Al bakalım: {}.", villager.name, item_name(game, quest.reward.0, quest.reward.1))
                } else {
                    // Ödül sığmadı: verdiğini geri ver.
                    if taken {
                        give(world, game, item, need);
                    }
                    format!("{}: Çantan dolu, biraz yer aç.", villager.name)
                }
            }
        }
        _ => format!("{}: Yolun açık olsun!", villager.name),
    };
    game.note = (text, 4.0);
    true
}

/// HUD için: süren işler, kısaca.
pub(super) fn quest_lines(world: &World, game: &Game) -> String {
    let bag = world.get::<Inventory>(game.player);
    let mut active: Vec<_> = game.quests.iter().filter(|(_, s)| **s == 1).map(|(id, _)| *id).collect();
    active.sort_unstable();
    active
        .iter()
        .map(|id| {
            let q = quest_of(*id);
            let have = bag.map_or(0, |b| b.count(q.want.0));
            format!("iş: {} ({}/{})", game.catalog.name(q.want.0), have.min(q.want.1), q.want.1)
        })
        .collect::<Vec<_>>()
        .join(" · ")
}
