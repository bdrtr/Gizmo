//! The four components here — [`Breakable`], [`Explosion`], [`RigidBody`] and [`Velocity`] —
//! are registered with `gizmo_core::impl_component!` and can be inserted on an entity
//! directly. [`BodyType`] and [`ExplosionFalloff`] are plain enums carried inside them, not
//! components in their own right.
//!
//! Which of them a body needs is not uniform: [`RigidBody`], [`Velocity`], a `Transform` and
//! a `Collider` are required *together*, and the ECS bridge quietly skips an entity that is
//! missing any one of the four rather than half-simulating it. [`Breakable`] and
//! [`Explosion`] are optional, read only by the destruction systems in [`crate::system`] and
//! invisible to the solver.
//!
//! Frames and units are stated per field. The recurring trap is that a body's centre of mass
//! need not sit at its transform origin; where that distinction matters, the field says so.

/// [`Breakable`] — the health pool and debris recipe that turns an impact into fracture.
pub mod breakable;
/// [`Explosion`] — a one-shot radial impulse/damage source, with its
/// [`ExplosionFalloff`] distance curves.
pub mod explosion;
/// [`RigidBody`] and [`BodyType`] — mass, inertia, damping, drag, sleep state and axis
/// locks.
pub mod rigid_body;
/// [`Velocity`] — the per-body velocity state.
pub mod velocity;

pub use breakable::Breakable;
pub use explosion::{Explosion, ExplosionFalloff};
pub use rigid_body::{BodyType, RigidBody};
pub use velocity::Velocity;
