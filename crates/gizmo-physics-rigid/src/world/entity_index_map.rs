//! The opaque `BodyHandle::id()` → SoA row index map.
//!
//! This module exists for one reason: to keep `rustc-hash` off this crate's public
//! surface. `PhysicsWorld::entity_index_map` used to be a `pub` field of type
//! `rustc_hash::FxHashMap<u32, usize>`. That is a type *alias* for
//! `HashMap<u32, usize, rustc_hash::FxBuildHasher>`, so the foreign hasher travelled
//! with every use of it without anyone ever having to write its name — and rustc-hash
//! 1.x → 2.0 changed precisely that alias. A `rustc-hash` 3.0 would therefore have been
//! a breaking change for `gizmo-physics-rigid`, i.e. a third party holding the power to
//! force our 2.0. See docs/ENGINE.md §4, criterion 2 ("the test is REACHABILITY").

use rustc_hash::FxHashMap;

/// [`BodyHandle::id()`](gizmo_physics_core::BodyHandle::id) → SoA row index, the reverse
/// of [`PhysicsWorld::entities`](crate::PhysicsWorld::entities).
///
/// An **opaque** wrapper over the crate's internal hash map. The map itself is a private
/// implementation detail — its hasher is not part of this crate's API and may change in a
/// patch release — but everything a caller ever did with it *by reading* is still here:
/// [`get`](Self::get), [`contains_key`](Self::contains_key), [`len`](Self::len) and
/// [`is_empty`](Self::is_empty) mirror the `HashMap` methods of the same names exactly.
///
/// # What you can no longer do, and why
///
/// * **Edit a world's map entry by entry.** Insertion, removal and clearing are
///   `pub(crate)`. The map has to stay in lockstep with the parallel arrays (a stale entry
///   silently points at whichever body now occupies that row), so this was never a safe
///   thing to reach into; use [`PhysicsWorld::add_body`](crate::PhysicsWorld::add_body),
///   [`remove_body_at`](crate::PhysicsWorld::remove_body_at),
///   [`sync_bodies`](crate::PhysicsWorld::sync_bodies) or
///   [`clear_bodies`](crate::PhysicsWorld::clear_bodies).
///
///   Read that as narrowly as it is written: it is **not** a claim that the lockstep
///   invariant is now enforced. `PhysicsWorld::entity_index_map` is still a `pub` field and
///   `Default`, `Clone`, `FromIterator` and `Deserialize` are all public here, so
///   `world.entity_index_map = …` replaces the map wholesale from outside the crate and
///   hands the caller back exactly the power `clear` + `insert` gave them. Sealing the
///   hasher is what this type is for; making the invariant unbreakable would mean
///   privatising the field, which docs/ENGINE.md §4 deliberately chose not to do so that
///   reading it stays free.
/// * **Iterate it.** Deliberately absent rather than merely unimplemented: iteration order
///   of a hash map is not part of the determinism contract (§5), and handing one out would
///   invite callers to build order-dependent behaviour on top of the simulation. Iterate
///   [`PhysicsWorld::entities`](crate::PhysicsWorld::entities) instead — it is a `Vec`, so
///   it has a defined order — and look each handle up here. Nothing inside this crate
///   iterates the map either, which is what makes the omission free.
/// * **Get the `HashMap` back out.** No `Deref`, `AsRef`, `Borrow`, `Into` or accessor:
///   any of those would re-export the hasher and seal nothing.
///
/// Building one from scratch — for a bare
/// [`JointSolver::solve_joints`](crate::JointSolver::solve_joints) or
/// [`ConstraintSolver::solve_contacts`](crate::solver::ConstraintSolver::solve_contacts)
/// call that is not driven by a [`PhysicsWorld`](crate::PhysicsWorld) — goes through
/// `FromIterator`, which keeps that embedding path open without naming the hasher:
///
/// ```
/// use gizmo_physics_rigid::world::EntityIndexMap;
///
/// // handle id → row index, exactly as the world would have built it
/// let map: EntityIndexMap = [(7u32, 0usize), (9, 1)].into_iter().collect();
///
/// assert_eq!(map.len(), 2);
/// assert!(map.contains_key(&7));
/// assert_eq!(map.get(&9), Some(&1));
/// assert_eq!(map.get(&1234), None);
/// ```
///
/// # Sealing guards
///
/// The doc-tests below are the regression tests for the seal itself: rustdoc compiles them
/// as an EXTERNAL consumer of this crate, so they see exactly the surface a downstream user
/// sees, and each one must fail to compile.
///
/// The first three are **proofs**: each was checked against the pre-seal shape (the field
/// typed `rustc_hash::FxHashMap<u32, usize>`) and compiled there, so it genuinely fails
/// without the fix. The last two, `AsRef` and `raw`, are **forward guards**, not evidence
/// that anything was sealed: `AsRef` failed before the seal too (`HashMap` has no
/// `AsRef<HashMap>`), and `raw` did not exist before the seal at all. They defend against a
/// later edit re-opening the hole.
///
/// They are written as bare `compile_fail` on purpose — this toolchain's rustdoc does
/// **not** check the `compile_fail,EXXXX` error code (verified on rustc 1.97.1:
/// `compile_fail,E0999` against an actual E0624 still passes), so the code would be
/// decoration that quietly rots. For the first four the real diagnostic was observed by hand
/// with the fence unmarked, and it is recorded in the prose above each one instead; the fifth
/// was added later and says so.
///
/// Entries cannot be inserted from outside the crate — note this is about the *entries*, not
/// the field, which is still `pub` and still assignable as a whole
/// (observed: ``error[E0624]: method `insert` is private``):
///
/// ```compile_fail
/// let mut world = gizmo_physics_rigid::PhysicsWorld::new();
/// world.entity_index_map.insert(1u32, 0usize);
/// ```
///
/// The `HashMap` API is not reachable, iteration included — this is also the guard
/// against a `Deref`/`DerefMut` being added later, since either would make `.iter()`
/// resolve again (observed: ``error[E0599]: no method named `iter` found for struct
/// `EntityIndexMap` in the current scope``):
///
/// ```compile_fail
/// let world = gizmo_physics_rigid::PhysicsWorld::new();
/// for (_id, _idx) in world.entity_index_map.iter() {}
/// ```
///
/// And the wrapper does not coerce to the foreign type it wraps — the load-bearing one,
/// because a `Deref` impl would make this compile and would have sealed nothing
/// (observed: `error[E0308]: mismatched types`, `expected` a `&HashMap<u32, usize, _>`,
/// `found reference &EntityIndexMap`):
///
/// ```compile_fail
/// let world = gizmo_physics_rigid::PhysicsWorld::new();
/// let _: &std::collections::HashMap<u32, usize, _> = &world.entity_index_map;
/// ```
///
/// Nor through `AsRef`, which is in the prelude and so would resolve if it existed
/// (observed: ``error[E0599]: no method named `as_ref` found for struct `EntityIndexMap`
/// in the current scope``):
///
/// ```compile_fail
/// let world = gizmo_physics_rigid::PhysicsWorld::new();
/// let _: &std::collections::HashMap<u32, usize, _> = world.entity_index_map.as_ref();
/// ```
///
/// Nor through `raw()`, the `pub(crate)` hatch the crate's own solver paths use.
/// This fence exists because widening that one method to `pub` is by far the likeliest way
/// the seal gets undone later, and **none of the four above would catch it**: `insert`
/// stays private, `iter` stays absent, and the wrapper still would not coerce. Its
/// diagnostic is a privacy error on `raw`, read off the signature rather than observed by
/// hand like the four above:
///
/// ```compile_fail
/// let world = gizmo_physics_rigid::PhysicsWorld::new();
/// let _ = world.entity_index_map.raw();
/// ```
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
// `transparent`: the wrapper must serialize exactly as the bare map did, because
// `PhysicsWorld` is `Serialize`/`Deserialize` and `trigger_snapshot` writes this field
// into the diagnostic JSON. Wrapping the field would silently change that file's shape.
#[serde(transparent)]
pub struct EntityIndexMap {
    map: FxHashMap<u32, usize>,
}

