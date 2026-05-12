use gizmo_core::World;
use gizmo_physics::components::Transform;
use std::collections::VecDeque;

/// Editör içinde yapılan geri alınabilir tekil bir işlem
#[derive(Clone, Debug)]
pub enum EditorAction {
    /// Objenin veya objelerin taşıma, dönme veya ölçeklenme değerlerinin değişmesi
    TransformsChanged {
        changes: Vec<(gizmo_core::entity::Entity, Transform, Transform)>, // (Entity, old_transform, new_transform)
    },
    /// Objelerin silinmesi (Soft Delete kullanılarak gizlenmiş)
    EntityDespawned { entity_ids: Vec<gizmo_core::entity::Entity> },
    /// Objelerin oluşturulması
    EntitySpawned {
        entity_ids: Vec<gizmo_core::entity::Entity>,
    },
    /// Dinamik / Diğer bileşenlerin değişimi
    ComponentChanged {
        entity: gizmo_core::entity::Entity,
        type_name: String, // Box<dyn Any> does not implement Clone across UI bounds easily, using typed names for future reflection implementation.
    },
}

/// Yapılan eylemlerin kaydını tutan History yöneticisi.
pub struct History {
    undo_stack: VecDeque<EditorAction>,
    redo_stack: VecDeque<EditorAction>,
    pub max_history: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(50) // Varsayılan 50 hamle hafızada kalsın (Prefs tarafından ezilir)
    }
}

impl History {
    pub fn new(max_history: usize) -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            max_history,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Yeni bir eylemi geçmişe kaydeder
    pub fn push(&mut self, action: EditorAction) {
        self.redo_stack.clear();
        self.undo_stack.push_back(action);

        if self.undo_stack.len() > self.max_history {
            self.undo_stack.pop_front();
        }
    }

    /// Son işlemi geri al (Undo) - Semantic Note: world state mutasyona uğratılır (interior mutability ile)
    pub fn undo(&mut self, world: &mut World) {
        if let Some(action) = self.undo_stack.pop_back() {
            match action.clone() {
                EditorAction::TransformsChanged { changes } => {
                    let mut transforms = world.borrow_mut::<Transform>();
                    {
                        for (entity, old_transform, _new_transform) in changes.iter() {
                            if let Some(t) = transforms.get_mut(entity.id()) {
                                *t = *old_transform;
                                t.update_local_matrix();
                            }
                        }
                    }
                    self.redo_stack
                        .push_back(EditorAction::TransformsChanged { changes });
                }
                EditorAction::EntityDespawned { entity_ids } => {
                    for entity in &entity_ids {
                        if let Some(ent) = world.get_entity(entity.id()) {
                            world.remove_component::<gizmo_core::component::IsDeleted>(ent);
                            world.remove_component::<gizmo_core::component::IsHidden>(ent);
                        }
                    }
                    self.redo_stack
                        .push_back(EditorAction::EntityDespawned { entity_ids });
                }
                EditorAction::EntitySpawned { entity_ids } => {
                    for entity in &entity_ids {
                        if let Some(ent) = world.get_entity(entity.id()) {
                            world.add_component(ent, gizmo_core::component::IsDeleted);
                            world.add_component(ent, gizmo_core::component::IsHidden);
                        }
                    }
                    self.redo_stack
                        .push_back(EditorAction::EntitySpawned { entity_ids });
                }
                _ => {
                    // Henüz implement edilmedi — stack'e geri koy
                    eprintln!("Uyarı: Bu action türü henüz geri alınamıyor (Undo desteklenmiyor).");
                    self.undo_stack.push_back(action);
                }
            }
        }
    }

    /// Geri alınan işlemi yeniden uygula (Redo) - Semantic Note: world state mutasyona uğratılır (interior mutability)
    pub fn redo(&mut self, world: &mut World) {
        if let Some(action) = self.redo_stack.pop_back() {
            match action.clone() {
                EditorAction::TransformsChanged { changes } => {
                    let mut transforms = world.borrow_mut::<Transform>();
                    {
                        for (entity, _old_transform, new_transform) in changes.iter() {
                            if let Some(t) = transforms.get_mut(entity.id()) {
                                *t = *new_transform;
                                t.update_local_matrix();
                            }
                        }
                    }
                    self.undo_stack
                        .push_back(EditorAction::TransformsChanged { changes });
                }
                EditorAction::EntityDespawned { entity_ids } => {
                    for entity in &entity_ids {
                        if let Some(ent) = world.get_entity(entity.id()) {
                            world.add_component(ent, gizmo_core::component::IsDeleted);
                            world.add_component(ent, gizmo_core::component::IsHidden);
                        }
                    }
                    self.undo_stack
                        .push_back(EditorAction::EntityDespawned { entity_ids });
                }
                EditorAction::EntitySpawned { entity_ids } => {
                    for entity in &entity_ids {
                        if let Some(ent) = world.get_entity(entity.id()) {
                            world.remove_component::<gizmo_core::component::IsDeleted>(ent);
                            world.remove_component::<gizmo_core::component::IsHidden>(ent);
                        }
                    }
                    self.undo_stack
                        .push_back(EditorAction::EntitySpawned { entity_ids });
                }
                _ => {
                    eprintln!(
                        "Uyarı: Bu action türü henüz ileri alınamıyor (Redo desteklenmiyor)."
                    );
                    self.redo_stack.push_back(action);
                }
            }
        }
    }
}
