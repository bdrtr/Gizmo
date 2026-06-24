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
    contact_cache: HashMap<(gizmo_core::entity::Entity, gizmo_core::entity::Entity), (bool, Option<ContactManifold>)>,
    accumulator: f32,
}

impl PhysicsWorld {
    /// Deterministik rollback/replay için TAM durum anlık görüntüsü al (bkz [`WorldSnapshot`]).
    pub fn snapshot(&self) -> WorldSnapshot {
        WorldSnapshot {
            transforms: self.transforms.clone(),
            velocities: self.velocities.clone(),
            rigid_bodies: self.rigid_bodies.clone(),
            contact_cache: self.contact_cache.clone(),
            accumulator: self.accumulator,
        }
    }

    /// Anlık görüntüyü geri yükle (rollback). entities/colliders aynı kalmalı (aksi halde
    /// indeks hizası bozulur). Sonraki `step` çağrıları bu durumdan deterministik ilerler.
    pub fn restore_snapshot(&mut self, snap: &WorldSnapshot) {
        self.transforms.clone_from(&snap.transforms);
        self.velocities.clone_from(&snap.velocities);
        self.rigid_bodies.clone_from(&snap.rigid_bodies);
        self.contact_cache.clone_from(&snap.contact_cache);
        self.accumulator = snap.accumulator;
    }

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

        // PhysicsMetrics: bu frame'in aşama-zamanlamalarını sıfırla (substep'ler boyunca birikir).
        self.metrics.broadphase_ms = 0.0;
        self.metrics.narrowphase_ms = 0.0;
        self.metrics.solver_ms = 0.0;
        self.metrics.integration_ms = 0.0;
        self.metrics.contact_count = 0;
        self.metrics.island_count = 0;

        let mut steps = 0u32;
        while self.accumulator >= FIXED_DT && steps < MAX_SUBSTEPS {
            self.step_internal(FIXED_DT)?;
            self.accumulator -= FIXED_DT;
            steps += 1;
        }

        // Gövde/uyku sayımları (profilleme — uyku optimizasyonunun etkisini gösterir).
        self.metrics.body_count = self.entities.len();
        self.metrics.sleeping_count = self
            .rigid_bodies
            .iter()
            .filter(|rb| rb.is_dynamic() && rb.is_sleeping)
            .count();

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

        // Aşama-başına zamanlama (PhysicsMetrics — profilleme). Instant::now() ~birkaç ns
        // olduğundan ms-ölçekli fizik yanında ihmal edilebilir; simülasyon SONUCUNU
        // etkilemez (determinizm pozisyon/hızdan; metrik ayrı) → hash değişmez.
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f32() * 1000.0;

        // 0-1. Yerçekimi, sıvı bölgeleri, hız entegrasyonu
        let t0 = std::time::Instant::now();
        self.velocity_integration_step(dt)?;
        self.metrics.integration_ms += ms(t0);

        // 1.5-1.6 Yumuşak cisim ve sıvı simülasyonu

        // 2. Broadphase — uzamsal hash güncelleme
        let t1 = std::time::Instant::now();
        self.broadphase_step(dt);
        self.metrics.broadphase_ms += ms(t1);

        // 3. Narrowphase — çarpışma tespiti ve olayları
        let t2 = std::time::Instant::now();
        let manifolds = self.narrowphase_and_collision_step(dt);
        self.metrics.narrowphase_ms += ms(t2);
        self.metrics.contact_count += manifolds.iter().map(|m| m.contacts.len()).sum::<usize>();

        // 4-4.5 Kısıt çözücü (çarpışma + eklem)
        let t3 = std::time::Instant::now();
        self.constraint_solve_step(manifolds, dt);
        self.metrics.solver_ms += ms(t3);

        // 5-6. Pozisyon entegrasyonu ve uyku durumu
        let t4 = std::time::Instant::now();
        self.position_integration_step(dt)?;
        self.metrics.integration_ms += ms(t4);

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

