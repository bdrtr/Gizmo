//! # Alan ışıkları ve yumuşak gölgeler
//!
//! Parlak bir metal zeminin üstünde üç **alan ışığı** yan yana, ve yumuşak gölgeli bir güneş:
//!
//! | ışık | bileşen | zemindeki yansıması |
//! |------|---------|---------------------|
//! | küre (ampul) | `PointLight::with_source_radius` | ampulün yansıması kadar bir leke |
//! | tüp (floresan) | `PointLight::with_source_length` | tüp boyunca uzanan bir çizgi |
//! | dikdörtgen panel (softbox) | `RectLight` | panelin şekli |
//!
//! Üçünde de specular **temsilî nokta** yöntemiyle hesaplanıyor (yansıyan ışına kaynağın en yakın
//! noktası) ve lob enerjisi yeniden normalize ediliyor: büyük kaynak, ışığını yayar, **eklemez**.
//! Dikdörtgenin diffuse'u Lambert'in çokgen formülüyle tam — duvara yakın büyük bir panel duvarı
//! panel gibi aydınlatır, ortasındaki bir ampul gibi değil. Panel **tek yüzlü**: arkası karanlık.
//!
//! Güneş `with_angular_radius` (PCSS: gölge, düşüren nesneden uzaklaştıkça yumuşar — soldaki alçak
//! kutunun gölgesi keskin, sağdaki yüksek kutununki yumuşak) ve `with_contact_shadows` (gölge
//! haritasının kaçırdığı ince temas çizgisi) ile.
//!
//! ## `PointLight::radius` bir tuzak
//!
//! `radius` **menzil**dir — düşüşün sıfıra ulaştığı mesafe ve ayıklama sınırı — kaynak boyutu
//! **değil**. Büyütmek ışığı yayar ve zayıflatır; gölgeyi yumuşatmaz, yansımayı büyütmez. Kaynak
//! boyutu `source_radius`. Ölçüldü (2026-08-23): `radius` 2 → 6 → 20 iken sahnenin ortalama
//! parlaklığı 136,13 → 139,09 → 143,16.
//!
//! Ölçümler ve testler: `docs/CAPABILITY_GAPS.md` §A7.
//!
//! ## Kontroller
//!   * `GIZMO_SOURCE_SCALE=<sayı>` — üç kaynağın boyutunu birlikte ölçekler (varsayılan 1;
//!     0 noktasal ışıkların eski hâli)
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Üç kaynağın boyut çarpanı.
fn source_scale() -> f32 {
    std::env::var("GIZMO_SOURCE_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0f32)
        .clamp(0.0, 5.0)
}

fn main() {
    let k = source_scale();
    App::<SimpleSceneState>::new("Gizmo Engine - Area Lights", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // İki kutu, farklı yükseklikte: alçaktakinin gölgesi keskin, yüksektekinin yumuşak.
            for (x, y) in [(-3.2f32, 0.25f32), (3.2, 2.2)] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, y, -1.5)).with_scale(Vec3::splat(0.5)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(Vec4::new(0.80, 0.55, 0.35, 1.0), 0.5, 0.0),
                    MeshRenderer::new(),
                ));
            }

            // Parlak metal zemin: alan ışıklarının yansıması burada görünür.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.3, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 26.0),
                Material::new(white).with_pbr(Vec4::new(0.85, 0.85, 0.88, 1.0), 0.18, 1.0),
                MeshRenderer::new(),
            ));

            let warm = Vec3::new(1.0, 0.85, 0.65);
            // Küre: 15 cm yarıçaplı bir ampul.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(-2.4, 1.2, 0.0)),
                GlobalTransform::default(),
                PointLight::new(warm, 6.0, 12.0).with_source_radius(0.15 * k),
            ));
            // Tüp: 1,2 m'lik bir floresan, varlığın yerel X ekseni boyunca.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 1.2, 0.0)),
                GlobalTransform::default(),
                PointLight::new(Vec3::new(0.75, 0.9, 1.0), 6.0, 12.0)
                    .with_source_radius(0.03 * k)
                    .with_source_length(1.2 * k),
            ));
            // Dikdörtgen: 1 m × 0,6 m bir softbox, aşağı bakıyor (yerel −Z, X etrafında −90°).
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(2.4, 1.2, 0.0))
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                GlobalTransform::default(),
                RectLight::new(warm, 6.0, 12.0, (1.0 * k).max(0.001), (0.6 * k).max(0.001)),
            ));

            // Güneş: 1,2° açısal yarıçap (yumuşak gölge) ve 30 cm temas gölgesi.
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO)
                    .with_rotation(Quat::from_rotation_y(0.25) * Quat::from_rotation_x(-0.7)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::ONE, 1.2, LightRole::Sun)
                    .with_angular_radius(0.02)
                    .with_contact_shadows(0.3),
            ));
            gizmo::gizmo_log!(Info, "alan ışıkları: kaynak çarpanı {}", k);
            scene.spawn_camera(state, Vec3::new(0.0, 3.0, 7.5), Vec3::new(0.0, 0.0, 0.0));
        })
        .set_ui(move |_world, _state, ctx| {
            gizmo::egui::Area::new("area".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(400.0);
                        ui.heading("Alan ışıkları ve yumuşak gölgeler");
                        ui.label(format!("kaynak çarpanı: {k}  (GIZMO_SOURCE_SCALE)"));
                        ui.separator();
                        ui.label("soldan sağa: küre · tüp · dikdörtgen panel");
                        ui.label("  PointLight::with_source_radius");
                        ui.label("  PointLight::with_source_length");
                        ui.label("  RectLight (tek yüzlü, arkası karanlık)");
                        ui.separator();
                        ui.label("güneş: with_angular_radius (PCSS) · with_contact_shadows");
                        ui.label("alçak kutunun gölgesi keskin, yüksek kutununki yumuşak.");
                        ui.separator();
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 160, 80),
                            "PointLight::radius = MENZİL, kaynak boyutu DEĞİL",
                        );
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
