use std::any::TypeId;
use std::collections::{HashMap, HashSet, VecDeque};
use std::{
    cell::RefCell,
    cell::{Ref, RefMut, BorrowError, BorrowMutError},
    marker::PhantomData,
};

use crate::archetype::{Archetype, ComponentInfo, EntityLocation};
use crate::storage::{StorageView, StorageViewMut};
use crate::entity::Entity;
use crate::component::Component;

// ═══════════════════════════════════════════════════════════════════════════
// ARCHETYPE INDEX — Yapısal indeks (veri depolamaz, sadece üyelik takibi)
// ═══════════════════════════════════════════════════════════════════════════

// ArchetypeRecord sildi ve yerine archetype.rs içindeki Archetype geçti.

/// World içindeki archetype yapısal indeksi.
/// SparseSet veri deposunun yanında çalışır — veri depolamaz,
/// sadece entity → component-set üyeliğini takip eder.
pub(crate) struct ArchetypeIndex {
    /// Sıralı component set → archetype indeksi
    pub(crate) set_to_id: HashMap<Vec<TypeId>, usize>,
    /// Archetype depolama tabloları (Gerçek veriler burada)
    pub(crate) archetypes: Vec<Archetype>,
    /// Entity ID → mevcut archetype indeksi
    pub(crate) entity_archetype: HashMap<u32, usize>,
    /// Query sonuç cache'i — sorted TypeId key → eşleşen archetype indeksleri
    query_cache: HashMap<Vec<TypeId>, Vec<usize>>,
    /// Cache'in geçerliliği — component ekleme/çıkarma sırasında true olur
    cache_dirty: bool,
}

impl ArchetypeIndex {
    fn new() -> Self {
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

    /// Entity spawn — boş archetype'a ekle
    fn on_spawn(&mut self, entity_id: u32) {
        self.archetypes[0].push_entity(entity_id);
        self.entity_archetype.insert(entity_id, 0);
    }

    /// Component ekleme — entity'yi yeni archetype'a taşı
    /// NOT: Veri taşıma işlemi World tarafından yapılır, bu metod sadece yapısal geçişi takip eder.
    fn get_add_component_target(&mut self, entity_id: u32, type_id: TypeId, component_infos: &HashMap<TypeId, ComponentInfo>) -> Option<usize> {
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
                infos.push(component_infos.get(&t).cloned().unwrap_or_else(|| ComponentInfo::of_type_id(t)));
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

    fn get_remove_component_target(&mut self, entity_id: u32, type_id: TypeId, component_infos: &HashMap<TypeId, ComponentInfo>) -> Option<usize> {
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
                infos.push(component_infos.get(&t).cloned().unwrap_or_else(|| ComponentInfo::of_type_id(t)));
            }
            self.archetypes.push(Archetype::new(id as u32, &infos));
            self.set_to_id.insert(new_types, id);
            id
        };

        self.archetypes[old_arch_id].set_remove_edge(type_id, new_arch_id as u32);
        self.archetypes[new_arch_id].set_add_edge(type_id, old_arch_id as u32);
        
        Some(new_arch_id)
    }

    /// Entity despawn — archetype'dan çıkar
    fn on_despawn(&mut self, entity_id: u32) {
        if let Some(_arch_id) = self.entity_archetype.remove(&entity_id) {
            // Satır bilgisini EntityLocation'dan alacağız, burada sadece remove edebiliriz
            // Ama Archetype::swap_remove_entity row bekler. 
            // World::despawn içinde location bilgisi olduğu için oradan çağırmak daha sağlıklı.
            self.cache_dirty = true;
        }
    }

    /// Belirtilen tüm component tiplerini içeren archetype'ların entity listelerini döndürür.
    /// Query sistemi tarafından kullanılır.
    pub(crate) fn matching_archetypes(&mut self, required_types: &[TypeId]) -> &[usize] {
        // Cache dirty ise temizle
        if self.cache_dirty {
            self.query_cache.clear();
            self.cache_dirty = false;
        }

        // Cache'den bak — sorted key olarak kullan
        let mut sorted_key: Vec<TypeId> = required_types.to_vec();
        sorted_key.sort();

        // Cache'de eşleşen archetype indeksleri var mı?
        // Cache miss — hesapla ve cache'le
        let mut matching_indices = Vec::new();
        for (idx, arch) in self.archetypes.iter().enumerate() {
            if required_types.iter().all(|&t| arch.has_component(t)) {
                matching_indices.push(idx);
            }
        }
        self.query_cache.insert(sorted_key.clone(), matching_indices);
        self.query_cache.get(&sorted_key).unwrap() 
    }