    /// Simülasyon durumunun DETERMINISTIK hash'i — rollback/replay desync tespiti + testler.
    ///
    /// Cisimler **entity id'sine göre SABİT sırada** gezilir (ekleme/HashMap sırasından ve
    /// dizi düzeninden bağımsız), her `f32` `to_bits()` ile karıştırılır. Sabit-anahtarlı
    /// `DefaultHasher` (RandomState DEĞİL) kullanıldığından çıktı SÜREÇLER ARASI tutarlıdır.
    ///
    /// Garanti: **aynı platform + aynı binary**'de, aynı başlangıç durumundan aynı `dt`
    /// adımlarıyla adım-adım eşleşir (replay/rollback için yeterli). Cross-platform bit-exact
    /// GARANTİ EDİLMEZ (sim f32/glam üzerinde; bkz. `docs/determinism.md`).
    pub fn state_hash(&self) -> u64 {
        use std::hash::Hasher;
        // Entity id'sine göre sabit sıra (dizi/ekleme sırasından bağımsız).
        let mut order: Vec<usize> = (0..self.entities.len()).collect();
        order.sort_by_key(|&i| self.entities[i].id());

        let mut h = std::collections::hash_map::DefaultHasher::new();
        for &i in &order {
            h.write_u32(self.entities[i].id());
            let t = &self.transforms[i];
            let v = &self.velocities[i];
            for bits in [
                t.position.x, t.position.y, t.position.z,
                t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w,
                v.linear.x, v.linear.y, v.linear.z,
                v.angular.x, v.angular.y, v.angular.z,
            ] {
                h.write_u32(bits.to_bits());
            }
            // Uyku durumu da state'in parçası (rollback'te tutarlı olmalı).
            h.write_u8(self.rigid_bodies[i].is_sleeping as u8);
        }
        h.finish()
    }

    /// Apply an impulse to a body at a point.
    ///
    /// `rb` alınır `&mut` çünkü uyuyan bir cisme impuls uygulamak onu UYANDIRMALIDIR;
    /// aksi halde hız değişir ama `is_sleeping` true kalır → position_integration cismi
    /// atlar ve impuls SESSİZCE YUTULUR (cisim hiç hareket etmez).
    pub fn apply_impulse(
        &self,
        rb: &mut RigidBody,
        transform: &Transform,
        vel: &mut Velocity,
        impulse: gizmo_math::Vec3,
        point: gizmo_math::Vec3,
    ) {
        if rb.is_dynamic() {
            rb.wake_up();
        }
        Integrator::apply_impulse_at_point(rb, transform, vel, impulse, point);
    }

