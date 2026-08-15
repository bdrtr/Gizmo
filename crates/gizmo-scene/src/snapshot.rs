//! In-memory scene backup for the editor's Play/Stop mode.
//!
//! Pressing **Play** captures the editable part of the world
//! ([`SceneSnapshot::capture`]); pressing **Stop** writes it back
//! ([`SceneSnapshot::restore`]). Nothing goes through the filesystem and no RON *file* is
//! produced, so a capture costs microseconds and can be taken every single time the user
//! starts the simulation. The in-tree caller is `gizmo-studio`'s `render_studio`, which
//! parks the snapshot in `EditorState::play_snapshot` for the duration of Play.
//!
//! Why this exists next to [`SceneData`](crate::scene::SceneData), which can already
//! round-trip a world through disk:
//!
//! - No file I/O and no RON *file* to write or parse on a path the user hits constantly.
//!   Individual components are still RON-encoded per entity, in both directions — see
//!   [`RestoreResult::duration`].
//! - [`restore`](SceneSnapshot::restore) **overwrites entities in place** instead of
//!   clearing the scene and respawning it, so everything the
//!   [`SceneRegistry`](crate::SceneRegistry) does *not* know about — mesh and material
//!   handles, hierarchy links, renderer-side state — survives Stop. Until 2026-05-14 restore
//!   really did despawn every non-protected entity and re-spawn bare copies, and objects
//!   visually vanished the moment the user pressed Stop.
//!
//! What it is **not**: a save format, and not a physics rollback.
//! [`SceneSnapshot`] holds an `Instant` and is deliberately not `Serialize`; only the
//! components in the registry handed to `capture` are stored (so no `MeshSource`,
//! `MaterialSource` or `Parent`, unlike [`EntityData`](crate::scene::EntityData)); the
//! `PhysicsWorld` resource — joints, contact caches, broadphase — is touched by neither
//! half; and `#[serde(skip)]` fields inside captured components (e.g. `Velocity`'s
//! `pre_linear`/`pre_angular` integrator history) come back at their `Default`, not at
//! their Play-time values.

use gizmo_core::World;

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// One entity's captured state: its name plus every registry-known component, each already
/// serialized to a RON string.
///
/// Only [`SceneSnapshot::capture`] produces these, and it drops three groups on the floor:
/// protected ids, the editor's own scaffolding, and entities that would produce an empty row
/// (no name and no registered component). The first two are skipped again by
/// [`SceneSnapshot::restore`] and so survive untouched; the third is not — a row-less entity
/// is indistinguishable from one Play created, and Stop despawns it.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct EntitySnapshot {
    /// Raw ECS entity id (the `u32` slot), with the generation deliberately dropped.
    ///
    /// Restore matches on this id alone. If a live entity still occupies the slot its
    /// components are overwritten in place — that is what keeps mesh/material handles alive
    /// across Play→Stop; if the slot is free, a replacement is spawned and, because the
    /// allocator's free list is FIFO, it may come back under a *different* id. Two
    /// consequences follow: raw ids held elsewhere (`Parent` links) can dangle after Stop,
    /// and a Play-time entity that happens to sit on a captured id is not despawned but
    /// written over.
    pub entity_id: u32,
    /// Copy of the entity's [`EntityName`](gizmo_core::EntityName) at capture time, `None`
    /// for unnamed entities.
    ///
    /// The only state captured outside the registry, and the reason a bare marker/group
    /// entity is snapshotted at all — the capture filter keeps a row when it has a name *or*
    /// at least one component. Restore re-adds it through `EntityName::new`, so the name
    /// returns even on a respawned entity, unlike its mesh or its place in the hierarchy.
    /// Names are also the editor filter: an entity called `"Editor …"` or `"Highlight Box"`
    /// is never captured in the first place.
    pub name: Option<String>,
    /// Registered components keyed by their [`SceneRegistry`](crate::SceneRegistry) name
    /// (`"Transform"`, `"RigidBody"`, …), each value the component serialized to a RON
    /// string — through `bevy_reflect` when the `reflect` feature is on, through plain
    /// `serde` otherwise.
    ///
    /// A `BTreeMap`, so restore re-inserts components in name order rather than in the
    /// archetype order they were read in: deterministic, but not the original insertion
    /// order. Only what the registry passed to `capture` knows about lands here; anything
    /// else (renderer handles, `Parent`/`Children`) is simply absent. A component whose
    /// serializer fails is dropped from this map and counted in the `tracing::warn!` that
    /// capture emits — that state is gone at Stop, and the warning is the only trace of it.
    pub components: std::collections::BTreeMap<String, String>,
}

