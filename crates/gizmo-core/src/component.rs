use std::any::Any;
use std::collections::HashMap;

pub trait Component: 'static + Any {}

// Blanket implementation — her 'static + Any tip otomatik Component olur
impl<T: 'static + Any> Component for T {}

pub trait ComponentStorage {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn remove_entity(&mut self, entity: u32);
}

#[derive(Debug, Clone)]
pub struct DenseEntry<T> {
    pub entity: u32,
    pub data: T,
}

pub struct SparseSet<T: Component> {
    pub dense: Vec<DenseEntry<T>>,
    pub sparse: HashMap<u32, usize>, // entity id → dense index (O(1) lookup)
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

    /// Swap-and-pop çıkarma — O(1) amortized.
    /// Son elemanı silinen elemanın yerine koyar, dense dizisini bitişik tutar.
    pub fn remove(&mut self, entity: u32) -> Option<T> {
        if let Some(dense_idx) = self.sparse.remove(&entity) {
            if dense_idx >= self.dense.len() {
                // Koleksiyonlar arasında senkronizasyon kayması yaşanmış, güvenli bir şekilde dön
                crate::gizmo_log!(
                    Warning,
                    "SparseSet desync: entity {} idx {} len {}",
                    entity,
                    dense_idx,
                    self.dense.len()
                );
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

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PrefabRequest(pub String);

impl PrefabRequest {
    pub fn new(name: &str) -> Self {
        Self(name.to_string())
    }
}

impl std::fmt::Display for EntityName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
