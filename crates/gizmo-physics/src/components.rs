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

    #[inline]
    pub fn inv_mass(&self) -> f32 {
        if self.mass == 0.0 {
            0.0
        } else {
            1.0 / self.mass
        }
    }

    #[inline]
    pub fn inv_local_inertia(&self) -> gizmo_math::Vec3 {
        if self.mass == 0.0 {
            gizmo_math::Vec3::ZERO
        } else {
            gizmo_math::Vec3::new(
                if self.local_inertia.x == 0.0 { 0.0 } else { 1.0 / self.local_inertia.x },
                if self.local_inertia.y == 0.0 { 0.0 } else { 1.0 / self.local_inertia.y },
                if self.local_inertia.z == 0.0 { 0.0 } else { 1.0 / self.local_inertia.z },
            )
        }
    }

    pub fn calculate_box_inertia(&mut self, w: f32, h: f32, d: f32) {
        let m = self.mass;
        self.local_inertia = Vec3::new(
            (m / 12.0) * (h * h + d * d),
            (m / 12.0) * (w * w + d * d),
            (m / 12.0) * (w * w + h * h),
        );
    }

    pub fn calculate_sphere_inertia(&mut self, r: f32) {
        let i = 0.4 * self.mass * r * r;
        self.local_inertia = Vec3::splat(i);
    }

    pub fn calculate_capsule_inertia(&mut self, r: f32, half_h: f32) {
        let m = self.mass;
        let h = half_h * 2.0;
        // Silindir + iki yarım küre yaklaşımı
        let i_axial = m * (3.0 * r * r + h * h) / 12.0 + m * r * r / 2.0;
        let i_radial = m * r * r * 2.0 / 5.0;
        self.local_inertia = Vec3::new(i_axial, i_radial, i_axial);
    }

    pub fn update_inertia_from_shape(&mut self, shape: &crate::shape::ColliderShape) {
        match shape {
            crate::shape::ColliderShape::Aabb(aabb) => {
                let w = aabb.half_extents.x * 2.0;
                let h = aabb.half_extents.y * 2.0;
                let d = aabb.half_extents.z * 2.0;
                self.calculate_box_inertia(w, h, d);
            }
            crate::shape::ColliderShape::Sphere(s) => {
                self.calculate_sphere_inertia(s.radius);
            }
            crate::shape::ColliderShape::Capsule(c) => {
                self.calculate_capsule_inertia(c.radius, c.half_height);
            }
            crate::shape::ColliderShape::Plane { .. } => {
                self.local_inertia = Vec3::splat(f32::INFINITY);
            }
        }
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Breakable {
    pub max_pieces: u32,
    pub threshold: f32, // required impulse/force to break
    pub is_broken: bool,
}

impl Default for Breakable {
    fn default() -> Self {
        Self {
            max_pieces: 10,
            threshold: 100.0,
            is_broken: false,
        }
    }
}

gizmo_core::impl_component!(Transform, Velocity, RigidBody, Breakable);
