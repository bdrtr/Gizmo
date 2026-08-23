//! # Bevy'nin `3d/bloom_3d` örneğinin Gizmo Engine portu
//!
//! Parlak şeylerin çevresine taşması. Sahne karanlık; içinde giderek daha güçlü ışıyan küreler
//! var, ve bloom onların ışığını siluetlerinin dışına yayıyor.
//!
//! ## Envanter: yedi düğmeye karşı iki
//!
//! | Bevy'nin `Bloom`'u | Gizmo |
//! |--------------------|-------|
//! | `intensity` | [`Renderer::bloom_intensity`] |
//! | `prefilter.threshold` | [`Renderer::bloom_threshold`] |
//! | `prefilter.threshold_softness` | **yok** |
//! | `low_frequency_boost` | **yok** |
//! | `low_frequency_boost_curvature` | **yok** |
//! | `composite_mode` (`Additive` / `EnergyConserving`) | **yok** |
//! | `max_mip_dimension`, `scale` | **yok** |
//!
//! Yani "ne kadar" ve "hangi parlaklıktan sonra" söylenebiliyor; halenin biçimi, yumuşaklığı ve
//! kareye nasıl karıştığı söylenemiyor. Bevy'nin örneğinin bütün gezinme menüsü burada iki
//! kaydırağa iniyor.
//!
//! İkisi de `Renderer`'ın düz alanı, yani `set_render` kapanışından yazılıyor — sistemlerin
//! erişimi yok.
//!
//! ## Bu portun bulduğu şey: `Material::emissive` ertelenmiş yolda hiçbir şey yapmıyor
//!
//! Demo önce ışıyan küreleri `Unlit` malzemeyle kurdu — küreler kapkara çıktı. Sonra `Pbr`
//! denendi — yine kapkara. Sebebi `gbuffer.wgsl`'de yazılı: ışımayı **malzeme uniform'undan**
//! okuyor (`material.emissive_and_normal_scale.xyz × t_emissive`), yani glTF yüklemesinin
//! doldurduğu tampondan. Rust'tan `Material::emissive`'e yazılan değer örnek tamponuna gidiyor
//! ve ertelenmiş yol oraya hiç bakmıyor.
//!
//! Örnek tamponundaki ışımayı okuyan tek shader **`baked_lit.wgsl`**. Ölçüldü: malzeme türü
//! `BakedLit` yapılınca aynı küreler 172,8 / 245,7 / 253,5 / 242,9 parlaklığa çıkıyor (arka plan
//! 60,5). Yani düğme çalışıyor, ama yalnız tek bir yolda. Bu, `alpha_cutoff`'ta bulunan
//! boşluğun aynısı: ertelenmiş yol malzeme uniform'unu okuyor, oyun kodu ise bileşeni yazıyor.
//!
//! Demo bu yüzden küreleri `BakedLit` kuruyor, ve gerekçesi kodun içinde de yazılı.
//!
//! ## Ölçülen: hale dar bir kenar parıltısı
//!
//! Bloom'un tanımı ışığın **siluetin dışına** taşması. Ölçüt, kürenin merkezinden yarıçap
//! bantlarında ortalama parlaklık (kare 240, `GIZMO_BLOOM=<şiddet>`):
//!
//! | şiddet | gövde (0–60 px) | **kenar (60–80)** | dış (80–120) | uzak (140–200) |
//! |--------|-----------------|-------------------|--------------|----------------|
//! | 0,0 | 245,03 | 110,37 | 60,71 | 60,47 |
//! | 0,4 | +4,03 | **+8,76** | +0,00 | +0,00 |
//! | 0,8 | +5,91 | **+13,65** | +0,00 | +0,00 |
//! | 1,6 | +8,44 | **+18,93** | +0,00 | +0,00 |
//!
//! Şiddet tek düze artıyor ve en çok kenar bandını kaldırıyor. Ama asıl bulgu son iki sütun:
//! **80 pikselden sonra katkı tam sıfır** — üç şiddette de, virgülden sonra iki hane. Motorun
//! bloom'u geniş bir hale değil, dar bir kenar parıltısı, ve **yarıçapını genişletecek bir düğme
//! yok**: Bevy'nin `scale`, `max_mip_dimension` ve `low_frequency_boost`'u tam olarak o işi
//! yapan düğmelerdi.
//!
//! Eşik de çalışıyor ama bu sahnede az: 0,2 → 0,85 → 1,5 için kenar bandı 130,99 → 129,31 →
//! 127,75. Beklenen yönde, çünkü eşik neyin parlayacağına kapı tutuyor ve buradaki kaynaklar
//! her üç eşiğin de çok üstünde.
//!
//! ## Kontroller
//!   * **1/2/3/4** — şiddet 0 / 0,4 / 0,8 / 1,6
//!   * **5/6/7/8** — eşik 0,2 / 0,6 / 0,85 / 1,5
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::core::input::Input;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Motorun iki bloom düğmesi.
#[derive(Clone, Copy)]
struct Bloom {
    intensity: f32,
    threshold: f32,
}

