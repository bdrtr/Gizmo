use gizmo::prelude::*;

struct PendingDecal {
    position: Vec3,
    rotation: Quat,
}

struct DemoState {
    car_entity: gizmo::core::Entity,
    wheel_entities: [gizmo::core::Entity; 4],
    suspension_entities: [gizmo::core::Entity; 4],
    camera_offset: Vec3,
    post_process: gizmo::renderer::gpu_types::PostProcessUniforms,
    pending_particles: std::cell::RefCell<Vec<gizmo::renderer::gpu_particles::GpuParticle>>,
    show_car: bool,
    show_physics_debug: bool,
    update_wheel_radius: Option<f32>,
    tire_track_bg: std::sync::Arc<wgpu::BindGroup>,
    decals: Vec<gizmo::core::Entity>,
    decal_index: usize,
    pending_decals: std::cell::RefCell<Vec<PendingDecal>>,
    engine_audio_id: Option<u64>,
    audio_manager: Option<gizmo::prelude::AudioManager>,
}

mod setup;
mod update;
mod render;
mod ui;

fn main() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::Layer;
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_filter(tracing_subscriber::filter::LevelFilter::INFO),
            )
            .with(tracing_tracy::TracyLayer::default()),
    )
    .expect("Set global default subscriber failed");
    let mut app = App::<DemoState>::new("Gizmo Engine - Hill Climb Racing 2D", 1280, 720)
        .set_setup(setup::setup)
        .set_update(update::update)
        .set_render(render::render)
        .set_ui(ui::ui_debug_panel);

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--record") {
        println!("== OYUN KAYDI BASLADI ==");
        app = app.start_recording();
    } else if let Some(idx) = args.iter().position(|arg| arg == "--playback") {
        if idx + 1 < args.len() {
            println!("== OYUN KAYDI OYNATILIYOR: {} ==", args[idx + 1]);
            app = app.start_playback(&args[idx + 1]);
        }
    }

    app.run().expect("uygulama çalıştırılamadı");
}
