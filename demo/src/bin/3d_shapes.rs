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
//! Kalan 20'sinin yapıcısı yoktu, ve eksikler bir liste değil **bir kavram**: 15 girdi
//! (8 ekstrüzyon + 7 halka) tek bir eksik yetenekten geliyordu.
//!
//! ## O yetenek 2026-08-24'te geldi: `primitives::extrude`
//!
//! Bir prizma, beşgen prizma, yuvarlatılmış dilim ve kare kesitli halka dört ayrı özellik değil;
//! tek bir makineye verilen dört **anahat**. Motorda artık ikisi var:
//!
//!   * `AssetManager::create_extrusion(outline, depth)` — kapalı bir 2B anahattı Z boyunca
//!     hacme çıkarır: iki kapak + her kenara bir dörtgen.
//!   * `AssetManager::create_sweep(profile, radius, segments)` — aynı anahattı Y ekseni etrafında
//!     döndürür. Dairesel profil torus, dikdörtgen profil kare kesitli halka verir.
//!
//! Anahatlar `extrude::outline` altında: `circle` (ki `circle(r, 5)` beşgendir — ayrı bir
//! "düzgün çokgen" yok, çünkü ayrı bir şekil yok), `ellipse`, `rectangle`, `stadium`, `star`.
//! Yeni bir şekil yeni bir `create_*` değil, `Vec<Vec2>` döndüren bir fonksiyon.
//!
//! **Zor olan yarısı kapaklar.** Duvar önemsiz — her kenar bir dörtgen. Kapaklar anahattın
//! üçgenlenmesini istiyor ve **üçgen yelpazesi** yalnız DIŞBÜKEY anahatta doğru; bu demonun eski
//! elle yazılmış beşgeni onu kullanıyordu. Yıldız, L ya da ok yelpazelenirse şeklin dışına üçgen
//! taşar. Motorunki kulak-kırpma, yani içbükey anahat da doğru çıkıyor — testi alan karşılaştırıyor,
//! üçgen sayısını değil, çünkü yanlış bir üçgenleme de doğru sayıda üçgen üretir.
//!
//! ## Hâlâ elle yazılanlar
//!
//! Beş şekil bu makineden çıkmıyor ve demo ikisini elle kuruyor: **dörtyüzlü** (4 üçgen) ve
//! **kesik koni** (iki farklı yarıçaplı silindir — sabit profilli bir süpürme değil, o yüzden
//! `create_sweep` onu vermiyor). Kalanlar: ikoza küre, doğru parçası, kırık çizgi.
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
use gizmo::renderer::asset::primitives::extrude::outline;
use gizmo::renderer::gpu_types::Vertex;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::TAU;

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

            // Orta sıra: tek makineden çıkan ekstrüzyonlar — beşi de aynı çağrı, farklı anahat.
            // Sonuncusu YILDIZ, yani içbükey: kapakları yelpaze ile üçgenleyen bir motor onu
            // gözle görülür biçimde yanlış çizer.
            let extrusions: [Mesh; 5] = [
                AssetManager::create_extrusion(device, &outline::rectangle(1.2, 0.8), 0.8)
                    .expect("dikdörtgen ekstrüde olur"),
                AssetManager::create_extrusion(device, &outline::circle(0.7, 5), 1.0)
                    .expect("beşgen ekstrüde olur"),
                AssetManager::create_extrusion(device, &outline::circle(0.7, 24), 1.0)
                    .expect("daire ekstrüde olur"),
                AssetManager::create_extrusion(device, &outline::stadium(0.35, 0.9, 8), 0.7)
                    .expect("stadyum ekstrüde olur"),
                AssetManager::create_extrusion(device, &outline::star(0.8, 0.33, 5), 0.5)
                    .expect("yıldız ekstrüde olur"),
            ];

            // Üst sıra: aynı anahatların Y ekseni etrafında süpürülmüş hâlleri (halka
            // ekstrüzyonu). Süpürme kapak istemediği için üçgenleme de istemiyor.
            let sweeps: [Mesh; 4] = [
                AssetManager::create_sweep(device, &outline::circle(0.22, 12), 0.7, 24)
                    .expect("torus"),
                AssetManager::create_sweep(device, &outline::rectangle(0.35, 0.35), 0.7, 24)
                    .expect("kare kesitli halka"),
                AssetManager::create_sweep(device, &outline::stadium(0.12, 0.3, 6), 0.7, 24)
                    .expect("yuvarlatılmış halka"),
                AssetManager::create_sweep(device, &outline::star(0.3, 0.12, 5), 0.75, 24)
                    .expect("yıldız kesitli halka"),
            ];

            // En üst sıra: makineden ÇIKMAYAN, hâlâ elle yazılan ikisi.
            let handmade: [Mesh; 2] = [
                Mesh::from_vertices(device, &tetrahedron(0.9), "3d_shapes::tetrahedron"),
                Mesh::from_vertices(
                    device,
                    &conical_frustum(0.7, 0.3, 1.2, 24),
                    "3d_shapes::conical_frustum",
                ),
            ];

            spawn_row(scene.world, &ready, &texture, 0.0, 0.0);
            spawn_row(scene.world, &extrusions, &texture, 2.4, 0.0);
            // Halkalar XZ düzleminde; kameraya kenarlarından bakmasınlar diye dikilmiş.
            spawn_row(scene.world, &sweeps, &texture, 4.6, -1.2);
            spawn_row(scene.world, &handmade, &texture, 6.8, 0.0);

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
            // Dört sıra var (y = 0 · 2,4 · 4,6 · 6,8), yani kamera hepsini alacak kadar geride.
            scene.spawn_camera(state, Vec3::new(0.0, 4.2, 19.0), Vec3::new(0.0, 3.2, 0.0));
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
                    ui.label("1. sıra: motorun hazır yapıcıları (6)");
                    ui.label("2. sıra: create_extrusion — tek makine, beş anahat");
                    ui.label("3. sıra: create_sweep — aynı anahatlar, Y ekseni etrafında");
                    ui.label("4. sıra: hâlâ elle yazılan ikisi");
                    ui.separator();
                    ui.label("katalogda 26 mesh:");
                    ui.label("  6  motorda hazır");
                    ui.label("  15 ekstrüzyon/halka — 2026-08-24'te tek makineyle geldi");
                    ui.label("  5  tek tek eksik (dörtyüzlü, kesik koni, ikoza küre,");
                    ui.label("     doğru parçası, kırık çizgi)");
                    ui.separator();
                    ui.label("2. sıranın sonundaki YILDIZ içbükey: kapakları");
                    ui.label("yelpazeleyen bir motor onu yanlış çizer.");
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
///
/// `tilt` X ekseni etrafında bir yatırma: süpürme halkaları XZ düzleminde duruyor, yani
/// yatırılmazlarsa kameraya kenarlarından bakıyorlar ve düz birer disk gibi görünüyorlar. Şeklin
/// kendisiyle ilgisi yok, bakış açısıyla ilgisi var.
fn spawn_row(
    world: &mut gizmo::core::World,
    meshes: &[Mesh],
    texture: &std::sync::Arc<gizmo::wgpu::BindGroup>,
    y: f32,
    tilt: f32,
) {
    let count = meshes.len();
    for (i, mesh) in meshes.iter().enumerate() {
        let x = if count > 1 {
            -X_EXTENT / 2.0 + i as f32 / (count - 1) as f32 * X_EXTENT
        } else {
            0.0
        };
        world.spawn_bundle((
            Transform::new(Vec3::new(x, y, 0.0)).with_rotation(Quat::from_rotation_x(tilt)),
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
