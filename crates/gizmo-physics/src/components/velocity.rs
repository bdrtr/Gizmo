use gizmo_math::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Velocity {
    pub linear: Vec3,
    pub angular: Vec3,
    pub last_linear: Vec3,
    pub force: Vec3,
}

impl Velocity {
    pub fn new(linear: Vec3) -> Self {
        Self {
            linear,
            angular: Vec3::ZERO,
            last_linear: linear,
            force: Vec3::ZERO,
        }
    }
}

impl Default for Velocity {
    fn default() -> Self {
        Self::new(Vec3::ZERO)
    }
}


gizmo_core::impl_component!(Velocity);
