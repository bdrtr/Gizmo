//! # Sahne aydınlatması
//!
//! Motorun aydınlatma yüzeyinin tamamı tek sahnede: **yönlü güneş** (gölge kaskadlarını süren),
//! **iki nokta ışığı** (kırmızı ve mavi), **bir spot ışığı** (yeşil koni) ve **ortam ışığı** —
//! iki duvar, bir zemin, bir küp ve bir kürenin üstünde. Güneş kendi ekseninde dönerek gölgeleri
//! süpürür; küp ve küre ok tuşlarıyla ışıkların arasında gezdirilebilir.
//!
//! ## Motorun kendine özgü iki noktası — demo bunları saklamıyor
//!
//! 1. **Ortam ışığı global bir kaynak değil, malzemenin kendi alanı.** Motorda ortam ışığını
//!    tek yerden ayarlayan bir kaynak **yok**; her `Material` kendi `ambient`'ını taşır
//!    (`with_ambient`). Bu yüzden Space tuşu burada tek bir kaynağı değil, sahnedeki BÜTÜN
//!    malzemeleri dolaşarak açıp kapatıyor.
//! 2. **Işık şiddeti fiziksel bir birim değil.** Gizmo'nun `intensity`'si kandela ya da lüks
//!    değil, düz bir çarpan. Değerler bu yüzden **görsel** olarak seçildi ve hangi değerin neye
//!    karşılık geldiği aşağıda sabit isimleriyle duruyor.
//!
//! Güneşin dönüşünü motorun [`Spin`] bileşeni yapıyor — ve `with_rest_rotation` sayesinde
//! güneşin yazar-duruşu (−45° pitch) korunur, dönüş onun ÜSTÜNE biner. Işıkların üstündeki
//! küçük parlayan işaretçiler ışığın ÇOCUĞU (`add_child`), yani konumları hiyerarşiden geliyor.
//!
//!
//! **Bir motor tuzağı, ve kolayca görünmez:** `add_system` sistemi **sabit
//! adımlı** çizelgeye kaydeder — kare başına sıfır ya da birkaç kez koşar. Girdi kenarları
//! (`is_key_just_pressed`) ise kare başına bir kez yakalanıp temizlenir, yani o çizelgede
//! okunursa basışlar kaçar ya da iki kez işlenir. Girdiye bakan her sistem bu yüzden
//! `add_update_system` ile kaydediliyor (kare başına tam bir kez, gerçek `dt` ile).
//! ## Kontroller
//!   * **Ok tuşları** — küpü ve küreyi hareket ettir · **Space** — ortam ışığını aç/kapat
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera · **Shift** — hızlı hareket

use gizmo::core::input::{Input, MoveKeys};
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res};
use gizmo::core::HierarchyExt;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

/// Ok tuşlarıyla gezdirilebilen nesnelerin işareti.
#[derive(Clone, Copy)]
struct Movable;
gizmo::core::impl_component!(Movable);

/// Ortam ışığının açık hâli — görsel olarak seçilmiş bir değer.
const AMBIENT_ON: Vec3 = Vec3::new(0.06, 0.004, 0.0); // ORANGE_RED × 0,06
/// CSS `INDIGO`, doğrusal uzayda — duvarlar.
const INDIGO: Vec4 = Vec4::new(0.070_36, 0.0, 0.223_228, 1.0);
/// CSS `DEEP_PINK`, doğrusal — küp.
const DEEP_PINK: Vec4 = Vec4::new(1.0, 0.006_995, 0.291_771, 1.0);
/// CSS `LIMEGREEN`, doğrusal — küre.
const LIMEGREEN: Vec4 = Vec4::new(0.031_896, 0.610_496, 0.031_896, 1.0);

