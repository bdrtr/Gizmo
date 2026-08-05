//! The ECS components that describe an entity's physical makeup.
//!
//! Shape, surface and filtering ([`collider`], [`physics_material`], [`collision_layer`]),
//! placement ([`transform`]), and the gameplay-side controllers that ride on top of them
//! ([`character`], [`fighter`], [`hitbox`]). Everything here is plain data: the systems that
//! read and advance these components live in the crates that depend on this one, not here.

/// Collision geometry: the [`Collider`] component plus the shape variants it can hold.
pub mod collider;
/// Layer/mask bitfield filtering — which bodies are eligible to collide at all. Layer ids
/// address bits of a `u32` mask, so only layers 0–31 are usable.
pub mod collision_layer;
/// Contact surface response (friction, restitution) and the [`CombineMode`] rules that mix
/// the two materials meeting in a contact.
pub mod physics_material;
pub mod transform;
pub mod fluid;
/// Tunables for kinematic character movement: speeds, slope and step limits, jump-feel
/// timers, and swimming.
pub mod character;
/// Fighting-game state measured in frames rather than seconds: health, block/crouch, the
/// active move and its frame data, hitstop/hitstun counters.
pub mod fighter;
/// Combat volumes ([`Hitbox`]/[`Hurtbox`]) carried in addition to the physics [`Collider`].
pub mod hitbox;

pub use collider::{BoxShape, CapsuleShape, Collider, ColliderShape, ConvexHullShape, PlaneShape, SphereShape, TriMeshShape};
pub use collision_layer::CollisionLayer;
pub use physics_material::{CombineMode, PhysicsMaterial};
pub use transform::{GlobalTransform, Transform};
pub use fluid::FluidSimulation;
pub use character::CharacterController;
pub use fighter::FighterController;
pub use hitbox::{Hitbox, Hurtbox};
pub mod gpu_physics_link;
pub use gpu_physics_link::GpuPhysicsLink;
