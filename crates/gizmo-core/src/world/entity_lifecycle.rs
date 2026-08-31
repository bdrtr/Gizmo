use super::{Entities, World};
use crate::archetype::EntityLocation;
use crate::entity::Entity;

use std::any::TypeId;

impl World {
    /// Allocates an entity id and materialises it immediately, so the returned handle is
    /// usable for `add_component` on the very next line.
    ///
    /// The new entity has no components and lives in the empty archetype. Ids of despawned
    /// entities are recycled FIFO — the id freed longest ago comes back first — and each
    /// recycle bumps the generation, so old handles to that id stop being
    /// [`World::is_alive`].
    ///
    /// # Panics
    /// If the [`Entities`] resource has been removed from the world, or if the id space
    /// (`u32::MAX` ids ever allocated) is exhausted.
    pub fn spawn(&mut self) -> Entity {
        let entity = {
            let entities = self
                .get_resource::<Entities>()
                .expect("Entities resource not initialized");
            entities.reserve_entity()
        };

        self.flush_spawn(entity);
        entity
    }

    /// Spawns a `Bundle` in one go — creates the entity and adds all of its
    /// components.
    ///
    /// ```
    /// # use gizmo_core::prelude::*;
    /// # #[derive(Clone)] struct Health(u32);
    /// # gizmo_core::impl_component!(Health);
    /// # let mut world = World::new();
    /// // Any `Bundle` will do: a tuple of components, or a named bundle struct from the
    /// // layers above, such as `MeshBundle`.
    /// let player = world.spawn_bundle((EntityName("Player".to_string()), Health(100)));
    ///
    /// assert_eq!(
    ///     world.query::<&EntityName>().unwrap().get(player.id()).unwrap().0.as_str(),
    ///     "Player"
    /// );
    /// assert_eq!(world.query::<&Health>().unwrap().get(player.id()).unwrap().0, 100);
    /// ```
    pub fn spawn_bundle<B: crate::component::Bundle>(&mut self, bundle: B) -> Entity {
        let entity = self.spawn();
        bundle.apply(self, entity);
        entity
    }

    /// Gives storage to an id that was already reserved from [`Entities`]: appends it as a
    /// new row of the empty archetype and records its `EntityLocation`. This is the second
    /// half of [`World::spawn`], exposed so a deferred spawn can hand out the id from
    /// `&World` and commit the storage later under `&mut World`.
    ///
    /// Call it exactly once per reserved id. A second call for the same id pushes another row
    /// into the empty archetype and overwrites the recorded location, orphaning the first —
    /// that is still the caller's to avoid, and it is a `debug_assert` here rather than a
    /// silent corruption.
    ///
    /// **A flush for an id that is no longer alive is a no-op, since 2026-08-31.** That is not
    /// a contract the caller can be asked to keep: `Commands::spawn` hands the handle out the
    /// moment it reserves the id, and despawning it before the queue is applied is ordinary
    /// use. The queued flush then ran anyway and appended a row for a dead id — and because
    /// `despawn` returned that id to the allocator, the very next `spawn` took it back and
    /// appended a SECOND row, leaving the empty archetype listing one id twice with only the
    /// later row recorded. Measured before the fix: `entity 0 at archetype 0 row 0, location
    /// says archetype 0 row 1`.
    ///
    /// Anything reachable from an entity in that state reads or writes the wrong row, and it is
    /// also the reason `World::compact` cannot yet truncate `entity_locations`: an entity
    /// listed in an archetype with no matching location is exactly what truncation must not
    /// find. See `docs/ENGINE.md` §3.
    pub fn flush_spawn(&mut self, entity: Entity) {
        if !self.is_alive(entity) {
            tracing::debug!(
                entity = entity.id(),
                "flush_spawn: id was freed between its reservation and this flush; skipped"
            );
            return;
        }
        debug_assert!(
            !self.entity_location(entity.id()).is_valid(),
            "flush_spawn called twice for entity {}: the first row it was given is now \
             orphaned, listed in the archetype with nothing pointing at it",
            entity.id()
        );

        // Yeni entity'yi boş archetype'a kaydet
        self.archetype_index.on_spawn(entity.id());

        // Entity location tracking — boş archetype (id=0), row = entity'nin sırası
        let eid = entity.id();
        let loc_idx = eid as usize;
        let row = self.archetype_index.archetypes[0].len() as u32 - 1;

        if loc_idx >= self.entity_locations.len() {
            self.entity_locations
                .resize(loc_idx + 1, EntityLocation::INVALID);
        }
        self.entity_locations[loc_idx] = EntityLocation {
            archetype_id: 0,
            row,
        };
        tracing::trace!(entity = eid, row, "spawn: entity placed in empty archetype");
    }

    // Eski A3 bridge ve rebuild metodları silindi (Archetype artık authoritative).

