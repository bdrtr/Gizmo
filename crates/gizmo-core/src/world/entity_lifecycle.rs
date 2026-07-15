use super::{Entities, World};
use crate::archetype::EntityLocation;
use crate::entity::Entity;

use std::any::TypeId;

impl World {
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

    /// Bir `Bundle`'ı tek seferde spawn eder — entity oluşturur ve tüm
    /// bileşenleri ekler.
    ///
    /// ```ignore
    /// let player = world.spawn_bundle(MeshBundle {
    ///     mesh: renderer.create_cube(),
    ///     material: Material::pbr(Color::BLUE, 0.5, 0.0),
    ///     name: "Oyuncu",
    ///     ..default()
    /// });
    /// ```
    pub fn spawn_bundle<B: crate::component::Bundle>(&mut self, bundle: B) -> Entity {
        let entity = self.spawn();
        bundle.apply(self, entity);
        entity
    }

    pub fn flush_spawn(&mut self, entity: Entity) {
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
    }

    // Eski A3 bridge ve rebuild metodları silindi (Archetype artık authoritative).

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

    /// Derin kopyalama (O(1) Prefab Splicing) işlemi.
    /// Var olan bir Entity'nin bulunduğu archetype tablosunda tamamen bitişik olarak N adet yeni kopyasını çıkarır.
    pub fn clone_entity(&mut self, source_id: u32, count: usize) -> Option<Vec<Entity>> {
        if count == 0 {
            return Some(Vec::new());
        }

        let loc = self.entity_locations.get(source_id as usize).copied()?;
        if !loc.is_valid() {
            return None;
        }

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

        Some(new_entities)
    }

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

        let loc = self.entity_locations[first_entity.id() as usize];
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

        entities.into_iter()
    }

    /// Tüm entityleri temizler.
    pub fn clear_entities(&mut self) {
        self.archetype_index.clear_entities();
        self.entity_locations.clear();
        self.entities_to_despawn.clear();

        // Entities resource'unu temizle (allocator state)
        if let Some(entities) = self.get_resource::<Entities>() {
            entities.clear();
        }
    }

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

                // Re-fetch location safely after hooks might have mutated state
                let loc = self.entity_locations[id as usize];
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

            {
                let entities = self
                    .get_resource::<Entities>()
                    .expect("Entities resource not initialized");
                entities.free(e);
            }

            self.archetype_index.entity_archetype.remove(&id);
            // Guard: a reserved-but-unflushed entity has no `entity_locations` slot
            // (see the bounds-safe read above); its location is already effectively
            // INVALID, so there is nothing to clear.
            if let Some(slot) = self.entity_locations.get_mut(id as usize) {
                *slot = EntityLocation::INVALID;
            }
        }
        self.is_despawning = false;
    }

    /// Hafızadaki boşlukları sıkıştırır ve kullanılmayan (boş) Archetype tablolarını silerek
    /// RAM'i ve sistem performansını ilk baştaki defregmante (temiz) haline getirir.
    /// Yükleme (Loading) ekranlarında veya düşük yoğunluklu anlarda çağrılması önerilir.
    pub fn compact(&mut self) {
        // 1. Önce eski, kullanılmayan boş archetype'ları silelim (GC)
        self.archetype_index
            .gc_empty_archetypes(&mut self.entity_locations);

        // 2. Kalan archetype'ların kapasitelerini minimuma indirelim (Shrink To Fit)
        for arch in &mut self.archetype_index.archetypes {
            arch.shrink_to_fit();
        }

        self.archetype_index.archetypes.shrink_to_fit();

        // 3. World seviyesindeki listeleri daraltalım.
        self.entities_to_despawn.shrink_to_fit();
        self.entity_locations.shrink_to_fit();

        let entities = self
            .get_resource::<Entities>()
            .expect("Entities resource not initialized");
        let mut state = entities.state.lock().unwrap_or_else(|e| e.into_inner());
        state.generations.shrink_to_fit();
        state.free_ids.shrink_to_fit();
        state.free_set.shrink_to_fit();
    }

    pub fn despawn_by_id(&mut self, id: u32) {
        if let Some(entity) = self.get_entity(id) {
            self.despawn(entity);
        }
    }

    /// `C` bileşenine sahip TÜM entity'leri despawn eder; silinen sayısını döndürür.
    /// "Bu sahneyi/grubu topluca temizle" (ör. bölüm yeniden yüklenince tüm `LevelEntity`'ler)
    /// ya da "tüm mermileri sil" gibi yaygın işlemleri tek satıra indirir — geliştirici artık
    /// `Vec<Entity>` tutup elle döngüyle silmez. Bir marker bileşeni ekle, bunu çağır.
    ///
    /// ```ignore
    /// #[derive(Clone, Copy)] struct Bullet;
    /// gizmo_core::impl_component!(Bullet);
    /// // …mermilere Bullet ekle…
    /// let cleared = world.despawn_all_with::<Bullet>();
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
        n
    }

    /// Yaşayan (despawn olmamış) tüm Entity'leri döndüren iterator.
    /// Uyarı: İterasyon boyunca Entities mutex kilidi tutulur!
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

    #[inline]
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.get_resource::<Entities>()
            .expect("Entities resource not initialized")
            .is_alive(entity)
    }

    /// Entity üzerindeki tüm bileşenlerin TypeId'lerini döndürür.
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

    /// Entity'nin archetype konumunu döndürür — O(1) lookup.
    #[inline]
    pub fn entity_location(&self, entity_id: u32) -> EntityLocation {
        let loc_idx = entity_id as usize;
        if loc_idx < self.entity_locations.len() {
            self.entity_locations[loc_idx]
        } else {
            EntityLocation::INVALID
        }
    }

    /// Toplam yaşayan entity sayısı
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
