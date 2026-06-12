//! Bevy'nin "3D Scene" örneğinin Gizmo Engine karşılığı.
//! Yüksek seviye SimpleAppExt API ile yazıldı.

use gizmo::prelude::*;
use gizmo::math::Vec3;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

fn main() {
    gizmo::app::App::<SimpleSceneState>::new("Gizmo Engine - 3D Scene", 1280, 720)
        .with_simple_scene(|scene, state| {
            // Circular base (zemin diski)
            scene.spawn_ground(4.0);
            
            // Cube (küp)
            scene.spawn_cube(Vec3::new(0.0, 0.5, 0.0), 1.0, Vec3::new(0.20, 0.28, 1.0));
            
            // Light (ışık)
            let light_ent = scene.world.spawn();
            let bundle = DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4) * Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
                intensity: 1.8,
                ..Default::default()
            };
            bundle.apply(scene.world, light_ent);
            
            // Camera (kamera)
            scene.spawn_camera(state, Vec3::new(-2.5, 4.5, 9.0), Vec3::ZERO);
        })
        .run();
}
