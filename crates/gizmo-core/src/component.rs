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

/// Yüksek performanslı ECS deposu.
/// 
/// `dense` vektörü bitişik bellekte tutulur → cache-friendly iterasyon.
/// `sparse` olarak HashMap kullanılır → O(1) lookup, sadece var olan entity'ler kadar bellek.
/// 
/// Bellek karşılaştırması (1000 entity, 10 component tipi):
///   Eski (Vec<Option<usize>>):  10 × 8KB sparse = 80KB sadece indeks!
///   Yeni (HashMap<u32, usize>): 10 × ~1KB hash  = ~10KB indeks (8× tasarruf)
pub struct SparseSet<T: Component> {
    pub dense: Vec<T>,
    pub entity_dense: Vec<u32>,           // dense index → entity id
    pub sparse: HashMap<u32, usize>,      // entity id → dense index (O(1) lookup)
}

impl<T: Component> SparseSet<T> {
    pub fn new() -> Self {
        Self {
            dense: Vec::new(),
            entity_dense: Vec::new(),
            sparse: HashMap::new(),
        }
    }

    #[inline]
    pub fn insert(&mut self, entity: u32, component: T) {
        if let Some(&dense_idx) = self.sparse.get(&entity) {
            // Zaten varsa yenisiyle değiştir (overwrite)
            self.dense[dense_idx] = component;
        } else {
            // Yeni ekle — dense sona eklenir
            let dense_idx = self.dense.len();
            self.dense.push(component);
            self.entity_dense.push(entity);
            self.sparse.insert(entity, dense_idx);
        }
    }

    #[inline]
    pub fn get(&self, entity: u32) -> Option<&T> {
        self.sparse.get(&entity).map(|&id| &self.dense[id])
    }

    #[inline]
    pub fn get_mut(&mut self, entity: u32) -> Option<&mut T> {
        if let Some(&id) = self.sparse.get(&entity) {
            Some(&mut self.dense[id])
        } else {
            None
        }
    }

    /// Swap-and-pop çıkarma — O(1) amortized.
    /// Son elemanı silinen elemanın yerine koyar, dense dizisini bitişik tutar.
    pub fn remove(&mut self, entity: u32) -> Option<T> {
        if let Some(dense_idx) = self.sparse.remove(&entity) {
            let last_idx = self.dense.len() - 1;

            if dense_idx != last_idx {
                let last_entity = self.entity_dense[last_idx];
                // Son elemanı silinen yere taşı
                self.dense.swap(dense_idx, last_idx);
                self.entity_dense.swap(dense_idx, last_idx);
                // Taşınan entity'nin sparse kaydını güncelle
                self.sparse.insert(last_entity, dense_idx);
            }

            self.entity_dense.pop();
            self.dense.pop()
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
