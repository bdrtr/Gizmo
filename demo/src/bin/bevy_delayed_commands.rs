//! # Bevy'nin `ecs/delayed_commands` örneğinin Gizmo Engine portu
//!
//! **Bir süre sonra etkisini gösterecek komut** göndermek. Bevy'nin örneği tıklanabilir bir ızgara
//! kuruyor ve tıklamadan sonra gecikmeli "dalgacıklar" doğuruyor.
//!
//! ## Motorda genel bir gecikmeli komut yok, bir özel hâli var
//!
//! | Bevy | Gizmo |
//! |------|-------|
//! | `commands.spawn_task(async move { … })` / zamanlayıcı + gözlemci | genel bir gecikme mekanizması **yok** |
//! | herhangi bir komutu geciktirmek | **yok** |
//! | *silmeyi* geciktirmek | var — [`DespawnAfter`] |
//! | `Timer` / `TimerMode::Repeating` | **yok** — `Time` var, sayacı siz tutuyorsunuz |
//!
//! Yani motorun sunduğu tek gecikmeli komut "şu varlığı `n` saniye sonra sil". Bunun dışındaki
//! her gecikme — doğur, bileşen tak, bir kaynağı değiştir — elde kalıyor.
//!
//! ## Karşılığı: bir kuyruk bileşeni
//!
//! Demo genel hâli kuruyor: her biri kendi geri sayımını taşıyan **bekleyen komutlar**, ve süresi
//! dolanı uygulayan tek bir sistem. Motorun `DespawnAfter`'ının yaptığı işin genelleştirilmişi —
//! ve zaten `LifetimeSystem` de tam olarak böyle yazılmış.
//!
//! Farkı görmek için demo iki yolu **yan yana** koşturuyor: sol sütun motorun `DespawnAfter`'ıyla,
//! sağ sütun elle kuyrukla. İkisinin ömrü aynı olmalı.
//!
//! ## Ölçüldü
//!
//! Aynı ömürler (0,4 · 0,8 · 1,2 · 1,6 · 2,0 sn), solda `DespawnAfter`, sağda elle kuyruk.
//! Ölçüldü (2026-08-23, `GIZMO_DELAY_SELFTEST=1`, 60 karede bir örnek):
//!
//! | kare | süre | motor tarafı | elle taraf |
//! |------|------|--------------|------------|
//! | 60 | 0,31 sn | 5 | 5 |
//! | 120 | 0,57 sn | 4 | 4 |
//! | 180 | 0,83 sn | 3 | 3 |
//! | 240 | 1,09 sn | 8 | 8 |
//! | 300 | 1,36 sn | 7 | 7 |
//! | 360 | 1,62 sn | 5 | 5 |
//! | 420 | 1,88 sn | 5 | 5 |
//! | 480 | 2,14 sn | 8 | 8 |
//! | 540 | 2,41 sn | 7 | 7 |
//! | 600 | 2,67 sn | 6 | 6 |
//! | 660 | 2,94 sn | 5 | 5 |
//!
//! On bir örneklemenin **on birinde de aynı** — elle kurulan kuyruk motorun `DespawnAfter`'ını
//! birebir taklit ediyor. Yani gecikmeli komutun *silme* hâli için motorun yolunu kullanmak bir
//! kolaylık; başka her iş için aynı şeyi yazmak zorundasınız, ve zor değil.
//!
//! ### Ölçüm notu: `DespawnAfter` eklentisiz atıl
//!
//! İlk koşuda motor tarafı hiç ölmedi — 5 → 10 → 15 diye büyüdü. Sebep motorun bir kusuru değil:
//! **`DespawnAfter`'ı işleyen sistem `LifetimePlugin` ile geliyor ve o eklenti varsayılan değil.**
//! Eklenti yokken bileşen takılıyor, kimse okumuyor, varlık ölmüyor, ve **hiçbir uyarı çıkmıyor**.
//!
//! Bu, bu ailedeki en pahalı tuzak: yan yana bir karşılaştırma olmasaydı "motorun `DespawnAfter`'ı
//! çalışmıyor" diye yanlış bir sonuç yazmış olurdum. Ölçüm, ölçenin kendi hatasını da yakalıyor.
//!
//! ## Kontroller
//!   * **Boşluk** — yeni bir dalga başlat
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Elle kurulan gecikmeli komut: süre dolunca `action` uygulanacak.
#[derive(Clone, Copy)]
struct Delayed {
    remaining: f32,
    action: Action,
}
gizmo::core::impl_component!(Delayed);

/// Geciktirilebilen işler. Bevy'de bunların hepsi sıradan bir `Command`; burada bir enum, çünkü
/// bileşen `Clone` olmak zorunda ve `Box<dyn FnOnce>` değil.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Varlığı sil — motorun `DespawnAfter`'ının yaptığı iş.
    Despawn,
    /// Varlığı büyüt — motorda karşılığı OLMAYAN türden bir gecikmeli komut.
    Grow,
}

