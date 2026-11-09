//! **Render world** — the picture is drawn from a copy of the game world, not from the game world.
//!
//! The game world spawns a cube every tenth of a second, keeps the newest 80, spins and orbits
//! them. Every frame the render hook brings a second world up to date with
//! `gizmo_core::extract::Extractor` and draws *that* with `default_render_pass`; the game world is
//! never handed to the draw.
//!
//! - **Space** stops the copying. The game carries on (the left-hand counters keep moving), the
//!   picture freezes on the last copy; press again and it jumps to where the game is.
//! - Each cube also carries `Brain`, a component the extractor was not told about: the HUD shows
//!   it on every cube in the game world and on none in the copy.
//!
//! This is stage 1 of CAPABILITY_GAPS A8: the copy works; the app does not yet own a render world,
//! and the render path still does simulation work, which is why this scene keeps to meshes, lights
//! and a camera.

use gizmo::core::extract::Extractor;
use gizmo::core::system::{IntoSystemConfig, Phase, ResMut};
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::time::Instant;

const KEEP: usize = 80;

#[derive(Clone)]
struct Spin {
    born: f32,
    radius: f32,
    height: f32,
}
gizmo::core::impl_component!(Spin);

/// Game-only: never copied.
#[derive(Clone)]
struct Brain;
gizmo::core::impl_component!(Brain);

#[derive(Clone)]
struct Game {
    cube: Mesh,
    materials: Vec<Material>,
    clock: f32,
    next_spawn: f32,
    born: u32,
    alive: std::collections::VecDeque<Entity>,
    copying: bool,
}

#[derive(Clone, Copy, Default)]
struct Report {
    game_entities: usize,
    game_brains: usize,
    copy_entities: usize,
    copy_brains: usize,
    extract_ms: f64,
    copying: bool,
}

fn main() {
    let mut render_world = World::new();
    let mut extractor = Extractor::new();
    extractor
        .component::<Transform>()
        .component::<GlobalTransform>()
        .component::<Mesh>()
        .component::<Material>()
        .component::<MeshRenderer>()
        .component::<Camera>()
        .component::<DirectionalLight>()
        .component::<gizmo::core::component::IsHidden>()
        .component::<Children>()
        .component::<Parent>()
        .resource::<gizmo::core::time::Time>();

    App::<SimpleSceneState>::new("Gizmo Engine - Render World", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let colours = [
                Vec4::new(0.95, 0.35, 0.25, 1.0),
                Vec4::new(0.30, 0.75, 0.95, 1.0),
                Vec4::new(0.95, 0.80, 0.25, 1.0),
                Vec4::new(0.45, 0.90, 0.45, 1.0),
            ];
            let materials = colours
                .iter()
                .map(|c| Material::new(white.clone()).with_pbr(*c, 0.45, 0.0))
                .collect();
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.5, 0.0)).with_scale(Vec3::new(12.0, 0.1, 12.0)),
                GlobalTransform::default(),
                AssetManager::create_cube(device),
                Material::new(white).with_pbr(Vec4::new(0.25, 0.25, 0.28, 1.0), 0.8, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.8),
                intensity: 2.6,
                ..Default::default()
            });
            scene.world.insert_resource(Game {
                cube: AssetManager::create_cube(device),
                materials,
                clock: 0.0,
                next_spawn: 0.0,
                born: 0,
                alive: Default::default(),
                copying: true,
            });
            scene.world.insert_resource(Report { copying: true, ..Default::default() });
            scene.spawn_camera(state, Vec3::new(0.0, 6.0, 14.0), Vec3::ZERO);
        })
        .add_update_system(play.in_phase(Phase::Update))
        .set_render(move |world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            let copying = world.get_resource::<Game>().is_some_and(|g| g.copying);
            let started = Instant::now();
            if copying {
                extractor.run(world, &mut render_world);
            }
            let extract_ms = started.elapsed().as_secs_f64() * 1e3;

            let report = Report {
                game_entities: world.query::<&Mesh>().iter().count(),
                game_brains: world.query::<&Brain>().iter().count(),
                copy_entities: render_world.query::<&Mesh>().iter().count(),
                copy_brains: render_world.query::<&Brain>().iter().count(),
                extract_ms,
                copying,
            };
            world.insert_resource(report);

            // The only world the draw sees.
            gizmo::systems::default_render_pass(&mut render_world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<Report>().map(|r| *r) else {
                return;
            };
            gizmo::egui::Area::new("rw".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(420.0);
                        ui.heading("Render world: çizim kopyadan");
                        ui.label("ekrandaki her şey ikinci dünyadan çiziliyor");
                        ui.separator();
                        ui.monospace(format!("oyun dünyası : {:>3} mesh · Brain {:>3}", r.game_entities, r.game_brains));
                        ui.monospace(format!("kopya        : {:>3} mesh · Brain {:>3}", r.copy_entities, r.copy_brains));
                        ui.monospace(format!("kopyalama    : {:.3} ms", r.extract_ms));
                        ui.separator();
                        if r.copying {
                            ui.label("Space: kopyalamayı durdur");
                        } else {
                            ui.colored_label(
                                gizmo::egui::Color32::from_rgb(255, 170, 60),
                                "KOPYALAMA DURDU — oyun sürüyor, resim son kopyada",
                            );
                            ui.label("Space: sürdür");
                        }
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// The game: spawn, keep the newest `KEEP`, move everything. Knows nothing about the copy.
fn play(
    mut game: ResMut<Game>,
    time: Res<gizmo::core::time::Time>,
    input: Res<Input>,
    mut cubes: Query<(&Spin, &mut Transform)>,
    mut commands: Commands,
) {
    if input.is_key_just_pressed(KeyCode::Space as u32) {
        game.copying = !game.copying;
    }
    game.clock += time.dt();
    let now = game.clock;

    while game.next_spawn <= now {
        game.next_spawn += 0.1;
        let n = game.born;
        game.born += 1;
        let spin = Spin {
            born: now,
            radius: 2.0 + (n % 5) as f32 * 0.9,
            height: (n % 7) as f32 * 0.45,
        };
        let material = game.materials[n as usize % game.materials.len()].clone();
        let entity = commands
            .spawn_bundle((
                Transform::new(Vec3::ZERO).with_scale(Vec3::splat(0.3)),
                GlobalTransform::default(),
                game.cube.clone(),
                material,
                MeshRenderer::new(),
                spin,
                Brain,
            ))
            .id();
        game.alive.push_back(entity);
        while game.alive.len() > KEEP {
            if let Some(old) = game.alive.pop_front() {
                commands.entity(old).despawn();
            }
        }
    }

    for (spin, mut transform) in &mut cubes {
        let age = now - spin.born;
        let angle = age * 1.3 + spin.radius;
        transform.position = Vec3::new(angle.cos() * spin.radius, spin.height - 0.8, angle.sin() * spin.radius);
        transform.rotation = Quat::from_rotation_y(age * 2.0) * Quat::from_rotation_x(age * 1.1);
    }
}
