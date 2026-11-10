//! # Kovala — sunucu yönetiminde ebelemece
//!
//! Kare bir meydan; biri ebe. Ebe değdiği oyuncuyu ebe yapar. Ebe olmadan geçen her saniye bir
//! puan. Motorun yirmi dokuzuncu oyunu ve **sunucu-istemci** ağını kullanan ilki: sunucu otoriter
//! (hareketi o hesaplar, ebeyi o seçer), istemci kendi oyuncusunu tahmin eder ve sunucunun
//! dediğiyle uzlaştırır, öbür oyuncuları sunucunun anlık görüntüleri arasında ara değerler.
//!
//! | kip | nasıl |
//! |---|---|
//! | ev sahibi | `kovala` — sunucu bu süreçte, üç bilgisayar oyuncusu onda; sen de bağlanırsın |
//! | katıl | `kovala --join 192.168.1.5:5005` |
//! | sınama | `kovala --test` — pencere açmaz: sunucu ve iki istemci 127.0.0.1 üstünde beş saniye oynar; tahmin düzeltmeleri ve öbür oyuncunun görüntüsü ölçülür |
//!
//! **WASD · sol çubuk**: koş.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Mesajların bir tel biçimi yoktu, uçların gönder-al çağrıları yoktu.** Oyun renet'e ve bir
//!   serileştiriciye kendisi bağlanmak zorundaydı. **Düzeltildi:** `encode`/`decode`,
//!   `NetworkServer::{send, broadcast, receive}`, `NetworkClient::{send, receive}`.
//! - **Uzlaştırmanın istediği ikili hiçbir mesajda yoktu.** Onay (`InputAck`) ve durum
//!   (`WorldStateUpdate`) ayrı geliyordu, durum da varlık indeksine göre anahtarlıydı: istemci
//!   kendini bulamazdı; farklı tick'lerden eşlenen ikili kusursuz bir tahmini iki adım geri
//!   çekiyordu. **Düzeltildi:** `ServerMessage::PlayerState`.
//! - **Oyun kendi verisini gönderemiyordu** (ebe kim, puanlar) — mesajlar sabit enum'lardı.
//!   **Düzeltildi:** `ClientMessage::Custom` / `ServerMessage::Custom`.

use gizmo::core::input::Input;
use gizmo::net::client_server::client::NetworkClient;
use gizmo::net::client_server::interpolation::SnapshotInterpolator;
use gizmo::net::client_server::prediction::{ClientPredictor, PredictedState};
use gizmo::net::client_server::protocol::{ClientMessage, PlayerInput, ServerChannel, ServerMessage, TransformData};
use gizmo::net::client_server::server::NetworkServer;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::collections::HashMap;

const DT: f32 = 1.0 / 60.0;
const SPEED: f32 = 5.0;
const BOT_SPEED: f32 = 4.2;
const ARENA: f32 = 9.0;
const TAG: f32 = 0.9;
const PORT: u16 = 5005;
const BOT_IDS: [u64; 3] = [1001, 1002, 1003];

/// Bir girdinin bir oyuncuyu bir adım ilerletmesi — sunucu da istemci de **bunu** çalıştırır,
/// uzlaştırma da tam bunun aynı olmasına dayanır.
fn step(s: &PredictedState, input: &PlayerInput) -> PredictedState {
    let dir = Vec2::new(input.move_x, input.move_z);
    let dir = if dir.length() > 1.0 { dir.normalize() } else { dir };
    let v = [dir.x * SPEED, 0.0, dir.y * SPEED];
    let p = [
        (s.position[0] + v[0] * input.dt).clamp(-ARENA, ARENA),
        0.5,
        (s.position[2] + v[2] * input.dt).clamp(-ARENA, ARENA),
    ];
    PredictedState { position: p, velocity: v }
}

fn flat(a: [f32; 3], b: [f32; 3]) -> f32 {
    Vec2::new(a[0] - b[0], a[2] - b[2]).length()
}

// ── Sunucu ──────────────────────────────────────────────────────────────────────────────────

struct Player {
    state: PredictedState,
    last_input: u32,
}

struct Server {
    net: NetworkServer,
    players: HashMap<u64, Player>,
    bots: HashMap<u64, [f32; 3]>,
    it: u64,
    tag_cooldown: f32,
    scores: HashMap<u64, f32>,
    tick: u32,
}

impl Server {
    fn new(addr: &str, with_bots: bool) -> Result<Self, String> {
        let net = NetworkServer::new(addr).map_err(|e| format!("{addr}: {e}"))?;
        let bots = if with_bots {
            BOT_IDS.iter().enumerate().map(|(i, &id)| (id, [-6.0 + i as f32 * 6.0, 0.5, -6.0])).collect()
        } else {
            HashMap::new()
        };
        let it = if with_bots { BOT_IDS[0] } else { 0 };
        Ok(Self { net, players: HashMap::new(), bots, it, tag_cooldown: 0.0, scores: HashMap::new(), tick: 0 })
    }

