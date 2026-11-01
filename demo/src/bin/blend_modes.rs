//! # Karıştırma kipleri ve saydam sıralaması
//!
//! Saydam bir yüzeyin arkasındakiyle nasıl birleştiği. Alfa karışımı en yaygını, ama toplamalı
//! (ateş, kıvılcım, cam parıltısı), çarpımsal (duman, renkli gölge) ve önceden-çarpılmış alfa da
//! ayrı ayrı gerekiyor.
//!
//! ## Dört karışım kipi (2026-11-02'den beri)
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | alfa karışımı | var — `BlendMode::Alpha`, varsayılan |
//! | toplamalı (additive) | var — `BlendMode::Additive` |
//! | çarpımsal (multiply) | var — `BlendMode::Multiply` (alfaya bakmaz; beyaza doğru solar) |
//! | önceden çarpılmış alfa | var — `BlendMode::Premultiplied` |
//! | kipi seçen bir alan | var — `Material::blend_mode` / `with_blend_mode`, inspector'da da |
//! | saydam + çift taraflı | **zaten öyle** — aşağıda ölçüldü, ve kaynaktaki yorum yanlıştı |
//!
//! Varsayılan sahne (`kipler`) dört kipi yan yana gösteriyor: aynı turuncu, yarı saydam, ışıksız
//! levha, yarısı koyu yarısı açık bir şeridin önünde. Toplamalı her yeri aydınlatır, çarpımsal her
//! yeri koyulaştırır, alfa ikisinin arasında karışır. Alfa dışındaki her kip malzemeyi saydam yapar.
//! Ölçüm: `each_blend_mode_combines_with_what_is_behind_it` (oyun yolu) ve
//! `the_editor_blends_by_the_materials_blend_mode` (editör).
//!
//! Aşağıdaki iki ölçüm kiplerden önce yapıldı ve hâlâ geçerli — sıralama kiplerden bağımsız.
//!
//! ## Saydam sıralaması örgü başına, üçgen başına değil
//!
//! Saydamlar arkadan öne sıralanıyor, ama çözünürlük örneğin merkezinde bitiyor: partiler arası
//! parti başına tek bir merkez uzaklığı, parti içinde örnek başına. **Üçgen başına hiç.**
//!
//! Bunun iki görünür sonucu var, ve ikisi de bu demoda ölçülüyor: iç içe geçmiş iki saydam nesne,
//! ve tek başına içbükey bir saydam nesne yanlış birleşiyor — ve düzeltmenin yolu yok.
//!
//! ## Ölçüldü — A: birleştirme gönderim sırasına bağlı
//!
//! İki saydam levha X biçiminde kesişiyor; tek değişen **doğurma sırası**. Ölçüldü (2026-08-23,
//! 948×492, sol yarı, HUD altı):
//!
//! | | değer |
//! |---|-------|
//! | `ab` ↔ `ba` arasında farklı piksel | **%11,98** |
//! | en büyük kanal farkı | **60** |
//! | fark kutusu | `(381, 42)–(696, 426)` — tam kesişme bölgesi |
//!
//! Aynı geometri, aynı malzeme, aynı kamera — yalnız hangisinin önce doğduğu değişiyor, ve kare
//! değişiyor. Sıradan bağımsız bir birleştirme (OIT) olsaydı fark sıfır olurdu.
//!
//! Sebebi sıralamanın çözünürlüğü: partiler arası **parti başına tek merkez uzaklığı**, parti
//! içinde **örnek başına**, üçgen başına **hiç**. İki nesne birbirinin içinden geçtiğinde tek bir
//! merkez uzaklığı hangisinin önde olduğunu söyleyemez.
//!
//! ## Ölçüldü — B: saydam yüzeyler zaten çift taraflı, ve motorun yorumu bunun tersini diyordu
//!
//! Tek bir saydam düzlem, kameraya **arkasını** dönmüş. Değişken yalnız `with_double_sided`:
//!
//! | koşu | ortalama RGB | kırmızı − mavi |
//! |------|--------------|----------------|
//! | ön yüz (kontrol) | (192,76 · 186,41 · 185,68) | +7,08 |
//! | arka yüz, `double_sided` **kapalı** | (187,30 · 179,63 · 180,94) | +6,36 |
//! | arka yüz, `double_sided` **açık** | (187,30 · 179,63 · 180,94) | +6,36 |
//!
//! İki şey birden okunuyor. Arka yüz **çiziliyor** (kırmızıya kayma +6,36, ön yüzünkine yakın),
//! ve `with_double_sided` **bit bit hiçbir şey değiştirmiyor** (0 farklı piksel, maks 0).
//!
//! Sebep kodda: `transparent` boru hattı `cull_mode: None` ile kuruluyor, ve `baked_lit_state(true)`
//! de aynısını söylüyor. Yani saydam geometri hiç yüz ayıklamıyor — `double_sided` **gereksiz**,
//! yok sayılan bir bayrak değil.
//!
//! `passes/forward.rs` bu noktada bunun **tersini** yazıyordu: *"a transparent double-sided
//! surface is single-sided in both paths"*. Ölçüm çürüttü ve yorum düzeltildi (2026-08-23).
//! Kaynaktaki bir yorumun kendi davranışını yanlış anlatması, ölçmenin okumaya üstün olduğu
//! yerlerden biri.
//!
//! ### Açık kalan
//!
//! Bunun bedeli de var ve ölçülmedi: yüz ayıklaması olmadığı için saydam bir nesnenin arka
//! yüzleri de çiziliyor, yani içbükey bir saydam örgü kendi içini de birleştiriyor — ve sıralama
//! üçgen başına olmadığı için o birleştirme de yanlış sırada. Bunu ayrı bir ölçüme bırakıyorum.
//!
//! ## Kontroller
//!   * `GIZMO_BLEND_SCENE=kipler|kesisen|cift_tarafli` — hangi sahne (varsayılan `kipler`)
//!   * `GIZMO_BLEND_ORDER=ab|ba` — kesişen sahnede iki levhayı hangi sırayla doğur
//!   * `GIZMO_BLEND_DS=1` — çift taraflı sahnede `with_double_sided` aç
//!   * `GIZMO_BLEND_FRONT=1` — levhayı ön yüzüyle çevir (kontrol)
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Hangi sahne kurulacak.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scene {
    /// Dört karışım kipi yan yana.
    Modes,
    /// İki saydam levha birbirinin içinden geçiyor — sıralamanın kırıldığı klasik durum.
    Crossing,
    /// Tek bir saydam levha, çift taraflı işaretli. Arkadan da görünmeli.
    DoubleSided,
}

