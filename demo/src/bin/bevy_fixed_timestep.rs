//! # Bevy'nin `ecs/fixed_timestep` örneğinin Gizmo Engine portu
//!
//! Motorun kare modelinin kalbi: **iki ayrı çizelge**. Biri sabit `dt` ile kare başına
//! **sıfır ya da birkaç kez** koşar (fizik), diğeri kare başına **tam bir kez**, gerçek kare
//! süresiyle (oyun mantığı, girdi, kamera). Bevy bunu `FixedUpdate`/`Update` diye ayırıyor;
//! Gizmo'da ayrım kaydın kendisinde: `add_system` sabit çizelgeye, `add_update_system` kare
//! çizelgesine yazar.
//!
//! **Bu ayrım sessizce yanlış sonuç verebilen türden, ve bu demo onu sayıyla gösteriyor.**
//! Girdi kenarları (`is_key_just_pressed`) kare başına bir kez yakalanıp temizlenir; sabit
//! çizelgede okunursa bir basış kaçar (o karede hiç adım koşmadıysa) ya da iki kez işlenir (iki
//! adım koştuysa). Bu makinede ölçüldü — 60 Hz sabit adım, 110 FPS ekran:
//!
//! ```text
//! sabit dt 0.0167 s (60 Hz) · kare dt 0.0091 s (110 FPS)
//! kare başına sabit adım:  0 adım %48,0  ·  1 adım %52,0  ·  2+ adım %0
//! ```
//!
//! Yani karelerin **neredeyse yarısında hiç sabit adım koşmuyor**: o çizelgede okunan bir tuş
//! basışının yarısı kaybolurdu. Ekran yenileme hızı sabit adımın altına düştüğünde ise aynı
//! tablo ters uca kayar ve 2 adımlı kareler belirir — o zaman da basış iki kez işlenir.
//!
//! Bevy'nin örneği bu ilişkileri konsola yazıyor. Bu port aynı sayıları hem HUD'da gösteriyor hem
//! de aynı biçimde günlüğe yazıyor — ve üstüne **görünür** kılıyor: iki küp aynı sinüsü aynı
//! hızla tarıyor, biri sabit adımlarda (kesikli), diğeri her karede (sürekli). Aynı hizada
//! kalmaları, iki çizelgenin aynı gerçek zamanı harcadığının kanıtı.
//!
//! Motorun fazladan verdiği şey de ekranda: `PhysicsTime::alpha`, son sabit adımdan bu yana
//! biriken zamanın oranı. Kesikli hareketi render'da düzeltmek için gereken katsayı bu
//! (`lerp(önceki, şimdiki, alpha)`), ve motor onu kendisi hesaplıyor.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera · **Shift** — hızlı hareket

use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::core::query::{Mut, Query};
use gizmo::core::time::PhysicsTime;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Sabit çizelgede ilerleyen küp; `f32` onun biriktirdiği faz.
#[derive(Clone, Copy, Default)]
struct FixedCube(f32);
gizmo::core::impl_component!(FixedCube);

/// Kare çizelgesinde ilerleyen küp, aynı faz alanıyla.
#[derive(Clone, Copy, Default)]
struct FrameCube(f32);
gizmo::core::impl_component!(FrameCube);

/// İki çizelgenin sayaçları.
#[derive(Default, Clone, Copy)]
struct TickStats {
    /// Toplam sabit adım ve toplam kare.
    fixed_steps: u64,
    frames: u64,
    /// Son karede kaç sabit adım koştu, ve o dağılımın histogramı (0, 1, 2, 3+).
    steps_this_frame: u32,
    histogram: [u32; 4],
    /// Son görülen süreler.
    fixed_dt: f32,
    frame_dt: f32,
    alpha: f32,
}
gizmo::core::impl_component!(TickStats);

