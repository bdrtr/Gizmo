//! Fighting-game input recording/playback: `FrameRecord`/`PlaybackData` (deterministic replay)
//! and the `FighterInputBuffer` motion/command buffer. Extracted verbatim from input.rs.

use super::*;
#[derive(Serialize, Deserialize, Clone)]
pub struct FrameRecord {
    pub dt: f32,
    pub input: Input,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PlaybackData {
    pub frames: Vec<FrameRecord>,
}

impl PlaybackData {
    pub fn save(&self, path: &str) -> Result<(), String> {
        let string_data = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| format!("Serilestirme hatasi: {}", e))?;
        std::fs::write(path, string_data).map_err(|e| format!("Dosya yazma hatasi: {}", e))?;
        Ok(())
    }

    pub fn load(path: &str) -> Result<Self, String> {
        let string_data =
            std::fs::read_to_string(path).map_err(|e| format!("Dosya okuma hatasi: {}", e))?;
        ron::from_str(&string_data).map_err(|e| format!("Deserilestirme hatasi: {}", e))
    }
}

// ==================== FIGHTER INPUT BUFFER ====================

/// Her frame için tuş durumlarını tutan yapı.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameActions {
    pub pressed: HashSet<String>,
    pub just_pressed: HashSet<String>,
    pub just_released: HashSet<String>,
}

/// Dövüş oyunları (Gizmo Fight) için özel olarak tasarlanmış Girdi Belleği (Input Buffer).
/// Son N karedeki tüm tuş hareketlerini hafızada tutarak kombo (Hadouken vb.) algılamayı sağlar.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FighterInputBuffer {
    pub frames: std::collections::VecDeque<FrameActions>,
    pub max_frames: usize,
}

impl FighterInputBuffer {
    /// 60 kare (1 saniye) standart bir buffer boyutu dövüş oyunları için idealdir.
    pub fn new(max_frames: usize) -> Self {
        Self {
            frames: std::collections::VecDeque::with_capacity(max_frames),
            max_frames,
        }
    }

    /// Her oyun karesinde çağrılıp buffer'ı günceller.
    pub fn update(&mut self, input: &Input, action_map: &ActionMap, actions_to_track: &[&str]) {
        let mut frame = FrameActions {
            pressed: HashSet::new(),
            just_pressed: HashSet::new(),
            just_released: HashSet::new(),
        };

        for &action in actions_to_track {
            if action_map.is_action_pressed(input, action) {
                frame.pressed.insert(action.to_string());
            }
            if action_map.is_action_just_pressed(input, action) {
                frame.just_pressed.insert(action.to_string());
            }
            if action_map.is_action_just_released(input, action) {
                frame.just_released.insert(action.to_string());
            }
        }

        self.frames.push_front(frame);
        if self.frames.len() > self.max_frames {
            self.frames.pop_back();
        }
    }

    /// Verilen kombo diziliminin son karelerde gerçekleşip gerçekleşmediğini kontrol eder.
    /// `sequence`: Sırasıyla basılması gereken tuşlar dizisi. Örn: ["Down", "Right", "Punch"]
    /// `max_gap`: İki tuş basımı arasında geçebilecek maksimum kare sayısı (Hata toleransı).
    /// Dövüş oyunlarında genellikle 10-15 kare tolerans verilir.
    pub fn check_combo_strict(&self, sequence: &[&str], max_gap: usize) -> bool {
        if sequence.is_empty() || self.frames.is_empty() {
            return false;
        }

        // Aramaya dizilimin SON tuşundan (en yakın zamandaki) başlıyoruz.
        // Çünkü `self.frames[0]` mevcut frame'i (şimdi) temsil eder.
        let mut seq_idx = sequence.len() as isize - 1;
        let mut frames_since_last_match = 0;

        for frame in &self.frames {
            if frames_since_last_match > max_gap {
                // Kombodaki iki tuş arasına çok fazla zaman girmiş, kombo bozuldu.
                return false;
            }

            let required_action = sequence[seq_idx as usize];

            // Dövüş oyunlarında yön tuşları 'pressed', saldırı tuşları 'just_pressed' olabilir
            // ama en güvenlisi kombodaki her adımın 'just_pressed' (yeni basılmış) olmasıdır.
            if frame.just_pressed.contains(required_action) || frame.pressed.contains(required_action) {
                // Eşleşme bulundu, komboda bir önceki adıma geç
                seq_idx -= 1;
                frames_since_last_match = 0;

                if seq_idx < 0 {
                    // Dizilimin en başına (ilk tuşa) başarıyla ulaştık! Kombo yapıldı!
                    return true;
                }
            } else {
                frames_since_last_match += 1;
            }
        }

        false
    }
}

