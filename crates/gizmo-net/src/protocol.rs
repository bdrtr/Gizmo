use renet::{ChannelConfig, ConnectionConfig, SendType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

pub const PROTOCOL_ID: u64 = 7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformData {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    PlayerInput {
        move_x: f32,
        move_z: f32,
        jump: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    PlayerConnected {
        client_id: u64,
    },
    PlayerDisconnected {
        client_id: u64,
    },
    WorldStateUpdate {
        players: HashMap<u64, TransformData>,
    },
}

pub enum ServerChannel {
    Reliable,
    Unreliable,
}

impl Into<u8> for ServerChannel {
    fn into(self) -> u8 {
        match self {
            ServerChannel::Reliable => 0,
            ServerChannel::Unreliable => 1,
        }
    }
}

pub enum ClientChannel {
    Command,
}

impl Into<u8> for ClientChannel {
    fn into(self) -> u8 {
        match self {
            ClientChannel::Command => 0,
        }
    }
}

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
