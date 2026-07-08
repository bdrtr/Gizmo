use gizmo_core::World;
use gizmo_physics_core::Transform;
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
            match action {
                EditorAction::TransformsChanged { changes } => {
                    // Generation-safe: yalnızca kaydedilen generation ile HÂLÂ canlı
                    // olan entity'lere uygula. GC slot'u geri dönüştürmüşse
                    // (is_alive == false), o slotta yaşayan BAŞKA bir objenin
                    // Transform'unu bozmamak için o girdiyi atla.
                    // Not: is_alive kaydedilen generation'ı karşılaştırır; bu yüzden
                    // Transform'u borrow etmeden ÖNCE canlı girdileri topluyoruz.
                    let to_apply: Vec<_> = changes
                        .iter()
                        .filter(|(entity, _, _)| world.is_alive(*entity))
                        .collect();
                    let transforms = world.borrow_mut::<Transform>();
                    for (entity, ref old_transform, _) in to_apply {
                        if let Some(mut t) = transforms.get_mut(entity.id()) {
                            *t = *old_transform;
                            t.update_local_matrix();
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
                other => {
                    // Henüz implement edilmedi — stack'e geri koy
                    tracing::error!("Uyarı: Bu action türü henüz geri alınamıyor (Undo desteklenmiyor).");
                    self.undo_stack.push_back(other);
                }
            }
        }
    }

    /// Geri alınan işlemi yeniden uygula (Redo) - Semantic Note: world state mutasyona uğratılır (interior mutability)
    pub fn redo(&mut self, world: &mut World) {
        if let Some(action) = self.redo_stack.pop_back() {
            match action {
                EditorAction::TransformsChanged { changes } => {
                    // Generation-safe: bkz. undo() — GC recycle etmiş slot'ta yaşayan
                    // farklı bir objeyi bozmamak için yalnızca hâlâ canlı entity'lere
                    // uygula. Transform borrow'undan önce canlı girdileri topla.
                    let to_apply: Vec<_> = changes
                        .iter()
                        .filter(|(entity, _, _)| world.is_alive(*entity))
                        .collect();
                    let transforms = world.borrow_mut::<Transform>();
                    for (entity, _, ref new_transform) in to_apply {
                        if let Some(mut t) = transforms.get_mut(entity.id()) {
                            *t = *new_transform;
                            t.update_local_matrix();
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
                other => {
                    tracing::error!(
                        "Uyarı: Bu action türü henüz ileri alınamıyor (Redo desteklenmiyor)."
                    );
                    self.redo_stack.push_back(other);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec3;

    /// GC bir slot'u geri dönüştürdükten sonra `TransformsChanged` undo'sunun
    /// o slotta artık yaşayan BAŞKA bir objeyi ezmemesi gerekir.
    ///
    /// Eski kod `transforms.get_mut(entity.id())` ile çıplak slot id'ye
    /// bakıyordu; bu test ESKİ kodda B'nin Transform'u old_a ile ezildiği için
    /// BAŞARISIZ olur, generation-safe düzeltmeyle GEÇER.
    #[test]
    fn transforms_undo_is_generation_safe_after_slot_recycle() {
        let mut world = World::new();

        // A: slot tahsis edilir ve bir Transform eklenir.
        let entity_a = world.spawn();
        world.add_component(entity_a, Transform::new(Vec3::new(1.0, 1.0, 1.0)));

        // A için bir TransformsChanged kaydı: old, dikkat çekici bir değer.
        let old_a = Transform::new(Vec3::new(5.0, 5.0, 5.0));
        let new_a = Transform::new(Vec3::new(1.0, 1.0, 1.0));
        let mut history = History::new(10);
        history.push(EditorAction::TransformsChanged {
            changes: vec![(entity_a, old_a, new_a)],
        });

        // A despawn → slot geri dönüşüme girer, generation artar.
        world.despawn(entity_a);

        // B: aynı slot'u geri dönüştürür (aynı id, farklı generation).
        let entity_b = world.spawn();
        assert_eq!(
            entity_b.id(),
            entity_a.id(),
            "test slot recycle'a dayanıyor; B, A ile aynı slot id'yi almalı"
        );
        assert_ne!(
            entity_b.generation(),
            entity_a.generation(),
            "geri dönüştürülen slot'un generation'ı artmış olmalı"
        );
        let b_pos = Vec3::new(9.0, 9.0, 9.0);
        world.add_component(entity_b, Transform::new(b_pos));

        // undo: A canlı DEĞİL → guard atlamalı, B'nin Transform'u DEĞİŞMEMELİ.
        history.undo(&mut world);

        let transforms = world.borrow_mut::<Transform>();
        let t_b = transforms
            .get_mut(entity_b.id())
            .expect("B'nin Transform'u mevcut olmalı");
        assert_eq!(
            t_b.position, b_pos,
            "generation-safe olmayan undo, geri dönüştürülen B'nin Transform'unu old_a ile ezerdi"
        );
    }

    /// redo() arm'ı için aynı generation-safety garantisi.
    /// Eski kod `get_mut(entity.id())` ile B'yi new_a ile ezdiği için BAŞARISIZ,
    /// düzeltmeyle GEÇER.
    #[test]
    fn transforms_redo_is_generation_safe_after_slot_recycle() {
        let mut world = World::new();

        let entity_a = world.spawn();
        world.add_component(entity_a, Transform::new(Vec3::new(1.0, 1.0, 1.0)));

        let old_a = Transform::new(Vec3::new(1.0, 1.0, 1.0));
        let new_a = Transform::new(Vec3::new(5.0, 5.0, 5.0));
        let mut history = History::new(10);
        history.push(EditorAction::TransformsChanged {
            changes: vec![(entity_a, old_a, new_a)],
        });

        // undo: A hâlâ canlıyken kaydı redo_stack'e taşır (old_a uygulanır).
        history.undo(&mut world);

        // A despawn + B spawn → slot recycle.
        world.despawn(entity_a);
        let entity_b = world.spawn();
        assert_eq!(entity_b.id(), entity_a.id());
        assert_ne!(entity_b.generation(), entity_a.generation());
        let b_pos = Vec3::new(9.0, 9.0, 9.0);
        world.add_component(entity_b, Transform::new(b_pos));

        // redo: A canlı DEĞİL → guard atlamalı, B DEĞİŞMEMELİ.
        history.redo(&mut world);

        let transforms = world.borrow_mut::<Transform>();
        let t_b = transforms
            .get_mut(entity_b.id())
            .expect("B'nin Transform'u mevcut olmalı");
        assert_eq!(
            t_b.position, b_pos,
            "generation-safe olmayan redo, geri dönüştürülen B'nin Transform'unu new_a ile ezerdi"
        );
    }

    /// Kontrol testi: entity HÂLÂ canlıysa undo gerçekten Transform'u eski
    /// değere geri almalı (guard mutlu-yolu bozmamalı).
    #[test]
    fn transforms_undo_applies_when_entity_still_alive() {
        let mut world = World::new();

        let entity = world.spawn();
        world.add_component(entity, Transform::new(Vec3::new(3.0, 3.0, 3.0)));

        let old = Transform::new(Vec3::new(1.0, 2.0, 3.0));
        let new = Transform::new(Vec3::new(3.0, 3.0, 3.0));
        let mut history = History::new(10);
        history.push(EditorAction::TransformsChanged {
            changes: vec![(entity, old, new)],
        });

        history.undo(&mut world);

        let transforms = world.borrow_mut::<Transform>();
        let t = transforms.get_mut(entity.id()).expect("Transform mevcut");
        assert_eq!(
            t.position,
            Vec3::new(1.0, 2.0, 3.0),
            "canlı entity için undo eski (old) değeri geri yüklemeli"
        );
    }
}