/// A whole scene held in memory, ready to be put back by [`restore`](Self::restore).
///
/// Cheap to take and cheap to hold — a `Vec` of per-entity maps of short RON strings — which
/// is why the editor keeps one alive for the entire Play session. It describes one specific
/// `World`: restoring into a different world would overwrite whatever entities happen to
/// occupy the same ids.
///
/// # The capture instant is not public
///
/// The monotonic reading taken at capture is a private field; [`age`](Self::age) is the only
/// way to it, and it answers in a `std::time::Duration`. On `wasm32` the clock type behind it
/// is a `0.x` crate's `Instant` (the `std` one panics in a browser), and that type was on this
/// struct — on the DEFAULT surface, selected by target rather than by feature, so no caller
/// could opt out of it. `gizmo-scene` is Stage A (docs/ENGINE.md §4), so that had to go before
/// 1.0. Nothing is lost: an `Instant` from another process's clock is meaningless anyway, and
/// the field had exactly one reader in the whole workspace — `age` itself.
///
/// ```compile_fail
/// fn taken_at(s: &gizmo_scene::SceneSnapshot) {
///     let _ = &s.timestamp;
/// }
/// ```
// The `compile_fail` example above is this seal's regression test: it compiled while the
// field was `pub` (verified by running it unmarked against the old struct, where it passed as
// an ordinary doc-test). `compile_fail` alone only asserts *some* error and this toolchain's
// rustdoc ignores a `compile_fail,E0nnn` code, so the code is recorded by hand — un-mark it
// and the diagnostic is E0616: field `timestamp` of struct `SceneSnapshot` is private.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SceneSnapshot {
    /// The captured entities in ascending entity-id order (`World::iter_alive_entities`
    /// walks id slots `0..next_entity_id`), minus protected/editor entities and entities
    /// with neither a name nor a registered component. Position in this `Vec` is *not* an
    /// id — read [`EntitySnapshot::entity_id`] for that.
    pub entities: Vec<EntitySnapshot>,
    /// Monotonic clock reading taken when `capture` finished (`web_time::Instant` on wasm32,
    /// `std::time::Instant` everywhere else).
    ///
    /// **Private on purpose** — see the type-level note. Only [`age`](Self::age) reads it, and
    /// nothing about restore depends on it — a snapshot never goes stale. It is also why this
    /// type is not `Serialize`: an `Instant` is meaningless outside the process that took it,
    /// which keeps the snapshot honestly memory-only.
    timestamp: Instant,
}