    /// Belirtilen tüm component tiplerini içeren archetype'ların entity listelerini döndürür.
    /// Immutable versiyon — cache kullanmaz.
    pub(crate) fn matching_archetypes_readonly(&self, required_types: &[TypeId]) -> Vec<usize> {
        let mut result = Vec::new();
        for (idx, arch) in self.archetypes.iter().enumerate() {
            if required_types.iter().all(|&t| arch.has_component(t)) {
                result.push(idx);
            }
        }
        result
    }

    /// Toplam archetype sayısı
    #[inline]
    pub(crate) fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }

    /// Entity'nin mevcut archetype indeksini döndürür
    #[inline]
    pub(crate) fn entity_archetype_id(&self, entity_id: u32) -> Option<usize> {
        self.entity_archetype.get(&entity_id).copied()
    }
}

pub type DespawnHook = fn(&mut World, Entity);

pub struct World {
    next_entity_id: u32,
    generations: Vec<u32>,
    free_ids: VecDeque<u32>,
    free_set: HashSet<u32>,
    
    // Entity'den bağımsız global veriler (Time, WindowSize, Input vs.)
    resources: HashMap<TypeId, RefCell<Box<dyn std::any::Any>>>,

    /// Entity ID → archetype konumu. Hızlı O(1) lookup sağlar.
    /// entity_id indeks olarak kullanılır.
    entity_locations: Vec<EntityLocation>,

    /// Archetype tabanlı depolama — tüm component verileri burada tutulur.
    pub(crate) archetype_index: ArchetypeIndex,

    /// Runtime component metadata cache'i. Archetype sütunları oluşturmak için gereklidir.
    component_infos: HashMap<TypeId, ComponentInfo>,
    
    despawn_hooks: Vec<DespawnHook>,
    entities_to_despawn: Vec<Entity>,
    is_despawning: bool,
}

impl World {
    pub fn new() -> Self {
        Self {
            next_entity_id: 0,
            generations: Vec::new(),
            free_ids: VecDeque::new(),
            free_set: HashSet::new(),
            resources: HashMap::new(),
            entity_locations: Vec::new(),
            archetype_index: ArchetypeIndex::new(),
            component_infos: HashMap::new(),
            despawn_hooks: Vec::new(),
            entities_to_despawn: Vec::new(),
            is_despawning: false,
        }
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    /// Belirli bir component turunun runtime metadata'sini kaydeder.
    /// Archetype storage migration asamalarinda column olusturma icin kullanilir.
    #[inline]
    pub fn register_component_type<T: Component>(&mut self) {
        let type_id = TypeId::of::<T>();
        self.component_infos
            .entry(type_id)
            .or_insert_with(ComponentInfo::of::<T>);
    }

    /// Belirli bir component turu kayitli mi?
    #[inline]
    pub fn is_component_registered<T: Component>(&self) -> bool {
        self.component_infos.contains_key(&TypeId::of::<T>())
    }

    /// Kayitli component metadata sayisi.
    #[inline]
    pub fn registered_component_count(&self) -> usize {
        self.component_infos.len()
    }

    pub fn spawn(&mut self) -> Entity {
        let entity = if let Some(id) = self.free_ids.pop_front() {
            self.free_set.remove(&id);
            let gen = self.generations[id as usize];
            Entity::new(id, gen)
        } else {
            let id = self.next_entity_id;
            self.next_entity_id += 1;
            self.generations.push(0);
            Entity::new(id, 0)
        };
        // Yeni entity'yi boş archetype'a kaydet
        self.archetype_index.on_spawn(entity.id());

        // Entity location tracking — boş archetype (id=0), row = entity'nin sırası
        let eid = entity.id();
        let loc_idx = eid as usize;
        let row = self.archetype_index.archetypes[0].len() as u32 - 1;

        if loc_idx >= self.entity_locations.len() {
            self.entity_locations.resize(loc_idx + 1, EntityLocation::INVALID);
        }
        self.entity_locations[loc_idx] = EntityLocation {
            archetype_id: 0,
            row,
        };

        entity
    }

    // Eski A3 bridge ve rebuild metodları silindi (Archetype artık authoritative).

    pub fn get_entity(&self, id: u32) -> Option<Entity> {
        if (id as usize) < self.generations.len() && !self.free_set.contains(&id) {
            return Some(Entity::new(id, self.generations[id as usize]));
        }
        None
    }

    pub fn register_despawn_hook(&mut self, hook: DespawnHook) {
        self.despawn_hooks.push(hook);
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

            let hooks = self.despawn_hooks.clone();
            for hook in hooks {
                hook(self, e);
            }

            let id = e.id();
            let loc = self.entity_locations[id as usize];
            
            if loc.is_valid() {
                // Archetype'tan verileri temizle
                if let Some(moved_eid) = self.archetype_index.archetypes[loc.archetype_id as usize].swap_remove_entity(loc.row as usize) {
                    // Kayan entity'nin location bilgisini güncelle
                    self.entity_locations[moved_eid as usize].row = loc.row;
                }
            }

            self.generations[id as usize] += 1;
            if self.free_set.insert(id) {
                self.free_ids.push_back(id);
            }

            self.archetype_index.entity_archetype.remove(&id);
            self.entity_locations[id as usize] = EntityLocation::INVALID;
        }
        self.is_despawning = false;
    }

