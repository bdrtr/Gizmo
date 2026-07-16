//! Bevy'nin "3D Scene" örneğinin Gizmo Engine karşılığı.
//!
//! Tek küp + zemin diski + güneş ışığı + kamera — motorun "merhaba dünya" sahnesi.
//!
//! Bilinçli olarak yüksek-seviye `with_simple_scene` idiomuyla yazıldı: bu TEK çağrı
//! fizik adımını, WASD/QE serbest-uçuş kamerasını (sağ-tık ile fare-look) ve render
//! geçişini bizim yerimize kurar. Sahne tek dinamik küpten ibaret olduğundan Prefab
//! (tekrar eden kutular için), DespawnAfter/`despawn_all_with` (uçan/geçici nesne yok)
//! ya da elle girdi-kenarı takibi (kamera kontrolü zaten motorda) GEREKMEZ — bu demoda
//! idiomatik olan, motorun hazır kısayolunu kullanmaktır.

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - 3D Scene", 1280, 720)
        .with_simple_scene(|scene, state| {
            // Zemin diski (statik plane collider)
            scene.spawn_ground(4.0);

            // Mavi küp (dinamik gövde — zeminin üstünde durur)
            scene.spawn_cube(Vec3::new(0.0, 0.5, 0.0), 1.0, Vec3::new(0.20, 0.28, 1.0));

            // Güneş ışığı — bundle'ı tek çağrıda spawn'la
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4)
                    * Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
                intensity: 1.8,
                ..Default::default()
            });

            // Kamera — (-2.5, 4.5, 9) konumundan orijine bakar
            scene.spawn_camera(state, Vec3::new(-2.5, 4.5, 9.0), Vec3::ZERO);
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
