use crate::archetype::index::ArchetypeIndex;
use crate::archetype::{Archetype, ComponentInfo, EntityLocation};
use crate::component::Component;
use crate::entity::Entity;

use std::any::TypeId;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::RwLock;

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
    pub(crate) sparse_sets: HashMap<TypeId, crate::archetype::sparse_set::ComponentSparseSet>,

    despawn_hooks: Vec<DespawnHook>,
    entities_to_despawn: Vec<Entity>,
    is_despawning: bool,
    pub(crate) entity_observers: HashMap<TypeId, Box<dyn std::any::Any + Send + Sync>>,
    pub tick: u32,
    /// Değişiklik tespiti (change detection) referans tick'i: `Changed<T>`/`Added<T>`
    /// filtreleri `ticks.changed > change_ref_tick` ile bu değere göre karşılaştırır.
    /// Schedule, her frame başında bunu bir önceki frame'in tick'ine ayarlar; böylece
    /// "son frame'den beri değişenler" doğru raporlanır. (Eskiden `== tick` idi ve tick
    /// hiç ilerlemediği için ya hiçbir şeyi ya da her şeyi eşliyordu.)
    pub change_ref_tick: u32,
}

impl World {
    pub fn new() -> Self {
        let mut world = Self {
            resources: HashMap::new(),
            entity_locations: Vec::new(),
            archetype_index: ArchetypeIndex::new(),
            component_infos: HashMap::new(),
            component_hooks: HashMap::new(),
            sparse_sets: HashMap::new(),
            despawn_hooks: Vec::new(),
            entities_to_despawn: Vec::new(),
            is_despawning: false,
            entity_observers: HashMap::new(),
            tick: 1,
            change_ref_tick: 0,
        };
        world.insert_resource(crate::commands::CommandQueue::new());
        world.insert_resource(Entities::new());
        world.insert_resource(Entities::new());
        world
    }

    fn run_hooks<F>(&mut self, type_id: TypeId, mut f: F)
    where
        F: FnMut(&mut ComponentHooks, &mut World),
    {
        let mut hooks = self.component_hooks.remove(&type_id);
        if let Some(ref mut h) = hooks {
            f(h, self);
        }
        if let Some(h) = hooks {
            if let Some(existing) = self.component_hooks.get_mut(&type_id) {
                existing.on_add.extend(h.on_add);
                existing.on_set.extend(h.on_set);
                existing.on_remove.extend(h.on_remove);
            } else {
                self.component_hooks.insert(type_id, h);
            }
        }
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

    /// Frame başında değişiklik-tespiti penceresini açar: bu frame'in karşılaştırma
    /// referansını `ref_tick`'e (bir önceki çalıştırmanın tick'i) ayarlar ve dünya
    /// tick'ini bu frame için ilerletir. `Changed<T>`/`Added<T>` filtreleri
    /// `ticks.changed > change_ref_tick` ile karşılaştırır. Yeni tick'i döndürür.
    /// (Sort yan-etkisi olan `increment_tick`'ten farklı olarak yalnızca sayaç ilerler.)
    pub fn begin_change_frame(&mut self, ref_tick: u32) -> u32 {
        self.change_ref_tick = ref_tick;
        self.tick = self.tick.wrapping_add(1);
        if self.tick == 0 {
            self.tick = 1;
        }
        self.tick
    }

    /// Ertelenmiş komut kuyruğunu (CommandQueue) işler.
    /// Entity ekleme/çıkarma işlemleri bu sayede kilitlenme (deadlock) yaşamadan batch halinde uygulanır.
    pub fn apply_commands(&mut self) {
        let queue_opt = self
            .get_resource::<crate::commands::CommandQueue>()
            .map(|q| (*q).clone());
        if let Some(queue) = queue_opt {
            queue.apply(self);
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

    /// Registers a Component hook (Observer) for `OnInsert`.
    pub fn add_observer<T: Component, F>(&mut self, mut system: F) -> &mut Self
    where
        F: FnMut(crate::observer::On<crate::observer::Insert, T>) + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        let mut hooks = self.component_hooks.remove(&type_id).unwrap_or_default();
        
        hooks.on_add.push(Box::new(move |_world, entity| {
            let event = crate::observer::On {
                event: crate::observer::Insert,
                entity,
                _marker: std::marker::PhantomData,
            };
            system(event);
        }));
        
        self.component_hooks.insert(type_id, hooks);
        self
    }

    /// Özel EntityEvent'ler için Entity bazlı Observer kaydı
    pub fn observe<E: crate::observer::EntityEvent, F>(&mut self, entity: Entity, listener: F) -> &mut Self
    where
        F: FnMut(crate::observer::On<E>) + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<E>();
        let map_any = self.entity_observers.entry(type_id).or_insert_with(|| {
            Box::new(HashMap::<Entity, Vec<Box<dyn FnMut(crate::observer::On<E>) + Send + Sync + 'static>>>::new())
        });
        
        let map = map_any.downcast_mut::<HashMap<Entity, Vec<Box<dyn FnMut(crate::observer::On<E>) + Send + Sync + 'static>>>>().unwrap();
        map.entry(entity).or_default().push(Box::new(listener));
        self
    }

    /// Bir Event'i tetikler ve hiyerarşide yukarı doğru yayar (bubble-up)
    pub fn trigger<E: crate::observer::EntityEvent>(&mut self, event: E) {
        use crate::component::Parent;
        let mut current_entity = event.target();

        loop {
            // Observer'ları bu entity için bul ve çalıştır
            let mut hooks_to_run = Vec::new();
            
            if let Some(map_any) = self.entity_observers.get_mut(&TypeId::of::<E>()) {
                if let Some(map) = map_any.downcast_mut::<HashMap<Entity, Vec<Box<dyn FnMut(crate::observer::On<E>) + Send + Sync + 'static>>>>() {
                    if let Some(listeners) = map.remove(&current_entity) {
                        hooks_to_run = listeners;
                    }
                }
            }

            for mut listener in hooks_to_run.drain(..) {
                let e = crate::observer::On {
                    event: event.clone(),
                    entity: current_entity,
                    _marker: std::marker::PhantomData,
                };
                listener(e);
                
                // Geri koy
                if let Some(map_any) = self.entity_observers.get_mut(&TypeId::of::<E>()) {
                    if let Some(map) = map_any.downcast_mut::<HashMap<Entity, Vec<Box<dyn FnMut(crate::observer::On<E>) + Send + Sync + 'static>>>>() {
                        map.entry(current_entity).or_default().push(listener);
                    }
                }
            }

            if !event.can_propagate() {
                break;
            }

            // Propagate to parent
            if let Some(parent_ptr) = self.get_component_ptr(current_entity, TypeId::of::<Parent>()) {
                current_entity = self.reconstruct_entity(unsafe { (*(parent_ptr as *const Parent)).0 }).unwrap();
            } else {
                break;
            }
        }
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
        self.component_hooks
            .entry(TypeId::of::<T>())
            .or_default()
            .on_add
            .push(hook);
    }