    /// Rebuilds a handle for a raw id from the allocator alone: `Some` carrying the id's
    /// *current* generation whenever the id has ever been allocated and is not on the free
    /// list, `None` otherwise.
    ///
    /// Weaker than [`World::entity`], which additionally demands live archetype storage: an
    /// id reserved through `Commands::spawn` but not yet flushed is `Some` here and `None`
    /// there. Neither can recover the generation an old handle had — a recycled id resolves
    /// to its new occupant.
    ///
    /// # Panics
    /// If the [`Entities`] resource has been removed from the world.
    pub fn get_entity(&self, id: u32) -> Option<Entity> {
        let entities = self
            .get_resource::<Entities>()
            .expect("Entities resource not initialized");
        let state = entities.state.lock().unwrap_or_else(|e| e.into_inner());
        if (id as usize) < state.generations.len() && !state.free_set.contains(&id) {
            return Some(Entity::new(id, state.generations[id as usize]));
        }
        None
    }

    /// The deep-copy (O(1) Prefab Splicing) operation.
    /// Produces N new copies of an existing Entity, fully contiguous, in the archetype table it is in.
    #[tracing::instrument(skip_all, name = "clone_entity")]
    pub fn clone_entity(&mut self, source_id: u32, count: usize) -> Option<Vec<Entity>> {
        if count == 0 {
            return Some(Vec::new());
        }

        // Kaynak entity'nin geçerli bir konumu yoksa (silinmiş/hiç var olmamış) klonlama
        // sessizce None döndürürdü; artık neden başarısız olduğu loglanıyor.
        let loc = match self.entity_locations.get(source_id as usize).copied() {
            Some(l) if l.is_valid() => l,
            _ => {
                tracing::debug!(
                    source = source_id,
                    "clone_entity: source has no valid location; nothing cloned"
                );
                return None;
            }
        };

        let arch_id = loc.archetype_id as usize;
        let row = loc.row as usize;

        // Kilitlenmeleri engellemek için önce ID'leri üretelim
        let mut new_entities = Vec::with_capacity(count);
        let mut new_eids = Vec::with_capacity(count);

        {
            let entities_res = self
                .get_resource::<Entities>()
                .expect("Entities resource not initialized");
            for _ in 0..count {
                let e = entities_res.reserve_entity();
                new_eids.push(e.id());
                new_entities.push(e);
            }
        }

        // Seçilen Archetype içinde kopyalamayı batch halinde yapıyoruz
        let arch = &mut self.archetype_index.archetypes[arch_id];
        let tick = self.tick;
        // SAFETY: `row` is a live row of this archetype (it is the template entity's own), and
        // `new_eids` has exactly `count` freshly reserved ids — `batch_clone_row`'s contract.
        let new_rows = unsafe { arch.batch_clone_row(row, count, &new_eids, tick) };

        // Location güncellemeleri
        for (i, &id) in new_eids.iter().enumerate() {
            let row = new_rows[i];
            let idx = id as usize;
            if idx >= self.entity_locations.len() {
                self.entity_locations
                    .resize(idx + 1, EntityLocation::INVALID);
            }
            self.entity_locations[idx] = EntityLocation {
                archetype_id: arch_id as u32,
                row,
            };
            self.archetype_index.entity_archetype.insert(id, arch_id);
            // NOT: on_spawn çağırmıyoruz çünkü batch_clone_row zaten entity'yi
            // doğru archetype'a ekledi. on_spawn boş archetype'a (0) tekrar eklerdi.
        }

        // batch_clone_row only cloned archetype (table) columns. Deep-clone the
        // source's SparseSet components into every clone too, otherwise clones
        // silently lack them.
        let sparse_types: Vec<std::any::TypeId> = self.sparse_sets.keys().copied().collect();
        for tid in sparse_types {
            if let Some(set) = self.sparse_sets.get_mut(&tid) {
                if set.contains(source_id) {
                    for &new_id in &new_eids {
                        set.clone_entry(source_id, new_id, tick);
                    }
                }
            }
        }

        tracing::debug!(
            source = source_id,
            count,
            archetype = arch_id,
            "clone_entity: prefab splice complete"
        );
        Some(new_entities)
    }

