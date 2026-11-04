//! # Makas — perdeden kırmızıyı kes
//!
//! Bir çubuktan asılı bir perde; ortasında kırmızı işaretli bir bölge var. Fareyle sürükleyerek
//! kumaşı kes: kırmızı bölgeyi perdeden ayırıp düşür, perdenin gerisi asılı kalsın. Ne kadar az
//! kesikle, o kadar iyi. Motorun yirmi yedinci oyunu ve **kumaş** kullanan ilki: perde bir `Cloth`
//! bileşeni, motorun fizik adımı sallıyor, motorun çizim yolu çiziyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **sol tuş basılı sürükle** | kes |
//! | **R** | yeni perde |
//!
//! `makas --test` pencere açmaz: kırmızı bölgenin çevresinden sekizgen bir kesik atar, kumaşı üç
//! saniye sallar ve kırmızının düştüğüne, mavinin asılı kaldığına bakar.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Dünyadaki bir `Cloth` hiç çizilmiyordu.** Fizik adımı her kumaşı simüle ediyordu ama iki
//!   çizim yolunun hiçbiri türü bilmiyordu; kumaş, düğümlerini yeniden bir yüzeye çevirecek ızgara
//!   genişliğini bile saklamıyordu. `cloth_demo` kumaşı ECS'nin dışında tutup kendi mesh'ini
//!   kendisi kuruyordu. **Düzeltildi:** `Cloth::grid_size` / `surface_triangles` (yırtık bir
//!   delik olarak), ve iki çizim yolu da `cloth_mesh_system`'i çağırıyor.

use gizmo::core::input::Input;
use gizmo::physics::cloth::Cloth;
use gizmo::physics::world::PhysicsWorld;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

const COLS: usize = 26;
const ROWS: usize = 20;
const SPACING: f32 = 0.22;
/// Perdenin sol alt köşesi, dünyada.
const ORIGIN: Vec3 = Vec3::new(-2.75, 0.4, 0.0);
/// Kırmızı bölge: perdenin düzleminde bir daire.
const RED_CENTRE: Vec2 = Vec2::new(0.2, 2.3);
const RED_RADIUS: f32 = 0.75;

/// Perde ve hangi düğümünün kırmızı olduğu.
fn curtain() -> (Cloth, Vec<bool>) {
    let mut cloth = Cloth::new(COLS, ROWS, SPACING, 0.05);
    for n in &mut cloth.nodes {
        n.position += ORIGIN;
        n.prev_position += ORIGIN;
    }
    for x in 0..COLS {
        cloth.pin_node((ROWS - 1) * COLS + x);
    }
    let red = cloth.nodes.iter().map(|n| Vec2::new(n.position.x, n.position.y).distance(RED_CENTRE) < RED_RADIUS).collect();
    (cloth, red)
}

