use super::input_buffer::{InputBuffer, PlayerInput};
use super::snapshot::{PhysicsStateSnapshot, RollbackBuffer};
use gizmo_core::World;

/// Oyundaki tüm ağ trafiği, tahminler ve rollback süreçlerini yöneten ana sistem.
#[derive(Debug, Clone)]
pub struct RollbackManager {
    /// Tick currently being simulated.
    pub current_tick: u64,
    /// Highest tick reached so far.
    pub latest_tick: u64,
    /// Ring buffer of past physics snapshots used to roll back.
    pub state_buffer: RollbackBuffer,
    /// Per-player input buffers, keyed by player id.
    pub input_buffers: std::collections::HashMap<u32, InputBuffer>,

    // Geçmişte yanlış tahmin edilen ve düzeltilmesi gereken en eski tick
    /// Oldest tick whose prediction diverged and must be re-simulated, if any.
    pub rollback_target_tick: Option<u64>,
}

impl RollbackManager {
    /// Creates a manager whose snapshot history holds `capacity` ticks.
    pub fn new(capacity: usize) -> Self {
        tracing::info!(snapshot_capacity = capacity, "RollbackManager oluşturuldu");
        Self {
            current_tick: 0,
            latest_tick: 0,
            state_buffer: RollbackBuffer::new(capacity),
            input_buffers: std::collections::HashMap::new(),
            rollback_target_tick: None,
        }
    }

    /// Registers a player and allocates an input buffer of `buffer_capacity` ticks for them.
    pub fn register_player(&mut self, player_id: u32, buffer_capacity: usize) {
        self.input_buffers.insert(player_id, InputBuffer::new(player_id, buffer_capacity));
        tracing::info!(
            player_id,
            buffer_capacity,
            player_count = self.input_buffers.len(),
            "Rollback oyuncusu kaydedildi"
        );
    }

    /// Uzak sunucudan/oyuncudan gelen girdiyi kabul eder
    pub fn receive_remote_input(&mut self, player_id: u32, input: PlayerInput) {
        if let Some(buffer) = self.input_buffers.get_mut(&player_id) {
            let past_predicted = buffer.get_or_predict(input.tick);
            
            buffer.insert(input);

            // Eğer daha önceden (bu tick için) tahmin ettiğimiz girdi ile, 
            // az önce uzaktan gelen GERÇEK girdi farklıysa ROLLBACK tetiklenir!
            let prediction_diverged = past_predicted.buttons != input.buttons
                || past_predicted.joystick_x != input.joystick_x
                || past_predicted.joystick_y != input.joystick_y;
            if prediction_diverged && input.tick <= self.current_tick {
                let min_target = match self.rollback_target_tick {
                    Some(target) => std::cmp::min(target, input.tick),
                    None => input.tick,
                };
                self.rollback_target_tick = Some(min_target);
                tracing::warn!(
                    player_id,
                    tick = input.tick,
                    current_tick = self.current_tick,
                    rollback_target = min_target,
                    "Tahmin uyumsuzluğu (misprediction): rollback hedefi güncellendi"
                );
            }
        } else {
            // Kaydı olmayan bir oyuncudan girdi gelmesi bir protokol/eşleşme anomalisidir;
            // davranışı korumak için yok sayıyoruz ama sessizce yutmuyoruz.
            tracing::warn!(
                player_id,
                tick = input.tick,
                "Kayıtlı olmayan oyuncudan girdi alındı, yok sayılıyor"
            );
        }
    }

