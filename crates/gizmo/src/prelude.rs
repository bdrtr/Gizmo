// ============================================================
//  Gizmo Engine — Prelude
//  `use gizmo::prelude::*;` ile her şey import edilir.
// ============================================================

// === ECS Temelleri ===
pub use crate::core::{
    asset::{Assets, Handle},
    component::{Children, Parent},
    in_state,
    system::IntoSystemConfig,
    Bundle, BundleExt, Component, Entity, EntityName, EventReader, EventWriter, Events,
    PoolManager, Pooled, Res, ResMut, Schedule, State, Time, WindowInfo, World,
};

// === Hazır Bundle'lar ===
pub use crate::bundles::{
    CameraBundle, DirectionalLightBundle, MeshBundle, PointLightBundle, SpotLightBundle,
};

// === ECS Sorgu Sistemi ===
pub use crate::core::query::{Changed, Mut, Or, Query, With, Without};

// === ECS Komut Kuyruğu ===
pub use crate::core::{Commands, EntityCommands};

// === Matematik ===
pub use crate::math::{EulerRot, Mat4, Quat, Ray, Vec2, Vec3, Vec4};

// === Sadelik API (Bevy tarzı) ===
pub use crate::app::{App, Plugin};
pub use crate::asset_server::AssetServer;
pub use crate::color::Color;
pub use crate::plugins::*;
pub use crate::spawner::{Commands as SpawnCommands, InputExt, WorldExt};

// Temel Makrolar
pub use crate::gizmo_log;

// === Fizik ===
pub use crate::physics::{
    Collider, ColliderShape, GlobalTransform, RigidBody, Transform, Velocity,
};
pub use gizmo_physics::shape::{Aabb, Sphere};

// === Renderer Bileşenleri ===
pub use crate::renderer::asset::AssetManager;
pub use crate::renderer::components::{
    Camera, DirectionalLight, LightRole, Material, Mesh, MeshRenderer, PointLight, SpotLight,
};
pub use crate::renderer::RenderContext;
pub use crate::renderer::Renderer;
pub use crate::renderer::{GizmoRendererSystem, Gizmos};

// === Hazır Sistemler ===
pub use crate::systems::render::default_render_pass;
pub use crate::systems::render::RenderContextExt;

// === Uygulama Çerçevesi ===

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
