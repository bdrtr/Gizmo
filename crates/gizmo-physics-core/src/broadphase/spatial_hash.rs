use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// SpatialHash uyumluluk katmanı
// ─────────────────────────────────────────────────────────────────────────────

/// Eski SpatialHash API'sini Dynamic BVH üzerine köprüler.
/// FIX-4: clear artık &mut self alıyor.
///
/// A compatibility shim: the name is left over from a uniform-grid broadphase, but
/// every call forwards to a [`DynamicAabbTree`] and there are no cells any more —
/// the cell size a caller passes to [`new`](Self::new) is ignored.
///
/// Bodies are keyed by [`BodyHandle::id`], stored as **fattened** world-space AABBs
/// (0.1 m added on every face by default), and every query is answered against those
/// enlarged boxes. Results are therefore a conservative superset: pairs and hits are
/// reported for bodies whose true bounds may not overlap at all, and the narrowphase
/// or an exact [`Raycast`](crate::Raycast) test is expected to reject them.
pub struct SpatialHash {
    tree: DynamicAabbTree,
}

impl Default for SpatialHash {
    fn default() -> Self {
        Self::new(10.0)
    }
}

impl SpatialHash {
    /// Creates an empty broadphase.
    ///
    /// `_cell_size` is **ignored** — it survives from the grid implementation this
    /// type used to be, and the tree underneath has no cells to size. `Default`
    /// passes 10.0, equally to no effect. To change the one tuning knob that does
    /// exist (the fattening margin) build a [`DynamicAabbTree`] directly with
    /// [`DynamicAabbTree::with_fat_margin`].
    pub fn new(_cell_size: f32) -> Self {
        Self {
            tree: DynamicAabbTree::new(),
        }
    }

    /// FIX-4: &self yerine &mut self
    pub fn clear(&mut self) {
        self.tree.clear();
    }

    /// Adds `entity` with its tight world-space `aabb`, or refreshes it if the id is
    /// already present — insert and [`update`](Self::update) are the same operation.
    ///
    /// Forwards to [`DynamicAabbTree::insert`], including its early-out for a body
    /// that has not left the box already stored for it.
    pub fn insert(&mut self, entity: BodyHandle, aabb: Aabb) {
        self.tree.insert(entity, aabb);
    }

    /// Refreshes an entity's bounds. Identical to [`insert`](Self::insert) — it is
    /// the same call underneath, and it inserts entities that are not present yet.
    pub fn update(&mut self, entity: BodyHandle, aabb: Aabb) {
        self.tree.insert(entity, aabb);
    }

    /// Drops the entity from the broadphase. A no-op for an id that is not present.
    pub fn remove(&mut self, entity: BodyHandle) {
        self.tree.remove(entity);
    }

    /// Drops every entity whose id `keep` rejects, and reports how many went. Forwards to
    /// [`DynamicAabbTree::retain`] — see it for why a caller wants this.
    pub fn retain(&mut self, keep: impl Fn(u32) -> bool) -> usize {
        self.tree.retain(keep)
    }

    /// Number of entities currently in the broadphase.
    pub fn entity_count(&self) -> usize {
        self.tree.entity_count()
    }

    /// Candidate collision pairs, ordered by id (`.0.id() < .1.id()`) so a tuple
    /// works as a symmetric cache key. See [`DynamicAabbTree::query_pairs`] for the
    /// guarantees in full.
    pub fn query_pairs(&self) -> Vec<(BodyHandle, BodyHandle)> {
        self.tree.query_pairs()
    }

    /// Entities overlapping `aabb`, touching included. See
    /// [`DynamicAabbTree::query_aabb`].
    pub fn query_aabb(&self, aabb: Aabb) -> Vec<BodyHandle> {
        self.tree.query_aabb(&aabb)
    }

    /// Entities near `point`, within an axis-aligned **cube** of half-extent
    /// `radius` — not a sphere.
    ///
    /// Convenience wrapper over [`query_aabb`](Self::query_aabb) with the same
    /// conservative, unordered semantics; corners of the cube reach about `1.7 *
    /// radius` from the point, so filter by true distance if that matters.
    pub fn query_point(&self, point: Vec3, radius: f32) -> Vec<BodyHandle> {
        let aabb = Aabb {
            min: Vec3::new(point.x - radius, point.y - radius, point.z - radius).into(),
            max: Vec3::new(point.x + radius, point.y + radius, point.z + radius).into(),
        };
        self.tree.query_aabb(&aabb)
    }

    /// Ray candidates, nearest first, each with the parameter at which the ray
    /// enters the stored box. The exact per-shape test that follows is
    /// [`Raycast::ray_shape`](crate::Raycast::ray_shape).
    ///
    /// See [`DynamicAabbTree::query_ray`] for what the `t` values mean, how they are
    /// scaled, and the degenerate-direction caveats.
    pub fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Vec<(BodyHandle, f32)> {
        self.tree.query_ray(origin, dir, max_t)
    }
}

