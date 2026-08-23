//! # Dünyanın kaydı ve geri yüklenmesi
//!
//! Dünyayı diske yazmak ve geri yüklemek. **F5** sahnenin tamamını bir RON dosyasına kaydediyor,
//! **F9** sahneyi dünyadan silip aynı dosyadan geri kuruyor. Kaydedilen şey kod değil **veri**:
//! dosyayı bir metin düzenleyicide açıp kutuların yerini değiştirebilir, F9 ile sonucu görürsünüz.
//!
//! Bu, `gizmo-scene` alt sistemini gösteren ilk demo — bugüne kadar yalnız editörün içinden
//! kullanılıyordu.
//!
//! ## Motorun modeli: kayıt defteri ne biliyorsa o yazılır
//!
//! Serileştirmenin kapısı [`SceneRegistry`] (yani `ComponentRegistry`): bir bileşen orada kayıtlı
//! değilse dosyaya **yazılmaz**. [`default_scene_registry`] motorun kendi bileşenlerini (Transform,
//! RigidBody, Collider, Velocity, Hitbox…) hazır kaydeder; demo kendi bileşenini de aynı yerden
//! ekliyor — "kendi tipini tanıt" adımı bu.
//!
//! ## Ve bir asimetri, motorun kendi belgesinden
//!
//! GPU `Mesh`'i doğrudan verilen bir varlık `mesh_source: None` ile kaydedilir ve **geometrisiz
//! geri yüklenir** — çünkü dosyaya yazılan şey mesh'in kendisi değil, onu adlandıran anahtar.
//! Bu yüzden bu demonun kutuları `MeshSource("standard_cube")` + [`MaterialSource`] ile kuruluyor:
//! motorun varlık yükleme sistemi her karede bu iki bileşeni görüp GPU tarafını kendisi kuruyor.
//! Kayıt/yükleme çevriminin çalışması için doğru kurulum bu; "spawn edip mesh'i elle vermek"
//! sessizce boş bir sahne geri yükler.
//!
//! ## Kontroller
//!   * **F5** — dünyayı kaydet · **F9** — sahneyi sil ve dosyadan geri yükle
//!
//! Klavyesiz doğrulama için `GIZMO_SCENE_IO_SELFTEST=1`: çevrim 60. ve 120. karelerde kendi
//! kendine koşar. Bu demo o kancayla ölçüldü — dosya 3 069 bayt, üç kutunun üçü de
//! `mesh_source: "standard_cube"`, `material_source` ve demonun kendi `Crate` bileşeniyle
//! (`"(index:1)"`) yazıldı, geri yüklemede renkleri ve yerleriyle döndüler.
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera

use gizmo::core::component::{MaterialSource, MeshSource};
use gizmo::core::input::Input;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::scene::{SceneData, SceneRegistry};
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Demonun kendi bileşeni — kendi tipini kayıt defterine tanıtma adımı.
///
/// Kayıt defterine eklendiği için dosyaya yazılır ve geri yüklenir; eklenmeseydi sessizce
/// kaybolurdu.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
struct Crate {
    /// Kutunun sırası — dosyada okunabilir bir alan olsun diye.
    index: u32,
}
gizmo::core::impl_component!(Crate);

/// Sahnenin yazıldığı dosya. Depoya değil, geçici dizine.
fn scene_path() -> String {
    std::env::temp_dir()
        .join("gizmo_world_serialization.ron")
        .to_string_lossy()
        .into_owned()
}

