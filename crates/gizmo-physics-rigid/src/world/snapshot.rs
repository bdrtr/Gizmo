use super::{PhysicsWorld, SnapshotError, WorldSnapshot};

use std::path::PathBuf;

impl PhysicsWorld {
    /// Take a COMPLETE state snapshot for deterministic rollback/replay (see [`WorldSnapshot`]).
    ///
    /// # Why this destructures instead of reading fields
    ///
    /// The pair below used to be two hand-written nine-line lists against a **28-field** struct,
    /// and every field in [`WorldSnapshot`] carries a comment explaining the divergence that
    /// earned it a place — gravity fields, joints and weather were each added *after* a
    /// resimulation ran under state the continuous run no longer had. That is the shape of the
    /// problem: leaving a field out is not an error, just a rollback that restores less than it
    /// claims, and the symptom appears somewhere else entirely.
    ///
    /// So both directions destructure exhaustively, with **no `..` arm**. Adding a field to
    /// `PhysicsWorld` (or to `WorldSnapshot`) now fails to compile *here*, which forces the one
    /// question nobody was being asked: is this carried simulation state, or is it not — and why?
    /// The `_`-bound names below are that answer, written down.
    pub fn snapshot(&self) -> WorldSnapshot {
        let PhysicsWorld {
            // ── Carried state: cannot be rederived from what is kept, so it must travel ──
            transforms,
            velocities,
            rigid_bodies,
            contact_cache,
            accumulator,
            // Carried, though chosen when the world is built: the accumulator, the warm-start
            // impulses and the sleep counters above are in its units.
            substep_rate,
            gravity_fields,
            fluid_zones,
            joints,
            weather,

            // ── Configuration. The caller's settings, not the simulation's state: restoring
            //    them would silently undo a change made since the snapshot was taken.
            integrator: _,
            solver: _,
            joint_solver: _,
            max_history_frames: _,
            watchlist: _,

            // ── Derived, and rebuilt before it is read again. The broadphase tree is refreshed
            //    for every body at the first substep of each step (`settled_bounds` is emptied
            //    there, and only skips within a step); a tree still shaped by the rolled-back future is
            //    only a different pair *order*, which the island solve is invariant to
            //    (`support_ordering`). The index map is a function of `entities`, which a
            //    restore requires to be unchanged anyway.
            spatial_hash: _,
            settled_bounds: _,
            body_bounds: _,
            body_revision: _,
            moved_asleep: _,
            stepped_gravity: _,
            #[cfg(feature = "ecs")]
            ecs_sync: _,
            entity_index_map: _,
            ecs_generations: _,
            departed_contacts: _,

            // ── Output of the last step. Nothing in the pipeline reads these back, and `step`
            //    clears and refills the event lists.
            collision_events: _,
            trigger_events: _,
            fracture_events: _,
            metrics: _,
            render_alpha: _,
            bodies_without_collider: _,
            // Written and taken inside one substep: empty between steps.
            joint_poses: _,

            // ── Structure. A restore is index-aligned and therefore *requires* these to be
            //    identical already; carrying them would hide a caller error rather than fix it.
            entities: _,
            colliders: _,

            // ── The caller's control flags, the rollback machinery itself (snapshotting the
            //    history inside a snapshot), and a preloaded asset cache.
            is_paused: _,
            step_once: _,
            persisting_contact_events: _,
            rewind_requested: _,
            history: _,
            fracture_cache: _,
        } = self;

        WorldSnapshot {
            transforms: transforms.clone(),
            velocities: velocities.clone(),
            rigid_bodies: rigid_bodies.clone(),
            contact_cache: contact_cache.clone(),
            accumulator: *accumulator,
            substep_rate: *substep_rate,
            gravity_fields: gravity_fields.clone(),
            fluid_zones: fluid_zones.clone(),
            joints: joints.clone(),
            weather: *weather,
        }
    }

