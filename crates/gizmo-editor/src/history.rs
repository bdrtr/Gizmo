use gizmo_core::World;
use gizmo_physics_core::Transform;
use std::collections::VecDeque;

/// A single undoable operation performed in the editor
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum EditorAction {
    /// An object's (or several objects') translation, rotation or scale changed
    TransformsChanged {
        /// One entry per entity: `(entity, the transform before, the transform after)`.
        changes: Vec<(gizmo_core::entity::Entity, Transform, Transform)>,
    },
    /// Objects were deleted (hidden by a soft delete)
    EntityDespawned {
        /// The entities that were hidden, and that an undo brings back.
        entity_ids: Vec<gizmo_core::entity::Entity>,
    },
    /// Objects were created
    EntitySpawned {
        /// The entities that were created, and that an undo soft-deletes again.
        entity_ids: Vec<gizmo_core::entity::Entity>,
    },
    /// The selected entity's animation clips, before and after a timeline edit.
    ///
    /// Snapshots the **whole** `Arc<[AnimationClip]>` rather than a "keyframe #3 moved from 1.2s
    /// to 2.4s" record. Two reasons, and the second is the one that matters:
    ///
    /// 1. `AnimationPlayer::animations` is documented as immutable and swapped wholesale, so both
    ///    halves already exist at the edit site. An `Arc` clone is a refcount bump — holding both
    ///    does not copy any keyframe data.
    /// 2. An index-based record breaks the moment an edit **reorders** the list, and a retime
    ///    reorders it by definition: keyframes must stay sorted, so dragging one past its
    ///    neighbour changes what every later index refers to. Undoing by index would then move a
    ///    different keyframe than the one the user dragged.
    AnimationClipsChanged {
        /// The entity whose player was edited.
        entity: gizmo_core::entity::Entity,
        /// Its clips before the edit.
        before: std::sync::Arc<[gizmo_renderer::AnimationClip]>,
        /// Its clips after it.
        after: std::sync::Arc<[gizmo_renderer::AnimationClip]>,
    },
    /// A dynamic or otherwise unclassified component changed
    ComponentChanged {
        /// The entity whose component changed.
        entity: gizmo_core::entity::Entity,
        /// The component's type name.
        ///
        /// A name rather than the value: `Box<dyn Any>` is not `Clone` across the UI boundary, so
        /// this variant records *that* something changed and is not undoable yet — `undo` puts it
        /// back on the stack rather than dropping it.
        type_name: String,
    },
}

/// The history manager, which keeps the record of what was done.
#[non_exhaustive]
pub struct History {
    undo_stack: VecDeque<EditorAction>,
    redo_stack: VecDeque<EditorAction>,
    /// How many entries the undo stack keeps before dropping its oldest.
    pub max_history: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(50) // Varsayılan 50 hamle hafızada kalsın (Prefs tarafından ezilir)
    }
}

