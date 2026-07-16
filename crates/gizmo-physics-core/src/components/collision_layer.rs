use serde::{Deserialize, Serialize};

pub const LAYER_DEFAULT: u32 = 0;
pub const LAYER_PLAYER: u32 = 1;
pub const LAYER_ENEMY: u32 = 2;
pub const LAYER_TRIGGER: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CollisionLayer {
    pub layer: u32, // Which layer this object is on (0-31)
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
    pub fn new(layer: u32) -> Self {
        Self {
            layer,
            mask: u32::MAX,
        }
    }

    pub fn from_layer(layer: u32) -> Self {
        Self::new(layer)
    }

    pub fn with_mask(mut self, mask: u32) -> Self {
        self.mask = mask;
        self
    }

    pub fn mask_from_layers(layers: &[u32]) -> u32 {
        layers
            .iter()
            .fold(0u32, |acc, &l| acc | (1u32.checked_shl(l).unwrap_or(0)))
    }

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