    /// Restore a snapshot (rollback). `entities`/`colliders` must be unchanged, or the index
    /// alignment breaks. Subsequent `step` calls advance deterministically from this state.
    ///
    /// Destructures the snapshot exhaustively for the reason given on [`Self::snapshot`]: a field
    /// added to [`WorldSnapshot`] and not restored is a snapshot that carries state nothing puts
    /// back, which is the same silence in the other direction.
    pub fn restore_snapshot(&mut self, snap: &WorldSnapshot) {
        let WorldSnapshot {
            transforms,
            velocities,
            rigid_bodies,
            contact_cache,
            accumulator,
            substep_rate,
            gravity_fields,
            fluid_zones,
            joints,
            weather,
        } = snap;

        // The rows change under the ECS bridge: it reads everything again next step.
        self.body_revision += 1;
        self.transforms.clone_from(transforms);
        self.velocities.clone_from(velocities);
        self.rigid_bodies.clone_from(rigid_bodies);
        self.contact_cache.clone_from(contact_cache);
        self.accumulator = *accumulator;
        self.substep_rate = *substep_rate;
        self.gravity_fields.clone_from(gravity_fields);
        self.fluid_zones.clone_from(fluid_zones);
        self.joints.clone_from(joints);
        self.weather = *weather;
    }

    /// A DETERMINISTIC hash of the simulation state — desync detection for rollback/replay, and
    /// tests.
    ///
    /// Bodies are walked in a **fixed order, by entity id** (independent of insertion order,
    /// HashMap order and array layout), and every `f32` is mixed in through `to_bits()`. It uses
    /// a fixed-key `DefaultHasher` (NOT RandomState), so the output is consistent ACROSS
    /// PROCESSES.
    ///
    /// The substep rate is mixed in first, so two worlds stepping at different rates — two peers
    /// of a rollback session built differently — never hash alike, even before they have moved. So
    /// is what a step reads and no pose shows: the time not yet stepped, the weather (a vehicle's
    /// grip), each body's sleep counter and gathered forces, and what each joint's solve left in it
    /// (its reference pose, a hinge's angle, a slider's travel), and the contacts the last solve
    /// left: their warm-start impulses, whether they slid, which pairs have touched.
    ///
    /// The guarantee: on the **same platform and the same binary**, stepping the same initial
    /// state with the same `dt` matches step for step (which is what replay/rollback needs).
    /// Cross-platform bit-exactness is NOT guaranteed (the sim runs on f32/glam; see
    /// `docs/ENGINE.md §5`).
    pub fn state_hash(&self) -> u64 {
        use std::hash::Hasher;
        // BodyHandle id'sine göre sabit sıra (dizi/ekleme sırasından bağımsız).
        let mut order: Vec<usize> = (0..self.entities.len()).collect();
        order.sort_by_key(|&i| self.entities[i].id());

        let mut h = std::collections::hash_map::DefaultHasher::new();
        h.write_u32(self.substep_rate);
        h.write_u32(self.accumulator.to_bits());
        h.write_u8(self.weather as u8);
        for &i in &order {
            h.write_u32(self.entities[i].id());
            let t = &self.transforms[i];
            let v = &self.velocities[i];
            let rb = &self.rigid_bodies[i];
            for bits in [
                t.position.x, t.position.y, t.position.z,
                t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w,
                v.linear.x, v.linear.y, v.linear.z,
                v.angular.x, v.angular.y, v.angular.z,
                rb.force_accumulator.x, rb.force_accumulator.y, rb.force_accumulator.z,
                rb.torque_accumulator.x, rb.torque_accumulator.y, rb.torque_accumulator.z,
            ] {
                h.write_u32(bits.to_bits());
            }
            // Uyku durumu da state'in parçası (rollback'te tutarlı olmalı).
            h.write_u8(rb.is_sleeping as u8);
            h.write_u32(rb.sleep_counter);
        }

        self.contact_cache.hash_into(&mut h);

        // Joint solver state. `is_broken`, the endpoint pair and what `JointData::take_state`
        // takes are CARRIED state: a broken joint or a reference pose cannot be recomputed from
        // transforms and velocities, and a restore that dropped it would resimulate with a joint
        // the continuous run no longer had.
        //
        // The λ slots are not carried state at the DEFAULT `JointSolver::warm_start_factor`
        // of 0 — `solve_joints` zeroes them at the start of every pass — so hashing them buys
        // something weaker but still useful: a tripwire. They hold the last substep's
        // accumulated impulses, so two runs that have already diverged in the solver but not
        // yet in the integrated velocities are distinguished here, one substep earlier than
        // they otherwise would be.
        //
        // With a NON-ZERO warm start factor they become genuinely carried state, and this
        // block already covers it exactly: the next pass warm starts from `rows`, which is
        // what is hashed here. (`JointScratch::prev_rows` is deliberately NOT hashed — between
        // steps it holds the pass BEFORE last, which nothing will ever read again.) The
        // restore side needs nothing either: `snapshot()` clones `joints` whole.
        //
        // Walked in ARRAY order, deliberately: `solve_joints`' answer already depends on the
        // order of the joint slice (Gauss–Seidel), and `restore_snapshot` restores that order,
        // so array order is the canonical one here in a way it is not for bodies.
        for j in &self.joints {
            h.write_u32(j.entity_a.id());
            h.write_u32(j.entity_b.id());
            h.write_u8(j.is_broken as u8);
            let (travel, pose) = j.data.clone().take_state();
            h.write_u32(travel.to_bits());
            h.write_u8(pose.is_some() as u8);
            for bits in pose.unwrap_or_default().to_array() {
                h.write_u32(bits.to_bits());
            }
            for s in 0..crate::joints::data::JointScratch::LEN {
                h.write_u32(j.scratch.row_value(s).to_bits());
            }
        }
        h.finish()
    }

