use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use crossbeam_queue::SegQueue;

static NEXT_HANDLE_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandleId(pub usize);

impl Default for HandleId {
    fn default() -> Self {
        Self::new()
    }
}

impl HandleId {
    pub fn new() -> Self {
        HandleId(NEXT_HANDLE_ID.fetch_add(1, Ordering::Relaxed))
    }
}

pub struct HandleIdTracker {
    pub id: usize,
    pub drop_queue: Arc<SegQueue<usize>>,
}

impl Drop for HandleIdTracker {
    fn drop(&mut self) {
        self.drop_queue.push(self.id);
    }
}

pub struct Handle<T> {
    pub id: HandleId,
    pub tracker: Option<Arc<HandleIdTracker>>,
    _marker: PhantomData<T>,
}

impl<T> Default for Handle<T> {
    fn default() -> Self {
        Self::weak(HandleId::new())
    }
}

impl<T> Handle<T> {
    pub fn new(id: HandleId, drop_queue: Arc<SegQueue<usize>>) -> Self {
        Self {
            id,
            tracker: Some(Arc::new(HandleIdTracker {
                id: id.0,
                drop_queue,
            })),
            _marker: PhantomData,
        }
    }

    pub fn weak(id: HandleId) -> Self {
        Self {
            id,
            tracker: None,
            _marker: PhantomData,
        }
    }
    
    pub fn is_weak(&self) -> bool {
        self.tracker.is_none()
    }
    
    pub fn make_strong(&mut self, drop_queue: Arc<SegQueue<usize>>) {
        if self.tracker.is_none() {
            self.tracker = Some(Arc::new(HandleIdTracker {
                id: self.id.0,
                drop_queue,
            }));
        }
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            tracker: self.tracker.clone(),
            _marker: PhantomData,
        }
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
        if self.is_weak() {
            write!(f, "WeakHandle({:?})", self.id)
        } else {
            write!(f, "StrongHandle({:?})", self.id)
        }
    }
}

/// Generic resource to store assets by their handle ID.
pub struct Assets<T> {
    pub data: HashMap<HandleId, T>,
    drop_queue: Arc<SegQueue<usize>>,
}

impl<T> Default for Assets<T> {
    fn default() -> Self {
        Self {
            data: HashMap::new(),
            drop_queue: Arc::new(SegQueue::new()),
        }
    }
}

impl<T> Assets<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, asset: T) -> Handle<T> {
        let id = HandleId::new();
        self.data.insert(id, asset);
        Handle::new(id, self.drop_queue.clone())
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
    
    /// Collects dropped handles and removes their corresponding assets.
    pub fn process_drops(&mut self) {
        while let Some(dropped_id) = self.drop_queue.pop() {
            self.data.remove(&HandleId(dropped_id));
        }
    }
}
