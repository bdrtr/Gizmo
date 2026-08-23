//! # Bevy'nin `3d/render_to_texture` + `3d/two_passes` + `3d/camera_sub_view` portu
//!
//! Sahneyi pencereye değil bir **dokuya** çizmek, ve aynı kareyi birden fazla kez çizmek.
//!
//! ## Motorda hedef seçilebiliyor — ama ikinci kameraya giden yolda üç engel var
//!
//! | Bevy örneği | Bevy | Gizmo |
//! |-------------|------|-------|
//! | `render_to_texture` | `Camera::target = Image(handle)` | **var** — [`default_render_pass`] bir `TextureView` alıyor |
//! | `two_passes` | iki kamera, iki `order` | **var, ama üç engel aşılarak** (aşağıda) |
//! | `camera_sub_view` | `Camera::sub_camera_view` | **yok** — `Camera`'da bölge alanı yok |
//!
//! Hedef seçimi çalışıyor, çünkü çizim işlevinin imzası hedefi dışarıdan alıyor:
//!
//! ```text
//! default_render_pass(world, encoder, view, renderer)
//! ```
//!
//! İkinci kameradan ikinci bir geçiş ise üç ayrı engelle karşılaşıyor, ve **üçü de ölçüldü**.
//!
//! ### Engel 1: çizim işlevi kamera parametresi almıyor
//!
//! Seçimi `active_camera` yapıyor: "işaretli olan, yoksa ilki". Yani kamerayı değiştirmenin tek
//! yolu geçişler arasında `primary` bayrağını çevirmek. **Bu kısım çalışıyor** — ölçüldü:
//!
//! ```text
//! 1. geçiş (çevrim dışı): active_camera=Some(5)
//! 2. geçiş (pencere):     active_camera=Some(4)
//! ```
//!
//! ### Engel 2: basit sahne HER kameraya tek bir duruş damgalıyor
//!
//! `with_simple_scene`'in güncelleme döngüsü şunu yapıyor, ve `primary` diye bir ayrım
//! gözetmiyor:
//!
//! ```text
//! for (_, (mut trans, mut cam)) in q.iter_mut() {
//!     trans.position = state.camera_pos;
//!     trans.rotation = rot;
//!     cam.yaw = state.camera_yaw;
//!     cam.pitch = state.camera_pitch;
//! }
//! ```
//!
//! Ölçüldü: ikinci kamera (0 · 7,0 · 0,2) konumunda, pitch −1,15 ile doğuruldu; çizim anında
//! okunan değer **`yaw −1,57 pitch −0,21 T(0 · 1,4 · 6,5)`** — yani birincisinin duruşu.
//! **Basit sahne altında ikinci bir kamera kendi yerinde duramıyor.** Çare: duruşu çizimden
//! hemen önce geri yazmak.
//!
//! ### Engel 3 (asıl olan): tek bir uniform tamponu var
//!
//! Duruş geri yazıldıktan sonra bile iki geçiş **aynı görüntüyü** verdi. Sebep boru hattının
//! kendisinde: kamera matrisleri **tek** bir `global_uniform_buffer`'a
//! `renderer.queue.write_buffer(...)` ile yazılıyor. `write_buffer` yazımları encoder'a kaydetme
//! anına göre değil **gönderime** göre sıralanıyor — yani iki geçiş aynı encoder'a kaydedilirse
//! ikisi de son yazılan kamerayı okur.
//!
//! Çare: birinci geçişi **kendi encoder'ına** kaydedip hemen `submit` etmek. Ölçüldü — çevrim
//! dışı hedefin parlaklık profili (2026-08-23, 512×288):
//!
//! | | üst %0-20 | orta %40-60 | alt %80-100 |
//! |---|-----------|-------------|-------------|
//! | tek encoder | 54,04 | 149,46 | 82,95 |
//! | ayrı encoder + hemen submit | **87,36** | **100,24** | **35,61** |
//!
//! Üç bant da tanınmayacak kadar değişiyor: tepeden bakan kamera nihayet kendi görüntüsünü
//! çiziyor. Tek encoder satırı ise pencereninkiyle aynı — yani o koşuda ikinci kamera hiç
//! kullanılmamış.
//!
//! ## Sonuç
//!
//! Dokuya çizim **çalışıyor** ve iki geçiş **mümkün**, ama ikisi de motorun kolay yolunun dışında:
//! `primary` çevir, duruşu geri yaz, ayrı encoder'a kaydedip gönder. Bunların hiçbiri belgelenmiş
//! bir desen değil; üçü de bu demoyu yazarken ölçümle bulundu.
//!
//! Ve `camera_sub_view`'ün istediği şey — hedefin bir alt dikdörtgenine çizmek — hiç yok.
//!
//! ## Kontroller
//!   * `GIZMO_RTT_DUMP=<dizin>` — iki geçişin çıktısını PNG olarak yaz
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::query::Mut;
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Sahnedeki iki kameradan hangisi olduğunu işaretler.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CameraTag(u8);
gizmo::core::impl_component!(CameraTag);

