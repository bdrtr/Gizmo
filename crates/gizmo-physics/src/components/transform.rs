use gizmo_math::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

fn default_mat4() -> Mat4 {
    Mat4::IDENTITY
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    #[serde(skip, default = "default_mat4")]
    pub local_matrix: Mat4,
}

impl Default for Transform {
    fn default() -> Self {
        Self::new(Vec3::ZERO)
    }
}

impl Transform {
    pub fn new(position: Vec3) -> Self {
        let mut t = Self {
            position,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            local_matrix: Mat4::IDENTITY,
        };
        t.update_local_matrix();
        t
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self.update_local_matrix();
        self
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self.update_local_matrix();
        self
    }

    pub fn set_position(&mut self, pos: Vec3) {
        self.position = pos;
        self.update_local_matrix();
    }

    pub fn set_rotation(&mut self, rot: Quat) {
        self.rotation = rot;
        self.update_local_matrix();
    }

    pub fn set_scale(&mut self, scale: Vec3) {
        self.scale = scale;
        self.update_local_matrix();
    }

    pub fn update_local_matrix(&mut self) {
        self.local_matrix =
            Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position);
    }

    pub fn model_matrix(&self) -> Mat4 {
        self.local_matrix
    }
}


gizmo_core::impl_component!(Transform);
