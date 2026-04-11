use gizmo_core::World;
use gizmo_physics::components::Transform;

/// Editör içinde yapılan geri alınabilir tekil bir işlem
#[derive(Clone, Debug)]
pub enum EditorAction {
    /// Objenin veya objelerin taşıma, dönme veya ölçeklenme değerlerinin değişmesi
    TransformsChanged {
        changes: Vec<(u32, Transform, Transform)>, // (entity_id, old_transform, new_transform)
    },
    /// Objelerin silinmesi
    EntityDespawned { data: Vec<Vec<u8>> },
    /// Objelerin oluşturulması
    EntitySpawned { entity_ids: Vec<u32> },
}

/// Yapılan eylemlerin kaydını tutan History yöneticisi.
pub struct History {
    pub undo_stack: Vec<EditorAction>,
    pub redo_stack: Vec<EditorAction>,
    max_history: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(50) // Varsayılan 50 hamle hafızada kalsın
    }
}

impl History {
    pub fn new(max_history: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_history,
        }
    }

    /// Yeni bir eylemi geçmişe kaydeder
    pub fn push(&mut self, action: EditorAction) {
        self.redo_stack.clear();
        self.undo_stack.push(action);

        if self.undo_stack.len() > self.max_history {
            self.undo_stack.remove(0);
        }
    }

    /// Son işlemi geri al (Undo)
    pub fn undo(&mut self, world: &World) {
        if let Some(action) = self.undo_stack.pop() {
            match action {
                EditorAction::TransformsChanged { changes } => {
                    if let Some(mut transforms) = world.borrow_mut::<Transform>() {
                        for (entity_id, old_transform, _new_transform) in changes.iter() {
                            if let Some(t) = transforms.get_mut(*entity_id) {
                                *t = *old_transform;
                                t.update_local_matrix();
                            }
                        }
                    }
                    self.redo_stack
                        .push(EditorAction::TransformsChanged { changes });
                }
                _ => {} // Gelecekte diğer actionlar burada geri alınacak
            }
        }
    }

    /// Geri alınan işlemi yeniden uygula (Redo)
    pub fn redo(&mut self, world: &World) {
        if let Some(action) = self.redo_stack.pop() {
            match action {
                EditorAction::TransformsChanged { changes } => {
                    if let Some(mut transforms) = world.borrow_mut::<Transform>() {
                        for (entity_id, _old_transform, new_transform) in changes.iter() {
                            if let Some(t) = transforms.get_mut(*entity_id) {
                                *t = *new_transform;
                                t.update_local_matrix();
                            }
                        }
                    }
                    self.undo_stack
                        .push(EditorAction::TransformsChanged { changes });
                }
                _ => {} // Gelecekte diğer actionlar burada ileri alınacak
            }
        }
    }
}
