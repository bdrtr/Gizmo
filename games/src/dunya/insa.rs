//! Üretim ve inşa: odun ve taştan kamp ateşi, çit, köprü ve barınak.
//!
//! **Üretim** envanter panelinde (I): bir tarif düğmesi, yeterince malzeme varsa yanar ve tıklayınca
//! motorun `Inventory::craft`'ı malzemeyi alıp ürünü koyar — eksikse ya da ürüne yer yoksa hiçbir
//! şey harcanmaz. **İnşa** B ile: oyuncunun önünde yapının yarı saydam bir hayaleti, yere oturmuş;
//! yeşilse G ya da sol tık koyar, kırmızıysa yer uygun değil (fazla dik). Q yapıyı döndürür, B bir
//! sonraki yapıya geçer, sonuncudan sonra çıkar.
//!
//! Yapılar katı (`RigidBody::new_static` kutuları): çitten geçilmez, barınağın damı ve köprünün
//! tahtası üstünde yürünür. Kamp ateşi geceleri etrafını aydınlatan, titreyen bir nokta ışık. Hepsi
//! kayda giriyor (`yapilar`), hücrelerden bağımsız: uzaklaşınca silinmiyor.

use super::*;
use gizmo::app::inventory::Recipe;
use gizmo::ui::{Clicked, Interaction};

/// Kurulabilen yapılar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Build {
    Fire,
    Fence,
    Bridge,
    Hut,
}

impl Build {
    pub(super) const ALL: [Build; 4] = [Build::Fire, Build::Fence, Build::Bridge, Build::Hut];

    /// Envanterdeki eşyanın anahtarı.
    pub(super) fn item(self) -> &'static str {
        match self {
            Build::Fire => "kamp_atesi",
            Build::Fence => "cit",
            Build::Bridge => "kopru",
            Build::Hut => "baraka",
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Build::Fire => "Kamp ateşi",
            Build::Fence => "Çit",
            Build::Bridge => "Köprü",
            Build::Hut => "Barınak",
        }
    }

    /// Ne harcıyor, kaç tane yapıyor.
    pub(super) fn recipe(self) -> Recipe {
        match self {
            Build::Fire => Recipe::new(self.item(), 1).needs("odun", 4).needs("tas", 2),
            Build::Fence => Recipe::new(self.item(), 2).needs("odun", 3),
            Build::Bridge => Recipe::new(self.item(), 1).needs("odun", 8),
            Build::Hut => Recipe::new(self.item(), 1).needs("odun", 12).needs("tas", 6),
        }
    }

    fn code(self) -> u8 {
        Build::ALL.iter().position(|b| *b == self).unwrap_or(0) as u8
    }

    fn from_code(code: u8) -> Option<Build> {
        Build::ALL.get(code as usize).copied()
    }

    /// Oyuncunun önünde ne kadar ileride kuruluyor (köprü önünden başlıyor).
    fn reach(self) -> f32 {
        match self {
            Build::Bridge => 1.5,
            Build::Hut => 4.0,
            _ => 2.6,
        }
    }
}

/// Envanter eşyaları olarak yapılar: adları ve yığın boyları.
pub(super) fn register_items(c: &mut ItemCatalog) {
    for b in Build::ALL {
        c.register(b.item(), ItemInfo::new(b.name(), 10));
    }
}

pub(super) fn item_colour(item: &str) -> Option<Vec4> {
    Some(match item {
        "kamp_atesi" => Vec4::new(0.95, 0.5, 0.2, 1.0),
        "cit" => Vec4::new(0.75, 0.6, 0.4, 1.0),
        "kopru" => Vec4::new(0.55, 0.45, 0.35, 1.0),
        "baraka" => Vec4::new(0.7, 0.55, 0.5, 1.0),
        _ => return None,
    })
}

/// Kurulmuş bir yapının ilk parçası: kayda ne yazılacağı.
#[derive(Clone, Copy)]
pub(super) struct Structure {
    pub(super) kind: Build,
    pub(super) yaw: f32,
}
gizmo::core::impl_component!(Structure);

