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
//! - **`on_enter` hook'unda GPU yok.** Bölüme giriş varlık doğuruyor, varlık mesh ister, mesh
//!   `device` ister — hook `&mut World` alıyor ve renderer'ı görmüyor. Çözüm: mesh ve malzemeler
//!   kurulumda **bir kez** üretilip kaynak (`Kit`) olarak saklanıyor, hook ve sistemler onları
//!   klonluyor (ikisi de `Arc`). Çalışıyor ve ucuz; ama "hook'ta yeni bir mesh üret" yazılamıyor.
//! - **Rastgelelik yoktu.** Oyun dört satırlık bir xorshift taşıyordu — `spotlight`,
//!   `parallel_query`, `change_detection` ve parçacık yayıcısı gibi. **Kapatıldı:**
//!   `gizmo_math::Rng` (prelude'da), tohumlu SplitMix64; oyun onu kaynak olarak tutuyor.
//! - **Fontsuz metin.** Repo font taşımıyor; `kule_yikici` gibi sistem yolları deneniyor.
//!
//! ## Ortam değişkenleri
//!   * `GIZMO_FONT=<yol>` — HUD fontu
//!   * `KRISTAL_OYNA=1` — menüyü atlayıp doğrudan oyunla aç (ekran görüntüsü için)

use gizmo::core::input::{Input, MoveKeys};
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::renderer::Fog;
use gizmo::simple::{sync_transforms, SimpleAppExt, SimpleSceneState};
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

/// Kurulumda bir kez üretilen mesh ve malzemeler — hook'ların ve sistemlerin GPU'su yok.
#[derive(Clone)]
struct Kit {
    sphere: Mesh,
    cube: Mesh,
    player: Material,
    hunter: Material,
    crystal: Material,
}

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
    status: u32,
    banner: u32,
}

