//! Snapshot Interpolation (Durum Aradeğerlemesi)
//!
//! Diğer oyuncuların ağ üzerinden gelen konum/dönüş verilerini
//! akıcı bir şekilde ekrana yansıtmak için geçmiş sunucu Snapshot'ları arasında
//! interpolasyon (Lerp/Slerp) yapar.

use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct TransformSnapshot {
    pub time: f64,
    pub position: [f32; 3],
    pub rotation: [f32; 4], // Quaternion (x, y, z, w)
}

pub struct SnapshotInterpolator {
    buffer: VecDeque<TransformSnapshot>,
    /// İstemci gösteriminde bırakılacak gecikme süresi (Saniye)
    /// Örneğin 0.1s (100ms) ise oyuncu 100ms geçmişi izler ama çok akıcı olur.
    pub interpolation_delay: f64,
}

impl SnapshotInterpolator {
    pub fn new(interpolation_delay_ms: f64) -> Self {
        Self {
            buffer: VecDeque::new(),
            interpolation_delay: interpolation_delay_ms / 1000.0,
        }
    }

    /// Sunucudan yeni bir durum geldiğinde tampona ekler
    pub fn add_snapshot(&mut self, time: f64, position: [f32; 3], rotation: [f32; 4]) {
        self.buffer.push_back(TransformSnapshot {
            time,
            position,
            rotation,
        });

        // Tampon çok büyürse eskileri temizle (Örn: 2 saniyeden eskiler çöp)
        while let Some(oldest) = self.buffer.front() {
            if time - oldest.time > 2.0 {
                self.buffer.pop_front();
            } else {
                break;
            }
        }
    }

    /// O anki zamana göre enterpole edilmiş Transform verisini döner
    pub fn get_interpolated_transform(
        &self,
        current_client_time: f64,
    ) -> Option<([f32; 3], [f32; 4])> {
        if self.buffer.is_empty() {
            return None;
        }

        let render_time = current_client_time - self.interpolation_delay;

        // Tamponda render_time'ı saran iki Snapshot bul: S1 ve S2
        // S1.time <= render_time <= S2.time

        let mut s1_index = None;
        let mut s2_index = None;

        for (i, snap) in self.buffer.iter().enumerate() {
            if snap.time <= render_time {
                s1_index = Some(i);
            } else if snap.time > render_time {
                s2_index = Some(i);
                break;
            }
        }

        match (s1_index, s2_index) {
            (Some(i1), Some(i2)) => {
                let s1 = &self.buffer[i1];
                let s2 = &self.buffer[i2];

                let t = ((render_time - s1.time) / (s2.time - s1.time)) as f32;
                let t = t.clamp(0.0, 1.0);

                // Lerp (Position)
                let pos = [
                    s1.position[0] + (s2.position[0] - s1.position[0]) * t,
                    s1.position[1] + (s2.position[1] - s1.position[1]) * t,
                    s1.position[2] + (s2.position[2] - s1.position[2]) * t,
                ];

                // Slerp veya basit Nlerp (Rotation) için yaklaşım (Nlerp performansı iyidir)
                let mut rot = [
                    s1.rotation[0] + (s2.rotation[0] - s1.rotation[0]) * t,
                    s1.rotation[1] + (s2.rotation[1] - s1.rotation[1]) * t,
                    s1.rotation[2] + (s2.rotation[2] - s1.rotation[2]) * t,
                    s1.rotation[3] + (s2.rotation[3] - s1.rotation[3]) * t,
                ];

                // Normalize Quaternion
                let len =
                    (rot[0] * rot[0] + rot[1] * rot[1] + rot[2] * rot[2] + rot[3] * rot[3]).sqrt();
                if len > 0.0001 {
                    rot[0] /= len;
                    rot[1] /= len;
                    rot[2] /= len;
                    rot[3] /= len;
                }

                Some((pos, rot))
            }
            (Some(i1), None) => {
                // Zaman tüm snapshotlardan daha ileride ise son bilinen konumu ver (Extrapolation da yapılabilir)
                let s = &self.buffer[i1];
                Some((s.position, s.rotation))
            }
            (None, Some(i2)) => {
                // Zaman çok gerideyse ilk bilineni ver
                let s = &self.buffer[i2];
                Some((s.position, s.rotation))
            }
            _ => None,
        }
    }
}