    pub fn despawn_by_id(&mut self, id: u32) {
        if let Some(entity) = self.get_entity(id) {
            self.despawn(entity);
        }
    }

    /// Yaşayan (despawn olmamış) tüm Entity'leri döndüren iterator — sıfır allocation
    pub fn iter_alive_entities(&self) -> AliveEntityIter<'_> {
        AliveEntityIter {
            current_id: 0,
            max_id: self.next_entity_id,
            free_set: &self.free_set,
            generations: &self.generations,
        }
    }



    #[inline]
    pub fn is_alive(&self, entity: Entity) -> bool {
        let id = entity.id() as usize;
        id < self.generations.len() && self.generations[id] == entity.generation()
    }

    /// Sisteme component ekleme — Veriyi archetype sütununa taşır.
    pub fn add_component<T: Component>(&mut self, entity: Entity, component: T) {
        if !self.is_alive(entity) {
            return;
        }

        let eid = entity.id();
        self.register_component_type::<T>();
        let type_id = TypeId::of::<T>();

        // 1. Hedef archetype'ı belirle
        let target_arch_id = self.archetype_index.get_add_component_target(eid, type_id, &self.component_infos).unwrap();
        let old_loc = self.entity_locations[eid as usize];

        if old_loc.archetype_id == target_arch_id as u32 {
            // Zaten bu archetype'ta (aynı tip tekrar eklenmiş olabilir) — sadece üzerine yaz
            let arch = &self.archetype_index.archetypes[target_arch_id];
            let col = arch.get_column_mut(type_id).unwrap();
            unsafe {
                let ptr = col.get_ptr(old_loc.row as usize) as *mut T;
                *ptr = component;
            }
            return;
        }

        // 2. Migration: Verileri eski archetype'tan hedef archetype'a taşı
        let (eid, old_arch_id, old_row) = (entity.id(), old_loc.archetype_id as usize, old_loc.row as usize);

        let (new_row, moved_eid) = unsafe {
            // Raw pointer ile iki archetype'ı ödünç alıyoruz (farklı indeksler olduğu garantidir)
            let old_arch_ptr = &mut self.archetype_index.archetypes[old_arch_id] as *mut Archetype;
            let target_arch_ptr = &mut self.archetype_index.archetypes[target_arch_id] as *mut Archetype;
            
            (&mut *old_arch_ptr).move_entity_to(old_row, &mut *target_arch_ptr)
        };

        if let Some(moved) = moved_eid {
            self.entity_locations[moved as usize].row = old_row as u32;
        }

        // 3. Yeni component'ı hedef archetype'a ekle
        {
            let arch = &self.archetype_index.archetypes[target_arch_id];
            let mut col = arch.get_column_mut(type_id).expect("Mandatory component column missing");
            unsafe {
                let ptr = col.get_ptr(new_row as usize) as *mut T;
                std::ptr::write(ptr, component);
            }
        }

        // 4. Location güncellemeleri
        self.entity_locations[eid as usize] = EntityLocation {
            archetype_id: target_arch_id as u32,
            row: new_row,
        };
        self.archetype_index.entity_archetype.insert(eid, target_arch_id);
    }

    /// Sistemden component silme
    pub fn remove_component<T: Component>(&mut self, entity: Entity) {
        if !self.is_alive(entity) {
            return;
        }

        let eid = entity.id();
        let type_id = TypeId::of::<T>();
        let old_loc = self.entity_locations[eid as usize];

        // 1. Hedef archetype'ı belirle
        let target_arch_id_opt = self.archetype_index.get_remove_component_target(eid, type_id, &self.component_infos);
        let target_arch_id = match target_arch_id_opt {
            Some(id) => id,
            None => return, // Zaten yok veya hata
        };

        if old_loc.archetype_id == target_arch_id as u32 {
            return; // Zaten yok
        }

        // 2. Migration
        let (new_row, moved_eid) = unsafe {
            let old_arch_ptr = &mut self.archetype_index.archetypes[old_loc.archetype_id as usize] as *mut Archetype;
            let target_arch_ptr = &mut self.archetype_index.archetypes[target_arch_id] as *mut Archetype;
            (&mut *old_arch_ptr).move_entity_to(old_loc.row as usize, &mut *target_arch_ptr)
        };

        if let Some(moved) = moved_eid {
            self.entity_locations[moved as usize].row = old_loc.row;
        }

        // 3. Location güncelle
        self.entity_locations[eid as usize] = EntityLocation {
            archetype_id: target_arch_id as u32,
            row: new_row,
        };
        self.archetype_index.entity_archetype.insert(eid, target_arch_id);
    }

    /// Component dizisine okuma erişimi (Read-Only, Ref ile paylaşılabilir).
    pub fn borrow<T: Component>(&self) -> Result<StorageView<'_, T>, BorrowError> {
        let type_id = TypeId::of::<T>();
        
        // Bu componenti içeren tüm archetype'ları bul
        let mut matching = Vec::new();
        // StorageView için bir ID haritası gerekebilir
        let mut arch_id_to_idx = vec![None; self.archetype_index.archetypes.len()];

        for arch in self.archetype_index.archetypes.iter() {
            if let Some(col) = arch.get_column(type_id) {
                arch_id_to_idx[arch.id as usize] = Some(matching.len());
                matching.push((arch.entities(), col));
            }
        }

        Ok(StorageView {
            archetypes: matching,
            arch_id_to_idx,
            entity_locations: &self.entity_locations,
            _marker: PhantomData,
        })
    }

    pub fn borrow_mut<T: Component>(&self) -> Result<StorageViewMut<'_, T>, BorrowMutError> {
        let type_id = TypeId::of::<T>();
        
        let mut matching = Vec::new();
        let mut arch_id_to_idx = vec![None; self.archetype_index.archetypes.len()];

        for arch in self.archetype_index.archetypes.iter() {
            if let Some(col) = arch.get_column_mut(type_id) {
                arch_id_to_idx[arch.id as usize] = Some(matching.len());
                matching.push((arch.entities(), col));
            }
        }

        Ok(StorageViewMut {
            archetypes: matching,
            arch_id_to_idx,
            entity_locations: &self.entity_locations,
            _marker: PhantomData,
        })
    }

    // ComponentStorage/SparseSet tabanlı eski metodlar silindi.
    // Query sistemi artık StorageView ve Archetype üzerinden çalışmaktadır.

    // ==========================================================
    // ERGONOMİK SORGULAR (QUERY API)
    // ==========================================================

    pub fn query<'w, Q: crate::query::WorldQuery<'w>>(&'w self) -> Option<crate::query::Query<'w, Q>> {
        crate::query::Query::new(self)
    }

    /// Cache'li query — archetype indeks cache'ini kullanır.
    /// &mut self gerektirdiği için sadece World sahibiyken çağrılabilir.
    pub fn query_cached<'w, Q: crate::query::WorldQuery<'w>>(&'w mut self) -> Option<crate::query::Query<'w, Q>> {
        crate::query::Query::new_cached(self)
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
        self.next_entity_id.saturating_sub(self.free_ids.len() as u32)
    }

    // ==========================================================
    // RESOURCE SİSTEMİ (GLOBAL VERİLER)
    // ==========================================================

    /// Sisteme global bir Resource ekler veya üzerine yazar.
    pub fn insert_resource<T: 'static>(&mut self, resource: T) {
        let type_id = TypeId::of::<T>();
        self.resources
            .insert(type_id, RefCell::new(Box::new(resource)));
    }

    /// Global bir Resource'u okumak için çağrılır (Immutable Borrow)
    pub fn get_resource<T: 'static>(&self) -> Result<Option<Ref<'_, T>>, BorrowError> {
        let type_id = TypeId::of::<T>();
        let storage = match self.resources.get(&type_id) {
            Some(s) => s,
            None => return Ok(None),
        };

        match storage.try_borrow() {
            Ok(borrowed) => Ok(Some(Ref::map(borrowed, |s| {
                s.downcast_ref::<T>().unwrap()
            }))),
            Err(e) => Err(e),
        }
    }

    /// Global bir Resource'u değiştirmek için çağrılır (Mutable Borrow)
    pub fn get_resource_mut<T: 'static>(&self) -> Result<Option<RefMut<'_, T>>, BorrowMutError> {
        let type_id = TypeId::of::<T>();
        let storage = match self.resources.get(&type_id) {
            Some(s) => s,
            None => return Ok(None),
        };

        match storage.try_borrow_mut() {
            Ok(borrowed) => Ok(Some(RefMut::map(borrowed, |s| {
                s.downcast_mut::<T>().unwrap()
            }))),
            Err(e) => Err(e),
        }
    }

    /// Global bir Resource yoksa Default olarak oluşturur, ardından Mutable Borrow döndürür.
    /// World mutable borrow gerektirir, böylece hashmap'e güvenle kayıt yapılabilir.
    pub fn get_resource_mut_or_default<T: Default + 'static>(&mut self) -> RefMut<'_, T> {
        let type_id = TypeId::of::<T>();
        self.resources.entry(type_id).or_insert_with(|| RefCell::new(Box::new(T::default())));

        let storage = self.resources.get(&type_id).unwrap();
        RefMut::map(storage.borrow_mut(), |s| s.downcast_mut::<T>().unwrap())
    }

    /// Global bir Resource'u ECS'ten tamamen çıkartır ve sahipliğini döndürür
    pub fn remove_resource<T: 'static>(&mut self) -> Option<T> {
        let type_id = TypeId::of::<T>();
        let cell = self.resources.remove(&type_id)?;
        let boxed_any = cell.into_inner();
        match boxed_any.downcast::<T>() {
            Ok(boxed_t) => Some(*boxed_t),
            Err(_) => None,
        }
    }
}