impl SceneSnapshot {
    /// Captures the current world into an in-memory snapshot — the Play half of Play/Stop.
    ///
    /// Every component `registry` knows about is read off every live entity and serialized
    /// to a RON string; nothing touches the disk. Three groups are deliberately skipped:
    ///
    /// - ids in `protected_ids` — the editor camera, gizmos and their whole subtree; the
    ///   caller collects them (`gizmo-studio::collect_protected_ids` walks `Children`
    ///   breadth-first). Protected entities keep running through Play/Stop untouched.
    /// - entities whose name starts with `"Editor "` or equals `"Highlight Box"`, i.e. the
    ///   editor's name-tagged scaffolding even when it is not in the explicit set.
    /// - entities that would produce an empty row: no name and no registered component.
    ///
    /// Components that fail to serialize are skipped, counted, and reported via
    /// `tracing::warn!` — a component dropped here is state that Play→Stop will silently
    /// lose, so that warning is the only signal you get. The whole call is traced as
    /// `snapshot_capture` with entity/component counts and a duration in ms.
    ///
    /// Cost is O(live entities × components on them) plus one RON encode per component;
    /// editor-sized scenes land in the microsecond range.
    ///
    /// A bug worth not reintroducing: the lookup used to fabricate `Entity::new(id, 0)`.
    /// For any entity sitting on a recycled id slot (despawn→spawn, generation ≥ 1) the
    /// generation check then failed, `entity_component_types` returned empty, and every
    /// dynamic component — `Transform`, `RigidBody`, `Collider` — vanished from the snapshot
    /// without a word. Pinned by `capture_preserves_components_for_recycled_id_entity`.
    #[tracing::instrument(skip_all, name = "snapshot_capture")]
    pub fn capture(
        world: &World,
        registry: &crate::registry::SceneRegistry,
        protected_ids: &std::collections::HashSet<u32>,
    ) -> Self {
        let start = Instant::now();
        let mut entities = Vec::new();
        // Aggregates: a dropped component here means Play→Stop will silently lose that state.
        let mut component_count = 0usize;
        let mut dropped_components = 0usize;
        let names = world.borrow::<gizmo_core::EntityName>();
        let editor_only_markers = world.borrow::<gizmo_core::component::EditorOnly>();

        for ent in world.iter_alive_entities() {
            let id = ent.id();
            if protected_ids.contains(&id) {
                continue;
            }

            // Editor internal entity'lerini atla — karar `gizmo_core`'da, tek yerde.
            if gizmo_core::component::is_editor_only(
                editor_only_markers.get(id).is_some(),
                names.get(id).map(|n| n.0.as_str()),
            ) {
                continue;
            }

            let name = names.get(id).map(|n| n.0.clone());

            // Registry üzerinden tüm dinamik bileşenleri RON AST'sine dönüştür
            let mut components = std::collections::BTreeMap::new();

            // `ent`'in GERÇEK generation'ını kullan. `Entity::new(id, 0)` ile sabit
            // generation-0 lookup'ı, id slotu yeniden kullanılmış (despawn→spawn,
            // generation ≥ 1) her entity için `is_alive` kontrolünü geçemez →
            // `entity_component_types` boş döner ve TÜM dinamik bileşenler (Transform,
            // RigidBody, Collider…) sessizce kaybolurdu.
            let entity = ent;
            let types = world.entity_component_types(entity);
            for type_id in types {
                if let Some(reg) = registry.get_registration(type_id) {
                    if let Some(ptr) = world.get_component_ptr(entity, type_id) {
                        match crate::serde_bridge::serialize_component(registry, reg, type_id, ptr) {
                            Some(string_repr) => {
                                component_count += 1;
                                components.insert(reg.name.clone(), string_repr);
                            }
                            // serde_bridge already logged the concrete reason.
                            None => dropped_components += 1,
                        }
                    }
                }
            }

            // Yalnızca bir bileşeni olan entity'leri kaydet
            if name.is_some() || !components.is_empty() {
                entities.push(EntitySnapshot {
                    entity_id: id,
                    name,
                    components,
                });
            }
        }

        let snapshot = Self {
            entities,
            timestamp: Instant::now(),
        };
        if dropped_components > 0 {
            tracing::warn!(
                dropped_components,
                entity_count = snapshot.entities.len(),
                "[Scene] snapshot capture sırasında bazı bileşenler serialize edilemedi (Play→Stop'ta state kaybı riski)",
            );
        }
        // Play is a mode change → info! is the right level for the one-shot completion.
        tracing::info!(
            entity_count = snapshot.entities.len(),
            component_count,
            duration_ms = start.elapsed().as_secs_f64() * 1000.0,
            "[Scene] Snapshot alındı (Play)",
        );
        snapshot
    }