impl Default for Bloom {
    fn default() -> Self {
        Self {
            // Başsız bir karede tuşa basacak kimse yok.
            intensity: env_or("GIZMO_BLOOM", 0.8),
            threshold: env_or("GIZMO_BLOOM_THRESHOLD", 0.85),
        }
    }
}
gizmo::core::impl_component!(Bloom);

/// Bir çevre değişkenini okur, yoksa varsayılanı verir.
fn env_or(name: &str, fallback: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fallback)
}

/// Kürelerin ışıma güçleri — soldan sağa artıyor.
const EMISSIVES: [f32; 4] = [0.6, 1.6, 4.0, 10.0];
/// Küreler arasındaki aralık.
const SPACING: f32 = 3.4;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Bloom 3D", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let sphere = AssetManager::create_sphere(device, 0.8, 22, 36);

            for (i, strength) in EMISSIVES.into_iter().enumerate() {
                let x = (i as f32 - 1.5) * SPACING;
                // Işıyan malzeme: albedo karanlık, parlaklık `emissive`'den geliyor.
                //
                // **`Unlit` değil `Pbr`** — ilk yazımda `with_unlit` kullanmıştım ve küreler
                // kapkara çıktı: ışıksız yol albedo'yu *olduğu gibi* çiziyor ve `emissive`'i hiç
                // okumuyor. Işıma ertelenmiş yolun işi.
                let mut material = Material::new(white.clone())
                    .with_pbr(Vec4::new(0.02, 0.02, 0.03, 1.0), 0.5, 0.0)
                    .with_emissive(Vec3::new(
                        strength * 0.35,
                        strength * 0.55,
                        strength * 1.0,
                    ));
                material.material_type = gizmo::renderer::components::MaterialType::BakedLit;
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)),
                    GlobalTransform::default(),
                    sphere.clone(),
                    material,
                    MeshRenderer::new(),
                ));
            }

            // Arka plan bilerek çok karanlık: hale ancak karanlıkta ölçülür.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, -6.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 60.0),
                Material::new(white).with_pbr(Vec4::new(0.03, 0.03, 0.035, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));

            scene.world.insert_resource(Bloom::default());
            scene.spawn_camera(state, Vec3::new(0.0, 0.0, 11.0), Vec3::ZERO);
        })
        .add_update_system(bloom_input.in_phase(Phase::Update))
        // İki düğme de `Renderer`'ın alanı.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            if let Some(bloom) = world.get_resource::<Bloom>().map(|b| *b) {
                renderer.bloom_intensity = bloom.intensity;
                renderer.bloom_threshold = bloom.threshold;
            }

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(bloom) = world.get_resource::<Bloom>().map(|b| *b) else {
                return;
            };
            gizmo::egui::Area::new("bloom".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(430.0);
                        ui.heading("Bloom");
                        ui.label(format!(
                            "şiddet {:.2} · eşik {:.2}",
                            bloom.intensity, bloom.threshold
                        ));
                        ui.label("küreler soldan sağa: ışıma 0,6 · 1,6 · 4,0 · 10,0");
                        ui.separator();
                        ui.label("motorun bloom düğmeleri bu ikisi, hepsi bu.");
                        ui.label("Bevy'de ayrıca: eşik yumuşaklığı, düşük frekans desteği");
                        ui.label("ve eğriliği, karıştırma kipi, mip sınırı, ölçek.");
                        ui.separator();
                        ui.label("1/2/3/4 — şiddet · 5/6/7/8 — eşik");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Şiddet ve eşik tuşları. Rakamlar bilerek: WASDQE kameranın.
fn bloom_input(mut bloom: ResMut<Bloom>, input: Res<Input>) {
    use gizmo::winit::keyboard::KeyCode;
    for (key, intensity) in [
        (KeyCode::Digit1, 0.0),
        (KeyCode::Digit2, 0.4),
        (KeyCode::Digit3, 0.8),
        (KeyCode::Digit4, 1.6),
    ] {
        if input.is_key_just_pressed(key as u32) {
            bloom.intensity = intensity;
        }
    }
    for (key, threshold) in [
        (KeyCode::Digit5, 0.2),
        (KeyCode::Digit6, 0.6),
        (KeyCode::Digit7, 0.85),
        (KeyCode::Digit8, 1.5),
    ] {
        if input.is_key_just_pressed(key as u32) {
            bloom.threshold = threshold;
        }
    }
}
