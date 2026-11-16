//! Küçük harita: sağ üst köşede, yukarıdan, kuzey (−Z) yukarıda.
//!
//! Motorun ikinci kamerası: bir `viewport`'u ve daha yüksek bir `order`'ı olan, odağın 160 m
//! üstünden aşağı bakan dik izdüşümlü (`Orthographic`) bir kamera — dünya kendi renkleriyle
//! çiziliyor: göller mavi, ormanlar koyu, karlı dağlar beyaz. Oyuncu haritanın tam ortasında
//! (kamera onu izliyor); üstüne yazı katmanında işaretler: ortada yürüdüğün yönün oku, kristaller
//! `*`, köyler `ev`, kayıklar `~` — her biri harita kamerasının kendi `world_to_screen`'iyle,
//! haritanın kenarında kırpılarak. **M** haritayı açar, kapar.

use super::*;
use gizmo::core::window::WindowInfo;
use gizmo::renderer::components::{ProjectionMode, Viewport};

/// Haritanın penceredeki yeri, pencerenin kesirleri olarak.
const VIEW: Viewport = Viewport::new(0.765, 0.035, 0.215, 0.3);
/// Haritanın dikey olarak gösterdiği, metre.
const SPAN: f32 = 260.0;
/// Kaç işaret var (kristal, köy, kayık).
const MARKS: usize = 24;

pub(super) struct Map {
    camera: Entity,
    me: Entity,
    title: Entity,
    marks: Vec<Entity>,
    pub(super) shown: bool,
}

pub(super) fn build(world: &mut World, font: FontId) -> Map {
    let camera = world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 160.0, 0.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -std::f32::consts::FRAC_PI_2 + 0.001,
        far: 600.0,
        primary: false,
        ..Default::default()
    });
    if let Some(mut c) = world.query_mut::<&mut Camera>().get_mut(camera) {
        c.viewport = Some(VIEW);
        c.order = 1;
        c.projection = ProjectionMode::Orthographic { height: SPAN };
        c.clear_color = Some(Vec3::new(0.1, 0.12, 0.1));
        // Yukarıdan bir harita gölge, ortam örtüşmesi, bloom istemiyor: sade çizim, tam bir ikinci
        // kare kadar pahalı değil.
        c.detail = gizmo::renderer::components::CameraDetail::Simple;
    }
    let at = |dx: f32, dy: f32| Vec2::new(VIEW.x + VIEW.width * dx, VIEW.y + VIEW.height * dy);
    let me = world.spawn_bundle((Text::pinned_at("↑", font, 24.0, at(0.5, 0.5), Vec2::ZERO)
        .with_anchor(TextAnchor::Center)
        .with_color(Vec4::new(1.0, 0.35, 0.2, 1.0)),));
    let title = world.spawn_bundle((Text::pinned_at("Harita · M", font, 15.0, at(0.0, 1.0), Vec2::new(4.0, 2.0)),));
    let marks = (0..MARKS)
        .map(|_| world.spawn_bundle((Text::pinned_at("", font, 17.0, at(0.0, 0.0), Vec2::ZERO).with_anchor(TextAnchor::Center),)))
        .collect();
    Map { camera, me, title, marks, shown: true }
}

fn set_mark(world: &mut World, e: Entity, label: &str, at: Vec2, offset: Vec2, colour: Vec4) {
    if let Some(mut t) = world.query_mut::<&mut Text>().get_mut(e) {
        if t.content != label {
            t.content = label.to_string();
        }
        t.space = gizmo::renderer::components::TextSpace::Pinned { at, offset };
        t.color = colour;
    }
}

