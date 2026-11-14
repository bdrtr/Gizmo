//! # Dünya — açık dünyada keşif
//!
//! Sonu olmayan, yöntemsel üretilmiş bir arazi: tepeler, dağlar, ormanlar, kayalar. Yürüyerek gez,
//! yoldaki arabalara bin, gece-gündüz dönerken araziye saçılmış parlak kristalleri topla. On beş
//! kristal bulunca kazanırsın; HUD en yakın kristalin uzaklığını ve yönünü gösteriyor. Motorun
//! otuz sekizinci oyunu ve bütün bunları aynı anda yapan ilki.
//!
//! | girdi | iş |
//! |---|---|
//! | **W A S D · sol çubuk** | yürü / sür |
//! | **Boşluk** | zıpla (yayayken) |
//! | **E** | arabaya bin / in |
//! | **F** | yakındaki ağacı kes (odun), kayayı kır (taş) |
//! | **I · Tab** | envanter: eşyayı yuvadan yuvaya sürükle, panelin dışına bırakırsan yere düşer |
//! | **sağ tık + fare · ← →** | kamerayı döndür |
//! | **↑ ↓** | kamerayı yükselt / alçalt |
//! | **R** | başlangıca dön |
//!
//! `DUNYA_ENVANTER_ACIK=1` envanteri açık ve içinde birkaç eşyayla başlatır (ekran görüntüsü için).
//!
//! ## Nasıl kurulu
//!
//! - **Arazi parça parça**: dünya 100 m'lik hücrelere bölünmüş ve oyuncunun (ya da sürdüğü
//!   arabanın) çevresindeki 5×5 hücre yüklü: motorun `open_world_chunk_system`'i bir hücreye
//!   girince uzaktakileri siliyor, yenilerini kuruyor. Her hücre bir yükseklik alanı çarpıştırıcısı,
//!   yüksekliğe ve eğime göre boyanmış bir ağ (çim, toprak, kaya, kar), ve hücrenin kendi tohumuyla
//!   dizilmiş ağaç, kaya ve kristal: aynı hücre her yüklendiğinde aynı.
//! - **Yükseklik** gürültüden: geniş tepeler ve sırtları sivri dağlar (`height`), her yerde aynı
//!   fonksiyon, yani komşu hücrelerin kenarları birebir buluşuyor.
//! - **Envanter** motorun (`gizmo::app::inventory`): oyuncunun üstünde bir `Inventory` bileşeni,
//!   24 yuva; kristal, odun ve taş yığılıyor, panel `Display::Grid` + `Drag` + `DropTarget`. Yere
//!   bırakılan eşya üstünden geçince geri alınıyor. Envanter ve dünyadan alınanlar `SaveData` ile
//!   kaydediliyor: oyunu kapatıp açınca yerinde.
//! - **Alınan** kristal, kesilen ağaç, kırılan kaya bir daha çıkmıyor: hücre silinip yeniden
//!   kurulduğunda da.
//! - **Gece-gündüz** dört dakikada bir tur: güneş dönüyor, gök, sis ve arka plan rengi ona göre;
//!   geceleri ay ışığı ve kristallerin kendi ışıltısı.
//!
//! `dunya --test` pencere açmaz: yaya otomatik pilot iki dakika en yakın kristale yürür, sonra
//! arabaya binip hücre sınırlarının üstünden bir dakika sürer. Hiçbir an oyuncu ya da araba
//! arazinin içine düşmemeli, hücreler yüklenip silinmeli ve kristal toplanmalı.

use std::collections::HashSet;

use gizmo::app::inventory::{Inventory, ItemCatalog, ItemInfo, ItemStack};
use gizmo::app::save::SaveData;
use gizmo::core::component::IsHidden;
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::physics::components::{CharacterController, PhysicsMaterial};
use gizmo::physics::vehicle::VehicleController;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{MeshData, Text, TextAlign, TextAnchor};
use gizmo::renderer::frame_uniforms::{Fog, Sky};
use gizmo::renderer::text::FontId;
use gizmo::simple::{set_text, sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};
use gizmo::systems::{open_world_chunk_system, ChunkCoord, ChunkManager, CHUNK_SIZE};
use gizmo::ui::{
    AlignItems, BackgroundColor, ButtonBundle, Display, Drag, DropTarget, FlexDirection, GridTrack, GridTracks, JustifyContent, NodeBundle,
    Style, UiPlugin, UiRect, UiTransform, Val, ZIndex,
};

/// Bir hücrenin kenarındaki örnek sayısı: 32 aralık, 3,125 m'de bir.
const SAMPLES: u32 = 33;
/// Arazinin en yüksek noktası, metre.
const MAX_HEIGHT: f32 = 42.0;
/// Kazanmak için.
const GOAL: usize = 15;
/// Gece-gündüz turu, saniye.
const DAY_LENGTH: f32 = 240.0;
/// Bir kristali toplamak için ne kadar yaklaşmak gerek.
const PICKUP: f32 = 2.2;

// =============================================================================================
// Arazi
// =============================================================================================

fn hash(x: i32, z: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8DA6_B343) ^ (z as u32).wrapping_mul(0xD816_3841) ^ seed.wrapping_mul(0xCB1A_B31F);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^ (h >> 15)
}

fn unit(h: u32) -> f32 {
    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
}

/// Değer gürültüsü, yumuşak ara değerli: 0..1.
fn value_noise(x: f32, z: f32, seed: u32) -> f32 {
    let (xi, zi) = (x.floor() as i32, z.floor() as i32);
    let (fx, fz) = (x - xi as f32, z - zi as f32);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v) = (s(fx), s(fz));
    let corner = |dx: i32, dz: i32| unit(hash(xi + dx, zi + dz, seed));
    let a = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * u;
    let b = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * u;
    a + (b - a) * v
}

fn fbm(x: f32, z: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += value_noise(x * freq, z * freq, seed + o * 101) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    sum / norm
}

/// Arazinin yüksekliği, dünyanın her yerinde aynı fonksiyon: geniş tepeler ve sivri sırtlı dağlar.
fn height(x: f32, z: f32) -> f32 {
    let hills = fbm(x / 160.0, z / 160.0, 5, 7);
    let ridge = 1.0 - (fbm(x / 420.0, z / 420.0, 4, 31) * 2.0 - 1.0).abs();
    let mountains = (ridge.powi(3) - 0.15).max(0.0) * 1.2;
    // Başlangıcın çevresi düzlük: ilk adımlar bir dağın yamacında olmasın.
    let calm = (Vec2::new(x, z).length() / 120.0).clamp(0.25, 1.0);
    ((hills * 14.0 + mountains * 34.0 * calm) + 2.0).clamp(0.0, MAX_HEIGHT)
}

/// Eğim: yüksekliğin değişimi, metrede metre.
fn slope(x: f32, z: f32) -> f32 {
    let e = 1.5;
    Vec2::new(height(x + e, z) - height(x - e, z), height(x, z + e) - height(x, z - e)).length() / (2.0 * e)
}

