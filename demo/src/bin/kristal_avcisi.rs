//! # Kristal Avcısı — motorun 3D yolu üstünde küçük ama bitmiş bir oyun
//!
//! Tepeden görülen bir arenada mavi küreyi gezdir, parlayan kristalleri topla, kırmızı avcılara
//! yakalanma. Üç canın ve altmış saniyen var; her kristal yeni bir avcı çağırır.
//!
//! | tuş | iş |
//! |---|---|
//! | **WASD** / oklar / gamepad sol çubuk | hareket |
//! | **Enter** | menüden başla · bitince menüye dön |
//!
//! ## Neden var
//!
//! [`kule_yikici`](../kule_yikici/index.html) motorun 2D yolunu bir oyunda sınadı; bu dosya 3D
//! yolunu ve 2026-11-02'de eklenenleri **aynı anda** kullanıyor: sürülen durum makinesi
//! (`add_state` / `on_enter` / `DespawnOnExit`), sıralı sistemler (`.chain()`), dikdörtgen alan
//! ışıkları, emissive + additive kristaller, el ayarı sis, kamera arka plan rengi, bloom
//! yarıçapı, deband dither. Yazarken karşılaşılan her eksik aşağıda.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Ekran metni mutlak pikseldi.** `Text::screen((640, 240))` 1280 genişliğinde bir pencerenin
//!   ortası; döşemeli pencere yöneticisi pencereyi 472 piksel açınca menü yazısı ekranın
//!   dışında kaldı ve ilk ekran görüntüsünde hiç yoktu. `text.rs` ve `kule_yikici` bunu her kare
//!   `WindowInfo`'dan elle yeniden konumlayarak çözüyordu. **Kapatıldı:** `Text::pinned` /
//!   `TextSpace::Pinned` — pencerenin ortasına ya da bir köşesine iğnelenmiş, boyutla birlikte
//!   gelen metin.
//! - **Sistemde doğurma `Commands` ile, bileşen bileşen yapılıyordu.** Bir avcı yedi `insert`
//!   çağrısıydı; `World::spawn_bundle`'ın ertelenmiş karşılığı yoktu. **Kapatıldı:**
//!   `Commands::spawn_bundle` / `EntityCommands::insert_bundle` — artık giriş hook'u ve toplama
//!   sistemi aynı `hunter(..)` demetini doğuruyor.
//! - **`on_enter` hook'unda GPU yok — sanılıyordu.** Hook `&mut World` alıyor, renderer'ı görmüyor;
//!   oyun ilk sürümde mesh ve malzemeleri kurulumda üretip bir `Kit` kaynağında saklıyordu. Oysa
//!   GPU'suz yol **vardı**: `MeshSource("sphere")` yükleyiciye bir anahtar, `MaterialDesc` çizim
//!   yoluna bir tarif — ikisi de sonraki karede GPU'ya çevriliyor. Eksik olan ikisinin
//!   kullanılabilirliğiydi: `MaterialDesc`'in kurucusu ve builder'ı yoktu (13 alan elle), ve
//!   yükleyici her `"sphere"` varlığına **ayrı** bir mesh yüklüyordu. **Kapatıldı:**
//!   `MaterialDesc::new()` ve `Material`'ın bütün `with_*`'ları (tek makro, iki tip),
//!   `AssetManager::primitive_mesh` (ilkel anahtar başına tek yükleme). `Kit` silindi.
//! - **Rastgelelik yoktu.** Oyun dört satırlık bir xorshift taşıyordu — `spotlight`,
//!   `parallel_query`, `change_detection` ve parçacık yayıcısı gibi. **Kapatıldı:**
//!   `gizmo_math::Rng` (prelude'da), tohumlu SplitMix64; oyun onu kaynak olarak tutuyor.
//! - **Kodla üretilmiş geometri hook'ta doğamıyordu.** `MeshSource` yalnızca hazır ilkelleri ve
//!   dosyaları biliyordu; bir ekstrüzyon `Device` istiyordu. **Kapatıldı:** `MeshData` —
//!   `MeshData::extrude(&yıldız, 0.5)` hook'ta kuruluyor, yükleyici bir sonraki kare GPU'ya
//!   taşıyor, kopyalar tek yüklemeyi paylaşıyor. Kristaller artık yıldız.
//! - **Ses dosyası yoksa ses yoktu.** `AudioManager` yalnızca dosya ya da dosya baytı
//!   çalıyordu; repo ses varlığı taşımıyor, ilk sürüm sessizdi. **Kapatıldı:**
//!   `gizmo::audio::synth` — `Blip::new(Wave::Square, 660.0, 0.12).with_slide(1320.0).wav()` bir
//!   toplama sesi, `AudioManager::load_samples` kodda üretilmiş PCM için. Dört efekt de burada
//!   üretiliyor; ses cihazı yoksa oyun sessiz sürüyor.
//! - **Parametre demeti yoktu.** `(time, state): (Res<Time>, Res<State<Screen>>)` derlenmiyordu
//!   — Bevy'den gelen herkesin ilk yazdığı şey. **Kapatıldı:** 2–8 elemanlı demetler (iç içe de)
//!   `SystemParam`; `collect` bir tane kullanıyor.
//! - **En iyi skor kapanınca kayboluyordu.** Oyunun bir oturumdan ötekine bir şey saklama yolu
//!   yoktu. **Kapatıldı:** `gizmo::app::save::SaveData` — en iyi skor
//!   `<veri dizini>/kristal_avcisi/save.json`'da (tarayıcıda `localStorage`).
//! - **Ekranı kızartmanın yolu yoktu.** Yakalanınca ekranın kısa bir an kırmızı parlaması için
//!   pencere boyu bir UI düğümü kurup onu elle soldurmak gerekiyordu — `ColorTween` UI zeminini
//!   sürmüyor, bir düğüm de her katmanın üstünde değil, bir katmanda duruyor. **Kapatıldı:**
//!   `ScreenFade` kaynağı (`flash`, `fade_out`/`fade_in`, `just_finished`); iki çizim yolu da gerçek
//!   kare süresiyle ilerletip her şeyin üstüne çiziyor.
//! - **Fontsuz metin.** Repo font taşımıyor; `kule_yikici` gibi sistem yolları deneniyor.
//!
//! ## Ortam değişkenleri
//!   * `GIZMO_FONT=<yol>` — HUD fontu
//!   * `KRISTAL_OYNA=1` — menüyü atlayıp doğrudan oyunla aç (ekran görüntüsü için)

