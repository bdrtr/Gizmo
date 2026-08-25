use super::World;
use crate::archetype::{ComponentInfo, EntityLocation};
use crate::component::Component;
use crate::entity::Entity;

use std::any::TypeId;

/// Drops the live value at `row` in each of `types`' columns, leaving every slot uninitialised
/// for the bundle write that follows.
///
/// Exists because [`Bundle::write_to_archetype`](crate::component::Bundle::write_to_archetype)
/// copies raw bytes and so cannot drop what it replaces, while the same call is also used on
/// the uninitialised holes [`Archetype::move_entity_to`](crate::archetype::Archetype) allocates
/// — where dropping would be undefined. Both sit below the column's `len`, so the write cannot
/// tell them apart; `World::add_bundle` can, and this is how it says so.
///
/// **Deduplicated by `TypeId`**, because a bundle may legitimately name one component twice —
/// `BundleExt::with` appends rather than substitutes — and dropping one slot twice is a double
/// free. A linear scan, not a set: a bundle is a handful of types and the allocation would cost
/// more than the comparisons.
///
/// Types the archetype has no column for are skipped rather than treated as an error, so the
/// caller may pass a superset.
///
/// # Safety
/// `row` must be a live row of `arch`, every named column's slot at `row` must hold a live
/// value, and the caller must re-initialise each dropped slot before anything reads the
/// archetype.
unsafe fn drop_live_bundle_rows(
    arch: &crate::archetype::Archetype,
    types: &[TypeId],
    row: usize,
) {
    let mut dropped: Vec<TypeId> = Vec::new();
    for &type_id in types {
        if dropped.contains(&type_id) {
            continue;
        }
        dropped.push(type_id);
        // SAFETY: the caller guarantees `row` is live in this column; `get_column_mut` hands
        // out one `&mut Column` per type and each type is visited once, so they never overlap.
        if let Some(col) = arch.get_column_mut(type_id) {
            col.drop_row_in_place(row);
        }
    }
}

