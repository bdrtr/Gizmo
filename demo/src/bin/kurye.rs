//! # Kurye — paketleri süresi dolmadan götür
//!
//! Meydanda bir paket belirir; kurye süresi dolmadan ona varırsa teslim eder, dolarsa bir ihtar
//! alır ve paket başka yere gider. Sekiz teslimde tur biter, üç ihtarda oyun. SPACE yeniden başlatır.
//!
//! **Durumların ve olayların oyunu**, `sapan` gibi tek bir eklenti:
//!
//! - **Durumlar:** `Asama::{Baslik, Yolda, Bitti}`. Her birine girerken bir kanca kendi varlıklarını
//!   kuruyor ve onları `DespawnOnExit` ile işaretliyor; durumdan çıkınca motor siliyor. Yeniden
//!   başlamak `Bitti → Yolda`: kurye, paket ve sayaçlar sıfırdan, eskilerden geriye bir şey kalmadan.
//! - **Olaylar:** teslim ve gecikme `Teslim` / `Gecikti` olayları. Paketi bulan sistem olayı yazıyor;
//!   puanı sayan, sonraki paketi koyan ve yazıyı yakıp söndüren üç ayrı sistem onu okuyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **WASD / oklar** | sür |
//! | **SPACE** | başla / yeniden başla |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Olaylar yalnız pencerede okunuyordu.** Olay kuyruklarını yalnız pencereli `App`'in
//!   `add_event`'i döndürüyordu; headless `App`'te ve `HeadlessFrames`'te hiçbir kuyruk dönmüyor,
//!   okuyucular hiçbir şey görmüyordu — bu oyunun `--test`'i tek bir teslim sayamazdı. Bir eklenti de
//!   olay türü kaydedemiyordu (`add_event`'e ulaşamıyordu). Oyun döngüsü (`PlayLoop`) fizik
//!   kuyruklarını ayrıca kendisi döndürüyordu: `CollisionEvent`'i `add_event` ile de isteyen bir oyunda
//!   kuyruk karede iki kez dönüyor, her parti okunmadan gidiyordu. **Düzeltildi:** kuyrukları dünya
//!   kaydediyor ve döndürüyor (`World::update_events`), her çalışma zamanının karesi bunu bir kez,
//!   ilk iş olarak çağırıyor.
//! - **Bir eklenti durum kuramıyordu**: `add_state` / `on_enter` / `on_exit` yalnız `App`'teydi. Artık
//!   eklentinin `AppLike`'ında da.
//!
//! ## Ortam değişkenleri
//!   * `KURYE_OTO=1` — kurye kendisi sürer (`kurye --test` aynısını pencere açmadan yapar)

use gizmo::core::input::Input;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, SimpleAppExt, SimpleSceneState};

