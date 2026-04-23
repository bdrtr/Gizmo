// ============================================================
//  Gizmo Engine — Prelude
//  `use gizmo::prelude::*;` ile her şey import edilir.
// ============================================================

// === ECS Temelleri ===
pub use crate::core::{
    component::{Children, Parent},
    Component, Entity, EntityName, Events, Schedule, Time, WindowInfo, World,
};

// === Matematik ===
pub use crate::math::{EulerRot, Mat4, Quat, Ray, Vec2, Vec3, Vec4};

// === Sadelik API (Bevy tarzı) ===
pub use crate::color::Color;
pub use crate::spawner::{Commands, InputExt, WorldExt};

// Temel Makrolar
pub use crate::gizmo_log;

// === Fizik ===
pub use crate::physics::{Collider, ColliderShape, RigidBody, Transform, Velocity};
pub use gizmo_physics::shape::{Aabb, Sphere};

// === Renderer Bileşenleri ===
pub use crate::renderer::asset::AssetManager;
pub use crate::renderer::components::{
    Camera, DirectionalLight, Material, Mesh, MeshRenderer, PointLight,
};
pub use crate::renderer::Renderer;

// === Uygulama Çerçevesi ===
pub use crate::app::App;

// === Windowing & Input ===
pub use crate::core::input::Input;
pub use winit::event::{ElementState, MouseButton};
/// `input.key(Key::W)` kısaltması için `KeyCode` alias'ı.
pub use winit::keyboard::KeyCode as Key;
pub use winit::keyboard::{KeyCode, PhysicalKey};

// === GPU (sık kullanılan tipler) ===
pub use wgpu;

// === Audio (feature flag ile) ===
#[cfg(feature = "audio")]
pub use crate::audio::{AudioManager, AudioSource};

// === Scene (feature flag ile) ===
#[cfg(feature = "scene")]
pub use crate::scene::{SceneData, SceneRegistry};

// === Scripting (feature flag ile) ===
#[cfg(feature = "scripting")]
pub use crate::scripting as gizmo_scripting;
