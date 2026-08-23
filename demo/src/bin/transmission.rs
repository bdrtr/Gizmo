//! # Işığın içinden geçtiği malzemeler
//!
//! Cam, su, buzlu cam, mermer: ışığı yansıtmakla kalmayıp **içinden geçiren** yüzeyler.
//! Fiziksel olarak bunun üç ayarı olur — ne kadar geçiriyor (transmission), ne kadar kalın
//! (thickness), ve içeride ne kadar soğuruyor (Beer–Lambert).
//!
//! Yanında bir dördüncü ayar daha durur: **speküler renk**. Metal olmayan bir yüzeyin
//! yansımasının rengi. Gerçekte 0,04 civarı nötr bir değerdir ama bazı malzemelerde (altın
//! kaplama, bazı boyalar, ıslak yüzeyler) renklidir.
//!
//! ## Motorda ikisi de yok, ve ikisi de aynı duvara çarpıyor
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | malzeme başına geçirgenlik | **yok** — `Material`'da alan yok |
//! | kalınlık / soğurma | **yok** (malzemede) |
//! | kırılma indisi | **yok** |
//! | speküler renk (F0 tinti) | **yok** — F0 sabit `0.04` |
//! | `MaterialType::Water` | **var ama ölü** — aşağıda |
//!
//! `Material`'ın gölgeleme alanları şunlar ve hepsi bu: `roughness`, `metallic`, `anisotropy`,
//! `clear_coat`, `subsurface`, `alpha_cutoff`, artı `albedo`, `ambient`, `emissive`. Geçirgenlik
//! ya da speküler renk yok.
//!
//! ## Duvar: G-tamponunun dört hedefi de dolu
//!
//! Ertelenmiş yolda malzeme bilgisi dört hedefe sığdırılmış, ve dördü de tamamen dolu:
//!
//! ```text
//! RT0  albedo.rgb        + metallic
//! RT1  normal.xyz        + roughness
//! RT2  world_pos.xyz     + w'de ondalık paketlemeyle subsurface VE anisotropy
//! RT3  tangent.xyz       + w'de el işareti VE clear_coat
//! ```
//!
//! Yani yeni bir malzeme kanalı eklemek boş bir alana yazmak değil, **beşinci bir hedef açmak**
//! ya da paketlemeyi daha da sıkıştırmak demek. Speküler renk üç bileşen ister; geçirgenlik en az
//! iki. İkisi de bu bütçeye sığmıyor.
//!
//! ## İroni: gölgelendirici zaten renkli speküler yapabiliyor
//!
//! BRDF fonksiyonlarının imzası bunu **zaten** kabul ediyor: `F_Schlick(VoH: f32, f0: vec3<f32>)`,
//! ve doğrudan aydınlatma girişleri de `f0: vec3<f32>` alıyor. Eksik olan gölgelendirici değil,
//! oraya renkli bir değer taşıyacak Rust tarafı. Bugün her iki gölgelendirici de aynı satırı
//! yazıyor:
//!
//! ```text
//! let f0 = mix(vec3<f32>(0.04), albedo, metallic);
//! ```
//!
//! Yani F0'ı oynatmanın **tek** yolu `metallic` — ve o, difüzü de birlikte kapatıyor
//! (`kD = (1 - F) * (1 - metallic)`). Demo bunu ölçüyor: metalikliği artırmak yansımayı
//! renklendiriyor ama nesneyi de karartıyor. İkisi ayrılamıyor.
//!
//! ## Üçüncü ölü rota: `MaterialType::Water`
//!
//! `water.wgsl` yazılmış, `water_pipeline` derleniyor, `renderer.scene`'e ve `Renderer`'a
//! konuyor — ve **hiçbir geçiş onu `set_pipeline` ile bağlamıyor**. `MaterialType::Water` yalnız
//! testlerde, kıyaslamalarda ve yönlendirme tablolarında geçiyor; hiçbir oyun kodu üretmiyor,
//! hiçbir çizim yolu seçmiyor. Doğrulandı 2026-08-23.
//!
//! Bu, `gizmo-renderer::gi` ve `gizmo-renderer::visibility`'den sonra **üçüncüsü**.
//!
//! ## Ölçüldü — F0'ı oynatmanın tek yolu, iki şeyi birden bozuyor
//!
//! Üç turuncu küre (albedo 0,90 · 0,62 · 0,20), tek değişen `metallic`. Ölçüm sol yarı, küre
//! bölgesi (2026-08-23, 948×492):
//!
//! | metaliklik | ortalama RGB | parlaklık | kırmızı − mavi |
//! |------------|--------------|-----------|----------------|
//! | 0,00 | (106,20 · 104,98 · 97,16) | 104,67 | **+9,04** |
//! | 0,25 | (101,83 · 100,68 · 93,39) | 100,40 | +8,44 |
//! | 0,50 | (95,45 · 95,07 · 89,53) | 94,75 | +5,92 |
//! | 1,00 | (79,32 · 79,39 · 79,49) | **79,38** | **−0,17** |
//!
//! İki sütun birden kötüye gidiyor, ve **üç ayrı boşluk üst üste biniyor**:
//!
//! 1. **Difüz ölüyor.** Parlaklık %24 düşüyor (104,67 → 79,38), çünkü `kD = (1 − F)(1 − metallic)`
//!    difüzü metalikliğe bağlamış. Speküleri renklendirmek isterken nesneyi karartıyorsunuz.
//!
//! 2. **Ve renk de gelmiyor.** F0 tam metalikte albedoya eşitleniyor — yani turuncu olmalı. Ama
//!    kırmızı-mavi farkı **+9,04'ten −0,17'ye** düşüyor: küre turuncudan **griye** dönüyor.
//!
//! 3. Sebep üçüncü boşluk: yansıtacak bir **çevre yok**. Motorun çevre küp haritası / yansıma
//!    probu yok, o yüzden tam metalik bir yüzeyin yansıtacağı tek şey tek yönlü ışığın parlaması
//!    ve ortam terimi — ikisi de renksiz.
//!
//! Yani "speküler renk" için elde kalan tek kol, hem difüzü söndürüyor hem de vaat ettiği rengi
//! getiremiyor. Ayrı bir `specular_tint` alanı olsaydı üçünü de aşardı — ama G-tamponunda ona yer
//! yok.
//!
//! ## Kontroller
//!   * `GIZMO_TRANS_METALLIC=<0..1>` — metaliklik (F0'ı oynatmanın tek yolu)
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

