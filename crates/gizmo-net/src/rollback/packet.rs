use serde::{Deserialize, Serialize};
use super::input_buffer::PlayerInput;
use super::snapshot::PhysicsStateSnapshot;

/// Ağ üzerinden gönderilen tüm verilerin genel zarfı (Envelope).
#[derive(Clone, Debug, Serialize, Deserialize)]
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
