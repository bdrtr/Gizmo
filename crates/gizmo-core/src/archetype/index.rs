use super::{Archetype, EntityLocation};
use crate::archetype::ComponentInfo;
use std::any::TypeId;
use std::collections::HashMap;

pub(crate) struct ArchetypeIndex {
    /// Sorted component set → archetype index
    pub(crate) set_to_id: HashMap<Vec<TypeId>, usize>,
    /// Archetype storage tables (the actual data is here)
    pub(crate) archetypes: Vec<Archetype>,
    /// Entity ID → current archetype index
    pub(crate) entity_archetype: HashMap<u32, usize>,
    /// Query result cache — TypeId of the query types → matching archetype indices
    query_cache: HashMap<TypeId, Vec<usize>>,
    /// Validity of the cache — becomes true during component addition/removal
    cache_dirty: bool,
}

impl ArchetypeIndex {
    pub(crate) fn new() -> Self {
        // İlk archetype: boş component seti (yeni spawn edilen entity'ler)
        let empty_arch = Archetype::new(0, &[]);
        let mut set_to_id = HashMap::new();
        set_to_id.insert(Vec::new(), 0);

        Self {
            set_to_id,
            archetypes: vec![empty_arch],
            entity_archetype: HashMap::new(),
            query_cache: HashMap::new(),
            cache_dirty: false,
        }
    }

    /// Entity spawn — add to the empty archetype
    pub(crate) fn on_spawn(&mut self, entity_id: u32) {
        self.archetypes[0].push_entity(entity_id);
        self.entity_archetype.insert(entity_id, 0);
    }

    /// Component addition — move the entity to the new archetype
    /// NOTE: The data move is performed by the World, this method only tracks the structural transition.
    #[allow(dead_code)]
    pub(crate) fn get_or_create_archetype(&mut self, infos: &[ComponentInfo]) -> usize {
        let mut new_types: Vec<TypeId> = infos.iter().map(|i| i.type_id).collect();
        new_types.sort();
        
        if let Some(&id) = self.set_to_id.get(&new_types) {
            id
        } else {
            let id = self.archetypes.len();
            self.archetypes.push(Archetype::new(id as u32, infos));
            self.set_to_id.insert(new_types, id);
            id
        }
    }

    pub(crate) fn get_add_component_target(
        &mut self,
        entity_id: u32,
        type_id: TypeId,
        component_infos: &HashMap<TypeId, ComponentInfo>,
    ) -> Option<usize> {
        let old_arch_id = *self.entity_archetype.get(&entity_id)?;

        // Cache kontrolü (Edge)
        if let Some(edge) = self.archetypes[old_arch_id].get_edge(type_id) {
            if let Some(target) = edge.add {
                return Some(target as usize);
            }
        }

        // Yeni component setini hesapla
        let mut new_types = self.archetypes[old_arch_id].sorted_component_types();
        if let Err(pos) = new_types.binary_search(&type_id) {
            new_types.insert(pos, type_id);
        } else {
            return Some(old_arch_id); // Zaten var
        }

        // Hedef archetype'ı bul veya oluştur
        let new_arch_id = if let Some(&id) = self.set_to_id.get(&new_types) {
            id
        } else {
            let id = self.archetypes.len();
            // Yeni archetype oluşturmak için ComponentInfo gerekir
            let mut infos = Vec::new();
            for &t in &new_types {
                infos.push(
                    component_infos
                        .get(&t)
                        .cloned()
                        .unwrap_or_else(|| ComponentInfo::of_type_id(t)),
                );
            }
            self.archetypes.push(Archetype::new(id as u32, &infos));
            self.set_to_id.insert(new_types, id);
            id
        };

        // Edge cache güncelle
        self.archetypes[old_arch_id].set_add_edge(type_id, new_arch_id as u32);
        self.archetypes[new_arch_id].set_remove_edge(type_id, old_arch_id as u32);

        Some(new_arch_id)
    }

    pub(crate) fn get_remove_component_target(
        &mut self,
        entity_id: u32,
        type_id: TypeId,
        component_infos: &HashMap<TypeId, ComponentInfo>,
    ) -> Option<usize> {
        let old_arch_id = *self.entity_archetype.get(&entity_id)?;

        if let Some(edge) = self.archetypes[old_arch_id].get_edge(type_id) {
            if let Some(target) = edge.remove {
                return Some(target as usize);
            }
        }

        let mut new_types = self.archetypes[old_arch_id].sorted_component_types();
        if let Ok(pos) = new_types.binary_search(&type_id) {
            new_types.remove(pos);
        } else {
            return Some(old_arch_id); // Zaten yok
        }

        let new_arch_id = if let Some(&id) = self.set_to_id.get(&new_types) {
            id
        } else {
            let id = self.archetypes.len();
            let mut infos = Vec::new();
            for &t in &new_types {
                infos.push(
                    component_infos
                        .get(&t)
                        .cloned()
                        .unwrap_or_else(|| ComponentInfo::of_type_id(t)),
                );
            }
            self.archetypes.push(Archetype::new(id as u32, &infos));
            self.set_to_id.insert(new_types, id);
            id
        };

        self.archetypes[old_arch_id].set_remove_edge(type_id, new_arch_id as u32);
        self.archetypes[new_arch_id].set_add_edge(type_id, old_arch_id as u32);

        Some(new_arch_id)
    }

    /// Entity despawn — remove from the archetype
    #[allow(dead_code)]
    pub(crate) fn on_despawn(&mut self, entity_id: u32) {
        if let Some(_arch_id) = self.entity_archetype.remove(&entity_id) {
            // Satır bilgisini EntityLocation'dan alacağız, burada sadece remove edebiliriz
            // Ama Archetype::detach_entity_row row bekler.
            // World::despawn içinde location bilgisi olduğu için oradan çağırmak daha sağlıklı.
            self.cache_dirty = true;
        }
    }

