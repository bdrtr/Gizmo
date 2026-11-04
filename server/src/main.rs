use gizmo_core::system::Schedule;
use gizmo_core::time::Time;
use gizmo_core::world::World;
use gizmo_net::client_server::protocol::{tick_is_newer, ClientMessage, PlayerInput, ServerChannel, ServerMessage, TransformData};
use gizmo_net::client_server::server::NetworkServer;
use std::collections::HashMap;

/// How fast a player moves at full stick, m/s — the reference movement below.
const SPEED: f32 = 5.0;

/// One connected player as the server has it: where it is and the newest input it consumed.
#[derive(Default)]
struct Player {
    position: [f32; 3],
    velocity: [f32; 3],
    last_processed_input: u32,
}

/// Sunucunun ağ durumu: otoriter tick sayacı ve her istemcinin oyuncusu.
#[derive(Default)]
struct ServerNetState {
    tick: u32,
    players: HashMap<u64, Player>,
}

/// The reference movement: the stick, capped at length one, times [`SPEED`], on the ground plane.
/// A client predicting this server must run exactly this — reconciliation replays its pending
/// inputs through its own copy, and any difference shows as a correction every time.
fn step(p: &mut Player, input: &PlayerInput) {
    let (x, z) = (input.move_x, input.move_z);
    let len = (x * x + z * z).sqrt().max(1.0);
    p.velocity = [x / len * SPEED, 0.0, z / len * SPEED];
    p.position[0] += p.velocity[0] * input.dt;
    p.position[2] += p.velocity[2] * input.dt;
}

/// One server tick: connections, inputs applied, then each client's own state and everyone's
/// positions sent out.
///
/// **What this reference used to show, and why it changed (2026-11-04).** It applied no input
/// (the physics was a no-op), sent each client an `InputAck` on the reliable channel and a
/// `WorldStateUpdate` keyed by *entity index* on the unreliable one. A client cannot reconcile
/// against that: the ack and the state arrive apart (an ack paired with another tick's state
/// pulls a perfect prediction back — `prediction`'s tests measure two whole steps), and it cannot
/// find itself in a map keyed by something it never sees. It now sends `PlayerState` — ack,
/// position and velocity in one message — and keys `WorldStateUpdate` by client id, which is the
/// shape `demo/src/bin/kovala.rs` plays on.
pub fn server_network_system(world: &World, dt: f32) {
    let Some(mut server) = world.get_resource_mut::<NetworkServer>() else { return };
    let Some(mut st) = world.get_resource_mut::<ServerNetState>() else { return };
    server.update(dt as f64);

    let mut announcements = Vec::new();
    while let Some(event) = server.server.get_event() {
        match event {
            renet::ServerEvent::ClientConnected { client_id } => {
                println!("Client {} bağlandı!", client_id);
                st.players.insert(client_id, Player::default());
                announcements.push(ServerMessage::PlayerConnected { client_id });
            }
            renet::ServerEvent::ClientDisconnected { client_id, .. } => {
                println!("Client {} düştü!", client_id);
                // renet reuses client_ids across sessions; a leftover high ACK would make a
                // reconnecting client evict all of its own predicted inputs.
                st.players.remove(&client_id);
                announcements.push(ServerMessage::PlayerDisconnected { client_id });
            }
        }
    }

    for client_id in server.clients() {
        for message in server.receive(client_id) {
            if let (ClientMessage::Input(input), Some(player)) = (message, st.players.get_mut(&client_id)) {
                step(player, &input);
                // Wraparound-safe (mirrors ClientPredictor::reconcile): a plain `>` would freeze
                // the ACK for ever once the client's tick wraps u32::MAX -> 0.
                if tick_is_newer(input.tick, player.last_processed_input) {
                    player.last_processed_input = input.tick;
                }
            }
        }
    }

    for message in &announcements {
        let _ = server.broadcast(ServerChannel::Reliable, message);
    }
    for (&client_id, p) in &st.players {
        let _ = server.send(client_id, ServerChannel::Unreliable, &ServerMessage::PlayerState {
            last_processed_input: p.last_processed_input,
            position: p.position,
            velocity: p.velocity,
        });
    }
    st.tick = st.tick.wrapping_add(1);
    let players = st.players.iter().map(|(&id, p)| (id, TransformData { position: p.position, rotation: [0.0, 0.0, 0.0, 1.0] })).collect();
    let _ = server.broadcast(ServerChannel::Unreliable, &ServerMessage::WorldStateUpdate { server_tick: st.tick, players });
    server.send_packets();
}

fn main() {
    println!("--- Gizmo Server: Başlatılıyor (Veloren Headless Modeli) ---");

    let mut world = World::default();
    let mut schedule = Schedule::new();

    // 1. Ağ Sistemini Başlat
    let network_server =
        NetworkServer::new("0.0.0.0:4000").expect("Ağ sunucusu 0.0.0.0:4000 üzerinde başlatılamadı");
    world.insert_resource(network_server);
    world.insert_resource(ServerNetState::default());

    // 2. Zamanlayıcıyı Başlat
    world.insert_resource(Time::default());

    // 3. Sistemleri Kaydet
    schedule.add_system(server_network_system);

    println!("Gizmo Server 4000 portunda dinliyor ve 60 Tick/saniye hızında çalışıyor.");

    let target_dt = 1.0 / 60.0;
    loop {
        let start = std::time::Instant::now();

        let mut time = world.get_resource_mut::<Time>().unwrap();
        time.update(target_dt as f32);
        drop(time);

        schedule.run(&mut world, target_dt as f32);

        // The tick is this loop's frame, and ending it is this loop's job: `Schedule::run` no
        // longer does it, because a schedule run is not a frame (a windowed frame runs two
        // schedules, one of them several times).
        if let Some(mut profiler) =
            world.get_resource_mut::<gizmo_core::profiler::FrameProfiler>()
        {
            profiler.end_frame();
        }

        let elapsed = start.elapsed().as_secs_f64();
        if elapsed < target_dt {
            std::thread::sleep(std::time::Duration::from_secs_f64(target_dt - elapsed));
        }
    }
}
