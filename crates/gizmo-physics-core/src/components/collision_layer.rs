use serde::{Deserialize, Serialize};

/// The layer a default-constructed [`CollisionLayer`] sits on, and so the one a collider that
/// was never configured ends up with. The default mask also contains its bit, which is why
/// two untouched colliders collide.
pub const LAYER_DEFAULT: u32 = 0;

/// Suggested index for player bodies. A naming convention and nothing more: the engine
/// attaches no behaviour to any particular index, so this is worth exactly what your game's
/// own masks make of it.
pub const LAYER_PLAYER: u32 = 1;

/// Suggested index for enemy bodies — the counterpart to [`LAYER_PLAYER`] in the two-sided
/// setup collision filtering requires, where each side's mask names the other's bit.
pub const LAYER_ENEMY: u32 = 2;

/// Suggested index for trigger volumes. It does **not** make a collider a trigger:
/// pass-through-and-report behaviour comes from
/// [`Collider::is_trigger`](crate::components::Collider), and a collider parked on this layer
/// with that flag unset is an ordinary solid.
pub const LAYER_TRIGGER: u32 = 3;

/// Which of the 32 collision layers a collider sits on, and which layers it is willing to
/// collide with.
///
/// Filtering is *mutual*: a pair survives only when each side's [`mask`](Self::mask) contains
/// the other side's [`layer`](Self::layer) bit, so either collider can veto the pair on its
/// own — see [`can_collide_with`](Self::can_collide_with). Scene queries are filtered the
/// other way round, one-directionally, and consult only a collider's `layer`.
///
/// This is applied late in the rigid-body pipeline — at narrowphase and at the CCD sweep
/// backstop, not in the broadphase — so a masked-out pair is still generated as a broadphase
/// candidate and only then discarded. Layers are a correctness filter, not a way to keep
/// pairs out of the broadphase.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CollisionLayer {
    /// Index of the layer this collider belongs to, valid in `0..=31` — it is used as a shift
    /// into a 32-bit mask, which is the hard cap on the number of layers.
    ///
    /// Out-of-range values are not wrapped here: a `debug_assert!` fires in debug builds, and
    /// in release the shift yields no bit at all, so such a collider silently collides with
    /// nothing. Other parts of the engine handle an oversized index differently (scene query
    /// filtering masks it down with `& 31`), which is a further reason to keep it in range.
    pub layer: u32, // Which layer this object is on (0-31)
    /// Bitfield of the layers this collider accepts: bit *i* set means "I am willing to
    /// collide with layer *i*". Defaults to `u32::MAX`, i.e. everything including its own
    /// layer.
    ///
    /// Zero is a total veto — the collider collides with nothing, even with colliders whose
    /// masks accept it. What is tested against this field is the *other* collider's `layer`,
    /// never its mask, and scene queries ignore this field entirely: they match the query's
    /// own mask against this collider's `layer`.
    pub mask: u32,  // Which layers this object collides with (bitfield)
}

impl Default for CollisionLayer {
    fn default() -> Self {
        Self {
            layer: LAYER_DEFAULT,
            mask: u32::MAX, // Collide with everything by default
        }
    }
}

impl CollisionLayer {
    /// Put a collider on `layer` with a mask that accepts every layer, its own included.
    ///
    /// `layer` is not validated; see [`layer`](Self::layer) for the `0..=31` requirement.
    /// The permissive mask is the intended starting point — narrow it with
    /// [`with_mask`](Self::with_mask) — so a half-configured collider errs toward colliding
    /// rather than toward passing through the world unnoticed.
    pub fn new(layer: u32) -> Self {
        Self {
            layer,
            mask: u32::MAX,
        }
    }

    /// Alias for [`new`](Self::new): same result, all-ones mask included. Nothing is
    /// converted or validated on the way through.
    pub fn from_layer(layer: u32) -> Self {
        Self::new(layer)
    }

    /// Set the acceptance mask and return `self`, for chaining onto [`new`](Self::new).
    ///
    /// This replaces the mask outright rather than OR-ing into it, and it does not re-add the
    /// collider's own layer bit: two colliders on the same layer whose mask omits that layer
    /// will pass straight through each other.
    pub fn with_mask(mut self, mask: u32) -> Self {
        self.mask = mask;
        self
    }

    /// OR together `1 << l` for each listed layer, producing a mask that accepts exactly
    /// those layers.
    ///
    /// Returns the raw bitfield, not a `CollisionLayer` — pass it to
    /// [`with_mask`](Self::with_mask). Repeated entries are harmless, an empty slice gives
    /// `0` (a mask that accepts nothing), and entries `>= 32` are silently dropped instead of
    /// wrapping around to a low bit or panicking.
    pub fn mask_from_layers(layers: &[u32]) -> u32 {
        layers
            .iter()
            .fold(0u32, |acc, &l| acc | (1u32.checked_shl(l).unwrap_or(0)))
    }

