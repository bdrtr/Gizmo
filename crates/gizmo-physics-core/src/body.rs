//! Opaque body handle for the physics world.

/// An opaque identifier for a body inside a physics world.
///
/// `BodyHandle` is the single identity the physics public API speaks in —
/// collision/trigger/fracture events, raycast hits, joints and broadphase
/// queries all carry `BodyHandle`s rather than the engine's ECS
/// [`Entity`](gizmo_core::entity::Entity). The ECS↔physics bridge converts at
/// the boundary (`BodyHandle::from_id(entity.id())` on the way in,
/// `Entity::new(handle.id(), 0)` on the way out), so a non-ECS embedding can
/// drive the simulation with its own handles and the internal representation
/// can evolve post-1.0 without breaking the physics API.
///
/// It is a transparent newtype over `u32`, so where it appears in serialized
/// data (e.g. scene-file joint endpoints) it round-trips as that single id.
///
/// # Determinism
/// A handle wraps the body's stable `u32` id — the sole identity physics keys
/// on (the deterministic state hash sorts and mixes bodies by [`id`](Self::id)).
/// Carrying the same id the old `Entity` did keeps the state hash bit-identical,
/// preserving cross-process/rollback determinism.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct BodyHandle(u32);

impl BodyHandle {
    /// Sentinel value for "no body"; usable in place of `Option<BodyHandle>`.
    pub const INVALID: Self = Self(u32::MAX);

    /// Builds a handle from a raw body id.
    #[inline]
    pub const fn from_id(id: u32) -> Self {
        Self(id)
    }

    /// The body's stable id (the value the deterministic state hash keys on).
    #[inline]
    pub const fn id(self) -> u32 {
        self.0
    }

    /// `true` unless this is [`INVALID`](Self::INVALID).
    #[inline]
    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }
}

impl std::fmt::Display for BodyHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if *self == Self::INVALID {
            write!(f, "BodyHandle(INVALID)")
        } else {
            write!(f, "BodyHandle({})", self.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_id_roundtrips() {
        let h = BodyHandle::from_id(42);
        assert_eq!(h.id(), 42);
        assert!(h.is_valid());
    }

    #[test]
    fn invalid_sentinel() {
        assert!(!BodyHandle::INVALID.is_valid());
        assert_eq!(BodyHandle::INVALID.id(), u32::MAX);
    }

    #[test]
    fn ordering_and_hash_by_id() {
        use std::collections::HashSet;
        assert!(BodyHandle::from_id(5) < BodyHandle::from_id(10));
        let mut set = HashSet::new();
        set.insert(BodyHandle::from_id(7));
        assert!(set.contains(&BodyHandle::from_id(7)));
        assert!(!set.contains(&BodyHandle::from_id(8)));
    }
}
