//! # Çoban — bir köpek, on iki koyunu ağıla sürer
//!
//! Koyunlar otlakta dağınık başlar. Birbirlerine sokulur, aynı yöne bakar, kalabalıktan kaçınır,
//! çit direklerinden uzak durur ve köpekten kaçar; köpek onları kuzeydeki ağıla sürer. Ağıla giren
//! koyun içeride kalır, köpek kapıda bekler.
//!
//! Motorun hangi yollarını yürüttüğü — oyunların hiçbirinin dokunmadığı **sürü yönlendirmesi**:
//! her koyun her karede `combined_steering` ile yön bulur (ayrılma, birleşme, hizalanma, engelden
//! kaçınma), ve köpekten kaçışı `seek` ile, köpeğin aynadaki noktasına.
//!
//! Köpeğin otomatik pilotu Strömbom'un (2014) çobanlık kuralı: bir koyun sürüden fazla
//! uzaklaşmışsa onun arkasına geçip toplar, yoksa sürünün ağılın karşı tarafına geçip sürer.
//!
//! Bu oyunla bulunan ve düzeltilen iki kusur, ikisi de "sürü hiç durulmuyor":
//!
//! - **`alignment` hızı değil yönü kopyalıyordu**: komşuların ortalama hızını normalleştirip tam
//!   hız istiyordu. Sürüden kopan altı koyun, köpek 46 m ötedeyken ve her biri otlamak için fren
//!   yaparken, bir dakikadan uzun süre 0.47 m/s ile yürüdü; fren ne kadar hız alırsa hizalanma geri
//!   istedi. Artık ortalama hızı (tavanı `max_speed`) ister.
//! - **`separate`'in itişi yarıçapta sıçrıyordu**: `max_speed / uzaklık`, yarıçapı geçen komşu için
//!   bir anda sıfıra iniyordu ve 1 m'de (birimden bağımsız olmayan) doyuyordu. Ağıldaki sürü o
//!   kenarda titreşti: bir dakika sonra hâlâ 0.04 m/s. Artık yarıçapta sıfırdan başlar, yarısında
//!   tam hıza varır (`avoid_obstacles` da aynı yasayla).
//!
//! `coban --test`: pencere açmadan oynar; koyunların hepsi iki dakika içinde ağılda olmalı, hiçbir
//! koyun bir çit direğinin içine girmemeli, ve ağılda sürü durulmalı: köpek geride beklerken
//! yirmi saniye sonra koyunların ortalama hızı 0.02 m/s'nin altında olmalı (eski `separate` ile
//! 0.04'te kalıyordu).
use gizmo::ai::steering::{combined_steering, SteeringWeights};
use gizmo::core::input::Input;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, SimpleAppExt, SimpleSceneState};

const SHEEP: usize = 12;
const SHEEP_SPEED: f32 = 2.0;
const SHEEP_FORCE: f32 = 4.0;
const DOG_SPEED: f32 = 3.5;
/// Koyun köpeği bu kadar yakında görürse kaçar.
const FEAR: f32 = 10.0;
/// Strömbom'un etkileşim uzaklığı: köpeğin sürüye durduğu mesafenin ölçüsü.
const R_A: f32 = 1.5;
/// Ağıl: merkezi, yarı genişliği ve kapısı (güney duvarında, yarı genişliği).
const PEN: Vec3 = Vec3::new(0.0, 0.0, 14.0);
const PEN_HALF: f32 = 4.0;
const GATE_HALF: f32 = 1.6;
const POST_R: f32 = 0.25;
/// Direklerin kaçınma yarıçapı: koyun bu kadar yaklaşınca itilir.
const POST_AVOID: f32 = 0.9;
const SHEEP_R: f32 = 0.3;
/// Hepsi ağıla girdikten bu kadar sonra sürünün durulmuş olması gerekir.
const SETTLE_SECS: f32 = 20.0;
const SETTLED_SPEED: f32 = 0.02;

#[derive(Clone, Copy, Default)]
struct Sheep {
    velocity: Vec3,
}
gizmo::core::impl_component!(Sheep);

#[derive(Clone, Copy)]
struct Dog;
gizmo::core::impl_component!(Dog);

struct Game {
    posts: Vec<Vec3>,
    hud: Entity,
}

/// Ağılın çit direkleri: dört duvar, güneydekinde bir kapı.
fn fence_posts() -> Vec<Vec3> {
    let mut posts = Vec::new();
    let n = (2.0 * PEN_HALF / 0.5) as i32;
    for i in 0..=n {
        let s = -PEN_HALF + i as f32 * 0.5;
        posts.push(PEN + Vec3::new(s, 0.0, PEN_HALF)); // kuzey
        posts.push(PEN + Vec3::new(-PEN_HALF, 0.0, s)); // batı
        posts.push(PEN + Vec3::new(PEN_HALF, 0.0, s)); // doğu
        if s.abs() > GATE_HALF {
            posts.push(PEN + Vec3::new(s, 0.0, -PEN_HALF)); // güney, kapısıyla
        }
    }
    posts
}

