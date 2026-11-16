//! Ölçüm kipi: `dunya --bench` — pencere açmadan, GPU'da, oyunun kendisini çizerek kare kare
//! nereye zaman gittiğini döker.
//!
//! Oyunun gerçek kurulumu (`setup`, modeller, yüzeyler, gök) ekran dışı bir renderer'la
//! (1280×800); otomatik pilot sınamadaki gibi yürüyor, sonra arabayla sürüyor. Her kare:
//!
//! - **oyun** — oyunun adımı, kamera, HUD, harita (pencereli oyunun `update`'inin sesi dışında hepsi);
//! - **yükleme** — motorun varlık yükleyicisi (ağlar, materyaller, dokular);
//! - **hazırlık+kodlama** — motorun karesi (`default_render_pass`): sahneyi toplama, gruplama,
//!   gölge ve çizim komutlarının kaydı — CPU;
//! - **GPU** — komutlar gönderildikten GPU bitirene dek: kare GPU'da ne kadar sürüyor.
//!
//! Pencere yok, dikey eşitleme yok: kare süresi `oyun + yükleme + hazırlık + GPU`, ve hangisinin
//! büyük olduğu darboğazı söylüyor. Her iki saniyede bir ve sonunda döküyor; `--bench 1800`
//! kare sayısı.

use super::*;
use gizmo::core::profiler::FrameProfiler;
use gizmo::core::time::Time;
use gizmo::renderer::components::RenderStats;
use gizmo::renderer::Renderer;
use gizmo::wgpu;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

#[derive(Default, Clone, Copy)]
struct Sample {
    game: f64,
    load: f64,
    encode: f64,
    gpu: f64,
}

