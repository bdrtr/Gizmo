//! # Karşılığı olmayan dört ECS yeteneği
//!
//! Bu demo dört ayrı yeteneği birden ele alıyor — değişmez bileşen, çalışma anında bileşen
//! tanımı, render dünyasına çıkarma, sistem sıcak yaması — ve dördünün de cevabı aynı biçimde
//! başlıyor: **Gizmo'da bu yok.**
//!
//! Ama "yok" tek başına bir bilgi değil. Demo her biri için *neyin* yok olduğunu, yerine ne
//! konabileceğini ve ne kaybedildiğini ölçüyor.
//!
//! ## 1. Değişmez bileşen
//!
//! Bir bileşenin **değişmez** ilan edilebilmesi şu demek: `&mut` erişimi **derleme zamanında**
//! engellenir, tek değiştirme yolu sökülüp yeniden takılmasıdır, ve bu sayede `on_insert`
//! kancası bileşenin tek doğruluk kapısı hâline gelir.
//!
//! Gizmo'nun `Component` trait'inin böyle bir ayarı yok: `Mut<T>` her bileşen için kurulabiliyor.
//! Yakınsayan tek şey **`&T` alan bir sorgu** yazmak — ama bu bir sözleşme değil, bir tercih:
//! aynı türe `Mut<T>` isteyen ikinci bir sorgu yazmak serbest.
//!
//! Sonuç: `component_hooks`'ta ölçülen sessiz yollar burada iki kat pahalı. Değişmez bir
//! bileşenin her değişimi bir kancadan geçmek zorunda olurdu; Gizmo'da hem doğrudan
//! yazılabiliyor hem de bazı yollar kanca ateşlemiyor.
//!
//! ## 2. Çalışma anında bileşen tanımlamak
//!
//! Bir betik dilinden gelen bileşenleri çalışma anında kaydedip ham baytlarla okuyup yazmak.
//!
//! Gizmo'da bileşen kaydı tipli: `World::register_component_type::<T>()`. Çalışma anında yeni
//! bir bileşen **türü** üretmenin yolu yok. Demo bunu ölçüyor: kayıtlı bileşen sayısı yalnız
//! tipli kayıtla artıyor.
//!
//! Ham erişim tamamen kapalı da değil — `get_component_ptr(entity, TypeId)` var, ama `TypeId`
//! istiyor, yani zaten var olan bir tür gerekiyor. Yani "dinamik" olan erişim, tanım değil.
//!
//! ## 3. Render dünyasına çıkarma
//!
//! Çıkarma iki dünya demek: oyun dünyası ve render dünyası, ve aradaki çıkarma çizelgesi her kare
//! gerekli veriyi kopyalar. Bu, render'ın oyun mantığıyla **paralel** koşabilmesini sağlar.
//!
//! Gizmo'da **tek dünya** var. Render sistemleri oyun bileşenlerini doğrudan okuyor; çıkarma
//! aşaması yok, çünkü kopyalanacak bir yer yok. Kazanç: bir kare gecikme yok, kod daha az.
//! Kayıp: render, oyun mantığıyla aynı dünyayı ödünç aldığı için ikisi gerçekten paralel koşamıyor.
//!
//! Demo bunu gösteriyor: `set_render` içinden oyun bileşenleri okunuyor, hiçbir çıkarma adımı yok.
//!
//! ## 4. Çalışırken kod değiştirmek
//!
//! Sistem gövdelerini çalışırken değiştirmek. Gizmo'da bunun karşılığı **yok** — ve depoda
//! `subsecond`/`hotpatch` diye bir bağımlılık da yok.
//!
//! Ama motorun bir sıcak yeniden yükleme yolu **var**, ve başka bir katmanda: **shader'lar**.
//! Her renderer shader'ı önce diskten bir üstünü arıyor, bulamazsa gömülü kopyaya düşüyor, ve
//! stüdyo `demo/assets` altındaki `.wgsl` değişimlerinde boru hatlarını yeniden kuruyor. Yani
//! "çalışırken değiştirilebilen" şey Rust kodu değil, shader.
//!
//! ## Ölçüldü
//!
//! Ölçüldü (2026-08-23):
//!
//! | ne | ölçülen | ne söylüyor |
//! |----|---------|-------------|
//! | `Mut<Frozen>` kurulabildi mi | **evet** | "değişmezlik" bir sözleşme değil, tercih |
//! | `Frozen`'ın kancasız yazılan değeri | 10 -> **−1** | doğrudan yazma açık, kapı yok |
//! | kayıtlı bileşen türü sayısı | 0 -> 1 -> **2** | yalnız **tipli** kayıtla artıyor |
//! | `get_component_ptr` + `TypeId` ile ham okuma | **`Some(100)`** | erişim dinamik, **tanım değil** |
//! | `set_render`'ın gördüğü oyun bileşeni | **4** | çıkarma adımı yok, tek dünya |
//!
//! Beş satırın hepsi aynı şeyi başka yerden söylüyor: dördü de *bir katman* eksik.
//! Değişmezlikte tür sistemi katmanı, dinamikte tanım katmanı, çıkarmada ikinci dünya, sıcak
//! yamada çalışma-anı kod değiştirme. Erişim ve veri tarafı çalışıyor; eksik olan **kapı**.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::system::{IntoSystemConfig, Phase, ResMut};
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Sözleşmesiz "değişmez" bileşen — yalnız `&T` ile okunması bir **tercih**, kural değil.
#[derive(Clone, Copy)]
struct Frozen(f32);
gizmo::core::impl_component!(Frozen);

