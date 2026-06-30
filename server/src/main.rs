use gizmo_core::system::Schedule;
use gizmo_core::time::Time;
use gizmo_core::world::World;
use gizmo_net::client_server::protocol::{
    ClientChannel, ClientMessage, ServerChannel, ServerMessage, TransformData,
};
use gizmo_net::client_server::server::NetworkServer;
use gizmo_physics_core::Transform;
use std::collections::HashMap;

/// Sunucunun ağ durumu: otoriter tick sayacı ve her istemci için işlenen son
/// girdinin tick'i (per-client reconciliation ACK'i).
#[derive(Default)]
struct ServerNetState {
    tick: u32,
    last_processed_input: HashMap<u64, u32>,
}

pub fn server_network_system(world: &World, dt: f32) {
    let dt_f64 = dt as f64;

    let mut messages_to_broadcast = Vec::new();

    if let Some(mut server_res) = world.get_resource_mut::<NetworkServer>() {
        server_res.update(dt_f64);

        while let Some(event) = server_res.server.get_event() {
            match event {
                renet::ServerEvent::ClientConnected { client_id } => {
                    println!("Client {} bağlandı!", client_id);
                    // Spawn logic
                    messages_to_broadcast.push(ServerMessage::PlayerConnected { client_id });
                }
                renet::ServerEvent::ClientDisconnected { client_id, .. } => {
                    println!("Client {} düştü!", client_id);
                    messages_to_broadcast.push(ServerMessage::PlayerDisconnected { client_id });
                }
            }
        }

        for client_id in server_res.server.clients_id() {
            while let Some(message) = server_res
                .server
                .receive_message(client_id, ClientChannel::Command)
            {
                if let Ok(ClientMessage::Input(input)) =
                    bincode::deserialize::<ClientMessage>(&message)
                {
                    // Otoriter fizik burada uygulanır (demo: no-op).
                    // Bu istemciden işlenen son girdi tick'ini ACK olarak kaydet.
                    if let Some(mut st) = world.get_resource_mut::<ServerNetState>() {
                        let entry = st.last_processed_input.entry(client_id).or_insert(0);
                        if input.tick > *entry {
                            *entry = input.tick;
                        }
                    }
                }
            }
        }

        for msg in messages_to_broadcast {
            if let Ok(serialized) = bincode::serialize(&msg) {
                server_res
                    .server
                    .broadcast_message(ServerChannel::Reliable, serialized);
            }
        }

        let mut players_map = HashMap::new();
        if let Some(query) = world.query::<&Transform>() {
            for (id, t) in query.iter() {
                let t_ref = *t;
                players_map.insert(
                    id as u64,
                    TransformData {
                        position: [t_ref.position.x, t_ref.position.y, t_ref.position.z],
                        rotation: [
                            t_ref.rotation.x,
                            t_ref.rotation.y,
                            t_ref.rotation.z,
                            t_ref.rotation.w,
                        ],
                    },
                );
            }
        }

        // Her istemciye yalnızca kendi reconciliation ACK'ini gönder (per-client, reliable).
        let acks: Vec<(u64, u32)> = {
            let st = world.get_resource::<ServerNetState>().unwrap();
            server_res
                .server
                .clients_id()
                .into_iter()
                .map(|cid| (cid, st.last_processed_input.get(&cid).copied().unwrap_or(0)))
                .collect()
        };
        for (cid, last_processed_input) in acks {
            if let Ok(serialized) = bincode::serialize(&ServerMessage::InputAck {
                last_processed_input,
            }) {
                server_res
                    .server
                    .send_message(cid, ServerChannel::Reliable, serialized);
            }
        }

        // Otoriter tick'i ilerlet ve ortak dünya durumunu yayınla (broadcast, interpolasyon için).
        let server_tick = {
            let mut st = world.get_resource_mut::<ServerNetState>().unwrap();
            st.tick = st.tick.wrapping_add(1);
            st.tick
        };
        if let Ok(serialized) = bincode::serialize(&ServerMessage::WorldStateUpdate {
            server_tick,
            players: players_map,
        }) {
            server_res
                .server
                .broadcast_message(ServerChannel::Unreliable, serialized);
        }

        server_res.send_packets();
    }
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

        let elapsed = start.elapsed().as_secs_f64();
        if elapsed < target_dt {
            std::thread::sleep(std::time::Duration::from_secs_f64(target_dt - elapsed));
        }
    }
}
