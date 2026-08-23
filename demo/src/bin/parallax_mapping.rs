//! # Yüzey ayrıntısı: dokuyla mı, üçgenle mi
//!
//! Bir duvarın tuğlalı görünmesi için tuğlaları modellemek gerekmez — normal haritası yüzeyin
//! eğimini boyar, paralaks haritalaması bir de derinlik yanılsaması ekler. İkisi de üçgen
//! harcamadan ayrıntı verir.
//!
//! ## Motorda paralaks yok — ama asıl duvar bir önceki adımda
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | yükseklik/derinlik dokusu bağlama | **yok** |
//! | paralaks adım sayısı / ölçek alanı | **yok** |
//! | teğet uzayda bakış vektörü | **yok** (parça aşamasına taşınmıyor) |
//! | ışın yürütme döngüsü | **yok** (hiçbir WGSL'de) |
//! | **oyunun normal haritası bağlaması** | **yok** — aşağıda |
//!
//! Paralaks için gereken teğet çerçevesi **hazır**: köşe teğeti var, `gbuffer.wgsl` TBN kuruyor,
//! ve G-tamponunun bir hedefi dünya teğeti taşıyor. Eksik olan üstyapı.
//!
//! Ama bir oyunun paralaksa sıra gelmeden çarptığı duvar daha erken: **normal haritası bile
//! bağlayamıyor.**
//!
//! ## Doku yuvaları var, kapısı yok
//!
//! Malzeme bağlama grubunun **yedi** girdisi var, ve dördü ayrıntı dokusu: taban renk, örnekleyici,
//! **normal**, metalik-pürüzlülük, ışıyan, **örtüşme**, artı parametre tamponu. Yani yuvalar
//! duruyor ve varsayılanlarla dolduruluyor (düz normal, beyaz MR/ışıyan/AO).
//!
//! Onları dolduran işlev `AssetManager::assemble_material_bind_group` — ve **`pub(crate)`**.
//! `gizmo-renderer` dışından çağrılamıyor. Bir oyunun elindeki genel yüzey şu kadar:
//!
//! | genel API | ne veriyor |
//! |-----------|------------|
//! | `create_white_texture` | hepsi varsayılan |
//! | `load_material_texture(yol)` | **yalnız taban renk**, diskten |
//! | `create_checkerboard_texture` | yerleşik desen |
//! | `create_uv_debug_texture` | yerleşik desen |
//!
//! Normal, MR, ışıyan ve AO haritalarına ulaşan tek yol **glTF yükleyicisi**. Yani elle kurulmuş
//! bir sahnede yüzey ayrıntısının tek kaynağı **geometri**.
//!
//! ## Ölçüldü — ayrıntının üçgenle bedeli
//!
//! İki levha, aynı malzeme, aynı ışık. Solda bir oyunun kurabileceği en iyi yüzey (düz, tek doku
//! yuvası), sağda aynı görünüm **geometriyle** — köşeler yer değiştirmiş, normaller analitik.
//! Ölçüldü (2026-08-23, 1904×1028, levha bölgeleri taranarak bulundu):
//!
//! | levha | köşe | ortalama | **gölgeleme std** | aralık |
//! |-------|------|----------|-------------------|--------|
//! | düz (2 üçgen) | **6** | 187,84 | **22,45** | 59..193 |
//! | kabartmalı (96×96) | **55 296** | 124,60 | **64,49** | 40..239 |
//!
//! Standart sapma, yüzeyin ne kadar "kabartmalı" göründüğünün ölçüsü: eğim değiştikçe gölgeleme
//! değişir. Sağdaki **2,87 kat** daha değişken, ve tonal aralığı da geniş (40..239'a karşı
//! 59..193).
//!
//! Bedeli **9 216 kat geometri**: 6 köşeden 55 296'ya. Normal haritası aynı değişkenliği bir
//! dokudan, sıfır ek üçgenle verirdi — ama bağlanamıyor.
//!
//! ### Ölçüm notu
//!
//! İlk koşuda kare neredeyse boştu. Sebep sarım yönüydü: ürettiğim üçgenler +Z'den bakınca saat
//! yönündeydi, opak boru hattı ise `FrontFace::Ccw` + arka yüz ayıklaması kullanıyor, yani iki
//! levha da tamamen yutuldu. Sarım çevrildi.
//!
//! Bir de karşılaştırmanın dürüstlüğü: ilk kurulumda düz levhayı da 96×96 kurmuştum, yani ikisi
//! de 55 296 köşeydi ve "bedel" görünmüyordu. Düz bir yüzeyin gerçek maliyeti iki üçgendir;
//! tessellasyon düz bir yüzeyin gölgelemesini değiştirmez, o yüzden 6 köşe hem doğru hem adil.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::prelude::*;
use gizmo::renderer::gpu_types::Vertex;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Ölçüm defteri.
#[derive(Clone, Copy, Default)]
struct Report {
    flat_verts: u32,
    bumpy_verts: u32,
}
gizmo::core::impl_component!(Report);

