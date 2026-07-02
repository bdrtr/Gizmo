//! Gizmo Engine — tarayıcı (WebGPU/WASM) demosu.
//!
//! `demo/src/bin/bevy_3d_scene.rs`'in web karşılığı: aynı yüksek seviye
//! `SimpleAppExt` API'si, aynı motor çekirdeği — hedef `wasm32-unknown-unknown`.
//! Sahneye havadan bırakılan bir küp yığını eklendi ki canlı fizik döngüsü
//! tarayıcıda gözle görülür olsun (küpler düşer, çarpışır, yerleşir).
//!
//! Derleme + paketleme (repo kökünden):
//! ```sh
//! cargo build -p demo-web --target wasm32-unknown-unknown --release
//! wasm-bindgen --target web --no-typescript \
//!     --out-dir demo-web/pkg \
//!     target/wasm32-unknown-unknown/release/demo_web.wasm
//! # sonra demo-web/ altında herhangi bir statik dosya sunucusu:
//! python3 -m http.server -d demo-web 8080   # → http://localhost:8080
//! ```
//!
//! Kontroller: sağ tık basılı + fare = bakış; WASD = hareket; Shift = hız.

#[cfg(target_arch = "wasm32")]
mod web {
    use gizmo::math::Vec3;
    use gizmo::prelude::*;
    use gizmo::simple::{SimpleAppExt, SimpleSceneState};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(start)]
    pub fn start() {
        // App::new panic hook'u + console_log/tracing-wasm'ı kurar; run() web'de
        // canvas'ı body'ye ekleyip event loop'u `spawn_app` ile başlatır ve
        // hemen döner (kareler requestAnimationFrame zincirinden akar).
        gizmo::app::App::<SimpleSceneState>::new("Gizmo Engine — Web Demo", 1280, 720)
            .with_simple_scene(|scene, state| {
                scene.spawn_ground(6.0);

                // Zemindeki referans küpü (bevy_3d_scene ile aynı).
                scene.spawn_cube(Vec3::new(0.0, 0.5, 0.0), 1.0, Vec3::new(0.20, 0.28, 1.0));

                // Havadan bırakılan yığın: canlı fizik kanıtı.
                let colors = [
                    Vec3::new(0.90, 0.30, 0.25),
                    Vec3::new(0.95, 0.75, 0.20),
                    Vec3::new(0.30, 0.80, 0.40),
                    Vec3::new(0.70, 0.40, 0.90),
                    Vec3::new(0.25, 0.75, 0.85),
                ];
                for (i, color) in colors.iter().enumerate() {
                    let f = i as f32;
                    scene.spawn_cube(
                        Vec3::new(-1.5 + f * 0.8, 3.0 + f * 1.2, -0.5 + (f % 2.0) * 0.6),
                        0.8,
                        *color,
                    );
                }
                scene.spawn_sphere(Vec3::new(1.8, 6.0, 0.8), 0.5, Vec3::new(0.95, 0.95, 0.95));

                let light_ent = scene.world.spawn();
                let bundle = DirectionalLightBundle {
                    rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4)
                        * Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
                    intensity: 1.8,
                    ..Default::default()
                };
                bundle.apply(scene.world, light_ent);

                scene.spawn_camera(state, Vec3::new(-2.5, 4.5, 9.0), Vec3::ZERO);
            })
            .run()
            .expect("gizmo web demo başlatılamadı");
    }
}
