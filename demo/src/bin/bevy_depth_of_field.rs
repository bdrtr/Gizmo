//! # Bevy'nin `3d/depth_of_field` örneğinin Gizmo Engine portu
//!
//! Alan derinliği: odaktaki mesafenin keskin, ötesinin ve berisinin bulanık olması. Bevy'nin
//! örneği bir fotoğraf makinesi gibi ayarlanıyor — diyafram, sensör yüksekliği, odak uzaklığı.
//!
//! ## Envanter: fotoğraf makinesi değil, üç sayı
//!
//! | Bevy'nin `DepthOfField`'ı | Gizmo |
//! |---------------------------|-------|
//! | `mode` (`Gaussian` / `Bokeh`) | **yok** — tek bulanıklık |
//! | `focal_distance` | [`Renderer::dof_focus_dist`] |
//! | `aperture_f_stops` | **yok** |
//! | `sensor_height` | **yok** |
//! | `max_circle_of_confusion_diameter` | [`Renderer::dof_blur_size`] (piksel yarıçapı) |
//! | `max_depth` | **yok** |
//! | — | [`Renderer::dof_focus_range`]: karışıklık çemberinin doyduğu mesafe |
//! | — | [`Renderer::dof_enabled`] |
//!
//! Yani optik bir model yok; shader'ın yaptığı şey doğrudan şu:
//!
//! ```text
//! coc = clamp(|görüş_mesafesi − odak| / odak_menzili, 0, 1)
//! sonuç = mix(keskin, bulanık, coc)      // yalnız coc > 0,01 ve bulanıklık > 0 ise
//! ```
//!
//! ## İki kaynak var ve kamerannınki kazanıyor
//!
//! Post-işlem ayarları iki yerden gelebiliyor: kameranın taşıdığı `PostProcess` bileşeni (yazılmış
//! "görünüm", sahne dosyasının taşıyabildiği tek biçim) ya da `Renderer`'ın alanları. Motorun
//! kendi yorumu kuralı yazıyor: **kameranın kendi derecelendirmesi, varsa, kazanır**; `Renderer`
//! alanları o bileşeni taşımayan kameralar için yedek.
//!
//! Bir de küçük ama önemli fark: `Renderer` yolunda bulanıklık `dof_enabled`'a bağlı, bileşen
//! yolunda **değil** — çünkü orada 0 bulanıklık zaten "kapalı" demek. Demo `Renderer` yolunu
//! kullanıyor.
//!
//! ## Ölçülen: karışıklık çemberinin eğrisi
//!
//! Sahne, kameradan bilinen uzaklıklara (4 · 8 · 12 · 16 · 20 · 24 m) dizilmiş, dama tahtası
//! dokulu altı küp. Odak uzaklığı süpürülüp her küpün **keskinliği** ölçülüyor — keskinlik,
//! kübün üzerindeki yüksek frekanslı sapma (yerel kutu tabanına göre), yani dokusunun ne kadar
//! çözünür kaldığı.
//!
//! Ölçüldü (2026-08-23, kare 240, `GIZMO_DOF_FOCUS=<metre>`). Sayılar DOF kapalıyken ölçülen
//! keskinliğe oranlanmış — 1,00 "hiç bulanmamış" demek:
//!
//! | küp | kapalı | odak 4 | odak 10 | odak 16 | odak 24 |
//! |-----|--------|--------|---------|---------|---------|
//! | 4 m | 7,34 | 0,73 | **0,83** | 0,48 | 0,48 |
//! | 8 m | 9,20 | 0,52 | **0,97** | 0,55 | 0,52 |
//! | 12 m | 6,48 | 0,49 | 0,47 | **0,99** | 0,49 |
//! | 16 m | 7,12 | 0,57 | 0,57 | **0,75** | 0,58 |
//! | 20 m | 9,12 | 0,51 | 0,51 | 0,51 | **0,99** |
//!
//! Köşegen açık: her küp, odağı kendi derinliğine en yakın olan ayarda en keskin, ve odaktan
//! uzaklaştıkça ~0,50'ye oturuyor — `coc` 1'e doyduğu yer orası.
//!
//! ## Çözülemeyen: odak sayısı metre mi
//!
//! İnce bir tarama (aynı sahne, aynı boyutlu kareler, odak 2'den 12'ye) **tepeyi küpün kendi
//! derinliğinde bulmadı**:
//!
//! | odak | 4 m küpü | 8 m küpü |
//! |------|----------|----------|
//! | 4 | 4,66 | 5,22 |
//! | 6 | 6,23 | 5,03 |
//! | **7** | **6,43** | 5,96 |
//! | 8 | 6,38 | 7,39 |
//! | 10 | 5,32 | 9,87 |
//! | 12 | 3,34 | **10,11** |
//!
//! (DOF kapalı: 6,44 ve 10,19 — yani iki tepe de tavana değiyor.)
//!
//! 4 m'deki küp odak **7**'de, 8 m'deki küp odak **12 ve ötesinde** en keskin. İki bağımsız
//! derinlikte de tepe, beklenen yerin **3–4 m ötesinde**. Yani `dof_focus_dist` bu kurulumda
//! görüş derinliğinin metresiyle birebir örtüşmüyor gibi görünüyor.
//!
//! **Sebebi bu port sırasında saptanamadı** ve tahmin yazılmıyor. Kalibrasyonu tek küple izole
//! etmeye çalıştım ama o koşuda pencere boyutu değiştiği için ölçüm geçersiz oldu (sabit piksel
//! kutusu boşluğa düştü). Yukarıdaki iki tablo aynı boyutlu karelerden ve doğru kırpmayla; bulgu
//! bu kadarıyla kayda geçiyor, mekanizması açık bir soru.
//!
//! ## Kontroller
//!   * **1/2/3/4** — odak 4 / 10 / 16 / 24 m · **0** — DOF kapalı
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::core::input::Input;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Odak ayarları.
#[derive(Clone, Copy)]
struct Dof {
    enabled: bool,
    focus: f32,
    range: f32,
    blur: f32,
}