impl EntityIndexMap {
    /// The row index `entity_id` currently occupies, or `None` when the id names no body
    /// in this world.
    ///
    /// Mirrors [`HashMap::get`](std::collections::HashMap::get), reference and all, so the
    /// `.copied()` / `if let Some(&idx)` shapes callers already wrote keep working.
    #[inline]
    pub fn get(&self, entity_id: &u32) -> Option<&usize> {
        self.map.get(entity_id)
    }

    /// Whether `entity_id` names a body in this world.
    #[inline]
    pub fn contains_key(&self, entity_id: &u32) -> bool {
        self.map.contains_key(entity_id)
    }

    /// Number of mapped bodies. Equal to `PhysicsWorld::entities.len()` whenever the world
    /// is consistent.
    #[inline]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the map is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Map `entity_id` to `index`, returning the index it displaced.
    #[inline]
    pub(crate) fn insert(&mut self, entity_id: u32, index: usize) -> Option<usize> {
        self.map.insert(entity_id, index)
    }

    /// Drop `entity_id`'s mapping, returning the index it held.
    #[inline]
    pub(crate) fn remove(&mut self, entity_id: &u32) -> Option<usize> {
        self.map.remove(entity_id)
    }

    /// Drop every mapping, keeping the allocation.
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.map.clear();
    }

    /// The wrapped map, for the crate-internal solver paths that still take a bare
    /// `FxHashMap` (`support_order_manifolds`, `solve_contacts_tgs` — both private, so
    /// neither is on the public surface).
    ///
    /// `pub(crate)` is what makes this not a leak: an accessor of any wider visibility
    /// would hand the hasher straight back out and undo the seal.
    #[inline]
    pub(crate) fn raw(&self) -> &FxHashMap<u32, usize> {
        &self.map
    }
}

