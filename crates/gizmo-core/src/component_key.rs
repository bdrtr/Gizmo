//! Which component a column, a sparse set, a hook or a system's access is about.
//!
//! A component is either a Rust type, keyed by its [`TypeId`], or one defined while the program runs
//! (`World::register_component`), which has no type and is keyed by the id the world gave it. Both are
//! a [`ComponentKey`], so storage, queries and the scheduler treat them alike: a dynamic component gets
//! an archetype column, takes part in change detection, and conflicts in the scheduler with any system
//! that writes it.
//!
//! Keys order as `TypeId` does for types, and every type's key comes before every dynamic one's, so an
//! archetype's columns, which are kept sorted by key, are where they were before dynamic components
//! existed.

use std::any::TypeId;

/// A component's identity: its Rust type, or the id a world gave it when it was defined at runtime.
///
/// [`ComponentKey::of`] needs no world, which is what lets queries and system parameters declare their
/// access without one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ComponentKey(Repr);

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
enum Repr {
    Static(TypeId),
    Dynamic(DynamicComponentId),
}

/// The id of a component defined at runtime, unique within the world that minted it. Two worlds share
/// ids only if they registered the same components in the same order: anything that moves data between
/// worlds should go by name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct DynamicComponentId(pub(crate) u32);

impl DynamicComponentId {
    /// Its index among the world's dynamic components, in registration order.
    #[must_use]
    pub fn index(self) -> u32 {
        self.0
    }
}

impl ComponentKey {
    /// The key of the component type `T`.
    #[must_use]
    pub fn of<T: 'static>() -> Self {
        Self(Repr::Static(TypeId::of::<T>()))
    }

    /// The Rust type behind it, or `None` for a component defined at runtime.
    #[must_use]
    pub fn type_id(self) -> Option<TypeId> {
        match self.0 {
            Repr::Static(t) => Some(t),
            Repr::Dynamic(_) => None,
        }
    }

    /// The runtime id behind it, or `None` for a component type.
    #[must_use]
    pub fn dynamic(self) -> Option<DynamicComponentId> {
        match self.0 {
            Repr::Static(_) => None,
            Repr::Dynamic(id) => Some(id),
        }
    }

    pub(crate) fn from_dynamic(id: DynamicComponentId) -> Self {
        Self(Repr::Dynamic(id))
    }
}

impl From<TypeId> for ComponentKey {
    fn from(type_id: TypeId) -> Self {
        Self(Repr::Static(type_id))
    }
}

impl PartialEq<TypeId> for ComponentKey {
    fn eq(&self, other: &TypeId) -> bool {
        self.type_id() == Some(*other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Types order among themselves as their `TypeId`s do, and before every dynamic component, so an
    /// archetype's sorted columns are where they were before keys replaced `TypeId`.
    #[test]
    fn types_keep_their_order_and_come_before_dynamic_components() {
        let (a, b) = (TypeId::of::<u8>(), TypeId::of::<u16>());
        assert_eq!(ComponentKey::from(a).cmp(&ComponentKey::from(b)), a.cmp(&b));
        let dynamic = ComponentKey::from_dynamic(DynamicComponentId(0));
        assert!(ComponentKey::from(a) < dynamic && ComponentKey::from(b) < dynamic);
        assert_eq!(ComponentKey::of::<u8>().type_id(), Some(a));
        assert_eq!(dynamic.type_id(), None);
        assert_eq!(dynamic.dynamic(), Some(DynamicComponentId(0)));
        assert!(ComponentKey::of::<u8>() == a);
    }
}