    /// Spawns one entity per bundle and yields their handles in input order.
    ///
    /// All the work happens eagerly, before the iterator is returned — dropping the result
    /// unconsumed still leaves every entity spawned. An empty input spawns nothing.
    ///
    /// Since every bundle has the same Rust type they all land in one archetype: the first
    /// is spawned normally to discover it and the rest are appended straight into its
    /// columns, which is where the win over a loop of [`World::spawn_bundle`] comes from.
    /// That fast path writes columns directly, so **no `on_add`/`on_set` hooks run for any
    /// entity after the first**.
    ///
    /// A bundle carrying a `SparseSet`-storage component cannot use the fast path (there is
    /// no archetype column to write) and transparently falls back to per-entity
    /// [`World::spawn_bundle`] — correct and hook-firing, but without the batching win.
    #[tracing::instrument(skip_all, name = "spawn_batch")]
    pub fn spawn_batch<I>(&mut self, iter: I) -> impl Iterator<Item = Entity>
    where
        I: IntoIterator,
        I::Item: crate::component::Bundle,
    {
        // Fast path below writes each bundle straight into archetype columns via
        // `write_to_archetype`, which has no column for SparseSet-storage
        // components (they live in `sparse_sets`, not the archetype). If the
        // bundle contains any sparse component, fall back to per-entity
        // `spawn_bundle`, which routes every component through `add_component`
        // (sparse-aware). All-table bundles keep the O(1) archetype-reuse path.
        let has_sparse = <I::Item as crate::component::Bundle>::get_infos()
            .iter()
            .any(|info| info.storage_type == crate::component::StorageType::SparseSet);
        if has_sparse {
            let entities: Vec<Entity> = iter.into_iter().map(|b| self.spawn_bundle(b)).collect();
            tracing::debug!(count = entities.len(), "spawn_batch: sparse fallback (per-entity)");
            return entities.into_iter();
        }

        let mut iter = iter.into_iter();
        let mut entities = Vec::new();

        let first_bundle = match iter.next() {
            Some(b) => b,
            None => return entities.into_iter(),
        };

        let first_entity = self.spawn_bundle(first_bundle);
        entities.push(first_entity);

        // `spawn_bundle` fires `on_add`/`on_set`, and `run_hooks` hands those a `&mut World`
        // that `hooks.rs` documents as free to spawn, despawn and mutate. A hook that despawns
        // the very entity it was handed leaves this batch with nothing to append to: the
        // location reads INVALID, whose `archetype_id` is `u32::MAX`, and the loop below
        // indexed `self.archetype_index.archetypes` with it — "index out of bounds" for any
        // batch of two or more. The read was also raw, so once `compact` learns to truncate
        // the location table it would panic one line earlier instead.
        //
        // Falling back to the per-entity path is the answer this function already gives when
        // it cannot use its fast path (see the sparse-component branch above): correct, and
        // slower only in a case that has already gone strange.
        // The hooks `spawn_bundle` just fired get a `&mut World` and may have done anything to
        // the entity whose archetype this batch is about to reuse. Two shapes matter, and the
        // second is not caught by checking the first:
        //
        //   · it was DESPAWNED. The location reads INVALID, whose `archetype_id` is `u32::MAX`,
        //     and the append loop below indexed `archetypes` with it.
        //   · it was MOVED — an `on_add` that removes the component it was just given migrates
        //     it to a different archetype. That location is perfectly VALID; it just names an
        //     archetype whose columns do not match this bundle, so `write_to_archetype` hit a
        //     missing column and panicked with a message blaming SparseSet storage, which had
        //     nothing to do with it.
        //
        // So the archetype is checked against the bundle rather than merely for existence.
        // Falling back to the per-entity path is what this function already does when it cannot
        // use its fast path, and it is correct for both shapes.
        let loc = self.entity_location(first_entity.id());
        let mut bundle_types: Vec<std::any::TypeId> =
            <I::Item as crate::component::Bundle>::get_infos()
                .iter()
                .map(|info| info.type_id)
                .collect();
        bundle_types.sort();
        let usable = loc.is_valid()
            && self
                .archetype_index
                .archetypes
                .get(loc.archetype_id as usize)
                .is_some_and(|arch| arch.sorted_component_types() == bundle_types);
        if !usable {
            tracing::debug!(
                entity = first_entity.id(),
                "spawn_batch: hooks moved or destroyed the first entity; per-entity fallback"
            );
            for bundle in iter {
                entities.push(self.spawn_bundle(bundle));
            }
            return entities.into_iter();
        }
        let target_arch_id = loc.archetype_id as usize;

        for bundle in iter {
            let entity = {
                let e_res = self.get_resource::<crate::entity::allocator::Entities>().expect("Entities not init");
                e_res.reserve_entity()
            };
            let eid = entity.id();

            let new_row = {
                let arch = &mut self.archetype_index.archetypes[target_arch_id];
                let row = arch.push_entity(eid);
                // SAFETY: the row was just pushed into the archetype chosen for this bundle's
                // component set, so the bundle writes every column of it exactly once.
                unsafe { crate::component::Bundle::write_to_archetype(bundle, arch, row as usize, self.tick); }
                row
            };

            let loc_idx = eid as usize;
            if loc_idx >= self.entity_locations.len() {
                self.entity_locations.resize(loc_idx + 1, crate::archetype::EntityLocation::INVALID);
            }
            self.entity_locations[loc_idx] = crate::archetype::EntityLocation {
                archetype_id: target_arch_id as u32,
                row: new_row,
            };
            self.archetype_index.entity_archetype.insert(eid, target_arch_id);

            entities.push(entity);
        }

        // Değişmez: batch sonunda her sütun uzunluğu entity sayısına eşit olmalı.
        #[cfg(debug_assertions)]
        self.archetype_index.archetypes[target_arch_id].debug_assert_consistent();

        tracing::debug!(
            count = entities.len(),
            archetype = target_arch_id,
            "spawn_batch: entities written into shared archetype"
        );
        entities.into_iter()
    }