    fn positions(&self) -> Vec<(u64, [f32; 3])> {
        let mut all: Vec<(u64, [f32; 3])> = self.players.iter().map(|(&id, p)| (id, p.state.position)).collect();
        all.extend(self.bots.iter().map(|(&id, &p)| (id, p)));
        all.sort_by_key(|(id, _)| *id);
        all
    }

    fn update(&mut self, dt: f32) {
        self.net.update(dt as f64);
        for id in self.net.clients() {
            let spawn = [3.0 - (id % 4) as f32 * 2.0, 0.5, 6.0];
            let player = self.players.entry(id).or_insert(Player { state: PredictedState { position: spawn, velocity: [0.0; 3] }, last_input: 0 });
            for m in self.net.receive(id) {
                if let ClientMessage::Input(i) = m {
                    player.state = step(&player.state, &i);
                    player.last_input = i.tick;
                }
            }
        }
        let gone: Vec<u64> = self.players.keys().copied().filter(|id| !self.net.clients().contains(id)).collect();
        for id in gone {
            self.players.remove(&id);
        }
        if self.it == 0 {
            self.it = self.players.keys().copied().min().unwrap_or(0);
        }
        // Bilgisayar oyuncuları: ebe en yakına koşar, öbürleri ebeden kaçar.
        let everyone = self.positions();
        let it_pos = everyone.iter().find(|(id, _)| *id == self.it).map(|(_, p)| *p);
        for (&id, pos) in self.bots.iter_mut() {
            let target = if id == self.it {
                everyone.iter().filter(|(o, _)| *o != id).min_by(|a, b| flat(a.1, *pos).total_cmp(&flat(b.1, *pos))).map(|(_, p)| Vec2::new(p[0] - pos[0], p[2] - pos[2]))
            } else {
                it_pos.map(|p| Vec2::new(pos[0] - p[0], pos[2] - p[2]))
            };
            let d = target.unwrap_or(Vec2::ZERO).normalize_or_zero() * BOT_SPEED * dt;
            pos[0] = (pos[0] + d.x).clamp(-ARENA, ARENA);
            pos[2] = (pos[2] + d.y).clamp(-ARENA, ARENA);
        }
        // Ebe değdiği ilk kişiyi ebe yapar; yeni ebe bir saniye kimseyi ebeleyemez.
        self.tag_cooldown = (self.tag_cooldown - dt).max(0.0);
        let everyone = self.positions();
        if let (Some(it_pos), true) = (everyone.iter().find(|(id, _)| *id == self.it).map(|(_, p)| *p), self.tag_cooldown == 0.0) {
            if let Some((other, _)) = everyone.iter().find(|(id, p)| *id != self.it && flat(*p, it_pos) < TAG) {
                self.it = *other;
                self.tag_cooldown = 1.0;
            }
        }
        for (id, _) in &everyone {
            if *id != self.it {
                *self.scores.entry(*id).or_insert(0.0) += dt;
            }
        }
        self.tick = self.tick.wrapping_add(1);
        // Her istemciye kendi durumu ve onun yansıttığı girdi, tek mesajda.
        for (&id, p) in &self.players {
            let _ = self.net.send(id, ServerChannel::Unreliable, &ServerMessage::PlayerState {
                last_processed_input: p.last_input,
                position: p.state.position,
                velocity: p.state.velocity,
            });
        }
        let players = everyone.iter().map(|&(id, p)| (id, TransformData { position: p, rotation: [0.0, 0.0, 0.0, 1.0] })).collect();
        let _ = self.net.broadcast(ServerChannel::Unreliable, &ServerMessage::WorldStateUpdate { server_tick: self.tick, players });
        if self.tick.is_multiple_of(6) {
            let _ = self.net.broadcast(ServerChannel::Unreliable, &ServerMessage::Custom(encode_info(self.it, &self.scores)));
        }
        self.net.send_packets();
    }
}

/// Oyunun kendi mesajı: ebe ve puanlar, küçük-sonlu baytlar.
fn encode_info(it: u64, scores: &HashMap<u64, f32>) -> Vec<u8> {
    let mut b = it.to_le_bytes().to_vec();
    let mut s: Vec<(&u64, &f32)> = scores.iter().collect();
    s.sort_by_key(|(id, _)| **id);
    for (id, v) in s {
        b.extend(id.to_le_bytes());
        b.extend(v.to_le_bytes());
    }
    b
}

