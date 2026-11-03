//! # Gizli — muhafızlara görünmeden çıkışa
//!
//! Yukarıdan bakan bir depo: muhafızlar devriye geziyor, önlerinde duvarda kesilen bir görüş konisi
//! var. Konide kalırsan şüphe doluyor; dolarsa yakalanırsın. Belgeyi al, yeşil çıkışa ulaş.
//! Motorun on dokuzuncu oyunu ve ilk **görme**si olanı: her muhafızın konisi her kare duvarlara
//! ışın atılarak yeniden kuruluyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **WASD · sol çubuk** | yürü |
//! | **R** | baştan |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Değişen bir `MeshData` ekrana yansımıyordu.** Koni her kare yeniden kuruluyor; yükleyici
//!   `MeshData`'yı yalnızca varlığın henüz `Mesh`'i yokken okuyordu — koniler ilk karedeki
//!   yerlerinde donup kaldı. **Düzeltildi:** değişen veri yeniden yükleniyor, eskisinin GPU
//!   tamponu hemen bırakılıyor (oyun yolu önbelleği hiç temizlemiyordu; her kare yeni bir koni
//!   sonsuz büyüyen bir önbellek olurdu), boşaltılan veri mesh'i kaldırıyor.
//! - **Kendi hücresine gönderilen bir ajan hiç varmıyordu.** Devriyenin ilk noktası muhafızın
//!   durduğu yer; rotası boş olan ajan `Moving`'de, hedefi takılı kalıyordu — muhafızlar
//!   kıpırdamadı. **Düzeltildi:** rotasız ajan da hedefine varınca `Reached`.
//! - **Çarpıştırıcısı olmayan bir `RigidBody` sessizce simüle edilmiyordu.** Muhafızlar kinematik
//!   gövdeydi ama `Collider`'ları yoktu: `NavAgent` hızlarını yazdı, hiçbiri hareket etmedi, tek
//!   satır uyarı yoktu. **Düzeltildi:** fizik, sayı arttığında bunu uyarıyor.
//! - **İki ayrı `Ray` vardı, aralarında dönüşüm yoktu.** `gizmo::math::Ray` (imleç ışını) ile
//!   `PhysicsWorld::raycast`'in istediği `gizmo::physics::Ray`. **Kapatıldı:** iki yönlü `From`.
//!
//! Gerisi ilk denemede çalıştı: `CharacterController` (artık `FixedPhysics` içinde de),
//! `raycast_excluding` (göz, muhafızın kendi çarpıştırıcısının içinde), `NavGrid` devriyesi.
//!
//! ## Ortam değişkenleri
//!   * `GIZLI_OTO=1` — oyuncu yerinde durur (ekran görüntüsü için)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::ai::{NavAgent, NavGrid};
use gizmo::core::input::Input;
use gizmo::physics::Ray;
use gizmo::physics::components::CharacterController;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{MeshData, Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::renderer::Vertex;
use gizmo::simple::{sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

/// `#` duvar, `.` zemin, `S` başlangıç, `E` çıkış, `D` belge, `a`…`d` devriye noktaları.
const MAP: [&str; 13] = [
    "###################",
    "#S....#.....#.....#",
    "#.....#..a..#..b..#",
    "#..####.....#.....#",
    "#.......##.....####",
    "#####...##........#",
    "#.....c.......#...#",
    "#.###....######...#",
    "#...#....#....#.D.#",
    "#...#.d..#....#...#",
    "#.......##........#",
    "#.....#......#...E#",
    "###################",
];
/// Muhafızlar ve gezdikleri noktalar.
const PATROLS: [&[char]; 2] = [&['a', 'c', 'd', 'c'], &['b', 'd']];
const SIGHT: f32 = 7.0;
const HALF_ANGLE: f32 = 0.6;
const RAYS: usize = 24;

#[derive(Clone, Copy)]
struct Guard {
    route: usize,
    next: usize,
    cone: u32,
}
gizmo::core::impl_component!(Guard);

struct Game {
    physics: FixedPhysics,
    player: u32,
    start: Vec3,
    exit: Vec3,
    document: Vec3,
    has_document: bool,
    suspicion: f32,
    points: Vec<(char, Vec3)>,
    camera: u32,
    hud: u32,
    banner: u32,
    over: Option<bool>,
    auto: bool,
}

fn cell(x: usize, z: usize) -> Vec3 {
    Vec3::new(x as f32 + 0.5, 0.0, z as f32 + 0.5)
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Gizli", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-1.1)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(0.8, 0.85, 1.0), 1.6, LightRole::Sun),
            ));
            let (w, h) = (MAP[0].len() as f32, MAP.len() as f32);
            let camera = scene
                .world
                .spawn_bundle(CameraBundle {
                    position: Vec3::new(w * 0.5, 17.0, h * 0.5 + 6.0),
                    yaw: -std::f32::consts::FRAC_PI_2,
                    pitch: -1.2,
                    primary: true,
                    ..Default::default()
                })
                .id();
            // Zemin ve duvarlar; ızgara muhafızların yol bulması için.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(w * 0.5, -0.1, h * 0.5)),
                RigidBody::new_static(),
                Velocity::default(),
                Collider::box_collider(Vec3::new(w * 0.5, 0.1, h * 0.5)),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(w * 0.5, -0.1, h * 0.5)).with_scale(Vec3::new(w * 0.5, 0.1, h * 0.5)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#3A3F47").to_vec4(), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            let mut grid = NavGrid::new(1.0, w as i32, h as i32);
            grid.needs_rebuild = false;
            let (mut start, mut exit, mut document) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
            let mut points = Vec::new();
            for (z, row) in MAP.iter().enumerate() {
                for (x, c) in row.chars().enumerate() {
                    let at = cell(x, z);
                    match c {
                        '#' => {
                            grid.add_obstacle_world(at);
                            scene.world.spawn_bundle((
                                Transform::new(at + Vec3::Y * 0.6).with_scale(Vec3::new(0.5, 0.6, 0.5)),
                                GlobalTransform::default(),
                                MeshSource("standard_cube".into()),
                                MaterialDesc::new().with_pbr(Color::srgb_hex("#6B6E73").to_vec4(), 0.9, 0.0),
                                MeshRenderer::new(),
                                RigidBody::new_static(),
                                Velocity::default(),
                                Collider::box_collider(Vec3::new(0.5, 0.6, 0.5)),
                            ));
                        }
                        'S' => start = at,
                        'E' => exit = at,
                        'D' => document = at,
                        'a'..='d' => points.push((c, at)),
                        _ => {}
                    }
                }
            }
            scene.world.insert_resource(grid);
            // Çıkış: yeşil bir kare.
            scene.world.spawn_bundle((
                Transform::new(exit + Vec3::Y * 0.02).with_scale(Vec3::new(0.45, 0.02, 0.45)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_unlit(Color::srgb_hex("#3CB371").to_vec4()),
                MeshRenderer::new(),
            ));
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),)).id();
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.35), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();
            let mut game = Game {
                physics: FixedPhysics::new(60),
                player: 0,
                start,
                exit,
                document,
                has_document: false,
                suspicion: 0.0,
                points,
                camera,
                hud,
                banner,
                over: None,
                auto: std::env::var_os("GIZLI_OTO").is_some(),
            };
            restart(scene.world, &mut game);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

