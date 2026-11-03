//! # Pist — üç arabalı, üç turluk bir yarış
//!
//! Stadyum biçiminde bir pist, iki rakip, üç tur. Gaz, fren, direksiyon; köşede küçük bir harita.
//! Arabalar motorun gerçek araç modeliyle sürülüyor (`VehicleController`: süspansiyon, Pacejka
//! lastik, vites) — motorun on altıncı oyunu ve ilk **araç** oyunu.
//!
//! | girdi | iş |
//! |---|---|
//! | **W / ↑ · sol çubuk ileri** | gaz |
//! | **S / ↓ · sol çubuk geri** | fren (dururken geri vites) |
//! | **A D / ← → · sol çubuk** | direksiyon |
//! | **R** | yeniden başla |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **`FixedPhysics` ile sürülen bir araba hiç kıpırdamıyordu.** `SimpleApp` oyunlarının sabit
//!   adımlı fizik sürücüsü yalnızca katı cisim adımını atıyordu; araç, karakter ve dövüş
//!   denetleyicileri yalnızca varsayılan olarak eklenmeyen `GameplayPhysicsPlugin`'de koşuyordu.
//!   Gaz sonuna kadar basılı araba 2,5 saniyede 8·10⁻¹² m gitti. **Düzeltildi:** her adım önce
//!   denetleyicileri, sonra katı cisim adımını koşuyor.
//! - **Köşedeki küçük harita ana görüntüyü karartıyordu.** Bir kameraya `viewport` verilince
//!   çizim bölünmüş ekran yoluna geçiyor ve yalnızca viewport'u olan kameraları çiziyordu —
//!   viewport'u olmayan birincil kamera atlanıyor, ekran haritadan başka simsiyah kalıyordu.
//!   **Düzeltildi:** viewport'u olmayan birincil kamera tüm hedefi kaplayan taban; daha yüksek
//!   `order`'lı küçük bir viewport onun üstüne çiziliyor (resim içinde resim).
//!
//! - **Dört tekerlekli bir araba elle kuruluyordu.** Dört köşe döngüsü, akslar, sol/sağ — ve
//!   `tuning.track_width` / `wheelbase` tekerleklerin yeriyle uyumlu tutulmazsa denge çubuğu ve
//!   direksiyon geometrisi başka bir arabayı hesaplıyor (bu oyun önce öyleydi). **Kapatıldı:**
//!   `VehicleController::four_wheeled(iz, dingil_mesafesi, yarıçap)`.
//!
//! Gerisi motorda vardı: `VehicleController` (dört tekerlek, süspansiyon, Pacejka lastik,
//! otomatik vites), statik bariyerler, `Sky` + `Fog`, kovalayan kamera. Yapay zekâ basit: bir
//! sonraki noktaya dümen, ilerideki virajın keskinliğine göre hız, duvara takılınca geri çıkma —
//! turları tamamlıyor, ama arada bir bariyerde vakit kaybediyor.
//!
//! ## Ortam değişkenleri
//!   * `PIST_OTO=1` — oyuncunun arabasını da yapay zekâ sürer (ekran görüntüsü için)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::core::input::{Input, MoveKeys};
use gizmo::physics::vehicle::VehicleController;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor, Viewport};
use gizmo::renderer::text::FontId;
use gizmo::renderer::{Fog, Sky};
use gizmo::simple::{sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

/// Düzlüklerin yarı boyu ve virajların yarıçapı (pistin orta çizgisi).
const STRAIGHT: f32 = 40.0;
const TURN_R: f32 = 28.0;
/// Pist genişliği.
const WIDTH: f32 = 12.0;
const LAPS: u32 = 3;
/// Orta çizgideki nokta sayısı (kontrol noktaları ve yapay zekânın hedefleri).
const POINTS: usize = 48;
const PLAYER_COLOUR: Vec4 = Vec4::new(0.9, 0.25, 0.15, 1.0);

#[derive(Clone, Copy)]
struct Racer {
    /// Bir sonraki kontrol noktası (orta çizgi indeksi).
    next: usize,
    /// Geçilen kontrol noktası sayısı — sıralama bundan.
    passed: u32,
    finished: Option<f32>,
    ai: bool,
    /// Yapay zekânın hız çarpanı (rakipler biraz farklı).
    pace: f32,
    /// Yapay zekâ: duvara takılı geçen süre, ve geri çıkmanın kalanı.
    stuck: f32,
    backing: f32,
}
gizmo::core::impl_component!(Racer);

struct Game {
    physics: FixedPhysics,
    line: Vec<Vec3>,
    player: u32,
    camera: u32,
    clock: f32,
    countdown: f32,
    hud: u32,
    banner: u32,
    auto: bool,
}

/// Stadyum biçimli orta çizgi: saat yönünün tersine, başlangıç sağ düzlüğün ortasında.
fn centre_line() -> Vec<Vec3> {
    let perimeter = 4.0 * STRAIGHT + 2.0 * std::f32::consts::PI * TURN_R;
    (0..POINTS)
        .map(|i| {
            let mut s = i as f32 / POINTS as f32 * perimeter;
            // Sağ düzlük (x = +R), z: +STRAIGHT → −STRAIGHT yarısından başlayarak.
            let segments = [2.0 * STRAIGHT, std::f32::consts::PI * TURN_R, 2.0 * STRAIGHT, std::f32::consts::PI * TURN_R];
            s += STRAIGHT; // başlangıç düzlüğün ortası
            s %= perimeter;
            let mut k = 0;
            while s > segments[k] {
                s -= segments[k];
                k += 1;
            }
            match k {
                0 => Vec3::new(TURN_R, 0.0, STRAIGHT - s),
                1 => {
                    let a = s / TURN_R;
                    Vec3::new(TURN_R * a.cos(), 0.0, -STRAIGHT - TURN_R * a.sin())
                }
                2 => Vec3::new(-TURN_R, 0.0, -STRAIGHT + s),
                _ => {
                    let a = s / TURN_R;
                    Vec3::new(-TURN_R * a.cos(), 0.0, STRAIGHT + TURN_R * a.sin())
                }
            }
        })
        .collect()
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Pist", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let r = &mut *scene.renderer;
            r.sky = Some(Sky::gradient([0.35, 0.55, 0.9], [0.75, 0.82, 0.9], [0.3, 0.32, 0.25]).with_intensity(0.5));
            r.fog = Some(Fog::new([0.75, 0.82, 0.9], 0.004).with_height(0.0, 0.0));
            let font = load_font(r);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.7) * Quat::from_rotation_x(-0.9)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.96, 0.88), 3.2, LightRole::Sun),
            ));
            let line = centre_line();
            build_track(scene.world, &line);

            let camera = scene.world.spawn_bundle(CameraBundle { position: Vec3::new(0.0, 10.0, 20.0), primary: true, ..Default::default() });
            if let Some(mut c) = scene.world.borrow_mut::<Camera>().get_mut(camera.id()) {
                c.clear_color = Some(Vec3::new(0.75, 0.82, 0.9));
            }
            // Küçük harita: yukarıdan bakan ikinci bir kamera, sağ üst köşede.
            let map = scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 120.0, 0.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -std::f32::consts::FRAC_PI_2 + 0.001,
                ..Default::default()
            });
            if let Some(mut c) = scene.world.borrow_mut::<Camera>().get_mut(map.id()) {
                c.viewport = Some(Viewport::new(0.75, 0.02, 0.23, 0.3));
                c.order = 1;
                c.clear_color = Some(Vec3::new(0.15, 0.2, 0.12));
            }

            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),)).id();
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 48.0, Vec2::new(0.5, 0.35), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();
            let mut game = Game {
                physics: FixedPhysics::new(120),
                line,
                player: 0,
                camera: camera.id(),
                clock: 0.0,
                countdown: 3.0,
                hud,
                banner,
                auto: std::env::var_os("PIST_OTO").is_some(),
            };
            restart(scene.world, &mut game);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn block(at: Vec3, rotation: Quat, half: Vec3, colour: Vec4) -> impl gizmo::core::component::Bundle + Send + Sync + 'static {
    (
        Transform::new(at).with_rotation(rotation).with_scale(half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(colour, 0.85, 0.0),
        MeshRenderer::new(),
    )
}