/// Arenada, duvarlardan bir birim içeride rastgele bir nokta.
fn arena_point(rng: &mut Rng) -> Vec3 {
    let half = ARENA - 1.0;
    Vec3::new(rng.range(-half..half), 0.5, rng.range(-half..half))
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Kristal Avcisi", 1280, 720)
        // `KRISTAL_OYNA=1`: menüyü atla — ekran görüntüsü oyunun içini göstersin.
        .add_state(if std::env::var_os("KRISTAL_OYNA").is_some() { Screen::Playing } else { Screen::Menu })
        .on_enter(Screen::Menu, |world| banner(world, "KRİSTAL AVCISI\nEnter ile başla", [0.6, 0.85, 1.0]))
        .on_enter(Screen::Playing, start_round)
        .on_enter(Screen::Over, |world| {
            let (score, best) = world.get_resource::<Run>().map(|r| (r.score, r.best)).unwrap_or((0, 0));
            banner(world, &format!("Oyun bitti — {score} kristal\nen iyi {best}\nEnter ile menü"), [1.0, 0.75, 0.3]);
        })
        .with_simple_scene(|scene, _state| {
            let r = &mut *scene.renderer;
            r.fog = Some(Fog::new([0.05, 0.06, 0.12], 0.02).with_height(0.15, -1.0));
            r.bloom_radius = 2.0;
            r.bloom_intensity = 1.2;
            r.deband_dither = true;
            let font = load_font(r);

            let white = scene.asset_manager.create_white_texture(&r.device, &r.queue, &r.scene.texture_bind_group_layout);
            let device = &r.device;
            let kit = Kit {
                sphere: AssetManager::create_sphere(device, 0.5, 16, 24),
                cube: AssetManager::create_cube(device),
                player: Material::new(white.clone()).with_pbr(Vec4::new(0.2, 0.45, 1.0, 1.0), 0.3, 0.2),
                hunter: Material::new(white.clone())
                    .with_pbr(Vec4::new(0.9, 0.15, 0.1, 1.0), 0.5, 0.0)
                    .with_emissive(Vec3::new(0.6, 0.05, 0.0)),
                crystal: Material::new(white.clone())
                    .with_unlit(Vec4::new(0.3, 1.0, 0.8, 0.9))
                    .with_emissive(Vec3::new(0.5, 2.0, 1.6))
                    .with_blend_mode(BlendMode::Additive),
            };

            // Kalıcı sahne: zemin, duvarlar, ışıklar. Durumlardan bağımsız.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.1, 0.0)).with_scale(Vec3::new(ARENA, 0.1, ARENA)),
                GlobalTransform::default(),
                kit.cube.clone(),
                Material::new(white.clone()).with_pbr(Vec4::new(0.35, 0.36, 0.42, 1.0), 0.7, 0.0),
                MeshRenderer::new(),
            ));
            for (x, z, sx, sz) in [(ARENA, 0.0, 0.2, ARENA), (-ARENA, 0.0, 0.2, ARENA), (0.0, ARENA, ARENA, 0.2), (0.0, -ARENA, ARENA, 0.2)] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.5, z)).with_scale(Vec3::new(sx, 0.5, sz)),
                    GlobalTransform::default(),
                    kit.cube.clone(),
                    Material::new(white.clone()).with_pbr(Vec4::new(0.2, 0.2, 0.26, 1.0), 0.8, 0.0),
                    MeshRenderer::new(),
                ));
            }
            // Arenanın iki yanında aşağı bakan sıcak panel ışıklar.
            for x in [-5.0f32, 5.0] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 4.0, 0.0)).with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                    GlobalTransform::default(),
                    RectLight::new(Vec3::new(1.0, 0.85, 0.6), 6.0, 14.0, 3.0, 1.5),
                ));
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-1.0)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(0.7, 0.75, 1.0), 0.8, LightRole::Sun).with_angular_radius(0.02),
            ));

            let camera = scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 16.0, 11.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -0.95,
                primary: true,
                ..Default::default()
            });
            if let Some(mut q) = scene.world.query_mut::<Mut<Camera>>() {
                if let Some(mut cam) = q.get_mut(camera.id()) {
                    cam.clear_color = Some(Vec3::new(0.01, 0.012, 0.03));
                }
            }

            // `pinned`: pencere boyutu ne olursa olsun sol üst köşe ve orta — mutlak piksel değil.
            let status = scene
                .world
                .spawn_bundle((Text::pinned("", font, 24.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
                .id();
            let banner_text = scene
                .world
                .spawn_bundle((Text::pinned("", font, 40.0, TextAnchor::Center, Vec2::new(0.0, -40.0))
                    .with_align(TextAlign::Center),))
                .id();
            scene.world.insert_resource(Hud { status, banner: banner_text });
            scene.world.insert_resource(kit);
            scene.world.insert_resource(Run { score: 0, lives: 3, time_left: ROUND_SECONDS, grace: 0.0, best: 0 });
            scene.world.insert_resource(Rng::new(0x9E37_79B9));
        })
        // Menü ve bitiş ekranı: Enter.
        .add_update_system(press_enter.in_phase(Phase::Update))
        // Oyun: sırayla — önce oyuncu, sonra avcılar ona doğru, sonra toplama, sonra çarpışma.
        .add_update_chain(
            (move_player, chase, collect, get_caught, tick_clock)
                .chain()
                .in_phase(Phase::Update),
        )
        .add_update_system(draw_hud.in_phase(Phase::PostUpdate))
        .set_update(|world, _state, _dt, _input| sync_transforms(world))
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Oyuna giriş: oyuncuyu, kristalleri ve ilk avcıyı doğur; turu sıfırla. Hepsi
/// `DespawnOnExit(Playing)` taşıyor — bitişte motor kendisi topluyor.
fn start_round(world: &mut World) {
    let Some(kit) = world.get_resource::<Kit>().map(|k| k.clone()) else { return };
    let mut rng = world.get_resource::<Rng>().map(|r| r.clone()).unwrap_or_else(|| Rng::new(1));
    banner(world, "", [1.0; 3]);
    let best = world.get_resource::<Run>().map(|r| r.best).unwrap_or(0);
    world.insert_resource(Run { score: 0, lives: 3, time_left: ROUND_SECONDS, grace: 1.0, best });

    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 0.5, 0.0)),
        GlobalTransform::default(),
        kit.sphere.clone(),
        kit.player.clone(),
        MeshRenderer::new(),
        Player,
        DespawnOnExit(Screen::Playing),
    ));
    for _ in 0..CRYSTALS {
        spawn_crystal(world, &kit, arena_point(&mut rng));
    }
    world.spawn_bundle(hunter(&kit, Vec3::new(ARENA - 1.5, 0.5, ARENA - 1.5)));
    world.insert_resource(rng);
}