    /// Puts the snapshot back — the Stop half of Play/Stop — and reports what it did in a
    /// [`RestoreResult`].
    ///
    /// Two phases, in order:
    ///
    /// 1. Every entity that is *not* in the snapshot is despawned — in practice mostly the
    ///    entities Play created (bullets, debris, fracture fragments), but NOT exactly that
    ///    set: `capture` only records an entity that has a name or at least one
    ///    registry-known component, so a row-less entity that predated Play is absent from
    ///    the snapshot too and is deleted here. See [`EntitySnapshot`] for that caveat — a
    ///    bare `Children`-only group node with no name is the realistic case.
    ///    `protected_ids` and the same
    ///    `"Editor …"` / `"Highlight Box"` name filter capture uses are applied again here, so
    ///    pass the *current* protected set: an editor entity missing from it would be deleted,
    ///    since it is not in the snapshot either.
    /// 2. For each snapshot row the name and the registered components are written **on top
    ///    of** whatever entity still holds that id; a replacement is spawned only when the
    ///    id slot has gone free.
    ///
    /// Overwriting instead of clear-and-respawn is the whole point. Mesh/material handles
    /// and hierarchy are not in the snapshot (the registry does not carry them), so an entity
    /// that is despawned and rebuilt comes back invisible — which is precisely what Stop did
    /// until 2026-05-14.
    ///
    /// Consequences of that design, all intentional:
    ///
    /// - Restore only adds and overwrites, it never removes. A component gained during Play
    ///   (a `Collider` attached at runtime, say) is still attached after Stop.
    /// - An entity deleted during Play returns with its name and registered components only:
    ///   mesh, material and parent link are gone, and `World::spawn` may hand it a different
    ///   id (the free list is FIFO).
    /// - The `PhysicsWorld` resource — joints, contact caches, broadphase — is not touched.
    ///   Only components are restored; joints created during Play stay.
    ///
    /// Per-component failures (a deserialize error, or a name this registry does not know)
    /// do not abort the restore: they are counted and logged with `tracing::warn!`, because
    /// the entity then silently comes back carrying less state than it had at Play. The
    /// `let _ = …` that used to swallow those errors is exactly how that went unnoticed.
    ///
    /// To bring GPU-side resources back as well, reload the scene from disk with
    /// [`SceneData::load_into`](crate::scene::SceneData::load_into) instead.
    #[tracing::instrument(skip_all, name = "snapshot_restore")]
    pub fn restore(
        &self,
        world: &mut World,
        registry: &crate::registry::SceneRegistry,
        protected_ids: &std::collections::HashSet<u32>,
    ) -> RestoreResult {
        let start = Instant::now();
        let mut restored_count = 0u32;
        let mut despawned_count = 0u32;
        // A failed/unknown component here means Stop leaves the entity with less state than
        // it had at Play — worth surfacing, since the whole point of restore is fidelity.
        let mut failed_components = 0usize;
        let mut unknown_types = 0usize;

        let mut snap_ids = std::collections::HashSet::new();
        for snap_ent in &self.entities {
            snap_ids.insert(snap_ent.entity_id);
        }

        // 1. Korunmayan ve snapshot'ta OLMAYAN (Play sırasında oluşturulmuş) entity'leri sil
        let alive = world.iter_alive_entities();
        let mut to_despawn = Vec::new();

        {
            let names = world.borrow::<gizmo_core::EntityName>();
            let editor_only_markers = world.borrow::<gizmo_core::component::EditorOnly>();
            for ent in &alive {
                let id = ent.id();
                if protected_ids.contains(&id) {
                    continue;
                }
                if gizmo_core::component::is_editor_only(
                    editor_only_markers.get(id).is_some(),
                    names.get(id).map(|n| n.0.as_str()),
                ) {
                    continue;
                }
                
                if !snap_ids.contains(&id) {
                    to_despawn.push(*ent);
                }
            }
        }

        for ent in to_despawn {
            world.despawn(ent);
            despawned_count += 1;
        }

        // 2. Snapshot'taki entity'lerin bileşenlerini mevcutların üzerine yaz
        for snap_entity in &self.entities {
            // Eğer entity silinmişse yeni bir tane oluştur (Not: Mesh gibi GPU verileri kaybolur, ideal değil ama çökmez)
            let ent = if let Some(e) = world.get_entity(snap_entity.entity_id) {
                if world.is_alive(e) {
                    e
                } else {
                    world.spawn()
                }
            } else {
                world.spawn()
            };

            if let Some(ref name) = snap_entity.name {
                world.add_component(ent, gizmo_core::EntityName::new(name));
            }

            for (comp_name, comp_val) in &snap_entity.components {
                match registry.get_type_id(comp_name) {
                    Some(type_id) => {
                        if let Some(reg) = registry.get_registration(type_id) {
                            // Was `let _ = …`: a deserialize failure silently vanished, so a
                            // component the editor captured at Play never came back at Stop.
                            if let Err(e) = crate::serde_bridge::deserialize_component(
                                world, ent, registry, reg, type_id, comp_val,
                            ) {
                                failed_components += 1;
                                tracing::warn!(
                                    component = %comp_name,
                                    entity = snap_entity.entity_id,
                                    error = %e,
                                    "[Scene] snapshot restore: bileşen deserialize edilemedi (Stop'ta state eksik)",
                                );
                            }
                        } else {
                            unknown_types += 1;
                        }
                    }
                    None => unknown_types += 1,
                }
            }

            restored_count += 1;
        }

        let result = RestoreResult {
            despawned: despawned_count,
            restored: restored_count,
            duration: start.elapsed(),
        };
        if failed_components > 0 || unknown_types > 0 {
            tracing::warn!(
                failed_components,
                unknown_types,
                "[Scene] snapshot restore sırasında bazı bileşenler geri yüklenemedi (Stop sonrası state eksik olabilir)",
            );
        }
        // Stop is a mode change → info! for the one-shot completion.
        tracing::info!(
            despawned = result.despawned,
            restored = result.restored,
            duration_ms = result.duration.as_secs_f64() * 1000.0,
            "[Scene] Snapshot geri yüklendi (Stop)",
        );
        result
    }