/// Motorun kendi yoluyla ölen küpleri işaretler.
#[derive(Clone, Copy)]
struct EngineSide;
gizmo::core::impl_component!(EngineSide);
/// Elle kurulan kuyrukla ölen küpleri işaretler.
#[derive(Clone, Copy)]
struct ManualSide;
gizmo::core::impl_component!(ManualSide);

/// Ölçüm defteri.
#[derive(Default, Clone, Copy)]
struct DelayReport {
    frame: u32,
    elapsed: f32,
    /// Yaşayan küp sayıları — iki yolun aynı anda boşalması bekleniyor.
    engine_alive: usize,
    manual_alive: usize,
    /// Uygulanan gecikmeli komut sayıları.
    applied_despawn: u32,
    applied_grow: u32,
    waves: u32,
}
gizmo::core::impl_component!(DelayReport);

/// Bir dalgada kaç küp, ve ömürleri.
const PER_WAVE: usize = 5;
const LIFETIMES: [f32; PER_WAVE] = [0.4, 0.8, 1.2, 1.6, 2.0];
/// Büyüme komutunun gecikmesi.
const GROW_DELAY: f32 = 1.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Delayed Commands", 1280, 720)
        // `DespawnAfter` BU EKLENTİ OLMADAN ATIL. Bileşen takılır, kimse okumaz, varlık ölmez —
        // ve hiçbir uyarı çıkmaz. İlk ölçümde tam olarak bu oldu (bkz. başlıktaki ölçüm notu).
        .add_plugin(gizmo::prelude::LifetimePlugin)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            scene.world.insert_resource(Kit {
                cube: AssetManager::create_cube(device),
                white: white.clone(),
            });
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.2, 0.0)),
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
            scene.world.insert_resource(DelayReport::default());
            scene.spawn_camera(state, Vec3::new(0.0, 3.0, 9.0), Vec3::ZERO);
        })
        .set_update(|world, state, dt, input| {
            // Bkz. `demo::simple_scene_update` — `set_update` basit sahnenin kancasını eziyor.
            demo::simple_scene_update(world, state, dt, input);
            tick(world, dt, input);
        })
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<DelayReport>().map(|r| *r) else {
                return;
            };
            gizmo::egui::Area::new("delay".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(470.0);
                        ui.heading("Gecikmeli komutlar");
                        ui.label(format!("dalga {} · {:.2} sn", r.waves, r.elapsed));
                        ui.separator();
                        ui.monospace(format!("sol  (DespawnAfter) : {} yaşayan", r.engine_alive));
                        ui.monospace(format!("sağ  (elle kuyruk)  : {} yaşayan", r.manual_alive));
                        if r.engine_alive == r.manual_alive {
                            ui.label("  -> iki yol aynı sayıda: kuyruk motoru birebir taklit ediyor");
                        } else {
                            ui.colored_label(
                                gizmo::egui::Color32::from_rgb(230, 160, 80),
                                "  -> AYRIŞTILAR",
                            );
                        }
                        ui.separator();
                        ui.label(format!("uygulanan sil : {}", r.applied_despawn));
                        ui.label(format!("uygulanan büyüt: {}", r.applied_grow));
                        ui.separator();
                        ui.label("motorda genel gecikmeli komut YOK.");
                        ui.label("DespawnAfter yalnız SİLMEYİ geciktiriyor;");
                        ui.label("büyütmek gibi bir iş için kuyruğu siz kuruyorsunuz.");
                        ui.separator();
                        ui.label("boşluk — yeni dalga");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Sahne varlıkları.
#[derive(Clone)]
struct Kit {
    cube: Mesh,
    white: std::sync::Arc<gizmo::wgpu::BindGroup>,
}
gizmo::core::impl_component!(Kit);

/// Kare işi: dalga başlat, iki yolu da ilerlet, sayaçları güncelle.
fn tick(world: &mut World, dt: f32, input: &Input) {
    let Some(mut report) = world.get_resource_mut::<DelayReport>() else {
        return;
    };
    report.frame += 1;
    report.elapsed += dt;
    let elapsed = report.elapsed;
    let frame = report.frame;
    let want_wave = input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32)
        || frame == 1
        || (std::env::var("GIZMO_DELAY_SELFTEST").is_ok() && frame.is_multiple_of(240));
    drop(report);

    if want_wave {
        spawn_wave(world, elapsed);
    }

    // Elle kurulan kuyruk: süresi dolan komutları uygula. Motorun `LifetimeSystem`'i de aynı
    // biçimde yazılmış — sorgu ham id veriyor, `world.entity(id)` ile `Entity`'ye çevriliyor.
    let mut due_ids: Vec<(u32, Action)> = Vec::new();
    if let Some(mut q) = world.query_mut::<gizmo::core::query::Mut<Delayed>>() {
        for (id, mut d) in q.iter_mut() {
            d.remaining -= dt;
            if d.remaining <= 0.0 {
                due_ids.push((id, d.action));
            }
        }
    }
    // Ham id -> `Entity`, nesil kontrollü. Sorgu `Entity` vermiyor, ve `Entity::new(id, 0)`
    // uydurmak geri dönüştürülmüş bir id'de yanlış tutamak üretiyor.
    let due: Vec<(gizmo::core::Entity, Action)> = due_ids
        .into_iter()
        .filter_map(|(id, a)| world.entity(id).map(|e| (e, a)))
        .collect();

    let mut applied_despawn = 0u32;
    let mut applied_grow = 0u32;
    for (entity, action) in due {
        match action {
            Action::Despawn => {
                world.despawn(entity);
                applied_despawn += 1;
            }
            Action::Grow => {
                if let Some(mut q) = world.query_mut::<gizmo::core::query::Mut<Transform>>() {
                    for (id, mut t) in q.iter_mut() {
                        if id == entity.id() {
                            t.scale *= 1.6;
                        }
                    }
                }
                world.remove_component::<Delayed>(entity);
                applied_grow += 1;
            }
        }
    }

    let engine_alive = world
        .query::<&EngineSide>()
        .map(|q| q.iter().count())
        .unwrap_or(0);
    let manual_alive = world
        .query::<&ManualSide>()
        .map(|q| q.iter().count())
        .unwrap_or(0);

    let Some(mut report) = world.get_resource_mut::<DelayReport>() else {
        return;
    };
    report.applied_despawn += applied_despawn;
    report.applied_grow += applied_grow;
    report.engine_alive = engine_alive;
    report.manual_alive = manual_alive;

    if std::env::var("GIZMO_DELAY_SELFTEST").is_ok() && report.frame.is_multiple_of(60) {
        gizmo::gizmo_log!(
            Info,
            "kare {:>4} {:.2} sn · motor {} · elle {} · sil {} · büyüt {} · dalga {}",
            report.frame,
            report.elapsed,
            report.engine_alive,
            report.manual_alive,
            report.applied_despawn,
            report.applied_grow,
            report.waves
        );
    }
}

