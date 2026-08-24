//! # Ertelenmiş ve ileri çizim yolları
//!
//! Aynı nesne, iki farklı çizim yolu: **ertelenmiş** (önce G-tamponuna yaz, sonra tek geçişte
//! ışıklandır) ve **ileri** (çizerken ışıklandır).
//!
//! ## Gizmo'da seçim serbest değil — malzeme **türünden** çıkıyor
//!
//! Motorun `routing.rs`'i tek bir soruyu tek yerde cevaplıyor: bir [`MaterialType`] bir çizim
//! döngüsü için ne demek. Ve cevabı `route()` veriyor, demo da onu doğrudan çağırıp HUD'a
//! basıyor — tahmin etmiyor:
//!
//! | tür | ertelenmişi atlar mı | örnek bayrağı |
//! |-----|----------------------|---------------|
//! | `Pbr` | hayır | 0,0 |
//! | `Water` | **evet** | 1,0 |
//! | `Unlit` | **evet** | 1,0 |
//! | `BakedLit` | **evet** | 1,0 |
//! | `Skybox` | **evet** | 2,0 |
//! | `Backdrop` | **evet** | 1,0 |
//! | `Grid` | hayır | 0,0 |
//!
//! Yani **PBR bir malzemeyi ileri yolda çizdirmenin yolu yok**. İleri yola geçmek malzemenin
//! türünü değiştirmek demek, ve tür yalnız yolu değil gölgelemeyi de değiştiriyor: `Unlit`
//! ışıktan hiç etkilenmez, `BakedLit` yalnız güneşin gölgesini alır (nokta ışıklarını almaz),
//! `Backdrop` kameraya kilitlenir ve derinlik yazmaz. Yani yol ile gölgeleme burada ayrı iki
//! eksen değil, tek eksen.
//!
//! Bu bir eksiklik değil, bir tasarım kararı — ve `routing.rs` gerekçesini de yazıyor: iki çizim
//! döngüsü (oyun ve editör) bu soruyu ayrı ayrı `match … { _ => 0.0 }` ile cevaplıyordu, ve o
//! joker dal iki yeteneğin sessizce kaybolduğu yerdi. Şimdi karar tek yerde ve orada joker yok,
//! yani `MaterialType`'a onuncu bir varyant eklemek **derleme hatası** veriyor.
//!
//! ## Ölçülen: yollar gerçekten farklı
//!
//! Demo aynı küreyi, aynı albedo ile, dört tür olarak yan yana koyuyor: `Pbr`, `BakedLit`,
//! `Unlit`, `Water`. Işık ve **kaynak** geometri özdeş; değişen tek şey tür, yani yol — ve
//! `Water`'da yol geometriyi de değiştiriyor, çünkü su gölgelendiricisi vertex'leri oynatıyor.
//!
//! Ölçüldü (2026-08-24, kare 240, 948×1028; üç koşu). "Gölge" = kürenin tam altındaki zemin ile
//! **oraya oturtulmuş temiz-zemin modeli** arasındaki fark — zemin parlaklığı kadrada düz değil
//! (x=460'ta 136,7 · x=900'de 125,2), o yüzden tek bir referans sütunu sağdaki küreye olmayan bir
//! gölge uyduruyor. Model x ve y'de kübik, hiçbir kürenin altında kalmayan zemin pikselleriyle
//! kuruluyor; artık σ 0,54, en büyük |artık| 2,15 — gölge sütununun gürültü tabanı bu:
//!
//! | tür | küre ort | küre **std** | gölge |
//! |-----|----------|--------------|-------|
//! | `Pbr` | 150,66 | **38,55** | −39,63 |
//! | `BakedLit` | 222,79 | **2,34** | −39,81 |
//! | `Unlit` | 222,81 | **2,34** | **+0,29** |
//! | `Water` | 208,6 – 221,1 | **29,9 – 31,6** | **−2,50** |
//!
//! Belirleyici sütun **std**: `Pbr` ve `Water` gölgeleniyor (küre üzerinde gerçek bir geçiş var),
//! `BakedLit` ile `Unlit` ise dümdüz. Gölge sütunu da `routing.rs`'in dediğini doğruluyor —
//! ertelenmişi atlayan üç türden yalnız `BakedLit` gölge düşürüyor; `Unlit` (+0,29) ve `Water`
//! (−2,50) düşürmüyor, ve ikinci sayı gürültü tabanının içinde.
//!
//! **`Water` satırı 2026-08-24'te değişti, ve ölçüm bunu iki yerden gösteriyor.** O tarihe kadar
//! `route(Water)` tam olarak `route(Pbr)`'nin cevabını veriyordu: su ertelenmiş PBR olarak
//! çiziliyor, gölge düşürüyordu (ölçülmüştü: −26,51). Artık kendi ileri boru hattında — bu yüzden
//! gölge sütunu sıfıra indi, ve bu yüzden küre **deforme**: `water.wgsl` vertex'leri Gerstner
//! dalgasıyla ötelediği için küre artık küre değil.
//!
//! Ve `Water` bu tablonun **tek oynayan satırı**: üç koşuda diğer üç tür bit-bit aynı sayıları
//! verirken (150,66 · 222,79 · 222,81, std'leri dâhil) su 221,1 → 208,6 → 209,8 geziyor. Sebebi
//! ölçüm gürültüsü değil, özelliğin kendisi — öteleme geçen SÜREYE bağlı, ve kare 240'a kaç
//! saniyede varıldığı koşudan koşuya değişiyor.
//!
//! **Ve bir yan bulgu:** `BakedLit` ile `Unlit` bu sahnede **aynı pikselleri** üretiyor —
//! 222,23'e karşı 222,26, ikisinin de std'si 1,46. Sebebi kendi belgesinde yazılı: `BakedLit`
//! ışığı **köşe renginden** okuyor, bu mesh'in köşe rengi yok, o yüzden düz albedo'ya düşüyor.
//! Aradaki tek görünür fark gölge düşürmesi.
//!
//! ### Ölçüm sahneyi ölçüme uydurmayı gerektirdi
//!
//! İlk kurulumda güneş eğikti ve gölgeler komşu kürelerin arasına taşıyordu; "hangi tür gölge
//! düşürüyor" sorusu ölçülemez hâle geliyordu, çünkü referans aldığım temiz zemin aslında başka
//! bir kürenin gölgesiydi. Güneş tepeye alınıp aralar açılınca her gölge kendi küresinin altında
//! kaldı ve ölçüm anlam kazandı.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::renderer::components::MaterialType;
use gizmo::renderer::routing::route;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Yan yana konan türler ve HUD'daki adları.
const SHOWN: [(MaterialType, &str); 4] = [
    (MaterialType::Pbr, "Pbr"),
    (MaterialType::BakedLit, "BakedLit"),
    (MaterialType::Unlit, "Unlit"),
    (MaterialType::Water, "Water"),
];