/// Kamp ateşinin ışığı; `phase` her ateşi ayrı titretiyor.
#[derive(Clone, Copy)]
pub(super) struct FireLight {
    phase: f32,
}
gizmo::core::impl_component!(FireLight);

/// Hayaletin kökü (yalnız görünüş, çarpıştırıcı yok).
#[derive(Clone, Copy)]
pub(super) struct Ghost;
gizmo::core::impl_component!(Ghost);

/// Bir tarif düğmesi panelde.
#[derive(Clone, Copy)]
pub(super) struct RecipeButton(Build);
gizmo::core::impl_component!(RecipeButton);

/// İnşa kipinin durumu.
pub(super) struct Building {
    pub(super) kind: Build,
    /// Q ile eklenen dönüş.
    pub(super) turn: f32,
    pub(super) ghost: Entity,
    pub(super) valid: bool,
}

#[derive(Clone, Copy)]
enum Look {
    Wood,
    DarkWood,
    Stone,
    Flame,
}

impl Look {
    fn material(self) -> MaterialDesc {
        match self {
            Look::Wood => MaterialDesc::new().with_pbr(Vec4::new(0.58, 0.4, 0.23, 1.0), 0.85, 0.0),
            Look::DarkWood => MaterialDesc::new().with_pbr(Vec4::new(0.32, 0.21, 0.12, 1.0), 0.9, 0.0),
            Look::Stone => MaterialDesc::new().with_pbr(Vec4::new(0.52, 0.52, 0.55, 1.0), 0.8, 0.0),
            Look::Flame => MaterialDesc::new().with_unlit(Vec4::new(1.0, 0.62, 0.2, 1.0)).with_emissive(Vec3::new(4.0, 1.8, 0.4)),
        }
    }
}

/// Bir yapının bir parçası, yapının kendi çerçevesinde (yukarı +Y, ileri −Z, yer y = 0).
struct Part {
    at: Vec3,
    half: Vec3,
    turn: Quat,
    mesh: &'static str,
    look: Look,
    solid: bool,
}

fn block(at: Vec3, half: Vec3, look: Look, solid: bool) -> Part {
    Part { at, half, turn: Quat::IDENTITY, mesh: "standard_cube", look, solid }
}

