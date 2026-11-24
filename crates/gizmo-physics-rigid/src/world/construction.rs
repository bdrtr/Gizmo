use super::{PhysicsWorld, Weather};
use crate::{
    components::{RigidBody, Velocity},
    integrator::Integrator,
    solver::ConstraintSolver,
};
use gizmo_physics_core::broadphase::SpatialHash;
use gizmo_physics_core::components::{Collider, Transform};
use gizmo_physics_core::BodyHandle;

use rustc_hash::FxHashMap;

impl PhysicsWorld {
    /// An empty world with default subsystems: the default [`Integrator`] (Earth-like
    /// gravity, still air) and the default [`ConstraintSolver`] tuning, sunny weather, and
    /// no bodies, joints, gravity fields or fluid zones.
    ///
    /// The rewind history is capped at 600 entries. That cap counts calls to `step` — one
    /// snapshot per call — not fixed substeps, so how far back it reaches in wall-clock
    /// time depends entirely on how often the caller steps the world.
    ///
    /// Equivalent to `PhysicsWorld::default()`.
    pub fn new() -> Self {
        Self {
            weather: Weather::Sunny,
            integrator: Integrator::default(),
            solver: ConstraintSolver::default(),
            spatial_hash: SpatialHash::new(10.0),
            settled_bounds: Vec::new(),
            body_revision: 0,
            moved_asleep: Vec::new(),
            stepped_gravity: None,
            #[cfg(feature = "ecs")]
            ecs_sync: crate::system::EcsSync::default(),
            collision_events: Vec::new(),
            trigger_events: Vec::new(),
            fracture_events: Vec::new(),
            fracture_cache: crate::fracture::PreFracturedCache::new(),
            joints: Vec::new(),
            joint_solver: crate::joints::JointSolver::default(),
            bodies_without_collider: 0,
            gravity_fields: Vec::new(),
            fluid_zones: Vec::new(),


            contact_cache: FxHashMap::default(),
            substep_rate: super::DEFAULT_SUBSTEP_RATE,
            accumulator: 0.0,
            render_alpha: 1.0,
            metrics: crate::island::PhysicsMetrics::default(),
            entities: Vec::new(),
            rigid_bodies: Vec::new(),
            transforms: Vec::new(),
            velocities: Vec::new(),
            colliders: Vec::new(),
            entity_index_map: crate::world::EntityIndexMap::default(),
            ecs_generations: rustc_hash::FxHashMap::default(),
            departed_contacts: Vec::new(),
            joint_poses: Vec::new(),
            is_paused: false,
            step_once: false,
            rewind_requested: false,
            history: std::collections::VecDeque::new(),
            // Rewind is OPT-IN — see `PhysicsWorld::max_history_frames`. It used to default to
            // 600 frames, which costs 160 B per body per frame: 37 MB on a 384-body pile and
            // 192 MB on this engine's own 2000-box stress scene, resident, for a debugging aid
            // most scenes never use.
            max_history_frames: 0,
            watchlist: std::collections::HashSet::new(),
        }
    }

    /// Sets the uniform gravitational acceleration, world space, m/s².
    ///
    /// Chainable form of assigning `world.integrator.gravity`; the integrator re-reads the
    /// field every substep, so either spelling takes effect immediately and there is no
    /// derived state to refresh. It does not wake anything, so bodies that have already
    /// gone to sleep keep ignoring gravity — including a brand-new direction — until
    /// something else wakes them.
    pub fn with_gravity(mut self, gravity: gizmo_math::Vec3) -> Self {
        self.integrator.gravity = gravity;
        self
    }

    /// This world stepping at `hz` substeps a second instead of
    /// [`DEFAULT_SUBSTEP_RATE`](super::DEFAULT_SUBSTEP_RATE), each running the whole pipeline once.
    /// Fewer substeps cost less; each is longer, so a body moves further between two looks at what
    /// it touches. What holds at any rate: a pair closing faster than its size allows has a
    /// speculative contact and does not pass through, and joints are solved at 240 Hz whatever the
    /// world's rate (a free joint in as many steps of the substep, one in a contact island against
    /// where its bodies have got to in the island's solve), and a body landing meets the ground instead
    /// of sinking a substep's travel into it. What does not: a row of touching balls struck faster than
    /// five diameters a substep (160 m/s at 60 Hz for 0.5 m balls) can end with two of them in one place.
    /// Chosen here, when the world is built, and not changed in play.
    ///
    /// # Errors
    /// [`SubstepRateError::OutOfRange`](super::SubstepRateError::OutOfRange) outside
    /// [`SUBSTEP_RATES`](super::SUBSTEP_RATES).
    pub fn with_substep_rate(mut self, hz: u32) -> Result<Self, super::SubstepRateError> {
        if !super::SUBSTEP_RATES.contains(&hz) {
            return Err(super::SubstepRateError::OutOfRange { hz });
        }
        self.substep_rate = hz;
        Ok(self)
    }

