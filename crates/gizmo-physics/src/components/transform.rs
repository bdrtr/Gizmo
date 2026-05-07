use gizmo_math::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TransformData {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(from = "TransformData")]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    #[serde(skip)]
    pub local_matrix: Mat4,
}

impl From<TransformData> for Transform {
    fn from(data: TransformData) -> Self {
        let mut t = Self {
            position: data.position,
            rotation: data.rotation,
            scale: data.scale,
            local_matrix: Mat4::IDENTITY,
        };
        t.update_local_matrix();
        t
    }
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

    pub fn with_position(mut self, position: Vec3) -> Self {
        self.position = position;
        self.update_local_matrix();
        self
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

    /// X ekseni etrafında döndürür (radyan).
    #[inline]
    pub fn rotate_x(&mut self, angle: f32) {
        self.rotation *= Quat::from_rotation_x(angle);
        self.update_local_matrix();
    }

    /// Y ekseni etrafında döndürür (radyan).
    #[inline]
    pub fn rotate_y(&mut self, angle: f32) {
        self.rotation *= Quat::from_rotation_y(angle);
        self.update_local_matrix();
    }

    /// Z ekseni etrafında döndürür (radyan).
    #[inline]
    pub fn rotate_z(&mut self, angle: f32) {
        self.rotation *= Quat::from_rotation_z(angle);
        self.update_local_matrix();
    }

    /// Mevcut pozisyona bir delta ekler.
    #[inline]
    pub fn translate(&mut self, delta: Vec3) {
        self.position += delta;
        self.update_local_matrix();
    }

    pub fn update_local_matrix(&mut self) {
        self.local_matrix =
            Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position);
    }

    pub fn world_matrix(&self, parent: Option<&Transform>) -> Mat4 {
        match parent {
            Some(p) => p.world_matrix(None) * self.local_matrix,
            None => self.local_matrix,
        }
    }
}

gizmo_core::impl_component!(Transform);
