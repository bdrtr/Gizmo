//! Entity identity: the id/generation handle and the allocator that recycles it.
//!
//! An [`Entity`] is an id plus a generation counter. Reusing an id after a despawn bumps the
//! generation, so a stale handle compares unequal to the live entity occupying the same slot
//! instead of silently addressing it — the whole reason the generation exists.
//!
//! Anything that stores a bare `u32` id rather than an `Entity` gives that protection up.
/// The ECS Entity identifier — a packed u64 representation.
///
/// The low 32 bits = the entity ID (slot index), the high 32 bits = the generation (reuse counter).
/// The generation is incremented when a new entity is assigned to the same slot (ID), and it makes
/// it possible to detect old references (dangling entities) safely.
///
/// # Layout
/// ```text
/// ┌──────────────────────────────────────────────────────────────────┐
/// │  63 ───────────── 32 │ 31 ───────────── 0 │
/// │     generation (u32) │       id (u32)     │
/// └──────────────────────────────────────────────────────────────────┘
/// ```
///
/// # In saved data
///
/// Serialized as its packed `u64`. Inside a [`references`] scope — which the scene format opens
/// around the components it writes and reads — a handle is a **reference** instead: written only
/// if it names an entity being saved, and read back as the entity that saved one became. Outside
/// a scope the number goes through unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entity(u64);

/// The wire form of an [`Entity`]: what `#[derive(Serialize, Deserialize)]` on `Entity(u64)` wrote,
/// kept so every file written before the scopes reads the same.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename = "Entity")]
struct Packed(u64);

impl serde::Serialize for Entity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Packed(references::on_save(*self).0).serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Entity {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Packed(bits) = Packed::deserialize(deserializer)?;
        Ok(references::on_load(Entity(bits)))
    }
}

/// Entity handles inside saved components, kept pointing at the right entity across a save and a
/// load.
///
/// A handle is a run's: a scene load, the editor's ▶ and a prefab spawn their entities again under
/// new ids. The scene format remaps `Parent`/`Children` and joint endpoints itself, but a
/// component's own `Entity` field — a turret's target, a door's key, a bee's flower — was written
/// as a number and read back as the same number, naming whatever held that id in the new world,
/// or nothing (until 2026-11-22). Inside these scopes every `Entity` a component serializes is
/// treated as what it is, a reference:
///
/// - [`saving`](references::saving): a handle to an entity in the saved set is written; one to any other entity —
///   not saved, or dead (a stale generation) — is written as [`Entity::INVALID`], since it can
///   name nothing in the file.
/// - [`loading`](references::loading): a handle is read back as the entity its saved id became; one the file does not
///   hold, as [`Entity::INVALID`].
///
/// `Entity`, `Option<Entity>`, `Vec<Entity>`, a map of them, a struct holding one — any shape
/// serde writes — is covered, with no per-type code. The scopes are per thread and nest (the
/// innermost wins); outside them a handle serializes as its number, as before.
pub mod references {
    use super::Entity;
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};

    enum Scope {
        Saving(HashSet<Entity>),
        Loading(HashMap<u32, Entity>),
    }

    thread_local! {
        static SCOPES: RefCell<Vec<Scope>> = const { RefCell::new(Vec::new()) };
    }

    /// An open scope: the scope ends when this is dropped — at the end of the block that holds it,
    /// or on a panic unwinding through it. Bound to its thread, as the scope is.
    #[must_use = "the scope ends as soon as this is dropped"]
    pub struct Scoped {
        _thread_bound: std::marker::PhantomData<*const ()>,
    }

    impl Drop for Scoped {
        fn drop(&mut self) {
            SCOPES.with(|s| s.borrow_mut().pop());
        }
    }

    fn open(scope: Scope) -> Scoped {
        SCOPES.with(|s| s.borrow_mut().push(scope));
        Scoped { _thread_bound: std::marker::PhantomData }
    }

    /// Opens a scope with `saved` as the entities being written, until the guard is dropped: see
    /// the [module](self).
    pub fn saving(saved: impl IntoIterator<Item = Entity>) -> Scoped {
        open(Scope::Saving(saved.into_iter().collect()))
    }

    /// Opens a scope with `map` — each saved entity's id to the entity it has become — as the
    /// entities being read, until the guard is dropped: see the [module](self).
    pub fn loading(map: &HashMap<u32, Entity>) -> Scoped {
        open(Scope::Loading(map.clone()))
    }

    pub(super) fn on_save(e: Entity) -> Entity {
        SCOPES.with(|s| match s.borrow().last() {
            Some(Scope::Saving(saved)) if !saved.contains(&e) => Entity::INVALID,
            _ => e,
        })
    }

    pub(super) fn on_load(e: Entity) -> Entity {
        SCOPES.with(|s| match s.borrow().last() {
            Some(Scope::Loading(map)) if e != Entity::INVALID => map.get(&e.id()).copied().unwrap_or(Entity::INVALID),
            _ => e,
        })
    }
}

