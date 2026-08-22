//! # Bevy'nin `transforms/scale` örneğinin Gizmo Engine portu
//!
//! Tek bir küpü sırayla X, Y ve Z eksenlerinde büyütüp küçültür: nesne bir eksende üst sınıra
//! çarpınca yön tersine döner, alt sınıra inince yön hem tersine döner hem de bir sonraki
//! eksene kayar (`zxy` swizzle'ı). Böylece küp sonsuz bir "nefes alma" döngüsüne girer.
//!
//! **Bu port, [`bevy_3d_rotation`]'ın tersini gösteriyor.** Orada Bevy'nin elle yazdığı
//! bileşen+sistem ikilisinin motorda hazır karşılığı vardı ([`Spin`]); burada YOK — ölçek
//! döngüsü oyuna özgü bir kural, motorun içine gömülecek bir şey değil. Dolayısıyla bu demo
//! Gizmo'nun ECS yüzeyini olduğu gibi kullanır: `impl_component!` ile bir bileşen,
//! `Query<(Mut<Transform>, Mut<Scaling>)>` + `Res<Time>` ile iki sistem, `Phase::Update`'e
//! kayıt. İki örneğin yan yana durması, "motor ne zaman yardım eder, ne zaman etmez"in
//! kendisidir.
//!
//! Sayısal parite notları (Bevy 0.19 kaynağına karşı):
//!   * Küp `Cuboid::default()` (1×1×1) — Gizmo'nun hazır küpü 2 birim olduğundan ölçek
//!     değerleri 0,5 ile çarpılarak uygulanır; sınırlar Bevy'deki gibi 1,0 ve 5,0 *birim*
//!     cinsinden tutulur.
//!   * Başlangıç dönüşü `Quat::from_rotation_y(PI/4)`, hız 2,0 birim/sn, yön `Vec3::X`.
//!   * Kamera `(0, 10, 20)`'den orijine, güneş `(3, 3, 3)`'ten orijine bakar.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera · **Shift** — hızlı hareket

use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, Res};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::{FRAC_PI_4, PI};

/// Bevy'nin `Scaling` bileşeni: hangi eksende, ne hızla, hangi sınırlar arasında.
#[derive(Clone, Copy)]
struct Scaling {
    scale_direction: Vec3,
    scale_speed: f32,
    max_element_size: f32,
    min_element_size: f32,
}

impl Scaling {
    fn new() -> Self {
        Self {
            scale_direction: Vec3::X,
            scale_speed: 2.0,
            max_element_size: 5.0,
            min_element_size: 1.0,
        }
    }
}

gizmo::core::impl_component!(Scaling);

/// Gizmo'nun hazır küp mesh'i 2 birim; Bevy'nin 1 birimlik küpüne getiren çarpan.
const UNIT: f32 = 0.5;

/// Güneşin (3, 3, 3) → orijin bakışının pitch'i: `asin(-1/√3)`.
const SUN_PITCH: f32 = -0.615_479_7;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Scale", 1280, 720)
        .with_simple_scene(|scene, state| {
            let cube_mesh = AssetManager::create_cube(&scene.renderer.device);
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let material = Material::new(white).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.5, 0.0);

            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO)
                    .with_rotation(Quat::from_rotation_y(PI / 4.0))
                    .with_scale(Vec3::splat(UNIT)),
                GlobalTransform::default(),
                cube_mesh,
                material,
                MeshRenderer::new(),
                Scaling::new(),
            ));

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(FRAC_PI_4) * Quat::from_rotation_x(SUN_PITCH),
                ..Default::default()
            });

            scene.spawn_camera(state, Vec3::new(0.0, 10.0, 20.0), Vec3::ZERO);
        })
        .add_system(change_scale_direction.in_phase(Phase::Update))
        .add_system(scale_cube.in_phase(Phase::Update))
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Sınıra çarpan ölçeği geri döndürür; alt sınırda ayrıca ekseni kaydırır.
///
/// Bevy'nin `floor`/`ceil` numarası aynen korundu: sınır aşıldığında ölçek tam sayıya
/// oturtulur, böylece aynı koşul bir sonraki frame'de yeniden tetiklenmez.
fn change_scale_direction(mut cubes: Query<(Mut<Transform>, Mut<Scaling>)>) {
    for (_entity, (mut transform, mut cube)) in cubes.iter_mut() {
        // Gizmo'nun küpü 2 birim olduğu için sınırlar Bevy'nin *birim* ölçeğinde okunur.
        let size = transform.scale / UNIT;

        if size.max_element() > cube.max_element_size {
            cube.scale_direction *= -1.0;
            transform.scale = size.floor() * UNIT;
        }

        if size.min_element() < cube.min_element_size {
            cube.scale_direction *= -1.0;
            transform.scale = size.ceil() * UNIT;
            // Bevy'nin `.zxy()` swizzle'ı — motorun matematiği glam ama `Vec3Swizzles`
            // trait'ini dışa vermiyor, o yüzden elle: X→Y→Z→X sırayla eksen değiştirir.
            let d = cube.scale_direction;
            cube.scale_direction = Vec3::new(d.z, d.x, d.y);
        }
    }
}

/// Ölçeği o anki yönde büyütür.
fn scale_cube(mut cubes: Query<(Mut<Transform>, Mut<Scaling>)>, time: Res<Time>) {
    let dt = time.dt();
    for (_entity, (mut transform, cube)) in cubes.iter_mut() {
        transform.scale += cube.scale_direction * cube.scale_speed * dt * UNIT;
    }
}