/// Yüksekliğe ve eğime göre arazinin rengi.
fn ground_colour(h: f32, s: f32) -> [f32; 4] {
    let grass = Vec3::new(0.32, 0.5, 0.2);
    let dirt = Vec3::new(0.45, 0.36, 0.24);
    let rock = Vec3::new(0.42, 0.41, 0.4);
    let snow = Vec3::new(0.92, 0.93, 0.96);
    let mut c = grass.lerp(dirt, ((h - 14.0) / 8.0).clamp(0.0, 1.0));
    c = c.lerp(rock, ((s - 0.45) / 0.35).clamp(0.0, 1.0).max(((h - 24.0) / 6.0).clamp(0.0, 1.0)));
    c = c.lerp(snow, ((h - 33.0) / 4.0).clamp(0.0, 1.0));
    [c.x, c.y, c.z, 1.0]
}

/// Bir hücrenin kendi tohumuyla bir sayı üreteci: aynı hücre her yüklendiğinde aynı dizilim.
struct ChunkRng(u32);

impl ChunkRng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        unit(self.0 >> 4)
    }
}

/// Toplanabilir bir kristal: hücresi ve hücredeki sırası, bir kez toplanınca bir daha çıkmasın diye.
#[derive(Clone, Copy)]
struct Crystal {
    id: (i32, i32, u8),
}
gizmo::core::impl_component!(Crystal);

/// Toplanabilir bir ağaç ya da kaya: `F` ile alınınca envantere ne giriyor.
#[derive(Clone, Copy)]
struct Gatherable {
    id: (i32, i32, u8),
    item: &'static str,
    amount: u32,
}
gizmo::core::impl_component!(Gatherable);

/// Yere bırakılmış bir eşya yığını; `ready_in` dolmadan alınmıyor (bırakan hemen geri almasın).
#[derive(Clone)]
struct Pickup {
    stack: ItemStack,
    ready_in: f32,
}
gizmo::core::impl_component!(Pickup);

/// Envanterin kaç yuvası var, panelde kaç sütun, bir yuva kaç piksel.
const SLOTS: usize = 24;
const COLS: usize = 6;
const CELL: f32 = 78.0;

/// Dünyanın eşyaları.
fn catalog() -> ItemCatalog {
    let mut c = ItemCatalog::default();
    c.register("kristal", ItemInfo::new("Kristal", 99));
    c.register("odun", ItemInfo::new("Odun", 20));
    c.register("tas", ItemInfo::new("Taş", 20));
    c
}

fn item_colour(item: &str) -> Vec4 {
    match item {
        "kristal" => Vec4::new(0.3, 0.85, 1.0, 1.0),
        "odun" => Vec4::new(0.62, 0.42, 0.24, 1.0),
        "tas" => Vec4::new(0.6, 0.6, 0.62, 1.0),
        _ => Vec4::new(0.8, 0.8, 0.8, 1.0),
    }
}

/// Bir hücreyi kurar: arazi (çarpıştırıcı ve ağ), ağaçlar, kayalar, toplanmamış kristaller.
fn load_chunk(world: &mut World, coord: ChunkCoord, collected: &HashSet<(i32, i32, u8)>, visual: bool) {
    let centre = ChunkManager::chunk_to_world_pos(coord);
    let n = SAMPLES as usize;
    let step = CHUNK_SIZE / (n - 1) as f32;
    let origin = Vec2::new(centre.x - CHUNK_SIZE * 0.5, centre.z - CHUNK_SIZE * 0.5);
    let mut heights = Vec::with_capacity(n * n);
    for row in 0..n {
        for col in 0..n {
            heights.push(height(origin.x + col as f32 * step, origin.y + row as f32 * step) / MAX_HEIGHT);
        }
    }
    let mut spawned = Vec::new();
    let ground = world.spawn_bundle((
        Transform::new(centre),
        GlobalTransform::default(),
        RigidBody::new_static(),
        Velocity::default(),
        gizmo::systems::terrain_collider(heights.clone(), SAMPLES, SAMPLES, CHUNK_SIZE, CHUNK_SIZE, MAX_HEIGHT)
            .with_material(PhysicsMaterial { static_friction: 0.9, dynamic_friction: 0.8, ..Default::default() }),
    ));
    spawned.push(ground);
    if visual {
        let mut triangles = AssetManager::terrain_triangles(&heights, SAMPLES, SAMPLES, CHUNK_SIZE, CHUNK_SIZE, MAX_HEIGHT);
        for v in &mut triangles {
            let (wx, wz) = (v.position[0] + centre.x, v.position[2] + centre.z);
            v.color = ground_colour(v.position[1], slope(wx, wz));
        }
        world.add_component(ground, MeshData::new(triangles));
        world.add_component(ground, MaterialDesc::new().with_pbr(Vec4::ONE, 0.95, 0.0));
        world.add_component(ground, MeshRenderer::new());
    }

    let mut rng = ChunkRng(hash(coord.0, coord.1, 99));
    // Ağaçlar: dik yamaçlarda ve karda değil. Her birinin hücredeki sırası kimliği (1..=22): kesilen
    // bir daha çıkmıyor. Sayılar kesilmiş olsa da çekiliyor, ki sonrakiler yerinden kaymasın.
    for k in 0..22u8 {
        let (x, z) = (origin.x + rng.next() * CHUNK_SIZE, origin.y + rng.next() * CHUNK_SIZE);
        let (h, s) = (height(x, z), slope(x, z));
        let scale = 0.8 + rng.next() * 0.7;
        let kind = rng.next();
        let id = (coord.0, coord.1, 1 + k);
        if s > 0.5 || h > 26.0 || Vec2::new(x, z).length() < 12.0 || collected.contains(&id) {
            continue;
        }
        let tree = spawn_tree(world, Vec3::new(x, h, z), scale, kind, visual);
        world.add_component(tree, Gatherable { id, item: "odun", amount: 3 });
        spawned.push(tree);
    }
    // Kayalar (100..).
    for k in 0..6u8 {
        let (x, z) = (origin.x + rng.next() * CHUNK_SIZE, origin.y + rng.next() * CHUNK_SIZE);
        let id = (coord.0, coord.1, 100 + k);
        let size = Vec3::new(0.6 + rng.next() * 1.4, 0.4 + rng.next() * 0.9, 0.6 + rng.next() * 1.4);
        let turn = Quat::from_rotation_y(rng.next() * std::f32::consts::TAU) * Quat::from_rotation_x(rng.next() * 0.4 - 0.2);
        if Vec2::new(x, z).length() < 12.0 || collected.contains(&id) {
            continue;
        }
        let rock = world.spawn_bundle((
            Transform::new(Vec3::new(x, height(x, z) + size.y * 0.4, z)).with_rotation(turn).with_scale(size),
            GlobalTransform::default(),
            RigidBody::new_static(),
            Velocity::default(),
            Collider::box_collider(size),
            Gatherable { id, item: "tas", amount: 2 },
        ));
        if visual {
            world.add_component(rock, MeshSource("standard_cube".into()));
            world.add_component(rock, MaterialDesc::new().with_pbr(Vec4::new(0.45, 0.44, 0.43, 1.0), 0.9, 0.0));
            world.add_component(rock, MeshRenderer::new());
        }
        spawned.push(rock);
    }
    // Kristaller: hücrelerin yarısında bir tane, toplanmadıysa.
    if rng.next() < 0.55 {
        let id = (coord.0, coord.1, 0u8);
        let (x, z) = (origin.x + 15.0 + rng.next() * (CHUNK_SIZE - 30.0), origin.y + 15.0 + rng.next() * (CHUNK_SIZE - 30.0));
        if !collected.contains(&id) && Vec2::new(x, z).length() > 8.0 {
            let crystal = world.spawn_bundle((
                Transform::new(Vec3::new(x, height(x, z) + 1.3, z)).with_scale(Vec3::new(0.35, 0.8, 0.35)),
                GlobalTransform::default(),
                Crystal { id },
            ));
            if visual {
                world.add_component(crystal, MeshSource("cone".into()));
                world.add_component(
                    crystal,
                    MaterialDesc::new().with_pbr(Vec4::new(0.3, 0.9, 1.0, 1.0), 0.15, 0.0).with_emissive(Vec3::new(0.4, 2.2, 2.8)),
                );
                world.add_component(crystal, MeshRenderer::new());
            }
            spawned.push(crystal);
        }
    }

    if let Some(mut manager) = world.get_resource_mut::<ChunkManager>() {
        for e in spawned {
            manager.register_entity(coord, e.to_bits());
        }
    }
}

