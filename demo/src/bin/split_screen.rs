//! # Bölünmüş ekran ve kamera değiştirme
//!
//! Aynı kareyi ikiye bölüp iki farklı kameradan göstermek. Bunun için her kameraya hedef
//! üstünde bir dikdörtgen (viewport) ve bir çizim sırası verilmesi gerekir; iki kamera
//! aynı kareye, yan yana iki bölgeye çizer.
//!
//! ## Motorda bunun yolu **yok**, ve sebebi tek satır
//!
//! Oyun çizim yolu kareye **bir** kamera seçiyor:
//!
//! ```text
//! cameras.iter().find(|(_, c)| c.primary).or_else(|| cameras.iter().next())
//! ```
//!
//! `active_camera`'nın tamamı bu — işaretli olan, yoksa ilki. Ve seçtiği kamerayla `Camera` tipi
//! üstünde **viewport dikdörtgeni diye bir alan yok**: `fov`, `near`, `far`, `yaw`, `pitch`,
//! `primary`, `exposure`, izdüşüm kipi. Yani "şu kamerayı ekranın sol yarısına çiz" cümlesi
//! kurulamıyor.
//!
//! `default_render_pass` de kamera ya da bölge parametresi almıyor; kendi geçişlerini kendi
//! kuruyor ve hedefin tamamına yazıyor. İki kez çağırmak da işe yaramaz — ikincisi birincinin
//! üstüne yazar, çünkü kırpılacağı bir bölge yok.
//!
//! ## Elde kalan: kamera **değiştirmek**
//!
//! Çoklu kamera tamamen yok değil — `primary` bayrağını çevirmek çalışıyor, ve bu bir oyunun
//! ihtiyaç duyduğu şeylerin bir kısmını karşılıyor (tekrar kamerası, güvenlik kamerası, sinematik
//! geçiş). Karşılamadığı şey aynı anda iki görüş.
//!
//! Demo iki kamera doğuruyor ve **B** ile aralarında geçiş yapıyor; hangisinin seçildiği
//! `active_camera`'nın kuralıyla ölçülüyor.
//!
//! ## İkinci bulgu: basit sahne **her** kamerayı eziyor
//!
//! `with_simple_scene`'in `set_update` kapanışı uçan kameranın durumunu dünyadaki **her** kameraya
//! yazıyor — `primary` filtresi yok. Yani ikinci bir kamera doğurmak yetmiyor, o kamera her kare
//! siliniyor.
//!
//! Ölçümü şöyle çıktı: `primary` bayrağı doğru şekilde ikinci kameraya geçtiği hâlde (etkin varlık
//! 6'dan 7'ye döndü) iki kare **bit bit aynıydı** — ortalama fark 0,00, maksimum 0. Kamera
//! değişmişti, duruşu değişmemişti.
//!
//! Demo bunu `set_render` içinde duruşu geri yazarak telafi ediyor (o kapanış `set_update`'ten
//! sonra koşuyor). Sonra:
//!
//! | | ortalama fark | maks | farklı piksel |
//! |---|---------------|------|---------------|
//! | duruş geri yazılmadan | **0,00** | **0** | %0,0 |
//! | geri yazıldıktan sonra | 24,05 | 147 | **%41,3** |
//!
//! Yani kamera değiştirme **çalışıyor**, ama basit sahneyle birlikte kullanılacaksa ikinci
//! kameranın duruşunu her kare geri koymak gerekiyor. Bu, `with_simple_scene`'in belgesinde
//! yazmıyor.
//!
//! ## Ölçülen: kamera seçimi
//!
//! | | değer |
//! |---|-------|
//! | dünyadaki kamera | 2 |
//! | `primary` işaretli | 1 |
//! | `active_camera`'nın seçtiği | işaretli olan (6 → geçişten sonra 7) |
//!
//! ## Kontroller
//!   * **B** — kameralar arasında geçiş
//!   * **Sağ-tık + fare / WASDQE** — etkin kamera

use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// İkinci kameranın işareti — birincisi basit sahnenin kendi kamerası.
#[derive(Clone, Copy)]
struct SecondCamera;
gizmo::core::impl_component!(SecondCamera);