/// Bir dalga: solda motorun `DespawnAfter`'ı, sağda elle kuyruk, aynı ömürlerle.
fn spawn_wave(world: &mut World, _elapsed: f32) {
    let Some(kit) = world.get_resource::<Kit>().map(|k| k.clone()) else {
        return;
    };
    for (i, life) in LIFETIMES.iter().enumerate() {
        let z = (i as f32 - (PER_WAVE as f32 - 1.0) * 0.5) * 1.3;

        // Sol: motorun yolu. Tek bileşen, gerisini `LifetimeSystem` yapıyor.
        world.spawn_bundle((
            Transform::new(Vec3::new(-2.2, 0.0, z)).with_scale(Vec3::splat(0.45)),
            GlobalTransform::default(),
            kit.cube.clone(),
            Material::new(kit.white.clone()).with_pbr(Vec4::new(0.30, 0.70, 0.95, 1.0), 0.5, 0.0),
            MeshRenderer::new(),
            gizmo::prelude::DespawnAfter::secs(*life),
            EngineSide,
        ));

        // Sağ: elle kuyruk. Aynı ömür, ama komutu bu demo uyguluyor.
        world.spawn_bundle((
            Transform::new(Vec3::new(2.2, 0.0, z)).with_scale(Vec3::splat(0.45)),
            GlobalTransform::default(),
            kit.cube.clone(),
            Material::new(kit.white.clone()).with_pbr(Vec4::new(0.95, 0.60, 0.30, 1.0), 0.5, 0.0),
            MeshRenderer::new(),
            Delayed {
                remaining: *life,
                action: Action::Despawn,
            },
            ManualSide,
        ));
    }

    // Ortada: motorda karşılığı OLMAYAN gecikmeli komut — "bir saniye sonra büyü".
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 0.0, 0.0)).with_scale(Vec3::splat(0.35)),
        GlobalTransform::default(),
        kit.cube.clone(),
        Material::new(kit.white).with_pbr(Vec4::new(0.55, 0.90, 0.45, 1.0), 0.4, 0.0),
        MeshRenderer::new(),
        Delayed {
            remaining: GROW_DELAY,
            action: Action::Grow,
        },
        gizmo::prelude::DespawnAfter::secs(LIFETIMES[PER_WAVE - 1] + 0.5),
    ));

    if let Some(mut report) = world.get_resource_mut::<DelayReport>() {
        report.waves += 1;
    }
}
