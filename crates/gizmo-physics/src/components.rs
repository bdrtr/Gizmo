use gizmo_math::{Mat3, Mat4, Quat, Vec3};
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
pub enum BodyType {
    Dynamic,   // Fully simulated
    Kinematic, // Moved by user, affects others
    Static,    // Never moves
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RigidBody {
    pub body_type: BodyType,
    pub mass: f32,
    pub restitution: f32,
    pub friction: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub use_gravity: bool,
    pub is_sleeping: bool,
    pub ccd_enabled: bool,
    pub local_inertia: Vec3,
    pub lock_rotation_x: bool,
    pub lock_rotation_y: bool,
    pub lock_rotation_z: bool,
    pub sleep_counter: u32, // Frames below sleep threshold
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            body_type: BodyType::Dynamic,
            mass: 1.0,
            restitution: 0.5,
            friction: 0.5,
            linear_damping: 0.01,
            angular_damping: 0.05,
            use_gravity: true,
            is_sleeping: false,
            ccd_enabled: false,
            local_inertia: Vec3::splat(1.0),
            lock_rotation_x: false,
            lock_rotation_y: false,
            lock_rotation_z: false,
            sleep_counter: 0,
        }
    }
}

impl RigidBody {
    pub fn new(mass: f32, restitution: f32, friction: f32, use_gravity: bool) -> Self {
        Self {
            body_type: BodyType::Dynamic,
            mass,
            restitution,
            friction,
            linear_damping: 0.01,
            angular_damping: 0.05,
            use_gravity,
            is_sleeping: false,
            ccd_enabled: false,
            local_inertia: Vec3::splat(1.0),
            lock_rotation_x: false,
            lock_rotation_y: false,
            lock_rotation_z: false,
            sleep_counter: 0,
        }
    }

    pub fn new_static() -> Self {
        Self {
            body_type: BodyType::Static,
            mass: 0.0,
            restitution: 0.0,
            friction: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            use_gravity: false,
            is_sleeping: true,
            ccd_enabled: false,
            local_inertia: Vec3::ZERO,
            lock_rotation_x: true,
            lock_rotation_y: true,
            lock_rotation_z: true,
            sleep_counter: 0,
        }
    }

    pub fn new_kinematic() -> Self {
        Self {
            body_type: BodyType::Kinematic,
            mass: 0.0,
            restitution: 0.0,
            friction: 0.5,
            linear_damping: 0.0,
            angular_damping: 0.0,
            use_gravity: false,
            is_sleeping: false,
            ccd_enabled: true,
            local_inertia: Vec3::ZERO,
            lock_rotation_x: false,
            lock_rotation_y: false,
            lock_rotation_z: false,
            sleep_counter: 0,
        }
    }

    pub fn wake_up(&mut self) {
        self.is_sleeping = false;
        self.sleep_counter = 0;
    }
    
    pub fn can_sleep(&self, velocity: &Velocity) -> bool {
        if !self.is_dynamic() {
            return true;
        }
        
        const SLEEP_LINEAR_THRESHOLD: f32 = 0.05;
        const SLEEP_ANGULAR_THRESHOLD: f32 = 0.05;
        
        velocity.linear.length_squared() < SLEEP_LINEAR_THRESHOLD * SLEEP_LINEAR_THRESHOLD
            && velocity.angular.length_squared() < SLEEP_ANGULAR_THRESHOLD * SLEEP_ANGULAR_THRESHOLD
    }
    
    pub fn update_sleep_state(&mut self, velocity: &Velocity) {
        const SLEEP_FRAMES_REQUIRED: u32 = 60; // ~1 second at 60fps
        
        if self.can_sleep(velocity) {
            self.sleep_counter += 1;
            if self.sleep_counter >= SLEEP_FRAMES_REQUIRED {
                self.is_sleeping = true;
            }
        } else {
            self.sleep_counter = 0;
            self.is_sleeping = false;
        }
    }

