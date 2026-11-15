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
//! | **E** | arabaya ya da kayığa bin / in |
//! | **F** | yakındaki ağacı kes (odun), kayayı kır (taş) |
//! | **I · Tab** | envanter: eşyayı yuvadan yuvaya sürükle, panelin dışına bırakırsan yere düşer; alttaki tariflerle üret |
//! | **T** | yakındaki köylüyle konuş: iş al, getirdiğini teslim et |
//! | **B** | inşa: kamp ateşi, çit, köprü, barınak (envanterdekiler) — **G · sol tık** koy, **Q** döndür, **B** sonraki |
//! | **sağ tık + fare · ← →** | kamerayı döndür |
//! | **↑ ↓** | kamerayı yükselt / alçalt |
//! | **M** | küçük haritayı aç / kapat |
//! | **H** | havayı değiştir (açık → bulutlu → yağmurlu → sisli…) |
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
//! - **Üretim ve inşa** (`insa`): odun ve taştan kamp ateşi, çit, köprü, barınak — motorun
//!   `Recipe` ve `Inventory::craft`'ı; kurulan yapılar katı, kamp ateşi titreyen bir ışık, hepsi kayıtta.
//! - **Canlılar ve köyler** (`canli`): koyunlar ve geyikler otluyor, yaklaşınca kaçıyor; bazı
//!   hücrelerde üç barınaklı bir köy ve iş isteyen iki köylü — hepsi motorun davranış ağaçlarıyla.
//! - **Göller ve kayık** (`su`): arazideki alçak çanaklar su seviyesinin altında kalınca göl; su
//!   motorun tek bir `FluidZone`'u, kayık motorun `BoatController`'ı, oyuncu suda yüzüyor.
//! - **Harita, hava, ses** (`harita`, `hava`): sağ üstte yukarıdan bakan ikinci bir kamera ve
//!   üstünde kristal, köy, kayık işaretleri; açık / bulutlu / yağmurlu / sisli gök, yağmur GPU
//!   parçacıkları ve kaygan yol; sentezlenmiş sesler, yağmurun ve motorun döngüleri.
//! - **Gece-gündüz** dört dakikada bir tur: güneş dönüyor, gök, sis ve arka plan rengi ona göre;
//!   geceleri ay ışığı ve kristallerin kendi ışıltısı.
//!
//! `dunya --test` pencere açmaz: yaya otomatik pilot iki dakika en yakın kristale yürür, sonra
//! arabaya binip hücre sınırlarının üstünden bir dakika sürer. Hiçbir an oyuncu ya da araba
//! arazinin içine düşmemeli, hücreler yüklenip silinmeli ve kristal toplanmalı.

mod canli;
mod harita;
mod hava;
mod insa;
mod karakter;
mod modeller;
mod olcum;
mod su;

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

/// Bir hücrenin kenarındaki örnek sayısı: 64 aralık, 1,5625 m'de bir. (32 aralıkta, 3 m'lik
/// üçgenlerin her biri tek bir renkti; yakından bakınca zemin düz ve bulanık görünüyordu.)
const SAMPLES: u32 = 65;
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
    ((hills * 14.0 + mountains * 34.0 * calm) + 2.0 - su::basin(x, z)).clamp(0.0, MAX_HEIGHT)
}

/// Eğim: yüksekliğin değişimi, metrede metre.
fn slope(x: f32, z: f32) -> f32 {
    let e = 1.5;
    Vec2::new(height(x + e, z) - height(x - e, z), height(x, z + e) - height(x, z - e)).length() / (2.0 * e)
}

/// Dokulu arazinin köşe rengi: dokuların ağırlıkları — kırmızı toprak, yeşil kaya, mavi kar; kalanı
/// çim (`arazi.wgsl`): yükseldikçe toprak, dik yamaçta ve yüksekte kaya, en tepede kar; çimin içinde
/// yer yer çıplak toprak lekeleri. Kıyının kumu gölgelendiricide, su seviyesinden.
fn ground_weights(h: f32, s: f32, x: f32, z: f32) -> [f32; 4] {
    let patch = value_noise(x / 7.0, z / 7.0, 61);
    let fine = value_noise(x / 2.2, z / 2.2, 62);
    let bare = ((fine * patch - 0.5) / 0.15).clamp(0.0, 1.0);
    let mut dirt = ((h - 14.0) / 8.0).clamp(0.0, 1.0).max(bare);
    let rock = ((s - 0.45) / 0.35).clamp(0.0, 1.0).max(((h - 24.0) / 6.0).clamp(0.0, 1.0));
    let snow = ((h - 33.0) / 4.0).clamp(0.0, 1.0);
    // Kar kayanın, kaya toprağın üstüne: ağırlıklar toplamda bir.
    let rock = rock * (1.0 - snow);
    dirt *= 1.0 - snow - rock;
    [dirt, rock, snow, 1.0]
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
    insa::register_items(&mut c);
    c
}