fn spawn_tree(world: &mut World, at: Vec3, scale: f32, kind: f32, visual: bool) -> Entity {
    let trunk_h = 1.6 * scale;
    let tree = world.spawn_bundle((
        Transform::new(at + Vec3::Y * trunk_h * 0.5),
        GlobalTransform::default(),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::cylinder(0.22 * scale, trunk_h * 0.5),
    ));
    if visual {
        // Gövde: motorun silindiri 1 m yarıçaplı ve 2 m boyunda, ölçek yarıçap ve yarı boy.
        let trunk = world.spawn_bundle((
            Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.22 * scale, trunk_h * 0.5, 0.22 * scale)),
            GlobalTransform::default(),
            MeshSource("cylinder".into()),
            MaterialDesc::new().with_pbr(Vec4::new(0.36, 0.25, 0.16, 1.0), 0.9, 0.0),
            MeshRenderer::new(),
        ));
        world.add_child(tree, trunk);
        // Yaprakları: koni (çam) ya da küre (yapraklı), gövdenin çocuğu; gövdenin ölçeği yok, yerel uzay metre.
        let pine = kind < 0.5;
        let canopy = world.spawn_bundle((
            Transform::new(Vec3::Y * (trunk_h * 0.5 + if pine { 1.5 } else { 1.1 } * scale))
                .with_scale(if pine { Vec3::new(1.1, 1.8, 1.1) * scale } else { Vec3::splat(1.3 * scale) }),
            GlobalTransform::default(),
            MeshSource(if pine { "cone".into() } else { "sphere".into() }),
            MaterialDesc::new().with_pbr(if pine { Vec4::new(0.13, 0.33, 0.17, 1.0) } else { Vec4::new(0.27, 0.48, 0.18, 1.0) }, 0.85, 0.0),
            MeshRenderer::new(),
        ));
        world.add_child(tree, canopy);
    }
    tree
}

fn unload_chunk(world: &mut World, ids: Vec<u64>) {
    for bits in ids {
        if let Some(e) = world.live(Entity::from_bits(bits)) {
            world.despawn_recursive(e);
        }
    }
}

// =============================================================================================
// Envanter
// =============================================================================================

/// Oyuncunun envanterine `count` tane `item` koyar; hepsi sığdıysa `true`. Sığmayan kısmı geri
/// alınıyor, hiçbir şey yarım girmiyor.
fn give(world: &mut World, game: &mut Game, item: &str, count: u32) -> bool {
    let left = {
        let mut bags = world.query_mut::<&mut Inventory>();
        let Some(mut bag) = bags.get_mut(game.player) else { return false };
        let left = bag.add(&game.catalog, item, count);
        if left > 0 {
            bag.remove(item, count - left);
        }
        left
    };
    if left > 0 {
        game.note = ("Envanter dolu".into(), 1.5);
        return false;
    }
    game.note = (format!("+{count} {}", game.catalog.name(item)), 1.2);
    game.dirty = true;
    true
}

fn crystals(world: &World, game: &Game) -> u32 {
    world.get::<Inventory>(game.player).map_or(0, |b| b.count("kristal"))
}

/// `F`: en yakın ağacı keser, kayayı kırar.
fn gather(world: &mut World, game: &mut Game) {
    let me = focus(world, game);
    let near = world
        .query::<(Entity, &Gatherable, &Transform)>()
        .iter()
        .map(|(e, g, t)| (e, *g, Vec2::new(t.position.x - me.x, t.position.z - me.z).length()))
        .filter(|(_, _, d)| *d < 2.8)
        .min_by(|a, b| a.2.total_cmp(&b.2));
    let Some((entity, g, _)) = near else {
        game.note = ("Yakında ağaç ya da kaya yok".into(), 1.2);
        return;
    };
    if give(world, game, g.item, g.amount) {
        game.collected.insert(g.id);
        world.despawn_recursive(entity);
    }
}

/// Yerdeki eşyalar: yaklaşan alıyor (bırakılandan bir buçuk saniye sonra).
fn pick_up(world: &mut World, game: &mut Game, dt: f32) {
    let me = focus(world, game);
    let mut ready = Vec::new();
    {
        let mut pickups = world.query_mut::<(Entity, &mut Pickup, &Transform)>();
        for (e, mut p, t) in pickups.iter_mut() {
            p.ready_in -= dt;
            if p.ready_in <= 0.0 && t.position.distance(me) < 1.6 && game.driving.is_none() {
                ready.push((e, p.stack.clone()));
            }
        }
    }
    for (e, stack) in ready {
        if give(world, game, &stack.item, stack.count) {
            world.despawn_recursive(e);
        }
    }
}

/// Bir yığını oyuncunun önüne, yere bırakır.
fn drop_stack(world: &mut World, game: &mut Game, stack: ItemStack) {
    let t = world.get::<Transform>(game.player).copied().unwrap_or_default();
    let ahead = t.rotation * Vec3::new(0.0, 0.0, -1.6);
    let (x, z) = (t.position.x + ahead.x, t.position.z + ahead.z);
    let e = world.spawn_bundle((
        Transform::new(Vec3::new(x, height(x, z) + 0.35, z)).with_scale(Vec3::splat(0.25)),
        GlobalTransform::default(),
        Pickup { stack: stack.clone(), ready_in: 1.5 },
    ));
    if game.visual {
        world.add_component(e, MeshSource("standard_cube".into()));
        world.add_component(e, MaterialDesc::new().with_pbr(item_colour(&stack.item), 0.4, 0.0).with_emissive(item_colour(&stack.item).truncate() * 0.3));
        world.add_component(e, MeshRenderer::new());
    }
    game.note = (format!("{} ×{} yere bırakıldı", game.catalog.name(&stack.item), stack.count), 1.5);
    game.dirty = true;
}