    /// Destroys every entity and every component, and resets the id allocator so the next
    /// spawn starts again from `Entity(0, gen 0)`.
    ///
    /// **No hooks run.** Not `on_remove`, not `on_replace`, not the despawn hooks — every other
    /// destruction path in this crate runs them and this one does not. That is deliberate: a
    /// teardown is not a sequence of removals, and a hook firing once per component per entity
    /// on a million-entity clear would cost more than the clear. It is written down here
    /// because it cannot be discovered any other way, and because a game that releases an
    /// external resource from `on_remove` — a physics body, a GPU buffer, an audio voice — will
    /// leak every one of them across a level change unless it releases them itself first.
    ///
    /// **Handles are recycled bit for bit**, not merely by id: the generation counter is reset
    /// along with the id counter, so an `Entity` captured before the call can compare equal to
    /// an unrelated entity spawned after it. That is the opposite of what an ordinary `despawn`
    /// guarantees, where the generation bump is what keeps a stale handle dead forever. Anything
    /// holding entity handles across a clear — a queued `Commands` closure, a selection list, a
    /// cache keyed by `Entity` — is holding live-looking pointers to strangers. The world's own
    /// per-entity state is cleared here for exactly this reason: the archetypes, the location
    /// table, the pending-despawn list, every sparse set, the per-entity observers, and — since
    /// 2026-08-31 — **the deferred [`CommandQueue`](crate::commands::CommandQueue)**, which this
    /// paragraph named as a hazard for six days while leaving it full. Those commands are
    /// *discarded*, not applied: applying would run a queued `despawn` and with it the hooks the
    /// paragraph above promises do not run here. To have them applied, call
    /// [`World::apply_commands`] **before** this, while the entities they name still exist.
    ///
    /// **What is still yours to reset:** an `Events<T>` queue. `CollisionEvent`, `TriggerEvent`
    /// and `HitEvent` all carry `Entity` handles, and a queue holding them across a clear has
    /// exactly the problem above. It cannot be closed here: `Events<T>` is an ordinary resource,
    /// type-erased in the resource map with no registry of which types are event queues, so
    /// there is nothing generic to drain — and unlike `entity_observers`, whose whole map could
    /// go because every value in it was per-entity, the resource map holds `Time`, `Input` and
    /// the world's own machinery. Drain the queues your game registered, or read them before the
    /// clear.
    ///
    /// [`PoolManager`](crate::pool::PoolManager) is the same shape and worse in one way: it is
    /// not a resource at all, so this function could not reach it even if it wanted to, and its
    /// own guard against a stale handle is `World::is_alive` — which a bit-identically recycled
    /// id passes. A pool that survives a clear will hand out entities belonging to the new
    /// scene. Rebuild the pools after a clear.
    pub fn clear_entities(&mut self) {
        self.archetype_index.clear_entities();
        self.entity_locations.clear();
        self.entities_to_despawn.clear();

        // SparseSet components live outside the archetypes, so clearing the archetypes does not
        // touch them. Until 2026-08-24 this line was missing and the omission was invisible in
        // the obvious way — nothing dangles, nothing panics — because a sparse set is keyed by
        // the RAW entity id: `Entities::clear` restarts ids at 0, so the next entity spawned
        // took id 0 back and inherited the sparse components of whoever held id 0 before. It
        // also made a genuine first attach look like an overwrite, since `add_component`'s
        // sparse branch asks the set whether the entity is already in it.
        for set in self.sparse_sets.values_mut() {
            set.clear();
        }

        // The world's OTHER per-entity map, and the same omission one turn sharper.
        // `Entities::clear` resets the GENERATIONS as well as the id counter, so the first
        // entity spawned after this call is `Entity(0, gen 0)` — the same 64 bits as the
        // entity 0 just destroyed, not merely the same id. The generation bump is what makes a
        // stale `Entity` key harmless after an ordinary `despawn`; there is no bump here, so
        // the key matches its unrelated successor exactly and that entity's listeners run for
        // it.
        //
        // The whole map goes, outer keys included. Its values are type-erased
        // `HashMap<Entity, Vec<EntityListener<E>>>` behind `Box<dyn Any>`, so pruning a single
        // entity out of them is not expressible without knowing `E` — and after a clear there
        // is no entity left whose listeners could still be wanted. `global_observers` and the
        // component hooks are keyed by TYPE, not by entity, and stay: they describe the world's
        // rules rather than its population.
        self.entity_observers.clear();

        // The deferred command queue is per-entity state too, and the sharpest case of it. A
        // queued closure captured an `Entity` by value; after this call the ids restart at 0 with
        // generation 0, so that handle is not stale-and-harmless, it is bit-identical to the
        // FIRST entity spawned next. Applying it later lands the command on a stranger:
        // `insert` gives a component nobody asked for, `despawn` kills the wrong entity.
        //
        // Cloning first, then dropping the guard, is what lets this run: `CommandQueue::clear`
        // takes `&self`, but holding the resource guard across the call would keep `self`
        // borrowed. The queue is an `Arc`, so the clone is the same queue.
        let queue = self
            .get_resource::<crate::commands::CommandQueue>()
            .map(|q| (*q).clone());
        let dropped_commands = queue.map_or(0, |q| q.clear());

        // Entities resource'unu temizle (allocator state)
        if let Some(entities) = self.get_resource::<Entities>() {
            entities.clear();
        }
        tracing::debug!(
            dropped_commands,
            "clear_entities: all entities and archetype rows reset"
        );
    }

