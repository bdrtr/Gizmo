//! # Kodla kurulmuş animasyon
//!
//! Kodla kurulmuş bir animasyon: bir gezegen kare bir yörünge çiziyor, çevresinde bir uydu
//! dönüyor, ve uydu bir yandan büyüyüp küçülüyor, saydamlaşıp geri geliyor; gezegenin üstündeki
//! lamba nabız gibi atıyor. Dosyadan hiçbir şey yüklenmiyor — altı eğri de elle yazılıyor. **Seride animasyon çalan ilk demo**, ve bunun bir gerekçesi var: animasyon
//! demolarının çoğu deponun taşımadığı bir glTF istiyor, bu demo ise istemiyor.
//!
//! ## Animasyon modelinin şekli
//!
//! | yetenek | Gizmo |
//! |---|---|
//! | klip | [`AnimationClip`] + [`Track`] |
//! | hedefleme | [`Track::target_name`] — düz **ad** dizesi, yolun karması **değil** |
//! | ne animasyonlanır | dönüşümün üç kanalı (`Translation`/`Rotation`/`Scale`), ve kayıtlı her bileşen alanı: [`Track::property`] ile `"PointLight.intensity"`, `"Material.albedo.w"` — motorun alanları `PropertyRegistry`'de, oyun kendininkini ekler |
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
//! taşıyor. Demo dört dönüşüm izini kuruyor; üstüne iki **özellik izi**: lambanın
//! `PointLight.intensity`'si (`"lamp"`, gezegenin çocuğu — adlı hedef, dönüşüm izi gibi çözülür)
//! ve uydunun malzemesinin alfası, `Material.albedo.w` (yalnız o şerit; renk olduğu gibi kalır —
//! malzeme saydam olmalı ki alfa görünsün).
//!
//! ## Kim sürüyor
//!
//! Motorun karesi: `advance_frame_clocks` (tween'lerle aynı saatle, hem çizilen hem penceresiz
//! karede) her `KeyedAnimationPlayer`'ı ilerletip izlerini dönüşümlere ve alanlara yazıyor. Demo
//! hiçbir sistem eklemiyor. HUD, gezegenin yerel konumu ile dünya matrisinin aynı karede ne kadar ayrı
//! olduğunu ölçmeyi sürdürüyor.
//!
//! ## Sınama
//!
//! `animated_transform --test` pencere açmaz: sahneyi meshsiz, malzemesiz kurar (penceresiz kare
//! GPU istemez), `HeadlessFrames` ile 0,05 s'lik karelerle oynatır ve lambanın parlaklığını sabit
//! karelerde eğrinin değeriyle, gezegenin yerini de çeviri iziyle karşılaştırır.
//!
//! ## Kontroller
//!   * **Boşluk** — duraklat/sürdür · **1/2/3** — hız ×0,25 / ×1 / ×3
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::animation::clip::{AnimationClip, Keyframes, Track};
use gizmo::animation::player::AnimationPlayer;
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::core::query::Query;
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
const LAMP: &str = "lamp";

/// Lambanın parlaklık anahtarları, saniye başına bir: nabız.
const PULSE: [f32; 5] = [0.5, 6.0, 0.5, 6.0, 0.5];

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Animated Transform", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            gizmo::animation::register(scene.world);

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
                // Saydam: alfa izi (`Material.albedo.w`) ancak saydam bir malzemede görünür.
                Material::new(white.clone()).with_pbr(Vec4::new(0.95, 0.55, 0.20, 1.0), 0.4, 0.0).with_transparent(true),
                MeshRenderer::new(),
                EntityName(SATELLITE.to_string()),
            ));
            scene.world.add_child(orbit, satellite);

            // Gezegenin üstünde, onunla gezen lamba; parlaklığını klip sürüyor.
            let lamp = scene.world.spawn_bundle(PointLightBundle {
                position: Vec3::new(0.0, 0.9, 0.0),
                color: Vec3::new(1.0, 0.8, 0.5),
                intensity: PULSE[0],
                radius: 4.0,
                ..Default::default()
            });
            scene.world.add_component(lamp, EntityName(LAMP.to_string()));
            scene.world.add_child(planet, lamp);

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
        // Animasyonu motorun karesi sürüyor; burası yalnız duraklatma ve hız.
        .add_update_system(controls.in_phase(Phase::Update))
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