const GOAL: u32 = 8;
const STRIKES: u32 = 3;
const SPEED: f32 = 7.0;
/// Bir paketin süresi: en uzak köşeye bile yetecek kadar, ama oyalanmaya yetmeyecek kadar.
const PACKAGE_TIME: f32 = 4.5;
/// Paketlerin sırayla belirdiği yerler.
const SPOTS: [(f32, f32); 7] = [(8.0, -6.0), (-9.0, -4.0), (-3.0, 8.0), (10.0, 7.0), (0.0, -10.0), (-10.0, 9.0), (6.0, 1.0)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asama {
    Baslik,
    Yolda,
    Bitti,
}

/// Kurye bir paketi süresi içinde götürdü; kalan süre puana eklenir.
struct Teslim {
    kalan: f32,
}

/// Bir paketin süresi doldu.
struct Gecikti;

struct Kurye {
    auto: bool,
}

/// Bir turun sayaçları; `Yolda`'ya her girişte sıfırlanır.
#[derive(Default)]
struct Tur {
    teslim: u32,
    ihtar: u32,
    puan: f32,
    /// Sıradaki paketin yeri (`SPOTS` içinde).
    sira: usize,
    /// Kaç tur bitti (oyunun ömrü boyunca).
    biten: u32,
}

/// Yazının rengi bir olaydan sonra bir süre değişir; o sürenin sayacı.
#[derive(Default)]
struct Parlama {
    kalan: f32,
    iyi: bool,
}

struct Oto(bool);

/// Varsa yazılar bununla basılır: pencerede sistem yazı tipi, `--test`'te sentetik yüz.
#[derive(Clone, Copy)]
struct Yazi(FontId);

#[derive(Clone, Copy)]
struct Courier;
gizmo::core::impl_component!(Courier);

#[derive(Clone, Copy)]
struct Package {
    kalan: f32,
}
gizmo::core::impl_component!(Package);

#[derive(Clone, Copy)]
struct Hud;
gizmo::core::impl_component!(Hud);

impl Plugin for Kurye {
    fn build(&self, app: &mut dyn AppLike) {
        app.insert_resource(Tur::default())
            .insert_resource(Parlama::default())
            .insert_resource(Oto(self.auto))
            .add_event::<Teslim>()
            .add_event::<Gecikti>()
            .add_state(Asama::Baslik)
            .on_enter(Asama::Baslik, |world| banner(world, "KURYE\nSPACE: başla", Asama::Baslik))
            .on_enter(Asama::Yolda, start_round)
            .on_enter(Asama::Bitti, |world| {
                let tur = world.get_resource::<Tur>().expect("tur");
                let line = if tur.teslim >= GOAL {
                    format!("Tur tamam: {} paket, {:.0} puan\nSPACE: yeniden", tur.teslim, tur.puan)
                } else {
                    format!("{} ihtar: oyun bitti ({} paket)\nSPACE: yeniden", tur.ihtar, tur.teslim)
                };
                drop(tur);
                world.get_resource_mut::<Tur>().expect("tur").biten += 1;
                banner(world, &line, Asama::Bitti);
            })
            .add_systems(Startup, setup_scene)
            .add_systems(Update, start_on_space.run_if(not_playing))
            .add_systems(
                Update,
                (steer, (package_clock, reach_package).chain(), (count_deliveries, next_package, flash), finish_round)
                    .chain()
                    .run_if(in_state(Asama::Yolda)),
            )
            .add_systems(Update, hud);
    }
}

fn not_playing(world: &World) -> bool {
    world.get_resource::<State<Asama>>().is_some_and(|s| *s.get() != Asama::Yolda)
}

/// Kalıcı olanlar: zemin, ışık, kamera, köşe yazısı.
fn setup_scene(mut commands: Commands, yazi: Option<Res<Yazi>>) {
    commands.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.5, 0.0)).with_scale(Vec3::new(14.0, 0.5, 14.0)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Color::srgb_hex("#5B5F66").to_vec4(), 0.95, 0.0),
        MeshRenderer::new(),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(Vec3::new(14.0, 0.5, 14.0)),
    ));
    commands.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-1.0)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 1.8, LightRole::Sun),
    ));
    commands.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 22.0, 15.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.95,
        primary: true,
        ..Default::default()
    });
    if let Some(yazi) = yazi {
        commands.spawn_bundle((Text::pinned("", yazi.0, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)), Hud));
    }
}

/// Bir durumun ortadaki büyük yazısı; o durumdan çıkınca motor siler.
fn banner(world: &mut World, line: &str, of: Asama) {
    let Some(Yazi(font)) = world.get_resource::<Yazi>().map(|y| *y) else { return };
    world.spawn_bundle((
        Text::pinned_at(line, font, 40.0, Vec2::new(0.5, 0.35), Vec2::ZERO).with_anchor(TextAnchor::Center).with_align(TextAlign::Center),
        DespawnOnExit(of),
    ));
}