impl World {
    /// Adding a component to the system — moves the data into the archetype column.
    ///
    /// A dead entity is a silent no-op, and so is an entity whose id was *reserved* from the
    /// allocator but never handed to [`World::flush_spawn`] (what `Commands::spawn` produces
    /// before its queue is applied): it is [`World::is_alive`] yet owns no archetype row, so
    /// there is nowhere to write. That mirrors [`World::add_component`]'s documented
    /// behaviour for the same entity state.
    ///
    /// **A component the entity already carries is replaced, and the old value is dropped.**
    /// Until 2026-08-25 it was overwritten in place without being dropped, so re-asserting a
    /// component that owned a heap allocation leaked it once per call — which a bundle used as
    /// a "set these fields" call does every frame. [`World::add_component`] documents the same
    /// sentence for the single-component path, and the two now agree.
    ///
    /// The drop is **silent**: it is not a `Remove`, and no hook or observer sees it. The
    /// bundle path fires nothing at all (below), so there is no phase for it to be reported in.
    ///
    /// Hooks: an all-`Table` bundle takes the block-move fast path and fires **no**
    /// `on_add`/`on_set` hooks — observers registered with [`World::add_observer`] do not see
    /// it. A bundle carrying a `SparseSet` component is routed through
    /// [`crate::component::Bundle::apply`] instead and does fire them per component. See
    /// [`crate::world::hooks::ComponentHooks`] for the full list of hook-free paths.
    pub fn add_bundle<B: crate::component::Bundle>(&mut self, entity: Entity, bundle: B) {
        if !self.is_alive(entity) { return; }
        let eid = entity.id();
        let infos = B::get_infos();

        // The block-move fast path below writes every component into an archetype
        // column, which has no home for SparseSet components (they live in
        // `sparse_sets`). If the bundle carries any sparse component, route the
        // whole bundle through `apply` — per-component `add_component`, which
        // places each component in its correct storage. All-table bundles keep the
        // single-migration fast path.
        if infos
            .iter()
            .any(|i| i.storage_type == crate::component::StorageType::SparseSet)
        {
            bundle.apply(self, entity);
            return;
        }

        for info in &infos {
            self.component_infos.entry(info.type_id).or_insert_with(|| *info);
        }

        // An id RESERVED from the allocator but never flushed (`Commands::spawn` hands one
        // out before its queued `flush_spawn` runs) is `is_alive` yet owns no archetype row.
        // Every path below indexes `entity_locations[eid]` raw, so such an entity panicked:
        // out of bounds when the slot was never allocated, or — for a recycled id, whose
        // slot exists but holds `EntityLocation::INVALID` — it fed `row == u32::MAX` into
        // `move_entity_to`. Bail out instead, which is exactly what `add_component` and
        // `Bundle::apply` already do for Table components on such an entity.
        //
        // Flushing here instead would be WRONG: `Commands::spawn` has already queued a
        // `flush_spawn` for this id and `flush_spawn` must run exactly once — a second call
        // pushes another empty-archetype row and overwrites the location, orphaning storage.
        if !self.entity_location(eid).is_valid() {
            tracing::warn!(
                entity = eid,
                "add_bundle: entity has no archetype row (reserved but not flushed); bundle dropped"
            );
            return;
        }

        // Table bazlı block move:
        let old_arch_id = match self.archetype_index.entity_archetype.get(&eid) {
            Some(&id) => id,
            None => {
                // Unreachable for a live, flushed entity: `flush_spawn` → `on_spawn` always
                // inserts `entity_archetype[eid] = 0`, so even a component-less entity takes
                // the `Some(0)` arm. Kept as a defensive fallback to the empty archetype.
                let _arch = &mut self.archetype_index.archetypes[0];
                0
            }
        };

        let mut new_types = self.archetype_index.archetypes[old_arch_id].sorted_component_types();
        for info in &infos {
            if let Err(pos) = new_types.binary_search(&info.type_id) {
                new_types.insert(pos, info.type_id);
            }
        }

        // The bundle's component types, in bundle order and with duplicates intact — a bundle
        // may legitimately name one type twice, since `BundleExt::with` appends rather than
        // substitutes. `drop_live_bundle_rows` is what deduplicates them, because dropping one
        // slot twice is a double free.
        let bundle_types: Vec<TypeId> = infos.iter().map(|i| i.type_id).collect();

        let target_arch_id = if let Some(&id) = self.archetype_index.set_to_id.get(&new_types) {
            id
        } else {
            let id = self.archetype_index.archetypes.len();
            let mut new_infos = Vec::new();
            for &t in &new_types {
                new_infos.push(self.component_infos.get(&t).cloned().unwrap());
            }
            self.archetype_index.archetypes.push(crate::archetype::Archetype::new(id as u32, &new_infos));
            self.archetype_index.set_to_id.insert(new_types, id);
            id
        };

        if old_arch_id == target_arch_id {
            // Sadece override
            let loc = self.entity_locations[eid as usize];
            let arch = &mut self.archetype_index.archetypes[target_arch_id];
            // Every column this bundle is about to write already holds a LIVE value — same
            // archetype, same row — and `write_to_archetype` copies raw bytes, so it cannot
            // drop what it replaces. Dropping is therefore this function's job, and it has to
            // be: the same write also lands on the uninitialised holes `move_entity_to` leaves
            // behind (see the migration path below), where dropping would be undefined. From
            // inside the write the two are indistinguishable — both sit below the column's
            // `len` — so only the caller can tell them apart.
            //
            // SAFETY: `target_arch_id == old_arch_id`, so this archetype has a column for every
            // one of `bundle_types` and `loc.row` is the entity's live row in all of them; the
            // write below re-initialises every slot dropped here.
            unsafe { drop_live_bundle_rows(arch, &bundle_types, loc.row as usize); }
            // SAFETY: `write_to_archetype`'s contract is that the bundle writes EVERY column of
            // this archetype at this row — the archetype was chosen for exactly this bundle's
            // component set, and `loc.row` is the entity's live row. A bundle that skipped a
            // column would desync column length from `entities`; `debug_assert_consistent`
            // catches that in debug builds.
            unsafe { bundle.write_to_archetype(arch, loc.row as usize, self.tick); }
            return;
        }

        tracing::trace!(
            entity = eid,
            from = old_arch_id,
            to = target_arch_id,
            "add_bundle: archetype migration"
        );

        let old_loc = self.entity_locations[eid as usize];

        // Which of the bundle's components will arrive at the new row ALIVE: exactly those the
        // old archetype already had, which `move_entity_to` copies across. Everything else is
        // one of the holes it allocates and leaves uninitialised for this write to fill, and
        // dropping a hole frees a garbage pointer. Computed BEFORE the move, because the move
        // is what makes the distinction unrecoverable.
        let live_after_move: Vec<TypeId> = {
            let old_arch = &self.archetype_index.archetypes[old_arch_id];
            bundle_types.iter().copied().filter(|t| old_arch.has_component(*t)).collect()
        };

        let (new_row, moved_eid) = {
            // İki archetype'ı FARKLI indekslerden disjoint ödünç al. Aynı Vec'ten
            // iki `&mut ...[i] as *mut` almak, ikinci retag ile ilk pointer'ın
            // provenance'ını geçersiz kılıp onu kullanınca UB üretiyordu (Miri
            // Stacked Borrows). `get_disjoint_mut` aliasing'siz iki &mut verir.
            let [old_arch, target_arch] = self
                .archetype_index
                .archetypes
                .get_disjoint_mut([old_arch_id, target_arch_id])
                .expect("old and target archetype indices are distinct and in bounds");
            // SAFETY: move_entity_to raw sütun kopyaları yapar; ödünçler disjoint.
            unsafe { old_arch.move_entity_to(old_loc.row as usize, target_arch) }
        };

        if let Some(moved) = moved_eid {
            self.entity_locations[moved as usize].row = old_loc.row;
        }

        let arch = &mut self.archetype_index.archetypes[target_arch_id];
        // The components the entity already carried came across the migration alive; the ones
        // the bundle is ADDING are holes. Drop the first group and only the first group — that
        // distinction is the whole reason `live_after_move` was computed before the move.
        //
        // SAFETY: every type in `live_after_move` was a column of the OLD archetype, so
        // `move_entity_to` moved its value into `new_row` of the target's matching column and
        // that slot is live; the write below re-initialises each one.
        unsafe { drop_live_bundle_rows(arch, &live_after_move, new_row as usize); }
        // SAFETY: as above, for the row just pushed into the target archetype during the move.
        unsafe { bundle.write_to_archetype(arch, new_row as usize, self.tick); }

        self.entity_locations[eid as usize] = EntityLocation {
            archetype_id: target_arch_id as u32,
            row: new_row,
        };
        self.archetype_index.entity_archetype.insert(eid, target_arch_id);
    }

