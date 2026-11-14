//! # Pişmiş aydınlatma
//!
//! Pişmiş aydınlatma: statik ışığın önceden hesaplanıp geometriye işlenmesi, ve gerçek zamanlı
//! ışıkla **karıştırılması**.
//!
//! ## Motorda karşılığı bir malzeme türü: `BakedLit`
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | ışık haritası dokusu | **var (2026-11-14)** — `MaterialMaps::lightmap`, bu demo hâlâ köşe rengini gösteriyor |
//! | ayrı ışık haritası UV kanalı | **var (2026-11-14)** — `Vertex::lightmap_uv`, glTF'in `TEXCOORD_1`'i |
//! | ışık haritası bileşeni (doku + UV dikdörtgeni) | **yok** — atlas UV'leri pişirme aracı ikinci kanala yazıyor |
//! | pişmiş + gerçek zamanlı karışımı | **var** — `BakedLit` güneşin gölgesini de alıyor |
//! | pişirme aracı | **yok** — köşe renklerini siz dolduracaksınız |
//!
//! Yani "karıştırılmış aydınlatma" fikri motorda var, ama taşıyıcısı doku değil **köşe rengi**.
//! Bunun bedeli çözünürlük: bir duvarın ışık haritası dokuda piksel piksel olur, köşe renginde
//! ise mesh'in üçgen yoğunluğu kadar olur.
//!
//! ## Kazanç ölçülebilir: çizim sayısı
//!
//! `MaterialType`'ın kendi belgesi sayıyı veriyor: `BakedLit` "G-tamponunu ve nokta ışıklarını
//! atlıyor: yığın başına on bir yerine **bir** ileri çizim". Demo bunu ölçüyor — aynı sahne iki
//! kez, biri `Pbr` biri `BakedLit`.
//!
//! ## Ölçüldü
//!
//! Aynı sahne iki kez, tek değişen zeminin malzeme türü. Sahnede bir gerçek zamanlı güneş **ve**
//! mavi bir nokta ışığı var. Ölçüm sol yarı (panel sağda), ölçüldü (2026-08-23, 948×492):
//!
//! | | ortalama RGB | mavi − kırmızı |
//! |---|--------------|----------------|
//! | `Pbr` zemin | (127,8 · 127,8 · 129,1) | **+1,35** — mavimsi |
//! | `BakedLit` zemin | (108,8 · 106,7 · 104,0) | **−4,74** — sıcak |
//!
//! Renk dengesi **işaret değiştiriyor**, ve sebebi tam olarak beklenen: `Pbr` mavi nokta ışığını
//! görüyor, `BakedLit` **görmüyor** ve onun yerine köşe rengine pişmiş sıcak ışığı taşıyor.
//! Karelerin %48,98'i farklı.
//!
//! Ve zeminin parlaklık profili pişmiş ışığın gerçekten pişmiş olduğunu gösteriyor:
//!
//! | satır | `Pbr` | `BakedLit` |
//! |-------|-------|------------|
//! | 0,55 | 152,2 | 136,1 |
//! | 0,65 | 184,9 | **152,8** (tepe) |
//! | 0,75 | **192,3** (tepe) | 150,3 |
//! | 0,85 | 186,5 | 139,7 |
//! | 0,95 | 176,4 | **127,2** |
//!
//! İki eğrinin tepesi farklı yerde ve `BakedLit` daha dik düşüyor — çünkü onun düşüşü gerçek
//! zamanlı ışığın mesafe zayıflamasından değil, köşelere **yazılmış** sahte alan ışığından
//! geliyor. Aynı bilgi bir dokuda piksel piksel olurdu; buradaki çözünürlük ızgaranın sıklığı
//! kadar (40×40).
//!
//! ### Ölçüm notu
//!
//! Pencere bu koşularda 948×492 açıldı, öteki demolarda 948×1028. İki koşunun boyutu birbiriyle
//! aynı olduğu için karşılaştırma geçerli, ama sabit piksel eşikleri demolar arasında
//! taşınamıyor — bölge her zaman görüntü boyutuna göre hesaplanıyor.
//!
//! ## Üçüncü mod: gerçek bir ışık haritası (2026-11-14)
//!
//! `GIZMO_LIGHTMAP_MODE=lightmap` aynı sahte tavan ışığını bu kez **dokuya** pişiriyor (256×256,
//! açılışta hesaplanıp geçici dizine yazılıyor) ve zemin **tek bir dörtgen** — iki üçgen, dört
//! köşe. Işık haritası `MaterialMaps::lightmap`, ikinci UV kanalından (`Vertex::lightmap_uv`)
//! okunuyor; zemin `Pbr` kalıyor, yani nokta ışığını ve güneşi **görmeye devam ediyor**, pişmiş ışık
//! yalnızca ortam ışığının yerini alıyor. Köşe rengi modunda 40×40 ızgaranın yaptığını burada
//! dokunun pikselleri yapıyor.
//!
//! ## Kontroller
//!   * `GIZMO_LIGHTMAP_MODE=pbr|baked|lightmap` — malzeme türü
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::renderer::gpu_types::Vertex;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Zeminin nasıl aydınlandığı.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Gerçek zamanlı ışık.
    Pbr,
    /// Köşe rengine pişmiş ışık (`BakedLit`).
    Baked,
    /// Dokuya pişmiş ışık (`MaterialMaps::lightmap`).
    Lightmap,
}