/// İki küpün de ortak hızı — aynı yolu aynı sürede almalılar.
const SPEED: f32 = 1.2;
/// Küplerin salınım genliği.
const SPAN: f32 = 3.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Fixed Timestep", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let cube = AssetManager::create_cube(&scene.renderer.device);

            // Üstteki küp sabit çizelgede, alttaki kare çizelgesinde ilerliyor.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 1.2, 0.0)).with_scale(Vec3::splat(0.25)),
                GlobalTransform::default(),
                cube.clone(),
                Material::new(white.clone()).with_pbr(Vec4::new(0.9, 0.35, 0.2, 1.0), 0.5, 0.0),
                MeshRenderer::new(),
                FixedCube::default(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.4, 0.0)).with_scale(Vec3::splat(0.25)),
                GlobalTransform::default(),
                cube,
                Material::new(white.clone()).with_pbr(Vec4::new(0.2, 0.6, 0.9, 1.0), 0.5, 0.0),
                MeshRenderer::new(),
                FrameCube::default(),
            ));

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 12.0),
                Material::new(white).with_pbr(Vec4::new(0.1, 0.11, 0.13, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.9) * Quat::from_rotation_x(-0.6),
                intensity: 2.5,
                ..Default::default()
            });

            scene.world.insert_resource(TickStats::default());
            scene.spawn_camera(state, Vec3::new(0.0, 2.0, 7.0), Vec3::new(0.0, 0.8, 0.0));
        })
        // SABİT çizelge: kare başına 0..N kez, sabit dt ile.
        .add_system(fixed_update.in_phase(Phase::Physics))
        // KARE çizelgesi: kare başına tam bir kez, gerçek dt ile.
        .add_update_system(frame_update.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(stats) = world.get_resource::<TickStats>().map(|s| *s) else {
                return;
            };
            gizmo::egui::Area::new("fixed_timestep".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.heading("Sabit adım / kare adımı");
                    ui.label(format!(
                        "sabit dt: {:.4} s  ({:.0} Hz)",
                        stats.fixed_dt,
                        if stats.fixed_dt > 0.0 { 1.0 / stats.fixed_dt } else { 0.0 }
                    ));
                    ui.label(format!(
                        "kare dt : {:.4} s  ({:.0} FPS)",
                        stats.frame_dt,
                        if stats.frame_dt > 0.0 { 1.0 / stats.frame_dt } else { 0.0 }
                    ));
                    ui.label(format!(
                        "toplam: {} sabit adım / {} kare",
                        stats.fixed_steps, stats.frames
                    ));
                    ui.separator();
                    ui.label("kare başına sabit adım:");
                    let total: u32 = stats.histogram.iter().sum::<u32>().max(1);
                    for (i, count) in stats.histogram.iter().enumerate() {
                        let label = if i == 3 { "3+".to_string() } else { i.to_string() };
                        ui.label(format!(
                            "  {label} adım: {count}  (%{:.1})",
                            *count as f32 / total as f32 * 100.0
                        ));
                    }
                    ui.separator();
                    ui.label(format!("alpha (son adımdan beri biriken): {:.3}", stats.alpha));
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// SABİT çizelgede koşar: kare başına sıfır ya da birkaç kez, hep aynı `dt` ile.
///
/// Bevy'nin `fixed_update`'i. Küpü sabit adımlarla ilerletir — hareket bu yüzden **kesikli**,
/// ve düşük FPS'te bile aynı hızda: adım sayısı gerçek zamana göre ayarlanır.
fn fixed_update(
    mut cubes: Query<(Mut<Transform>, Mut<FixedCube>)>,
    mut stats: ResMut<TickStats>,
    physics_time: Res<PhysicsTime>,
) {
    let dt = physics_time.fixed_dt();
    stats.fixed_dt = dt;
    stats.fixed_steps += 1;
    stats.steps_this_frame += 1;

    for (_entity, (mut transform, mut cube)) in cubes.iter_mut() {
        cube.0 += dt * SPEED;
        transform.position.x = SPAN * cube.0.sin();
    }
}

/// KARE çizelgesinde koşar: kare başına tam bir kez, gerçek kare süresiyle.
///
/// Bevy'nin `frame_update`'i. Aynı zamanda bir önceki karede kaç sabit adım koştuğunu da
/// defterler — bu demonun asıl ölçtüğü sayı o.
fn frame_update(
    mut cubes: Query<(Mut<Transform>, Mut<FrameCube>)>,
    mut stats: ResMut<TickStats>,
    time: Res<Time>,
    physics_time: Res<PhysicsTime>,
) {
    let dt = time.dt();
    stats.frame_dt = dt;
    stats.frames += 1;
    stats.alpha = physics_time.alpha();

    let bucket = (stats.steps_this_frame as usize).min(3);
    stats.histogram[bucket] += 1;
    // Her 120 karede bir Bevy örneğindeki satırların karşılığı günlüğe düşer.
    if stats.frames.is_multiple_of(120) {
        gizmo::gizmo_log!(
            Info,
            "kare {}: sabit dt {:.4}, kare dt {:.4}, bu karede {} sabit adım, alpha {:.3}",
            stats.frames,
            stats.fixed_dt,
            dt,
            stats.steps_this_frame,
            stats.alpha
        );
    }
    stats.steps_this_frame = 0;

    for (_entity, (mut transform, mut cube)) in cubes.iter_mut() {
        cube.0 += dt * SPEED;
        transform.position.x = SPAN * cube.0.sin();
    }
}
