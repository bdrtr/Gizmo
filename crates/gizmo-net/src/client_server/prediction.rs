//! Client-Side Prediction & Server Reconciliation
//!
//! İstemcinin (Client) kendi hareketlerini sunucuyu beklemeden anında uygulaması (Prediction)
//! ve sunucudan gelen kesin (Authoritative) sonuçlara göre gerekirse geçmişe dönüp düzeltmesi (Reconciliation)

use super::protocol::PlayerInput;
use std::collections::VecDeque;

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

impl Default for ClientPredictor {
    fn default() -> Self {
        Self::new()
    }
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
        self.pending_inputs.push_back(input);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Basit deterministik fizik: pozisyonu girdiye göre ilerletir.
    fn step(state: &PredictedState, input: &PlayerInput) -> PredictedState {
        PredictedState {
            position: [
                state.position[0] + input.move_x * input.dt,
                state.position[1],
                state.position[2] + input.move_z * input.dt,
            ],
            velocity: state.velocity,
        }
    }

    #[test]
    fn add_input_assigns_increasing_ticks() {
        let mut p = ClientPredictor::new();
        let a = p.add_input(1.0, 0.0, false, 1.0);
        let b = p.add_input(1.0, 0.0, false, 1.0);
        assert_eq!(a.tick, 0);
        assert_eq!(b.tick, 1);
        assert_eq!(p.current_tick, 2);
        assert_eq!(p.pending_inputs.len(), 2);
    }

    #[test]
    fn reconcile_acks_processed_inputs_and_replays_rest() {
        let mut p = ClientPredictor::new();
        // tick 0,1,2 — her biri +1.0 x ekler
        p.add_input(1.0, 0.0, false, 1.0); // tick 0
        p.add_input(1.0, 0.0, false, 1.0); // tick 1
        p.add_input(1.0, 0.0, false, 1.0); // tick 2

        // Sunucu tick 0'a kadar işledi ve x=10.0 otoriter durumunu bildirdi.
        let server_state = PredictedState { position: [10.0, 0.0, 0.0], velocity: [0.0; 3] };
        let corrected = p.reconcile(0, server_state, step);

        // tick 1 ve 2 hâlâ onaysız → kuyrukta kalmalı ve replay edilmeli.
        assert_eq!(p.pending_inputs.len(), 2);
        assert_eq!(p.pending_inputs.front().unwrap().tick, 1);
        // 10.0 + 1.0 (tick1) + 1.0 (tick2) = 12.0
        assert_eq!(corrected.position[0], 12.0);
    }

    #[test]
    fn reconcile_clears_queue_when_all_acked() {
        let mut p = ClientPredictor::new();
        p.add_input(1.0, 0.0, false, 1.0); // tick 0
        p.add_input(1.0, 0.0, false, 1.0); // tick 1

        let server_state = PredictedState { position: [5.0, 0.0, 0.0], velocity: [0.0; 3] };
        // Sunucu son girdiyi (tick 1) işledi → tüm kuyruk onaylı.
        let corrected = p.reconcile(1, server_state, step);

        assert!(p.pending_inputs.is_empty());
        assert_eq!(corrected.position[0], 5.0); // replay yok
    }
}
