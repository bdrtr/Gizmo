//! # Bölünmüş ekran — `Camera::viewport`
//!
//! Aynı kare iki kameradan: sahnenin uçan kamerası ve sahneye tepeden bakan ikinci bir kamera.
//! Bir kameraya hedefin bir dikdörtgenini vermek yetiyor:
//!
//! ```ignore
//! camera.viewport = Some(Viewport::left_half());
//! ```
//!
//! `default_render_pass` bir kamerada `viewport` görünce kareyi `render_viewports` ile çiziyor:
//! `viewport`'u olan **her** kamera kendi dikdörtgenine, `order` sırasıyla (küçük önce, büyük
//! üstte), sonra HUD bir kez bütün hedefin üstüne. Hiçbir kamerada `viewport` yoksa kare eskisi
//! gibi tek `primary` kameradan çiziliyor.
//!
//! Her görüşün kendine ait olanlar — ve her birinin neden gerektiği `render_viewports`'un
//! belgesinde ölçümüyle:
//!   * **dikdörtgeninin en-boy oranı** — yarım genişlikte bir küp kare kalıyor, ezilmiyor;
//!   * **kendi gönderimi (submit)** — sprite köşeleri, ayıklanmış instance listesi ve post-process
//!     ayarları `queue.write_buffer` ile yükleniyor, ve bu yazmalar kayda değil gönderime göre
//!     sıralanıyor: iki görüş tek encoder'da olsa ikisi de son yazılanı çizerdi;
//!   * **kendi TAA/SSGI geçmişi** — yoksa her kare öbür kameranın resmi bu kameranınkine
//!     harmanlanırdı.
//!
//! Kare başına **bir kez** olanlar (görüş başına değil): dönüşümler, materyaller, animasyon, GPU
//! fiziği, sıvı ve parçacıklar. Görüş başına olsalar bölünmüş ekranda her animasyon iki kat hızlı
//! oynardı.
//!
//! ## Kontroller
//!   * **B** — düzen: yan yana → üst üste → resim içinde resim → tek kamera
//!   * **Sağ-tık + fare / WASDQE** — sahnenin kamerası (primary)
//!
//! Başsız koşuda B'ye basacak kimse yok: `GIZMO_SPLIT_LAYOUT=0..3` düzeni seçerek açıyor.

use gizmo::core::input::Input;
use gizmo::core::query::Mut;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::renderer::components::Viewport;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// İkinci kameranın işareti — birincisi basit sahnenin kendi kamerası.
#[derive(Clone, Copy)]
struct TopDown;
gizmo::core::impl_component!(TopDown);

/// Seçili düzen.
#[derive(Default, Clone, Copy)]
struct Layout(u32);
gizmo::core::impl_component!(Layout);

const LAYOUTS: [&str; 4] = ["yan yana", "üst üste", "resim içinde resim", "tek kamera"];

/// `(sahnenin kamerası, tepeden bakan)` için `(viewport, order)` — düzene göre.
fn layout(index: u32) -> [(Option<Viewport>, i32); 2] {
    match index % 4 {
        0 => [(Some(Viewport::left_half()), 0), (Some(Viewport::right_half()), 0)],
        1 => [(Some(Viewport::top_half()), 0), (Some(Viewport::bottom_half()), 0)],
        // Tam ekran ve sağ üstte küçük bir harita; harita `order` 1 ile üstte.
        2 => [(Some(Viewport::FULL), 0), (Some(Viewport::new(0.72, 0.04, 0.25, 0.3)), 1)],
        // Hiçbir kamerada viewport yok: tek kamera, eski yol.
        _ => [(None, 0), (None, 0)],
    }
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Split Screen", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Yönü belli bir sahne: iki kameranın farklı baktığı görülsün.
            for (i, colour) in [
                Vec4::new(0.90, 0.30, 0.25, 1.0),
                Vec4::new(0.25, 0.80, 0.35, 1.0),
                Vec4::new(0.30, 0.45, 0.95, 1.0),
                Vec4::new(0.90, 0.80, 0.25, 1.0),
            ]
            .into_iter()
            .enumerate()
            {
                let angle = i as f32 / 4.0 * std::f32::consts::TAU;
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(angle.cos() * 3.0, 0.0, angle.sin() * 3.0))
                        .with_scale(Vec3::splat(0.5)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(colour, 0.5, 0.0),
                    MeshRenderer::new(),
                ));
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 20.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.7),
                intensity: 2.4,
                ..Default::default()
            });

            // Basit sahnenin kamerası: primary, uçan kamera onu sürüyor.
            scene.spawn_camera(state, Vec3::new(0.0, 3.0, 8.0), Vec3::ZERO);
            // İkinci kamera: tepeden, primary DEĞİL — uçan kamera ona dokunmuyor.
            let top = scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 9.0, 0.1),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -1.4,
                primary: false,
                ..Default::default()
            });
            scene.world.add_component(top, TopDown);

            let start = std::env::var("GIZMO_SPLIT_LAYOUT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            scene.world.insert_resource(Layout(start));
        })
        .add_update_system(watch.in_phase(Phase::Update))
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            apply_layout(world);
            // Bölünmüş ekran için ayrı bir çağrı yok: viewport'u olan bir kamera varsa bu çağrı
            // kareyi kendisi böler.
            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let index = world.get_resource::<Layout>().map(|l| l.0).unwrap_or(0);
            gizmo::egui::Area::new("split".into())
                .anchor(gizmo::egui::Align2::LEFT_BOTTOM, [12.0, -12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(format!("düzen: {} — B ile değiştir", LAYOUTS[(index % 4) as usize]));
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// **B**: bir sonraki düzen.
fn watch(mut layout: ResMut<Layout>, input: Res<Input>) {
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyB as u32) {
        layout.0 = layout.0.wrapping_add(1);
    }
}

/// Seçili düzenin dikdörtgenlerini iki kameraya yazar.
fn apply_layout(world: &mut gizmo::core::World) {
    let index = world.get_resource::<Layout>().map(|l| l.0).unwrap_or(0);
    let [scene_cam, top_cam] = layout(index);
    let top = world
        .query::<(&TopDown, &Camera)>()
        .and_then(|q| q.iter().next().map(|(entity, _)| entity));
    if let Some(mut q) = world.query_mut::<Mut<Camera>>() {
        for (entity, mut camera) in q.iter_mut() {
            let (viewport, order) = if Some(entity) == top { top_cam } else { scene_cam };
            camera.viewport = viewport;
            camera.order = order;
        }
    }
}