    /// Destroys an entity: runs the despawn hooks, then the `on_remove` hooks of every
    /// component it holds, drops its component data (archetype columns *and* sparse sets),
    /// and returns the id to the allocator with a bumped generation — which invalidates
    /// every outstanding handle to it.
    ///
    /// Despawning a dead entity is a silent no-op, so double-despawn does not panic; an id
    /// that was reserved but never flushed is freed without touching storage.
    ///
    /// Only this entity goes. Children keep a now-dangling `Parent`; use
    /// `HierarchyExt::despawn_recursive` to take the subtree with it.
    ///
    /// Re-entrant rather than recursive: a `despawn` issued from inside a hook is appended
    /// to a pending list that the outermost call drains, popping from the back (LIFO).
    ///
    /// The row is swap-removed, so the archetype's **last** row moves into the vacated
    /// slot — survivors keep their data but not their relative iteration order.
    ///
    /// Hook timing differs by storage: for Table components `on_remove` runs *before* the
    /// row is dropped (the value is still readable), for `SparseSet` components *after*.
    /// The order across component types comes from a `HashMap` and is arbitrary — do not
    /// depend on it.
    pub fn despawn(&mut self, entity: Entity) {
        self.entities_to_despawn.push(entity);
        if self.is_despawning {
            return;
        }
        self.is_despawning = true;

        while let Some(e) = self.entities_to_despawn.pop() {
            if !self.is_alive(e) {
                continue;
            }

            let mut hooks = std::mem::take(&mut self.despawn_hooks);
            for hook in &mut hooks {
                hook(self, e);
            }
            self.despawn_hooks.extend(hooks);

            let id = e.id();
            // Bounds-safe: an entity RESERVED via `Commands::spawn` (its generation is
            // already in the allocator, so `is_alive` is true) but not yet flushed has
            // NO `entity_locations` slot. A raw index would panic; treat a missing slot
            // as INVALID (mirrors `World::entity`), so we still free the id + clean its
            // sparse sets below without touching non-existent archetype data.
            let loc = self
                .entity_locations
                .get(id as usize)
                .copied()
                .unwrap_or(crate::archetype::EntityLocation::INVALID);

            tracing::trace!(entity = id, archetype = loc.archetype_id, "despawn");

            if loc.is_valid() {
                // Call OnRemove hooks for all currently held components
                let comp_types = {
                    let arch = &self.archetype_index.archetypes[loc.archetype_id as usize];
                    arch.component_types()
                };
                for t in comp_types {
                    self.run_hooks(t, |h, w| {
                        for hook in &mut h.on_remove {
                            hook(w, e);
                        }
                    });
                }

                // Re-fetch the location after the hooks, which `run_hooks` hands a `&mut World`
                // and which are documented as free to spawn, despawn and mutate. This line said
                // "safely" while indexing RAW, which is the one thing that is not safe here: a
                // hook that reaches `clear_entities` empties this vector outright, and the
                // re-fetch then panicked with "index out of bounds" on the entity being
                // despawned. The entry read forty lines up is bounds-checked and carries a
                // comment explaining why; this one — added precisely because state may have
                // changed underneath — was not. Fixed 2026-08-31.
                let loc = self.entity_location(id);
                if loc.is_valid() {
                    // Archetype'tan verileri temizle
                    if let Some(moved_eid) = self.archetype_index.archetypes
                        [loc.archetype_id as usize]
                        .swap_remove_entity(loc.row as usize)
                    {
                        // Kayan entity'nin location bilgisini güncelle
                        self.entity_locations[moved_eid as usize].row = loc.row;
                    }
                }
            }

            // THE ROW IS GONE, SO THE LOCATION MUST GO WITH IT — before any more user code
            // runs, not at the end of the function.
            //
            // The sparse `on_remove` hooks below get a `&mut World`, and until 2026-08-31 they
            // got it while this entity's location still named the row that had just been
            // swap-removed out from under it. That row now belongs to whoever was last in the
            // archetype. A hook that did anything routing through the location — `add_component`
            // is enough — handed that stale row to `move_entity_to`, which moves whoever is
            // sitting in it, and dragged a stranger into the target archetype under this
            // entity's id. (In a debug build `Moved`'s assertion catches that now; in release it
            // was silent.) Clearing both the location and the archetype-map entry here makes the
            // world consistent for the hooks instead: the entity has no row and no location, so
            // every path that asks either question gets the same answer, and `add_component`
            // returns early through `get_add_component_target`'s `None` rather than acting on a
            // row that is not this entity's.
            self.archetype_index.entity_archetype.remove(&id);
            if let Some(slot) = self.entity_locations.get_mut(id as usize) {
                *slot = EntityLocation::INVALID;
            }

            // SparseSet components live outside the archetype, so the swap-remove
            // above never touched them. Remove the entity from every sparse set
            // (firing on_remove) BEFORE the id is freed — otherwise the component
            // leaks and, since sets are keyed by raw id, a reused id inherits the
            // dead entity's stale value.
            let sparse_types: Vec<std::any::TypeId> =
                self.sparse_sets.keys().copied().collect();
            for tid in sparse_types {
                let removed = self
                    .sparse_sets
                    .get_mut(&tid)
                    .is_some_and(|set| set.remove(id));
                if removed {
                    self.run_hooks(tid, |h, w| {
                        for hook in &mut h.on_remove {
                            hook(w, e);
                        }
                    });
                }
            }

            // A hook cannot be allowed to put the entity BACK while it is being destroyed.
            // `flush_spawn` on a deferred spawn is enough to do it, and is that function's
            // documented purpose — the entity is still alive here, so its own liveness guard
            // does not fire. The row such a hook adds is orphaned the moment the location is
            // cleared, and orphaned rows are what stop `compact` from truncating the location
            // table (see there, and `docs/ENGINE.md` §3). Release behaviour is unchanged; a
            // debug build now names it at the despawn that caused it.
            debug_assert!(
                !self.entity_location(id).is_valid(),
                "an `on_remove` hook re-listed entity {id} into an archetype while it was being \
                 despawned; that row is about to be orphaned"
            );

            // Per-entity listeners go with the entity. Until 2026-08-31 nothing removed them
            // except `clear_entities` dropping the whole map, because the map's values are
            // per-event-type and `despawn` has no `E` to downcast with. They were harmless —
            // the key carries a generation and `free` below bumps it, so a stale listener can
            // never match a later occupant of the id — but they accumulated for the life of the
            // world in any game that pairs `observe` with destroying things. `EntityObserverMap`
            // records the removal where `E` is still known, the same way `ComponentInfo` records
            // a drop thunk.
            //
            // Before `free`, so the handle here is still the one the listener was filed under.
            for map in self.entity_observers.values_mut() {
                map.remove_entity(e);
            }

            {
                let entities = self
                    .get_resource::<Entities>()
                    .expect("Entities resource not initialized");
                entities.free(e);
            }

            // Cleared above, before the hooks ran. Repeated because a hook may have written a
            // location for this id in between — see the assertion — and a dead entity must not
            // be left with a live-looking one whatever else is wrong.
            self.archetype_index.entity_archetype.remove(&id);
            if let Some(slot) = self.entity_locations.get_mut(id as usize) {
                *slot = EntityLocation::INVALID;
            }
        }
        self.is_despawning = false;
    }

