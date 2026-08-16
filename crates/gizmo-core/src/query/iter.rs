use super::WorldQuery;
use crate::world::World;
use std::marker::PhantomData;

// =========================================================================
// QUERY ITERATOR
// =========================================================================

/// Row-by-row iterator over a [`Query`](super::Query), yielding `(entity_id, item)`.
///
/// The `u32` is the raw entity index **without** the generation counter — the same value as
/// [`Entity::id`](crate::entity::Entity::id). It is only usable with the `u32`-keyed accessors,
/// which cannot tell a live entity from a despawned slot that has been reused.
///
/// Visits archetypes in ascending archetype index and rows in ascending row order, skipping
/// rows rejected by `filter_row` — so the number of items yielded can be smaller than
/// [`Query::len`](super::Query::len), which counts rows without filtering. Empty archetypes, and
/// archetypes for which the fetch cannot be resolved, are skipped silently rather than ending
/// iteration.
///
/// The order is reproducible for an identical sequence of world operations, but it carries no
/// meaning — it is neither spawn nor id order (see [`Query`](super::Query)). Do not rely on it
/// as a sort.
///
/// `Iterator::for_each` is overridden with a flatter archetype-at-a-time loop that avoids the
/// resumable per-row state machine of `next`. The override always restarts from the first
/// matching archetype and does not consult how far `next` has already advanced, so call it on
/// a *fresh* iterator: on a partially consumed one it replays the items already yielded.
pub struct QueryIter<'a, 'w, Q: WorldQuery> {
    pub(super) world: &'a World,
    pub(super) archetype_indices: &'a [usize],
    pub(super) current_arch_idx: usize,
    pub(super) current_row: usize,
    pub(super) current_fetch: Option<Q::Fetch<'a>>,
    pub(super) _marker: PhantomData<Q>,
    pub(super) _marker_w: PhantomData<&'w ()>,
}

impl<'a, 'w, Q: WorldQuery> Iterator for QueryIter<'a, 'w, Q>
where
    'w: 'a,
{
    type Item = (u32, Q::Item<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.current_arch_idx >= self.archetype_indices.len() {
                return None;
            }

            let arch_idx = self.archetype_indices[self.current_arch_idx];
            let arch = &self.world.archetype_index.archetypes[arch_idx];

            let fetch = match self.current_fetch {
                Some(f) => f,
                None => {
                    // SAFETY: `arch` is one of `matching_archetypes` — an archetype this query
                    // was matched against — and it is borrowed from the world for `'w`, so the
                    // fetch pointer stays valid for as long as this iterator does, which is
                    // `fetch_raw`'s contract. The no-aliasing half was settled once at
                    // construction by `Query::new`'s `check_aliasing`.
                    match unsafe { Q::fetch_raw(self.world, arch, self.world.tick) } {
                        Some(f) => {
                            self.current_fetch = Some(f);
                            self.current_row = 0;
                            f
                        }
                        None => {
                            // Bu archetype bu query'ye uymuyor, sonrakine geç
                            self.current_arch_idx += 1;
                            continue;
                        }
                    }
                }
            };

            if self.current_row < arch.len() {
                let row = self.current_row;
                self.current_row += 1;
                let id = arch.entities()[row];
                // SAFETY: `row < arch.len()` was just checked, and `fetch` came from this same
                // archetype — so both calls address a live row of the columns it points at.
                if unsafe { Q::filter_row(fetch, row, id, self.world.change_ref_tick) } {
                    // SAFETY: as above, and the row passed the filter, so the item exists.
                    let item = unsafe { Q::get_item(fetch, row, id) };
                    return Some((id, item));
                }
                continue;
            }

            self.current_fetch = None;
            self.current_arch_idx += 1;
        }
    }

    #[inline(always)]
    fn for_each<F>(self, mut f: F)
    where
        Self: Sized,
        F: FnMut(Self::Item),
    {
        for &arch_idx in self.archetype_indices {
            let arch = &self.world.archetype_index.archetypes[arch_idx];
            let len = arch.len();
            if len == 0 {
                continue;
            }
            // SAFETY: a matched archetype borrowed from the world, as in `next` above; aliasing
            // was checked once by `Query::new`.
            if let Some(fetch) = unsafe { Q::fetch_raw(self.world, arch, self.world.tick) } {
                let entities = arch.entities();
                for (row, &id) in entities.iter().enumerate().take(len) {
                    // SAFETY: `take(len)` keeps `row` inside the archetype, and `fetch` is this
                    // archetype's.
                    if unsafe { Q::filter_row(fetch, row, id, self.world.change_ref_tick) } {
                        // SAFETY: as above; the row passed the filter.
                        let item = unsafe { Q::get_item(fetch, row, id) };
                        f((id, item));
                    }
                }
            }
        }
    }
}

