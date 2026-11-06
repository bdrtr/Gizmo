//! # Kayık — şamandıra kapılarından tekne yarışı
//!
//! Bir göl, altı şamandıra kapısı, bir motorlu kayık. Kapılardan sırayla geç, süre işliyor; ara
//! sıra yandan bir rüzgâr darbesi kayığı yatırıyor. Motorun otuz birinci oyunu ve **suyun
//! taşıdığı** ilki: kayık, şamandıralar ve göl bir `FluidZone`.
//!
//! | girdi | iş |
//! |---|---|
//! | **W / S · sol çubuk** | gaz / geri |
//! | **A / D · sol çubuk** | dümen |
//! | **R** | baştan |
//!
//! `kayik --test` pencere açmaz: otomatik pilot üç dakika sürer, her dört saniyede bir yandan
//! rüzgâr vurur; kaç kapıdan geçtiği, kayığın en çok kaç derece yattığı ve suyun ne kadar
//! altında yüzdüğü yazılır. (`KAYIK_OTO=1` pencereli oyunda da otomatik pilotu açar.)
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Su, yüzeni yanlış derinlikte ve hiç doğrultmadan taşıyordu.** Kaldırma, gövdenin yalnız
//!   *merkezi* suyun içindeyken ve tek bir kuvvet olarak ağırlık merkezine uygulanıyordu: yoğunluğu
//!   350 olan bu kayık %35 yerine yarı yarıya batık yüzüyordu, yandan vuran rüzgâr onu yatırınca
//!   hiçbir şey geri döndürmüyordu, dönerken de su ona hiç direnmiyordu. **Düzeltildi:** kaldırma
//!   ve sürüklenme gövdenin içinden geçen 3×3×3 hücre üzerinden, her hücrenin kendi yerinde.
//! - **Sürekli bir itme, kare hızına göre farklı itiyordu.** Motorun kuvveti
//!   `RigidBody::force_accumulator`'a karede bir yazılırsa, sabit adımlı sürücüde kare başına
//!   sıfır, bir ya da birkaç adım koştuğu için 30 fps'te yarısı, 144 fps'te (`+=` ile) iki
//!   buçuk katı uygulanıyordu. **Düzeltildi:** `ExternalForce` — bileşen gövdede durdukça her
//!   adımda.

use gizmo::core::input::Input;
use gizmo::physics::components::ExternalForce;
use gizmo::physics::world::{FluidZone, PhysicsWorld, ZoneShape};
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::simple::{set_text, sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};

const LAKE: f32 = 120.0;
/// Kapıların ortası; her kapı iki şamandıra, aralarında `GATE_WIDTH`.
const GATES: [(f32, f32); 6] = [(0.0, -30.0), (28.0, -50.0), (45.0, -15.0), (20.0, 20.0), (-25.0, 10.0), (-30.0, -30.0)];
const GATE_WIDTH: f32 = 8.0;
const HULL: Vec3 = Vec3::new(0.8, 0.3, 2.2);
const HULL_DENSITY: f32 = 350.0;
const THRUST: f32 = 9000.0;

fn hull_mass() -> f32 {
    HULL_DENSITY * 8.0 * HULL.x * HULL.y * HULL.z
}

fn spawn_lake(world: &mut World) {
    let mut physics = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    physics.fluid_zones.push(FluidZone {
        shape: ZoneShape::Box { min: Vec3::new(-LAKE, -30.0, -LAKE), max: Vec3::new(LAKE, 0.0, LAKE) },
        density: 1000.0,
        linear_drag: 150.0,
        quadratic_drag: 150.0,
        ..Default::default()
    });
    world.insert_resource(physics);
}

fn spawn_boat(world: &mut World, visual: bool) -> Entity {
    let collider = Collider::box_collider(HULL);
    let mut body = RigidBody::new(hull_mass(), true);
    body.update_inertia_from_collider(&collider);
    let id = world
        .spawn_bundle((Transform::new(Vec3::new(0.0, 0.1, 0.0)), body, Velocity::default(), collider, ExternalForce::default()))
        ;
    if visual {
        if let Some(e) = world.live(id) {
            world.add_component(e, GlobalTransform::default());
            for (offset, half, colour) in [
                (Vec3::ZERO, HULL, Color::srgb_hex("#ECF0F1").to_vec4()),
                (Vec3::new(0.0, 0.3, 0.4), Vec3::new(0.6, 0.25, 0.7), Color::srgb_hex("#C0392B").to_vec4()),
            ] {
                let part = world.spawn_bundle((
                    Transform::new(offset).with_scale(half),
                    GlobalTransform::default(),
                    MeshSource("standard_cube".into()),
                    MaterialDesc::new().with_pbr(colour, 0.5, 0.1),
                    MeshRenderer::new(),
                ));
                use gizmo::core::hierarchy::HierarchyExt;
                world.add_child(e, part);
            }
        }
    }
    id
}

