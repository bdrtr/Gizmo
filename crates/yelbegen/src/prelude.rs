// ============================================================
//  Yelbegen Engine — Prelude
//  `use yelbegen::prelude::*;` ile her şey import edilir.
// ============================================================

// === ECS Temelleri ===
pub use crate::core::{World, Entity, Schedule, Component, SparseSet};

// === Matematik ===
pub use crate::math::{Vec2, Vec3, Vec4, Mat4, Quat, Ray};

// === Fizik ===
pub use crate::physics::{
    Collider, ColliderShape, Aabb, Sphere,
    Transform, Velocity, RigidBody,
};

// === Renderer Bileşenleri ===
pub use crate::renderer::components::{Mesh, Material, MeshRenderer, Camera, PointLight};
pub use crate::renderer::asset::AssetManager;
pub use crate::renderer::Renderer;

// === Uygulama Çerçevesi ===
pub use crate::app::App;

// === Windowing & Input ===
pub use winit::event::{Event, WindowEvent, DeviceEvent, ElementState, MouseButton};
pub use winit::keyboard::{PhysicalKey, KeyCode};

// === GPU (sık kullanılan tipler) ===
pub use wgpu;
pub use bytemuck;
pub use egui;

// === Audio (feature flag ile) ===
#[cfg(feature = "audio")]
pub use crate::audio as yelbegen_audio;
