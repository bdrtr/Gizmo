use super::{PhysicsWorld, SnapshotError, WorldSnapshot};

use std::path::PathBuf;

impl PhysicsWorld {
    /// Deterministik rollback/replay için TAM durum anlık görüntüsü al (bkz [`WorldSnapshot`]).
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
            //    for every body each substep; a tree still shaped by the rolled-back future is
            //    only a different pair *order*, which the island solve is invariant to
            //    (`support_ordering`). The index map is a function of `entities`, which a
            //    restore requires to be unchanged anyway.
            spatial_hash: _,
            entity_index_map: _,

            // ── Output of the last step. Nothing in the pipeline reads these back, and `step`
            //    clears and refills the event lists.
            collision_events: _,
            trigger_events: _,
            fracture_events: _,
            metrics: _,
            render_alpha: _,

            // ── Structure. A restore is index-aligned and therefore *requires* these to be
            //    identical already; carrying them would hide a caller error rather than fix it.
            entities: _,
            colliders: _,

            // ── The caller's control flags, the rollback machinery itself (snapshotting the
            //    history inside a snapshot), and a preloaded asset cache.
            is_paused: _,
            step_once: _,
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
            gravity_fields: gravity_fields.clone(),
            fluid_zones: fluid_zones.clone(),
            joints: joints.clone(),
            weather: *weather,
        }
    }

    /// Anlık görüntüyü geri yükle (rollback). entities/colliders aynı kalmalı (aksi halde
    /// indeks hizası bozulur). Sonraki `step` çağrıları bu durumdan deterministik ilerler.
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
            gravity_fields,
            fluid_zones,
            joints,
            weather,
        } = snap;

        self.transforms.clone_from(transforms);
        self.velocities.clone_from(velocities);
        self.rigid_bodies.clone_from(rigid_bodies);
        self.contact_cache.clone_from(contact_cache);
        self.accumulator = *accumulator;
        self.gravity_fields.clone_from(gravity_fields);
        self.fluid_zones.clone_from(fluid_zones);
        self.joints.clone_from(joints);
        self.weather = *weather;
    }

    /// Simülasyon durumunun DETERMINISTIK hash'i — rollback/replay desync tespiti + testler.
    ///
    /// Cisimler **entity id'sine göre SABİT sırada** gezilir (ekleme/HashMap sırasından ve
    /// dizi düzeninden bağımsız), her `f32` `to_bits()` ile karıştırılır. Sabit-anahtarlı
    /// `DefaultHasher` (RandomState DEĞİL) kullanıldığından çıktı SÜREÇLER ARASI tutarlıdır.
    ///
    /// Garanti: **aynı platform + aynı binary**'de, aynı başlangıç durumundan aynı `dt`
    /// adımlarıyla adım-adım eşleşir (replay/rollback için yeterli). Cross-platform bit-exact
    /// GARANTİ EDİLMEZ (sim f32/glam üzerinde; bkz. `docs/ENGINE.md §5`).
    pub fn state_hash(&self) -> u64 {
        use std::hash::Hasher;
        // BodyHandle id'sine göre sabit sıra (dizi/ekleme sırasından bağımsız).
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

        // Joint solver state. `is_broken` and the endpoint pair are CARRIED state: a broken
        // joint cannot be recomputed from transforms and velocities, and a restore that
        // dropped it would resimulate with a joint the continuous run no longer had.
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
            for s in 0..crate::joints::data::JointScratch::LEN {
                h.write_u32(j.scratch.row_value(s).to_bits());
            }
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
