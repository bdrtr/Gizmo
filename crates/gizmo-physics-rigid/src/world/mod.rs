use crate::{
    components::{RigidBody, Velocity},
    integrator::Integrator,
    solver::ConstraintSolver,
};
use gizmo_physics_core::broadphase::SpatialHash;
use gizmo_physics_core::{CollisionEvent, ContactManifold, TriggerEvent};
use gizmo_physics_core::components::{Collider, Transform};
use gizmo_physics_core::BodyHandle;

use rustc_hash::FxHashMap;
use std::path::PathBuf;

mod construction;
mod entity_index_map;
mod query;
mod scene_query;
mod snapshot;
mod step;
#[cfg(test)]
#[allow(clippy::field_reassign_with_default)] // testlerde Default sonrası alan atama okunabilirlik için
mod tests;

pub use entity_index_map::EntityIndexMap;
pub use scene_query::{QueryFilter, ShapeCastHit};

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

/// A world-space volume that bounds a [`GravityField`] or a [`FluidZone`].
///
/// All coordinates are world-space metres. Membership is always tested against a
/// *single point* — the body's `Transform::position`, i.e. its transform origin, which
/// coincides with the centre of mass only when `RigidBody::center_of_mass` is zero.
/// The body's collider extents play no part in the test, so a large body is either
/// wholly in or wholly out depending on where its origin sits.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum ZoneShape {
    /// Axis-aligned box. It carries no rotation, so a tilted region cannot be expressed
    /// as one.
    Box {
        /// Lower corner, world metres. Every component should be ≤ the matching
        /// component of `max`; if any axis is inverted the box is empty and
        /// [`contains`](ZoneShape::contains) is never true.
        min: gizmo_math::Vec3,
        /// Upper corner, world metres. For a [`FluidZone`] this is also the face the
        /// water surface sits on — see [`FluidZone`].
        max: gizmo_math::Vec3,
    },
    /// Ball. This is the shape both [`GravityField`] and [`FluidZone`] default to, with
    /// unit radius at the world origin.
    Sphere {
        /// Centre in world metres. Moving it moves the whole volume, including — for a
        /// [`FluidZone`] — the water surface derived from it.
        center: gizmo_math::Vec3,
        /// Radius in metres. Only its square is ever used, so a negative value behaves
        /// exactly like its absolute value; `0.0` matches only the exact centre point.
        radius: f32,
    },
}