    /// Removes every component listed by `B` from `entity` in one archetype migration.
    ///
    /// Only `B`'s *type list* is used, never any value, hence the turbofish:
    /// `world.remove_bundle::<(Transform, Velocity)>(e)`. Components of `B` the entity does
    /// not have are ignored, and a dead entity is a silent no-op.
    ///
    /// The migration swap-removes the entity's old row, so another entity in the source
    /// archetype may change position (its data is preserved).
    ///
    /// Hooks: `on_remove` fires once for each of `B`'s components the entity actually held,
    /// whatever its storage — sparse ones from the explicit loop, Table ones after the archetype
    /// migration, which is where [`World::remove_component`] fires for the same storage.
    ///
    /// Until 2026-08-24 the Table half fired nothing: the components went out through the block
    /// move in silence, so the same component removed two ways answered differently. Nothing had
    /// to care while `On<Remove, T>` was undeliverable; giving it a dispatch path made the
    /// asymmetry a broken promise.
    pub fn remove_bundle<B: crate::component::Bundle>(&mut self, entity: Entity) {
        if !self.is_alive(entity) { return; }
        let eid = entity.id();
        let infos = B::get_infos();

        // SparseSet components live in `sparse_sets`, not archetype columns, so the
        // block-move below never touches them — remove them explicitly (mirrors
        // remove_component's sparse branch, on_remove hooks included).
        for info in &infos {
            if info.storage_type == crate::component::StorageType::SparseSet {
                let removed = self
                    .sparse_sets
                    .get_mut(&info.type_id)
                    .is_some_and(|set| set.remove(eid));
                if removed {
                    let tid = info.type_id;
                    self.run_hooks(tid, |h, w| {
                        for hook in &mut h.on_remove {
                            hook(w, entity);
                        }
                    });
                }
            }
        }

        let old_arch_id = match self.archetype_index.entity_archetype.get(&eid) {
            Some(&id) => id,
            None => return,
        };

        let mut new_types = self.archetype_index.archetypes[old_arch_id].sorted_component_types();
        // Which of the bundle's Table components the entity actually holds. Collected here,
        // before the migration edits `new_types`, because afterwards the old archetype no
        // longer says what this entity had. They are notified below, after the move, which
        // is where `remove_component` notifies for the same storage.
        //
        // Until 2026-08-24 they were not notified at all: the sparse loop above fired
        // `on_remove` and the Table half went out through `move_entity_to` in silence, so the
        // same component removed two ways answered differently. `On<Remove, T>` made that a
        // public promise rather than an internal quirk.
        let mut detached_table_types = Vec::new();
        for info in &infos {
            if let Ok(pos) = new_types.binary_search(&info.type_id) {
                new_types.remove(pos);
                if info.storage_type == crate::component::StorageType::Table {
                    detached_table_types.push(info.type_id);
                }
            }
        }

        let target_arch_id = if let Some(&id) = self.archetype_index.set_to_id.get(&new_types) {
            id
        } else {
            let id = self.archetype_index.archetypes.len();
            let mut new_infos = Vec::new();
            for &t in &new_types {
                new_infos.push(self.component_infos.get(&t).cloned().unwrap());
            }
            self.archetype_index.archetypes.push(crate::archetype::Archetype::new(id as u32, &new_infos));
            self.archetype_index.set_to_id.insert(new_types, id);
            id
        };

        if old_arch_id == target_arch_id { return; }

        tracing::trace!(
            entity = eid,
            from = old_arch_id,
            to = target_arch_id,
            "remove_bundle: archetype migration"
        );

        let old_loc = self.entity_locations[eid as usize];
        let (new_row, moved_eid) = {
            // İki archetype'ı FARKLI indekslerden disjoint ödünç al. Aynı Vec'ten
            // iki `&mut ...[i] as *mut` almak, ikinci retag ile ilk pointer'ın
            // provenance'ını geçersiz kılıp onu kullanınca UB üretiyordu (Miri
            // Stacked Borrows). `get_disjoint_mut` aliasing'siz iki &mut verir.
            let [old_arch, target_arch] = self
                .archetype_index
                .archetypes
                .get_disjoint_mut([old_arch_id, target_arch_id])
                .expect("old and target archetype indices are distinct and in bounds");
            // SAFETY: move_entity_to raw sütun kopyaları yapar; ödünçler disjoint.
            unsafe { old_arch.move_entity_to(old_loc.row as usize, target_arch) }
        };

        if let Some(moved) = moved_eid {
            self.entity_locations[moved as usize].row = old_loc.row;
        }

        self.entity_locations[eid as usize] = EntityLocation {
            archetype_id: target_arch_id as u32,
            row: new_row,
        };
        self.archetype_index.entity_archetype.insert(eid, target_arch_id);

        for tid in detached_table_types {
            self.run_hooks(tid, |h, w| {
                for hook in &mut h.on_remove {
                    hook(w, entity);
                }
            });
        }
    }