impl Default for FighterInputBuffer {
    fn default() -> Self {
        Self::new(60)
    }
}

#[cfg(test)]
mod fighter_tests {
    use super::*;

    #[test]
    fn test_fighter_input_buffer_combo() {
        let mut buffer = FighterInputBuffer::new(60);
        let _input = Input::new();
        let _action_map = ActionMap::new();

        // 1. Frame: Sadece Down (pressed olarak gelecek)
        let frame1 = FrameActions {
            pressed: ["Down".to_string()].into_iter().collect(),
            just_pressed: [].into_iter().collect(),
            just_released: [].into_iter().collect(),
        };
        buffer.frames.push_front(frame1);

        // 2. Frame: DownRight (Down + Right pressed)
        let frame2 = FrameActions {
            pressed: ["Down".to_string(), "Right".to_string()].into_iter().collect(),
            just_pressed: ["Right".to_string()].into_iter().collect(),
            just_released: [].into_iter().collect(),
        };
        buffer.frames.push_front(frame2);

        // 3. Frame: Sadece Right (pressed), Down bırakıldı
        let frame3 = FrameActions {
            pressed: ["Right".to_string()].into_iter().collect(),
            just_pressed: [].into_iter().collect(),
            just_released: ["Down".to_string()].into_iter().collect(),
        };
        buffer.frames.push_front(frame3);

        // 4. Frame: Punch (just_pressed)
        let frame4 = FrameActions {
            pressed: ["LightPunch".to_string()].into_iter().collect(),
            just_pressed: ["LightPunch".to_string()].into_iter().collect(),
            just_released: [].into_iter().collect(),
        };
        buffer.frames.push_front(frame4);

        // Şimdi kombo arıyoruz: ["Down", "Right", "LightPunch"]
        let combo = ["Down", "Right", "LightPunch"];
        
        // max_gap = 5 kare (Çok rahat yetişir)
        assert!(buffer.check_combo_strict(&combo, 5), "Kombo basariyla algilanmali");
        
        // Kombo sırasını bozarak test edelim
        let wrong_combo = ["LightPunch", "Right", "Down"];
        assert!(!buffer.check_combo_strict(&wrong_combo, 5), "Yanlis kombo sirasi algilanmamali");
    }

    #[test]
    fn test_fighter_input_buffer_max_gap() {
        let mut buffer = FighterInputBuffer::new(60);

        let frame_down = FrameActions {
            pressed: ["Down".to_string()].into_iter().collect(),
            just_pressed: ["Down".to_string()].into_iter().collect(),
            just_released: [].into_iter().collect(),
        };
        buffer.frames.push_front(frame_down);

        // Araya 10 boş kare girsin
        for _ in 0..10 {
            let empty = FrameActions {
                pressed: [].into_iter().collect(),
                just_pressed: [].into_iter().collect(),
                just_released: [].into_iter().collect(),
            };
            buffer.frames.push_front(empty);
        }

        let frame_punch = FrameActions {
            pressed: ["LightPunch".to_string()].into_iter().collect(),
            just_pressed: ["LightPunch".to_string()].into_iter().collect(),
            just_released: [].into_iter().collect(),
        };
        buffer.frames.push_front(frame_punch);

        let combo = ["Down", "LightPunch"];
        
        // max_gap = 5 ise başarısız olmalı (10 kare boşluk var)
        assert!(!buffer.check_combo_strict(&combo, 5), "Cok yavas basildi, algilanmamali");
        
        // max_gap = 15 ise başarılı olmalı
        assert!(buffer.check_combo_strict(&combo, 15), "Max gap genis oldugu icin algilanmali");
    }
}