/// Defter.
#[derive(Default, Clone)]
struct Split {
    /// Dünyadaki toplam kamera sayısı.
    cameras: usize,
    /// `primary` işaretli olanların sayısı.
    primaries: usize,
    /// `active_camera`'nın seçtiği varlık.
    active: Option<u32>,
    /// İkinci kamera mı etkin.
    second_active: bool,
    switch: bool,
    logged: bool,
}
gizmo::core::impl_component!(Split);

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

            scene.world.insert_resource(Split::default());
            // Basit sahnenin kamerası: primary = true.
            scene.spawn_camera(state, Vec3::new(0.0, 3.0, 8.0), Vec3::ZERO);

            // İkinci kamera: aynı sahneye tepeden bakıyor, ama primary DEĞİL.
            let second = scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 9.0, 0.1),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -1.4,
                primary: false,
                ..Default::default()
            });
            scene.world.add_component(second, SecondCamera);

            // Başsız koşuda B'ye basacak kimse yok: `GIZMO_SPLIT_SECOND=1` ikinci kamerayla
            // açıyor. `primary` bayrağı tek olmalı — `active_camera` ilk işaretliyi alıyor.
            if std::env::var("GIZMO_SPLIT_SECOND").is_ok() {
                if let Some(mut q) = scene.world.query_mut::<Mut<Camera>>() {
                    for (entity, mut camera) in q.iter_mut() {
                        camera.primary = entity == second.id();
                    }
                }
            }
        })
        .add_update_system(watch.in_phase(Phase::Update))
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            switch_camera(world);

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(s) = world.get_resource::<Split>().map(|s| s.clone()) else {
                return;
            };
            gizmo::egui::Area::new("split".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(470.0);
                        ui.heading("Bölünmüş ekran");
                        ui.label(format!(
                            "dünyada {} kamera · {} tanesi primary",
                            s.cameras, s.primaries
                        ));
                        ui.label(format!(
                            "etkin: {} ({})",
                            s.active.map(|e| e.to_string()).unwrap_or_else(|| "-".into()),
                            if s.second_active { "tepeden bakan" } else { "sahnenin kamerası" }
                        ));
                        ui.separator();
                        ui.label("Camera tipinde VIEWPORT DİKDÖRTGENİ YOK.");
                        ui.label("active_camera tek satır: ilk primary, yoksa ilk kamera.");
                        ui.label("default_render_pass kamera/bölge parametresi almıyor ve");
                        ui.label("hedefin tamamına yazıyor — iki kez çağırmak üstüne yazar.");
                        ui.separator();
                        ui.label("elde kalan: kamera DEĞİŞTİRMEK (B), aynı anda ikisi değil");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// **B**'yi dinler ve defteri doldurur.
fn watch(cameras: Query<(&Camera, &Transform)>, mut split: ResMut<Split>, input: Res<Input>) {
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyB as u32) {
        split.switch = true;
    }
    split.cameras = cameras.iter().count();
    split.primaries = cameras.iter().filter(|(_, (c, _))| c.primary).count();
}

/// İkinci kameranın duruşu — basit sahne onu her kare ezdiği için her kare geri yazılıyor.
const SECOND_POS: Vec3 = Vec3::new(0.0, 9.0, 0.1);
const SECOND_YAW: f32 = -std::f32::consts::FRAC_PI_2;
const SECOND_PITCH: f32 = -1.4;

/// `primary` bayrağını çevirir ve ikinci kameranın duruşunu geri yazar.
///
/// **Geri yazma zorunlu.** `with_simple_scene`'in `set_update` kapanışı uçan kameranın durumunu
/// dünyadaki **her** kameraya yazıyor — `primary` filtresi yok:
///
/// ```text
/// for (_, (mut trans, mut cam)) in q.iter_mut() {
///     trans.position = state.camera_pos;  trans.rotation = rot;
///     cam.yaw = state.camera_yaw;         cam.pitch = state.camera_pitch;
/// }
/// ```
///
/// Yani ikinci kamera her kare siliniyor. `set_render` `set_update`'ten sonra koştuğu için duruşu
/// burada geri koymak işe yarıyor; ilk yazımda bu yoktu ve iki kamera **bit bit aynı** kareyi
/// veriyordu.
fn switch_camera(world: &mut gizmo::core::World) {
    let asked = world.get_resource::<Split>().map(|s| s.switch).unwrap_or(false);

    // Etkin kamerayı motorun kendi kuralıyla oku — demo kendi kuralını uydurmuyor.
    let active = gizmo::renderer::components::active_camera(world);
    let second = world
        .query::<(&SecondCamera, &Camera)>()
        .and_then(|q| q.iter().next().map(|(entity, _)| entity));
    let second_active = matches!((active, second), (Some(a), Some(s)) if a == s);

    if asked {
        if let Some(mut q) = world.query_mut::<Mut<Camera>>() {
            for (entity, mut camera) in q.iter_mut() {
                let is_second = second == Some(entity);
                // Hedef: ikinci kamera etkinse sahnenin kamerasına dön, değilse ikinciye geç.
                camera.primary = if second_active { !is_second } else { is_second };
            }
        }
    }

    // İkinci kameranın duruşunu geri yaz — basit sahne onu az önce ezdi.
    if let Some(second) = second {
        if let Some(entity) = world.get_entity(second) {
            if let Some(mut q) = world.query_mut::<(Mut<Transform>, Mut<Camera>)>() {
                if let Some((mut transform, mut camera)) = q.get_mut_entity(entity) {
                    transform.position = SECOND_POS;
                    camera.yaw = SECOND_YAW;
                    camera.pitch = SECOND_PITCH;
                }
            }
        }
    }

    if let Some(mut split) = world.get_resource_mut::<Split>() {
        split.active = active;
        split.second_active = second_active;
        split.switch = false;
        if !split.logged {
            split.logged = true;
            gizmo::gizmo_log!(
                Info,
                "kamera {} · primary {} · etkin {:?} (ikinci mi: {})",
                split.cameras,
                split.primaries,
                active,
                second_active
            );
        }
    }
}