#[derive(Clone, Copy)]
struct Actor;
gizmo::core::impl_component!(Actor);

fn restart(world: &mut World, game: &mut Game) {
    let actors: Vec<u32> = world.borrow::<Actor>().iter().map(|(id, _)| id).collect();
    for id in actors {
        if let Some(e) = world.get_entity(id) {
            use gizmo::core::hierarchy::HierarchyExt;
            world.despawn_recursive(e);
        }
    }
    let controller = CharacterController { speed: 3.5, ..Default::default() };
    game.player = world
        .spawn_bundle((
            Transform::new(game.start + Vec3::Y * 0.5).with_scale(Vec3::new(0.3, 0.5, 0.3)),
            GlobalTransform::default(),
            MeshSource("cylinder".into()),
            MaterialDesc::new().with_pbr(Color::srgb_hex("#E0C060").to_vec4(), 0.5, 0.0),
            MeshRenderer::new(),
            controller,
            Velocity::default(),
            Collider::capsule(0.3, 0.2),
            Actor,
        ))
        .id();
    world.spawn_bundle((
        Transform::new(game.document + Vec3::Y * 0.3).with_scale(Vec3::new(0.25, 0.03, 0.18)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_unlit(Color::srgb_hex("#F5F5DC").to_vec4()),
        MeshRenderer::new(),
        Actor,
        Document,
    ));
    for (route, patrol) in PATROLS.iter().enumerate() {
        let at = game.points.iter().find(|(c, _)| *c == patrol[0]).map(|p| p.1).unwrap_or(game.start);
        let cone = world
            .spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                MeshData::new(Vec::<Vertex>::new()),
                MaterialDesc::new().with_unlit(Vec4::new(1.0, 0.85, 0.3, 0.35)),
                MeshRenderer::new(),
                Actor,
            ))
            .id();
        let mut agent = NavAgent::new(1.6, 8.0, 0.3).with_separation(0.0, 0.0);
        agent.set_target(at);
        world.spawn_bundle((
            Transform::new(at + Vec3::Y * 0.5).with_scale(Vec3::new(0.32, 0.5, 0.32)),
            GlobalTransform::default(),
            MeshSource("cylinder".into()),
            MaterialDesc::new().with_pbr(Color::srgb_hex("#C0392B").to_vec4(), 0.5, 0.0),
            MeshRenderer::new(),
            RigidBody::new_kinematic(),
            Velocity::default(),
            Collider::sphere(0.32),
            agent,
            Guard { route, next: 0, cone },
            Actor,
        ));
    }
    game.has_document = false;
    game.suspicion = 0.0;
    game.over = None;
    game.physics.reset();
}

