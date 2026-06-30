use crate::{
    components::{RigidBody, Velocity},
    integrator::Integrator,
    solver::ConstraintSolver,
};
use gizmo_physics_core::broadphase::SpatialHash;
use gizmo_physics_core::{CollisionEvent, ContactManifold, TriggerEvent};
use gizmo_physics_core::components::{Collider, Transform};
use gizmo_physics_core::BodyHandle;

use std::collections::HashMap;
use std::path::PathBuf;

mod construction;
mod query;
mod snapshot;
mod step;
#[cfg(test)]
mod tests;

/// Errors that can occur while writing a physics-world diagnostic snapshot
/// via [`PhysicsWorld::trigger_snapshot`].
#[derive(Debug)]
#[non_exhaustive]
pub enum SnapshotError {
    /// The snapshot file could not be created on disk.
    Create {
        /// Path the snapshot was being written to.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The world state could not be serialized to JSON.
    Serialize(serde_json::Error),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::Create { path, .. } => {
                write!(f, "failed to create physics snapshot file '{}'", path.display())
            }
            SnapshotError::Serialize(_) => {
                write!(f, "failed to serialize physics snapshot to JSON")
            }
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SnapshotError::Create { source, .. } => Some(source),
            SnapshotError::Serialize(source) => Some(source),
        }
    }
}

impl From<serde_json::Error> for SnapshotError {
    fn from(e: serde_json::Error) -> Self {
        SnapshotError::Serialize(e)
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum ZoneShape {
    Box {
        min: gizmo_math::Vec3,
        max: gizmo_math::Vec3,
    },
    Sphere {
        center: gizmo_math::Vec3,
        radius: f32,
    },
}

impl ZoneShape {
    pub fn contains(&self, p: gizmo_math::Vec3) -> bool {
        match self {
            ZoneShape::Box { min, max } => {
                p.x >= min.x
                    && p.x <= max.x
                    && p.y >= min.y
                    && p.y <= max.y
                    && p.z >= min.z
                    && p.z <= max.z
            }
            ZoneShape::Sphere { center, radius } => {
                (p - *center).length_squared() <= radius * radius
            }
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct GravityField {
    pub shape: ZoneShape,
    pub gravity: gizmo_math::Vec3,
    pub falloff_radius: f32, // If > 0, gravity drops off
    pub priority: i32,
}

impl Default for GravityField {
    fn default() -> Self {
        Self {
            shape: ZoneShape::Sphere {
                center: gizmo_math::Vec3::ZERO,
                radius: 1.0,
            },
            gravity: gizmo_math::Vec3::new(0.0, -9.81, 0.0),
            falloff_radius: 0.0,
            priority: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct FluidZone {
    pub shape: ZoneShape,
    pub density: f32,        // kg/m^3
    pub viscosity: f32,      // dynamic viscosity for Stokes drag
    pub linear_drag: f32,    // fallback linear drag
    pub quadratic_drag: f32, // fallback quadratic drag
}

impl Default for FluidZone {
    fn default() -> Self {
        Self {
            shape: ZoneShape::Sphere {
                center: gizmo_math::Vec3::ZERO,
                radius: 1.0,
            },
            density: 1000.0,
            viscosity: 1.0,
            linear_drag: 0.0,
            quadratic_drag: 0.0,
        }
    }
}

/// Sabit iç fizik frekansı (Hz) - 240Hz (Sub-stepping ile mükemmel çarpışma tespiti)
const PHYSICS_HZ: f32 = 240.0;
const FIXED_DT: f32 = 1.0 / PHYSICS_HZ;
/// Sub-step başına maksimum adım sayısı — spiral'i önler
const MAX_SUBSTEPS: u32 = 64; // Increased from 8 to support larger DTs without losing simulation time

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[derive(Default)]
#[non_exhaustive]
pub enum Weather {
    #[default]
    Sunny,
    Rain,
    Snow,
}


/// A compact snapshot of the physics state for rewinding
#[derive(Debug, Clone)]
pub struct PhysicsStateSnapshot {
    pub transforms: Vec<Transform>,
    pub velocities: Vec<Velocity>,
}

/// Main physics world that manages all physics simulation
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PhysicsWorld {
    pub weather: Weather,

    #[serde(skip)]
    pub integrator: Integrator,
    #[serde(skip)]
    pub solver: ConstraintSolver,
    #[serde(skip)]
    pub spatial_hash: SpatialHash,
    #[serde(skip)]
    pub collision_events: Vec<CollisionEvent>,
    #[serde(skip)]
    pub trigger_events: Vec<TriggerEvent>,
    #[serde(skip)]
    pub fracture_events: Vec<gizmo_physics_core::FractureEvent>,
    #[serde(skip)]
    pub fracture_cache: crate::fracture::PreFracturedCache,
    #[serde(skip)]
    pub joints: Vec<crate::joints::Joint>,
    #[serde(skip)]
    pub joint_solver: crate::joints::JointSolver,

    pub gravity_fields: Vec<GravityField>,
    pub fluid_zones: Vec<FluidZone>,

    #[serde(skip)]
    pub(crate) contact_cache: HashMap<(BodyHandle, BodyHandle), (bool, Option<ContactManifold>)>,

    pub accumulator: f32,
    pub render_alpha: f32,

    #[serde(skip)]
    pub metrics: crate::island::PhysicsMetrics,

    // SoA (Structure of Arrays) Memory Layout
    pub entities: Vec<BodyHandle>,
    pub rigid_bodies: Vec<RigidBody>,
    pub transforms: Vec<Transform>,
    pub velocities: Vec<Velocity>,
    pub colliders: Vec<Collider>,
    pub entity_index_map: HashMap<u32, usize>,

    // Timeline and Debugging
    #[serde(skip)]
    pub is_paused: bool,
    #[serde(skip)]
    pub step_once: bool,
    #[serde(skip)]
    pub rewind_requested: bool,
    #[serde(skip)]
    pub history: std::collections::VecDeque<PhysicsStateSnapshot>,
    pub max_history_frames: usize,

    #[serde(skip)]
    pub watchlist: std::collections::HashSet<BodyHandle>,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// Rollback/replay için TAM simülasyon durumu anlık görüntüsü (Faz 3 netcode).
///
/// `PhysicsStateSnapshot`'tan (yalnız transform+velocity, 1-kare rewind için) FARKLI:
/// deterministik RE-SİMÜLASYON için gereken İÇ DURUMU da taşır — `rigid_bodies` (uyku
/// durumu + sayaçlar), **`contact_cache` (warm-start impuls'ları)** ve substep
/// `accumulator`. Bunlar olmadan restore sonrası çözücü farklı warm-start'la yakınsar →
/// rollback re-simülasyonu kesintisiz simülasyondan SAPAR. (entities/colliders/
/// entity_index_map rollback penceresinde DEĞİŞMEZ varsayılır — ekleme/silme yok.)
#[derive(Debug, Clone)]
pub struct WorldSnapshot {
    transforms: Vec<Transform>,
    velocities: Vec<crate::components::Velocity>,
    rigid_bodies: Vec<crate::components::RigidBody>,
    contact_cache: HashMap<(BodyHandle, BodyHandle), (bool, Option<ContactManifold>)>,
    accumulator: f32,
}
