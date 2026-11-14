//! **Render thread** — the same scene drawn on a render thread (the default) and on the loop's
//! thread (`--inline`), measured.
//!
//! A grid of cubes (one mesh, four materials, so the draw batches them) turns every frame, so
//! every frame changes every transform the draw reads; `--work MS` adds that much game work to
//! the update. After `--warmup` frames, `--frames` frames are measured and the app exits with one
//! line on stdout:
//!
//! - **frame**: the loop's frame period, the time between two updates. Drawn inline, a frame is
//!   the update plus the draw. Drawn on a render thread, it is the longer of the two (the draw of
//!   frame N runs during the update of frame N + 1), plus the hand-over.
//! - **latency**: from the start of a frame's update to the end of its draw — how old the game
//!   state is when its picture has been recorded. A render thread does not make a frame's own work
//!   shorter, so this should not change much; what it changes is how many frames are in flight.
//!
//! ```text
//! cargo run --release -p demo --bin render_thread -- [--inline] [--cubes N] [--work MS]
//!     [--frames N] [--warmup N] [--vsync]
//! ```
//!
//! Measured 2026-11-10 on this project's laptop (16 threads, RTX 3050 Laptop, under Xwayland),
//! no vsync, median frame / median latency, one run each after a run-to-run spread of ~0.3 ms had
//! been seen:
//!
//! | scene                   | `--inline`      | render thread   |
//! |-------------------------|-----------------|-----------------|
//! | 2 000 cubes             | 3.9 / 2.6 ms    | 4.2 / 3.9 ms    |
//! | 2 000 cubes, 8 ms work  | 11.0 / 10.0 ms  | 9.2 / 10.1 ms   |
//! | 20 000 cubes            | 20.8 / 19.7 ms  | 19.5 / 20.5 ms  |
//! | 20 000 cubes, 8 ms work | 27.4 / 26.3 ms  | 19.9 / 26.2 ms  |
//!
//! With game work the render thread is 19–37 % faster at the same latency; without it the frame
//! rate is the same within the noise (an earlier run of the first row had the thread at 3.4 ms
//! against 4.3) and the latency up to 1.3 ms longer. Before the loop paced itself
//! (`gizmo_app`'s `pacing`), the thread's latency in the two light rows was 5.6 and 27.7 ms: the
//! update started at once and then waited for the draw at the sync point, holding old input.
//!
//! CAPABILITY_GAPS A8: this is the measurement behind the render thread's default.

use gizmo::app::RenderStage;
use gizmo::core::system::{IntoSystemConfig, Phase, ResMut};
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::systems::render::RenderWorld;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
struct Args {
    inline: bool,
    cubes: usize,
    work_ms: f64,
    frames: usize,
    warmup: usize,
    vsync: bool,
}

impl Args {
    fn parse() -> Self {
        let mut args = Self { inline: false, cubes: 20_000, work_ms: 0.0, frames: 600, warmup: 180, vsync: false };
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            let mut value = |name: &str| it.next().unwrap_or_else(|| panic!("{name} needs a value"));
            match a.as_str() {
                "--inline" => args.inline = true,
                "--vsync" => args.vsync = true,
                "--cubes" => args.cubes = value("--cubes").parse().expect("--cubes N"),
                "--work" => args.work_ms = value("--work").parse().expect("--work MS"),
                "--frames" => args.frames = value("--frames").parse().expect("--frames N"),
                "--warmup" => args.warmup = value("--warmup").parse().expect("--warmup N"),
                other => panic!("unknown argument {other}"),
            }
        }
        args
    }
}

#[derive(Clone, Copy)]
struct Spin(f32);
gizmo::core::impl_component!(Spin);

/// When this frame's update began: the first system of the update writes it.
#[derive(Clone, Copy)]
struct FrameStart(Instant);

/// What the measurement has seen so far.
struct Bench {
    args: Args,
    frame: usize,
    last: Option<Instant>,
    periods: Vec<f64>,
    latencies: Arc<Mutex<Vec<f64>>>,
}

/// The engine's own stage, timed: `prepare` notes when the frame's update began, `draw` how long
/// after that its draw ended — on whichever thread it runs.
struct Timed {
    inner: RenderWorld,
    started: Option<Instant>,
    latencies: Arc<Mutex<Vec<f64>>>,
}

impl RenderStage for Timed {
    fn prepare(&mut self, world: &mut World, renderer: &mut Renderer) {
        self.started = world.get_resource::<FrameStart>().map(|s| s.0);
        self.inner.prepare(world, renderer);
    }

    fn draw(&mut self, encoder: &mut gizmo::wgpu::CommandEncoder, view: &gizmo::wgpu::TextureView, renderer: &mut Renderer) {
        self.inner.draw(encoder, view, renderer);
        if let Some(started) = self.started {
            if let Ok(mut l) = self.latencies.lock() {
                l.push(started.elapsed().as_secs_f64() * 1e3);
            }
        }
    }
}

