use super::World;
use crate::archetype::{Archetype, ComponentInfo, EntityLocation};
use crate::component::Component;
use crate::entity::Entity;

use std::any::TypeId;

impl World {
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
}
