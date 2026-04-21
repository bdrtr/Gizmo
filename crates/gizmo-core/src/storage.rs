use std::cell::{Ref, RefMut};
use std::marker::PhantomData;
use crate::archetype::{Column, EntityLocation};
use crate::component::Component;

/// Archetype tabanlı depolamaya dışarıdan (Physics, Editor vb.) erişim sağlayan görünüm.
/// SparseSet ile aynı API'yi (get, iter) sunarak geriye uyumluluk sağlar.
pub struct StorageView<'w, T: Component> {
    /// Her bir archetype için (entities, column) ikilisi
    pub(crate) archetypes: Vec<(&'w [u32], Ref<'w, Column>)>,
    /// Archetype ID -> archetypes içindeki indeksi
    pub(crate) arch_id_to_idx: Vec<Option<usize>>,
    pub(crate) entity_locations: &'w [EntityLocation],
    pub(crate) _marker: PhantomData<T>,
}

impl<'w, T: Component> StorageView<'w, T> {
    /// Bir entity için component verisini döndürür.
    #[inline]
    pub fn get(&self, entity_id: u32) -> Option<&T> {
        let loc = self.entity_locations.get(entity_id as usize)?;
        if !loc.is_valid() {
            return None;
        }

        let arch_idx = self.arch_id_to_idx.get(loc.archetype_id as usize)?.as_ref()?;
        let (_, col) = &self.archetypes[*arch_idx];
        
        unsafe {
            let ptr = col.get_ptr(loc.row as usize) as *const T;
            Some(&*ptr)
        }
    }

    /// Tüm bileşenler üzerinden iterasyon yapar.
    pub fn iter(self) -> impl Iterator<Item = (u32, &'w T)> {
        self.into_iter()
    }
}

impl<'w, T: Component> IntoIterator for StorageView<'w, T> {
    type Item = (u32, &'w T);
    type IntoIter = Box<dyn Iterator<Item = Self::Item> + 'w>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.archetypes.into_iter().flat_map(|(entities, col)| {
            entities.iter().enumerate().map(move |(row, &id)| {
                unsafe {
                    let ptr = col.get_ptr(row) as *const T;
                    (id, &*ptr)
                }
            })
        }))
    }
}

pub struct StorageViewMut<'w, T: Component> {
    /// Her bir archetype için (entities, column) ikilisi
    pub(crate) archetypes: Vec<(&'w [u32], RefMut<'w, Column>)>,
    /// Archetype ID -> archetypes içindeki indeksi
    pub(crate) arch_id_to_idx: Vec<Option<usize>>,
    pub(crate) entity_locations: &'w [EntityLocation],
    pub(crate) _marker: PhantomData<T>,
}

impl<'w, T: Component> StorageViewMut<'w, T> {
    #[inline]
    pub fn get(&self, entity_id: u32) -> Option<&T> {
        let loc = self.entity_locations.get(entity_id as usize)?;
        if !loc.is_valid() {
            return None;
        }

        let arch_idx = self.arch_id_to_idx.get(loc.archetype_id as usize)?.as_ref()?;
        let (_, col) = &self.archetypes[*arch_idx];
        
        unsafe {
            let ptr = col.get_ptr(loc.row as usize) as *const T;
            Some(&*ptr)
        }
    }

    #[inline]
    pub fn get_mut(&mut self, entity_id: u32) -> Option<&mut T> {
        let loc = self.entity_locations.get(entity_id as usize)?;
        if !loc.is_valid() {
            return None;
        }

        let arch_idx = self.arch_id_to_idx.get(loc.archetype_id as usize)?.as_ref()?;
        let (_, col) = &mut self.archetypes[*arch_idx];
        
        unsafe {
            // DİKKAT: RefMut guard'ı &mut T dönmek için kullanılabilir 
            // ama guard bu fonksiyon çıkışında düşerse pointer geçersizleşebilir.
            // StorageView/Mut geçici olduğu için &'w mut T dönmek zordur.
            // Bunun yerine sadece deref ediyoruz.
            let ptr = col.get_ptr(loc.row as usize) as *mut T;
            Some(&mut *ptr)
        }
    }

    /// Tüm bileşenler üzerinden mutable iterasyon yapar.
    pub fn iter_mut(self) -> impl Iterator<Item = (u32, &'w mut T)> {
        self.into_iter()
    }
}

impl<'w, T: Component> IntoIterator for StorageViewMut<'w, T> {
    type Item = (u32, &'w mut T);
    type IntoIter = Box<dyn Iterator<Item = Self::Item> + 'w>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.archetypes.into_iter().flat_map(|(entities, col)| {
            entities.iter().enumerate().map(move |(row, &id)| {
                unsafe {
                    let ptr = col.get_ptr(row) as *mut T;
                    (id, &mut *ptr)
                }
            })
        }))
    }
}
