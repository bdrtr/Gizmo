use std::sync::{RwLockReadGuard, RwLockWriteGuard};
use std::marker::PhantomData;
use crate::archetype::{Column, EntityLocation};
use crate::component::Component;

/// Archetype tabanlı depolamaya dışarıdan (Physics, Editor vb.) erişim sağlayan görünüm.
/// SparseSet ile aynı API'yi (get, iter) sunarak geriye uyumluluk sağlar.
pub struct StorageView<'w, T: Component> {
    /// Her bir archetype için (entities, column) ikilisi
    pub(crate) archetypes: Vec<(&'w [u32], RwLockReadGuard<'w, Column>)>,
    /// Archetype ID -> archetypes içindeki indeksi
    pub(crate) arch_id_to_idx: Vec<Option<usize>>,
    pub(crate) entity_locations: &'w [EntityLocation],
    pub(crate) _marker: PhantomData<T>,
}

unsafe impl<'w, T: Component + Sync> Sync for StorageView<'w, T> {}
unsafe impl<'w, T: Component + Send> Send for StorageView<'w, T> {}

impl<'w, T: Component> StorageView<'w, T> {
    /// Bu görünümde hiç varlık olup olmadığını kontrol eder.
    pub fn is_empty(&self) -> bool {
        self.archetypes.iter().all(|(entities, _)| entities.is_empty())
    }

    /// Bu görünümdeki tüm varlık ID'lerini döndüren bir iterator sağlar.
    /// View'ı tüketmez.
    pub fn entities(&self) -> impl Iterator<Item = u32> + '_ {
        self.archetypes.iter().flat_map(|(entities, _)| entities.iter().copied())
    }

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

    /// Bir varlığın bu görünümde olup olmadığını kontrol eder.
    #[inline]
    pub fn contains(&self, entity_id: u32) -> bool {
        self.get(entity_id).is_some()
    }

    /// Tüm bileşenler üzerinden iterasyon yapar.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &T)> + '_ {
        self.archetypes.iter().flat_map(|(entities, col)| {
            entities.iter().copied().zip(0..entities.len()).map(move |(entity, row)| {
                unsafe {
                    let ptr = col.get_ptr(row) as *const T;
                    (entity, &*ptr)
                }
            })
        })
    }

    /// Toplam bileşen sayısını döner.
    pub fn len(&self) -> usize {
        self.archetypes.iter().map(|(entities, _)| entities.len()).sum()
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
    pub(crate) archetypes: Vec<(&'w [u32], RwLockWriteGuard<'w, Column>)>,
    /// Archetype ID -> archetypes içindeki indeksi
    pub(crate) arch_id_to_idx: Vec<Option<usize>>,
    pub(crate) entity_locations: &'w [EntityLocation],
    pub(crate) _marker: PhantomData<T>,
}

unsafe impl<'w, T: Component + Sync> Sync for StorageViewMut<'w, T> {}
unsafe impl<'w, T: Component + Send> Send for StorageViewMut<'w, T> {}

impl<'w, T: Component> StorageViewMut<'w, T> {
    /// Bu görünümde hiç varlık olup olmadığını kontrol eder.
    pub fn is_empty(&self) -> bool {
        self.archetypes.iter().all(|(entities, _)| entities.is_empty())
    }

    /// Bu görünümdeki tüm varlık ID'lerini döndüren bir iterator sağlar.
    /// View'ı tüketmez.
    pub fn entities(&self) -> impl Iterator<Item = u32> + '_ {
        self.archetypes.iter().flat_map(|(entities, _)| entities.iter().copied())
    }

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

    /// Bir varlığın bu görünümde olup olmadığını kontrol eder.
    #[inline]
    pub fn contains(&self, entity_id: u32) -> bool {
        self.get(entity_id).is_some()
    }

    /// Read-only iterator over entities and components in a mutable view.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &T)> + '_ {
        self.archetypes.iter().flat_map(|(entities, col)| {
            entities.iter().copied().zip(0..entities.len()).map(move |(entity, row)| {
                unsafe {
                    let ptr = col.get_ptr(row) as *const T;
                    (entity, &*ptr)
                }
            })
        })
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

    /// Toplam bileşen sayısını döner.
    pub fn len(&self) -> usize {
        self.archetypes.iter().map(|(entities, _)| entities.len()).sum()
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
