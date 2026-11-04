//! # İtiş — iki oyunculu, ağ üzerinden, rollback'li
//!
//! Kare bir pist, iki ağır pul. Kendi pulunu sür, ötekini pistten düşür; üç düşüren kazanır.
//! Motorun yirmi ikinci oyunu ve **ağ** kullanan ilki: iki taraf aynı fiziği kendi makinesinde
//! simüle eder, yalnız girdiler gider gelir; geç gelen bir girdi eski bir tahmini bozarsa oturum
//! o tick'e geri sarar ve yeniden simüle eder (`gizmo::net::rollback::RollbackSession`).
//!
//! | kip | nasıl |
//! |---|---|
//! | yerel iki kişi | `itis` — iki oturum aynı süreçte, aralarında 4 tick gecikmeli bir sahte ağ; 1. oyuncu **WASD**, 2. oyuncu **ok tuşları** |
//! | ağ | `itis --port 7000 --peer 127.0.0.1:7001 --player 0` ve öbür makinede `--port 7001 --peer <ip>:7000 --player 1` |
//! | sınama | `itis --test` — pencere açmaz: iki oturumu önce gecikmeli, paket kaybeden bir sahte ağda, sonra 127.0.0.1 üstünde gerçek UDP soketleriyle 1500 tick oynatır; iki taraf birbiriyle ve gerçek maçla karşılaştırılır |
//!
//! Oyun bitince **Boşluk** yeni maç başlatır (iki taraftan biri basınca).
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Önde başlayan taraf hep önde kalıyordu.** Oturum karşı tarafı hiç beklemiyordu; ikinci
//!   pencere birkaç saniye sonra açılınca birincinin bütün tahminleri geri sarma penceresinin
//!   dışında düzeltildi ve iki taraf bir daha aynı durumu görmedi. **Düzeltildi:**
//!   `RollbackSession::try_advance` öndeki tarafı `max_lead` tick'te tutuyor.
//! - **Skor fizikle birlikte geri sarılmıyordu.** Oturum yalnız `PhysicsWorld`'ü saklıyordu; bir
//!   düşüşü tahmin üstünden sayan taraf, geri sarmadan sonra o düşüş hiç olmamış olsa da sayıyı
//!   tutuyordu. **Düzeltildi:** `RollbackSession::with_state` — oyunun kendi durumu her tick'in
//!   anlık görüntüsüyle saklanıyor ve geri sarılıyor; kurallar `try_advance_with`'in adım
//!   sonrası kancasında, geri sarmada da çalışıyor.
//! - **`cargo test -p gizmo-net` derlenmiyordu** — bir test dosyası `rollback` özelliğine
//!   bağlıydı ama bunu söylemiyordu. **Düzeltildi.**

use gizmo::core::input::{Input, MoveKeys};
use gizmo::net::rollback::{LoopbackTransport, PlayerInput, RollbackSession, Transport, UdpTransport};
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{BodyHandle, Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

const DT: f32 = 1.0 / 60.0;
const ARENA: f32 = 5.0;
const PUCK_R: f32 = 0.6;
const PUCK_MASS: f32 = 8.0;
const ACCEL: f32 = 14.0;
const MAX_SPEED: f32 = 7.0;
const WIN: u8 = 3;
const MAX_ROLLBACK: u64 = 60;
const MAX_LEAD: u64 = 8;
/// Gövdeler eklendikleri sırayla: zemin, 1. pul, 2. pul.
const PUCK: [usize; 2] = [1, 2];
const START: [Vec3; 2] = [Vec3::new(-2.5, PUCK_R, 0.0), Vec3::new(2.5, PUCK_R, 0.0)];
/// Girdi düğmesi: maç bittiyse yenisini başlat.
const BUTTON_RESTART: u32 = 1;

/// Fizikle birlikte geri sarılan oyun durumu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Match {
    score: [u8; 2],
    winner: Option<u8>,
}

fn build_world() -> PhysicsWorld {
    let mut w = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    w.add_body(
        BodyHandle::from_id(0),
        RigidBody::new_static(),
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(ARENA, 0.5, ARENA)),
    );
    for (i, at) in START.iter().enumerate() {
        let col = Collider::sphere(PUCK_R);
        let mut rb = RigidBody::new(PUCK_MASS, true);
        rb.update_inertia_from_collider(&col);
        w.add_body(BodyHandle::from_id(1 + i as u32), rb, Transform::new(*at), Velocity::default(), col);
    }
    w
}