fn metallic() -> f32 {
    std::env::var("GIZMO_TRANS_METALLIC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0f32)
        .clamp(0.0, 1.0)
}

fn main() {
    let m = metallic();

    App::<SimpleSceneState>::new("Gizmo Engine - Transmission", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let sphere = AssetManager::create_sphere(device, 1.0, 32, 44);

            // Renkli bir albedo: F0 metalikliğe göre bu renge kayıyor. Ölçülen şey tam olarak
            // bu kaymanın difüzü de götürüp götürmediği.
            for (i, x) in [-2.6f32, 0.0, 2.6].iter().enumerate() {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(*x, 0.3, 0.0)),
                    GlobalTransform::default(),
                    sphere.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.90, 0.62, 0.20, 1.0),
                        0.15 + i as f32 * 0.2,
                        m,
                    ),
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 26.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.30, 0.31, 0.34, 1.0), 0.85, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.55),
                intensity: 3.0,
                ..Default::default()
            });
            let _ = white;
            scene.spawn_camera(state, Vec3::new(0.0, 1.4, 7.5), Vec3::new(0.0, 0.2, 0.0));
            gizmo::gizmo_log!(Info, "metaliklik: {:.2}", m);
        })
        .set_ui(move |_world, _state, ctx| {
            gizmo::egui::Area::new("tr".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(420.0);
                        ui.heading("Geçirgenlik ve speküler renk");
                        ui.label(format!("metaliklik: {m:.2}"));
                        ui.separator();
                        ui.label("geçirgenlik / kalınlık / kırılma: malzemede YOK.");
                        ui.label("speküler renk: F0 sabit 0.04.");
                        ui.separator();
                        ui.label("gölgelendirici zaten f0: vec3 alıyor —");
                        ui.label("eksik olan oraya değer taşıyan Rust tarafı.");
                        ui.separator();
                        ui.label("F0'ı oynatmanın tek yolu metallic,");
                        ui.label("ve o difüzü de kapatıyor: kD = (1-F)(1-metallic).");
                        ui.separator();
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 160, 80),
                            "MaterialType::Water: derleniyor, hiç bağlanmıyor",
                        );
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