/// Her kare: kamerayı odağın üstüne koy, işaretleri yerleştir. `toggle` M'ye basıldı mı.
pub(super) fn update(world: &mut World, map: &mut Map, me: Vec3, heading: Vec3, toggle: bool) {
    if toggle {
        map.shown = !map.shown;
        if let Some(mut c) = world.query_mut::<&mut Camera>().get_mut(map.camera) {
            // Görüntü alanı olmayan, birincil olmayan bir kamera çizilmiyor.
            c.viewport = map.shown.then_some(VIEW);
        }
    }
    let (eye, turn) = {
        let mut ts = world.query_mut::<&mut Transform>();
        let Some(mut t) = ts.get_mut(map.camera) else { return };
        t.position = Vec3::new(me.x, me.y + 160.0, me.z);
        (t.position, t.rotation)
    };
    let window = world.get_resource::<WindowInfo>().map(|w| Vec2::new(w.width, w.height));
    let size = window.map(|w| Vec2::new(w.x * VIEW.width, w.y * VIEW.height));
    // Yürüdüğün yön, sekiz okun biriyle (kuzey yukarıda).
    let arrows = ["↑", "↗", "→", "↘", "↓", "↙", "←", "↖"];
    let angle = heading.x.atan2(-heading.z);
    let k = ((angle / std::f32::consts::TAU * 8.0).round() as i32).rem_euclid(8) as usize;
    let me_label = if map.shown { arrows[k] } else { "" };
    let centre = Vec2::new(VIEW.x + VIEW.width * 0.5, VIEW.y + VIEW.height * 0.5);
    set_mark(world, map.me, me_label, centre, Vec2::ZERO, Vec4::new(1.0, 0.35, 0.2, 1.0));
    if let Some(mut t) = world.query_mut::<&mut Text>().get_mut(map.title) {
        let want = if map.shown { "Harita · M" } else { "" };
        if t.content != want {
            t.content = want.to_string();
        }
    }
    // İşaretler: haritanın içinde kalan kristaller, köyler, kayıklar.
    let mut found: Vec<(&'static str, Vec3, Vec4)> = Vec::new();
    if map.shown {
        // Kaba bir ön eleme; haritaya düşüp düşmediğini aşağıda kameranın kendisi söylüyor.
        let half = SPAN;
        let near = |p: Vec3| (p.x - me.x).abs() < half && (p.z - me.z).abs() < half;
        for (_, t) in world.query::<(&Crystal, &Transform)>().iter() {
            if near(t.position) {
                found.push(("*", t.position, Vec4::new(0.35, 0.95, 1.0, 1.0)));
            }
        }
        let active: Vec<ChunkCoord> = world.get_resource::<ChunkManager>().map(|m| m.active_chunks.iter().copied().collect()).unwrap_or_default();
        for coord in active {
            if let Some(v) = super::canli::village_at(coord) {
                if near(v) {
                    found.push(("ev", v, Vec4::new(1.0, 0.85, 0.4, 1.0)));
                }
            }
        }
        for (_, t) in world.query::<(&gizmo::physics::BoatController, &Transform)>().iter() {
            if near(t.position) {
                found.push(("~", t.position, Vec4::new(0.95, 0.95, 1.0, 1.0)));
            }
        }
        found.sort_by(|a, b| a.1.distance(me).total_cmp(&b.1.distance(me)));
    }
    let camera = world.get::<Camera>(map.camera).copied();
    let marks = map.marks.clone();
    for (i, e) in marks.into_iter().enumerate() {
        let placed = match (found.get(i), size, camera) {
            (Some((label, p, colour)), Some(size), Some(cam)) => cam
                .world_to_screen(*p, (size.x, size.y), eye, turn)
                .filter(|(x, y)| *x > 6.0 && *y > 6.0 && *x < size.x - 6.0 && *y < size.y - 6.0)
                .map(|(x, y)| (*label, Vec2::new(x, y), *colour)),
            _ => None,
        };
        match placed {
            Some((label, px, colour)) => set_mark(world, e, label, Vec2::new(VIEW.x, VIEW.y), px, colour),
            None => set_mark(world, e, "", Vec2::new(VIEW.x, VIEW.y), Vec2::ZERO, Vec4::ONE),
        }
    }
}

/// Sınama için: haritada bu kare kaç işaret görünüyor.
pub(super) fn shown_marks(world: &World, map: &Map) -> usize {
    map.marks.iter().filter(|e| world.get::<Text>(**e).is_some_and(|t| !t.content.is_empty())).count()
}