    /// Attaches `component` to `entity`, overwriting any value already there, and registers
    /// `T`'s runtime metadata with the world as a side effect (so
    /// [`World::register_component_type`] is never strictly required first).
    ///
    /// A dead entity is a silent no-op. An overwrite assigns over the existing slot, which
    /// drops the previous value — no leak for a `T` that owns a heap allocation. A first
    /// attach migrates the entity to the archetype with `T` added, swap-removing its old
    /// row, so another entity in the source archetype may change position.
    ///
    /// Hooks: a first attach fires `on_add` then `on_set`; an overwrite fires `on_set` then
    /// `on_replace`. Exactly one of `on_add` / `on_replace` runs per write — they partition the
    /// writes `on_set` sees. That holds identically for `Table` and `SparseSet` storage, though
    /// the two reach the decision differently: Table storage reads it off which branch the
    /// migration took, sparse storage asks the set whether the entity was already in it.
    ///
    /// One asymmetry to watch for on entities whose id was *reserved* from the allocator but
    /// never passed to [`World::flush_spawn`]: they are [`World::is_alive`] but have no
    /// archetype row, so a `Table`-storage component is dropped silently while a `SparseSet`
    /// one is stored anyway, since sparse sets live outside the archetype.
    ///
    /// # Panics
    /// If the target archetype turns out to lack `T`'s column, which would mean the
    /// archetype index and the component metadata registry have diverged.
    pub fn add_component<T: Component>(&mut self, entity: Entity, component: T) {
        if !self.is_alive(entity) { return; }
        let eid = entity.id();
        self.register_component_type::<T>();
        let type_id = TypeId::of::<T>();

        if T::storage_type() == crate::component::StorageType::SparseSet {
            let info = self.component_infos.get(&type_id).copied().unwrap_or_else(|| ComponentInfo::of::<T>());
            let set = self.sparse_sets.entry(type_id).or_insert_with(|| {
                crate::archetype::sparse_set::ComponentSparseSet::new(info)
            });
            // Overwrite vs. new insert: fire on_add ONLY when the entity did not already
            // have the component, matching the Table-storage path below (overwrite → on_set
            // only). Previously SparseSet unconditionally fired on_add, so re-adding a
            // SparseSet component double-fired Insert observers — storage-dependent behavior.
            let existed = set.contains(eid);
            let ptr = &component as *const T as *const u8;
            // SAFETY: `ptr`, set'in `info.layout`'u ile birebir eşleşen `T` bileşenini gösterir;
            // sahiplik set'e devredilir ve aşağıda `forget` ile çift-drop engellenir.
            unsafe { set.insert(eid, ptr, self.tick); }
            std::mem::forget(component);

            self.run_hooks(type_id, |h, w| {
                if !existed {
                    for hook in &mut h.on_add { hook(w, entity); }
                }
                for hook in &mut h.on_set { hook(w, entity); }
                // `existed` is the whole distinction, and the sparse path is the only one
                // that has to compute it — the Table paths get it from which branch they
                // are in. Same partition either way: add XOR replace, never both.
                if existed {
                    for hook in &mut h.on_replace { hook(w, entity); }
                }
            });
            return;
        }

        // Original logic follows but skip register and eid assignments



        // 1. Hedef archetype'ı belirle
        let target_arch_id =
            match self
                .archetype_index
                .get_add_component_target(eid, type_id, &self.component_infos)
            {
                Some(id) => id,
                None => return,
            };
        let old_loc = self.entity_locations[eid as usize];

        if old_loc.archetype_id == target_arch_id as u32 {
            // Zaten bu archetype'ta (aynı tip tekrar eklenmiş olabilir) — sadece üzerine yaz
            {
                let arch = &self.archetype_index.archetypes[target_arch_id];
                // SAFETY: query/scheduler bu archetype sütununa ayrık erişimi garanti eder.
                let col = unsafe { arch.get_column_mut(type_id) }
                    .expect("component column missing in current archetype");
                // SAFETY: `type_id` is `T`'s, so the column's layout is `T`'s, and `old_loc.row`
                // is the entity's live row. Assignment (not `ptr::write`) is deliberate: the slot
                // already holds a live `T` and `*ptr = ..` drops it, where a write would leak.
                unsafe {
                    let ptr = col.get_ptr(old_loc.row as usize) as *mut T;
                    *ptr = component;
                    col.ticks_ptr_mut()
                        .add(old_loc.row as usize)
                        .write(crate::archetype::ComponentTicks::new(self.tick));
                }
            }
            // An overwrite: `on_set` for every write, then `on_replace` for the half of
            // them that had something to replace. `on_add` is deliberately absent — that is
            // what makes the two lists a partition rather than an overlap.
            //
            // This used to hand-roll `run_hooks`'s take-and-merge-back, which is how a
            // fourth hook list becomes a silent bug: the copy would have kept merging three.
            self.run_hooks(type_id, |h, w| {
                for hook in &mut h.on_set {
                    hook(w, entity);
                }
                for hook in &mut h.on_replace {
                    hook(w, entity);
                }
            });
            return;
        }

        // 2. Migration: Verileri eski archetype'tan hedef archetype'a taşı
        let (eid, old_arch_id, old_row) = (
            entity.id(),
            old_loc.archetype_id as usize,
            old_loc.row as usize,
        );
        tracing::trace!(
            entity = eid,
            from = old_arch_id,
            to = target_arch_id,
            "add_component: archetype migration"
        );

        // İki archetype'ı FARKLI indekslerden disjoint olarak ödünç al. Önceki hal
        // aynı Vec'ten iki `&mut ...[i] as *mut` alıyordu; ikinci retag ilk
        // pointer'ın provenance'ını geçersiz kılıp onu kullanınca UB üretiyordu
        // (Miri Stacked Borrows ihlali). `get_disjoint_mut` iki ayrı indekse
        // aliasing'siz `&mut` verir — unsafe'e gerek yok.
        let (new_row, moved_eid) = {
            let [old_arch, target_arch] = self
                .archetype_index
                .archetypes
                .get_disjoint_mut([old_arch_id, target_arch_id])
                .expect("old and target archetype indices must be distinct and in bounds");
            // SAFETY: move_entity_to raw sütun kopyaları yapar; ödünçler disjoint.
            unsafe { old_arch.move_entity_to(old_row, target_arch) }
        };

        if let Some(moved) = moved_eid {
            self.entity_locations[moved as usize].row = old_row as u32;
        }

        // 3. Yeni component'ı hedef archetype'a ekle
        {
            let arch = &self.archetype_index.archetypes[target_arch_id];
            // SAFETY: yeni satır bu archetype'a az önce ayrıldı; sütuna tekil erişim.
            let col = unsafe { arch.get_column_mut(type_id) }
                .expect("Mandatory component column missing");
            // SAFETY: `type_id` is `T`'s and `new_row` was just allocated in this archetype, so
            // the slot is uninitialised — `ptr::write` (not assignment) is the right one here,
            // because there is no old value to drop.
            unsafe {
                let ptr = col.get_ptr(new_row as usize) as *mut T;
                std::ptr::write(ptr, component);
                col.ticks_ptr_mut()
                    .add(new_row as usize)
                    .write(crate::archetype::ComponentTicks::new(self.tick));
            }
        }

        // 4. Location güncellemeleri
        self.entity_locations[eid as usize] = EntityLocation {
            archetype_id: target_arch_id as u32,
            row: new_row,
        };
        self.archetype_index
            .entity_archetype
            .insert(eid, target_arch_id);

        // A genuinely new insert — the entity migrated to a different archetype to get here,
        // so there was no old value and `on_replace` must stay silent. The other
        // hand-rolled copy of `run_hooks` in this file, for the same reason as the first.
        self.run_hooks(type_id, |h, w| {
            for hook in &mut h.on_add {
                hook(w, entity);
            }
            for hook in &mut h.on_set {
                hook(w, entity);
            }
        });
    }

