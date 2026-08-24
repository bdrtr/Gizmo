//! # Bileşen kaldırmayı yakalamak
//!
//! Bir bileşen bir varlıktan koptuğunda haberdar olmak: bileşen kalkınca bir geri çağrının
//! çalışması ve demoda küpü camgöbeğine boyaması.
//!
//! ## Gözlemci de var, kanca da — ve bu demo kancayı seçiyor
//!
//! [`World::add_observer`] 2026-08-24'ten beri üç evrenin hepsini alıyor:
//! `On<Insert, T>`, `On<Replace, T>`, `On<Remove, T>` (bkz. `observers` demosu). Bu demo yine
//! de ham [`World::register_on_remove`] kancasını kullanıyor, çünkü ölçtüğü şey kancaya
//! verilen **`&mut World`**: geri çağrı içeriden malzemeyi boyuyor ve deftere yazıyor.
//! Gözlemci `On`'dan başka bir şey almaz, yani bu demoyu gözlemciyle yazmak mümkün değil —
//! ikisi aynı listenin iki ucu ve fark tam olarak burada görünüyor.
//!
//! ## Asıl ölçülen şey: hangi yol kancayı tetikliyor, hangisi sessiz
//!
//! Kanca "bileşen her koptuğunda çalışır" değil — motorun `hooks.rs` belgesi hangi yolların
//! sessiz kaldığını tek tek sayıyor, ve bu demo o listeyi **çalıştırıp doğruluyor**. Beş küp,
//! beş yol; iki saniye sonra her biri kendi yoluyla bileşenini kaybediyor, kanca çalışırsa küp
//! camgöbeğine dönüyor (`srgb(0.5, 1, 1)`) ve HUD "çalıştı" yazıyor.
//!
//! Ölçüldü (`GIZMO_REMOVAL_SELFTEST=1`, 2026-08-24):
//!
//! | yol                       | kanca   | 2026-08-23'te |
//! |---------------------------|---------|---------------|
//! | `remove_component::<T>`   | çalıştı | çalıştı |
//! | `remove_batch::<T>`       | çalıştı | çalıştı |
//! | `remove_bundle` (Table)   | çalıştı | **sessizdi** |
//! | `remove_bundle` (Sparse)  | çalıştı | çalıştı |
//! | `despawn`                 | çalıştı | çalıştı |
//!
//! **Üçüncü satır bu demonun bulduğu kusurdu ve artık kapalı.** `remove_bundle` demetin
//! yalnız seyrek (SparseSet) bileşenleri için tetikliyordu; Table depolamalı olanlar
//! `move_entity_to` ile sessizce kopuyordu, yani aynı bileşen iki farklı yolla kaldırılınca
//! iki farklı cevap veriyordu. 2026-08-24'te `On<Remove, T>`'ye dağıtım yolu açılınca bu
//! içerideki bir tuhaflık olmaktan çıkıp bozulmuş bir söz hâline geldi ve **aynı gün, aynı
//! değişiklikte** düzeltildi — yani o tarih sessizliğin başladığı değil, bittiği gün.
//!
//! Beş yol artık **beş** kanca çağrısı üretiyor, ve bu sayı da HUD'da. Demoda iki depolama
//! türü de duruyor: fark kapandı diye ayrımın kendisi kaybolmadı — `remove_bundle` hâlâ
//! seyrek olanları ayrı bir döngüde, Table olanları göçün ardından bildiriyor.
//!
//! ## Kancanın kendisi hakkında bilinmesi gerekenler
//!
//!   * Kanca `&mut World` alır — yani içinden doğurabilir, yok edebilir, kaynak yazabilir. Bu
//!     demo tam olarak bunu yapıyor: malzemeyi boyayıp denetim defterine işliyor.
//!   * Bir kancanın çalıştığı sırada o bileşen türünün kanca listesi dünyadan **geçici olarak
//!     ayrılır**; içeriden aynı türe kanca eklemek o an uçan olay için görünmez.
//!   * Kancalar belirleyici benzetim sözleşmesinin parçası **değil**: bir despawn sırasında
//!     bileşen türleri arası tetiklenme sırası bir `HashMap`'ten gelir, yani keyfîdir.
//!
//! ## Kontroller
//!   * **R** — sahneyi baştan kur (yolları yeniden çalıştır)
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::query::Mut;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Kaldırılacak olan bileşen. **Table** depolamalı (varsayılan).
#[derive(Clone, Copy)]
struct MyComponent;
gizmo::core::impl_component!(MyComponent);

/// Aynı işaret, ama **seyrek** depolamalı. `remove_bundle` farkı yalnız bunun üzerinden görülür.
#[derive(Clone, Copy)]
struct MySparseComponent;
gizmo::core::impl_component!(MySparseComponent; gizmo::core::component::StorageType::SparseSet);

/// Demetin ikinci üyesi — `remove_bundle`'a kaldıracak bir demet vermek için.
#[derive(Clone, Copy)]
struct Companion;
gizmo::core::impl_component!(Companion);

