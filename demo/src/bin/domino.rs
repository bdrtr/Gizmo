//! # Domino — bir topun devirdiği, kıvrılan bir taş dizisi
//!
//! Otuz ince taş bir düz, bir çeyrek çember ve yine bir düzden oluşan bir yol boyunca dikilir,
//! uyurlar; atılan bir top ilkini devirir ve zincir sonuna kadar gider.
//!
//! Motorun hangi yollarını yürüttüğü, ve onun için yazıldığı:
//!
//! - **İnce kutular dik durur ve uyur**: 10 cm kalınlığında, 1 m boyunda taşlar, birbirine
//!   dokunmadan. Dikilen taşın kendi başına devrilmemesi, kımıldamaması ve uyuması gerekir.
//! - **Uyuyan bir dizi zincirleme uyanır**: her taş, bir öncekinin değmesiyle uyanmalıdır. Ada
//!   her temasla büyür; uyuyan bir taşın yanındaki uyanık bir taş onu uyandırmazsa zincir kopar.
//! - **Kenar kenara temas**: devrilen taş bir sonrakinin sırtına kenarıyla dayanır ve kayarak
//!   iter. Sonunda bütün dizi birbirine yaslanmış bir yığındır.
//! - **Küre–kutu teması**: zinciri başlatan top bir kürenin taşa değmesidir.
//!
//! `domino --test`: pencere açmadan taşların yerleşip uyumasını bekler, topu atar; bütün taşlar
//! sırayla devrilmeli, hiçbiri yolundan kopmamalı, ve yığın durulup uyumalı — her taş bir
//! sonrakine `cos θ = kalınlık / aralık` açısında (~78.5°) yaslanmış, geometrinin söylediği gibi.
use gizmo::core::input::Input;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, HeadlessFrames, SimpleAppExt, SimpleSceneState};

const PHYSICS_HZ: u32 = 120;
/// Taş sayısı ve yol boyunca aralığı, metre.
const STONES: usize = 30;
const SPACING: f32 = 0.5;
/// Bir taşın yarı boyları: kalınlık (yol yönünde), boy, en.
const STONE_HALF: Vec3 = Vec3::new(0.05, 0.5, 0.25);
/// Yolun ilk düzünün ve çeyrek çemberinin uzunluğu; ikinci düz geri kalanıdır.
const FIRST_RUN: f32 = 6.0;
const ARC_RUN: f32 = 5.0;
/// Topun atıldığı an, saniye: taşlar o zamana kadar yerleşip uyur.
const THROW_AT: f32 = 2.5;
const BALL_R: f32 = 0.15;
/// Bir taş, dikeyden bu kadar yattığında devrilmiş sayılır. Zincirin sonunda taşlar birbirine
/// ~24°'de yaslanır (aralık 0.5 m, boy 1 m); dik duran bir taş 1°'nin altındadır.
const FALLEN: f32 = 15.0 * std::f32::consts::PI / 180.0;

#[derive(Clone, Copy)]
struct Stone(usize);
gizmo::core::impl_component!(Stone);

struct Game {
    physics: FixedPhysics,
    clock: f32,
    thrown: bool,
    /// Her taşın devrildiği an, saniye; devrilmediyse `None`.
    fell_at: Vec<Option<f32>>,
    /// Atıştan hemen önce, taşların kaçı uyuyordu.
    asleep_at_throw: usize,
    hud: Entity,
}

/// Yolun `s` metresindeki yer ve yönü (y ekseni çevresinde açı): `FIRST_RUN` metre +x'e, sonra
/// sola dönen bir çeyrek çember, sonra −z'ye.
fn path(s: f32) -> (Vec3, f32) {
    let r = ARC_RUN / std::f32::consts::FRAC_PI_2;
    if s < FIRST_RUN {
        (Vec3::new(s, 0.0, 0.0), 0.0)
    } else if s < FIRST_RUN + ARC_RUN {
        let phi = (s - FIRST_RUN) / r;
        (Vec3::new(FIRST_RUN + r * phi.sin(), 0.0, -r + r * phi.cos()), phi)
    } else {
        let phi = std::f32::consts::FRAC_PI_2;
        (Vec3::new(FIRST_RUN + r, 0.0, -r - (s - FIRST_RUN - ARC_RUN)), phi)
    }
}