    /// Getting a raw Component Pointer (for Reflection/Editor)
    pub fn get_component_ptr(&self, entity: Entity, type_id: TypeId) -> Option<*const u8> {
        // SparseSet components live outside the archetype — otherwise type-erased
        // access (reflection, scene serialization) can't see them.
        if let Some(set) = self.sparse_sets.get(&type_id) {
            if let Some(p) = set.get_ptr(entity.id()) {
                return Some(p);
            }
        }
        let loc = self.entity_locations.get(entity.id() as usize).copied()?;
        if !loc.is_valid() {
            return None;
        }
        let arch = &self.archetype_index.archetypes[loc.archetype_id as usize];
        let col = arch.get_column(type_id)?;
        // SAFETY: `loc` was checked valid above, so `loc.row` is a live row of this archetype and
        // the column belongs to it. The pointer is raw and borrows nothing — the caller must not
        // hold it across a structural change.
        Some(unsafe { col.get_ptr(loc.row as usize) })
    }

    /// Getting a Mut mutable Component pointer (for HierarchyExt etc.)
    pub fn get_component_mut_ptr(&mut self, entity: Entity, type_id: TypeId) -> Option<*mut u8> {
        if let Some(set) = self.sparse_sets.get_mut(&type_id) {
            if let Some(p) = set.get_ptr_mut(entity.id()) {
                return Some(p);
            }
        }
        let loc = self.entity_locations.get(entity.id() as usize).copied()?;
        if !loc.is_valid() {
            return None;
        }
        let arch = &mut self.archetype_index.archetypes[loc.archetype_id as usize];
        // SAFETY: &mut self ile tekil archetype erişimi; sütuna tekil &mut.
        let col = unsafe { arch.get_column_mut(type_id) }?;
        // SAFETY: as in `get_component_ptr`, and `&mut self` makes this the only live view.
        Some(unsafe { col.get_mut_ptr(loc.row as usize) })
    }

    /// Deleting a component from the system
    pub fn remove_component<T: Component>(&mut self, entity: Entity) {
        if !self.is_alive(entity) { return; }
        let eid = entity.id();
        let type_id = TypeId::of::<T>();

        if T::storage_type() == crate::component::StorageType::SparseSet {
            if let Some(set) = self.sparse_sets.get_mut(&type_id) {
                if set.remove(eid) {
                    self.run_hooks(type_id, |h, w| {
                        for hook in &mut h.on_remove { hook(w, entity); }
                    });
                }
            }
            return;
        }


        let old_loc = self.entity_locations[eid as usize];

        // 1. Hedef archetype'ı belirle
        let target_arch_id_opt =
            self.archetype_index
                .get_remove_component_target(eid, type_id, &self.component_infos);
        let target_arch_id = match target_arch_id_opt {
            Some(id) => id,
            None => return, // Zaten yok veya hata
        };

        if old_loc.archetype_id == target_arch_id as u32 {
            return; // Zaten yok
        }

        tracing::trace!(
            entity = eid,
            from = old_loc.archetype_id,
            to = target_arch_id,
            "remove_component: archetype migration"
        );

        // 2. Migration — iki archetype'ı FARKLI indekslerden disjoint ödünç al
        // (aynı Vec'ten iki `&mut ... as *mut` = geçersiz-kılınan-provenance UB'si).
        let (new_row, moved_eid) = {
            let [old_arch, target_arch] = self
                .archetype_index
                .archetypes
                .get_disjoint_mut([old_loc.archetype_id as usize, target_arch_id])
                .expect("old and target archetype indices are distinct and in bounds");
            // SAFETY: move_entity_to raw sütun kopyaları yapar; ödünçler disjoint.
            unsafe { old_arch.move_entity_to(old_loc.row as usize, target_arch) }
        };

        if let Some(moved) = moved_eid {
            self.entity_locations[moved as usize].row = old_loc.row;
        }

        // 3. Location güncelle
        self.entity_locations[eid as usize] = EntityLocation {
            archetype_id: target_arch_id as u32,
            row: new_row,
        };
        self.archetype_index
            .entity_archetype
            .insert(eid, target_arch_id);

        self.run_hooks(type_id, |h, w| {
            for hook in &mut h.on_remove {
                hook(w, entity);
            }
        });
    }