    /// The world's substep rate, in hertz.
    pub fn substep_rate(&self) -> u32 {
        self.substep_rate
    }

    /// The length of one substep, in seconds: `1 / substep_rate`. A [`step`](Self::step) of a
    /// whole number of these runs exactly that many substeps and leaves nothing in the accumulator.
    pub fn substep_dt(&self) -> f32 {
        1.0 / self.substep_rate as f32
    }

    /// The most substeps one [`step`](Self::step) runs: enough for its longest clamped frame and a
    /// little debt, `MAX_SIMULATED_PER_STEP` of them (64 at 240 Hz, 16 at 60).
    pub(crate) fn max_substeps(&self) -> u32 {
        ((super::MAX_SIMULATED_PER_STEP / self.substep_dt()).round() as u32).max(1)
    }


    /// Does nothing. There is no GPU stepping path in this crate; the call is accepted and
    /// ignored, and the simulation continues to run entirely on the CPU.
    pub fn enable_gpu_compute(&mut self) {
    }

    /// Replaces the broadphase acceleration structure.
    ///
    /// `cell_size` is currently ignored: the structure behind the historical `SpatialHash`
    /// name is a dynamic AABB tree that derives its bounds from the proxies inserted into
    /// it, and its constructor discards the argument. What the call really does is swap in
    /// a *fresh* structure, dropping every proxy registered so far. Collision is unaffected
    /// — every substep rebuilds the broadphase before it is read (see
    /// [`PhysicsWorld::spatial_hash`]) — but a scene query issued between this call and the
    /// next `step` runs against an empty structure and finds nothing.
    pub fn with_cell_size(mut self, cell_size: f32) -> Self {
        self.spatial_hash = SpatialHash::new(cell_size);
        self
    }

    // ── SoA Body Management ───────────────────────────────────────────────────

    /// Appends a body to the parallel component arrays and registers its broadphase proxy.
    ///
    /// The body lands at the next free index and is addressed by `entity` from then on;
    /// indices are not stable across removal, handles are. The proxy AABB is computed from
    /// `t` in world space, and for a CCD-enabled body it is additionally fattened by one
    /// 60 Hz frame of `v.linear` — a fixed assumption about how far the body can travel,
    /// unrelated to the actual substep length.
    ///
    /// Nothing rejects a duplicate: passing a handle that is already present appends a
    /// second row and repoints the handle at it, after which the earlier row is still in
    /// the arrays but can no longer be reached through its handle.
    pub fn add_body(
        &mut self,
        entity: BodyHandle,
        rb: RigidBody,
        t: Transform,
        v: Velocity,
        c: Collider,
    ) {
        let idx = self.entities.len();
        self.body_revision += 1;

        let bounds = synced_bounds(&rb, &t, &v, &c);
        if rb.is_static() {
            self.spatial_hash.insert_static(entity, bounds);
        } else {
            self.spatial_hash.insert(entity, bounds);
        }

        self.entities.push(entity);
        self.rigid_bodies.push(rb);
        self.transforms.push(t);
        self.velocities.push(v);
        self.colliders.push(c);
        self.entity_index_map.insert(entity.id(), idx);
    }

    /// Removes every body: the component arrays, the handle→index map and the broadphase
    /// are all emptied.
    ///
    /// Deliberately narrow. Joints, cached contact/warm-start data, queued
    /// collision/trigger/fracture events, the rewind history, the substep accumulator and
    /// the gravity/fluid zones all survive untouched, so a world reused after this call
    /// still holds entries naming handles that no longer exist.
    pub fn clear_bodies(&mut self) {
        self.body_revision += 1;
        self.entities.clear();
        self.rigid_bodies.clear();
        self.transforms.clear();
        self.velocities.clear();
        self.colliders.clear();
        self.entity_index_map.clear();
        self.spatial_hash.clear();
    }

