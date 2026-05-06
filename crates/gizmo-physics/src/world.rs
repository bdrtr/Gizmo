use crate::{
    broadphase::SpatialHash,
    collision::{CollisionEvent, CollisionEventType, ContactManifold, TriggerEvent},
    components::{Collider, RigidBody, Transform, Velocity},
    integrator::Integrator,
    narrowphase::NarrowPhase,
    raycast::{Ray, Raycast, RaycastHit},
    solver::ConstraintSolver,
    soft_body::SoftBodyMesh,
};

use gizmo_core::entity::Entity;

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum ZoneShape {
    Box { min: gizmo_math::Vec3, max: gizmo_math::Vec3 },
    Sphere { center: gizmo_math::Vec3, radius: f32 },
}

impl ZoneShape {
    pub fn contains(&self, p: gizmo_math::Vec3) -> bool {
        match self {
            ZoneShape::Box { min, max } => {
                p.x >= min.x && p.x <= max.x &&
                p.y >= min.y && p.y <= max.y &&
                p.z >= min.z && p.z <= max.z
            }
            ZoneShape::Sphere { center, radius } => {
                (p - *center).length_squared() <= radius * radius
            }
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct GravityField {
    pub shape: ZoneShape,
    pub gravity: gizmo_math::Vec3,
    pub falloff_radius: f32, // If > 0, gravity drops off
    pub priority: i32,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct FluidZone {
    pub shape: ZoneShape,
    pub density: f32,        // kg/m^3
    pub viscosity: f32,      // dynamic viscosity for Stokes drag
    pub linear_drag: f32,    // fallback linear drag
    pub quadratic_drag: f32, // fallback quadratic drag
}

/// Sabit iç fizik frekansı (Hz) - 240Hz (Sub-stepping ile mükemmel çarpışma tespiti)
const PHYSICS_HZ: f32 = 240.0;
const FIXED_DT:   f32 = 1.0 / PHYSICS_HZ;
/// Sub-step başına maksimum adım sayısı — spiral'i önler
const MAX_SUBSTEPS: u32 = 64; // Increased from 8 to support larger DTs without losing simulation time

/// A compact snapshot of the physics state for rewinding
#[derive(Clone)]
pub struct PhysicsStateSnapshot {
    pub transforms: Vec<Transform>,
    pub velocities: Vec<Velocity>,
}

/// Main physics world that manages all physics simulation
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PhysicsWorld {
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
    pub fracture_events: Vec<crate::collision::FractureEvent>,
    #[serde(skip)]
    pub fracture_cache: crate::fracture::PreFracturedCache,
    #[serde(skip)]
    pub joints: Vec<crate::joints::Joint>,
    #[serde(skip)]
    pub joint_solver: crate::joints::JointSolver,
    
    pub gravity_fields: Vec<GravityField>,
    pub fluid_zones: Vec<FluidZone>,
    
    #[serde(skip)]
    pub gpu_compute: Option<crate::gpu_compute::GpuCompute>,
    #[serde(skip)]
    pub gpu_fluid_compute: Option<crate::gpu_fluid::GpuFluidCompute>,
    #[serde(skip)]
    pub(crate) contact_cache: HashMap<(Entity, Entity), (bool, Option<ContactManifold>)>,
    
    pub accumulator: f32,
    pub render_alpha: f32,
    
    #[serde(skip)]
    pub metrics: crate::island::PhysicsMetrics,

    // SoA (Structure of Arrays) Memory Layout
    pub entities: Vec<Entity>,
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
    pub watchlist: std::collections::HashSet<Entity>,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsWorld {
    pub fn new() -> Self {
        Self {
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
            gpu_compute: None,
            gpu_fluid_compute: None,
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
        self.gpu_compute = pollster::block_on(crate::gpu_compute::GpuCompute::new());
    }

    pub fn with_cell_size(mut self, cell_size: f32) -> Self {
        self.spatial_hash = SpatialHash::new(cell_size);
        self
    }

    // ── SoA Body Management ───────────────────────────────────────────────────

    pub fn add_body(&mut self, entity: Entity, rb: RigidBody, t: Transform, v: Velocity, c: Collider) {
        let idx = self.entities.len();
        
        let mut aabb = c.compute_aabb(t.position, t.rotation);
        if rb.ccd_enabled {
            let movement = v.linear * (1.0 / 60.0); // Fatten by max expected delta movement
            let min_mov = aabb.min.min((gizmo_math::Vec3::from(aabb.min) + movement).into());
            let max_mov = aabb.max.max((gizmo_math::Vec3::from(aabb.max) + movement).into());
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
        self.spatial_hash.clear_mut();
    }

    pub fn sync_bodies<'a>(&mut self, incoming_bodies: impl Iterator<Item = &'a (Entity, RigidBody, Transform, Velocity, Collider)>) {
        let mut active_ids = std::collections::HashSet::new();

        for (entity, rb, trans, vel, col) in incoming_bodies {
            let e_id = entity.id();
            active_ids.insert(e_id);

            if let Some(&idx) = self.entity_index_map.get(&e_id) {
                // Update existing body without dropping/allocating mappings
                self.rigid_bodies[idx] = rb.clone();
                self.transforms[idx] = trans.clone();
                self.velocities[idx] = vel.clone();
                
                // Shapes use Arc internally, so clone is cheap
                self.colliders[idx] = col.clone();
                
                // Update spatial hash (Fatten for CCD if enabled)
                let mut aabb = col.compute_aabb(trans.position, trans.rotation);
                if rb.ccd_enabled {
                    let movement = vel.linear * (1.0 / 60.0);
                    let min_mov = aabb.min.min((gizmo_math::Vec3::from(aabb.min) + movement).into());
                    let max_mov = aabb.max.max((gizmo_math::Vec3::from(aabb.max) + movement).into());
                    aabb = gizmo_math::Aabb::new(min_mov, max_mov);
                }
                self.spatial_hash.update(*entity, aabb);
            } else {
                // Add new body
                self.add_body(*entity, rb.clone(), trans.clone(), vel.clone(), col.clone());
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

    /// Ana fizik adımı — sabit 120Hz sub-stepping ile
    /// Render dt'yi (değişken) sabit iç fizik dt'ye dönüştürür.
    pub fn step(
        &mut self,
        soft_bodies: &mut [(Entity, SoftBodyMesh, Transform)],
        fluid_sims: &mut [(Entity, crate::components::FluidSimulation, Transform)],
        dt: f32,
    ) -> Result<(), crate::error::GizmoError> {
        if self.rewind_requested {
            self.rewind_requested = false;
            if let Some(snapshot) = self.history.pop_back() {
                if snapshot.transforms.len() == self.transforms.len() {
                    self.transforms = snapshot.transforms;
                    self.velocities = snapshot.velocities;
                    tracing::info!("Physics rewound by 1 frame!");
                } else {
                    tracing::warn!("Cannot rewind: Entity count changed.");
                }
            }
            return Ok(());
        }

        if self.is_paused && !self.step_once {
            // Clear events so we don't dispatch old collisions repeatedly
            self.collision_events.clear();
            self.trigger_events.clear();
            self.fracture_events.clear();
            return Ok(());
        }

        // --- STEP ONCE (DEBUG) ---
        let frame_dt = if self.step_once {
            self.step_once = false;
            self.accumulator = 0.0; // Reset accumulator so we step exactly once
            FIXED_DT
        } else {
            dt.min(0.25) // Maksimum 250ms — death-spiral koruması
        };

        // Olayları her render frame'de temizle
        self.collision_events.clear();
        self.trigger_events.clear();
        self.fracture_events.clear();

        // Birikimci: render dt'yi sub-step'lere böl
        self.accumulator += frame_dt;

        let mut steps = 0u32;
        while self.accumulator >= FIXED_DT && steps < MAX_SUBSTEPS {
            self.step_internal(soft_bodies, fluid_sims, FIXED_DT)?;
            self.accumulator -= FIXED_DT;
            steps += 1;
        }

        // Alpha: render interpolasyonu için (0 = önceki adım, 1 = mevcut adım)
        self.render_alpha = self.accumulator / FIXED_DT;

        // Record history snapshot at the end of the frame
        self.history.push_back(PhysicsStateSnapshot {
            transforms: self.transforms.clone(),
            velocities: self.velocities.clone(),
        });
        if self.history.len() > self.max_history_frames {
            self.history.pop_front();
        }

        Ok(())
    }

    /// İç fizik adımı — sabit FIXED_DT ile çağrılır
    /// İç fizik adımı — sabit FIXED_DT ile çağrılır
    /// Modüler pipeline: her aşama ayrı fonksiyonda (pipeline.rs)
    fn step_internal(
        &mut self,
        soft_bodies: &mut [(Entity, SoftBodyMesh, Transform)],
        fluid_sims: &mut [(Entity, crate::components::FluidSimulation, Transform)],
        dt: f32,
    ) -> Result<(), crate::error::GizmoError> {
        // Energy Conservation Check: Record initial energy (Zero-cost in release mode)
        let _initial_energy = if cfg!(debug_assertions) { self.calculate_total_energy() } else { 0.0 };

        // 0-1. Yerçekimi, sıvı bölgeleri, hız entegrasyonu
        self.velocity_integration_step(dt)?;

        // 1.5-1.6 Yumuşak cisim ve sıvı simülasyonu
        self.soft_body_and_fluid_step(soft_bodies, fluid_sims, dt);

        // 2. Broadphase — uzamsal hash güncelleme
        self.broadphase_step(soft_bodies, dt);

        // 3. Narrowphase — çarpışma tespiti ve olayları
        let manifolds = self.narrowphase_and_collision_step(soft_bodies, dt);

        // 4-4.5 Kısıt çözücü (çarpışma + eklem)
        self.constraint_solve_step(manifolds, dt);

        // 5-6. Pozisyon entegrasyonu ve uyku durumu
        self.position_integration_step(dt)?;

        // Energy Conservation Check: Validate energy bounds (Zero-cost in release mode)
        if cfg!(debug_assertions) {
            let _final_energy = self.calculate_total_energy();
        }

        Ok(())
    }


    /// Get collision events from last step
    pub fn collision_events(&self) -> &[CollisionEvent] {
        &self.collision_events
    }

    /// Get trigger events from last step
    pub fn trigger_events(&self) -> &[TriggerEvent] {
        &self.trigger_events
    }

    /// Apply an impulse to a body at a point
    pub fn apply_impulse(
        &self,
        rb: &RigidBody,
        transform: &Transform,
        vel: &mut Velocity,
        impulse: gizmo_math::Vec3,
        point: gizmo_math::Vec3,
    ) {
        Integrator::apply_impulse_at_point(rb, transform, vel, impulse, point);
    }

    /// Apply a force to a body
    pub fn apply_force(
        &self,
        rb: &RigidBody,
        vel: &mut Velocity,
        force: gizmo_math::Vec3,
        dt: f32,
    ) {
        Integrator::apply_force(rb, vel, force, dt);
    }

    /// Perform a raycast against all bodies
    pub fn raycast(
        &self,
        ray: &Ray,
        max_distance: f32,
    ) -> Option<RaycastHit> {
        let mut closest_hit: Option<RaycastHit> = None;
        let mut closest_distance = max_distance;

        let potential_hits = self.spatial_hash.query_ray(ray.origin, ray.direction, max_distance);

        for (entity, _aabb_t) in potential_hits {
            if let Some(&i) = self.entity_index_map.get(&entity.id()) {
                let transform = &self.transforms[i];
                let collider = &self.colliders[i];

                // Detailed shape test
                if let Some((distance, normal)) = Raycast::ray_shape(ray, &collider.shape, transform) {
                    if distance < closest_distance {
                        closest_distance = distance;
                        closest_hit = Some(RaycastHit {
                            entity,
                            point: ray.point_at(distance),
                            normal,
                            distance,
                        });
                    }
                }
            }
        }

        closest_hit
    }

    /// Perform a raycast and return all hits
    pub fn raycast_all(
        &self,
        ray: &Ray,
        max_distance: f32,
    ) -> Vec<RaycastHit> {
        let mut hits = Vec::new();

        let potential_hits = self.spatial_hash.query_ray(ray.origin, ray.direction, max_distance);

        for (entity, _aabb_t) in potential_hits {
            if let Some(&i) = self.entity_index_map.get(&entity.id()) {
                let transform = &self.transforms[i];
                let collider = &self.colliders[i];

                // Detailed shape test
                if let Some((distance, normal)) = Raycast::ray_shape(ray, &collider.shape, transform) {
                    hits.push(RaycastHit {
                        entity,
                        point: ray.point_at(distance),
                        normal,
                        distance,
                    });
                }
            }
        }

        // Sort by distance
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        hits
    }

    /// Telemetry and Debugging: Create a JSON snapshot of the physical world state
    pub fn trigger_snapshot(&self, reason: &str) {
        tracing::error!("Creating physics snapshot due to: {}", reason);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let filename = format!("physics_snapshot_{}.json", timestamp);
        
        match std::fs::File::create(&filename) {
            Ok(file) => {
                if let Err(e) = serde_json::to_writer_pretty(file, self) {
                    tracing::error!("Failed to serialize physics snapshot: {:?}", e);
                } else {
                    tracing::info!("Physics snapshot successfully saved to {}", filename);
                }
            }
            Err(e) => {
                tracing::error!("Failed to create snapshot file {}: {:?}", filename, e);
            }
        }
    }

    /// Calculate total kinetic and potential energy of the simulation
    pub fn calculate_total_energy(&self) -> f32 {
        let default_gravity = self.integrator.gravity;
        let mut total_energy = 0.0;
        
        for i in 0..self.entities.len() {
            let rb = &self.rigid_bodies[i];
            let vel = &self.velocities[i];
            let trans = &self.transforms[i];
            
            if rb.is_dynamic() && !rb.is_sleeping {
                // Kinetic Energy: 1/2 * m * v^2
                let ke_linear = 0.5 * rb.mass * vel.linear.length_squared();
                
                // Rotational Kinetic Energy: 1/2 * I * w^2
                // Approximation using scalar local inertia for speed
                let ke_angular = 0.5 * (
            rb.local_inertia.x * vel.angular.x * vel.angular.x +
            rb.local_inertia.y * vel.angular.y * vel.angular.y +
            rb.local_inertia.z * vel.angular.z * vel.angular.z
        );
                
                // Potential Energy: m * g * h
                let pe = if rb.use_gravity {
                    -rb.mass * default_gravity.dot(trans.position)
                } else {
                    0.0
                };
                
                total_energy += ke_linear + ke_angular + pe;
            }
        }
        total_energy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::entity::Entity;
    use gizmo_math::Vec3;

    #[test]
    fn test_physics_world_creation() {
        let world = PhysicsWorld::new();
        assert_eq!(world.integrator.gravity, Vec3::new(0.0, -9.81, 0.0));
    }

    #[test]
    fn test_physics_step() {
        let mut world = PhysicsWorld::new();

        let entity = Entity::new(1, 0);
        let rb = RigidBody::default();
        let transform = Transform::new(Vec3::new(0.0, 10.0, 0.0));
        let vel = Velocity::default();
        let collider = Collider::sphere(1.0);

        world.add_body(entity, rb, transform, vel, collider);

        // Simulate for 1 second
        for _ in 0..60 {
            let _ = world.step(&mut [], &mut [], 1.0 / 60.0);
        }

        // Object should have fallen due to gravity
        assert!(world.transforms[0].position.y < 10.0);
    }
}
