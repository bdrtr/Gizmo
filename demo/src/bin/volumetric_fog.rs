//! # Hacimsel sis ve tanrı ışınları
//!
//! Işığın havadaki toza çarpıp **görünür** hâle gelmesi: pencereden düşen huzmeler, ormanda
//! ağaçların arasından süzülen ışık. Yüzeye değil, aradaki hacme uygulanan bir etki.
//!
//! ## Motorda var — ve **hiçbir ayarı yok**
//!
//! Tanrı ışınları çalışıyor: [`VolumetricState`] bir ışın yürütme geçişi kuruyor ve kareye
//! uyguluyor. Ama `VolumetricState`'in genel yüzeyi yalnız GPU nesneleri — doku, görünüm, boru
//! hattı, bağlama grubu, genişlik, yükseklik. **Tek bir şekillendirme alanı yok.**
//!
//! Etkiyi belirleyen her sayı gölgelendiricide gömülü:
//!
//! | ne | değer | nerede |
//! |----|-------|--------|
//! | faz anizotropisi (g) | `0.55` | `volumetric.wgsl:105` |
//! | ışın yürütme adımı | `16` | `:91` |
//! | yürütme mesafesi tavanı | `100.0` m | `:79`, `:83` |
//! | güneş saçılım katsayısı | `0.0015` | `:106` |
//! | ampul saçılım katsayısı | `0.0008` | `:152` |
//! | gölge sapması | `0.16` | `:129` |
//!
//! ## Ve kapatmanın da bir yolu yok — yalnız yok etmenin
//!
//! `enabled` diye bir bayrak yok. Tek kapatma yolu `renderer.volumetric = None`, ve bu **durumu
//! yok ediyor**: geri açmak `VolumetricState::new(device, scene, deferred, w, h)` ile yeniden
//! kurmak demek. Aynı desen SSR'da da var (`docs/CAPABILITY_GAPS.md` §B).
//!
//! ## Mesafe sisi de sabit, ve tabanı kıpırdamıyor
//!
//! Mesafe sisi dört gömülü preset + bir karışım katsayısı. Renk, yoğunluk ve yükseklik düşüşü
//! gölgelendiricide sabit — "yoğunluk 0,02" cümlesi kurulamıyor.
//!
//! Ve sis düzleminin yüksekliği preset'e göre bile değişmiyor: `deferred_lighting.wgsl:254`'te
//! tek bir satır, `fog_base_height = -5.0`. Sis tabakası yukarı ya da aşağı taşınamıyor.
//!
//! ## Sürüklenen / gürültü sürücülü sis yok
//!
//! Motorun hiçbir yerinde `yoğunluk = fbm(p − rüzgâr·t)` yok, ve yazılacak bir rüzgâr vektörü de
//! yok: ne uniform, ne bileşen, ne `Material` alanı. Zamanla değişen tek hacimsel yoğunluk
//! `SmokeVolume`'un taşınan ızgarası, ve orada bile gürültü **hıza** uygulanıyor — yoğunluk
//! kaynak enjeksiyonu + yarı-Lagrange taşıma + dağılmadan geliyor, yani duran ve sürüklenen bir
//! sis olarak yazılamıyor.
//!
//! ## Ölçüldü — etki gerçek, ayar sıfır
//!
//! Aynı sahne iki kez, tek fark hacimsel geçişin yok edilip edilmemesi (2026-08-23, 948×1028,
//! sol yarı, HUD altı):
//!
//! | | değer |
//! |---|-------|
//! | farklı piksel | **%19,55** |
//! | ortalama fark | 12,01 |
//! | en büyük kanal farkı | **134** |
//! | fark kutusu | tüm kare |
//! | ortalama parlaklık, yok | 117,63 |
//! | ortalama parlaklık, açık | 114,87 (**−2,76**) |
//!
//! Etki karenin beşte birine dokunuyor ve yer yer 134 seviyelik fark yapıyor — yani çalışıyor,
//! ve zayıf da değil. Parlaklığın **düşmesi** de doğru işaret: hacimsel saçılım yalnız ışık
//! eklemiyor, ışığın önünü de kesiyor.
//!
//! Ama bu 134'ün hiçbir yanı ayarlanabilir değil. Yukarıdaki altı sabitin hiçbirine Rust
//! tarafından ulaşılmıyor, ve `enabled` bayrağı olmadığı için "biraz daha az" demenin bir yolu
//! yok — yalnız "hiç" demenin, o da durumu yok ederek.
//!
//! ## Kontroller
//!   * `GIZMO_VOL=0` — hacimsel geçişi yok et (tek kapatma yolu)
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

fn main() {
    let on = !matches!(std::env::var("GIZMO_VOL").as_deref(), Ok("0"));

    App::<SimpleSceneState>::new("Gizmo Engine - Volumetric Fog", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Işığı kesen sütunlar: huzmelerin görünmesi için gölge gerekiyor.
            for i in 0..5 {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new((i as f32 - 2.0) * 2.4, 2.2, -2.0))
                        .with_scale(Vec3::new(0.5, 4.4, 0.5)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.30, 0.31, 0.34, 1.0),
                        0.8,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 40.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.40, 0.40, 0.43, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));

            // Güneş alçaktan ve sütunların arkasından: huzmeler kameraya doğru düşsün.
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.05) * Quat::from_rotation_x(-0.28),
                intensity: 4.0,
                ..Default::default()
            });
            let _ = white;
            scene.spawn_camera(state, Vec3::new(0.0, 1.0, 7.0), Vec3::new(0.0, 1.2, -2.0));
        })
        .set_render(move |world, _state, encoder, view, renderer, _lt| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            // Kapatmanın TEK yolu: durumu yok etmek. `enabled` bayrağı yok, ve geri açmak
            // `VolumetricState::new(...)` ile yeniden kurmak demek.
            if !on {
                renderer.volumetric = None;
            }
            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(move |world, _state, ctx| {
            let live = world
                .get_resource::<gizmo::renderer::Renderer>()
                .is_some();
            let _ = live;
            gizmo::egui::Area::new("vf".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(420.0);
                        ui.heading("Hacimsel sis");
                        ui.label(format!("hacimsel geçiş: {}", if on { "açık" } else { "yok edildi" }));
                        ui.separator();
                        ui.label("çalışıyor — ama ŞEKİLLENDİRME ALANI YOK.");
                        ui.monospace("  g = 0.55 · adım = 16 · tavan = 100 m");
                        ui.monospace("  güneş 0.0015 · ampul 0.0008 · sapma 0.16");
                        ui.label("altısı da gölgelendiricide gömülü sabit.");
                        ui.separator();
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 160, 80),
                            "enabled bayrağı yok — kapatmak = yok etmek",
                        );
                        ui.separator();
                        ui.label("mesafe sisi: 4 preset + karışım katsayısı.");
                        ui.label("fog_base_height = -5.0, preset'e göre bile değişmiyor.");
                        ui.label("sürüklenen/gürültülü sis: hiç yok.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