/// Zemin, orta çizgi boyunca asfalt parçaları ve iki yanda bariyerler (statik kutular).
fn build_track(world: &mut World, line: &[Vec3]) {
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        RigidBody::new_static(),
        Velocity::default(),
        Collider::box_collider(Vec3::new(150.0, 0.5, 150.0)),
    ));
    world.spawn_bundle(block(Vec3::new(0.0, -0.5, 0.0), Quat::IDENTITY, Vec3::new(150.0, 0.5, 150.0), Vec4::new(0.3, 0.45, 0.22, 1.0)));
    for i in 0..line.len() {
        let (a, b) = (line[i], line[(i + 1) % line.len()]);
        let along = b - a;
        let yaw = along.x.atan2(along.z);
        let rotation = Quat::from_rotation_y(yaw);
        let mid = (a + b) * 0.5;
        let half_len = along.length() * 0.5 + 0.6;
        let colour = if i == 0 { Vec4::new(0.9, 0.9, 0.9, 1.0) } else { Vec4::new(0.22, 0.22, 0.24, 1.0) };
        world.spawn_bundle(block(mid + Vec3::Y * 0.01, rotation, Vec3::new(WIDTH * 0.5, 0.01, half_len), colour));
        let side = rotation * Vec3::X;
        for s in [-1.0f32, 1.0] {
            let at = mid + side * s * (WIDTH * 0.5 + 0.6) + Vec3::Y * 0.5;
            let half = Vec3::new(0.3, 0.5, half_len);
            let stripe = if i % 2 == 0 { Vec4::new(0.85, 0.1, 0.1, 1.0) } else { Vec4::new(0.95, 0.95, 0.95, 1.0) };
            world.spawn_bundle(block(at, rotation, half, stripe));
            world.spawn_bundle((
                Transform::new(at).with_rotation(rotation),
                RigidBody::new_static(),
                Velocity::default(),
                Collider::box_collider(half),
            ));
        }
    }
}

