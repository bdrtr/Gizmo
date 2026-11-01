//! # Kule Yıkıcı — motorun üstüne kurulmuş küçük ama bitmiş bir oyun
//!
//! Soldaki mancınıktan taş fırlat, sağdaki kulenin tepesindeki **kırmızı hedef kutuları**
//! platformdan düşür. Beş atış hakkın var.
//!
//! | tuş | iş |
//! |---|---|
//! | **↑/↓**, **W/S** ya da gamepad sol çubuk | nişan açısı |
//! | **Boşluk** basılı tut → bırak | güç topla → fırlat |
//! | **R** | bölümü baştan kur |
//!
//! ## Neden var
//!
//! Demolar bir yeteneği tek başına gösterir; bir oyun hepsini **aynı anda** ister. Bu dosya
//! motorun 2D sprite yolunu, ekran üstü HUD'u (sprite + metin, tone-mapping'den sonra), gerçek
//! rigid-body fiziğini (Z ekseni ve X/Y dönmeleri kilitli — düzlemde fizik) ve oyun durumunu
//! birlikte kullanıyor. Yazarken karşılaşılan her eksik aşağıda, ölçüsüyle.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **2D çarpışma şekli yok.** Kutular 3D `Collider::box_collider`, taş 3D küre; düzlemde
//!   tutan şey `RigidBody`'nin eksen kilitleri. Çalışıyor, ama "2D fizik" diye bir yol yok.
//! - **Görsel ile collider elle eşleniyor.** Bir sprite'ın boyu ile collider'ın yarı-boyu iki ayrı
//!   sayı; `spawn_box` ikisini aynı yerden türetsin diye yazıldı. Motor bunu bağlamıyor.
//! - **Bölümü sıfırlamak:** "bölüme ait her şey" bir işaretleyici bileşen (`Level`) ve
//!   `World::despawn_all_with::<Level>()`. Bu yardımcı motorda zaten vardı; oyun ilk yazıldığında
//!   bulunamadı ve döngü elle yazıldı — eksik olan özellik değil, bulunabilirlikti.
//! - **Fontsuz metin.** Repo font taşımıyor (lisans kararı); sistemde font yoksa sentetik yüz
//!   kutular çiziyor. `GIZMO_FONT` ile seçilebilir.

use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{ProjectionMode, Sprite, Text, TextAnchor};
use gizmo::renderer::sprite::{SpriteFilter, SpriteImageId};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};
use gizmo::winit::keyboard::KeyCode;

/// Saniyede kaç fizik adımı. `FixedPhysics` sayesinde kare süresinden bağımsız: aynı atış her
/// makinede aynı yere düşer.
const PHYSICS_HZ: u32 = 120;
const SHOTS: u32 = 5;
/// Mancınığın taşı bıraktığı nokta.
const LAUNCH: Vec2 = Vec2::new(-11.0, 1.6);
/// Hedeflerin durduğu platformun üst yüzeyi; bunun altına inen hedef "düştü" sayılır.
const PLATFORM_TOP: f32 = 2.0;
const GRAVITY: f32 = 9.81;

/// Bölüme ait her varlık — yeniden başlatırken silinecek olanlar.
#[derive(Clone, Copy)]
struct Level;
gizmo::core::impl_component!(Level);

/// Platformdan düşürülmesi gereken kutu.
#[derive(Clone, Copy)]
struct Target;
gizmo::core::impl_component!(Target);

#[derive(Clone, Copy)]
struct Images {
    crate_: SpriteImageId,
    target: SpriteImageId,
    stone: SpriteImageId,
    white: SpriteImageId,
    dot: SpriteImageId,
}

#[derive(Clone, Copy, PartialEq)]
enum Outcome {
    Playing,
    Won,
    Lost,
}

