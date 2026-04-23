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

    pub fn reserve_entity(&self) -> Entity {
        let mut state = self.state.lock().unwrap();
        if let Some(id) = state.free_ids.pop_front() {
            state.free_set.remove(&id);
            let gen = state.generations[id as usize];
            Entity::new(id, gen)
        } else {
            let id = state.next_entity_id;
            state.next_entity_id += 1;
            state.generations.push(0);
            Entity::new(id, 0)
        }
    }

    pub fn free(&self, entity: Entity) -> bool {
        let mut state = self.state.lock().unwrap();
        let id = entity.id();
        let id_us = id as usize;
        if id_us < state.generations.len() && state.generations[id_us] == entity.generation() {
            state.generations[id_us] += 1;
            if state.free_set.insert(id) {
                state.free_ids.push_back(id);
            }
            return true; // Successfully freed
        }
        false
    }

    #[inline]
    pub fn is_alive(&self, entity: Entity) -> bool {
        let state = self.state.lock().unwrap();
        let id = entity.id() as usize;
        id < state.generations.len() && state.generations[id] == entity.generation()
    }
}
