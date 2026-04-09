use gizmo::core::world::World;
use gizmo::prelude::*;
use gizmo_net::client::NetworkClient;
use gizmo_net::protocol::{ClientMessage, ServerMessage, ClientChannel, ServerChannel};

// Demo is purely a client now.
pub fn client_network_system(world: &mut World, dt: f32) {
    let dt_f64 = dt as f64;
    
    // Process input
    let mut commands = Vec::new();
    if let Some(input) = world.get_resource::<gizmo::core::input::Input>() {
        let x = if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyD as u32) { 1.0 } else { 0.0 }
              - if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyA as u32) { 1.0 } else { 0.0 };
        let z = if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyW as u32) { -1.0 } else { 0.0 }
              - if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyS as u32) { -1.0 } else { 0.0 };
        
        let jump = input.is_key_pressed(gizmo::winit::keyboard::KeyCode::Space as u32);
        if x != 0.0 || z != 0.0 || jump {
            commands.push(ClientMessage::PlayerInput { move_x: x, move_z: z, jump });
        }
    }

    if let Some(mut client_res) = world.get_resource_mut::<NetworkClient>() {
        client_res.update(dt_f64);

        if client_res.client.is_connected() {
            // Send our inputs
            for cmd in commands {
                if let Ok(encoded) = bincode::serialize(&cmd) {
                    client_res.client.send_message(ClientChannel::Command, encoded);
                }
            }

            // Unreliable Updates
            while let Some(msg) = client_res.client.receive_message(ServerChannel::Unreliable) {
                if let Ok(ServerMessage::WorldStateUpdate { players }) = bincode::deserialize(&msg) {
                    // Update local transforms for these IDs
                    if let Some(mut query) = world.query_mut::<&mut gizmo::prelude::Transform>() {
                        for (raw_id, t) in query.iter_mut() {
                            if let Some(data) = players.get(&(raw_id as u64)) {
                                t.position = Vec3::new(data.position[0], data.position[1], data.position[2]);
                                t.rotation = Quat::from_xyzw(data.rotation[0], data.rotation[1], data.rotation[2], data.rotation[3]);
                            }
                        }
                    }
                }
            }

            // Reliable Updates
            while let Some(_msg) = client_res.client.receive_message(ServerChannel::Reliable) {
                // handle disconnect/connect if we wanted to spawn proxies
            }
        }

        client_res.send_packets();
    }
}
