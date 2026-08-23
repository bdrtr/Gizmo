//! # Kodla kurulmuş animasyon
//!
//! Kodla kurulmuş bir animasyon: bir gezegen kare bir yörünge çiziyor, çevresinde bir uydu
//! dönüyor, ve uydu bir yandan büyüyüp küçülüyor. Dosyadan hiçbir şey yüklenmiyor — dört eğri de
//! elle yazılıyor. **Seride animasyon çalan ilk demo**, ve bunun bir gerekçesi var: animasyon
//! demolarının çoğu deponun taşımadığı bir glTF istiyor, bu demo ise istemiyor.
//!
//! ## Animasyon modelinin şekli
//!
//! | yetenek | Gizmo |
//! |---|---|
//! | klip | [`AnimationClip`] + [`Track`] |
//! | hedefleme | [`Track::target_name`] — düz **ad** dizesi, yolun karması **değil** |
//! | ne animasyonlanır | yalnız üç kanal: [`Keyframes`]'in `Translation`/`Rotation`/`Scale`'i; herhangi bir alan animasyonlanamıyor |
//! | çalıcı | [`AnimationPlayer`], tek klip — ağırlıklı düğümlerden kurulu bir karışım grafiği **yok** |
//! | ara değer | [`Interpolation`]: `Linear` / `Step` / `CubicSpline` |
//!
//! **Hedefleme modeli önemli.** Motorun sistemi kökün altındaki ağacı gezip **yalnız yaprak
//! adına** bakıyor (`track.target_name == name.0`), ve bulduğunu ada göre bir haritaya yazıyor.
//! Aynı adı taşıyan iki varlık aynı hedeftir; ikincisi birincisinin üzerine yazar. Hedef *yolun*
//! karmasıyla bulunsaydı `planet/orbit_controller/satellite` ile başka bir daldaki `satellite`
//! iki ayrı hedef olurdu; burada tek hedeftir.
//!
//! **Ve çözümleme bir kez yapılıyor.** Sistemin kendi yorumu bunu söylüyor: harita boşsa çözer,
//! doldu mu bir daha bakmaz. Yani animasyon başladıktan *sonra* doğan bir hedef hiç bulunmaz —
//! hiyerarşinin, çalıcı çalmaya başlamadan önce kurulmuş olması gerekir.
//!
//! ## Kanal başına bir iz
//!
//! Uydu hem dönüyor hem ölçekleniyor, yani tek bir hedefe iki kanal gerekiyor. `Keyframes` kanal
//! başına bir varyant olduğu için iki kanal iki [`Track`] demek, ikisi de aynı `target_name`'i
//! taşıyor. Demo dört izin dördünü de kuruyor.
//!
//! ## Ölçülen: basit sahnede dünya matrisi bir kare geride
//!
//! `gizmo_animation::register()` animasyon sistemini `.before("transform_propagate")` diye
//! sıralıyor. Ama **basit sahnede o etiket yok**: `with_simple_scene` yayılımı çizelgeye değil
//! `set_update` kapanışının içine koyuyor, ve orada koşan bir şeye kısıt bağlanamaz. Kısıt
//! yazılırsa hata değil **uyarı** basılıp düşürülüyor (motorun sıralama sözleşmesi bu; bkz.
//! `transform`).
//!
//! Sonucu HUD ölçüyor: gezegenin yerel konumu ile yayılmış dünya matrisinin ötelemesi aynı karede
//! **0,0086 birim** ayrı çıkıyor. Gezegen kenar başına 2 birim/saniye gittiğine ve kare ~5 ms
//! sürdüğüne göre bu tam olarak **bir karelik gecikme** — yayılım animasyondan önce koşuyor.
//! Gözle görülmez ama ışın atma ya da çarpışma gibi dünya matrisini okuyan bir iş bir kare eski
//! veri görür.
//!
//! ## Kontroller
//!   * **Boşluk** — duraklat/sürdür · **1/2/3** — hız ×0,25 / ×1 / ×3
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::animation::clip::{AnimationClip, Keyframes, Track};
use gizmo::animation::player::AnimationPlayer;
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::renderer::gpu_types::Vertex;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::PI;
use std::sync::Arc;

