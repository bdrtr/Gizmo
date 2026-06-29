use super::WorldQuery;
use crate::world::World;
use std::marker::PhantomData;

// =========================================================================
// QUERY ITERATOR
// =========================================================================

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
                if unsafe { Q::filter_row(fetch, row, id, self.world.change_ref_tick) } {
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
            if let Some(fetch) = unsafe { Q::fetch_raw(self.world, arch, self.world.tick) } {
                let entities = arch.entities();
                for (row, &id) in entities.iter().enumerate().take(len) {
                    if unsafe { Q::filter_row(fetch, row, id, self.world.change_ref_tick) } {
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

            let fetch = match unsafe { Q::fetch_raw(self.world, arch, self.world.tick) } {
                Some(f) => f,
                None => continue,
            };

            let ids = unsafe { std::slice::from_raw_parts(arch.entities().as_ptr(), len) };
            let slice = unsafe { Q::get_slice(fetch, len) };

            return Some((ids, slice));
        }
        None
    }
}