#[derive(Clone, Copy)]
struct Document;
gizmo::core::impl_component!(Document);

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        restart(world, &mut game);
    }
    if game.over.is_none() {
        let (mx, my) = if game.auto { (0.0, 0.0) } else { input.move_axis() };
        if let Some(mut c) = world.borrow_mut::<CharacterController>().get_mut(game.player) {
            c.target_velocity = Vec3::new(mx, 0.0, -my) * c.speed;
        }
        patrol(world, &game);
        game.physics.advance(world, dt);
        gizmo::ai::system::ai_frame(world, dt);
        sync_transforms(world);
        sight(world, &mut game, dt);
        pickups(world, &mut game);
    }
    hud(world, &game);
    world.insert_resource(game);
}

/// Bir muhafız hedefine varınca rotasının bir sonraki noktasına yönelir.
fn patrol(world: &mut World, game: &Game) {
    let guards: Vec<u32> = world.borrow::<Guard>().iter().map(|(id, _)| id).collect();
    for id in guards {
        let done = world.borrow::<NavAgent>().get(id).is_some_and(|a| a.target.is_none());
        if !done {
            continue;
        }
        let Some(mut g) = world.borrow::<Guard>().get(id).copied() else { continue };
        g.next = (g.next + 1) % PATROLS[g.route].len();
        let label = PATROLS[g.route][g.next];
        if let Some(at) = game.points.iter().find(|(c, _)| *c == label).map(|p| p.1) {
            if let Some(mut a) = world.borrow_mut::<NavAgent>().get_mut(id) {
                a.set_target(at);
            }
        }
        if let Some(mut stored) = world.borrow_mut::<Guard>().get_mut(id) {
            *stored = g;
        }
    }
}