/// Taş `i`'nin dikildiği yer ve dönüşü: ince ekseni (yerel x) yolun yönünde.
fn stone_pose(i: usize) -> (Vec3, Quat) {
    let (at, yaw) = path(i as f32 * SPACING);
    (at + Vec3::Y * STONE_HALF.y, Quat::from_rotation_y(yaw))
}

/// Bir taşın dikeyden yatıklığı, radyan.
fn tilt(rotation: Quat) -> f32 {
    (rotation * Vec3::Y).y.clamp(-1.0, 1.0).acos()
}

fn setup(world: &mut World, font: FontId) -> Game {
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.9)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(4.0, 7.0, 8.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -0.6,
        primary: true,
        ..Default::default()
    });
    // Zemin yolun kapladığı yerden, her yanında 3 m payla: aralığı ya da taş sayısını değiştiren
    // biri son taşları boşluğa dikmesin.
    let (lo, hi) = (0..STONES).map(|i| stone_pose(i).0).fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p)));
    let floor_half = Vec3::new((hi.x - lo.x) * 0.5 + 3.0, 0.1, (hi.z - lo.z) * 0.5 + 3.0);
    let floor_at = Vec3::new((lo.x + hi.x) * 0.5, -0.1, (lo.z + hi.z) * 0.5);
    world.spawn_bundle((
        Transform::new(floor_at).with_scale(floor_half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.35, 0.4, 0.35, 1.0), 0.8, 0.0),
        MeshRenderer::new(),
        Collider::box_collider(floor_half),
    ));
    for i in 0..STONES {
        let (at, rotation) = stone_pose(i);
        let shade = 0.15 + 0.7 * i as f32 / STONES as f32;
        world.spawn_bundle((
            Transform::new(at).with_rotation(rotation).with_scale(STONE_HALF),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(Vec4::new(0.9, shade, 0.2, 1.0), 0.4, 0.0),
            MeshRenderer::new(),
            RigidBody::new(0.5, true),
            Velocity::default(),
            Collider::box_collider(STONE_HALF),
            Stone(i),
        ));
    }
    let hud = world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
    Game {
        physics: FixedPhysics::new(PHYSICS_HZ),
        clock: 0.0,
        thrown: false,
        fell_at: vec![None; STONES],
        asleep_at_throw: 0,
        hud,
    }
}

/// İlk taşa, tepesine doğru atılan top.
fn throw_ball(world: &mut World) {
    world.spawn_bundle((
        Transform::new(Vec3::new(-2.0, 0.9, 0.0)).with_scale(Vec3::splat(BALL_R)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.2, 0.3, 0.85, 1.0), 0.3, 0.2),
        MeshRenderer::new(),
        // Hafif ve yavaş: bir itiş, bir darbe değil. 0.65 s'lik bir yay, taşın tepesine (y ≈ 0.8).
        RigidBody::new(0.2, true),
        Velocity::new(Vec3::new(3.0, 3.0, 0.0)),
        Collider::sphere(BALL_R),
    ));
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, _input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.clock += dt;
    if !game.thrown && game.clock >= THROW_AT {
        game.asleep_at_throw = world.query::<(&RigidBody, &Stone)>().iter().filter(|(rb, _)| rb.is_sleeping).count();
        throw_ball(world);
        game.thrown = true;
    }
    game.physics.advance(world, dt);
    sync_transforms(world);

    let clock = game.clock;
    for (t, stone) in world.query::<(&Transform, &Stone)>().iter() {
        if game.fell_at[stone.0].is_none() && tilt(t.rotation) > FALLEN {
            game.fell_at[stone.0] = Some(clock);
        }
    }
    let fallen = game.fell_at.iter().flatten().count();
    if let Some(mut text) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        text.content = format!("Devrilen: {fallen} / {STONES}");
    }
    world.insert_resource(game);
}

