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
    Ping { timestamp: u64 },

    /// Ping'e verilen cevap.
    Pong { timestamp: u64 },

    /// Nadiren, eğer oyun çok fazla asenkron (desync) olursa
    /// veya yeni bir oyuncu odaya katılırsa tüm sahne gönderilir.
    FullState(PhysicsStateSnapshot),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rollback::snapshot::EntityState;
    use gizmo_core::Entity;
    use gizmo_math::{Quat, Vec3};

    #[test]
    fn input_packet_roundtrips() {
        let inp = PlayerInput { tick: 123, buttons: 0b1010, joystick_x: -12, joystick_y: 34 };
        let bytes = bincode::serialize(&NetworkPacket::Input(inp)).unwrap();
        match bincode::deserialize::<NetworkPacket>(&bytes).unwrap() {
            NetworkPacket::Input(back) => assert_eq!(back, inp),
            other => panic!("Input bekleniyordu, gelen {other:?}"),
        }
    }

    // Ping ve Pong ayrı varyantlar: zarf, timestamp'i koruyarak ve varyantı KARIŞTIRMADAN
    // tur-gidiş yapmalı (gecikme ölçümü buna dayanır).
    #[test]
    fn ping_and_pong_roundtrip_without_variant_confusion() {
        let ping_bytes = bincode::serialize(&NetworkPacket::Ping { timestamp: 9 }).unwrap();
        match bincode::deserialize::<NetworkPacket>(&ping_bytes).unwrap() {
            NetworkPacket::Ping { timestamp } => assert_eq!(timestamp, 9),
            other => panic!("Ping bekleniyordu, gelen {other:?}"),
        }
        let pong_bytes = bincode::serialize(&NetworkPacket::Pong { timestamp: 9 }).unwrap();
        match bincode::deserialize::<NetworkPacket>(&pong_bytes).unwrap() {
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

        let bytes = bincode::serialize(&NetworkPacket::FullState(snap)).unwrap();
        match bincode::deserialize::<NetworkPacket>(&bytes).unwrap() {
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