/// Kapının iki şamandırası — yüzen küreler.
fn spawn_buoys(world: &mut World, visual: bool) -> Vec<Entity> {
    let mut ids = Vec::new();
    for (k, &(x, z)) in GATES.iter().enumerate() {
        let across = gate_across(k);
        for side in [-0.5f32, 0.5] {
            let at = Vec3::new(x, 0.0, z) + across * (side * GATE_WIDTH);
            let col = Collider::sphere(0.45);
            let mut rb = RigidBody::new(200.0 * col.volume(), true);
            rb.update_inertia_from_collider(&col);
            let id = world.spawn_bundle((Transform::new(at), rb, Velocity::default(), col));
            if visual {
                if let Some(e) = world.live(id) {
                    world.add_component(e, GlobalTransform::default());
                    world.add_component(e, MeshSource("sphere".into()));
                    world.add_component(e, MaterialDesc::new().with_pbr(Color::srgb_hex("#BDC3C7").to_vec4(), 0.4, 0.0));
                    world.add_component(e, MeshRenderer::new());
                    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(id) {
                        t.scale = Vec3::splat(0.45);
                    }
                }
            }
            ids.push(id);
        }
    }
    ids
}

/// Kapıdan geçiş yönüne dik — iki şamandırayı ayıran doğrultu.
fn gate_across(k: usize) -> Vec3 {
    let here = Vec2::from(GATES[k]);
    let prev = if k == 0 { Vec2::ZERO } else { Vec2::from(GATES[k - 1]) };
    let along = (here - prev).normalize_or_zero();
    Vec3::new(-along.y, 0.0, along.x)
}

fn gate_centre(k: usize) -> Vec3 {
    let (x, z) = GATES[k % GATES.len()];
    Vec3::new(x, 0.0, z)
}

/// Motor ve dümen: `throttle` −1…1, `rudder` −1…1. Kuvvet kıçta, suyun altında; dümen, hızla
/// orantılı bir sapma momenti; omurga yana kaymayı söndürür.
fn steer(world: &mut World, boat: Entity, throttle: f32, rudder: f32) {
    let (t, v, w) = {
        let ts = world.borrow::<Transform>();
        let vs = world.borrow::<Velocity>();
        let Some(t) = ts.get(boat).copied() else { return };
        let vel = vs.get(boat).copied().unwrap_or_default();
        (t, vel.linear, vel.angular)
    };
    let forward = t.rotation * Vec3::NEG_Z;
    let side = t.rotation * Vec3::X;
    let stern = t.position + t.rotation * Vec3::new(0.0, -0.25, HULL.z);
    let flat_forward = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let speed = v.dot(flat_forward);
    let thrust = flat_forward * (throttle * THRUST);
    // Omurga: yana kaymanın büyük kısmını alır, ki dümen kayığı döndürsün, kaydırmasın. Yatay:
    // yatmış bir kayıkta bordanın doğrultusu eğik, ve eğik bir kuvvet kayığı sudan kaldırırdı.
    let flat_side = Vec3::new(side.x, 0.0, side.z).normalize_or_zero();
    let keel = -flat_side * (v.dot(flat_side) * hull_mass() * 2.0);
    let yaw = Vec3::Y * (-rudder * speed.clamp(-6.0, 6.0) * 900.0 - w.y * 1500.0);
    // Sintine omurgası: yalpayı söndürür. `FluidZone`'un sürüklenmesi her yönde aynı ve bir
    // tekne gövdesinin yalpaya karşı koyan biçimini bilmiyor; o payı gövdenin kendisi veriyor.
    let roll = -forward * (w.dot(forward) * 2500.0) - side * (w.dot(side) * 2500.0);
    if let Some(mut push) = world.borrow_mut::<ExternalForce>().get_mut(boat) {
        *push = ExternalForce::at_point(thrust, stern, t.position).with_torque(yaw + roll);
        push.force += keel;
    }
}

