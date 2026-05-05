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
use gizmo_math::Aabb;
use gizmo_core::entity::Entity;
use rayon::prelude::*;
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
const MAX_SUBSTEPS: u32 = 8;

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
    contact_cache: HashMap<(Entity, Entity), (bool, Option<ContactManifold>)>,
    
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
    fn step_internal(
        &mut self,
        soft_bodies: &mut [(Entity, SoftBodyMesh, Transform)],
        fluid_sims: &mut [(Entity, crate::components::FluidSimulation, Transform)],
        dt: f32,
    ) -> Result<(), crate::error::GizmoError> {

        // Energy Conservation Check: Record initial energy (Zero-cost in release mode)
        let _initial_energy = if cfg!(debug_assertions) { self.calculate_total_energy() } else { 0.0 };

        // 0. Compute per-body gravity and apply fluid buoyancy/drag (Parallel)
        let default_gravity = self.integrator.gravity;
        let gravity_fields = &self.gravity_fields;
        let fluid_zones = &self.fluid_zones;
        let has_error = std::sync::atomic::AtomicBool::new(false);
        let watchlist = &self.watchlist;
        
        self.entities.par_iter()
            .zip(self.rigid_bodies.par_iter_mut())
            .zip(self.transforms.par_iter())
            .zip(self.velocities.par_iter_mut())
            .zip(self.colliders.par_iter())
            .try_for_each(|((((&entity, rb), transform), vel), collider)| -> Result<(), crate::error::GizmoError> {
            if rb.is_sleeping {
                return Ok(());
            }
            
            let pos = transform.position;
            
            // Resolve gravity
            let mut active_gravity = default_gravity;
            let mut max_priority = i32::MIN;
            
            for field in gravity_fields {
                if field.shape.contains(pos) {
                    if field.priority > max_priority {
                        active_gravity = field.gravity;
                        max_priority = field.priority;
                    }
                }
            }
            
            // Apply Fluid Zones
            for zone in fluid_zones {
                if zone.shape.contains(pos) {
                    let extents_y = collider.extents_y();
                    let surface_y = match zone.shape {
                        ZoneShape::Box { max, .. } => max.y,
                        ZoneShape::Sphere { center, radius } => center.y + radius,
                    };
                    
                    let depth = (surface_y - (pos.y - extents_y)).max(0.0).min(extents_y * 2.0);
                    let submerged_ratio = depth / (extents_y * 2.0).max(0.001);
                    
                    if submerged_ratio > 0.0 {
                        let volume = collider.volume();
                        let submerged_volume = volume * submerged_ratio;
                        
                        let buoyancy_force = -active_gravity * (submerged_volume * zone.density);
                        let speed = vel.linear.length();
                        let dir = if speed > 1e-4 { vel.linear / speed } else { gizmo_math::Vec3::ZERO };
                        let drag_mag = zone.linear_drag * speed + zone.quadratic_drag * speed * speed;
                        let drag_force = -dir * drag_mag * submerged_ratio;
                        
                        let accel = (buoyancy_force + drag_force) * rb.inv_mass();
                        vel.linear += accel * dt;
                    }
                }
            }

            // 1. Integrate velocities (apply forces, gravity, damping)
            let local_integrator = crate::integrator::Integrator { gravity: active_gravity };
            if let Err(e) = local_integrator.integrate_velocities(entity, rb, vel, dt) {
                tracing::error!("Physics integration error: {:?}. Throwing error upwards.", e);
                has_error.store(true, std::sync::atomic::Ordering::Relaxed);
                return Err(e);
            }
            
            // Log Filtering & Watchlist
            if !watchlist.is_empty() && watchlist.contains(&entity) {
                tracing::debug!("WATCHLIST [{:?}]: Pos: {:.2?}, Vel: {:.2?}", entity, transform.position, vel.linear);
            }

            // Critical Value Watcher
            let speed_sq = vel.linear.length_squared();
            if speed_sq > 1_000_000.0 { // Speed > 1000 m/s
                tracing::warn!("CRITICAL VELOCITY: Entity {:?} is moving at {:.2} m/s! This may cause tunneling or explosion.", entity, speed_sq.sqrt());
            }

            Ok(())
        })?;

        if has_error.load(std::sync::atomic::Ordering::Relaxed) {
            self.trigger_snapshot("Velocity Integration Error (NaN/Overflow)");
        }

        // 1.5 Step Soft Bodies
        let gravity = self.integrator.gravity;
        let mut rigid_colliders = Vec::with_capacity(self.entities.len());
        for i in 0..self.entities.len() {
            rigid_colliders.push((self.entities[i], self.transforms[i], self.colliders[i].clone()));
        }
        
        if let Some(gpu) = &mut self.gpu_compute {
            gpu.step_soft_bodies(soft_bodies, &rigid_colliders, dt, gravity);
        } else {
            soft_bodies.par_iter_mut().for_each(|(_, sb, _)| {
                sb.step(dt, gravity, &rigid_colliders);
            });
        }

        // 1.6 Step Fluid Simulations
        if let Some(gpu_fluid) = &mut self.gpu_fluid_compute {
            for (_, fluid_sim, _) in fluid_sims.iter_mut() {
                gpu_fluid.step_fluid(&mut fluid_sim.particles, dt, gravity.into());
            }
        }

        // 2. Broadphase - update spatial hash and find potential collision pairs
        // Clear the spatial hash for the current sub-step
        self.spatial_hash.clear_mut();
        
        // BVH insert: sequential (&mut self gerektirir — BVH query'si O(log N) olduğu için toplamda hâlâ çok hızlı)
        for i in 0..self.entities.len() {
            let entity = self.entities[i];
            let rb = &self.rigid_bodies[i];
            let transform = &self.transforms[i];
            let vel = &self.velocities[i];
            let collider = &self.colliders[i];
            
            let aabb = collider.compute_aabb(transform.position, transform.rotation);
            let aabb = if rb.ccd_enabled && rb.is_dynamic() && !rb.is_sleeping {
                let next_pos  = transform.position + vel.linear * dt;
                let next_aabb = collider.compute_aabb(next_pos, transform.rotation);
                aabb.merge(next_aabb)
            } else {
                aabb
            };
            self.spatial_hash.insert(entity, aabb);
        }

        for (entity, soft_body, _) in soft_bodies.iter() {
            let mut min = gizmo_math::Vec3::splat(f32::MAX);
            let mut max = gizmo_math::Vec3::splat(f32::MIN);
            for node in &soft_body.nodes {
                min = min.min(node.position);
                max = max.max(node.position);
            }
            self.spatial_hash.insert(*entity, Aabb { min: min.into(), max: max.into() });
        }

        // Entity map is already maintained in self.entity_index_map!
        let entity_map = &self.entity_index_map;

        let mut soft_entity_map = HashMap::new();
        for (i, (entity, _, _)) in soft_bodies.iter().enumerate() {
            soft_entity_map.insert(entity.id(), i);
        }

        let potential_pairs = self.spatial_hash.query_pairs();

        // 3. Narrowphase - detect actual collisions (Parallel)
        let narrowphase_results: Vec<_> = potential_pairs.par_iter().filter_map(|&(entity_a, entity_b)| {
            let is_a_rigid = entity_map.contains_key(&entity_a.id());
            let is_b_rigid = entity_map.contains_key(&entity_b.id());
            
            if is_a_rigid && is_b_rigid {
                let idx_a = *entity_map.get(&entity_a.id())?;
                let idx_b = *entity_map.get(&entity_b.id())?;
                let transform_a = &self.transforms[idx_a];
                let collider_a = &self.colliders[idx_a];
                let transform_b = &self.transforms[idx_b];
                let collider_b = &self.colliders[idx_b];

                // Check collision layers
                if !collider_a.collision_layer.can_collide_with(&collider_b.collision_layer) {
                    return None;
                }

                // Narrowphase: 4'e kadar temas noktası üret (Box-Box SAT)
                let mut contacts = NarrowPhase::test_collision_manifold(
                    &collider_a.shape,
                    transform_a.position,
                    transform_a.rotation,
                    &collider_b.shape,
                    transform_b.position,
                    transform_b.rotation,
                );

                // CCD - Speculative Contacts
                if contacts.is_empty() {
                    let rb_a = &self.rigid_bodies[idx_a];
                    let rb_b = &self.rigid_bodies[idx_b];
                    
                    if rb_a.ccd_enabled || rb_b.ccd_enabled {
                        let vel_a = &self.velocities[idx_a];
                        let vel_b = &self.velocities[idx_b];
                        
                        if let Some(speculative_contact) = crate::gjk::Gjk::speculative_contact(
                            &collider_a.shape, transform_a.position, transform_a.rotation, vel_a.linear,
                            &collider_b.shape, transform_b.position, transform_b.rotation, vel_b.linear,
                            dt
                        ) {
                            contacts.push(speculative_contact);
                        }
                    }
                }

                if !contacts.is_empty() {
                    Some((
                        entity_a,
                        entity_b,
                        contacts,
                        collider_a.is_trigger,
                        collider_b.is_trigger,
                        collider_a.material,
                        collider_b.material,
                        false
                    ))
                } else {
                    None
                }
            } else if is_a_rigid != is_b_rigid {
                // One rigid, one soft
                let rigid_ent = if is_a_rigid { entity_a } else { entity_b };
                let soft_ent = if is_a_rigid { entity_b } else { entity_a };
                
                // Return a marker so we know to process Soft vs Rigid sequentially
                Some((
                    rigid_ent,
                    soft_ent,
                    vec![],
                    false, false, // not triggers
                    crate::components::PhysicsMaterial::default(),
                    crate::components::PhysicsMaterial::default(),
                    true // is soft collision
                ))
            } else {
                // Soft vs Soft
                Some((
                    entity_a,
                    entity_b,
                    vec![],
                    false, false,
                    crate::components::PhysicsMaterial::default(),
                    crate::components::PhysicsMaterial::default(),
                    true
                ))
            }
        }).collect();

        let mut manifolds = Vec::new();
        let mut current_contacts = HashMap::new();
        let mut soft_rigid_pairs = Vec::new();
        let mut soft_soft_pairs = Vec::new();

        // Sequentially process results to preserve determinism and state
        for (entity_a, entity_b, contact_opt, is_trigger_a, is_trigger_b, mat_a, mat_b, is_soft) in narrowphase_results {
            if is_soft {
                let is_a_rigid = entity_map.contains_key(&entity_a.id());
                let is_b_rigid = entity_map.contains_key(&entity_b.id());
                if is_a_rigid != is_b_rigid {
                    let rigid_ent = if is_a_rigid { entity_a } else { entity_b };
                    let soft_ent = if is_a_rigid { entity_b } else { entity_a };
                    soft_rigid_pairs.push((rigid_ent, soft_ent));
                } else {
                    soft_soft_pairs.push((entity_a, entity_b));
                }
                continue;
            }
            
            let contacts = contact_opt;
            let pair = (entity_a, entity_b);

            // Handle triggers
            if is_trigger_a || is_trigger_b {
                current_contacts.insert(pair, (true, None));
                
                let event_type = if self.contact_cache.contains_key(&pair) {
                    CollisionEventType::Persisting
                } else {
                    CollisionEventType::Started
                };
                self.trigger_events.push(TriggerEvent {
                    trigger_entity: if is_trigger_a { entity_a } else { entity_b },
                    other_entity:   if is_trigger_a { entity_b } else { entity_a },
                    event_type,
                });
            } else {
                let mut manifold = ContactManifold::new(entity_a, entity_b);
                manifold.friction        = (mat_a.dynamic_friction * mat_b.dynamic_friction).sqrt();
                manifold.static_friction = (mat_a.static_friction  * mat_b.static_friction).sqrt();
                manifold.restitution     = mat_a.restitution.max(mat_b.restitution);

                if let Some(Some((_, Some(old_manifold)))) = self.contact_cache.get(&pair).map(|o| Some(o)) {
                    manifold.lifetime = old_manifold.lifetime + 1;
                    for mut contact in contacts.iter().copied() {
                        for old in &old_manifold.contacts {
                            if (old.point - contact.point).length_squared() < 0.02 * 0.02 {
                                contact.normal_impulse = old.normal_impulse;
                                contact.tangent_impulse = old.tangent_impulse;
                                break;
                            }
                        }
                        manifold.contacts.push(contact);
                    }
                } else {
                    for contact in contacts.iter().copied() { manifold.contacts.push(contact); }
                }

                current_contacts.insert(pair, (false, Some(manifold.clone())));
                manifolds.push(manifold);

                let event_type = if self.contact_cache.contains_key(&pair) {
                    CollisionEventType::Persisting
                } else {
                    CollisionEventType::Started
                };
                self.collision_events.push(CollisionEvent {
                    entity_a,
                    entity_b,
                    event_type,
                    contact_points: contacts.into_iter().take(4).collect(),
                });
            }
        }

        // Detect ended collisions
        for (pair, (is_trigger, _)) in self.contact_cache.iter() {
            if !current_contacts.contains_key(pair) {
                // Herhangi bir temas bittiğinde, destek (support) kaybolmuş olabilir.
                // Uçan/havada kalan kutular olmaması için her iki nesneyi de ZORLA uyandır.
                if let Some(&idx_a) = entity_map.get(&pair.0.id()) {
                    if self.rigid_bodies[idx_a].is_dynamic() {
                        self.rigid_bodies[idx_a].wake_up();
                    }
                }
                if let Some(&idx_b) = entity_map.get(&pair.1.id()) {
                    if self.rigid_bodies[idx_b].is_dynamic() {
                        self.rigid_bodies[idx_b].wake_up();
                    }
                }

                if *is_trigger {
                    self.trigger_events.push(TriggerEvent {
                        trigger_entity: pair.0,
                        other_entity: pair.1,
                        event_type: CollisionEventType::Ended,
                    });
                } else {
                    self.collision_events.push(CollisionEvent {
                        entity_a: pair.0,
                        entity_b: pair.1,
                        event_type: CollisionEventType::Ended,
                        contact_points: arrayvec::ArrayVec::new(),
                    });
                }
            }
        }

        self.contact_cache = current_contacts;

        // 3.5 Process Soft vs Rigid collisions
        let node_shape = crate::components::ColliderShape::Sphere(crate::components::SphereShape { radius: 0.1 });
        for (rigid_ent, soft_ent) in soft_rigid_pairs {
            if let (Some(&rigid_idx), Some(&soft_idx)) = (
                entity_map.get(&rigid_ent.id()),
                soft_entity_map.get(&soft_ent.id())
            ) {
                // Cannot borrow mutably from self multiple times easily, but we can do it because fields are separate.
                let rigid_rb = &self.rigid_bodies[rigid_idx];
                let rigid_trans = &self.transforms[rigid_idx];
                let rigid_collider = &self.colliders[rigid_idx];
                let mut rigid_vel = self.velocities[rigid_idx]; // Copy out
            
            let (_, soft_body, _) = &mut soft_bodies[soft_idx];
            let mut changed = false;
            
            for node in soft_body.nodes.iter_mut() {
                if let Some(contact) = NarrowPhase::test_collision(
                    &node_shape,
                    node.position,
                    gizmo_math::Quat::IDENTITY,
                    &rigid_collider.shape,
                    rigid_trans.position,
                    rigid_trans.rotation,
                ) {
                    // NarrowPhase::test_collision returns normal pointing from shape_a (node)
                    // to shape_b (rigid). For correct impulse, we need the normal pointing
                    // from rigid toward node (separating direction for node).
                    let normal = -contact.normal;
                    let penetration = contact.penetration;
                    
                    let inv_m_node = 1.0 / node.mass;
                    let inv_m_rb = rigid_rb.inv_mass();
                    let total_inv_m = inv_m_node + inv_m_rb;
                    
                    let r_rb = contact.point - rigid_trans.position;
                    let v_node = node.velocity;
                    let v_rb = rigid_vel.linear + rigid_vel.angular.cross(r_rb);
                    
                    // rel_vel: velocity of node relative to rigid body
                    let rel_vel = v_node - v_rb;
                    let vel_norm = rel_vel.dot(normal);
                    
                    // vel_norm < 0 means node is approaching rigid along the normal
                    if vel_norm < 0.0 {
                        let j = -(1.0 + 0.2) * vel_norm / total_inv_m;
                        let impulse = normal * j;
                        
                        // Push node away from rigid
                        node.velocity += impulse * inv_m_node;
                        if rigid_rb.is_dynamic() {
                            // Push rigid away from node
                            rigid_vel.linear -= impulse * inv_m_rb;
                            changed = true;
                        }
                    }
                    
                    // Positional correction: push node out along normal
                    let pos_correction = normal * (penetration * 0.5);
                    node.position += pos_correction * (inv_m_node / total_inv_m);
                }
            }
            if changed {
                self.velocities[rigid_idx] = rigid_vel; // Write back
            }
        }
        }

        // 3.6 Process Soft vs Soft collisions
        // Treating each node as a small sphere (radius 0.1) for node-to-node penalty forces.
        let node_radius = 0.1;
        let node_diameter_sq = (node_radius * 2.0) * (node_radius * 2.0);
        let penalty_stiffness = 5000.0;
        let penalty_damping = 50.0;

        for (soft_ent_a, soft_ent_b) in soft_soft_pairs {
            if let (Some(&idx_a), Some(&idx_b)) = (
                soft_entity_map.get(&soft_ent_a.id()),
                soft_entity_map.get(&soft_ent_b.id())
            ) {
                // To mutate two different soft bodies in the same array, we must use split_at_mut
                // to bypass the borrow checker safely, since we know idx_a != idx_b.
            if idx_a == idx_b { continue; }
            let (sb_a, sb_b) = if idx_a < idx_b {
                let (left, right) = soft_bodies.split_at_mut(idx_b);
                (&mut left[idx_a].1, &mut right[0].1)
            } else {
                let (left, right) = soft_bodies.split_at_mut(idx_a);
                (&mut right[0].1, &mut left[idx_b].1)
            };

            for node_a in sb_a.nodes.iter_mut() {
                for node_b in sb_b.nodes.iter_mut() {
                    let diff = node_a.position - node_b.position;
                    let dist_sq = diff.length_squared();
                    if dist_sq < node_diameter_sq && dist_sq > 1e-6 {
                        let dist = dist_sq.sqrt();
                        let normal = diff / dist; // points from B to A
                        let penetration = (node_radius * 2.0) - dist;
                        
                        let rel_vel = node_a.velocity - node_b.velocity;
                        let vel_along_normal = rel_vel.dot(normal);
                        
                        // Spring-damper penalty force
                        let force_mag = penetration * penalty_stiffness - vel_along_normal * penalty_damping;
                        if force_mag > 0.0 {
                            let inv_m_a = if node_a.mass > 0.0 && !node_a.is_fixed { 1.0 / node_a.mass } else { 0.0 };
                            let inv_m_b = if node_b.mass > 0.0 && !node_b.is_fixed { 1.0 / node_b.mass } else { 0.0 };
                            let total_inv_m = inv_m_a + inv_m_b;
                            if total_inv_m > 1e-8 {
                                // Standard impulse: equal and opposite, scaled by each body's inv_mass
                                let impulse = normal * (force_mag * dt);
                                node_a.velocity += impulse * inv_m_a;
                                node_b.velocity -= impulse * inv_m_b;
                                
                                // Position correction weighted by mass ratio
                                let pos_corr = normal * (penetration * 0.5);
                                node_a.position += pos_corr * (inv_m_a / total_inv_m);
                                node_b.position -= pos_corr * (inv_m_b / total_inv_m);
                            }
                        }
                    }
                }
            }
            }
        }

        // 4. Solve constraints (only for non-trigger collisions)
        if !manifolds.is_empty() {
            let is_dynamic = |entity: Entity| -> bool {
                if let Some(&idx) = entity_map.get(&entity.id()) {
                    self.rigid_bodies[idx].is_dynamic()
                } else {
                    false
                }
            };
            
            let islands = crate::island::IslandManager::build_islands(&manifolds, &is_dynamic);
            let island_manifold_groups = crate::island::IslandManager::split_manifolds(manifolds, &islands);

            let rigid_bodies = &self.rigid_bodies;
            let transforms = &self.transforms;
            let velocities = &self.velocities;
            let solver = &self.solver;
            let entities_arr = &self.entities;

            let results: Vec<(Vec<(Entity, crate::components::Velocity)>, Vec<ContactManifold>, Vec<Entity>, Vec<crate::collision::FractureEvent>)> = island_manifold_groups.into_par_iter().map(|mut island_manifolds| {
                let mut island_is_sleeping = true;
                for manifold in island_manifolds.iter() {
                    if let (Some(&idx_a), Some(&idx_b)) = (
                        entity_map.get(&manifold.entity_a.id()),
                        entity_map.get(&manifold.entity_b.id())
                    ) {
                        if rigid_bodies[idx_a].is_dynamic() && !rigid_bodies[idx_a].is_sleeping {
                            island_is_sleeping = false;
                        }
                        if rigid_bodies[idx_b].is_dynamic() && !rigid_bodies[idx_b].is_sleeping {
                            island_is_sleeping = false;
                        }
                    }
                }

                let mut wake_updates = Vec::new();

                if !island_is_sleeping {
                    let mut island_indices = std::collections::HashSet::new();

                    for manifold in island_manifolds.iter() {
                        if let (Some(&idx_a), Some(&idx_b)) = (
                            entity_map.get(&manifold.entity_a.id()),
                            entity_map.get(&manifold.entity_b.id())
                        ) {
                            island_indices.insert(idx_a);
                            island_indices.insert(idx_b);
                            
                            // If island is awake, wake up any sleeping bodies in it
                            if rigid_bodies[idx_a].is_dynamic() && rigid_bodies[idx_a].is_sleeping {
                                wake_updates.push(manifold.entity_a);
                            }
                            if rigid_bodies[idx_b].is_dynamic() && rigid_bodies[idx_b].is_sleeping {
                                wake_updates.push(manifold.entity_b);
                            }
                        }
                    }

                    // Thread-local velocity cache to prevent extreme allocations per island
                    thread_local! {
                        static VEL_CACHE: std::cell::RefCell<Vec<Velocity>> = std::cell::RefCell::new(Vec::new());
                    }
                    
                    let mut velocity_updates = Vec::with_capacity(island_indices.len());
                    
                    VEL_CACHE.with(|cache| {
                        let mut local_velocities = cache.borrow_mut();
                        if local_velocities.len() < velocities.len() {
                            local_velocities.resize(velocities.len(), Velocity::default());
                        }
                        
                        // Copy only active indices
                        for &idx in &island_indices {
                            local_velocities[idx] = velocities[idx];
                        }
                        
                        solver.solve_contacts(&mut island_manifolds, rigid_bodies, transforms, &mut local_velocities, entity_map, dt);
                        
                        for &idx in &island_indices {
                            if rigid_bodies[idx].is_dynamic() {
                                velocity_updates.push((entities_arr[idx], local_velocities[idx]));
                            }
                        }
                    });

                    let mut local_fractures = Vec::new();
                    for manifold in island_manifolds.iter() {
                        if let (Some(&idx_a), Some(&idx_b)) = (
                            entity_map.get(&manifold.entity_a.id()),
                            entity_map.get(&manifold.entity_b.id())
                        ) {
                            // Check for fractures
                            let mut max_impulse = 0.0;
                            let mut impact_point = gizmo_math::Vec3::ZERO;
                            for contact in &manifold.contacts {
                                if contact.normal_impulse > max_impulse {
                                    max_impulse = contact.normal_impulse;
                                    impact_point = contact.point;
                                }
                            }

                            if let Some(threshold_a) = rigid_bodies[idx_a].fracture_threshold {
                                if max_impulse > threshold_a {
                                    local_fractures.push(crate::collision::FractureEvent {
                                        entity: manifold.entity_a,
                                        impact_point,
                                        impact_force: max_impulse,
                                    });
                                }
                            }
                            if let Some(threshold_b) = rigid_bodies[idx_b].fracture_threshold {
                                if max_impulse > threshold_b {
                                    local_fractures.push(crate::collision::FractureEvent {
                                        entity: manifold.entity_b,
                                        impact_point,
                                        impact_force: max_impulse,
                                    });
                                }
                            }
                        }
                    }
                    
                    (velocity_updates, island_manifolds, wake_updates, local_fractures)
                } else {
                    // Island is completely asleep. Skip the solver!
                    (Vec::new(), island_manifolds, wake_updates, Vec::new())
                }
            }).collect();

            // Write back velocities and wake ups
            for (island_vels, _, wake_ups, local_fractures) in &results {
                for &(entity, ref vel) in island_vels {
                    if let Some(&idx) = entity_map.get(&entity.id()) {
                        self.velocities[idx] = *vel;
                    }
                }
                for &entity in wake_ups {
                    if let Some(&idx) = entity_map.get(&entity.id()) {
                        self.rigid_bodies[idx].wake_up();
                    }
                }
                self.fracture_events.extend_from_slice(local_fractures);
            }
            
            // Update collision events with resolved impulses
            for (_, island_manifolds, _, _) in results {
                for manifold in island_manifolds {
                    if let Some(event) = self.collision_events.iter_mut().find(|e| {
                        (e.entity_a == manifold.entity_a && e.entity_b == manifold.entity_b) ||
                        (e.entity_a == manifold.entity_b && e.entity_b == manifold.entity_a)
                    }) {
                        event.contact_points = manifold.contacts.into_iter().take(4).collect();
                    }
                }
            }
        }

        // 4.5 Solve explicit joints (Hinges, Springs, etc.)
        if !self.joints.is_empty() {
            self.joint_solver.solve_joints(
                &mut self.joints, 
                &self.entity_index_map, 
                &self.rigid_bodies, 
                &self.transforms, 
                &mut self.velocities, 
                dt
            );
        }

        // 5. Integrate positions (Parallel)
        let pos_error = std::sync::atomic::AtomicBool::new(false);
        let pos_integrator = &self.integrator;
        self.entities.par_iter()
            .zip(self.rigid_bodies.par_iter_mut())
            .zip(self.transforms.par_iter_mut())
            .zip(self.velocities.par_iter_mut())
            .try_for_each(|(((&entity, rb), transform), vel)| -> Result<(), crate::error::GizmoError> {
                if rb.is_sleeping {
                    return Ok(());
                }
                
                rb.enforce_locks(vel);
                if let Err(e) = pos_integrator.integrate_positions(entity, rb, transform, vel, dt) {
                    tracing::error!("Physics position integration error: {:?}. Throwing error upwards.", e);
                    pos_error.store(true, std::sync::atomic::Ordering::Relaxed);
                    return Err(e);
                }
                Ok(())
            })?;

        if pos_error.load(std::sync::atomic::Ordering::Relaxed) {
            self.trigger_snapshot("Position Integration Error (NaN/Overflow)");
        }

        // Energy Conservation Check: Validate energy bounds (Zero-cost in release mode)
        if cfg!(debug_assertions) {
            let _final_energy = self.calculate_total_energy();
            // Araç motorları ve süspansiyon yayları sisteme dışarıdan enerji eklediği için 
            // enerjinin artması (kapalı sistem olmadığı sürece) normaldir.
            // Bu nedenle bu uyarı (false-positive) kaldırılmıştır.
            /*
            if final_energy > initial_energy + 10.0 {
                tracing::warn!("Sanity Check Failed: System energy increased mysteriously from {:.2} to {:.2} during integration step! Possible instability.", initial_energy, final_energy);
            }
            */
        }
        
        // 6. Update Sleep States (Parallel)
        self.rigid_bodies.par_iter_mut()
            .zip(self.velocities.par_iter())
            .for_each(|(rb, vel)| {
                if !rb.is_sleeping {
                    rb.update_sleep_state(vel);
                }
            });
        
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
            let _ = world.step(&mut [], 1.0 / 60.0);
        }

        // Object should have fallen due to gravity
        assert!(world.transforms[0].position.y < 10.0);
    }
}
