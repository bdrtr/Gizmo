//! Port of Bevy's material animation example to Gizmo Engine.
//! Spawns a 3x3 grid of cubes with different HSL hues and animates them dynamically over time.

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::core::system::{Res, IntoSystemConfig, Phase};
use gizmo::core::query::{Query, Mut};

/// Component to track the current hue of each cube (in degrees, 0.0..360.0).
#[derive(Clone, Copy)]
struct CubeHue(f32);

impl Component for CubeHue {}

fn main() {
    let mut app = gizmo::app::App::<SimpleSceneState>::new("Gizmo Engine - Bevy Material Animation Parity", 1280, 720);

    app = app
        .with_simple_scene(|scene, state| {
            // Spawn Camera looking at the cubes
            scene.spawn_camera(state, Vec3::new(3.0, 2.0, 4.0), Vec3::new(0.0, -0.5, 0.0));

            // Setup a directional sun light
            let sun_ent = scene.world.spawn();
            let sun_bundle = DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4),
                intensity: 0.8,
                color: Vec3::new(1.0, 1.0, 1.0),
                ..Default::default()
            };
            sun_bundle.apply(scene.world, sun_ent);

            // Mesh & texture for cubes
            let cube_mesh = AssetManager::create_cube(&scene.renderer.device);
            let white_texture = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );

            // Parity math
            const GOLDEN_ANGLE: f32 = 137.507_77;
            let mut current_hue = 0.0;

            for x in -1..2 {
                for z in -1..2 {
                    let ent = scene.world.spawn();
                    
                    // The Gizmo Engine cube is size 2.0 by default.
                    // Scale it to 0.25 (making it size 0.5, matching Bevy's Cuboid::new(0.5, 0.5, 0.5))
                    let transform = Transform::new(Vec3::new(x as f32, 0.0, z as f32))
                        .with_scale(Vec3::splat(0.25));
                        
                    scene.world.add_component(ent, transform);
                    scene.world.add_component(ent, GlobalTransform::default());
                    scene.world.add_component(ent, cube_mesh.clone());
                    
                    // Convert initial HSL to RGB albedo
                    let rgb = hsl_to_rgb(current_hue, 1.0, 0.5);
                    let mat = Material::new(white_texture.clone())
                        .with_pbr(Vec4::new(rgb[0], rgb[1], rgb[2], 1.0), 0.1, 0.1); // Beautiful smooth PBR
                    
                    scene.world.add_component(ent, mat);
                    scene.world.add_component(ent, MeshRenderer::new());
                    
                    // Spawn with tracking component
                    scene.world.add_component(ent, CubeHue(current_hue));

                    current_hue += GOLDEN_ANGLE;
                }
            }
        })
        .add_system(animate_materials.in_phase(Phase::Update));

    app.run();
}

/// Dynamic HSL to RGB conversion helper.
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 4] {
    let h = h.rem_euclid(360.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;

    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    [r + m, g + m, b + m, 1.0]
}

/// Dynamic ECS system that animates the cube colors smoothly over time.
fn animate_materials(
    mut q_materials: Query<(Mut<Material>, Mut<CubeHue>)>,
    time: Res<Time>,
) {
    let dt = time.dt();
    // Rotate hue dynamically by 100 degrees per second (just like Bevy example!)
    for (_entity, (mut mat, mut hue)) in q_materials.iter_mut() {
        hue.0 = (hue.0 + dt * 100.0).rem_euclid(360.0);
        let rgb = hsl_to_rgb(hue.0, 1.0, 0.5);
        mat.albedo = Vec4::new(rgb[0], rgb[1], rgb[2], 1.0);
    }
}
