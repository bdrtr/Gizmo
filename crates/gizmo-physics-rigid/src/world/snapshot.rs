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