fn spawn_car(world: &mut World, at: Vec3, heading: f32, colour: Vec4, racer: Racer) -> u32 {
    let collider = Collider::box_collider(Vec3::new(0.9, 0.35, 2.0));
    let mut body = RigidBody::new(900.0, true);
    body.update_inertia_from_collider(&collider);
    let mut car = VehicleController::four_wheeled(1.7, 2.8, 0.35);
    for wheel in &mut car.wheels {
        wheel.suspension_rest_length = 0.4;
        wheel.suspension_max_travel = 0.3;
    }
    car.max_steering_angle = 0.5;
    let id = world
        .spawn_bundle((
            Transform::new(at + Vec3::Y * 0.8).with_rotation(Quat::from_rotation_y(heading)),
            GlobalTransform::default(),
            body,
            Velocity::default(),
            collider,
            car,
            racer,
        ))
        .id();
    // Görünüş: gövde ve kabin, arabanın çocukları.
    let e = world.get_entity(id).expect("just spawned");
    for (offset, half, c) in [
        (Vec3::ZERO, Vec3::new(0.9, 0.35, 2.0), colour),
        (Vec3::new(0.0, 0.55, 0.3), Vec3::new(0.75, 0.25, 1.0), Vec4::new(0.15, 0.17, 0.22, 1.0)),
    ] {
        let part = world.spawn_bundle(block(offset, Quat::IDENTITY, half, c));
        use gizmo::core::hierarchy::HierarchyExt;
        world.add_child(e, part);
    }
    id
}

