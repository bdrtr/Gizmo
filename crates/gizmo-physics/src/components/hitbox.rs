use gizmo_math::Vec3;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hitbox {
    pub offset: Vec3,
    pub half_extents: Vec3,
    pub damage: f32,
    pub active: bool,
}

impl Default for Hitbox {
    fn default() -> Self {
        Self {
            offset: Vec3::ZERO,
            half_extents: Vec3::new(0.2, 0.2, 0.2),
            damage: 10.0,
            active: true,
        }
    }
}

impl Hitbox {
    pub fn new(half_extents: Vec3, damage: f32) -> Self {
        Self {
            offset: Vec3::ZERO,
            half_extents,
            damage,
            active: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hurtbox {
    pub offset: Vec3,
    pub half_extents: Vec3,
    pub damage_multiplier: f32,
}

impl Default for Hurtbox {
    fn default() -> Self {
        Self {
            offset: Vec3::ZERO,
            half_extents: Vec3::new(0.3, 0.5, 0.3),
            damage_multiplier: 1.0,
        }
    }
}

impl Hurtbox {
    pub fn new(half_extents: Vec3) -> Self {
        Self {
            offset: Vec3::ZERO,
            half_extents,
            damage_multiplier: 1.0,
        }
    }
}

gizmo_core::impl_component!(Hitbox);
gizmo_core::impl_component!(Hurtbox);
