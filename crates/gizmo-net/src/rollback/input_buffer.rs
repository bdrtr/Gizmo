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