/// İki düzlem parçası kesişiyor mu (XY'de).
fn crosses(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let cross = |o: Vec2, p: Vec2, q: Vec2| (p - o).perp_dot(q - o);
    let (d1, d2) = (cross(c, d, a), cross(c, d, b));
    let (d3, d4) = (cross(a, b, c), cross(a, b, d));
    (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
}

/// Bıçağın `from`→`to` çizgisinin kestiği bütün bağları siler; kaç tane gittiğini döndürür.
fn cut(cloth: &mut Cloth, from: Vec2, to: Vec2) -> usize {
    let before = cloth.constraints.len();
    let nodes = &cloth.nodes;
    cloth.constraints.retain(|k| {
        let (p, q) = (nodes[k.node_a].position, nodes[k.node_b].position);
        !crosses(from, to, Vec2::new(p.x, p.y), Vec2::new(q.x, q.y))
    });
    before - cloth.constraints.len()
}

/// Çubuğa (sabit düğümlere) hâlâ bağlı düğümler.
fn attached(cloth: &Cloth) -> Vec<bool> {
    let n = cloth.nodes.len();
    let mut adj = vec![Vec::new(); n];
    for k in &cloth.constraints {
        adj[k.node_a].push(k.node_b);
        adj[k.node_b].push(k.node_a);
    }
    let mut seen = vec![false; n];
    let mut stack: Vec<usize> = (0..n).filter(|&i| cloth.nodes[i].inv_mass == 0.0).collect();
    for &i in &stack {
        seen[i] = true;
    }
    while let Some(i) = stack.pop() {
        for &j in &adj[i] {
            if !seen[j] {
                seen[j] = true;
                stack.push(j);
            }
        }
    }
    seen
}

/// `(kırmızının ayrılan kesri, mavinin asılı kalan kesri)`.
fn score(cloth: &Cloth, red: &[bool]) -> (f32, f32) {
    let on = attached(cloth);
    let (mut red_n, mut red_off, mut blue_n, mut blue_on) = (0, 0, 0, 0);
    for (i, &r) in red.iter().enumerate() {
        if r {
            red_n += 1;
            red_off += usize::from(!on[i]);
        } else {
            blue_n += 1;
            blue_on += usize::from(on[i]);
        }
    }
    (red_off as f32 / red_n.max(1) as f32, blue_on as f32 / blue_n.max(1) as f32)
}

fn spawn_curtain(world: &mut World) -> (u32, Vec<bool>) {
    let (cloth, red) = curtain();
    let id = world
        .spawn_bundle((
            Transform::new(Vec3::ZERO),
            cloth,
            MaterialDesc::new().with_pbr(Color::srgb_hex("#3D6FB6").to_vec4(), 0.85, 0.0),
            Curtain,
        ))
        .id();
    (id, red)
}

#[derive(Clone, Copy)]
struct Curtain;
gizmo::core::impl_component!(Curtain);

/// `--test`: kırmızının çevresinden sekizgen bir kesik, sonra üç saniye.
fn self_test() -> bool {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let (id, red) = spawn_curtain(&mut world);
    let ring: Vec<Vec2> = (0..=8).map(|k| RED_CENTRE + Vec2::from_angle(k as f32 * std::f32::consts::TAU / 8.0) * (RED_RADIUS + 0.25)).collect();
    let mut snips = 0;
    let mut cuts = 0;
    if let Some(mut c) = world.borrow_mut::<Cloth>().get_mut(id) {
        for pair in ring.windows(2) {
            cuts += cut(&mut c, pair[0], pair[1]);
            snips += 1;
        }
    }
    let red_y0: f32 = {
        let c = world.borrow::<Cloth>();
        let c = c.get(id).unwrap();
        red.iter().zip(&c.nodes).filter(|(r, _)| **r).map(|(_, n)| n.position.y).sum::<f32>() / red.iter().filter(|r| **r).count() as f32
    };
    for _ in 0..180 {
        gizmo::systems::cpu_physics_step_system(&world, 1.0 / 60.0);
    }
    let c = world.borrow::<Cloth>();
    let c = c.get(id).unwrap();
    let (red_off, blue_on) = score(c, &red);
    let red_y: f32 = red.iter().zip(&c.nodes).filter(|(r, _)| **r).map(|(_, n)| n.position.y).sum::<f32>() / red.iter().filter(|r| **r).count() as f32;
    let fell = red_y < red_y0 - 1.0;
    let surface = c.surface_triangles().len();
    let ok = red_off == 1.0 && blue_on > 0.9 && fell && surface < (COLS - 1) * (ROWS - 1) * 2;
    println!(
        "makas --test: {snips} kesik, {cuts} bağ; kırmızının %{:.0}'i ayrıldı ve {:.2} m düştü, mavinin %{:.0}'i asılı; yüzey {surface} üçgen → {}",
        red_off * 100.0,
        red_y0 - red_y,
        blue_on * 100.0,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

struct Game {
    physics: FixedPhysics,
    curtain: u32,
    red: Vec<bool>,
    markers: Vec<u32>,
    blade: Option<Vec2>,
    snips: u32,
    hud: u32,
    banner: u32,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Makas", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.3) * Quat::from_rotation_x(-0.6)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 1.8, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 2.6, 7.5),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -0.05,
                primary: true,
                ..Default::default()
            });
            // Çubuk ve yer: yalnız görüntü.
            scene.world.spawn_bundle((
                Transform::new(ORIGIN + Vec3::new(COLS as f32 * SPACING * 0.5, (ROWS - 1) as f32 * SPACING + 0.05, 0.0))
                    .with_scale(Vec3::new(COLS as f32 * SPACING * 0.55, 0.05, 0.05)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#8D6E63").to_vec4(), 0.5, 0.3),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.05, 0.0)).with_scale(Vec3::new(6.0, 0.05, 3.0)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#E0D6C8").to_vec4(), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),)).id();
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 40.0, Vec2::new(0.5, 0.2), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();
            let mut game = Game { physics: FixedPhysics::new(60), curtain: 0, red: Vec::new(), markers: Vec::new(), blade: None, snips: 0, hud, banner };
            restart(scene.world, &mut game);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn restart(world: &mut World, game: &mut Game) {
    let old: Vec<u32> = world.borrow::<Curtain>().iter().map(|(id, _)| id).chain(game.markers.drain(..)).collect();
    for id in old {
        if let Some(e) = world.get_entity(id) {
            world.despawn(e);
        }
    }
    let (id, red) = spawn_curtain(world);
    game.curtain = id;
    // Kırmızı düğümlerin üstünde küçük işaretler; düğümle birlikte hareket ederler.
    for _ in red.iter().filter(|r| **r) {
        let m = world
            .spawn_bundle((
                Transform::new(Vec3::ZERO).with_scale(Vec3::splat(0.05)),
                GlobalTransform::default(),
                MeshSource("sphere".into()),
                MaterialDesc::new().with_unlit(Color::srgb_hex("#E74C3C").to_vec4()),
                MeshRenderer::new(),
            ))
            .id();
        game.markers.push(m);
    }
    game.red = red;
    game.snips = 0;
    game.blade = None;
    game.physics.reset();
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        restart(world, &mut game);
    }
    // Bıçak: basılıyken imlecin perde düzlemindeki izi.
    let at = gizmo::pointer::cursor_on_plane(world, Vec3::ZERO, Vec3::Z).map(|p| Vec2::new(p.x, p.y));
    if input.is_mouse_button_pressed(0) {
        if let (Some(prev), Some(now)) = (game.blade, at) {
            if prev.distance(now) > 0.01 {
                if let Some(mut c) = world.borrow_mut::<Cloth>().get_mut(game.curtain) {
                    cut(&mut c, prev, now);
                }
                if let Some(mut g) = world.get_resource_mut::<Gizmos>() {
                    g.draw_line(prev.extend(0.05), now.extend(0.05), [1.0, 1.0, 1.0, 1.0]);
                }
            }
        }
        if input.is_mouse_button_just_pressed(0) {
            game.snips += 1;
        }
        game.blade = at;
    } else {
        game.blade = None;
    }
    game.physics.advance(world, dt.min(0.05));
    sync_transforms(world);
    let (positions, red_off, blue_on) = {
        let c = world.borrow::<Cloth>();
        match c.get(game.curtain) {
            Some(c) => {
                let p: Vec<Vec3> = game.red.iter().zip(&c.nodes).filter(|(r, _)| **r).map(|(_, n)| n.position).collect();
                let (r, b) = score(c, &game.red);
                (p, r, b)
            }
            None => (Vec::new(), 0.0, 1.0),
        }
    };
    for (m, p) in game.markers.iter().zip(positions) {
        if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(*m) {
            t.position = p + Vec3::Z * 0.03;
        }
    }
    let text = format!(
        "kesik {}   kırmızı ayrıldı %{:.0}   mavi asılı %{:.0}\nsol tuşla sürükle: kes   R: yeni perde",
        game.snips,
        red_off * 100.0,
        blue_on * 100.0
    );
    set_text(world, game.hud, &text);
    let banner = if blue_on < 0.9 {
        "Fazla kestin — perde düştü.\nR ile yeniden"
    } else if red_off >= 1.0 {
        "Kırmızı kesildi!\nR ile yeniden"
    } else {
        ""
    };
    set_text(world, game.banner, banner);
    world.insert_resource(game);
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