use gizmo::audio::synth::{Blip, Wave};
use gizmo::app::save::SaveData;
use gizmo::audio::AudioManager;
use gizmo::core::input::{Input, MoveKeys};
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::renderer::Fog;
use gizmo::simple::{sync_transforms, HeadlessFrames, SimpleAppExt, SimpleSceneState};
use gizmo::winit::keyboard::KeyCode;

/// Arenanın yarı genişliği (kare).
const ARENA: f32 = 9.0;
const PLAYER_SPEED: f32 = 7.0;
const HUNTER_SPEED: f32 = 2.6;
const ROUND_SECONDS: f32 = 60.0;
const CRYSTALS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Menu,
    Playing,
    Over,
}

#[derive(Clone, Copy)]
struct Player;
gizmo::core::impl_component!(Player);

#[derive(Clone, Copy)]
struct Hunter;
gizmo::core::impl_component!(Hunter);

#[derive(Clone, Copy)]
struct Crystal;
gizmo::core::impl_component!(Crystal);


/// Bir turun durumu.
#[derive(Clone, Copy)]
struct Run {
    score: u32,
    lives: u32,
    time_left: f32,
    /// Son çarpışmadan sonra dokunulmazlık (saniye).
    grace: f32,
    best: u32,
}

/// HUD metinleri.
#[derive(Clone, Copy)]
struct Hud {
    status: Entity,
    banner: Entity,
}

/// Arenada, duvarlardan bir birim içeride rastgele bir nokta.
fn arena_point(rng: &mut Rng) -> Vec3 {
    let half = ARENA - 1.0;
    Vec3::new(rng.range(-half..half), 0.5, rng.range(-half..half))
}

