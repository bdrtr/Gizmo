use renet::{ChannelConfig, ConnectionConfig, SendType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Wire-level protocol version; client and server must agree on this to connect.
pub const PROTOCOL_ID: u64 = 7;

/// Position + rotation of a single networked entity, sent for interpolation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformData {
    /// World-space position `[x, y, z]`.
    pub position: [f32; 3],
    /// Orientation quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
}

/// İstemcinin tek bir tick için ürettiği girdi — hem ağ üzerinden gönderilen
/// (wire) format hem de client-side prediction/reconciliation'ın işlediği birim.
///
/// `tick` alanı, sunucunun bu girdiyi işledikten sonra istemciye hangi tick'e
/// kadar ilerlediğini (ACK) bildirebilmesi ve istemcinin onaylanmamış girdileri
/// yeniden simüle edebilmesi (reconciliation) için zorunludur.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlayerInput {
    pub tick: u32,
    pub move_x: f32,
    pub move_z: f32,
    pub jump: bool,
    pub dt: f32,
}

/// Messages sent from a client to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ClientMessage {
    /// A single tick's player input.
    Input(PlayerInput),
}

/// Messages sent from the server to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ServerMessage {
    /// A player joined the session.
    PlayerConnected {
        /// Renet client id of the player that connected.
        client_id: u64,
    },
    /// A player left the session.
    PlayerDisconnected {
        /// Renet client id of the player that disconnected.
        client_id: u64,
    },
    /// Tüm istemcilere yayınlanan (broadcast) ortak dünya durumu — interpolasyon için.
    WorldStateUpdate {
        /// Sunucunun bu state'i ürettiği otoriter tick — interpolasyon zaman çizelgesi için.
        server_tick: u32,
        players: HashMap<u64, TransformData>,
    },
    /// Yalnızca ilgili istemciye gönderilen (per-client) reconciliation ACK'i:
    /// sunucunun o istemciden işlediği son girdinin tick'i. İstemci bu tick'e
    /// kadar olan girdileri kuyruğundan siler, kalanları yeniden simüle eder.
    InputAck {
        last_processed_input: u32,
    },
}

/// Network channels the server sends on.
#[non_exhaustive]
pub enum ServerChannel {
    /// Reliable, ordered delivery (e.g. connect/disconnect events).
    Reliable,
    /// Unreliable delivery (e.g. frequent world-state updates).
    Unreliable,
}

impl From<ServerChannel> for u8 {
    fn from(val: ServerChannel) -> Self {
        match val {
            ServerChannel::Reliable => 0,
            ServerChannel::Unreliable => 1,
        }
    }
}

/// Network channels the client sends on.
#[non_exhaustive]
pub enum ClientChannel {
    /// Player commands / inputs.
    Command,
}

impl From<ClientChannel> for u8 {
    fn from(val: ClientChannel) -> Self {
        match val {
            ClientChannel::Command => 0,
        }
    }
}

/// Builds the renet [`ConnectionConfig`] shared by client and server (channels + bandwidth).
pub fn connection_config() -> ConnectionConfig {
    ConnectionConfig {
        available_bytes_per_tick: 1024 * 1024,
        client_channels_config: vec![ChannelConfig {
            channel_id: ClientChannel::Command.into(),
            max_memory_usage_bytes: 5 * 1024 * 1024,
            send_type: SendType::Unreliable,
        }],
        server_channels_config: vec![
            ChannelConfig {
                channel_id: ServerChannel::Reliable.into(),
                max_memory_usage_bytes: 5 * 1024 * 1024,
                send_type: SendType::ReliableOrdered {
                    resend_time: Duration::from_millis(200),
                },
            },
            ChannelConfig {
                channel_id: ServerChannel::Unreliable.into(),
                max_memory_usage_bytes: 5 * 1024 * 1024,
                send_type: SendType::Unreliable,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_input_roundtrip() {
        let input = PlayerInput { tick: 42, move_x: 1.0, move_z: -0.5, jump: true, dt: 0.016 };
        let bytes = bincode::serialize(&ClientMessage::Input(input)).unwrap();
        let ClientMessage::Input(back) = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back, input);
    }

    #[test]
    fn input_ack_roundtrip() {
        let bytes = bincode::serialize(&ServerMessage::InputAck { last_processed_input: 7 }).unwrap();
        match bincode::deserialize::<ServerMessage>(&bytes).unwrap() {
            ServerMessage::InputAck { last_processed_input } => assert_eq!(last_processed_input, 7),
            other => panic!("beklenmeyen varyant: {other:?}"),
        }
    }

    #[test]
    fn world_state_roundtrip() {
        let mut players = HashMap::new();
        players.insert(1u64, TransformData { position: [1.0, 2.0, 3.0], rotation: [0.0, 0.0, 0.0, 1.0] });
        let bytes = bincode::serialize(&ServerMessage::WorldStateUpdate { server_tick: 100, players }).unwrap();
        match bincode::deserialize::<ServerMessage>(&bytes).unwrap() {
            ServerMessage::WorldStateUpdate { server_tick, players } => {
                assert_eq!(server_tick, 100);
                assert_eq!(players[&1].position, [1.0, 2.0, 3.0]);
            }
            other => panic!("beklenmeyen varyant: {other:?}"),
        }
    }
}