    /// Compacts the gaps in memory and, by deleting the unused (empty) Archetype tables, brings
    /// RAM and system performance back towards their initial defragmented (clean) state.
    /// Calling it on Loading screens or at low-intensity moments is recommended.
    ///
    /// **`SparseSet` storage is compacted too, since 2026-08-31**, and it is the piece most worth
    /// compacting. A sparse set's reverse index is indexed directly by entity id, so it is sized
    /// by the largest id ever inserted rather than by the number of components, and `remove` only
    /// writes a sentinel into it. Until this call learned about it, a world that had spawned a
    /// million entities carrying a sparse component kept that index's four megabytes **per
    /// component type** across every `compact`, and only [`World::clear_entities`] — which
    /// destroys every entity — gave it back. Each set now drops its trailing absent entries and
    /// shrinks all four of its arrays; see
    /// [`ComponentSparseSet::shrink_to_fit`](crate::archetype::sparse_set::ComponentSparseSet::shrink_to_fit).
    ///
    /// What it still does not reclaim: an empty set's `HashMap` entry — after shrinking, such a
    /// set holds no heap at all, so dropping it would buy a slot and would change
    /// `WorldStats::sparse_set_components`. And, deliberately, **`entity_locations`**, which is
    /// the same defect one size larger: it too is indexed directly by entity id, at 8 bytes per
    /// id against a sparse index's 4, and this function only `shrink_to_fit`s it. Truncating it
    /// is the same argument — everything past the last valid slot is absent — but it is not the
    /// same *audit*: `entity_locations[..]` is indexed from **dozens** of places against the one
    /// blind indexer `sparse` has, and each would have to be shown to run only for an entity that
    /// owns a row. Count them rather than trusting a number here — the first two counts written
    /// down for this were both wrong, once by including the two mentions inside comments and once
    /// by missing that four sites carry their own length check. `grep -rn 'entity_locations\['`
    /// over `crates/gizmo-core/src`, drop the comment lines, then drop the ones whose preceding
    /// lines test `entity_locations.len()`. That is its own change, not a rider on this one.
    #[tracing::instrument(skip_all, name = "compact")]
    pub fn compact(&mut self) {
        // 1. Önce eski, kullanılmayan boş archetype'ları silelim (GC)
        let removed = self
            .archetype_index
            .gc_empty_archetypes(&mut self.entity_locations);

        // 2. Kalan archetype'ların kapasitelerini minimuma indirelim (Shrink To Fit)
        for arch in &mut self.archetype_index.archetypes {
            arch.shrink_to_fit();
        }

        self.archetype_index.archetypes.shrink_to_fit();

        // 3. World seviyesindeki listeleri daraltalım.
        self.entities_to_despawn.shrink_to_fit();
        self.entity_locations.shrink_to_fit();

        // 4. SparseSet depolaması archetype'ların dışında yaşıyor, o yüzden yukarıdaki hiçbir
        //    adım ona dokunmuyor — ve en büyük iki tahsisten biri orada (öbürü, id başına iki
        //    katı yer tutan `entity_locations`; bkz. yukarıdaki not). Ters indeks entity id ile
        //    indexlendiği için uzunluğu "eklenmiş en büyük id + 1"; `remove` onu kısaltmaz,
        //    yalnız sentinel yazar.
        for set in self.sparse_sets.values_mut() {
            set.shrink_to_fit();
        }

        // 5. THE INVARIANT CHECK, and the truncation it was written for is NOT here.
        //
        // `entity_locations` is the same id-indexed shape as a sparse index at twice the bytes
        // per id, and truncating it past the last VALID slot would be the same fix. It is not
        // done, because truncation is only sound while every entity listed in an archetype has
        // a location naming it — and an adversarial sweep on 2026-08-31 constructed that
        // violation ten different ways, all confirmed, several through ordinary public API.
        // Two of the generators are fixed (see `flush_spawn` and `spawn_batch`); the rest are
        // filed in `docs/ENGINE.md` §3, and `despawn`'s sparse `on_remove` window is the one
        // that still has no answer.
        //
        // The check stays anyway, and is the useful half. It runs in every debug build — the
        // whole test suite, the property test, Miri — and reports the violation at the GC tick
        // with its own shape named. Without it the same corruption surfaces later as a wrong
        // component value or an out-of-bounds write in whatever migration trips over it, with
        // nothing to say why.
        #[cfg(debug_assertions)]
        for (arch_idx, arch) in self.archetype_index.archetypes.iter().enumerate() {
            for (row, &id) in arch.entities().iter().enumerate() {
                let loc = self
                    .entity_locations
                    .get(id as usize)
                    .copied()
                    .unwrap_or(crate::archetype::EntityLocation::INVALID);
                debug_assert!(
                    loc.is_valid()
                        && loc.archetype_id as usize == arch_idx
                        && loc.row as usize == row,
                    "compact: entity {id} is listed at archetype {arch_idx} row {row}, but its \
                     location says archetype {} row {} — truncating would drop a slot that is \
                     still in use",
                    loc.archetype_id,
                    loc.row
                );
            }
        }

        let entities = self
            .get_resource::<Entities>()
            .expect("Entities resource not initialized");
        let mut state = entities.state.lock().unwrap_or_else(|e| e.into_inner());
        state.generations.shrink_to_fit();
        state.free_ids.shrink_to_fit();
        state.free_set.shrink_to_fit();
        drop(state);

        tracing::debug!(
            removed_archetypes = removed,
            remaining_archetypes = self.archetype_index.archetypes.len(),
            "compact: reclaimed empty archetypes and shrank storage"
        );
    }

