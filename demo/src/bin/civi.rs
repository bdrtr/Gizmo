//! # Çivi — çivili bir tahtadan dört hazneye
//!
//! Toplar tahtanın tepesinden birer birer bırakılır, çivilere çarpa çarpa iner ve en alttaki dört
//! hazneden birine düşer. Duvarlardaki kamalar kenara kaçanı geri iter.
//!
//! Motorun hangi yollarını yürüttüğü — oyunların hiçbirinin dokunmadığı **2D katmanı**:
//!
//! - **Toplar düzleme kilitli** (`RigidBody::planar_xy`): üç boyutlu şekiller, XY düzlemine
//!   kilitli gövdelerde. Kilit hızları kısıtlıyor; yüzlerce temasın konum düzeltmesi topu düzlemden
//!   kaçırmamalı.
//! - **Tahtanın çerçevesi bir ızgara** (`gizmo::physics::grid_collider`): duvarlar, zemin ve hazne
//!   bölmeleri hücre hücre, birleşik kutulara. Izgara yalnız bir `Tilemap`'ten, yani bir render
//!   bileşeninden kurulabiliyordu; bu oyun başsız çalıştığı için çerçeveyi kuramazdı.
//! - **Çiviler `circle_2d`, kamalar `polygon_2d`**, hepsi `RigidBody`'siz: statik geometri.
//!
//! `civi --test`: pencere açmadan bütün topları bırakır; her top bir haznede olmalı, tahtadan
//! çıkmamalı ve düzlemden hiç ayrılmamalı (z ve düzlem dışı dönüş sıfır).
use gizmo::core::input::Input;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};

const PHYSICS_HZ: u32 = 240;
const BALLS: usize = 24;
const BALL_R: f32 = 0.15;
const RELEASE_EVERY: f32 = 0.4;
/// Çerçevenin ızgarası: 17 × 24 hücre, hücre yarım metre.
const COLS: u32 = 17;
const ROWS: u32 = 24;
const CELL: f32 = 0.5;
/// Hazne bölmelerinin sütunları ve yüksekliği (alttan, zeminin üstünde kaç hücre).
const DIVIDERS: [u32; 3] = [4, 8, 12];
const BIN_ROWS: u32 = 4;
const DEPTH: f32 = 0.6;

#[derive(Clone, Copy)]
struct Ball;
gizmo::core::impl_component!(Ball);

struct Game {
    physics: FixedPhysics,
    released: usize,
    next_release: f32,
    clock: f32,
    hud: Entity,
}

/// Izgaranın sol üst köşesi, dünyada: tahta x'te ortalanmış, zemini y = 0'da.
fn grid_origin() -> Vec3 {
    Vec3::new(-(COLS as f32) * CELL / 2.0, ROWS as f32 * CELL, 0.0)
}

/// Hücre `(x, y)` katı mı: iki yan duvar, zemin ve hazne bölmeleri.
fn solid(x: u32, y: u32) -> bool {
    x == 0 || x == COLS - 1 || y == ROWS - 1 || (DIVIDERS.contains(&x) && y >= ROWS - 1 - BIN_ROWS)
}

/// Zeminin üstü ve iç duvarların yüzleri, dünyada.
fn floor_top() -> f32 {
    grid_origin().y - (ROWS - 1) as f32 * CELL
}
fn inner_wall() -> f32 {
    -grid_origin().x - CELL
}

