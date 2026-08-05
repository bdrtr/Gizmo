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
            collision_events: Vec::new(),
            trigger_events: Vec::new(),
            fracture_events: Vec::new(),
            fracture_cache: crate::fracture::PreFracturedCache::new(),
            joints: Vec::new(),
            joint_solver: crate::joints::JointSolver::default(),
            gravity_fields: Vec::new(),
            fluid_zones: Vec::new(),


            contact_cache: FxHashMap::default(),
            accumulator: 0.0,
            render_alpha: 1.0,
            metrics: crate::island::PhysicsMetrics::default(),
            entities: Vec::new(),
            rigid_bodies: Vec::new(),
            transforms: Vec::new(),
            velocities: Vec::new(),
            colliders: Vec::new(),
            entity_index_map: FxHashMap::default(),
            is_paused: false,
            step_once: false,
            rewind_requested: false,
            history: std::collections::VecDeque::new(),
            max_history_frames: 600, // 5 seconds of history at 120Hz
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

        let mut aabb = c.compute_aabb(t.position, t.rotation);
        if rb.ccd_enabled {
            let movement = v.linear * (1.0 / 60.0); // Fatten by max expected delta movement
            let min_mov = aabb
                .min
                .min((gizmo_math::Vec3::from(aabb.min) + movement).into());
            let max_mov = aabb
                .max
                .max((gizmo_math::Vec3::from(aabb.max) + movement).into());
            aabb = gizmo_math::Aabb::new(min_mov, max_mov);
        }
        self.spatial_hash.insert(entity, aabb);

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
        let mut active_ids = std::collections::HashSet::new();

        for (entity, rb, trans, vel, col) in incoming_bodies {
            let e_id = entity.id();
            active_ids.insert(e_id);

            if let Some(&idx) = self.entity_index_map.get(&e_id) {
                // Update existing body without dropping/allocating mappings
                self.rigid_bodies[idx] = *rb;
                self.transforms[idx] = *trans;
                self.velocities[idx] = *vel;

                // Shapes use Arc internally, so clone is cheap
                self.colliders[idx] = col.clone();

                // Update spatial hash (Fatten for CCD if enabled)
                let mut aabb = col.compute_aabb(trans.position, trans.rotation);
                if rb.ccd_enabled {
                    let movement = vel.linear * (1.0 / 60.0);
                    let min_mov = aabb
                        .min
                        .min((gizmo_math::Vec3::from(aabb.min) + movement).into());
                    let max_mov = aabb
                        .max
                        .max((gizmo_math::Vec3::from(aabb.max) + movement).into());
                    aabb = gizmo_math::Aabb::new(min_mov, max_mov);
                }
                self.spatial_hash.update(*entity, aabb);
            } else {
                // Add new body
                self.add_body(*entity, *rb, *trans, *vel, col.clone());
            }
        }

        // Cleanup removed entities
        let mut i = 0;
        while i < self.entities.len() {
            if !active_ids.contains(&self.entities[i].id()) {
                self.remove_body_at(i);
            } else {
                i += 1;
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
    /// Only the component arrays, the index map and the broadphase proxy are touched:
    /// joints attached to the body and cached contacts naming it are left in place.
    ///
    /// # Panics
    ///
    /// If `idx` is not a valid body index, including any call on a world with no bodies.
    pub fn remove_body_at(&mut self, idx: usize) {
        let last_idx = self.entities.len() - 1;
        let entity = self.entities[idx];

        self.spatial_hash.remove(entity);
        self.entity_index_map.remove(&entity.id());

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
