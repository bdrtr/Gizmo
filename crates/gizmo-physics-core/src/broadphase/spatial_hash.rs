use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// SpatialHash uyumluluk katmanı
// ─────────────────────────────────────────────────────────────────────────────

/// Eski SpatialHash API'sini Dynamic BVH üzerine köprüler.
/// FIX-4: clear artık &mut self alıyor.
pub struct SpatialHash {
    tree: DynamicAabbTree,
}

impl Default for SpatialHash {
    fn default() -> Self {
        Self::new(10.0)
    }
}

impl SpatialHash {
    pub fn new(_cell_size: f32) -> Self {
        Self {
            tree: DynamicAabbTree::new(),
        }
    }

    /// FIX-4: &self yerine &mut self
    pub fn clear(&mut self) {
        self.tree.clear();
    }

    pub fn insert(&mut self, entity: BodyHandle, aabb: Aabb) {
        self.tree.insert(entity, aabb);
    }

    pub fn update(&mut self, entity: BodyHandle, aabb: Aabb) {
        self.tree.insert(entity, aabb);
    }

    pub fn remove(&mut self, entity: BodyHandle) {
        self.tree.remove(entity);
    }

    pub fn query_pairs(&self) -> Vec<(BodyHandle, BodyHandle)> {
        self.tree.query_pairs()
    }

    pub fn query_aabb(&self, aabb: Aabb) -> Vec<BodyHandle> {
        self.tree.query_aabb(&aabb)
    }

    pub fn query_point(&self, point: Vec3, radius: f32) -> Vec<BodyHandle> {
        let aabb = Aabb {
            min: Vec3::new(point.x - radius, point.y - radius, point.z - radius).into(),
            max: Vec3::new(point.x + radius, point.y + radius, point.z + radius).into(),
        };
        self.tree.query_aabb(&aabb)
    }

    pub fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Vec<(BodyHandle, f32)> {
        self.tree.query_ray(origin, dir, max_t)
    }
}