fn parts(kind: Build) -> Vec<Part> {
    let mut p = Vec::new();
    match kind {
        Build::Fire => {
            for k in 0..7 {
                let a = k as f32 / 7.0 * std::f32::consts::TAU;
                p.push(block(Vec3::new(a.cos() * 0.45, 0.08, a.sin() * 0.45), Vec3::new(0.11, 0.08, 0.09), Look::Stone, false));
            }
            for k in 0..3 {
                let a = k as f32 / 3.0 * std::f32::consts::PI;
                // Motorun silindiri 1 m yarıçaplı, 2 m boyunda: ölçek yarıçap ve yarı boy; yatırılmış.
                p.push(Part {
                    at: Vec3::new(0.0, 0.1, 0.0),
                    half: Vec3::new(0.06, 0.32, 0.06),
                    turn: Quat::from_rotation_y(a) * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2 * 0.8),
                    mesh: "cylinder",
                    look: Look::DarkWood,
                    solid: false,
                });
            }
            p.push(Part { at: Vec3::new(0.0, 0.3, 0.0), half: Vec3::new(0.16, 0.24, 0.16), turn: Quat::IDENTITY, mesh: "sphere", look: Look::Flame, solid: false });
            // İçine yürünmesin: alçak bir taş halka.
            p.push(Part { at: Vec3::new(0.0, 0.15, 0.0), half: Vec3::new(0.55, 0.15, 0.55), turn: Quat::IDENTITY, mesh: "", look: Look::Stone, solid: true });
        }
        Build::Fence => {
            for x in [-1.0f32, 1.0] {
                p.push(block(Vec3::new(x, 0.55, 0.0), Vec3::new(0.07, 0.55, 0.07), Look::DarkWood, false));
            }
            for y in [0.38f32, 0.8] {
                p.push(block(Vec3::new(0.0, y, 0.0), Vec3::new(1.0, 0.05, 0.035), Look::Wood, false));
            }
            p.push(Part { at: Vec3::new(0.0, 0.6, 0.0), half: Vec3::new(1.05, 0.6, 0.1), turn: Quat::IDENTITY, mesh: "", look: Look::Wood, solid: true });
        }
        Build::Bridge => {
            // Sekiz metre, oyuncunun önünden ileri; tahtalar ayrı ayrı görünüyor, çarpıştırıcı tek.
            for k in 0..16 {
                let z = -0.25 - k as f32 * 0.5;
                p.push(block(Vec3::new(0.0, 0.0, z), Vec3::new(0.75, 0.06, 0.22), if k % 2 == 0 { Look::Wood } else { Look::DarkWood }, false));
            }
            for x in [-0.72f32, 0.72] {
                p.push(block(Vec3::new(x, 0.55, -4.0), Vec3::new(0.04, 0.04, 4.0), Look::DarkWood, false));
                for k in 0..5 {
                    p.push(block(Vec3::new(x, 0.28, -0.1 - k as f32 * 1.95), Vec3::new(0.05, 0.3, 0.05), Look::DarkWood, false));
                }
            }
            p.push(Part { at: Vec3::new(0.0, -0.02, -4.0), half: Vec3::new(0.78, 0.1, 4.0), turn: Quat::IDENTITY, mesh: "", look: Look::Wood, solid: true });
        }
        Build::Hut => {
            for (x, z) in [(-1.5f32, -1.5f32), (1.5, -1.5), (-1.5, 1.5), (1.5, 1.5)] {
                p.push(block(Vec3::new(x, 1.15, z), Vec3::new(0.1, 1.15, 0.1), Look::DarkWood, true));
            }
            // Arka ve iki yan duvar; ön (+Z, oyuncuya bakan) açık.
            p.push(block(Vec3::new(0.0, 1.0, -1.5), Vec3::new(1.45, 1.0, 0.06), Look::Wood, true));
            p.push(block(Vec3::new(-1.5, 1.0, 0.0), Vec3::new(0.06, 1.0, 1.45), Look::Wood, true));
            p.push(block(Vec3::new(1.5, 1.0, 0.0), Vec3::new(0.06, 1.0, 1.45), Look::Wood, true));
            p.push(block(Vec3::new(0.0, 2.36, 0.0), Vec3::new(1.85, 0.07, 1.85), Look::Stone, true));
        }
    }
    p
}

/// Yapının yere oturduğu yükseklik: köprü iki ucunun yükseği, ötekiler ayak izinin en alçağı
/// (yamaçta bir kenarı havada kalmasın diye değil, gömülmesin diye — en alçak noktaya oturuyor).
fn ground_y(kind: Build, at: Vec3, rotation: Quat) -> f32 {
    let foot: &[Vec3] = match kind {
        Build::Bridge => &[Vec3::ZERO, Vec3::new(0.0, 0.0, -8.0)],
        Build::Hut => &[Vec3::new(-1.5, 0.0, -1.5), Vec3::new(1.5, 0.0, -1.5), Vec3::new(-1.5, 0.0, 1.5), Vec3::new(1.5, 0.0, 1.5)],
        Build::Fence => &[Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)],
        Build::Fire => &[Vec3::ZERO],
    };
    let ys = foot.iter().map(|f| {
        let p = at + rotation * *f;
        height(p.x, p.z)
    });
    match kind {
        Build::Bridge => ys.fold(f32::MIN, f32::max) + 0.12,
        _ => ys.fold(f32::MAX, f32::min),
    }
}