/// Dördünün de paylaştığı albedo — fark yalnız yoldan gelsin.
const ALBEDO: Vec4 = Vec4::new(0.55, 0.62, 0.78, 1.0);
/// Küreler arasındaki aralık.
const SPACING: f32 = 4.2;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Deferred Rendering", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.6, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 40.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.14, 0.15, 0.17, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));

            let sphere = AssetManager::create_sphere(device, 1.2, 24, 40);
            for (i, (material_type, _)) in SHOWN.into_iter().enumerate() {
                let x = (i as f32 - 1.5) * SPACING;
                // Tek fark tür: albedo, pürüz, metaliklik ve kaynak mesh özdeş. (Ekranda
                // `Water` yine de deforme görünür — su boru hattı vertex'leri Gerstner dalgasıyla
                // öteliyor, yani tür yalnız gölgelemeyi değil şekli de seçiyor.)
                let mut material = Material::new(white.clone()).with_pbr(ALBEDO, 0.45, 0.0);
                material.material_type = material_type;
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)),
                    GlobalTransform::default(),
                    sphere.clone(),
                    material,
                    MeshRenderer::new(),
                ));
            }

            // Güneş neredeyse tam tepede: her gölge kendi küresinin ALTINA düşsün, aradaki
            // zemin temiz kalsın. Eğik ışıkta gölgeler komşuların arasına taşıyor ve "hangi tür
            // gölge düşürüyor" sorusu ölçülemez hâle geliyor — bu sahnede tam olarak o oldu.
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-1.45),
                intensity: 1.8,
                ..Default::default()
            });
            // Nokta ışığı bilerek: `BakedLit` onu almıyor, `Pbr` alıyor — fark burada da görünsün.
            scene.world.spawn_bundle(PointLightBundle {
                position: Vec3::new(0.0, 3.0, 6.0),
                intensity: 6.0,
                radius: 30.0,
                ..Default::default()
            });

            scene.spawn_camera(state, Vec3::new(0.0, 2.4, 15.0), Vec3::new(0.0, 0.0, 0.0));
        })
        .set_ui(|_world, _state, ctx| {
            gizmo::egui::Area::new("deferred".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(470.0);
                        ui.heading("Ertelenmiş mi, ileri mi");
                        ui.label("soldan sağa: Pbr · BakedLit · Unlit · Water");
                        ui.label("albedo, pürüz, metaliklik ve kaynak mesh ÖZDEŞ");
                        ui.label("(Water deforme: su gölgelendiricisi vertex öteliyor)");
                        ui.separator();
                        // Tablo motorun kendi `route()`'undan — demo tahmin etmiyor.
                        ui.label("routing::route() ne diyor:");
                        for (material_type, name) in SHOWN {
                            let r = route(material_type);
                            ui.label(format!(
                                "  {name:9} ertelenmişi atlar: {:5} · örnek bayrağı {:.1}",
                                r.skips_deferred, r.instance_flag
                            ));
                        }
                        ui.separator();
                        ui.label("PBR bir malzemeyi ileri yolda çizdirmenin yolu YOK");
                        ui.label("yol ile gölgeleme ayrı iki eksen değil, tek eksen");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