/// Envanter paneli: ekranın ortasında altı sütunlu bir ızgara, her yuva bir bırakma hedefi ve içinde
/// sürüklenebilir eşyası. Kapalı başlıyor.
fn build_panel(world: &mut World, font: FontId) -> (Option<Entity>, Vec<Entity>, Vec<Entity>) {
    let root = world.spawn_bundle(NodeBundle {
        style: Style {
            display: Display::None,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        },
        background_color: BackgroundColor(Vec4::ZERO),
        ..Default::default()
    });
    let title = world.spawn_bundle(NodeBundle {
        style: Style { width: Val::Px(COLS as f32 * (CELL + 6.0) + 18.0), height: Val::Px(40.0), ..Default::default() },
        background_color: BackgroundColor(Vec4::new(0.08, 0.09, 0.12, 0.9)),
        ..Default::default()
    });
    world.add_component(title, Text::screen("Envanter — sürükle: taşı · dışarı bırak: yere at · I: kapat", font, 18.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
    world.add_child(root, title);
    let grid = world.spawn_bundle(NodeBundle {
        style: Style {
            display: Display::Grid,
            grid_template_columns: GridTracks::repeat(COLS, GridTrack::Px(CELL)),
            row_gap: Val::Px(6.0),
            column_gap: Val::Px(6.0),
            padding: UiRect::all(Val::Px(9.0)),
            ..Default::default()
        },
        background_color: BackgroundColor(Vec4::new(0.1, 0.11, 0.15, 0.88)),
        ..Default::default()
    });
    world.add_child(root, grid);
    let mut cells = Vec::with_capacity(SLOTS);
    let mut views = Vec::with_capacity(SLOTS);
    for _ in 0..SLOTS {
        let cell = world.spawn_bundle(NodeBundle {
            style: Style { width: Val::Px(CELL), height: Val::Px(CELL), padding: UiRect::all(Val::Px(4.0)), ..Default::default() },
            background_color: BackgroundColor(Vec4::new(0.2, 0.21, 0.26, 0.9)),
            ..Default::default()
        });
        world.add_component(cell, DropTarget::default());
        world.add_child(grid, cell);
        let view = world.spawn_bundle(ButtonBundle {
            style: Style { width: Val::Px(CELL - 8.0), height: Val::Px(CELL - 8.0), ..Default::default() },
            background_color: BackgroundColor(Vec4::ZERO),
            ..Default::default()
        });
        world.add_component(view, Drag::default());
        world.add_component(view, Text::screen("", font, 15.0, Vec2::ZERO).with_anchor(TextAnchor::Center).with_color(Vec4::new(0.05, 0.05, 0.06, 1.0)));
        world.add_child(cell, view);
        cells.push(cell);
        views.push(view);
    }
    (Some(root), cells, views)
}

fn set_fill(world: &mut World, e: Entity, colour: Vec4) {
    if let Some(mut bg) = world.query_mut::<&mut BackgroundColor>().get_mut(e) {
        if bg.0 != colour {
            bg.0 = colour;
        }
    }
}

/// Panel: aç/kapat, sürükle-bırak, yuvaları envanterden yeniden çiz.
fn update_panel(world: &mut World, game: &mut Game, toggle: bool) {
    let Some(root) = game.panel else { return };
    if toggle {
        if let Some(mut style) = world.query_mut::<&mut Style>().get_mut(root) {
            style.display = if style.display == Display::None { Display::Flex } else { Display::None };
        }
    }
    let open = world.get::<Style>(root).is_some_and(|s| s.display != Display::None);
    if !open {
        return;
    }
    // Bırakılanlar: bir yuvaya (taşı, birleştir, yer değiştir), ya da panelin dışına (yere at).
    let mut drops: Vec<(usize, Option<Entity>, f32)> = Vec::new();
    {
        let drags = world.query::<&Drag>();
        for (i, &view) in game.views.iter().enumerate() {
            if let Some(d) = drags.get(view) {
                if d.just_ended() {
                    drops.push((i, d.dropped_on(), d.delta().length()));
                }
            }
        }
    }
    for (from, target, moved) in drops {
        match target.and_then(|t| game.cells.iter().position(|c| *c == t)) {
            Some(to) => {
                let catalog = game.catalog.clone();
                let moved = {
                    let mut bags = world.query_mut::<&mut Inventory>();
                    bags.get_mut(game.player).is_some_and(|mut bag| bag.move_stack(&catalog, from, to))
                };
                game.dirty |= moved;
            }
            None if moved > CELL * 0.6 => {
                let stack = {
                    let mut bags = world.query_mut::<&mut Inventory>();
                    bags.get_mut(game.player).and_then(|mut b| b.take(from))
                };
                if let Some(stack) = stack {
                    drop_stack(world, game, stack);
                }
            }
            None => {}
        }
        let view = game.views[from];
        world.add_component(view, UiTransform::default());
        world.remove_component::<ZIndex>(view);
    }
    // Yuvalar: içindeki eşya, sürükleneni imleci izliyor, üstüne gelinen yuva aydınlık.
    let bag = world.get::<Inventory>(game.player).cloned().unwrap_or_default();
    for i in 0..SLOTS.min(game.views.len()) {
        let (view, cell) = (game.views[i], game.cells[i]);
        let drag = world.get::<Drag>(view).copied().unwrap_or_default();
        if drag.is_dragging() && bag.slot(i).is_some() {
            if drag.just_started() {
                world.add_component(view, ZIndex(50));
            }
            world.add_component(view, UiTransform { translation: drag.delta(), opacity: 1.0 });
        }
        let (colour, label) = match bag.slot(i) {
            Some(stack) => (item_colour(&stack.item), format!("{}\n×{}", game.catalog.name(&stack.item), stack.count)),
            None => (Vec4::ZERO, String::new()),
        };
        set_fill(world, view, colour);
        set_text(world, view, &label);
        let lit = world.get::<DropTarget>(cell).is_some_and(|t| t.hovered_by().is_some());
        set_fill(world, cell, if lit { Vec4::new(0.35, 0.45, 0.6, 0.95) } else { Vec4::new(0.2, 0.21, 0.26, 0.9) });
    }
}

/// Envanteri ve dünyadan alınanları kaydeder: bir değişiklikten en geç beş saniye sonra.
fn autosave(world: &World, game: &mut Game, dt: f32) {
    if !game.visual {
        return;
    }
    game.save_in -= dt;
    if !game.dirty || game.save_in > 0.0 {
        return;
    }
    game.save_in = 5.0;
    game.dirty = false;
    let mut save = SaveData::open("dunya");
    if let Some(bag) = world.get::<Inventory>(game.player) {
        let _ = save.set("envanter", bag);
    }
    let taken: Vec<(i32, i32, u8)> = game.collected.iter().copied().collect();
    let _ = save.set("alinan", &taken);
    if let Err(e) = save.save() {
        game.note = (format!("Kayıt yazılamadı: {e}"), 3.0);
    }
}

// =============================================================================================
// Oyuncu, arabalar, kamera
// =============================================================================================

struct Game {
    physics: FixedPhysics,
    player: Entity,
    cars: Vec<Entity>,
    driving: Option<Entity>,
    camera: Entity,
    sun: Entity,
    moon: Entity,
    hud: Entity,
    banner: Entity,
    collected: HashSet<(i32, i32, u8)>,
    /// Gece-gündüz: 0 gece yarısı, 0,25 sabah, 0,5 öğle.
    time_of_day: f32,
    cam_yaw: f32,
    cam_pitch: f32,
    visual: bool,
    /// Bu oyunda yüklenen ve silinen hücre sayısı (sınama için).
    loads: usize,
    unloads: usize,
    won: Option<f32>,
    clock: f32,
    catalog: ItemCatalog,
    /// Envanter paneli: kök (açık mı), yuvalar ve içlerindeki sürüklenebilir eşyalar.
    panel: Option<Entity>,
    cells: Vec<Entity>,
    views: Vec<Entity>,
    /// Ekranın ortasında kısa bir not ("Envanter dolu", "+3 Odun") ve kalan süresi.
    note: (String, f32),
    /// Bir sonraki kayda kalan süre; değişiklik olunca kısalıyor.
    save_in: f32,
    dirty: bool,
}

fn start_point() -> Vec3 {
    Vec3::new(0.0, height(0.0, 0.0) + 1.6, 0.0)
}

fn spawn_player(world: &mut World, at: Vec3, visual: bool) -> Entity {
    let player = world.spawn_bundle((
        Transform::new(at),
        GlobalTransform::default(),
        CharacterController { speed: 6.0, jump_speed: 6.5, ..Default::default() },
        Velocity::default(),
        Collider::capsule(0.35, 0.55),
        Inventory::new(SLOTS),
    ));
    if visual {
        let body = world.spawn_bundle((
            // Çarpıştırıcıyla aynı kapsül: yarıçap 0,35, gövdesi 1,1 m.
            Transform::new(Vec3::ZERO),
            GlobalTransform::default(),
            MeshSource("capsule_0.35_1.1_6_16".into()),
            MaterialDesc::new().with_pbr(Vec4::new(0.85, 0.35, 0.2, 1.0), 0.5, 0.0),
            MeshRenderer::new(),
        ));
        let face = world.spawn_bundle((
            Transform::new(Vec3::new(0.0, 0.55, -0.3)).with_scale(Vec3::new(0.18, 0.08, 0.06)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_unlit(Vec4::new(0.1, 0.1, 0.12, 1.0)),
            MeshRenderer::new(),
        ));
        world.add_child(player, body);
        world.add_child(player, face);
    }
    player
}

fn spawn_car(world: &mut World, at: Vec3, colour: Vec4, visual: bool) -> Entity {
    let collider = Collider::box_collider(Vec3::new(0.9, 0.35, 2.0));
    let mut body = RigidBody::new(900.0, true);
    body.update_inertia_from_collider(&collider);
    let mut car = VehicleController::four_wheeled(1.7, 2.8, 0.38);
    for wheel in &mut car.wheels {
        wheel.suspension_rest_length = 0.45;
        wheel.suspension_max_travel = 0.35;
    }
    car.max_steering_angle = 0.55;
    let id = world.spawn_bundle((Transform::new(at), GlobalTransform::default(), body, Velocity::default(), collider, car));
    if visual {
        for (offset, half, c) in [
            (Vec3::ZERO, Vec3::new(0.9, 0.35, 2.0), colour),
            (Vec3::new(0.0, 0.55, 0.3), Vec3::new(0.75, 0.25, 1.0), Vec4::new(0.15, 0.17, 0.22, 1.0)),
        ] {
            let part = world.spawn_bundle((
                Transform::new(offset).with_scale(half),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(c, 0.4, 0.3),
                MeshRenderer::new(),
            ));
            world.add_child(id, part);
        }
        for (x, z) in [(-0.9f32, -1.4f32), (0.9, -1.4), (-0.9, 1.4), (0.9, 1.4)] {
            let wheel = world.spawn_bundle((
                Transform::new(Vec3::new(x, -0.35, z)).with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)).with_scale(Vec3::new(0.38, 0.12, 0.38)),
                GlobalTransform::default(),
                MeshSource("cylinder".into()),
                MaterialDesc::new().with_pbr(Vec4::new(0.08, 0.08, 0.08, 1.0), 0.8, 0.0),
                MeshRenderer::new(),
            ));
            world.add_child(id, wheel);
        }
    }
    id
}

/// Sürülen araba ya da yayanın kendisi: hücreleri ve kamerayı süren konum.
fn focus(world: &World, game: &Game) -> Vec3 {
    let e = game.driving.unwrap_or(game.player);
    world.get::<Transform>(e).map(|t| t.position).unwrap_or(Vec3::ZERO)
}

/// Odağın **çizildiği** pozu: fizik 60 Hz'de adımlıyor, ekran daha sık tazeleniyor, ve araba iki adımın
/// arasında çiziliyor. Kamera ham pozu izlerse araba pürüzsüz giderken bütün dünya titriyor.
fn drawn_focus(world: &World, game: &Game) -> Transform {
    let e = game.driving.unwrap_or(game.player);
    gizmo::systems::interpolation::drawn_transform(world, e).unwrap_or_default()
}

/// Odak noktası değiştiyse hücreleri yükler, silinmesi gerekenleri siler.
fn stream_chunks(world: &mut World, game: &mut Game) {
    let at = focus(world, game);
    let (collected, visual) = (game.collected.clone(), game.visual);
    let (mut loads, mut unloads) = (0, 0);
    open_world_chunk_system(
        world,
        at,
        |w, coord| {
            load_chunk(w, coord, &collected, visual);
            loads += 1;
        },
        |w, _coord, ids| {
            unload_chunk(w, ids);
            unloads += 1;
        },
    );
    game.loads += loads;
    game.unloads += unloads;
}

fn setup(world: &mut World, font: FontId, visual: bool) -> Game {
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let sun = world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.88), 2.0, LightRole::Sun),
    ));
    let moon = world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(0.55, 0.65, 1.0), 0.0, LightRole::Generic),
    ));
    let camera = world.spawn_bundle(CameraBundle { position: Vec3::new(0.0, 12.0, 14.0), primary: true, fov: 1.0, far: 600.0, ..Default::default() });
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    let banner = world.spawn_bundle((Text::pinned_at("", font, 46.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
        .with_anchor(TextAnchor::Center)
        .with_align(TextAlign::Center),));
    let player = spawn_player(world, start_point(), visual);
    // Kayıtlı oyun: envanter ve dünyadan alınanlar (yalnız pencereli oyunda; sınama kayda dokunmuyor).
    let mut collected = HashSet::new();
    if visual {
        let save = SaveData::open("dunya");
        if let Some(bag) = save.get::<Inventory>("envanter") {
            world.add_component(player, bag);
        }
        if let Some(taken) = save.get::<Vec<(i32, i32, u8)>>("alinan") {
            collected.extend(taken);
        }
    }
    let (panel, cells, views) = if visual { build_panel(world, font) } else { (None, Vec::new(), Vec::new()) };
    // Ekran görüntüsü için: panel açık, içinde birkaç eşya.
    if let (Some(root), true) = (panel, std::env::var_os("DUNYA_ENVANTER_ACIK").is_some()) {
        if let Some(mut style) = world.query_mut::<&mut Style>().get_mut(root) {
            style.display = Display::Flex;
        }
        let c = catalog();
        if let Some(mut bag) = world.query_mut::<&mut Inventory>().get_mut(player) {
            bag.add(&c, "kristal", 3);
            bag.add(&c, "odun", 27);
            bag.add(&c, "tas", 8);
        }
    }
    let mut game = Game {
        physics: FixedPhysics::new(60),
        player,
        cars: Vec::new(),
        driving: None,
        camera,
        sun,
        moon,
        hud,
        banner,
        collected,
        time_of_day: 0.3,
        cam_yaw: 0.0,
        cam_pitch: 0.35,
        visual,
        loads: 0,
        unloads: 0,
        won: None,
        clock: 0.0,
        catalog: catalog(),
        panel,
        cells,
        views,
        note: (String::new(), 0.0),
        save_in: 5.0,
        dirty: false,
    };
    stream_chunks(world, &mut game);
    for (k, (x, z)) in [(6.0f32, 3.0f32), (-7.0, 5.0), (3.0, -8.0)].into_iter().enumerate() {
        let colour = [Vec4::new(0.85, 0.3, 0.1, 1.0), Vec4::new(0.15, 0.45, 0.85, 1.0), Vec4::new(0.9, 0.8, 0.2, 1.0)][k];
        game.cars.push(spawn_car(world, Vec3::new(x, height(x, z) + 1.4, z), colour, visual));
    }
    game
}