/// Çevrim dışı hedef ve ölçüm durumu.
struct OffscreenTarget {
    texture: wgpu::Texture,
    view: std::sync::Arc<wgpu::TextureView>,
    width: u32,
    height: u32,
}

#[derive(Default, Clone)]
struct RttReport {
    frame: u32,
    /// Çevrim dışı geçiş kaç kez koştu.
    offscreen_passes: u32,
    /// Yazılan dosyalar.
    dumped: Vec<String>,
    size: (u32, u32),
}
gizmo::core::impl_component!(RttReport);

const OFF_W: u32 = 512;
const OFF_H: u32 = 288;

/// Tepedeki kameranın istenen duruşu — her karede geri yazılıyor, çünkü üstüne yazılıyor.
const TOP_POS: Vec3 = Vec3::new(0.0, 7.0, 0.2);
const TOP_YAW: f32 = 0.0;
const TOP_PITCH: f32 = -1.15;

fn main() {
    // Hedef, `set_render` kapatmasının içinde yaşıyor: `wgpu::Texture` `Component` değil.
    let mut offscreen: Option<OffscreenTarget> = None;

    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Render To Texture", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);
            let torus = AssetManager::create_torus(device, 1.0, 0.35, 24, 16);

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(-1.8, 0.0, 0.0))
                    .with_rotation(Quat::from_rotation_x(0.5)),
                GlobalTransform::default(),
                torus,
                Material::new(white.clone()).with_pbr(Vec4::new(0.90, 0.50, 0.25, 1.0), 0.45, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(1.8, 0.0, 0.0)).with_scale(Vec3::splat(0.8)),
                GlobalTransform::default(),
                cube,
                Material::new(white.clone()).with_pbr(Vec4::new(0.30, 0.65, 0.95, 1.0), 0.45, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.4, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 24.0),
                Material::new(white).with_pbr(Vec4::new(0.14, 0.15, 0.17, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.6),
                intensity: 2.6,
                ..Default::default()
            });

            // İki kamera: biri önden (ekran), biri tepeden (çevrim dışı hedef).
            scene.spawn_camera(state, Vec3::new(0.0, 1.4, 6.5), Vec3::ZERO);
            let front = scene
                .world
                .query::<&Camera>()
                .and_then(|q| q.iter().next().map(|(id, _)| id));
            if let Some(id) = front.and_then(|id| scene.world.entity(id)) {
                scene.world.add_component(id, CameraTag(0));
            }
            // İkinci kamera — tepeden bakıyor, ve `primary` DEĞİL.
            let mut top = Camera::new(55_f32.to_radians(), 0.1, 300.0, TOP_YAW, TOP_PITCH, false);
            top.exposure = 1.0;
            scene.world.spawn_bundle((
                Transform::new(TOP_POS),
                GlobalTransform::default(),
                top,
                CameraTag(1),
            ));

            scene.world.insert_resource(RttReport {
                size: (OFF_W, OFF_H),
                ..Default::default()
            });
        })
        .set_render(move |world, _state, encoder, view, renderer, _light_time| {
            // Hedefi ilk karede kur.
            let target = offscreen.get_or_insert_with(|| {
                let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("bevy_render_to_texture::offscreen"),
                    size: wgpu::Extent3d {
                        width: OFF_W,
                        height: OFF_H,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: renderer.config.format,
                    // COPY_SRC olmadan geri okunamaz — `texture_to_png` bunu şart koşuyor.
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = std::sync::Arc::new(
                    texture.create_view(&wgpu::TextureViewDescriptor::default()),
                );
                OffscreenTarget {
                    texture,
                    view,
                    width: OFF_W,
                    height: OFF_H,
                }
            });

            // --- 1. geçiş: TEPEDEKİ kamera, çevrim dışı dokuya ---
            //
            // Kamera seçimi kareye bir tane olduğu için `primary` bayrağını çeviriyoruz.
            // `default_render_pass` kamera parametresi almıyor; seçimi `active_camera` yapıyor.
            set_primary(world, 1);
            // Duruşu GERİ YAZ. Basit sahnenin güncelleme döngüsü her karede TÜM kameralara tek
            // bir duruş damgalıyor (`primary` kontrolü yok), yani ikinci kamera kendi yerinde
            // duramıyor. Bu satır olmadan iki geçiş aynı görüntüyü veriyor — ölçüldü.
            if !std::env::var("GIZMO_RTT_NO_RESTORE").is_ok() {
                restore_top_camera(world);
            }
            log_active(world, "1. geçiş (çevrim dışı)");

            // 1. geçiş KENDİ encoder'ında, ve hemen gönderiliyor.
            //
            // Sebebi ölçülmüş bir kısıt: kamera matrisleri TEK bir `global_uniform_buffer`'a
            // `queue.write_buffer` ile yazılıyor. `write_buffer` yazımları encoder'a kaydetme
            // anına göre değil GÖNDERİME göre sıralanıyor, yani iki geçiş aynı encoder'a
            // kaydedilirse ikisi de SON yazılan kamerayı okur. `GIZMO_RTT_ONE_ENCODER=1` ile
            // eski (yanlış) hâline dönülüp fark görülebiliyor.
            if std::env::var("GIZMO_RTT_ONE_ENCODER").is_ok() {
                gizmo::systems::default_render_pass(world, encoder, &target.view, renderer);
            } else {
                let mut first = renderer
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("bevy_render_to_texture::pass1"),
                    });
                gizmo::systems::default_render_pass(world, &mut first, &target.view, renderer);
                renderer.queue.submit(std::iter::once(first.finish()));
            }

            // --- 2. geçiş: ÖNDEKİ kamera, pencereye ---
            set_primary(world, 0);
            log_active(world, "2. geçiş (pencere)");
            gizmo::systems::default_render_pass(world, encoder, view, renderer);

            let mut dumped = Vec::new();
            let frame = world
                .get_resource::<RttReport>()
                .map(|r| r.frame)
                .unwrap_or(0);
            if let (Ok(dir), 60) = (std::env::var("GIZMO_RTT_DUMP"), frame) {
                let path = std::path::Path::new(&dir).join("rtt_offscreen.png");
                match gizmo::renderer::capture::texture_to_png(
                    &renderer.device,
                    &renderer.queue,
                    &target.texture,
                    &path,
                ) {
                    Ok(()) => {
                        gizmo::gizmo_log!(
                            Info,
                            "çevrim dışı hedef yazıldı: {} ({}x{})",
                            path.display(),
                            target.width,
                            target.height
                        );
                        dumped.push(path.display().to_string());
                    }
                    Err(e) => gizmo::gizmo_log!(Warning, "çevrim dışı hedef yazılamadı: {e}"),
                }
            }

            if let Some(mut report) = world.get_resource_mut::<RttReport>() {
                report.frame += 1;
                report.offscreen_passes += 1;
                report.dumped.extend(dumped);
            }
        })
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<RttReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("rtt".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(440.0);
                        ui.heading("Dokuya çizim");
                        ui.label(format!("kare {} · çevrim dışı geçiş {}", r.frame, r.offscreen_passes));
                        ui.label(format!("hedef {}x{}", r.size.0, r.size.1));
                        ui.separator();
                        ui.label("her karede İKİ geçiş:");
                        ui.label("  1) tepeden kamera -> çevrim dışı doku");
                        ui.label("  2) önden kamera   -> pencere");
                        ui.separator();
                        ui.label("default_render_pass hedefi dışarıdan alıyor,");
                        ui.label("ama KAMERAYI almıyor — seçim active_camera'da.");
                        ui.label("o yüzden geçişler arasında primary bayrağı çevriliyor.");
                        ui.separator();
                        ui.label("Camera'da alt-bölge (sub view) alanı YOK.");
                        for line in &r.dumped {
                            ui.monospace(line);
                        }
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Tepedeki kameranın kendi duruşunu geri yazar.
///
/// Basit sahnenin güncelleme döngüsü şunu yapıyor ve `primary` diye bir ayrım gözetmiyor:
///
/// ```text
/// for (_, (mut trans, mut cam)) in q.iter_mut() {
///     trans.position = state.camera_pos;
///     trans.rotation = rot;
///     cam.yaw = state.camera_yaw;
///     cam.pitch = state.camera_pitch;
/// }
/// ```
///
/// Yani dünyadaki HER kamera aynı duruşa çekiliyor. İkinci bir kameranın kendi açısını
/// koruyabilmesinin tek yolu, çizimden hemen önce duruşu geri yazmak.
fn restore_top_camera(world: &mut World) {
    let mut targets: Vec<u32> = Vec::new();
    if let Some(q) = world.query::<(&CameraTag, &Camera)>() {
        for (id, (tag, _)) in q.iter() {
            if tag.0 == 1 {
                targets.push(id);
            }
        }
    }
    if let Some(mut q) = world.query_mut::<(Mut<Camera>, Mut<Transform>)>() {
        for (id, (mut camera, mut transform)) in q.iter_mut() {
            if !targets.contains(&id) {
                continue;
            }
            camera.yaw = TOP_YAW;
            camera.pitch = TOP_PITCH;
            transform.position = TOP_POS;
        }
    }
}

/// Hangi kameranın etkin olduğunu bir kez günlüğe basar.
fn log_active(world: &World, label: &str) {
    let frame = world.get_resource::<RttReport>().map(|r| r.frame).unwrap_or(0);
    if frame != 30 {
        return;
    }
    let active = gizmo::renderer::components::active_camera(world);
    let mut rows = Vec::new();
    if let Some(q) = world.query::<(&CameraTag, &Camera)>() {
        for (id, (tag, cam)) in q.iter() {
            rows.push(format!("id{id}=etiket{} primary={}", tag.0, cam.primary));
        }
    }
    // Kameraların GERÇEK duruşu: yaw/pitch ve iki ayrı konum kaynağı.
    let mut poses = Vec::new();
    if let Some(q) = world.query::<(&CameraTag, &Camera, &Transform, &GlobalTransform)>() {
        for (id, (tag, cam, t, g)) in q.iter() {
            let gp = g.matrix.to_scale_rotation_translation().2;
            poses.push(format!(
                "id{id}/e{}: yaw {:.2} pitch {:.2} T{:?} G{:?}",
                tag.0,
                cam.yaw,
                cam.pitch,
                (t.position.x, t.position.y, t.position.z),
                (gp.x, gp.y, gp.z)
            ));
        }
    }
    gizmo::gizmo_log!(
        Info,
        "{label}: active_camera={:?} · {:?} · {:?}",
        active,
        rows,
        poses
    );
}

/// `primary` bayrağını verilen etikete taşır — kamera seçiminin tek kolu bu.
fn set_primary(world: &mut World, tag: u8) {
    let mut wanted: Vec<(u32, bool)> = Vec::new();
    if let Some(q) = world.query::<(&CameraTag, &Camera)>() {
        for (id, (t, _)) in q.iter() {
            wanted.push((id, t.0 == tag));
        }
    }
    if let Some(mut q) = world.query_mut::<Mut<Camera>>() {
        for (id, mut camera) in q.iter_mut() {
            if let Some((_, want)) = wanted.iter().find(|(wid, _)| *wid == id) {
                camera.primary = *want;
            }
        }
    }
}
