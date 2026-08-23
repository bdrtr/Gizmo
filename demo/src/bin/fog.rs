//! # Mesafe sisi
//!
//! Uzaklıkla kapanan görüş: uzaktaki nesnelerin sis rengine karışması. Tam biçimiyle bu, kameraya
//! takılan bir sis ayarı demek — rengi, düşüş eğrisi (lineer / üstel / üstel-kare / atmosferik) ve
//! mesafeleri çalışma anında yazılabilen.
//!
//! ## Motorda sis var, ama **ayarları shader'ın içinde**
//!
//! Gizmo'nun ertelenmiş aydınlatma shader'ında `compute_height_fog` diye bir fonksiyon var ve
//! gerçek bir yükseklik sisi hesaplıyor (analitik saçılma integrali, sis düzleminin altına
//! indikçe yoğunlaşan). Ama rengi, yoğunluğu ve yükseklik düşüşü **sabit** — `deferred_lighting.wgsl`
//! içinde, `environment_preset`'e göre seçilen dört gömülü takım:
//!
//! | ayar | renk (lineer) | yoğunluk | yükseklik düşüşü |
//! |------|---------------|----------|------------------|
//! | 0 — Gün Batımı | `(0,85, 0,38, 0,15)` | 0,025 | 0,08 |
//! | 1 — Nötr Stüdyo | `(0,20, 0,22, 0,25)` | 0,008 | 0,04 |
//! | 2 — Gece Neon | `(0,12, 0,02, 0,25)` | 0,035 | 0,12 |
//! | diğer — Gündüz | `(0,50, 0,60, 0,70)` | 0,015 | 0,05 |
//!
//! Yani sisin tek tek yazılabilir olması gereken her ayarı burada **seçilebilir, ayarlanamaz**.
//! Rust tarafında ulaşılabilen üç sayı var, üçü de `Renderer`'ın alanı:
//! `environment_preset`, `environment_preset_2` ve `environment_blend_t` — yani "şu ayardan bu
//! ayara %40 geçilmiş" diyebiliyorsunuz, ama "yoğunluk 0,02 olsun" diyemiyorsunuz. Demo bunun
//! ikisini de gösteriyor: ayarları geziyor **ve** ikisi arasında sürekli karıştırıyor.
//!
//! Sis düzleminin taban yüksekliği (`fog_base_height = −5`) ve düşüş eğrisinin biçimi hiç
//! seçilemiyor; düşüş kipini seçmenin bir yolu **yok**.
//!
//! Ve dikkat: `environment_preset` bir **sis** ayarı değil, bir **ortam** ayarı — aynı sayı
//! sahnenin gökyüzünü ve genel aydınlatma havasını da seçiyor. Yani sisi değiştirmek sahnenin
//! bütün havasını değiştirmek demek; sis ayarı ötekilerden **bağımsız değil**.
//!
//! ## Ölçüldü (2026-08-23, kare 150, `GIZMO_FOG_PRESET=<n>`)
//!
//! Ayarın shader'a gerçekten ulaştığının kanıtı — aynı sahne, aynı malzeme, üç ayar; uzaktaki
//! piramidin piksel rengi:
//!
//! | ayar | ufuk rengi | uzak piramit |
//! |------|------------|--------------|
//! | 0 — Gün Batımı | `(226, 191, 128)` | `(222, 197, 169)` |
//! | 2 — Gece Neon | `(112, 26, 164)` | `(167, 146, 178)` |
//! | 3 — Gündüz | `(205, 214, 220)` | `(228, 227, 227)` |
//!
//! ## Boş arka plan sisin dışında kalıyor
//!
//! Motorda **temizleme rengi ayarı yok** ve sis yalnız geometriyi boyuyor, yani gökyüzü siyah
//! kalıp sahneyi ufkun üstünde bıçak gibi kesiyor. Buradaki çözüm ters çevrilmiş dev bir küp:
//! ışıksız bir malzemeyle, rengi her kare seçili ayardan alınıyor. (Motorun asıl cevabı
//! `MaterialType::Skybox` ama o bir küp haritası ister, depoda yok.)
//!
//! ## Motorun ikinci sisi
//!
//! Bir de `UnderwaterFog` var: son-işlem geçişinde üstel bir sis, rengi ve yoğunluğu **Rust'tan
//! yazılabiliyor**. Ama o kameranın bir sıvı hacminin içinde olmasına bağlı bir efekt, mesafe
//! sisinin yerine geçmiyor — bu yüzden bu demonun konusu değil.
//!
//! ## Sahne
//!
//! Uzaklığa doğru dizilmiş **beş piramit** var, çünkü sisin ölçüsü mesafeyle değişimi ve onu tek
//! bir nesne göstermiyor. En yakını 8 m, en uzağı 72 m'de.
//!
//! ## Kontroller
//!   * **1/2/3/4** — birinci ayar · **5/6/7/8** — ikinci ayar
//!   * **−/=** — karışım oranını azalt/artır · **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Hangi ayarların seçili olduğu ve aralarındaki karışım.
#[derive(Clone, Copy)]
struct FogState {
    preset_a: u32,
    preset_b: u32,
    blend: f32,
}