impl History {
    /// An empty history that keeps at most `max_history` entries.
    pub fn new(max_history: usize) -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            max_history,
        }
    }

    /// Is there anything to undo?
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Is there anything to redo?
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// The recorded actions, oldest first — a read-only view.
    ///
    /// [`History::can_undo`] answers whether there is *an* action; this answers **what**, which
    /// is what a caller needs to describe the next undo in the UI, and what a test needs to
    /// check that an operation recorded exactly the work it did. The delete cascade, for one,
    /// pushes an id per entity it VISITS, so the length of its `EntityDespawned` entry is the
    /// only place a duplicated visit is observable — tagging is idempotent and leaves no trace.
    pub fn undo_stack(&self) -> &VecDeque<EditorAction> {
        &self.undo_stack
    }

    /// Records a new action in the history
    pub fn push(&mut self, action: EditorAction) {
        self.redo_stack.clear();
        self.undo_stack.push_back(action);

        if self.undo_stack.len() > self.max_history {
            self.undo_stack.pop_front();
        }
    }

    /// Undo the last operation. Note: the world state is mutated (through interior mutability).
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
                    let mut transforms = world.borrow_mut::<Transform>();
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
                        // Resolve generation-safely: `get_entity(id)` looks up a bare
                        // SLOT and returns whatever entity lives there now, so after the
                        // GC recycles the slot an undo would mutate a DIFFERENT entity.
                        // `is_alive` compares the RECORDED generation, so we skip a
                        // recycled/dead handle instead.
                        if world.is_alive(*entity) {
                            world.remove_component::<gizmo_core::component::IsDeleted>(*entity);
                            world.remove_component::<gizmo_core::component::IsHidden>(*entity);
                        }
                    }
                    self.redo_stack
                        .push_back(EditorAction::EntityDespawned { entity_ids });
                }
                EditorAction::EntitySpawned { entity_ids } => {
                    for entity in &entity_ids {
                        if world.is_alive(*entity) {
                            world.add_component(*entity, gizmo_core::component::IsDeleted);
                            world.add_component(*entity, gizmo_core::component::IsHidden);
                        }
                    }
                    self.redo_stack
                        .push_back(EditorAction::EntitySpawned { entity_ids });
                }
                EditorAction::AnimationClipsChanged { entity, before, after } => {
                    // Generation-safe like the arms above: a recycled slot holds a DIFFERENT
                    // entity, and writing a clip set onto it would replace whatever animation it
                    // actually has.
                    if world.is_alive(entity) {
                        let mut players =
                            world.borrow_mut::<gizmo_renderer::components::AnimationPlayer>();
                        if let Some(mut p) = players.get_mut(entity.id()) {
                            p.animations = before.clone();
                        }
                    }
                    self.redo_stack
                        .push_back(EditorAction::AnimationClipsChanged { entity, before, after });
                }
                other => {
                    // Henüz implement edilmedi — stack'e geri koy
                    tracing::error!("Uyarı: Bu action türü henüz geri alınamıyor (Undo desteklenmiyor).");
                    self.undo_stack.push_back(other);
                }
            }
        }
    }

    /// Redo the operation that was undone. Note: the world state is mutated (through interior
    /// mutability).
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
                    let mut transforms = world.borrow_mut::<Transform>();
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
                        // Generation-safe (see the undo path): skip a GC-recycled slot
                        // instead of soft-deleting a different entity now living there.
                        if world.is_alive(*entity) {
                            world.add_component(*entity, gizmo_core::component::IsDeleted);
                            world.add_component(*entity, gizmo_core::component::IsHidden);
                        }
                    }
                    self.undo_stack
                        .push_back(EditorAction::EntityDespawned { entity_ids });
                }
                EditorAction::EntitySpawned { entity_ids } => {
                    for entity in &entity_ids {
                        if world.is_alive(*entity) {
                            world.remove_component::<gizmo_core::component::IsDeleted>(*entity);
                            world.remove_component::<gizmo_core::component::IsHidden>(*entity);
                        }
                    }
                    self.undo_stack
                        .push_back(EditorAction::EntitySpawned { entity_ids });
                }
                EditorAction::AnimationClipsChanged { entity, before, after } => {
                    if world.is_alive(entity) {
                        let mut players =
                            world.borrow_mut::<gizmo_renderer::components::AnimationPlayer>();
                        if let Some(mut p) = players.get_mut(entity.id()) {
                            p.animations = after.clone();
                        }
                    }
                    self.undo_stack
                        .push_back(EditorAction::AnimationClipsChanged { entity, before, after });
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

    use gizmo_renderer::components::AnimationPlayer;
    use gizmo_renderer::{AnimationClip, InterpolationMode, Keyframe, Track};

    fn clips(times: &[f32], duration: f32) -> std::sync::Arc<[AnimationClip]> {
        std::sync::Arc::new([AnimationClip {
            name: "test".to_string(),
            duration,
            translations: vec![Track {
                target_node: 0,
                target_node_name: None,
                interpolation: InterpolationMode::Linear,
                keyframes: times
                    .iter()
                    .map(|&t| Keyframe {
                        time: t,
                        value: Vec3::ZERO,
                        in_tangent: None,
                        out_tangent: None,
                    })
                    .collect(),
            }],
            rotations: Vec::new(),
            scales: Vec::new(),
        }])
    }

    fn keyframe_times(world: &World, entity: gizmo_core::entity::Entity) -> Vec<f32> {
        world
            .borrow::<AnimationPlayer>()
            .get(entity.id())
            .map(|p| {
                p.animations[0].translations[0]
                    .keyframes
                    .iter()
                    .map(|k| k.time)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The timeline's undo round trip. The snapshot is the whole clip set, so undo has to put
    /// back both the keyframe positions AND the duration that grew to cover them.
    #[test]
    fn animation_undo_restores_the_clips_and_redo_reapplies_them() {
        let mut world = World::new();
        let e = world.spawn();
        let before = clips(&[0.0, 1.0, 2.0], 2.0);
        let after = clips(&[1.0, 2.0, 5.0], 5.0);
        world.add_component(
            e,
            AnimationPlayer { animations: after.clone(), ..Default::default() },
        );

        let mut history = History::new(10);
        history.push(EditorAction::AnimationClipsChanged {
            entity: e,
            before: before.clone(),
            after: after.clone(),
        });

        history.undo(&mut world);
        assert_eq!(keyframe_times(&world, e), vec![0.0, 1.0, 2.0]);
        assert_eq!(
            world.borrow::<AnimationPlayer>().get(e.id()).unwrap().animations[0].duration,
            2.0,
            "undo must restore the duration too — it grew with the edit"
        );

        history.redo(&mut world);
        assert_eq!(keyframe_times(&world, e), vec![1.0, 2.0, 5.0]);
        assert_eq!(
            world.borrow::<AnimationPlayer>().get(e.id()).unwrap().animations[0].duration,
            5.0
        );
    }

    /// Same generation-safety rule as the transform arm: after the GC recycles a slot, an undo
    /// must not write a clip set onto whatever different entity now lives there — which would
    /// replace an unrelated object's animation outright.
    #[test]
    fn animation_undo_is_generation_safe_after_slot_recycle() {
        let mut world = World::new();
        let a = world.spawn();
        world.add_component(a, AnimationPlayer { animations: clips(&[9.0], 9.0), ..Default::default() });

        let mut history = History::new(10);
        history.push(EditorAction::AnimationClipsChanged {
            entity: a,
            before: clips(&[0.0], 0.0),
            after: clips(&[9.0], 9.0),
        });

        world.despawn(a);
        let b = world.spawn();
        world.add_component(b, AnimationPlayer { animations: clips(&[7.0], 7.0), ..Default::default() });

        history.undo(&mut world);

        assert_eq!(
            keyframe_times(&world, b),
            vec![7.0],
            "B's own animation must survive an undo aimed at the entity that used to hold its slot"
        );
    }

    /// After the GC recycles a slot, a `TransformsChanged` undo must not overwrite a DIFFERENT
    /// object now living in that slot.
    ///
    /// The old code looked at the bare slot id via `transforms.get_mut(entity.id())`; this test
    /// FAILS against that code (B's Transform is overwritten with old_a) and PASSES with the
    /// generation-safe fix.
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

        let mut transforms = world.borrow_mut::<Transform>();
        let t_b = transforms
            .get_mut(entity_b.id())
            .expect("B'nin Transform'u mevcut olmalı");
        assert_eq!(
            t_b.position, b_pos,
            "generation-safe olmayan undo, geri dönüştürülen B'nin Transform'unu old_a ile ezerdi"
        );
    }

    /// The same generation-safety guarantee for the redo() arm.
    /// The old code overwrote B with new_a through `get_mut(entity.id())` and so FAILS; it
    /// PASSES with the fix.
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

        let mut transforms = world.borrow_mut::<Transform>();
        let t_b = transforms
            .get_mut(entity_b.id())
            .expect("B'nin Transform'u mevcut olmalı");
        assert_eq!(
            t_b.position, b_pos,
            "generation-safe olmayan redo, geri dönüştürülen B'nin Transform'unu new_a ile ezerdi"
        );
    }

    /// The control: when the entity is STILL alive, undo really must restore the old Transform
    /// (the guard must not break the happy path).
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

        let mut transforms = world.borrow_mut::<Transform>();
        let t = transforms.get_mut(entity.id()).expect("Transform mevcut");
        assert_eq!(
            t.position,
            Vec3::new(1.0, 2.0, 3.0),
            "canlı entity için undo eski (old) değeri geri yüklemeli"
        );
    }

    // === Yardımcılar ===

    /// Returns a World with Transform storage registered (an empty-changes undo calls
    /// `borrow_mut::<Transform>()`, so the type has to be registered).
    fn world_with_transform_storage() -> World {
        let mut w = World::new();
        let e = w.spawn();
        w.add_component(e, Transform::new(Vec3::ZERO));
        w
    }

    fn is_soft_deleted(world: &World, e: gizmo_core::entity::Entity) -> bool {
        world
            .borrow::<gizmo_core::component::IsDeleted>()
            .get(e.id())
            .is_some()
    }

    fn is_hidden(world: &World, e: gizmo_core::entity::Entity) -> bool {
        world
            .borrow::<gizmo_core::component::IsHidden>()
            .get(e.id())
            .is_some()
    }

    // === Stack invariant'ları ===

    #[test]
    fn empty_history_cannot_undo_or_redo() {
        let h = History::new(10);
        assert!(!h.can_undo());
        assert!(!h.can_redo());
    }

    #[test]
    fn default_history_max_is_50() {
        assert_eq!(History::default().max_history, 50);
    }

    /// A new push must clear the redo stack (after undo→push, redo is gone).
    #[test]
    fn push_clears_redo_stack() {
        let mut world = world_with_transform_storage();
        let mut h = History::new(10);

        h.push(EditorAction::TransformsChanged { changes: vec![] });
        assert!(h.can_undo());

        // undo → kayıt redo_stack'e taşınır
        h.undo(&mut world);
        assert!(h.can_redo());
        assert!(!h.can_undo());

        // yeni push → redo_stack temizlenmeli
        h.push(EditorAction::TransformsChanged { changes: vec![] });
        assert!(!h.can_redo(), "yeni push sonrası redo geçmişi silinmeli");
        assert!(h.can_undo());
    }

    /// Past max_history the oldest record must be dropped (ring-buffer behaviour).
    #[test]
    fn max_history_evicts_oldest_when_capacity_exceeded() {
        let mut world = world_with_transform_storage();
        let mut h = History::new(2);

        for _ in 0..3 {
            h.push(EditorAction::TransformsChanged { changes: vec![] });
        }

        // Kapasite 2 → yalnızca 2 undo mümkün olmalı.
        let mut undo_count = 0;
        while h.can_undo() {
            h.undo(&mut world);
            undo_count += 1;
            assert!(undo_count <= 3, "sonsuz döngü koruması");
        }
        assert_eq!(undo_count, 2, "kapasite aşımında en eski kayıt düşmeliydi");
    }

    // === Spawn / Despawn round-trip (soft-delete işaretçileri) ===

    /// EntitySpawned: undo objeyi soft-delete + gizler; redo geri getirir.
    #[test]
    fn entity_spawned_undo_hides_then_redo_restores() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));

        let mut h = History::new(10);
        h.push(EditorAction::EntitySpawned { entity_ids: vec![e] });

        // Başlangıçta ne silinmiş ne gizli
        assert!(!is_soft_deleted(&world, e));
        assert!(!is_hidden(&world, e));

        // undo → spawn geri alınır (soft-delete + hide)
        h.undo(&mut world);
        assert!(is_soft_deleted(&world, e));
        assert!(is_hidden(&world, e));
        assert!(h.can_redo());

        // redo → obje yeniden görünür (işaretçiler kalkar)
        h.redo(&mut world);
        assert!(!is_soft_deleted(&world, e));
        assert!(!is_hidden(&world, e));
        assert!(h.can_undo());
    }

    /// EntityDespawned: undo removes the markers and brings the object back; redo soft-deletes
    /// and hides it again.
    #[test]
    fn entity_despawned_undo_restores_then_redo_hides() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        // Silinmiş durumu simüle et
        world.add_component(e, gizmo_core::component::IsDeleted);
        world.add_component(e, gizmo_core::component::IsHidden);

        let mut h = History::new(10);
        h.push(EditorAction::EntityDespawned { entity_ids: vec![e] });

        // undo → despawn geri alınır (işaretçiler kalkar, obje geri gelir)
        h.undo(&mut world);
        assert!(!is_soft_deleted(&world, e));
        assert!(!is_hidden(&world, e));

        // redo → tekrar despawn (soft-delete + hide)
        h.redo(&mut world);
        assert!(is_soft_deleted(&world, e));
        assert!(is_hidden(&world, e));
    }

    /// An action type that is not implemented yet (ComponentChanged) must not be LOST when
    /// undone — it goes back onto the undo stack and redo stays empty.
    #[test]
    fn unsupported_action_is_preserved_on_undo() {
        let mut world = World::new();
        let e = world.spawn();

        let mut h = History::new(10);
        h.push(EditorAction::ComponentChanged {
            entity: e,
            type_name: "Velocity".to_string(),
        });

        h.undo(&mut world);
        // Geri konduğu için hâlâ undo edilebilir, redo'ya taşınmamış olmalı
        assert!(h.can_undo(), "desteklenmeyen action stack'te korunmalı");
        assert!(!h.can_redo(), "desteklenmeyen action redo'ya taşınmamalı");
    }

    /// Spawn/despawn undos must be generation-safe too: after the GC recycles a slot, an undo
    /// must NOT soft-delete the DIFFERENT object living there.
    #[test]
    fn entity_spawned_undo_is_generation_safe_after_recycle() {
        let mut world = World::new();
        let entity_a = world.spawn();
        world.add_component(entity_a, Transform::new(Vec3::ZERO));

        let mut h = History::new(10);
        h.push(EditorAction::EntitySpawned {
            entity_ids: vec![entity_a],
        });

        world.despawn(entity_a);
        let entity_b = world.spawn();
        assert_eq!(entity_b.id(), entity_a.id());
        assert_ne!(entity_b.generation(), entity_a.generation());
        world.add_component(entity_b, Transform::new(Vec3::ZERO));

        // undo: A canlı değil → B soft-delete/hide EDİLMEMELİ.
        h.undo(&mut world);
        assert!(
            !is_soft_deleted(&world, entity_b),
            "geri dönüştürülen B soft-delete edilmemeli"
        );
        assert!(
            !is_hidden(&world, entity_b),
            "geri dönüştürülen B gizlenmemeli"
        );
    }
}
