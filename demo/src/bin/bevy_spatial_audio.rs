//! # Bevy'nin `audio/spatial_audio_3d` örneğinin Gizmo Engine portu
//!
//! Sesin **nereden geldiğini** duymak: dönen bir kaynak, iki kulaklı bir dinleyici, ve mesafeyle
//! sönümlenen bir ses. **Seride sesi çalan ilk demo.**
//!
//! ## Ses dosyası yok, o yüzden ton çalışma anında üretiliyor
//!
//! Bevy `sounds/Windless Slopes.ogg`'yi yüklüyor; bu depo hiç ses varlığı taşımıyor. Demo bu
//! yüzden bellekte bir **WAV kuruyor** (44 baytlık RIFF başlığı + 440 Hz sinüs) ve
//! [`AudioManager::load_sound_bytes`]'a veriyor — motorun WASM/gömülü varlık yolu. Diskten tek
//! bayt okunmuyor.
//!
//! ## Üç fark, üçü de motorun tasarımından
//!
//! **1. Dinleyici ayrı bir varlık değil, KAMERA.** Bevy'de `SpatialListener` bileşenli bağımsız
//! bir varlık var ve örnekte ok tuşlarıyla kameradan bağımsız hareket ediyor. Gizmo'nun
//! [`listener`] fonksiyonu birincil kamerayı bulup konumunu, hızını ve sağ vektörünü okuyor —
//! yani dinleyiciyi kameradan ayırmanın yolu yok. Kamerayı gezdirmek dinleyiciyi gezdirmektir.
//!
//! **2. Kulak aralığı ayarlanamıyor.** Bevy'nin `SpatialListener::new(gap)`'i aralığı parametre
//! alıyor (örnekte 4,0 m). Motorda bu bir sabit: `Listener::EAR_DISTANCE = 0,2` m — gerçek bir
//! kafa kadar.
//!
//! **3. Uzamsal ses sistemi kendiliğinden koşmuyor.** Motorun kendi belgesi *"`DefaultPlugins` bunu
//! otomatik kaydetmez"* diyor: [`audio_frame`] oyunun kendi çizelgesine elle eklenmeli. Bu demo
//! onu `set_render` kapanışında çağırıyor, çünkü `&mut World` isteyen tek kanca orası.
//!
//! Buna karşılık motorun Bevy'nin bu örneğinde olmayan bir şeyi var: **Doppler**. Uzamsal sistem
//! kaynağın ve dinleyicinin `Velocity`'sini okuyup perdeyi kaydırıyor.
//!
//! ## `spatial_gain` duyulan ses değildir — ve bunu bilmeden okumak yanlış sayı verir
//!
//! Motorun [`spatial_gain`]'i **1'in üstünde** değerler döndürüyor, ve bu bir hata değil: rodio
//! sesi kendi başına **ters-kare** ile (`min(1/d², 1)`) sönümlüyor, motor da bu terimi
//! **önceden iptal edip** kendi doğrusal konisini bırakıyor. Yani fonksiyonun döndürdüğü şey
//! sink'e yazılacak sayı; kulağa giden oran o koninin kendisi.
//!
//! Ölçülen bir kare: mesafe 10,24 m, `max_distance` 20 m → koni `1 − 10,24/20 = 0,488`, rodio
//! terimi `10,24² = 104,9`, fonksiyonun dönüşü `0,488 × 104,9 = 51,17`. HUD ikisini de ayrı ayrı
//! gösteriyor, çünkü "51" rakamını kazanç sanmak sesin elli kat açıldığını sanmak olurdu.
//!
//! ## Bu port bir hata buldu: kulaklar terstiydi
//!
//! Demo dinleyicinin kullandığı sağ vektörünü motorun kendi `Camera::get_right()`'ıyla
//! karşılaştırıyor. İlk ölçüm **her karede `−1,00`** verdi: tam ters. Sonucu doğrudan da
//! sınadım — kameranın kendi sağına konan bir ses **sol kulağa** daha yakın çıkıyordu. Yani
//! basit sahne üzerine kurulu her oyunda stereo yönlendirme **aynalıydı**.
//!
//! Sebep: [`listener`] yön bilgisini `Transform.rotation`'dan okuyordu, ama kameranın yönü
//! `Camera` bileşenindeki yaw/pitch'te durur — görüş matrisini kuran da odur. `Transform.rotation`
//! ise kamerayı süren koda ait ve ağaçta **en az iki farklı gelenek** var: basit sahnenin
//! `rotation_y(-yaw + π/2)`'si tam ters bir vektör, `FpsLook`'un `rotation_y(-yaw)`'ı ise
//! kameranın **ileri** vektörünü veriyordu (kulaklar yan yana değil arka arkaya).
//!
//! Düzeltildi: dinleyici artık `Camera::get_right()` okuyor, ve `gizmo-engine`'e bir gerileme
//! testi eklendi. Bu demonun ölçümü aynı şeyi uçtan uca doğruluyor — HUD `+1,00` ve "sağ (doğru)"
//! yazıyorsa kulaklar ekrana diktir.
//!
//! ## Doğrulama
//!
//! Sesin çaldığını ekran görüntüsü göstermez. `GIZMO_AUDIO_SELFTEST=1` her 120 karede bir
//! mesafeyi, duyulan oranı, sink kazancını ve yukarıdaki kulak sınamasını günlüğe basar; kaynak
//! yörüngede döndüğü için sayılar inip çıkıyorsa sönümleme gerçekten işliyor demektir.
//!
//! ## Kontroller
//!   * **Boşluk** — kaynağın hareketini durdur/başlat · **M** — sustur
//!   * **Sağ-tık + fare / WASDQE** — kamera (yani **dinleyici**)