fn decode_info(b: &[u8]) -> Option<(u64, Vec<(u64, f32)>)> {
    let it = u64::from_le_bytes(b.get(..8)?.try_into().ok()?);
    let scores = b[8..].as_chunks::<12>().0.iter().filter_map(|c| Some((u64::from_le_bytes(c[..8].try_into().ok()?), f32::from_le_bytes(c[8..].try_into().ok()?)))).collect();
    Some((it, scores))
}

// ── İstemci ─────────────────────────────────────────────────────────────────────────────────

struct Client {
    net: NetworkClient,
    id: u64,
    predictor: ClientPredictor,
    me: PredictedState,
    others: HashMap<u64, SnapshotInterpolator>,
    it: u64,
    scores: Vec<(u64, f32)>,
    clock: f64,
    worst_correction: f32,
    reconciled: u32,
}

impl Client {
    fn new(addr: &str, id: u64) -> Result<Self, String> {
        let net = NetworkClient::new(addr, id).map_err(|e| format!("{addr}: {e}"))?;
        Ok(Self {
            net,
            id,
            predictor: ClientPredictor::new(),
            me: PredictedState { position: [0.0, 0.5, 0.0], velocity: [0.0; 3] },
            others: HashMap::new(),
            it: 0,
            scores: Vec::new(),
            clock: 0.0,
            worst_correction: 0.0,
            reconciled: 0,
        })
    }

    fn update(&mut self, dt: f32, move_x: f32, move_z: f32) {
        self.clock += dt as f64;
        self.net.update(dt as f64);
        for m in self.net.receive() {
            match m {
                ServerMessage::PlayerState { last_processed_input, position, velocity } => {
                    let fresh = self.reconciled == 0;
                    let fixed = self.predictor.reconcile(last_processed_input, PredictedState { position, velocity }, step);
                    // Kusursuz bir tahminin düzeltmesi sıfırdır; ilk durum tahminin başlangıcıdır.
                    if !fresh {
                        self.worst_correction = self.worst_correction.max(flat(fixed.position, self.me.position));
                    }
                    self.me = fixed;
                    self.reconciled += 1;
                }
                ServerMessage::WorldStateUpdate { players, .. } => {
                    for (id, t) in players {
                        if id != self.id {
                            self.others.entry(id).or_insert_with(|| SnapshotInterpolator::new(100.0)).add_snapshot(self.clock, t.position, t.rotation);
                        }
                    }
                }
                ServerMessage::Custom(bytes) => {
                    if let Some((it, scores)) = decode_info(&bytes) {
                        self.it = it;
                        self.scores = scores;
                    }
                }
                _ => {}
            }
        }
        if self.net.is_connected() && self.reconciled > 0 {
            let input = self.predictor.add_input(move_x, move_z, false, dt);
            let _ = self.net.send(&ClientMessage::Input(input));
            self.me = step(&self.me, &input);
        }
        self.net.send_packets();
    }

    fn seen(&self, id: u64) -> Option<[f32; 3]> {
        self.others.get(&id)?.get_interpolated_transform(self.clock).map(|(p, _)| p)
    }
}

// ── Sınama ──────────────────────────────────────────────────────────────────────────────────

