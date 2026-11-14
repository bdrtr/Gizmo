//! # Pişmiş aydınlatma
//!
//! Pişmiş aydınlatma: statik ışığın önceden hesaplanıp geometriye işlenmesi, ve gerçek zamanlı
//! ışıkla **karıştırılması**.
//!
//! ## Motorda karşılığı bir malzeme türü: `BakedLit`
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | ışık haritası dokusu | **var (2026-11-14)** — `MaterialMaps::lightmap` (`GIZMO_LIGHTMAP_MODE=lightmap`) |
//! | ayrı ışık haritası UV kanalı | **var (2026-11-14)** — `Vertex::lightmap_uv`, glTF'in `TEXCOORD_1`'i |
//! | ışık haritası bileşeni (doku + UV dikdörtgeni) | **yok** — atlas UV'leri pişirme aracı ikinci kanala yazıyor |
//! | pişmiş + gerçek zamanlı karışımı | **var** — `BakedLit` güneşin gölgesini de alıyor |
//! | pişirme aracı | **var (2026-11-14)** — `gizmo::renderer::lightmap_bake`: gök, ışıyan yüzeyler, sekme, yalnız-pişmiş ışıklar |
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
//! ## Pişirme motorun (2026-11-14)
//!
//! Üç mod da aynı pişirmeyi gösteriyor, ve artık elle yazılmış bir formül değil:
//! `gizmo::renderer::lightmap_bake::LightmapBake` sahnenin kendisinden ışın izliyor. Zemin alıcı
//! (256×256), dört kutu ışığı kesiyor ve sekiyor (`BakeMesh::from_mesh`, çizilen küpün üçgenleri),
//! tavandaki panel **ışıyan bir dörtgen** — yalnız pişirmede var, sahnede yok. Güneş ve mavi nokta
//! ışığı çalışma zamanında da var, o yüzden doğrudan ışıkları pişmiyor (iki kez sayılırdı), yalnız
//! **sekmeleri** pişiyor. Köşe rengi modu aynı haritayı ızgaranın köşelerinde okuyor.
//!
//! ## Kontroller
//!   * `GIZMO_LIGHTMAP_MODE=pbr|baked|lightmap` — malzeme türü
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::renderer::gpu_types::Vertex;
use gizmo::renderer::lightmap_bake;
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

            // Üstünde birkaç kutu: gölgeyi görünür kılıyorlar, ve `BakedLit` gölge ALDIĞI için
            // karışım burada okunuyor. Pişirme de onları görüyor: altlarında ve yanlarında zemin kararıyor.
            let cube = AssetManager::create_cube(device);
            let boxes: Vec<Transform> = (0..4)
                .map(|i| Transform::new(Vec3::new((i as f32 - 1.5) * 2.0, 0.1, 0.5)).with_scale(Vec3::new(0.5, 0.7, 0.5)))
                .collect();
            for at in &boxes {
                scene.world.spawn_bundle((
                    *at,
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(BOX_ALBEDO.extend(1.0), 0.5, 0.0),
                    MeshRenderer::new(),
                ));
            }

            // Gerçek zamanlı güneş — karışımın "gerçek zamanlı" yarısı.
            let sun_rotation = Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.8);
            scene.world.spawn_bundle(DirectionalLightBundle { rotation: sun_rotation, intensity: 2.2, ..Default::default() });
            // Nokta ışığı: `Pbr` bunu görür, `BakedLit` GÖRMEZ. Farkın bir yarısı bu.
            let point = (Vec3::new(0.0, 1.8, 2.4), Vec3::new(0.3, 0.6, 1.0), 8.0, 8.0);
            scene.world.spawn_bundle(PointLightBundle {
                position: point.0,
                color: point.1,
                intensity: point.2,
                radius: point.3,
                casts_shadows: false,
            });

            let boxes: Vec<(Transform, &Mesh)> = boxes.iter().map(|t| (*t, &cube)).collect();
            let map = bake_floor(&floor_at, &boxes, sun_rotation * Vec3::NEG_Z, point);
            if mode == Mode::Lightmap {
                // Tek dörtgen; ışık haritası ikinci UV kanalında. Malzeme bir açıklama olarak
                // veriliyor: haritalarını adlarıyla taşıyan bir `MaterialDesc`, motor dokuları yükleyip
                // bağlıyor. Şiddet, 8 bitlik görüntünün ölçeği — `save_png` döndürüyor.
                let (lightmap, intensity) = write_lightmap(&map);
                scene.world.spawn_bundle((
                    floor_at,
                    GlobalTransform::default(),
                    Mesh::from_vertices(device, &lightmapped_floor_vertices(), "lightmap::quad"),
                    gizmo::renderer::components::MaterialDesc::default()
                        .with_pbr(FLOOR_ALBEDO.extend(1.0), 0.85, 0.0)
                        .with_maps(gizmo::renderer::components::MaterialMaps::default().with_lightmap(lightmap, intensity)),
                    MeshRenderer::new(),
                ));
            } else {
                // Aynı pişirme köşe renklerinde: 40×40 ızgara haritayı köşelerinde okuyor.
                let base = Material::new(white.clone());
                let material = if mode == Mode::Baked {
                    // `BakedLit`: köşe rengi × doku, artı güneşin gölgesi. Nokta ışıkları YOK.
                    base.with_baked_lit(FLOOR_ALBEDO.extend(1.0))
                } else {
                    base.with_pbr(FLOOR_ALBEDO.extend(1.0), 0.85, 0.0)
                };
                scene.world.spawn_bundle((
                    floor_at,
                    GlobalTransform::default(),
                    Mesh::from_vertices(device, &baked_floor_vertices(&map), "lightmap::floor"),
                    material,
                    MeshRenderer::new(),
                ));
            }

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
                        ui.label("pişirme: motorun (lightmap_bake) — gök, tavandaki ışıyan panel,");
                        ui.label("kutuların gölgesi, güneşin ve nokta ışığının sekmesi.");
                        ui.separator();
                        ui.label("çizim: yığın başına 11 yerine 1 (belge).");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Zeminin yarı genişliği, metre.
