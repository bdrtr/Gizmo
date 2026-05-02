use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicsMaterial {
    pub static_friction: f32,
    pub dynamic_friction: f32,
    pub restitution: f32,
    pub density: f32,
}

impl Default for PhysicsMaterial {
    fn default() -> Self {
        Self {
            static_friction: 0.6,
            dynamic_friction: 0.5,
            restitution: 0.5,
            density: 1.0,
        }
    }
}

impl PhysicsMaterial {
    pub fn rubber() -> Self {
        Self {
            static_friction: 1.0,
            dynamic_friction: 0.9,
            restitution: 0.8,
            density: 1.1,
        }
    }

    pub fn ice() -> Self {
        Self {
            static_friction: 0.05,
            dynamic_friction: 0.03,
            restitution: 0.1,
            density: 0.92,
        }
    }

    pub fn metal() -> Self {
        Self {
            static_friction: 0.4,
            dynamic_friction: 0.3,
            restitution: 0.3,
            density: 7.8,
        }
    }

    pub fn wood() -> Self {
        Self {
            static_friction: 0.5,
            dynamic_friction: 0.4,
            restitution: 0.4,
            density: 0.6,
        }
    }
}


gizmo_core::impl_component!(PhysicsMaterial);