    pub fn register_on_remove<T: Component>(&mut self, hook: RemoveHook) {
        self.component_hooks
            .entry(TypeId::of::<T>())
            .or_default()
            .on_remove
            .push(hook);
    }

    pub fn register_on_set<T: Component>(&mut self, hook: SetHook) {
        self.component_hooks
            .entry(TypeId::of::<T>())
            .or_default()
            .on_set
            .push(hook);
    }

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
        let state = entities.state.lock().expect("Entities mutex poisoned");
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

        Some(new_entities)
    }

    pub fn register_despawn_hook(&mut self, hook: DespawnHook) {
        self.despawn_hooks.push(hook);
    }

    
    pub fn spawn_batch<I>(&mut self, iter: I) -> impl Iterator<Item = Entity>
    where
        I: IntoIterator,
        I::Item: crate::component::Bundle,
    {
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
            let loc = self.entity_locations[id as usize];

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

            {
                let entities = self
                    .get_resource::<Entities>()
                    .expect("Entities resource not initialized");
                entities.free(e);
            }

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
        let mut state = entities.state.lock().expect("Entities mutex poisoned");
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
        let entities = self
            .get_resource::<Entities>()
            .expect("Entities resource not initialized");
        let state = entities.state.lock().expect("Entities mutex poisoned");
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

    /// Sisteme component ekleme — Veriyi archetype sütununa taşır.
    pub fn add_bundle<B: crate::component::Bundle>(&mut self, entity: Entity, bundle: B) {
        if !self.is_alive(entity) { return; }
        let eid = entity.id();
        let infos = B::get_infos();
        
        for info in &infos {
            self.component_infos.entry(info.type_id).or_insert_with(|| *info);
        }
        
        // Şimdilik SparseSet desteklemiyor (bundle içi SparseSet olursa ayrıştırmak zor, future work)
        // Table bazlı block move:
        let old_arch_id = match self.archetype_index.entity_archetype.get(&eid) {
            Some(&id) => id,
            None => {
                // Eğer entity önceden bomboşsa (sadece spawn edilmişse)
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
            unsafe { bundle.write_to_archetype(arch, loc.row as usize, self.tick); }
            return;
        }

        let old_loc = self.entity_locations[eid as usize];
        let (new_row, moved_eid) = unsafe {
            let old_arch_ptr = &mut self.archetype_index.archetypes[old_arch_id] as *mut crate::archetype::Archetype;
            let target_arch_ptr = &mut self.archetype_index.archetypes[target_arch_id] as *mut crate::archetype::Archetype;
            (&mut *old_arch_ptr).move_entity_to(old_loc.row as usize, &mut *target_arch_ptr)
        };

        if let Some(moved) = moved_eid {
            self.entity_locations[moved as usize].row = old_loc.row;
        }

        let arch = &mut self.archetype_index.archetypes[target_arch_id];
        unsafe { bundle.write_to_archetype(arch, new_row as usize, self.tick); }

        self.entity_locations[eid as usize] = EntityLocation {
            archetype_id: target_arch_id as u32,
            row: new_row,
        };
        self.archetype_index.entity_archetype.insert(eid, target_arch_id);
    }

    pub fn remove_bundle<B: crate::component::Bundle>(&mut self, entity: Entity) {
        if !self.is_alive(entity) { return; }
        let eid = entity.id();
        let infos = B::get_infos();

        let old_arch_id = match self.archetype_index.entity_archetype.get(&eid) {
            Some(&id) => id,
            None => return,
        };

        let mut new_types = self.archetype_index.archetypes[old_arch_id].sorted_component_types();
        for info in &infos {
            if let Ok(pos) = new_types.binary_search(&info.type_id) {
                new_types.remove(pos);
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

        let old_loc = self.entity_locations[eid as usize];
        let (new_row, moved_eid) = unsafe {
            let old_arch_ptr = &mut self.archetype_index.archetypes[old_arch_id] as *mut crate::archetype::Archetype;
            let target_arch_ptr = &mut self.archetype_index.archetypes[target_arch_id] as *mut crate::archetype::Archetype;
            (&mut *old_arch_ptr).move_entity_to(old_loc.row as usize, &mut *target_arch_ptr)
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
            let ptr = &component as *const T as *const u8;
            // SAFETY: `ptr`, set'in `info.layout`'u ile birebir eşleşen `T` bileşenini gösterir;
            // sahiplik set'e devredilir ve aşağıda `forget` ile çift-drop engellenir.
            unsafe { set.insert(eid, ptr, self.tick); }
            std::mem::forget(component);

            self.run_hooks(type_id, |h, w| {
                for hook in &mut h.on_add { hook(w, entity); }
                for hook in &mut h.on_set { hook(w, entity); }
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
                unsafe {
                    let ptr = col.get_ptr(old_loc.row as usize) as *mut T;
                    *ptr = component;
                    col.ticks_ptr_mut()
                        .add(old_loc.row as usize)
                        .write(crate::archetype::ComponentTicks::new(self.tick));
                }
            }
            // Trigger OnSet hooks
            let mut hooks = self.component_hooks.remove(&type_id);
            if let Some(ref mut h) = hooks {
                for hook in &mut h.on_set {
                    hook(self, entity);
                }
            }
            if let Some(h) = hooks {
                if let Some(existing) = self.component_hooks.get_mut(&type_id) {
                    existing.on_add.extend(h.on_add);
                    existing.on_set.extend(h.on_set);
                    existing.on_remove.extend(h.on_remove);
                } else {
                    self.component_hooks.insert(type_id, h);
                }
            }
            return;
        }

        // 2. Migration: Verileri eski archetype'tan hedef archetype'a taşı
        let (eid, old_arch_id, old_row) = (
            entity.id(),
            old_loc.archetype_id as usize,
            old_loc.row as usize,
        );

        let (new_row, moved_eid) = unsafe {
            // Raw pointer ile iki archetype'ı ödünç alıyoruz (farklı indeksler olduğu garantidir)
            let old_arch_ptr = &mut self.archetype_index.archetypes[old_arch_id] as *mut Archetype;
            let target_arch_ptr =
                &mut self.archetype_index.archetypes[target_arch_id] as *mut Archetype;

            (&mut *old_arch_ptr).move_entity_to(old_row, &mut *target_arch_ptr)
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

        let mut hooks = self.component_hooks.remove(&type_id);
        if let Some(ref mut h) = hooks {
            for hook in &mut h.on_add {
                hook(self, entity);
            }
            for hook in &mut h.on_set {
                hook(self, entity);
            }
        }
        if let Some(h) = hooks {
            if let Some(existing) = self.component_hooks.get_mut(&type_id) {
                existing.on_add.extend(h.on_add);
                existing.on_set.extend(h.on_set);
                existing.on_remove.extend(h.on_remove);
            } else {
                self.component_hooks.insert(type_id, h);
            }
        }
    }

    /// Entity üzerindeki tüm bileşenlerin TypeId'lerini döndürür.
    pub fn get_entity_component_types(&self, entity: Entity) -> Vec<TypeId> {
        if !self.is_alive(entity) {
            return Vec::new();
        }
        if let Some(&loc) = self.entity_locations.get(entity.id() as usize) {
            if loc.is_valid() {
                let arch = &self.archetype_index.archetypes[loc.archetype_id as usize];
                return arch.component_types();
            }
        }
        Vec::new()
    }

    /// Raw Component Pointer alma (Reflection/Editor için)
    pub fn get_component_ptr(&self, entity: Entity, type_id: TypeId) -> Option<*const u8> {
        let loc = self.entity_locations.get(entity.id() as usize).copied()?;
        if !loc.is_valid() {
            return None;
        }
        let arch = &self.archetype_index.archetypes[loc.archetype_id as usize];
        let col = arch.get_column(type_id)?;
        Some(unsafe { col.get_ptr(loc.row as usize) })
    }

    /// Mut mutable Component pointer alma (HierarchyExt vs için)
    pub fn get_component_mut_ptr(&mut self, entity: Entity, type_id: TypeId) -> Option<*mut u8> {
        let loc = self.entity_locations.get(entity.id() as usize).copied()?;
        if !loc.is_valid() {
            return None;
        }
        let arch = &mut self.archetype_index.archetypes[loc.archetype_id as usize];
        // SAFETY: &mut self ile tekil archetype erişimi; sütuna tekil &mut.
        let col = unsafe { arch.get_column_mut(type_id) }?;
        Some(unsafe { col.get_mut_ptr(loc.row as usize) })
    }

    /// Entity ID'sinden geçerli nesneyi tekrar yapılandırır
    pub fn reconstruct_entity(&self, id: u32) -> Option<Entity> {
        if id as usize >= self.entity_locations.len() || !self.entity_locations[id as usize].is_valid() {
            return None;
        }
        let entities = self.get_resource::<Entities>()?;
        let state = entities.state.lock().unwrap();
        if id as usize >= state.generations.len() || state.free_set.contains(&id) {
            return None;
        }
        Some(Entity::new(id, state.generations[id as usize]))
    }

    /// Sistemden component silme
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

        // 2. Migration
        let (new_row, moved_eid) = unsafe {
            let old_arch_ptr = &mut self.archetype_index.archetypes[old_loc.archetype_id as usize]
                as *mut Archetype;
            let target_arch_ptr =
                &mut self.archetype_index.archetypes[target_arch_id] as *mut Archetype;
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
        self.archetype_index
            .entity_archetype
            .insert(eid, target_arch_id);

        self.run_hooks(type_id, |h, w| {
            for hook in &mut h.on_remove {
                hook(w, entity);
            }
        });
    }



    // ==========================================================
    // ERGONOMİK SORGULAR (QUERY API)
    // ==========================================================

    pub fn query<'w, Q: crate::query::WorldQuery>(&'w self) -> Option<crate::query::Query<'w, Q>> {
        crate::query::Query::new(self)
    }

    /// Geriye uyumluluk için StorageView alternatifi
    #[inline]
    pub fn borrow<'w, T: Component>(&'w self) -> crate::query::Query<'w, &'w T> {
        self.query::<&T>().expect("Failed to create borrow Query")
    }

    /// Geriye uyumluluk için StorageViewMut alternatifi
    #[inline]
    pub fn borrow_mut<'w, T: Component>(&'w self) -> crate::query::Query<'w, crate::query::Mut<'w, T>> {
        self.query::<crate::query::Mut<T>>().expect("Failed to create borrow_mut Query")
    }

    /// Cache'li query — archetype indeks cache'ini kullanır.
    /// &mut self gerektirdiği için sadece World sahibiyken çağrılabilir.
    pub fn query_cached<'w, Q: crate::query::WorldQuery>(
        &'w mut self,
    ) -> Option<crate::query::Query<'w, Q>> {
        crate::query::Query::new_cached(self)
    }

    /// Tek bir entity üzerinde `Query` çalıştırıp anında sonuç almanızı sağlar.
    ///
    /// # Örnek
    /// ```ignore
    /// if let Some((mut t, mut v)) = world.query_entity_mut::<(Mut<Transform>, Mut<Velocity>)>(id) {
    ///     t.position += v.linear * dt;
    /// }
    /// ```
    ///
    /// Toplu (Batch) component ekleme. O(N) archetype lookup maliyetini O(1)'e düşürür.
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
                    unsafe {
                        std::ptr::write(col.get_ptr(row) as *mut T, component.clone());
                        col.ticks_ptr_mut().add(row).write(crate::archetype::ComponentTicks::new(self.tick));
                    }
                }
                self.run_hooks(type_id, |h, w| {
                    for e in &group_entities {
                        for hook in &mut h.on_set {
                            hook(w, *e);
                        }
                    }
                });
                continue;
            }

            for e in &group_entities {
                let eid = e.id();
                let old_loc = self.entity_locations[eid as usize];
                let old_row = old_loc.row as usize;

                let (new_row, moved_eid) = unsafe {
                    let old_arch_ptr = &mut self.archetype_index.archetypes[source_arch_id as usize] as *mut Archetype;
                    let target_arch_ptr = &mut self.archetype_index.archetypes[target_arch_id] as *mut Archetype;
                    (&mut *old_arch_ptr).move_entity_to(old_row, &mut *target_arch_ptr)
                };

                if let Some(moved) = moved_eid {
                    self.entity_locations[moved as usize].row = old_row as u32;
                }

                {
                    let arch = &self.archetype_index.archetypes[target_arch_id];
                    // SAFETY: yeni ayrılan satır; sütuna tekil erişim.
                    let col = unsafe { arch.get_column_mut(type_id) }.unwrap();
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
        }
    }

    /// Toplu (Batch) component çıkarma
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
                
                let (new_row, moved_eid) = unsafe {
                    let old_arch_ptr = &mut self.archetype_index.archetypes[source_arch_id as usize] as *mut Archetype;
                    let target_arch_ptr = &mut self.archetype_index.archetypes[target_arch_id] as *mut Archetype;
                    (&mut *old_arch_ptr).move_entity_to(old_loc.row as usize, &mut *target_arch_ptr)
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
        }
    }

    /// **Ham `u32` id ile — generation kontrolü yapmaz.** Despawn+reuse sonrası yanlış
    /// entity'nin verisi dönebilir; canlılık kritikse önce [`World::is_alive`] çağırın.
    pub fn query_entity_mut<'w, Q: crate::query::WorldQuery>(
        &'w mut self,
        entity_id: u32,
    ) -> Option<Q::Item<'w>> {
        let loc = self.entity_location(entity_id);
        if !loc.is_valid() {
            return None;
        }
        let arch = &self.archetype_index.archetypes[loc.archetype_id as usize];
        if !Q::matches_archetype(arch) {
            return None;
        }
        unsafe {
            let fetch = Q::fetch_raw(self, arch, self.tick)?;
            if !Q::filter_row(fetch, loc.row as usize, entity_id, self.change_ref_tick) {
                return None;
            }
            Some(Q::get_item(fetch, loc.row as usize, entity_id))
        }
    }

    /// Tek bir entity üzerinde read-only `Query` çalıştırıp anında sonuç almanızı sağlar.
    ///
    /// **Ham `u32` id ile — generation kontrolü yapmaz** (bkz. [`World::query_entity_mut`]).
    pub fn query_entity<'w, Q: crate::query::WorldQuery>(
        &'w self,
        entity_id: u32,
    ) -> Option<Q::Item<'w>> {
        let loc = self.entity_location(entity_id);
        if !loc.is_valid() {
            return None;
        }
        let arch = &self.archetype_index.archetypes[loc.archetype_id as usize];
        if !Q::matches_archetype(arch) {
            return None;
        }
        unsafe {
            let fetch = Q::fetch_raw(self, arch, self.tick)?;
            if !Q::filter_row(fetch, loc.row as usize, entity_id, self.change_ref_tick) {
                return None;
            }
            Some(Q::get_item(fetch, loc.row as usize, entity_id))
        }
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
        let state = entities.state.lock().expect("Entities mutex poisoned");
        state
            .next_entity_id
            .saturating_sub(state.free_ids.len() as u32)
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
        self.try_get_resource::<T>().ok()
    }

    /// Global bir Resource'u değiştirmek için çağrılır (Mutable Borrow)
    pub fn get_resource_mut<T: 'static>(&self) -> Option<ResourceWriteGuard<'_, T>> {
        self.try_get_resource_mut::<T>().ok()
    }

    /// `get_resource` ile aynı işlev, ama hata sebebini `Result` ile taşır.
    pub fn try_get_resource<T: 'static>(
        &self,
    ) -> Result<ResourceReadGuard<'_, T>, ResourceFetchError> {
        let type_id = TypeId::of::<T>();
        let storage = self
            .resources
            .get(&type_id)
            .ok_or(ResourceFetchError::NotFound(type_id))?;
        let guard = storage
            .try_read()
            .map_err(|_| ResourceFetchError::BorrowConflict(type_id))?;
        Ok(ResourceReadGuard {
            guard,
            _marker: PhantomData,
        })
    }

    /// `get_resource_mut` ile aynı işlev, ama hata sebebini `Result` ile taşır.
    pub fn try_get_resource_mut<T: 'static>(
        &self,
    ) -> Result<ResourceWriteGuard<'_, T>, ResourceFetchError> {
        let type_id = TypeId::of::<T>();
        let storage = self
            .resources
            .get(&type_id)
            .ok_or(ResourceFetchError::NotFound(type_id))?;
        let guard = storage
            .try_write()
            .map_err(|_| ResourceFetchError::BorrowConflict(type_id))?;
        Ok(ResourceWriteGuard {
            guard,
            _marker: PhantomData,
        })
    }

    /// Global bir Resource yoksa Default olarak oluşturur, ardından Mutable Borrow döndürür.
    /// World mutable borrow gerektirir, böylece hashmap'e güvenle kayıt yapılabilir.
    pub fn get_resource_mut_or_default<T: Default + Send + Sync + 'static>(
        &mut self,
    ) -> ResourceWriteGuard<'_, T> {
        let type_id = TypeId::of::<T>();
        self.resources
            .entry(type_id)
            .or_insert_with(|| RwLock::new(Box::new(T::default())));

        let storage = self
            .resources
            .get(&type_id)
            .expect("resource just inserted");
        let guard = storage.write().expect("resource write lock poisoned");
        ResourceWriteGuard {
            guard,
            _marker: PhantomData,
        }
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

    /// Bir resource'u geçici olarak world'den çıkarıp closure'a geçirir ve sonra geri koyar.
    /// Bu, resource'un içindeyken `&mut World` kullanmanız gerektiğinde borrow checker'ı
    /// mutlu etmenin en temiz yoludur (Bevy'deki `resource_scope` benzeri).
    ///
    /// # Örnek
    /// ```ignore
    /// world.resource_scope::<PoolManager, ()>(|world, pool| {
    ///     pool.instantiate(world, "enemy");
    /// });
    /// ```
    pub fn resource_scope<T: Send + Sync + 'static, U, F>(&mut self, f: F) -> Option<U>
    where
        F: FnOnce(&mut World, &mut T) -> U,
    {
        let resource = self.remove_resource::<T>()?;

        // Panic güvenliği: Closure panic yaparsa bile resource geri yerleştirilir.
        // Drop guard, stack unwind sırasında resource'u tekrar world'e koyar.
        struct Guard<'a, T: Send + Sync + 'static> {
            world: *mut World,
            resource: Option<T>,
            _marker: std::marker::PhantomData<&'a mut World>,
        }
        impl<'a, T: Send + Sync + 'static> Drop for Guard<'a, T> {
            fn drop(&mut self) {
                if let Some(resource) = self.resource.take() {
                    // SAFETY: self.world is valid for the lifetime of the Guard.
                    unsafe { &mut *self.world }.insert_resource(resource);
                }
            }
        }

        let mut guard = Guard::<T> {
            world: self as *mut World,
            resource: Some(resource),
            _marker: std::marker::PhantomData,
        };

        let result = f(self, guard.resource.as_mut().unwrap());

        // Normal dönüş: guard.drop() resource'u geri koyacak.
        Some(result)
    }

    /// Belirli bir Archetype içindeki iki satırı güvenli bir şekilde takaslar ve entity lokasyonlarını günceller.
    pub fn swap_archetype_rows(&mut self, arch_id: u32, row_a: usize, row_b: usize) {
        if row_a == row_b {
            return;
        }

        let arch = &self.archetype_index.archetypes[arch_id as usize];
        if row_a >= arch.len() || row_b >= arch.len() {
            return;
        }

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
            if arch_len <= 1 {
                continue;
            }

            let mut visited = std::collections::HashSet::new();

            for row in 0..arch_len {
                let parent_entity_id = self.archetype_index.archetypes[arch_idx].entities()[row];

                if visited.contains(&parent_entity_id) {
                    continue;
                }
                visited.insert(parent_entity_id);

                let children_opt = {
                    let fetch = unsafe {
                        <&crate::component::Children as crate::query::FetchComponent>::fetch_raw(self, &self.archetype_index.archetypes[arch_idx], self.tick)
                    };
                    fetch.map(|f| unsafe {
                        <&crate::component::Children as crate::query::FetchComponent>::get_item(f, row, parent_entity_id)
                    })
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
                            self.swap_archetype_rows(
                                arch_idx as u32,
                                current_insert_row,
                                child_row,
                            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Children;

    #[derive(Clone, PartialEq, Debug)]
    struct Transform(f32);
    impl crate::component::Component for Transform {}

    #[test]
    fn test_sort_archetype_hierarchy() {
        let mut world = World::new();

        // 5 entity oluşturalım: e0, e1, e2, e3, e4
        let e0 = world.spawn();
        let e1 = world.spawn();
        let e2 = world.spawn();
        let e3 = world.spawn();
        let e4 = world.spawn();

        // Hepsi aynı bileşenlere sahip olsun (aynı archetype'a girmeleri için)
        // Sırasıyla Transform ekliyoruz:
        world.add_component(e0, Transform(0.0));
        world.add_component(e1, Transform(1.0));
        world.add_component(e2, Transform(2.0));
        world.add_component(e3, Transform(3.0));
        world.add_component(e4, Transform(4.0));

        // Hiyerarşi kuralım: e0'ın çocukları e3 ve e4 olsun.
        // Başlangıçta e0(0), e1(1), e2(2), e3(3), e4(4) sırasıyla dizilidir.
        world.add_component(e0, Children(vec![e3.id(), e4.id()]));

        // Sadece e0'da Children olunca farklı archetype'a geçer (Archetype değişimi).
        // Bu yüzden hepsine Children eklemeliyiz ki AYNI archetype'da kalsınlar.
        world.add_component(e1, Children(vec![]));
        world.add_component(e2, Children(vec![]));
        world.add_component(e3, Children(vec![]));
        world.add_component(e4, Children(vec![]));

        // Şu an hepsi (Transform, Children) archetype'ında.
        // Beklenen indeksler: e0, e1, e2, e3, e4.

        // Hiyerarşi kaydırmasını çalıştır!
        world.sort_archetype_hierarchy();

        // Kontrol edelim. e0'dan hemen sonra e3 ve e4 gelmeli.
        let loc0 = world.entity_location(e0.id());
        let loc3 = world.entity_location(e3.id());
        let loc4 = world.entity_location(e4.id());

        assert_eq!(
            loc0.row + 1,
            loc3.row,
            "e3 (child), e0 (parent)'dan hemen sonra gelmeli"
        );
        assert_eq!(
            loc0.row + 2,
            loc4.row,
            "e4 (child), e3'ten hemen sonra gelmeli"
        );

        // Diğerleri (e1 ve e2) kaydırılmış olmalı.
        let loc1 = world.entity_location(e1.id());
        let loc2 = world.entity_location(e2.id());
        assert!(
            loc1.row > loc4.row || loc2.row > loc4.row,
            "Bağımsız entityler sona itilmeli"
        );
    }

    #[test]
    fn test_sort_archetype_hierarchy_deep() {
        let mut world = World::new();

        let e0 = world.spawn();
        let e1 = world.spawn();
        let e2 = world.spawn();
        let e3 = world.spawn();

        world.add_component(e0, Transform(0.0));
        world.add_component(e1, Transform(1.0));
        world.add_component(e2, Transform(2.0));
        world.add_component(e3, Transform(3.0));

        // e0 -> e1 -> e2 -> e3 zinciri
        world.add_component(e0, Children(vec![e1.id()]));
        world.add_component(e1, Children(vec![e2.id()]));
        world.add_component(e2, Children(vec![e3.id()]));
        world.add_component(e3, Children(vec![]));

        world.sort_archetype_hierarchy();

        let l0 = world.entity_location(e0.id());
        let l1 = world.entity_location(e1.id());
        let l2 = world.entity_location(e2.id());
        let l3 = world.entity_location(e3.id());

        assert_eq!(l0.row + 1, l1.row);
        // Not: Algoritma şu an sadece doğrudan çocukları hemen arkasına koyar.
        // e1 işlendiğinde e2 onun arkasına geçer, e2 işlendiğinde e3 onun arkasına geçer.
        // Sonuçta e0, e1, e2, e3 dizilimi kendiliğinden oluşur (visited mantığı).
        assert_eq!(l1.row + 1, l2.row);
        assert_eq!(l2.row + 1, l3.row);
    }


    #[test]
    fn spawn_despawn_generation() {
        let mut world = World::new();
        let e1 = world.spawn();
        world.despawn(e1);
        
        let e2 = world.spawn(); // aynı id, farklı generation
        assert_eq!(e1.id(), e2.id());
        assert_ne!(e1.generation(), e2.generation());
        
        // Eski handle artık geçersiz
        assert!(!world.is_alive(e1));
        assert!(world.is_alive(e2));
    }

    #[test]
    fn despawn_updates_swapped_entity_location() {
        #[derive(Clone)]
        struct TestComp(i32);
        impl crate::component::Component for TestComp {}

        let mut world = World::new();
        world.register_component_type::<TestComp>();
        
        let e1 = world.spawn(); world.add_component(e1, TestComp(1));
        let e2 = world.spawn(); world.add_component(e2, TestComp(2));
        let e3 = world.spawn(); world.add_component(e3, TestComp(3));
        
        // e2'yi despawn et — e3 onun yerine swap_remove ile gelir
        world.despawn(e2);
        
        // e3 hâlâ erişilebilir olmalı
        let comps = world.borrow::<TestComp>();
        let val = comps.get(e3.id()).unwrap();
        assert_eq!(val.0, 3);
    }

    #[test]
    fn add_component_migrates_archetype() {
        #[derive(Clone, Debug, PartialEq)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        #[derive(Clone, Debug, PartialEq)]
        struct TestCompF32(f32);
        impl crate::component::Component for TestCompF32 {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        world.register_component_type::<TestCompF32>();
        
        let e = world.spawn();
        world.add_component(e, TestCompI32(10));
        
        let loc1 = world.entity_location(e.id());
        
        world.add_component(e, TestCompF32(2.5));
        
        let loc2 = world.entity_location(e.id());
        assert_ne!(loc1.archetype_id, loc2.archetype_id);
        
        assert_eq!(world.borrow::<TestCompI32>().get(e.id()).unwrap().0, 10);
        assert_eq!(world.borrow::<TestCompF32>().get(e.id()).unwrap().0, 2.5);
    }

    #[test]
    fn spawn_batch_keeps_columns_and_entities_consistent() {
        #[derive(Clone, Debug, PartialEq)]
        struct BatchI(i32);
        impl crate::component::Component for BatchI {}
        #[derive(Clone, Debug, PartialEq)]
        struct BatchF(f32);
        impl crate::component::Component for BatchF {}

        let mut world = World::new();
        world.register_component_type::<BatchI>();
        world.register_component_type::<BatchF>();

        let n = 100usize;
        let bundles = (0..n).map(|i| (BatchI(i as i32), BatchF(i as f32 * 1.5)));
        let ents: Vec<_> = world.spawn_batch(bundles).collect();
        assert_eq!(ents.len(), n);

        // Her entity'nin iki bileşeni de doğru olmalı (column/entities desync veya OOB yok).
        let bi = world.borrow::<BatchI>();
        let bf = world.borrow::<BatchF>();
        for (i, e) in ents.iter().enumerate() {
            assert_eq!(bi.get(e.id()).map(|c| c.0), Some(i as i32), "BatchI[{i}]");
            assert_eq!(bf.get(e.id()).map(|c| c.0), Some(i as f32 * 1.5), "BatchF[{i}]");
        }
        // Query iterasyonu tam n eleman vermeli (her sütun uzunluğu == entities sayısı).
        assert_eq!(bi.iter().count(), n, "column/entities tutarsızlığı");
        assert_eq!(bf.iter().count(), n, "column/entities tutarsızlığı");
    }

    #[test]
    fn add_same_component_overwrites() {
        #[derive(Clone, Debug, PartialEq)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        
        let e = world.spawn();
        world.add_component(e, TestCompI32(1));
        world.add_component(e, TestCompI32(99)); // overwrite
        
        assert_eq!(world.borrow::<TestCompI32>().get(e.id()).unwrap().0, 99);
    }

    #[test]
    fn archetype_graph_reuses_archetypes() {
        #[derive(Clone, Debug, PartialEq)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        #[derive(Clone, Debug, PartialEq)]
        struct TestCompF32(f32);
        impl crate::component::Component for TestCompF32 {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        world.register_component_type::<TestCompF32>();
        
        let e1 = world.spawn(); world.add_component(e1, TestCompI32(1)); world.add_component(e1, TestCompF32(1.0));
        let e2 = world.spawn(); world.add_component(e2, TestCompI32(2)); world.add_component(e2, TestCompF32(2.0));
        
        let loc1 = world.entity_location(e1.id());
        let loc2 = world.entity_location(e2.id());
        assert_eq!(loc1.archetype_id, loc2.archetype_id);
        
        assert!(world.archetype_index.archetypes.len() < 5);
    }

    #[test]
    fn query_finds_matching_archetypes() {
        #[derive(Clone)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        #[derive(Clone)]
        struct TestCompF32(f32);
        impl crate::component::Component for TestCompF32 {}

        #[derive(Clone)]
        struct TestCompBool(bool);
        impl crate::component::Component for TestCompBool {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        world.register_component_type::<TestCompF32>();
        world.register_component_type::<TestCompBool>();
        
        let e1 = world.spawn(); world.add_component(e1, TestCompI32(1)); world.add_component(e1, TestCompF32(1.0));
        let e2 = world.spawn(); world.add_component(e2, TestCompI32(2)); world.add_component(e2, TestCompBool(true));
        let e3 = world.spawn(); world.add_component(e3, TestCompI32(3)); // sadece i32
        
        // i32 query'si 3 entity'yi de bulmalı
        let count = world.query::<&TestCompI32>().unwrap().iter().count();
        assert_eq!(count, 3);
        
        // (i32, f32) query'si sadece e1'i bulmalı
        let count = world.query::<(&TestCompI32, &TestCompF32)>().unwrap().iter().count();
        assert_eq!(count, 1);
    }

    #[test]
    fn query_mut_modifies_data() {
        #[derive(Clone)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        
        let e1 = world.spawn(); world.add_component(e1, TestCompI32(1));
        let e2 = world.spawn(); world.add_component(e2, TestCompI32(2));
        
        // Query ile tüm i32'leri iki katına çıkar
        if let Some(mut q) = world.query::<crate::query::Mut<TestCompI32>>() {
            for (_, mut val) in q.iter_mut() {
                val.0 *= 2;
            }
        }
        
        assert_eq!(world.borrow::<TestCompI32>().get(e1.id()).unwrap().0, 2);
        assert_eq!(world.borrow::<TestCompI32>().get(e2.id()).unwrap().0, 4);
    }

    #[test]
    fn query_skips_non_matching() {
        #[derive(Clone)]
        struct CompA;
        impl crate::component::Component for CompA {}
        #[derive(Clone)]
        struct CompB;
        impl crate::component::Component for CompB {}

        let mut world = World::new();
        world.register_component_type::<CompA>();
        world.register_component_type::<CompB>();

        for _ in 0..100 {
            let e = world.spawn();
            world.add_component(e, CompA);
        }

        for _ in 0..50 {
            let e = world.spawn();
            world.add_component(e, CompB);
        }

        let a_count = world.query::<&CompA>().unwrap().iter().count();
        let b_count = world.query::<&CompB>().unwrap().iter().count();
        let both_count = world.query::<(&CompA, &CompB)>().unwrap().iter().count();

        assert_eq!(a_count, 100);
        assert_eq!(b_count, 50);
        assert_eq!(both_count, 0);
    }

    #[test]
    fn spawn_despawn_10k_entities_archetype_stability() {
        #[derive(Clone)]
        struct CompA(i32);
        impl crate::component::Component for CompA {}
        #[derive(Clone)]
        struct CompB(f32);
        impl crate::component::Component for CompB {}

        let mut world = World::new();
        world.register_component_type::<CompA>();
        world.register_component_type::<CompB>();

        let initial_archetypes = world.archetype_index.archetypes.len();

        // Spawn 10k entities
        let mut entities = Vec::new();
        for i in 0..10_000 {
            let e = world.spawn();
            world.add_component(e, CompA(i as i32));
            if i % 2 == 0 {
                world.add_component(e, CompB(i as f32));
            }
            entities.push(e);
        }

        // Despawn all
        for e in entities {
            world.despawn(e);
        }

        // Archetype sayısı aynı kalmalı
        let final_archetypes = world.archetype_index.archetypes.len();
        // 1 empty, 1 for CompA, 1 for (CompA, CompB) = 3 total usually.
        assert!(final_archetypes <= initial_archetypes + 2);
    }
}
