use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicsMaterial {
    /// General friction coefficient for sliding (0.0 = frictionless, 1.0 = highly resistive)
    pub friction: f32,
    /// Bounciness (0.0 = no bounce, 1.0 = perfect elastic collision)
    pub restitution: f32,
    /// Multiplier for tire grip. Ice would be ~0.1, Asphalt ~1.0
    pub grip_multiplier: f32,
    /// Multiplier for rolling resistance and linear drag. Mud/Sand would be > 1.0
    pub drag_multiplier: f32,
}

impl Default for PhysicsMaterial {
    fn default() -> Self {
        Self {
            friction: 0.8,
            restitution: 0.1,
            grip_multiplier: 1.0,
            drag_multiplier: 1.0,
        }
    }
}

impl PhysicsMaterial {
    pub fn ice() -> Self {
        Self {
            friction: 0.05,
            restitution: 0.0,
            grip_multiplier: 0.05,
            drag_multiplier: 0.2, // Slides easily
        }
    }

    pub fn mud() -> Self {
        Self {
            friction: 0.5,
            restitution: 0.0,
            grip_multiplier: 0.5,
            drag_multiplier: 5.0, // High drag (slows down cars rapidly)
        }
    }

    pub fn asphalt() -> Self {
        Self::default()
    }
}

gizmo_core::impl_component!(PhysicsMaterial);
