use std::any::Any;

pub trait Component: 'static + Any {}

// Boş bir blanket implementation ile her Any olabilen ve 'static olan tipi Component yapabiliriz,
// ancak şimdilik kullanıcı manifestosuyla belirlemek daha güvenli olabilir.
impl<T: 'static + Any> Component for T {}

pub trait ComponentStorage {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn remove_entity(&mut self, entity: u32);
}

pub struct SparseSet<T: Component> {
    pub dense: Vec<T>,
    pub entity_dense: Vec<u32>,     // dense index -> entity id
    pub sparse: Vec<Option<usize>>, // entity id -> dense index
}

impl<T: Component> SparseSet<T> {
    pub fn new() -> Self {
        Self {
            dense: Vec::new(),
            entity_dense: Vec::new(),
            sparse: Vec::new(),
        }
    }

    pub fn insert(&mut self, entity: u32, component: T) {
        if entity as usize >= self.sparse.len() {
            self.sparse.resize(entity as usize + 1, None);
        }

        if let Some(dense_idx) = self.sparse[entity as usize] {
            // Zaten varsa yenisiyle değiştir
            self.dense[dense_idx] = component;
        } else {
            // Yeni ekle
            let dense_idx = self.dense.len();
            self.dense.push(component);
            self.entity_dense.push(entity);
            self.sparse[entity as usize] = Some(dense_idx);
        }
    }

    pub fn get(&self, entity: u32) -> Option<&T> {
        if entity as usize >= self.sparse.len() {
            return None;
        }
        self.sparse[entity as usize].map(|id| &self.dense[id])
    }

    pub fn get_mut(&mut self, entity: u32) -> Option<&mut T> {
        if entity as usize >= self.sparse.len() {
            return None;
        }
        if let Some(id) = self.sparse[entity as usize] {
            Some(&mut self.dense[id])
        } else {
            None
        }
    }

    pub fn remove(&mut self, entity: u32) -> Option<T> {
        if entity as usize >= self.sparse.len() { return None; }
        if let Some(dense_idx) = self.sparse[entity as usize] {
            let last_idx = self.dense.len() - 1;
            let last_entity = self.entity_dense[last_idx];
            
            self.dense.swap(dense_idx, last_idx);
            self.entity_dense.swap(dense_idx, last_idx);
            
            self.sparse[last_entity as usize] = Some(dense_idx);
            self.sparse[entity as usize] = None;
            
            self.entity_dense.pop();
            return self.dense.pop();
        }
        None
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