fn self_test() -> bool {
    let port = std::net::UdpSocket::bind("127.0.0.1:0").and_then(|s| s.local_addr()).map(|a| a.port()).unwrap_or(PORT);
    let addr = format!("127.0.0.1:{port}");
    let (mut server, mut a, mut b) = match (Server::new(&addr, false), Client::new(&addr, 1), Client::new(&addr, 2)) {
        (Ok(s), Ok(a), Ok(b)) => (s, a, b),
        _ => {
            println!("kovala --test: soket açılamadı");
            return false;
        }
    };
    let mut worst_view = 0.0f32;
    for frame in 0..60 * 5 {
        let t = frame as f32 * DT;
        a.update(DT, t.cos(), t.sin());
        b.update(DT, -(t * 0.7).sin(), (t * 0.7).cos());
        server.update(DT);
        // İkinci saniyeden sonra: B'nin gördüğü A, sunucunun A'sından ne kadar uzak?
        if frame > 120 {
            if let (Some(seen), Some(real)) = (b.seen(1), server.players.get(&1).map(|p| p.state.position)) {
                worst_view = worst_view.max(flat(seen, real));
            }
        }
        std::thread::sleep(std::time::Duration::from_micros(500));
    }
    let processed = server.players.get(&1).map(|p| p.last_input).unwrap_or(0);
    // 100 ms gecikmeyle bakan B, 5 m/s koşan A'yı en fazla ~0,5 m (+ bir paket arası) geriden görür.
    let ok = a.reconciled > 100 && b.reconciled > 100 && a.worst_correction < 1e-4 && b.worst_correction < 1e-4 && worst_view < 1.0 && processed > 200;
    println!(
        "kovala --test: A {} B {} uzlaştırma, en büyük düzeltme {:.6} / {:.6} m; sunucu A'nın {} girdisini işledi; B, A'yı en çok {:.2} m geriden gördü → {}",
        a.reconciled,
        b.reconciled,
        a.worst_correction,
        b.worst_correction,
        processed,
        worst_view,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

// ── Oyun ────────────────────────────────────────────────────────────────────────────────────

struct Game {
    server: Option<Server>,
    client: Client,
    me: Entity,
    others: HashMap<u64, Entity>,
    hud: Entity,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--test") {
        gizmo::simple::run_self_test(self_test);
    }
    let join = args.iter().position(|a| a == "--join").and_then(|i| args.get(i + 1)).cloned();
    let id = std::process::id() as u64;
    let (server, client) = match &join {
        Some(addr) => (None, Client::new(addr, id)),
        None => {
            let addr = format!("0.0.0.0:{PORT}");
            match Server::new(&addr, true) {
                Ok(s) => (Some(s), Client::new(&format!("127.0.0.1:{PORT}"), id)),
                Err(e) => {
                    eprintln!("kovala: sunucu açılamadı: {e}");
                    std::process::exit(2);
                }
            }
        }
    };
    let client = match client {
        Ok(c) => c,
        Err(e) => {
            eprintln!("kovala: bağlanılamadı: {e}");
            std::process::exit(2);
        }
    };
    let mut parts = Some((server, client));
    App::<SimpleSceneState>::new("Gizmo Engine - Kovala", 1280, 720)
        .with_simple_scene(move |scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-1.0)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 1.8, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 19.0, 15.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -0.9,
                primary: true,
                ..Default::default()
            });
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.05, 0.0)).with_scale(Vec3::new(ARENA + 0.5, 0.05, ARENA + 0.5)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#C9B79C").to_vec4(), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            let me = spawn_runner(scene.world, "#2E86C1");
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
            let (server, client) = parts.take().expect("kurulum bir kez çalışır");
            scene.world.insert_resource(Game { server, client, me, others: HashMap::new(), hud });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn spawn_runner(world: &mut World, colour: &str) -> Entity {
    world
        .spawn_bundle((
            Transform::new(Vec3::new(0.0, 0.5, 0.0)).with_scale(Vec3::new(0.4, 0.5, 0.4)),
            GlobalTransform::default(),
            MeshSource("cylinder".into()),
            MaterialDesc::new().with_pbr(Color::srgb_hex(colour).to_vec4(), 0.5, 0.0),
            MeshRenderer::new(),
        ))
        
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.clamp(1.0 / 240.0, 0.05);
    let (mx, my) = input.move_axis();
    game.client.update(dt, mx, -my);
    if let Some(server) = &mut game.server {
        server.update(dt);
    }
    let it = game.client.it;
    let place = |world: &mut World, id: Entity, p: [f32; 3], is_it: bool| {
        if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(id) {
            t.position = Vec3::from_array(p);
        }
        if let Some(mut m) = world.query_mut::<&mut MaterialDesc>().get_mut(id) {
            let want = if is_it { Color::srgb_hex("#E74C3C").to_vec4() } else { m.albedo };
            m.emissive = if is_it { Vec3::new(0.6, 0.05, 0.05) } else { Vec3::ZERO };
            m.albedo = want;
        }
    };
    place(world, game.me, game.client.me.position, it == game.client.id);
    let ids: Vec<u64> = game.client.others.keys().copied().collect();
    for id in ids {
        let entity = match game.others.get(&id) {
            Some(&e) => e,
            None => {
                let e = spawn_runner(world, if id >= 1000 { "#7F8C8D" } else { "#27AE60" });
                game.others.insert(id, e);
                e
            }
        };
        if let Some(p) = game.client.seen(id) {
            place(world, entity, p, it == id);
        }
    }
    let mut lines: Vec<String> = game
        .client
        .scores
        .iter()
        .map(|(id, s)| {
            let who = if *id == game.client.id { "sen".to_string() } else if *id >= 1000 { format!("bot {}", id - 1000) } else { format!("oyuncu {id}") };
            format!("{who}: {s:.0}{}", if *id == it { "  (ebe)" } else { "" })
        })
        .collect();
    lines.sort();
    let status = if game.client.net.is_connected() { "bağlı" } else { "bağlanıyor…" };
    let text = format!("{status}{}\n{}\nWASD: koş — ebe değdiğini ebeler", if game.server.is_some() { ", ev sahibi" } else { "" }, lines.join("\n"));
    if let Some(mut t) = world.query_mut::<&mut Text>().get_mut(game.hud) {
        if t.content != text {
            t.content = text;
        }
    }
    world.insert_resource(game);
}

fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
