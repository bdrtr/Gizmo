//! # Bevy'nin `transforms/transform` örneğinin Gizmo Engine portu
//!
//! Üç dönüşümün bir arada çalışması: bir küp merkezdeki küreyi **yörüngede** dolaşıyor (her kare
//! biraz küreye doğru dönüp kendi önüne doğru ilerleyerek), ve küre küpün başlangıç noktasından
//! ne kadar uzaklaştığına göre **küçülüyor**.
//!
//! ## Bu port bir eksik buldu — ve o eksik artık kapalı
//!
//! Bevy'nin örneği neredeyse tamamen `transform.forward()`, `transform.local_y()` ve
//! `transform.looking_at(hedef, yukarı)` üzerine kurulu. Port yazıldığında Gizmo'nun
//! [`Transform`]'unda **üçü de yoktu**; demo üçünü de yerel fonksiyon olarak yazmak zorunda
//! kalmıştı.
//!
//! **2026-08-23'te motora eklendi:** [`Transform::forward`], [`Transform::right`],
//! [`Transform::up`] ve [`Transform::looking_at`]. Demo artık motorun kendi metotlarını
//! çağırıyor. İşaretler uydurma değil, motorun kendi kuralı: renderer her yerde
//! `Mat4::look_at_rh` ve `Vec3::NEG_Z` kullanıyor, yani **sağ elli, ileri −Z**. Bevy'nin kuralı da
//! aynı, o yüzden örneğin sayıları olduğu gibi taşındı.
//!
//! `looking_at` dönüşü **uygulamıyor, döndürüyor** — çünkü bu örneğin yaptığı şey tam olarak onu
//! harmanlamak (`rotation.lerp(hedef, ağırlık)`), ve doğrudan uygulasaydı o imkân elden giderdi.
//!
//! ## İkinci fark: `.chain()` yok, sıralama etiketle kuruluyor
//!
//! Bevy üç sistemi `.chain()` ile bağlıyor. Gizmo'da zincir yok; sıralama
//! [`SystemConfig::label`] + [`SystemConfig::after`] ile **kenar kenar** yazılıyor. İki şeyi
//! bilmek gerek:
//!
//!   * Kısıt **kendi fazının içinde** çözülür; başka bir fazdaki sistemi adıyla çağırmak hiçbir
//!     şeyle eşleşmez.
//!   * Eşleşmeyen kısıt **hata değil** — uyarı basılır ve kısıt düşürülür. Yani etiketi yanlış
//!     yazmak sessizce sırasız bir çizelge bırakır; günlüğe bakmak gerekir.
//!
//! Burada sıra gerçekten önemli: `rotate` küpü küreye doğru biraz çevirir, `move` onu yeni
//! önüne doğru taşır, `scale` de sonucu okur. Ters sırada yörünge bir kare gecikir.
//!
//! ## Doğrulama
//!
//! `GIZMO_TRANSFORM_SELFTEST=1` her 120 karede bir küpün merkeze uzaklığını, yörüngedeki açısını
//! ve kürenin o anki ölçeğini günlüğe basar — yörüngenin gerçekten daire olduğu ve kürenin
//! küçüldüğü böyle görülüyor (ekranda tek kare bunu söylemez).
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera

use gizmo::core::query::{Mut, Query, With, Without};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::PI;

/// Yörüngedeki küpün ek verisi — Bevy'nin `CubeState`'i.
#[derive(Clone, Copy)]
struct CubeState {
    start_pos: Vec3,
    move_speed: f32,
    turn_speed: f32,
}
gizmo::core::impl_component!(CubeState);

/// Merkezdeki kürenin ölçek sınırları — Bevy'nin `Center`'ı.
#[derive(Clone, Copy)]
struct Center {
    max_size: f32,
    min_size: f32,
    scale_factor: f32,
}
gizmo::core::impl_component!(Center);

/// Kendi kendine ölçüm sayacı.
#[derive(Default, Clone, Copy)]
struct Probe {
    frame: u32,
    radius: f32,
    angle_deg: f32,
    sphere_scale: f32,
}
gizmo::core::impl_component!(Probe);