fn in_pen(p: Vec3) -> bool {
    (p.x - PEN.x).abs() < PEN_HALF - 0.2 && (p.z - PEN.z).abs() < PEN_HALF - 0.2
}

fn material(r: f32, g: f32, b: f32) -> MaterialDesc {
    MaterialDesc::new().with_pbr(Vec4::new(r, g, b, 1.0), 0.7, 0.0)
}

fn setup(world: &mut World, font: FontId) {
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-1.0)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.92), 2.5, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(0.0, 30.0, -8.0),
        yaw: std::f32::consts::FRAC_PI_2,
        pitch: -1.2,
        primary: true,
        ..Default::default()
    });
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.1, 4.0)).with_scale(Vec3::new(16.0, 0.1, 16.0)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        material(0.35, 0.6, 0.25),
        MeshRenderer::new(),
    ));
    let posts = fence_posts();
    for p in &posts {
        world.spawn_bundle((
            Transform::new(*p + Vec3::Y * 0.4).with_scale(Vec3::new(POST_R, 0.4, POST_R)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            material(0.45, 0.3, 0.15),
            MeshRenderer::new(),
        ));
    }
    // Koyunlar güneyde, dağınık: belirlenimli bir sözde rastgele serpme.
    let mut seed = 2024u32;
    let mut rand = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    for _ in 0..SHEEP {
        let at = Vec3::new(-8.0 + 16.0 * rand(), SHEEP_R, -6.0 + 8.0 * rand());
        world.spawn_bundle((
            Transform::new(at).with_scale(Vec3::splat(SHEEP_R)),
            GlobalTransform::default(),
            MeshSource("sphere".into()),
            material(0.95, 0.95, 0.9),
            MeshRenderer::new(),
            Sheep::default(),
        ));
    }
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 0.3, -11.0)).with_scale(Vec3::splat(0.3)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        material(0.15, 0.1, 0.08),
        MeshRenderer::new(),
        Dog,
    ));
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    world.insert_resource(Game { posts, hud });
}

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// Köpeğin bu karede gitmek istediği yer.
fn dog_goal(sheep: &[Vec3], dog: Vec3) -> Vec3 {
    let outside: Vec<Vec3> = sheep.iter().copied().filter(|p| !in_pen(*p)).collect();
    if outside.is_empty() {
        // Hepsi içeride: kapının önünde, koyunları korkutmayacak kadar geride bekle.
        return PEN + Vec3::new(0.0, 0.0, -PEN_HALF - FEAR - 1.0);
    }
    let centre = outside.iter().copied().sum::<Vec3>() / outside.len() as f32;
    let reach = R_A * (outside.len() as f32).powf(2.0 / 3.0);
    let gate = PEN + Vec3::new(0.0, 0.0, -PEN_HALF + 1.0);
    // Toplama: sürüden en uzak koyun fazla uzaktaysa onun arkasına.
    let (far, far_d) = outside.iter().map(|p| (*p, flat(*p - centre).length())).fold((centre, 0.0), |a, b| if b.1 > a.1 { b } else { a });
    if far_d > reach {
        let behind = flat(far - centre).normalize_or_zero();
        return far + behind * R_A;
    }
    // Sürme: sürünün, kapının tam karşı tarafına.
    let away = flat(centre - gate).normalize_or(Vec3::NEG_Z);
    let _ = dog;
    centre + away * R_A * (outside.len() as f32).sqrt()
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, _input: &Input) {
    let posts = world.get_resource::<Game>().map(|g| g.posts.clone()).unwrap_or_default();
    let flock: Vec<(Entity, Vec3, Vec3)> = world.query::<(Entity, &Transform, &Sheep)>().iter().map(|(e, t, s)| (e, t.position, s.velocity)).collect();
    let Some((dog_e, dog)) = world.query::<(Entity, &Transform, &Dog)>().iter().next().map(|(e, t, _)| (e, t.position)) else { return };

    // Köpek.
    let goal = dog_goal(&flock.iter().map(|f| f.1).collect::<Vec<_>>(), dog);
    let to = flat(goal - dog);
    let step = to.clamp_length_max(DOG_SPEED * dt);
    if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(dog_e) {
        t.position += step;
    }

    // Koyunlar.
    let obstacles: Vec<(Vec3, f32)> = posts.iter().map(|p| (*p + Vec3::Y * SHEEP_R, POST_AVOID)).collect();
    for &(e, p, v) in &flock {
        let neighbours: Vec<(Vec3, Vec3)> = flock.iter().filter(|o| o.0 != e).map(|o| (o.1, o.2)).collect();
        let near: Vec<(Vec3, f32)> = obstacles.iter().copied().filter(|(o, r)| o.distance(p) < r + 0.5).collect();
        let from_dog = flat(p - dog);
        let urgency = (1.0 - from_dog.length() / FEAR).max(0.0);
        // Köpekten kaçış, hedef terimi olarak: köpeğin aynadaki noktasına `seek`, ne kadar yakınsa
        // o kadar güçlü. Bütçeyi (`max_force`) engelden kaçınmayla paylaşır, ki koyun kaçarken
        // direğe girmesin. Tehdit altında sürüye daha sıkı sokulur.
        let mut weights = SteeringWeights::default();
        weights.seek = 3.0 * urgency;
        weights.cohesion = 1.0 + 0.4 * urgency;
        let target = (urgency > 0.0).then_some(p + from_dog);
        let mut steer = combined_steering(p, v, target, &near, &neighbours, &weights, SHEEP_SPEED, SHEEP_FORCE, (1.2, 8.0, 3.0));
        if urgency == 0.0 {
            // Otlarken yavaşla.
            steer += -v * 0.8;
        }
        let mut v = flat(v + steer.clamp_length_max(SHEEP_FORCE) * dt).clamp_length_max(SHEEP_SPEED);
        if !v.is_finite() {
            v = Vec3::ZERO;
        }
        if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(e) {
            t.position += v * dt;
        }
        if let Some(mut s) = world.query_mut::<&mut Sheep>().get_mut(e) {
            s.velocity = v;
        }
    }

    let penned = world.query::<(&Transform, &Sheep)>().iter().filter(|(t, _)| in_pen(t.position)).count();
    if let Some(hud) = world.get_resource::<Game>().map(|g| g.hud) {
        if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(hud) {
            text.content = format!("Ağılda: {penned} / {SHEEP}");
        }
    }
}

