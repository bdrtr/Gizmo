use serde::{Deserialize, Serialize};

/// Oyuncunun tek bir karedeki (tick) girdisi.
/// Çoğu dövüş ve fizik oyununda 32-bit veya 64-bit bir maske (bitmask) tüm tuşlara yeter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlayerInput {
    /// Simulation tick this input belongs to.
    pub tick: u64,
    /// Bitmask of pressed buttons.
    pub buttons: u32,
    pub joystick_x: i8, // -127 to 127
    /// Vertical analog-stick axis, range -127..=127.
    pub joystick_y: i8,
}

impl PlayerInput {
    /// Returns a neutral (no buttons, centered stick) input for the given tick.
    pub fn empty(tick: u64) -> Self {
        Self {
            tick,
            buttons: 0,
            joystick_x: 0,
            joystick_y: 0,
        }
    }
}

/// Her bir oyuncunun uzak ve yerel (local) girdilerini sakladığı dairesel tampon.
#[derive(Debug, Clone)]
pub struct InputBuffer {
    /// Id of the player this buffer stores inputs for.
    pub player_id: u32,
    buffer: Vec<Option<PlayerInput>>,
    capacity: usize,
    /// En son "doğrulanan" girdi tick'i (bu tick'e kadar her şey kesin doğru)
    pub last_confirmed_tick: u64,
}

impl InputBuffer {
    /// Creates a ring buffer holding up to `capacity` recent inputs for `player_id`.
    pub fn new(player_id: u32, capacity: usize) -> Self {
        // Modulo-by-zero koruması: ring buffer indekslemesi `% self.capacity`
        // kullandığı için capacity=0 ilk insert/get'te panik üretir. İmza
        // değiştirmeden en az 1 kapasiteye normalize ediyoruz (RollbackSession
        // içindeki .max(64) deseniyle tutarlı; başarı yolu etkilenmez).
        let capacity = capacity.max(1);
        Self {
            player_id,
            buffer: vec![None; capacity],
            capacity,
            last_confirmed_tick: 0,
        }
    }

    /// Stores an input at its tick slot and advances `last_confirmed_tick` if it is newer.
    pub fn insert(&mut self, input: PlayerInput) {
        let index = (input.tick as usize) % self.capacity;
        self.buffer[index] = Some(input);
        
        // Uzak oyuncudan veri geldikçe confirmed_tick güncellenir.
        if input.tick > self.last_confirmed_tick {
            self.last_confirmed_tick = input.tick;
        }
    }

    /// Belirtilen Tick'teki girdiyi okur. Eğer girdi henüz ağdan gelmemişse, 
    /// tahmin (Prediction) yaparak en son bilinen girdiyi döndürür.
    pub fn get_or_predict(&self, tick: u64) -> PlayerInput {
        let index = (tick as usize) % self.capacity;
        
        if let Some(input) = &self.buffer[index] {
            if input.tick == tick {
                return *input; // Orijinal gerçek girdi var
            }
        }

        // Girdi henüz gelmemiş, "Tahmin" (Prediction) yap.
        // Genellikle oyuncu tuşa basılı tutmaya devam ediyordur mantığıyla
        // en son doğrulanmış girdi kopyalanır.
        let mut predicted = PlayerInput::empty(tick);
        let last_idx = (self.last_confirmed_tick as usize) % self.capacity;
        
        if let Some(last_input) = &self.buffer[last_idx] {
            if last_input.tick == self.last_confirmed_tick {
                predicted.buttons = last_input.buttons;
                predicted.joystick_x = last_input.joystick_x;
                predicted.joystick_y = last_input.joystick_y;
            }
        }
        
        predicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(tick: u64, buttons: u32) -> PlayerInput {
        PlayerInput {
            tick,
            buttons,
            joystick_x: 0,
            joystick_y: 0,
        }
    }

    #[test]
    fn insert_then_get_returns_the_exact_input() {
        let mut b = InputBuffer::new(1, 64);
        b.insert(input(10, 0b101));
        let got = b.get_or_predict(10);
        assert_eq!(got.tick, 10);
        assert_eq!(got.buttons, 0b101, "confirmed input must be returned verbatim");
    }

    #[test]
    fn last_confirmed_tracks_the_newest_insert_only() {
        let mut b = InputBuffer::new(1, 64);
        b.insert(input(5, 0));
        b.insert(input(9, 0));
        b.insert(input(7, 0)); // older than 9 — must NOT lower last_confirmed
        assert_eq!(b.last_confirmed_tick, 9);
    }

    #[test]
    fn missing_input_predicts_by_repeating_last_confirmed() {
        let mut b = InputBuffer::new(1, 64);
        b.insert(input(4, 0b11));
        // Tick 5 hasn't arrived → predict; GGPO repeats the last confirmed input.
        let p = b.get_or_predict(5);
        assert_eq!(p.tick, 5, "prediction carries the requested tick");
        assert_eq!(p.buttons, 0b11, "prediction must repeat the last confirmed buttons");
    }

    #[test]
    fn prediction_is_neutral_before_any_input() {
        let b = InputBuffer::new(1, 64);
        let p = b.get_or_predict(3);
        assert_eq!(p, PlayerInput::empty(3), "no history → neutral prediction");
    }

    // The ring buffer indexes by `tick % capacity`. A stale entry sharing a slot
    // with the requested tick must NOT be mistaken for a real input — otherwise a
    // rollback peer would treat an ancient input as confirmed and desync. Two
    // inserts make the correct answer (predict from the last confirmed) differ
    // from the stale slot occupant, so the two cases are distinguishable.
    #[test]
    fn wraparound_slot_is_not_returned_as_a_real_input() {
        let cap = 8;
        let mut b = InputBuffer::new(1, cap);
        b.insert(input(2, 0b1)); // occupies slot 2 (== slot for tick 10)
        b.insert(input(3, 0b10)); // slot 3; now the last confirmed input
        // Tick 10 (== 2 + cap) shares slot 2 with the stale tick-2 input.
        let got = b.get_or_predict(2 + cap as u64);
        assert_eq!(got.tick, 10);
        assert_eq!(
            got.buttons, 0b10,
            "tick 10 must be predicted from the last confirmed input (tick 3), not \
             read from the stale tick-2 occupant of its ring slot"
        );
    }

    #[test]
    fn capacity_zero_is_normalized_and_never_panics() {
        let mut b = InputBuffer::new(1, 0);
        b.insert(input(0, 1));
        let _ = b.get_or_predict(0);
        let _ = b.get_or_predict(1000);
    }
}