/// `Yolda`'ya giriş: sayaçlar sıfırdan, kurye meydanın ortasında, ilk paket.
fn start_round(world: &mut World) {
    let biten = world.get_resource::<Tur>().map_or(0, |t| t.biten);
    world.insert_resource(Tur { biten, ..Default::default() });
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 0.5, 0.0)).with_scale(Vec3::new(0.5, 0.4, 0.7)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Color::srgb_hex("#E67E22").to_vec4(), 0.5, 0.0),
        MeshRenderer::new(),
        RigidBody::new(1.0, true),
        Velocity::default(),
        Collider::box_collider(Vec3::new(0.5, 0.4, 0.7)),
        Courier,
        DespawnOnExit(Asama::Yolda),
    ));
    world.spawn_bundle(package(0));
}

/// `sira`'daki yerde bir paket.
fn package(sira: usize) -> impl Bundle {
    let (x, z) = SPOTS[sira % SPOTS.len()];
    (
        Transform::new(Vec3::new(x, 0.5, z)).with_scale(Vec3::splat(0.35)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Color::srgb_hex("#F1C40F").to_vec4(), 0.4, 0.0),
        MeshRenderer::new(),
        Package { kalan: PACKAGE_TIME },
        DespawnOnExit(Asama::Yolda),
    )
}

fn start_on_space(input: Res<Input>, oto: Res<Oto>, time: Res<Time>, mut wait: Local<f32>, mut state: ResMut<State<Asama>>) {
    *wait += time.dt();
    let auto_go = oto.0 && *wait > 1.0;
    if auto_go || input.is_key_just_pressed(KeyCode::Space as u32) {
        *wait = 0.0;
        state.set(Asama::Yolda);
    }
}

/// Sürüş: oyuncu WASD ile, otomatik pilot pakete dümdüz.
fn steer(input: Res<Input>, oto: Res<Oto>, packages: Query<&Transform, With<Package>>, mut couriers: Query<(&Transform, &mut Velocity), With<Courier>>) {
    let target = packages.iter().next().map(|t| t.position);
    for (at, mut v) in couriers.iter_mut() {
        let wish = if oto.0 {
            target.map_or(Vec3::ZERO, |t| Vec3::new(t.x - at.position.x, 0.0, t.z - at.position.z).normalize_or_zero())
        } else {
            let (x, z) = input.move_axis();
            Vec3::new(x, 0.0, -z).normalize_or_zero()
        };
        v.linear = Vec3::new(wish.x * SPEED, v.linear.y, wish.z * SPEED);
    }
}

/// Paketlerin süresi; dolan paket gider ve `Gecikti` yazılır.
fn package_clock(mut commands: Commands, time: Res<Time>, mut packages: Query<(Entity, &mut Package)>, mut late: EventWriter<Gecikti>) {
    for (e, mut p) in packages.iter_mut() {
        p.kalan -= time.dt();
        if p.kalan <= 0.0 {
            commands.entity(e).despawn();
            late.send(Gecikti);
        }
    }
}

/// Kurye pakete değdiyse paket gider ve `Teslim` yazılır.
fn reach_package(mut commands: Commands, couriers: Query<&Transform, With<Courier>>, packages: Query<(Entity, &Transform, &Package)>, mut done: EventWriter<Teslim>) {
    let Some(courier) = couriers.iter().next().map(|t| t.position) else { return };
    for (e, t, p) in packages.iter() {
        if p.kalan > 0.0 && Vec2::new(t.position.x - courier.x, t.position.z - courier.z).length() < 0.8 {
            commands.entity(e).despawn();
            done.send(Teslim { kalan: p.kalan });
        }
    }
}

/// Okuyucu 1: puan ve ihtar.
fn count_deliveries(done: EventReader<Teslim>, late: EventReader<Gecikti>, mut tur: ResMut<Tur>) {
    for t in done.iter() {
        tur.teslim += 1;
        tur.puan += 100.0 + t.kalan * 20.0;
    }
    tur.ihtar += late.len() as u32;
}

/// Okuyucu 2: teslim de olsa gecikme de olsa sıradaki paket.
fn next_package(mut commands: Commands, done: EventReader<Teslim>, late: EventReader<Gecikti>, mut tur: ResMut<Tur>) {
    for _ in 0..done.len() + late.len() {
        tur.sira += 1;
        commands.spawn_bundle(package(tur.sira));
    }
}