/// Bir kaldırma yolunun kimliği ve sonucu.
#[derive(Clone)]
struct Probe {
    entity: u32,
    path: &'static str,
    /// Kanca bu varlık için çalıştı mı.
    fired: bool,
    /// Yol uygulandı mı.
    applied: bool,
}

/// Beş yolun denetim defteri.
#[derive(Default, Clone)]
struct RemovalAudit {
    probes: Vec<Probe>,
    elapsed: f32,
    /// Kanca toplam kaç kez çalıştı — despawn edilen varlık için de sayılır.
    hook_calls: u32,
    restart: bool,
}
gizmo::core::impl_component!(RemovalAudit);

/// Kanca çalışınca küpün aldığı renk — `srgb(0.5, 1.0, 1.0)`.
const REACTED: Vec4 = Vec4::new(0.5, 1.0, 1.0, 1.0);
/// Başlangıç rengi.
const RESTING: Vec4 = Vec4::new(0.55, 0.35, 0.30, 1.0);
/// Yolların uygulandığı an.
const TRIGGER_AT: f32 = 2.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Removal Detection", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let cube = AssetManager::create_cube(&scene.renderer.device);

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.55, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 16.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.7) * Quat::from_rotation_x(-0.7),
                intensity: 2.2,
                ..Default::default()
            });

            // Kancanın kendisi: kaldırmaya tepki veren geri çağrı. `&mut World`
            // aldığı için hem boyayabiliyor hem deftere yazabiliyor.
            scene
                .world
                .register_on_remove::<MyComponent>(Box::new(react_on_removal));
            scene
                .world
                .register_on_remove::<MySparseComponent>(Box::new(react_on_removal));

            scene.world.insert_resource(RemovalAudit::default());
            let template = Material::new(white).with_pbr(RESTING, 0.5, 0.0);
            build_probes(scene.world, &cube, &template);
            scene.spawn_camera(state, Vec3::new(0.0, 4.0, 12.5), Vec3::new(0.0, 0.0, 0.0));
        })
        .add_update_system(watch_restart.in_phase(Phase::Update))
        // Kaldırma yolları `&mut World` istiyor; sistemler onu almadığı için burada.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            apply_removals(world);

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(audit) = world.get_resource::<RemovalAudit>().map(|a| a.clone()) else {
                return;
            };
            gizmo::egui::Area::new("removal".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.set_min_width(340.0);
                    ui.heading("Kaldırma tespiti");
                    if audit.elapsed < TRIGGER_AT {
                        ui.label(format!(
                            "yollar {:.1} s sonra uygulanacak",
                            TRIGGER_AT - audit.elapsed
                        ));
                    } else {
                        ui.label("yollar uygulandı:");
                    }
                    ui.separator();
                    for probe in &audit.probes {
                        ui.label(format!(
                            "{:24}{}",
                            probe.path,
                            match (probe.applied, probe.fired) {
                                (false, _) => "bekliyor",
                                (true, true) => "ÇALIŞTI",
                                (true, false) => "SESSİZ",
                            }
                        ));
                    }
                    ui.separator();
                    ui.label(format!("kanca toplam {} kez çağrıldı", audit.hook_calls));
                    ui.label("R — baştan kur");
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Kaldırmaya tepki: kanca çalışınca küpü camgöbeğine boyar ve deftere işler.
///
/// Kancanın imzası `(&mut World, Entity)` — kaldırma olayının taşıdığı varlık
/// burada ikinci parametre. Olayın *hangi* bileşen için tetiklendiği bilgisi yok: kanca zaten
/// bileşen türüne kayıtlı olduğu için o bilgi kaydın kendisinde.
fn react_on_removal(world: &mut gizmo::core::World, entity: gizmo::core::entity::Entity) {
    let id = entity.id();

    if let Some(mut audit) = world.get_resource_mut::<RemovalAudit>() {
        audit.hook_calls += 1;
        if let Some(probe) = audit.probes.iter_mut().find(|p| p.entity == id) {
            probe.fired = true;
        }
    }

    // Boyama: despawn edilen varlıkta bu sessizce başarısız olur, ki doğrusu da budur.
    if let Some(mut query) = world.query_mut::<Mut<Material>>() {
        if let Some(mut material) = query.get_mut_entity(entity) {
            material.albedo = REACTED;
        }
    }
}

/// Beş küpü ve beş yolu kurar.
fn build_probes(world: &mut gizmo::core::World, cube: &Mesh, template: &Material) {
    // Yolların adları HUD'da göründüğü gibi; sıra ekrandaki soldan sağa sırayla aynı.
    let paths = [
        "remove_component",
        "remove_batch",
        "remove_bundle (Table)",
        "remove_bundle (Sparse)",
        "despawn",
    ];

    let mut probes = Vec::new();
    for (i, path) in paths.into_iter().enumerate() {
        let x = (i as f32 - 2.0) * 2.6;
        let entity = world.spawn_bundle((
            Transform::new(Vec3::new(x, 0.0, 0.0)).with_scale(Vec3::splat(0.55)),
            GlobalTransform::default(),
            cube.clone(),
            {
                let mut material = template.clone();
                material.albedo = RESTING;
                material
            },
            MeshRenderer::new(),
            Companion,
        ));
        // Dördüncü küp seyrek bileşeni taşıyor, ötekiler Table olanı.
        if i == 3 {
            world.add_component(entity, MySparseComponent);
        } else {
            world.add_component(entity, MyComponent);
        }
        probes.push(Probe {
            entity: entity.id(),
            path,
            fired: false,
            applied: false,
        });
    }

    if let Some(mut audit) = world.get_resource_mut::<RemovalAudit>() {
        audit.probes = probes;
        audit.elapsed = 0.0;
        audit.hook_calls = 0;
        audit.restart = false;
    }
}

/// İki saniye sonra beş yolu bir kez uygular.
fn apply_removals(world: &mut gizmo::core::World) {
    let restart = world
        .get_resource::<RemovalAudit>()
        .map(|a| a.restart)
        .unwrap_or(false);
    if restart {
        rebuild(world);
        return;
    }

    let Some((elapsed, already)) = world
        .get_resource::<RemovalAudit>()
        .map(|a| (a.elapsed, a.probes.iter().all(|p| p.applied)))
    else {
        return;
    };
    if elapsed < TRIGGER_AT || already {
        return;
    }

    let ids: Vec<u32> = world
        .get_resource::<RemovalAudit>()
        .map(|a| a.probes.iter().map(|p| p.entity).collect())
        .unwrap_or_default();
    let entities: Vec<Option<gizmo::core::entity::Entity>> =
        ids.iter().map(|id| world.get_entity(*id)).collect();

    // 1) Tek bileşen — kanca çalışır.
    if let Some(Some(entity)) = entities.first() {
        world.remove_component::<MyComponent>(*entity);
    }
    // 2) Toplu kaldırma — kanca çalışır.
    if let Some(Some(entity)) = entities.get(1) {
        world.remove_batch::<MyComponent>(&[*entity]);
    }
    // 3) Demet, Table depolamalı bileşenle — 2026-08-24'e kadar SESSİZDİ, artık kanca çalışır.
    if let Some(Some(entity)) = entities.get(2) {
        world.remove_bundle::<(MyComponent, Companion)>(*entity);
    }
    // 4) Aynı çağrı, seyrek depolamalı bileşenle — kanca çalışır.
    if let Some(Some(entity)) = entities.get(3) {
        world.remove_bundle::<(MySparseComponent, Companion)>(*entity);
    }
    // 5) Bütün varlığı yok et — kanca taşıdığı her bileşen türü için bir kez çalışır, ama
    //    boyayacak bir malzeme kalmaz.
    if let Some(Some(entity)) = entities.get(4) {
        world.despawn(*entity);
    }

    if let Some(mut audit) = world.get_resource_mut::<RemovalAudit>() {
        for probe in audit.probes.iter_mut() {
            probe.applied = true;
        }
    }
}

/// Sahneyi baştan kurar: eski küpleri siler, yenilerini doğurur.
fn rebuild(world: &mut gizmo::core::World) {
    let ids: Vec<u32> = world
        .get_resource::<RemovalAudit>()
        .map(|a| a.probes.iter().map(|p| p.entity).collect())
        .unwrap_or_default();
    for id in ids {
        if let Some(entity) = world.get_entity(id) {
            world.despawn(entity);
        }
    }

    // Mesh ve malzeme yeniden üretilmiyor: hayatta kalan bir varlıktan (zeminden) kopyalanıyor,
    // çünkü GPU kaynağı yaratmak cihaz ister ve burada cihaz yok.
    let template = world
        .query::<(&Mesh, &Material)>()
        .and_then(|q| q.iter().next().map(|(_, (m, mat))| (m.clone(), mat.clone())));
    let Some((mesh, material)) = template else {
        return;
    };
    build_probes(world, &mesh, &material);
}

/// **R** yeniden kurulum ister; ayrıca geçen süreyi biriktirir.
///
/// `GIZMO_REMOVAL_SELFTEST=1` ile: yollar uygulandıktan bir saniye sonra beş yolun sonucunu
/// günlüğe basar. Başlıktaki tablo bu çıktıdır — elle okunmuş bir HUD değil, yeniden
/// üretilebilir bir ölçüm; `remove_bundle`'ın Table yarısı 2026-08-24'te sustuğu için o
/// tablonun bir satırı zaten bir kez bayatladı.
fn watch_restart(mut audit: ResMut<RemovalAudit>, input: Res<Input>, time: Res<Time>) {
    let before = audit.elapsed;
    audit.elapsed += time.dt();
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyR as u32) {
        audit.restart = true;
    }

    let report_at = TRIGGER_AT + 1.0;
    if before < report_at && audit.elapsed >= report_at && std::env::var("GIZMO_REMOVAL_SELFTEST").is_ok() {
        for probe in &audit.probes {
            gizmo::gizmo_log!(
                Info,
                "{:26} uygulandı={} kanca={}",
                probe.path,
                probe.applied,
                probe.fired
            );
        }
        gizmo::gizmo_log!(Info, "toplam kanca çağrısı: {}", audit.hook_calls);
    }
}
