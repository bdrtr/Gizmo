//! Snapshot Interpolation (Durum Aradeğerlemesi)
//!
//! Diğer oyuncuların ağ üzerinden gelen konum/dönüş verilerini
//! akıcı bir şekilde ekrana yansıtmak için geçmiş sunucu Snapshot'ları arasında
//! interpolasyon (Lerp/Slerp) yapar.

use std::collections::VecDeque;

/// A timestamped transform sample received from the server, used as an interpolation keyframe.
#[derive(Debug, Clone)]
pub struct TransformSnapshot {
    /// Server timestamp (seconds) this sample is valid for.
    pub time: f64,
    /// World-space position `[x, y, z]`.
    pub position: [f32; 3],
    pub rotation: [f32; 4], // Quaternion (x, y, z, w)
}

/// Buffers server snapshots and produces smoothed transforms by interpolating slightly in the past.
#[derive(Debug, Clone)]
pub struct SnapshotInterpolator {
    buffer: VecDeque<TransformSnapshot>,
    /// İstemci gösteriminde bırakılacak gecikme süresi (Saniye)
    /// Örneğin 0.1s (100ms) ise oyuncu 100ms geçmişi izler ama çok akıcı olur.
    pub interpolation_delay: f64,
}

impl SnapshotInterpolator {
    /// Creates an interpolator with the given render delay in milliseconds.
    pub fn new(interpolation_delay_ms: f64) -> Self {
        Self {
            buffer: VecDeque::new(),
            interpolation_delay: interpolation_delay_ms / 1000.0,
        }
    }