/// Kabartma ızgarasının sıklığı — üçgen bedelinin geldiği yer.
const N: usize = 96;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Parallax Mapping", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // SOL: bir oyunun kurabileceği en iyi malzeme — düz bir yüzey, tek doku yuvası.
            // Normal haritası bağlanamadığı için yüzey pürüzsüz kalıyor.
            // Düz bir levhanın gerçek maliyeti: iki üçgen. Kabartmayı üçgenle almanın bedeli
            // tam olarak bu ikisinin farkı.
            let flat = Mesh::from_vertices(device, &grid(0.0, 1), "parallax::flat");
            // SAĞ: aynı görünüm ama GEOMETRİYLE — köşeler yer değiştirmiş, normaller elde
            // hesaplanmış. Tek yol bu.
            let bumpy = Mesh::from_vertices(device, &grid(0.16, N), "parallax::bumpy");

            let report = Report {
                flat_verts: flat.vertex_count,
                bumpy_verts: bumpy.vertex_count,
            };
            gizmo::gizmo_log!(
                Info,
                "düz {} köşe · kabartmalı {} köşe · ızgara {}x{}",
                report.flat_verts,
                report.bumpy_verts,
                N,
                N
            );

            for (x, mesh) in [(-2.4f32, &flat), (2.4, &bumpy)] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0))
                        .with_rotation(Quat::from_rotation_x(-0.35)),
                    GlobalTransform::default(),
                    mesh.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.72, 0.60, 0.48, 1.0),
                        0.55,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                // Yandan gelen ışık: kabartmayı ortaya çıkaran şey eğim farkı.
                rotation: Quat::from_rotation_y(1.1) * Quat::from_rotation_x(-0.35),
                intensity: 3.2,
                ..Default::default()
            });
            let _ = white;
            scene.world.insert_resource(report);
            scene.spawn_camera(state, Vec3::new(0.0, 1.0, 6.4), Vec3::new(0.0, 0.0, 0.0));
        })
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<Report>().map(|r| *r) else {
                return;
            };
            gizmo::egui::Area::new("px".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(430.0);
                        ui.heading("Yüzey ayrıntısı");
                        ui.monospace(format!("sol  düz       : {} köşe", r.flat_verts));
                        ui.monospace(format!("sağ  geometrik : {} köşe", r.bumpy_verts));
                        ui.separator();
                        ui.label("bağlama grubunda 7 girdi, dördü ayrıntı dokusu.");
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 160, 80),
                            "ama assemble_material_bind_group pub(crate)",
                        );
                        ui.label("oyunun elindeki: yalnız taban renk dokusu.");
                        ui.label("normal/MR/ışıyan/AO'ya tek yol glTF yükleyicisi.");
                        ui.separator();
                        ui.label("paralaks için teğet çerçevesi hazır (TBN + teğet hedefi),");
                        ui.label("eksik olan yükseklik dokusu, adım sayısı ve döngü.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// `N`×`N` ızgara. `amp = 0` düz bir levha; `amp > 0` köşeleri kaldırıp normalleri analitik
/// olarak hesaplıyor — normal haritasının yapacağı işin geometriyle yapılmış hâli.
fn grid(amp: f32, n: usize) -> Vec<Vertex> {
    const HALF: f32 = 1.8;
    // Yükseklik alanı: iki eksenli sinüs, yani düzenli bir kabartma.
    let height = |u: f32, v: f32| -> f32 {
        if amp == 0.0 {
            0.0
        } else {
            amp * ((u * 18.0).sin() * (v * 18.0).sin())
        }
    };
    // Normal, yükseklik alanının kısmi türevlerinden.
    let normal = |u: f32, v: f32| -> Vec3 {
        if amp == 0.0 {
            return Vec3::Z;
        }
        let d = 0.01;
        let dhdu = (height(u + d, v) - height(u - d, v)) / (2.0 * d);
        let dhdv = (height(u, v + d) - height(u, v - d)) / (2.0 * d);
        Vec3::new(-dhdu, -dhdv, 1.0).normalize()
    };
    let at = |i: usize, j: usize| -> Vertex {
        let u = i as f32 / n as f32;
        let v = j as f32 / n as f32;
        let x = -HALF + u * HALF * 2.0;
        let y = -HALF + v * HALF * 2.0;
        let n = normal(u, v);
        Vertex {
            position: [x, y, height(u, v)],
            normal: [n.x, n.y, n.z],
            tex_coords: [u, v],
            ..Default::default()
        }
    };
    let mut out = Vec::with_capacity(n * n * 6);
    for i in 0..n {
        for j in 0..n {
            let (a, b, c, d) = (at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j));
            // Sarım CCW: +Z'den bakınca saat yönünün tersi. Ters sarımda opak boru hattı
            // (`FrontFace::Ccw` + arka yüz ayıklaması) levhayı tamamen yutuyor — ilk koşuda
            // kare neredeyse boştu.
            out.extend_from_slice(&[a, c, b, a, d, c]);
        }
    }
    out
}