fn config() -> (Scene, bool) {
    let scene = match std::env::var("GIZMO_BLEND_SCENE").as_deref() {
        Ok("cift_tarafli") => Scene::DoubleSided,
        Ok("kesisen") => Scene::Crossing,
        _ => Scene::Modes,
    };
    let swap = matches!(std::env::var("GIZMO_BLEND_ORDER").as_deref(), Ok("ba"));
    (scene, swap)
}

fn main() {
    let (which, swap) = config();

    App::<SimpleSceneState>::new("Gizmo Engine - Blend Modes", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Opak arka plan: karışımın üstüne bineceği bir şey.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, -4.0)).with_scale(Vec3::new(14.0, 8.0, 0.3)),
                GlobalTransform::default(),
                cube.clone(),
                Material::new(white.clone()).with_pbr(Vec4::new(0.55, 0.55, 0.58, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));

            match which {
                Scene::Modes => {
                    // Arka planın önünde koyu bir şerit: kiplerin farkı açık ve koyu zeminde
                    // ayrı ayrı okunur.
                    scene.world.spawn_bundle((
                        Transform::new(Vec3::new(0.0, -0.9, -3.5)).with_scale(Vec3::new(14.0, 1.6, 0.1)),
                        GlobalTransform::default(),
                        cube.clone(),
                        Material::new(white.clone()).with_unlit(Vec4::new(0.05, 0.06, 0.12, 1.0)),
                        MeshRenderer::new(),
                    ));
                    let modes = [BlendMode::Alpha, BlendMode::Additive, BlendMode::Multiply, BlendMode::Premultiplied];
                    for (i, mode) in modes.into_iter().enumerate() {
                        scene.world.spawn_bundle((
                            Transform::new(Vec3::new(-3.3 + 2.2 * i as f32, 0.0, 0.0))
                                .with_scale(Vec3::new(0.85, 1.6, 0.03)),
                            GlobalTransform::default(),
                            cube.clone(),
                            Material::new(white.clone())
                                .with_unlit(Vec4::new(1.0, 0.55, 0.2, 0.6))
                                .with_blend_mode(mode),
                            MeshRenderer::new(),
                        ));
                    }
                }
                Scene::Crossing => {
                    // İki saydam levha, X biçiminde kesişiyor. Doğurma sırası ölçümün değişkeni.
                    let a = (
                        Vec3::new(-0.6, 0.0, 0.0),
                        Quat::from_rotation_y(0.7),
                        Vec4::new(0.95, 0.35, 0.25, 0.55),
                    );
                    let b = (
                        Vec3::new(0.6, 0.0, 0.0),
                        Quat::from_rotation_y(-0.7),
                        Vec4::new(0.25, 0.55, 0.95, 0.55),
                    );
                    let order = if swap { [b, a] } else { [a, b] };
                    for (pos, rot, colour) in order {
                        scene.world.spawn_bundle((
                            Transform::new(pos)
                                .with_rotation(rot)
                                .with_scale(Vec3::new(3.2, 3.2, 0.08)),
                            GlobalTransform::default(),
                            cube.clone(),
                            Material::new(white.clone())
                                .with_pbr(colour, 0.35, 0.0)
                                .with_transparent(true),
                            MeshRenderer::new(),
                        ));
                    }
                }
                Scene::DoubleSided => {
                    // AÇIK yüzey şart: küp kapalı geometri, 180° döndürsen de kameraya hep bir
                    // ön yüz bakar, yani çift taraflılığı hiç sınamaz. Düzlem tek yüzlü.
                    //
                    // TEK levha, kameraya ARKASINI dönmüş, tam ekranın ortasında. Değişken
                    // yalnız `with_double_sided`. Çift taraflılık saydamda işliyorsa iki koşu
                    // farklı çıkar; işlemiyorsa aynı.
                    let ds = std::env::var("GIZMO_BLEND_DS").is_ok();
                    let plane = AssetManager::create_plane(device, 3.0);
                    let mat = Material::new(white.clone())
                        .with_pbr(Vec4::new(0.95, 0.30, 0.15, 0.75), 0.35, 0.0)
                        .with_transparent(true);
                    scene.world.spawn_bundle((
                        // −90° X: düzlemin normali kameradan UZAĞA bakıyor (arka yüz).
                        // +90° ile ön yüz — kontrol koşusu.
                        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(
                            if std::env::var("GIZMO_BLEND_FRONT").is_ok() {
                                std::f32::consts::FRAC_PI_2
                            } else {
                                -std::f32::consts::FRAC_PI_2
                            },
                        )),
                        GlobalTransform::default(),
                        plane,
                        if ds { mat.with_double_sided(true) } else { mat },
                        MeshRenderer::new(),
                    ));
                    gizmo::gizmo_log!(Info, "çift taraflı: {}", ds);
                }
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.3) * Quat::from_rotation_x(-0.5),
                intensity: 2.6,
                ..Default::default()
            });
            let _ = white;
            scene.spawn_camera(state, Vec3::new(0.0, 0.4, 7.0), Vec3::ZERO);
            gizmo::gizmo_log!(
                Info,
                "sahne: {} · doğurma sırası: {}",
                match which {
                    Scene::Modes => "kipler",
                    Scene::Crossing => "kesişen",
                    Scene::DoubleSided => "çift taraflı",
                },
                if swap { "b,a" } else { "a,b" }
            );
        })
        .set_ui(move |_world, _state, ctx| {
            gizmo::egui::Area::new("bm".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(400.0);
                        ui.heading("Karıştırma");
                        ui.label(format!(
                            "sahne: {} · sıra: {}",
                            match which {
                                Scene::Modes => "kipler",
                                Scene::Crossing => "kesişen levhalar",
                                Scene::DoubleSided => "çift taraflı",
                            },
                            if swap { "b,a" } else { "a,b" }
                        ));
                        ui.separator();
                        ui.label("soldan sağa: Alpha · Additive · Multiply · Premultiplied");
                        ui.label("Material::with_blend_mode(BlendMode::…)");
                        ui.separator();
                        ui.label("sıralama örgü başına — üçgen başına değil.");
                        ui.label("saydam yüzeyler zaten çift taraflı çiziliyor.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