use gizmo::audio::{spatial_gain, AudioManager, AudioSource};
use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::TAU;

/// Ses kaynağının işareti — Bevy'nin `Emitter`'ı.
#[derive(Clone, Copy)]
struct Emitter;
gizmo::core::impl_component!(Emitter);

/// Demonun durumu ve ölçüm defteri.
#[derive(Clone)]
struct AudioDemo {
    /// Ses aygıtı açıldı mı — açılmadıysa demo sessiz ama çalışmaya devam eder.
    device_ready: bool,
    /// Kaynak hareket ediyor mu (Bevy'de boşluk tuşu).
    moving: bool,
    /// Susturuldu mu (Bevy'de M).
    muted: bool,
    /// Yörünge açısı.
    angle: f32,
    /// Dinleyiciye uzaklık, motorun sink'e yazdığı kazanç, ve gerçekten duyulan oran.
    distance: f32,
    sink_gain: f32,
    heard: f32,
    /// Kulakların dünya konumu.
    left_ear: Vec3,
    right_ear: Vec3,
    /// Dinleyicinin kullandığı sağ vektör ile kameranın kendi `get_right()`'ı arasındaki
    /// nokta çarpımı. +1 uyumlu, −1 ters, 0 dik demek.
    right_agreement: f32,
    /// Kaynak kameranın sağındayken hangi kulak daha yakın.
    nearer_ear: &'static str,
    frame: u32,
}

impl Default for AudioDemo {
    fn default() -> Self {
        Self {
            device_ready: false,
            moving: true,
            muted: false,
            angle: 0.0,
            distance: 0.0,
            sink_gain: 0.0,
            heard: 0.0,
            left_ear: Vec3::ZERO,
            right_ear: Vec3::ZERO,
            right_agreement: 0.0,
            nearer_ear: "-",
            frame: 0,
        }
    }
}
gizmo::core::impl_component!(AudioDemo);