/// Oyunun durumları ve sistemleri — pencereli uygulamaya da `--test`'in başsız koşusuna da aynı
/// eklenti kuruluyor. `keep_records` kapalıyken en iyi skor kayda yazılmaz.
struct KristalPlugin {
    initial: Screen,
    keep_records: bool,
}

impl gizmo::app::Plugin for KristalPlugin {
    fn build(&self, app: &mut dyn gizmo::app::AppLike) {
        let parts = app.parts_mut();
        let (world, update_schedule) = (parts.world, parts.update_schedule);
        let keep_records = self.keep_records;
        gizmo::core::state::add_state(world, self.initial);
        gizmo::core::state::on_enter(world, Screen::Menu, |world| banner(world, "KRİSTAL AVCISI\nEnter ile başla", [0.6, 0.85, 1.0]));
        gizmo::core::state::on_enter(world, Screen::Playing, start_round);
        gizmo::core::state::on_enter(world, Screen::Over, move |world| {
            play(world.get_resource_mut::<AudioManager>().as_deref_mut(), "bitis");
            let (score, best) = world.get_resource::<Run>().map(|r| (r.score, r.best)).unwrap_or((0, 0));
            // En iyi skor kalıcı: bir sonraki açılışta da orada.
            let mut save = SaveData::open("kristal_avcisi");
            if keep_records && save.get::<u32>("best").unwrap_or(0) < best {
                let _ = save.set("best", &best);
                if let Err(e) = save.save() {
                    gizmo::gizmo_log!(Warning, "en iyi skor kaydedilemedi: {e}");
                }
            }
            banner(world, &format!("Oyun bitti — {score} kristal\nen iyi {best}\nEnter ile menü"), [1.0, 0.75, 0.3]);
        });
        // Menü ve bitiş ekranı: Enter.
        update_schedule.add_di_system(press_enter.in_phase(Phase::Update));
        // Oyun: sırayla — önce oyuncu, sonra avcılar ona doğru, sonra toplama, sonra çarpışma.
        update_schedule.add_chain(
            (move_player, chase, collect, get_caught, tick_clock)
                .chain()
                .in_phase(Phase::Update),
        );
        update_schedule.add_di_system(draw_hud.in_phase(Phase::PostUpdate));
    }
}