/// Yer uygun mu: köprü her yere; ötekiler fazla dik olmayan yere.
fn site_ok(kind: Build, at: Vec3, rotation: Quat) -> bool {
    match kind {
        Build::Bridge => true,
        _ => {
            let span = match kind {
                Build::Hut => 1.5,
                _ => 0.8,
            };
            let hs: Vec<f32> = [Vec3::new(-span, 0.0, -span), Vec3::new(span, 0.0, -span), Vec3::new(-span, 0.0, span), Vec3::new(span, 0.0, span)]
                .iter()
                .map(|f| {
                    let p = at + rotation * *f;
                    height(p.x, p.z)
                })
                .collect();
            let (lo, hi) = hs.iter().fold((f32::MAX, f32::MIN), |(lo, hi), h| (lo.min(*h), hi.max(*h)));
            hi - lo < span * 0.9
        }
    }
}

/// Bir yapıyı kurar: her katı parça kendi dünyadaki konumunda bir statik cisim, görünen parçalar
/// kökün çocukları. Oyuncunun kurduğu (`persist`) kökü `Structure` taşıyor — kayda o giriyor; bir
/// köyün evleri taşımıyor, hücreleriyle gelip gidiyorlar. Dönen ilki kök, ötekiler katı parçalar.
pub(super) fn spawn_structure(world: &mut World, kind: Build, at: Vec3, yaw: f32, visual: bool, persist: bool) -> Vec<Entity> {
    let rotation = Quat::from_rotation_y(yaw);
    let base = Vec3::new(at.x, ground_y(kind, at, rotation), at.z);
    let root = world.spawn_bundle((Transform::new(base).with_rotation(rotation), GlobalTransform::default()));
    if persist {
        world.add_component(root, Structure { kind, yaw });
    }
    let mut spawned = vec![root];
    for part in parts(kind) {
        if part.solid {
            let body = world.spawn_bundle((
                Transform::new(base + rotation * part.at).with_rotation(rotation * part.turn),
                GlobalTransform::default(),
                RigidBody::new_static(),
                Velocity::default(),
                Collider::box_collider(part.half),
            ));
            spawned.push(body);
        }
        if visual && !part.mesh.is_empty() {
            let piece = world.spawn_bundle((
                Transform::new(part.at).with_rotation(part.turn).with_scale(part.half),
                GlobalTransform::default(),
                MeshSource(part.mesh.into()),
                part.look.material(),
                MeshRenderer::new(),
            ));
            world.add_child(root, piece);
        }
    }
    if kind == Build::Fire {
        let light = world.spawn_bundle((
            Transform::new(Vec3::new(0.0, 0.7, 0.0)),
            GlobalTransform::default(),
            PointLight::new(Vec3::new(1.0, 0.62, 0.3), 6.0, 14.0),
            FireLight { phase: at.x * 1.7 + at.z * 0.9 },
        ));
        world.add_child(root, light);
    }
    spawned
}

fn spawn_ghost(world: &mut World, kind: Build) -> Entity {
    let root = world.spawn_bundle((Transform::new(Vec3::ZERO), GlobalTransform::default(), Ghost));
    for part in parts(kind) {
        if part.mesh.is_empty() {
            continue;
        }
        let piece = world.spawn_bundle((
            Transform::new(part.at).with_rotation(part.turn).with_scale(part.half),
            GlobalTransform::default(),
            MeshSource(part.mesh.into()),
            ghost_material(true),
            MeshRenderer::new(),
        ));
        world.add_child(root, piece);
    }
    root
}

fn ghost_material(valid: bool) -> MaterialDesc {
    let c = if valid { Vec4::new(0.35, 1.0, 0.5, 0.4) } else { Vec4::new(1.0, 0.3, 0.25, 0.4) };
    MaterialDesc::new().with_unlit(c).with_transparent(true)
}