/// Otomatik pilot: sıradaki kapının ortasına dön, gaz.
fn autopilot(world: &World, boat: Entity, target: Vec3) -> (f32, f32) {
    let Some(t) = world.borrow::<Transform>().get(boat).copied() else { return (0.0, 0.0) };
    let forward = t.rotation * Vec3::NEG_Z;
    let f = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let to = Vec3::new(target.x - t.position.x, 0.0, target.z - t.position.z).normalize_or_zero();
    let angle = f.cross(to).y.atan2(f.dot(to));
    let throttle = if angle.abs() > 1.2 { 0.4 } else { 1.0 };
    (throttle, (-angle * 2.0).clamp(-1.0, 1.0))
}

/// Kapıdan geçti mi: kapının ortasından yarım genişlikten yakın.
fn through_gate(p: Vec3, k: usize) -> bool {
    Vec2::new(p.x, p.z).distance(Vec2::from(GATES[k])) < GATE_WIDTH * 0.5
}

/// Yandan bir rüzgâr darbesi: kayığın üst kısmına, bordasına dik bir itme.
fn gust(world: &mut World, boat: Entity, strength: f32) {
    let Some(t) = world.borrow::<Transform>().get(boat).copied() else { return };
    let side = t.rotation * Vec3::X;
    let impulse = side * strength;
    let point = t.position + t.rotation * Vec3::new(0.0, 0.6, 0.0);
    let com = t.position;
    let Some((inv_mass, inv_inertia)) = world.borrow_mut::<RigidBody>().get_mut(boat).map(|mut rb| {
        rb.wake_up();
        (rb.inv_mass(), rb.inv_world_inertia_tensor(t.rotation))
    }) else {
        return;
    };
    if let Some(mut v) = world.borrow_mut::<Velocity>().get_mut(boat) {
        v.linear += impulse * inv_mass;
        v.angular += inv_inertia * (point - com).cross(impulse);
    }
}

fn tilt_degrees(world: &World, id: Entity) -> f32 {
    world.borrow::<Transform>().get(id).map_or(0.0, |t| (t.rotation * Vec3::Y).angle_between(Vec3::Y).to_degrees())
}