impl FromIterator<(u32, usize)> for EntityIndexMap {
    fn from_iter<I: IntoIterator<Item = (u32, usize)>>(iter: I) -> Self {
        Self {
            map: iter.into_iter().collect(),
        }
    }
}

// Hand-written and opaque, following the `crossbeam-queue` seal in `gizmo-core`
// (`asset::AssetDropQueue`): a derived `Debug` would forward to the map's own `Debug`,
// which prints every entry in HASH ORDER — non-deterministic output from a type that sits
// inside a determinism-critical struct, and a formatting detail we would then owe
// stability to. The length is the only thing worth reporting and it is order-free.
impl std::fmt::Debug for EntityIndexMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntityIndexMap").field("len", &self.map.len()).finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_api_mirrors_the_map_it_replaced() {
        let mut m = EntityIndexMap::default();
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);

        assert_eq!(m.insert(3, 0), None);
        assert_eq!(m.insert(8, 1), None);
        assert_eq!(m.insert(3, 2), Some(0), "insert returns the displaced index");

        assert_eq!(m.len(), 2);
        assert!(!m.is_empty());
        assert_eq!(m.get(&3), Some(&2));
        assert_eq!(m.get(&8), Some(&1));
        assert_eq!(m.get(&99), None);
        assert!(m.contains_key(&8));
        assert!(!m.contains_key(&99));

        assert_eq!(m.remove(&8), Some(1));
        assert_eq!(m.remove(&8), None);
        assert_eq!(m.len(), 1);

        m.clear();
        assert!(m.is_empty());
    }

    #[test]
    fn from_iter_round_trips() {
        let m: EntityIndexMap = (0u32..4).map(|i| (i, i as usize * 10)).collect();
        assert_eq!(m.len(), 4);
        for i in 0u32..4 {
            assert_eq!(m.get(&i), Some(&(i as usize * 10)));
        }
    }

    /// `PhysicsWorld` is `Serialize`/`Deserialize` and this field is NOT `serde(skip)`, so
    /// the wrapper has to be wire-identical to the bare map or `trigger_snapshot`'s JSON
    /// changes shape under everyone reading it.
    #[test]
    fn serializes_transparently_as_a_bare_map() {
        let m: EntityIndexMap = [(5u32, 1usize)].into_iter().collect();
        let bare: FxHashMap<u32, usize> = [(5u32, 1usize)].into_iter().collect();
        assert_eq!(serde_json::to_string(&m).unwrap(), serde_json::to_string(&bare).unwrap());

        let back: EntityIndexMap = serde_json::from_str(r#"{"5":1}"#).unwrap();
        assert_eq!(back.get(&5), Some(&1));
    }

    /// The `Debug` is opaque on purpose — see the impl. If someone swaps it for a derive,
    /// the map's contents (and its hash order) start showing up in logs and in rendered
    /// docs, which is the leak the `crossbeam-queue` seal in `gizmo-core` avoided.
    #[test]
    fn debug_is_opaque_and_order_free() {
        let m: EntityIndexMap = (0u32..6).map(|i| (i, i as usize)).collect();
        let s = format!("{m:?}");
        assert_eq!(s, "EntityIndexMap { len: 6, .. }");
    }
}