    #[inline]
    pub fn is_dynamic(&self) -> bool {
        matches!(self.body_type, BodyType::Dynamic)
    }

    #[inline]
    pub fn is_kinematic(&self) -> bool {
        matches!(self.body_type, BodyType::Kinematic)
    }

    #[inline]
    pub fn is_static(&self) -> bool {
        matches!(self.body_type, BodyType::Static)
    }

    #[inline]
    pub fn inv_mass(&self) -> f32 {
        if self.mass == 0.0 || !self.is_dynamic() {
            0.0
        } else {
            1.0 / self.mass
        }
    }

    #[inline]
    pub fn inv_local_inertia(&self) -> Vec3 {
        if self.mass == 0.0 || !self.is_dynamic() {
            Vec3::ZERO
        } else {
            Vec3::new(
                if self.local_inertia.x == 0.0 || self.lock_rotation_x {
                    0.0
                } else {
                    1.0 / self.local_inertia.x
                },
                if self.local_inertia.y == 0.0 || self.lock_rotation_y {
                    0.0
                } else {
                    1.0 / self.local_inertia.y
                },
                if self.local_inertia.z == 0.0 || self.lock_rotation_z {
                    0.0
                } else {
                    1.0 / self.local_inertia.z
                },
            )
        }
    }

    /// Get world-space inertia tensor from local inertia and rotation
    pub fn world_inertia_tensor(&self, rotation: Quat) -> Mat3 {
        let rot_mat = Mat3::from_quat(rotation);
        let local_inertia_mat = Mat3::from_diagonal(self.local_inertia);
        rot_mat * local_inertia_mat * rot_mat.transpose()
    }