/// Bir oyuncunun bu tick'teki girdisi: pulunu itmek, maç bittiyse yenisini istemek.
fn apply(w: &mut PhysicsWorld, m: &mut Match, player: u32, input: &PlayerInput) {
    if m.winner.is_some() {
        if input.buttons & BUTTON_RESTART != 0 {
            *m = Match::default();
            reset_pucks(w);
        }
        return;
    }
    let idx = PUCK[player as usize & 1];
    let dir = Vec3::new(input.joystick_x as f32, 0.0, -(input.joystick_y as f32)) / 127.0;
    if dir.length_squared() > 0.0 {
        w.rigid_bodies[idx].wake_up();
        let v = &mut w.velocities[idx].linear;
        *v += dir * ACCEL * DT;
        let flat = Vec3::new(v.x, 0.0, v.z);
        if flat.length() > MAX_SPEED {
            let capped = flat.normalize() * MAX_SPEED;
            v.x = capped.x;
            v.z = capped.z;
        }
    }
}

/// Her adımdan sonra (geri sarmadaki yeniden simülasyonda da): düşen pul ötekine sayı yazar.
fn rules(w: &mut PhysicsWorld, m: &mut Match) {
    if m.winner.is_some() {
        return;
    }
    for (p, &idx) in PUCK.iter().enumerate() {
        if w.transforms[idx].position.y < -2.0 {
            let other = 1 - p;
            m.score[other] += 1;
            if m.score[other] >= WIN {
                m.winner = Some(other as u8);
            }
            reset_pucks(w);
            return;
        }
    }
}

fn reset_pucks(w: &mut PhysicsWorld) {
    for (p, &idx) in PUCK.iter().enumerate() {
        w.transforms[idx].position = START[p];
        w.transforms[idx].rotation = Quat::IDENTITY;
        w.velocities[idx] = Velocity::default();
        w.rigid_bodies[idx].wake_up();
    }
}

type Session<T> = RollbackSession<T, Match>;

fn session<T: Transport>(transport: T, local: u32) -> Session<T> {
    RollbackSession::with_state(build_world(), Match::default(), transport, local, 1 - local, MAX_ROLLBACK, DT)
}

fn quantise(x: f32) -> i8 {
    (x.clamp(-1.0, 1.0) * 127.0).round() as i8
}

/// Bir maçı iki oturumla sonuna kadar oynatır: biri öbüründen 40 kare sonra başlar; `pause`
/// verilirse her karede o kadar beklenir (gerçek soketlerde paket o arada gelir).
fn play_out<T: Transport>(
    a: &mut Session<T>,
    b: &mut Session<T>,
    inputs: &[Vec<PlayerInput>; 2],
    end: u64,
    pause: Option<std::time::Duration>,
) {
    for frame in 0..end * 3 {
        for (s, p, from) in [(&mut *a, 0usize, 0u64), (&mut *b, 1usize, 40)] {
            if frame < from {
                continue;
            }
            if s.tick < end {
                let i = inputs[p][s.tick as usize];
                s.try_advance_with(i, &apply, &rules, MAX_LEAD);
            } else {
                s.poll_remote();
            }
        }
        if a.tick >= end && b.tick >= end && frame > end + 60 {
            break;
        }
        if let Some(d) = pause {
            std::thread::sleep(d);
        }
    }
}

