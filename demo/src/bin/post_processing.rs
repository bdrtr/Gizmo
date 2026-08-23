//! # Oyunun kendi son-işlem geçişi
//!
//! Motorun sunmadığı bir tam-ekran etkisi eklemek: tarama çizgileri, renk ayrımı, kendi
//! bulanıklığınız. Klasik yolu, karenin üstüne kendi gölgelendiricinizle bir geçiş daha çizmek.
//!
//! ## Motorda kapı **var**
//!
//! [`App::set_render`] tam bunun için: `&mut wgpu::CommandEncoder`, hedef `&wgpu::TextureView` ve
//! `&mut Renderer` ham hâlleriyle veriliyor. Kendi belgesi de öyle diyor — *"the hook for a pass
//! the engine does not have"*.
//!
//! Bu demo o kapıyı kullanıyor: kendi WGSL'ini derliyor, kendi boru hattını kuruyor, ve motorun
//! karesinin üstüne alfa karışımlı tarama çizgileri çiziyor. Yaklaşık altmış satır.
//!
//! ## Ama eklenen geçiş, üstüne çizdiği kareyi **okuyamıyor**
//!
//! Sunum yüzeyi `RENDER_ATTACHMENT` (ve destekleniyorsa `COPY_SRC`) ile yapılandırılıyor —
//! **`TEXTURE_BINDING` yok**. Yani hedef doku bir gölgelendiriciye bağlanamıyor.
//!
//! Sonucu şu: kendi geçişiniz kareye **karışabiliyor** ama karenin komşuluğunu okuyan hiçbir şey
//! yapamıyor. Kendi FXAA'nız, kendi radyal bulanıklığınız, kendi ton eşlemeniz, karenin
//! histogramı — hiçbiri yazılamıyor.
//!
//! Yan kapı `COPY_SRC`: kareyi başka bir dokuya kopyalayıp onu örneklemek. Motorun kendi
//! `capture::texture_to_png`'si de bunu yapıyor, ve kendi belgesi onu "bir tanılama" diye anıp
//! "okuma dönene kadar bloke oluyor, bir kare maliyeti var" diyor.
//!
//! ## Ölçüldü — kendi geçişimiz gerçekten kareye giriyor
//!
//! Aynı sahne iki kez, tek fark kendi geçişimizin çalışıp çalışmaması (2026-08-23, 948×1028,
//! sol yarı, HUD altı):
//!
//! | | değer |
//! |---|-------|
//! | farklı piksel | **%38,67** |
//! | en büyük kanal farkı | 44 |
//!
//! Ve imza tartışmasız. Ardışık on iki satırın parlaklığı:
//!
//! | | satırlar |
//! |---|----------|
//! | kapalı | 113,3 · 115,1 · 116,9 · **118,5** · 120,3 · 121,9 · 123,4 · **125,0** · **126,4** · 127,8 · 129,0 · **130,3** |
//! | açık | 113,3 · 111,8 · 95,6 · **118,5** · 120,3 · 115,8 · 101,3 · **125,0** · **126,4** · 117,8 · 106,8 · **130,3** |
//!
//! Kapalıyken parlaklık düzgün tırmanıyor; açıkken araya karartılmış satırlar giriyor. Kalın
//! yazılanlar iki koşuda da **birebir aynı** — yani geçiş `LoadOp::Load` ile motorun karesini
//! koruyup yalnız kendi piksellerini karıştırıyor, üstüne baştan yazmıyor.
//!
//! Yani kapı gerçekten açık: kendi WGSL'i, kendi boru hattı, kendi geçişi, altmış satır.
//!
//! ## Ama boru hattının bağlama grubu **boş** — ve olmak zorunda
//!
//! Yukarıdaki geçişin `bind_group_layouts: &[]` olduğuna dikkat: bağlanacak bir şey yok.
//! Gölgelendiricinin tek girdisi kendi piksel konumu. Altındaki kareyi okuyamıyor, çünkü sunum
//! yüzeyinde `TEXTURE_BINDING` yok.
//!
//! Bu yüzden tarama çizgisi yazılabiliyor (yalnız konuma bakıyor) ama kendi FXAA'nız
//! yazılamıyor (komşu pikselleri okur), kendi radyal bulanıklığınız yazılamıyor, karenin
//! histogramı alınamıyor — [`auto_exposure`](../auto_exposure/index.html) demosunun bulduğu
//! eksik sensörün kökü de bu.
//!
//! ## Kontroller
//!   * `GIZMO_PP=0` — kendi geçişimizi kapat
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Tam-ekran üçgen + alfa karışımlı tarama çizgileri. Motorda böyle bir etki yok; bu demo onu
/// kendi ekliyor.
const SCANLINES_WGSL: &str = r#"
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Üç köşeyle bütün ekranı kaplayan klasik üçgen.
    let x = f32(i32(i) / 2) * 4.0 - 1.0;
    let y = f32(i32(i) & 1) * 4.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // Tek girdi kendi piksel konumumuz. Altındaki kareyi OKUYAMIYORUZ — sunum yüzeyi
    // TEXTURE_BINDING taşımıyor, yani bir gölgelendiriciye bağlanamıyor.
    let line = sin(pos.y * 1.6);
    let dark = smoothstep(0.0, 1.0, line) * 0.35;
    return vec4<f32>(0.0, 0.0, 0.0, dark);
}
"#;

