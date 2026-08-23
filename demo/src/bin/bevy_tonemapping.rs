//! # Bevy'nin `3d/tonemapping` örneğinin Gizmo Engine portu
//!
//! Ton eşleme: sınırsız (HDR) ışık değerlerini ekranın taşıyabileceği 0–1 aralığına indiren eğri.
//! Bevy'nin örneği yedi ayrı eğriyi yan yana geziyor ve aralarındaki karakter farkını gösteriyor.
//!
//! ## Envanter: yedi eğriye karşı bir
//!
//! | Bevy'nin `Tonemapping`'i | Gizmo |
//! |--------------------------|-------|
//! | `None` | **yok** |
//! | `Reinhard` | **yok** |
//! | `ReinhardLuminance` | **yok** |
//! | `AcesFitted` | tek seçenek, `post_process.wgsl`'de gömülü |
//! | `AgX` | **yok** |
//! | `SomewhatBoringDisplayTransform` | **yok** |
//! | `TonyMcMapface` | **yok** |
//! | `BlenderFilmic` | **yok** |
//! | deband dither açık/kapalı | **yok** |
//!
//! Motorun eğrisi seçilemiyor; `aces_tonemap` fonksiyonu shader'ın içinde ve tek çağrı yeri var.
//! Ayarlanabilen tek şey **pozlama**: [`Renderer::exposure`], eğriden *önce* uygulanan doğrusal
//! bir çarpan.
//!
//! ## Ölçülen: eğrinin kendisi
//!
//! Bir ton eğrisini anlatmanın yolu onu çizmektir. Demo bunu yapıyor: sahnede on tane **ışıksız**
//! yama var, her birinin albedo'su bilinen bir doğrusal değer (0,03'ten 8,0'e). Işıksız yol
//! albedo'yu olduğu gibi çizdiği için ekrana düşen sayı yalnız pozlama ve ACES'ten geçmiş oluyor —
//! yani yamaların okunan parlaklığı motorun **gerçek transfer eğrisidir**, belgeden değil
//! ölçümden.
//!
//! Ölçüldü (2026-08-23, kare 240, bloom kapalı, `GIZMO_EXPOSURE=<çarpan>`). Yamaların ekrandaki
//! parlaklığı (0–255):
//!
//! | doğrusal | poz 0,3 | poz 0,6 | poz 1,15 | poz 2,5 |
//! |----------|---------|---------|----------|---------|
//! | 0,03 | 10,0 | 22,6 | 41,7 | 78,5 |
//! | 0,06 | 23,3 | 45,0 | 75,9 | 127,1 |
//! | 0,12 | 45,0 | 79,0 | 122,0 | 176,9 |
//! | 0,25 | 81,0 | 128,0 | 174,0 | 215,7 |
//! | 0,50 | 128,0 | 176,9 | 212,3 | 237,0 |
//! | 1,00 | 176,9 | 214,2 | 235,2 | 247,8 |
//! | 1,50 | 200,5 | 228,6 | 243,0 | 251,6 |
//! | 2,50 | 222,7 | 240,7 | 249,4 | 254,4 |
//! | 4,00 | 234,7 | 246,0 | 251,3 | 253,4 |
//! | 8,00 | 240,1 | 245,5 | 247,4 | 247,4 |
//!
//! Tablo iki şeyi doğruluyor. **Pozlama gerçekten eğriden önce çarpıyor:** aynı çarpım aynı
//! sonucu veriyor — 0,50 × 0,3 ile 0,25 × 0,6 ikisi de tam 128,0. **Ve tepe geri gelmiyor:**
//! doğrusal 2,5'in üstünde çıktı 247–254 bandına oturuyor, 8,0'de hafifçe geri bile düşüyor.
//! ACES beyaza hiç ulaşmıyor; yanmış bir vurgu pozlama kısılarak kurtarılamaz, çünkü kurtarılacak
//! ayrım ekranda kalmamıştır.
//!
//! ## Bu portun bulduğu şey: `Renderer::exposure` ölü bir alan
//!
//! İlk ölçümde dört pozlama da **birebir aynı** sayıları verdi. Sebebi şuydu: demo
//! `Renderer::exposure`'a yazıyordu, ama oyun çizim yolu pozlamayı **`Camera::exposure`**'dan
//! okuyor (`systems/render/mod.rs`, `cam_exposure = cam.exposure`). `Renderer`'ın `exposure`
//! alanını motorda **hiçbir yer okumuyor**.
//!
//! Ve tek yazanı da bu depodaydı: `bevy_anisotropy`'nin "Exposure" kaydırağı o ölü alana
//! bağlıydı, yani hiçbir şey yapmıyordu. Bu port onu da düzeltti — kaydırak artık demonun kendi
//! alanına yazıyor ve `camera_orbit` her kare kameraya taşıyor.
//!
//! ## Kontroller
//!   * **1/2/3/4** — pozlama 0,3 / 0,6 / 1,15 / 2,5
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Pozlama — eğriden önce uygulanan doğrusal çarpan.
#[derive(Clone, Copy)]
struct Exposure(f32);