/// Kameranın baktığı yatay yön (ileri) ve sağı.
fn camera_axes(game: &Game) -> (Vec3, Vec3) {
    let forward = Vec3::new(-game.cam_yaw.sin(), 0.0, -game.cam_yaw.cos());
    (forward, Vec3::new(-forward.z, 0.0, forward.x))
}

fn enter_or_leave(world: &mut World, game: &mut Game) {
    if let Some(car) = game.driving.take() {
        // İn: arabanın soluna, arazinin üstüne.
        let t = world.get::<Transform>(car).copied().unwrap_or_default();
        let side = t.rotation * Vec3::new(-2.2, 0.0, 0.0);
        let at = Vec3::new(t.position.x + side.x, 0.0, t.position.z + side.z);
        let at = Vec3::new(at.x, height(at.x, at.z) + 1.4, at.z);
        if let Some(mut p) = world.query_mut::<&mut Transform>().get_mut(game.player) {
            p.position = at;
        }
        world.add_component(game.player, Collider::capsule(0.35, 0.55));
        world.add_component(game.player, CharacterController { speed: 6.0, jump_speed: 6.5, ..Default::default() });
        world.remove_component::<IsHidden>(game.player);
        hide_children(world, game.player, false);
        if let Some(mut c) = world.query_mut::<&mut VehicleController>().get_mut(car) {
            c.throttle_input = 0.0;
            c.steering_input = 0.0;
            c.brake_input = 1.0;
        }
        return;
    }
    let me = focus(world, game);
    let near = game
        .cars
        .iter()
        .copied()
        .filter_map(|c| world.get::<Transform>(c).map(|t| (c, t.position.distance(me))))
        .filter(|(_, d)| *d < 3.5)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((car, _)) = near {
        game.driving = Some(car);
        world.remove_component::<Collider>(game.player);
        world.remove_component::<CharacterController>(game.player);
        world.add_component(game.player, IsHidden);
        hide_children(world, game.player, true);
    }
}

