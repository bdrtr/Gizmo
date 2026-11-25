use super::{PhysicsWorld, PhysicsStateSnapshot, MAX_FRAME_DT};
use gizmo_physics_core::{CollisionEvent, TriggerEvent};

// PhysicsMetrics per-phase timing. `std::time::Instant::now()` panics on wasm
// (no clock backend); `web_time::Instant` bridges to the browser clock. Timing
// never feeds the simulation result → determinism-neutral on both targets.
#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

impl PhysicsWorld {
    /// Advances the world by `dt` seconds of frame time, in whole substeps of
    /// [`substep_dt`](Self::substep_dt): what is left over waits in the accumulator for the next
    /// call. A `dt` that is a whole number of substeps runs exactly that many.
    #[tracing::instrument(skip_all, name = "physics_frame")]
    pub fn step(
        &mut self,


        dt: f32,
    ) -> Result<(), gizmo_physics_core::GizmoError> {
        self.moved_asleep.clear();
        // A change of gravity wakes every sleeping dynamic body: one asleep on the floor under one
        // gravity is not at rest under another. Until 2026-11-23 a game that turned gravity over
        // left its sleepers lying on the floor, still asleep, until something touched them.
        let gravity = self.integrator.gravity;
        if self.stepped_gravity.is_some_and(|was| was != gravity) {
            for rb in &mut self.rigid_bodies {
                if rb.is_dynamic() && rb.is_sleeping {
                    rb.wake_up();
                }
            }
        }
        self.stepped_gravity = Some(gravity);
        if self.rewind_requested {
            self.rewind_requested = false;
            if let Some(snapshot) = self.history.pop_back() {
                if snapshot.transforms.len() == self.transforms.len() {
                    // Rows changed outside a step's own work: see `body_revision`.
                    self.body_revision += 1;
                    self.transforms = snapshot.transforms;
                    self.velocities = snapshot.velocities;
                    tracing::info!(body_count = self.transforms.len(), "Physics rewound by 1 frame");
                } else {
                    // A body was spawned/despawned between snapshot and rewind, so the SoA
                    // arrays no longer line up — restoring would corrupt the index mapping.
                    tracing::warn!(
                        stored = snapshot.transforms.len(),
                        current = self.transforms.len(),
                        "Cannot rewind: entity count changed since snapshot"
                    );
                }
            } else {
                tracing::debug!("Rewind requested but history buffer is empty");
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

        // What contacts give a kinematic body with a contact mass is this call's (`contact_response`).
        for rb in &mut self.rigid_bodies {
            rb.contact_response = gizmo_math::Vec3::ZERO;
        }

        // --- STEP ONCE (DEBUG) ---
        let frame_dt = if self.step_once {
            self.step_once = false;
            self.accumulator = 0.0; // Reset accumulator so we step exactly once
            self.substep_dt()
        } else {
            dt.min(MAX_FRAME_DT) // the spiral of death: a frame takes in at most this much
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
        self.metrics.solver_sweeps = 0;
        self.metrics.max_island_depth = 0;
        // Alt-faz sayaçları da aynı pencerede: adım başında sıfırlanır, alt-adımlar
        // boyunca birikir, adım sonunda okunur.
        crate::profile::PHASES.reset();

        // A force written into `RigidBody::force_accumulator` or `torque_accumulator` WAKES the
        // body it is written to. Every other way of pushing a body already does this —
        // `PhysicsWorld::apply_impulse` and `apply_force` take `&mut RigidBody` for exactly this
        // reason, and the explosion system wakes what it moves — but the accumulators are public
        // fields, so nothing stood between a game and the one path that did not.
        //
        // What that cost, measured (2026-08-18): a 1 kg box settled on a plate, then given 50 N
        // sideways for two seconds through `force_accumulator`, moved **0.0000 m**. The same box
        // with `wake_up()` called alongside the push moved **92.7 m**. Not less — nothing at all,
        // silently, for as long as the body stays asleep: a thruster, a wind volume or a conveyor
        // stops working the moment its target comes to rest, which is the moment it is most
        // obviously supposed to work.
        for rb in &mut self.rigid_bodies {
            if rb.is_dynamic()
                && rb.is_sleeping
                && (rb.force_accumulator != gizmo_math::Vec3::ZERO
                    || rb.torque_accumulator != gizmo_math::Vec3::ZERO)
            {
                rb.wake_up();
            }
        }

        // The first substep refreshes every body's broadphase bounds: whatever moved a body
        // between steps — the ECS sync, a restored snapshot, a write to the public arrays — is
        // caught there. See `settled_bounds`.
        self.settled_bounds.clear();

        let (substep, max_substeps) = (self.substep_dt(), self.max_substeps());
        // A substep runs once the accumulator holds all of one but `SUBSTEP_SNAP` of it, and a
        // balance within that of zero, either side, is zero. Without it a `dt` of a whole number of
        // substeps did not run as that many in f32: three substeps of a 300 Hz world, added and
        // drained, leave 0.99999 of one, so the call ran two and carried a whole substep into the
        // next (2, 4, 3, …) — and of all the pairings of a rate in `SUBSTEP_RATES` with a step of
        // up to 64 substeps, 39 % went uneven within 3000 calls. Measured in an f32 model of this
        // loop, 2026-11-24: with the snap, none did. Time is not gained or lost but by rounding
        // noise: the snap is a ten-thousandth of a substep, and a variable `dt` lands that close to
        // a whole substep about once in five thousand calls.
        let snap = substep * super::SUBSTEP_SNAP;
        let mut steps = 0u32;
        while self.accumulator >= substep - snap && steps < max_substeps {
            self.step_internal(substep)?;
            self.accumulator -= substep;
            steps += 1;
        }
        if self.accumulator.abs() < snap {
            self.accumulator = 0.0;
        }

        // Forces are drained ONCE PER FRAME, here, not once per substep.
        //
        // The accumulator holds a force, and a force acts continuously: every substep integrates
        // `F·substep_dt`, and the sum over the frame is `F·frame_dt`. Clearing inside the
        // substep loop instead — which is what the integrator used to do — delivered `F/steps`,
        // so the same push produced a quarter of the acceleration at 60 fps and an eighth at 30.
        //
        // Cleared even on a frame that ran ZERO substeps (a delta shorter than 1/240 s): the
        // force was asked for during that frame, and carrying it into the next one would make a
        // 1000 fps game apply each force four times over.
        for rb in &mut self.rigid_bodies {
            rb.clear_forces();
        }

        // Gövde/uyku sayımları (profilleme — uyku optimizasyonunun etkisini gösterir).
        self.metrics.body_count = self.entities.len();
        self.metrics.sleeping_count = self
            .rigid_bodies
            .iter()
            .filter(|rb| rb.is_dynamic() && rb.is_sleeping)
            .count();

        // Per-frame AGGREGATE: one debug line summarising the whole step — how many fixed
        // substeps ran, how much of the world is asleep, contact/island load and the phase
        // timings accumulated across substeps. Cheap (all counts already computed above),
        // and the single highest-signal line for "is the sim healthy this frame?".
        tracing::debug!(
            substeps = steps,
            frame_dt,
            accumulator = self.accumulator,
            body_count = self.metrics.body_count,
            sleeping_count = self.metrics.sleeping_count,
            island_count = self.metrics.island_count,
            contact_count = self.metrics.contact_count,
            solver_ms = self.metrics.solver_ms,
            total_ms = self.metrics.total_ms(),
            "physics frame stepped"
        );
        if steps == max_substeps {
            // Hit the substep ceiling: this frame stopped stepping with time still in the
            // accumulator, so the sim is behind wall-clock.
            //
            // **Nothing is lost, though** — the old text here said "time lost" in the same breath
            // as "the accumulator still holds unspent time", which cannot both be true. This
            // accumulator is never clamped: the leftover carries into the next frame. Nor does it
            // run away, and the two durations are why: intake is at most `MAX_FRAME_DT` (0.25 s)
            // a frame while a saturated frame drains `MAX_SIMULATED_PER_STEP` (0.267 s), so every
            // ceiling-hitting frame pays off 1/60 s of debt, at any rate. That is the opposite
            // arrangement from `PhysicsTime`, which clamps and genuinely drops time — see its docs
            // on which ceiling governs where.
            //
            // Through the ECS this branch is unreachable: `physics_step_system` is handed a fixed
            // step of a few substeps. It is for a game driving `step` itself with a large variable
            // delta.
            tracing::warn!(
                max_substeps,
                accumulator = self.accumulator,
                "Physics hit the substep ceiling — simulation is behind real time (the leftover \
                 carries to the next frame; it is not dropped)"
            );
        }

        // Alpha: render interpolasyonu için (0 = önceki adım, 1 = mevcut adım)
        // Alt-faz sayaçlarını metriklere aktar. Atomikler adalar arası paralel yazılıyor;
        // burada tek iş parçacığı okuyor, yani tutarlı bir kesit.
        {
            use crate::profile::{Phases, PHASES};
            self.metrics.solver_order_ms = Phases::ms(&PHASES.order);
            self.metrics.solver_prepare_ms = Phases::ms(&PHASES.prepare);
            self.metrics.solver_sweep_ms = Phases::ms(&PHASES.sweep);
            self.metrics.solver_relax_ms = Phases::ms(&PHASES.relax);
            self.metrics.narrowphase_dispatch_ms = Phases::ms(&PHASES.dispatch);
            self.metrics.narrowphase_manifold_ms = Phases::ms(&PHASES.manifold);
        }

        self.render_alpha = self.accumulator / substep;

        // Record history snapshot at the end of the frame.
        //
        // Skipped entirely at `max_history_frames == 0`. The push/pop pair below left the buffer
        // empty either way, so the two clones were pure waste — two full per-body arrays copied
        // every frame for a rewind depth of zero. Behaviour is unchanged: an empty history is an
        // empty history.
        if self.max_history_frames > 0 {
            self.history.push_back(PhysicsStateSnapshot {
                transforms: self.transforms.clone(),
                velocities: self.velocities.clone(),
            });
            if self.history.len() > self.max_history_frames {
                self.history.pop_front();
            }
        }

        Ok(())
    }

    /// The inner physics step — always called with the world's [`substep_dt`](Self::substep_dt).
    /// A modular pipeline: each stage lives in its own function (pipeline.rs).
    #[tracing::instrument(skip_all, name = "physics_substep")]
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
        let (manifolds, ends, pending_triggers) = self.narrowphase_and_collision_step(dt);
        self.metrics.narrowphase_ms += ms(t2);
        self.metrics.contact_count += manifolds.iter().map(|m| m.contacts.len()).sum::<usize>();

        // 4-4.5 Kısıt çözücü (çarpışma + eklem)
        let t3 = Instant::now();
        let bounces = self.constraint_solve_step(manifolds, ends, dt);
        self.metrics.solver_ms += ms(t3);
        // Triggers found by a speculative contact only, now that the substep's motion is known.
        self.confirm_triggers(pending_triggers, dt);

        // 5-6. Pozisyon entegrasyonu ve uyku durumu
        let t4 = Instant::now();
        let kinematic_starts = self.kinematic_ccd_starts(dt);
        self.position_integration_step(dt)?;
        // Substepped joints' bodies are where their own steps took them.
        for (i, pose) in std::mem::take(&mut self.joint_poses) {
            self.transforms[i] = pose;
        }
        // A fast kinematic CCD body is held at static surfaces, which no impulse could do.
        self.kinematic_ccd_step(kinematic_starts, dt);
        // Speculative contacts' bounces, now that the bodies have travelled to each other.
        self.apply_bounces(&bounces);
        self.metrics.integration_ms += ms(t4);

        // Energy Conservation Check: Validate energy bounds (Zero-cost in release mode)
        if cfg!(debug_assertions) {
            let _final_energy = self.calculate_total_energy();
        }

        Ok(())
    }

    /// The consecutive quiet substeps a body needs before it may sleep:
    /// [`RigidBody::SLEEP_TIME`](crate::RigidBody::SLEEP_TIME) of this world's substep.
    pub fn sleep_steps(&self) -> u32 {
        crate::RigidBody::sleep_steps(self.substep_dt())
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