impl Sample {
    fn total(&self) -> f64 {
        self.game + self.load + self.encode + self.gpu
    }
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn summary(samples: &[Sample]) -> String {
    let n = samples.len().max(1) as f64;
    let mean = |f: fn(&Sample) -> f64| samples.iter().map(f).sum::<f64>() / n;
    let worst = samples.iter().map(Sample::total).fold(0.0, f64::max);
    let total = mean(Sample::total);
    format!(
        "{:.0} FPS · kare {total:.1} ms (en kötü {worst:.1}) · oyun {:.1} · yükleme {:.1} · hazırlık+kodlama {:.1} · GPU {:.1} ms",
        1000.0 / total.max(0.001),
        mean(|s| s.game),
        mean(|s| s.load),
        mean(|s| s.encode),
        mean(|s| s.gpu),
    )
}

/// `frames` kare ölçer ve döker; GPU yoksa `false`.
pub fn bench(frames: usize) -> bool {
    pollster::block_on(async {
        if !Renderer::headless_adapter_available().await {
            eprintln!("dunya --bench: GPU yok");
            return false;
        }
        // Hangi GPU'da: renderer'ın istediği gibi, yüksek performanslısı.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            display: None,
        });
        if let Ok(adapter) = instance
            .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })
            .await
        {
            let info = adapter.get_info();
            println!("dunya --bench: {} ({:?}, {:?}), {WIDTH}×{HEIGHT}", info.name, info.backend, info.device_type);
        }
        let mut renderer = Renderer::new_headless(WIDTH, HEIGHT, None).await;
        {
            let mut assets = renderer.asset_manager.write().unwrap_or_else(|p| p.into_inner());
            for (path, bytes) in modeller::EMBEDDED {
                assets.embedded_assets.insert(path.to_string(), std::borrow::Cow::Borrowed(bytes));
            }
        }
        let mut world = World::new();
        let ground = renderer.add_surface_material("dunya_arazi", modeller::GROUND_WGSL).unwrap_or_else(|e| panic!("{e}"));
        world.insert_resource(modeller::ground_look(ground));
        let water = renderer.add_surface_material("dunya_su", modeller::WATER_WGSL).unwrap_or_else(|e| panic!("{e}"));
        world.insert_resource(modeller::water_look(water));
        // Pencereli oyunun kurduğu gibi (`with_simple_scene`'in varsayılanları): bunlar kapalı.
        renderer.gpu_physics = None;
        renderer.gpu_fluid = None;
        renderer.ssr = None;
        renderer.ssgi = None;
        // `DUNYA_BENCH_OFF=ssao,taa,…`: bu efektler kapalı — her birinin payını görmek için.
        let off = std::env::var("DUNYA_BENCH_OFF").unwrap_or_default();
        let off = |name: &str| off.split(',').any(|o| o.trim() == name);
        if off("ssao") {
            renderer.ssao = None;
        }
        if off("taa") {
            renderer.taa = None;
        }
        if off("volumetric") {
            renderer.volumetric = None;
        }
        if off("smaa") {
            renderer.smaa = None;
            renderer.fxaa = None;
        }
        if off("cas") {
            renderer.cas = None;
        }
        if off("decal") {
            renderer.decal = None;
        }
        if off("exposure") {
            renderer.auto_exposure = None;
        }
        if off("bloom") {
            renderer.bloom_intensity = 0.0;
        }
        println!(
            "dunya --bench: açık: ssao {} · taa {} · volumetrik {} · smaa {} · fxaa {} · cas {} · çıkartma {} · pozlama {} · bloom {:.2} · ssr {} · ssgi {}",
            renderer.ssao.is_some(),
            renderer.taa.is_some(),
            renderer.volumetric.is_some(),
            renderer.smaa.is_some(),
            renderer.fxaa.is_some(),
            renderer.cas.is_some(),
            renderer.decal.is_some(),
            renderer.auto_exposure.is_some(),
            renderer.bloom_intensity,
            renderer.ssr.is_some(),
            renderer.ssgi.is_some(),
        );
        let font = renderer.default_font();
        let mut game = setup(&mut world, font, true);
        // `DUNYA_BENCH_NOMAP=1`: küçük harita kapalı — ikinci kameranın payını görmek için.
        if std::env::var("DUNYA_BENCH_NOMAP").is_ok_and(|v| !v.is_empty() && v != "0") {
            update_map(&mut world, &mut game, true);
        }
        world.insert_resource(gizmo::renderer::Gizmos::default());
        world.insert_resource(Time::new());
        world.insert_resource(gizmo::core::window::WindowInfo { width: WIDTH as f32, height: HEIGHT as f32 });
        let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dunya bench frame"),
            size: wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: renderer.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let dt = 1.0 / 60.0;
        let mut samples = Vec::with_capacity(frames);
        let mut window: Vec<Sample> = Vec::new();
        let drive_from = frames * 2 / 3;
        for frame in 0..frames {
            let t0 = std::time::Instant::now();
            if let Some(mut time) = world.get_resource_mut::<Time>() {
                time.update(dt);
            }
            let controls = autopilot(&world, &game, frame >= drive_from);
            if frame == drive_from && game.driving.is_none() {
                let me = focus(&world, &game);
                if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(game.cars[0]) {
                    t.position = Vec3::new(me.x + 2.0, height(me.x + 2.0, me.z) + 1.4, me.z);
                }
                enter_or_leave(&mut world, &mut game);
            }
            step(&mut world, &mut game, dt, controls);
            update_map(&mut world, &mut game, false);
            place_lights_and_camera(&mut world, &mut game, dt);
            draw_hud(&mut world, &game);
            world.insert_resource(game);
            let t1 = std::time::Instant::now();
            {
                let manager = renderer.asset_manager.clone();
                let mut assets = manager.write().unwrap_or_else(|p| p.into_inner());
                gizmo::renderer::asset_loading::run_asset_loading_system(
                    &mut world,
                    &renderer.device,
                    &renderer.queue,
                    &renderer.scene.texture_bind_group_layout,
                    &mut assets,
                );
            }
            let t2 = std::time::Instant::now();
            frame_settings(&world, &mut renderer);
            let mut encoder = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("dunya bench") });
            gizmo::systems::render::default_render_pass(&mut world, &mut encoder, &view, &mut renderer);
            let t3 = std::time::Instant::now();
            renderer.queue.submit(Some(encoder.finish()));
            let _ = renderer.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
            let t4 = std::time::Instant::now();
            game = world.remove_resource::<Game>().expect("oyun");
            if let Some(mut p) = world.get_resource_mut::<FrameProfiler>() {
                p.end_frame();
            }
            let s = Sample { game: ms(t1 - t0), load: ms(t2 - t1), encode: ms(t3 - t2), gpu: ms(t4 - t3) };
            // İlk saniye ölçülmüyor: varlıklar yükleniyor, boru hatları derleniyor.
            if frame >= 60 {
                samples.push(s);
                window.push(s);
            }
            if window.len() == 120 {
                let what = if game.driving.is_some() { "arabada" } else { "yaya" };
                println!("dunya --bench [{what}, kare {frame}]: {}", summary(&window));
                window.clear();
            }
        }
        world.insert_resource(game);
        println!("dunya --bench: toplam {} kare: {}", samples.len(), summary(&samples));
        if let Some(s) = world.get_resource::<RenderStats>() {
            println!(
                "dunya --bench: son kare: çizim çağrısı {} · üçgen {:.2} M · örnek {} · gölge {} geçiş, {} örnek",
                s.draw_calls,
                s.triangles as f64 / 1.0e6,
                s.instances,
                s.shadow_passes,
                s.shadow_instances
            );
        }
        if let Some(p) = world.get_resource::<FrameProfiler>() {
            let n = p.history().len().min(600);
            let parts = ["render.sync", "render.models", "render.animation", "render.effects", "render.viewports", "render.ahead", "render.mirrors", "render.camera"]
                .iter()
                .map(|k| format!("{} {:.2}", k.trim_start_matches("render."), p.avg_scope_ms(k, n)))
                .collect::<Vec<_>>()
                .join(" · ");
            println!("dunya --bench: motorun karesi (CPU, ms): {parts}");
            // Görünüm başına (ana görünüm ve harita birlikte, kare başına toplam).
            let per_frame = |name: &str| p.history().iter().rev().take(n).flat_map(|f| f.scopes.iter()).filter(|s| s.name == name).map(|s| s.duration_ms()).sum::<f64>().max(0.0) / n as f64;
            let views = ["view.setup", "view.batch", "view.shadows", "view.geometry", "view.forward", "view.ui", "view.post"]
                .iter()
                .map(|k| format!("{} {:.2}", k.trim_start_matches("view."), per_frame(k)))
                .collect::<Vec<_>>()
                .join(" · ");
            println!("dunya --bench: görünümlerin kodlaması (CPU, kare başına ms): {views}");
        }
        if let Some(r) = olcum::report(&world) {
            println!("dunya --bench: oyunun adımları: {}", r.lines().skip(1).collect::<Vec<_>>().join(" | "));
        }
        true
    })
}