struct Game {
    images: Images,
    angle: f32,
    power: f32,
    charging: bool,
    shots_left: u32,
    /// Son atıştan bu yana geçen süre — "atış bitti ama taşlar hâlâ yuvarlanıyor" ile
    /// "kaybettin"i ayıran şey.
    since_last_shot: f32,
    targets_total: usize,
    outcome: Outcome,
    /// Sabit adımlı fizik. `simple_scene_update` kullanılamıyor, çünkü kamerayı da WASD ile uçuruyor.
    physics: FixedPhysics,
    hud_text: u32,
    banner: u32,
    power_fill: u32,
    shot_icons: Vec<u32>,
    aim_dots: Vec<u32>,
    launcher_arm: u32,
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Kule Yikici", 1280, 720)
        .with_simple_scene(|scene, state| {
            let r = &mut *scene.renderer;
            let load = |r: &mut gizmo::renderer::Renderer, (px, w, h): (Vec<u8>, u32, u32)| {
                r.load_sprite_rgba(&px, w, h, SpriteFilter::Nearest).expect("üretilen görüntü yüklenemedi")
            };
            let images = Images {
                crate_: load(r, crate_image([168, 112, 60], [120, 76, 38])),
                target: load(r, crate_image([215, 60, 50], [140, 30, 28])),
                stone: load(r, disc(16, [130, 130, 140], [90, 90, 100])),
                white: load(r, (vec![255; 4], 1, 1)),
                dot: load(r, disc(8, [255, 255, 255], [255, 255, 255])),
            };
            let sky = load(r, sky());
            let font = load_font(r);
            // Gökyüzü geniş: dikey bir pencerede görüş 30 birimden yükseğe çıkabiliyor.

            // Kalıcı sahne: gökyüzü, zemin görseli, mancınık. Bölümle birlikte silinmez.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 6.0, -5.0)),
                GlobalTransform::default(),
                Sprite::world(sky, Vec2::new(80.0, 60.0)).with_layer(-10),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(LAUNCH.x - 0.2, 0.0, 0.0)),
                GlobalTransform::default(),
                Sprite::world(images.white, Vec2::new(0.6, 1.6))
                    .with_anchor(Vec2::new(0.5, 1.0))
                    .with_color(Vec4::new(0.22, 0.12, 0.05, 1.0))
                    .with_layer(1),
            ));
            let launcher_arm = scene
                .world
                .spawn_bundle((
                    Transform::new(Vec3::new(LAUNCH.x, LAUNCH.y, 0.1)),
                    GlobalTransform::default(),
                    Sprite::world(images.white, Vec2::new(1.8, 0.25))
                        .with_anchor(Vec2::new(0.0, 0.5))
                        .with_color(Vec4::new(0.35, 0.2, 0.08, 1.0))
                        .with_layer(2),
                ))
                .id();

            // HUD — ekran uzayı; tone-mapping'den sonra çizildiği için renkler olduğu gibi.
            let hud = |world: &mut World, s: Sprite| world.spawn_bundle((s,)).id();
            hud(
                scene.world,
                Sprite::screen(images.white, Vec2::new(206.0, 18.0), Vec2::new(16.0, 52.0))
                    .with_color(Vec4::new(0.02, 0.02, 0.03, 0.8)),
            );
            let power_fill = hud(
                scene.world,
                Sprite::screen(images.white, Vec2::new(0.0, 12.0), Vec2::new(19.0, 55.0))
                    .with_color(Vec4::new(0.95, 0.55, 0.05, 1.0))
                    .with_layer(1),
            );
            let shot_icons = (0..SHOTS)
                .map(|i| hud(scene.world, Sprite::screen(images.stone, Vec2::splat(20.0), Vec2::new(16.0 + i as f32 * 24.0, 80.0))))
                .collect();
            let hud_text = scene
                .world
                .spawn_bundle((Text::screen("", font, 22.0, Vec2::new(16.0, 16.0)),))
                .id();
            let banner = scene
                .world
                .spawn_bundle((Text::screen("", font, 40.0, Vec2::new(640.0, 200.0)).with_anchor(TextAnchor::Center),))
                .id();

            let aim_dots = (0..14)
                .map(|_| {
                    scene
                        .world
                        .spawn_bundle((
                            Transform::new(Vec3::ZERO),
                            GlobalTransform::default(),
                            Sprite::world(images.dot, Vec2::splat(0.18))
                                .with_color(Vec4::new(1.0, 1.0, 1.0, 0.6))
                                .with_layer(5),
                        ))
                        .id()
                })
                .collect();

            scene.spawn_camera(state, Vec3::new(0.0, 5.0, 20.0), Vec3::new(0.0, 5.0, 0.0));
            if let Some(mut q) = scene.world.query_mut::<(Mut<Camera>, Mut<Transform>)>() {
                for (_, (mut cam, _)) in q.iter_mut() {
                    cam.projection =
                        ProjectionMode::OrthographicFit { width: VIEW_WIDTH, height: VIEW_MIN_HEIGHT };
                }
            }

            let mut game = Game {
                images,
                angle: 0.6,
                power: 0.0,
                charging: false,
                shots_left: SHOTS,
                since_last_shot: 0.0,
                targets_total: 0,
                outcome: Outcome::Playing,
                physics: FixedPhysics::new(PHYSICS_HZ),
                hud_text,
                banner,
                power_fill,
                shot_icons,
                aim_dots,
                launcher_arm,
            };
            build_level(scene.world, &mut game);
            scene.world.insert_resource(game);
        })
        .set_update(|world, _state, dt, input| {
            update(world, dt.min(0.1), input);
            sync_transforms(world);
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Bölümü kurar: zemin, platform, kule, hedefler. Önceki bölümden kalan her şeyi siler.
fn build_level(world: &mut World, game: &mut Game) {
    world.despawn_all_with::<Level>();

    let img = game.images;
    // Zemin ve platform: statik gövdeler.
    // Renkler LİNEER: 0,3 ekranda sRGB 149 olur; doygun bir ton için düşük kanallar gerçekten düşük.
    spawn_box(world, Vec2::new(0.0, -3.0), Vec2::new(40.0, 3.0), None, img.white, Vec4::new(0.05, 0.16, 0.03, 1.0));
    spawn_box(world, Vec2::new(8.0, 1.0), Vec2::new(4.0, 1.0), None, img.white, Vec4::new(0.12, 0.11, 0.10, 1.0));

    // Kule: iki sütun, üstlerinde kalaslar, en tepede hedefler. Platformun üstü y = 2.
    let mut targets = 0;
    let mut y = PLATFORM_TOP;
    for _floor in 0..3 {
        for x in [5.5, 10.5] {
            spawn_box(world, Vec2::new(x, y + 0.75), Vec2::new(0.35, 0.75), Some(2.0), img.crate_, Vec4::ONE);
        }
        y += 1.5;
        spawn_box(world, Vec2::new(8.0, y + 0.2), Vec2::new(3.0, 0.2), Some(3.0), img.crate_, Vec4::ONE);
        y += 0.4;
        let e = spawn_box(world, Vec2::new(8.0, y + 0.5), Vec2::new(0.5, 0.5), Some(1.0), img.target, Vec4::ONE);
        world.add_component(e, Target);
        targets += 1;
    }
    for x in [6.2, 9.8] {
        let e = spawn_box(world, Vec2::new(x, PLATFORM_TOP + 0.5), Vec2::new(0.5, 0.5), Some(1.0), img.target, Vec4::ONE);
        world.add_component(e, Target);
        targets += 1;
    }

    game.targets_total = targets;
    game.shots_left = SHOTS;
    game.since_last_shot = 0.0;
    game.outcome = Outcome::Playing;
    game.power = 0.0;
    game.charging = false;
    // Yeniden başlarken birikmiş fizik zamanı yeni bölüme taşınmasın.
    game.physics.reset();
}

/// Bir kutu: görseli ve collider'ı AYNI yarı-boydan. `mass: None` statik demek.
fn spawn_box(world: &mut World, center: Vec2, half: Vec2, mass: Option<f32>, image: SpriteImageId, tint: Vec4) -> Entity {
    let collider = Collider::box_collider(Vec3::new(half.x, half.y, 0.5));
    let body = match mass {
        Some(m) => planar(RigidBody::new(m, true)),
        None => RigidBody::new_static(),
    };
    world.spawn_bundle((
        Level,
        Transform::new(Vec3::new(center.x, center.y, 0.0)),
        GlobalTransform::default(),
        Sprite::world(image, half * 2.0).with_color(tint),
        body,
        Velocity::default(),
        collider,
    ))
}

/// Gövdeyi XY düzlemine kilitler: Z'de kayamaz, yalnız Z ekseni etrafında döner.
fn planar(mut body: RigidBody) -> RigidBody {
    body.lock_translation_z = true;
    body.lock_rotation_x = true;
    body.lock_rotation_y = true;
    body
}

fn update(world: &mut World, dt: f32, input: &gizmo::core::input::Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };

    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        build_level(world, &mut game);
    }

    // Nişan: ok tuşları, W/S ya da çubuk — hepsi `move_axis`'tan.
    let (_, wasd) = input.move_axis();
    let (_, arrows) = input.move_axis_with(gizmo::core::input::MoveKeys::ARROWS);
    let aim = if arrows.abs() > wasd.abs() { arrows } else { wasd };
    game.angle = (game.angle + aim * 1.2 * dt).clamp(0.05, 1.45);
    fit_camera(world);

    let can_shoot = game.outcome == Outcome::Playing && game.shots_left > 0;
    if can_shoot && input.is_key_pressed(KeyCode::Space as u32) {
        game.charging = true;
        game.power = (game.power + dt * 0.9).min(1.0);
    } else if game.charging {
        game.charging = false;
        if can_shoot {
            fire(world, &game);
            game.shots_left -= 1;
            game.since_last_shot = 0.0;
        }
        game.power = 0.0;
    }
    game.since_last_shot += dt;

    // Sabit adımlı fizik.
    game.physics.advance(world, dt);

    // Oyun durumu: hedeflerden kaçı platformun altına indi?
    let down = {
        let targets = world.borrow::<Target>();
        let tf = world.borrow::<Transform>();
        targets
            .iter()
            .filter(|(id, _)| tf.get(*id).is_some_and(|t| t.position.y < PLATFORM_TOP - 0.3))
            .count()
    };
    if game.outcome == Outcome::Playing {
        if down == game.targets_total {
            game.outcome = Outcome::Won;
        } else if game.shots_left == 0 && game.since_last_shot > 4.0 {
            game.outcome = Outcome::Lost;
        }
    }

    draw_ui(world, &game, down);
    world.insert_resource(game);
}

