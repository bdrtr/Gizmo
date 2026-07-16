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
    CameraBundle, DirectionalLightBundle, MeshBundle, PointLightBundle, Prefab, SpotLightBundle,
    RigidBodyBundle,
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
pub use crate::spawner::{Commands as SpawnCommands, GltfLoadError, InputExt, WorldExt};

// Temel Makrolar
pub use crate::gizmo_log;

// === Fizik ===
pub use gizmo_physics_core::{Collider, ColliderShape, Transform};
pub use gizmo_physics_core::components::GlobalTransform;
pub use gizmo_physics_rigid::components::{RigidBody, Velocity};
pub use gizmo_math::Aabb;

// === Renderer Bileşenleri ===
#[cfg(feature = "render")]
pub use crate::renderer::asset::AssetManager;
#[cfg(feature = "render")]
pub use crate::renderer::components::{
    Camera, DirectionalLight, LightRole, Material, Mesh, MeshRenderer, PointLight, SpotLight,
};
#[cfg(feature = "render")]
pub use crate::renderer::RenderContext;
#[cfg(feature = "render")]
pub use crate::renderer::Renderer;
#[cfg(feature = "render")]
pub use crate::renderer::{GizmoRendererSystem, Gizmos};

// === Hazır Sistemler ===
#[cfg(feature = "render")]
pub use crate::systems::render::default_render_pass;
#[cfg(feature = "render")]
pub use crate::systems::render::RenderContextExt;
// "Bir komponent ekle, plugin çalıştır, motor halletsin" ergonomik sistemleri —
// prelude'da öne çıkar (elle her frame yeniden yazma tuzağını önlemek için).
pub use crate::systems::lifetime::{DespawnAfter, DespawnBelowY, LifetimePlugin};
pub use crate::systems::auto_collider::{AutoBoxCollider, derived_box_half_extents};
pub use crate::systems::spin::{Spin, SpinPlugin};
#[cfg(feature = "render")]
pub use crate::systems::fps_look::{FpsLook, FpsLookPlugin};

// === Uygulama Çerçevesi ===

// === Windowing & Input ===
pub use crate::core::input::Input;

#[cfg(feature = "window")]
pub use winit::event::{ElementState, MouseButton};
/// `input.key(Key::W)` kısaltması için `KeyCode` alias'ı.
#[cfg(feature = "window")]
pub use winit::keyboard::KeyCode as Key;
#[cfg(feature = "window")]
pub use winit::keyboard::{KeyCode, PhysicalKey};

// === GPU (sık kullanılan tipler) ===
#[cfg(feature = "render")]
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

// === UI (feature flag ile) ===
#[cfg(feature = "ui")]
pub use crate::ui::prelude::*;

// === Animation (feature flag ile) ===
#[cfg(feature = "animation")]
pub use crate::animation::{clip::*, player::*};

// === Sadelik Modüler Sistemleri (Phase 3) ===
#[cfg(feature = "window")]
pub use crate::simple::{CameraSettings, LightingSettings, CameraState, EditorState};