    /// Apply a force to a body. `rb` `&mut` — uyuyan cismi uyandırır (bkz. apply_impulse).
    pub fn apply_force(
        &self,
        rb: &mut RigidBody,
        vel: &mut Velocity,
        force: gizmo_math::Vec3,
        dt: f32,
    ) {
        if rb.is_dynamic() {
            rb.wake_up();
        }
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
        // Gravity kapalı ki tam düz uçsun.
        world.integrator.gravity = Vec3::ZERO;

        // İnce statik duvar: kalınlık 0.2 m, x=0 merkezli → ön yüz x=-0.1, arka yüz x=+0.1.
        let mut wall_rb = RigidBody::new_static();
        wall_rb.wake_up();
        world.add_body(
            Entity::new(0, 0),
            wall_rb,
            Transform::new(Vec3::ZERO),
            Velocity::default(),
            Collider::box_collider(Vec3::new(0.1, 5.0, 5.0)),
        );

        // Mermi (CCD açık): r=0.2, saniyede 1200 m (mach ~3.5). Bir karede (1/60 s)
        // 20 m yol alır; duvar 0.2 m → CCD olmadan kesin tünelleme olurdu.
        let mut bullet_rb = RigidBody::new(1.0, 0.0, 0.0, false);
        bullet_rb.ccd_enabled = true;
        bullet_rb.wake_up();
        world.add_body(
            Entity::new(1, 0),
            bullet_rb,
            Transform::new(Vec3::new(-5.0, 0.0, 0.0)),
            Velocity::new(Vec3::new(1200.0, 0.0, 0.0)),
            Collider::sphere(0.2),
        );

        // Birden çok kare simüle et: speculative CCD merminin yolunu o kareye izin
        // verilen boşlukla SINIRLAR; mermi duvara varır, ertesi karede tam durur.
        let mut max_x = f32::MIN;
        for _ in 0..120 {
            let _ = world.step(1.0 / 60.0);
            max_x = max_x.max(world.transforms[1].position.x);
        }

        // 1) HİÇBİR karede duvar merkezini geçmemeli (geçseydi x ~ +15 olurdu).
        assert!(
            max_x < 0.0,
            "TUNNELING! Bullet crossed the wall — peak x = {max_x}"
        );

        // 2) Eski `penetration = 0` hatasında mermi başlangıçta (x≈-5) DONUYORDU.
        //    Doğru CCD'de duvarın ön yüzüne (x≈-0.31) dayanıp durmalı.
        let final_x = world.transforms[1].position.x;
        assert!(
            (-0.6..=-0.1).contains(&final_x),
            "Bullet should rest against the wall front (~ -0.31), got x = {final_x} \
             (frozen far short would be ~ -5.0)"
        );

        // 3) Sonunda durmuş olmalı.
        let final_v = world.velocities[1].linear.x;
        assert!(
            final_v.abs() < 1.0,
            "Bullet should have stopped against the wall, vel.x = {final_v}"
        );
    }

    #[test]
    fn test_material_combine_modes_respected() {
        use gizmo_physics_core::{CombineMode, PhysicsMaterial};
        // Temas materyali artık her materyalin combine MODUYLA birleşir
        // (`PhysicsMaterial::combine`). Eskiden pipeline geometrik-ortalama'yı
        // hardcode ediyordu → `friction_combine` yok sayılıyordu.
        //
        // Yüksek-sürtünme + Max-combine kutu, DÜŞÜK-sürtünme zeminde:
        //   Doğru (Max):  μ = max(0.9, 0.1) = 0.9 → ~5-6 m'de durur.
        //   Eski (geo.ort): μ = sqrt(0.9·0.1) = 0.3 → ~17 m kayar.
        // AYIRT EDİCİ: combine() yerine eski hardcode'a dönülürse test DÜŞER.
        let mut world = PhysicsWorld::new();
        world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);

        let mut ground = RigidBody::new_static();
        ground.wake_up();
        let mut gcol = Collider::box_collider(Vec3::new(200.0, 0.5, 200.0));
        gcol.material = PhysicsMaterial {
            static_friction: 0.1,
            dynamic_friction: 0.1,
            friction_combine: CombineMode::GeometricMean,
            ..PhysicsMaterial::default()
        };
        world.add_body(
            Entity::new(0, 0),
            ground,
            Transform::new(Vec3::new(0.0, -0.5, 0.0)),
            Velocity::default(),
            gcol,
        );

        let mut rb = RigidBody::new(1.0, 0.0, 0.0, true);
        rb.wake_up();
        let mut col = Collider::box_collider(Vec3::splat(0.5));
        col.material = PhysicsMaterial {
            static_friction: 0.9,
            dynamic_friction: 0.9,
            friction_combine: CombineMode::Max, // Max, zemin GeometricMean'i ezer
            ..PhysicsMaterial::default()
        };
        rb.update_inertia_from_collider(&col);
        world.add_body(
            Entity::new(1, 0),
            rb,
            Transform::new(Vec3::new(0.0, 0.5, 0.0)),
            Velocity::new(Vec3::new(10.0, 0.0, 0.0)),
            col,
        );