/// Her muhafızın görüş konisi: önüne yayılan ışınlar duvarda kesiliyor; oyuncu konide ve
/// görünürse şüphe artar.
fn sight(world: &mut World, game: &mut Game, dt: f32) {
    let player = world.borrow::<Transform>().get(game.player).map(|t| t.position).unwrap_or(game.start);
    let guards: Vec<(u32, Vec3, Vec3, u32)> = {
        let gs = world.borrow::<Guard>();
        let ts = world.borrow::<Transform>();
        let vs = world.borrow::<Velocity>();
        gs.iter()
            .filter_map(|(id, g)| {
                let at = ts.get(id)?.position;
                let v = vs.get(id).map(|v| v.linear).unwrap_or(Vec3::Z);
                let facing = Vec3::new(v.x, 0.0, v.z).try_normalize().unwrap_or(Vec3::Z);
                Some((id, at, facing, g.cone))
            })
            .collect()
    };
    let mut seen = false;
    for (id, at, facing, cone) in guards {
        // The eye is inside the guard's own collider: every ray must skip it.
        let me = gizmo::physics::BodyHandle::from_id(id);
        let eye = Vec3::new(at.x, 0.6, at.z);
        let mut vertices = Vec::with_capacity(RAYS * 3);
        let ends: Vec<Vec3> = {
            let Some(physics) = world.get_resource::<PhysicsWorld>() else { continue };
            (0..=RAYS)
                .map(|i| {
                    let a = -HALF_ANGLE + 2.0 * HALF_ANGLE * i as f32 / RAYS as f32;
                    let dir = Quat::from_rotation_y(a) * facing;
                    let d = physics.raycast_excluding(&Ray::new(eye, dir), SIGHT, me).map_or(SIGHT, |hit| hit.distance);
                    eye + dir * d
                })
                .collect()
        };
        for pair in ends.windows(2) {
            for p in [eye, pair[1], pair[0]] {
                vertices.push(Vertex { position: [p.x, 0.03, p.z], ..Default::default() });
            }
        }
        if let Some(mut m) = world.borrow_mut::<MeshData>().get_mut(cone) {
            *m = MeshData::new(vertices);
        }
        // Oyuncu konide mi ve arada duvar var mı?
        let to = Vec3::new(player.x, 0.6, player.z) - eye;
        let dist = to.length();
        if dist < SIGHT && dist > 1e-3 && to.normalize().dot(facing) > HALF_ANGLE.cos() {
            let blocked = world
                .get_resource::<PhysicsWorld>()
                .and_then(|p| p.raycast_excluding(&Ray::new(eye, to / dist), dist - 0.4, me))
                .is_some();
            if !blocked {
                seen = true;
            }
        }
    }
    game.suspicion = if seen { (game.suspicion + dt * 0.9).min(1.0) } else { (game.suspicion - dt * 0.4).max(0.0) };
    if game.suspicion >= 1.0 {
        game.over = Some(false);
    }
}

fn pickups(world: &mut World, game: &mut Game) {
    let Some(p) = world.borrow::<Transform>().get(game.player).map(|t| t.position) else { return };
    let flat = |a: Vec3, b: Vec3| Vec2::new(a.x - b.x, a.z - b.z).length();
    if !game.has_document && flat(p, game.document) < 0.6 {
        game.has_document = true;
        let doc: Vec<u32> = world.borrow::<Document>().iter().map(|(id, _)| id).collect();
        for id in doc {
            if let Some(e) = world.get_entity(id) {
                world.despawn(e);
            }
        }
    }
    if game.has_document && flat(p, game.exit) < 0.6 {
        game.over = Some(true);
    }
    let _ = game.camera;
}

fn hud(world: &mut World, game: &Game) {
    let doc = if game.has_document { "belge alındı — çıkışa!" } else { "belgeyi bul" };
    let bar = "#".repeat((game.suspicion * 20.0) as usize);
    set_text(world, game.hud, &format!("{doc}\nşüphe [{bar:<20}]\nWASD: yürü   R: baştan"));
    let banner = match game.over {
        Some(true) => "Kaçtın!\nR ile yeniden",
        Some(false) => "Yakalandın.\nR ile yeniden",
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