impl Default for Exposure {
    fn default() -> Self {
        // Motorun kendi varsayılanı 1,15: ACES eğrisi aksi hâlde donuk okunuyor.
        Self(
            std::env::var("GIZMO_EXPOSURE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.15f32)
                .clamp(0.01, 20.0),
        )
    }
}
gizmo::core::impl_component!(Exposure);

/// Yamaların doğrusal (HDR) değerleri — logaritmik aralıklı, 0,03'ten 8,0'e.
const STEPS: [f32; 10] = [0.03, 0.06, 0.12, 0.25, 0.5, 1.0, 1.5, 2.5, 4.0, 8.0];

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Tonemapping", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            // Yamalar düz dörtgen: eğrilik gölgelemeyi karıştırırdı.
            let quad = AssetManager::create_sprite_quad(device, 1.6, 3.4);

            for (i, value) in STEPS.into_iter().enumerate() {
                let x = (i as f32 - 4.5) * 1.75;
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)),
                    GlobalTransform::default(),
                    quad.clone(),
                    // Işıksız: albedo olduğu gibi çiziliyor, yani ekrana düşen sayı yalnız
                    // pozlama + ACES'ten geçmiş oluyor. Ölçüm bunu istiyor.
                    Material::new(white.clone()).with_unlit(Vec4::new(value, value, value, 1.0)),
                    MeshRenderer::new(),
                ));
            }

            scene.world.insert_resource(Exposure::default());
            scene.spawn_camera(state, Vec3::new(0.0, 0.0, 19.0), Vec3::ZERO);
        })
        .add_update_system(exposure_input.in_phase(Phase::Update))
        // Pozlama `Renderer`'ın alanı; sistemler oraya erişemiyor.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            // Bloom bilerek kapalı: parlak yamalar aralarındaki boşluklara taşıyıp kamayı
            // okunamaz hâle getiriyordu. Ölçülmek istenen şey ton eğrisi, ton eğrisi + hale değil.
            renderer.bloom_intensity = 0.0;



            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let exposure = world.get_resource::<Exposure>().map(|e| e.0).unwrap_or(1.15);
            gizmo::egui::Area::new("tone".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(440.0);
                        ui.heading("Ton eşleme");
                        ui.label(format!("pozlama: {exposure:.2} (eğriden ÖNCE, doğrusal)"));
                        ui.label("yamalar soldan sağa doğrusal: 0,03 ... 8,0");
                        ui.separator();
                        ui.label("motorun eğrisi TEK: aces_tonemap, shader'da gömülü");
                        ui.label("Bevy'de yedi eğri + deband dither seçilebiliyor");
                        ui.label("burada seçilebilen tek şey pozlama");
                        ui.separator();
                        ui.label("1/2/3/4 — 0,3 / 0,6 / 1,15 / 2,5");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Pozlama tuşları, ve değerin **kameraya** yazılması. Rakamlar bilerek: WASDQE kameranın.
///
/// Pozlamanın adresi `Camera::exposure` — `Renderer::exposure` DEĞİL. Bu portun bulduğu şey de
/// bu: `Renderer`'ın `exposure` alanını oyun çizim yolu hiç okumuyor.
fn exposure_input(
    mut cameras: Query<(Mut<Camera>, &Transform)>,
    mut exposure: ResMut<Exposure>,
    input: Res<Input>,
) {
    use gizmo::winit::keyboard::KeyCode;
    for (key, value) in [
        (KeyCode::Digit1, 0.3),
        (KeyCode::Digit2, 0.6),
        (KeyCode::Digit3, 1.15),
        (KeyCode::Digit4, 2.5),
    ] {
        if input.is_key_just_pressed(key as u32) {
            exposure.0 = value;
        }
    }
    for (_entity, (mut camera, _)) in cameras.iter_mut() {
        if camera.primary {
            camera.exposure = exposure.0;
        }
    }
}