impl Default for FogState {
    fn default() -> Self {
        Self {
            // Başsız bir karede tuşa basacak kimse yok: `GIZMO_FOG_PRESET=<0..3>` başlangıç
            // ayarını seçer, ki ayar değişiminin gerçekten shader'a ulaştığı gösterilebilsin.
            preset_a: std::env::var("GIZMO_FOG_PRESET")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3), // Gündüz — shader'ın "diğer" dalı
            preset_b: 2,       // Gece Neon
            blend: 0.0,
        }
    }
}
gizmo::core::impl_component!(FogState);

/// Ayarların insan tarafındaki adları ve shader'daki gömülü sayıları.
const PRESETS: [(&str, [f32; 3], f32, f32); 4] = [
    ("Gün Batımı", [0.85, 0.38, 0.15], 0.025, 0.08),
    ("Nötr Stüdyo", [0.20, 0.22, 0.25], 0.008, 0.04),
    ("Gece Neon", [0.12, 0.02, 0.25], 0.035, 0.12),
    ("Gündüz", [0.50, 0.60, 0.70], 0.015, 0.05),
];

/// Piramitlerin uzaklıkları — sisin mesafeyle değişimi ancak böyle görünüyor.
const DEPTHS: [f32; 5] = [-8.0, -24.0, -40.0, -56.0, -72.0];