/// Sahnenin genişliği — mancınıktan kulenin ötesine. Pencere ne şekilde olursa olsun sığmalı.
const VIEW_WIDTH: f32 = 30.0;
const VIEW_MIN_HEIGHT: f32 = 16.0;

/// Kamerayı zemine oturtur. Sığdırmayı motor yapıyor (`OrthographicFit`: pencere hangi şekilde
/// olursa olsun en az 30 × 16 birim gösterir); burada kalan iş, görünen yüksekliği
/// (`view_extent`) okuyup görüşün alt kenarını zeminin hemen altına koymak.
///
/// Bu fonksiyon önce yüksekliği en-boy oranından elle hesaplıyordu: sabit `Orthographic
/// { height }` dikey bir pencerede mancınığı ekran dışında bırakıyordu (ölçüldü: 948×1028'de
/// görünen genişlik ±7,4 birim, mancınık x = −11).
fn fit_camera(world: &mut World) {
    let Some(info) = world.get_resource::<gizmo::core::window::WindowInfo>().map(|i| *i) else { return };
    let aspect = info.width / info.height.max(1.0);
    if let Some(mut q) = world.query_mut::<(Mut<Camera>, Mut<Transform>)>() {
        for (_, (cam, mut t)) in q.iter_mut() {
            if let Some(extent) = cam.projection.view_extent(aspect) {
                // Zemin ekranın altına otursun: görüşün alt kenarı y ≈ −1.
                t.position.y = extent.y * 0.5 - 1.0;
            }
        }
    }
}

