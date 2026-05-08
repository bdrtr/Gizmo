use crate::world::World;

/// Oyundaki mantıksal durumları yönetmek için kullanılan State yapısı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State<S: Clone + PartialEq + Eq + Send + Sync + 'static> {
    current: S,
    next: Option<S>,
}

impl<S: Clone + PartialEq + Eq + Send + Sync + 'static> State<S> {
    pub fn new(initial: S) -> Self {
        Self {
            current: initial,
            next: None,
        }
    }

    pub fn get(&self) -> &S {
        &self.current
    }

    pub fn set(&mut self, state: S) {
        if self.current != state {
            self.next = Some(state);
        }
    }

    /// Bir sonraki durumu aktif duruma geçirir. (Genellikle PreUpdate fazında çalıştırılır).
    pub fn apply_transitions(&mut self) -> bool {
        if let Some(next) = self.next.take() {
            self.current = next;
            true
        } else {
            false
        }
    }
}

/// Sistemin sadece belirli bir state'teyken çalışmasını sağlayan "Run Condition" fonksiyonu.
pub fn in_state<S>(state: S) -> impl FnMut(&World) -> bool + Send + Sync + 'static
where
    S: Clone + PartialEq + Eq + Send + Sync + 'static,
{
    move |world: &World| {
        if let Some(current_state) = world.get_resource::<State<S>>() {
            *current_state.get() == state
        } else {
            false
        }
    }
}