/// `--test`: iki oturum, önce 4 tick gecikmeli ve her 9. paketi düşüren bir sahte ağda, sonra
/// 127.0.0.1 üstünde iki gerçek UDP soketiyle; biri öbüründen 40 kare sonra başlıyor, ikisi de
/// rastgele sürüyor. Sonda iki tarafın fiziği ve skoru aynı mı, ve iki girdinin tek bir dünyada
/// oynatıldığı gerçek maçla aynı mı?
fn self_test() -> bool {
    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    const TICKS: u64 = 1500;
    // Her taraf girdisini kendi tick'ine göre seçer: aynı tick hep aynı girdi.
    let mut inputs: [Vec<PlayerInput>; 2] = [Vec::new(), Vec::new()];
    for t in 0..TICKS + 60 {
        for (p, list) in inputs.iter_mut().enumerate() {
            let r = next();
            let idle = t >= TICKS;
            list.push(PlayerInput {
                tick: t,
                buttons: if !idle && r % 97 == 0 { BUTTON_RESTART } else { 0 },
                joystick_x: if idle { 0 } else { (((r >> 8) % 255) as i32 - 127) as i8 },
                joystick_y: if idle || p == 1 { 0 } else { (((r >> 16) % 255) as i32 - 127) as i8 },
            });
        }
    }
    let end = TICKS + 60;
    // Gerçek maç: ağ yok, her tick iki girdi oyuncu sırasıyla.
    let (mut truth_world, mut truth) = (build_world(), Match::default());
    for (i0, i1) in inputs[0].iter().zip(&inputs[1]) {
        apply(&mut truth_world, &mut truth, 0, i0);
        apply(&mut truth_world, &mut truth, 1, i1);
        let _ = truth_world.step(DT);
        rules(&mut truth_world, &mut truth);
    }
    let verdict = |name: &str, a_tick: u64, b_tick: u64, ha: u64, hb: u64, sa: Match, sb: Match| {
        let agree = a_tick == b_tick && ha == hb && sa == sb;
        let real = ha == truth_world.state_hash() && sa == truth;
        println!(
            "itis --test ({name}): tick {a_tick} / {b_tick}, hash {ha:016x} / {hb:016x} (gerçek {:016x}), skor {:?} / {:?} (gerçek {:?}) → {}",
            truth_world.state_hash(),
            sa.score,
            sb.score,
            truth.score,
            match (agree, real) {
                (true, true) => "aynı, gerçekle de",
                (true, false) => "birbirleriyle aynı ama gerçekten FARKLI",
                _ => "AYRIŞTI",
            }
        );
        agree && real
    };

    let (ta, tb) = LoopbackTransport::pair(4, 9);
    let (mut a, mut b) = (session(ta, 0), session(tb, 1));
    play_out(&mut a, &mut b, &inputs, end, None);
    let sahte = verdict("sahte ağ", a.tick, b.tick, a.state_hash(), b.state_hash(), a.state, b.state);

    let udp = (|| -> std::io::Result<bool> {
        let mut ua = UdpTransport::bind(0)?;
        let mut ub = UdpTransport::bind(0)?;
        let (pa, pb) = (ua.local_addr()?.port(), ub.local_addr()?.port());
        ua.set_remote(std::net::SocketAddr::from(([127, 0, 0, 1], pb)));
        ub.set_remote(std::net::SocketAddr::from(([127, 0, 0, 1], pa)));
        let (mut a, mut b) = (session(ua, 0), session(ub, 1));
        play_out(&mut a, &mut b, &inputs, end, Some(std::time::Duration::from_micros(200)));
        Ok(verdict("UDP", a.tick, b.tick, a.state_hash(), b.state_hash(), a.state, b.state))
    })();
    let udp = udp.unwrap_or_else(|e| {
        println!("itis --test (UDP): soket açılamadı: {e}");
        false
    });
    sahte && udp
}

enum Net {
    Local { a: Box<Session<LoopbackTransport>>, b: Box<Session<LoopbackTransport>> },
    Udp { s: Box<Session<UdpTransport>>, player: u32 },
}

struct Game {
    net: Net,
    acc: f32,
    pucks: [u32; 2],
    hud: u32,
    banner: u32,
}