/// Nokta ve spot ışıklarının şiddeti.
const LAMP_INTENSITY: f32 = 12.0;
/// Işıkların menzili — 20 metre.
const LAMP_RANGE: f32 = 20.0;
/// Güneşin şiddeti.
const SUN_INTENSITY: f32 = 1.2;
/// Güneşin kendi ekseninde dönüş hızı.
const SUN_SPIN: f32 = 0.5;
/// Ok tuşlarının hareket hızı.
const MOVE_SPEED: f32 = 2.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Lighting", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let mat = |albedo: Vec4, roughness: f32| {
                Material::new(white.clone())
                    .with_pbr(albedo, roughness, 0.0)
                    .with_ambient(AMBIENT_ON)
            };

            // ── Zemin: 10×10, beyaz, tamamen pürüzlü ────────────────────────────────
            let plane = AssetManager::create_plane(&scene.renderer.device, 10.0);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                plane,
                mat(Vec4::new(1.0, 1.0, 1.0, 1.0), 1.0),
                MeshRenderer::new(),
            ));

            // ── İki duvar: 5 × 0,15 × 5 dilim, biri sağda dik, biri arkada yatık ────
            let cube = AssetManager::create_cube(&scene.renderer.device);
            let wall_scale = Vec3::new(2.5, 0.075, 2.5); // motorun küpü 2 birim
            for (position, rotation) in [
                (Vec3::new(2.5, 2.5, 0.0), Quat::from_rotation_z(FRAC_PI_2)),
                (Vec3::new(0.0, 2.5, -2.5), Quat::from_rotation_x(FRAC_PI_2)),
            ] {
                scene.world.spawn_bundle((
                    Transform::new(position)
                        .with_rotation(rotation)
                        .with_scale(wall_scale),
                    GlobalTransform::default(),
                    cube.clone(),
                    mat(INDIGO, 1.0),
                    MeshRenderer::new(),
                ));
            }

            // ── Gezdirilebilir küp ve küre ──────────────────────────────────────────
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.5, 0.0)).with_scale(Vec3::splat(0.5)),
                GlobalTransform::default(),
                cube.clone(),
                mat(DEEP_PINK, 0.5),
                MeshRenderer::new(),
                Movable,
            ));
            let sphere = AssetManager::create_sphere(&scene.renderer.device, 0.5, 18, 32);
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(1.5, 1.0, 1.5)),
                GlobalTransform::default(),
                sphere,
                mat(LIMEGREEN, 0.5),
                MeshRenderer::new(),
                Movable,
            ));

            // ── Işıklar, her biri kendi parlayan işaretçisiyle (işaretçi = ÇOCUK) ───
            let bulb = AssetManager::create_sphere(&scene.renderer.device, 0.1, 12, 18);
            let lamp = |scene: &mut gizmo::simple::SceneBuilder,
                            position: Vec3,
                            color: Vec3,
                            emissive: Vec3| {
                let light = scene.world.spawn_bundle(PointLightBundle {
                    position,
                    color,
                    intensity: LAMP_INTENSITY,
                    radius: LAMP_RANGE,
                });
                let marker = scene.world.spawn_bundle((
                    Transform::new(Vec3::ZERO),
                    GlobalTransform::default(),
                    bulb.clone(),
                    Material::new(white.clone())
                        .with_pbr(Vec4::new(color.x, color.y, color.z, 1.0), 0.5, 0.0)
                        .with_emissive(emissive),
                    MeshRenderer::new(),
                ));
                scene.world.add_child(light, marker);
            };
            // Kırmızı ve mavi nokta ışıkları.
            lamp(
                scene,
                Vec3::new(1.0, 2.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(4.0, 0.0, 0.0),
            );
            lamp(
                scene,
                Vec3::new(0.0, 4.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(0.0, 0.0, 4.0),
            );

            // Yeşil spot: (−1, 2, 0)'dan düz aşağı bakar (yerel −Z aşağıya dönsün diye −90° pitch).
            let spot = scene.world.spawn_bundle(SpotLightBundle {
                position: Vec3::new(-1.0, 2.0, 0.0),
                rotation: Quat::from_rotation_x(-FRAC_PI_2),
                color: Vec3::new(0.0, 1.0, 0.0),
                intensity: LAMP_INTENSITY,
                radius: LAMP_RANGE,
                inner_angle: 0.6,
                outer_angle: 0.8,
            });
            let spot_marker = scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_capsule(&scene.renderer.device, 0.1, 0.125, 8, 16),
                Material::new(white.clone())
                    .with_pbr(Vec4::new(0.0, 1.0, 0.0, 1.0), 0.5, 0.0)
                    .with_emissive(Vec3::new(0.0, 4.0, 0.0)),
                MeshRenderer::new(),
            ));
            scene.world.add_child(spot, spot_marker);

            // ── Güneş: −45° yatık, kendi ekseninde dönüyor ──────────────────────────
            let rest = Quat::from_rotation_x(-FRAC_PI_4);
            let sun = scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: rest,
                intensity: SUN_INTENSITY,
                ..Default::default()
            });
            scene
                .world
                .add_component(sun, Spin::new(Vec3::Y, SUN_SPIN).with_rest_rotation(rest));

            scene.spawn_camera(state, Vec3::new(-2.0, 2.5, 5.0), Vec3::new(0.0, 0.5, 0.0));
        })
        .add_plugin(SpinPlugin)
        .add_update_system(movement.in_phase(Phase::Update))
        .add_update_system(toggle_ambient_light.in_phase(Phase::Update))
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Ok tuşlarıyla `Movable` nesneleri X/Y düzleminde gezdirir.
///
/// Tuşlar tek tek okunmuyor: motorun kuralı hareketin **tek bir yerden** — `Input::move_axis`
/// ailesinden — geçmesi, ve deponun bir testi bunu zorluyor. Kazancı somut: oyun kolunun sol
/// çubuğu bedavaya çalışır ve çapraz basış %41 hızlı olmaz (iki birim vektörün toplamı
/// normalize edilir). Dört tuşu elle okuyup `Vec3`'ü toplamanın yerini `MoveKeys::ARROWS`
/// tutuyor.
fn movement(
    mut movables: Query<(Mut<Transform>, With<Movable>)>,
    input: Res<Input>,
    time: Res<Time>,
) {
    let (x, y) = input.move_axis_with(MoveKeys::ARROWS);
    if x == 0.0 && y == 0.0 {
        return;
    }

    let step = Vec3::new(x, y, 0.0) * MOVE_SPEED * time.dt();
    for (_entity, (mut transform, _)) in movables.iter_mut() {
        transform.position += step;
    }
}

/// Space ortam ışığını açıp kapatır.
///
/// Ortam ışığı motorda malzemenin alanı olduğu için bunun karşılığı sahnedeki bütün
/// `Material`'ları dolaşmak — tek bir kaynağı değiştirmek değil.
fn toggle_ambient_light(mut materials: Query<Mut<Material>>, input: Res<Input>) {
    if !input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32) {
        return;
    }
    for (_entity, mut material) in materials.iter_mut() {
        material.ambient = if material.ambient == Vec3::ZERO {
            AMBIENT_ON
        } else {
            Vec3::ZERO
        };
    }
}
