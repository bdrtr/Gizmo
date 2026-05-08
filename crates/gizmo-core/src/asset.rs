use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::hash::{Hash, Hasher};
use std::collections::HashMap;

static NEXT_HANDLE_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandleId(pub usize);

impl HandleId {
    pub fn new() -> Self {
        HandleId(NEXT_HANDLE_ID.fetch_add(1, Ordering::Relaxed))
    }
}

pub struct Handle<T> {
    pub id: HandleId,
    _marker: PhantomData<T>,
}

impl<T> Handle<T> {
    pub fn new() -> Self {
        Self { id: HandleId::new(), _marker: PhantomData }
    }

    pub fn weak(id: HandleId) -> Self {
        Self { id, _marker: PhantomData }
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self { id: self.id, _marker: PhantomData }
    }
}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: 'static + Send + Sync> crate::component::Component for Handle<T> {}

impl<T> std::fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Handle({:?})", self.id)
    }
}

/// Generic resource to store assets by their handle ID.
pub struct Assets<T> {
    pub data: HashMap<HandleId, T>,
}

impl<T> Default for Assets<T> {
    fn default() -> Self {
        Self { data: HashMap::new() }
    }
}

impl<T> Assets<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, asset: T) -> Handle<T> {
        let id = HandleId::new();
        self.data.insert(id, asset);
        Handle::weak(id)
    }

    pub fn get(&self, handle: &Handle<T>) -> Option<&T> {
        self.data.get(&handle.id)
    }

    pub fn get_mut(&mut self, handle: &Handle<T>) -> Option<&mut T> {
        self.data.get_mut(&handle.id)
    }

    pub fn insert(&mut self, handle: &Handle<T>, asset: T) {
        self.data.insert(handle.id, asset);
    }

    pub fn remove(&mut self, handle: &Handle<T>) -> Option<T> {
        self.data.remove(&handle.id)
    }
}
