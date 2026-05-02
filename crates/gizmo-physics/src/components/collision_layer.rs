use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CollisionLayer {
    pub layer: u32, // Which layer this object is on (0-31)
    pub mask: u32,  // Which layers this object collides with (bitfield)
}

impl Default for CollisionLayer {
    fn default() -> Self {
        Self {
            layer: 0,
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

    pub fn with_mask(mut self, mask: u32) -> Self {
        self.mask = mask;
        self
    }

    #[inline]
    pub fn can_collide_with(&self, other: &CollisionLayer) -> bool {
        let layer_bit = 1 << self.layer;
        let other_layer_bit = 1 << other.layer;
        (self.mask & other_layer_bit) != 0 && (other.mask & layer_bit) != 0
    }
}


gizmo_core::impl_component!(CollisionLayer);