const HALF: f32 = 7.0;
/// Zeminin rengi.
const FLOOR_ALBEDO: Vec3 = Vec3::new(0.72, 0.70, 0.66);
/// Kutuların rengi.
const BOX_ALBEDO: Vec3 = Vec3::new(0.85, 0.45, 0.30);

/// Sahneyi pişiriyor: zeminin ışık haritası. Tavandaki panel yalnız burada var — pişmiş bir alan
/// ışığı; güneş ve nokta ışığı sahnede de olduğu için yalnız sekmeleri pişiyor.
fn bake_floor(floor_at: &Transform, boxes: &[(Transform, &Mesh)], sun: Vec3, point: (Vec3, Vec3, f32, f32)) -> lightmap_bake::Lightmap {
    use lightmap_bake::{BakeLight, BakeMesh, BakeSettings, LightmapBake};
    let matrix = |t: &Transform| Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position);
    let mut bake = LightmapBake::new()
        .with_sky(gizmo::renderer::frame_uniforms::Sky::uniform([0.10, 0.10, 0.11]))
        .with_settings(BakeSettings::default());
    let floor = bake.add_mesh(
        BakeMesh::new(lightmapped_floor_vertices(), matrix(floor_at)).with_albedo(FLOOR_ALBEDO).receiving(256, 256),
    );
    for (at, mesh) in boxes {
        bake.add_mesh(BakeMesh::from_mesh(mesh, matrix(at)).with_albedo(BOX_ALBEDO));
    }
    // Tavandaki panel: 4,4 × 2,4 m, zeminin 3 m üstünde, aşağı bakıyor, sıcak.
    bake.add_mesh(
        BakeMesh::new(ceiling_panel_vertices(), Mat4::from_translation(floor_at.position + Vec3::Y * 3.0))
            .with_emissive(Vec3::new(1.0, 0.93, 0.76) * 3.0),
    );
    bake.add_light(BakeLight::directional(sun, Vec3::ONE, 2.2));
    let (position, color, intensity, radius) = point;
    bake.add_light(BakeLight::point(position, color, intensity, radius));
    let started = std::time::Instant::now();
    let baked = bake.run().expect("ışık haritası pişirilemedi");
    gizmo::gizmo_log!(Info, "ışık haritası pişti: 256×256, {:.2} s", started.elapsed().as_secs_f32());
    baked.get(floor).expect("zemin alıcı").clone()
}

/// Tavandaki panelin iki üçgeni, aşağı bakan.
fn ceiling_panel_vertices() -> Vec<Vertex> {
    let (hx, hz) = (2.2, 1.2);
    let corner = |x: f32, z: f32| Vertex { position: [x, 0.0, z], normal: [0.0, -1.0, 0.0], ..Default::default() };
    let (a, b, c, d) = (corner(-hx, -hz), corner(-hx, hz), corner(hx, hz), corner(hx, -hz));
    vec![a, c, b, a, d, c]
}

/// Haritayı geçici dizine yazıyor; yolunu ve `with_lightmap`'e verilecek şiddeti döndürüyor.
fn write_lightmap(map: &lightmap_bake::Lightmap) -> (String, f32) {
    let path = std::env::temp_dir().join("gizmo-lightmaps-demo.png");
    let intensity = map.save_png(&path).expect("ışık haritası yazılamadı");
    (path.to_string_lossy().into_owned(), intensity)
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

fn baked_floor_vertices(map: &lightmap_bake::Lightmap) -> Vec<Vertex> {
    const N: usize = 40;
    let mut out = Vec::with_capacity(N * N * 6);
    let at = |i: usize, j: usize| -> Vertex {
        let (u, v) = (i as f32 / N as f32, j as f32 / N as f32);
        let x = -HALF + u * HALF * 2.0;
        let z = -HALF + v * HALF * 2.0;
        // Haritanın bu köşeye düşen texel'i: köşe rengi aynı pişirmeyi ızgaranın sıklığında taşıyor.
        let px = ((u * map.width() as f32) as u32).min(map.width() - 1);
        let py = ((v * map.height() as f32) as u32).min(map.height() - 1);
        let light = map.texel(px, py);
        Vertex {
            position: [x, 0.0, z],
            normal: [0.0, 1.0, 0.0],
            tex_coords: [u, v],
            color: [light.x, light.y, light.z, 1.0],
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
