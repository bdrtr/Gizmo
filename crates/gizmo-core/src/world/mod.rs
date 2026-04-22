use std::collections::HashMap;
use std::any::TypeId;
use std::sync::RwLock;
use crate::entity::Entity;
use crate::archetype::{EntityLocation, ComponentInfo, Archetype};
use crate::archetype::index::ArchetypeIndex;
use crate::storage::{StorageView, StorageViewMut};
use std::marker::PhantomData;
use crate::component::Component;

pub mod hooks;
pub mod resources;

pub use self::hooks::*;
pub use self::resources::*;
pub use crate::entity::allocator::Entities;

pub struct World {
    // Entity'den bağımsız global veriler (Time, WindowSize, Input vs.)
    resources: HashMap<TypeId, RwLock<Box<dyn std::any::Any + Send + Sync>>>,

    /// Entity ID → archetype konumu. Hızlı O(1) lookup sağlar.
    /// entity_id indeks olarak kullanılır.
    entity_locations: Vec<EntityLocation>,

    /// Archetype tabanlı depolama — tüm component verileri burada tutulur.
    pub(crate) archetype_index: ArchetypeIndex,

    /// Runtime component metadata cache'i. Archetype sütunları oluşturmak için gereklidir.
    component_infos: HashMap<TypeId, ComponentInfo>,
    
    pub(crate) component_hooks: HashMap<TypeId, ComponentHooks>,
    
    despawn_hooks: Vec<DespawnHook>,
    entities_to_despawn: Vec<Entity>,
    is_despawning: bool,
    pub tick: u32,
}

impl World {
    pub fn new() -> Self {
        let mut world = Self {
            resources: HashMap::new(),
            entity_locations: Vec::new(),
            archetype_index: ArchetypeIndex::new(),
            component_infos: HashMap::new(),
            component_hooks: HashMap::new(),
            despawn_hooks: Vec::new(),
            entities_to_despawn: Vec::new(),
            is_despawning: false,
            tick: 1,
        };
        world.insert_resource(crate::commands::CommandQueue::new());
        world.insert_resource(Entities::new());
        world
    }

    /// Increments the local tick counter, guaranteeing it skips 0 on wrap.
    pub fn increment_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self.tick == 0 {
            self.tick = 1;
        }

        // Apply topological memory alignment for caching locality
        self.sort_archetype_hierarchy();
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

    pub fn register_on_add<T: Component>(&mut self, hook: AddHook) {
        self.component_hooks.entry(TypeId::of::<T>()).or_default().on_add.push(hook);
    }

    pub fn register_on_remove<T: Component>(&mut self, hook: RemoveHook) {
        self.component_hooks.entry(TypeId::of::<T>()).or_default().on_remove.push(hook);
    }

    pub fn register_on_set<T: Component>(&mut self, hook: SetHook) {
        self.component_hooks.entry(TypeId::of::<T>()).or_default().on_set.push(hook);
    }

