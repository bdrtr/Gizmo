//! # Yüzey ayrıntısı: dokuyla mı, üçgenle mi
//!
//! Bir duvarın tuğlalı görünmesi için tuğlaları modellemek gerekmez — normal haritası yüzeyin
//! eğimini boyar, paralaks haritalaması bir de derinlik yanılsaması ekler. İkisi de üçgen
//! harcamadan ayrıntı verir.
//!
//! ## Motorda paralaks yok — ama asıl duvar 2026-08-23'te yıkıldı
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | **oyunun normal haritası bağlaması** | **var** (2026-08-23) — `AssetManager::material()` |
//! | yükseklik/derinlik dokusu bağlama | **yok** |
//! | paralaks adım sayısı / ölçek alanı | **yok** |
//! | teğet uzayda bakış vektörü | **yok** (parça aşamasına taşınmıyor) |
//! | ışın yürütme döngüsü | **yok** (hiçbir WGSL'de) |
//!
//! Paralaks için gereken teğet çerçevesi **hazır**: köşe teğeti var, `gbuffer.wgsl` TBN kuruyor,
//! ve G-tamponunun bir hedefi dünya teğeti taşıyor. Eksik olan üstyapı.
//!
//! Ama bir oyunun paralaksa sıra gelmeden çarptığı duvar daha erkendi: **normal haritası bile
//! bağlayamıyordu.**
//!
//! ## Doku yuvaları vardı, kapısı yoktu
//!
//! Malzeme bağlama grubunun **yedi** girdisi var, ve dördü ayrıntı dokusu: taban renk, örnekleyici,
//! **normal**, metalik-pürüzlülük, ışıyan, **örtüşme**, artı parametre tamponu. Yuvalar duruyordu
//! ve varsayılanlarla dolduruluyordu (düz normal, beyaz MR/ışıyan/AO).
//!
//! Onları dolduran işlev `AssetManager::assemble_material_bind_group` — ve **`pub(crate)`**'ti.
//! `gizmo-renderer` dışından çağrılamıyordu. Bir oyunun elindeki genel yüzey şu kadardı:
//!
//! | genel API | ne veriyordu |
//! |-----------|--------------|
//! | `create_white_texture` | hepsi varsayılan |
//! | `load_material_texture(yol)` | **yalnız taban renk**, diskten |
//! | `create_checkerboard_texture` | yerleşik desen |
//! | `create_uv_debug_texture` | yerleşik desen |
//!
//! Normal, MR, ışıyan ve AO haritalarına ulaşan tek yol glTF yükleyicisiydi. Yani elle kurulmuş
//! bir sahnede yüzey ayrıntısının tek kaynağı **geometriydi**.
//!
//! ## Kapı: `AssetManager::material()`
//!
//! Kurucu yedi yuvanın hepsini açıyor, ve boş bırakılan her yuva nötr varsayılanını alıyor —
//! taban renk ve örnekleyici dahil. Yani bu demonun ihtiyacı olan tek satır:
//!
//! ```ignore
//! AssetManager::material().normal(&view).build(&mut assets, device, queue, layout)
//! ```
//!
//! Otomatik olması gereken otomatik: altı yuvaya dokunulmuyor. Detaya inmek isteyenin de sınırı
//! yok: `base_colour`, `sampler`, `metallic_roughness`, `emissive`, `occlusion`, `params`,
//! `label` — yedisi de adlandırılabiliyor, `params` dahil, yani gölgelendiricinin malzeme
//! sabitlerini okuduğu tampon bile kullanıcının olabiliyor.
//!
//! Dokunun kendisi ham `wgpu` ile kuruluyor (`make_normal_map`): motorun "piksellerden doku" diye
//! genel bir kapısı yok, ama `wgpu` tipleri zaten dışarıya açık olduğu için bu bir duvar değil bir
//! basamak. Bir tuzağı var ve demo ona düşmemek için açıkça yazıyor: normal haritası renk değil
//! **veri**, yani `Rgba8Unorm` — sRGB olsaydı motor onu doğrusallaştırır ve bütün eğimler yanlış
//! çıkardı.
//!
//! ## Ölçüldü — ayrıntının üçgenle bedeli, ve dokuyla bedeli
//!
//! Üç levha, aynı yükseklik alanı, aynı ışık, aynı malzeme sabitleri. Solda düz bir yüzey; ortada
//! aynı kabartma **geometriyle** (köşeler yer değiştirmiş, normaller analitik); sağda aynı kabartma
//! **normal haritasıyla**, geometri düz levhanın ta kendisi. Harita da aynı yükseklik alanının
//! türevinden üretiliyor, yani üçü tam olarak aynı yüzeyi anlatıyor.
//!
//! Ölçüldü (2026-08-23, `GIZMO_PX_SLAB=<0|1|2>` ile her levha tek başına, 948×1028):
//!
//! | levha | köşe | ortalama | **gölgeleme std** | aralık |
//! |-------|------|----------|-------------------|--------|
//! | düz | **6** | 191,91 | **3,03** | 108..193 |
//! | geometrik | **55 296** | 178,79 | **39,27** | 71..229 |
//! | normal haritalı | **6** | 173,99 | **38,98** | 68..229 |
//!
//! Standart sapma, yüzeyin ne kadar "kabartmalı" göründüğünün ölçüsü: eğim değiştikçe gölgeleme
//! değişir. Düz levhada 3,03 — neredeyse hiç. Geometrik levhada **12,9 kat** fazla.
//!
//! Ve normal haritalı levha o değişkenliğin **%99,3'ünü 6 köşeyle** veriyor. Tonal aralığı da
//! neredeyse aynı (68..229'a karşı 71..229). Yani **9 216 kat** geometri farkı, ve görünürde
//! %0,7'lik bir fark.
//!
//! ### Ölçüm notu: üç şey ayrı ayrı yanlıştı
//!
//! **Sarım.** İlk koşuda kare neredeyse boştu. Ürettiğim üçgenler +Z'den bakınca saat yönündeydi,
//! opak boru hattı ise `FrontFace::Ccw` + arka yüz ayıklaması kullanıyor, yani iki levha da
//! tamamen yutuldu. Sarım çevrildi.
//!
//! **Adalet.** İlk kurulumda düz levhayı da 96×96 kurmuştum, yani ikisi de 55 296 köşeydi ve
//! "bedel" görünmüyordu. Düz bir yüzeyin gerçek maliyeti iki üçgendir; tessellasyon düz bir
//! yüzeyin gölgelemesini değiştirmez, o yüzden 6 köşe hem doğru hem adil.
//!
//! **Bölge.** Üç levhayı yan yana koyup piksel taramasıyla ayırmayı denedim; bölgeler bitişik
//! bulundu ve arka plan ölçüme karıştı — o kurulumda **düz** levha en yüksek standart sapmayı
//! veriyordu (47,92), ki bu sonucun tersi. Pencere boyutu da koşudan koşuya değişiyor (948×1028
//! ve 948×492 arasında), yani sabit oranlı bir kutu da güvenilmezdi. `GIZMO_PX_SLAB` her levhayı
//! tek başına ortaya koyuyor ve maske levhayı renginden ayırıyor (bej: `R − B > 12`), böylece
//! arka plan ve arayüz ölçümün dışında kalıyor.
//!
//! O maskenin de bir kenarı var ve saklamıyorum: kabartmanın en koyu vadileri eşiğin altına
//! düşüyor, o yüzden kabartmalı levhalarda daha az piksel sayılıyor (62 839 ve 64 296'ya karşı
//! düzde 107 802). İki kabartmalı levha aynı şekilde etkilendiği için aralarındaki oran geçerli;
//! düz levhayla karşılaştırma ise bu yüzden bir **alt sınır**.
//!
//! ## Kontroller
//!   * `GIZMO_PX_SLAB=<0|1|2>` — tek levhayı ortada yalnız bırak (ölçüm için gerekli)
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::prelude::*;
use gizmo::renderer::gpu_types::Vertex;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Ölçüm defteri.
#[derive(Clone, Copy, Default)]
struct Report {
    flat_verts: u32,
    bumpy_verts: u32,
    mapped_verts: u32,
}
gizmo::core::impl_component!(Report);

