//! Wire format for the authoritative client-server architecture: the two message enums each
//! side sends, the channel table those messages are routed over, and the tick-comparison rule
//! everything downstream depends on.
//!
//! The types here only derive `serde` — framing is the caller's choice. The reference server
//! (`server/src/main.rs`) and this module's tests use `bincode`, so both ends of a session must
//! agree on that too; [`PROTOCOL_ID`] gates the netcode handshake, not the payload encoding, so
//! an encoding mismatch surfaces as messages that quietly fail to decode on a connection that
//! came up fine.
//!
//! Ticks are `u32` counters, not times, and are expected to wrap. Never order them with
//! `<`/`>`; use [`tick_is_newer`].

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

/// The input a client produces for a single tick — both the wire format sent over the network
/// and the unit client-side prediction/reconciliation operates on.
///
/// The `tick` field is mandatory: it is how the server tells the client which tick it has
/// processed up to (the ACK), and how the client knows which unacknowledged inputs to
/// re-simulate (reconciliation).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlayerInput {
    /// Client simulation tick this input was sampled on, stamped by
    /// [`ClientPredictor::add_input`](crate::client_server::prediction::ClientPredictor::add_input).
    ///
    /// A free-running counter, not a time. It wraps at `u32::MAX` by design, so every
    /// comparison against it must go through [`tick_is_newer`]. The server echoes the newest
    /// one it has consumed back as [`ServerMessage::InputAck`].
    pub tick: u32,
    /// One of the two horizontal movement axes for this tick, conventionally a stick value in
    /// `-1.0..=1.0` — nothing here clamps or normalises it.
    ///
    /// The frame it is interpreted in (world- or camera-relative) and the acceleration it
    /// implies live entirely in the physics closure handed to
    /// [`ClientPredictor::reconcile`](crate::client_server::prediction::ClientPredictor::reconcile)
    /// and in the server's authoritative step. Those two must agree exactly: any difference
    /// shows up as a correction on *every* reconciliation, not as a one-off glitch.
    pub move_x: f32,
    /// The other horizontal movement axis, same convention and same caveat as `move_x`.
    ///
    /// There is deliberately no `move_y`: movement input spans the ground plane only (this
    /// engine is Y-up), and vertical motion comes from `jump` plus gravity.
    pub move_z: f32,
    /// Whether jump was held on this tick — a level, not an edge.
    ///
    /// Nothing debounces it, so a held key sends `true` on every tick of the hold; if one
    /// press should mean one impulse, the physics step has to detect that itself. Because
    /// reconciliation replays stored inputs verbatim, that detection must be a pure function
    /// of the state being replayed, or a correction will produce a second jump.
    pub jump: bool,
    /// Seconds of simulation this one input covers.
    ///
    /// It travels on the wire because reconciliation replays the *stored* step rather than
    /// the current one: re-applying a 16 ms input with whatever `dt` the correction frame
    /// happens to have would land the player somewhere the server never put them.
    ///
    /// Client-supplied and therefore untrusted — sessions here authenticate with
    /// `ServerAuthentication::Unsecure`, so clamp it to a plausible range before feeding it
    /// into an authoritative step.
    pub dt: f32,
}