fn hide_children(world: &mut World, parent: Entity, hidden: bool) {
    let children: Vec<Entity> = world.get::<gizmo::core::component::Children>(parent).map(|c| c.0.clone()).unwrap_or_default();
    for c in children {
        if hidden {
            world.add_component(c, IsHidden);
        } else {
            world.remove_component::<IsHidden>(c);
        }
    }
}

/// Bu karede ne istendiği: oyuncudan ya da otomatik pilottan.
#[derive(Default, Clone, Copy)]
struct Controls {
    mx: f32,
    my: f32,
    jump: bool,
    use_car: bool,
    gather: bool,
    yaw: f32,
    pitch: f32,
    reset: bool,
}

fn read_controls(input: &Input) -> Controls {
    let (mx, my) = input.move_axis();
    let mut yaw = 0.0;
    if input.is_mouse_button_pressed(1) {
        yaw -= input.mouse_delta().0 * 0.005;
    }
    if input.is_key_pressed(KeyCode::ArrowLeft as u32) {
        yaw += 0.03;
    }
    if input.is_key_pressed(KeyCode::ArrowRight as u32) {
        yaw -= 0.03;
    }
    let mut pitch = 0.0;
    if input.is_key_pressed(KeyCode::ArrowUp as u32) {
        pitch += 0.015;
    }
    if input.is_key_pressed(KeyCode::ArrowDown as u32) {
        pitch -= 0.015;
    }
    Controls {
        mx,
        my,
        jump: input.is_key_just_pressed(KeyCode::Space as u32),
        use_car: input.is_key_just_pressed(KeyCode::KeyE as u32),
        gather: input.is_key_just_pressed(KeyCode::KeyF as u32),
        yaw,
        pitch,
        reset: input.is_key_just_pressed(KeyCode::KeyR as u32),
    }
}

