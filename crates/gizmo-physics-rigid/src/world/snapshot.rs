use super::{PhysicsWorld, SnapshotError, WorldSnapshot};

use std::path::PathBuf;

impl PhysicsWorld {
    /// Deterministik rollback/replay için TAM durum anlık görüntüsü al (bkz [`WorldSnapshot`]).
    pub fn snapshot(&self) -> WorldSnapshot {
        WorldSnapshot {
            transforms: self.transforms.clone(),
            velocities: self.velocities.clone(),
            rigid_bodies: self.rigid_bodies.clone(),
            contact_cache: self.contact_cache.clone(),
            accumulator: self.accumulator,
            gravity_fields: self.gravity_fields.clone(),
            fluid_zones: self.fluid_zones.clone(),
            joints: self.joints.clone(),
            weather: self.weather,
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
        self.gravity_fields.clone_from(&snap.gravity_fields);
        self.fluid_zones.clone_from(&snap.fluid_zones);
        self.joints.clone_from(&snap.joints);
        self.weather = snap.weather;
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
        // The λ slots are NOT carried state — `solve_joints` zeroes them at the start of every
        // pass — so hashing them buys something weaker but still useful: a tripwire. They hold
        // the last substep's accumulated impulses, so two runs that have already diverged in
        // the solver but not yet in the integrated velocities are distinguished here, one
        // substep earlier than they otherwise would be. If the warm start ever lands (see
        // docs/FIXPLAN.md, B4 commit 5), λ becomes carried state and this stops being optional.
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