    /// How many entities this snapshot will write back — deliberately *not* the world's
    /// entity count: protected ids, editor scaffolding and entities with nothing worth
    /// saving were filtered out at capture time.
    ///
    /// The editor reports it in its "▶ Play: N entity…" line, and it is also exactly what
    /// [`RestoreResult::restored`] will come back as, since restore counts rows processed
    /// rather than rows that fully succeeded.
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Time elapsed on the monotonic clock since [`capture`](Self::capture) finished.
    ///
    /// A convenience for editor UI ("snapshot taken 12 s ago"). Nothing in the engine reads
    /// it today, and it has no bearing on [`restore`](Self::restore) — a snapshot does not
    /// expire.
    pub fn age(&self) -> std::time::Duration {
        self.timestamp.elapsed()
    }
}

/// What one [`SceneSnapshot::restore`] actually did — the numbers the editor prints in its
/// status line after Stop.
///
/// Counters only. Per-component failures are not represented here (they go to
/// `tracing::warn!`), so a restore whose counts look perfect can still have lost state.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct RestoreResult {
    /// Entities despawned in phase 1 because they were alive in the world but absent from
    /// the snapshot. Mostly what Play spawned, but see [`SceneSnapshot::restore`]: an entity
    /// that `capture` skipped for having neither a name nor a registry-known component is
    /// also absent, and so is counted here. Protected ids and editor scaffolding are
    /// filtered out before the despawn list is built, so they can never appear in this
    /// count.
    pub despawned: u32,
    /// Snapshot rows applied in phase 2, counting entities overwritten in place and
    /// respawned ones alike.
    ///
    /// Incremented once per row unconditionally, so it always equals
    /// [`SceneSnapshot::entity_count`]: it means "the snapshot was applied", not "every
    /// component came back". Component-level losses only show up in the log.
    pub restored: u32,
    /// Wall-clock duration of the whole `restore` call, taken from the monotonic clock and
    /// covering the despawn pass plus every RON decode. Sub-millisecond for editor-sized
    /// scenes; the [`Display`](std::fmt::Display) impl renders it in milliseconds.
    pub duration: std::time::Duration,
}