/// Sahneyi kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır). `sound` kapalıyken
/// ses aygıtı açılmaz.
fn setup(world: &mut World, font: FontId, sound: bool) {

    // Kalıcı sahne: zemin, duvarlar, ışıklar. Durumlardan bağımsız. Kurulumda GPU var ama
    // gerekmiyor — hook'larla aynı GPU'suz tarif.
    world.spawn_bundle(block(
        Vec3::new(0.0, -0.1, 0.0),
        Vec3::new(ARENA, 0.1, ARENA),
        MaterialDesc::new().with_pbr(Vec4::new(0.35, 0.36, 0.42, 1.0), 0.7, 0.0),
    ));
    for (x, z, sx, sz) in [(ARENA, 0.0, 0.2, ARENA), (-ARENA, 0.0, 0.2, ARENA), (0.0, ARENA, ARENA, 0.2), (0.0, -ARENA, ARENA, 0.2)] {
        world.spawn_bundle(block(
            Vec3::new(x, 0.5, z),
            Vec3::new(sx, 0.5, sz),
            MaterialDesc::new().with_pbr(Vec4::new(0.2, 0.2, 0.26, 1.0), 0.8, 0.0),
        ));
    }
    // Arenanın iki yanında aşağı bakan sıcak panel ışıklar.
    for x in [-5.0f32, 5.0] {
        world.spawn_bundle((
            Transform::new(Vec3::new(x, 4.0, 0.0)).with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            GlobalTransform::default(),
            RectLight::new(Vec3::new(1.0, 0.85, 0.6), 6.0, 14.0, 3.0, 1.5),
        ));
    }
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-1.0)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(0.7, 0.75, 1.0), 0.8, LightRole::Sun).with_angular_radius(0.02),
    ));

    let camera = world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 16.0, 11.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.95,
        primary: true,
        ..Default::default()
    });
    {
        let mut q = world.query_mut::<Mut<Camera>>();
        if let Some(mut cam) = q.get_mut(camera) {
            cam.clear_color = Some(Vec3::new(0.01, 0.012, 0.03));
        }
    }

    // `pinned`: pencere boyutu ne olursa olsun sol üst köşe ve orta — mutlak piksel değil.
    let status = world
        .spawn_bundle((Text::pinned("", font, 24.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
        ;
    let banner_text = world
        .spawn_bundle((Text::pinned("", font, 40.0, TextAnchor::Center, Vec2::new(0.0, -40.0))
            .with_align(TextAlign::Center),))
        ;
    world.insert_resource(Hud { status, banner: banner_text });
    // En iyi skor bir önceki oturumdan.
    let best = SaveData::open("kristal_avcisi").get("best").unwrap_or(0);
    world.insert_resource(Run { score: 0, lives: 3, time_left: ROUND_SECONDS, grace: 0.0, best });
    world.insert_resource(gizmo::renderer::screen_fade::ScreenFade::default());
    world.insert_resource(Rng::new(0x9E37_79B9));
    // Sesler dosyadan değil sayılardan. Cihaz yoksa kaynak hiç konmuyor, `play` susuyor.
    if let Some(mut audio) = sound.then(AudioManager::new).and_then(Result::ok) {
        audio.load_sound_bytes("kristal", Blip::new(Wave::Square, 660.0, 0.12).with_slide(1320.0).with_volume(0.25).wav());
        audio.load_sound_bytes("yakalandi", Blip::new(Wave::Noise, 1800.0, 0.35).with_slide(120.0).with_volume(0.5).wav());
        audio.load_sound_bytes("basla", Blip::new(Wave::Sine, 440.0, 0.25).with_slide(880.0).with_volume(0.35).wav());
        audio.load_sound_bytes("bitis", Blip::new(Wave::Saw, 330.0, 0.8).with_slide(80.0).with_volume(0.3).wav());
        world.insert_resource(audio);
    }
}

/// `--test`: pencere açmadan bir tur. Oyun doğrudan "oynuyor"da başlıyor; oyuncu her karede en
/// yakın kristale doğru WASD'ye basıyor (oyunun kendi girdi yolu). Kristal toplanmalı, avcılar
/// kovalamalı, tur bitmeli — süreyle ya da üç yakalanmayla — ve bitişte turun varlıkları
/// (`DespawnOnExit`) toplanmalı.
fn self_test() -> bool {
    let mut world = World::new();
    setup(&mut world, HeadlessFrames::font(), false);
    let mut run = HeadlessFrames::new(world).add_plugin(KristalPlugin { initial: Screen::Playing, keep_records: false });
    let keys = [KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD].map(|k| k as u32);
    let over = |w: &World| w.get_resource::<State<Screen>>().is_some_and(|s| *s.get() == Screen::Over);
    let mut lost = 0u32;
    let mut lives_before = 3u32;
    while run.elapsed < ROUND_SECONDS + 10.0 && !over(&run.world) {
        let (player, target) = {
            let tfs = run.world.borrow::<Transform>();
            let player = run.world.query::<(Entity, &Player)>().iter().find_map(|(entity, _)| tfs.get(entity).map(|t| t.position));
            let crystals: Vec<Vec3> = run.world.query::<(Entity, &Crystal)>().iter().filter_map(|(entity, _)| tfs.get(entity).map(|t| t.position)).collect();
            let target = player.and_then(|p| crystals.into_iter().min_by(|a, b| a.distance(p).total_cmp(&b.distance(p))));
            (player, target)
        };
        for &k in &keys {
            run.input.on_key_released(k);
        }
        if let (Some(p), Some(t)) = (player, target) {
            let d = t - p;
            if d.x > 0.2 {
                run.input.on_key_pressed(keys[3]);
            } else if d.x < -0.2 {
                run.input.on_key_pressed(keys[1]);
            }
            if d.z < -0.2 {
                run.input.on_key_pressed(keys[0]);
            } else if d.z > 0.2 {
                run.input.on_key_pressed(keys[2]);
            }
        }
        run.frame(1.0 / 60.0, update);
        let lives = run.world.get_resource::<Run>().map_or(0, |r| r.lives);
        lost += lives_before.saturating_sub(lives);
        lives_before = lives;
    }
    let (score, time_left) = run.world.get_resource::<Run>().map_or((0, 0.0), |r| (r.score, r.time_left));
    let leftovers = run.world.query::<(Entity, &Player)>().iter().count() + run.world.query::<(Entity, &Crystal)>().iter().count();
    let ok = over(&run.world) && score >= 3 && leftovers == 0;
    println!(
        "kristal_avcisi --test: tur {:.0} s sürdü (kalan süre {time_left:.0} s), {score} kristal, {lost} kez yakalandı; bitişte kalan tur varlığı {leftovers} → {}",
        run.elapsed,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    let initial = if std::env::var_os("KRISTAL_OYNA").is_some() { Screen::Playing } else { Screen::Menu };
    App::<SimpleSceneState>::new("Gizmo Engine - Kristal Avcisi", 1280, 720)
        .add_plugin(KristalPlugin { initial, keep_records: true })
        .with_simple_scene(|scene, _state| {
            let r = &mut *scene.renderer;
            r.fog = Some(Fog::new([0.05, 0.06, 0.12], 0.02).with_height(0.15, -1.0));
            r.bloom_radius = 2.0;
            r.bloom_intensity = 1.2;
            r.deband_dither = true;
            let font = load_font(r);
            setup(scene.world, font, true);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, _dt: f32, _input: &Input) {
    sync_transforms(world);
}

/// Oyuna giriş: oyuncuyu, kristalleri ve ilk avcıyı doğur; turu sıfırla. Hepsi
/// `DespawnOnExit(Playing)` taşıyor — bitişte motor kendisi topluyor.
fn start_round(world: &mut World) {
    play(world.get_resource_mut::<AudioManager>().as_deref_mut(), "basla");
    let mut rng = world.get_resource::<Rng>().map(|r| r.clone()).unwrap_or_else(|| Rng::new(1));
    banner(world, "", [1.0; 3]);
    let best = world.get_resource::<Run>().map(|r| r.best).unwrap_or(0);
    world.insert_resource(Run { score: 0, lives: 3, time_left: ROUND_SECONDS, grace: 1.0, best });

    world.spawn_bundle((
        // `"sphere"`in yarıçapı 1: 0.5'lik bir top için yarım ölçek.
        Transform::new(Vec3::new(0.0, 0.5, 0.0)).with_scale(Vec3::splat(0.5)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.2, 0.45, 1.0, 1.0), 0.3, 0.2),
        MeshRenderer::new(),
        Player,
        DespawnOnExit(Screen::Playing),
    ));
    for _ in 0..CRYSTALS {
        spawn_crystal(world, arena_point(&mut rng));
    }
    world.spawn_bundle(hunter(Vec3::new(ARENA - 1.5, 0.5, ARENA - 1.5)));
    world.insert_resource(rng);
}

/// Zemin ve duvarlar: ölçeklenmiş birer küp.
fn block(at: Vec3, half: Vec3, material: MaterialDesc) -> impl gizmo::core::component::Bundle + Send + Sync + 'static {
    (
        Transform::new(at).with_scale(half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        material,
        MeshRenderer::new(),
    )
}

/// Altı köşeli, kalınlaştırılmış bir yıldız — kodla üretilmiş geometri, GPU'suz. Bir kez
/// kuruluyor; her kristal bir kopyasını taşıyor ve hepsi tek bir yüklemeyi paylaşıyor.
fn crystal_shape() -> MeshData {
    static SHAPE: std::sync::OnceLock<MeshData> = std::sync::OnceLock::new();
    SHAPE
        .get_or_init(|| {
            let star: Vec<Vec2> = (0..12)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 12.0;
                    let r = if i % 2 == 0 { 1.0 } else { 0.5 };
                    Vec2::new(a.cos() * r, a.sin() * r)
                })
                .collect();
            MeshData::extrude(&star, 0.5).expect("yıldız üçgenlenir")
        })
        .clone()
}

fn spawn_crystal(world: &mut World, at: Vec3) {
    world.spawn_bundle((
        Transform::new(at).with_scale(Vec3::splat(0.35)),
        GlobalTransform::default(),
        crystal_shape(),
        MaterialDesc::new()
            .with_unlit(Vec4::new(0.3, 1.0, 0.8, 0.9))
            .with_emissive(Vec3::new(0.5, 2.0, 1.6))
            .with_blend_mode(BlendMode::Additive),
        MeshRenderer::new().with_receives_shadows(false),
        Crystal,
        DespawnOnExit(Screen::Playing),
    ));
}

/// Bir avcı — `World::spawn_bundle` (giriş hook'u) ve `Commands::spawn_bundle` (toplama
/// sistemi) aynı demeti alıyor. Hepsi tek bir `"sphere"` yüklemesini paylaşıyor.
fn hunter(at: Vec3) -> impl gizmo::core::component::Bundle + Send + Sync + 'static {
    (
        Transform::new(at).with_scale(Vec3::splat(0.4)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        MaterialDesc::new()
            .with_pbr(Vec4::new(0.9, 0.15, 0.1, 1.0), 0.5, 0.0)
            .with_emissive(Vec3::new(0.6, 0.05, 0.0)),
        MeshRenderer::new(),
        Hunter,
        DespawnOnExit(Screen::Playing),
    )
}

/// Bir efekt çal — ses cihazı yoksa hiçbir şey yapma.
fn play(audio: Option<&mut AudioManager>, name: &str) {
    if let Some(audio) = audio {
        let _ = audio.play(name);
    }
}

fn banner(world: &mut World, text: &str, colour: [f32; 3]) {
    let Some(hud) = world.get_resource::<Hud>().map(|h| *h) else { return };
    let mut texts = world.borrow_mut::<Text>();
    if let Some(mut t) = texts.get_mut(hud.banner) {
        t.content = text.to_string();
        t.color = Vec4::new(colour[0], colour[1], colour[2], 1.0);
    }
}

/// Menüde ve bitişte Enter oyunu başlatır / menüye döndürür.
fn press_enter(mut state: ResMut<State<Screen>>, input: Res<Input>) {
    if !input.is_key_just_pressed(KeyCode::Enter as u32) {
        return;
    }
    match *state.get() {
        Screen::Menu => state.set(Screen::Playing),
        Screen::Over => state.set(Screen::Menu),
        Screen::Playing => {}
    }
}

fn playing(state: &State<Screen>) -> bool {
    *state.get() == Screen::Playing
}

fn move_player(mut players: Query<(Mut<Transform>, With<Player>)>, input: Res<Input>, time: Res<Time>, state: Res<State<Screen>>) {
    if !playing(&state) {
        return;
    }
    let (x, z) = input.move_axis();
    let (ax, az) = input.move_axis_with(MoveKeys::ARROWS);
    let dir = Vec3::new(x + ax, 0.0, -(z + az));
    let step = dir.normalize_or_zero() * PLAYER_SPEED * time.dt();
    for (mut tf, _) in players.iter_mut() {
        tf.position = (tf.position + step).clamp(Vec3::new(-ARENA + 0.6, 0.5, -ARENA + 0.6), Vec3::new(ARENA - 0.6, 0.5, ARENA - 0.6));
    }
}

fn player_position(players: &Query<(&Transform, With<Player>)>) -> Option<Vec3> {
    players.iter().next().map(|(tf, _)| tf.position)
}

fn chase(mut hunters: Query<(Mut<Transform>, With<Hunter>)>, players: Query<(&Transform, With<Player>)>, time: Res<Time>, state: Res<State<Screen>>) {
    if !playing(&state) {
        return;
    }
    let Some(target) = player_position(&players) else { return };
    for (mut tf, _) in hunters.iter_mut() {
        let to = (target - tf.position) * Vec3::new(1.0, 0.0, 1.0);
        tf.position += to.normalize_or_zero() * HUNTER_SPEED * time.dt();
    }
}

/// Bir kristale değmek: puan, kristal başka yerde yeniden belirir, bir avcı daha gelir.
fn collect(
    mut crystals: Query<(Mut<Transform>, With<Crystal>)>,
    players: Query<(&Transform, With<Player>)>,
    mut run: ResMut<Run>,
    mut rng: ResMut<Rng>,
    mut commands: gizmo::core::Commands,
    // Bir demet de parametre: ilk yazıldığında derlenmiyordu.
    (time, state): (Res<Time>, Res<State<Screen>>),
    mut audio: Option<ResMut<AudioManager>>,
) {
    if !playing(&state) {
        return;
    }
    let Some(player) = player_position(&players) else { return };
    let spin = Quat::from_rotation_y(time.dt() * 2.0) * Quat::from_rotation_x(time.dt() * 1.3);
    for (mut tf, _) in crystals.iter_mut() {
        tf.rotation = spin * tf.rotation;
        if tf.position.distance(player) < 0.9 {
            run.score += 1;
            play(audio.as_deref_mut(), "kristal");
            tf.position = arena_point(&mut rng);
            let corner = Vec3::new(if rng.bool() { -1.0 } else { 1.0 }, 0.0, if rng.bool() { -1.0 } else { 1.0 })
                * (ARENA - 1.5)
                + Vec3::Y * 0.5;
            commands.spawn_bundle(hunter(corner));
        }
    }
}

/// Bir avcıya yakalanmak can götürür; ardından kısa bir dokunulmazlık.
fn get_caught(
    hunters: Query<(&Transform, With<Hunter>)>,
    players: Query<(&Transform, With<Player>)>,
    mut run: ResMut<Run>,
    mut state: ResMut<State<Screen>>,
    time: Res<Time>,
    mut audio: Option<ResMut<AudioManager>>,
    mut fade: Option<ResMut<gizmo::renderer::screen_fade::ScreenFade>>,
) {
    if *state.get() != Screen::Playing {
        return;
    }
    run.grace = (run.grace - time.dt()).max(0.0);
    let Some(player) = player_position(&players) else { return };
    let caught = hunters.iter().any(|(tf, _)| tf.position.distance(player) < 0.9);
    if caught && run.grace == 0.0 {
        run.lives = run.lives.saturating_sub(1);
        run.grace = 1.5;
        if let Some(fade) = fade.as_deref_mut() {
            fade.flash(Vec3::new(0.9, 0.05, 0.05), 0.45, 0.4);
        }
        if run.lives > 0 {
            play(audio.as_deref_mut(), "yakalandi");
        }
        if run.lives == 0 {
            run.best = run.best.max(run.score);
            state.set(Screen::Over);
        }
    }
}

fn tick_clock(mut run: ResMut<Run>, mut state: ResMut<State<Screen>>, time: Res<Time>) {
    if *state.get() != Screen::Playing {
        return;
    }
    run.time_left -= time.dt();
    if run.time_left <= 0.0 {
        run.time_left = 0.0;
        run.best = run.best.max(run.score);
        state.set(Screen::Over);
    }
}

fn draw_hud(mut texts: Query<Mut<Text>>, run: Res<Run>, hud: Res<Hud>, state: Res<State<Screen>>) {
    if let Some(mut t) = texts.get_mut(hud.status) {
        t.content = if playing(&state) {
            format!("Kristal {}   Can {}   Süre {:.0}", run.score, run.lives, run.time_left.ceil())
        } else {
            String::new()
        };
    }
}


/// `GIZMO_FONT`, sonra bilinen sistem yolları, sonra sentetik yüz.
fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    // A font the machine has (`GIZMO_FONT`, or the platform's usual faces); without one, the
    // synthetic test face, so the game still runs.
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
