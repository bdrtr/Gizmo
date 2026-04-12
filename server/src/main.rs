use gizmo_core::system::Schedule;
use gizmo_core::time::Time;
use gizmo_core::world::World;
use gizmo_net::protocol::{
    ClientChannel, ClientMessage, ServerChannel, ServerMessage, TransformData,
};
use gizmo_net::server::NetworkServer;
use gizmo_physics::components::Transform;
use std::collections::HashMap;

pub fn server_network_system(world: &mut World, dt: f32) {
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
                if let Ok(ClientMessage::PlayerInput {
                    move_x: _,
                    move_z: _,
                    jump: _,
                }) = bincode::deserialize(&message)
                {
                    // Update physics...
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
        if let Some(mut query) = world.query::<&mut Transform>() {
            for (id, t) in query.iter_mut() {
                let t_ref = t.clone();
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

        if let Ok(serialized) = bincode::serialize(&ServerMessage::WorldStateUpdate {
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
    world.insert_resource(NetworkServer::new("0.0.0.0:4000"));

    // 2. Zamanlayıcıyı Başlat
    world.insert_resource(Time::default());

    // 3. Sistemleri Kaydet
    schedule.add_system(server_network_system);

    println!("Gizmo Server 4000 portunda dinliyor ve 60 Tick/saniye hızında çalışıyor.");

    let target_dt = 1.0 / 60.0;
    loop {
        let start = std::time::Instant::now();

        let mut time = world.get_resource_mut::<Time>().unwrap();
        time.dt = target_dt as f32;
        time.elapsed_seconds += target_dt;
        drop(time);

        schedule.run(&mut world, target_dt as f32);

        let elapsed = start.elapsed().as_secs_f64();
        if elapsed < target_dt {
            std::thread::sleep(std::time::Duration::from_secs_f64(target_dt - elapsed));
        }
    }
}