/// Gökyüzü kubbesinin işareti.
#[derive(Clone, Copy)]
struct SkyDome;
gizmo::core::impl_component!(SkyDome);

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Fog", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let white_sky = white.clone();
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            for (i, z) in DEPTHS.into_iter().enumerate() {
                // Kademeli piramit: derinliği okunaklı kılan bir yapı.
                let x = if i % 2 == 0 { -3.0 } else { 3.0 };
                build_pyramid(scene.world, &cube, &white, Vec3::new(x, 0.0, z));
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.0, -40.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 200.0),
                Material::new(white).with_pbr(Vec4::new(0.16, 0.17, 0.19, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.6),
                intensity: 2.6,
                ..Default::default()
            });

            // Gökyüzü kubbesi. Motorda **temizleme rengi ayarı yok** ve sis yalnız geometriyi
            // boyuyor, yani boş arka plan siyah kalıyor — sisli bir sahne ufkun üstünde bıçak
            // gibi kesiliyor. Motorun cevabı ya bir skybox (küp haritası ister, depoda yok) ya
            // da buradaki gibi ters çevrilmiş dev bir küp. Rengi her kare seçili ayardan
            // alınıyor.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, -40.0)).with_scale(Vec3::splat(150.0)),
                GlobalTransform::default(),
                AssetManager::create_inverted_cube(device),
                Material::new(white_sky).with_unlit(Vec4::new(0.5, 0.6, 0.7, 1.0)),
                MeshRenderer::new(),
                SkyDome,
            ));

            scene.world.insert_resource(FogState::default());
            scene.spawn_camera(state, Vec3::new(0.0, 3.0, 6.0), Vec3::new(0.0, 1.0, -40.0));
        })
        .add_update_system(fog_input.in_phase(Phase::Update).label("fog_input"))
        .add_update_system(tint_sky.in_phase(Phase::Update).after("fog_input"))
        // Sisin üç sayısı `Renderer`'ın alanı; sistemler oraya erişemiyor.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            if let Some(fog) = world.get_resource::<FogState>().map(|f| *f) {
                renderer.environment_preset = fog.preset_a;
                renderer.environment_preset_2 = fog.preset_b;
                renderer.environment_blend_t = fog.blend;
            }

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(fog) = world.get_resource::<FogState>().map(|f| *f) else {
                return;
            };
            let a = PRESETS[fog.preset_a.min(3) as usize];
            let b = PRESETS[fog.preset_b.min(3) as usize];
            gizmo::egui::Area::new("fog".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    // Bu demonun arka planı açık: çerçevesiz bir panel okunmuyor.
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(460.0);
                    ui.heading("Mesafe sisi");
                    ui.label(format!(
                        "1. ayar: {} — renk ({:.2}, {:.2}, {:.2}) yoğunluk {:.3} düşüş {:.2}",
                        a.0, a.1[0], a.1[1], a.1[2], a.2, a.3
                    ));
                    ui.label(format!(
                        "2. ayar: {} — renk ({:.2}, {:.2}, {:.2}) yoğunluk {:.3} düşüş {:.2}",
                        b.0, b.1[0], b.1[1], b.1[2], b.2, b.3
                    ));
                    ui.label(format!("karışım: {:.2}", fog.blend));
                    ui.separator();
                    ui.label("bu sayılar deferred_lighting.wgsl'de GÖMÜLÜ.");
                    ui.label("Rust'tan yazılabilen tek şey hangi ayar ve karışım oranı.");
                    ui.label("renk, yoğunluk ve düşüş kipi ayarlanamıyor.");
                    ui.separator();
                    ui.label("1/2/3/4 — 1. ayar · 5/6/7/8 — 2. ayar · -/= — karışım");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Ayar ve karışım tuşları.
///
/// Tuşlar bilerek rakam ve `-`/`=`: **WASDQE kameranın**, ve motorun `movement_input` kapısı
/// hareket tuşlarının elle okunmasına zaten izin vermiyor. İlk yazımda 2. ayar Q/W/E/R'deydi ve
/// hem kapı kırıldı hem de A'ya basmak aynı anda yana kaydırıyordu.
fn fog_input(mut fog: ResMut<FogState>, input: Res<Input>, time: Res<Time>) {
    use gizmo::winit::keyboard::KeyCode;
    for (key, preset) in [
        (KeyCode::Digit1, 0),
        (KeyCode::Digit2, 1),
        (KeyCode::Digit3, 2),
        (KeyCode::Digit4, 3),
    ] {
        if input.is_key_just_pressed(key as u32) {
            fog.preset_a = preset;
        }
    }
    for (key, preset) in [
        (KeyCode::Digit5, 0),
        (KeyCode::Digit6, 1),
        (KeyCode::Digit7, 2),
        (KeyCode::Digit8, 3),
    ] {
        if input.is_key_just_pressed(key as u32) {
            fog.preset_b = preset;
        }
    }
    // Sisin sürekli sayıları (yoğunluk, mesafe, renk kanalları) tuşla değiştirilemiyor; burada
    // değiştirilebilen tek sürekli sayı karışım oranı.
    let step = time.dt() * 0.8;
    if input.is_key_pressed(KeyCode::Minus as u32) {
        fog.blend = (fog.blend - step).max(0.0);
    }
    if input.is_key_pressed(KeyCode::Equal as u32) {
        fog.blend = (fog.blend + step).min(1.0);
    }
}

/// Gökyüzünü seçili sis rengine boyar — ikisi arasında karışım varsa ona da uyar.
fn tint_sky(mut skies: Query<(Mut<Material>, With<SkyDome>)>, fog: Res<FogState>) {
    let a = PRESETS[fog.preset_a.min(3) as usize].1;
    let b = PRESETS[fog.preset_b.min(3) as usize].1;
    let t = fog.blend;
    let mix = |x: f32, y: f32| x + (y - x) * t;
    let color = Vec4::new(
        mix(a[0], b[0]),
        mix(a[1], b[1]),
        mix(a[2], b[2]),
        1.0,
    );
    for (_entity, (mut material, _)) in skies.iter_mut() {
        material.albedo = color;
    }
}

/// Beş katlı kademeli piramit — derinliği okunaklı kılan yapı.
fn build_pyramid(
    world: &mut gizmo::core::World,
    cube: &Mesh,
    texture: &std::sync::Arc<gizmo::wgpu::BindGroup>,
    base: Vec3,
) {
    for level in 0..5 {
        let half = (4 - level) as f32 * 0.5;
        let y = base.y + level as f32 * 1.0;
        world.spawn_bundle((
            Transform::new(Vec3::new(base.x, y, base.z))
                .with_scale(Vec3::new(half.max(0.5), 0.5, half.max(0.5))),
            GlobalTransform::default(),
            cube.clone(),
            Material::new(texture.clone()).with_pbr(Vec4::new(0.75, 0.72, 0.68, 1.0), 0.7, 0.0),
            MeshRenderer::new(),
        ));
    }
}
