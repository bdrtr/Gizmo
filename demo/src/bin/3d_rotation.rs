//! # Eksen etrafında sabit hızda dönüş
//!
//! Tek bir küpü kendi Y ekseni etrafında sabit hızda döndürür: "bir nesneyi bir eksen
//! etrafında nasıl döndürürüm" sorusunun en kısa cevabı.
//!
//! **Bunun için elle bir hız bileşeni ve her frame `transform.rotate_y(speed * TAU * dt)` çağıran
//! bir sistem yazmak GEREKMİYOR**: motorun kendisi tam bu işi yapan bir bileşen taşıyor,
//! [`Spin`] + [`SpinPlugin`]. Demo da bu yüzden bileşen+sistem ikilisini tek satıra
//! indiriyor — ve hız tek bir ifadeyle veriliyor: saniyede 0,3 tur, yani `0.3 * TAU` rad/s.
//!
//! Sayısal notlar:
//!   * Küp 1×1×1. Gizmo'nun hazır küp mesh'i 2 birim olduğundan
//!     `Vec3::splat(0.5)` ölçekle aynı boyuta getirilir.
//!   * Kamera `(0, 10, 20)`'den orijine bakar.
//!   * Güneş `(3, 3, 3)`'ten orijine bakar. Gizmo'nun yönlü-ışık kuralı: ışık kendi yerel
//!     **−Z**'sine bakar (yaw 45°, pitch −35,26° — `asin(-1/√3)`).
//!
//! `Prefab`, `DespawnAfter`/`DespawnBelowY` ve `AutoBoxCollider` BİLEREK kullanılmadı: sahnede
//! tek bir görsel varlık var, tekrar eden kutu yok, uçan/geçici nesne yok ve küpün fizik
//! gövdesi de yok. Gövdesiz varlık CPU-fizik adımına girmediği için küp havada asılı kalır.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera · **Shift** — hızlı hareket

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::{FRAC_PI_4, TAU};

/// Saniyedeki tur sayısı.
const ROTATIONS_PER_SECOND: f32 = 0.3;

/// Güneşin (3, 3, 3) → orijin bakışının pitch'i: `asin(-1/√3)`.
const SUN_PITCH: f32 = -0.615_479_7;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - 3D Rotation", 1280, 720)
        .with_simple_scene(|scene, state| {
            // Beyaz küp — 1×1×1.
            let cube_mesh = AssetManager::create_cube(&scene.renderer.device);
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let material =
                Material::new(white).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.5, 0.0);

            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_scale(Vec3::splat(0.5)),
                GlobalTransform::default(),
                cube_mesh,
                material,
                MeshRenderer::new(),
                // Elle yazılacak hız bileşeni + dönüş sisteminin tamamı, tek satır.
                Spin::new(Vec3::Y, ROTATIONS_PER_SECOND * TAU),
            ));

            // Güneş — (3, 3, 3)'ten orijine bakar.
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(FRAC_PI_4) * Quat::from_rotation_x(SUN_PITCH),
                ..Default::default()
            });

            // Kamera — (0, 10, 20)'den orijine bakar.
            scene.spawn_camera(state, Vec3::new(0.0, 10.0, 20.0), Vec3::ZERO);
        })
        // Dönüşü motor sürer; demoda tek satır sistem kodu yok.
        .add_plugin(SpinPlugin)
        .run()
        .expect("uygulama çalıştırılamadı");
}