impl ZoneShape {
    /// Whether the world-space point `p` lies inside this volume.
    ///
    /// Both variants are closed — a point exactly on the boundary counts as inside.
    /// Any NaN component in `p` or in the bounds makes the result `false`.
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

/// A volume that replaces the world's default gravity for the bodies inside it.
///
/// Resolution is winner-takes-all, once per body per substep: of the fields whose
/// [`shape`](Self::shape) contains the body's transform origin, the one with the
/// highest [`priority`](Self::priority) supplies the entire gravity vector. If no
/// field contains it, `PhysicsWorld::integrator.gravity` applies. Fields never blend
/// and never sum — exactly one gravity vector is in effect for a given body. Which
/// field wins when several tie on priority is unspecified; do not rely on it.
///
/// `PhysicsWorld::gravity_fields` is part of the rollback snapshot, so editing the
/// list mid-simulation stays replayable.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct GravityField {
    /// Region of influence, in world metres; see [`ZoneShape`] for how membership is
    /// tested.
    pub shape: ZoneShape,
    /// Acceleration in m/s², world frame. This *replaces* the default rather than
    /// adding to it, so it must carry the full magnitude — Earth is
    /// `(0.0, -9.81, 0.0)` and `Vec3::ZERO` means weightless, not "use the default".
    ///
    /// A body with `use_gravity == false` does not free-fall under it, but this vector
    /// still sets the direction and magnitude of that body's buoyancy in any
    /// overlapping [`FluidZone`] — so an inverted field pushes floating bodies down.
    pub gravity: gizmo_math::Vec3,
    /// Intended distance-based attenuation radius in metres.
    ///
    /// **Currently inert**: no code reads this field, so gravity is uniform across the
    /// whole `shape` whatever the value. Treat it as unimplemented rather than as a
    /// knob with a subtle effect.
    pub falloff_radius: f32, // If > 0, gravity drops off
    /// Tie-break rank among *overlapping* fields; higher wins.
    ///
    /// Purely relative to the other fields — it is not compared against anything
    /// representing the world default, so a very negative priority does not fall back
    /// to `integrator.gravity`. Any containing field, at any priority, suppresses it.
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

/// A volume of liquid that applies buoyancy and drag to the bodies inside it, and
/// supplies the underwater fog a submerged camera should use.
///
/// Unlike [`GravityField`], zones do **not** compete: a body whose origin lies in two
/// overlapping zones receives both zones' buoyancy and both zones' drag, additively.
/// Submersion is an approximation, not an exact intersected volume — the body's
/// vertical half-extent is compared against a flat surface plane (`max.y` for a box,
/// `center.y + radius` for a sphere) to get a ratio in `0.0..=1.0`, and both the
/// displaced volume and the drag are scaled by it. Rotation is ignored in that
/// estimate, so a long thin body reports the same submersion whatever its attitude,
/// and a collider with no vertical extent (a plane) never registers as submerged.
///
/// Forces are applied to linear velocity only; a spinning body is not slowed by the
/// fluid. `PhysicsWorld::fluid_zones` is part of the rollback snapshot.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct FluidZone {
    /// Region of liquid, in world metres. It fixes both membership (see [`ZoneShape`])
    /// and the height of the surface plane described above, so moving or resizing the
    /// zone moves the water surface with it.
    pub shape: ZoneShape,
    /// Fluid density in kg/m³ (fresh water ≈ 1000, the [`Default`]). Buoyancy is
    /// `-gravity * submerged_volume * density`, so this scales lift linearly; `0.0`
    /// removes lift while leaving drag intact, and values above the body's own density
    /// make it float.
    pub density: f32,        // kg/m^3
    /// Dynamic viscosity, intended for a Stokes-drag term.
    ///
    /// **Currently inert**: the rigid-body pipeline builds drag from
    /// [`linear_drag`](Self::linear_drag) and [`quadratic_drag`](Self::quadratic_drag)
    /// only and never reads this field. Setting it has no effect on the simulation.
    pub viscosity: f32,      // dynamic viscosity for Stokes drag
    /// Coefficient of the speed-proportional drag term.
    ///
    /// The drag force magnitude is `(linear_drag * |v| + quadratic_drag * |v|²)`
    /// scaled by the submerged ratio, directed against the linear velocity. Below
    /// about `1e-4` m/s no drag is applied at all — the direction would be
    /// ill-defined. `0.0` (the [`Default`]) means an inviscid zone that only floats
    /// bodies; nothing damps them and they will bob indefinitely.
    pub linear_drag: f32,    // fallback linear drag
    /// Coefficient of the `|v|²` term of the same drag force; it dominates
    /// [`linear_drag`](Self::linear_drag) at high speed and is negligible at low
    /// speed. Same submerged-ratio scaling and same low-speed cut-off.
    pub quadratic_drag: f32, // fallback quadratic drag
    /// The underwater fog colour applied while the camera is inside this volume (linear RGB),
    /// so each body of water defines its own underwater look (shallow turquoise vs deep navy).
    /// Missing in serde → [0;3].
    #[serde(default)]
    pub fog_color: [f32; 3],
    /// Underwater fog density (Beer-Lambert; larger = visibility closes in sooner). Missing in
    /// serde → 0.
    #[serde(default)]
    pub fog_density: f32,
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
            fog_color: [0.02, 0.10, 0.14], // deniz mavisi-yeşili
            fog_density: 0.08,
        }
    }
}

/// A water sample at a point: the surface height, depth and density of the fluid zone that
/// contains it. The swimming character controller and the camera's underwater test share this
/// one query.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterSample {
    /// The water surface's world Y — the top of the zone.
    pub surface_y: f32,
    /// How far the point is below the surface (m, ≥0).
    pub depth: f32,
    /// Fluid density (kg/m³).
    pub density: f32,
    /// This volume's underwater fog colour (for post-processing while the camera is submerged).
    pub fog_color: [f32; 3],
    /// This volume's underwater fog density.
    pub fog_density: f32,
}