impl Entity {
    /// The invalid / null entity sentinel value.
    /// It can be used instead of `Option<Entity>` (ergonomic and cache friendly).
    pub const INVALID: Self = Self(u64::MAX);

    /// Builds an `Entity` handle from an explicit `id` + `generation`.
    ///
    /// FOOTGUN: if you only have a raw `id` (e.g. from the UI, a script, or a saved
    /// reference) do NOT fabricate `Entity::new(id, 0)` to call a generation-checked API
    /// (`World::is_alive`/`entity_component_types`/`get_entity`, queries): once the id slot
    /// has been recycled (despawn→spawn bumps the generation) that gen-0 handle is stale
    /// and silently misses / mis-targets. Use [`World::entity(id)`](crate::World::entity),
    /// which reconstructs the CURRENT generation. `new` is for deserialization and tests
    /// where the generation is known, or for purely id-keyed internal addressing.
    #[inline]
    pub fn new(id: u32, generation: u32) -> Self {
        Self(((generation as u64) << 32) | id as u64)
    }

    /// Returns the Entity's slot index (ID).
    #[inline]
    pub fn id(self) -> u32 {
        self.0 as u32
    }

    /// Returns the Entity's generation counter.
    #[inline]
    pub fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    /// Checks whether this entity is valid (i.e. not INVALID).
    #[inline]
    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }

    /// Converts the Entity into its raw u64 bit representation.
    /// It can be used for serialization, network sync and as a hash key.
    #[inline]
    pub fn to_bits(self) -> u64 {
        self.0
    }

    /// Creates an Entity from a raw u64 bit representation.
    #[inline]
    pub fn from_bits(bits: u64) -> Self {
        Self(bits)
    }
}