fn step(world: &mut World, game: &mut Game, dt: f32, c: Controls) {
    game.clock += dt;
    game.time_of_day = (game.time_of_day + dt / DAY_LENGTH).fract();
    game.cam_yaw += c.yaw;
    game.cam_pitch = (game.cam_pitch + c.pitch).clamp(0.05, 1.2);
    if c.reset {
        if let Some(car) = game.driving {
            game.driving = None;
            let _ = car;
            world.add_component(game.player, Collider::capsule(0.35, 0.55));
            world.add_component(game.player, CharacterController { speed: 6.0, jump_speed: 6.5, ..Default::default() });
            world.remove_component::<IsHidden>(game.player);
            hide_children(world, game.player, false);
        }
        if let Some(mut p) = world.query_mut::<&mut Transform>().get_mut(game.player) {
            p.position = start_point();
        }
    }
    if c.use_car {
        enter_or_leave(world, game);
    }
    if c.gather && game.driving.is_none() {
        gather(world, game);
    }
    let (forward, right) = camera_axes(game);
    match game.driving {
        Some(car) => {
            if let Some(mut v) = world.query_mut::<&mut VehicleController>().get_mut(car) {
                v.throttle_input = c.my.max(0.0);
                v.brake_input = (-c.my).max(0.0);
                // Motorun direksiyonunun artısı sola (`VehicleController::steering_input`), D tuşu +1.
                v.steering_input = -c.mx;
            }
            // Kamera arabanın arkasına yavaşça dönüyor.
            if c.yaw == 0.0 {
                if let Some(t) = gizmo::systems::interpolation::drawn_transform(world, car) {
                    let heading = t.rotation * Vec3::NEG_Z;
                    let want = (-heading.x).atan2(-heading.z);
                    let mut d = want - game.cam_yaw;
                    d = (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                    game.cam_yaw += d * (dt * 2.0).min(1.0);
                }
            }
        }
        None => {
            let wish = (forward * c.my + right * c.mx).clamp_length_max(1.0);
            if let Some(mut k) = world.query_mut::<&mut CharacterController>().get_mut(game.player) {
                k.target_velocity = wish * k.speed;
                if c.jump {
                    k.jump_buffer_timer = k.jump_buffer_time.max(0.1);
                }
            }
            if wish.length_squared() > 0.01 {
                if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(game.player) {
                    let want = Quat::from_rotation_arc(Vec3::NEG_Z, wish.normalize());
                    t.rotation = t.rotation.slerp(want, (dt * 10.0).min(1.0));
                }
            }
        }
    }
    stream_chunks(world, game);
    game.physics.advance(world, dt.min(0.1));
    world.apply_commands();
    // Kristaller dönüyor; yaklaşan topluyor.
    let me = focus(world, game);
    let mut taken = Vec::new();
    {
        let spin = Quat::from_rotation_y(dt * 1.5);
        let mut crystals = world.query_mut::<(Entity, &Crystal, &mut Transform)>();
        for (e, crystal, mut t) in crystals.iter_mut() {
            t.rotation = spin * t.rotation;
            if t.position.distance(me) < PICKUP + if game.driving.is_some() { 1.5 } else { 0.0 } {
                taken.push((e, crystal.id));
            }
        }
    }
    for (e, id) in taken {
        if give(world, game, "kristal", 1) {
            game.collected.insert(id);
            world.despawn_recursive(e);
        }
    }
    pick_up(world, game, dt);
    if game.won.is_none() && crystals(world, game) >= GOAL as u32 {
        game.won = Some(game.clock);
    }
    game.note.1 -= dt;
    sync_transforms(world);
}

/// Gece-gündüz: güneşin ve ayın yönü, şiddeti; gök, sis ve arka plan rengi.
fn sky_at(time_of_day: f32) -> (Vec3, f32, f32, [f32; 3], [f32; 3], [f32; 3]) {
    let angle = (time_of_day - 0.25) * std::f32::consts::TAU;
    // Güneşin ışığı nereye gidiyor: sabah doğudan, öğlen tepeden, akşam batıdan.
    let shines = -Vec3::new(angle.cos(), angle.sin(), 0.35).normalize();
    let elevation = angle.sin();
    let day = ((elevation + 0.08) / 0.3).clamp(0.0, 1.0);
    let dusk = (1.0 - (elevation.abs() / 0.25)).clamp(0.0, 1.0);
    let zenith = Vec3::new(0.02, 0.03, 0.08).lerp(Vec3::new(0.25, 0.45, 0.85), day);
    let mut horizon = Vec3::new(0.03, 0.04, 0.09).lerp(Vec3::new(0.52, 0.66, 0.86), day);
    horizon = horizon.lerp(Vec3::new(0.95, 0.5, 0.25), dusk * 0.7);
    let ground = horizon * 0.35;
    (shines, 2.2 * day, 0.35 * (1.0 - day), zenith.to_array(), horizon.to_array(), ground.to_array())
}

fn place_lights_and_camera(world: &mut World, game: &Game, dt: f32) {
    let (shines, sun_i, moon_i, _, horizon, _) = sky_at(game.time_of_day);
    if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(game.sun) {
        t.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, shines);
    }
    if let Some(mut l) = world.query_mut::<&mut DirectionalLight>().get_mut(game.sun) {
        l.intensity = sun_i;
    }
    if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(game.moon) {
        t.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::new(0.3, -0.8, 0.5).normalize());
    }
    if let Some(mut l) = world.query_mut::<&mut DirectionalLight>().get_mut(game.moon) {
        l.intensity = moon_i;
    }
    // Üçüncü şahıs kamera: odağın arkasında, arazinin üstünde.
    let target = drawn_focus(world, game).position + Vec3::Y * 1.4;
    let distance = if game.driving.is_some() { 11.0 } else { 7.0 };
    let back = Vec3::new(game.cam_yaw.sin() * game.cam_pitch.cos(), game.cam_pitch.sin(), game.cam_yaw.cos() * game.cam_pitch.cos());
    let mut want = target + back * distance;
    want.y = want.y.max(height(want.x, want.z) + 1.0);
    if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(game.camera) {
        t.position = t.position.lerp(want, (dt * 8.0).min(1.0));
        let look = (target - t.position).normalize_or_zero();
        if look != Vec3::ZERO {
            t.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, look);
        }
    }
    // Kameranın baktığı yön `Camera`'nın yaw/pitch'inden okunuyor, `Transform`'un dönüşünden değil
    // (`Camera::get_front`): ikisi de aynı yöne.
    let look = world.get::<Transform>(game.camera).map(|t| (target - t.position).normalize_or_zero()).unwrap_or(Vec3::NEG_Z);
    if let Some(mut cam) = world.query_mut::<&mut Camera>().get_mut(game.camera) {
        cam.clear_color = Some(Vec3::from(horizon));
        if look != Vec3::ZERO {
            cam.yaw = look.z.atan2(look.x);
            cam.pitch = look.y.clamp(-1.0, 1.0).asin();
        }
    }
}

