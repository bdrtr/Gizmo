use gizmo_core::impl_component;
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
    pub global_matrix: Mat4,
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
            global_matrix: Mat4::IDENTITY,
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

    pub fn update_local_matrix(&mut self) {
        self.global_matrix =
            Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position);
    }
    
    pub fn model_matrix(&self) -> Mat4 {
        self.global_matrix
    }
    
    pub fn local_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Velocity {
    pub linear: Vec3,
    pub angular: Vec3,
}

impl Velocity {
    pub fn new(linear: Vec3) -> Self {
        Self {
            linear,
            angular: Vec3::ZERO,
        }
    }
}

impl Default for Velocity {
    fn default() -> Self {
        Self::new(Vec3::ZERO)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RigidBody {
    pub mass: f32,
    pub restitution: f32,
    pub friction: f32,
    pub use_gravity: bool,
    pub is_sleeping: bool,
    pub ccd_enabled: bool,
    pub local_inertia: Vec3,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            mass: 1.0,
            restitution: 0.5,
            friction: 0.5,
            use_gravity: true,
            is_sleeping: false,
            ccd_enabled: false,
            local_inertia: Vec3::splat(1.0),
        }
    }
}

impl RigidBody {
    pub fn new(mass: f32, restitution: f32, friction: f32, use_gravity: bool) -> Self {
        Self {
            mass,
            restitution,
            friction,
            use_gravity,
            is_sleeping: false,
            ccd_enabled: false,
            local_inertia: Vec3::splat(1.0),
        }
    }
    
    pub fn new_static() -> Self {
        Self {
            mass: 0.0,
            restitution: 0.0,
            friction: 1.0,
            use_gravity: false,
            is_sleeping: true,
            ccd_enabled: false,
            local_inertia: Vec3::ZERO,
        }
    }
    
    pub fn wake_up(&mut self) {
        self.is_sleeping = false;
    }
    
    pub fn calculate_box_inertia(&mut self, _width: f32, _height: f32, _depth: f32) {}
    pub fn calculate_sphere_inertia(&mut self, _radius: f32) {}
    pub fn calculate_capsule_inertia(&mut self, _radius: f32, _half_height: f32) {}
    
    pub fn update_inertia_from_shape(&mut self, _shape: &crate::shape::ColliderShape) {}
}

impl_component!(Transform, Velocity, RigidBody);