/// Turkish one-line summary of a restore:
/// `"Restore: {despawned} entity silindi, {restored} entity geri yüklendi ({ms:.2}ms)"`.
///
/// Nothing in the engine formats a `RestoreResult` this way — `gizmo-studio` builds its own
/// Stop line out of the fields — so the only consumer is
/// `restore_result_display_formats_counts_and_millis`, which asserts on both counts and the
/// two-decimal millisecond formatting.
impl std::fmt::Display for RestoreResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Restore: {} entity silindi, {} entity geri yüklendi ({:.2}ms)",
            self.despawned,
            self.restored,
            self.duration.as_secs_f64() * 1000.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The capture instant is private now, so `age()` is the ONLY way to it. This pins the
    // capability the seal had to preserve: a real reading off the same monotonic clock, in a
    // `std::time::Duration` that names no dependency type. (A guard, not a proof — it would
    // have compiled before the field went private too; the proof is the `compile_fail`
    // doc-test on `SceneSnapshot`.)
    #[test]
    fn age_reads_the_capture_instant() {
        let world = World::new();
        let registry = crate::registry::SceneRegistry::new();
        let protected = std::collections::HashSet::new();

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        let first = snapshot.age();
        let second = snapshot.age();
        assert!(second >= first, "age must not run backwards: {second:?} < {first:?}");
        // Sanity bound: a snapshot taken microseconds ago is not hours old — catches a
        // timestamp that was never set from the clock at all (e.g. a `Default`).
        assert!(
            snapshot.age() < std::time::Duration::from_secs(60),
            "age of a just-taken snapshot is nonsense: {:?}",
            snapshot.age(),
        );
    }

    #[test]
    fn test_scene_snapshot_capture_empty_world() {
        let world = World::new();
        let registry = crate::registry::SceneRegistry::new();
        let protected = std::collections::HashSet::new();

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        assert_eq!(snapshot.entity_count(), 0);
    }

    #[test]
    fn test_scene_snapshot_round_trip() {
        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();
        let protected = std::collections::HashSet::new();

        // Entity oluştur
        let ent = world.spawn();
        world.add_component(ent, gizmo_core::EntityName::new("TestCube"));
        world.add_component(
            ent,
            gizmo_physics_core::Transform::new(gizmo_math::Vec3::new(1.0, 2.0, 3.0)),
        );

        // Snapshot al
        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        assert_eq!(snapshot.entity_count(), 1);
        assert_eq!(snapshot.entities[0].name.as_deref(), Some("TestCube"));
        assert!(snapshot.entities[0].components.contains_key("Transform"));

        // Entity'yi sil
        world.despawn(ent);
        assert_eq!(world.iter_alive_entities().len(), 0);

        // Restore et
        let result = snapshot.restore(&mut world, &registry, &protected);
        assert_eq!(result.restored, 1);

        // İsmi kontrol et
        let names = world.borrow::<gizmo_core::EntityName>();
        let alive = world.iter_alive_entities();
        assert_eq!(alive.len(), 1);
        let restored_name = names.get(alive[0].id());
        assert!(restored_name.is_some());
        assert_eq!(restored_name.unwrap().0, "TestCube");
    }

    // REGRESYON (audit 2026-06-29): id slotu yeniden kullanılmış entity'lerin
    // (despawn→spawn, generation ≥ 1) dinamik bileşenleri capture'da kaybolmamalı.
    // Eski kod `Entity::new(id, 0)` ile lookup yaptığından `entity_component_types`'ın
    // generation kontrolü başarısız olur, boş döner ve Transform sessizce düşerdi
    // (editör Play→Stop akışında fizik/transform kaybı).
    #[test]
    fn capture_preserves_components_for_recycled_id_entity() {
        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();
        let protected = std::collections::HashSet::new();

        // id 0'ı yak → sonraki spawn onu generation 1 ile geri kullanır.
        let burned = world.spawn();
        let burned_id = burned.id();
        world.despawn(burned);

        let ent = world.spawn();
        assert_eq!(ent.id(), burned_id, "ön koşul: id yeniden kullanılmalı");
        assert_ne!(ent.generation(), 0, "ön koşul: recycled entity generation ≥ 1");
        world.add_component(ent, gizmo_core::EntityName::new("Recycled"));
        world.add_component(
            ent,
            gizmo_physics_core::Transform::new(gizmo_math::Vec3::new(7.0, 8.0, 9.0)),
        );

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        assert_eq!(snapshot.entity_count(), 1);
        assert!(
            snapshot.entities[0].components.contains_key("Transform"),
            "recycled-id entity'nin Transform'ı capture'da DÜŞTÜ (generation-0 lookup bug)"
        );
    }

    // Capture must honor `protected_ids` (editor camera, gizmos, …): a protected entity is
    // never pulled into the Play snapshot, so Stop can't clobber/duplicate it.
    #[test]
    fn capture_skips_protected_ids() {
        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();

        let keep = world.spawn();
        world.add_component(keep, gizmo_core::EntityName::new("EditorCam"));
        let normal = world.spawn();
        world.add_component(normal, gizmo_core::EntityName::new("Cube"));

        let mut protected = std::collections::HashSet::new();
        protected.insert(keep.id());

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        assert_eq!(snapshot.entity_count(), 1, "only the unprotected entity is captured");
        assert_eq!(snapshot.entities[0].name.as_deref(), Some("Cube"));
    }

    // Capture must also filter the editor's name-tagged scaffolding ("Editor …",
    // "Highlight Box") even when they aren't in the explicit protected set.
    #[test]
    fn capture_skips_editor_named_scaffolding() {
        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();
        let protected = std::collections::HashSet::new();

        let ed = world.spawn();
        world.add_component(ed, gizmo_core::EntityName::new("Editor Grid"));
        let hl = world.spawn();
        world.add_component(hl, gizmo_core::EntityName::new("Highlight Box"));
        let real = world.spawn();
        world.add_component(real, gizmo_core::EntityName::new("Enemy"));

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        let names: Vec<_> = snapshot.entities.iter().filter_map(|e| e.name.clone()).collect();
        assert_eq!(names, vec!["Enemy".to_string()], "only the real entity is snapshotted");
    }

    // The full Play→Stop contract: `restore` must (a) DESPAWN entities spawned during play
    // that aren't in the snapshot, and (b) preserve the snapshot entities. The `despawned`
    // count is the load-bearing statistic the editor reports.
    #[test]
    fn restore_despawns_play_time_entities_not_in_snapshot() {
        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();
        let protected = std::collections::HashSet::new();

        let original = world.spawn();
        world.add_component(original, gizmo_core::EntityName::new("Original"));

        // Snapshot the pre-Play state (only `Original` exists).
        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        assert_eq!(snapshot.entity_count(), 1);

        // Simulate Play: a bullet is spawned at runtime.
        let bullet = world.spawn();
        world.add_component(bullet, gizmo_core::EntityName::new("Bullet"));
        assert_eq!(world.iter_alive_entities().len(), 2);

        // Stop: restore must remove the play-time bullet, keep Original.
        let result = snapshot.restore(&mut world, &registry, &protected);
        assert_eq!(result.despawned, 1, "the play-time bullet must be despawned");
        assert_eq!(result.restored, 1, "the one snapshot entity must be restored");

        let names = world.borrow::<gizmo_core::EntityName>();
        let alive_names: Vec<_> = world
            .iter_alive_entities()
            .into_iter()
            .filter_map(|e| names.get(e.id()).map(|n| n.0.clone()))
            .collect();
        assert!(alive_names.contains(&"Original".to_string()));
        assert!(!alive_names.contains(&"Bullet".to_string()), "bullet must be gone after Stop");
    }

    // Restore must reinstate component VALUES, not just names — the whole point of the
    // in-memory snapshot is that physics/transform state (position, …) is exactly recovered
    // on Stop. Round-trips the value through the serde bridge (capture → despawn → restore).
    #[test]
    fn restore_reinstates_component_values() {
        use gizmo_math::Vec3;
        use gizmo_physics_core::Transform;

        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();
        let protected = std::collections::HashSet::new();

        let ent = world.spawn();
        world.add_component(ent, gizmo_core::EntityName::new("Mover"));
        world.add_component(ent, Transform::new(Vec3::new(3.0, -7.0, 11.0)));

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);

        // Play destroyed the entity; Stop must bring it back WITH its transform value.
        world.despawn(ent);
        snapshot.restore(&mut world, &registry, &protected);

        let names = world.borrow::<gizmo_core::EntityName>();
        let id = world
            .iter_alive_entities()
            .into_iter()
            .map(|e| e.id())
            .find(|&id| names.get(id).map(|n| n.0.as_str()) == Some("Mover"))
            .expect("Mover restore edilmeli");
        let transforms = world.borrow::<Transform>();
        assert_eq!(
            transforms.get(id).map(|t| t.position),
            Some(Vec3::new(3.0, -7.0, 11.0)),
            "restore must recover the snapshotted transform value, not just the name"
        );
    }

    // `RestoreResult`'s Display formatting (counts + milliseconds to 2 dp) is a stable
    // contract.
    #[test]
    fn restore_result_display_formats_counts_and_millis() {
        let result = RestoreResult {
            despawned: 2,
            restored: 3,
            duration: std::time::Duration::from_millis(5),
        };
        let s = result.to_string();
        assert!(s.contains("2 entity silindi"), "despawn count missing: {s}");
        assert!(s.contains("3 entity geri yüklendi"), "restore count missing: {s}");
        // 5 ms → "5.00ms" (secs_f64 * 1000, {:.2}).
        assert!(s.contains("5.00ms"), "duration ms formatting wrong: {s}");
    }

    // A snapshot of an entity carrying ONLY a name (no registered components) is still
    // captured and restored — name-only entities (empty groups/markers) must not vanish.
    #[test]
    fn name_only_entity_is_captured_and_restored() {
        let mut world = World::new();
        let registry = crate::registry::default_scene_registry();
        let protected = std::collections::HashSet::new();

        let marker = world.spawn();
        world.add_component(marker, gizmo_core::EntityName::new("SpawnMarker"));

        let snapshot = SceneSnapshot::capture(&world, &registry, &protected);
        assert_eq!(snapshot.entity_count(), 1);
        assert!(
            snapshot.entities[0].components.is_empty(),
            "no registered components expected on a bare marker"
        );

        world.despawn(marker);
        snapshot.restore(&mut world, &registry, &protected);

        let names = world.borrow::<gizmo_core::EntityName>();
        assert!(
            world
                .iter_alive_entities()
                .into_iter()
                .any(|e| names.get(e.id()).map(|n| n.0.as_str()) == Some("SpawnMarker")),
            "name-only entity must be recreated on restore"
        );
    }
}