// =========================================================================
// QUERY CHUNKS ITERATOR
// =========================================================================

/// Archetype-at-a-time iterator yielding `(&[u32], Q::Slice)` — the entity ids of one whole
/// archetype paired with that archetype's component data as a contiguous slice, for bulk or
/// SIMD-friendly work.
///
/// At most one item per matching archetype. Within an item the two slices have equal length
/// and are index-aligned: `ids[i]` owns element `i` of the component slice. Empty archetypes
/// are skipped, and so is any archetype whose fetch cannot be resolved — so the iterator can
/// yield nothing even when archetypes matched. Do not rely on the order items come out in.
///
/// There is **no per-row filtering** here — a chunk is all-or-nothing. Queries that need it
/// (`Changed`, `Added`, `Or`, sparse `With`/`Without`) are rejected up front by
/// [`Query::iter_chunks`](super::Query::iter_chunks), which panics when the iterator is created.
/// A plain `&T`/`Mut<T>` operand on a `SparseSet`-stored component declares no row filter and so
/// slips past that check; its data is not contiguous, so it fails later instead — panicking on
/// the first chunk pulled from this iterator once the world actually holds a sparse-set store
/// for `T`.
///
/// Change detection follows the *operands*, not the constructor that produced the iterator.
/// Every `Mut<T>` operand conservatively stamps the current tick onto every row of each chunk
/// it yields, written or not, because a raw `&mut [T]` cannot say which elements were touched.
/// `&T` operands and filters stamp nothing. So for a mixed `Q` such as `(&A, Mut<B>)` only
/// `B`'s ticks move, and a chunk query with no `Mut<T>` in it marks nothing even when it came
/// from [`Query::iter_chunks_mut`](super::Query::iter_chunks_mut).
pub struct QueryChunksIter<'a, 'w, Q: WorldQuery> {
    pub(super) world: &'a World,
    pub(super) archetype_indices: &'a [usize],
    pub(super) current_arch_idx: usize,
    pub(super) _marker: PhantomData<&'w Q>,
}

impl<'a, 'w, Q: WorldQuery> Iterator for QueryChunksIter<'a, 'w, Q>
where
    'w: 'a,
{
    type Item = (&'a [u32], Q::Slice<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        while self.current_arch_idx < self.archetype_indices.len() {
            let arch_idx = self.archetype_indices[self.current_arch_idx];
            self.current_arch_idx += 1;

            let arch = &self.world.archetype_index.archetypes[arch_idx];
            let len = arch.len();
            if len == 0 {
                continue;
            }

            // SAFETY: matched archetype, borrowed from the world for `'w` — see `next`.
            let fetch = match unsafe { Q::fetch_raw(self.world, arch, self.world.tick) } {
                Some(f) => f,
                None => continue,
            };

            // SAFETY: `len == arch.len()` and the entity vector is exactly that long, so this
            // rebuilds the slice the archetype already owns, with its lifetime tied to `'w`.
            let ids = unsafe { std::slice::from_raw_parts(arch.entities().as_ptr(), len) };
            // SAFETY: `fetch` is this archetype's and `len` is its row count, which is what
            // `get_slice` requires; it panics for SparseSet storage rather than fabricating one.
            let slice = unsafe { Q::get_slice(fetch, len) };

            return Some((ids, slice));
        }
        None
    }
}