struct Outcome {
    penned_at: Option<f32>,
    deepest_post: f32,
    settled_speed: f32,
    non_finite: bool,
}

fn play() -> Outcome {
    let mut world = World::new();
    setup(&mut world, HeadlessFrames::font());
    let posts = fence_posts();
    let mut run = HeadlessFrames::new(world);
    let mut penned_at = None;
    let mut deepest_post = 0.0f32;
    let mut non_finite = false;
    let mut settled_speed = f32::NAN;
    while run.elapsed < 120.0 + SETTLE_SECS {
        run.frame(1.0 / 60.0, update);
        let sheep: Vec<(Vec3, Vec3)> = run.world.query::<(&Transform, &Sheep)>().iter().map(|(t, s)| (t.position, s.velocity)).collect();
        non_finite |= sheep.iter().any(|(p, v)| !p.is_finite() || !v.is_finite());
        for (p, _) in &sheep {
            for post in &posts {
                deepest_post = deepest_post.max(POST_R + SHEEP_R - flat(*p - *post).length());
            }
        }
        if penned_at.is_none() && sheep.iter().all(|(p, _)| in_pen(*p)) {
            penned_at = Some(run.elapsed);
        }
        if let Some(t) = penned_at {
            if run.elapsed >= t + SETTLE_SECS {
                settled_speed = sheep.iter().map(|(_, v)| v.length()).sum::<f32>() / sheep.len() as f32;
                break;
            }
        }
    }
    Outcome { penned_at, deepest_post, settled_speed, non_finite }
}

/// `--test`: bkz. modül belgesi.
fn self_test() -> bool {
    let o = play();
    let ok = o.penned_at.is_some_and(|t| t < 120.0) && o.deepest_post <= 0.0 && o.settled_speed < SETTLED_SPEED && !o.non_finite;
    println!(
        "coban --test: hepsi ağılda {}, direğe en derin giriş {:.2} m, {SETTLE_SECS:.0} saniye sonra ortalama hız {:.4} m/s{} → {}",
        o.penned_at.map_or("hiç".to_string(), |t| format!("{t:.0} s'de")),
        o.deepest_post,
        o.settled_speed,
        if o.non_finite { ", SONLU OLMAYAN DEĞER" } else { "" },
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Çoban", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            setup(scene.world, font);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    // Makinenin bir yazı tipi (`GIZMO_FONT` ya da platformun olağan yüzleri); yoksa sentetik test
    // yüzü, ki oyun yine çalışsın.
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