    /// Fizik döngüsünden ÖNCE çağrılır. Gerekirse geçmişe döner.
    /// Geriye dönülürse true döner, böylece oyun motoru mevcut current_tick'e 
    /// tekrar ulaşana kadar "sessizce" (render olmadan) fiziği simüle eder.
    #[tracing::instrument(skip_all, name = "rollback_begin_frame")]
    pub fn begin_frame(&mut self, world: &mut World) -> bool {
        if let Some(target_tick) = self.rollback_target_tick {
            if let Some(past_state) = self.state_buffer.get(target_tick) {
                // Motorun bu rollback'ten sonra current_tick'e tekrar ulaşmak için
                // kaç kareyi sessizce yeniden simüle edeceği.
                let resim_frames = self.current_tick.saturating_sub(target_tick);
                // Zaman Makinesi: Evreni (World) geçmişteki haline geri yükle
                past_state.restore(world);

                // Engine tick'i geçmişe çek
                self.current_tick = target_tick;
                self.rollback_target_tick = None; // Hedefe ulaşıldı
                tracing::debug!(
                    target_tick,
                    resim_frames,
                    latest_tick = self.latest_tick,
                    "Rollback: dünya geçmiş tick'e geri yüklendi, yeniden simülasyon başlıyor"
                );
                return true; // Rollback gerçekleşti
            } else {
                // Buffer'da o kadar eski bir kare yoksa (Çok yüksek ping/paket kaybı)
                // Senkronizasyon kalıcı olarak kopmuş olabilir (Desync).
                // Gerçek oyunda burada sunucudan Full State Update istenir.
                tracing::error!(
                    target_tick,
                    latest_tick = self.latest_tick,
                    "Rollback desync: hedef snapshot tamponda yok (çok eski / pencere aşımı); FullState gerekli"
                );
                self.rollback_target_tick = None;
            }
        }
        false
    }

    /// Fizik döngüsünün tam SONUNDA çağrılır. O anki dünyanın anlık kopyasını alır.
    #[tracing::instrument(skip_all, name = "rollback_end_frame")]
    pub fn end_frame(&mut self, world: &World) {
        let snapshot = PhysicsStateSnapshot::capture(world, self.current_tick);
        self.state_buffer.save(snapshot);
        self.current_tick += 1;
        if self.current_tick > self.latest_tick {
            self.latest_tick = self.current_tick;
        }
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)] // testlerde Default sonrası alan atama okunabilirlik için
mod tests {
    use super::*;
    use crate::rollback::input_buffer::PlayerInput;
    use gizmo_core::World;
    use gizmo_physics_core::components::transform::Transform;
    use gizmo_physics_rigid::components::velocity::Velocity;
    use gizmo_math::Vec3;

    #[test]
    fn test_rollback_prediction_and_restore() {
        let mut world = World::new();
        let ent = world.spawn();
        
        let mut t = Transform::default();
        t.position = Vec3::new(10.0, 0.0, 0.0);
        world.add_component(ent, t);

        let mut v = Velocity::default();
        v.linear = Vec3::new(1.0, 0.0, 0.0);
        world.add_component(ent, v);

        let mut manager = RollbackManager::new(60);
        manager.register_player(1, 60);

        // Frame 0: Başlangıç
        manager.end_frame(&world); // tick 0 kaydedildi, current_tick 1 oldu

        // Frame 1: Objeyi hareket ettir (Simülasyon adımı)
        {
            let mut transforms = world.borrow_mut::<Transform>();
            if let Some(mut trans) = transforms.get_mut(ent.id()) {
                trans.position.x += 1.0; // pozisyon 11.0 oldu
            }
        }
        manager.end_frame(&world); // tick 1 kaydedildi, current_tick 2 oldu

        // Şimdi uzak oyuncu 1'den gecikmeli bir paket geldi, Tick 0 için!
        // Oyuncu butona basmış (buttons = 1)
        let mut remote_input = PlayerInput::empty(0);
        remote_input.buttons = 1;
        manager.receive_remote_input(1, remote_input);

        // Beklenti: manager geçmişte bir yanlış tahmin fark ettiğinden rollback tetiklenmeli
        assert_eq!(manager.rollback_target_tick, Some(0));

        // Engine bir sonraki kareye başlarken manager.begin_frame() çağıracak
        let did_rollback = manager.begin_frame(&mut world);
        assert!(did_rollback);

        // Rollback gerçekleştiği için dünya Tick 0'a dönmüş olmalı!
        // Tick 0'da pozisyon 10.0'dı.
        {
            let transforms = world.borrow::<Transform>();
            let trans = transforms.get(ent.id()).unwrap();
            assert_eq!(trans.position.x, 10.0);
        }

        // Manager'ın saati (current_tick) de 0'a çekilmiş olmalı
        assert_eq!(manager.current_tick, 0);
    }