fn parse_net() -> Result<Net, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let Some(port) = get("--port") else {
        let (ta, tb) = LoopbackTransport::pair(4, 0);
        return Ok(Net::Local { a: Box::new(session(ta, 0)), b: Box::new(session(tb, 1)) });
    };
    let port: u16 = port.parse().map_err(|_| format!("--port bir sayı olmalı: {port}"))?;
    let peer = get("--peer").ok_or("--port ile --peer <ip:port> de gerekli")?;
    let peer = peer.parse().map_err(|_| format!("--peer bir adres olmalı: {peer}"))?;
    let player: u32 = get("--player").unwrap_or_else(|| "0".into()).parse().map_err(|_| "--player 0 ya da 1")?;
    let mut t = UdpTransport::bind(port).map_err(|e| format!("{port} bağlanamadı: {e}"))?;
    t.set_remote(peer);
    Ok(Net::Udp { s: Box::new(session(t, player & 1)), player: player & 1 })
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    let net = match parse_net() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("itis: {e}");
            std::process::exit(2);
        }
    };
    let mut net = Some(net);
    App::<SimpleSceneState>::new("Gizmo Engine - İtiş", 1280, 720)
        .with_simple_scene(move |scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.9)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 1.7, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 9.0, 9.5),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -0.75,
                primary: true,
                ..Default::default()
            });
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.5, 0.0)).with_scale(Vec3::new(ARENA, 0.5, ARENA)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#D9CBA3").to_vec4(), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            let colours = ["#2E86C1", "#C0392B"];
            let pucks = [0, 1].map(|p| {
                scene
                    .world
                    .spawn_bundle((
                        Transform::new(START[p]).with_scale(Vec3::splat(PUCK_R)),
                        GlobalTransform::default(),
                        MeshSource("sphere".into()),
                        MaterialDesc::new().with_pbr(Color::srgb_hex(colours[p]).to_vec4(), 0.35, 0.1),
                        MeshRenderer::new(),
                    ))
                    .id()
            });
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),)).id();
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 44.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();
            let net = net.take().expect("kurulum bir kez çalışır");
            scene.world.insert_resource(Game { net, acc: 0.0, pucks, hud, banner });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn player_input(input: &Input, keys: MoveKeys) -> PlayerInput {
    let (x, y) = input.move_axis_with(keys);
    let restart = input.is_key_pressed(KeyCode::Space as u32);
    PlayerInput { tick: 0, buttons: if restart { BUTTON_RESTART } else { 0 }, joystick_x: quantise(x), joystick_y: quantise(y) }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.acc = (game.acc + dt.min(0.1)).min(DT * 8.0);
    while game.acc >= DT {
        game.acc -= DT;
        let advanced = match &mut game.net {
            Net::Local { a, b } => {
                // Yerelde iki taraf da aynı karede ilerler; ikisi de beklerse bu kare biter.
                let ia = player_input(input, MoveKeys::WASD);
                let ib = player_input(input, MoveKeys::ARROWS);
                let x = a.try_advance_with(ia, &apply, &rules, MAX_LEAD);
                let y = b.try_advance_with(ib, &apply, &rules, MAX_LEAD);
                x || y
            }
            Net::Udp { s, .. } => {
                let mut i = player_input(input, MoveKeys::WASD);
                if i.joystick_x == 0 && i.joystick_y == 0 {
                    i = player_input(input, MoveKeys::ARROWS);
                }
                s.try_advance_with(i, &apply, &rules, MAX_LEAD)
            }
        };
        if !advanced {
            game.acc = 0.0;
            break;
        }
    }
    // Görüntü: oturumun (bu tarafın) fizik dünyasından.
    let (w, m, ahead, mode) = match &game.net {
        Net::Local { a, .. } => (&a.world, a.state, a.frames_ahead(), "yerel (4 tick gecikmeli sahte ağ)".to_string()),
        Net::Udp { s, player } => (&s.world, s.state, s.frames_ahead(), format!("ağ, {}. oyuncu", player + 1)),
    };
    let poses: Vec<(Vec3, Quat)> = PUCK.iter().map(|&i| (w.transforms[i].position, w.transforms[i].rotation)).collect();
    for (p, &id) in game.pucks.iter().enumerate() {
        if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(id) {
            t.position = poses[p].0;
            t.rotation = poses[p].1;
        }
    }
    set_text(
        world,
        game.hud,
        &format!("mavi {}  —  kırmızı {}\n{mode}, {ahead} tick önde\nmavi: WASD   kırmızı: oklar", m.score[0], m.score[1]),
    );
    let banner = match m.winner {
        Some(0) => "Mavi kazandı!\nBoşluk: yeni maç",
        Some(_) => "Kırmızı kazandı!\nBoşluk: yeni maç",
        None => "",
    };
    set_text(world, game.banner, banner);
    world.insert_resource(game);
}

fn set_text(world: &mut World, id: u32, s: &str) {
    if let Some(mut t) = world.borrow_mut::<Text>().get_mut(id) {
        if t.content != s {
            t.content = s.to_string();
        }
    }
}

fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
