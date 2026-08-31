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
    /// **If a component's `Drop` or the `Bundle` impl panics**, the two branches answer
    /// differently and both answers are deliberate. A bundle that changes the entity's
    /// archetype abandons its staged row: the entity survives with **no components and no
    /// archetype row**, in the same reserved-but-unflushed state a `Commands::spawn` id sits in.
    /// A bundle that does not change it keeps the entity exactly where it was, with **every
    /// column holding a live value** — some the new one, some the old — because the old values
    /// are duplicated out of reach before the write rather than dropped before it. Whatever was
    /// not reached is leaked, which is the safe answer on a panic path. Neither branch can leave
    /// a slot that is counted and destroyed; that was possible until 2026-08-31 and needed no
    /// `unsafe` from the caller to reach.
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
            // SADECE ÜZERİNE YAZMA — no migration, and therefore no staged row to abandon.
            //
            // Every column this bundle is about to write already holds a LIVE value — same
            // archetype, same row — and `write_to_archetype` copies raw bytes, so it cannot
            // drop what it replaces. Dropping is therefore this function's job, and it has to
            // be: the same write also lands on the uninitialised holes `move_entity_to` leaves
            // behind (see the migration path below), where dropping would be undefined. From
            // inside the write the two are indistinguishable — both sit below the column's
            // `len` — so only the caller can tell them apart.
            //
            // WHAT THIS USED TO DO WAS DROP THEM FIRST. `drop_live_bundle_rows` walked the
            // bundle's types calling each column's drop glue, and the write refilled them one
            // statement later. Both are user code — a component's `Drop`, and a public trait
            // method any caller may implement — so a panic out of either left slots that
            // `entities` and the column's own `len` both still counted holding a destroyed or
            // uninitialised value, and the world's teardown ran the drop glue over them. No
            // `unsafe` from the caller, no contract broken, and no assertion fires along the
            // way: every column length still equals `entities.len()`, so
            // `debug_assert_consistent` passes and `assert_inv` passes.
            //
            // THE FIX IS TO PUT THE OLD VALUE OUT OF REACH BEFORE THE WRITE, NOT AFTER IT.
            // `Archetype::len` is `entities.len()` and every query bounds itself by it, so a
            // column longer than `entities` is a row nothing can see — the same fact the
            // migration branch's staged row rests on. Here it is used the other way round: each
            // of the bundle's columns gets a bytewise DUPLICATE of the live row appended above
            // `entities`, the bundle writes over the live copy exactly as it always did, and the
            // duplicate is then dropped by a `swap_remove_and_drop` of the last index, which
            // does its bookkeeping before it runs the drop glue.
            //
            // The panic algebra has only two outcomes, and no third:
            //   · a panic in the write — every live slot still holds either its original value
            //     or the new one, and `forget_rows_above` abandons the duplicates. Since they
            //     are duplicates, the slots the write did not reach lose nothing at all.
            //   · a panic in the drop loop — the bundle is fully applied, and the old values not
            //     yet reached are abandoned. A leak on a panic path is safe.
            // Neither ever leaves a slot both counted and destroyed, which is the only thing
            // that was ever undefined here.
            //
            // Writing the bundle into the duplicate row instead, and swapping afterwards, also
            // works and was tried. It is worse: it moves every same-archetype `add_bundle` onto
            // `write_to_archetype`'s append branch, which nothing else in the crate reaches from
            // this function, and it makes "an implementation must append for every type
            // `get_infos` names" a new obligation on out-of-crate implementors. This workspace
            // already ships five `Bundle` impls whose `write_to_archetype` body is empty. Writing
            // at the live row keeps tick stamping, duplicate-type leak semantics and ZST
            // handling byte-identical to today by construction rather than by argument.
            let loc = self.entity_locations[eid as usize];
            let row = loc.row as usize;
            let tick = self.tick;

            // Deduplicated by `TypeId`, in bundle order — the same rule `drop_live_bundle_rows`
            // uses, and load-bearing for the same reason: one column must be duplicated once and
            // dropped once. A bundle may legitimately name a type twice (`BundleExt::with`
            // appends rather than substitutes).
            let mut dedup: Vec<TypeId> = Vec::with_capacity(bundle_types.len());
            for &t in &bundle_types {
                if !dedup.contains(&t) {
                    dedup.push(t);
                }
            }

            // THE EVERY-FRAME CASE PAYS NOTHING. If no column this bundle writes has drop glue
            // there is no old value to drop, no user `Drop` can run, and the plain write was
            // already sound: whatever a panicking `Bundle` impl does, every slot ends holding a
            // valid value of the right type — the old one or the new one. This is the shape this
            // function's own rustdoc calls the "set these fields" call.
            if dedup.is_empty() || infos.iter().all(|i| i.drop_fn.is_none()) {
                let arch = &mut self.archetype_index.archetypes[target_arch_id];
                // SAFETY: as the general path below, minus the duplicates — `target_arch_id ==
                // old_arch_id`, so every one of the bundle's columns exists here and `row` is
                // the entity's live row in all of them.
                unsafe { bundle.write_to_archetype(arch, row, tick) };
                return;
            }

            let arch = &mut self.archetype_index.archetypes[target_arch_id];
            let base_len = arch.len();
            debug_assert!(row < base_len);
            #[cfg(debug_assertions)]
            arch.debug_assert_consistent();
            // Buy the duplicate row's capacity up front. Not needed for the sound path — every
            // panic below is caught — but it means a release build whose `Bundle` impl grew a
            // column behind our back still indexes inside the allocation.
            arch.reserve_row();

            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                // 1. Duplicate each of the bundle's live slots above `entities`. Raw memcpy of
                //    data and tick alike: no clone, no user code, no panic point.
                for &t in &dedup {
                    // SAFETY: `&mut Archetype` here, and each type is visited once, so no two
                    // `&mut Column` overlap.
                    if let Some(col) = unsafe { arch.get_column_mut(t) } {
                        // SAFETY: the archetype has a column for every bundle type (same
                        // archetype) and `row` is the entity's live row in it. Every duplicate
                        // made here is either dropped by step 3 or abandoned by
                        // `forget_rows_above` — exactly one of the two copies, which is
                        // `push_copy_of_row`'s obligation.
                        unsafe { col.push_copy_of_row(row) };
                    }
                }

                // 2. The write, at the LIVE row and unchanged from what it always was. Every
                //    column is now one row longer than `entities`, so `col.len() > row` holds
                //    for the bundle's columns exactly as it did before — the same overwrite
                //    branch, the same tick stamp, the same duplicate-type behaviour.
                //
                // SAFETY: `write_to_archetype`'s contract is that the bundle writes every column
                // it names at this row; the archetype was chosen for this bundle's component set
                // and `row` is the entity's live row.
                unsafe { bundle.write_to_archetype(arch, row, tick) };

                // 3. Drop the duplicates, which are the OLD values. `base_len` is the last
                //    index, so this is a pop-and-drop: `Column::swap_remove_and_drop` shortens
                //    ticks and data before it calls any drop glue, so a panicking `Drop` leaks
                //    the values not yet reached rather than leaving one counted and destroyed.
                for &t in &dedup {
                    // SAFETY: as step 1 — `&mut Archetype`, one visit per type, so no two
                    // `&mut Column` overlap.
                    let col = unsafe { arch.get_column_mut(t) };
                    let Some(col) = col else { continue };
                    debug_assert_eq!(
                        col.len(),
                        base_len + 1,
                        "add_bundle: a Bundle impl changed a column's length behind the write"
                    );
                    if col.len() != base_len + 1 {
                        continue;
                    }
                    // SAFETY: `base_len` is the duplicate pushed in step 1 and is the column's
                    // last index; it is out of `entities`' range, so nothing else can see it.
                    unsafe { col.swap_remove_and_drop(base_len) };
                }
            }));

            if let Err(payload) = outcome {
                // Nothing above ever touched `entities`, so `base_len` is still the entity count
                // and the only thing that can be out of step is a column one row long. A no-op
                // for every column that never grew, `entities` included. The entity keeps its
                // row, its location and its archetype entry — there is nothing else to undo.
                //
                // SAFETY: `base_len == entities.len()` and every column is `base_len` or
                // `base_len + 1` long, which is `forget_rows_above`'s precondition.
                unsafe {
                    self.archetype_index.archetypes[target_arch_id].forget_rows_above(base_len)
                };
                tracing::warn!(
                    entity = eid,
                    "add_bundle: a component `Drop` or `Bundle` impl panicked during a \
                     same-archetype overwrite; the entity keeps its row and every column holds \
                     a live value, but some old or new values were abandoned"
                );
                std::panic::resume_unwind(payload);
            }
            #[cfg(debug_assertions)]
            self.archetype_index.archetypes[target_arch_id].debug_assert_consistent();
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

        // THE ROW IS STAGED, NOT COMMITTED, AND THAT IS THE WHOLE FIX.
        //
        // Both statements further down call USER CODE — `drop_live_bundle_rows` runs the `Drop`
        // of every component that came across, and `write_to_archetype` is a public trait method
        // a caller implements. A panic out of either used to unwind past everything after it,
        // leaving the target archetype holding a row that `entities` counted and whose
        // bundle-added columns held uninitialised bytes. Teardown then ran drop glue over
        // garbage, which is worse than a double drop and needs no `unsafe` from the caller to
        // reach: `catch_unwind` is safe std and nothing here promises a component's `Drop` will
        // not panic.
        //
        // Neither of the two obvious repairs works. Taking the old values out to temporaries
        // fixes the drop but not the holes the migration itself leaves. Catching the unwind and
        // repairing needs to know how far `write_to_archetype` got, and nothing reports that.
        //
        // What works is not repairing. `Archetype::len` is `entities.len()` and every query
        // bounds itself by it, so a column longer than `entities` is a row nothing can reach —
        // only the drop paths follow a column's own length. So the row is written while it is
        // still invisible, and `commit_staged_row` — one store into capacity `stage_entity_into`
        // already reserved — is the single instant it exists. If anything panics first,
        // `forget_rows_above` puts every column's length back and the bytes above it are
        // abandoned. That recovery is IDENTICAL whatever was written, which is exactly why not
        // knowing is an acceptable answer. It leaks whatever those rows owned; a leak on a panic
        // path is safe, and it is the only sound thing to do with memory that is part live and
        // part uninitialised with no way to tell which.
        let base_len = self.archetype_index.archetypes[target_arch_id].len();
        let crate::archetype::Moved { moved, new_row, swapped: moved_eid } = {
            // İki archetype'ı FARKLI indekslerden disjoint ödünç al. Aynı Vec'ten
            // iki `&mut ...[i] as *mut` almak, ikinci retag ile ilk pointer'ın
            // provenance'ını geçersiz kılıp onu kullanınca UB üretiyordu (Miri
            // Stacked Borrows). `get_disjoint_mut` aliasing'siz iki &mut verir.
            let [old_arch, target_arch] = self
                .archetype_index
                .archetypes
                .get_disjoint_mut([old_arch_id, target_arch_id])
                .expect("old and target archetype indices are distinct and in bounds");
            // THIS PATH NEVER DETACHES, which is why it calls the stage directly and owes no
            // `drop_detached_rows`. `new_types` is the union of the old archetype's types and
            // the bundle's, so the target has a column for every one of the source's and the
            // stage's detach loop never runs. `move_entity_to` is the composition that does owe
            // the disposal, and it performs it after its own commit.
            debug_assert!(
                old_arch
                    .component_types()
                    .iter()
                    .all(|t| target_arch.has_component(*t)),
                "add_bundle: the target lost a source column, so the stage detached a row nobody \
                 will dispose of"
            );
            // SAFETY: raw sütun kopyaları yapar; ödünçler disjoint. The SOURCE is left fully
            // consistent by this call, so abandoning the target below does not have to undo it.
            unsafe { old_arch.stage_entity_into(old_loc.row as usize, target_arch) }
        };
        // `stage_entity_into` takes a ROW and moves whoever is in it. This says the
        // row was still the one this entity owns — see `Moved`.
        debug_assert_eq!(
            moved, eid,
            "migration moved entity {moved} while the caller meant {eid}: a stale row"
        );

        if let Some(moved) = moved_eid {
            self.entity_locations[moved as usize].row = old_loc.row;
        }

        // Out of the source and not yet in the target: for the length of the window below this
        // entity owns no row anywhere, and INVALID is the only true thing to say about it. It is
        // also not a new state — it is the "reserved but never flushed" one that `add_component`,
        // `remove_component`, `despawn` and this function's own entry guard all already handle.
        self.entity_locations[eid as usize] = EntityLocation::INVALID;

        let tick = self.tick;
        let arch = &mut self.archetype_index.archetypes[target_arch_id];
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            // The components the entity already carried came across the migration alive; the
            // ones the bundle is ADDING are holes. Drop the first group and only the first —
            // that distinction is the whole reason `live_after_move` was computed before the
            // move.
            //
            // SAFETY: every type in `live_after_move` was a column of the OLD archetype, so the
            // stage moved its value into `new_row` of the target's matching column and that slot
            // is live; the write below re-initialises each one.
            unsafe { drop_live_bundle_rows(arch, &live_after_move, new_row as usize) };
            // SAFETY: as above, for the row the stage just prepared in the target archetype.
            unsafe { bundle.write_to_archetype(arch, new_row as usize, tick) };
            // Everything is written. THIS is where the row starts existing.
            arch.commit_staged_row(eid);
        }));

        match outcome {
            Ok(()) => {
                self.entity_locations[eid as usize] = EntityLocation {
                    archetype_id: target_arch_id as u32,
                    row: new_row,
                };
                self.archetype_index.entity_archetype.insert(eid, target_arch_id);
            }
            Err(payload) => {
                // SAFETY: `base_len` is the target's entity count from before the stage, and the
                // stage only ever added rows above it. Nothing has committed, so `entities` is
                // still at `base_len` and every column is one longer.
                unsafe {
                    self.archetype_index.archetypes[target_arch_id].forget_rows_above(base_len)
                };
                // The entity is in no archetype now. Its location is already INVALID; this is
                // the other half of saying so.
                self.archetype_index.entity_archetype.remove(&eid);
                tracing::warn!(
                    entity = eid,
                    "add_bundle: a component `Drop` or `Bundle` impl panicked mid-migration; the \
                     entity's components are abandoned and it now owns no archetype row"
                );
                std::panic::resume_unwind(payload);
            }
        }
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
        let crate::archetype::Moved { moved, new_row, swapped: moved_eid } = {
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
        // `move_entity_to` takes a ROW and moves whoever is in it. This says the
        // row was still the one this entity owns — see `Moved`.
        debug_assert_eq!(
            moved, eid,
            "migration moved entity {moved} while the caller meant {eid}: a stale row"
        );

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
            // OWNERSHIP IS RELINQUISHED BEFORE THE CALL, not after it. `insert`'s overwrite
            // branch copies these bytes into the set before it drops the value they replace, so
            // a panic out of that `Drop` unwinds straight out of `add_component` — and a
            // `mem::forget` sitting on the line after the call would never run, leaving this
            // frame to drop a value the set already owns. `ManuallyDrop` is the form that cannot
            // get that wrong: nothing here drops `component` on any path.
            let component = std::mem::ManuallyDrop::new(component);
            let ptr = &*component as *const T as *const u8;
            // SAFETY: `ptr` points at a live `T`, whose layout is exactly the set's `info.layout`
            // (the set was created from `T`'s own `ComponentInfo`), and it is a stack local
            // rather than a pointer into `dense`. Ownership passes to the set here.
            unsafe { set.insert(eid, ptr, self.tick); }

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
            let old = {
                let arch = &self.archetype_index.archetypes[target_arch_id];
                // SAFETY: query/scheduler bu archetype sütununa ayrık erişimi garanti eder.
                let col = unsafe { arch.get_column_mut(type_id) }
                    .expect("component column missing in current archetype");
                // The old value comes back UNDROPPED and is dropped below, after the slot holds
                // the new value AND the tick is stamped. `*ptr = component` was already safe in
                // the first respect — rustc lowers an assignment to move-out/write/drop, which
                // was measured rather than assumed — but the stamp was the statement after it,
                // so a component whose `Drop` panicked left the row holding the new value under
                // the old timestamp, which no tick filter would ever report again.
                //
                // SAFETY: `type_id` is `T`'s, so the column's component type is `T`, and
                // `old_loc.row` is the entity's live row in it.
                unsafe { col.replace_typed::<T>(old_loc.row as usize, component, self.tick) }
            };
            // Before `run_hooks`, deliberately: `ReplaceHook` documents the old value as already
            // dropped by the time a hook sees the entity.
            drop(old);
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
        let crate::archetype::Moved { moved, new_row, swapped: moved_eid } = {
            let [old_arch, target_arch] = self
                .archetype_index
                .archetypes
                .get_disjoint_mut([old_arch_id, target_arch_id])
                .expect("old and target archetype indices must be distinct and in bounds");
            // SAFETY: move_entity_to raw sütun kopyaları yapar; ödünçler disjoint.
            unsafe { old_arch.move_entity_to(old_row, target_arch) }
        };
        // `move_entity_to` takes a ROW and moves whoever is in it. This says the
        // row was still the one this entity owns — see `Moved`.
        debug_assert_eq!(
            moved, eid,
            "migration moved entity {moved} while the caller meant {eid}: a stale row"
        );

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


        // `entity_location`, not `entity_locations[eid]`. An id RESERVED from the allocator but
        // never flushed — `Commands::spawn` hands one out before its queued `flush_spawn` runs —
        // is `is_alive`, so the check at the top of this function lets it through, yet it owns
        // no archetype row and may have no slot in the location table at all. Indexing raw
        // PANICKED with "index out of bounds" for such an id; the accessor answers `INVALID`,
        // which every branch below already handles. `add_bundle` documents the same entity at
        // its own guard. Fixed 2026-08-31, here and in `insert_batch`/`remove_batch`.
        let old_loc = self.entity_location(eid);

        // 1. Hedef archetype'ı belirle
        let target_arch_id_opt =
            self.archetype_index
                .get_remove_component_target(eid, type_id, &self.component_infos);
        let target_arch_id = match target_arch_id_opt {
            Some(id) => id,
            None => return, // Zaten yok, ya da entity'nin hiç satırı yok
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
        let crate::archetype::Moved { moved, new_row, swapped: moved_eid } = {
            let [old_arch, target_arch] = self
                .archetype_index
                .archetypes
                .get_disjoint_mut([old_loc.archetype_id as usize, target_arch_id])
                .expect("old and target archetype indices are distinct and in bounds");
            // SAFETY: move_entity_to raw sütun kopyaları yapar; ödünçler disjoint.
            unsafe { old_arch.move_entity_to(old_loc.row as usize, target_arch) }
        };
        // `move_entity_to` takes a ROW and moves whoever is in it. This says the
        // row was still the one this entity owns — see `Moved`.
        debug_assert_eq!(
            moved, eid,
            "migration moved entity {moved} while the caller meant {eid}: a stale row"
        );

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
    ///
    /// **A repeated entity in `entities` is applied once.** Duplicates are collapsed rather than
    /// rejected — a slice collected from two overlapping sources is an ordinary way to make one,
    /// and there is nothing a second application could usefully mean. Until 2026-08-31 they were
    /// neither collapsed nor rejected but *migrated twice*, which corrupted the location table
    /// silently; see the note in the grouping loop.
    ///
    /// Entities that are not alive, and ids reserved from the allocator but never flushed, are
    /// skipped.
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
        // Ids already placed in a group — see the note at the `insert` below.
        let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();

        for &e in entities {
            if !self.is_alive(e) { continue; }
            // Bounds-checked: `is_alive` is true for an id reserved but never flushed, which
            // owns no row and may have no slot at all. See `remove_component`.
            let loc = self.entity_location(e.id());
            if !loc.is_valid() { continue; }
            // A DUPLICATE IN THE CALLER'S SLICE IS NOT HARMLESS. The migration loop below reads
            // each member's location fresh, and the first pass writes that location to the
            // TARGET archetype — so a second pass hands a target row to `move_entity_to` on the
            // SOURCE archetype, dragging whichever entity now sits at that row into the target
            // and recording the new row under the duplicate's id. The dragged entity is then
            // listed in an archetype its own location does not name, and the duplicate's
            // location points at a row it does not occupy: every later read and write through
            // it lands on a different entity's data, silently. Nothing here forbids duplicates
            // and nothing about a slice collected from two overlapping sources suggests they
            // are forbidden, so they are collapsed rather than rejected. Fixed 2026-08-31.
            if !seen.insert(e.id()) { continue; }
            groups.entry(loc.archetype_id).or_default().push(e);
        }

        for (source_arch_id, group_entities) in groups {
            // Re-check the snapshot before using it. EVERY group was computed above, before any
            // of them was processed, and each group's hooks run before the next group starts —
            // `run_hooks` hands each hook a `&mut World` that `hooks.rs` documents as free to
            // spawn, despawn and mutate. So by the time this group is reached, a hook fired for
            // an EARLIER group may have despawned some of its members, or moved them to another
            // archetype by adding a component. Within a group there is no such window: its hooks
            // fire only after all of its entities have migrated.
            //
            // Three things went wrong without this, and they are why the fix is a filter here
            // rather than a bounds check at one of the three use sites. A despawned entity's
            // location is `EntityLocation::INVALID`, whose row is `u32::MAX`:
            //
            //   · the MIGRATION branch handed it to `move_entity_to`, which indexes
            //     `self.entities[source_row]` safely — a panic, and the mildest outcome;
            //   · the SAME-ARCHETYPE branch handed it to `Column::get_ptr`, whose bounds check
            //     is a `debug_assert` and therefore ABSENT in release: a write through a pointer
            //     `u32::MAX * size_of::<T>()` bytes past the column;
            //   · and with no crash at all, the target archetype is looked up from
            //     `group_entities[0]`, so a despawned FIRST member returned `None` and skipped
            //     the whole group — every live entity in it silently not getting the component.
            //
            // The `is_alive` test is not redundant with the location test: a hook that despawns
            // and then spawns can put a NEW entity in the freed id's slot, and a fresh entity
            // lands in the empty archetype — which is a real `source_arch_id` when the group is
            // the component-less one. Only the generation separates them.
            let group_entities: Vec<Entity> = group_entities
                .into_iter()
                .filter(|e| {
                    self.is_alive(*e)
                        && self
                            .entity_locations
                            .get(e.id() as usize)
                            .is_some_and(|loc| loc.archetype_id == source_arch_id)
                })
                .collect();
            if group_entities.is_empty() {
                continue;
            }

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
                    // CLONE FIRST, on its own line — the same rule the migration branch below
                    // states. `*place = component.clone()` happened to evaluate in this order
                    // too, but only because the language says the value operand goes first;
                    // written out, the ordering is the code's rather than the reference's.
                    let value = component.clone();
                    let row = self.entity_locations[e.id() as usize].row as usize;
                    // Same-archetype overwrite: the slot already holds a live `T`, so the old
                    // value has to be dropped and `ptr::write` alone would leak it (a
                    // String/Vec/Handle re-asserted every frame is unbounded heap growth). It
                    // comes back undropped and is dropped once the slot and the tick are both
                    // final — the tick being the half the assignment left after the user code.
                    // See `Column::replace_typed`, and `add_component`'s twin above.
                    //
                    // SAFETY: every entity in this group is in `target_arch_id` (that is how the
                    // group was formed), so `row` is a live row of the column just taken, and
                    // `type_id` is `T`'s.
                    let old = unsafe { col.replace_typed::<T>(row, value, self.tick) };
                    drop(old);
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

                // CLONE FIRST. `T::clone` is user code, and calling it between the migration and
                // the location write below would run it in the one window where this entity has
                // a row in the target archetype and a location still naming the source — the
                // same inconsistency `despawn` was reordered to close. It cannot reach `&mut
                // World` without the caller's own `unsafe`, so this is not a soundness fix; it
                // is the window not existing rather than being hard to reach. Hoisting it out of
                // the loop instead would be wrong: one clone per entity is the point.
                let value = component.clone();

                let old_loc = self.entity_locations[eid as usize];
                let old_row = old_loc.row as usize;

                // Disjoint ödünç (source != target, yukarıda 422'de guard'landı).
                let crate::archetype::Moved { moved, new_row, swapped: moved_eid } = {
                    let [old_arch, target_arch] = self
                        .archetype_index
                        .archetypes
                        .get_disjoint_mut([source_arch_id as usize, target_arch_id])
                        .expect("source and target archetype indices are distinct and in bounds");
                    // SAFETY: move_entity_to raw sütun kopyaları yapar; ödünçler disjoint.
                    unsafe { old_arch.move_entity_to(old_row, target_arch) }
        };
        // `move_entity_to` takes a ROW and moves whoever is in it. This says the
        // row was still the one this entity owns — see `Moved`.
        debug_assert_eq!(
            moved, eid,
            "migration moved entity {moved} while the caller meant {eid}: a stale row"
        );

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
                        std::ptr::write(col.get_ptr(new_row as usize) as *mut T, value);
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

    /// Batch component removal — one archetype lookup per source group instead of per entity.
    ///
    /// Same contract as [`World::insert_batch`] on the input: **a repeated entity is applied
    /// once**, and entities that are not alive or were reserved but never flushed are skipped.
    /// Removing a component an entity does not have is a no-op for that entity, not an error.
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
        // Ids already placed in a group — see the note at the `insert` below.
        let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();

        for &e in entities {
            if !self.is_alive(e) { continue; }
            // Bounds-checked: `is_alive` is true for an id reserved but never flushed, which
            // owns no row and may have no slot at all. See `remove_component`.
            let loc = self.entity_location(e.id());
            if !loc.is_valid() { continue; }
            // A DUPLICATE IN THE CALLER'S SLICE IS NOT HARMLESS. The migration loop below reads
            // each member's location fresh, and the first pass writes that location to the
            // TARGET archetype — so a second pass hands a target row to `move_entity_to` on the
            // SOURCE archetype, dragging whichever entity now sits at that row into the target
            // and recording the new row under the duplicate's id. The dragged entity is then
            // listed in an archetype its own location does not name, and the duplicate's
            // location points at a row it does not occupy: every later read and write through
            // it lands on a different entity's data, silently. Nothing here forbids duplicates
            // and nothing about a slice collected from two overlapping sources suggests they
            // are forbidden, so they are collapsed rather than rejected. Fixed 2026-08-31.
            if !seen.insert(e.id()) { continue; }
            groups.entry(loc.archetype_id).or_default().push(e);
        }

        for (source_arch_id, group_entities) in groups {
            // Same re-check as `insert_batch`, for the same reason and with the same window: the
            // groups are a snapshot taken before any hook ran, and this function's `on_remove`
            // hooks fire between groups. See the long note there. Only the migration branch is
            // reachable here — an entity whose target archetype equals its source is skipped
            // below — so the symptom was the `move_entity_to` panic rather than the wild write.
            let group_entities: Vec<Entity> = group_entities
                .into_iter()
                .filter(|e| {
                    self.is_alive(*e)
                        && self
                            .entity_locations
                            .get(e.id() as usize)
                            .is_some_and(|loc| loc.archetype_id == source_arch_id)
                })
                .collect();
            if group_entities.is_empty() {
                continue;
            }

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
                let crate::archetype::Moved { moved, new_row, swapped: moved_eid } = {
                    let [old_arch, target_arch] = self
                        .archetype_index
                        .archetypes
                        .get_disjoint_mut([source_arch_id as usize, target_arch_id])
                        .expect("source and target archetype indices are distinct and in bounds");
                    // SAFETY: move_entity_to raw sütun kopyaları yapar; ödünçler disjoint.
                    unsafe { old_arch.move_entity_to(old_loc.row as usize, target_arch) }
        };
        // `move_entity_to` takes a ROW and moves whoever is in it. This says the
        // row was still the one this entity owns — see `Moved`.
        debug_assert_eq!(
            moved, eid,
            "migration moved entity {moved} while the caller meant {eid}: a stale row"
        );

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

    /// A hook that despawns during `insert_batch` must not leave a later group holding a
    /// dead entity's row.
    ///
    /// `insert_batch` computes EVERY group up front and then processes them one at a time,
    /// running that group's `on_add`/`on_set` hooks before the next group starts. `run_hooks`
    /// hands each hook a `&mut World` and `hooks.rs` documents it as free to spawn, despawn and
    /// mutate — so a hook fired for group A can despawn a member of group B, which has not been
    /// touched yet. Nothing revalidated the snapshot, and a despawned entity's location is
    /// `EntityLocation::INVALID`, whose row is `u32::MAX`.
    ///
    /// Both branches were reachable and they fail differently, which is why the fix is a filter
    /// rather than a bounds check in one of them:
    ///
    /// * the MIGRATION branch feeds `u32::MAX` to `move_entity_to`, which indexes
    ///   `self.entities[source_row]` safely and panics;
    /// * the SAME-ARCHETYPE branch feeds it to `Column::get_ptr`, which is unchecked — a wild
    ///   pointer written through, not a panic.
    ///
    /// There is a third symptom with no crash at all: `get_add_component_target` is asked about
    /// `group_entities[0]`, so if the despawned entity happened to be first, it returned `None`
    /// and the whole group was skipped — every live entity in it silently not getting the
    /// component.
    ///
    /// **Both groups take the migration branch here on purpose.** Group order comes from a
    /// `HashMap`, so it differs run to run; making both groups fail the same way is what makes
    /// the test deterministic, and choosing the branch that panics rather than the one that
    /// writes through a wild pointer keeps the pre-fix failure observable instead of undefined.
    /// The same-archetype branch is covered by the sibling test below.
    ///
    /// Found by review 2026-08-25.
    #[test]
    fn a_hook_despawning_during_insert_batch_cannot_strand_a_later_group() {
        use std::sync::Mutex;

        #[derive(Clone, PartialEq, Debug)]
        struct Tag(i32);
        impl Component for Tag {}

        static VICTIMS: Mutex<Vec<Entity>> = Mutex::new(Vec::new());

        let mut world = World::new();

        // Two source archetypes, so `insert_batch` sees two groups. Neither carries `Tag`, so
        // both take the migration branch whichever order the map yields them in.
        let a1 = world.spawn();
        world.add_component(a1, Pos(1));
        let a2 = world.spawn();
        world.add_component(a2, Pos(2));
        let b1 = world.spawn();
        world.add_component(b1, Vel(1));
        let b2 = world.spawn();
        world.add_component(b2, Vel(2));

        // One victim per group, so whichever group is processed first, the hook it fires
        // despawns a member of the group that has NOT been processed.
        *VICTIMS.lock().unwrap() = vec![a2, b2];

        world.register_component_type::<Tag>();
        world.register_on_add::<Tag>(Box::new(|w, _e| {
            let victims: Vec<Entity> = std::mem::take(&mut *VICTIMS.lock().unwrap());
            for v in victims {
                w.despawn(v);
            }
        }));

        world.insert_batch(&[a1, a2, b1, b2], Tag(7));

        // The survivors got the component…
        assert_eq!(world.query_entity::<&Tag>(a1.id()).map(|t| t.0), Some(7), "a1 lost its insert");
        assert_eq!(world.query_entity::<&Tag>(b1.id()).map(|t| t.0), Some(7), "b1 lost its insert");
        // …and kept what they already had, so no row was moved out from under them.
        assert_eq!(world.query_entity::<&Pos>(a1.id()).map(|p| p.0), Some(1));
        assert_eq!(world.query_entity::<&Vel>(b1.id()).map(|v| v.0), Some(1));
        // The despawned ones are gone rather than half-written.
        assert!(!world.is_alive(a2) && !world.is_alive(b2), "the victims should be despawned");
        assert_eq!(world.query::<&Tag>().unwrap().iter().count(), 2, "exactly the survivors carry Tag");
    }

    /// The same hazard on `insert_batch`'s SAME-ARCHETYPE branch, plus the symptom that never
    /// crashes at all.
    ///
    /// Group order comes from a `HashMap` and differs run to run, so this test is built to fail
    /// in BOTH orders — and they fail differently, which is the point:
    ///
    /// * **overwrite group first** — its hooks despawn `a1`, the FIRST member of the migration
    ///   group. `get_add_component_target` is asked about `group_entities[0]`, gets `None` for a
    ///   dead entity, and `continue`s the whole group: `a2` silently never receives the
    ///   component. No panic, no warning, just a missing write — which is why `a2` exists and is
    ///   asserted on. An earlier version of this test had no `a2`, and passed 7 times in 20.
    /// * **migration group first** — its hooks despawn `b2`, and the overwrite loop then feeds
    ///   `b2`'s `u32::MAX` row to `Column::get_ptr`, whose bounds check is a `debug_assert`:
    ///   a panic here, a write through a wild pointer in release.
    ///
    /// Both are the same defect and the same one-line fix, but only the second one crashes, so a
    /// test that watched for a crash would have graded the fix on a coin flip.
    #[test]
    fn a_hook_despawning_during_insert_batch_cannot_strand_an_overwrite_group() {
        use std::sync::Mutex;

        #[derive(Clone, PartialEq, Debug)]
        struct Mark(i32);
        impl Component for Mark {}

        static VICTIMS: Mutex<Vec<Entity>> = Mutex::new(Vec::new());

        let mut world = World::new();

        // Migration group (`Pos`, no `Mark`): `a1` is the victim AND first in the group, which is
        // what exposes the silent-skip symptom; `a2` is the survivor whose write proves it.
        let a1 = world.spawn();
        world.add_component(a1, Pos(1));
        let a2 = world.spawn();
        world.add_component(a2, Pos(2));
        // Overwrite group (already carries `Mark`): `b1` survives, `b2` is the victim.
        let b1 = world.spawn();
        world.add_component(b1, Vel(1));
        world.add_component(b1, Mark(0));
        let b2 = world.spawn();
        world.add_component(b2, Vel(2));
        world.add_component(b2, Mark(0));

        *VICTIMS.lock().unwrap() = vec![a1, b2];

        let despawn_victims = |w: &mut World, _e: Entity| {
            let victims: Vec<Entity> = std::mem::take(&mut *VICTIMS.lock().unwrap());
            for v in victims {
                w.despawn(v);
            }
        };
        world.register_on_add::<Mark>(Box::new(despawn_victims));
        world.register_on_set::<Mark>(Box::new(despawn_victims));

        world.insert_batch(&[a1, a2, b1, b2], Mark(9));

        assert!(!world.is_alive(a1) && !world.is_alive(b2), "the victims should be despawned");
        assert_eq!(
            world.query_entity::<&Mark>(a2.id()).map(|m| m.0),
            Some(9),
            "the migration group was skipped wholesale because its first member was despawned"
        );
        assert_eq!(
            world.query_entity::<&Mark>(b1.id()).map(|m| m.0),
            Some(9),
            "the surviving member of the overwrite group lost its write"
        );
        // Neither survivor had a row moved out from under it.
        assert_eq!(world.query_entity::<&Pos>(a2.id()).map(|p| p.0), Some(2));
        assert_eq!(world.query_entity::<&Vel>(b1.id()).map(|v| v.0), Some(1));
    }

    /// The same hazard on `remove_batch`, which shares the snapshot-then-process shape and runs
    /// its `on_remove` hooks in the same place — between groups.
    ///
    /// Only the migration branch is reachable here (a group whose target archetype equals its
    /// source is skipped), so before the fix this was the `move_entity_to` panic rather than the
    /// wild write. Covered separately anyway, because the fix is a separate edit: a single
    /// function fixed and its twin forgotten is the failure mode a shared explanation invites.
    #[test]
    fn a_hook_despawning_during_remove_batch_cannot_strand_a_later_group() {
        use std::sync::Mutex;

        #[derive(Clone, PartialEq, Debug)]
        struct Doomed(i32);
        impl Component for Doomed {}

        static VICTIMS: Mutex<Vec<Entity>> = Mutex::new(Vec::new());

        let mut world = World::new();

        // Two source archetypes, both carrying `Doomed`, so both groups migrate when it goes.
        let a1 = world.spawn();
        world.add_component(a1, Pos(1));
        world.add_component(a1, Doomed(1));
        let a2 = world.spawn();
        world.add_component(a2, Pos(2));
        world.add_component(a2, Doomed(2));
        let b1 = world.spawn();
        world.add_component(b1, Vel(1));
        world.add_component(b1, Doomed(3));
        let b2 = world.spawn();
        world.add_component(b2, Vel(2));
        world.add_component(b2, Doomed(4));

        // One victim per group, so whichever group runs first strands a member of the other.
        *VICTIMS.lock().unwrap() = vec![a2, b2];

        world.register_on_remove::<Doomed>(Box::new(|w, _e| {
            let victims: Vec<Entity> = std::mem::take(&mut *VICTIMS.lock().unwrap());
            for v in victims {
                w.despawn(v);
            }
        }));

        world.remove_batch::<Doomed>(&[a1, a2, b1, b2]);

        assert!(!world.is_alive(a2) && !world.is_alive(b2), "the victims should be despawned");
        assert!(
            world.query_entity::<&Doomed>(a1.id()).is_none(),
            "a1 kept the component remove_batch was asked to take"
        );
        assert!(
            world.query_entity::<&Doomed>(b1.id()).is_none(),
            "b1 kept the component remove_batch was asked to take"
        );
        // The survivors keep everything else, so no row was moved out from under them.
        assert_eq!(world.query_entity::<&Pos>(a1.id()).map(|p| p.0), Some(1));
        assert_eq!(world.query_entity::<&Vel>(b1.id()).map(|v| v.0), Some(1));
        assert_eq!(world.query::<&Doomed>().unwrap().iter().count(), 0);
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

    // ── A component whose `Drop` panics, in the three places that overwrite a LIVE slot. ──
    //
    // The class: an operation that drops the old value in place and refills the slot one
    // statement later. A panic out of that drop leaves the slot destroyed while the column's
    // `len` and the archetype's `entities` both still count it, and the world's teardown then
    // runs the drop glue over it a second time. It needs no `unsafe` from the caller and breaks
    // no documented contract — `catch_unwind` is safe std and nothing here promises a
    // component's `Drop` will not panic.
    //
    // WHAT THESE TESTS LOOK LIKE WHEN THEY FAIL is worth knowing before running them. The
    // payload owns a `String`, so the second drop is an invalid free rather than a count that is
    // one too high: on an unfixed tree the process ABORTS at teardown (glibc "double free
    // detected"), taking the whole test binary with it, and under Miri it is reported as a use
    // after free at the read. A payload whose drop only bumps a counter would make the defect
    // visible to the counter and INVISIBLE to Miri — measured, not assumed, when the migration
    // branch was fixed (see `an_abandoned_migration_never_drops_the_column_it_had_not_written_yet`).
    //
    // Every fixture declares its component types and its statics INSIDE the test function. The
    // harness runs tests in parallel, and a shared `ARMED` flag can be consumed by another
    // test's ordinary drop — which turns "the fixture is wrong if not" into a random failure.

    /// `add_bundle`'s SAME-ARCHETYPE branch: re-asserting components an entity already has.
    ///
    /// The bundle's types are already the archetype's, so no migration happens and the staged
    /// row the migration branch abandons on a panic does not exist here. The old values are
    /// duplicated above `entities` instead, the write lands on the live row exactly as it always
    /// did, and the duplicates are dropped afterwards — so the entity ends up with the bundle
    /// applied and one destructor's worth of leak, rather than with a corpse in its row.
    #[test]
    fn a_panicking_drop_during_a_same_archetype_bundle_overwrite_leaves_no_corpse() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

        static PAYLOAD_DROPS: AtomicU32 = AtomicU32::new(0);
        static ARMED: AtomicBool = AtomicBool::new(false);

        /// Owns a heap allocation. This is the value whose slot used to be left destroyed and
        /// counted, so its second drop is a genuine invalid free.
        #[derive(Clone)]
        struct Payload(String);
        impl Component for Payload {}
        impl Drop for Payload {
            fn drop(&mut self) {
                PAYLOAD_DROPS.fetch_add(1, SeqCst);
                // Reads the buffer. `is_empty()` would only read the inline length field, which
                // stays perfectly initialised on a corpse — the read has to reach the heap for
                // Miri to have anything to report.
                let _ = std::hint::black_box(self.0.as_bytes().first().copied());
            }
        }

        /// Owns nothing, and is the one that panics — so the value abandoned on the panic path
        /// leaks no memory and the test stays clean without `-Zmiri-ignore-leaks`.
        #[derive(Clone)]
        struct Fuse(&'static str);
        impl Component for Fuse {}
        impl Drop for Fuse {
            fn drop(&mut self) {
                // Disarmed before it panics: the point of the test is what happens to the OTHER
                // column, and a second panic during the unwind would abort instead.
                if self.0 == "BOOM" && ARMED.swap(false, SeqCst) {
                    panic!("Fuse::drop");
                }
            }
        }

        let mut world = World::new();
        let e = world.spawn();
        // First attach: a migration, nothing to drop. Establishes archetype {Payload, Fuse}.
        world.add_bundle(e, (Payload("old".into()), Fuse("BOOM")));
        PAYLOAD_DROPS.store(0, SeqCst);
        ARMED.store(true, SeqCst);

        // Same type set → same archetype → the branch under test. `Payload`'s old value is
        // dropped cleanly, then `Fuse`'s old value panics.
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.add_bundle(e, (Payload("new".into()), Fuse("quiet")));
        }));
        assert!(unwound.is_err(), "the drop was supposed to panic; the fixture is wrong if not");

        assert_eq!(
            PAYLOAD_DROPS.load(SeqCst),
            1,
            "exactly one `Payload` was disposed of: the old one. Two means the bundle's own \
             value was released by the unwind because the write never happened."
        );
        assert_eq!(
            world.query_entity::<&Payload>(e.id()).map(|p| p.0.clone()).as_deref(),
            Some("new"),
            "the bundle was applied before any destructor ran, so the row holds the new value"
        );

        let before = PAYLOAD_DROPS.load(SeqCst);
        drop(world);
        assert_eq!(
            PAYLOAD_DROPS.load(SeqCst),
            before + 1,
            "teardown must drop the one live value exactly once"
        );
    }

    /// The other half of the same branch: the panic comes out of `Bundle::write_to_archetype`
    /// rather than out of a `Drop`.
    ///
    /// Implementing a trait's `unsafe fn` is something safe code may do, and nothing in
    /// `write_to_archetype`'s contract says it must return. Half a bundle therefore lands, and
    /// the columns it did not reach keep their ORIGINAL values — which is only true because the
    /// old values were duplicated before the write instead of dropped before it.
    #[test]
    fn a_bundle_that_gives_up_mid_write_leaves_every_same_archetype_slot_live() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};

        static PAYLOAD_DROPS: AtomicU32 = AtomicU32::new(0);

        #[derive(Clone)]
        struct Payload(String);
        impl Component for Payload {}
        impl Drop for Payload {
            fn drop(&mut self) {
                PAYLOAD_DROPS.fetch_add(1, SeqCst);
                let _ = std::hint::black_box(self.0.as_bytes().first().copied());
            }
        }
        #[derive(Clone)]
        struct Tail(String);
        impl Component for Tail {}

        /// Names two components and writes one.
        struct HalfWritten(Payload, Tail);
        impl crate::component::Bundle for HalfWritten {
            fn get_infos() -> Vec<crate::archetype::ComponentInfo> {
                vec![
                    crate::archetype::ComponentInfo::of::<Payload>(),
                    crate::archetype::ComponentInfo::of::<Tail>(),
                ]
            }
            unsafe fn write_to_archetype(
                self,
                arch: &mut crate::archetype::Archetype,
                row: usize,
                tick: u32,
            ) {
                // SAFETY: forwarded verbatim to the single-component impl, at the row and
                // archetype this bundle was handed.
                unsafe { crate::component::Bundle::write_to_archetype(self.0, arch, row, tick) };
                // Bound so it is released by the unwind rather than left as a dead field — the
                // ordinary fate of a bundle member an implementation never gets to.
                let _unwritten = self.1;
                panic!("HalfWritten::write_to_archetype");
            }
        }

        let mut world = World::new();
        let e = world.spawn();
        world.add_bundle(e, (Payload("old".into()), Tail("tail".into())));
        PAYLOAD_DROPS.store(0, SeqCst);

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.add_bundle(e, HalfWritten(Payload("new".into()), Tail("unused".into())));
        }));
        assert!(unwound.is_err(), "the bundle was supposed to panic; the fixture is wrong if not");

        assert_eq!(
            PAYLOAD_DROPS.load(SeqCst),
            0,
            "the write was abandoned before any duplicate was dropped, so nothing was disposed \
             of yet — a 1 here means the old value was destroyed before the write, which is the \
             defect"
        );
        assert_eq!(
            world.query_entity::<&Payload>(e.id()).map(|p| p.0.clone()).as_deref(),
            Some("new"),
            "the write reached `Payload` before giving up"
        );
        assert_eq!(
            world.query_entity::<&Tail>(e.id()).map(|t| t.0.clone()).as_deref(),
            Some("tail"),
            "the write never reached `Tail`, so it must still hold its original value — not a \
             hole left by a drop that happened up front"
        );

        drop(world);
        assert_eq!(
            PAYLOAD_DROPS.load(SeqCst),
            1,
            "teardown drops the live `Payload` once; the abandoned old value is leaked, which \
             is the safe answer on a panic path"
        );
    }

    /// `add_component`'s same-archetype overwrite — the `*ptr = component` shape, and the one
    /// place in this family where the defect turned out **not to be there**.
    ///
    /// `docs/ENGINE.md` §3 listed these assignments as the third open shape, on the reading that
    /// `*ptr = value` drops the old value and then moves the new one in. It does not.
    /// **Measured on rustc 1.98**, against the tree with the assignment still in place: after a
    /// panic out of the old value's `Drop` the slot already held `"new"` and the drop count was
    /// **1**, not 2 — so rustc lowers the assignment to move-out, write, drop-the-temporary,
    /// which is `ptr::replace` by another name and is panic-safe. The corpse the doc predicted
    /// was never there, and the two tests that pin this shape are the only ones in this group
    /// that stayed GREEN when the fix was reverted.
    ///
    /// What *was* wrong is the line after it: the tick stamp came after the user code, so a
    /// panicking `Drop` left the row holding the new value under the OLD timestamp, invisible to
    /// `Changed<T>` and `Added<T>` for good. That is what this test fails on without the fix,
    /// and it is why the site was still worth rewriting — `Column::replace_typed` makes the
    /// ordering the code's own rather than a property of the compiler's drop elaboration, and
    /// puts the stamp in front of the drop.
    #[test]
    fn a_panicking_drop_during_an_overwrite_leaves_the_new_value_installed() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

        static DROPS: AtomicU32 = AtomicU32::new(0);
        static ARMED: AtomicBool = AtomicBool::new(false);

        #[derive(Clone)]
        struct Loud(String);
        impl Component for Loud {}
        impl Drop for Loud {
            fn drop(&mut self) {
                DROPS.fetch_add(1, SeqCst);
                let _ = std::hint::black_box(self.0.as_bytes().first().copied());
                if self.0 == "old" && ARMED.swap(false, SeqCst) {
                    panic!("Loud::drop");
                }
            }
        }

        let mut world = World::new();
        world.tick = 3;
        let e = world.spawn();
        world.add_component(e, Loud("old".into()));
        DROPS.store(0, SeqCst);
        ARMED.store(true, SeqCst);
        world.tick = 7;

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.add_component(e, Loud("new".into()));
        }));
        assert!(unwound.is_err(), "the drop was supposed to panic; the fixture is wrong if not");

        assert_eq!(
            DROPS.load(SeqCst),
            1,
            "only the old value was disposed of. A 2 is the new value being released by the \
             unwind because the write never happened."
        );
        assert_eq!(
            world.query_entity::<&Loud>(e.id()).map(|l| l.0.clone()).as_deref(),
            Some("new"),
            "the slot is refilled before the old value's `Drop` gets control"
        );
        // THE HALF THAT WAS ACTUALLY BROKEN. The stamp used to be the statement after the
        // assignment, so the panic skipped it and the row kept tick 3 while holding "new".
        assert_eq!(
            tick_of::<Loud>(&world, e),
            (7, 7),
            "the row must be stamped before the old value's `Drop` can unwind past the stamp"
        );

        drop(world);
        assert_eq!(DROPS.load(SeqCst), 2, "teardown drops the live value exactly once");
    }

    /// `(added, changed)` of `T`'s row for `entity`, read straight out of the column.
    ///
    /// Change detection is the only thing that can see whether the tick stamp of an overwrite
    /// happened before or after the user code that may unwind past it, and no query filter
    /// exposes the raw pair.
    fn tick_of<T: Component>(world: &World, entity: Entity) -> (u32, u32) {
        let loc = world.entity_location(entity.id());
        let arch = &world.archetype_index.archetypes[loc.archetype_id as usize];
        let col = arch
            .get_column(std::any::TypeId::of::<T>())
            .expect("component column missing");
        let t = col.ticks[loc.row as usize];
        (t.added, t.changed)
    }

    /// `insert_batch`'s same-archetype group — `add_component`'s twin, once per member.
    ///
    /// The same measurement applies: the assignment was already panic-safe, and the stamp was
    /// not. The victim is the MIDDLE entity, so the members before it are already written when
    /// the panic happens and the ones after it are never reached; all three rows must hold a
    /// live value whichever side of the panic they are on, and the victim's own row must carry
    /// the tick of the write that landed in it.
    #[test]
    fn a_panicking_drop_in_an_insert_batch_group_leaves_every_row_live() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

        static DROPS: AtomicU32 = AtomicU32::new(0);
        static ARMED: AtomicBool = AtomicBool::new(false);

        #[derive(Clone)]
        struct Batched(String);
        impl Component for Batched {}
        impl Drop for Batched {
            fn drop(&mut self) {
                DROPS.fetch_add(1, SeqCst);
                let _ = std::hint::black_box(self.0.as_bytes().first().copied());
                if self.0 == "BOOM" && ARMED.swap(false, SeqCst) {
                    panic!("Batched::drop");
                }
            }
        }

        let mut world = World::new();
        world.tick = 3;
        let a = world.spawn();
        let b = world.spawn();
        let c = world.spawn();
        world.add_component(a, Batched("a".into()));
        world.add_component(b, Batched("BOOM".into()));
        world.add_component(c, Batched("c".into()));
        DROPS.store(0, SeqCst);
        ARMED.store(true, SeqCst);
        world.tick = 7;

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.insert_batch(&[a, b, c], Batched("new".into()));
        }));
        assert!(unwound.is_err(), "the drop was supposed to panic; the fixture is wrong if not");

        let read = |w: &World, e: Entity| w.query_entity::<&Batched>(e.id()).map(|v| v.0.clone());
        assert_eq!(read(&world, a).as_deref(), Some("new"), "written before the panic");
        assert_eq!(
            read(&world, b).as_deref(),
            Some("new"),
            "the victim's own row: refilled before its old value's `Drop` ran"
        );
        assert_eq!(read(&world, c).as_deref(), Some("c"), "never reached");
        assert_eq!(
            tick_of::<Batched>(&world, b),
            (7, 7),
            "the victim's row is stamped before its old value's `Drop` can unwind past the stamp"
        );
        assert_eq!(tick_of::<Batched>(&world, c), (3, 3), "never reached, so never restamped");

        // "a"'s old value and "BOOM" itself. "c" was never touched, and the template value and
        // the clone destined for "c" are released by the unwind.
        let after_unwind = DROPS.load(SeqCst);
        drop(world);
        assert_eq!(
            DROPS.load(SeqCst) - after_unwind,
            3,
            "teardown drops exactly the three live values"
        );
    }

    /// `ComponentSparseSet::insert`'s overwrite branch, reached through `add_component`.
    ///
    /// Sparse storage has no archetype row to stage or abandon, and the four arrays that
    /// describe it never disagreed here — the slot was simply destroyed in place with everything
    /// still counting it. The incoming value goes to the end of `dense` first now, and the
    /// swap-remove that puts it in place is also what puts the old value out of range before its
    /// destructor runs.
    #[test]
    fn a_panicking_drop_during_a_sparse_overwrite_leaves_the_new_value_installed() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

        static DROPS: AtomicU32 = AtomicU32::new(0);
        static ARMED: AtomicBool = AtomicBool::new(false);

        #[derive(Clone)]
        struct SparseLoud(String);
        impl Component for SparseLoud {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }
        impl Drop for SparseLoud {
            fn drop(&mut self) {
                DROPS.fetch_add(1, SeqCst);
                let _ = std::hint::black_box(self.0.as_bytes().first().copied());
                if self.0 == "old" && ARMED.swap(false, SeqCst) {
                    panic!("SparseLoud::drop");
                }
            }
        }

        let mut world = World::new();
        // A second entry, so the set is not a single row and the swap-remove has to permute.
        let keep = world.spawn();
        world.add_component(keep, SparseLoud("keep".into()));
        let e = world.spawn();
        world.add_component(e, SparseLoud("old".into()));
        DROPS.store(0, SeqCst);
        ARMED.store(true, SeqCst);

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.add_component(e, SparseLoud("new".into()));
        }));
        assert!(unwound.is_err(), "the drop was supposed to panic; the fixture is wrong if not");

        // The set's four arrays must still describe each other. `dense` grew by one for the
        // incoming value and the swap-remove put it back; a panic between the two would have
        // left them disagreeing.
        let set = &world.sparse_sets[&std::any::TypeId::of::<SparseLoud>()];
        assert_eq!(set.dense.len(), set.ticks.len());
        assert_eq!(set.dense.len(), set.entities.len());
        for (id, &row) in set.sparse.iter().enumerate() {
            if row != u32::MAX {
                assert!((row as usize) < set.dense.len(), "sparse[{id}] names row {row}");
                assert_eq!(set.entities[row as usize], id as u32);
            }
        }

        assert_eq!(
            DROPS.load(SeqCst),
            1,
            "only the old value was disposed of. A 2 is the new value being dropped by the \
             unwind while the set already owns a copy of it — a double free, and what happens \
             if the caller relinquishes ownership after the call instead of before it."
        );
        assert_eq!(
            world.query_entity::<&SparseLoud>(e.id()).map(|l| l.0.clone()).as_deref(),
            Some("new"),
            "the incoming value is in the set before the old one's destructor runs"
        );

        let before = DROPS.load(SeqCst);
        drop(world);
        assert_eq!(DROPS.load(SeqCst), before + 2, "teardown drops \"new\" and \"keep\"");
    }

    // ── The world-level half of the same family: `entities` is the visibility switch. ──
    //
    // These live here rather than beside the operations they test because the Miri job filters
    // by module path, and `world::component_ops` is one of the names it lists. Each of them
    // arms one destructor, catches the unwind, and asserts a length or an index invariant —
    // deterministic in both profiles, unlike the out-of-bounds read the invariant prevents.

    /// Reads every archetype and asserts that no column is out of step with its entity list.
    ///
    /// This is the property the whole family is about: `Archetype::len()` is `entities.len()`,
    /// queries bound themselves by it, and `query::fetch` then indexes the column with no check
    /// in either profile.
    fn assert_columns_match_entities(world: &World, context: &str) {
        for (idx, arch) in world.archetype_index.archetypes.iter().enumerate() {
            let n = arch.entities().len();
            for t in arch.component_types() {
                let col = arch.get_column(t).expect("column for a type the archetype has");
                assert_eq!(
                    col.len(),
                    n,
                    "{context}: archetype {idx} has {n} entities and a column of {}",
                    col.len()
                );
            }
        }
    }

    /// `World::despawn` — the removal every frame runs, and the one with the widest reach.
    ///
    /// One component type on purpose: `Archetype`'s column order is sorted `TypeId`, not
    /// declaration order, so a two-column fixture cannot say which column the panic lands in.
    #[test]
    fn a_panicking_drop_during_a_despawn_leaves_the_world_indexable() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

        static ARMED: AtomicBool = AtomicBool::new(false);
        static DROPS: AtomicU32 = AtomicU32::new(0);

        #[derive(Clone)]
        struct Fuse(&'static str);
        impl Component for Fuse {}
        impl Drop for Fuse {
            fn drop(&mut self) {
                DROPS.fetch_add(1, SeqCst);
                if self.0 == "BOOM" && ARMED.swap(false, SeqCst) {
                    panic!("Fuse::drop");
                }
            }
        }

        let mut world = World::new();
        let victim = world.spawn();
        let survivor = world.spawn();
        world.add_component(victim, Fuse("BOOM"));
        world.add_component(survivor, Fuse("quiet"));
        // The victim must NOT be the last row, so the survivor has to be swap-moved into its
        // place — that fixup is the part that used to run after the destructor.
        assert_eq!(world.entity_location(victim.id()).row, 0);
        DROPS.store(0, SeqCst);
        ARMED.store(true, SeqCst);

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.despawn(victim);
        }));
        assert!(unwound.is_err(), "the drop was supposed to panic; the fixture is wrong if not");

        assert_columns_match_entities(&world, "after a panicking despawn");
        // The survivor's recorded row has to be the one it actually occupies. Left where it was
        // — after the removal — the unwind stranded it naming a row the archetype no longer has,
        // which `query_entity` and `get_component_ptr` index with no bound check at all.
        let loc = world.entity_location(survivor.id());
        assert_eq!(loc.row, 0, "the survivor's location must name the row it was moved into");
        assert_eq!(
            world.query_entity::<&Fuse>(survivor.id()).map(|f| f.0),
            Some("quiet"),
            "reading the survivor must give the survivor"
        );
        // The victim owns no row and no location, but its id is still `is_alive`: `Entities::free`
        // is further down `despawn` and the unwind escaped before it. That is the same
        // reserved-but-unflushed state a `Commands::spawn` id sits in, which every path already
        // handles, and it is unchanged by this fix — recorded here so it is a documented outcome
        // rather than a surprise for the next reader.
        assert!(!world.entity_location(victim.id()).is_valid());
        assert!(world.query_entity::<&Fuse>(victim.id()).is_none());
    }

    /// `ComponentSparseSet::remove`, reached through `World::remove_component`.
    ///
    /// A sparse set has no archetype row; its four parallel arrays are the invariant, and a
    /// panicking destructor used to leave `sparse` naming rows that `dense` no longer had.
    #[test]
    fn a_panicking_drop_during_a_sparse_remove_leaves_the_set_consistent() {
        use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

        static ARMED: AtomicBool = AtomicBool::new(false);

        #[derive(Clone)]
        struct SparseFuse(&'static str);
        impl Component for SparseFuse {
            fn storage_type() -> crate::component::StorageType {
                crate::component::StorageType::SparseSet
            }
        }
        impl Drop for SparseFuse {
            fn drop(&mut self) {
                if self.0 == "BOOM" && ARMED.swap(false, SeqCst) {
                    panic!("SparseFuse::drop");
                }
            }
        }

        let mut world = World::new();
        let a = world.spawn();
        let victim = world.spawn();
        let tail = world.spawn();
        world.add_component(a, SparseFuse("a"));
        world.add_component(victim, SparseFuse("BOOM"));
        world.add_component(tail, SparseFuse("tail"));
        ARMED.store(true, SeqCst);

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.remove_component::<SparseFuse>(victim);
        }));
        assert!(unwound.is_err(), "the drop was supposed to panic; the fixture is wrong if not");

        // THE ASSERTION. Before the reorder, `dense` had shortened itself and the other three
        // had not: `sparse[victim]` still named a row now holding `tail`'s value, and
        // `sparse[tail]` named `dense.len()` — one past the end, indexed with no bounds check by
        // `get_ptr` and by `query::fetch`'s sparse branch.
        let set = &world.sparse_sets[&std::any::TypeId::of::<SparseFuse>()];
        assert_eq!(set.dense.len(), set.ticks.len(), "dense/ticks desync");
        assert_eq!(set.dense.len(), set.entities.len(), "dense/entities desync");
        for (id, &row) in set.sparse.iter().enumerate() {
            if row == u32::MAX {
                continue;
            }
            assert!((row as usize) < set.dense.len(), "sparse[{id}] names row {row}");
            assert_eq!(set.entities[row as usize], id as u32, "sparse[{id}] names a stranger");
        }
        // The removal completed as far as anything can see.
        assert!(world.query_entity::<&SparseFuse>(victim.id()).is_none());
        assert_eq!(world.query_entity::<&SparseFuse>(tail.id()).map(|f| f.0), Some("tail"));
    }

    /// `World::clear_entities` over a world that has already survived one caught panic.
    ///
    /// **This one is green before the fix as well, and it is here on purpose.** It is the check
    /// that the family closes: `clear_entities` is the recovery a caller reaches for after
    /// catching a destructor panic, so it must not itself be the thing that turns a survivable
    /// state into a double free. It was measured against the unfixed tree and against the fixed
    /// one, and it passes on both — which is the claim, not an oversight.
    ///
    /// It also fences a design choice that is otherwise invisible. `Archetype::clear` takes one
    /// row count PER COLUMN rather than one `entities.len()` for the whole archetype. With every
    /// site in this commit closed, no reachable operation can leave a column shorter than
    /// `entities`, so the two are equivalent today — but the shared count would be a double free
    /// the first time anything reintroduced a tear, and this is the function whose whole job is
    /// to survive one.
    #[test]
    fn clearing_a_world_after_a_caught_panicking_despawn_drops_nothing_twice() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

        static ARMED: AtomicBool = AtomicBool::new(false);
        static LIVE: AtomicU32 = AtomicU32::new(0);

        #[derive(Clone)]
        struct Counted(&'static str);
        impl Component for Counted {}
        impl Drop for Counted {
            fn drop(&mut self) {
                LIVE.fetch_sub(1, SeqCst);
                if self.0 == "BOOM" && ARMED.swap(false, SeqCst) {
                    panic!("Counted::drop");
                }
            }
        }

        let mut world = World::new();
        LIVE.store(0, SeqCst);
        for tag in ["BOOM", "b", "c"] {
            let e = world.spawn();
            LIVE.fetch_add(1, SeqCst);
            world.add_component(e, Counted(tag));
        }
        let victim = world.entity(0).expect("entity 0 is alive");
        ARMED.store(true, SeqCst);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.despawn(victim);
        }));
        assert!(unwound.is_err(), "the drop was supposed to panic; the fixture is wrong if not");
        // The victim's own value is gone either way — its destructor ran, panic and all.
        assert_eq!(LIVE.load(SeqCst), 2, "two values are still live at this point");

        world.clear_entities();
        // Every remaining value dropped exactly once. A negative wrap here is the double free.
        assert_eq!(LIVE.load(SeqCst), 0, "clear_entities dropped a value twice, or missed one");
        assert_columns_match_entities(&world, "after clear_entities");
    }

    /// `World::spawn_batch` — the addition side, where the row used to become visible before any
    /// column held data for it.
    #[test]
    fn a_bundle_that_panics_mid_spawn_batch_leaves_no_row_without_data() {
        #[derive(Clone)]
        struct Head(u32);
        impl Component for Head {}
        #[derive(Clone)]
        struct Tail(#[allow(dead_code)] u32);
        impl Component for Tail {}

        /// Names two components and writes one, then gives up — which safe code may do:
        /// nothing in `write_to_archetype`'s contract says an implementation has to return.
        struct HalfWritten(Head, Tail);
        impl crate::component::Bundle for HalfWritten {
            fn get_infos() -> Vec<crate::archetype::ComponentInfo> {
                vec![
                    crate::archetype::ComponentInfo::of::<Head>(),
                    crate::archetype::ComponentInfo::of::<Tail>(),
                ]
            }
            fn apply(self, world: &mut World, entity: Entity) {
                world.add_component(entity, self.0);
                world.add_component(entity, self.1);
            }
            unsafe fn write_to_archetype(
                self,
                arch: &mut crate::archetype::Archetype,
                row: usize,
                tick: u32,
            ) {
                if self.0 .0 == 0 {
                    // The first bundle writes properly, so the batch discovers its archetype and
                    // takes the fast path at all.
                    // SAFETY: forwarded verbatim to the single-component impls.
                    unsafe {
                        crate::component::Bundle::write_to_archetype(self.0, arch, row, tick);
                        crate::component::Bundle::write_to_archetype(self.1, arch, row, tick);
                    }
                    return;
                }
                // SAFETY: as above, for the one column this implementation does write.
                unsafe { crate::component::Bundle::write_to_archetype(self.0, arch, row, tick) };
                let _unwritten = self.1;
                panic!("HalfWritten::write_to_archetype");
            }
        }

        let mut world = World::new();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = world.spawn_batch(vec![
                HalfWritten(Head(0), Tail(0)),
                HalfWritten(Head(1), Tail(1)),
            ]);
        }));
        assert!(unwound.is_err(), "the bundle was supposed to panic; the fixture is wrong if not");

        // THE ASSERTION. The row the panic interrupted was never pushed into `entities`, so the
        // archetype counts only the rows that are complete. Pushing the id first — which is what
        // this used to do — left `entities` one longer than every column it had not reached, and
        // an ordinary `world.query::<&Head>()` then read past the end of one.
        assert_columns_match_entities(&world, "after a panicking spawn_batch");
    }
}