fn launch_velocity(angle: f32, power: f32) -> Vec2 {
    Vec2::new(angle.cos(), angle.sin()) * (7.0 + power * 15.0)
}

fn fire(world: &mut World, game: &Game) {
    let v = launch_velocity(game.angle, game.power);
    let r = 0.35;
    let mut body = planar(RigidBody::new(4.0, true));
    body.ccd_enabled = true;
    world.spawn_bundle((
        Level,
        Transform::new(Vec3::new(LAUNCH.x, LAUNCH.y, 0.0)),
        GlobalTransform::default(),
        Sprite::world(game.images.stone, Vec2::splat(r * 2.0)).with_layer(3),
        body,
        Velocity::new(Vec3::new(v.x, v.y, 0.0)),
        Collider::sphere(r),
    ));
}

fn draw_ui(world: &mut World, game: &Game, down: usize) {
    // Nişan çizgisi: balistik yol, çarpışmasız. Güç toplanmıyorken orta güçle gösterilir.
    let power = if game.charging { game.power } else { 0.5 };
    let v = launch_velocity(game.angle, power);
    let show = game.outcome == Outcome::Playing && game.shots_left > 0;
    {
        let mut tf = world.borrow_mut::<Transform>();
        for (i, &dot) in game.aim_dots.iter().enumerate() {
            let t = 0.08 * (i + 1) as f32;
            let p = LAUNCH + v * t - Vec2::new(0.0, 0.5 * GRAVITY * t * t);
            if let Some(mut d) = tf.get_mut(dot) {
                d.position = Vec3::new(p.x, p.y, 0.2);
            }
        }
        if let Some(mut arm) = tf.get_mut(game.launcher_arm) {
            arm.rotation = Quat::from_rotation_z(game.angle);
        }
    }
    {
        let mut sprites = world.borrow_mut::<Sprite>();
        for &dot in &game.aim_dots {
            if let Some(mut s) = sprites.get_mut(dot) {
                s.color.w = if show { 0.6 } else { 0.0 };
            }
        }
        if let Some(mut s) = sprites.get_mut(game.power_fill) {
            s.size.x = 200.0 * game.power;
        }
        for (i, &icon) in game.shot_icons.iter().enumerate() {
            if let Some(mut s) = sprites.get_mut(icon) {
                s.color.w = if (i as u32) < game.shots_left { 1.0 } else { 0.15 };
            }
        }
    }
    let size = world
        .get_resource::<gizmo::core::window::WindowInfo>()
        .map(|i| Vec2::new(i.width, i.height))
        .unwrap_or(Vec2::new(1280.0, 720.0));
    let mut texts = world.borrow_mut::<Text>();
    if let Some(mut t) = texts.get_mut(game.hud_text) {
        t.content = format!("Hedef {down}/{}   Atış {}", game.targets_total, game.shots_left);
    }
    if let Some(mut t) = texts.get_mut(game.banner) {
        t.content = match game.outcome {
            Outcome::Playing => String::new(),
            Outcome::Won => "KULE YIKILDI!\nR ile yeniden".into(),
            Outcome::Lost => "Atışlar bitti\nR ile yeniden".into(),
        };
        t.color = match game.outcome {
            Outcome::Won => Vec4::new(1.0, 0.85, 0.2, 1.0),
            _ => Vec4::new(1.0, 1.0, 1.0, 1.0),
        };
        t.space = gizmo::renderer::components::TextSpace::Screen { position: Vec2::new(size.x * 0.5, size.y * 0.3) };
    }
}

