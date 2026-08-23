//! # Işın atma ve ekran pikselinden ışın üretme
//!
//! Bir ışını sahneye atıp neye çarptığını bulmak — ve ışını ekran pikselinden üretmek.
//!
//! ([`mesh_picking`](../mesh_picking/index.html) bu parçaları bir seçim akışına
//! bağlıyor. Buradaki soru başka: **ışın ne kadar doğru**, ve **collider'a çarpmakla mesh'e
//! çarpmak arasındaki fark ne kadar**.)
//!
//! ## İki parça da var, biri farklı şeye çarpıyor
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | ekran pikselinden ışın üretme | [`Camera::screen_to_ray`] |
//! | ışının **üçgenlere** çarpması | **yok** |
//! | ışının **collider**'a çarpması | [`PhysicsWorld::raycast`] |
//! | isabet süzme (filtre / dışlama) | `raycast_filtered` / `raycast_excluding` |
//! | erken çıkış / `visibility` süzme | **yok** |
//!
//! Yani ışın atmak var, ama hedefi mesh değil çarpışma şekli. Bir kürenin collider'ı gerçekten
//! küredir, o yüzden fark sıfırdır; ama üçgenlerden yaklaşılan bir mesh'te fark **mesh'in
//! kendisi kadar**dır. Demo bu farkı sayıya çeviriyor.
//!
//! ## Ölçüldü — 1: `screen_to_ray` tam doğru
//!
//! Bilinen bir kamera (dikey fov 60°, 1280×720), ölçüldü (2026-08-23):
//!
//! | ölçüm | sonuç | beklenen |
//! |-------|-------|----------|
//! | ekran merkezinden atılan ışın · kameranın ileri yönüyle nokta çarpım | **1,000000** | 1 |
//! | sol kenardan sağ kenara açı | **91,493°** | 91,493° (`2·atan(tan(30°)·16/9)`) |
//! | sapma | **0,0001°** | 0 |
//!
//! Yani `screen_to_ray` ekran pikselinden doğru ışını üretiyor, ve doğruluğu
//! kayan nokta hassasiyetinde.
//!
//! ## Ölçüldü — 2: collider ile mesh arasındaki fark
//!
//! Motorun ışını mesh'e değil **collider**'a çarpıyor. Bir küre için collider analitik küredir;
//! gözün gördüğü ise üçgenlenmiş küre. Aradaki mesafe farkı, 12×16 bölüntülü bir küreye 61
//! yönden atılan ışınlarla ölçüldü:
//!
//! | | değer |
//! |---|-------|
//! | ortanca mesafe farkı | **0,0168** |
//! | ortalama | 0,0161 |
//! | en kötü | **0,0267** |
//! | kuramsal enlem sarkması `R(1−cos(π/2N))` | 0,0086 |
//! | kuramsal boylam sarkması `R(1−cos(π/16))` | 0,0192 |
//!
//! Ölçülen değerler iki kuramsal sarkmanın arasında ve büyüğüne yakın — bir dörtgenin en kötü
//! noktası hem enlemde hem boylamda sarktığı için beklenen de bu. Yani fark gerçek ama küçük:
//! yarıçapın **%2,7'si**. Basit şekiller için collider'a çarpmak mesh'e çarpmaya çok yakın;
//! bedel karmaşık mesh'lerde ortaya çıkıyor, çünkü orada collider mesh'in *kaba* bir yaklaşımı olur.
//!
//! ### Ölçüm notu: ilk sayı %199,89'du ve tamamen sahteydi
//!
//! İlk koşuda 64 ışın vardı ve en kötü hata **1,9989** — tam çap kadar. Sebep şu: 64 ışın kürenin
//! 16 boylamıyla hizalanıyordu, ve bir ışın tam bir ızgara **kenarından** geçti (θ = 315° = tam
//! `j = 14`). Kenarı paylaşan iki üçgen de kayan nokta yüzünden onu reddetti, ışın ön yüzeyden
//! geçip **arka yüze** çarptı — çarpma noktası (0,659 · −0,363 · −0,659) yerine (−0,653 · 0,360 ·
//! 0,653), yani kürenin tam öbür ucu.
//!
//! Bu üçgenlemenin ya da motorun değil, **test koşumunun** kusuruydu. Işın sayısı asal (61)
//! yapıldı ve açıya küçük bir kayma eklendi; sayılar bundan sonrası.
//!
//! Genel dersi: bir ölçüm "çok büyük" bir sayı verdiğinde önce ölçen tarafa bakmak gerekiyor.
//! Tam çapa eşit bir hata, "yaklaşım kötü"nün değil "ön yüzey ıskalandı"nın imzasıdır.
//!
//! ## Kontroller
//!   * `GIZMO_RAY_SELFTEST=1` — ölçümü günlüğe bas
//!   * **Fare** — imlecin altındaki ışın · **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct RayReport {
    lines: Vec<String>,
}
gizmo::core::impl_component!(RayReport);