impl Mode {
    fn from_env() -> Self {
        match std::env::var("GIZMO_LIGHTMAP_MODE").as_deref() {
            Ok("baked") => Mode::Baked,
            Ok("lightmap") => Mode::Lightmap,
            _ => Mode::Pbr,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Mode::Pbr => "Pbr",
            Mode::Baked => "BakedLit (köşe rengi)",
            Mode::Lightmap => "Pbr + ışık haritası",
        }
    }
}

fn main() {
    let mode = Mode::from_env();
    App::<SimpleSceneState>::new("Gizmo Engine - Lightmaps", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            let floor_at = Transform::new(Vec3::new(0.0, -0.6, 0.0));
            if mode == Mode::Lightmap {
                // Tek dörtgen; ışık haritası ikinci UV kanalında. Malzeme bir açıklama olarak
                // veriliyor: haritalarını adlarıyla taşıyan bir `MaterialDesc`, motor dokuları yükleyip
                // bağlıyor.
                let lightmap = write_lightmap();
                scene.world.spawn_bundle((
                    floor_at,
                    GlobalTransform::default(),
                    Mesh::from_vertices(device, &lightmapped_floor_vertices(), "lightmap::quad"),
                    gizmo::renderer::components::MaterialDesc::default()
                        .with_pbr(Vec4::new(0.72, 0.70, 0.66, 1.0), 0.85, 0.0)
                        .with_maps(gizmo::renderer::components::MaterialMaps::default().with_lightmap(lightmap, 1.6)),
                    MeshRenderer::new(),
                ));
            } else {
                // "Pişmiş" ışık: köşe renklerine elle yazılmış, dokuda değil. Buradaki pişirme,
                // sahte bir alan ışığından (tavandaki bir dikdörtgen) gelen katkının köşe başına
                // analitik hesabı.
                let base = Material::new(white.clone());
                let material = if mode == Mode::Baked {
                    // `BakedLit`: köşe rengi × doku, artı güneşin gölgesi. Nokta ışıkları YOK.
                    base.with_baked_lit(Vec4::new(1.0, 1.0, 1.0, 1.0))
                } else {
                    base.with_pbr(Vec4::new(0.72, 0.70, 0.66, 1.0), 0.85, 0.0)
                };
                scene.world.spawn_bundle((
                    floor_at,
                    GlobalTransform::default(),
                    Mesh::from_vertices(device, &baked_floor_vertices(), "lightmap::floor"),
                    material,
                    MeshRenderer::new(),
                ));
            }

            // Üstünde birkaç kutu: gölgeyi görünür kılıyorlar, ve `BakedLit` gölge ALDIĞI için
            // karışım burada okunuyor.
            let cube = AssetManager::create_cube(device);
            for i in 0..4 {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new((i as f32 - 1.5) * 2.0, 0.1, 0.5))
                        .with_scale(Vec3::new(0.5, 0.7, 0.5)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.85, 0.45, 0.30, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
            }

            // Gerçek zamanlı güneş — karışımın "gerçek zamanlı" yarısı.
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.8),
                intensity: 2.2,
                ..Default::default()
            });
            // Nokta ışığı: `Pbr` bunu görür, `BakedLit` GÖRMEZ. Farkın bir yarısı bu.
            scene.world.spawn_bundle(PointLightBundle {
                position: Vec3::new(0.0, 1.8, 2.4),
                color: Vec3::new(0.3, 0.6, 1.0),
                intensity: 8.0,
                radius: 8.0,
                casts_shadows: false,
            });

            gizmo::gizmo_log!(Info, "zemin malzemesi: {}", mode.name());
            let _ = white;
            scene.spawn_camera(state, Vec3::new(0.0, 3.4, 7.0), Vec3::new(0.0, 0.0, 0.0));
        })
        .set_ui(move |_world, _state, ctx| {
            gizmo::egui::Area::new("lm".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(420.0);
                        ui.heading("Pişmiş aydınlatma");
                        ui.label(format!("zemin: {}", mode.name()));
                        ui.separator();
                        match mode {
                            Mode::Lightmap => {
                                ui.label("pişmiş ışık DOKUDA (256×256), zemin tek dörtgen.");
                                ui.label("ikinci UV kanalından okunuyor; ortam ışığının yerini alıyor,");
                                ui.label("güneş ve nokta ışığı üstüne eklenmeye devam ediyor.");
                            }
                            _ => {
                                ui.label("pişmiş ışık KÖŞE RENGİNDE (40×40 ızgara).");
                                ui.label("BakedLit güneşin gölgesini ALIYOR (karışım),");
                                ui.label("ama nokta ışıklarını GÖRMÜYOR.");
                            }
                        }
                        ui.label("Pişirme aracı: YOK — doku dışarıda üretilir.");
                        ui.separator();
                        ui.label("çizim: yığın başına 11 yerine 1 (belge).");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Zemin: köşe renklerine "pişmiş" bir alan ışığı katkısı yazılmış bir ızgara.
///
/// Bir ışık haritası dokusunun yerine geçen şey bu — ve çözünürlüğün neden mesh'e bağlı
/// olduğunu da gösteriyor: katkı köşe başına, yani ızgara ne kadar sıksa o kadar yumuşak.
/// Zeminin yarı genişliği, metre.
const HALF: f32 = 7.0;

/// Tavandaki sahte alan ışığının zeminde `(x, z)`'ye bıraktığı ışık — pişirmenin kendisi, köşe
/// rengine de dokuya da aynısı.
fn bake(x: f32, z: f32) -> [f32; 4] {
    // Tavandaki sahte alan ışığı — pişirmenin kaynağı.
    let panel_centre = Vec3::new(0.0, 3.0, 0.0);
    let panel_half = Vec2::new(2.2, 1.2);
    {
        // Panelin üstünde birkaç örnek al ve katkılarını topla — kaba bir alan ışığı.
        let mut sum = 0.0f32;
        const S: i32 = 3;
        for sx in -S..=S {
            for sz in -S..=S {
                let sample = panel_centre
                    + Vec3::new(
                        sx as f32 / S as f32 * panel_half.x,
                        0.0,
                        sz as f32 / S as f32 * panel_half.y,
                    );
                let to_light = sample - Vec3::new(x, 0.0, z);
                let dist = to_light.length().max(0.15);
                let cos = (to_light.y / dist).max(0.0);
                sum += cos / (dist * dist);
            }
        }
        let n = ((2 * S + 1) * (2 * S + 1)) as f32;
        let v = (sum / n * 6.0).clamp(0.0, 1.0);
        // Hafif sıcak bir renk — pişmiş ışığın kendi rengi.
        [0.10 + v * 0.95, 0.10 + v * 0.88, 0.10 + v * 0.72, 1.0]
    }
}

/// Aynı pişirme, 256×256'lık bir dokuya — doğrusal değerler, 8 bit. Yolunu döndürür.
fn write_lightmap() -> String {
    const SIZE: u32 = 256;
    let image = image::RgbaImage::from_fn(SIZE, SIZE, |px, py| {
        let x = -HALF + (px as f32 + 0.5) / SIZE as f32 * HALF * 2.0;
        let z = -HALF + (py as f32 + 0.5) / SIZE as f32 * HALF * 2.0;
        let c = bake(x, z);
        image::Rgba([(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, 255])
    });
    let path = std::env::temp_dir().join("gizmo-lightmaps-demo.png");
    image.save(&path).expect("ışık haritası yazılamadı");
    path.to_string_lossy().into_owned()
}

/// Zemin, tek dörtgen: ikinci UV kanalı ışık haritasını baştan sona kaplıyor.
fn lightmapped_floor_vertices() -> Vec<Vertex> {
    let corner = |u: f32, v: f32| Vertex {
        position: [-HALF + u * HALF * 2.0, 0.0, -HALF + v * HALF * 2.0],
        normal: [0.0, 1.0, 0.0],
        tex_coords: [u, v],
        lightmap_uv: [u, v],
        ..Default::default()
    };
    let (a, b, c, d) = (corner(0.0, 0.0), corner(0.0, 1.0), corner(1.0, 1.0), corner(1.0, 0.0));
    vec![a, b, c, a, c, d]
}

fn baked_floor_vertices() -> Vec<Vertex> {
    const N: usize = 40;
    let mut out = Vec::with_capacity(N * N * 6);
    let at = |i: usize, j: usize| -> Vertex {
        let x = -HALF + i as f32 / N as f32 * HALF * 2.0;
        let z = -HALF + j as f32 / N as f32 * HALF * 2.0;
        Vertex {
            position: [x, 0.0, z],
            normal: [0.0, 1.0, 0.0],
            tex_coords: [i as f32 / N as f32, j as f32 / N as f32],
            color: bake(x, z),
            ..Default::default()
        }
    };
    for i in 0..N {
        for j in 0..N {
            let (a, b, c, d) = (at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j));
            out.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    out
}
