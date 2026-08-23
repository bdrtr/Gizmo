//! # Bevy'nin `ecs/entity_disabling` örneğinin Gizmo Engine portu
//!
//! Bir varlığı **yok etmeden devre dışı bırakmak**: yok sayılsın ama verisi dursun, sonra geri
//! açılabilsin. Bevy'de bu `Disabled` bileşeni, ve işin püf noktası şu — motor
//! `DefaultQueryFilters` ile *her* sorguya sessizce `Without<Disabled>` ekliyor, yani devre dışı
//! varlık hiçbir sıradan sorguda görünmüyor.
//!
//! ## Motorda yok, ve eksik olan asıl parça filtre
//!
//! | Bevy | Gizmo |
//! |------|-------|
//! | `Disabled` bileşeni | **yok** (elle bir işaret bileşeni yazılır) |
//! | `DefaultQueryFilters` — her sorguya örtük filtre | **yok** |
//! | `Allows<Disabled>` ile filtreyi devre dışı bırakma | **yok** |
//! | `world.entity_mut(e).insert(Disabled)` | `add_component` ile aynı |
//!
//! İşaret bileşenini yazmak kolay. **Örtük filtre yazılamıyor** — ve asıl iş orada. Bevy'de
//! `Disabled` takınca varlık her sorgudan kendiliğinden düşüyor; Gizmo'da her sorguya
//! `Without<Disabled>` eklemeyi *siz* hatırlamak zorundasınız, ve unutulan her sorgu devre dışı
//! varlığı işlemeye devam ediyor.
//!
//! ## Ölçüldü
//!
//! Dokuz küp, ortadaki üçü 60. karede devre dışı bırakılıyor. İki sistem aynı işi yapıyor; biri
//! `Without<Disabled>` yazmayı hatırlıyor, öteki unutuyor. Ölçüldü (2026-08-23):
//!
//! | kare | filtreyi hatırlayan | filtreyi unutan | unutanın fazladan işlemesi |
//! |------|---------------------|-----------------|----------------------------|
//! | 60 | **6** | **9** | 3 |
//! | 120 | 6 | 9 | 183 |
//! | 180 | 6 | 9 | 363 |
//! | 240 | 6 | 9 | **543** |
//!
//! İşaret bileşeni işini görüyor — hatırlayan sistem doğru sayıyı görüyor. Ama unutan sistem
//! **her karede üç varlığı fazladan işliyor**, ve 180 karede 543 fazladan işleme birikiyor.
//!
//! Bevy'de bu hata **yazılamıyor**: `DefaultQueryFilters` filtreyi motorun içinden her sorguya
//! ekliyor, ve devre dışı varlığı görmek isteyen `Allows<Disabled>` demek zorunda. Yani Bevy'de
//! varsayılan güvenli, istisna açık; Gizmo'da varsayılan sızdırıyor, güvenlik açık.
//!
//! Eksik olan parça bileşen değil — **örtük filtre**. Ve o filtre kullanıcı kodunda yazılamıyor,
//! çünkü sorgu türleri mühürlü (bkz. `bevy_ecs_extension_points`).
//!
//! ## Kontroller
//!   * **Z** — ortadaki üç küpü devre dışı bırak / geri aç
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query, With, Without};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Bevy'nin `Disabled`'ının elle karşılığı. Bileşen aynı; eksik olan **örtük filtre**.
#[derive(Clone, Copy)]
struct Disabled;
gizmo::core::impl_component!(Disabled);

/// Dönen küp.
#[derive(Clone, Copy)]
struct Spinner(f32);
gizmo::core::impl_component!(Spinner);

/// Ölçüm defteri.
#[derive(Default, Clone, Copy)]
struct DisableReport {
    frame: u32,
    disabled_now: bool,
    /// Filtreyi HATIRLAYAN sorgunun gördüğü varlık sayısı.
    filtered: usize,
    /// Filtreyi UNUTAN sorgunun gördüğü varlık sayısı.
    unfiltered: usize,
    /// Devre dışı olanların kaç kez yine de işlendiği — Bevy'de bu sayı 0 olurdu.
    leaked_updates: u64,
}
gizmo::core::impl_component!(DisableReport);

