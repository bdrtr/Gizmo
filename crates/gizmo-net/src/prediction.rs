//! Client-Side Prediction & Server Reconciliation
//! 
//! İstemcinin (Client) kendi hareketlerini sunucuyu beklemeden anında uygulaması (Prediction)
//! ve sunucudan gelen kesin (Authoritative) sonuçlara göre gerekirse geçmişe dönüp düzeltmesi (Reconciliation)

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// İstemcinin sunucuya gönderdiği tek bir girdi (Input) karesi
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerInput {
    pub tick: u32,
    pub move_x: f32,
    pub move_z: f32,
    pub jump: bool,
    pub dt: f32,
}

/// İstemcinin öngördüğü yerel durum (Fizik motoru üzerinde anlık uygulanacak)
#[derive(Debug, Clone)]
pub struct PredictedState {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
}

pub struct ClientPredictor {
    /// Sunucuya gönderilmiş ama henüz sunucudan onayı (ACK) gelmemiş girdiler
    pub pending_inputs: VecDeque<PlayerInput>,
    /// İstemcinin şu anki simülasyon tick'i
    pub current_tick: u32,
}

impl ClientPredictor {
    pub fn new() -> Self {
        Self {
            pending_inputs: VecDeque::new(),
            current_tick: 0,
        }
    }

    /// Yeni bir girdi üret ve kuyruğa ekle
    pub fn add_input(&mut self, move_x: f32, move_z: f32, jump: bool, dt: f32) -> PlayerInput {
        let input = PlayerInput {
            tick: self.current_tick,
            move_x,
            move_z,
            jump,
            dt,
        };
        self.pending_inputs.push_back(input.clone());
        self.current_tick = self.current_tick.wrapping_add(1);
        input
    }

    /// Sunucudan Authoritative State geldiğinde (Reconciliation)
    /// Hatalı tahmin varsa düzeltmek için çağrılır.
    ///
    /// - `server_tick`: Sunucunun işlediği son girdinin tick değeri
    /// - `server_state`: Sunucunun onayladığı kesin pozisyon/hız
    /// - `apply_physics_fn`: Geçmiş girdileri yeniden simüle etmek için kullanılacak closure
    pub fn reconcile<F>(
        &mut self,
        server_tick: u32,
        server_state: PredictedState,
        mut apply_physics_fn: F,
    ) -> PredictedState
    where
        F: FnMut(&PredictedState, &PlayerInput) -> PredictedState,
    {
        // 1. Sunucunun onayladığı girdileri kuyruktan sil
        self.pending_inputs.retain(|input| {
            // tick wraparound durumlarını handle etmek için geniş mesafe kontrolü
            let diff = input.tick.wrapping_sub(server_tick) as i32;
            diff > 0
        });

        // 2. Onaylanmış (Authoritative) durumu al
        let mut corrected_state = server_state;

        // 3. Kalan onaylanmamış girdileri sırayla tekrar uygula (Replay)
        for input in &self.pending_inputs {
            corrected_state = apply_physics_fn(&corrected_state, input);
        }

        corrected_state
    }
}