    /// Despawns whichever live entity currently occupies id slot `id`, resolving the
    /// generation through [`World::get_entity`]. A free or never-allocated slot is a no-op.
    ///
    /// Because the generation is resolved at call time this cannot target a *specific*
    /// incarnation: if the id was recycled since you obtained it, this kills the new
    /// occupant. Use [`World::despawn`] with a real handle whenever that distinction
    /// matters.
    ///
    /// # Panics
    /// If the [`Entities`] resource has been removed from the world.
    pub fn despawn_by_id(&mut self, id: u32) {
        if let Some(entity) = self.get_entity(id) {
            self.despawn(entity);
        }
    }

    /// Despawns ALL entities that have the `C` component; returns the number deleted.
    /// It reduces common operations such as "clear this scene/group wholesale" (e.g. every
    /// `LevelEntity` when the level is reloaded) or "delete all the bullets" to a single line —
    /// the developer no longer keeps a `Vec<Entity>` and deletes by hand in a loop. Add a marker
    /// component, call this.
    ///
    /// ```
    /// # use gizmo_core::prelude::*;
    /// # let mut world = World::new();
    /// #[derive(Clone, Copy)] struct Bullet;
    /// gizmo_core::impl_component!(Bullet);
    ///
    /// for _ in 0..3 {
    ///     let b = world.spawn();
    ///     world.add_component(b, Bullet);
    /// }
    /// let survivor = world.spawn(); // Bullet yok → dokunulmaz
    ///
    /// let cleared = world.despawn_all_with::<Bullet>();
    /// assert_eq!(cleared, 3);
    /// assert!(world.is_alive(survivor));
    /// ```
    pub fn despawn_all_with<C: crate::component::Component>(&mut self) -> usize {
        // Önce id'leri topla (query &self ödünç alır), sonra despawn et (&mut self).
        let ids: Vec<u32> = match self.query::<&C>() {
            Some(q) => q.iter().map(|(id, _)| id).collect(),
            None => Vec::new(),
        };
        let n = ids.len();
        for id in ids {
            self.despawn_by_id(id);
        }
        tracing::debug!(
            removed = n,
            component = std::any::type_name::<C>(),
            "despawn_all_with"
        );
        n
    }