/// HUD'un okuduğu defter.
#[derive(Default, Clone)]
struct Playback {
    /// Klibin süresi ve o anki konumu.
    duration: f32,
    elapsed: f32,
    playing: bool,
    speed: f32,
    /// Sistemin ada göre çözdüğü hedefler.
    resolved: Vec<String>,
    /// Gezegenin yerel konumu ile yayılmış dünya konumu arasındaki fark. Yayılım animasyondan
    /// ÖNCE koşuyorsa bu sıfır olmaz: dünya matrisi bir kare eskidir.
    propagation_lag: f32,
}
gizmo::core::impl_component!(Playback);

/// Adlar — hem hiyerarşide hem izlerde aynısı kullanılıyor.
const PLANET: &str = "planet";
const ORBIT: &str = "orbit_controller";
const SATELLITE: &str = "satellite";

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Animated Transform", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Animasyon bileşenleri elle kaydediliyor: `register()` bir `&mut Schedule` ister,
            // `with_simple_scene` ise onu vermiyor — sistem aşağıda `add_update_system` ile
            // ekleniyor.
            scene.world.register_component_type::<AnimationPlayer>();
            scene
                .world
                .register_component_type::<gizmo::animation::player::Animated>();

            // Hiyerarşi: gezegen → yörünge denetleyicisi → uydu.
            let planet = scene.world.spawn_bundle((
                Transform::new(Vec3::new(1.0, 0.0, 1.0)),
                GlobalTransform::default(),
                AssetManager::create_sphere(device, 0.5, 18, 32),
                Material::new(white.clone()).with_pbr(Vec4::new(0.30, 0.55, 0.90, 1.0), 0.5, 0.0),
                MeshRenderer::new(),
                EntityName(PLANET.to_string()),
            ));

            // Yörünge denetleyicisi görünmez: işi yalnız çocuğunu döndürmek.
            let orbit = scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                EntityName(ORBIT.to_string()),
            ));
            scene.world.add_child(planet, orbit);

            let satellite = scene.world.spawn_bundle((
                Transform::new(Vec3::new(1.5, 0.0, 0.0)).with_scale(Vec3::splat(0.5)),
                GlobalTransform::default(),
                // Kenarı 0.5 olan bir küp. Motorun `create_cube`'ü boyut
                // parametresi almıyor ve **2 birim** (köşeleri ±1), yani dört kat büyük olurdu;
                // ölçek anahtarlarını bozmamak için küp elle kuruluyor.
                Mesh::from_vertices(device, &cube_vertices(0.5), "animated_transform::sat"),
                Material::new(white.clone()).with_pbr(Vec4::new(0.95, 0.55, 0.20, 1.0), 0.4, 0.0),
                MeshRenderer::new(),
                EntityName(SATELLITE.to_string()),
            ));
            scene.world.add_child(orbit, satellite);

            let clip = Arc::new(build_clip());
            let duration = clip.duration();
            let mut player = AnimationPlayer::new().looping(true);
            player.play(clip);
            scene.world.add_component(planet, player);

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 20.0),
                Material::new(white).with_pbr(Vec4::new(0.11, 0.12, 0.14, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(PointLightBundle {
                position: Vec3::new(0.0, 2.5, 0.0),
                intensity: 9.0,
                radius: 25.0,
                ..Default::default()
            });

            scene.world.insert_resource(Playback {
                duration,
                speed: 1.0,
                playing: true,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(-2.0, 2.5, 5.0), Vec3::ZERO);
        })
        // Animasyon sistemi elle. `register()` buna `.before("transform_propagate")` de ekliyor
        // ama **basit sahnede o etiket yok**: `with_simple_scene` yayılımı çizelgeye değil
        // `set_update` kapanışının içine koyuyor, ve orada koşan bir şeyin etiketi olmaz. Kısıt
        // yazılsaydı hata değil UYARI basılıp düşürülürdü; ölçülen gecikme aşağıda.
        .add_update_system(
            gizmo::animation::system::animation_system
                .in_phase(Phase::Update)
                .label("animation_update"),
        )
        .add_update_system(
            controls
                .in_phase(Phase::Update)
                .after("animation_update"),
        )
        .set_ui(|world, _state, ctx| {
            let Some(playback) = world.get_resource::<Playback>().map(|p| p.clone()) else {
                return;
            };
            gizmo::egui::Area::new("anim".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.set_min_width(400.0);
                    ui.heading("Kodla kurulmuş animasyon");
                    ui.label(format!(
                        "klip {:.2} s · konum {:.2} s · hız x{:.2}",
                        playback.duration, playback.elapsed, playback.speed
                    ));
                    ui.label(if playback.playing {
                        "durum: çalıyor"
                    } else {
                        "durum: duraklatıldı"
                    });
                    ui.separator();
                    ui.label(format!(
                        "sistemin ada göre çözdüğü hedefler ({}):",
                        playback.resolved.len()
                    ));
                    for name in &playback.resolved {
                        ui.label(format!("  {name}"));
                    }
                    ui.separator();
                    ui.label("hedef YOL değil, düz ad — aynı adlı iki varlık aynı hedeftir");
                    ui.label("çözümleme bir kez: sonradan doğan hedef bulunmaz");
                    ui.separator();
                    ui.label(format!(
                        "yerel ile dünya konumu farkı: {:.4}",
                        playback.propagation_lag
                    ));
                    ui.label("  >0 ise yayılım animasyondan ÖNCE koşuyor (bir kare gecikme)");
                    ui.separator();
                    ui.label("boşluk — duraklat · 1/2/3 — hız");
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Klibin dört eğrisi.
///
/// Uydunun iki kanalı var (ölçek ve dönüş), yani iki ayrı [`Track`], ikisi de `"satellite"`'i
/// hedefliyor. `Keyframes` kanal başına bir varyant olduğu için başka türlüsü zaten yazılamaz.
fn build_clip() -> AnimationClip {
    let quarter_turns = vec![
        Quat::IDENTITY,
        Quat::from_axis_angle(Vec3::Y, PI / 2.0),
        Quat::from_axis_angle(Vec3::Y, PI),
        Quat::from_axis_angle(Vec3::Y, PI / 2.0 * 3.0),
        Quat::IDENTITY,
    ];

    let mut clip = AnimationClip::default();
    clip.name = "planet_orbit".to_string();
    clip.tracks = vec![
        // Gezegen kare bir yol çiziyor. Son kare ilkinin aynısı: döngü dikişsiz olsun.
        Track::new(
            PLANET,
            vec![0.0, 1.0, 2.0, 3.0, 4.0],
            Keyframes::Translation(vec![
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(-1.0, 0.0, 1.0),
                Vec3::new(-1.0, 0.0, -1.0),
                Vec3::new(1.0, 0.0, -1.0),
                Vec3::new(1.0, 0.0, 1.0),
            ]),
        )
        .expect("gezegen izi geçerli"),
        // Yörünge denetleyicisi kendi ekseninde tam tur atıyor; uydu onun çocuğu olduğu için
        // yörüngeye giren şey bu dönüş.
        Track::new(
            ORBIT,
            vec![0.0, 1.0, 2.0, 3.0, 4.0],
            Keyframes::Rotation(quarter_turns.clone()),
        )
        .expect("yörünge izi geçerli"),
        // Uydu büyüyüp küçülüyor — yarım saniyede bir.
        Track::new(
            SATELLITE,
            vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0],
            Keyframes::Scale(vec![
                Vec3::splat(0.8),
                Vec3::splat(1.2),
                Vec3::splat(0.8),
                Vec3::splat(1.2),
                Vec3::splat(0.8),
                Vec3::splat(1.2),
                Vec3::splat(0.8),
                Vec3::splat(1.2),
                Vec3::splat(0.8),
            ]),
        )
        .expect("uydu ölçek izi geçerli"),
        // Aynı hedefe ikinci bir kanal.
        Track::new(
            SATELLITE,
            vec![0.0, 1.0, 2.0, 3.0, 4.0],
            Keyframes::Rotation(quarter_turns),
        )
        .expect("uydu dönüş izi geçerli"),
    ];
    clip
}

/// Duraklatma, hız, ve defterin doldurulması.
fn controls(
    mut players: Query<Mut<AnimationPlayer>>,
    planets: Query<(&Transform, &GlobalTransform, &EntityName)>,
    mut playback: ResMut<Playback>,
    input: Res<Input>,
) {
    use gizmo::winit::keyboard::KeyCode;
    let toggle = input.is_key_just_pressed(KeyCode::Space as u32);
    let speed = if input.is_key_just_pressed(KeyCode::Digit1 as u32) {
        Some(0.25)
    } else if input.is_key_just_pressed(KeyCode::Digit2 as u32) {
        Some(1.0)
    } else if input.is_key_just_pressed(KeyCode::Digit3 as u32) {
        Some(3.0)
    } else {
        None
    };

    for (_entity, mut player) in players.iter_mut() {
        if toggle {
            if player.playing {
                player.pause();
            } else {
                player.resume();
            }
        }
        if let Some(speed) = speed {
            player.speed = speed;
        }

        playback.playing = player.playing;
        playback.elapsed = player.elapsed_time;
        playback.speed = player.speed;
        // Sistemin ada göre neyi çözdüğü — hedefleme modelinin görünür hâli.
        let mut resolved: Vec<String> = player.target_entities.keys().cloned().collect();
        resolved.sort();
        playback.resolved = resolved;
    }

    // Yayılımın ne zaman koştuğunun ölçüsü: kökün yerel konumu ile dünya matrisinin
    // ötelemesi aynı karede eşit olmalı. Değilse dünya matrisi bir kare geridedir.
    for (_entity, (transform, global, name)) in planets.iter() {
        if name.0 == PLANET {
            let world = global.matrix.w_axis.truncate();
            playback.propagation_lag = (world - transform.position).length();
        }
    }
}

/// `size` kenar uzunluklu bir küp. Motorda boyutlu küp yapıcısı yok.
fn cube_vertices(size: f32) -> Vec<Vertex> {
    let h = size / 2.0;
    let faces: [([Vec3; 4], Vec3); 6] = [
        ([Vec3::new(-h, h, h), Vec3::new(h, h, h), Vec3::new(h, h, -h), Vec3::new(-h, h, -h)], Vec3::Y),
        ([Vec3::new(-h, -h, -h), Vec3::new(h, -h, -h), Vec3::new(h, -h, h), Vec3::new(-h, -h, h)], Vec3::NEG_Y),
        ([Vec3::new(h, -h, h), Vec3::new(h, -h, -h), Vec3::new(h, h, -h), Vec3::new(h, h, h)], Vec3::X),
        ([Vec3::new(-h, -h, -h), Vec3::new(-h, -h, h), Vec3::new(-h, h, h), Vec3::new(-h, h, -h)], Vec3::NEG_X),
        ([Vec3::new(-h, -h, h), Vec3::new(h, -h, h), Vec3::new(h, h, h), Vec3::new(-h, h, h)], Vec3::Z),
        ([Vec3::new(h, -h, -h), Vec3::new(-h, -h, -h), Vec3::new(-h, h, -h), Vec3::new(h, h, -h)], Vec3::NEG_Z),
    ];
    let uv = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

    let mut vertices = Vec::with_capacity(36);
    for (corners, normal) in faces {
        for &i in &[0usize, 1, 2, 0, 2, 3] {
            vertices.push(Vertex {
                position: [corners[i].x, corners[i].y, corners[i].z],
                normal: [normal.x, normal.y, normal.z],
                tex_coords: uv[i],
                ..Default::default()
            });
        }
    }
    vertices
}