fn main() {
    let args = Args::parse();
    let latencies = Arc::new(Mutex::new(Vec::new()));
    let log = latencies.clone();

    let app = App::<SimpleSceneState>::new("Gizmo Engine - Render Thread", 1280, 720)
        .with_simple_scene(move |scene, state| {
            // A frame-time measurement: uncapped unless asked, since the engine's default is vsync.
            scene.renderer.set_present_mode(if args.vsync {
                gizmo::wgpu::PresentMode::AutoVsync
            } else {
                gizmo::wgpu::PresentMode::AutoNoVsync
            });
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let colours = [
                Vec4::new(0.95, 0.35, 0.25, 1.0),
                Vec4::new(0.30, 0.75, 0.95, 1.0),
                Vec4::new(0.95, 0.80, 0.25, 1.0),
                Vec4::new(0.45, 0.90, 0.45, 1.0),
            ];
            let materials: Vec<Handle<Material>> = {
                let mut assets = scene.world.get_resource_mut_or_default::<Assets<Material>>();
                colours.iter().map(|c| assets.add(Material::new(white.clone()).with_pbr(*c, 0.5, 0.0))).collect()
            };
            let cube = scene
                .world
                .get_resource_mut_or_default::<Assets<Mesh>>()
                .add(AssetManager::create_cube(&scene.renderer.device));

            let side = (args.cubes as f32).sqrt().ceil() as usize;
            let spacing = 1.6;
            let half = side as f32 * spacing * 0.5;
            for i in 0..args.cubes {
                let (x, z) = ((i % side) as f32 * spacing - half, (i / side) as f32 * spacing - half);
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, z)),
                    GlobalTransform::default(),
                    cube.clone(),
                    materials[i % materials.len()].clone(),
                    MeshRenderer::new(),
                    Spin(i as f32 * 0.013),
                ));
            }
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.9),
                intensity: 2.4,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(0.0, half * 0.9 + 10.0, half * 1.1 + 10.0), Vec3::ZERO);
            scene.world.insert_resource(FrameStart(Instant::now()));
            scene.world.insert_resource(Bench {
                args,
                frame: 0,
                last: None,
                periods: Vec::new(),
                latencies: log.clone(),
            });
        })
        .add_update_system(start_frame.in_phase(Phase::PreUpdate))
        .add_update_system(turn.in_phase(Phase::Update))
        // The end of the frame's update: the last hook to run before the sync point.
        .add_update_hook(|world, _state, _dt, _input| measure(world))
        .set_render_stage(Timed { inner: RenderWorld::new(), started: None, latencies });
    app.with_render_thread(!args.inline).run().expect("the application failed to run");
}

fn start_frame(mut start: ResMut<FrameStart>) {
    start.0 = Instant::now();
}

fn turn(time: Res<gizmo::core::time::Time>, bench: Res<Bench>, mut cubes: Query<(&Spin, &mut Transform)>) {
    let t = time.elapsed() as f32;
    for (spin, mut transform) in cubes.iter_mut() {
        transform.rotation = Quat::from_rotation_y(t + spin.0) * Quat::from_rotation_x(0.5 * t + spin.0);
    }
    // The rest of a game's update, as plain CPU time.
    if bench.args.work_ms > 0.0 {
        let until = Instant::now() + Duration::from_secs_f64(bench.args.work_ms / 1e3);
        while Instant::now() < until {
            std::hint::spin_loop();
        }
    }
}

fn measure(world: &mut World) {
    let now = Instant::now();
    let Some(mut bench) = world.remove_resource::<Bench>() else { return };
    bench.frame += 1;
    if bench.frame == bench.args.warmup {
        // Measured from here: what was drawn before was warm-up.
        if let Ok(mut l) = bench.latencies.lock() {
            l.clear();
        }
    }
    if bench.frame > bench.args.warmup {
        if let Some(last) = bench.last {
            bench.periods.push((now - last).as_secs_f64() * 1e3);
        }
    }
    bench.last = Some(now);
    if bench.frame >= bench.args.warmup + bench.args.frames {
        let latencies = bench.latencies.lock().map(|l| l.clone()).unwrap_or_default();
        let a = bench.args;
        println!(
            "render_thread mode={} cubes={} work={:.1}ms vsync={} | frame median {:.3} ms p95 {:.3} ms ({:.1} fps) | latency median {:.3} ms p95 {:.3} ms",
            if a.inline { "inline" } else { "threaded" },
            a.cubes,
            a.work_ms,
            a.vsync,
            percentile(&bench.periods, 0.5),
            percentile(&bench.periods, 0.95),
            1e3 / mean(&bench.periods),
            percentile(&latencies, 0.5),
            percentile(&latencies, 0.95),
        );
        world.insert_resource(gizmo::core::window::AppExit);
    }
    world.insert_resource(bench);
}

fn percentile(samples: &[f64], p: f64) -> f64 {
    if samples.is_empty() {
        return f64::NAN;
    }
    let mut s = samples.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[((s.len() - 1) as f64 * p).round() as usize]
}

fn mean(samples: &[f64]) -> f64 {
    samples.iter().sum::<f64>() / samples.len().max(1) as f64
}