/// Is the `candidate` tick STRICTLY newer than `reference` — using signed wraparound arithmetic
/// so the ordering stays correct after the `u32` tick space wraps (`u32::MAX -> 0`).
///
/// The SINGLE source of truth for "is this tick ahead of that one": both client reconciliation
/// ([`crate::client_server::prediction::ClientPredictor::reconcile`]) and the server's per-client
/// ACK ledger use it. A plain `>` desyncs on wraparound (the server's ACK never advances again
/// after the wrap → the client's queue grows without bound), which is why it lives in one place.
#[inline]
pub fn tick_is_newer(candidate: u32, reference: u32) -> bool {
    (candidate.wrapping_sub(reference) as i32) > 0
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
    /// The shared world state broadcast to every client — what interpolation runs on.
    WorldStateUpdate {
        /// The authoritative tick at which the server produced this state — the timeline
        /// interpolation runs against.
        server_tick: u32,
        /// One entry per replicated entity, keyed by an application-chosen id.
        ///
        /// This key space is **not** the renet `client_id` carried by `PlayerConnected` /
        /// `PlayerDisconnected`: the reference server fills the map straight from an ECS
        /// query and keys it by the entity index widened to `u64`. Mapping one space onto
        /// the other is the application's job.
        ///
        /// Broadcast on [`ServerChannel::Unreliable`]. The whole `WorldStateUpdate` is a
        /// single message, so individual entries never go missing — a loss drops the entire
        /// map at once, and surviving updates can arrive out of order. Feed them to
        /// [`SnapshotInterpolator`](crate::client_server::interpolation::SnapshotInterpolator),
        /// which re-sorts by timestamp, instead of applying them to transforms directly.
        players: HashMap<u64, TransformData>,
    },
    /// The per-client reconciliation ACK, sent only to the client it concerns: the tick of the
    /// last input the server processed from it. The client drops everything up to that tick from
    /// its queue and re-simulates the rest.
    InputAck {
        /// Tick of the newest input from *this* client that the server has consumed.
        ///
        /// Hand it to
        /// [`ClientPredictor::reconcile`](crate::client_server::prediction::ClientPredictor::reconcile)
        /// as `server_tick`: everything up to and including it leaves the pending queue, and
        /// what remains is replayed on top of the authoritative state. The server only ever
        /// advances this through [`tick_is_newer`], so it stays monotone across wraparound.
        ///
        /// Caveat: the reference server reports `0` for a client whose input it has not
        /// processed yet, which is indistinguishable from "processed tick 0" — so a client's
        /// very first input can leave the replay queue one ACK early.
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

/// Lowers the channel to the `channel_id` byte renet routes on.
///
/// These numbers are wire contract, not an implementation detail: they must be identical on
/// both ends and must match the ids registered in [`connection_config`]. The netcode handshake
/// authenticates on [`PROTOCOL_ID`] alone and never sees the channel table, so renumbering
/// these without bumping it still lets mismatched builds connect; renet then rejects the first
/// packet naming an unregistered id by disconnecting
/// (`DisconnectReason::ReceivedInvalidChannelId`). The failure is a mid-session drop, not a
/// refused connection.
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

/// Lowers the channel to the `channel_id` byte renet routes on.
///
/// Client and server channel ids are numbered independently — both start at 0 — so
/// `ClientChannel::Command` and `ServerChannel::Reliable` share the byte `0` without
/// colliding. Same contract as the server side: identical on both ends, matching
/// [`connection_config`].
impl From<ClientChannel> for u8 {
    fn from(val: ClientChannel) -> Self {
        match val {
            ClientChannel::Command => 0,
        }
    }
}

/// Builds the renet [`ConnectionConfig`] shared by client and server (channels + bandwidth).
///
/// Both ends must build from this one function. Renet routes strictly by `channel_id`: a peer
/// that registers a different set does not mis-route, it drops the connection — a packet naming
/// an id the receiver has not registered (or has registered with the other send type) triggers
/// `DisconnectReason::ReceivedInvalidChannelId`, and sending on an id missing from the local
/// config panics outright.
///
/// The numbers, since they are otherwise invisible: a 1 MiB per-tick send budget, and a 5 MiB
/// ceiling on the memory any single channel may hold in its queues (the backstop that stops a
/// stalled peer from eating the process). The server's reliable channel resends after 200 ms.
///
/// Player input travels on an *unreliable* channel on purpose: a dropped input must not stall
/// the ones behind it. That choice is exactly why the client keeps a queue of unacknowledged
/// inputs and the server acknowledges only the newest tick it has processed
/// ([`ServerMessage::InputAck`]) rather than every one of them.
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

    /// Test-local codec, mirroring `rollback::transport`'s: bincode 2 with the LEGACY config, so
    /// these round-trips exercise the same encoding the wire uses rather than 2.x's varint
    /// default. Kept here rather than shared because `client_server` and `rollback` are separate
    /// protocols behind separate features — one accidentally following the other's config change
    /// is precisely the failure this comment exists to prevent.
    fn enc<T: serde::Serialize>(v: &T) -> Vec<u8> {
        bincode::serde::encode_to_vec(v, bincode::config::legacy()).unwrap()
    }
    fn dec<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> T {
        bincode::serde::decode_from_slice::<T, _>(bytes, bincode::config::legacy())
            .unwrap()
            .0
    }
    use super::*;

    #[test]
    fn tick_is_newer_handles_wraparound() {
        // Normal ordering.
        assert!(tick_is_newer(1, 0));
        assert!(!tick_is_newer(0, 0));
        assert!(!tick_is_newer(0, 1));
        assert!(tick_is_newer(5000, 4999));
        // Wraparound: 0 comes right after u32::MAX and must count as newer — a
        // plain `>` would say `0 > u32::MAX == false` and freeze the ACK forever.
        assert!(tick_is_newer(0, u32::MAX));
        assert!(tick_is_newer(5, u32::MAX - 2));
        assert!(!tick_is_newer(u32::MAX, 0));
        assert!(!tick_is_newer(u32::MAX - 2, 5));
    }

    #[test]
    fn client_input_roundtrip() {
        let input = PlayerInput { tick: 42, move_x: 1.0, move_z: -0.5, jump: true, dt: 0.016 };
        let bytes = enc(&ClientMessage::Input(input));
        let ClientMessage::Input(back) = dec(&bytes);
        assert_eq!(back, input);
    }

    #[test]
    fn input_ack_roundtrip() {
        let bytes = enc(&ServerMessage::InputAck { last_processed_input: 7 });
        match dec::<ServerMessage>(&bytes) {
            ServerMessage::InputAck { last_processed_input } => assert_eq!(last_processed_input, 7),
            other => panic!("beklenmeyen varyant: {other:?}"),
        }
    }

    #[test]
    fn world_state_roundtrip() {
        let mut players = HashMap::new();
        players.insert(1u64, TransformData { position: [1.0, 2.0, 3.0], rotation: [0.0, 0.0, 0.0, 1.0] });
        let bytes = enc(&ServerMessage::WorldStateUpdate { server_tick: 100, players });
        match dec::<ServerMessage>(&bytes) {
            ServerMessage::WorldStateUpdate { server_tick, players } => {
                assert_eq!(server_tick, 100);
                assert_eq!(players[&1].position, [1.0, 2.0, 3.0]);
            }
            other => panic!("beklenmeyen varyant: {other:?}"),
        }
    }

    #[test]
    fn player_connected_roundtrip() {
        let bytes = enc(&ServerMessage::PlayerConnected { client_id: 77 });
        match dec::<ServerMessage>(&bytes) {
            ServerMessage::PlayerConnected { client_id } => assert_eq!(client_id, 77),
            other => panic!("beklenmeyen varyant: {other:?}"),
        }
    }

    // 64-bit client_id'nin üst bitleri wire üzerinde korunmalı (32-bit'e kırpılmamalı).
    #[test]
    fn player_disconnected_roundtrip_preserves_full_64bit_id() {
        let big = 0x1234_5678_9ABC_DEF0u64;
        let bytes =
            enc(&ServerMessage::PlayerDisconnected { client_id: big });
        match dec::<ServerMessage>(&bytes) {
            ServerMessage::PlayerDisconnected { client_id } => assert_eq!(client_id, big),
            other => panic!("beklenmeyen varyant: {other:?}"),
        }
    }

    // tick_is_newer, pencere içinde (diff < 2^31) tam bir sıralama: a≠b için tam biri
    // "daha yeni"dir. Tam zıt kutupta (diff = 2^31) yön tanımsızdır → TASARIM GEREĞİ
    // her iki yön de false.
    #[test]
    fn tick_is_newer_is_antisymmetric_and_false_at_the_antipode() {
        for base in [0u32, 1000, u32::MAX - 5, u32::MAX / 2] {
            for d in 1u32..40 {
                let a = base.wrapping_add(d);
                assert!(tick_is_newer(a, base), "base+{d}, base'ten yeni olmalı");
                assert!(!tick_is_newer(base, a), "base, base+{d}'ten yeni OLMAMALI");
            }
        }
        let antipode = 1u32 << 31;
        assert!(!tick_is_newer(0, antipode), "zıt kutupta yön belirsiz → false");
        assert!(!tick_is_newer(antipode, 0), "zıt kutupta yön belirsiz → false");
    }

    // Birden çok oyuncu ve dönüş verisi HashMap tur-gidişinde eksiksiz korunmalı
    // (mevcut test yalnız tek oyuncu/tek alan bakıyordu).
    #[test]
    fn world_state_roundtrip_preserves_all_players_and_rotation() {
        let mut players = HashMap::new();
        players.insert(1u64, TransformData { position: [1.0, 2.0, 3.0], rotation: [0.1, 0.2, 0.3, 0.9] });
        players.insert(9u64, TransformData { position: [-4.0, 5.5, 6.0], rotation: [0.0, 0.0, 1.0, 0.0] });
        let bytes =
            enc(&ServerMessage::WorldStateUpdate { server_tick: 42, players });
        match dec::<ServerMessage>(&bytes) {
            ServerMessage::WorldStateUpdate { server_tick, players } => {
                assert_eq!(server_tick, 42);
                assert_eq!(players.len(), 2);
                assert_eq!(players[&9].position, [-4.0, 5.5, 6.0]);
                assert_eq!(players[&9].rotation, [0.0, 0.0, 1.0, 0.0]);
                assert_eq!(players[&1].rotation[3], 0.9);
            }
            other => panic!("beklenmeyen varyant: {other:?}"),
        }
    }
}