    /// An iterator returning all the Entities that are alive (not despawned).
    /// Warning: the Entities mutex lock is held for the duration of the iteration!
    pub fn iter_alive_entities(&self) -> Vec<Entity> {
        let entities = self
            .get_resource::<Entities>()
            .expect("Entities resource not initialized");
        let state = entities.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut alive = Vec::new();
        for id in 0..state.next_entity_id {
            if !state.free_set.contains(&id) {
                alive.push(Entity::new(id, state.generations[id as usize]));
            }
        }
        alive
    }

    /// Generation-checked liveness: `true` only while `entity`'s generation still matches
    /// the allocator's current generation for that id, so a stale handle to a recycled id
    /// correctly reports `false`.
    ///
    /// "Alive" means *the id is allocated*, not that it has storage — an id reserved via
    /// `Commands::spawn` and not yet flushed is already alive here while having no
    /// components and no valid `EntityLocation`. Use [`World::entity_location`] if you need
    /// to know that storage exists.
    ///
    /// Takes the allocator mutex, so despite `#[inline]` this is a lock acquisition, not a
    /// field read; hoist it out of hot loops.
    ///
    /// # Panics
    /// If the [`Entities`] resource has been removed from the world.
    #[inline]
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.get_resource::<Entities>()
            .expect("Entities resource not initialized")
            .is_alive(entity)
    }

    /// Returns the TypeIds of all the components on the Entity.
    pub fn entity_component_types(&self, entity: Entity) -> Vec<TypeId> {
        if !self.is_alive(entity) {
            return Vec::new();
        }
        let mut types = Vec::new();
        if let Some(&loc) = self.entity_locations.get(entity.id() as usize) {
            if loc.is_valid() {
                let arch = &self.archetype_index.archetypes[loc.archetype_id as usize];
                types = arch.component_types();
            }
        }
        // Include SparseSet components the entity holds — they aren't archetype
        // columns, so callers (reflection, scene save) would otherwise miss them.
        for (tid, set) in &self.sparse_sets {
            if set.contains(entity.id()) {
                types.push(*tid);
            }
        }
        types
    }

    /// The canonical way to turn a raw `u32` id into a live [`Entity`] handle with its
    /// CURRENT generation. Returns `None` if no live entity occupies that id slot.
    ///
    /// Prefer this over fabricating `Entity::new(id, 0)`: the generation-checked APIs
    /// (`is_alive`, `entity_component_types`, `get_entity`, …) reject a gen-0 handle once
    /// the id slot has been recycled (despawn→spawn bumps the generation), which silently
    /// loses data / points at the wrong entity. This was the root of several audit bugs.
    pub fn entity(&self, id: u32) -> Option<Entity> {
        if id as usize >= self.entity_locations.len() || !self.entity_locations[id as usize].is_valid() {
            return None;
        }
        let entities = self.get_resource::<Entities>()?;
        let state = entities.state.lock().unwrap_or_else(|e| e.into_inner());
        if id as usize >= state.generations.len() || state.free_set.contains(&id) {
            return None;
        }
        Some(Entity::new(id, state.generations[id as usize]))
    }

    /// Deprecated alias for [`World::entity`].
    #[deprecated(note = "renamed to `World::entity`")]
    pub fn reconstruct_entity(&self, id: u32) -> Option<Entity> {
        self.entity(id)
    }

    /// Returns the Entity's archetype location — O(1) lookup.
    #[inline]
    pub fn entity_location(&self, entity_id: u32) -> EntityLocation {
        let loc_idx = entity_id as usize;
        if loc_idx < self.entity_locations.len() {
            self.entity_locations[loc_idx]
        } else {
            EntityLocation::INVALID
        }
    }

    /// The total number of living entities
    #[inline]
    pub fn entity_count(&self) -> u32 {
        let entities = self
            .get_resource::<Entities>()
            .expect("Entities resource not initialized");
        let state = entities.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .next_entity_id
            .saturating_sub(state.free_ids.len() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn despawn_reserved_but_unflushed_entity_does_not_panic() {
        let mut world = World::new();
        // Reserve an id WITHOUT flushing it (this is what `Commands::spawn` does).
        // `is_alive` is true because the generation is registered in the allocator,
        // but there is no `entity_locations` slot yet — the old raw index panicked.
        let reserved = {
            let entities = world
                .get_resource::<Entities>()
                .expect("Entities resource");
            entities.reserve_entity()
        };
        assert!(world.is_alive(reserved), "a reserved entity is considered alive");

        world.despawn(reserved); // must not panic (bounds-safe location lookup)

        assert!(!world.is_alive(reserved), "despawn freed the reserved id");
    }

    #[test]
    fn despawn_all_with_removes_only_tagged() {
        #[derive(Clone, Copy)]
        struct Tag;
        crate::impl_component!(Tag);

        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        let c = world.spawn();
        world.add_component(a, Tag);
        world.add_component(c, Tag);
        // b has no Tag.

        let removed = world.despawn_all_with::<Tag>();
        assert_eq!(removed, 2, "yalnız 2 tag'li silinmeli");
        assert!(!world.is_alive(a) && !world.is_alive(c), "tag'liler gitti");
        assert!(world.is_alive(b), "tag'siz korunmalı");

        // Boş çağrı 0 döner, panik yok.
        assert_eq!(world.despawn_all_with::<Tag>(), 0);
    }
}
