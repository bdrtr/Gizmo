use serde::{Deserialize, Serialize};
use gizmo_math::Vec3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ExplosionFalloff {
    None,       // Sabit kuvvet
    Linear,     // 1 - (dist / radius)
    Quadratic,  // (1 - dist/radius)^2
}

impl Default for ExplosionFalloff {
    fn default() -> Self {
        Self::Linear
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Explosion {
    pub force_radius: f32,
    pub force: f32,
    pub damage: f32,
    pub damage_radius: f32,
    pub falloff: ExplosionFalloff,
    pub offset: Vec3,
    pub is_active: bool,
}

impl Default for Explosion {
    fn default() -> Self {
        Self {
            force_radius: 5.0,
            force: 1000.0,
            damage: 100.0,
            damage_radius: 5.0,
            falloff: ExplosionFalloff::Linear,
            offset: Vec3::ZERO,
            is_active: true,
        }
    }
}

gizmo_core::impl_component!(Explosion);