/// Normal haritasının çözünürlüğü. Kabartma ızgarasıyla aynı yükseklik alanından üretiliyor, yani
/// üç levha da tam olarak aynı yüzeyi anlatıyor — biri üçgenle, biri dokuyla, biri hiç.
const MAP: u32 = 512;

/// Yükseklik alanı. `grid` ile paylaşılıyor: karşılaştırmanın adil olması buna bağlı.
fn height_at(u: f32, v: f32, amp: f32) -> f32 {
    amp * ((u * 18.0).sin() * (v * 18.0).sin())
}

/// Aynı yükseklik alanından teğet-uzay normal haritası üretir ve GPU'ya yükler.
///
/// Doku ham `wgpu` ile kuruluyor — motorun hazır bir "piksellerden doku" kapısı yok, ama `wgpu`
/// tipleri zaten dışarıya açık, yani bu bir duvar değil bir basamak. Sonra
/// [`AssetManager::material`] onu bağlama grubuna takıyor.
fn make_normal_map(device: &wgpu::Device, queue: &wgpu::Queue, amp: f32) -> wgpu::TextureView {
    let mut pixels = vec![0u8; (MAP * MAP * 4) as usize];
    let d = 1.0 / MAP as f32;
    for y in 0..MAP {
        for x in 0..MAP {
            let (u, v) = (x as f32 / MAP as f32, y as f32 / MAP as f32);
            // `grid`'in analitik normaliyle aynı türev, aynı işaretler.
            let dhdu = (height_at(u + d, v, amp) - height_at(u - d, v, amp)) / (2.0 * d);
            let dhdv = (height_at(u, v + d, amp) - height_at(u, v - d, amp)) / (2.0 * d);
            let n = Vec3::new(-dhdu, -dhdv, 1.0).normalize();
            let base = ((y * MAP + x) * 4) as usize;
            // Teğet-uzay kodlaması: [-1,1] -> [0,255].
            pixels[base] = ((n.x * 0.5 + 0.5) * 255.0) as u8;
            pixels[base + 1] = ((n.y * 0.5 + 0.5) * 255.0) as u8;
            pixels[base + 2] = ((n.z * 0.5 + 0.5) * 255.0) as u8;
            pixels[base + 3] = 255;
        }
    }

    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("parallax::normal_map"),
        size: wgpu::Extent3d {
            width: MAP,
            height: MAP,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Normal haritası renk değil VERİ: sRGB olsaydı motor onu doğrusallaştırır ve eğimler
        // yanlış çıkardı.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(MAP * 4),
            rows_per_image: Some(MAP),
        },
        wgpu::Extent3d {
            width: MAP,
            height: MAP,
            depth_or_array_layers: 1,
        },
    );
    tex.create_view(&wgpu::TextureViewDescriptor::default())
}

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
            // ORTA: aynı görünüm GEOMETRİYLE — köşeler yer değiştirmiş, normaller elde
            // hesaplanmış. 2026-08-23 öncesinde tek yol buydu.
            let bumpy = Mesh::from_vertices(device, &grid(0.16, N), "parallax::bumpy");
            // SAĞ: aynı görünüm NORMAL HARİTASIYLA — geometri düz levhanın ta kendisi.
            let mapped = Mesh::from_vertices(device, &grid(0.0, 1), "parallax::mapped");

            let report = Report {
                flat_verts: flat.vertex_count,
                bumpy_verts: bumpy.vertex_count,
                mapped_verts: mapped.vertex_count,
            };
            gizmo::gizmo_log!(
                Info,
                "düz {} köşe · geometrik {} köşe · haritalı {} köşe · ızgara {}x{}",
                report.flat_verts,
                report.bumpy_verts,
                report.mapped_verts,
                N,
                N
            );

            // Normal haritalı malzeme: doku ham `wgpu` ile, bağlama grubu `AssetManager::material`
            // ile. Taban rengi ve örnekleyici de motorun paylaşılan varsayılanlarından — yani
            // yalnız DOLDURULMASI GEREKEN yuva dolduruluyor, ötekiler kendiliğinden.
            let normal_view = make_normal_map(&scene.renderer.device, &scene.renderer.queue, 0.16);
            let mapped_bg = AssetManager::material()
                .normal(&normal_view)
                .label("parallax::mapped")
                .build(
                    scene.asset_manager,
                    &scene.renderer.device,
                    &scene.renderer.queue,
                    &scene.renderer.scene.texture_bind_group_layout,
                );

            // `GIZMO_PX_SLAB=<0|1|2>` tek levhayı ortada yalnız bırakıyor. Ölçüm bunu kullanıyor:
            // üçünü yan yana koyup piksel taramasıyla ayırmak, pencere boyutu koşudan koşuya
            // değiştiği için güvenilmezdi — bölgeler bitişik bulunuyordu.
            let only: Option<usize> = std::env::var("GIZMO_PX_SLAB")
                .ok()
                .and_then(|v| v.parse::<usize>().ok());
            let device = &scene.renderer.device;
            let slabs = [
                (-3.9f32, &flat, white.clone()),
                (0.0, &bumpy, white.clone()),
                (3.9, &mapped, mapped_bg),
            ];
            for (i, (x, mesh, bg)) in slabs.into_iter().enumerate() {
                let x = match only {
                    Some(k) if k == i => 0.0,
                    Some(_) => continue,
                    None => x,
                };
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0))
                        .with_rotation(Quat::from_rotation_x(-0.35)),
                    GlobalTransform::default(),
                    mesh.clone(),
                    Material::new(bg).with_pbr(Vec4::new(0.72, 0.60, 0.48, 1.0), 0.55, 0.0),
                    MeshRenderer::new(),
                ));
            }
            let _ = device;

            scene.world.spawn_bundle(DirectionalLightBundle {
                // Yandan gelen ışık: kabartmayı ortaya çıkaran şey eğim farkı.
                rotation: Quat::from_rotation_y(1.1) * Quat::from_rotation_x(-0.35),
                intensity: 3.2,
                ..Default::default()
            });
            let _ = white;
            scene.world.insert_resource(report);
            scene.spawn_camera(state, Vec3::new(0.0, 1.2, 9.6), Vec3::new(0.0, 0.0, 0.0));
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
                        ui.monospace(format!("sol   düz       : {} köşe", r.flat_verts));
                        ui.monospace(format!("orta  geometrik : {} köşe", r.bumpy_verts));
                        ui.monospace(format!("sağ   haritalı  : {} köşe", r.mapped_verts));
                        ui.separator();
                        ui.label("bağlama grubunda 7 girdi, dördü ayrıntı dokusu.");
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(120, 200, 130),
                            "AssetManager::material() ile hepsi bağlanabiliyor",
                        );
                        ui.monospace("  material().normal(&view).build(..)");
                        ui.label("boş bırakılan yuva nötr varsayılanı alıyor.");
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