/// Klibin altı eğrisi.
///
/// Uydunun iki kanalı var (ölçek ve dönüş), yani iki ayrı [`Track`], ikisi de `"satellite"`'i
/// hedefliyor. `Keyframes` kanal başına bir varyant olduğu için başka türlüsü zaten yazılamaz.
/// Son ikisi özellik izi: lambanın parlaklığı ve uydu malzemesinin alfası.
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
        // Lamba nabız gibi atıyor: bir bileşen alanı, adıyla.
        Track::property(LAMP, "PointLight.intensity", vec![0.0, 1.0, 2.0, 3.0, 4.0], Keyframes::Scalar(PULSE.to_vec()))
            .expect("lamba izi geçerli"),
        // Uydu saydamlaşıp geri geliyor: albedonun yalnız alfa şeridi.
        Track::property(SATELLITE, "Material.albedo.w", vec![0.0, 2.0, 4.0], Keyframes::Scalar(vec![1.0, 0.25, 1.0]))
            .expect("uydu alfa izi geçerli"),
    ];
    clip
}

/// `--test`: aynı hiyerarşi ve aynı klip, mesh ve malzeme olmadan (onlar GPU ister), penceresiz
/// karelerle. Lambanın parlaklığı sabit karelerde eğrinin kendisi olmalı, gezegen de çeviri izinin
/// dediği yerde. Malzemenin alfası burada sınanamaz — malzeme yok —; onu motorun kendi testleri
/// (`advance_frame_clocks_animates_point_light_and_material`) tutuyor.
fn self_test() -> bool {
    let mut world = World::new();
    gizmo::animation::register(&mut world);
    let planet = world.spawn_bundle((Transform::new(Vec3::new(1.0, 0.0, 1.0)), GlobalTransform::default(), EntityName(PLANET.to_string())));
    let lamp = world.spawn_bundle(PointLightBundle { position: Vec3::new(0.0, 0.9, 0.0), intensity: PULSE[0], ..Default::default() });
    world.add_component(lamp, EntityName(LAMP.to_string()));
    world.add_child(planet, lamp);
    let clip = Arc::new(build_clip());
    let mut player = AnimationPlayer::new().looping(true);
    player.play(clip.clone());
    world.add_component(planet, player);

    let mut run = gizmo::simple::HeadlessFrames::new(world);
    let mut ok = true;
    for frame in 1..=60 {
        run.frame(0.05, |_, _, _, _| {});
        if frame % 10 != 0 {
            continue;
        }
        let t = frame as f32 * 0.05;
        let want = match clip.tracks.iter().find(|tr| tr.property.is_some()).map(|tr| tr.sample(t)) {
            Some(gizmo::animation::clip::InterpolatedValue::Scalar(v)) => v,
            _ => f32::NAN,
        };
        let got = run.world.get::<PointLight>(lamp).map_or(f32::NAN, |l| l.intensity);
        let at = run.world.get::<Transform>(planet).map_or(Vec3::NAN, |p| p.position);
        let want_at = match clip.tracks[0].sample(t) {
            gizmo::animation::clip::InterpolatedValue::Translation(v) => v,
            _ => Vec3::NAN,
        };
        let good = (got - want).abs() < 1.0e-3 && (at - want_at).length() < 1.0e-3;
        println!("animated_transform --test: kare {frame} ({t:.2} s) lamba {got:.4} (eğri {want:.4}), gezegen {at:?} → {}", if good { "tamam" } else { "YANLIŞ" });
        ok &= good;
    }
    ok
}

/// Duraklatma, hız, ve defterin doldurulması.
fn controls(
    mut players: Query<&mut AnimationPlayer>,
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

    for mut player in players.iter_mut() {
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
    for (transform, global, name) in planets.iter() {
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