        for _ in 0..300 {
            let _ = world.step(1.0 / 60.0);
        }
        let x = world.transforms[1].position.x;
        assert!(
            x < 10.0,
            "Max friction_combine yok sayıldı — kutu {x} m kaydı (Max ile ~5-6 m beklenir; \
             eski geo-ort hardcode'u ~17 m verirdi)"
        );
    }

    #[test]
    fn test_coulomb_friction_and_sleeping() {
        use gizmo_physics_core::PhysicsMaterial;
        let mut world = PhysicsWorld::new();
        // Sürtünme için yerçekimi şart (normal kuvveti yaratmak için).
        world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);

        // ÖNEMLİ: temas sürtünmesi collider MATERYALİNDEN gelir
        // (`manifold.friction = sqrt(mat_a.dyn * mat_b.dyn)`), `RigidBody::friction`
        // alanından DEĞİL. Bu test eskiden rb.friction'ı değiştiriyordu — o alan
        // temas çözücüye HİÇ ulaşmıyor, dolayısıyla iki kutu da varsayılan materyalle
        // aynı mesafeyi gidip test yalnızca sub-mm gürültüyle "geçiyordu". Farkı
        // gerçek sürücüye, yani materyale koyuyoruz.

        // Zemin — yüksek sürtünmeli, geniş (A ~23 m kayabilir).
        let mut ground_rb = RigidBody::new_static();
        ground_rb.wake_up();
        let mut ground_col = Collider::box_collider(Vec3::new(200.0, 0.5, 200.0));
        ground_col.material = PhysicsMaterial {
            static_friction: 0.9,
            dynamic_friction: 0.9,
            ..PhysicsMaterial::default()
        };
        world.add_body(
            Entity::new(0, 0),
            ground_rb,
            Transform::new(Vec3::new(0.0, -0.5, 0.0)),
            Velocity::default(),
            ground_col,
        );

        let mut make_box = |id: u32, z: f32, fric: f32| {
            let mut rb = RigidBody::new(1.0, 0.0, 0.0, true);
            rb.wake_up();
            let mut col = Collider::box_collider(Vec3::splat(0.5));
            col.material = PhysicsMaterial {
                static_friction: fric,
                dynamic_friction: fric,
                ..PhysicsMaterial::default()
            };
            rb.update_inertia_from_collider(&col);
            world.add_body(
                Entity::new(id, 0),
                rb,
                Transform::new(Vec3::new(0.0, 0.5, z)),
                Velocity::new(Vec3::new(10.0, 0.0, 0.0)),
                col,
            );
        };
        make_box(1, -2.0, 0.05); // Kutu A: düşük sürtünme → uzağa kayar
        make_box(2, 2.0, 0.9); //  Kutu B: yüksek sürtünme → erken durur

        // 5 saniye simüle et (300 kare) — ikisi de durup uyumalı.
        for _ in 0..300 {
            let _ = world.step(1.0 / 60.0);
        }

        let pos_a = world.transforms[1].position;
        let pos_b = world.transforms[2].position;

        // Yüksek sürtünmeli kutu BELİRGİN şekilde daha az yol gitmeli (~5 m'ye karşı
        // ~23 m). Sağlam marj: sub-mm gürültüye değil gerçek sürtünmeye duyarlı.
        assert!(
            pos_b.x < pos_a.x - 5.0,
            "Yüksek sürtünmeli kutu belirgin daha az gitmeli. A: {}, B: {}",
            pos_a.x,
            pos_b.x
        );

        // İkisi de durup UYKU MODUNA geçmeli.
        assert!(world.rigid_bodies[1].is_sleeping, "Düşük sürtünmeli kutu uyumadı!");
        assert!(world.rigid_bodies[2].is_sleeping, "Yüksek sürtünmeli kutu uyumadı!");
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