impl std::fmt::Display for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if *self == Self::INVALID {
            write!(f, "Entity(INVALID)")
        } else {
            write!(f, "Entity({}:{})", self.id(), self.generation())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_and_accessors() {
        let e = Entity::new(42, 7);
        assert_eq!(e.id(), 42);
        assert_eq!(e.generation(), 7);
    }

    #[test]
    fn test_zero_generation() {
        let e = Entity::new(0, 0);
        assert_eq!(e.id(), 0);
        assert_eq!(e.generation(), 0);
        assert!(e.is_valid());
    }

    #[test]
    fn test_max_values() {
        let e = Entity::new(u32::MAX - 1, u32::MAX - 1);
        assert_eq!(e.id(), u32::MAX - 1);
        assert_eq!(e.generation(), u32::MAX - 1);
        assert!(e.is_valid());
    }

    #[test]
    fn test_invalid_sentinel() {
        assert!(!Entity::INVALID.is_valid());
        assert_eq!(Entity::INVALID.id(), u32::MAX);
        assert_eq!(Entity::INVALID.generation(), u32::MAX);
    }

    #[test]
    fn test_to_bits_from_bits_roundtrip() {
        let e = Entity::new(123, 456);
        let bits = e.to_bits();
        let e2 = Entity::from_bits(bits);
        assert_eq!(e, e2);
        assert_eq!(e2.id(), 123);
        assert_eq!(e2.generation(), 456);
    }

    #[test]
    fn test_display() {
        let e = Entity::new(5, 2);
        assert_eq!(format!("{}", e), "Entity(5:2)");
    }

    #[test]
    fn test_display_invalid() {
        assert_eq!(format!("{}", Entity::INVALID), "Entity(INVALID)");
    }

    #[test]
    fn test_equality_and_hash() {
        use std::collections::HashSet;
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(1, 0);
        let e3 = Entity::new(1, 1); // Aynı ID, farklı generation

        assert_eq!(e1, e2);
        assert_ne!(e1, e3);

        let mut set = HashSet::new();
        set.insert(e1);
        assert!(set.contains(&e2));
        assert!(!set.contains(&e3));
    }

    #[test]
    fn test_copy_semantics() {
        let e1 = Entity::new(10, 3);
        let e2 = e1; // Copy
        assert_eq!(e1, e2);
        assert_eq!(e1.id(), e2.id());
    }

    #[test]
    fn test_serde_roundtrip() {
        let e = Entity::new(999, 42);
        let serialized = ron::to_string(&e).expect("serialize failed");
        let deserialized: Entity = ron::from_str(&serialized).expect("deserialize failed");
        assert_eq!(e, deserialized);
    }

    /// The wire form is the derived one's, so files written before the scopes read the same.
    #[test]
    fn the_wire_form_is_the_packed_number() {
        let e = Entity::new(7, 3);
        let bits = (3u64 << 32) | 7;
        assert_eq!(ron::to_string(&e).unwrap(), format!("({bits})"));
        assert_eq!(serde_json::to_string(&e).unwrap(), bits.to_string());
        assert_eq!(serde_json::from_str::<Entity>(&bits.to_string()).unwrap(), e);
    }

    /// Saving: a handle to a saved entity is written, one to anything else — not saved, or a stale
    /// generation of a saved slot — as `INVALID`. Loading: a saved id is read as what it became,
    /// an id the file does not hold as `INVALID`. Every shape serde writes is covered.
    #[test]
    fn references_are_written_and_read_as_references() {
        use std::collections::HashMap;
        let (kept, stale, outside) = (Entity::new(4, 2), Entity::new(4, 1), Entity::new(9, 0));
        let value = (kept, Some(stale), vec![outside, kept, Entity::INVALID]);
        let written = {
            let _scope = references::saving([kept]);
            serde_json::to_string(&value).unwrap()
        };
        let invalid = Entity::INVALID;
        assert_eq!(written, serde_json::to_string(&(kept, Some(invalid), vec![invalid, kept, invalid])).unwrap());
        let became = Entity::new(0, 0);
        let map = HashMap::from([(4u32, became)]);
        let read: (Entity, Option<Entity>, Vec<Entity>) = {
            let _scope = references::loading(&map);
            serde_json::from_str(&written).unwrap()
        };
        assert_eq!(read, (became, Some(invalid), vec![invalid, became, invalid]));
        // Outside a scope, numbers go through.
        let plain: (Entity, Option<Entity>, Vec<Entity>) = serde_json::from_str(&serde_json::to_string(&value).unwrap()).unwrap();
        assert_eq!(plain, value);
        // A panic inside a scope does not leave it open.
        let _ = std::panic::catch_unwind(|| {
            let _scope = references::loading(&map);
            panic!("inside")
        });
        assert_eq!(serde_json::from_str::<Entity>(&serde_json::to_string(&outside).unwrap()).unwrap(), outside);
    }
}
/// Entity id allocation: hands out [`Entity`] handles, recycles ids through a free list and
/// keeps the per-slot generation counters that make a stale handle detectable.
pub mod allocator;