    /// Returns the indices of the archetypes satisfying the specified logical query filter.
    /// Used by the query system.
    pub(crate) fn matching_archetypes(
        &mut self,
        query_type_id: TypeId,
        predicate: fn(&Archetype) -> bool,
    ) -> &[usize] {
        // Cache dirty ise temizle
        if self.cache_dirty {
            self.query_cache.clear();
            self.cache_dirty = false;
        }

        // Cache miss — hesapla ve cache'le
        if !self.query_cache.contains_key(&query_type_id) {
            let mut matching_indices = Vec::new();
            for (idx, arch) in self.archetypes.iter().enumerate() {
                if predicate(arch) {
                    matching_indices.push(idx);
                }
            }
            self.query_cache.insert(query_type_id, matching_indices);
        }

        self.query_cache.get(&query_type_id).unwrap()
    }

    /// Returns the indices of the archetypes satisfying the specified logical query filter.
    /// Immutable version — does not use the cache.
    pub(crate) fn matching_archetypes_readonly(
        &self,
        predicate: fn(&Archetype) -> bool,
    ) -> Vec<usize> {
        let mut result = Vec::new();
        for (idx, arch) in self.archetypes.iter().enumerate() {
            if predicate(arch) {
                result.push(idx);
            }
        }
        result
    }

    /// Total archetype count
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }

    /// Returns the entity's current archetype index
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn entity_archetype_id(&self, entity_id: u32) -> Option<usize> {
        self.entity_archetype.get(&entity_id).copied()
    }

    pub(crate) fn gc_empty_archetypes(&mut self, entity_locations: &mut [EntityLocation]) -> usize {
        let mut i = 1; // Başlangıç archetype'ı (0) asla silinmez.
        let mut removed_count = 0;

        while i < self.archetypes.len() {
            if self.archetypes[i].is_empty() {
                let last_id = (self.archetypes.len() - 1) as u32;
                let target_id = i as u32;

                // 1. İmzayı set_to_id'den çıkar
                let dead_signature = self.archetypes[i].sorted_component_types();
                self.set_to_id.remove(&dead_signature);

                if target_id != last_id {
                    // Swap remove yapılacak, son eleman `target_id` indeksine geçecek.
                    let last_signature = self.archetypes[last_id as usize].sorted_component_types();
                    self.set_to_id.insert(last_signature, target_id as usize);
                }

                // 2. Archetype'ı listeden swap_remove yap.
                self.archetypes.swap_remove(i);
                removed_count += 1;

                if target_id != last_id {
                    self.archetypes[i].id = target_id;

                    // Bu archetype içindeki entity_location referanslarını güncelle
                    for &entity in self.archetypes[i].entities() {
                        let e_idx = entity as usize;
                        if e_idx < entity_locations.len() {
                            entity_locations[e_idx].archetype_id = target_id;
                        }
                        self.entity_archetype.insert(entity, target_id as usize);
                    }
                }

                // 3. Edges (ArchEdge cache) güncellemesi. Tüm kalan archetype'ları dolaş.
                for arch in &mut self.archetypes {
                    let mut dead_edges = Vec::new();

                    for (&type_id, edge) in arch.edges.iter_mut() {
                        if edge.add == Some(target_id) {
                            edge.add = None;
                        } else if edge.add == Some(last_id) {
                            edge.add = Some(target_id);
                        }

                        if edge.remove == Some(target_id) {
                            edge.remove = None;
                        } else if edge.remove == Some(last_id) {
                            edge.remove = Some(target_id);
                        }

                        if edge.add.is_none() && edge.remove.is_none() {
                            dead_edges.push(type_id);
                        }
                    }

                    for k in dead_edges {
                        arch.edges.remove(&k);
                    }
                }

                // Cache invalidate.
                self.cache_dirty = true;

                // İlerlemeyi durdur (Çünkü i'ye yeni bir Archetype kondu, onu da kontrol etmemiz lazım)
            } else {
                i += 1;
            }
        }

        // Map'leri küçült
        self.set_to_id.shrink_to_fit();
        self.entity_archetype.shrink_to_fit();
        self.query_cache.shrink_to_fit();

        removed_count
    }

    pub(crate) fn clear_entities(&mut self) {
        // Clearing one archetype at a time is not enough, and the reason is the same one
        // `Archetype::clear` splits itself for, one level up: a destructor in archetype `j` can
        // panic, and every archetype after it is then still fully populated and still iterable —
        // queries walk archetypes directly, so the ones that were never reached are not hidden
        // by anything. So the whole index is emptied first and every destructor runs afterwards.
        //
        // The one allocation is bought before anything is touched; if it fails, nothing changed.
        let mut counts: Vec<usize> =
            Vec::with_capacity(self.archetypes.iter().map(|a| a.columns.len()).sum());
        // ── PHASE 1: infallible, index-wide. No drop glue anywhere in here. ──
        let mut spans: Vec<usize> = Vec::with_capacity(self.archetypes.len());
        for arch in &mut self.archetypes {
            let before = counts.len();
            arch.forget_all_rows(&mut counts);
            spans.push(counts.len() - before);
        }
        self.entity_archetype.clear();
        self.cache_dirty = true;

        // ── PHASE 2: the only user code, over an index that already shows nothing. ──
        let mut at = 0;
        for (arch, span) in self.archetypes.iter_mut().zip(spans) {
            // SAFETY: `counts[at..at + span]` is exactly what this archetype pushed in phase 1,
            // in order, and nothing has touched it since.
            unsafe { arch.drop_forgotten_rows(&counts[at..at + span]) };
            at += span;
        }
    }
}
