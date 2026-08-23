//! # Bevy'nin `3d/shadow_caster_receiver` örneğinin Gizmo Engine portu
//!
//! Nesne başına gölge ayarı: bu gölge **düşürsün mü**, bu gölge **alsın mı**.
//!
//! ## Motorda yarısı var — ve olan yarısı Bevy'den zengin
//!
//! | Bevy | Gizmo |
//! |------|-------|
//! | `NotShadowCaster` | [`ShadowCasting::Off`] |
//! | (Bevy'de yok) | **[`ShadowCasting::Only`]** — gölge düşürür ama **çizilmez** |
//! | `NotShadowReceiver` | **yok** — alma tarafı kapatılamıyor |
//!
//! Yani düşürme tarafı Gizmo'da daha ifadeli: üçüncü bir hâl var, ve gerekçesi kodun kendisinde
//! yazılı — *"pahalı bir mesh için gölge düşüren düşük poligonlu bir vekil, ya da görünmeden ışığı
//! şekillendiren bir engel."* Bevy'de bunun için ayrı bir görünmezlik bileşeni gerekiyor.
//!
//! Alma tarafı ise hiç yok. Bir nesnenin üstüne gölge düşmesini engellemek istiyorsanız
//! (kendi ışığını yayan bir panel, bir arayüz nesnesi) yolu yok — tek çare malzemeyi ışık
//! almayan bir yola sokmak, ki o da gölgeyi değil **bütün aydınlatmayı** kapatıyor.
//!
//! ## Ve ayar nesne başına, malzeme başına değil — bu bir düzeltme
//!
//! Kod bunun geçmişini de anlatıyor: gölge düşürme eskiden malzemenin yönlendirmesinden
//! türüyordu, yani aynı malzemeyi paylaşan iki nesne farklı cevap veremiyordu. Şimdi
//! [`MeshRenderer::shadows`] alanında, yani nesne başına.
//!
//! ## Ölçüldü
//!
//! Üç koşu, tek değişen ortadaki küpün hâli. Ölçüm bölgesi panelin altı (tam genişlik, `y > 420`),
//! ölçüldü (2026-08-23, 948×1028, kare 90):
//!
//! | ortadaki küp | koyu piksel | okunuşu |
//! |--------------|-------------|---------|
//! | `Off` | **%1,06** | küp çizili, gölgesi **yok** |
//! | `On` | **%2,07** | gölge geldi — koyu alan neredeyse **iki katı** |
//! | `Only` | **%4,23** | küp çizilmiyor, altındaki gölgeli zemin de görünür oldu |
//!
//! Ve kare farkları üçlünün tutarlı olduğunu gösteriyor:
//!
//! | karşılaştırma | farklı piksel | ne değişti |
//! |---------------|---------------|------------|
//! | `On` ↔ `Off` | %1,17 | yalnız gölge |
//! | `On` ↔ `Only` | %4,59 | yalnız görünürlük |
//! | `Off` ↔ `Only` | **%5,66** | **ikisi birden** — en büyük fark, beklendiği gibi |
//!
//! Üç hâl gerçekten üç ayrı davranış, ve `Only` ikisini birden çeviriyor.
//!
//! ### Ölçüm notu
//!
//! İlk ölçümde panel **solda**ydı ve içindeki "ortadaki küp: On/Off/Only" satırı koşular arasında
//! değişiyordu — yani kare farkının bir kısmı gölge değil **yazıydı**. Panel sağa alındı ve ölçüm
//! panelin altındaki bölgeye çekildi. Sayılar bundan sonrası.
//!
//! ## Kontroller
//!   * `GIZMO_SHADOW_MODE=on|off|only` — ortadaki küpün hâli
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::renderer::components::mesh::ShadowCasting;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Ortadaki küpün hâli — ölçüm koşuları arasında değişen tek şey.
fn mode() -> ShadowCasting {
    match std::env::var("GIZMO_SHADOW_MODE").as_deref() {
        Ok("off") => ShadowCasting::Off,
        Ok("only") => ShadowCasting::Only,
        _ => ShadowCasting::On,
    }
}

fn main() {
    let mode = mode();
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Shadow Caster/Receiver", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Üç küp: soldaki hep düşürür, ortadaki değişken, sağdaki hiç düşürmez.
            let settings = [
                (-2.4, ShadowCasting::On, Vec4::new(0.85, 0.55, 0.30, 1.0)),
                (0.0, mode, Vec4::new(0.35, 0.65, 0.90, 1.0)),
                (2.4, ShadowCasting::Off, Vec4::new(0.55, 0.85, 0.45, 1.0)),
            ];
            for (x, shadows, colour) in settings {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.6, 0.0)).with_scale(Vec3::splat(0.7)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(colour, 0.5, 0.0),
                    MeshRenderer::new().with_shadows(shadows),
                ));
            }

            // Zemin — gölgeyi ALAN yüzey. Almayı kapatmanın bir yolu YOK.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.6, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 24.0),
                Material::new(white).with_pbr(Vec4::new(0.55, 0.55, 0.58, 1.0), 0.85, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                // Güneş yandan: gölgeler zeminde yana düşsün, ölçülebilsin.
                rotation: Quat::from_rotation_y(0.35) * Quat::from_rotation_x(-0.75),
                intensity: 3.2,
                ..Default::default()
            });

            gizmo::gizmo_log!(Info, "ortadaki küpün hâli: {:?}", mode);
            scene.spawn_camera(state, Vec3::new(0.0, 4.2, 7.5), Vec3::new(0.0, 0.2, 0.0));
        })
        // Panel SAĞDA: ölçüm sol yarıyı okuyor, ve paneldeki metin modlar arasında değiştiği
        // için solda durursa farkın içine karışıyordu (ilk ölçümde tam olarak bu oldu).
        .set_ui(move |_world, _state, ctx| {
            gizmo::egui::Area::new("shadow".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(470.0);
                        ui.heading("Nesne başına gölge");
                        ui.label(format!("ortadaki küp: {:?}", mode));
                        ui.separator();
                        ui.label("sol  : On   — çizilir, gölge düşürür");
                        ui.label("orta : değişken (GIZMO_SHADOW_MODE)");
                        ui.label("sağ  : Off  — çizilir, gölge düşürmez");
                        ui.separator();
                        ui.label("Only: gölge düşürür ama ÇİZİLMEZ — Bevy'de yok.");
                        ui.label("NotShadowReceiver: Gizmo'da YOK, alma kapatılamıyor.");
                        ui.separator();
                        ui.label("ayar nesne başına (MeshRenderer::shadows),");
                        ui.label("malzeme başına değil — bu bir düzeltmeydi.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