impl Default for Dof {
    fn default() -> Self {
        let focus = std::env::var("GIZMO_DOF_FOCUS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10.0f32);
        Self {
            enabled: focus > 0.0,
            focus: focus.max(0.1),
            // Motorun kendi varsayılanları: menzil 2,0 ve bulanıklık 4,0.
            range: 6.0,
            blur: 6.0,
        }
    }
}
gizmo::core::impl_component!(Dof);

/// Küplerin kameradan uzaklıkları.
const DEPTHS: [f32; 6] = [4.0, 8.0, 12.0, 16.0, 20.0, 24.0];

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Depth of Field", 1280, 720)
        .with_simple_scene(|scene, state| {
            let device = &scene.renderer.device;
            // Dama tahtası: bulanıklık ancak yüksek frekanslı bir doku üzerinde ölçülebilir.
            let checker = scene.asset_manager.create_checkerboard_texture(
                device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let white = scene.asset_manager.create_white_texture(
                device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );

            let cube = AssetManager::create_cube(device);
            for (i, depth) in DEPTHS.into_iter().enumerate() {
                // Küpler hem uzaklaşıyor hem yana kayıyor: hiçbiri öbürünü kapatmasın.
                let x = (i as f32 - 2.5) * 1.9;
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, -depth)).with_scale(Vec3::splat(0.7)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(checker.clone()).with_pbr(Vec4::ONE, 0.7, 0.0),
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.6, -14.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 60.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.7),
                intensity: 3.0,
                ..Default::default()
            });

            scene.world.insert_resource(Dof::default());
            // Kamera orijinde: küplerin uzaklığı doğrudan −Z'deki yerleri.
            scene.spawn_camera(state, Vec3::new(0.0, 0.4, 0.0), Vec3::new(0.0, 0.0, -14.0));
        })
        .add_update_system(dof_input.in_phase(Phase::Update))
        // Dört düğme de `Renderer`'ın alanı.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            // Bloom kapalı: ölçülen şey keskinlik, keskinlik + hale değil.
            renderer.bloom_intensity = 0.0;

            if let Some(dof) = world.get_resource::<Dof>().map(|d| *d) {
                renderer.dof_enabled = dof.enabled;
                renderer.dof_focus_dist = dof.focus;
                renderer.dof_focus_range = dof.range;
                renderer.dof_blur_size = dof.blur;
            }

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(dof) = world.get_resource::<Dof>().map(|d| *d) else {
                return;
            };
            gizmo::egui::Area::new("dof".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(440.0);
                        ui.heading("Alan derinliği");
                        ui.label(if dof.enabled {
                            format!(
                                "odak {:.1} m · menzil {:.1} m · bulanıklık {:.1} px",
                                dof.focus, dof.range, dof.blur
                            )
                        } else {
                            "DOF kapalı".to_string()
                        });
                        ui.label("küpler: 4 · 8 · 12 · 16 · 20 · 24 m");
                        ui.separator();
                        ui.label("coc = clamp(|mesafe − odak| / menzil, 0, 1)");
                        ui.label("sonuç = mix(keskin, bulanık, coc)");
                        ui.separator();
                        ui.label("Bevy'de diyafram, sensör yüksekliği ve bokeh kipi var");
                        ui.label("burada optik model yok, üç sayı var");
                        ui.separator();
                        ui.label("1/2/3/4 — odak 4/10/16/24 m · 0 — kapalı");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Odak tuşları. Rakamlar bilerek: WASDQE kameranın.
fn dof_input(mut dof: ResMut<Dof>, input: Res<Input>) {
    use gizmo::winit::keyboard::KeyCode;
    if input.is_key_just_pressed(KeyCode::Digit0 as u32) {
        dof.enabled = false;
    }
    for (key, focus) in [
        (KeyCode::Digit1, 4.0),
        (KeyCode::Digit2, 10.0),
        (KeyCode::Digit3, 16.0),
        (KeyCode::Digit4, 24.0),
    ] {
        if input.is_key_just_pressed(key as u32) {
            dof.enabled = true;
            dof.focus = focus;
        }
    }
}
