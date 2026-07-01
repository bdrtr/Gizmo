use super::Entity;
use std::collections::{HashSet, VecDeque};

#[derive(Default)]
pub struct EntityAllocatorState {
    pub next_entity_id: u32,
    pub generations: Vec<u32>,
    pub free_ids: VecDeque<u32>,
    pub free_set: HashSet<u32>,
}

#[derive(Default)]
pub struct Entities {
    pub state: std::sync::Mutex<EntityAllocatorState>,
}

impl Entities {
    pub fn new() -> Self {
        Self {
            state: std::sync::Mutex::new(EntityAllocatorState {
                next_entity_id: 0,
                generations: Vec::new(),
                free_ids: VecDeque::new(),
                free_set: HashSet::new(),
            }),
        }
    }

    pub fn clear(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.next_entity_id = 0;
        state.generations.clear();
        state.free_ids.clear();
        state.free_set.clear();
    }

    pub fn reserve_entity(&self) -> Entity {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(id) = state.free_ids.pop_front() {
            state.free_set.remove(&id);
            let gen = state.generations[id as usize];
            Entity::new(id, gen)
        } else {
            let id = state.next_entity_id;
            // Taşma kontrolü: u32::MAX'a ulaşıldığında sarma (id yeniden kullanımı +
            // generation=0 çakışması) yerine net bir panik ver. 2^32 entity gerçekçi
            // olmayan bir ölçek olduğundan bu bir programlama/kaynak-tükenmesi hatasıdır.
            state.next_entity_id = id
                .checked_add(1)
                .expect("EntityAllocator: entity ID alanı tükendi (u32::MAX)");
            state.generations.push(0);
            Entity::new(id, 0)
        }
    }

    pub fn free(&self, entity: Entity) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = entity.id();
        let id_us = id as usize;
        if id_us < state.generations.len() && state.generations[id_us] == entity.generation() {
            // saturating_add: generation u32::MAX'a ulaşırsa sarma yerine doygunlaşır.
            // Sarma olsaydı eski (id, generation=0) handle'ları tekrar geçerli görünüp
            // ABA-tipi bir çakışmaya yol açabilirdi; doygunlaşma bu ID'yi güvenli şekilde
            // "kalıcı ölü" durumda tutar (u32::MAX generation bir daha eşleşmez varsayımı).
            state.generations[id_us] = state.generations[id_us].saturating_add(1);
            if state.free_set.insert(id) {
                state.free_ids.push_back(id);
            }
            return true; // Successfully freed
        }
        false
    }

    #[inline]
    pub fn is_alive(&self, entity: Entity) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = entity.id() as usize;
        id < state.generations.len() && state.generations[id] == entity.generation()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_entity_panics_on_id_exhaustion() {
        let entities = Entities::new();
        // ID sayacını doğrudan tükenmiş sınıra ayarla. free listesi boş olduğundan
        // reserve_entity() checked_add(1) yoluna girer ve generations.push'tan ÖNCE panik atar,
        // bu yüzden dev bir generations vektörü tahsis etmeye gerek yoktur.
        {
            let mut state = entities.state.lock().unwrap();
            state.next_entity_id = u32::MAX;
        }
        // free listesi boş olduğundan next_entity_id yolu çalışır ve
        // checked_add(1) taştığı için panik beklenir.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            entities.reserve_entity();
        }));
        assert!(result.is_err(), "ID tükendiğinde sarma yerine panik bekleniyordu");
    }

    #[test]
    fn generation_saturates_instead_of_wrapping() {
        let entities = Entities::new();
        let e = entities.reserve_entity();
        let id = e.id();
        // generation'ı u32::MAX'a manuel getir ve free çağrısının sarmadığını doğrula.
        {
            let mut state = entities.state.lock().unwrap();
            state.generations[id as usize] = u32::MAX;
        }
        // u32::MAX generation'lı bir handle uydur; free onu doygunlaştırmalı, 0'a sarmamalı.
        let stale = Entity::new(id, u32::MAX);
        assert!(entities.free(stale));
        let state = entities.state.lock().unwrap();
        assert_eq!(
            state.generations[id as usize],
            u32::MAX,
            "generation doygunlaşmalıydı, 0'a sarmamalıydı"
        );
    }
}