fn restart(world: &mut World, game: &mut Game) {
    let racers: Vec<u32> = world.borrow::<Racer>().iter().map(|(id, _)| id).collect();
    for id in racers {
        if let Some(e) = world.get_entity(id) {
            use gizmo::core::hierarchy::HierarchyExt;
            world.despawn_recursive(e);
        }
    }
    game.physics.reset();
    // Izgara: başlangıç çizgisinin arkasında üç sıra. Pist başta −Z yönüne gidiyor.
    let start = game.line[0];
    let heading = 0.0; // araba yerelde −Z'ye bakıyor; pist burada −Z yönünde
    let racer = |ai: bool, pace: f32| Racer { next: 1, passed: 0, finished: None, ai, pace, stuck: 0.0, backing: 0.0 };
    game.player = spawn_car(world, start + Vec3::new(-3.0, 0.0, 6.0), heading, PLAYER_COLOUR, racer(game.auto, 0.9));
    spawn_car(world, start + Vec3::new(3.0, 0.0, 6.0), heading, Vec4::new(0.2, 0.45, 0.9, 1.0), racer(true, 0.86));
    spawn_car(world, start + Vec3::new(0.0, 0.0, 12.0), heading, Vec4::new(0.95, 0.8, 0.2, 1.0), racer(true, 0.92));
    game.clock = 0.0;
    game.countdown = 3.0;
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        restart(world, &mut game);
    }
    let racing = game.countdown <= 0.0;
    if racing {
        game.clock += dt;
    } else {
        game.countdown -= dt;
    }
    drive(world, &game, input, racing, dt);
    game.physics.advance(world, dt);
    sync_transforms(world);
    checkpoints(world, &game);
    chase_camera(world, &game, dt);
    hud(world, &game);
    world.insert_resource(game);
}

/// Oyuncunun girdisi ya da yapay zekâ: bir sonraki kontrol noktasının biraz ötesine dümen kır,
/// virajda gazı kes.
fn drive(world: &mut World, game: &Game, input: &Input, racing: bool, dt: f32) {
    let (wx, wy) = input.move_axis();
    let (ax, ay) = input.move_axis_with(MoveKeys::ARROWS);
    let (mx, my) = (if wx.abs() > ax.abs() { wx } else { ax }, if wy.abs() > ay.abs() { wy } else { ay });
    let racers: Vec<(u32, Racer, Vec3, Quat, f32)> = {
        let racers = world.borrow::<Racer>();
        let transforms = world.borrow::<Transform>();
        let cars = world.borrow::<VehicleController>();
        racers
            .iter()
            .filter_map(|(id, r)| {
                let t = transforms.get(id)?;
                Some((id, *r, t.position, t.rotation, cars.get(id)?.current_speed_kmh))
            })
            .collect()
    };
    for (id, racer, at, rotation, speed) in racers {
        // Sağa açı (yerel +X), ileri yerel −Z. Direksiyonun artısı sola.
        let angle_to = |p: Vec3| {
            let local = rotation.inverse() * (p - at);
            local.x.atan2(-local.z)
        };
        let mut backing = false;
        let (throttle, brake, steer) = if !racing || racer.finished.is_some() {
            (0.0, 1.0, 0.0)
        } else if racer.ai {
            let n = game.line.len();
            let angle = angle_to(game.line[(racer.next + 1) % n]);
            // Viraj: üç nokta ötedeki dönüş ne kadar keskinse o kadar yavaş (28 m'lik virajda
            // tutunma ~60 km/sa'te bitiyor).
            let ahead = angle_to(game.line[(racer.next + 2) % n]).abs().max(angle_to(game.line[(racer.next + 4) % n]).abs() * 0.8);
            let limit = (110.0 * racer.pace - ahead * 150.0).clamp(28.0, 110.0);
            if racer.backing > 0.0 {
                backing = true;
                (0.6, 0.0, (angle * 1.5).clamp(-1.0, 1.0))
            } else {
                let throttle = if speed < limit { 1.0 } else { 0.0 };
                let brake = if speed > limit + 12.0 { 0.7 } else { 0.0 };
                // Hızlıyken direksiyon az: tam kırmak kaydırır.
                let gain = (35.0 / speed.abs().max(1.0)).clamp(0.35, 1.3);
                (throttle, brake, (-angle * gain).clamp(-1.0, 1.0))
            }
        } else {
            (my.max(0.0), (-my).max(0.0), -mx)
        };
        if racer.ai && racing {
            // Duvara dayanıp kalan yapay zekâ bir saniye geri çıkıyor.
            if let Some(mut r) = world.borrow_mut::<Racer>().get_mut(id) {
                if r.backing > 0.0 {
                    r.backing -= dt;
                } else if speed.abs() < 3.0 {
                    r.stuck += dt;
                    if r.stuck > 1.0 {
                        r.backing = 1.2;
                        r.stuck = 0.0;
                    }
                } else {
                    r.stuck = 0.0;
                }
            }
        }
        if let Some(mut car) = world.borrow_mut::<VehicleController>().get_mut(id) {
            // Dururken fren geri vitese geçirir.
            let reverse = backing || (brake > 0.0 && speed < 2.0 && racing && !racer.ai);
            car.set_reverse(reverse);
            car.throttle_input = if reverse && !backing { brake } else { throttle };
            car.brake_input = if reverse { 0.0 } else { brake };
            car.steering_input = steer;
        }
    }
}