/// `--test`: taşlar yerleşip uyur, top atılır; hepsi sırayla devrilmeli, yolunda kalmalı, ve yığın
/// geometrinin söylediği açıda durulup uyumalı.
fn self_test() -> bool {
    let mut world = World::new();
    let game = setup(&mut world, HeadlessFrames::font());
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let dt = 1.0 / 60.0;
    let mut standing_before_throw = true;
    while run.elapsed < THROW_AT + 25.0 {
        run.frame(dt, update);
        let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
        if !game.thrown {
            let leaning = run.world.query::<(&Transform, &Stone)>().iter().map(|(t, _)| tilt(t.rotation)).fold(0.0f32, f32::max);
            standing_before_throw &= leaning < 1.0f32.to_radians();
        }
        if game.fell_at.iter().all(Option::is_some) {
            break;
        }
    }
    // Zincir bittikten sonra yığının durulması: hiçbir taş yolundan kopmamalı, zeminin altına inmemeli.
    for _ in 0..180 {
        run.frame(dt, update);
    }
    let game = run.world.get_resource::<Game>().expect("oyun kaynağı");
    let asleep_after = run.world.query::<(&RigidBody, &Stone)>().iter().filter(|(rb, _)| rb.is_sleeping).count();
    // Yığının şekli: yaslanan iki taş, aralarındaki dik uzaklık kalınlıklarına inince değer — tabanları
    // `SPACING` aralıklı paralel iki levha için `cos θ = kalınlık / aralık`, ~78.5°. Sonuncusu yerde
    // yatar. Çeyrek çemberde komşular 9° açılı, onun için 3°'lik pay.
    let shingle = (2.0 * STONE_HALF.x / SPACING).acos();
    let worst_shingle = run
        .world
        .query::<(&Transform, &Stone)>()
        .iter()
        .filter(|(_, s)| s.0 + 1 < STONES)
        .map(|(t, _)| (tilt(t.rotation) - shingle).abs())
        .fold(0.0f32, f32::max);
    let shingled = worst_shingle < 3.0f32.to_radians();
    let fallen = game.fell_at.iter().flatten().count();
    let in_order = game.fell_at.windows(2).all(|w| match (w[0], w[1]) {
        (Some(a), Some(b)) => a <= b,
        _ => true,
    });
    let strayed: Vec<String> = run
        .world
        .query::<(&Transform, &Stone)>()
        .iter()
        .filter_map(|(t, s)| {
            let (home, _) = stone_pose(s.0);
            let d = t.position - home;
            let away = Vec2::new(d.x, d.z).length();
            (away > 1.2 || t.position.y < 0.0).then(|| format!("{} ({away:.2} m, y {:.2})", s.0, t.position.y))
        })
        .collect();
    let took = match (game.fell_at.first().copied().flatten(), game.fell_at.last().copied().flatten()) {
        (Some(a), Some(b)) => b - a,
        _ => f32::NAN,
    };
    let ok = standing_before_throw
        && game.asleep_at_throw == STONES
        && fallen == STONES
        && in_order
        && strayed.is_empty()
        && shingled
        && asleep_after == STONES;
    println!(
        "domino --test: atıştan önce {}/{STONES} uyuyor, {}; devrilen {fallen}/{STONES}, {}, zincir {took:.2} s, yolundan kopan {}; yığın {:.1}°'den en çok {:.1}° sapık, {asleep_after}/{STONES} uyuyor → {}",
        game.asleep_at_throw,
        if standing_before_throw { "hepsi dik" } else { "biri yattı" },
        if in_order { "sırayla" } else { "SIRASIZ" },
        if strayed.is_empty() { "yok".to_string() } else { strayed.join(", ") },
        shingle.to_degrees(),
        worst_shingle.to_degrees(),
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Domino", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font);
            scene.world.insert_resource(game);
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
