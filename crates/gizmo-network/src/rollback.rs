use crate::input_buffer::{InputBuffer, PlayerInput};
use crate::snapshot::{PhysicsStateSnapshot, RollbackBuffer};
use gizmo_core::World;

/// Oyundaki tüm ağ trafiği, tahminler ve rollback süreçlerini yöneten ana sistem.
pub struct RollbackManager {
    pub current_tick: u64,
    pub latest_tick: u64,
    pub state_buffer: RollbackBuffer,
    pub input_buffers: std::collections::HashMap<u32, InputBuffer>,
    
    // Geçmişte yanlış tahmin edilen ve düzeltilmesi gereken en eski tick
    pub rollback_target_tick: Option<u64>,
}

impl RollbackManager {
    pub fn new(capacity: usize) -> Self {
        Self {
            current_tick: 0,
            latest_tick: 0,
            state_buffer: RollbackBuffer::new(capacity),
            input_buffers: std::collections::HashMap::new(),
            rollback_target_tick: None,
        }
    }

    pub fn register_player(&mut self, player_id: u32, buffer_capacity: usize) {
        self.input_buffers.insert(player_id, InputBuffer::new(player_id, buffer_capacity));
    }

    /// Uzak sunucudan/oyuncudan gelen girdiyi kabul eder
    pub fn receive_remote_input(&mut self, player_id: u32, input: PlayerInput) {
        if let Some(buffer) = self.input_buffers.get_mut(&player_id) {
            let past_predicted = buffer.get_or_predict(input.tick);
            
            buffer.insert(input);

            // Eğer daha önceden (bu tick için) tahmin ettiğimiz girdi ile, 
            // az önce uzaktan gelen GERÇEK girdi farklıysa ROLLBACK tetiklenir!
            if past_predicted.buttons != input.buttons 
                || past_predicted.joystick_x != input.joystick_x 
                || past_predicted.joystick_y != input.joystick_y 
            {
                if input.tick <= self.current_tick {
                    let min_target = match self.rollback_target_tick {
                        Some(target) => std::cmp::min(target, input.tick),
                        None => input.tick,
                    };
                    self.rollback_target_tick = Some(min_target);
                }
            }
        }
    }

    /// Fizik döngüsünden ÖNCE çağrılır. Gerekirse geçmişe döner.
    /// Geriye dönülürse true döner, böylece oyun motoru mevcut current_tick'e 
    /// tekrar ulaşana kadar "sessizce" (render olmadan) fiziği simüle eder.
    pub fn begin_frame(&mut self, world: &mut World) -> bool {
        if let Some(target_tick) = self.rollback_target_tick {
            if let Some(past_state) = self.state_buffer.get(target_tick) {
                // Zaman Makinesi: Evreni (World) geçmişteki haline geri yükle
                past_state.restore(world);
                
                // Engine tick'i geçmişe çek
                self.current_tick = target_tick;
                self.rollback_target_tick = None; // Hedefe ulaşıldı
                return true; // Rollback gerçekleşti
            } else {
                // Buffer'da o kadar eski bir kare yoksa (Çok yüksek ping/paket kaybı)
                // Senkronizasyon kalıcı olarak kopmuş olabilir (Desync).
                // Gerçek oyunda burada sunucudan Full State Update istenir.
                tracing::error!("Rollback desync! Frame {} is lost in buffer.", target_tick);
                self.rollback_target_tick = None;
            }
        }
        false
    }

    /// Fizik döngüsünün tam SONUNDA çağrılır. O anki dünyanın anlık kopyasını alır.
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
mod tests {
    use super::*;
    use crate::input_buffer::PlayerInput;
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
            let transforms = world.borrow_mut::<Transform>();
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
}