/// Kaydet/yükle isteklerini ve son sonucu taşıyan kaynak.
#[derive(Default, Clone)]
struct SceneIo {
    save: bool,
    load: bool,
    status: String,
    entities: usize,
}
gizmo::core::impl_component!(SceneIo);

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - World Serialization", 1280, 720)
        .with_simple_scene(|scene, state| {
            // Zemin ve güneş — bunlar da kaydedilir (Transform'ları kayıtlı).
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.5, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 10.0),
                Material::new(white).with_pbr(Vec4::new(0.1, 0.11, 0.13, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.9) * Quat::from_rotation_x(-0.6),
                intensity: 2.5,
                ..Default::default()
            });

            // Kutular: GPU mesh'i ELLE VERİLMİYOR, adıyla isteniyor — kayıt/yükleme çevriminin
            // çalışması için gereken tek fark bu.
            for i in 0..3u32 {
                let x = (i as f32 - 1.0) * 1.6;
                let entity = scene.world.spawn();
                scene.world.add_component(
                    entity,
                    Transform::new(Vec3::new(x, 0.5, 0.0)).with_scale(Vec3::splat(0.4)),
                );
                scene
                    .world
                    .add_component(entity, GlobalTransform::default());
                scene
                    .world
                    .add_component(entity, MeshSource("standard_cube".to_string()));
                scene.world.add_component(
                    entity,
                    MaterialSource {
                        albedo: [0.2 + 0.3 * i as f32, 0.55, 0.9 - 0.25 * i as f32, 1.0],
                        roughness: 0.5,
                        metallic: 0.0,
                        unlit: 0.0,
                        texture_source: None,
                    },
                );
                scene.world.add_component(entity, Crate { index: i });
                scene
                    .world
                    .add_component(entity, EntityName(format!("Kutu {i}")));
            }

            scene.world.insert_resource(SceneIo::default());
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 6.0), Vec3::new(0.0, 0.5, 0.0));
        })
        .add_update_system(scene_io_input.in_phase(Phase::Update))
        // Kaydetme/yükleme dünyayı topluca değiştirir; render kapanışı bunun için uygun yer.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            let request = world.get_resource::<SceneIo>().map(|io| (io.save, io.load));
            if let Some((save, load)) = request {
                if save || load {
                    let registry = demo_registry();
                    let path = scene_path();
                    let (status, entities) = if save {
                        match SceneData::save(world, &path, &registry) {
                            Ok(()) => (
                                format!("kaydedildi → {path}"),
                                std::fs::metadata(&path).map(|m| m.len() as usize).unwrap_or(0),
                            ),
                            Err(e) => (format!("kayıt hatası: {e}"), 0),
                        }
                    } else {
                        despawn_crates(world);
                        match SceneData::load_into(&path, world, &registry) {
                            Ok(()) => ("dosyadan geri yüklendi".to_string(), 0),
                            Err(e) => (format!("yükleme hatası: {e}"), 0),
                        }
                    };
                    if let Some(mut io) = world.get_resource_mut::<SceneIo>() {
                        io.save = false;
                        io.load = false;
                        io.status = status;
                        io.entities = entities;
                    }
                }
            }

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(io) = world.get_resource::<SceneIo>().map(|io| io.clone()) else {
                return;
            };
            gizmo::egui::Area::new("scene_io".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.heading("Dünya kaydı / yüklemesi");
                    ui.label("F5 — kaydet · F9 — sil ve geri yükle");
                    ui.label(format!("dosya: {}", scene_path()));
                    if !io.status.is_empty() {
                        ui.separator();
                        ui.label(&io.status);
                        if io.entities > 0 {
                            ui.label(format!("dosya boyutu: {} bayt", io.entities));
                        }
                    }
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// F5/F9 yalnız bayrak koyar; işi render kapanışı yapar.
fn scene_io_input(mut io: ResMut<SceneIo>, input: Res<Input>, time: Res<Time>) {
    use gizmo::winit::keyboard::KeyCode;
    // `GIZMO_SCENE_IO_SELFTEST=1`: çevrimi tuşa basmadan koşturur — 60. karede kaydeder,
    // 120.'de siler ve geri yükler. Başsız bir kare yakalamasıyla (GIZMO_SCREENSHOT) demoyu
    // doğrulamanın tek yolu bu; klavye orada yok.
    if std::env::var("GIZMO_SCENE_IO_SELFTEST").is_ok() {
        match time.frame() {
            60 => io.save = true,
            120 => io.load = true,
            _ => {}
        }
    }
    if input.is_key_just_pressed(KeyCode::F5 as u32) {
        io.save = true;
    }
    if input.is_key_just_pressed(KeyCode::F9 as u32) {
        io.load = true;
    }
}

/// Motorun hazır kayıt defteri + demonun kendi bileşeni.
fn demo_registry() -> SceneRegistry {
    let mut registry = gizmo::scene::registry::default_scene_registry();
    registry
        .register_serializable::<Crate>("Crate")
        .expect("demo bileşeni 'Crate' kayıt defterinde çakışmamalı");
    registry
}

/// Geri yüklemeden önce sahnedeki kutuları toplar.
///
/// Yalnız `Crate` taşıyanlar siliniyor: zemin, güneş ve kamera yerinde kalsın diye. Dosya
/// hepsini içeriyor, ama bu demoda görülmek istenen şey kutuların gidip geri gelmesi.
fn despawn_crates(world: &mut World) {
    let doomed: Vec<u32> = world
        .query::<&Crate>()
        .map(|q| q.iter().map(|(entity, _)| entity).collect())
        .unwrap_or_default();
    for entity in doomed {
        world.despawn_by_id(entity);
    }
}
