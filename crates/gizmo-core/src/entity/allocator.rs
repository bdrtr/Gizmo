use super::Entity;
use std::collections::{HashSet, VecDeque};

/// Raw state of the entity id allocator: the id high-water mark, the per-slot generation
/// counters and the free list.
///
/// Exposed so callers can lock once and scan every slot instead of paying a lock per
/// entity. Every field is public and every field is load-bearing — writing one without
/// maintaining the others corrupts liveness checks for the whole world.
///
/// Invariants:
/// - `generations.len() == next_entity_id` — exactly one generation counter per id ever
///   handed out, indexed by the id itself.
/// - `free_set` holds exactly the ids in `free_ids`; the set is what keeps `free_ids`
///   duplicate-free.
/// - An id is currently occupied iff `id < next_entity_id && !free_set.contains(&id)`.
#[derive(Default)]
pub struct EntityAllocatorState {
    /// The next id that has never been handed out. Ids are dense from 0, so this equals
    /// `generations.len()` and only ever grows (except through [`Entities::clear`]).
    ///
    /// It is a high-water mark, not a live count: subtract `free_ids.len()` for that.
    pub next_entity_id: u32,

    /// Generation counter per id slot, indexed by [`Entity::id`]. A handle is live exactly
    /// when its generation equals `generations[id]`, so bumping an entry kills every
    /// outstanding handle to that slot at once.
    ///
    /// For an id currently sitting in `free_ids` the counter has *already* been bumped —
    /// it is the generation the slot's next occupant will be born with, not the one its
    /// last occupant had.
    pub generations: Vec<u32>,

    /// Ids available for reuse, oldest freed first: [`Entities::free`] pushes to the back,
    /// [`Entities::reserve_entity`] pops from the front.
    ///
    /// The FIFO order is what keeps id assignment a pure function of the reserve/free call
    /// sequence, so the same sequence of operations yields the same ids on every run. It
    /// buys nothing for handle safety — [`Entities::free`] bumps the slot's generation
    /// before queueing the id, so handles into that slot are already dead however long the
    /// id then waits here.
    pub free_ids: VecDeque<u32>,

    /// Membership mirror of `free_ids`, so "is this id already queued for reuse?" is O(1)
    /// instead of a scan of the whole free list.
    ///
    /// Never iterated — it is only probed, inserted into and removed from, alongside the
    /// matching push/pop on `free_ids` — so the `HashSet`'s nondeterministic ordering
    /// cannot leak into anything the simulation observes. Reuse order comes from
    /// [`free_ids`](Self::free_ids) alone.
    pub free_set: HashSet<u32>,
}

/// The world's entity id allocator: hands out [`Entity`] handles and recycles ids behind
/// generation counters, so a handle that outlives its entity can be detected rather than
/// silently addressing the id's next occupant.
///
/// Every method takes `&self` and locks the inner mutex — that is what allows an entity id
/// to be reserved from a system that only holds a shared reference to the world. The lock
/// covers the whole allocator, so concurrent reserve/free is correct but serialises.
///
/// Holds nothing but ids and generations: no components, no archetype rows. Freeing an id
/// here retires the handle only; tearing down the entity's data is the world's job.
#[derive(Default)]
pub struct Entities {
    /// The allocator state behind its lock. Public for bulk inspection (scanning all slots
    /// under one lock is far cheaper than locking per entity).
    ///
    /// The mutex is not reentrant: do not call another [`Entities`] method — or any world
    /// operation that reaches the allocator, such as spawn or despawn — while holding this
    /// guard, or the thread deadlocks against itself.
    ///
    /// Every method in this type recovers from a poisoned lock via `into_inner` instead of
    /// panicking, so a panic elsewhere in the process does not make the allocator
    /// permanently unusable.
    pub state: std::sync::Mutex<EntityAllocatorState>,
}

impl Entities {
    /// An empty allocator: nothing handed out, nothing free. The first
    /// [`reserve_entity`](Self::reserve_entity) therefore returns id 0, generation 0.
    ///
    /// Equivalent to `Entities::default()`.
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

    /// Resets the allocator to its just-constructed state: id counter back to 0, all
    /// generation counters and the free list dropped.
    ///
    /// This is worse than invalidating outstanding handles — it is an ABA hazard. Ids
    /// restart at 0 with generation 0, so a handle captured *before* the clear can match a
    /// completely unrelated entity spawned *after* it, and [`is_alive`](Self::is_alive)
    /// will happily say it is alive. Drop every stored [`Entity`] across a clear.
    ///
    /// Releases no component storage; it is a step in tearing down a whole world, not a way
    /// to despawn entities.
    pub fn clear(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.next_entity_id = 0;
        state.generations.clear();
        state.free_ids.clear();
        state.free_set.clear();
    }

    /// Allocates an entity id and returns a handle for it.
    ///
    /// The free list is drained first (oldest freed id, see
    /// [`free_ids`](EntityAllocatorState::free_ids)) and the id space only grows once it is
    /// empty. A recycled id comes back carrying the generation [`free`](Self::free) bumped
    /// it to, so handles from that slot's previous life stay dead.
    ///
    /// Takes `&self`, so ids can be reserved while systems run. The result is nothing but
    /// an id plus a generation — the entity has no archetype row and no components until
    /// the world flushes the spawn — yet it already counts as alive for
    /// [`is_alive`](Self::is_alive).
    ///
    /// # Panics
    /// When the id space is exhausted (2^32 ids handed out with none free). Wrapping would
    /// hand id 0 back at generation 0 and alias live handles, so exhaustion is reported as
    /// the resource-exhaustion bug it is. The panic happens with the lock held, but no id is
    /// consumed and the poisoned lock is recovered by every later call.
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

    /// Retires `entity`'s id: bumps its slot's generation — which kills every outstanding
    /// handle to that slot, not just this one — and queues the id for reuse.
    ///
    /// Returns `true` if the handle was live and has now been retired; `false` if it was
    /// already stale or its id was never allocated, in which case nothing is modified. A
    /// double free is therefore a no-op, and the id can never appear twice in the free list.
    ///
    /// Frees the id only — no component or archetype state is touched here.
    ///
    /// At `u32::MAX` the generation saturates rather than wrapping. Wrapping would send the
    /// slot back to generation 0 and revive its very oldest handles; saturating instead
    /// freezes the counter, after which handles for that one slot are no longer
    /// distinguishable by generation. Reaching it takes 2^32 despawns of a single id.
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

    /// Whether `entity`'s generation still matches its id slot — i.e. the handle has not
    /// been outlived by a [`free`](Self::free).
    ///
    /// This is a generation match, *not* a free-list check. Two consequences: an id that
    /// was reserved but whose spawn has not been flushed yet reports `true`, and a
    /// fabricated handle whose generation happens to equal a currently-free slot's counter
    /// also reports `true` even though nothing occupies that slot. Consult
    /// [`free_set`](EntityAllocatorState::free_set) as well when the question is really
    /// "is there an entity here?".
    ///
    /// Ids beyond the allocated range — including [`Entity::INVALID`] — return `false`.
    ///
    /// `#[inline]` but not cheap: it takes the allocator lock. Checking liveness per entity
    /// inside a hot loop contends on a single mutex; lock [`state`](Self::state) once and
    /// compare against `generations` instead.
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