/// Tarifi envanterde yapar; sonucu not olarak söyler.
pub(super) fn craft(world: &mut World, game: &mut Game, kind: Build) -> bool {
    let recipe = kind.recipe();
    let result = {
        let mut bags = world.query_mut::<&mut Inventory>();
        match bags.get_mut(game.player) {
            Some(mut bag) => bag.craft(&game.catalog, &recipe),
            None => return false,
        }
    };
    match result {
        Ok(()) => {
            game.sounds.push("uret");
            game.dirty = true;
            game.note = (format!("+{} {}", recipe.output.count, kind.name()), 1.5);
            true
        }
        Err(gizmo::app::inventory::CraftError::Missing { item, have, need }) => {
            game.note = (format!("{}: {} {} gerek, {have} var", kind.name(), need, game.catalog.name(&item)), 2.0);
            false
        }
        Err(_) => {
            game.note = ("Envanter dolu".to_string(), 2.0);
            false
        }
    }
}

fn held(world: &World, game: &Game, kind: Build) -> u32 {
    world.get::<Inventory>(game.player).map_or(0, |b| b.count(kind.item()))
}

/// B: inşa kipine gir, bir sonraki eldeki yapıya geç, ya da çık.
pub(super) fn cycle_build(world: &mut World, game: &mut Game) {
    let after = game.building.as_ref().map(|b| b.kind);
    if let Some(b) = game.building.take() {
        world.despawn_recursive(b.ghost);
    }
    let start = after.map_or(0, |k| k.code() as usize + 1);
    let next = Build::ALL[start.min(Build::ALL.len())..].iter().copied().find(|k| held(world, game, *k) > 0);
    match next {
        Some(kind) => {
            let ghost = if game.visual { spawn_ghost(world, kind) } else { world.spawn_bundle((Transform::new(Vec3::ZERO), Ghost)) };
            game.building = Some(Building { kind, turn: 0.0, ghost, valid: false });
            game.note = (format!("İnşa: {} — G / tık: koy · Q: döndür · B: sonraki", kind.name()), 2.5);
        }
        None if after.is_none() => {
            game.note = ("İnşa edecek bir şey yok — envanterde (I) üret".to_string(), 2.5);
        }
        None => {}
    }
}

/// Yapının nereye ve hangi yöne kurulacağı: oyuncunun önünde, kameranın baktığı yöne.
pub(super) fn placement(world: &World, game: &Game, kind: Build, turn: f32) -> (Vec3, f32) {
    let me = focus(world, game);
    let (forward, _) = camera_axes(game);
    let yaw = (-forward.x).atan2(-forward.z) + turn;
    let at = me + forward * kind.reach();
    (at, yaw)
}

/// İnşa kipinde her kare: hayaleti yerine koy, Q ile döndür, G / tık ile kur.
pub(super) fn update_building(world: &mut World, game: &mut Game, rotate: bool, place: bool) {
    let Some(mut b) = game.building.take() else { return };
    if game.driving.is_some() {
        world.despawn_recursive(b.ghost);
        return;
    }
    if rotate {
        b.turn += std::f32::consts::FRAC_PI_4;
    }
    let (at, yaw) = placement(world, game, b.kind, b.turn);
    let rotation = Quat::from_rotation_y(yaw);
    let valid = site_ok(b.kind, at, rotation);
    if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(b.ghost) {
        t.position = Vec3::new(at.x, ground_y(b.kind, at, rotation), at.z);
        t.rotation = rotation;
    }
    if valid != b.valid && game.visual {
        let pieces: Vec<Entity> = world.get::<gizmo::core::component::Children>(b.ghost).map(|c| c.0.clone()).unwrap_or_default();
        for piece in pieces {
            world.add_component(piece, ghost_material(valid));
        }
    }
    b.valid = valid;
    if place {
        if !valid {
            game.note = ("Burası fazla dik".to_string(), 1.5);
        } else if place_structure(world, game, b.kind, at, yaw) && held(world, game, b.kind) == 0 {
            // Bitti: bir sonrakine geç ya da çık.
            game.building = Some(b);
            cycle_build(world, game);
            return;
        }
    }
    game.building = Some(b);
}

/// Envanterden bir tane harcayıp yapıyı kurar.
pub(super) fn place_structure(world: &mut World, game: &mut Game, kind: Build, at: Vec3, yaw: f32) -> bool {
    let spent = {
        let mut bags = world.query_mut::<&mut Inventory>();
        bags.get_mut(game.player).is_some_and(|mut bag| bag.remove(kind.item(), 1))
    };
    if !spent {
        return false;
    }
    spawn_structure(world, kind, at, yaw, game.visual, true);
    game.sounds.push("kur");
    game.dirty = true;
    game.note = (format!("{} kuruldu", kind.name()), 1.5);
    true
}

