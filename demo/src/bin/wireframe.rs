//! # Tel kafes çizimi
//!
//! Geometriyi dolu yüzeyler yerine kenar çizgileri olarak çizmek — bir hata ayıklama görünümü.
//!
//! ## Motorda var, ama tek bayrak
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | genel tel kafes anahtarı | [`WireframeConfig::global`] |
//! | varsayılan tel kafes rengi | **yok** |
//! | nesne bazında tel kafes açma (bu nesneyi tel kafes çiz) | **yok** |
//! | nesne bazında hariç tutma (bunu hariç tut) | **yok** |
//! | nesne başına tel kafes rengi | **yok** |
//!
//! Yani ya hepsi ya hiçbiri. Nesne başına seçim ve renk yok; `WireframeConfig` tek alanlı bir
//! kaynak.
//!
//! ## İki şey kodun kendisinden okunuyor
//!
//! Tel kafes **ileri geçişte** çiziliyor (`passes/forward.rs`), ve iki tür bilerek dışarıda:
//! skybox ile backdrop. Gerekçe de orada yazılı — tel kafes boru hattı `shader.wgsl`, yani ne
//! kamerayı kilitliyor ne derinliği sabitliyor; bir backdrop'un tel kafesi backdrop'un olmadığı
//! bir yere çizilirdi.
//!
//! Ve WASM'de `PolygonMode::Line` desteklenmediği için boru hattı orada **dolu** kalıyor: aynı
//! kod tarayıcıda tel kafes üretmiyor.
//!
//! ## Ölçülen — ve eksik düğmenin bedeli burada görünüyor
//!
//! Ölçüldü (2026-08-23, kare 200, dönüş yok):
//!
//! | | kenar enerjisi | iki kare farkı |
//! |---|----------------|----------------|
//! | kapalı | 0,209 | — |
//! | açık | 0,251 (**×1,20**) | ortalama 0,33 · maks 38 · **%1,17** piksel |
//!
//! Tel kafes **çalışıyor** ama etkisi küçük, ve sebebi tesadüf değil: çizgiler dolu geometrinin
//! **üstüne** ekleniyor ve rengi **seçilemiyor**. Açık renkli bir malzemenin üstünde açık renkli
//! bir çizgi neredeyse görünmez oluyor — nesne başına renk ayarı tam olarak bunun için gerekli.
//!
//! Yani buradaki ×1,20, motorun tel kafesinin zayıf çizdiğini değil, **kontrastı ayarlanamadığını**
//! ölçüyor. Koyu bir malzemede aynı çizgiler çok daha okunur olurdu, ama o da malzemeyi
//! değiştirmek demek — tel kafesi değil.
//!
//! ### Ölçüm notu
//!
//! İlk kurulumda şekiller `Spin` ile dönüyordu ve iki koşu aynı karede farklı açıdaydı, çünkü
//! `Spin` `dt` ile ilerliyor. Dönüş kaldırılıp sabit bir eğiklik konuldu; sayılar bundan sonra
//! anlamlı.
//!
//! ## Kontroller
//!   * **T** — tel kafes aç/kapa
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::systems::render::WireframeConfig;

/// Demonun kendi bayrağı — kaynağa `set_render`'da yazılıyor.
#[derive(Clone, Copy)]
struct Wire(bool);

impl Default for Wire {
    fn default() -> Self {
        // Başsız koşuda T'ye basacak kimse yok.
        Self(std::env::var("GIZMO_WIREFRAME").is_ok())
    }
}
gizmo::core::impl_component!(Wire);

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Wireframe", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Kenarı bol geometri: tel kafes ancak üçgen yoğunluğu olan bir şeyde okunur.
            let shapes: [(f32, Mesh); 3] = [
                (-2.6, AssetManager::create_sphere(device, 1.0, 16, 24)),
                (0.0, AssetManager::create_torus(device, 0.9, 0.35, 20, 14)),
                (2.6, AssetManager::create_cylinder(device, 0.8, 1.6, 18)),
            ];
            for (x, mesh) in shapes {
                scene.world.spawn_bundle((
                    // Dönüş YOK: `Spin` `dt` ile ilerlediği için iki koşu aynı karede farklı
                    // açıda olur ve ölçüm kirlenir. Sabit bir eğiklik yeter.
                    Transform::new(Vec3::new(x, 0.0, 0.0))
                        .with_rotation(Quat::from_rotation_y(0.6) * Quat::from_rotation_x(0.3)),
                    GlobalTransform::default(),
                    mesh,
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.65, 0.68, 0.75, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.6, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 20.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.6),
                intensity: 2.4,
                ..Default::default()
            });

            scene.world.insert_resource(Wire::default());
            scene.spawn_camera(state, Vec3::new(0.0, 1.5, 7.5), Vec3::ZERO);
        })
        .add_update_system(toggle.in_phase(Phase::Update))
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            // Tek bayrak, tek kaynak — nesne başına seçim yok.
            let on = world.get_resource::<Wire>().map(|w| w.0).unwrap_or(false);
            world.insert_resource(WireframeConfig { global: on });

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let on = world.get_resource::<Wire>().map(|w| w.0).unwrap_or(false);
            gizmo::egui::Area::new("wire".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(460.0);
                        ui.heading("Tel kafes");
                        ui.label(format!("global: {}", if on { "açık" } else { "kapalı" }));
                        ui.separator();
                        ui.label("WireframeConfig tek alanlı: ya hepsi ya hiçbiri.");
                        ui.label("varsayılan renk, nesne bazında açma/hariç tutma ve");
                        ui.label("nesne başına renk ayarı yok.");
                        ui.separator();
                        ui.label("ileri geçişte çiziliyor; skybox ve backdrop bilerek hariç.");
                        ui.label("WASM'de PolygonMode::Line yok — orada dolu kalıyor.");
                        ui.separator();
                        ui.label("T — aç/kapa");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// **T** bayrağı çevirir.
fn toggle(mut wire: ResMut<Wire>, input: Res<Input>) {
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyT as u32) {
        wire.0 = !wire.0;
    }
}
