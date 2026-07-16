//! Client-Side Prediction & Server Reconciliation
//!
//! İstemcinin (Client) kendi hareketlerini sunucuyu beklemeden anında uygulaması (Prediction)
//! ve sunucudan gelen kesin (Authoritative) sonuçlara göre gerekirse geçmişe dönüp düzeltmesi (Reconciliation)

use super::protocol::{tick_is_newer, PlayerInput};
use std::collections::VecDeque;

/// İstemcinin öngördüğü yerel durum (Fizik motoru üzerinde anlık uygulanacak)
#[derive(Debug, Clone)]
pub struct PredictedState {
    /// Predicted world-space position `[x, y, z]`.
    pub position: [f32; 3],
    /// Predicted linear velocity `[x, y, z]`.
    pub velocity: [f32; 3],
}

/// Tracks unacknowledged inputs so the client can reconcile against authoritative server state.
#[derive(Debug, Clone)]
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
    /// Creates an empty predictor starting at tick 0.
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
    #[tracing::instrument(skip_all, name = "client_reconcile")]
    pub fn reconcile<F>(
        &mut self,
        server_tick: u32,
        server_state: PredictedState,
        mut apply_physics_fn: F,
    ) -> PredictedState
    where
        F: FnMut(&PredictedState, &PlayerInput) -> PredictedState,
    {
        // 1. Sunucunun onayladığı girdileri kuyruktan sil (yalnız server_tick'ten
        //    KESİN olarak daha yeni olanlar kalır — wraparound-güvenli, `tick_is_newer`).
        self.pending_inputs
            .retain(|input| tick_is_newer(input.tick, server_tick));

        // 2. Onaylanmış (Authoritative) durumu al
        let mut corrected_state = server_state;

        // 3. Kalan onaylanmamış girdileri sırayla tekrar uygula (Replay)
        for input in &self.pending_inputs {
            corrected_state = apply_physics_fn(&corrected_state, input);
        }

        // Kaç onaylanmamış girdinin yeniden simüle edildiği (resimülasyon kare sayısı).
        tracing::debug!(
            server_tick,
            replayed = self.pending_inputs.len(),
            current_tick = self.current_tick,
            "Reconciliation: onaylanmamış girdiler yeniden oynatıldı"
        );
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

    // add_input, current_tick'i wrapping_add ile ilerletir: u32::MAX'ta 0'a sarmalı
    // (panik/overflow YOK) ve üretilen girdi taşmadan ÖNCEki tick'i taşımalı.
    #[test]
    fn add_input_wraps_current_tick_at_u32_max() {
        let mut p = ClientPredictor::new();
        p.current_tick = u32::MAX;
        let a = p.add_input(0.0, 0.0, false, 1.0);
        assert_eq!(a.tick, u32::MAX, "girdi taşmadan önceki tick'i taşımalı");
        assert_eq!(p.current_tick, 0, "current_tick 0'a sarmalı");
    }

    // Kuyruk boşken reconcile hiç replay yapmadan otoriter durumu aynen döndürmeli.
    #[test]
    fn reconcile_with_empty_queue_returns_server_state_untouched() {
        let mut p = ClientPredictor::new();
        let server = PredictedState { position: [3.0, 4.0, 5.0], velocity: [1.0, 0.0, 0.0] };
        let corrected = p.reconcile(0, server, |_s, _i| panic!("boş kuyrukta replay olmamalı"));
        assert_eq!(corrected.position, [3.0, 4.0, 5.0]);
        assert_eq!(corrected.velocity, [1.0, 0.0, 0.0]);
        assert!(p.pending_inputs.is_empty());
    }

    // Onaylanmamış girdiler kuyruğa eklenme (FIFO) sırasıyla oynatılmalı. Sıra-duyarlı
    // (komütatif OLMAYAN) fizik ile ters sıra farklı sonuç verir → sıra ayırt edicidir.
    #[test]
    fn reconcile_replays_pending_in_fifo_order() {
        fn scale_step(s: &PredictedState, i: &PlayerInput) -> PredictedState {
            // x' = x*2 + move_x  → uygulama sırası sonucu değiştirir.
            PredictedState {
                position: [s.position[0] * 2.0 + i.move_x, s.position[1], s.position[2]],
                velocity: s.velocity,
            }
        }
        let mut p = ClientPredictor::new();
        p.current_tick = 5;
        p.add_input(1.0, 0.0, false, 1.0); // tick 5, move_x=1
        p.add_input(2.0, 0.0, false, 1.0); // tick 6, move_x=2
        // Sunucu tick 4'e kadar işledi → tick 5,6 onaysız, ikisi de replay edilir.
        // FIFO (5 sonra 6): ((0*2+1)*2+2) = 4.  Ters sıra ((0*2+2)*2+1) = 5 olurdu.
        let server = PredictedState { position: [0.0, 0.0, 0.0], velocity: [0.0; 3] };
        let corrected = p.reconcile(4, server, scale_step);
        assert_eq!(corrected.position[0], 4.0, "pending girdiler FIFO sırayla oynatılmalı");
    }

    // REGRESYON (wraparound-güvenli reconcile): tick uzayı u32::MAX→0 taştıktan sonra
    // sunucu onayı KESİN daha yeni olan girdileri korumalı. Düz `>` ile 0 > (u32::MAX-1)
    // YANLIŞ (false) olur ve sarma-sonrası girdiler hatalı düşerdi (kuyruk sonsuz büyür).
    #[test]
    fn reconcile_keeps_post_wraparound_inputs_a_naive_compare_would_drop() {
        let mut p = ClientPredictor::new();
        p.current_tick = u32::MAX - 1;
        p.add_input(1.0, 0.0, false, 1.0); // tick u32::MAX-1
        p.add_input(1.0, 0.0, false, 1.0); // tick u32::MAX
        p.add_input(1.0, 0.0, false, 1.0); // tick 0 (sardı), current_tick → 1
        assert_eq!(p.pending_inputs.len(), 3);

        // Sunucu tick (u32::MAX-1)'i işledi → kalanlar u32::MAX ve 0.
        let server = PredictedState { position: [100.0, 0.0, 0.0], velocity: [0.0; 3] };
        let corrected = p.reconcile(u32::MAX - 1, server, step);

        assert_eq!(p.pending_inputs.len(), 2, "sarma sonrası girdiler korunmalı");
        assert_eq!(p.pending_inputs.front().unwrap().tick, u32::MAX);
        assert_eq!(p.pending_inputs.back().unwrap().tick, 0);
        // 100 + 1 (tick u32::MAX) + 1 (tick 0) = 102.
        assert_eq!(corrected.position[0], 102.0);
    }
}