impl PhysicsWorld {
    /// Returns a water sample if `p` is inside any fluid zone (when several zones overlap, the
    /// one with the HIGHEST surface wins); `None` outside every zone. The shared query behind
    /// swimming and camera-submersion detection.
    pub fn water_at(&self, p: gizmo_math::Vec3) -> Option<WaterSample> {
        let mut best: Option<WaterSample> = None;
        for zone in &self.fluid_zones {
            if !zone.shape.contains(p) {
                continue;
            }
            let surface_y = match zone.shape {
                ZoneShape::Box { max, .. } => max.y,
                ZoneShape::Sphere { center, radius } => center.y + radius,
            };
            let sample = WaterSample {
                surface_y,
                depth: (surface_y - p.y).max(0.0),
                density: zone.density,
                fog_color: zone.fog_color,
                fog_density: zone.fog_density,
            };
            if best.is_none_or(|b| surface_y > b.surface_y) {
                best = Some(sample);
            }
        }
        best
    }

    /// Is `p` inside a body of water (i.e. submerged)?
    pub fn is_submerged(&self, p: gizmo_math::Vec3) -> bool {
        self.fluid_zones.iter().any(|z| z.shape.contains(p))
    }
}

/// Fixed internal physics frequency (Hz) — 240 Hz (with sub-stepping, excellent collision
/// detection)
const PHYSICS_HZ: f32 = 240.0;
const FIXED_DT: f32 = 1.0 / PHYSICS_HZ;
/// Maximum number of steps per sub-step — prevents the spiral of death
const MAX_SUBSTEPS: u32 = 64; // Increased from 8 to support larger DTs without losing simulation time

/// Global weather condition carried on the world as a shared setting.
///
/// The rigid-body pipeline itself ignores it completely: nothing in integration,
/// broadphase, narrowphase or the constraint solver reads this value, so changing it
/// cannot by itself alter a rigid-body trajectory. It lives here only so that one
/// value can be shared by the subsystems that do care — the vehicle tyre model reads
/// it to scale the friction-circle limit.
///
/// It IS captured by [`PhysicsWorld::snapshot`] and restored by
/// [`PhysicsWorld::restore_snapshot`] — not because the rigid pipeline needs it, but because
/// the subsystems that read it do, and it cannot be recomputed from transforms or velocities.
/// Being `#[non_exhaustive]`, downstream `match`es need a fallback arm; the vehicle model
/// treats unknown variants as the no-penalty case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[derive(Default)]
#[non_exhaustive]
pub enum Weather {
    /// Dry conditions and the [`Default`] — the neutral, no-penalty baseline every
    /// consumer is expected to calibrate against.
    #[default]
    Sunny,
    /// Wet conditions. The only variant whose grip penalty is speed-dependent in the
    /// vehicle model (aquaplaning above a threshold speed); the other two are flat.
    Rain,
    /// Snow or ice — a grip penalty that does not vary with speed.
    Snow,
}


/// A compact snapshot of the physics state for rewinding
///
/// Captured once per simulated frame (not per substep) into
/// [`PhysicsWorld::history`] and consumed by the one-frame debug rewind. It holds
/// pose and motion **only**: sleep state, contact warm-start impulses, joint latches
/// and the substep accumulator are all absent, so restoring one gives a plausible
/// visual state but *not* a bit-exact continuation. Use [`WorldSnapshot`] via
/// [`PhysicsWorld::snapshot`] when the resimulation has to match.
#[derive(Debug, Clone)]
pub struct PhysicsStateSnapshot {
    /// One entry per body, in the world's SoA row order at capture time. Since that
    /// order is not stable across body removal, a rewind is refused outright when the
    /// length no longer matches the world's — restoring would pair poses with the
    /// wrong bodies.
    pub transforms: Vec<Transform>,
    /// Linear (m/s) and angular (rad/s) velocity per body, world frame, in the same
    /// row order as [`transforms`](Self::transforms) and always the same length.
    pub velocities: Vec<Velocity>,
}