    /// Reconciles the world against an external (ECS-owned) body list: handles already
    /// present are updated in place, unknown handles are added, and any body *absent* from
    /// `incoming_bodies` is removed. Absence is deletion, so an incomplete iterator empties
    /// the world.
    ///
    /// The incoming components win outright: the stored rigid body, transform and velocity
    /// rows are overwritten wholesale, which also replaces simulation-owned state the
    /// caller may not be tracking — sleep flag and sleep counter, the force and torque
    /// accumulators, and the previous-substep velocities the position integrator needs.
    /// Copy the world's results back to the ECS before calling this, or the step that
    /// produced them is discarded. Colliders are cloned rather than moved, but the heavy
    /// shape payloads (mesh vertices/indices, hulls, BVHs) sit behind an `Arc`, so no
    /// geometry is copied.
    ///
    /// Removals go through [`Self::remove_body_at`], so surviving bodies can change index.
    pub fn sync_bodies<'a>(
        &mut self,
        incoming_bodies: impl Iterator<Item = &'a (BodyHandle, RigidBody, Transform, Velocity, Collider)>,
    ) {
        // Whatever the rows say, they came from outside the step: the ECS bridge's record of
        // what it last brought over no longer describes this world.
        self.body_revision += 1;
        let mut active_ids = rustc_hash::FxHashSet::default();
        self.update_bodies(incoming_bodies.inspect(|(entity, ..)| {
            active_ids.insert(entity.id());
        }));
        self.retain_bodies(|id| active_ids.contains(&id));
    }

    /// The update half of [`Self::sync_bodies`]: the bodies given are updated in place or added;
    /// nothing is removed.
    pub(crate) fn update_bodies<'a>(
        &mut self,
        incoming_bodies: impl Iterator<Item = &'a (BodyHandle, RigidBody, Transform, Velocity, Collider)>,
    ) {
        for (entity, rb, trans, vel, col) in incoming_bodies {
            let e_id = entity.id();

            if let Some(&idx) = self.entity_index_map.get(&e_id) {
                // Update existing body without dropping/allocating mappings
                self.rigid_bodies[idx] = *rb;
                self.transforms[idx] = *trans;

                // A velocity written to a **sleeping dynamic** body is dropped rather than stored.
                //
                // Nothing reads it while the body sleeps: `advance_sleep_counter` — the only thing
                // that could wake a body because of its own speed — is called from inside
                // `integrate_velocities`, below its `is_sleeping` early return, so the sleep state
                // machine never runs on a sleeper. The number therefore does not move the body, is
                // not damped, and is not spent. It waits. Whenever something else wakes the body —
                // an ended collision, an island mover, a joint, the editor's button — position
                // integration picks the stored value up and applies it in full, on that substep,
                // as a stale impulse from an event the player may have forgotten.
                //
                // The engine already fixed one instance of exactly this (the explosion system,
                // `system.rs`, with `tests/explosion.rs` pinning it) by waking at the writer. That
                // is the contract `RigidBody::wake_up` states: change a velocity, wake the body.
                // This guard makes the contract enforceable instead of advisory — a writer that
                // forgets now loses its write immediately and visibly, rather than banking it.
                //
                // Dynamic only, deliberately: `new_static` presets `is_sleeping = true`, and a
                // kinematic body is *driven* by the velocity written here. Both must keep syncing
                // exactly as before.
                let sleeping_dynamic =
                    self.rigid_bodies[idx].is_sleeping && self.rigid_bodies[idx].is_dynamic();
                if !sleeping_dynamic {
                    self.velocities[idx] = *vel;
                }

                // Shapes use Arc internally, so clone is cheap
                self.colliders[idx] = col.clone();

                // Update spatial hash (Fatten for CCD if enabled); a static body in the static
                // tree (see `SpatialHash`), which also moves one whose type changed.
                let bounds = synced_bounds(rb, trans, vel, col);
                if rb.is_static() {
                    self.spatial_hash.insert_static(*entity, bounds);
                } else {
                    self.spatial_hash.update(*entity, bounds);
                }
            } else {
                // Add new body
                self.add_body(*entity, *rb, *trans, *vel, col.clone());
            }
        }
    }

    /// The removal half of [`Self::sync_bodies`]: every body whose id `keep` rejects is removed,
    /// walking the rows in order (through [`Self::remove_body_at`], so survivors can change index).
    pub(crate) fn retain_bodies(&mut self, keep: impl Fn(u32) -> bool) {
        let mut i = 0;
        while i < self.entities.len() {
            if !keep(self.entities[i].id()) {
                self.remove_body_at(i);
            } else {
                i += 1;
            }
        }
    }

    /// Where the last step left the body whose ECS entity is `id` of `generation` — `None` when the
    /// world has no such body, or the id last synced belonged to another generation (a recycled id is
    /// a new body). What a kinematic body moved outside the step is carried from.
    #[cfg(feature = "ecs")]
    #[must_use]
    pub fn last_pose(&self, id: u32, generation: u32) -> Option<Transform> {
        if self.ecs_generations.get(&id) != Some(&generation) {
            return None;
        }
        self.entity_index_map.get(&id).map(|&row| self.transforms[row])
    }

    /// Records that body `id` is, on the ECS side, the entity of `generation`. If the id was
    /// last synced under a different generation it now names a different entity: the old body
    /// is removed — its joints with it — and the sync that follows adds the new one fresh.
    /// Only the ECS sync (`system.rs`, `ecs` feature) calls it.
    #[cfg(feature = "ecs")]
    pub(crate) fn note_ecs_generation(&mut self, id: u32, generation: u32) {
        match self.ecs_generations.get(&id) {
            Some(&g) if g == generation => {}
            Some(_) => {
                if let Some(&idx) = self.entity_index_map.get(&id) {
                    self.remove_body_at(idx);
                }
                self.ecs_generations.insert(id, generation);
            }
            None => {
                self.ecs_generations.insert(id, generation);
            }
        }
    }

    /// Removes the body at component-array index `idx` by swapping the last row into its
    /// place — O(1), but it permutes the arrays.
    ///
    /// The body that was last now lives at `idx` and its handle→index entry is rewritten;
    /// all other indices keep their meaning. Any index the caller cached for the moved body
    /// is stale afterwards, as is a rollback snapshot taken earlier: those store per-index
    /// vectors and can only be restored onto an unchanged body set. Look bodies up by
    /// handle across a removal.
    ///
    /// The component arrays, the index map and the broadphase proxy are updated, and **the
    /// joints attached to the body are removed with it**. They used to be left in place, and a
    /// joint finds its bodies by raw id: the next body to recycle the id was tied to the other
    /// end — measured, a body spawned 15 m away was pulled 2.5 m toward the dead body's partner in
    /// a second. Its cached contacts are taken out too — the next body on the id was told it had
    /// been in them all along — and reported `Ended` by the next step.
    ///
    /// # Panics
    ///
    /// If `idx` is not a valid body index, including any call on a world with no bodies.
    pub fn remove_body_at(&mut self, idx: usize) {
        self.body_revision += 1;
        let last_idx = self.entities.len() - 1;
        let entity = self.entities[idx];

        self.spatial_hash.remove(entity);
        self.entity_index_map.remove(&entity.id());
        self.joints.retain(|joint| joint.entity_a != entity && joint.entity_b != entity);
        self.ecs_generations.remove(&entity.id());
        // The body's cached contacts go with it: keyed by raw id, they were inherited by the next
        // body on the id. They are still reported `Ended` by the next step.
        let departed: Vec<(BodyHandle, BodyHandle)> =
            self.contact_cache.keys().filter(|(a, b)| *a == entity || *b == entity).copied().collect();
        for pair in departed {
            if let Some((is_trigger, _)) = self.contact_cache.remove(&pair) {
                self.departed_contacts.push((pair, is_trigger));
            }
        }

        if idx != last_idx {
            let last_entity = self.entities[last_idx];

            self.entities.swap(idx, last_idx);
            self.rigid_bodies.swap(idx, last_idx);
            self.transforms.swap(idx, last_idx);
            self.velocities.swap(idx, last_idx);
            self.colliders.swap(idx, last_idx);

            self.entity_index_map.insert(last_entity.id(), idx);
        }

        self.entities.pop();
        self.rigid_bodies.pop();
        self.transforms.pop();
        self.velocities.pop();
        self.colliders.pop();
    }

    // ──────────────────────────────────────────────────────────────────────────
}

/// The broadphase bounds a body is given when it is added or synced: its collider's box at `t`,
/// and for a CCD-enabled body that box stretched by one 60 Hz frame of `v.linear` — a fixed
/// assumption about how far it can travel, unrelated to the actual substep length.
pub(crate) fn synced_bounds(rb: &RigidBody, t: &Transform, v: &Velocity, c: &Collider) -> gizmo_math::Aabb {
    let aabb = c.compute_aabb(t.position, t.rotation);
    if !rb.ccd_enabled {
        return aabb;
    }
    let movement = v.linear * (1.0 / 60.0);
    let min_mov = aabb.min.min((gizmo_math::Vec3::from(aabb.min) + movement).into());
    let max_mov = aabb.max.max((gizmo_math::Vec3::from(aabb.max) + movement).into());
    gizmo_math::Aabb::new(min_mov, max_mov)
}