    /// Sunucudan yeni bir durum geldiğinde tampona ekler.
    ///
    /// `get_interpolated_transform`'daki tarama, tampon **zaman-artan** sıralı olduğunu
    /// varsayar (S1.time <= render_time <= S2.time'ı saran ikiliyi soldan sağa bulur).
    /// Ağ paketleri sırasız gelebileceğinden (jitter/yeniden-sıralama), burada sıralı
    /// konuma ekleyerek bu değişmezi (invariant) her zaman koruruz. Aksi halde
    /// `buffer = [{5.0}, {3.0}]` gibi bir durumda tarama yanlış ikiliyi seçip
    /// interpolasyon yerine tek (gelecek) snapshot'ı döndürürdü.
    pub fn add_snapshot(&mut self, time: f64, position: [f32; 3], rotation: [f32; 4]) {
        let snapshot = TransformSnapshot {
            time,
            position,
            rotation,
        };

        // Yaygın (sıralı) durum hızlı yol: en yeni örnek genelde en büyük zamandır.
        if self.buffer.back().is_none_or(|last| time >= last.time) {
            self.buffer.push_back(snapshot);
        } else {
            // Sırasız geldi: zaman-artan sırayı koruyacak konuma ekle.
            let insert_at = self
                .buffer
                .iter()
                .position(|s| s.time > time)
                .unwrap_or(self.buffer.len());
            self.buffer.insert(insert_at, snapshot);
        }

        // Tampon çok büyürse eskileri temizle (Örn: 2 saniyeden eskiler çöp).
        // Referans olarak eklenen `time`'ı değil, tampondaki EN YENİ zamanı (artık
        // sıralı olduğundan `back`) kullan; böylece sırasız gelen eski bir örnek
        // budama eşiğini yanlış hesaplamaz.
        let newest = self.buffer.back().map_or(time, |s| s.time);
        while let Some(oldest) = self.buffer.front() {
            if newest - oldest.time > 2.0 {
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

                // Nlerp (Rotation). KRİTİK: `q` ve `-q` AYNI yönelimi temsil eder, bu yüzden
                // ardışık iki snapshot rutin olarak ZIT yarıkürelerde gelir. Yarıküreyi
                // hizalamadan (dot < 0 iken birini negatiflemeden) lerp UZUN yoldan gider ve
                // sıfır-norm quaternion'a yaklaşır → uzak oyuncunun yönelimi yanlış yöne
                // döner / sıçrar. dot işaretine göre s2'yi s1 ile aynı yarıküreye çek.
                let dot = s1.rotation[0] * s2.rotation[0]
                    + s1.rotation[1] * s2.rotation[1]
                    + s1.rotation[2] * s2.rotation[2]
                    + s1.rotation[3] * s2.rotation[3];
                let sign = if dot < 0.0 { -1.0 } else { 1.0 };
                let mut rot = [
                    s1.rotation[0] + (s2.rotation[0] * sign - s1.rotation[0]) * t,
                    s1.rotation[1] + (s2.rotation[1] * sign - s1.rotation[1]) * t,
                    s1.rotation[2] + (s2.rotation[2] * sign - s1.rotation[2]) * t,
                    s1.rotation[3] + (s2.rotation[3] * sign - s1.rotation[3]) * t,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_returns_none() {
        let interp = SnapshotInterpolator::new(0.0);
        assert!(interp.get_interpolated_transform(0.5).is_none());
    }

    #[test]
    fn interpolates_midpoint_between_two_snapshots() {
        let mut interp = SnapshotInterpolator::new(0.0); // gecikme yok
        interp.add_snapshot(0.0, [0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]);
        interp.add_snapshot(1.0, [10.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]);

        // render_time = 0.5 → tam orta nokta
        let (pos, _rot) = interp.get_interpolated_transform(0.5).unwrap();
        assert!((pos[0] - 5.0).abs() < 1e-5, "beklenen 5.0, gelen {}", pos[0]);
    }

    // REGRESYON (audit round 2): ardışık snapshot'lar zıt yarıkürelerde (q vs -q)
    // geldiğinde Nlerp KISA yoldan gitmeli. s2, 90°-Y dönüşünün NEGATİF quaternion'u
    // olarak gelir (aynı yönelim); düzeltme olmadan lerp uzun yoldan ~135°'ye gider.
    #[test]
    fn nlerp_takes_short_way_across_opposite_hemispheres() {
        let mut interp = SnapshotInterpolator::new(0.0);
        // s1 = identity. s2 = 90° about Y, stored as its NEGATED quaternion.
        let h = std::f32::consts::FRAC_1_SQRT_2; // sin45 = cos45 ≈ 0.7071
        interp.add_snapshot(0.0, [0.0; 3], [0.0, 0.0, 0.0, 1.0]);
        interp.add_snapshot(1.0, [0.0; 3], [0.0, -h, 0.0, -h]); // == +Y 90°, opposite hemisphere

        let (_pos, rot) = interp.get_interpolated_transform(0.5).unwrap();
        // Short way to halfway (45° about +Y): w ≈ cos22.5 ≈ 0.924, y ≈ sin22.5 ≈ 0.383.
        // The old long-way bug gave w ≈ 0.383 (≈135°) — discriminating.
        assert!(rot[3] > 0.9, "took the long way: w = {} (expected ≈0.924)", rot[3]);
        assert!(rot[1] > 0.3, "wrong rotation axis sign: y = {}", rot[1]);
    }

    // REGRESYON (bulgu 32): snapshot'lar sırasız gelirse (jitter/yeniden-sıralama)
    // tampon zaman-artan kalmalı, böylece render_time'ı saran ikili doğru bulunur.
    // Düzeltme öncesi buffer=[{5.0},{3.0}] olurdu ve render_time=4.0 için tarama
    // interpolasyon yerine tek (gelecek) snapshot'ı döndürürdü.
    #[test]
    fn interpolates_when_snapshots_arrive_out_of_order() {
        let mut interp = SnapshotInterpolator::new(0.0);
        // Önce gelecek örnek (5.0), sonra geçmiş örnek (3.0) gelir.
        interp.add_snapshot(5.0, [50.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]);
        interp.add_snapshot(3.0, [30.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]);

        // render_time = 4.0 → 3.0 ve 5.0 arası tam orta nokta (x = 40.0).
        let (pos, _rot) = interp.get_interpolated_transform(4.0).unwrap();
        assert!(
            (pos[0] - 40.0).abs() < 1e-5,
            "sırasız gelen snapshot'lar arasında interpolasyon başarısız: beklenen 40.0, gelen {}",
            pos[0]
        );
    }

    #[test]
    fn clamps_to_last_known_when_ahead() {
        let mut interp = SnapshotInterpolator::new(0.0);
        interp.add_snapshot(0.0, [0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]);
        interp.add_snapshot(1.0, [10.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]);

        // Tüm snapshot'lardan ileride → son bilinen konum
        let (pos, _rot) = interp.get_interpolated_transform(5.0).unwrap();
        assert_eq!(pos[0], 10.0);
    }
}
