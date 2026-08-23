//! # Şekil ilkellerinden mesh üretmek
//!
//! Şekil ilkelleri: matematiksel tanımdan mesh üretmek. Taranan katalog 26 mesh içeriyor ve
//! hepsine çalışma anında üretilmiş bir UV hata ayıklama dokusu giydiriliyor.
//!
//! ## Sayılan envanter: 26 şeklin 6'sının motorda hazır yapıcısı var
//!
//! Katalog üç kümeden oluşuyor — 11 katı şekil, 8 ekstrüzyon (2B şekli derinlik vererek
//! hacme çıkarmak) ve 7 halka ekstrüzyonu. Motorun `AssetManager`'ında karşılığı olanlar:
//!
//! | yetenek                    | Gizmo                        |
//! |----------------------------|------------------------------|
//! | dikdörtgenler prizması     | `create_cube`                |
//! | kapsül                     | `create_capsule`             |
//! | halka (torus)              | `create_torus`               |
//! | silindir                   | `create_cylinder`            |
//! | koni                       | `create_cone`                |
//! | küre (uv)                  | `create_sphere`              |
//!
//! Kalan 20'sinin yapıcısı yok, ve eksikler bir liste değil **bir kavram**: motorda
//! **ekstrüzyon makinesi hiç yok**. 15 girdi (8 ekstrüzyon + 7 halka) tek bir eksik yetenekten
//! geliyor. Geri kalan beşi ayrı ayrı eksik: dörtyüzlü, kesik koni, ikoza küre,
//! doğru parçası ve kırık çizgi.
//!
//! ## Eksik olan kapatılabilir, ve bedeli demoda görünüyor
//!
//! [`Mesh::from_vertices`] bir üçgen listesinden mesh kuruyor, yani eksik şekiller elle
//! yazılabilir. Demo üç tanesini yazıyor ve satır sayısını saklamıyor: **dörtyüzlü** (4 üçgen),
//! **kesik koni** (silindirin iki farklı yarıçaplısı) ve **beşgenin
//! ekstrüzyonu** (düzgün çokgene 1.0 derinlik vermek: iki kapak +
//! yan yüzler). Üçü de üst sırada, motorun hazır altısı alt sırada duruyor.
//!
//! Bir şey hazır geliyor: çalışma anında üretilen UV hata ayıklama dokusu motorda var —
//! [`AssetManager::create_uv_debug_texture`].
//!
//! ## Kontroller
//!   * **R** — dönmeyi durdur/başlat
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::renderer::gpu_types::Vertex;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::{PI, TAU};

/// Dönen şekillerin işareti.
#[derive(Clone, Copy)]
struct Shape;
gizmo::core::impl_component!(Shape);

/// Dönüşün açık olup olmadığı.
#[derive(Clone, Copy)]
struct Spinning(bool);
gizmo::core::impl_component!(Spinning);