    /// Get inverse world-space inertia tensor
    pub fn inv_world_inertia_tensor(&self, rotation: Quat) -> Mat3 {
        if self.mass == 0.0 || !self.is_dynamic() {
            return Mat3::ZERO;
        }
        let rot_mat = Mat3::from_quat(rotation);
        let inv_local = Mat3::from_diagonal(self.inv_local_inertia());
        rot_mat * inv_local * rot_mat.transpose()
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

    pub fn update_inertia_from_collider(&mut self, collider: &Collider) {
        match &collider.shape {
            ColliderShape::Box(b) => {
                let w = b.half_extents.x * 2.0;
                let h = b.half_extents.y * 2.0;
                let d = b.half_extents.z * 2.0;
                self.calculate_box_inertia(w, h, d);
            }
            ColliderShape::Sphere(s) => {
                self.calculate_sphere_inertia(s.radius);
            }
            ColliderShape::Capsule(c) => {
                self.calculate_capsule_inertia(c.radius, c.half_height);
            }
            ColliderShape::Plane(_) => {
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CollisionLayer {
    pub layer: u32, // Which layer this object is on (0-31)
    pub mask: u32,  // Which layers this object collides with (bitfield)
}

impl Default for CollisionLayer {
    fn default() -> Self {
        Self {
            layer: 0,
            mask: u32::MAX, // Collide with everything by default
        }
    }
}

impl CollisionLayer {
    pub fn new(layer: u32) -> Self {
        Self {
            layer,
            mask: u32::MAX,
        }
    }

    pub fn with_mask(mut self, mask: u32) -> Self {
        self.mask = mask;
        self
    }

    #[inline]
    pub fn can_collide_with(&self, other: &CollisionLayer) -> bool {
        let layer_bit = 1 << self.layer;
        let other_layer_bit = 1 << other.layer;
        (self.mask & other_layer_bit) != 0 && (other.mask & layer_bit) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Collider {
    pub shape: ColliderShape,
    pub is_trigger: bool,
    pub material: PhysicsMaterial,
    pub collision_layer: CollisionLayer,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: ColliderShape::Sphere(SphereShape { radius: 0.5 }),
            is_trigger: false,
            material: PhysicsMaterial::default(),
            collision_layer: CollisionLayer::default(),
        }
    }
}

impl Collider {
    /// Calculate AABB for this collider at given transform
    pub fn compute_aabb(&self, position: Vec3, rotation: Quat) -> gizmo_math::Aabb {
        match &self.shape {
            ColliderShape::Sphere(s) => {
                let radius_vec = Vec3::splat(s.radius);
                gizmo_math::Aabb::from_center_half_extents(position, radius_vec)
            }
            ColliderShape::Box(b) => {
                // Rotate the half extents to get world-space AABB
                let corners = [
                    Vec3::new(b.half_extents.x, b.half_extents.y, b.half_extents.z),
                    Vec3::new(-b.half_extents.x, b.half_extents.y, b.half_extents.z),
                    Vec3::new(b.half_extents.x, -b.half_extents.y, b.half_extents.z),
                    Vec3::new(b.half_extents.x, b.half_extents.y, -b.half_extents.z),
                    Vec3::new(-b.half_extents.x, -b.half_extents.y, b.half_extents.z),
                    Vec3::new(-b.half_extents.x, b.half_extents.y, -b.half_extents.z),
                    Vec3::new(b.half_extents.x, -b.half_extents.y, -b.half_extents.z),
                    Vec3::new(-b.half_extents.x, -b.half_extents.y, -b.half_extents.z),
                ];

                let mut min = Vec3::splat(f32::INFINITY);
                let mut max = Vec3::splat(f32::NEG_INFINITY);

                for corner in &corners {
                    let rotated = rotation * (*corner);
                    let world_pos = position + rotated;
                    min = min.min(world_pos);
                    max = max.max(world_pos);
                }

                gizmo_math::Aabb::new(min, max)
            }
            ColliderShape::Capsule(c) => {
                // Capsule AABB is sphere radius + half height along Y axis
                let half_height_vec = rotation * Vec3::new(0.0, c.half_height, 0.0);
                let extent = Vec3::splat(c.radius) + half_height_vec.abs();
                gizmo_math::Aabb::from_center_half_extents(position, extent)
            }
            ColliderShape::Plane(_) => {
                // Infinite plane - use a very large AABB
                let large = 10000.0;
                gizmo_math::Aabb::new(
                    position - Vec3::splat(large),
                    position + Vec3::splat(large),
                )
            }
        }
    }

    pub fn sphere(radius: f32) -> Self {
        Self {
            shape: ColliderShape::Sphere(SphereShape { radius }),
            ..Default::default()
        }
    }

    pub fn box_collider(half_extents: Vec3) -> Self {
        Self {
            shape: ColliderShape::Box(BoxShape { half_extents }),
            ..Default::default()
        }
    }

    pub fn capsule(radius: f32, half_height: f32) -> Self {
        Self {
            shape: ColliderShape::Capsule(CapsuleShape {
                radius,
                half_height,
            }),
            ..Default::default()
        }
    }

    pub fn with_trigger(mut self, is_trigger: bool) -> Self {
        self.is_trigger = is_trigger;
        self
    }

    pub fn with_material(mut self, material: PhysicsMaterial) -> Self {
        self.material = material;
        self
    }

    pub fn with_layer(mut self, layer: CollisionLayer) -> Self {
        self.collision_layer = layer;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    Sphere(SphereShape),
    Box(BoxShape),
    Capsule(CapsuleShape),
    Plane(PlaneShape),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SphereShape {
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoxShape {
    pub half_extents: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CapsuleShape {
    pub radius: f32,
    pub half_height: f32, // Height of cylindrical part (not including hemispheres)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlaneShape {
    pub normal: Vec3,
    pub distance: f32,
}

gizmo_core::impl_component!(
    Transform,
    Velocity,
    RigidBody,
    Breakable,
    Collider,
    PhysicsMaterial,
    CollisionLayer
);