/// Main physics world that manages all physics simulation
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PhysicsWorld {
    /// Shared weather setting. Inert for rigid-body simulation — see [`Weather`].
    pub weather: Weather,

    /// Default gravity, air density and wind for bodies that no [`GravityField`]
    /// covers. Changing `integrator.gravity` retunes the whole world at once.
    ///
    /// Not serialized: a world loaded from a JSON snapshot comes back with
    /// `Integrator::default()` (Earth gravity, sea-level air, no wind), so a scene
    /// with custom gravity must reapply it after loading.
    #[serde(skip)]
    pub integrator: Integrator,
    /// Contact-solver tuning: iteration counts, warm-start factor, penetration slop,
    /// TGS-soft parameters — see [`ConstraintSolver`]. Not serialized; reset to the
    /// defaults on load.
    #[serde(skip)]
    pub solver: ConstraintSolver,
    /// Broadphase acceleration structure. Despite the name it is backed by a dynamic
    /// AABB tree, and the `cell_size` its constructor takes is ignored.
    ///
    /// Derived state, not authoritative. Every substep refreshes every body's bounds — swept
    /// along the velocity for CCD-enabled movers — and then evicts any id the world no longer
    /// has, so anything you insert or remove by hand is undone by the next `step`.
    ///
    /// The tree is now KEPT between substeps rather than cleared and refilled (2026-08-06). It
    /// fattens each leaf's box, so refreshing a body that has not left its box costs a lookup
    /// and a containment test instead of a descent, an allocation and a refit — which is most
    /// bodies most of the time, and all of them in a sleeping pile. Measured: the 1024-body
    /// broadphase benchmark 1.73 ms -> 564 us. Pair emission order differs from a freshly built
    /// tree, which is safe because the island solve is pair-order-invariant (`support_ordering`,
    /// on by default) — and the determinism hash did not move.
    #[serde(skip)]
    pub spatial_hash: SpatialHash,
    /// Non-trigger contacts observed during the last `step`: `Started` on the first
    /// substep a pair touches, `Persisting` afterwards, `Ended` on the substep the
    /// pair separates. Cleared and refilled by `step`, so entries accumulate across
    /// that frame's substeps: one continuous contact yields one event per substep, not
    /// one per frame. A paused `step` clears without refilling, but one servicing a
    /// rewind returns before the clear and leaves the previous frame's entries in
    /// place. Each event carries at most 4 contact points, whatever the manifold's real
    /// size. Not serialized.
    #[serde(skip)]
    pub collision_events: Vec<CollisionEvent>,
    /// Overlaps where at least one collider has `is_trigger` set. Such a pair never
    /// produces a manifold, so it exchanges no impulse and the bodies pass through
    /// each other. Same clear-and-refill cadence as
    /// [`collision_events`](Self::collision_events). Not serialized.
    #[serde(skip)]
    pub trigger_events: Vec<TriggerEvent>,
    /// Bodies whose solved contact impulse exceeded their `fracture_threshold` during
    /// the last `step`. Recording the event is all the pipeline does — the body is
    /// left intact and unchanged; replacing it with chunks is a separate step the
    /// caller (or the fracture system) performs. Not serialized.
    #[serde(skip)]
    pub fracture_events: Vec<gizmo_physics_core::FractureEvent>,
    /// Pre-computed shatter chunks keyed by body, so a fracture at runtime is a clone
    /// of stored geometry instead of a Voronoi decomposition. Meant to be filled ahead
    /// of time (during loading); it is empty by default, and a body with no entry
    /// simply misses — what happens then is the caller's choice, not this cache's.
    /// Not serialized.
    #[serde(skip)]
    pub fracture_cache: crate::fracture::PreFracturedCache,
    /// Explicit constraints (fixed, hinge, ball-socket, slider, spring, distance, D6),
    /// solved after contacts in every substep. Joints with `is_broken` set are skipped
    /// by the solver but stay in the list, and a joint with `collision_enabled == false`
    /// also suppresses narrowphase between its two bodies.
    ///
    /// Not serialized — a JSON snapshot loses them — but they *are* carried by
    /// [`snapshot`](Self::snapshot)/[`restore_snapshot`](Self::restore_snapshot),
    /// because the broken latch and the latched reference poses cannot be rederived
    /// from transforms and velocities.
    #[serde(skip)]
    pub joints: Vec<crate::joints::Joint>,
    /// Iteration count and correction/bias clamps for the joint pass. Like
    /// [`solver`](Self::solver) these are simulation inputs, and more iterations trade
    /// speed for stiffer, less stretchy joints. Not serialized.
    #[serde(skip)]
    pub joint_solver: crate::joints::JointSolver,

    /// Localized gravity overrides; see [`GravityField`] for how overlaps resolve.
    /// Empty (the default) means every body uses `integrator.gravity`.
    pub gravity_fields: Vec<GravityField>,
    /// Liquid volumes applying buoyancy and drag; see [`FluidZone`].
    pub fluid_zones: Vec<FluidZone>,

    #[serde(skip)]
    pub(crate) contact_cache: FxHashMap<(BodyHandle, BodyHandle), (bool, Option<ContactManifold>)>,

    /// Unspent simulation time in seconds, always less than one fixed substep except
    /// when the substep ceiling was hit. `step` adds the (clamped) frame delta to it
    /// and drains it in fixed 1/240 s substeps, which is what decouples the physics
    /// rate from the frame rate.
    ///
    /// Part of the rollback snapshot: it decides how many substeps the next frame
    /// runs, so restoring poses without it makes the resimulation diverge.
    pub accumulator: f32,
    /// Interpolation factor for rendering, written at the end of each `step` as
    /// `accumulator / fixed_dt`: `0.0` means the last substep landed exactly on the
    /// frame boundary, values near `1.0` mean nearly a full substep of unsimulated
    /// time is pending. Output only — nothing in the pipeline reads it back.
    ///
    /// Normally in `0.0..1.0`, but it can exceed `1.0` on a frame that hit the substep
    /// ceiling and left real time unspent, so clamp it before use. A `step` that was
    /// paused or that serviced a rewind returns early and leaves the previous value.
    pub render_alpha: f32,

    /// Per-frame profiling counters — see [`PhysicsMetrics`](crate::island::PhysicsMetrics)
    /// for the accumulation window and why they cannot perturb the simulation. Read only
    /// for logging and HUDs. Not serialized.
    #[serde(skip)]
    pub metrics: crate::island::PhysicsMetrics,

    // SoA (Structure of Arrays) Memory Layout
    /// Handle of the body in each SoA row; row `i` of this and of the four arrays
    /// below describe the same body, and all five always have the same length.
    ///
    /// Row order is an implementation detail and is **not** stable: removing a body
    /// moves the last row into the hole. Never cache a row index across a removal —
    /// keep the [`BodyHandle`], which is what events, joints and queries refer to.
    /// (`state_hash` sorts by handle id precisely so that it does not depend on this
    /// order.)
    pub entities: Vec<BodyHandle>,
    /// Mass, inertia, damping, body type, lock flags, sleep state and accumulators.
    /// Mutated in place during a step: forces are drained, sleep counters advance, and
    /// contacts or joints can wake a body. Part of the rollback snapshot, because the
    /// sleep flag and its counter cannot be rederived from pose and velocity.
    pub rigid_bodies: Vec<RigidBody>,
    /// World-space pose per body. `position` is the *transform origin*, not the centre
    /// of mass — the two coincide only when `RigidBody::center_of_mass` is zero, and
    /// zone containment ([`GravityField`], [`FluidZone`]) tests the origin. The solver
    /// writes both the integrated pose and split-impulse position corrections here.
    pub transforms: Vec<Transform>,
    /// Linear velocity in m/s and angular velocity in rad/s, both in the world frame.
    /// Written by integration and by the solver.
    ///
    /// `RigidBody` axis locks act on this array in place: velocity integration zeroes
    /// the locked components of the stored velocity rather than masking a copy.
    pub velocities: Vec<Velocity>,
    /// Shape, material and collision layer per body. A collider with `is_trigger` set
    /// reports overlaps through [`trigger_events`](Self::trigger_events) and is never
    /// given a manifold, so it exerts no force.
    pub colliders: Vec<Collider>,
    /// `BodyHandle::id()` → SoA row index, the reverse of
    /// [`entities`](Self::entities). It must be kept in lockstep with the arrays: a
    /// stale entry silently points at whichever body now occupies that row. Lookups
    /// that miss are treated as "not a rigid body" rather than as an error.
    ///
    /// No longer a `rustc_hash::FxHashMap` — see [`EntityIndexMap`] for the read API
    /// (`get`, `contains_key`, `len`, `is_empty`, unchanged in behaviour) and for why the
    /// hash map is sealed away. Individual entries can no longer be edited from outside
    /// the crate (`insert`/`remove`/`clear` are `pub(crate)`), but the field itself is
    /// still `pub`: assigning a whole new map over it is possible and still breaks the
    /// lockstep invariant, so that invariant remains a convention, not an enforcement.
    pub entity_index_map: EntityIndexMap,

    // Timeline and Debugging
    /// While set, `step` clears the event lists and returns without simulating, unless
    /// [`step_once`](Self::step_once) is also set. Time does not accumulate while
    /// paused, so unpausing does not produce a catch-up burst. Not serialized.
    #[serde(skip)]
    pub is_paused: bool,
    /// One-shot single-substep advance, mainly to inch a paused world forward. `step`
    /// consumes it (clearing the flag itself), zeroes the accumulator and runs exactly
    /// one fixed substep — so the advance is 1/240 s of simulation, not one render
    /// frame, and any time already banked in the accumulator is discarded. It takes
    /// effect whether or not the world is paused, overriding the `dt` passed to `step`.
    /// Not serialized.
    #[serde(skip)]
    pub step_once: bool,
    /// One-shot request to step back one recorded frame. The next `step` consumes it,
    /// pops the newest [`history`](Self::history) entry, restores pose and velocity
    /// from it and returns **without simulating** that frame. The restore is skipped
    /// with a warning when the body count has changed since capture, and it is silently
    /// a no-op when the history is empty.
    ///
    /// A debugging aid only: since only pose and velocity are restored, the world does
    /// not resume bit-identically. Not serialized.
    #[serde(skip)]
    pub rewind_requested: bool,
    /// Ring buffer of end-of-frame [`PhysicsStateSnapshot`]s, newest at the back. One
    /// entry is pushed per *simulated* `step` — paused and rewinding frames record
    /// nothing. Not serialized.
    #[serde(skip)]
    pub history: std::collections::VecDeque<PhysicsStateSnapshot>,
    /// Cap on [`history`](Self::history) length; the oldest entry is dropped once it
    /// is exceeded. Also the rewind depth, in simulated frames.
    ///
    /// **Defaults to `0`, so rewind is off until you ask for it** (changed 2026-08-06; it used
    /// to default to 600). One retained frame costs 160 bytes per body — `Transform` 112 plus
    /// `Velocity` 48 — so the old default was 37 MB resident on a 384-body pile and 192 MB on
    /// this engine's own 2000-box stress scene, for a debugging aid most scenes never use. Set
    /// it to the number of frames you actually want to be able to step back.
    ///
    /// At `0` nothing is recorded and no clone is taken, so the timeline costs exactly nothing;
    /// [`rewind_requested`](Self::rewind_requested) then finds an empty buffer and is a no-op.
    pub max_history_frames: usize,

    /// Bodies to trace-log during velocity integration, one line per body per substep
    /// with position and linear velocity. Purely diagnostic — membership never changes
    /// the simulation. An empty set (the default) short-circuits the check, so leave it
    /// empty in production. Not serialized.
    #[serde(skip)]
    pub watchlist: std::collections::HashSet<BodyHandle>,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// A COMPLETE simulation-state snapshot for rollback/replay (phase 3 netcode).
///
/// DIFFERENT from `PhysicsStateSnapshot` (transform+velocity only, for a 1-frame rewind): it
/// also carries the INTERNAL state deterministic RE-SIMULATION needs — `rigid_bodies` (sleep
/// state + counters), **`contact_cache` (the warm-start impulses)**, the substep `accumulator`
/// and **`joints`** (the `is_broken` latch + latched reference poses). Without those the solver
/// converges from a different warm start after a restore, and the rollback re-simulation DIVERGES
/// from the uninterrupted one. (`entities`/`colliders`/`entity_index_map` are assumed UNCHANGED
/// across the rollback window — no spawns, no despawns.)
///
/// **The rule for adding a field:** the test is not "is it big" but *can it be derived from
/// `transforms`/`velocities`*. If it cannot, it MUST be here — and for most fields its absence
/// CANNOT be caught by `state_hash`. That is exactly how joint state stayed silently missing for
/// years.
///
/// Today `state_hash` mixes in, IN ADDITION to transform/velocity/sleep, these per joint: the
/// endpoint handle pair, `is_broken`, and `JointScratch`'s ten λ slots. Only the first two are
/// CARRIED state; λ is zeroed at the start of every pass, so it sits there as a tripwire (it
/// separates two runs that have diverged inside the solver but not yet leaked into velocities,
/// one substep earlier). What it does not mix is still there and the rule has not changed for
/// them: `initial_relative_rotation`, `current_angle`, `current_position` and the rest of
/// `JointData` in general are carried in the snapshot but INVISIBLE to the hash — a stale
/// reference pose is only noticed once it has leaked into the velocities.
#[derive(Debug, Clone)]
pub struct WorldSnapshot {
    transforms: Vec<Transform>,
    velocities: Vec<crate::components::Velocity>,
    rigid_bodies: Vec<crate::components::RigidBody>,
    contact_cache: FxHashMap<(BodyHandle, BodyHandle), (bool, Option<ContactManifold>)>,
    accumulator: f32,
    // Force-field state also feeds `velocity_integration_step`, so it MUST be part of
    // the rollback snapshot: these are public mutable `Vec`s that gameplay can add to /
    // clear at runtime, and if one changes inside a rollback window a restore that left
    // them untouched would resimulate under the wrong forces and diverge.
    gravity_fields: Vec<GravityField>,
    fluid_zones: Vec<FluidZone>,
    // Eklemler de simülasyon durumunun parçası. `transforms`/`velocities`'ten TÜRETİLEMEYEN
    // runtime alanları var:
    //   * `is_broken` TEK YÖNLÜ bir mandal — sahne yüklemesi dışında hiçbir yer `false`'a
    //     çekmiyor. Rollback penceresi içinde kopan bir eklem restore'dan sonra da kopuk
    //     kalıyor, yani re-simülasyon kesintisiz simülasyonun hâlâ sahip olduğu bir eklem
    //     olmadan koşuyordu.
    //   * `initial_relative_rotation` eklemin İLK çözümünde mandallanan referans pozu
    //     (ball-socket / slider / D6). Bütün koni/twist/swing limitleri ona göre ölçülüyor,
    //     dolayısıyla bayat bir referans eklemin dinlenme pozunu sessizce yeniden tanımlar.
    //
    // `is_broken` artık `state_hash`'e giriyor (uç handle çifti ve `JointScratch`'in λ
    // yuvalarıyla birlikte), yani ondaki bir desync anında görünür.
    // `initial_relative_rotation` GİRMİYOR: snapshot onu taşıyor ama hash görmüyor, dolayısıyla
    // bayat bir referans pozu ancak limit hesabından hızlara sızdıktan sonra fark edilir.
    joints: Vec<crate::joints::Joint>,
    // `weather` girdiği için: rigid pipeline onu okumuyor ama araç lastik modeli okuyor
    // (sürtünme dairesi limitini ölçekliyor) ve transform/velocity'den TÜRETİLEMİYOR — yani
    // yukarıdaki ekleme kuralının tam olarak kapsadığı şey. Rollback penceresi içinde hava
    // değişirse re-simülasyon, çoktan geri alınmış bir havanın tutuşuyla koşuyordu.
    weather: Weather,
}
