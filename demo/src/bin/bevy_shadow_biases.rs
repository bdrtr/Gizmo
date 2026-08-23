//! # Bevy'nin `3d/shadow_biases` örneğinin Gizmo Engine portu
//!
//! Gölge haritalarının iki klasik hatası: yüzeyin kendi kendini gölgelemesi (**akne** — düz bir
//! yüzeyde beliren çizgili lekeler) ve gölgenin nesnenin dibinden kopması (**peter-panning**).
//! Bevy'nin örneği ışığın `shadow_depth_bias` ve `shadow_normal_bias` düğmelerini çalışma anında
//! oynatıp ikisi arasındaki ödünleşmeyi gösteriyor.
//!
//! ## Motorda o düğmeler yok — sapma türetiliyor
//!
//! Gizmo'da ne `shadow_depth_bias` var ne `shadow_normal_bias`. Bunun yerine
//! `deferred_lighting.wgsl` normal-offset'i **cascade'in dünya texel boyutundan** hesaplıyor:
//! `world_texel = 2 · uv_texel / sx`, ve örnekleme noktası yüzeyden `N · world_texel · 2` kadar
//! ayrılıyor. Shader'ın kendi yorumu bunun neden böyle olduğunu da yazıyor: sabit bir
//! `N · 0,0018` yalnız en yakın cascade'e uyuyormuş, uzak cascade'lerde texel çok daha büyük
//! olduğu için offset texel'in dörtte biri kadar kalıp yüzeyi temizleyemiyormuş — sonuç çapraz
//! akne.
//!
//! Ve asıl akne çözümü sapma bile değil: **gölge geçişinde ön-yüz eleme** (arka yüzler haritaya
//! yazılıyor), yani aydınlık ön yüz kendi derinliğiyle hiç karşılaştırılmıyor. Kalan normal-offset
//! yalnız silüet ve temas için.
//!
//! ## Ayarlanamayan öteki her şey
//!
//! Gölge sisteminin bütün parametreleri `csm.rs`'te **sabit**:
//!
//! | | değer | Bevy |
//! |---|-------|------|
//! | cascade sayısı | 4 | `CascadeShadowConfig` ile ayarlanır |
//! | gölge haritası çözünürlüğü | 3072² | ayarlanır |
//! | gölge menzili | 100 m | ayarlanır |
//! | bölme λ (log↔düzgün) | 0,75 | ayarlanır |
//! | derinlik/normal sapması | **yok** — türetiliyor | iki ayrı düğme |
//!
//! Demo bunları HUD'da gösteriyor, ve cascade sınırlarını motorun kendi
//! [`cascade_split_distances`] fonksiyonundan okuyor — uydurmuyor.
//!
//! ## Ölçülen: akne bir sayıdır
//!
//! Bevy'nin örneği "sapmayı kısınca akne çıkar" diyor. Burada sapma kısılamadığına göre asıl
//! soru şu: **motorun türettiği sapma yeterli mi?** Sahne bunu sınamak için kuruluyor — geniş
//! düz bir zemin, üstünde sıra sıra küreler, ve ışık **sıyırma açısında** (akne tam olarak orada
//! çıkar).
//!
//! Ölçüt, zeminin **hiçbir kürenin gölgesine denk gelmeyen** şeridinde **yüksek frekanslı
//! sapma**: her piksel kendi yerel ortalamasından (9 piksellik kutu bulanıklığı) ne kadar
//! ayrılıyor. Yerel taban şart — şeridin kendisinde uzaklığa bağlı doğal bir parlaklık geçişi
//! var, ve global bir eşikle bakınca o geçiş "%20 koyu piksel" diye okunuyor. Akne ise yüksek
//! frekanslıdır: yerel tabandan sapar.
//!
//! Ölçüldü (2026-08-23, kare 240, `GIZMO_SUN_ELEV=<derece>`):
//!
//! | güneş yüksekliği | ort \|sapma\| | \|sapma\|>4 | maks |
//! |------------------|--------------|------------|------|
//! | 45° | 0,254 | %0,0000 | 2 |
//! | 20° | 0,252 | %0,0000 | 2 |
//! | 10° | 0,192 | %0,0000 | 2 |
//! | 5° | 0,161 | %0,0000 | 2 |
//!
//! **Hiçbir yükseklikte akne yok** — 5°'lik sıyırma açısında bile. Maksimum yerel sapma tam 2,
//! yani nicemleme gürültüsü.
//!
//! ### Ölçüt kör mü? Kontrol grubu
//!
//! "Her yerde sıfır" bir ölçütün bozuk olduğunun da işareti olabilir, o yüzden aynı ölçüt
//! kürelerin ve gölge kenarlarının bulunduğu şeride uygulandı: **ortalama sapma 3,119,
//! piksellerin %20,6'sı 4'ü aşıyor, maksimum 64**. Yani ölçüt yüksek frekansı gayet iyi
//! görüyor; açık zemindeki sıfır gerçek bir sıfır.
//!
//! Bu bir sahne için ölçülmüş bir sonuç, evrensel bir kanıt değil: 3072²'lik gölge haritası,
//! bu sahne ölçeği ve ertelenmiş oyun yolu için geçerli.
//!
//! ## Kontroller
//!   * **1/2/3/4** — güneş yüksekliği 45° / 20° / 10° / 5°
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Güneşin yüksekliği (derece). Alçaldıkça akne baskısı artar.
#[derive(Clone, Copy)]
struct Sun {
    elevation_deg: f32,
}