/// Sıfır allocation ile yaşayan entity'ler üzerinde iterasyon yapan iterator.
pub struct AliveEntityIter<'a> {
    current_id: u32,
    max_id: u32,
    free_set: &'a HashSet<u32>,
    generations: &'a [u32],
}

impl<'a> Iterator for AliveEntityIter<'a> {
    type Item = Entity;

    #[inline]
    fn next(&mut self) -> Option<Entity> {
        while self.current_id < self.max_id {
            let id = self.current_id;
            self.current_id += 1;
            if !self.free_set.contains(&id) {
                let gen = self.generations[id as usize];
                return Some(Entity::new(id, gen));
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.max_id - self.current_id) as usize;
        (0, Some(remaining))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::impl_component;

    // Test component types
    #[derive(Debug, Clone, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct Health(u32);

    impl_component!(Position, Health);

    #[test]
    fn test_spawn_and_alive_count() {
        let mut world = World::new();
        let e1 = world.spawn();
        let e2 = world.spawn();
        let e3 = world.spawn();
        assert_eq!(world.entity_count(), 3);
        assert!(world.is_alive(e1));
        assert!(world.is_alive(e2));
        assert!(world.is_alive(e3));
    }

    #[test]
    fn test_despawn_removes_components() {
        let mut world = World::new();
        let e1 = world.spawn();
        world.add_component(e1, Position { x: 1.0, y: 2.0 });
        world.add_component(e1, Health(100));

        assert!(world.borrow::<Position>().unwrap().get(e1.id()).is_some());
        assert!(world.borrow::<Health>().unwrap().get(e1.id()).is_some());

        world.despawn(e1);

        assert!(!world.is_alive(e1));
        assert!(world.borrow::<Position>().unwrap().get(e1.id()).is_none());
        assert!(world.borrow::<Health>().unwrap().get(e1.id()).is_none());
    }

    #[test]
    fn test_despawn_only_touches_relevant_storages() {
        let mut world = World::new();
        let e1 = world.spawn();
        let e2 = world.spawn();

        // e1 has Position only, e2 has both
        world.add_component(e1, Position { x: 0.0, y: 0.0 });
        world.add_component(e2, Position { x: 1.0, y: 1.0 });
        world.add_component(e2, Health(50));

        // Despawn e1 — should not affect e2
        world.despawn(e1);

        assert!(world.borrow::<Position>().unwrap().get(e2.id()).is_some());
        assert!(world.borrow::<Health>().unwrap().get(e2.id()).is_some());
        assert_eq!(world.entity_count(), 1);
    }

    #[test]
    fn test_iter_alive_entities_zero_allocation() {
        let mut world = World::new();
        let _e1 = world.spawn();
        let e2 = world.spawn();
        let _e3 = world.spawn();

        world.despawn(e2);

        // Iterator should return 2 entities (e1 and e3), skipping e2
        let alive: Vec<Entity> = world.iter_alive_entities().collect();
        assert_eq!(alive.len(), 2);
        assert!(alive.iter().all(|e| e.id() != e2.id()));
    }

    #[test]
    fn test_entity_id_reuse_after_despawn() {
        let mut world = World::new();
        let e1 = world.spawn();
        let old_id = e1.id();
        let old_gen = e1.generation();

        world.despawn(e1);

        let e_new = world.spawn();
        // Should reuse the same ID with bumped generation
        assert_eq!(e_new.id(), old_id);
        assert_eq!(e_new.generation(), old_gen + 1);

        // Old entity should not be alive
        assert!(!world.is_alive(e1));
        assert!(world.is_alive(e_new));
    }

    #[test]
    fn test_add_component_overwrites() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Health(100));
        world.add_component(e, Health(50)); // Overwrite

        let hp = world.borrow::<Health>().unwrap();
        assert_eq!(hp.get(e.id()).unwrap().0, 50);
    }