/// Kamp ateşleri titriyor.
pub(super) fn flicker(world: &mut World, clock: f32) {
    for (fire, mut light) in world.query_mut::<(&FireLight, &mut PointLight)>().iter_mut() {
        let t = clock + fire.phase;
        light.intensity = 6.0 * (1.0 + 0.16 * (t * 11.0).sin() + 0.09 * (t * 23.7).sin() + 0.05 * (t * 5.3).sin());
    }
}

/// Kayda: her yapı (tür, x, y, z, yön).
pub(super) fn saved(world: &World) -> Vec<(u8, f32, f32, f32, f32)> {
    world
        .query::<(&Structure, &Transform)>()
        .iter()
        .map(|(s, t)| (s.kind.code(), t.position.x, t.position.y, t.position.z, s.yaw))
        .collect()
}

pub(super) fn restore(world: &mut World, list: &[(u8, f32, f32, f32, f32)], visual: bool) {
    for &(code, x, y, z, yaw) in list {
        if let Some(kind) = Build::from_code(code) {
            spawn_structure(world, kind, Vec3::new(x, y, z), yaw, visual, true);
        }
    }
}

/// Paneldeki tarif satırı.
pub(super) fn build_recipes(world: &mut World, root: Entity, font: FontId) {
    let row = world.spawn_bundle(NodeBundle {
        style: Style {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(6.0),
            padding: UiRect::all(Val::Px(9.0)),
            ..Default::default()
        },
        background_color: BackgroundColor(Vec4::new(0.1, 0.11, 0.15, 0.88)),
        ..Default::default()
    });
    world.add_child(root, row);
    for kind in Build::ALL {
        let r = kind.recipe();
        let cost: Vec<String> = r.inputs.iter().map(|s| format!("{} {}", s.count, if s.item == "tas" { "taş" } else { s.item.as_str() })).collect();
        let label = format!("{} ×{}\n{}", kind.name(), r.output.count, cost.join(" · "));
        let button = world.spawn_bundle(ButtonBundle {
            style: Style { width: Val::Px(COLS as f32 * (CELL + 6.0) / 4.0 - 6.0), height: Val::Px(58.0), ..Default::default() },
            background_color: BackgroundColor(Vec4::new(0.25, 0.26, 0.3, 0.95)),
            ..Default::default()
        });
        world.add_component(button, Text::screen(label, font, 15.0, Vec2::ZERO).with_anchor(TextAnchor::Center));
        world.add_component(button, RecipeButton(kind));
        world.add_child(row, button);
    }
}

/// Tarif düğmeleri: yapılabiliyorsa yeşil, üstündeyken açık, tıklanınca üret.
pub(super) fn update_recipes(world: &mut World, game: &mut Game) {
    let bag = world.get::<Inventory>(game.player).cloned().unwrap_or_default();
    let mut clicked = None;
    {
        let mut buttons = world.query_mut::<(&RecipeButton, &Interaction, &Clicked, &mut BackgroundColor)>();
        for (button, interaction, click, mut bg) in buttons.iter_mut() {
            let can = bag.check_craft(&game.catalog, &button.0.recipe()).is_ok();
            let base = if can { Vec4::new(0.2, 0.45, 0.28, 0.95) } else { Vec4::new(0.25, 0.26, 0.3, 0.95) };
            let colour = if matches!(*interaction, Interaction::Hovered | Interaction::Pressed) { base + Vec4::new(0.1, 0.1, 0.1, 0.0) } else { base };
            if bg.0 != colour {
                bg.0 = colour;
            }
            if click.0 {
                clicked = Some(button.0);
            }
        }
    }
    if let Some(kind) = clicked {
        craft(world, game, kind);
    }
}