    // Nötr (0) tahminden sapan bir girdi üretir → rollback tetikleyebilir.
    fn btn(tick: u64, buttons: u32) -> PlayerInput {
        let mut i = PlayerInput::empty(tick);
        i.buttons = buttons;
        i
    }

    #[test]
    fn end_frame_advances_current_and_latest_tick() {
        let world = World::new();
        let mut m = RollbackManager::new(60);
        assert_eq!(m.current_tick, 0);
        m.end_frame(&world);
        m.end_frame(&world);
        assert_eq!(m.current_tick, 2, "end_frame current_tick'i ilerletmeli");
        assert_eq!(m.latest_tick, 2, "latest_tick en yüksek tick'i izlemeli");
    }

    #[test]
    fn begin_frame_without_target_is_noop() {
        let mut world = World::new();
        let mut m = RollbackManager::new(60);
        assert!(!m.begin_frame(&mut world), "rollback hedefi yokken false dönmeli");
    }

    // Kayıtlı olmayan oyuncudan gelen girdi sessizce yoksayılmalı (panik/rollback yok).
    #[test]
    fn remote_input_for_unregistered_player_is_ignored() {
        let mut m = RollbackManager::new(60);
        m.receive_remote_input(42, btn(0, 1));
        assert_eq!(m.rollback_target_tick, None);
    }

    // Gelen girdi tahminle AYNIysa (nötr↔nötr) rollback tetiklenmemeli.
    #[test]
    fn matching_prediction_does_not_trigger_rollback() {
        let mut m = RollbackManager::new(60);
        m.register_player(1, 60);
        m.receive_remote_input(1, PlayerInput::empty(0)); // nötr = tahmin
        assert_eq!(m.rollback_target_tick, None);
    }

    // Henüz simüle edilmemiş (gelecek) bir tick için sapma bile rollback tetiklememeli
    // (input.tick <= current_tick koşulu).
    #[test]
    fn future_input_does_not_trigger_rollback() {
        let mut m = RollbackManager::new(60);
        m.register_player(1, 60);
        m.receive_remote_input(1, btn(5, 1)); // current_tick=0, tick 5 gelecek
        assert_eq!(m.rollback_target_tick, None, "gelecek tick rollback tetiklememeli");
    }

    // Birden çok sapmada rollback hedefi en ESKİ (minimum) sapan tick olmalı; daha yeni
    // bir sapma hedefi ileri ÇEKMEMELİ.
    #[test]
    fn rollback_target_is_the_minimum_of_diverged_ticks() {
        let mut m = RollbackManager::new(60);
        m.register_player(1, 60);
        m.current_tick = 10; // sapan tickler geçmişte kalsın
        m.receive_remote_input(1, btn(5, 1)); // tahmin nötr(0) → sapar
        assert_eq!(m.rollback_target_tick, Some(5));
        m.receive_remote_input(1, btn(3, 2)); // tahmin son onaylanan(1) → 2≠1 sapar
        assert_eq!(m.rollback_target_tick, Some(3), "hedef en eski sapan tick olmalı");
        m.receive_remote_input(1, btn(8, 4)); // daha yeni sapma
        assert_eq!(m.rollback_target_tick, Some(3), "daha yeni sapma hedefi ileri çekmemeli");
    }

    // Hedef snapshot tamponda yoksa (çok eski / pencere aşımı) begin_frame PANİK ETMEMELİ:
    // desync yolu → false döner ve hedef temizlenir.
    #[test]
    fn begin_frame_target_missing_from_buffer_is_desync_not_panic() {
        let mut world = World::new();
        let mut m = RollbackManager::new(4);
        m.rollback_target_tick = Some(999);
        assert!(!m.begin_frame(&mut world), "eksik snapshot'ta false dönmeli");
        assert_eq!(m.rollback_target_tick, None, "desync sonrası hedef temizlenmeli");
    }
}