/// Kendi geçişimizin GPU nesneleri. `set_render` kapatmasının içinde yaşıyor.
struct Scanlines {
    pipeline: wgpu::RenderPipeline,
}

fn main() {
    let on = !matches!(std::env::var("GIZMO_PP").as_deref(), Ok("0"));
    let mut pass: Option<Scanlines> = None;

    App::<SimpleSceneState>::new("Gizmo Engine - Post Processing", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let sphere = AssetManager::create_sphere(device, 1.0, 28, 40);
            for i in 0..5 {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new((i as f32 - 2.0) * 2.3, 0.4, 0.0))
                        .with_scale(Vec3::splat(0.85)),
                    GlobalTransform::default(),
                    sphere.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.85, 0.55 + i as f32 * 0.08, 0.30, 1.0),
                        0.35,
                        0.1,
                    ),
                    MeshRenderer::new(),
                ));
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 30.0),
                Material::new(white).with_pbr(Vec4::new(0.28, 0.29, 0.33, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.6),
                intensity: 3.0,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(0.0, 1.2, 8.0), Vec3::new(0.0, 0.2, 0.0));
        })
        .set_render(move |world, _state, encoder, view, renderer, _lt| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            gizmo::systems::default_render_pass(world, encoder, view, renderer);

            if !on {
                return;
            }

            // --- Motorun bitirdiği karenin ÜSTÜNE kendi geçişimiz ---
            let sl = pass.get_or_insert_with(|| {
                let shader = renderer
                    .device
                    .create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("post_processing::scanlines"),
                        source: wgpu::ShaderSource::Wgsl(SCANLINES_WGSL.into()),
                    });
                let layout = renderer
                    .device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("post_processing::layout"),
                        // Bağlama grubu YOK — bağlayacak bir şey de yok: kareyi okuyamıyoruz.
                        bind_group_layouts: &[],
                        immediate_size: 0,
                    });
                let pipeline =
                    renderer
                        .device
                        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                            label: Some("post_processing::scanlines"),
                            layout: Some(&layout),
                            vertex: wgpu::VertexState {
                                module: &shader,
                                entry_point: Some("vs_main"),
                                compilation_options: Default::default(),
                                buffers: &[],
                            },
                            fragment: Some(wgpu::FragmentState {
                                module: &shader,
                                entry_point: Some("fs_main"),
                                compilation_options: Default::default(),
                                targets: &[Some(wgpu::ColorTargetState {
                                    format: renderer.config.format,
                                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                                    write_mask: wgpu::ColorWrites::ALL,
                                })],
                            }),
                            primitive: wgpu::PrimitiveState::default(),
                            depth_stencil: None,
                            multisample: wgpu::MultisampleState::default(),
                            multiview_mask: None,
                            cache: None,
                        });
                Scanlines { pipeline }
            });

            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post_processing::pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Load: motorun çizdiğini koruyup üstüne karıştırıyoruz.
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&sl.pipeline);
            rp.draw(0..3, 0..1);
        })
        .set_ui(move |_world, _state, ctx| {
            gizmo::egui::Area::new("pp".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(420.0);
                        ui.heading("Kendi son-işlem geçişin");
                        ui.label(format!("kendi geçişimiz: {}", if on { "açık" } else { "kapalı" }));
                        ui.separator();
                        ui.label("set_render ham encoder + hedef view veriyor —");
                        ui.label("kendi boru hattını kurup üstüne çizebiliyorsun.");
                        ui.separator();
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 160, 80),
                            "ama üstüne çizdiğin kareyi OKUYAMIYORSUN",
                        );
                        ui.label("sunum yüzeyinde TEXTURE_BINDING yok.");
                        ui.label("kendi FXAA'n, radyal bulanıklığın, histogramın: yazılamaz.");
                        ui.separator();
                        ui.label("yan kapı COPY_SRC — kopyala, sonra örnekle.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
