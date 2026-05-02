use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Explosion {
    pub radius: f32,
    pub force: f32,
    pub is_active: bool,
}

impl Default for Explosion {
    fn default() -> Self {
        Self {
            radius: 5.0,
            force: 1000.0,
            is_active: true,
        }
    }
}


gizmo_core::impl_component!(Explosion);