/// Çalışma anında kaydedilmiş gibi davranan bileşenler — aslında hepsi derleme zamanında tipli.
#[derive(Clone, Copy)]
struct SlotA(u32);
gizmo::core::impl_component!(SlotA);
#[derive(Clone, Copy)]
struct SlotB;
gizmo::core::impl_component!(SlotB);

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct AbsentReport {
    /// Kayıtlı bileşen türü sayısı, üç noktada.
    registered_start: usize,
    registered_after_one: usize,
    registered_after_two: usize,
    /// `Mut<Frozen>` kurulabildi mi — evet ise "değişmezlik" bir sözleşme değil.
    frozen_mutable: bool,
    /// `Frozen`'ın doğrudan yazılarak değişmiş değeri.
    frozen_value: f32,
    /// Render kancasının oyun dünyasından okuyabildiği varlık sayısı — çıkarma adımı yok.
    seen_from_render: usize,
    /// Ham işaretçiyle okunan değer — `dynamic`'in erişim yarısı.
    raw_read: Option<u32>,
}
gizmo::core::impl_component!(AbsentReport);

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - ECS Absent APIs", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            let mut report = AbsentReport {
                registered_start: scene.world.registered_component_count(),
                ..Default::default()
            };

            // 2. `dynamic`: kayıt TİPLİ. Çalışma anında yeni bir tür üretilemiyor; yapılabilen
            //    tek şey derleme zamanında var olan bir türü kaydetmek.
            scene.world.register_component_type::<SlotA>();
            report.registered_after_one = scene.world.registered_component_count();
            scene.world.register_component_type::<SlotB>();
            report.registered_after_two = scene.world.registered_component_count();

            for i in 0..4 {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new((i as f32 - 1.5) * 1.6, 0.0, 0.0))
                        .with_scale(Vec3::splat(0.55)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.45, 0.70, 0.55, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Frozen(10.0 + i as f32),
                    SlotA(100 + i as u32),
                ));
            }

            // 1. `immutable_components`: "değişmez" olması gereken bileşen doğrudan yazılıyor,
            //    ve motor buna karşı çıkmıyor. Sözleşme yok.
            if let Some(mut q) = scene.world.query_mut::<gizmo::core::query::Mut<Frozen>>() {
                report.frozen_mutable = true;
                for (_id, mut frozen) in q.iter_mut() {
                    frozen.0 = -1.0; // hiçbir kanca, hiçbir kapı
                }
            }
            report.frozen_value = scene
                .world
                .query::<&Frozen>()
                .and_then(|q| q.iter().next().map(|(_, f)| f.0))
                .unwrap_or(0.0);

            // 2b. `dynamic`'in erişim yarısı: ham işaretçi var, ama `TypeId` istiyor — yani
            //     zaten var olan bir tür gerekiyor.
            let first = scene.world.query::<&SlotA>().and_then(|q| q.iter().next().map(|(id, _)| id));
            report.raw_read = first
                .and_then(|id| scene.world.entity(id))
                .and_then(|entity| {
                    scene
                        .world
                        .get_component_ptr(entity, std::any::TypeId::of::<SlotA>())
                })
                // SAFETY: işaretçi `TypeId::of::<SlotA>()` ile anahtarlandı, yani baytlar
                // `SlotA`; değer hemen kopyalanıyor ve işaretçi saklanmıyor.
                .map(|ptr| unsafe { (*(ptr as *const SlotA)).0 });

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 30.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.6),
                intensity: 2.5,
                ..Default::default()
            });

            gizmo::gizmo_log!(
                Info,
                "kayıtlı bileşen türü: {} -> {} -> {} · Mut<Frozen> kurulabildi: {} · Frozen artık {} · ham okuma {:?}",
                report.registered_start,
                report.registered_after_one,
                report.registered_after_two,
                report.frozen_mutable,
                report.frozen_value,
                report.raw_read
            );
            scene.world.insert_resource(report);
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 8.0), Vec3::ZERO);
        })
        .add_update_system(noop.in_phase(Phase::Update))
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            // 3. `extraction`: çıkarma adımı YOK. Render kancası oyun bileşenlerini doğrudan
            //    okuyor, çünkü tek dünya var.
            let seen = world.query::<&Frozen>().map(|q| q.iter().count()).unwrap_or(0);
            if let Some(mut report) = world.get_resource_mut::<AbsentReport>() {
                report.seen_from_render = seen;
            }
            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<AbsentReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("absent".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(500.0);
                        ui.heading("Karşılığı olmayan dört API");
                        ui.separator();
                        ui.label("1. değişmez bileşen — YOK");
                        ui.monospace(format!(
                            "   Mut<Frozen> kuruldu: {} · Frozen artık {}",
                            r.frozen_mutable, r.frozen_value
                        ));
                        ui.label("   (\"değişmez\" olması gereken bileşen kancasız yazıldı)");
                        ui.separator();
                        ui.label("2. çalışma anında bileşen tanımı — YOK");
                        ui.monospace(format!(
                            "   kayıtlı tür: {} -> {} -> {}",
                            r.registered_start, r.registered_after_one, r.registered_after_two
                        ));
                        ui.monospace(format!("   ham okuma (TypeId ile): {:?}", r.raw_read));
                        ui.label("   erişim dinamik, TANIM değil.");
                        ui.separator();
                        ui.label("3. render dünyasına çıkarma — YOK (tek dünya)");
                        ui.monospace(format!(
                            "   set_render oyun bileşenlerini gördü: {}",
                            r.seen_from_render
                        ));
                        ui.separator();
                        ui.label("4. sistem sıcak yaması — YOK");
                        ui.label("   ama shader'lar sıcak yükleniyor (demo/assets/shaders).");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Çizelgenin boş kalmaması için — ölçümler kurulumda ve render kancasında yapılıyor.
fn noop(mut report: ResMut<AbsentReport>) {
    let _ = &mut *report;
}

/// `World`'ün bu demoda kullanılan yüzeyi tek yerde dursun diye.
#[allow(dead_code)]
fn _world_surface_note(_: &World) {}
