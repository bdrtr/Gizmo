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
