// ============================================================
//  Gizmo Engine — Prelude
//  `use gizmo::prelude::*;` ile her şey import edilir.
// ============================================================

// === ECS Temelleri ===
pub use crate::core::{
    component::{Children, Parent},
    Component, Entity, EntityName, Events, Schedule, SparseSet, Time, World,
};

// === Matematik ===
pub use crate::math::{Mat4, Quat, Ray, Vec2, Vec3, Vec4};

// === Sadelik API (Bevy tarzı) ===
pub use crate::color::Color;
pub use crate::spawner::{Commands, InputExt, WorldExt};

// Temel Makrolar
pub use crate::gizmo_log;

// === Fizik ===
pub use crate::physics::{Aabb, Collider, ColliderShape, RigidBody, Sphere, Transform, Velocity};

// === Renderer Bileşenleri ===
pub use crate::renderer::asset::AssetManager;
pub use crate::renderer::components::{
    Camera, DirectionalLight, Material, Mesh, MeshRenderer, PointLight,
};
pub use crate::renderer::Renderer;

// === Uygulama Çerçevesi ===
pub use crate::app::App;
pub use crate::default_systems::default_render_pass;

// === Windowing & Input ===
pub use crate::core::input::{mouse, Input};
pub use winit::event::{DeviceEvent, ElementState, Event, MouseButton, WindowEvent};
/// `input.key(Key::W)` kısaltması için `KeyCode` alias'ı.
pub use winit::keyboard::KeyCode as Key;
pub use winit::keyboard::{KeyCode, PhysicalKey};

// === GPU (sık kullanılan tipler) ===
pub use bytemuck;
pub use egui;
pub use wgpu;

// === Audio (feature flag ile) ===
#[cfg(feature = "audio")]
pub use crate::audio as gizmo_audio;