fn build_board(world: &mut World) {
    let frame = gizmo::physics::grid_collider(COLS, ROWS, Vec2::splat(CELL), DEPTH, solid).expect("çerçevenin katı hücreleri var");
    world.spawn_bundle((Transform::new(grid_origin()), GlobalTransform::default(), frame));
    // Görünüş: her katı hücre bir küp.
    let wood = Vec4::new(0.55, 0.38, 0.22, 1.0);
    for y in 0..ROWS {
        for x in 0..COLS {
            if solid(x, y) {
                let centre = grid_origin() + Vec3::new((x as f32 + 0.5) * CELL, -(y as f32 + 0.5) * CELL, 0.0);
                world.spawn_bundle((
                    Transform::new(centre).with_scale(Vec3::new(CELL / 2.0, CELL / 2.0, DEPTH / 2.0)),
                    GlobalTransform::default(),
                    MeshSource("standard_cube".into()),
                    MaterialDesc::new().with_pbr(wood, 0.7, 0.0),
                    MeshRenderer::new(),
                ));
            }
        }
    }
    // Çiviler: şaşırtmalı yedi sıra.
    let brass = Vec4::new(0.85, 0.7, 0.3, 1.0);
    for row in 0..7 {
        let y = floor_top() + 4.4 + row as f32 * 0.8;
        let shift = if row % 2 == 0 { 0.0 } else { 0.5 };
        let mut x = -3.5 + shift;
        while x < inner_wall() - 0.2 {
            world.spawn_bundle((
                Transform::new(Vec3::new(x, y, 0.0)).with_scale(Vec3::new(0.08, 0.08, DEPTH / 2.0)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(brass, 0.3, 0.8),
                MeshRenderer::new(),
                Collider::circle_2d(0.08),
            ));
            x += 1.0;
        }
    }
    // Kamalar: iki duvarda, içeri bakan üçgenler.
    for (side, y) in [(-1.0f32, 3.5f32), (1.0, 3.5), (-1.0, 6.4), (1.0, 6.4)] {
        let wall = side * inner_wall();
        let tip = -side * 0.5;
        world.spawn_bundle((
            Transform::new(Vec3::new(wall, floor_top() + y, 0.0)),
            GlobalTransform::default(),
            Collider::polygon_2d(&[Vec2::new(0.0, -0.4), Vec2::new(tip, 0.0), Vec2::new(0.0, 0.4)], DEPTH),
        ));
    }
}

fn setup(world: &mut World, font: FontId) -> Game {
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.3) * Quat::from_rotation_x(-0.6)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, floor_top() + 6.0, 16.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: 0.0,
        primary: true,
        ..Default::default()
    });
    build_board(world);
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    Game { physics: FixedPhysics::new(PHYSICS_HZ), released: 0, next_release: 0.0, clock: 0.0, hud }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, _input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.clock += dt;
    if game.released < BALLS && game.clock >= game.next_release {
        // Ortanın çevresinde, topdan topa değişen bir yerden.
        let x = ((game.released as f32 * 0.37).sin()) * 0.4;
        world.spawn_bundle((
            Transform::new(Vec3::new(x, floor_top() + 10.6, 0.0)).with_scale(Vec3::splat(BALL_R)),
            GlobalTransform::default(),
            MeshSource("sphere".into()),
            MaterialDesc::new().with_pbr(Vec4::new(0.2, 0.5 + 0.02 * game.released as f32, 0.9, 1.0), 0.25, 0.2),
            MeshRenderer::new(),
            RigidBody::new(0.1, true).planar_xy(),
            Velocity::default(),
            Collider::circle_2d(BALL_R),
            Ball,
        ));
        game.released += 1;
        game.next_release = game.clock + RELEASE_EVERY;
    }
    game.physics.advance(world, dt);
    sync_transforms(world);
    let counts = bin_counts(world);
    if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        text.content = format!("Top: {} / {}   Hazneler: {:?}", game.released, BALLS, counts);
    }
    world.insert_resource(game);
}

/// Hazne `i`'nin x aralığı: duvarla ilk bölme arası, bölmeler arası, son bölmeyle duvar arası.
fn bin_of(x: f32) -> Option<usize> {
    let edges: Vec<f32> = std::iter::once(0u32)
        .chain(DIVIDERS)
        .chain(std::iter::once(COLS - 1))
        .map(|c| grid_origin().x + (c as f32 + 0.5) * CELL)
        .collect();
    edges.windows(2).position(|w| x > w[0] && x < w[1])
}

/// Hazne başına, bölmelerin boyunun altında duran top sayısı.
fn bin_counts(world: &World) -> [usize; 4] {
    let mut counts = [0; 4];
    for (t, _) in world.query::<(&Transform, &Ball)>().iter() {
        if t.position.y < floor_top() + BIN_ROWS as f32 * CELL {
            if let Some(i) = bin_of(t.position.x) {
                counts[i] += 1;
            }
        }
    }
    counts
}

/// `--test`: bütün toplar bırakılır, hepsi bir hazneye inene kadar (en çok bir dakika).
fn self_test() -> bool {
    let mut world = World::new();
    let game = setup(&mut world, HeadlessFrames::font());
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    // Düzlemden en çok ne kadar ayrıldılar: z'de ve düzlem dışına dönüşte.
    let (mut off_plane, mut off_axis) = (0.0f32, 0.0f32);
    let mut settled = false;
    while run.elapsed < 60.0 {
        run.frame(dt, update);
        let mut still = 0;
        for (t, v, _) in run.world.query::<(&Transform, &Velocity, &Ball)>().iter() {
            off_plane = off_plane.max(t.position.z.abs());
            off_axis = off_axis.max(t.rotation.x.abs().max(t.rotation.y.abs()));
            if v.linear.length() < 0.05 {
                still += 1;
            }
        }
        let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
        if game.released == BALLS && still == BALLS && bin_counts(&run.world).iter().sum::<usize>() == BALLS {
            settled = true;
            break;
        }
    }
    let counts = bin_counts(&run.world);
    let lost = run.world.query::<(&Transform, &Ball)>().iter().filter(|(t, _)| t.position.y < floor_top() - 0.5 || t.position.x.abs() > inner_wall()).count();
    let ok = settled && lost == 0 && off_plane < 1e-3 && off_axis < 1e-3;
    println!(
        "civi --test: {BALLS} top, hazneler {counts:?}, kaybolan {lost}, düzlemden en çok z {off_plane:.2e}, düzlem dışı dönüş {off_axis:.2e}, {:.0} s → {}",
        run.elapsed,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Çivi", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font);
            scene.world.insert_resource(game);
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