    /// Batch component insertion. It reduces the O(N) archetype lookup cost to O(1).
    ///
    /// # Example
    /// ```
    /// # use gizmo_core::prelude::*;
    /// # #[derive(Clone, Copy)] struct Health(u32);
    /// # #[derive(Clone, Copy)] struct Team(u8);
    /// # gizmo_core::impl_component!(Health, Team);
    /// # let mut world = World::new();
    /// let ids: Vec<Entity> = (0..3).map(|_| world.spawn_bundle(Health(100))).collect();
    /// world.insert_batch(&ids, Team(2)); // one archetype lookup for the whole group
    ///
    /// let q = world.query::<&Team>().unwrap();
    /// assert_eq!(q.iter().count(), 3);
    /// assert_eq!(q.get(ids[2].id()).unwrap().0, 2);
    /// ```
    #[tracing::instrument(skip_all, name = "insert_batch")]
    pub fn insert_batch<T: Component + Clone>(&mut self, entities: &[Entity], component: T) {
        if T::storage_type() == crate::component::StorageType::SparseSet {
            for &e in entities {
                self.add_component(e, component.clone());
            }
            return;
        }

        self.register_component_type::<T>();
        let type_id = TypeId::of::<T>();

        // 1. Gruplama: source_arch_id -> Vec<Entity>
        let mut groups: std::collections::HashMap<u32, Vec<Entity>> = std::collections::HashMap::new();

        for &e in entities {
            if !self.is_alive(e) { continue; }
            let loc = self.entity_locations[e.id() as usize];
            if !loc.is_valid() { continue; }
            groups.entry(loc.archetype_id).or_default().push(e);
        }

        for (source_arch_id, group_entities) in groups {
            let target_arch_id = match self.archetype_index.get_add_component_target(
                group_entities[0].id(), type_id, &self.component_infos
            ) {
                Some(id) => id,
                None => continue,
            };

            if source_arch_id == target_arch_id as u32 {
                let arch = &self.archetype_index.archetypes[target_arch_id];
                // SAFETY: batch insert sırasında bu sütuna tekil erişim.
                let col = unsafe { arch.get_column_mut(type_id) }.unwrap();
                for e in &group_entities {
                    let row = self.entity_locations[e.id() as usize].row as usize;
                    // SAFETY: every entity in this group is in `target_arch_id` (that is how the
                    // group was formed), so `row` is a live row of the column just taken, and
                    // `type_id` is `T`'s.
                    unsafe {
                        // Same-archetype overwrite: the slot already holds a live `T`.
                        // Assignment (`*ptr = ..`) drops the old value; `ptr::write` would
                        // leak it for any `T: Drop` (e.g. String/Vec/Handle re-asserted each
                        // frame → unbounded heap growth). Mirrors `add_component`'s path.
                        *(col.get_ptr(row) as *mut T) = component.clone();
                        col.ticks_ptr_mut().add(row).write(crate::archetype::ComponentTicks::new(self.tick));
                    }
                }
                self.run_hooks(type_id, |h, w| {
                    for e in &group_entities {
                        for hook in &mut h.on_set {
                            hook(w, *e);
                        }
                        for hook in &mut h.on_replace {
                            hook(w, *e);
                        }
                    }
                });
                tracing::debug!(
                    count = group_entities.len(),
                    archetype = target_arch_id,
                    "insert_batch: same-archetype overwrite group"
                );
                continue;
            }

            let migrated = group_entities.len();
            for e in &group_entities {
                let eid = e.id();
                let old_loc = self.entity_locations[eid as usize];
                let old_row = old_loc.row as usize;

                // Disjoint ödünç (source != target, yukarıda 422'de guard'landı).
                let (new_row, moved_eid) = {
                    let [old_arch, target_arch] = self
                        .archetype_index
                        .archetypes
                        .get_disjoint_mut([source_arch_id as usize, target_arch_id])
                        .expect("source and target archetype indices are distinct and in bounds");
                    // SAFETY: move_entity_to raw sütun kopyaları yapar; ödünçler disjoint.
                    unsafe { old_arch.move_entity_to(old_row, target_arch) }
                };

                if let Some(moved) = moved_eid {
                    self.entity_locations[moved as usize].row = old_row as u32;
                }

                {
                    let arch = &self.archetype_index.archetypes[target_arch_id];
                    // SAFETY: yeni ayrılan satır; sütuna tekil erişim.
                    let col = unsafe { arch.get_column_mut(type_id) }.unwrap();
                    // SAFETY: `new_row` was just allocated in this archetype, so the slot is
                    // uninitialised and `ptr::write` is correct; `type_id` is `T`'s, so the
                    // column's layout matches what is written.
                    unsafe {
                        std::ptr::write(col.get_ptr(new_row as usize) as *mut T, component.clone());
                        col.ticks_ptr_mut().add(new_row as usize).write(crate::archetype::ComponentTicks::new(self.tick));
                    }
                }

                self.entity_locations[eid as usize] = EntityLocation {
                    archetype_id: target_arch_id as u32,
                    row: new_row,
                };
                self.archetype_index.entity_archetype.insert(eid, target_arch_id);
            }

            self.run_hooks(type_id, |h, w| {
                for e in &group_entities {
                    for hook in &mut h.on_add { hook(w, *e); }
                    for hook in &mut h.on_set { hook(w, *e); }
                }
            });
            tracing::debug!(
                count = migrated,
                from = source_arch_id,
                to = target_arch_id,
                "insert_batch: migrated group to new archetype"
            );
        }
    }