    pub fn spawn(&mut self) -> Entity {
        let entity = {
            let entities = self.get_resource::<Entities>().unwrap();
            entities.reserve_entity()
        };
        
        self.flush_spawn(entity);
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
            self.entity_locations.resize(loc_idx + 1, EntityLocation::INVALID);
        }
        self.entity_locations[loc_idx] = EntityLocation {
            archetype_id: 0,
            row,
        };
    }

    // Eski A3 bridge ve rebuild metodları silindi (Archetype artık authoritative).

    pub fn get_entity(&self, id: u32) -> Option<Entity> {
        let entities = self.get_resource::<Entities>().unwrap();
        let state = entities.state.lock().unwrap();
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
            let entities_res = self.get_resource::<Entities>().unwrap();
            for _ in 0..count {
                let e = entities_res.reserve_entity();
                new_eids.push(e.id());
                new_entities.push(e);
            }
        }

        // Seçilen Archetype içinde kopyalamayı batch halinde yapıyoruz
        let arch = &mut self.archetype_index.archetypes[arch_id];
        let tick = self.tick;
        let new_rows = unsafe {
            arch.batch_clone_row(row, count, &new_eids, tick)
        };
        
        // Location güncellemeleri
        for (i, &id) in new_eids.iter().enumerate() {
            let row = new_rows[i];
            let idx = id as usize;
            if idx >= self.entity_locations.len() {
                self.entity_locations.resize(idx + 1, EntityLocation::INVALID);
            }
            self.entity_locations[idx] = EntityLocation {
                archetype_id: arch_id as u32,
                row,
            };
            self.archetype_index.entity_archetype.insert(id, arch_id);
            // NOT: on_spawn çağırmıyoruz çünkü batch_clone_row zaten entity'yi
            // doğru archetype'a ekledi. on_spawn boş archetype'a (0) tekrar eklerdi.
        }

        Some(new_entities)
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
                // Call OnRemove hooks for all currently held components
                let comp_types = {
                    let arch = &self.archetype_index.archetypes[loc.archetype_id as usize];
                    arch.component_types()
                };
                for t in comp_types {
                    let c_hooks = self.component_hooks.get(&t).cloned();
                    if let Some(c_hooks) = c_hooks {
                        for hook in c_hooks.on_remove {
                            hook(self, e);
                        }
                    }
                }
                
                // Re-fetch location safely after hooks might have mutated state
                let loc = self.entity_locations[id as usize];
                if loc.is_valid() {
                    // Archetype'tan verileri temizle
                    if let Some(moved_eid) = self.archetype_index.archetypes[loc.archetype_id as usize].swap_remove_entity(loc.row as usize) {
                        // Kayan entity'nin location bilgisini güncelle
                        self.entity_locations[moved_eid as usize].row = loc.row;
                    }
                }
            }

            { let entities = self.get_resource::<Entities>().unwrap(); entities.free(e); }

            self.archetype_index.entity_archetype.remove(&id);
            self.entity_locations[id as usize] = EntityLocation::INVALID;
        }
        self.is_despawning = false;
    }

    /// Hafızadaki boşlukları sıkıştırır ve kullanılmayan (boş) Archetype tablolarını silerek
    /// RAM'i ve sistem performansını ilk baştaki defregmante (temiz) haline getirir.
    /// Yükleme (Loading) ekranlarında veya düşük yoğunluklu anlarda çağrılması önerilir.
    pub fn compact(&mut self) {
        // 1. Önce eski, kullanılmayan boş archetype'ları silelim (GC)
        self.archetype_index.gc_empty_archetypes(&mut self.entity_locations);

        // 2. Kalan archetype'ların kapasitelerini minimuma indirelim (Shrink To Fit)
        for arch in &mut self.archetype_index.archetypes {
            arch.shrink_to_fit();
        }
        
        self.archetype_index.archetypes.shrink_to_fit();

        // 3. World seviyesindeki listeleri daraltalım.
        self.entities_to_despawn.shrink_to_fit();
        self.entity_locations.shrink_to_fit();
        
        let entities = self.get_resource::<Entities>().unwrap();
        let mut state = entities.state.lock().unwrap();
        state.generations.shrink_to_fit();
        state.free_ids.shrink_to_fit();
        state.free_set.shrink_to_fit();
    }

    pub fn despawn_by_id(&mut self, id: u32) {
        if let Some(entity) = self.get_entity(id) {
            self.despawn(entity);
        }
    }

    /// Yaşayan (despawn olmamış) tüm Entity'leri döndüren iterator.
    /// Uyarı: İterasyon boyunca Entities mutex kilidi tutulur!
    pub fn iter_alive_entities(&self) -> Vec<Entity> {
        let entities = self.get_resource::<Entities>().unwrap();
        let state = entities.state.lock().unwrap();
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
        self.get_resource::<Entities>().unwrap().is_alive(entity)
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
            {
                let arch = &self.archetype_index.archetypes[target_arch_id];
                let mut col = arch.get_column_mut(type_id).unwrap();
                unsafe {
                    let ptr = col.get_ptr(old_loc.row as usize) as *mut T;
                    *ptr = component;
                    col.ticks_ptr_mut().add(old_loc.row as usize).write(crate::archetype::ComponentTicks::new(self.tick));
                }
            }
            // Trigger OnSet hooks
            let hooks = self.component_hooks.get(&type_id).cloned();
            if let Some(hooks) = hooks {
                for hook in hooks.on_set {
                    hook(self, entity);
                }
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
                col.ticks_ptr_mut().add(new_row as usize).write(crate::archetype::ComponentTicks::new(self.tick));
            }
        }

        // 4. Location güncellemeleri
        self.entity_locations[eid as usize] = EntityLocation {
            archetype_id: target_arch_id as u32,
            row: new_row,
        };
        self.archetype_index.entity_archetype.insert(eid, target_arch_id);

        let hooks = self.component_hooks.get(&type_id).cloned();
        if let Some(hooks) = hooks {
            for hook in hooks.on_add {
                hook(self, entity);
            }
            for hook in hooks.on_set {
                hook(self, entity);
            }
        }
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

        let hooks = self.component_hooks.get(&type_id).cloned();
        if let Some(hooks) = hooks {
            for hook in hooks.on_remove {
                hook(self, entity);
            }
        }
    }

    /// Component dizisine okuma erişimi (Read-Only, Ref ile paylaşılabilir).
    pub fn borrow<T: Component>(&self) -> StorageView<'_, T> {
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

        StorageView {
            archetypes: matching,
            arch_id_to_idx,
            entity_locations: &self.entity_locations,
            _marker: PhantomData,
        }
    }

    pub fn borrow_mut<T: Component>(&self) -> StorageViewMut<'_, T> {
        let type_id = TypeId::of::<T>();
        
        let mut matching = Vec::new();
        let mut arch_id_to_idx = vec![None; self.archetype_index.archetypes.len()];

        for arch in self.archetype_index.archetypes.iter() {
            if let Some(col) = arch.get_column_mut(type_id) {
                arch_id_to_idx[arch.id as usize] = Some(matching.len());
                matching.push((arch.entities(), col));
            }
        }

        StorageViewMut {
            archetypes: matching,
            arch_id_to_idx,
            entity_locations: &self.entity_locations,
            _marker: PhantomData,
        }
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
        let entities = self.get_resource::<Entities>().unwrap();
        let state = entities.state.lock().unwrap();
        state.next_entity_id.saturating_sub(state.free_ids.len() as u32)
    }

    // ==========================================================
    // RESOURCE SİSTEMİ (GLOBAL VERİLER)
    // ==========================================================

    /// Sisteme global bir Resource ekler veya üzerine yazar.
    pub fn insert_resource<T: Send + Sync + 'static>(&mut self, resource: T) {
        let type_id = TypeId::of::<T>();
        self.resources
            .insert(type_id, RwLock::new(Box::new(resource)));
    }

    /// Global bir Resource'u okumak için çağrılır (Immutable Borrow)
    pub fn get_resource<T: 'static>(&self) -> Option<ResourceReadGuard<'_, T>> {
        let type_id = TypeId::of::<T>();
        let storage = self.resources.get(&type_id)?;
        let guard = storage.try_read().expect("ECS Aliasing Error: Resource borrow conflict");
        Some(ResourceReadGuard { guard, _marker: PhantomData })
    }

    /// Global bir Resource'u değiştirmek için çağrılır (Mutable Borrow)
    pub fn get_resource_mut<T: 'static>(&self) -> Option<ResourceWriteGuard<'_, T>> {
        let type_id = TypeId::of::<T>();
        let storage = self.resources.get(&type_id)?;
        let guard = storage.try_write().expect("ECS Aliasing Error: Resource mutable borrow conflict");
        Some(ResourceWriteGuard { guard, _marker: PhantomData })
    }

    /// Global bir Resource yoksa Default olarak oluşturur, ardından Mutable Borrow döndürür.
    /// World mutable borrow gerektirir, böylece hashmap'e güvenle kayıt yapılabilir.
    pub fn get_resource_mut_or_default<T: Default + Send + Sync + 'static>(&mut self) -> ResourceWriteGuard<'_, T> {
        let type_id = TypeId::of::<T>();
        self.resources.entry(type_id).or_insert_with(|| RwLock::new(Box::new(T::default())));

        let storage = self.resources.get(&type_id).unwrap();
        let guard = storage.write().unwrap();
        ResourceWriteGuard { guard, _marker: PhantomData }
    }

    /// Global bir Resource'u ECS'ten tamamen çıkartır ve sahipliğini döndürür
    pub fn remove_resource<T: 'static>(&mut self) -> Option<T> {
        let type_id = TypeId::of::<T>();
        let cell = self.resources.remove(&type_id)?;
        let boxed_any = cell.into_inner().ok()?;
        match boxed_any.downcast::<T>() {
            Ok(boxed_t) => Some(*boxed_t),
            Err(_) => None,
        }
    }

    /// Belirli bir Archetype içindeki iki satırı güvenli bir şekilde takaslar ve entity lokasyonlarını günceller.
    pub fn swap_archetype_rows(&mut self, arch_id: u32, row_a: usize, row_b: usize) {
        if row_a == row_b { return; }
        
        let arch = &self.archetype_index.archetypes[arch_id as usize];
        if row_a >= arch.len() || row_b >= arch.len() { return; }

        let entity_a = arch.entities()[row_a];
        let entity_b = arch.entities()[row_b];
        
        unsafe {
            let mut_arch = &mut self.archetype_index.archetypes[arch_id as usize];
            mut_arch.swap_rows(row_a, row_b);
        }
        
        self.entity_locations[entity_a as usize].row = row_b as u32;
        self.entity_locations[entity_b as usize].row = row_a as u32;
    }

    /// Aynı archetype'da bulunan ebeveyn ve çocuk düğümleri bellekte sırt sırta verecek şekilde kümelendirir. O(N) cache swap.
    pub fn sort_archetype_hierarchy(&mut self) {
        let type_id = std::any::TypeId::of::<crate::component::Children>();
        let mut arches_to_sort: Vec<usize> = Vec::new();

        for (idx, arch) in self.archetype_index.archetypes.iter().enumerate() {
            if arch.has_component(type_id) {
                arches_to_sort.push(idx);
            }
        }

        for arch_idx in arches_to_sort {
            let arch_len = self.archetype_index.archetypes[arch_idx].len();
            if arch_len <= 1 { continue; }

            let mut visited = std::collections::HashSet::new();

            for row in 0..arch_len {
                let parent_entity_id = self.archetype_index.archetypes[arch_idx].entities()[row];
                
                if visited.contains(&parent_entity_id) { continue; }
                visited.insert(parent_entity_id);

                let children_opt = {
                    let fetch = unsafe { 
                        <&crate::component::Children as crate::query::FetchComponent>::fetch_raw(&self.archetype_index.archetypes[arch_idx], self.tick) 
                    };
                    if let Some(f) = fetch {
                        Some(unsafe { <&crate::component::Children as crate::query::FetchComponent>::get_item(f, row) })
                    } else { None }
                };

                let children_list = match children_opt {
                    Some(c) => c.0.clone(),
                    None => continue,
                };

                let mut current_insert_row = row + 1;
                for child_id in children_list {
                    let loc = self.entity_location(child_id);
                    if loc.is_valid() && loc.archetype_id == arch_idx as u32 {
                        let child_row = loc.row as usize;
                        if child_row > current_insert_row {
                            self.swap_archetype_rows(arch_idx as u32, current_insert_row, child_row);
                            visited.insert(child_id);
                            current_insert_row += 1;
                        } else if child_row == current_insert_row {
                            visited.insert(child_id);
                            current_insert_row += 1;
                        }
                    }
                }
            }
        }
    }
}