const TOTAL: usize = 9;
const DISABLE_FROM: usize = 3;
const DISABLE_TO: usize = 6;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Entity Disabling", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            for i in 0..TOTAL {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new((i as f32 - 4.0) * 1.25, 0.0, 0.0))
                        .with_scale(Vec3::splat(0.5)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.80, 0.60, 0.30, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Spinner(i as f32 * 0.3),
                ));
            }
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
            scene.world.insert_resource(DisableReport::default());
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 9.5), Vec3::ZERO);
        })
        // Devre dışı bırakma `set_update`'te: sorgu ham id veriyor ve onu `Entity`'ye güvenle
        // çevirmek `World::entity(id)` istiyor — çizelgeye kayıtlı bir sistem `&World` alamıyor.
        .set_update(|world, _state, _dt, input| toggle(world, input))
        // Filtreyi HATIRLAYAN sistem: devre dışı olanları döndürmüyor.
        .add_update_system(spin_filtered.in_phase(Phase::Update).after("toggle"))
        // Filtreyi UNUTAN sistem: aynı işi yapıyor ama devre dışı olanları da sayıyor.
        .add_update_system(count_unfiltered.in_phase(Phase::Update).after("toggle"))
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<DisableReport>().map(|r| *r) else {
                return;
            };
            gizmo::egui::Area::new("dis".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(480.0);
                        ui.heading("Varlığı devre dışı bırakmak");
                        ui.label(format!(
                            "{} küp · ortadaki {} tanesi: {}",
                            TOTAL,
                            DISABLE_TO - DISABLE_FROM,
                            if r.disabled_now { "DEVRE DIŞI" } else { "açık" }
                        ));
                        ui.separator();
                        ui.monospace(format!("Without<Disabled> hatırlayan: {} varlık", r.filtered));
                        ui.monospace(format!("filtreyi unutan            : {} varlık", r.unfiltered));
                        ui.separator();
                        if r.leaked_updates > 0 {
                            ui.colored_label(
                                gizmo::egui::Color32::from_rgb(230, 120, 80),
                                format!("unutan sistem {} kez fazladan işledi", r.leaked_updates),
                            );
                        }
                        ui.separator();
                        ui.label("Bevy'de DefaultQueryFilters her sorguya Without<Disabled>");
                        ui.label("ekliyor — filtreyi unutmak MÜMKÜN DEĞİL.");
                        ui.label("Gizmo'da örtük filtre yok: her sorguda siz yazacaksınız.");
                        ui.separator();
                        ui.label("Z — devre dışı bırak / geri aç");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// **D**: ortadaki küplere `Disabled` takar / söker.
fn toggle(world: &mut gizmo::core::World, input: &Input) {
    let Some(mut report) = world.get_resource_mut::<DisableReport>() else {
        return;
    };
    report.frame += 1;
    let frame = report.frame;
    let want = input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyZ as u32)
        || (frame == 60 && std::env::var("GIZMO_DISABLE_SELFTEST").is_ok());
    if !want {
        return;
    }
    let now = !report.disabled_now;
    report.disabled_now = now;
    drop(report);

    let mut ids: Vec<u32> = world
        .query::<&Spinner>()
        .map(|q| q.iter().map(|(id, _)| id).collect())
        .unwrap_or_default();
    ids.sort_unstable();
    for (index, id) in ids.iter().enumerate() {
        if !(DISABLE_FROM..DISABLE_TO).contains(&index) {
            continue;
        }
        // Nesil kontrollü: uydurma yok.
        let Some(entity) = world.entity(*id) else { continue };
        if now {
            world.add_component(entity, Disabled);
        } else {
            world.remove_component::<Disabled>(entity);
        }
    }
}

/// Filtreyi **hatırlayan** sistem.
fn spin_filtered(
    mut spinners: Query<(Mut<Transform>, &Spinner, With<Spinner>, Without<Disabled>)>,
    mut report: ResMut<DisableReport>,
    time: Res<Time>,
) {
    let dt = time.dt();
    let mut seen = 0usize;
    for (_entity, (mut transform, spinner, _, _)) in spinners.iter_mut() {
        seen += 1;
        // Her küpün kendi hızı — `Spinner`'ın yükü burada iş görüyor.
        transform.rotation *= Quat::from_rotation_y(dt * (0.6 + spinner.0));
    }
    report.filtered = seen;
}

/// Filtreyi **unutan** sistem — Bevy'de bu hata yazılamıyor.
fn count_unfiltered(
    spinners: Query<(&Spinner, With<Spinner>)>,
    disabled: Query<(&Disabled, With<Disabled>)>,
    mut report: ResMut<DisableReport>,
) {
    report.unfiltered = spinners.iter().count();
    // Devre dışı olan her varlık, bu sistemin her koşusunda "fazladan işlenmiş" sayılıyor.
    report.leaked_updates += disabled.iter().count() as u64;

    if std::env::var("GIZMO_DISABLE_SELFTEST").is_ok() && report.frame.is_multiple_of(60) {
        gizmo::gizmo_log!(
            Info,
            "kare {:>4} · devre dışı: {} · filtreli {} · filtresiz {} · sızan işleme {}",
            report.frame,
            report.disabled_now,
            report.filtered,
            report.unfiltered,
            report.leaked_updates
        );
    }
}
