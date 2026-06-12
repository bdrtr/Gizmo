use crate::{
    components::{RigidBody, Velocity},
    integrator::Integrator,
    solver::ConstraintSolver,
};
use gizmo_physics_core::broadphase::SpatialHash;
use gizmo_physics_core::{CollisionEvent, ContactManifold, TriggerEvent};
use gizmo_physics_core::raycast::{Ray, Raycast, RaycastHit};
use gizmo_physics_core::components::{Collider, Transform};
use gizmo_core::entity::Entity;

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
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
const FIXED_DT: f32 = 1.0 / PHYSICS_HZ;
/// Sub-step başına maksimum adım sayısı — spiral'i önler
const MAX_SUBSTEPS: u32 = 64; // Increased from 8 to support larger DTs without losing simulation time

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[derive(Default)]
pub enum Weather {
    #[default]
    Sunny,
    Rain,
    Snow,
}


/// A compact snapshot of the physics state for rewinding
#[derive(Clone)]
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

    /// Ana fizik adımı — sabit 120Hz sub-stepping ile
    /// Render dt'yi (değişken) sabit iç fizik dt'ye dönüştürür.
    pub fn step(
        &mut self,
        
        
        dt: f32,
    ) -> Result<(), gizmo_physics_core::GizmoError> {
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
            self.step_internal(FIXED_DT)?;
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
        
        
        dt: f32,
    ) -> Result<(), gizmo_physics_core::GizmoError> {
        // Energy Conservation Check: Record initial energy (Zero-cost in release mode)
        let _initial_energy = if cfg!(debug_assertions) {
            self.calculate_total_energy()
        } else {
            0.0
        };

        // 0-1. Yerçekimi, sıvı bölgeleri, hız entegrasyonu
        self.velocity_integration_step(dt)?;

        // 1.5-1.6 Yumuşak cisim ve sıvı simülasyonu

        // 2. Broadphase — uzamsal hash güncelleme
        self.broadphase_step(dt);

        // 3. Narrowphase — çarpışma tespiti ve olayları
        let manifolds = self.narrowphase_and_collision_step(dt);

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
    pub fn raycast(&self, ray: &Ray, max_distance: f32) -> Option<RaycastHit> {
        let mut closest_hit: Option<RaycastHit> = None;
        let mut closest_distance = max_distance;

        let potential_hits = self
            .spatial_hash
            .query_ray(ray.origin, ray.direction, max_distance);

        for (entity, _aabb_t) in potential_hits {
            if let Some(&i) = self.entity_index_map.get(&entity.id()) {
                let transform = &self.transforms[i];
                let collider = &self.colliders[i];

                // Detailed shape test
                if let Some((distance, normal)) =
                    Raycast::ray_shape(ray, &collider.shape, transform)
                {
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
    pub fn raycast_all(&self, ray: &Ray, max_distance: f32) -> Vec<RaycastHit> {
        let mut hits = Vec::new();

        let potential_hits = self
            .spatial_hash
            .query_ray(ray.origin, ray.direction, max_distance);

        for (entity, _aabb_t) in potential_hits {
            if let Some(&i) = self.entity_index_map.get(&entity.id()) {
                let transform = &self.transforms[i];
                let collider = &self.colliders[i];

                // Detailed shape test
                if let Some((distance, normal)) =
                    Raycast::ray_shape(ray, &collider.shape, transform)
                {
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
        #[cfg(target_arch = "wasm32")]
        let timestamp = web_time::SystemTime::now()
            .duration_since(web_time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        #[cfg(not(target_arch = "wasm32"))]
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
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
                let ke_angular = 0.5
                    * (rb.local_inertia.x * vel.angular.x * vel.angular.x
                        + rb.local_inertia.y * vel.angular.y * vel.angular.y
                        + rb.local_inertia.z * vel.angular.z * vel.angular.z);

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
            let _ = world.step(1.0 / 60.0);
        }

        // Object should have fallen due to gravity
        assert!(world.transforms[0].position.y < 10.0);
    }

    #[test]
    fn test_high_stack_stability() {
        let mut world = PhysicsWorld::new();
        // Akademik doğrulama için iterasyon sayısını yüksek tutalım
        world.solver.iterations = 30;

        // Ground
        let mut ground_rb = RigidBody::default();
        ground_rb.body_type = crate::components::rigid_body::BodyType::Static;
        ground_rb.wake_up();
        world.add_body(
            Entity::new(0, 0),
            ground_rb,
            Transform::new(Vec3::new(0.0, -0.5, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(50.0, 0.5, 50.0)),
        );

        // 10 Kutuluk bir kule inşa et
        let box_count = 10;
        let box_size = 1.0;
        let half_size = box_size / 2.0;

        for i in 1..=box_count {
            let mut rb = RigidBody::new(1.0, 0.5, 0.5, true);
            rb.wake_up(); // Uyumasını engelle ki solver test edilsin
            
            let y_pos = half_size + (i - 1) as f32 * box_size;
            
            world.add_body(
                Entity::new(i, 0),
                rb,
                Transform::new(Vec3::new(0.0, y_pos, 0.0)),
                Velocity::default(),
                Collider::box_collider(Vec3::new(half_size, half_size, half_size)),
            );
        }

        // 10 saniye (600 kare) simüle et
        for i in 0..600 {
            let _ = world.step(1.0 / 60.0);
            if i % 60 == 0 {
                println!("Frame {}: Y={}, X={}, Z={}", i, world.transforms[10].position.y, world.transforms[10].position.x, world.transforms[10].position.z);
            }
        }

        // Kule yıkılmamış olmalı (X ve Z ekseninde çok kaymamış olmalı)
        // En üstteki kutunun durumuna bakalım
        let top_box_idx = box_count as usize; // Entity ID starts from 1 for boxes, so idx is `box_count` because ground is 0
        let top_box_pos = world.transforms[top_box_idx].position;

        // Akademik limitler: 10 saniye boyunca dik durmalı, yana yatmamalı
        assert!(
            top_box_pos.x.abs() < 0.1,
            "Top box slid too much on X axis: {}",
            top_box_pos.x
        );
        assert!(
            top_box_pos.z.abs() < 0.1,
            "Top box slid too much on Z axis: {}",
            top_box_pos.z
        );

        // Yüksekliği korunmalı (Jitter / Penetrasyon testi)
        let expected_y = half_size + (box_count - 1) as f32 * box_size;
        let y_error = (top_box_pos.y - expected_y).abs();
        assert!(
            y_error < 0.1,
            "Top box sunk or bounced too much. Expected Y: {}, Actual Y: {}",
            expected_y,
            top_box_pos.y
        );
    }

    #[test]
    fn test_ccd_tunneling_prevention() {
        let mut world = PhysicsWorld::new();
        // Gravity kapalı ki tam düz uçsun
        world.integrator.gravity = Vec3::ZERO;

        // İnce bir duvar (kalınlık 0.2m)
        let mut wall_rb = RigidBody::new_static();
        wall_rb.wake_up();
        world.add_body(
            Entity::new(0, 0),
            wall_rb,
            Transform::new(Vec3::new(0.0, 0.0, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(0.1, 5.0, 5.0)),
        );

        // Mermi (CCD Açık)
        let mut bullet_rb = RigidBody::new(1.0, 0.0, 0.0, false);
        bullet_rb.ccd_enabled = true;
        bullet_rb.wake_up();

        // Mermiyi duvarın önünden, saniyede 1200 metre hızla (mach 3.5) ateşle!
        // 1 kare (1/60) saniyede 20 metre yol alır. Duvar sadece 0.2m kalınlığında!
        world.add_body(
            Entity::new(1, 0),
            bullet_rb,
            Transform::new(Vec3::new(-5.0, 0.0, 0.0)),
            Velocity::new(Vec3::new(1200.0, 0.0, 0.0)),
            Collider::sphere(0.2),
        );

        // 1 Frame simüle et (Tunneling ihtimali olan an)
        let _ = world.step(1.0 / 60.0);

        let bullet_pos = world.transforms[1].position;
        let bullet_vel = world.velocities[1].linear;

        // CCD çalıştığı için mermi duvarı GEÇMEMELİ!
        // Eğer CCD çalışmasaydı, X konumu 15.0 civarında olurdu.
        // CCD çalıştığı için duvarda durmalı (X <= 0) veya sekip eksi hıza geçmeli.
        assert!(
            bullet_pos.x <= 0.0,
            "TUNNELING FAILED! Bullet phased through the wall. Position: {}",
            bullet_pos.x
        );
        // Hız durdurulmuş veya sekmiş olmalı
        assert!(
            bullet_vel.x <= 0.01,
            "Bullet did not lose velocity after hitting the wall. Vel: {}",
            bullet_vel.x
        );
    }

    #[test]
    fn test_coulomb_friction_and_sleeping() {
        let mut world = PhysicsWorld::new();
        // Sürtünme için yerçekimi şart (Normal kuvveti yaratmak için)
        world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);

        // Zemin (Sürtünme: 0.5)
        let mut ground_rb = RigidBody::new_static();
        ground_rb.friction = 0.5;
        ground_rb.wake_up();
        world.add_body(
            Entity::new(0, 0),
            ground_rb,
            Transform::new(Vec3::new(0.0, -0.5, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(100.0, 0.5, 100.0)),
        );

        // Kutu A: Düşük Sürtünme (0.1)
        let mut box_a = RigidBody::new(1.0, 0.0, 0.1, true);
        box_a.wake_up();
        world.add_body(
            Entity::new(1, 0),
            box_a,
            Transform::new(Vec3::new(0.0, 0.5, -2.0)),
            Velocity::new(Vec3::new(10.0, 0.0, 0.0)),
            Collider::box_collider(Vec3::splat(0.5)),
        );

        // Kutu B: Yüksek Sürtünme (0.8)
        let mut box_b = RigidBody::new(1.0, 0.0, 0.8, true);
        box_b.wake_up();
        world.add_body(
            Entity::new(2, 0),
            box_b,
            Transform::new(Vec3::new(0.0, 0.5, 2.0)),
            Velocity::new(Vec3::new(10.0, 0.0, 0.0)),
            Collider::box_collider(Vec3::splat(0.5)),
        );

        // 5 saniye simüle et (300 kare) - İkisi de tamamen durup uyumalı
        for _ in 0..300 {
            let _ = world.step(1.0 / 60.0);
        }

        let pos_a = world.transforms[1].position;
        let pos_b = world.transforms[2].position;
        let sleep_a = world.rigid_bodies[1].is_sleeping;
        let sleep_b = world.rigid_bodies[2].is_sleeping;

        // Yüksek sürtünmeli kutu daha erken durmuş olmalı (Daha az X mesafesi)
        assert!(
            pos_b.x < pos_a.x,
            "High friction box should travel less. Pos A: {}, Pos B: {}",
            pos_a.x,
            pos_b.x
        );

        // İkisi de kinetik enerjisini sıfırlayıp UYKU MODUNA geçmiş olmalı
        assert!(
            sleep_a,
            "Low friction box did not enter sleeping mode!"
        );
        assert!(
            sleep_b,
            "High friction box did not enter sleeping mode!"
        );
    }

    #[test]
    fn test_car_simulation() {
        use crate::joints::data::{Joint, JointData, HingeJointData};

        let mut world = PhysicsWorld::new();
        // Yerçekimi açık (Sürtünme ve ağırlık için gerekli)
        world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);

        // --- Zemin ---
        let mut ground_rb = RigidBody::new_static();
        ground_rb.friction = 0.8;
        ground_rb.wake_up();
        world.add_body(
            Entity::new(0, 0),
            ground_rb,
            Transform::new(Vec3::new(0.0, -0.5, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(100.0, 0.5, 100.0)),
        );

        // --- Şasi (Chassis) ---
        // 1000 kg, sürtünme önemsiz, dinamik
        let mut chassis_rb = RigidBody::new(1000.0, 0.1, 0.5, true);
        chassis_rb.wake_up();
        let chassis_col = Collider::box_collider(Vec3::new(1.0, 0.5, 2.0)); // Genişlik 2, Yükseklik 1, Uzunluk 4 (Yarıçaplar)
        chassis_rb.update_inertia_from_collider(&chassis_col);
        let chassis_entity = Entity::new(1, 0);
        let chassis_pos = Vec3::new(0.0, 1.5, 0.0);
        world.add_body(
            chassis_entity,
            chassis_rb,
            Transform::new(chassis_pos),
            Velocity::default(),
            chassis_col,
        );

        // Tekerlek Şablonu
        let wheel_radius = 0.5;
        let mut wheel_rb = RigidBody::new(50.0, 0.1, 0.9, true); // Yüksek kütle (50kg) ve yüksek sürtünme (0.9)
        wheel_rb.wake_up();
        let wheel_col = Collider::sphere(wheel_radius);
        wheel_rb.update_inertia_from_collider(&wheel_col);

        let wheel_offsets = vec![
            Vec3::new(-1.2, -0.2, 1.5),  // Sol Ön
            Vec3::new(1.2, -0.2, 1.5),   // Sağ Ön
            Vec3::new(-1.2, -0.2, -1.5), // Sol Arka
            Vec3::new(1.2, -0.2, -1.5),  // Sağ Arka
        ];

        let mut wheel_entities = Vec::new();

        for (i, offset) in wheel_offsets.iter().enumerate() {
            let wheel_entity = Entity::new(2 + i as u32, 0);
            wheel_entities.push(wheel_entity);

            world.add_body(
                wheel_entity,
                wheel_rb,
                Transform::new(chassis_pos + *offset),
                Velocity::default(),
                wheel_col.clone(),
            );

            // Menteşe Eklemi (Hinge Joint) oluştur
            let is_rear = i >= 2;
            let hinge_data = HingeJointData {
                axis: Vec3::X, // Tekerlekler X ekseni etrafında dönecek
                use_limits: false,
                lower_limit: 0.0,
                upper_limit: 0.0,
                use_motor: is_rear, // Sadece arka tekerleklerde motor var
                motor_target_velocity: if is_rear { 10.0 } else { 0.0 }, // İleri doğru 10 rad/s
                motor_max_force: if is_rear { 10000.0 } else { 0.0 }, // 10000 N güç
                current_angle: 0.0,
            };

            let joint = Joint {
                entity_a: chassis_entity,
                entity_b: wheel_entity,
                local_anchor_a: *offset, // Şasinin lokal uzayında bağlantı noktası
                local_anchor_b: Vec3::ZERO, // Tekerleğin tam ortası
                break_force: f32::MAX, // Asla kopmasın
                break_torque: f32::MAX,
                is_broken: false,
                collision_enabled: false, // Şasi ile tekerlek çarpışmasın
                data: JointData::Hinge(hinge_data),
            };

            world.joints.push(joint);
        }
        // --- Simülasyon ---
        // Motorlar çalışacak ve arabayı 5 saniye boyunca (300 kare) ileri doğru (Z+) sürecek
        for _ in 0..300 {
            let _ = world.step(1.0 / 60.0);
        }
        // Doğrulama
        let final_chassis_pos = world.transforms[1].position;
        
        // 1. İleri Sürüş: Araba Z ekseninde (ileri) hareket etmiş olmalı
        assert!(
            final_chassis_pos.z > 3.0,
            "Araba yeterince ileri gidemedi! Motor veya sürtünme çalışmıyor. Z pozisyonu: {}",
            final_chassis_pos.z
        );

        // 2. Denge (Devrilmeme): Arabanın Y pozisyonu stabil kalmalı (uçmamalı veya batmamalı)
        // Başlangıç Y: 1.5, Tekerlek yarıçapı 0.5. Araba yere oturunca Y ~1.0 - 1.2 civarı olmalı
        assert!(
            final_chassis_pos.y > 0.5 && final_chassis_pos.y < 2.0,
            "Araba devrildi, uçtu veya yere battı! Y pozisyonu: {}",
            final_chassis_pos.y
        );

        // 3. X Ekseninde Düz Gitme (Sağa sola savrulmama)
        assert!(
            final_chassis_pos.x.abs() < 1.0,
            "Araba düz gidemedi, sağa sola savruldu! X pozisyonu: {}",
            final_chassis_pos.x
        );
    }

    /// 2-tangent sürtünme, eksen-hizalı olmayan (diyagonal) bir kaymayı her iki
    /// tangent bileşeninde simetrik yavaşlatıp durdurmalı. Eski tek-tangent yöntemi
    /// birikmiş impulsun dik bileşenini kaybedebiliyordu.
    #[test]
    fn friction_decelerates_diagonal_slide_symmetrically() {
        let mut world = PhysicsWorld::new();
        world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);

        let mut ground = RigidBody::new_static();
        ground.friction = 0.9;
        ground.wake_up();
        world.add_body(
            Entity::new(0, 0),
            ground,
            Transform::new(Vec3::new(0.0, -0.5, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(100.0, 0.5, 100.0)),
        );

        // Diyagonal kayan kutu (vx = vz); dönmeyi kilitle → saf öteleme sürtünmesi.
        let mut rb = RigidBody::new(1.0, 0.0, 0.9, true);
        rb.lock_rotation_x = true;
        rb.lock_rotation_y = true;
        rb.lock_rotation_z = true;
        rb.wake_up();
        let col = Collider::box_collider(Vec3::new(0.5, 0.5, 0.5));
        rb.update_inertia_from_collider(&col);
        world.add_body(
            Entity::new(1, 0),
            rb,
            Transform::new(Vec3::new(0.0, 0.5, 0.0)),
            Velocity::new(Vec3::new(3.0, 0.0, 3.0)),
            col,
        );

        for _ in 0..10 {
            world.step(1.0 / 60.0).unwrap();
        }
        let v_mid = world.velocities[1].linear;
        let speed_mid = (v_mid.x * v_mid.x + v_mid.z * v_mid.z).sqrt();

        for _ in 0..150 {
            world.step(1.0 / 60.0).unwrap();
        }
        let v_end = world.velocities[1].linear;
        let speed_end = (v_end.x * v_end.x + v_end.z * v_end.z).sqrt();

        // Simetri: x ve z bileşenleri yakın kalmalı (dik bileşen kaybolmaz).
        assert!(
            (v_mid.x - v_mid.z).abs() < 0.2,
            "diyagonal simetri bozuldu: vx={} vz={}",
            v_mid.x,
            v_mid.z
        );
        // Sürtünme belirgin yavaşlatıp neredeyse durdurmalı.
        assert!(speed_end < speed_mid, "yavaşlamadı: {speed_mid} -> {speed_end}");
        assert!(speed_end < 0.5, "durmaya yakın olmalı, kalan hız: {speed_end}");
    }

    /// Hareket eden kinematik platform, üstündeki UYUYAN dinamik cismi uyandırmalı ve
    /// sürtünmeyle sürüklemeli. (Eskiden kinematik gövde "mover" sayılmadığından ada
    /// uyanmıyor, uyuyan cisim hiç uyandırılmıyordu.)
    #[test]
    fn moving_kinematic_platform_wakes_sleeping_body() {
        let mut world = PhysicsWorld::new();
        world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);

        // Kinematik platform: merkez 0, üst yüz +0.5.
        let mut plat = RigidBody::new_kinematic();
        plat.friction = 1.0;
        world.add_body(
            Entity::new(0, 0),
            plat,
            Transform::new(Vec3::new(0.0, 0.0, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(5.0, 0.5, 5.0)),
        );

        // Üstünde dinamik kutu: merkez 1.0, alt 0.5 = platform üstü.
        let mut box_rb = RigidBody::new(1.0, 0.0, 1.0, true);
        box_rb.lock_rotation_x = true;
        box_rb.lock_rotation_y = true;
        box_rb.lock_rotation_z = true;
        box_rb.wake_up();
        let col = Collider::box_collider(Vec3::new(0.5, 0.5, 0.5));
        box_rb.update_inertia_from_collider(&col);
        world.add_body(
            Entity::new(1, 0),
            box_rb,
            Transform::new(Vec3::new(0.0, 1.0, 0.0)),
            Velocity::default(),
            col,
        );

        // Platform sabitken kutuyu uyut.
        for _ in 0..400 {
            world.step(1.0 / 60.0).unwrap();
        }
        assert!(
            world.rigid_bodies[1].is_sleeping,
            "kutu önce uyumalı (uyumadıysa senaryo geçersiz)"
        );
        let x_before = world.transforms[1].position.x;

        // Platformu +x yönünde hareket ettir.
        world.velocities[0].linear = Vec3::new(2.0, 0.0, 0.0);
        for _ in 0..30 {
            world.step(1.0 / 60.0).unwrap();
        }

        assert!(
            !world.rigid_bodies[1].is_sleeping,
            "hareket eden kinematik platform kutuyu uyandırmalı"
        );
        let x_after = world.transforms[1].position.x;
        assert!(
            x_after > x_before + 0.05,
            "kutu platformla sürüklenmeli: {x_before} -> {x_after}"
        );
    }
}
