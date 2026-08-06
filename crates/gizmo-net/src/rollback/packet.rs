//! Wire format: the one envelope every rollback peer sends and receives.
//!
//! Everything that crosses the network is a [`NetworkPacket`] serialized with `bincode`
//! (see `UdpTransport::send_packet`), which encodes the variant as a leading index. So
//! the variant *order* is part of the wire format: appending a variant is safe for old
//! peers' existing traffic, but an old build that receives the new index fails to
//! deserialize and `UdpTransport::poll_events` logs the datagram and drops it. Both peers
//! must therefore run the same build — there is no version handshake. Separately, the enum
//! carries `#[non_exhaustive]` from the workspace-wide 1.0 semver pass (adding a variant must
//! not break downstream crates), so downstream `match`es need a wildcard arm.
//!
//! Only `Input` is actually driven by this crate. `RollbackSession::advance` matches
//! `Input` and ignores every other variant, and nothing here emits a `Ping`, answers one
//! with a `Pong`, or acts on a `FullState`: those variants exist so an application (or a
//! future resync path) can carry them over the same socket.

use serde::{Deserialize, Serialize};
use super::input_buffer::PlayerInput;
use super::snapshot::PhysicsStateSnapshot;

/// Ağ üzerinden gönderilen tüm verilerin genel zarfı (Envelope).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum NetworkPacket {
    /// Oynanış sırasında en sık yollanacak paket.
    /// Sadece oyuncunun o karedeki (tick) girişlerini içerir.
    Input(PlayerInput),

    /// İki bilgisayar arasındaki gecikmeyi ölçmek için.
    Ping {
        /// The sender's clock reading when the ping left, to be echoed back verbatim in
        /// the matching [`NetworkPacket::Pong`]; subtracting it from the sender's clock
        /// on arrival of the pong gives the round trip.
        ///
        /// The unit is whatever the application stamps — nothing in this crate writes or
        /// interprets it, so both peers must agree on a scale (milliseconds is the usual
        /// choice). It is *not* a simulation tick and has no relation to
        /// [`PlayerInput::tick`].
        timestamp: u64,
    },

    /// Ping'e verilen cevap.
    Pong {
        /// The originating [`NetworkPacket::Ping`]'s `timestamp`, copied through
        /// unchanged. A responder that re-stamps it with its own clock makes the
        /// round-trip measurement meaningless, and worse, silently so — the two peers'
        /// clocks share no epoch.
        timestamp: u64,
    },

    /// Nadiren, eğer oyun çok fazla asenkron (desync) olursa
    /// veya yeni bir oyuncu odaya katılırsa tüm sahne gönderilir.
    FullState(PhysicsStateSnapshot),
}

#[cfg(test)]
mod tests {

    /// bincode 2 with the LEGACY config — the same encoding `rollback::transport` puts on the
    /// wire. Using 2.x's `standard()` default here would round-trip perfectly while testing an
    /// encoding no peer ever sees.
    fn enc<T: serde::Serialize>(v: &T) -> Vec<u8> {
        bincode::serde::encode_to_vec(v, bincode::config::legacy()).unwrap()
    }
    fn dec<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> T {
        bincode::serde::decode_from_slice::<T, _>(bytes, bincode::config::legacy())
            .unwrap()
            .0
    }
    use super::*;
    use crate::rollback::snapshot::EntityState;
    use gizmo_core::Entity;
    use gizmo_math::{Quat, Vec3};

    #[test]
    fn input_packet_roundtrips() {
        let inp = PlayerInput { tick: 123, buttons: 0b1010, joystick_x: -12, joystick_y: 34 };
        let bytes = enc(&NetworkPacket::Input(inp));
        match dec::<NetworkPacket>(&bytes) {
            NetworkPacket::Input(back) => assert_eq!(back, inp),
            other => panic!("Input bekleniyordu, gelen {other:?}"),
        }
    }

    // Ping ve Pong ayrı varyantlar: zarf, timestamp'i koruyarak ve varyantı KARIŞTIRMADAN
    // tur-gidiş yapmalı (gecikme ölçümü buna dayanır).
    #[test]
    fn ping_and_pong_roundtrip_without_variant_confusion() {
        let ping_bytes = enc(&NetworkPacket::Ping { timestamp: 9 });
        match dec::<NetworkPacket>(&ping_bytes) {
            NetworkPacket::Ping { timestamp } => assert_eq!(timestamp, 9),
            other => panic!("Ping bekleniyordu, gelen {other:?}"),
        }
        let pong_bytes = enc(&NetworkPacket::Pong { timestamp: 9 });
        match dec::<NetworkPacket>(&pong_bytes) {
            NetworkPacket::Pong { timestamp } => assert_eq!(timestamp, 9),
            other => panic!("Pong bekleniyordu, gelen {other:?}"),
        }
    }

    // FullState, iç içe PhysicsStateSnapshot'ı (EntityState listesi dahil) korumalı.
    #[test]
    fn full_state_packet_roundtrips_nested_snapshot() {
        let mut snap = PhysicsStateSnapshot {
            tick: 55,
            ..Default::default()
        };
        snap.states.push(EntityState {
            entity: Entity::INVALID,
            position: Vec3::new(1.0, -2.0, 3.0),
            rotation: Quat::IDENTITY,
            linear_velocity: Vec3::new(0.5, 0.0, -0.5),
            angular_velocity: Vec3::ZERO,
            is_sleeping: true,
        });

        let bytes = enc(&NetworkPacket::FullState(snap));
        match dec::<NetworkPacket>(&bytes) {
            NetworkPacket::FullState(back) => {
                assert_eq!(back.tick, 55);
                assert_eq!(back.states.len(), 1);
                assert_eq!(back.states[0].position, Vec3::new(1.0, -2.0, 3.0));
                assert_eq!(back.states[0].linear_velocity, Vec3::new(0.5, 0.0, -0.5));
                assert!(back.states[0].is_sleeping);
            }
            other => panic!("FullState bekleniyordu, gelen {other:?}"),
        }
    }
}