    /// Aynı component türü iki kez eklenince archetype migration'da veri güncellenmeli.
    #[test]
    fn test_double_add_component_despawn_safe() {
        let mut world = World::new();
        let e = world.spawn();
        let id = e.id();
        world.add_component(e, Health(100));
        world.add_component(e, Health(50));

        world.despawn(e);

        assert!(!world.is_alive(e));
        assert!(world.borrow::<Health>().unwrap().get(id).is_none());

        // ID yeniden kullanıldığında eski Health taşınmamalı
        let e2 = world.spawn();
        assert_eq!(e2.id(), id);
        assert!(world.borrow::<Health>().unwrap().get(e2.id()).is_none());
    }

    #[test]
    fn test_component_registration_metadata() {
        let mut world = World::new();
        assert_eq!(world.registered_component_count(), 0);
        assert!(!world.is_component_registered::<Position>());
        assert!(!world.is_component_registered::<Health>());

        let e = world.spawn();
        world.add_component(e, Position { x: 1.0, y: 2.0 });
        assert!(world.is_component_registered::<Position>());
        assert_eq!(world.registered_component_count(), 1);

        world.add_component(e, Health(100));
        assert!(world.is_component_registered::<Health>());
        assert_eq!(world.registered_component_count(), 2);

        // remove metadata'yi silmez
        world.remove_component::<Health>(e);
        assert!(world.is_component_registered::<Health>());
        assert_eq!(world.registered_component_count(), 2);
    }
}
