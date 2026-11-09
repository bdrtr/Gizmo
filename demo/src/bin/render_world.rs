//! **Render world** — the picture is drawn from a copy of the game world, on a thread of its own.
//!
//! The game world spawns a cube every tenth of a second, keeps the newest 80, spins and orbits
//! them. The app's render stage (`Shown`, a `RenderStage`) has the frame's two halves. At the sync
//! point, with the game world, `prepare` prepares it (`prepare_frame`: clocks, transforms,
//! materials — everything a frame writes that the game can see) and brings a `RenderWorld` — a
//! second world kept by `render_extractor`, which copies every type the draw reads — up to date.
//! Then `draw` draws *that* with `draw_frame`, which takes the world read-only, and is handed no
//! game world at all. `App::with_render_thread` runs `draw` on a render thread, while the game
//! goes on with the next frame's update. `RenderWorld` is itself such a stage, and what `SimpleApp`
//! and `with_scene_render` draw with; this one spells the steps out to be able to stop the copy.
//!
//! - **Space** stops the copying. The game carries on (the left-hand counters keep moving), the
//!   picture freezes on the last copy; press again and it jumps to where the game is.
//! - Each cube also carries `Brain`, a component the extractor was not told about: the HUD shows
//!   it on every cube in the game world and on none in the copy.
//! - The cubes are drawn through handles: their mesh and material live in `Assets<Mesh>` /
//!   `Assets<Material>`, which the extractor keeps in the copy entry by entry (`assets::<T>()`).
//!   Every two seconds the game recolours one material in its map (`Assets::get_mut`); the copy
//!   clones that one entry and no other, and the cubes using it change colour in the picture.
//! - The draw's time is measured on the render thread and reaches the HUD a frame later, at the
//!   next sync point — the first moment the game world is in reach after the draw.
//!
//! CAPABILITY_GAPS A8: the draw only reads, reads nothing a copy cannot hold (asset maps, the
//! visibility index, the water for underwater fog, resolved localized labels), and every game
//! drawn by the engine's own render hooks is drawn from a copy.

use gizmo::app::RenderStage;
use gizmo::systems::render::RenderWorld;
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
    cube: Handle<Mesh>,
    materials: Vec<Handle<Material>>,
    next_recolour: f32,
    recoloured: u32,
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
    game_materials: usize,
    copy_materials: usize,
    extract_ms: f64,
    /// The last draw's time, measured on the render thread.
    draw_ms: f64,
    copying: bool,
}

/// The app's render stage: the copy, and what the last draw took.
struct Shown {
    copy: RenderWorld,
    draw_ms: f64,
}

impl RenderStage for Shown {
    fn prepare(&mut self, world: &mut World, renderer: &mut Renderer) {
        renderer.gpu_physics = None;
        renderer.gpu_fluid = None;
        renderer.gpu_particles = None;
        renderer.ssr = None;
        renderer.ssgi = None;

        // What the last draw cost: the game world is in reach again for the first time since.
        gizmo::systems::render::publish_render_stats(world, renderer);
        // The frame's game-visible work happens in the game world, before the copy is taken.
        gizmo::systems::render::prepare_frame(world, renderer);
        let copying = world.get_resource::<Game>().is_some_and(|g| g.copying);
        let started = Instant::now();
        if copying {
            self.copy.extract(world);
        }
        let extract_ms = started.elapsed().as_secs_f64() * 1e3;

        let copy = self.copy.world();
        let materials = |w: &World| w.get_resource::<Assets<Material>>().map_or(0, |a| a.len());
        let report = Report {
            game_entities: world.query::<&MeshRenderer>().iter().count(),
            game_brains: world.query::<&Brain>().iter().count(),
            copy_entities: copy.query::<&MeshRenderer>().iter().count(),
            copy_brains: copy.query::<&Brain>().iter().count(),
            game_materials: materials(world),
            copy_materials: materials(copy),
            extract_ms,
            draw_ms: self.draw_ms,
            copying,
        };
        world.insert_resource(report);
    }

    fn draw(&mut self, encoder: &mut gizmo::wgpu::CommandEncoder, view: &gizmo::wgpu::TextureView, renderer: &mut Renderer) {
        // The only world the draw sees — and only to read it.
        let started = Instant::now();
        gizmo::systems::render::draw_frame(self.copy.world(), encoder, view, renderer);
        self.draw_ms = started.elapsed().as_secs_f64() * 1e3;
    }
}

fn main() {
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
            let materials = {
                let mut assets = scene.world.get_resource_mut_or_default::<Assets<Material>>();
                colours
                    .iter()
                    .map(|c| assets.add(Material::new(white.clone()).with_pbr(*c, 0.45, 0.0)))
                    .collect()
            };
            let cube = scene.world.get_resource_mut_or_default::<Assets<Mesh>>().add(AssetManager::create_cube(device));
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
                cube,
                materials,
                next_recolour: 2.0,
                recoloured: 0,
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
        .set_render_stage(Shown { copy: RenderWorld::new(), draw_ms: 0.0 })
        .with_render_thread()
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
                        ui.monospace(format!(
                            "materyal     : oyun {} · kopya {} (handle ile)",
                            r.game_materials, r.copy_materials
                        ));
                        ui.monospace(format!("kopyalama    : {:.3} ms", r.extract_ms));
                        ui.monospace(format!("çizim        : {:.3} ms (render thread'de)", r.draw_ms));
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
    mut materials: ResMut<Assets<Material>>,
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

    // One material changes in the game's asset map; the copy clones that entry alone.
    if now >= game.next_recolour {
        game.next_recolour += 2.0;
        game.recoloured += 1;
        let which = game.materials[game.recoloured as usize % game.materials.len()].clone();
        let hue = (game.recoloured as f32 * 0.37).fract();
        if let Some(material) = materials.get_mut(&which) {
            material.albedo = Vec4::new(0.3 + 0.7 * hue, 0.9 - 0.6 * hue, 0.4 + 0.5 * (1.0 - hue), 1.0);
        }
    }

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