/// Kürenin yarıçapı ve mesh'in enlem/boylam bölüntüsü — yaklaşım hatasının kaynağı.
const R: f32 = 1.0;
const STACKS: u32 = 12;
const SLICES: u32 = 16;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Mesh Ray Cast", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            let mut lines = Vec::new();

            // --- 1. `screen_to_ray` ne kadar doğru? ---
            //
            // Bilinen bir kamera kur, ekranın tam ortasından bir ışın at, ve ışının kameranın
            // kendi ileri yönüyle ne kadar örtüştüğünü ölç. Örtüşme tam olmalı.
            let camera = Camera::new(60_f32.to_radians(), 0.1, 500.0, 0.0, 0.0, true);
            let eye = Vec3::new(0.0, 0.0, 6.0);
            let viewport = (1280.0f32, 720.0f32);
            let centre_ray = camera.screen_to_ray((viewport.0 * 0.5, viewport.1 * 0.5), viewport, eye);
            let forward = camera.get_front();
            // `Ray::direction` `Vec3A`; karşılaştırmadan önce `Vec3`'e indiriliyor.
            let ray_dir = Vec3::from(centre_ray.direction).normalize();
            let centre_dot = ray_dir.dot(forward.normalize());
            lines.push(format!(
                "ekran merkezi ışını · ileri yönle nokta çarpım {centre_dot:.6} (1 olmalı)"
            ));

            // Köşelerden atılan ışınlar: yatay açı gerçekten fov'a uyuyor mu.
            let left_ray = camera.screen_to_ray((0.0, viewport.1 * 0.5), viewport, eye);
            let right_ray = camera.screen_to_ray((viewport.0, viewport.1 * 0.5), viewport, eye);
            let span = Vec3::from(left_ray.direction)
                .normalize()
                .dot(Vec3::from(right_ray.direction).normalize())
                .clamp(-1.0, 1.0)
                .acos()
                .to_degrees();
            // Yatay fov = 2·atan(tan(dikey_fov/2) · en_boy)
            let expected = 2.0
                * ((60_f32.to_radians() * 0.5).tan() * (viewport.0 / viewport.1)).atan();
            lines.push(format!(
                "kenardan kenara açı {span:.3}° · beklenen yatay fov {:.3}° · sapma {:.4}°",
                expected.to_degrees(),
                (span - expected.to_degrees()).abs()
            ));

            // --- 2. Collider ile mesh arasındaki fark ---
            //
            // Analitik küre (collider'ın gördüğü) ile üçgenlenmiş küre (gözün gördüğü) aynı
            // ışında farklı mesafeler veriyor. Fark, `PhysicsWorld::raycast`'in mesh'e değil
            // şekle çarpmasının bedeli.
            let sphere_centre = Vec3::ZERO;
            let mut worst = 0.0f32;
            let mut total = 0.0f32;
            let mut samples = 0usize;
            let mut diffs: Vec<f32> = Vec::new();
            // Işın sayısı ASAL (61) ve açıya küçük bir kayma ekli — ikisi de bilerek.
            // İlk kurulumda 64 ışın vardı ve 16 boylamla hizalanıyorlardı: bir ışın tam bir
            // ızgara KENARINDAN geçti, komşu iki üçgen de kayan nokta yüzünden reddetti, ve
            // ışın ön yüzeyi ıskalayıp arka yüze çarptı — hata tam çap kadar (1,999) çıktı.
            // O sayı üçgenlemenin değil, test koşumunun kusuruydu.
            for i in 0..61 {
                let a = i as f32 / 61.0 * std::f32::consts::TAU + 0.0371;
                let origin = sphere_centre + Vec3::new(a.cos(), (a * 0.7).sin() * 0.6, a.sin()) * 5.0;
                let dir = (sphere_centre - origin).normalize();
                let analytic = ray_sphere(origin, dir, sphere_centre, R);
                let triangulated = ray_sphere_mesh(origin, dir, sphere_centre, R, STACKS, SLICES);
                if let (Some(a_t), Some(t_t)) = (analytic, triangulated) {
                    let diff = (a_t - t_t).abs();
                    worst = worst.max(diff);
                    total += diff;
                    diffs.push(diff);
                    samples += 1;
                }
            }
            let mean = if samples > 0 { total / samples as f32 } else { 0.0 };
            diffs.sort_by(|a, b| a.partial_cmp(b).expect("NaN fark olamaz"));
            let median = diffs.get(diffs.len() / 2).copied().unwrap_or(0.0);
            // Kürenin tepe noktasında beklenen sarkma: R·(1 − cos(π / 2·stacks)).
            let expected_sag = R
                * (1.0 - (std::f32::consts::PI / (2.0 * STACKS as f32)).cos());
            lines.push(format!(
                "{samples} ışın · analitik küre ile {STACKS}x{SLICES} üçgenlenmiş küre arasında"
            ));
            lines.push(format!(
                "  ortanca {median:.4} · ortalama {mean:.4} · en kötü {worst:.4} (yarıçap {R})"
            ));
            lines.push(format!(
                "  kuramsal sarkma R(1-cos(pi/2N)) = {expected_sag:.4}"
            ));

            for line in &lines {
                gizmo::gizmo_log!(Info, "{}", line);
            }

            // Sahne: analitik ve üçgenlenmiş küre yan yana.
            let sphere = AssetManager::create_sphere(device, R, STACKS, SLICES);
            let smooth = AssetManager::create_sphere(device, R, 48, 64);
            for (x, mesh, colour) in [
                (-1.6, &sphere, Vec4::new(0.90, 0.55, 0.30, 1.0)),
                (1.6, &smooth, Vec4::new(0.35, 0.70, 0.90, 1.0)),
            ] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)),
                    GlobalTransform::default(),
                    mesh.clone(),
                    Material::new(white.clone()).with_pbr(colour, 0.45, 0.0),
                    MeshRenderer::new(),
                ));
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.4, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 24.0),
                Material::new(white).with_pbr(Vec4::new(0.14, 0.15, 0.17, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.6),
                intensity: 2.6,
                ..Default::default()
            });

            scene.world.insert_resource(RayReport { lines });
            scene.spawn_camera(state, Vec3::new(0.0, 1.6, 6.5), Vec3::ZERO);
        })
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<RayReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("ray".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(460.0);
                        ui.heading("Işın atmak");
                        ui.label("screen_to_ray var · MeshRayCast YOK");
                        ui.label("ışın COLLIDER'a çarpıyor, üçgenlere değil");
                        ui.separator();
                        for line in &r.lines {
                            ui.monospace(line);
                        }
                        ui.separator();
                        ui.label("solda kaba (12x16), sağda ince (48x64) küre —");
                        ui.label("ikisinin de collider'ı aynı analitik küre olurdu.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Analitik küre kesişimi — collider'ın gördüğü yüzey.
fn ray_sphere(origin: Vec3, dir: Vec3, centre: Vec3, radius: f32) -> Option<f32> {
    let oc = origin - centre;
    let b = oc.dot(dir);
    let c = oc.length_squared() - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    (t > 0.0).then_some(t)
}

/// Üçgenlenmiş küre kesişimi — gözün gördüğü yüzey.
///
/// Motorun `create_sphere`'inin ürettiği ızgaranın aynısını kurup her üçgene Möller–Trumbore
/// uyguluyor; en yakın isabet döner.
fn ray_sphere_mesh(
    origin: Vec3,
    dir: Vec3,
    centre: Vec3,
    radius: f32,
    stacks: u32,
    slices: u32,
) -> Option<f32> {
    use std::f32::consts::{PI, TAU};
    let at = |i: u32, j: u32| -> Vec3 {
        let phi = i as f32 / stacks as f32 * PI;
        let theta = j as f32 / slices as f32 * TAU;
        centre
            + Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin()) * radius
    };
    let mut best: Option<f32> = None;
    for i in 0..stacks {
        for j in 0..slices {
            let (a, b, c, d) = (at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j));
            for tri in [(a, b, c), (a, c, d)] {
                if let Some(t) = ray_triangle(origin, dir, tri.0, tri.1, tri.2) {
                    best = Some(best.map_or(t, |x: f32| x.min(t)));
                }
            }
        }
    }
    best
}

/// Möller–Trumbore.
fn ray_triangle(origin: Vec3, dir: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
    const EPS: f32 = 1e-7;
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < EPS {
        return None;
    }
    let inv = 1.0 / det;
    let tvec = origin - v0;
    let u = tvec.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = tvec.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > EPS).then_some(t)
}