/// `--test`: üç dakika otomatik pilot, dört saniyede bir rüzgâr.
fn self_test() -> bool {
    let mut world = World::new();
    spawn_lake(&mut world);
    let boat = spawn_boat(&mut world, false);
    let buoys = spawn_buoys(&mut world, false);
    let mut physics = FixedPhysics::new(60);
    let dt = 1.0 / 60.0;
    let (mut next, mut worst_tilt, mut draft_sum, mut draft_n) = (0usize, 0.0f32, 0.0f32, 0u32);
    let mut buoy_low = f32::MAX;
    for frame in 0..60 * 180 {
        let (throttle, rudder) = if frame < 120 { (0.0, 0.0) } else { autopilot(&world, boat, gate_centre(next)) };
        steer(&mut world, boat, throttle, rudder);
        if frame > 120 && frame % 240 == 0 {
            gust(&mut world, boat, 700.0);
        }
        physics.advance(&mut world, dt);
        world.apply_commands();
        let p = world.borrow::<Transform>().get(boat).map(|t| t.position).unwrap_or(Vec3::ZERO);
        if !p.is_finite() {
            break;
        }
        worst_tilt = worst_tilt.max(tilt_degrees(&world, boat));
        if (60..120).contains(&frame) {
            draft_sum += HULL.y - p.y;
            draft_n += 1;
        }
        if frame > 600 {
            for &b in &buoys {
                buoy_low = buoy_low.min(world.borrow::<Transform>().get(b).map_or(0.0, |t| t.position.y));
            }
        }
        if through_gate(p, next) {
            next += 1;
            if next == GATES.len() {
                break;
            }
        }
    }
    let draft = draft_sum / draft_n.max(1) as f32 / (2.0 * HULL.y);
    let ok = next == GATES.len() && worst_tilt < 45.0 && (draft - HULL_DENSITY / 1000.0).abs() < 0.05 && buoy_low > -0.6;
    println!(
        "kayik --test: {} / {} kapı; en çok {:.0}° yattı; gövdenin %{:.0}'i suyun altında (Arşimet %{:.0}); şamandıralar en derin {:.2} m → {}",
        next,
        GATES.len(),
        worst_tilt,
        draft * 100.0,
        HULL_DENSITY / 10.0,
        buoy_low,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

struct Game {
    physics: FixedPhysics,
    boat: Entity,
    buoys: Vec<Entity>,
    next: usize,
    clock: f32,
    gust_in: f32,
    finished: Option<f32>,
    camera: Entity,
    hud: Entity,
    banner: Entity,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Kayık", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = scene.renderer.default_font();
            spawn_lake(scene.world);
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.02, 0.0)).with_scale(Vec3::new(LAKE, 0.01, LAKE)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#1F6F8B").to_vec4(), 0.15, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.9)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.95, 0.85), 2.2, LightRole::Sun),
            ));
            let boat = spawn_boat(scene.world, true);
            let buoys = spawn_buoys(scene.world, true);
            let camera = scene
                .world
                .spawn_bundle(CameraBundle { position: Vec3::new(0.0, 8.0, 14.0), pitch: -0.4, primary: true, ..Default::default() })
                ;
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                ;
            scene.world.insert_resource(Game {
                physics: FixedPhysics::new(60),
                boat,
                buoys,
                next: 0,
                clock: 0.0,
                gust_in: 6.0,
                finished: None,
                camera,
                hud,
                banner,
            });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        if let Some(e) = world.live(game.boat) {
            use gizmo::core::hierarchy::HierarchyExt;
            world.despawn_recursive(e);
        }
        game.boat = spawn_boat(world, true);
        game.next = 0;
        game.clock = 0.0;
        game.finished = None;
        game.physics.reset();
    }
    let (mx, my) = input.move_axis();
    let auto = std::env::var_os("KAYIK_OTO").is_some();
    let (throttle, rudder) = if auto { autopilot(world, game.boat, gate_centre(game.next)) } else { (my, mx) };
    let throttle = if game.finished.is_some() { 0.0 } else { throttle };
    steer(world, game.boat, throttle, rudder);
    game.gust_in -= dt;
    let mut gusted = false;
    if game.gust_in <= 0.0 {
        gust(world, game.boat, 700.0);
        game.gust_in = 5.0 + (game.clock * 1.7).sin().abs() * 4.0;
        gusted = true;
    }
    game.physics.advance(world, dt);
    sync_transforms(world);
    if game.finished.is_none() {
        game.clock += dt;
    }
    let t = world.borrow::<Transform>().get(game.boat).copied().unwrap_or_default();
    if game.finished.is_none() && through_gate(t.position, game.next) {
        game.next += 1;
        if game.next == GATES.len() {
            game.finished = Some(game.clock);
        }
    }
    for (i, &b) in game.buoys.iter().enumerate() {
        if let Some(mut m) = world.borrow_mut::<MaterialDesc>().get_mut(b) {
            m.albedo = match (i / 2).cmp(&game.next) {
                std::cmp::Ordering::Less => Color::srgb_hex("#27AE60").to_vec4(),
                std::cmp::Ordering::Equal => Color::srgb_hex("#F39C12").to_vec4(),
                std::cmp::Ordering::Greater => Color::srgb_hex("#BDC3C7").to_vec4(),
            };
        }
    }
    if let Some(mut c) = world.borrow_mut::<Transform>().get_mut(game.camera) {
        let back = t.rotation * Vec3::Z;
        let want = t.position + Vec3::new(back.x, 0.0, back.z).normalize_or_zero() * 10.0 + Vec3::Y * 4.5;
        c.position = c.position.lerp(want, (dt * 2.5).min(1.0));
        let look = (t.position - c.position).normalize_or_zero();
        c.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, look);
    }
    let speed = world.borrow::<Velocity>().get(game.boat).map_or(0.0, |v| Vec2::new(v.linear.x, v.linear.z).length());
    let target = gate_centre(game.next);
    let dist = Vec2::new(t.position.x - target.x, t.position.z - target.z).length();
    set_text(
        world,
        game.hud,
        &format!(
            "kapı {}/{}   sıradakine {:.0} m\n{:.1} düğüm   yatma {:.0}°   süre {:.1} s{}\nW/S: gaz   A/D: dümen   R: baştan",
            game.next.min(GATES.len()),
            GATES.len(),
            dist,
            speed * 1.944,
            tilt_degrees(world, game.boat),
            game.finished.unwrap_or(game.clock),
            if gusted { "   — rüzgâr!" } else { "" }
        ),
    );
    let banner = game.finished.map(|s| format!("Bitti: {s:.1} s\nR ile yeniden")).unwrap_or_default();
    set_text(world, game.banner, &banner);
    world.insert_resource(game);
}
