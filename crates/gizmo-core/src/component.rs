use std::any::Any;
use std::collections::HashMap;

pub trait Component: 'static + Any {}

#[macro_export]
macro_rules! impl_component {
    ($($t:ty),+ $(,)?) => {
        $(
            impl $crate::Component for $t {}
        )+
    };
}

pub trait ComponentStorage {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn get_component_as_any(&self, entity: u32) -> Option<&dyn Any>;
    fn get_component_mut_as_any(&mut self, entity: u32) -> Option<&mut dyn Any>;
    fn remove_entity(&mut self, entity: u32);
}

#[derive(Debug, Clone)]
pub struct DenseEntry<T> {
    pub entity: u32,
    pub data: T,
}

pub struct SparseSet<T: Component> {
    pub(crate) dense: Vec<DenseEntry<T>>,
    pub(crate) sparse: HashMap<u32, usize>, // entity id → dense index (O(1) lookup)
}

impl<T: Component> SparseSet<T> {
    pub fn new() -> Self {
        Self {
            dense: Vec::new(),
            sparse: HashMap::new(),
        }
    }

    #[inline]
    pub fn insert(&mut self, entity: u32, component: T) {
        if let Some(&dense_idx) = self.sparse.get(&entity) {
            // Zaten varsa yenisiyle değiştir (overwrite)
            self.dense[dense_idx].data = component;
        } else {
            // Yeni ekle — dense sona eklenir
            let dense_idx = self.dense.len();
            self.dense.push(DenseEntry {
                entity,
                data: component,
            });
            self.sparse.insert(entity, dense_idx);
        }
    }

    #[inline]
    pub fn get(&self, entity: u32) -> Option<&T> {
        self.sparse.get(&entity).map(|&id| &self.dense[id].data)
    }

    #[inline]
    pub fn get_mut(&mut self, entity: u32) -> Option<&mut T> {
        if let Some(&id) = self.sparse.get(&entity) {
            Some(&mut self.dense[id].data)
        } else {
            None
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &T)> {
        self.dense.iter().map(|entry| (entry.entity, &entry.data))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T)> {
        self.dense.iter_mut().map(|entry| (entry.entity, &mut entry.data))
    }

    /// Swap-and-pop çıkarma — O(1) amortized.
    /// Son elemanı silinen elemanın yerine koyar, dense dizisini bitişik tutar.
    pub fn remove(&mut self, entity: u32) -> Option<T> {
        if let Some(&dense_idx) = self.sparse.get(&entity) {
            if dense_idx >= self.dense.len() {
                // Koleksiyonlar arasında senkronizasyon kayması yaşanmış, güvenli bir şekilde dön
                #[cfg(debug_assertions)]
                eprintln!(
                    "Warning: SparseSet desync: entity {} idx {} len {}",
                    entity,
                    dense_idx,
                    self.dense.len()
                );
                self.sparse.remove(&entity); // Sparse'daki bozuk kaydı temizle
                return None;
            }

            self.sparse.remove(&entity);
            
            if self.dense.is_empty() {
                return None;
            }

            let last_idx = self.dense.len() - 1;

            if dense_idx != last_idx {
                let last_entity = self.dense[last_idx].entity;
                // Son elemanı silinen yere taşı
                self.dense.swap(dense_idx, last_idx);
                // Taşınan entity'nin sparse kaydını güncelle
                self.sparse.insert(last_entity, dense_idx);
            }

            self.dense.pop().map(|e| e.data)
        } else {
            None
        }
    }

    /// Depodaki toplam component sayısı
    #[inline]
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    /// Bu entity'de bu component var mı?
    #[inline]
    pub fn contains(&self, entity: u32) -> bool {
        self.sparse.contains_key(&entity)
    }
}

impl<T: Component> Default for SparseSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Component> ComponentStorage for SparseSet<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn get_component_as_any(&self, entity: u32) -> Option<&dyn Any> {
        self.get(entity).map(|c| c as &dyn Any)
    }
    fn get_component_mut_as_any(&mut self, entity: u32) -> Option<&mut dyn Any> {
        self.get_mut(entity).map(|c| c as &mut dyn Any)
    }
    fn remove_entity(&mut self, entity: u32) {
        self.remove(entity);
    }
}

// --- Hiyerarşi (Scene Graph) Bileşenleri ---
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Parent(pub u32);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Children(pub Vec<u32>);

/// Entity isim bileşeni — Editor, Lua ve Scene Serialization tarafından kullanılır.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EntityName(pub String);

impl EntityName {
    pub fn new(name: &str) -> Self {
        Self(name.to_string())
    }
}

/// Görünmezlik etiketi: Eğer bu component bir objede varsa render edilmez (veya aktifliği kapatılır).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IsHidden;

/// Prefab spawn talebi. Entity'ye eklendiğinde prefab yükleme sistemi tarafından işlenir
/// ve işlendikten sonra component kaldırılır.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PrefabRequest(pub String);

impl PrefabRequest {
    pub fn new(name: &str) -> Self {
        Self(name.to_string())
    }

    /// Prefab adını döndürür.
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for EntityName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl_component!(Parent, Children, EntityName, IsHidden, PrefabRequest);