    /// Whether this pair is allowed to collide: true only when each side's mask holds the
    /// other side's layer bit. Consent is mutual, so one refusal drops the pair, and the
    /// predicate is exactly symmetric — `a.can_collide_with(&b)` and `b.can_collide_with(&a)`
    /// always agree, which means the caller need not care which collider it started from.
    ///
    /// A pure bit test on the two components: no world access, no allocation, and no
    /// dependence on iteration order, so it does not perturb replay.
    ///
    /// If either layer is `>= 32` the shift produces no bit and the answer is `false` in
    /// release builds; in debug a `debug_assert!` fires first.
    #[inline]
    pub fn can_collide_with(&self, other: &CollisionLayer) -> bool {
        debug_assert!(
            self.layer < 32,
            "CollisionLayer: layer {} >= 32",
            self.layer
        );
        debug_assert!(
            other.layer < 32,
            "CollisionLayer: layer {} >= 32",
            other.layer
        );

        let layer_bit = 1u32.checked_shl(self.layer).unwrap_or(0);
        let other_layer_bit = 1u32.checked_shl(other.layer).unwrap_or(0);
        (self.mask & other_layer_bit) != 0 && (other.mask & layer_bit) != 0
    }
}

#[cfg(feature = "ecs")]
gizmo_core::impl_component!(CollisionLayer);

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_collides_with_everything() {
        // mask = u32::MAX ⇒ collides with any layer, in either direction.
        let a = CollisionLayer::default();
        let b = CollisionLayer::new(5);
        assert!(a.can_collide_with(&b));
        assert!(b.can_collide_with(&a));
    }

    #[test]
    fn can_collide_with_is_symmetric() {
        // A (layer 1) collides with all; B (layer 2) masks OUT layer 1.
        // B's veto must block the pair from BOTH sides — the predicate ANDs both masks.
        let a = CollisionLayer::new(1);
        let b = CollisionLayer::new(2).with_mask(!(1u32 << 1));
        assert!(!a.can_collide_with(&b));
        assert!(
            !b.can_collide_with(&a),
            "collision predicate must be symmetric"
        );
    }

    #[test]
    fn mutual_masks_required() {
        // Both objects must list the other's layer in their mask.
        let player = CollisionLayer::new(LAYER_PLAYER).with_mask(1u32 << LAYER_ENEMY);
        let enemy = CollisionLayer::new(LAYER_ENEMY).with_mask(1u32 << LAYER_PLAYER);
        assert!(player.can_collide_with(&enemy));

        // A "pacifist" player collides with nothing (mask = 0): one-sided veto.
        let pacifist = CollisionLayer::new(LAYER_PLAYER).with_mask(0);
        assert!(!pacifist.can_collide_with(&enemy));
        assert!(!enemy.can_collide_with(&pacifist));
    }

    #[test]
    fn same_layer_self_collision_follows_mask() {
        // Two objects on the same layer collide iff that layer's own bit is in the mask.
        let both = CollisionLayer::new(3); // mask = all ⇒ bit 3 present
        assert!(both.can_collide_with(&both));
        let self_excluded = CollisionLayer::new(3).with_mask(!(1u32 << 3));
        assert!(!self_excluded.can_collide_with(&self_excluded));
    }

    #[test]
    fn mask_from_layers_sets_expected_bits() {
        assert_eq!(CollisionLayer::mask_from_layers(&[0, 1, 3]), 0b1011);
        // Empty set ⇒ no bits.
        assert_eq!(CollisionLayer::mask_from_layers(&[]), 0);
        // Duplicate layers are idempotent (OR-fold).
        assert_eq!(CollisionLayer::mask_from_layers(&[2, 2, 2]), 1 << 2);
        // Full span of representable layers.
        assert_eq!(CollisionLayer::mask_from_layers(&[0, 31]), 1 | (1 << 31));
    }

    #[test]
    fn mask_from_layers_ignores_out_of_range() {
        // Layers >= 32 are unrepresentable; `checked_shl` returns None ⇒ 0 bits added,
        // rather than panicking or wrapping to `1 << (l % 32)`.
        assert_eq!(CollisionLayer::mask_from_layers(&[32]), 0);
        assert_eq!(CollisionLayer::mask_from_layers(&[64, 100]), 0);
        // A valid layer mixed with an out-of-range one keeps only the valid bit.
        assert_eq!(CollisionLayer::mask_from_layers(&[5, 40]), 1 << 5);
    }
}
