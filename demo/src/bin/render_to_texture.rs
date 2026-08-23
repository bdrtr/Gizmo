//! # Dokuya çizim ve iki geçiş
//!
//! Sahneyi pencereye değil bir **dokuya** çizmek, ve aynı kareyi birden fazla kez çizmek.
//!
//! ## Motorda hedef seçilebiliyor — ikinci kameraya giden üç engelin üçü de kapandı
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | çizim hedefini seçmek (pencere yerine doku) | **var** — [`default_render_pass`] bir `TextureView` alıyor |
//! | aynı karede iki kameradan iki geçiş | **var** — `SceneView` ile tek encoder (2026-08-23) |
//! | ikinci kameranın kendi gölge kaskadları | **yok** — kaskadlar ve kümeler hâlâ paylaşılıyor |
//! | hedefin bir alt dikdörtgenine çizmek (alt-görüş) | **yok** — `Camera`'da bölge alanı yok |
//!
//! Hedef seçimi çalışıyor, çünkü çizim işlevinin imzası hedefi dışarıdan alıyor:
//!
//! ```text
//! default_render_pass(world, encoder, view, renderer)
//! ```
//!
//! İkinci kameradan ikinci bir geçiş üç ayrı engelle karşılaştı, **üçü de ölçüldü**, ve biri
//! bu demo sayesinde motorda kapandı.
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
//! ### Engel 2: basit sahne HER kameraya tek bir duruş damgalıyordu — **kapatıldı**
//!
//! Bu demo yazılırken bulundu ve motorda düzeltildi (2026-08-23). Basit sahnenin güncelleme
//! döngüsü duruşu `(Transform, Camera)` eşleşen **her** varlığa yazıyordu, hiçbir ayrım
//! gözetmeden:
//!
//! ```text
//! for (_, (mut trans, mut cam)) in q.iter_mut() {   // filtre yok
//! ```
//!
//! Ölçülen sonuç: ikinci kamera (0 · 7,0 · 0,2) konumunda, pitch −1,15 ile doğuruldu; çizim
//! anında okunan değer **`yaw −1,57 pitch −0,21 T(0 · 1,4 · 6,5)`** — yani birincisinin duruşu.
//! İkinci bir kamera kendi yerinde duramıyordu.
//!
//! Artık duruş yalnız `active_camera`'nın seçtiği kameraya yazılıyor — yani renderer'ın kendi
//! kuralına, ikinci bir kopyasına değil. Böylece "oyuncunun uçurduğu kamera" ile "karenin
//! çizildiği kamera" aynı cümle oluyor. `gizmo::simple::simple_scene_update` artık dışa açık ve
//! bir regresyon testi ile kilitli (`the_fly_camera_writes_only_to_the_rendered_camera` —
//! filtreyi geri alınca kırmızıya dönüyor).
//!
//! Demonun taşıdığı geçici çözüm bu yüzden **kaldırıldı**: ikinci kamera artık hiçbir şey
//! yapmadan kendi yerinde duruyor.
//!
//! ### Engel 3 (asıl olan): tek bir uniform tamponu vardı — **kapatıldı**
//!
//! Duruş geri yazıldıktan sonra bile iki geçiş **aynı görüntüyü** verdi. Sebep boru hattının
//! kendisindeydi: kamera matrisleri **tek** bir `global_uniform_buffer`'a
//! `renderer.queue.write_buffer(...)` ile yazılıyordu. `write_buffer` yazımları encoder'a kaydetme
//! anına göre değil **gönderime** göre sıralanıyor — yani iki geçiş aynı encoder'a kaydedilirse
//! ikisi de son yazılan kamerayı okur.
//!
//! O zamanki çare birinci geçişi **kendi encoder'ına** kaydedip hemen `submit` etmekti: kamera
//! başına bir gönderim, belgelenmemiş, ve bir düzlemsel aynanın ya da yansıma probunun istediği
//! kaç görünüm varsa o kadar gönderim demek.
//!
//! **2026-08-23: `SceneView`.** Kendi uniform tamponu ve kendi grup-0 bağlama grubu olan bir
//! görünüm. İki yazımın çarpışacağı yer kalmıyor, o yüzden tek encoder ve tek gönderim yetiyor:
//!
//! ```ignore
//! renderer.scene.views.push(SceneView::new(&renderer.device, &renderer.scene, "ayna"));
//! renderer.scene.active_view = Some(0);
//! default_render_pass(world, encoder, &hedef, renderer);   // 1. kamera
//! renderer.scene.active_view = None;
//! default_render_pass(world, encoder, pencere, renderer);  // 2. kamera, aynı encoder
//! ```
//!
//! Küme tablosu ve ışık dizini paylaşılıyor, çoğaltılmıyor: onlar sahneden türetiliyor, kameradan
//! değil.
//!
//! ### Ölçüldü — ve `SceneView`'ın sınırı da ölçüldü
//!
//! Çevrim dışı hedefin parlaklık profili (2026-08-23, 512×288, kare 90):
//!
//! | kip | üst %0-20 | orta %40-60 | alt %80-100 |
//! |-----|-----------|-------------|-------------|
//! | tek encoder (tuzak) | 54,42 | 139,18 | 84,39 |
//! | ayrı encoder + hemen submit | 88,80 | 101,40 | **63,07** |
//! | **`SceneView`, tek encoder** | 89,70 | 99,02 | **86,14** |
//!
//! Üst ve orta bant düzeliyor — tepeden bakan kamera kendi görüntüsünü çiziyor. **Alt bant
//! düzelmiyor**, ve aradaki 23 puan `SceneView`'ın sınırı.
//!
//! Sınırın sebebini tahmin etmek yerine ölçtüm. `GIZMO_RTT_NO_SHADOW=1` güneşi gölgesiz bir dolgu
//! ışığına (`LightRole::Generic`) çeviriyor, ve o zaman:
//!
//! | kip (gölgesiz) | üst | orta | alt |
//! |----------------|-----|------|-----|
//! | tek encoder (tuzak) | 54,21 | 66,16 | 19,50 |
//! | ayrı encoder + submit | 28,79 | 30,34 | 23,83 |
//! | **`SceneView`, tek encoder** | **30,21** | **30,59** | **24,59** |
//!
//! Üç bant da **1,5 puanın altında** uyuşuyor. Yani `SceneView` gölge yokken ayrı-encoder
//! çözümüyle eşdeğer, ve kalan bütün fark **gölge kaskadlarından** geliyor.
//!
//! Sebep tutarlı: `SceneView` kamera uniform'unu ayırıyor, ama gölge kaskadları ve küme tablosu
//! kameradan **türetilen** ayrı uniform aileleri (`shadow_cascade_uniform_buffers`,
//! `upload_clusters`) ve hâlâ paylaşılıyor. Aynı encoder'da iki geçiş, ikisi de son yazılan
//! kaskadları okuyor — engel 3'ün birebir aynısı, bir seviye aşağıda.
//!
//! Yani madde kapandı ama tamamlanmadı: kamera başına kaskad ve küme ayrı bir iş, ve
//! `docs/CAPABILITY_GAPS.md`'de öyle duruyor.
//!
//! ## Sonuç
//!
//! Dokuya çizim **çalışıyor**, iki geçiş **tek encoder'da mümkün**, ve `primary` çevirmek dışında
//! bir geçici çözüm kalmadı. Motorun `two_cameras_in_one_encoder_render_two_different_frames`
//! testi ikisini birden kilitliyor: görünümsüz iki geçiş **birebir aynı** kareyi vermeli (tuzak
//! hâlâ orada), görünümlü iki geçiş farklı kareler vermeli.
//!
//! Ve hedefin bir alt dikdörtgenine çizmek — alt-görüş — hâlâ hiç yok.
//!
//! ## Kontroller
//!   * `GIZMO_RTT_DUMP=<dizin>` — iki geçişin çıktısını PNG olarak yaz
//!   * `GIZMO_RTT_SCENE_VIEW=1` — `SceneView` ile tek encoder (2026-08-23'ün çözümü)
//!   * `GIZMO_RTT_ONE_ENCODER=1` — 3. engeli geri getir, iki geçiş aynı kareyi versin
//!   * `GIZMO_RTT_NO_SHADOW=1` — güneşi gölgesiz dolgu ışığına çevir (sınırı ölçmek için)
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

    App::<SimpleSceneState>::new("Gizmo Engine - Render To Texture", 1280, 720)
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
                // `GIZMO_RTT_NO_SHADOW=1` güneşi gölgesiz bir dolgu ışığına çeviriyor. Ölçüm
                // için: SceneView kamerayı ayırıyor, ama gölge kaskadları kameradan TÜRETİLEN
                // ayrı bir uniform ailesi. Sorumluyu ayırmanın yolu bu.
                role: if std::env::var("GIZMO_RTT_NO_SHADOW").is_ok() {
                    gizmo::renderer::components::LightRole::Generic
                } else {
                    gizmo::renderer::components::LightRole::Sun
                },
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
                    label: Some("render_to_texture::offscreen"),
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
            log_active(world, "1. geçiş (çevrim dışı)");

            // 1. geçiş KENDİ encoder'ında, ve hemen gönderiliyor.
            //
            // Sebebi ölçülmüş bir kısıt: kamera matrisleri TEK bir `global_uniform_buffer`'a
            // `queue.write_buffer` ile yazılıyor. `write_buffer` yazımları encoder'a kaydetme
            // anına göre değil GÖNDERİME göre sıralanıyor, yani iki geçiş aynı encoder'a
            // kaydedilirse ikisi de SON yazılan kamerayı okur. `GIZMO_RTT_ONE_ENCODER=1` ile
            // eski (yanlış) hâline dönülüp fark görülebiliyor.
            if std::env::var("GIZMO_RTT_SCENE_VIEW").is_ok() {
                // 2026-08-23: engel 3'ün gerçek çözümü. Ayrı bir `SceneView`, yani ayrı bir
                // uniform tamponu — iki yazımın çarpışacağı bir yer kalmıyor, o yüzden TEK
                // encoder ve TEK gönderim yetiyor.
                if renderer.scene.views.is_empty() {
                    let v = gizmo::renderer::pipeline::SceneView::new(
                        &renderer.device,
                        &renderer.scene,
                        "render_to_texture::offscreen_view",
                    );
                    renderer.scene.views.push(v);
                }
                renderer.scene.active_view = Some(0);
                gizmo::systems::default_render_pass(world, encoder, &target.view, renderer);
                renderer.scene.active_view = None;
            } else if std::env::var("GIZMO_RTT_ONE_ENCODER").is_ok() {
                gizmo::systems::default_render_pass(world, encoder, &target.view, renderer);
            } else {
                let mut first = renderer
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("render_to_texture::pass1"),
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