    /// A hash of what decides how this world steps and is not its state: the integrator, the
    /// contact and joint solvers, the substep rate, the gravity fields and fluid zones, every
    /// joint's definition, and every body's rigid body and collider — mass, inertia, damping,
    /// shape, material. [`state_hash`](Self::state_hash) is where the bodies are and how they
    /// move; this is what they are and the rules they move by.
    ///
    /// Bodies and joints are visited in the order the world holds them, so the order they were
    /// added in counts: a solve visits them in it, and two rollback peers must agree on it. What a
    /// step changes — a body's sleep, the forces gathered on it, what its contacts gave it, a joint
    /// broken, its solver's row or what its solve keeps (an angle, a reference pose) — is left out,
    /// and so is what only the picture reads (a fluid zone's fog).
    ///
    /// **Why it exists.** The solvers' and the integrator's settings are in neither a rollback
    /// snapshot nor the world's serialized state, and the solver's documentation leaves keeping
    /// them in sync to the caller. Two rollback peers that set up their worlds a little
    /// differently — a gravity, a solver's iteration count, a body's mass — started from equal
    /// states and drifted apart with nothing to say why; `gizmo-net`'s session opening compares
    /// this.
    ///
    /// Each value is hashed by its `Debug` form, streamed into the hasher, so a field added to one
    /// of them is covered without a list to keep; none holds a map or a pointer, whose form would
    /// differ from run to run. The same build gives the same hash everywhere, another build may
    /// not. It costs a formatting of every collider's geometry — something to ask once, at a
    /// session's start, not every step.
    pub fn settings_hash(&self) -> u64 {
        use std::fmt::Write as _;
        use std::hash::Hasher as _;
        /// `fmt::Write` into a hasher: a `Debug` form hashed as it is written, never built.
        struct Into<'a>(&'a mut std::collections::hash_map::DefaultHasher);
        impl std::fmt::Write for Into<'_> {
            fn write_str(&mut self, s: &str) -> std::fmt::Result {
                self.0.write(s.as_bytes());
                Ok(())
            }
        }
        // Exhaustive, as `snapshot` is and for its reason: a field added to the world has to say
        // whether it decides how the world steps.
        let PhysicsWorld {
            // ── Settings ──
            integrator,
            solver,
            joint_solver,
            substep_rate,
            gravity_fields,
            fluid_zones,
            joints,
            entities,
            rigid_bodies,
            colliders,

            // ── State: `state_hash`'s, or carried in a snapshot. ──
            transforms: _,
            velocities: _,
            contact_cache: _,
            accumulator: _,
            weather: _,

            // ── Nothing a step reads to decide where a body goes: derived structures, the last
            //    step's output, control flags, history and an asset cache. ──
            max_history_frames: _,
            watchlist: _,
            spatial_hash: _,
            settled_bounds: _,
            body_bounds: _,
            body_revision: _,
            moved_asleep: _,
            stepped_gravity: _,
            #[cfg(feature = "ecs")]
            ecs_sync: _,
            entity_index_map: _,
            ecs_generations: _,
            departed_contacts: _,
            collision_events: _,
            trigger_events: _,
            fracture_events: _,
            metrics: _,
            render_alpha: _,
            bodies_without_collider: _,
            joint_poses: _,
            is_paused: _,
            step_once: _,
            persisting_contact_events: _,
            rewind_requested: _,
            history: _,
            fracture_cache: _,
        } = self;

        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut out = Into(&mut h);
        // Every `write!` below goes to a hasher, which takes anything.
        let _ = write!(out, "{substep_rate}{integrator:?}{solver:?}{joint_solver:?}{gravity_fields:?}");
        for zone in fluid_zones {
            let super::FluidZone { shape, density, viscosity, linear_drag, quadratic_drag, fog_color: _, fog_density: _ } = zone;
            let _ = write!(out, "{shape:?}{density:?}{viscosity:?}{linear_drag:?}{quadratic_drag:?}|");
        }
        for ((entity, body), collider) in entities.iter().zip(rigid_bodies).zip(colliders) {
            let mut body = *body;
            body.is_sleeping = false;
            body.sleep_counter = 0;
            body.force_accumulator = gizmo_math::Vec3::ZERO;
            body.torque_accumulator = gizmo_math::Vec3::ZERO;
            body.contact_response = gizmo_math::Vec3::ZERO;
            body.pose_driven = false;
            let _ = write!(out, "{}{body:?}{collider:?}", entity.id());
        }
        for j in joints {
            let crate::joints::Joint {
                entity_a,
                entity_b,
                local_anchor_a,
                local_anchor_b,
                break_force,
                break_torque,
                collision_enabled,
                data,
                // State.
                is_broken: _,
                scratch: _,
            } = j;
            let mut data = data.clone();
            data.take_state();
            let _ = write!(
                out,
                "{}{}{local_anchor_a:?}{local_anchor_b:?}{break_force:?}{break_torque:?}{collision_enabled}{data:?}",
                entity_a.id(),
                entity_b.id(),
            );
        }
        h.finish()
    }

    /// Telemetry and Debugging: Create a JSON snapshot of the physical world state.
    ///
    /// Writes a `physics_snapshot_<timestamp>.json` file to the current working
    /// directory and returns the path it was written to. Any I/O or
    /// serialization failure is surfaced as a [`SnapshotError`] instead of being
    /// silently logged, so callers can react to (or escalate) the failure.
    pub fn trigger_snapshot(&self, reason: &str) -> Result<PathBuf, SnapshotError> {
        tracing::error!("Creating physics snapshot due to: {}", reason);
        // unwrap_or_default: a clock set before UNIX_EPOCH (or WASM quirks)
        // must not panic during a diagnostic snapshot; fall back to ts=0.
        #[cfg(target_arch = "wasm32")]
        let timestamp = web_time::SystemTime::now()
            .duration_since(web_time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        #[cfg(not(target_arch = "wasm32"))]
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let path = PathBuf::from(format!("physics_snapshot_{}.json", timestamp));

        let file = std::fs::File::create(&path).map_err(|source| SnapshotError::Create {
            path: path.clone(),
            source,
        })?;
        serde_json::to_writer_pretty(file, self)?;
        tracing::info!("Physics snapshot successfully saved to {}", path.display());
        Ok(path)
    }
}