// ── Prosedürel görüntüler ───────────────────────────────────────────────────────────────

fn image(w: u32, h: u32, mut f: impl FnMut(u32, u32) -> [u8; 4]) -> (Vec<u8>, u32, u32) {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&f(x, y));
        }
    }
    (px, w, h)
}

/// Çerçeveli, çapraz destekli bir sandık.
fn crate_image(fill: [u8; 3], edge: [u8; 3]) -> (Vec<u8>, u32, u32) {
    image(16, 16, |x, y| {
        let border = x == 0 || y == 0 || x == 15 || y == 15;
        let brace = x == y || x + y == 15;
        let c = if border || brace { edge } else { fill };
        [c[0], c[1], c[2], 255]
    })
}

fn disc(n: u32, fill: [u8; 3], rim: [u8; 3]) -> (Vec<u8>, u32, u32) {
    image(n, n, |x, y| {
        let c = (n as f32 - 1.0) * 0.5;
        let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt() / (n as f32 * 0.5);
        if d > 1.0 {
            [0, 0, 0, 0]
        } else if d > 0.8 {
            [rim[0], rim[1], rim[2], 255]
        } else {
            [fill[0], fill[1], fill[2], 255]
        }
    })
}

fn sky() -> (Vec<u8>, u32, u32) {
    image(1, 32, |_, y| {
        let t = y as f32 / 31.0;
        let l = |a: f32, b: f32| (a + (b - a) * t) as u8;
        [l(70.0, 200.0), l(130.0, 225.0), l(210.0, 240.0), 255]
    })
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