/// Bir arabanın bir sonraki kontrol noktasına yakınlaşması onu geçmek sayılır; başlangıç
/// noktasından geçmek bir tur.
fn checkpoints(world: &mut World, game: &Game) {
    let positions: Vec<(u32, Vec3)> = {
        let racers = world.borrow::<Racer>();
        let transforms = world.borrow::<Transform>();
        racers.iter().filter_map(|(id, _)| transforms.get(id).map(|t| (id, t.position))).collect()
    };
    for (id, at) in positions {
        if let Some(mut r) = world.borrow_mut::<Racer>().get_mut(id) {
            let target = game.line[r.next];
            if (at - target).truncate_y_len() < WIDTH {
                r.next = (r.next + 1) % game.line.len();
                r.passed += 1;
                if r.passed >= LAPS * POINTS as u32 && r.finished.is_none() {
                    r.finished = Some(game.clock);
                }
            }
        }
    }
}

trait FlatLen {
    fn truncate_y_len(self) -> f32;
}
impl FlatLen for Vec3 {
    fn truncate_y_len(self) -> f32 {
        Vec2::new(self.x, self.z).length()
    }
}

fn chase_camera(world: &mut World, game: &Game, dt: f32) {
    let Some((at, rotation)) = world.borrow::<Transform>().get(game.player).map(|t| (t.position, t.rotation)) else { return };
    let back = rotation * Vec3::Z;
    let flat = Vec3::new(back.x, 0.0, back.z).normalize_or_zero();
    let want = at + flat * 9.0 + Vec3::Y * 3.5;
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(game.camera) {
        t.position = t.position.lerp(want, (dt * 5.0).min(1.0));
    }
    let from = world.borrow::<Transform>().get(game.camera).map(|t| t.position).unwrap_or(want);
    let look = (at + Vec3::Y * 1.0 - from).normalize_or_zero();
    if let Some(mut c) = world.borrow_mut::<Camera>().get_mut(game.camera) {
        c.yaw = look.z.atan2(look.x);
        c.pitch = look.y.asin();
    }
}

fn hud(world: &mut World, game: &Game) {
    let racers: Vec<(u32, Racer)> = world.borrow::<Racer>().iter().map(|(id, r)| (id, *r)).collect();
    let Some(me) = racers.iter().find(|(id, _)| *id == game.player).map(|(_, r)| *r) else { return };
    let place = 1 + racers.iter().filter(|(id, r)| *id != game.player && (r.passed > me.passed || r.finished.is_some_and(|t| me.finished.is_none_or(|m| t < m)))).count();
    let lap = (me.passed / POINTS as u32 + 1).min(LAPS);
    let speed = world.borrow::<VehicleController>().get(game.player).map_or(0.0, |c| c.current_speed_kmh);
    let text = format!(
        "Tur {lap}/{LAPS}   Sıra {place}/{}   {:.1} sn   {:.0} km/sa\nW/S: gaz/fren   A/D: direksiyon   R: baştan",
        racers.len(),
        game.clock,
        speed.abs()
    );
    set_text(world, game.hud, &text);
    let banner = if game.countdown > 0.0 {
        format!("{}", game.countdown.ceil() as i32)
    } else if let Some(t) = me.finished {
        format!("Bitiş! {place}. — {t:.1} sn\nR ile yeniden")
    } else if game.clock < 1.0 {
        "Başla!".to_string()
    } else {
        String::new()
    };
    set_text(world, game.banner, &banner);
}

fn set_text(world: &mut World, id: u32, s: &str) {
    if let Some(mut t) = world.borrow_mut::<Text>().get_mut(id) {
        if t.content != s {
            t.content = s.to_string();
        }
    }
}


fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    // A font the machine has (`GIZMO_FONT`, or the platform's usual faces); without one, the
    // synthetic test face, so the game still runs.
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