    /// Batch component removal
    #[tracing::instrument(skip_all, name = "remove_batch")]
    pub fn remove_batch<T: Component>(&mut self, entities: &[Entity]) {
        if T::storage_type() == crate::component::StorageType::SparseSet {
            for &e in entities {
                self.remove_component::<T>(e);
            }
            return;
        }

        let type_id = TypeId::of::<T>();
        let mut groups: std::collections::HashMap<u32, Vec<Entity>> = std::collections::HashMap::new();

        for &e in entities {
            if !self.is_alive(e) { continue; }
            let loc = self.entity_locations[e.id() as usize];
            if !loc.is_valid() { continue; }
            groups.entry(loc.archetype_id).or_default().push(e);
        }

        for (source_arch_id, group_entities) in groups {
            let target_arch_id = match self.archetype_index.get_remove_component_target(
                group_entities[0].id(), type_id, &self.component_infos
            ) {
                Some(id) => id,
                None => continue,
            };

            if source_arch_id == target_arch_id as u32 {
                continue;
            }

            for e in &group_entities {
                let eid = e.id();
                let old_loc = self.entity_locations[eid as usize];

                // Disjoint ödünç (source != target, yukarıda 520'de guard'landı).
                let (new_row, moved_eid) = {
                    let [old_arch, target_arch] = self
                        .archetype_index
                        .archetypes
                        .get_disjoint_mut([source_arch_id as usize, target_arch_id])
                        .expect("source and target archetype indices are distinct and in bounds");
                    // SAFETY: move_entity_to raw sütun kopyaları yapar; ödünçler disjoint.
                    unsafe { old_arch.move_entity_to(old_loc.row as usize, target_arch) }
                };

                if let Some(moved) = moved_eid {
                    self.entity_locations[moved as usize].row = old_loc.row;
                }

                self.entity_locations[eid as usize] = EntityLocation {
                    archetype_id: target_arch_id as u32,
                    row: new_row,
                };
                self.archetype_index.entity_archetype.insert(eid, target_arch_id);
            }

            self.run_hooks(type_id, |h, w| {
                for e in &group_entities {
                    for hook in &mut h.on_remove { hook(w, *e); }
                }
            });
            tracing::debug!(
                count = group_entities.len(),
                from = source_arch_id,
                to = target_arch_id,
                "remove_batch: migrated group to new archetype"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::component::Component;
    use crate::entity::Entity;
    use crate::world::World;

    #[derive(Clone, PartialEq, Debug)]
    struct Pos(i32);
    impl Component for Pos {}
    #[derive(Clone, PartialEq, Debug)]
    struct Vel(i32);
    impl Component for Vel {}

    /// Migrating an entity between archetypes needs mutable access to two distinct
    /// archetypes stored in the same `Vec`. The old code took two
    /// `&mut archetypes[i] as *mut` — the second retag invalidated the first
    /// pointer's provenance, so using it was aliasing UB (caught by Miri). The fix
    /// uses `get_disjoint_mut`. This test drives the swap-remove `moved_eid`
    /// relocation path and must stay green under `cargo miri test` (see the Miri
    /// CI job) to fence the invariant.
    #[test]
    fn archetype_migration_preserves_all_components() {
        let mut world = World::new();
        let e0 = world.spawn();
        world.add_component(e0, Pos(0));
        let e1 = world.spawn();
        world.add_component(e1, Pos(1));
        let e2 = world.spawn();
        world.add_component(e2, Pos(2));

        // All three share archetype {Pos}. Adding Vel to the MIDDLE entity migrates
        // e1 to {Pos,Vel}; e2 swap-fills e1's vacated row in {Pos} (moved_eid path).
        world.add_component(e1, Vel(10));

        assert_eq!(world.borrow::<Pos>().get(e0.id()).unwrap().0, 0);
        assert_eq!(world.borrow::<Pos>().get(e1.id()).unwrap().0, 1);
        assert_eq!(world.borrow::<Pos>().get(e2.id()).unwrap().0, 2);
        assert_eq!(world.borrow::<Vel>().get(e1.id()).unwrap().0, 10);
        assert!(world.borrow::<Vel>().get(e0.id()).is_none());
        assert!(world.borrow::<Vel>().get(e2.id()).is_none());

        // Remove Vel → e1 migrates back to {Pos}; every entity's data stays intact.
        world.remove_component::<Vel>(e1);
        assert_eq!(world.borrow::<Pos>().get(e0.id()).unwrap().0, 0);
        assert_eq!(world.borrow::<Pos>().get(e1.id()).unwrap().0, 1);
        assert_eq!(world.borrow::<Pos>().get(e2.id()).unwrap().0, 2);
        assert!(world.borrow::<Vel>().get(e1.id()).is_none());
    }

    /// `add_bundle` on a reserved-but-unflushed id must not panic.
    ///
    /// `Commands::spawn` reserves the id immediately and only queues `flush_spawn`, so
    /// between those two moments the entity is `is_alive` with NO `entity_locations` slot —
    /// the same legal state `despawn_reserved_but_unflushed_entity_does_not_panic` covers.
    /// `add_bundle` indexed that slot raw and panicked with "index out of bounds".
    /// `add_component` on the same entity is a documented silent no-op; the bundle path now
    /// matches it instead of crashing.
    #[test]
    fn add_bundle_on_reserved_but_unflushed_entity_does_not_panic() {
        let mut world = World::new();
        let reserved = {
            let entities = world
                .get_resource::<crate::entity::allocator::Entities>()
                .expect("Entities resource");
            entities.reserve_entity()
        };
        assert!(world.is_alive(reserved), "a reserved entity is considered alive");

        world.add_bundle(reserved, (Pos(1), Vel(2))); // panicked here (entity_locations OOB)

        assert!(
            world.entity_component_types(reserved).is_empty(),
            "no storage exists for an unflushed id, so nothing can have been attached"
        );
        // The world must still be usable — the dropped bundle left no half-built archetype.
        let ok = world.spawn();
        world.add_component(ok, Pos(7));
        assert_eq!(world.borrow::<Pos>().get(ok.id()).unwrap().0, 7);
    }

    /// The recycled-id half of the same defect: here the `entity_locations` slot EXISTS
    /// (despawn wrote `EntityLocation::INVALID` into it), so the raw index did not trip —
    /// it read `row == u32::MAX` and handed that to `move_entity_to` as a source row, which
    /// is worse than a panic. The liveness/validity guard covers both shapes.
    #[test]
    fn add_bundle_on_recycled_unflushed_id_does_not_corrupt() {
        let mut world = World::new();
        let victim = world.spawn();
        world.add_component(victim, Pos(1));
        let survivor = world.spawn();
        world.add_component(survivor, Pos(2));
        world.despawn(victim); // frees the id; its location slot becomes INVALID

        // The allocator hands the freed id straight back, un-flushed.
        let recycled = {
            let entities = world
                .get_resource::<crate::entity::allocator::Entities>()
                .expect("Entities resource");
            entities.reserve_entity()
        };
        assert_eq!(recycled.id(), victim.id(), "id was recycled as expected");
        assert!(!world.entity_location(recycled.id()).is_valid());

        world.add_bundle(recycled, (Pos(3), Vel(4)));

        // Survivor untouched: no bogus row move happened.
        assert_eq!(world.borrow::<Pos>().get(survivor.id()).unwrap().0, 2);
        assert!(world.entity_component_types(recycled).is_empty());
    }

    // ── `add_bundle`'s drop discipline. These two live HERE rather than in `world/tests`
    // because this module is one of the six the CI Miri job runs
    // (`cargo miri test -p gizmo-core --lib world::component_ops`), and the whole point of that
    // job is to fence the archetype-migration unsafe surface. A migration test that Miri never
    // sees is a test the gate does not cover; measured 2026-08-25, these run under Tree Borrows
    // in 1.8 s, while the module they came from takes over ten minutes.

    // Regression: `add_bundle` must drop the value it replaces — and must NOT drop the hole
    // it fills. Those are the same line of code seen from two sides, which is the whole
    // difficulty, and it is why the three shapes below are one test rather than three.
    //
    // `Bundle::write_to_archetype` copies raw bytes, so it never drops what it overwrites: a
    // re-asserted component leaked whatever the old value owned. The obvious fix — make the
    // write an assignment, which drops first — was committed on 2026-08-24 and is UNSOUND.
    // `Archetype::move_entity_to` extends every target column by one row and leaves the
    // newly-added ones uninitialised for the write to fill, so from inside the write a hole
    // and a live value are indistinguishable: both sit below `len`. Assignment frees a garbage
    // pointer on the hole, and the ordinary `spawn()` + `add_bundle(e, (T,))` aborted with
    // `free(): invalid pointer` for any `T` owning a heap allocation.
    //
    // 337 tests stayed green under that, because every component they hand to `add_bundle` is
    // plain data whose drop glue is a no-op. So the counter here is a `Drop` impl, and the
    // component owns a `String`: the leak is an absence and needs something that would have
    // spoken, and the unsoundness needs an allocation real enough for the allocator to reject.
    #[test]
    fn add_bundle_drops_what_it_replaces_and_not_what_it_fills() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static DROPS: AtomicUsize = AtomicUsize::new(0);

        #[derive(Clone)]
        struct Loud(String);
        impl Drop for Loud {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::SeqCst);
            }
        }
        impl Component for Loud {}

        /// A second component, so a bundle can force an archetype MIGRATION while still
        /// re-asserting `Loud` — the third shape, and the one neither of the others reaches.
        #[derive(Clone)]
        struct Tag;
        impl Component for Tag {}

        let read = |w: &World, e: Entity| w.query_entity::<&Loud>(e.id()).map(|l| l.0.clone());

        DROPS.store(0, Ordering::SeqCst);
        let mut world = World::new();

        // ── 1. FILLING A HOLE. The entity has no `Loud`, so the bundle migrates it to a new
        // archetype and writes into the slot `move_entity_to` just allocated. There is nothing
        // there to drop, and dropping it is the unsoundness.
        let e = world.spawn();
        world.add_bundle(e, (Loud("first".into()),));
        assert_eq!(DROPS.load(Ordering::SeqCst), 0, "a first attach has nothing to drop");
        assert_eq!(read(&world, e).as_deref(), Some("first"));

        // ── 2. REPLACING IN PLACE. Same archetype, same row: the override path, where the slot
        // is live and the old value is the one that used to leak.
        world.add_bundle(e, (Loud("second".into()),));
        assert_eq!(
            DROPS.load(Ordering::SeqCst),
            1,
            "add_bundle overwrote a live component without dropping it"
        );
        assert_eq!(read(&world, e).as_deref(), Some("second"), "the fix must not lose the write");

        // ── 3. REPLACING ACROSS A MIGRATION. The bundle re-asserts `Loud` *and* adds `Tag`, so
        // the entity changes archetype: `Loud` arrives at the new row alive (copied by the
        // move) while `Tag` arrives as a hole. One write, two slots, opposite obligations —
        // this is the case that decides whether the fix understood the problem or just moved it.
        world.add_bundle(e, (Loud("third".into()), Tag));
        assert_eq!(
            DROPS.load(Ordering::SeqCst),
            2,
            "a component carried across a migration was overwritten without being dropped"
        );
        assert_eq!(read(&world, e).as_deref(), Some("third"));

        world.despawn(e);
        assert_eq!(DROPS.load(Ordering::SeqCst), 3, "the last value must be dropped by despawn");
    }