/// Sesin motordaki adı.
const TONE: &str = "bevy_spatial_audio::tone";
/// Yörüngenin yarıçapı ve hızı.
const ORBIT_RADIUS: f32 = 6.0;
const ORBIT_SPEED: f32 = 0.6;
/// Sönümlemenin bittiği mesafe — `AudioSource::with_max_distance`.
const MAX_DISTANCE: f32 = 20.0;
/// Kendi kendine ölçümün periyodu.
const PROBE_PERIOD: u32 = 120;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Spatial Audio", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            let mut demo = AudioDemo::default();

            // Ses aygıtını aç ve tonu belleğe koy. Aygıt yoksa (başsız CI, ses kartsız makine)
            // demo görüntüyle çalışmaya devam ediyor — sessizce.
            match AudioManager::new() {
                Ok(mut audio) => {
                    audio.load_sound_bytes(TONE, sine_wav(440.0, 1.0));
                    scene.world.insert_resource(audio);
                    demo.device_ready = true;
                }
                Err(error) => {
                    gizmo::gizmo_log!(Warning, "ses aygıtı açılamadı ({error}); demo sessiz");
                }
            }

            // Bevy'nin mavi ses kaynağı küresi.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(ORBIT_RADIUS, 0.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_sphere(device, 0.35, 18, 32),
                Material::new(white.clone()).with_pbr(Vec4::new(0.20, 0.35, 0.95, 1.0), 0.4, 0.0),
                MeshRenderer::new(),
                Emitter,
                AudioSource::new(TONE)
                    .with_loop(true)
                    .with_max_distance(MAX_DISTANCE),
            ));

            // Yörüngenin merkezini işaretleyen halka — kaynağın nerede döndüğü görünsün.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.05, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_torus(device, ORBIT_RADIUS, 0.04, 64, 8),
                Material::new(white.clone()).with_pbr(Vec4::new(0.3, 0.3, 0.34, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.5, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 40.0),
                Material::new(white).with_pbr(Vec4::new(0.10, 0.11, 0.13, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.7) * Quat::from_rotation_x(-0.8),
                intensity: 2.4,
                ..Default::default()
            });

            scene.world.insert_resource(demo);
            scene.world.insert_resource(Gizmos::default());
            scene.spawn_camera(state, Vec3::new(0.0, 4.0, 10.0), Vec3::ZERO);
        })
        .add_update_system(orbit_emitter.in_phase(Phase::Update))
        // Uzamsal ses sistemi elle kaydedilmeli, ve `&mut World` istiyor.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            measure_listener(world);
            gizmo::systems::audio::audio_frame(world, 1.0 / 60.0);

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(demo) = world.get_resource::<AudioDemo>().map(|d| d.clone()) else {
                return;
            };
            gizmo::egui::Area::new("audio".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.set_min_width(420.0);
                    ui.heading("Uzamsal ses");
                    ui.label(if demo.device_ready {
                        "ses aygıtı: açık"
                    } else {
                        "ses aygıtı: AÇILAMADI (demo sessiz)"
                    });
                    ui.separator();
                    ui.label(format!("kaynak-dinleyici mesafesi: {:.2} m", demo.distance));
                    ui.label(format!("DUYULAN oran (motorun konisi): {:.3}", demo.heard));
                    ui.label(format!(
                        "spatial_gain(...) = {:.1}  <- sink'e yazılan, 1'i asabilir",
                        demo.sink_gain
                    ));
                    ui.label("  cunku rodio'nun 1/d^2 sonumlemesini onceden iptal ediyor");
                    ui.label(format!(
                        "sol kulak : ({:.2}, {:.2}, {:.2})",
                        demo.left_ear.x, demo.left_ear.y, demo.left_ear.z
                    ));
                    ui.label(format!(
                        "sağ kulak : ({:.2}, {:.2}, {:.2})",
                        demo.right_ear.x, demo.right_ear.y, demo.right_ear.z
                    ));
                    ui.separator();
                    ui.label("dinleyici = birincil KAMERA (ayrı bir varlık yok)");
                    ui.label("kulak aralığı sabit: 0,20 m");
                    ui.separator();
                    ui.label(format!(
                        "listener.right . Camera::get_right() = {:+.2}",
                        demo.right_agreement
                    ));
                    ui.label(format!(
                        "kaynak kameranın sağındayken yakın kulak: {}",
                        demo.nearer_ear
                    ));
                    ui.separator();
                    ui.label(format!(
                        "hareket: {} (boşluk) · ses: {} (M)",
                        if demo.moving { "açık" } else { "durdu" },
                        if demo.muted { "kısık" } else { "açık" }
                    ));
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Kaynağı yörüngede döndürür; boşluk durdurur, M susturur.
///
/// Susturma [`AudioSource::volume`] üzerinden — uzamsal sistem onu `base_volume` olarak okuyor,
/// yani Bevy'nin `PlaybackSettings` üzerinden yaptığının ECS karşılığı.
fn orbit_emitter(
    mut emitters: Query<(Mut<Transform>, Mut<AudioSource>, With<Emitter>)>,
    mut demo: ResMut<AudioDemo>,
    input: Res<Input>,
    time: Res<Time>,
) {
    use gizmo::winit::keyboard::KeyCode;
    if input.is_key_just_pressed(KeyCode::Space as u32) {
        demo.moving = !demo.moving;
    }
    if input.is_key_just_pressed(KeyCode::KeyM as u32) {
        demo.muted = !demo.muted;
    }
    if demo.moving {
        demo.angle += time.dt() * ORBIT_SPEED;
    }

    let angle = demo.angle;
    let volume = if demo.muted { 0.0 } else { 1.0 };
    for (_entity, (mut transform, mut source, _)) in emitters.iter_mut() {
        transform.position = Vec3::new(angle.cos() * ORBIT_RADIUS, 0.0, angle.sin() * ORBIT_RADIUS);
        source.volume = volume;
    }
}

/// Dinleyicinin (yani kameranın) kulaklarını ve o andaki sönümlemeyi ölçer.
fn measure_listener(world: &mut gizmo::core::World) {
    let listener = gizmo::systems::audio::listener(world);
    let (left, right) = listener.ears();

    let emitter = world.query::<(&Transform, &Emitter)>().and_then(|q| {
        q.iter()
            .next()
            .map(|(_, (transform, _))| transform.position)
    });
    let Some(emitter) = emitter else { return };

    // Kameranın KENDİ sağ vektörü — motorun `Camera::right_from(yaw)`'ı, görüş matrisini kuran
    // fonksiyonun ta kendisi. Dinleyicinin kullandığıyla karşılaştırılıyor.
    let camera_right = world
        .query::<(&Camera, &Transform)>()
        .and_then(|q| {
            q.iter()
                .find(|(_, (camera, _))| camera.primary)
                .map(|(_, (camera, _))| camera.get_right())
        })
        .unwrap_or(Vec3::X);
    let right_agreement = listener.right.dot(camera_right);

    // Kaynağı kameranın sağına koyup hangi kulağın yakın olduğunu sor: doğrusu sağ kulak.
    let probe = listener.position + camera_right * 5.0;
    let left_v = Vec3::new(left[0], left[1], left[2]);
    let right_v = Vec3::new(right[0], right[1], right[2]);
    let nearer_ear = if (probe - left_v).length() < (probe - right_v).length() {
        "SOL (ters!)"
    } else {
        "sağ (doğru)"
    };

    let distance = (emitter - listener.position).length();
    // Motorun kendi eğrisi — demo kendi formülünü uydurmuyor. Dönen sayı **duyulan ses değil**:
    // rodio'nun ters-kare sönümlemesini önceden iptal ettiği için 1'i aşabiliyor. Duyulan oran
    // o iptal edilen terim çıkarıldığında kalan doğrusal koni.
    let sink_gain = spatial_gain(distance, MAX_DISTANCE, 1.0);
    let heard = (1.0 - distance / MAX_DISTANCE).max(0.0);

    let Some(mut demo) = world.get_resource_mut::<AudioDemo>() else {
        return;
    };
    demo.distance = distance;
    demo.sink_gain = sink_gain;
    demo.heard = heard;
    demo.left_ear = left_v;
    demo.right_ear = right_v;
    demo.right_agreement = right_agreement;
    demo.nearer_ear = nearer_ear;
    demo.frame += 1;

    if std::env::var("GIZMO_AUDIO_SELFTEST").is_ok() && demo.frame.is_multiple_of(PROBE_PERIOD) {
        gizmo::gizmo_log!(
            Info,
            "kare {} · mesafe {:.2} m · duyulan {:.3} · sink {:.1} · right·get_right {:+.2} · yakın kulak {} · aygıt {}",
            demo.frame,
            distance,
            heard,
            sink_gain,
            right_agreement,
            nearer_ear,
            if demo.device_ready { "açık" } else { "kapalı" }
        );
    }
}

/// Bellekte bir WAV kurar: 16 bit, tek kanal, 44,1 kHz, `seconds` saniyelik `freq` Hz sinüs.
///
/// `load_sound_bytes` **tam bir ses dosyası** bekliyor (rodio'nun çözebileceği bir kap), ham
/// örnek değil — bu yüzden 44 baytlık RIFF başlığı da elle yazılıyor.
fn sine_wav(freq: f32, seconds: f32) -> Vec<u8> {
    const RATE: u32 = 44_100;
    let sample_count = (RATE as f32 * seconds) as u32;
    let data_len = sample_count * 2; // 16 bit, tek kanal

    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt bloğunun boyu
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // kanal
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // bayt/saniye
    wav.extend_from_slice(&2u16.to_le_bytes()); // blok hizası
    wav.extend_from_slice(&16u16.to_le_bytes()); // bit/örnek
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());

    for i in 0..sample_count {
        let t = i as f32 / RATE as f32;
        // Uçlarda kısa bir yumuşatma: döngüde tık sesi olmasın.
        let fade = (t / 0.01).min(1.0).min((seconds - t) / 0.01).clamp(0.0, 1.0);
        let sample = ((t * freq * TAU).sin() * fade * 0.35 * i16::MAX as f32) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}