impl Default for Sun {
    fn default() -> Self {
        Self {
            elevation_deg: std::env::var("GIZMO_SUN_ELEV")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20.0f32)
                .clamp(2.0, 89.0),
        }
    }
}
gizmo::core::impl_component!(Sun);

/// Işığın işareti — her kare yeniden yönlendiriliyor.
#[derive(Clone, Copy)]
struct SunLight;
gizmo::core::impl_component!(SunLight);

/// Kürelerin sayısı ve aralığı.
const SPHERES: i32 = 7;
const SPACING: f32 = 4.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Shadow Biases", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Geniş düz zemin: akne varsa burada görünür.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, -20.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 120.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.78, 0.78, 0.80, 1.0), 0.95, 0.0),
                MeshRenderer::new(),
            ));

            // Küreler tek sıra ve arkaya doğru: gölgeleri arkalarına düşsün, önlerindeki şerit
            // açık kalsın. Ölçüm o şeridi okuyor.
            for i in -SPHERES / 2..=SPHERES / 2 {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(i as f32 * SPACING, 1.0, -12.0)),
                    GlobalTransform::default(),
                    AssetManager::create_sphere(device, 1.0, 20, 32),
                    Material::new(white.clone())
                        .with_pbr(Vec4::new(0.85, 0.82, 0.78, 1.0), 0.6, 0.0),
                    MeshRenderer::new(),
                ));
            }

            let sun = Sun::default();
            // Yönlü ışığın YÖNÜ `DirectionalLight`'ta değil varlığın `Transform.rotation`'ında
            // duruyor — bileşenin alanları yalnız renk, şiddet ve rol.
            let light = scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: sun_rotation(sun.elevation_deg),
                intensity: 3.0,
                ..Default::default()
            });
            scene.world.add_component(light, SunLight);

            scene.world.insert_resource(sun);
            scene.spawn_camera(state, Vec3::new(0.0, 5.0, 10.0), Vec3::new(0.0, 0.5, -14.0));
        })
        .add_update_system(sun_input.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(sun) = world.get_resource::<Sun>().map(|s| *s) else {
                return;
            };
            // Cascade sınırları motorun kendi fonksiyonundan — demo uydurmuyor.
            let splits = gizmo::renderer::csm::cascade_split_distances(
                0.1,
                gizmo::renderer::csm::SHADOW_DISTANCE,
                gizmo::renderer::csm::CASCADE_LAMBDA,
            );
            gizmo::egui::Area::new("bias".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(440.0);
                        ui.heading("Gölge sapması");
                        ui.label(format!("güneş yüksekliği: {:.0}°", sun.elevation_deg));
                        ui.separator();
                        ui.label("motorda shadow_depth_bias / shadow_normal_bias YOK");
                        ui.label("normal-offset cascade'in dünya texel boyutundan türetiliyor");
                        ui.label("asıl akne çözümü: gölge geçişinde ön-yüz eleme");
                        ui.separator();
                        ui.label(format!(
                            "cascade {} · harita {}² · menzil {:.0} m · λ {:.2}",
                            gizmo::renderer::csm::CASCADE_COUNT,
                            gizmo::renderer::csm::SHADOW_MAP_RES,
                            gizmo::renderer::csm::SHADOW_DISTANCE,
                            gizmo::renderer::csm::CASCADE_LAMBDA
                        ));
                        ui.label(format!(
                            "bölmeler: {:.1} · {:.1} · {:.1} · {:.1} m",
                            splits[0], splits[1], splits[2], splits[3]
                        ));
                        ui.label("hepsi csm.rs'te SABİT — Bevy'de hepsi ayarlanır");
                        ui.separator();
                        ui.label("1/2/3/4 — 45° / 20° / 10° / 5°");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Verilen yükseklik açısı için güneşin dönüşü.
///
/// Yükseklik küçüldükçe ışık yüzeye sıyırarak vurur — gölge haritasında bir texel'e düşen dünya
/// mesafesi büyür, ve akne tam olarak orada çıkar.
fn sun_rotation(elevation_deg: f32) -> Quat {
    Quat::from_rotation_y(0.35) * Quat::from_rotation_x(-elevation_deg.to_radians())
}

/// Yükseklik tuşları ve ışığın yeniden yönlendirilmesi.
fn sun_input(
    mut lights: Query<(Mut<Transform>, With<SunLight>)>,
    mut sun: ResMut<Sun>,
    input: Res<Input>,
) {
    use gizmo::winit::keyboard::KeyCode;
    let mut changed = false;
    for (key, elevation) in [
        (KeyCode::Digit1, 45.0),
        (KeyCode::Digit2, 20.0),
        (KeyCode::Digit3, 10.0),
        (KeyCode::Digit4, 5.0),
    ] {
        if input.is_key_just_pressed(key as u32) {
            sun.elevation_deg = elevation;
            changed = true;
        }
    }
    if !changed {
        return;
    }
    let rotation = sun_rotation(sun.elevation_deg);
    for (_entity, (mut transform, _)) in lights.iter_mut() {
        transform.rotation = rotation;
    }
}