fn spawn_crystal(world: &mut World, kit: &Kit, at: Vec3) {
    world.spawn_bundle((
        Transform::new(at).with_scale(Vec3::splat(0.3)),
        GlobalTransform::default(),
        kit.cube.clone(),
        kit.crystal.clone(),
        MeshRenderer::new().with_receives_shadows(false),
        Crystal,
        DespawnOnExit(Screen::Playing),
    ));
}

/// Bir avcı — `World::spawn_bundle` (giriş hook'u) ve `Commands::spawn_bundle` (toplama
/// sistemi) aynı demeti alıyor.
fn hunter(kit: &Kit, at: Vec3) -> impl gizmo::core::component::Bundle + Send + Sync + 'static {
    (
        Transform::new(at).with_scale(Vec3::splat(0.8)),
        GlobalTransform::default(),
        kit.sphere.clone(),
        kit.hunter.clone(),
        MeshRenderer::new(),
        Hunter,
        DespawnOnExit(Screen::Playing),
    )
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
    for (_, (mut tf, _)) in players.iter_mut() {
        tf.position = (tf.position + step).clamp(Vec3::new(-ARENA + 0.6, 0.5, -ARENA + 0.6), Vec3::new(ARENA - 0.6, 0.5, ARENA - 0.6));
    }
}

fn player_position(players: &Query<(&Transform, With<Player>)>) -> Option<Vec3> {
    players.iter().next().map(|(_, (tf, _))| tf.position)
}

fn chase(mut hunters: Query<(Mut<Transform>, With<Hunter>)>, players: Query<(&Transform, With<Player>)>, time: Res<Time>, state: Res<State<Screen>>) {
    if !playing(&state) {
        return;
    }
    let Some(target) = player_position(&players) else { return };
    for (_, (mut tf, _)) in hunters.iter_mut() {
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
    kit: Res<Kit>,
    mut commands: gizmo::core::Commands,
    time: Res<Time>,
    state: Res<State<Screen>>,
) {
    if !playing(&state) {
        return;
    }
    let Some(player) = player_position(&players) else { return };
    let spin = Quat::from_rotation_y(time.dt() * 2.0) * Quat::from_rotation_x(time.dt() * 1.3);
    for (_, (mut tf, _)) in crystals.iter_mut() {
        tf.rotation = spin * tf.rotation;
        if tf.position.distance(player) < 0.9 {
            run.score += 1;
            tf.position = arena_point(&mut rng);
            let corner = Vec3::new(if rng.bool() { -1.0 } else { 1.0 }, 0.0, if rng.bool() { -1.0 } else { 1.0 })
                * (ARENA - 1.5)
                + Vec3::Y * 0.5;
            commands.spawn_bundle(hunter(&kit, corner));
        }
    }
}

/// Bir avcıya yakalanmak can götürür; ardından kısa bir dokunulmazlık.
fn get_caught(hunters: Query<(&Transform, With<Hunter>)>, players: Query<(&Transform, With<Player>)>, mut run: ResMut<Run>, mut state: ResMut<State<Screen>>, time: Res<Time>) {
    if *state.get() != Screen::Playing {
        return;
    }
    run.grace = (run.grace - time.dt()).max(0.0);
    let Some(player) = player_position(&players) else { return };
    let caught = hunters.iter().any(|(_, (tf, _))| tf.position.distance(player) < 0.9);
    if caught && run.grace == 0.0 {
        run.lives = run.lives.saturating_sub(1);
        run.grace = 1.5;
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

const FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    "C:/Windows/Fonts/arial.ttf",
];

/// `GIZMO_FONT`, sonra bilinen sistem yolları, sonra sentetik yüz.
fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    let env = std::env::var("GIZMO_FONT").ok();
    for path in env.iter().map(String::as_str).chain(FONT_CANDIDATES.iter().copied()) {
        if let Ok(id) = renderer.load_font_file(path) {
            return id;
        }
    }
    gizmo::gizmo_log!(Warning, "font bulunamadı; sentetik yüz kutular çizecek (GIZMO_FONT ile seçin)");
    renderer
        .load_font(gizmo::renderer::text::synthetic::synthetic_face())
        .expect("sentetik yüz her zaman ayrıştırılır")
}