/// Kendi kendine ölçümün periyodu.
const PROBE_PERIOD: u32 = 120;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Transform", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );

            // Ölçeklenmeyi gösteren sarı küre — merkezde durur, yalnız boyu değişir.
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_scale(Vec3::ONE),
                GlobalTransform::default(),
                AssetManager::create_sphere(&scene.renderer.device, 3.0, 32, 48),
                Material::new(white.clone()).with_pbr(Vec4::new(0.95, 0.85, 0.15, 1.0), 0.4, 0.0),
                MeshRenderer::new(),
                Center {
                    max_size: 1.0,
                    min_size: 0.1,
                    scale_factor: 0.05,
                },
            ));

            // Dönmeyi ve yer değiştirmeyi gösteren beyaz küp. Başlangıçta küreden uzakta ve
            // 90° dönük duruyor: böylece "önü" küreye değil, kürenin çevresine bakıyor.
            let start_pos = Vec3::new(0.0, 0.0, -10.0);
            let start_rotation = Quat::from_rotation_y(PI / 2.0);
            scene.world.spawn_bundle((
                Transform::new(start_pos).with_rotation(start_rotation),
                GlobalTransform::default(),
                AssetManager::create_cube(&scene.renderer.device),
                Material::new(white).with_pbr(Vec4::new(0.9, 0.9, 0.9, 1.0), 0.5, 0.0),
                MeshRenderer::new(),
                CubeState {
                    start_pos,
                    move_speed: 2.0,
                    turn_speed: 0.2,
                },
            ));

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.6),
                intensity: 2.5,
                ..Default::default()
            });

            scene.world.insert_resource(Probe::default());
            scene.spawn_camera(state, Vec3::new(0.0, 10.0, 20.0), Vec3::ZERO);
        })
        // Bevy'nin `.chain()`'i burada iki kenar: rotate → move → scale.
        .add_update_system(rotate_cube.in_phase(Phase::Update).label("rotate_cube"))
        .add_update_system(
            move_cube
                .in_phase(Phase::Update)
                .label("move_cube")
                .after("rotate_cube"),
        )
        .add_update_system(scale_center.in_phase(Phase::Update).after("move_cube"))
        .set_ui(|world, _state, ctx| {
            let Some(probe) = world.get_resource::<Probe>().map(|p| *p) else {
                return;
            };
            gizmo::egui::Area::new("transform".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.heading("Üç dönüşüm bir arada");
                    ui.label(format!("küpün merkeze uzaklığı: {:.2}", probe.radius));
                    ui.label(format!("yörüngedeki açı: {:.1}°", probe.angle_deg));
                    ui.label(format!("kürenin ölçeği: {:.3}", probe.sphere_scale));
                    ui.separator();
                    ui.label("dönüş + ileri hareket = daire");
                    ui.label("forward()/looking_at() artık motorun kendi metotları");
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Küpü kendi **önüne** doğru taşır (Bevy'nin `move_cube`'ü).
fn move_cube(mut cubes: Query<(Mut<Transform>, &CubeState)>, time: Res<Time>) {
    for (_entity, (mut transform, cube)) in cubes.iter_mut() {
        let ahead = transform.forward();
        let delta = ahead * cube.move_speed * time.dt();
        transform.position += delta;
    }
}

/// Küpü her kare biraz küreye doğru çevirir (Bevy'nin `rotate_cube`'ü).
///
/// İleri hareketle birleşince ortaya çıkan şey daire: dönüş hızı ne kadar yüksekse yörünge o
/// kadar dar olur.
fn rotate_cube(
    mut cubes: Query<(Mut<Transform>, &CubeState, Without<Center>)>,
    centers: Query<(&Transform, With<Center>)>,
    time: Res<Time>,
) {
    // Dönülecek nokta: kürelerin konumlarının toplamı (Bevy de böyle yapıyor — tek küre var).
    let mut center = Vec3::ZERO;
    for (_entity, (transform, _)) in centers.iter() {
        center += transform.position;
    }

    for (_entity, (mut transform, cube, _)) in cubes.iter_mut() {
        // Küreye tam baksaydı hangi dönüş olurdu — ve o dönüşe doğru küçük bir adım.
        let target = transform.looking_at(center, transform.up());
        let weight = cube.turn_speed * time.dt();
        transform.rotation = transform.rotation.lerp(target, weight);
    }
}

/// Küreyi, küpün başlangıcından ne kadar uzaklaştığına göre küçültür (Bevy'nin uzun adlı
/// sistemi). Ayrıca ölçüm defterini doldurur.
fn scale_center(
    cubes: Query<(&Transform, &CubeState, Without<Center>)>,
    mut centers: Query<(Mut<Transform>, &Center)>,
    mut probe: ResMut<Probe>,
) {
    let mut distances = 0.0;
    let mut cube_position = Vec3::ZERO;
    for (_entity, (transform, state, _)) in cubes.iter() {
        distances += (state.start_pos - transform.position).length();
        cube_position = transform.position;
    }

    let mut sphere_scale = 0.0;
    for (_entity, (mut transform, center)) in centers.iter_mut() {
        let new_size = (center.max_size - center.scale_factor * distances).max(center.min_size);
        transform.scale = Vec3::splat(new_size);
        sphere_scale = new_size;
    }

    probe.frame += 1;
    probe.radius = cube_position.length();
    // Yörüngedeki açı: XZ düzleminde, başlangıç noktası (0, −10) sıfır kabul edilerek.
    probe.angle_deg = cube_position.x.atan2(-cube_position.z).to_degrees();
    probe.sphere_scale = sphere_scale;

    if std::env::var("GIZMO_TRANSFORM_SELFTEST").is_ok() && probe.frame.is_multiple_of(PROBE_PERIOD)
    {
        gizmo::gizmo_log!(
            Info,
            "kare {} · yarıçap {:.2} · açı {:.1}° · küre ölçeği {:.3}",
            probe.frame,
            probe.radius,
            probe.angle_deg,
            probe.sphere_scale
        );
    }
}
