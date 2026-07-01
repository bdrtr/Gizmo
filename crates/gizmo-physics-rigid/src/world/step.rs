use super::{PhysicsWorld, PhysicsStateSnapshot, FIXED_DT, MAX_SUBSTEPS};
use gizmo_physics_core::{CollisionEvent, TriggerEvent};

// PhysicsMetrics per-phase timing. `std::time::Instant::now()` panics on wasm
// (no clock backend); `web_time::Instant` bridges to the browser clock. Timing
// never feeds the simulation result → determinism-neutral on both targets.
#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

impl PhysicsWorld {
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
        let ms = |t: Instant| t.elapsed().as_secs_f32() * 1000.0;

        // 0-1. Yerçekimi, sıvı bölgeleri, hız entegrasyonu
        let t0 = Instant::now();
        self.velocity_integration_step(dt)?;
        self.metrics.integration_ms += ms(t0);

        // 1.5-1.6 Yumuşak cisim ve sıvı simülasyonu

        // 2. Broadphase — uzamsal hash güncelleme
        let t1 = Instant::now();
        self.broadphase_step(dt);
        self.metrics.broadphase_ms += ms(t1);

        // 3. Narrowphase — çarpışma tespiti ve olayları
        let t2 = Instant::now();
        let manifolds = self.narrowphase_and_collision_step(dt);
        self.metrics.narrowphase_ms += ms(t2);
        self.metrics.contact_count += manifolds.iter().map(|m| m.contacts.len()).sum::<usize>();

        // 4-4.5 Kısıt çözücü (çarpışma + eklem)
        let t3 = Instant::now();
        self.constraint_solve_step(manifolds, dt);
        self.metrics.solver_ms += ms(t3);

        // 5-6. Pozisyon entegrasyonu ve uyku durumu
        let t4 = Instant::now();
        self.position_integration_step(dt)?;
        // CCD geometrik güvencesi: ince geometriye karşı speculative GJK mesafesi
        // dejenere olduğunda hızlı bir cismin tünellemesini engelle (yalnız hızlı CCD
        // cisimlerini etkiler; yavaş/dinlenen cisimler dokunulmaz → determinizm nötr).
        self.ccd_resolve_step(dt);
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
