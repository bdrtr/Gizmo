//! # Işınım hacimleri
//!
//! Sahneye bir **prob ızgarası** serilir; motor her probun yerinden sahneyi altı yöne çizer
//! (yansıma probu gibi), gördüğünü küresel harmoniklere izdüşürür, ve ertelenmiş aydınlatma geçişi
//! nesnelerin dolaylı ışığını aradaki problardan karıştırarak okur. Statik dolaylı aydınlatmanın
//! klasik yolu.
//!
//! Burada güneş solda kırmızı, sağda mavi bir duvarı aydınlatıyor; aradaki beyaz küreler ve beyaz
//! zemin, hacim açıkken yakınlarındaki duvarın rengini alıyor — duvardan seken ışık. **G** hacmi
//! kaldırıp geri koyar: kapalıyken küreler yalnız güneşi ve gökyüzünü görür.
//!
//! ## Hacim neyi tutuyor, neyi tutmuyor
//!
//! - **Sahneden seken ışığı** tutuyor: çizimde görünen her yüzey, aydınlanmış hâliyle.
//! - **Gökyüzünü tutmuyor**: yakalamada arka plan siyah, ortam haritası, skybox ve backdrop
//!   çizilmiyor — aydınlatma geçişi gökyüzünü her yüzeye zaten kendisi ekliyor.
//! - **Işıkların kendi doğrudan ışığını tutmuyor**: bir ışık çizimde görünen bir şey değil. 2026-11-12'ye
//!   kadar hacim ışıklardan analitik pişiriliyordu — her ışığın doğrudan ışığı, gölgesiz — ve
//!   aydınlatma geçişi onu ışığın kendi ışığının üstüne bir kez daha ekliyordu.
//!
//! Bir pişirme **bir kez** yapılır (4×3×4 = 48 prob, 288 çizim): hacim ilk göründüğünde, değiştiğinde
//! ve `request_capture` ile. Işığı oynatıp yeniden pişirmek için **R**.
//!
//! ## Kontroller
//!   * **G** — hacmi aç / kapat
//!   * **R** — yeniden pişir
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::renderer::irradiance::IrradianceVolume;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Hacmin açık olup olmadığı ve kaç kez pişirme istendiği.
#[derive(Clone, Copy)]
struct GiState {
    on: bool,
    rebakes: u32,
}
gizmo::core::impl_component!(GiState);

/// Hacmin taşıyıcısı.
#[derive(Clone, Copy)]
struct VolumeHolder;
gizmo::core::impl_component!(VolumeHolder);

fn volume() -> IrradianceVolume {
    IrradianceVolume::new(Vec3::new(-5.0, -0.5, -4.0), Vec3::new(5.0, 3.5, 4.0), [4, 3, 4])
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Irradiance Volumes", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let sphere = AssetManager::create_sphere(device, 1.0, 20, 28);

            // Zemin ve iki renkli duvar (küp ±1 uzanır: ölçek yarı genişlik).
            let mut slab = |at: Vec3, half: Vec3, albedo: Vec4| {
                scene.world.spawn_bundle((
                    Transform::new(at).with_scale(half),
                    GlobalTransform::default(),
                    AssetManager::create_cube(device),
                    Material::new(white.clone()).with_pbr(albedo, 0.9, 0.0),
                    MeshRenderer::new(),
                ));
            };
            slab(Vec3::new(0.0, -0.7, 0.0), Vec3::new(12.0, 0.2, 12.0), Vec4::new(0.8, 0.8, 0.8, 1.0));
            slab(Vec3::new(-4.6, 1.5, 0.0), Vec3::new(0.15, 2.0, 4.0), Vec4::new(0.9, 0.06, 0.05, 1.0));
            slab(Vec3::new(4.6, 1.5, 0.0), Vec3::new(0.15, 2.0, 4.0), Vec4::new(0.05, 0.12, 0.9, 1.0));

            // Bir sıra beyaz küre: duvarlara yakın olanlar o duvarın rengini alır.
            for i in 0..7 {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new((i as f32 - 3.0) * 1.2, 0.1, 0.0)).with_scale(Vec3::splat(0.5)),
                    GlobalTransform::default(),
                    sphere.clone(),
                    Material::new(white.clone()).with_pbr(Vec4::ONE, 0.6, 0.0),
                    MeshRenderer::new(),
                ));
            }

            // Tepeden, iki duvara da eğik düşen bir güneş.
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_z(0.35) * Quat::from_rotation_x(-1.2),
                intensity: 3.0,
                ..Default::default()
            });
            scene.world.spawn_bundle((volume(), VolumeHolder));
            scene.world.insert_resource(GiState { on: true, rebakes: 0 });
            scene.spawn_camera(state, Vec3::new(0.0, 3.0, 8.5), Vec3::new(0.0, 0.5, 0.0));
        })
        .add_update_hook(|world, _state, _dt, input| {
            let Some(mut gi) = world.get_resource::<GiState>().map(|g| *g) else { return };
            let holder = world.query::<(Entity, &VolumeHolder)>().iter().next().map(|(e, _)| e);
            let Some(holder) = holder.and_then(|e| world.live(e)) else { return };
            if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyG as u32) {
                gi.on = !gi.on;
                if gi.on {
                    world.add_component(holder, volume());
                } else {
                    world.remove_component::<IrradianceVolume>(holder);
                }
            }
            if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyR as u32) {
                if let Some(mut v) = world.query_mut::<&mut IrradianceVolume>().get_mut(holder) {
                    v.request_capture();
                    gi.rebakes += 1;
                }
            }
            world.insert_resource(gi);
        })
        .set_ui(|world, _state, ctx| {
            let Some(gi) = world.get_resource::<GiState>().map(|g| *g) else { return };
            gizmo::egui::Area::new("gi".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.heading("Işınım hacmi");
                        ui.label("4×3×4 = 48 prob, her biri 6 yöne çizilir");
                        ui.label(format!("hacim: {} (G)", if gi.on { "açık" } else { "kapalı" }));
                        ui.label(format!("yeniden pişirme: {} (R)", gi.rebakes));
                        ui.separator();
                        ui.label("açıkken küreler yakın duvarın rengini alır:");
                        ui.label("duvardan seken ışık, gökyüzü ve ışıkların");
                        ui.label("kendi ışığı hacimde değil.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