/// Okuyucu 3: yazı bir an yeşil ya da kırmızı.
fn flash(done: EventReader<Teslim>, late: EventReader<Gecikti>, time: Res<Time>, mut parlama: ResMut<Parlama>) {
    parlama.kalan = (parlama.kalan - time.dt()).max(0.0);
    if !done.is_empty() {
        *parlama = Parlama { kalan: 0.6, iyi: true };
    }
    if !late.is_empty() {
        *parlama = Parlama { kalan: 0.6, iyi: false };
    }
}

fn finish_round(tur: Res<Tur>, mut state: ResMut<State<Asama>>) {
    if tur.teslim >= GOAL || tur.ihtar >= STRIKES {
        state.set(Asama::Bitti);
    }
}

fn hud(tur: Res<Tur>, parlama: Res<Parlama>, packages: Query<&Package>, mut texts: Query<&mut Text, With<Hud>>) {
    let kalan = packages.iter().next().map_or(0.0, |p| p.kalan);
    let isaret = if parlama.kalan > 0.0 { if parlama.iyi { "  ✓" } else { "  ✗" } } else { "" };
    let line = format!("paket {}/{}   ihtar {}/{}   puan {:.0}   süre {:.1}{isaret}", tur.teslim, GOAL, tur.ihtar, STRIKES, tur.puan, kalan);
    for mut t in texts.iter_mut() {
        if t.content != line {
            t.content = line.clone();
        }
    }
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Kurye", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = scene.renderer.load_system_font().unwrap_or_else(|_| {
                scene.renderer.load_font(gizmo::renderer::text::synthetic::synthetic_face()).expect("sentetik yüz")
            });
            scene.world.insert_resource(Yazi(font));
            scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)), Hud));
        })
        .add_plugin(Kurye { auto: std::env::var_os("KURYE_OTO").is_some() })
        .add_plugin(PhysicsPlugin::default())
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// `--test`: iki tur. Otomatik pilot her turda sekiz paketi ihtarsız götürmeli; olayların her biri
/// bir kez sayılmalı (teslim sayısı, paket sırası ve puan aynı olayları okuyan üç sistemde tutarlı);
/// ve ikinci tur, ilkinden geriye hiçbir varlık kalmadan başlamalı.
fn self_test() -> bool {
    let mut world = World::new();
    world.insert_resource(Yazi(HeadlessFrames::font()));
    let mut run = HeadlessFrames::new(world).add_plugin(Kurye { auto: true }).add_plugin(PhysicsPlugin::default());
    let dt = 1.0 / 60.0;
    let state = |run: &HeadlessFrames| *run.world.get_resource::<State<Asama>>().expect("durum").get();
    let mut rounds = Vec::new();
    let mut entities_at_start = Vec::new();
    while run.elapsed < 120.0 && rounds.len() < 2 {
        let before = state(&run);
        run.frame(dt, |_, _, _, _| {});
        let now = state(&run);
        if before != Asama::Yolda && now == Asama::Yolda {
            entities_at_start.push(run.world.entity_count());
        }
        if before == Asama::Yolda && now == Asama::Bitti {
            let tur = run.world.get_resource::<Tur>().expect("tur");
            rounds.push((tur.teslim, tur.ihtar, tur.sira, tur.puan));
        }
    }
    let leftovers = run.world.query::<&Courier>().iter().count() + run.world.query::<&Package>().iter().count();
    let consistent = rounds.iter().all(|&(teslim, ihtar, sira, puan)| teslim == GOAL && ihtar == 0 && sira as u32 == teslim && puan > 100.0 * GOAL as f32);
    let same_start = entities_at_start.len() == 2 && entities_at_start[0] == entities_at_start[1];
    let ok = rounds.len() == 2 && consistent && same_start && leftovers == 0;
    println!(
        "kurye --test: {:.1} s; turlar (teslim, ihtar, sıra, puan) {:?}; tur başında varlık {:?}; bitince kurye+paket {} → {}",
        run.elapsed,
        rounds,
        entities_at_start,
        leftovers,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}