    // The `BundleExt::with` composition path, which has its own copy of `write_to_archetype`
    // and so its own copy of the hazard above. It is separated from the test before it because
    // what it pins is a KNOWN LIMITATION rather than a fixed bug: `with` appends rather than
    // substitutes, so a bundle naming one component twice writes that column twice, and the
    // second write overwrites a value this code path has no way to know is live. That value
    // leaks — exactly as `DynamicBundle`'s own documentation says it does.
    //
    // Asserting the leak rather than ignoring it is the point: closing it later should be a
    // decision, taken with this test going red and the rustdoc updated in the same change,
    // not a silent side effect. What is NOT negotiable is the part this asserts first — that
    // the duplicate write does not abort, and that the appended component is the survivor.
    #[test]
    fn a_with_composed_duplicate_leaks_the_inner_value_and_says_so() {
        use crate::component::BundleExt;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static DROPS: AtomicUsize = AtomicUsize::new(0);

        #[derive(Clone)]
        struct Loud(String);
        impl Drop for Loud {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::SeqCst);
            }
        }
        impl Component for Loud {}

        DROPS.store(0, Ordering::SeqCst);
        let mut world = World::new();
        let e = world.spawn();
        world.add_bundle(e, (Loud("inner".into()),).with(Loud("outer".into())));

        assert_eq!(
            world.query_entity::<&Loud>(e.id()).map(|l| l.0.clone()).as_deref(),
            Some("outer"),
            "`with` documents the appended component as the one that survives"
        );
        assert_eq!(
            DROPS.load(Ordering::SeqCst),
            0,
            "the inner value is documented as leaked — if this is now 1, the limitation was \
             closed and `DynamicBundle`'s rustdoc must stop claiming otherwise"
        );

        world.despawn(e);
        assert_eq!(DROPS.load(Ordering::SeqCst), 1, "the surviving value is dropped by despawn");
    }

}