fn draw_hud(world: &mut World, game: &Game) {
    let me = focus(world, game);
    let nearest = world
        .query::<(&Crystal, &Transform)>()
        .iter()
        .map(|(_, t)| t.position)
        .min_by(|a, b| a.distance(me).total_cmp(&b.distance(me)));
    let (forward, right) = camera_axes(game);
    let arrows = ["↑", "↗", "→", "↘", "↓", "↙", "←", "↖"];
    let pointer = nearest.map_or("—".to_string(), |p| {
        let d = Vec3::new(p.x - me.x, 0.0, p.z - me.z);
        let angle = d.dot(right).atan2(d.dot(forward));
        let k = ((angle / std::f32::consts::TAU * 8.0).round() as i32).rem_euclid(8) as usize;
        format!("{} {:.0} m", arrows[k], d.length())
    });
    let hours = game.time_of_day * 24.0;
    let speed = game.driving.and_then(|c| world.get::<VehicleController>(c).map(|v| v.current_speed_kmh));
    let mode = match speed {
        Some(s) => format!("arabada, {s:.0} km/s · E: in"),
        None => "yaya · E: arabaya bin (yakındaysa)".to_string(),
    };
    set_text(
        world,
        game.hud,
        &format!(
            "Kristal {}/{GOAL}   odun {}   taş {}   en yakın kristal: {pointer}   saat {:02}:{:02}\n{mode}\nWASD: yürü/sür · boşluk: zıpla · F: kes/kır · I: envanter · sağ tık+fare / oklar: kamera · R: başlangıç",
            crystals(world, game),
            world.get::<Inventory>(game.player).map_or(0, |b| b.count("odun")),
            world.get::<Inventory>(game.player).map_or(0, |b| b.count("tas")),
            hours as u32,
            ((hours.fract()) * 60.0) as u32
        ),
    );
    let banner = match game.won {
        Some(t) => format!("On beş kristal: {:.0} dakika {:.0} saniye", (t / 60.0).floor(), t % 60.0),
        None if game.note.1 > 0.0 => game.note.0.clone(),
        None => String::new(),
    };
    set_text(world, game.banner, &banner);
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    step(world, &mut game, dt, read_controls(input));
    let toggle = input.is_key_just_pressed(KeyCode::KeyI as u32) || input.is_key_just_pressed(KeyCode::Tab as u32);
    update_panel(world, &mut game, toggle);
    autosave(world, &mut game, dt);
    place_lights_and_camera(world, &game, dt);
    draw_hud(world, &game);
    world.insert_resource(game);
}

/// Oyunun tamamı bir `App` olarak.
pub fn app() -> App<SimpleSceneState> {
    App::<SimpleSceneState>::new("Gizmo Engine - Dünya", 1280, 800)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, _state| {
            let font = scene.renderer.default_font();
            let game = setup(scene.world, font, true);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        // Gök, sis: günün saatine göre, oyunun kaynağından her senkron noktasında.
        .set_render_stage(RenderWorld::new().with_settings(|game, renderer| {
            if let Some(g) = game.get_resource::<Game>() {
                let (_, _, _, zenith, horizon, ground) = sky_at(g.time_of_day);
                renderer.sky = Some(Sky::gradient(zenith, horizon, ground));
                renderer.fog = Some(Fog::new(horizon, 0.0016).with_height(0.03, 0.0));
            }
        }))
}

// =============================================================================================
// Sınama
// =============================================================================================

/// En yakın kristale doğru yürüyen ya da süren otomatik pilot.
fn autopilot(world: &World, game: &Game, drive_straight: bool) -> Controls {
    let me = focus(world, game);
    if drive_straight {
        return Controls { my: 0.8, mx: 0.0, ..Default::default() };
    }
    let target = world
        .query::<(&Crystal, &Transform)>()
        .iter()
        .map(|(_, t)| t.position)
        .min_by(|a, b| a.distance(me).total_cmp(&b.distance(me)));
    let Some(target) = target else { return Controls { my: 1.0, ..Default::default() } };
    let (forward, right) = camera_axes(game);
    let d = Vec3::new(target.x - me.x, 0.0, target.z - me.z).normalize_or_zero();
    // Yolda, erişimde bir ağaç ya da kaya varsa kes — bir oyuncunun yapacağı gibi.
    let gather = world
        .query::<(&Gatherable, &Transform)>()
        .iter()
        .any(|(_, t)| Vec2::new(t.position.x - me.x, t.position.z - me.z).length() < 2.6);
    Controls { mx: d.dot(right), my: d.dot(forward), jump: game.clock % 1.5 < 0.02, gather, ..Default::default() }
}

/// `--test`: iki dakika yürüyerek kristal topla, sonra arabayla bir dakika hücre sınırlarını geç.
pub fn self_test() -> bool {
    let mut world = World::new();
    let game = setup(&mut world, HeadlessFrames::font(), false);
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    let mut deepest_walk = f32::MAX;
    let mut deepest_drive = f32::MAX;
    let mut walked_far = 0.0f32;
    let mut drove_far = 0.0f32;
    for frame in 0..(60 * 180) {
        let driving_phase = frame >= 60 * 120;
        run.frame(dt, |w, _s, dt, _i| {
            let Some(mut game) = w.remove_resource::<Game>() else { return };
            if driving_phase && game.driving.is_none() {
                // Arabaya: en yakını yanına getir, bin.
                let me = focus(w, &game);
                let car = game.cars[0];
                if let Some(mut t) = w.query_mut::<&mut Transform>().get_mut(car) {
                    t.position = Vec3::new(me.x + 2.0, height(me.x + 2.0, me.z) + 1.4, me.z);
                    t.rotation = Quat::IDENTITY;
                }
                enter_or_leave(w, &mut game);
            }
            let controls = autopilot(w, &game, driving_phase);
            step(w, &mut game, dt, controls);
            w.insert_resource(game);
        });
        let game = run.world.get_resource::<Game>().expect("oyun");
        let p = focus(&run.world, &game);
        if !p.is_finite() {
            deepest_walk = f32::MIN;
            break;
        }
        let above = p.y - height(p.x, p.z);
        if frame > 60 {
            if game.driving.is_some() {
                deepest_drive = deepest_drive.min(above);
                drove_far = drove_far.max(Vec2::new(p.x, p.z).length());
            } else {
                deepest_walk = deepest_walk.min(above);
                walked_far = walked_far.max(Vec2::new(p.x, p.z).length());
            }
        }
    }
    let game = run.world.get_resource::<Game>().expect("oyun");
    let bag = run.world.get::<Inventory>(game.player).cloned().unwrap_or_default();
    let crystals_taken = game.collected.iter().filter(|id| id.2 == 0).count() as u32;
    let (wood, stone) = (bag.count("odun"), bag.count("tas"));
    let ok = deepest_walk > -0.5
        && deepest_drive > -0.5
        && crystals_taken >= 2
        && bag.count("kristal") == crystals_taken
        && wood + stone > 0
        && game.unloads > 0
        && game.driving.is_some();
    println!(
        "dunya --test: envanterde {} kristal (dünyadan {crystals_taken} alındı), {wood} odun, {stone} taş; hücre {} yüklendi, {} silindi; yürüyüşte başlangıçtan {:.0} m, arabayla {:.0} m; zeminin altına en çok yayada {:.2} m, arabada {:.2} m → {}",
        bag.count("kristal"),
        game.loads,
        game.unloads,
        walked_far,
        drove_far,
        -deepest_walk.min(0.0),
        -deepest_drive.min(0.0),
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}
