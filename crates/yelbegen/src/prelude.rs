// Bevy tarzı "use yelbegen::prelude::*" için toplu dışa aktarım (Export) tablosu
pub use crate::core::{World, Schedule, Component};
pub use crate::math::{Vec2, Vec3, Vec4, Mat4, Quat};
pub use crate::renderer::Renderer;
pub use crate::physics::{
    Collider, ColliderShape, Aabb, Sphere, 
    Transform, Velocity, RigidBody,
    physics_movement_system, physics_collision_system
};