fn item_colour(item: &str) -> Vec4 {
    if let Some(c) = insa::item_colour(item) {
        return c;
    }
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
    let lowest = heights.iter().fold(f32::MAX, |a, h| a.min(*h)) * MAX_HEIGHT;
    spawned.extend(su::surface(world, coord, lowest, visual));
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
            v.color = ground_weights(v.position[1], slope(wx, wz), wx, wz);
        }
        world.add_component(ground, MeshData::new(triangles));
        // Dokulu arazi (köşe rengi dokuların ağırlığı): pencereli oyun gölgelendiricisini kurarken kaydetti.
        let look = world.get_resource::<modeller::GroundLook>().map(|g| g.0.clone()).expect("the windowed game registers the ground's surface first");
        world.add_component(ground, look);
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
        if s > 0.5 || h > 26.0 || su::wet(x, z, 0.4) || Vec2::new(x, z).length() < 12.0 || collected.contains(&id) {
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
        if Vec2::new(x, z).length() < 12.0 || su::wet(x, z, 0.0) || collected.contains(&id) {
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
            // Kaya modeli çarpıştırıcının kutusunu dolduruyor: kayanın kendi uzayı ±1 (ölçeği
            // `size`), modelin dibi kutunun dibinde.
            let (model, half) = modeller::ROCKS[(hash(coord.0, coord.1, 700 + k as u32) % 3) as usize];
            let fit = Vec3::new(1.0 / half.x, 2.0 / half.y, 1.0 / half.z);
            modeller::place(world, rock, model, Vec3::Y * -1.0, Quat::IDENTITY, fit);
        }
        spawned.push(rock);
    }
    // Kristaller: hücrelerin yarısında bir tane, toplanmadıysa.
    if rng.next() < 0.55 {
        let id = (coord.0, coord.1, 0u8);
        let (x, z) = (origin.x + 15.0 + rng.next() * (CHUNK_SIZE - 30.0), origin.y + 15.0 + rng.next() * (CHUNK_SIZE - 30.0));
        if !collected.contains(&id) && Vec2::new(x, z).length() > 8.0 && !su::wet(x, z, 0.3) {
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

    canli::populate_chunk(world, coord, visual, &mut spawned);
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
        // Modeli: iki çeşit, 4–6,5 m boyunda (`scale` 0,8–1,5), dibi toprakta, her biri kendi
        // yönüne dönük. Çarpıştırıcı gövde, silindir.
        let model = modeller::TREES[usize::from(kind >= 0.5)];
        let size = 4.4 * scale / modeller::TREE_HEIGHT;
        let turn = Quat::from_rotation_y(kind * 37.0);
        modeller::place(world, tree, model, Vec3::Y * -(trunk_h * 0.5), turn, Vec3::splat(size));
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
        game.sounds.push(if g.item == "tas" { "tas" } else { "odun" });
        karakter::act(world, game.player, "PickUp");
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
    insa::build_recipes(world, root, font);
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
    insa::update_recipes(world, game);
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
    let _ = save.set("yapilar", &insa::saved(world));
    let quests: Vec<((i32, i32, u8), u8)> = game.quests.iter().map(|(k, v)| (*k, *v)).collect();
    let _ = save.set("isler", &quests);
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
    /// FPS göstergesi ve darboğaz dökümü (F3).
    meter: olcum::Meter,
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
    /// İnşa kipi (B), açıksa.
    building: Option<insa::Building>,
    /// Köylülerin işleri: 1 verildi, 2 bitti.
    quests: std::collections::HashMap<(i32, i32, u8), u8>,
    /// Göllerin kayıkları, ve hangi hücrelere kayık kondu.
    boats: Vec<Entity>,
    boat_chunks: HashSet<ChunkCoord>,
    weather: hava::Weather,
    /// Kameranın izlediği yükseklik, yumuşatılmış.
    cam_focus_y: Option<f32>,
    map: harita::Map,
    /// Bu kare çalınacak sesler (adlarıyla), ve sesler (pencereli oyunda).
    sounds: Vec<&'static str>,
    audio: Option<hava::Sounds>,
}

fn start_point() -> Vec3 {
    Vec3::new(0.0, height(0.0, 0.0) + 1.6, 0.0)
}

fn spawn_player(world: &mut World, at: Vec3) -> Entity {
    let player = world.spawn_bundle((
        Transform::new(at),
        GlobalTransform::default(),
        CharacterController { speed: 6.0, jump_speed: 6.5, ..Default::default() },
        Velocity::default(),
        Collider::capsule(0.35, 0.55),
        Inventory::new(SLOTS),
    ));
    // Şövalye: ayakları kapsülün dibinde (yarıçap 0,35 + yarı boy 0,55), boyu 1,8 m. Penceresiz
    // sınamada da giydiriliyor — yalnızca veri; model kurulmuyor, klip seçimi yine sınanıyor.
    karakter::dress(world, player, karakter::KNIGHT, 0.9, 1.8, &karakter::PERSON);
    player
}

/// Bir araba: KayKit'in arabalarından biri (`model`), 4,6 kat büyük — 4,3 m boyunda. Fizik modelin
/// tekerleklerine göre: iz 1,62 m, dingil arası 2,3 m, tekerlek yarıçapı 0,33 m; böylece tekerlekler
/// çamurluklarının içinde dönüyor, süspansiyonla iniyor, direksiyonla kırılıyor (motorun
/// `WheelVisual`'ı, model kurulunca [`modeller::bind_wheels`] bağlıyor).
fn spawn_car(world: &mut World, at: Vec3, model: &str, visual: bool) -> Entity {
    let collider = Collider::box_collider(Vec3::new(0.9, 0.35, 2.0));
    let mut body = RigidBody::new(900.0, true);
    body.update_inertia_from_collider(&collider);
    let mut car = VehicleController::four_wheeled(1.62, 2.3, 0.33);
    for wheel in &mut car.wheels {
        wheel.suspension_rest_length = 0.45;
        wheel.suspension_max_travel = 0.35;
    }
    car.max_steering_angle = 0.55;
    let id = world.spawn_bundle((Transform::new(at), GlobalTransform::default(), body, Velocity::default(), collider, car));
    if visual {
        // Modelin tekerlek merkezi kendi biriminde 0,011 yukarıda; süspansiyon dinlenirken tekerlek
        // gövde merkezinin 0,45 m altında: modelin kökü −0,45 − 0,05.
        modeller::place(world, id, model, Vec3::new(0.0, -0.5, 0.0), Quat::from_rotation_y(std::f32::consts::PI), Vec3::splat(modeller::CAR_SCALE));
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
    stream_chunks_only(world, game);
    su::place_boats(world, game);
}

fn stream_chunks_only(world: &mut World, game: &mut Game) {
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
    let mut physics = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    su::install(&mut physics);
    world.insert_resource(physics);
    canli::install(world);
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
    let meter = olcum::install(world, font);
    let banner = world.spawn_bundle((Text::pinned_at("", font, 46.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
        .with_anchor(TextAnchor::Center)
        .with_align(TextAlign::Center),));
    let player = spawn_player(world, start_point());
    // Kayıtlı oyun: envanter ve dünyadan alınanlar (yalnız pencereli oyunda; sınama kayda dokunmuyor).
    let mut collected = HashSet::new();
    let mut quests = std::collections::HashMap::new();
    if visual {
        let save = SaveData::open("dunya");
        if let Some(bag) = save.get::<Inventory>("envanter") {
            world.add_component(player, bag);
        }
        if let Some(taken) = save.get::<Vec<(i32, i32, u8)>>("alinan") {
            collected.extend(taken);
        }
        if let Some(built) = save.get::<Vec<(u8, f32, f32, f32, f32)>>("yapilar") {
            insa::restore(world, &built, true);
        }
        if let Some(saved) = save.get::<Vec<((i32, i32, u8), u8)>>("isler") {
            quests.extend(saved);
        }
    }
    let (panel, cells, views) = if visual { build_panel(world, font) } else { (None, Vec::new(), Vec::new()) };
    let map = harita::build(world, font);
    let audio = if visual { hava::install(world) } else { None };
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
        meter,
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
        building: None,
        quests,
        boats: Vec::new(),
        boat_chunks: HashSet::new(),
        weather: hava::Weather::new(),
        cam_focus_y: None,
        map,
        sounds: Vec::new(),
        audio,
    };
    stream_chunks(world, &mut game);
    for (k, (x, z)) in [(6.0f32, 3.0f32), (-7.0, 5.0), (3.0, -8.0)].into_iter().enumerate() {
        game.cars.push(spawn_car(world, Vec3::new(x, height(x, z) + 1.4, z), modeller::CARS[k], visual));
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
        // İn: aracın soluna, arazinin (ya da suyun) üstüne.
        let t = world.get::<Transform>(car).copied().unwrap_or_default();
        let side = t.rotation * Vec3::new(-2.2, 0.0, 0.0);
        let at = Vec3::new(t.position.x + side.x, 0.0, t.position.z + side.z);
        let at = Vec3::new(at.x, height(at.x, at.z).max(su::WATER) + 1.4, at.z);
        su::drive(world, car, 0.0, 0.0);
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
        .chain(&game.boats)
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

/// `parent`'ın altındaki her şeyi gizle ya da göster — torunları da: karakterin modeli birkaç kat
/// derin.
fn hide_children(world: &mut World, parent: Entity, hidden: bool) {
    let mut stack: Vec<Entity> = world.get::<gizmo::core::component::Children>(parent).map(|c| c.0.clone()).unwrap_or_default();
    while let Some(c) = stack.pop() {
        if hidden {
            world.add_component(c, IsHidden);
        } else {
            world.remove_component::<IsHidden>(c);
        }
        if let Some(more) = world.get::<gizmo::core::component::Children>(c) {
            stack.extend(more.0.iter().copied());
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
    build: bool,
    place: bool,
    rotate: bool,
    talk: bool,
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
        build: input.is_key_just_pressed(KeyCode::KeyB as u32),
        place: input.is_key_just_pressed(KeyCode::KeyG as u32) || input.is_mouse_button_just_pressed(0),
        rotate: input.is_key_just_pressed(KeyCode::KeyQ as u32),
        talk: input.is_key_just_pressed(KeyCode::KeyT as u32),
    }
}

fn step(world: &mut World, game: &mut Game, dt: f32, c: Controls) {
    game.clock += dt;
    game.weather.step(dt);
    if let Some(mut physics) = world.get_resource_mut::<PhysicsWorld>() {
        physics.weather = game.weather.grip();
    }
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
    if c.build && game.driving.is_none() {
        insa::cycle_build(world, game);
    }
    insa::update_building(world, game, c.rotate, c.place);
    insa::flicker(world, game.clock);
    if c.talk && game.driving.is_none() {
        canli::talk(world, game);
    }
    let (forward, right) = camera_axes(game);
    match game.driving {
        Some(boat) if su::is_boat(world, boat) => {
            su::drive(world, boat, c.my, c.mx);
        }
        Some(car) => {
            if let Some(mut v) = world.query_mut::<&mut VehicleController>().get_mut(car) {
                // W/S: gaz, fren, durunca geri vites (motorun `drive`'ı).
                v.drive(c.my);
                // Direksiyonun artısı sağa (`VehicleController::steering_input`), D tuşu +1.
                v.steering_input = c.mx;
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
    olcum::timed(world, "hücreler", |w| stream_chunks(w, game));
    let me = focus(world, game);
    olcum::timed(world, "canlılar", |w| canli::tick(w, me, dt));
    olcum::timed(world, "fizik", |w| {
        game.physics.advance(w, dt.min(0.1));
        w.apply_commands();
    });
    olcum::timed(world, "beden", |w| {
        karakter::animate(w, dt);
        modeller::bind_wheels(w);
    });
    let crystal_start = std::time::Instant::now();
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
            game.sounds.push("kristal");
            game.collected.insert(id);
            world.despawn_recursive(e);
        }
    }
    pick_up(world, game, dt);
    if let Some(mut p) = world.get_resource_mut::<gizmo::core::profiler::FrameProfiler>() {
        p.record_scope("kristal", crystal_start.elapsed());
    }
    if game.won.is_none() && crystals(world, game) >= GOAL as u32 {
        game.won = Some(game.clock);
        karakter::act(world, game.player, "Cheer");
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
    // Yerden yansıyan: çimin rengi, gökten değil (mavi yansıma gölgeleri de maviye boyuyordu).
    let ground = Vec3::new(0.32, 0.45, 0.22) * (0.05 + 0.3 * day);
    (shines, 3.0 * day, 0.35 * (1.0 - day), zenith.to_array(), horizon.to_array(), ground.to_array())
}

fn place_lights_and_camera(world: &mut World, game: &mut Game, dt: f32) {
    let (shines, sun_i, moon_i, _, horizon, _) = sky_at(game.time_of_day);
    if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(game.sun) {
        t.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, shines);
    }
    if let Some(mut l) = world.query_mut::<&mut DirectionalLight>().get_mut(game.sun) {
        l.intensity = sun_i * game.weather.light();
    }
    if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(game.moon) {
        t.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::new(0.3, -0.8, 0.5).normalize());
    }
    if let Some(mut l) = world.query_mut::<&mut DirectionalLight>().get_mut(game.moon) {
        l.intensity = moon_i * game.weather.light();
    }
    // Üçüncü şahıs kamera: odağın arkasında, arazinin üstünde. Yüksekliği ayrı ve yumuşak
    // izliyor: arazi 3 m'lik üçgenlerden, yürüyüş her kırılmada yön değiştiriyor, zıplama ve
    // süspansiyon gövdeyi oynatıyor — hepsini aynen izleyen kamera ilerledikçe aşağı yukarı
    // çalkalanıyordu. Yatayda sıkı (dönüşte geride kalmasın), dikeyde ~0,3 s'lik bir gecikmeyle.
    let raw = drawn_focus(world, game).position + Vec3::Y * 1.4;
    let follow = |k: f32| 1.0 - (-dt * k).exp();
    let smooth_y = match game.cam_focus_y {
        Some(y) if (raw.y - y).abs() < 6.0 => y + (raw.y - y) * follow(3.5),
        _ => raw.y,
    };
    game.cam_focus_y = Some(smooth_y);
    let target = Vec3::new(raw.x, smooth_y, raw.z);
    let distance = if game.driving.is_some() { 11.0 } else { 7.0 };
    let back = Vec3::new(game.cam_yaw.sin() * game.cam_pitch.cos(), game.cam_pitch.sin(), game.cam_yaw.cos() * game.cam_pitch.cos());
    let mut want = target + back * distance;
    want.y = want.y.max(height(want.x, want.z) + 1.0);
    if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(game.camera) {
        let to = want - t.position;
        t.position += Vec3::new(to.x * follow(9.0), to.y * follow(5.0), to.z * follow(9.0));
        // The aim is the rotation; `looking_at` keeps +Y up (an arc rotation from −Z rolls).
        if let Some(aim) = Camera::looking_at(t.position, target) {
            t.rotation = aim;
        }
    }
    if let Some(mut cam) = world.query_mut::<&mut Camera>().get_mut(game.camera) {
        cam.clear_color = Some(Vec3::from(game.weather.grade(horizon)));
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
    let sky = game.weather.now.name();
    let speed = game.driving.map(|c| {
        let kmh = world.get::<VehicleController>(c).map_or_else(|| world.get::<Velocity>(c).map_or(0.0, |v| v.linear.length() * 3.6), |v| v.current_speed_kmh);
        (kmh, if su::is_boat(world, c) { "kayıkta" } else { "arabada" })
    });
    let mut mode = match speed {
        Some((s, what)) => format!("{what}, {s:.0} km/s · E: in"),
        None => match canli::nearest_villager(world, me) {
            Some((_, v)) => format!("yaya · T: {} ile konuş", v.name),
            None => "yaya · E: arabaya bin (yakındaysa)".to_string(),
        },
    };
    let quests = canli::quest_lines(world, game);
    if !quests.is_empty() {
        mode = format!("{mode}   {quests}");
    }
    set_text(
        world,
        game.hud,
        &format!(
            "Kristal {}/{GOAL}   odun {}   taş {}   en yakın kristal: {pointer}   saat {:02}:{:02} ({sky})\n{mode}\nWASD: yürü/sür · boşluk: zıpla · F: kes/kır · I: envanter ve üretim · B: inşa · M: harita · H: hava · sağ tık+fare / oklar: kamera · R: başlangıç",
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
    let mut controls = read_controls(input);
    // Bir düğmeye (tarif, yuva) tıklamak yapı koymasın.
    controls.place &= !gizmo::ui::pointer_over_ui(world);
    step(world, &mut game, dt, controls);
    let toggle = input.is_key_just_pressed(KeyCode::KeyI as u32) || input.is_key_just_pressed(KeyCode::Tab as u32);
    olcum::timed(world, "panel", |w| update_panel(w, &mut game, toggle));
    olcum::timed(world, "harita", |w| update_map(w, &mut game, input.is_key_just_pressed(KeyCode::KeyM as u32)));
    if input.is_key_just_pressed(KeyCode::KeyH as u32) {
        game.weather.turn();
        game.note = (format!("Hava: {}", game.weather.now.name()), 2.0);
    }
    let engine = game.driving.map(|v| {
        let speed = world.get::<Velocity>(v).map_or(0.0, |v| v.linear.length());
        let throttle = world
            .get::<VehicleController>(v)
            .map(|c| c.throttle_input - c.brake_input)
            .or_else(|| world.get::<gizmo::physics::BoatController>(v).map(|b| b.throttle_input))
            .unwrap_or(0.0);
        (throttle, speed)
    });
    if let Some(sounds) = game.audio.take() {
        let rain = game.weather.rain;
        olcum::timed(world, "ses", |w| hava::play(w, &sounds, &mut game.sounds, rain, engine));
        game.audio = Some(sounds);
    }
    game.sounds.clear();
    autosave(world, &mut game, dt);
    place_lights_and_camera(world, &mut game, dt);
    olcum::timed(world, "hud", |w| draw_hud(w, &game));
    olcum::update(world, &mut game.meter, input.is_key_just_pressed(KeyCode::F3 as u32), dt);
    world.insert_resource(game);
}

/// Küçük harita: odağın üstünde, yürünen (ya da sürülen) yönün okuyla.
fn update_map(world: &mut World, game: &mut Game, toggle: bool) {
    let me = focus(world, game);
    let body = game.driving.unwrap_or(game.player);
    let heading = world.get::<Transform>(body).map_or(Vec3::NEG_Z, |t| t.rotation * Vec3::NEG_Z);
    harita::update(world, &mut game.map, me, heading, toggle);
}

/// Oyunun tamamı bir `App` olarak.
pub fn app() -> App<SimpleSceneState> {
    let mut app = App::<SimpleSceneState>::new("Gizmo Engine - Dünya", 1280, 800);
    for (path, bytes) in modeller::EMBEDDED {
        app = app.add_embedded_asset(path, std::borrow::Cow::Borrowed(bytes));
    }
    app.add_plugin(UiPlugin)
        .with_simple_scene(|scene, _state| {
            let font = scene.renderer.default_font();
            // Arazinin dokulu gölgelendiricisi. Derlenmemesi oyunun bir hatası — sınaması
            // `modeller::tests::the_ground_shader_composes`.
            let surface = scene.renderer.add_surface_material("dunya_arazi", modeller::GROUND_WGSL).unwrap_or_else(|e| panic!("{e}"));
            scene.world.insert_resource(modeller::ground_look(surface));
            let water = scene.renderer.add_surface_material("dunya_su", modeller::WATER_WGSL).unwrap_or_else(|e| panic!("{e}"));
            scene.world.insert_resource(modeller::water_look(water));
            let game = setup(scene.world, font, true);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        // Gök, sis: günün saatine göre, oyunun kaynağından her senkron noktasında.
        .set_render_stage(RenderWorld::new().with_settings(|game, renderer| {
            if let Some(g) = game.get_resource::<Game>() {
                let (_, _, _, zenith, horizon, ground) = sky_at(g.time_of_day);
                let w = &g.weather;
                let (zenith, horizon, ground) = (w.grade(zenith), w.grade(horizon), w.grade(ground));
                // Göğün ışığı güneşin yanında kısık: tam şiddette güneşe denk geliyordu, gölgeler
                // kayboluyor ve her şey mavimsi bir pastele dönüyordu (gölgedeki çim güneştekiyle
                // aynı renkteydi). Gerçek bir açık gökte gökten gelen, güneşinkinin beşte biri
                // kadar. Gece, ışığın hepsi gökten olduğu için kısılmıyor.
                let day = ((g.time_of_day - 0.25) * std::f32::consts::TAU).sin().clamp(0.0, 1.0);
                renderer.sky = Some(Sky::gradient(zenith, horizon, ground).with_intensity(1.0 - 0.68 * day.sqrt()));
                renderer.fog = Some(Fog::new(horizon, w.fog_density(0.0016)).with_height(0.03, 0.0));
                // Yağmur: kameranın çevresine bu karenin damlaları.
                if w.rain > 0.02 {
                    let (eye, ahead) = game.get::<Transform>(g.camera).map_or((Vec3::ZERO, Vec3::NEG_Z), |t| (t.position, Camera::front(t.rotation)));
                    let drops = hava::raindrops(eye, ahead, w.rain, g.clock);
                    if let Some(particles) = &renderer.gpu_particles {
                        particles.spawn_particles(&renderer.queue, &drops);
                    }
                }
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
    // Haritanın işaretleri pencerenin boyuyla yerleşiyor: sınamanın da bir penceresi varmış gibi.
    world.insert_resource(gizmo::core::window::WindowInfo { width: 1280.0, height: 800.0 });
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    let mut deepest_walk = f32::MAX;
    let mut deepest_drive = f32::MAX;
    let mut walked_far = 0.0f32;
    let mut drove_far = 0.0f32;
    // Köy ve hayvan aşaması: 100. saniyede en yakın köye; bir köylüden iş al ve teslim et; bir
    // hayvanın yanına git, üç saniyede kaçmış olmalı.
    let village_at = 60 * 100;
    let mut village_report = String::from("köy aşaması çalışmadı");
    let mut village_ok = false;
    let mut flee: Option<(Entity, f32)> = None;
    let mut fled = 0.0f32;
    let mut animals_seen = 0usize;
    // Hava: 60. saniyede yağmur; on saniye sonra yağıyor, yol kaygan, damlalar düşüyor.
    let rain_at = 60 * 60;
    let mut weather_report = String::from("hava aşaması çalışmadı");
    let mut weather_ok = false;
    let mut map_village = false;
    let mut map_marks = 0usize;
    // Kayık aşaması: 85. saniyede en yakın gölün kayığına bin, altı saniye tam gaz sür, in, ve
    // yedi saniye kıyıya yüz (köy aşamasından önce bitiyor).
    let boat_at = 60 * 85;
    let mut boat_start: Option<(Entity, Vec3)> = None;
    let mut boat_report = String::from("kayık aşaması çalışmadı");
    let mut boat_ok = false;
    let mut boat_y = (f32::MAX, f32::MIN);
    // Kayıktan inince kıyıya yüz: 7. ve 14. saniyeler arası.
    let mut shore_target: Option<Vec3> = None;
    let mut swim_report = String::new();
    let mut swim_ok = false;
    let mut deepest_swim = f32::MAX;
    // İnşa aşaması: 115. saniyede malzeme, dört tarif, önüne bir çit; üç saniye çite yürü.
    let build_at = 60 * 115;
    let mut fence: Option<(Vec3, Vec3)> = None;
    let mut pushed_through = f32::MIN;
    let mut build_report = String::new();
    let mut build_ok = false;
    // Köprünün korkuluğu: köprünün ortasından 1,7 saniye yana yürü (3,2 s sonra, araba aşamasından
    // önce biter); köprüde kalmalı.
    let rail_at = build_at + 190;
    let mut rail: Option<(Vec3, Quat, f32)> = None;
    let mut rail_report = String::from("köprü aşaması çalışmadı");
    let mut rail_ok = false;
    // Karakter: oyuncunun karakterinin bu koşuda istediği klipler; köylülerin hepsi giydirilmiş mi.
    let mut clips_seen: HashSet<&'static str> = HashSet::new();
    let mut villagers_dressed = (0usize, 0usize);
    // Hayvanlar: hepsi giydirilmiş mi; kaçan hayvanın istediği klipler.
    let mut animals_dressed = (0usize, 0usize);
    let mut flee_clips: HashSet<&'static str> = HashSet::new();
    for frame in 0..(60 * 180) {
        let driving_phase = frame >= 60 * 120;
        let walking_into_fence = (build_at..build_at + 180).contains(&frame);
        // Hayvanın yanında dur, kovalamadan: kaçıyorsa kendi kaçıyor.
        let standing = (village_at..village_at + 300).contains(&frame);
        let mut built_check: Option<(bool, String)> = None;
        run.frame(dt, |w, _s, dt, _i| {
            let Some(mut game) = w.remove_resource::<Game>() else { return };
            let teleport = |w: &mut World, game: &Game, x: f32, z: f32| {
                if let Some(mut t) = w.query_mut::<&mut Transform>().get_mut(game.player) {
                    t.position = Vec3::new(x, height(x, z) + 1.6, z);
                }
                if let Some(mut v) = w.query_mut::<&mut Velocity>().get_mut(game.player) {
                    v.linear = Vec3::ZERO;
                }
            };
            if frame == rain_at {
                while game.weather.now != hava::Sky::Rain {
                    game.weather.turn();
                }
            }
            if frame == rain_at + 600 {
                let grip = w.get_resource::<PhysicsWorld>().map(|p| p.weather);
                let eye = w.get::<Transform>(game.camera).map_or(Vec3::ZERO, |t| t.position);
                let drops = hava::raindrops(eye, Vec3::NEG_Z, game.weather.rain, game.clock);
                // Doğarken canlı (yaş sıfır, ömrü var): `life = max_life` ile hepsi ölü doğuyordu.
                let alive = drops.iter().filter(|d| d.life < d.max_life && d.max_life > 0.0).count();
                let drops = drops.len().min(alive);
                weather_ok = game.weather.rain > 0.8 && grip == Some(gizmo::physics::world::Weather::Rain) && drops > 30 && game.weather.light() < 0.6;
                weather_report = format!("yağmur {:.2}, yol {:?}, bu kare {drops} damla, ışık ×{:.2}", game.weather.rain, grip, game.weather.light());
            }
            if frame == village_at + 30 {
                update_map(w, &mut game, false);
                map_marks = harita::shown_marks(w, &game.map);
                map_village = w.query::<&Text>().iter().any(|t| t.content == "ev");
            }
            if frame == village_at {
                let near = (-6..=6).flat_map(|x| (-6..=6).map(move |z| (x, z))).filter_map(canli::village_at).min_by(|a, b| a.length().total_cmp(&b.length()));
                if let Some(v) = near {
                    teleport(w, &game, v.x, v.z + 6.0);
                    village_report = format!("köy {:.0} m ötede", Vec2::new(v.x, v.z).length());
                } else {
                    village_report = "yakında köy yok".to_string();
                }
            }
            if frame == village_at + 60 {
                let me = focus(w, &game);
                let nearest = w.query::<(&canli::Villager, &Transform)>().iter().map(|(v, t)| (*v, t.position)).min_by(|a, b| a.1.distance(me).total_cmp(&b.1.distance(me)));
                if let Some((villager, at)) = nearest {
                    teleport(w, &game, at.x + 1.5, at.z);
                    sync_transforms(w);
                    let q = canli::quest_of(villager.id);
                    let villagers: Vec<Entity> = w.query::<(Entity, &canli::Villager)>().iter().map(|(e, _)| e).collect();
                    villagers_dressed = (villagers.iter().filter(|v| karakter::wanted_clip(w, **v).is_some()).count(), villagers.len());
                    let talked = canli::talk(w, &mut game);
                    let offered = game.quests.get(&villager.id) == Some(&1);
                    let before = w.get::<Inventory>(game.player).map_or(0, |b| b.count(q.reward.0));
                    give(w, &mut game, q.want.0, q.want.1);
                    canli::talk(w, &mut game);
                    let after = w.get::<Inventory>(game.player).map_or(0, |b| b.count(q.reward.0));
                    let done = game.quests.get(&villager.id) == Some(&2);
                    village_ok = talked && offered && done && after == before + q.reward.1;
                    village_report = format!("{village_report}; {} {} {} istedi, {} {} verdi ({})", villager.name, q.want.1, q.want.0, q.reward.1, q.reward.0, if village_ok { "tamam" } else { "olmadı" });
                }
            }
            if frame == village_at + 120 {
                let me = focus(w, &game);
                animals_seen = w.query::<&canli::Animal>().iter().count();
                let animals: Vec<Entity> = w.query::<(Entity, &canli::Animal)>().iter().map(|(e, _)| e).collect();
                animals_dressed = (animals.iter().filter(|a| karakter::wanted_clip(w, **a).is_some()).count(), animals.len());
                let nearest = w.query::<(Entity, &canli::Animal, &Transform)>().iter().map(|(e, _, t)| (e, t.position)).min_by(|a, b| a.1.distance(me).total_cmp(&b.1.distance(me)));
                if let Some((animal, at)) = nearest {
                    teleport(w, &game, at.x + 2.5, at.z);
                    flee = Some((animal, 2.5));
                }
            }
            if frame == village_at + 300 {
                if let Some((animal, d0)) = flee {
                    let me = focus(w, &game);
                    fled = w.get::<Transform>(animal).map_or(0.0, |t| Vec2::new(t.position.x - me.x, t.position.z - me.z).length() - d0);
                }
            }
            if frame == boat_at {
                let lake = (-6..=6).flat_map(|x| (-6..=6).map(move |z| (x, z))).filter_map(su::boat_spot).min_by(|a, b| a.length().total_cmp(&b.length()));
                match lake {
                    Some(spot) => {
                        // Kıyıya: kayık yerinin yanındaki kuru yer.
                        let shore = [(4.0f32, 0.0f32), (-4.0, 0.0), (0.0, 4.0), (0.0, -4.0)]
                            .into_iter()
                            .map(|(dx, dz)| Vec2::new(spot.x + dx, spot.z + dz))
                            .find(|p| !su::wet(p.x, p.y, 0.0))
                            .unwrap_or(Vec2::new(spot.x, spot.z));
                        teleport(w, &game, shore.x, shore.y);
                        boat_report = format!("göl {:.0} m ötede", Vec2::new(spot.x, spot.z).length());
                    }
                    None => boat_report = "yakında göl yok".to_string(),
                }
            }
            if frame == boat_at + 60 {
                let me = focus(w, &game);
                let nearest = game.boats.iter().copied().filter_map(|b| w.get::<Transform>(b).map(|t| (b, t.position))).min_by(|a, b| a.1.distance(me).total_cmp(&b.1.distance(me)));
                if let Some((boat, at)) = nearest {
                    teleport(w, &game, at.x + 1.5, at.z);
                    sync_transforms(w);
                    enter_or_leave(w, &mut game);
                    if game.driving == Some(boat) {
                        boat_start = Some((boat, at));
                    }
                }
            }
            if frame == boat_at + 60 * 7 {
                if let Some((boat, start)) = boat_start {
                    let at = w.get::<Transform>(boat).map_or(start, |t| t.position);
                    let moved = Vec2::new(at.x - start.x, at.z - start.z).length();
                    enter_or_leave(w, &mut game);
                    let afloat = boat_y.0 > su::WATER - 1.0 && boat_y.1 < su::WATER + 1.0;
                    boat_ok = moved > 10.0 && afloat && game.driving.is_none();
                    // En yakın kuru yer: kıyı.
                    let me = focus(w, &game);
                    shore_target = (1..60)
                        .flat_map(|r| (0..16).map(move |k| (r as f32 * 2.0, k as f32 / 16.0 * std::f32::consts::TAU)))
                        .map(|(r, a)| Vec3::new(me.x + a.cos() * r, 0.0, me.z + a.sin() * r))
                        .find(|p| height(p.x, p.z) > su::WATER + 0.8);
                    boat_report = format!("{boat_report}; kayık 6 s'de {moved:.0} m gitti, su yüzünden {:.2}…{:.2} m", boat_y.0 - su::WATER, boat_y.1 - su::WATER);
                }
            }
            if frame == boat_at + 60 * 7 + 60 * 7 {
                let me = focus(w, &game);
                let ground = height(me.x, me.z);
                swim_ok = me.y > su::WATER && me.y > ground - 0.2;
                swim_report = format!(
                    "kıyıya yüzdü: {:.1} m kaldı, su yüzünden {:.2} m, zeminden {:.2} m, yüzüyor mu {}",
                    shore_target.map_or(f32::NAN, |s| Vec2::new(s.x - me.x, s.z - me.z).length()),
                    me.y - su::WATER,
                    me.y - ground,
                    w.get::<CharacterController>(game.player).is_some_and(|k| k.is_submerged)
                );
            }
            if frame == build_at - 60 {
                // Kuru, düz yere: başlangıcın düzlüğü (kayıktan göle inmişti).
                teleport(w, &game, 6.0, -6.0);
            }
            if frame == build_at {
                let before = w.get::<Inventory>(game.player).cloned().unwrap_or_default();
                give(w, &mut game, "odun", 30);
                give(w, &mut game, "tas", 10);
                let crafted = insa::Build::ALL.iter().all(|k| insa::craft(w, &mut game, *k));
                let bag = w.get::<Inventory>(game.player).cloned().unwrap_or_default();
                let spent_right = bag.count("odun") + 27 == before.count("odun") + 30 && bag.count("tas") + 8 == before.count("tas") + 10 && bag.count("cit") == 2;
                let (at, yaw) = insa::placement(w, &game, insa::Build::Fence, 0.0);
                let placed = insa::place_structure(w, &mut game, insa::Build::Fence, at, yaw);
                let me = focus(w, &game);
                let (forward, _) = camera_axes(&game);
                fence = Some((me, forward));
                built_check = Some((crafted && spent_right && placed, format!("4 tarif üretildi {crafted}, harcanan doğru {spent_right}")));
            }
            if frame == build_at + 180 {
                let me = focus(w, &game);
                let (forward, right) = camera_axes(&game);
                let player = game.player;
                let held = |w: &World| insa::Build::ALL.map(|k| w.get::<Inventory>(player).map_or(0, |b| b.count(k.item())));
                let before = held(w);
                let fire = insa::place_structure(w, &mut game, insa::Build::Fire, me + right * 4.0, 0.0);
                let hut = insa::place_structure(w, &mut game, insa::Build::Hut, me - forward * 7.0 - right * 3.0, 0.4);
                let bridge = insa::place_structure(w, &mut game, insa::Build::Bridge, me - right * 4.0, 1.2);
                let structures = w.query::<&insa::Structure>().iter().count();
                // Oyuncunun ateşinin ışığı (köylerin ateşleri kayda girmiyor, `Structure` taşımıyor).
                let lights = w
                    .query::<(&insa::FireLight, &gizmo::core::component::Parent)>()
                    .iter()
                    .filter(|(_, parent)| w.get::<insa::Structure>(parent.0).is_some())
                    .count();
                let after = held(w);
                let spent: Vec<u32> = before.iter().zip(&after).map(|(b, a)| b - a).collect();
                // Kayıt: yapılar aynen geri geliyor.
                let saved = insa::saved(w);
                let mut fresh = World::new();
                insa::restore(&mut fresh, &saved, false);
                let back = insa::saved(&fresh);
                let same = back.len() == saved.len() && back.iter().zip(&saved).all(|(a, b)| a.0 == b.0 && (a.1 - b.1).abs() < 1e-4 && (a.3 - b.3).abs() < 1e-4);
                let ok = fire && hut && bridge && structures == 4 && lights == 1 && spent == [1, 0, 1, 1] && same;
                built_check = Some((ok, format!("4 yapı kuruldu {}, ateş ışığı {lights}, harcanan {spent:?}, kayıttan geri {}", structures == 4, same)));
            }
            if frame == rail_at {
                let bridge = w
                    .query::<(&insa::Structure, &Transform)>()
                    .iter()
                    .find(|(s, _)| s.kind == insa::Build::Bridge)
                    .map(|(_, t)| (t.position, t.rotation));
                if let Some((base, turn)) = bridge {
                    let middle = base + turn * Vec3::new(0.0, 0.0, -4.0);
                    if let Some(mut t) = w.query_mut::<&mut Transform>().get_mut(game.player) {
                        t.position = middle + Vec3::Y * 1.1;
                    }
                    if let Some(mut v) = w.query_mut::<&mut Velocity>().get_mut(game.player) {
                        v.linear = Vec3::ZERO;
                    }
                    rail = Some((middle, turn, base.y));
                }
            }
            if frame == rail_at + 100 {
                if let Some((middle, turn, deck)) = rail {
                    let me = focus(w, &game);
                    let local = turn.inverse() * (me - middle);
                    // Korkuluk ±0,72'de, kapsül 0,35: içeride en çok ~0,4.
                    rail_ok = local.x.abs() < 0.6 && me.y > deck;
                    rail_report = format!("köprüde yana 1,7 saniye: ortadan {:.2} m, zeminin {:.2} m üstünde", local.x, me.y - deck);
                }
            }
            if driving_phase && game.driving.is_none() {
                // Arabaya: en yakını yanına getir, bin.
                let me = focus(w, &game);
                let car = game.cars[0];
                if let Some(mut t) = w.query_mut::<&mut Transform>().get_mut(car) {
                    t.position = Vec3::new(me.x + 2.0, height(me.x + 2.0, me.z) + 1.4, me.z);
                    t.rotation = Quat::IDENTITY;
                }
                if let Some(mut v) = w.query_mut::<&mut Velocity>().get_mut(car) {
                    v.linear = Vec3::ZERO;
                    v.angular = Vec3::ZERO;
                }
                enter_or_leave(w, &mut game);
            }
            let boating = boat_start.is_some() && (boat_at + 60..boat_at + 60 * 7).contains(&frame);
            let swimming = (boat_at + 60 * 7..boat_at + 60 * 14).contains(&frame);
            let controls = if let (true, Some(target)) = (swimming, shore_target) {
                let me = focus(w, &game);
                let (forward, right) = camera_axes(&game);
                let d = Vec3::new(target.x - me.x, 0.0, target.z - me.z).normalize_or_zero();
                Controls { mx: d.dot(right), my: d.dot(forward), ..Default::default() }
            } else if boating {
                // Tam gaz: burnu açık suya bakıyor.
                Controls { my: 1.0, ..Default::default() }
            } else if let (true, Some((_, turn, _))) = ((rail_at..rail_at + 100).contains(&frame), rail) {
                // Köprünün sağına, korkuluğa doğru.
                let (forward, right) = camera_axes(&game);
                let d = turn * Vec3::X;
                Controls { mx: d.dot(right), my: d.dot(forward), ..Default::default() }
            } else if walking_into_fence {
                Controls { my: 1.0, ..Default::default() }
            } else if standing {
                Controls::default()
            } else {
                autopilot(w, &game, driving_phase)
            };
            step(w, &mut game, dt, controls);
            w.insert_resource(game);
            if let Some(mut p) = w.get_resource_mut::<gizmo::core::profiler::FrameProfiler>() {
                p.end_frame();
            }
        });
        if let Some((ok, text)) = built_check {
            build_ok = if frame == build_at { ok } else { build_ok && ok };
            build_report = if build_report.is_empty() { text } else { format!("{build_report}; {text}") };
        }
        let game = run.world.get_resource::<Game>().expect("oyun");
        if game.driving.is_none() {
            clips_seen.extend(karakter::wanted_clip(&run.world, game.player));
        }
        if let (Some((animal, _)), true) = (flee, (village_at + 130..village_at + 300).contains(&frame)) {
            flee_clips.extend(karakter::wanted_clip(&run.world, animal));
        }
        let p = focus(&run.world, &game);
        if let (Some((boat, _)), true) = (boat_start, (boat_at + 90..boat_at + 60 * 7).contains(&frame)) {
            if let Some(t) = run.world.get::<Transform>(boat) {
                boat_y = (boat_y.0.min(t.position.y), boat_y.1.max(t.position.y));
            }
        }
        if (boat_at + 60 * 7..boat_at + 60 * 14).contains(&frame) {
            deepest_swim = deepest_swim.min(p.y - height(p.x, p.z));
        }
        if walking_into_fence {
            if let Some((start, forward)) = fence {
                pushed_through = pushed_through.max((p - start).dot(forward));
            }
        }
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
    // Başlangıçta bırakılan, hiç sürülmeyen arabalar: hücreleri silinince düşüyorlardı (6,9 km);
    // şimdi hücre yokken bekletiliyorlar, yerlerinde.
    let parked_ok = game.cars[1..].iter().all(|&car| {
        run.world.get::<Transform>(car).is_some_and(|t| t.position.y > height(t.position.x, t.position.z) - 0.5)
    });
    let (wood, stone) = (bag.count("odun"), bag.count("tas"));
    // Yürüdü, durdu, konuştu, kurdu, topladı: her birinin klibi istendi; köylülerin hepsinde karakter.
    // (Otomatik pilot tam hızda, 6 m/s: koşuyor.)
    let clips_ok = ["Idle", "Interact", "Use_Item", "PickUp"].iter().all(|c| clips_seen.contains(c))
        && (clips_seen.contains("Walking_A") || clips_seen.contains("Running_A"))
        && villagers_dressed.1 > 0
        && villagers_dressed.0 == villagers_dressed.1
        && animals_dressed.1 > 0
        && animals_dressed.0 == animals_dressed.1
        && (flee_clips.contains("Walk") || flee_clips.contains("Gallop"));
    let ok = deepest_walk > -0.5
        && deepest_drive > -0.5
        && crystals_taken >= 2
        && bag.count("kristal") == crystals_taken
        && wood + stone > 0
        && game.unloads > 0
        && game.driving.is_some()
        && build_ok
        && village_ok
        && weather_ok
        && map_village
        && boat_ok
        && swim_ok
        && deepest_swim > -0.5
        && parked_ok
        && animals_seen > 0
        && fled > 3.0
        // Çit 2,6 m önde, 0,1 m kalın: yüzü 2,5 m'de. Kapsülün yarıçapı 0,35 ama yamaçta biraz daha
        // yaklaşıyor (ölçülen 2,29 m); 2,5 m'yi geçen çitin içinden geçmiştir. Çitsiz üç saniye 18 m.
        && pushed_through < 2.5
        && clips_ok
        && rail_ok;
    println!("dunya --test: canlılar: {village_report}; {animals_seen} hayvan, en yakını üç saniyede {fled:.1} m kaçtı; bırakılan arabalar yerinde: {parked_ok}");
    println!("dunya --test: su: {boat_report}; {swim_report}; yüzerken zeminin en çok {:.2} m altında", -deepest_swim.min(0.0));
    println!("dunya --test: hava: {weather_report}; harita köyde {map_marks} işaret, köy görünüyor: {map_village}");
    let mut seen: Vec<&str> = clips_seen.iter().copied().collect();
    seen.sort_unstable();
    let mut fled_clips: Vec<&str> = flee_clips.iter().copied().collect();
    fled_clips.sort_unstable();
    println!(
        "dunya --test: karakter: istenen klipler {seen:?}; giydirilen köylü {}/{}, hayvan {}/{}; kaçan hayvanın klipleri {fled_clips:?}",
        villagers_dressed.0, villagers_dressed.1, animals_dressed.0, animals_dressed.1
    );
    println!("dunya --test: köprü: {rail_report}");
    if let Some(r) = olcum::report(&run.world) {
        println!("dunya --test: ölçüm (son iki saniye, penceresiz): {}", r.replace('\n', " | "));
    }
    println!("dunya --test: inşa: {build_report}; çite yürüyüş en çok {pushed_through:.2} m (çit 2,6 m önde)");
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