/// Sıraların yatay yayılımı.
const X_EXTENT: f32 = 12.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - 3D Shapes", 1280, 720)
        .with_simple_scene(|scene, state| {
            let texture = scene.asset_manager.create_uv_debug_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Alt sıra: motorun hazır yapıcıları.
            let ready: [Mesh; 6] = [
                AssetManager::create_cube(device),
                AssetManager::create_capsule(device, 0.5, 0.5, 12, 24),
                AssetManager::create_torus(device, 0.7, 0.25, 24, 16),
                AssetManager::create_cylinder(device, 0.5, 1.2, 24),
                AssetManager::create_cone(device, 0.6, 1.2, 24),
                AssetManager::create_sphere(device, 0.7, 18, 32),
            ];

            // Üst sıra: motorda olmayan, elle kurulanlar.
            let handmade: [Mesh; 3] = [
                Mesh::from_vertices(device, &tetrahedron(0.9), "3d_shapes::tetrahedron"),
                Mesh::from_vertices(
                    device,
                    &conical_frustum(0.7, 0.3, 1.2, 24),
                    "3d_shapes::conical_frustum",
                ),
                Mesh::from_vertices(
                    device,
                    &extruded_polygon(0.7, 5, 1.0),
                    "3d_shapes::extruded_pentagon",
                ),
            ];

            spawn_row(scene.world, &ready, &texture, 0.0);
            spawn_row(scene.world, &handmade, &texture, 2.4);

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -2.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 40.0),
                Material::new(white).with_pbr(Vec4::new(0.10, 0.11, 0.13, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.7),
                intensity: 2.6,
                ..Default::default()
            });

            scene.world.insert_resource(Spinning(true));
            scene.spawn_camera(state, Vec3::new(0.0, 3.0, 14.0), Vec3::new(0.0, 1.0, 0.0));
        })
        .add_update_system(rotate.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let spinning = world
                .get_resource::<Spinning>()
                .map(|s| s.0)
                .unwrap_or(true);
            gizmo::egui::Area::new("shapes".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.set_min_width(420.0);
                    ui.heading("Şekil ilkelleri");
                    ui.label("alt sıra: motorun hazır yapıcıları (6)");
                    ui.label("üst sıra: elle kurulanlar (3)");
                    ui.separator();
                    ui.label("katalogda 26 mesh:");
                    ui.label("  6  motorda hazır");
                    ui.label("  15 ekstrüzyon/halka — motorda ekstrüzyon makinesi YOK");
                    ui.label("  5  tek tek eksik (dörtyüzlü, kesik koni, ikoza küre,");
                    ui.label("     doğru parçası, kırık çizgi)");
                    ui.separator();
                    ui.label(format!(
                        "dönüş: {} (R)",
                        if spinning { "açık" } else { "kapalı" }
                    ));
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Şekilleri döndürür — R ile durdurulabiliyor.
fn rotate(
    mut shapes: Query<(Mut<Transform>, With<Shape>)>,
    mut spinning: ResMut<Spinning>,
    input: Res<Input>,
    time: Res<Time>,
) {
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyR as u32) {
        spinning.0 = !spinning.0;
    }
    if !spinning.0 {
        return;
    }
    for (_entity, (mut transform, _)) in shapes.iter_mut() {
        transform.rotate_y(time.dt() / 2.0);
    }
}

/// Bir mesh dizisini yatay sıraya dizer.
fn spawn_row(
    world: &mut gizmo::core::World,
    meshes: &[Mesh],
    texture: &std::sync::Arc<gizmo::wgpu::BindGroup>,
    y: f32,
) {
    let count = meshes.len();
    for (i, mesh) in meshes.iter().enumerate() {
        let x = if count > 1 {
            -X_EXTENT / 2.0 + i as f32 / (count - 1) as f32 * X_EXTENT
        } else {
            0.0
        };
        world.spawn_bundle((
            Transform::new(Vec3::new(x, y, 0.0)),
            GlobalTransform::default(),
            mesh.clone(),
            Material::new(texture.clone()).with_pbr(Vec4::ONE, 0.55, 0.0),
            MeshRenderer::new(),
            Shape,
        ));
    }
}

// ── Motorun taşımadığı üç şekil ──────────────────────────────────────────────────────────────

/// Bir üçgeni köşe listesine yazar. Normal üç köşeden hesaplanıyor, yani yüzler düz gölgeleniyor.
fn push_triangle(out: &mut Vec<Vertex>, a: Vec3, b: Vec3, c: Vec3, uv: [[f32; 2]; 3]) {
    let normal = (b - a).cross(c - a).normalize_or_zero();
    for (position, tex_coords) in [(a, uv[0]), (b, uv[1]), (c, uv[2])] {
        out.push(Vertex {
            position: [position.x, position.y, position.z],
            normal: [normal.x, normal.y, normal.z],
            tex_coords,
            ..Default::default()
        });
    }
}

/// Dörtyüzlü: dört eşkenar üçgen.
fn tetrahedron(size: f32) -> Vec<Vertex> {
    // Birim küpün dört köşesi — aralarındaki uzaklıklar eşit, yani düzgün dörtyüzlü.
    let s = size / 2.0;
    let a = Vec3::new(s, s, s);
    let b = Vec3::new(s, -s, -s);
    let c = Vec3::new(-s, s, -s);
    let d = Vec3::new(-s, -s, s);
    let uv = [[0.5, 0.0], [0.0, 1.0], [1.0, 1.0]];

    let mut out = Vec::with_capacity(12);
    push_triangle(&mut out, a, b, c, uv);
    push_triangle(&mut out, a, c, d, uv);
    push_triangle(&mut out, a, d, b, uv);
    push_triangle(&mut out, b, d, c, uv);
    out
}

/// Kesik koni: alt ve üst yarıçapı farklı bir silindir.
///
/// Motorda `create_cylinder` (tek yarıçap) ve `create_cone` (üstü sıfır) var; aradaki hâl yok.
fn conical_frustum(bottom: f32, top: f32, height: f32, segments: u32) -> Vec<Vertex> {
    let half = height / 2.0;
    let mut out = Vec::new();
    for i in 0..segments {
        let (t0, t1) = (
            i as f32 / segments as f32 * TAU,
            (i + 1) as f32 / segments as f32 * TAU,
        );
        let (u0, u1) = (i as f32 / segments as f32, (i + 1) as f32 / segments as f32);

        let b0 = Vec3::new(t0.cos() * bottom, -half, t0.sin() * bottom);
        let b1 = Vec3::new(t1.cos() * bottom, -half, t1.sin() * bottom);
        let t0p = Vec3::new(t0.cos() * top, half, t0.sin() * top);
        let t1p = Vec3::new(t1.cos() * top, half, t1.sin() * top);

        // Yan yüz: iki üçgen.
        push_triangle(&mut out, b0, b1, t1p, [[u0, 1.0], [u1, 1.0], [u1, 0.0]]);
        push_triangle(&mut out, b0, t1p, t0p, [[u0, 1.0], [u1, 0.0], [u0, 0.0]]);
        // Kapaklar.
        push_triangle(
            &mut out,
            Vec3::new(0.0, -half, 0.0),
            b1,
            b0,
            [[0.5, 0.5], [u1, 1.0], [u0, 1.0]],
        );
        push_triangle(
            &mut out,
            Vec3::new(0.0, half, 0.0),
            t0p,
            t1p,
            [[0.5, 0.5], [u0, 0.0], [u1, 0.0]],
        );
    }
    out
}

/// Ekstrüzyon: bir düzgün çokgeni `depth` kadar derinlik vererek hacme çıkarmak.
///
/// **Motorda böyle bir makine yok** ve katalogdaki 26 girdinin 15'i tam olarak bundan geliyor —
/// yani buradaki otuz satır, eksik listesinin yarısından çoğunun tek gerekçesi.
fn extruded_polygon(radius: f32, sides: u32, depth: f32) -> Vec<Vertex> {
    let half = depth / 2.0;
    let corner = |i: u32, z: f32| {
        // Çokgen bir köşesi yukarı bakacak şekilde döndürülüyor.
        let angle = i as f32 / sides as f32 * TAU + PI / 2.0;
        Vec3::new(angle.cos() * radius, angle.sin() * radius, z)
    };
    let uv_of = |p: Vec3| {
        [
            p.x / (radius * 2.0) + 0.5,
            0.5 - p.y / (radius * 2.0),
        ]
    };

    let mut out = Vec::new();
    for i in 0..sides {
        let (a0, a1) = (corner(i, half), corner((i + 1) % sides, half));
        let (b0, b1) = (corner(i, -half), corner((i + 1) % sides, -half));

        // Ön ve arka kapak — yelpaze üçgenleri.
        push_triangle(
            &mut out,
            Vec3::new(0.0, 0.0, half),
            a0,
            a1,
            [[0.5, 0.5], uv_of(a0), uv_of(a1)],
        );
        push_triangle(
            &mut out,
            Vec3::new(0.0, 0.0, -half),
            b1,
            b0,
            [[0.5, 0.5], uv_of(b1), uv_of(b0)],
        );
        // Yan yüz.
        let (u0, u1) = (i as f32 / sides as f32, (i + 1) as f32 / sides as f32);
        push_triangle(&mut out, a0, b0, b1, [[u0, 0.0], [u0, 1.0], [u1, 1.0]]);
        push_triangle(&mut out, a0, b1, a1, [[u0, 0.0], [u1, 1.0], [u1, 0.0]]);
    }
    out
}
