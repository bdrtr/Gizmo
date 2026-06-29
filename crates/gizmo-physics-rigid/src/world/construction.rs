use super::{PhysicsWorld, Weather};
use crate::{
    components::{RigidBody, Velocity},
    integrator::Integrator,
    solver::ConstraintSolver,
};
use gizmo_physics_core::broadphase::SpatialHash;
use gizmo_physics_core::components::{Collider, Transform};
use gizmo_core::entity::Entity;

use std::collections::HashMap;

impl PhysicsWorld {
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


            contact_cache: HashMap::new(),
            accumulator: 0.0,
            render_alpha: 1.0,
            metrics: crate::island::PhysicsMetrics::default(),
            entities: Vec::new(),
            rigid_bodies: Vec::new(),
            transforms: Vec::new(),
            velocities: Vec::new(),
            colliders: Vec::new(),
            entity_index_map: HashMap::new(),
            is_paused: false,
            step_once: false,
            rewind_requested: false,
            history: std::collections::VecDeque::new(),
            max_history_frames: 600, // 5 seconds of history at 120Hz
            watchlist: std::collections::HashSet::new(),
        }
    }

    pub fn with_gravity(mut self, gravity: gizmo_math::Vec3) -> Self {
        self.integrator.gravity = gravity;
        self
    }


    pub fn enable_gpu_compute(&mut self) {
    }

    pub fn with_cell_size(mut self, cell_size: f32) -> Self {
        self.spatial_hash = SpatialHash::new(cell_size);
        self
    }

    // ── SoA Body Management ───────────────────────────────────────────────────

    pub fn add_body(
        &mut self,
        entity: Entity,
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

    pub fn clear_bodies(&mut self) {
        self.entities.clear();
        self.rigid_bodies.clear();
        self.transforms.clear();
        self.velocities.clear();
        self.colliders.clear();
        self.entity_index_map.clear();
        self.spatial_hash.clear();
    }

    pub fn sync_bodies<'a>(
        &mut self,
        incoming_bodies: impl Iterator<Item = &'a (Entity, RigidBody, Transform, Velocity, Collider)>,
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
