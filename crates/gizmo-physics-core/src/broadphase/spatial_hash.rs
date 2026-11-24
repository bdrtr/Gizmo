use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// SpatialHash uyumluluk katmanı
// ─────────────────────────────────────────────────────────────────────────────

/// Bridges the old SpatialHash API onto the Dynamic BVH.
/// FIX-4: clear now takes &mut self.
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
///
/// **Two trees.** Bodies that never move — static ones, put in with
/// [`insert_static`](Self::insert_static) — live in a tree of their own, which is never walked
/// against itself: two of them can never start touching, and the narrowphase skipped every such
/// pair anyway. [`query_pairs`](Self::query_pairs) is the moving tree against itself and against
/// the static one. With 14 600 static bodies and nothing moving, walking them against each other
/// four times a step was a quarter of the step. A body is in one tree at a time: putting it in
/// one takes it out of the other.
pub struct SpatialHash {
    /// Bodies that may move: dynamic (awake or asleep) and kinematic.
    tree: DynamicAabbTree,
    /// Static bodies.
    fixed: DynamicAabbTree,
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
            fixed: DynamicAabbTree::new(),
        }
    }

    /// FIX-4: &mut self instead of &self
    pub fn clear(&mut self) {
        self.tree.clear();
        self.fixed.clear();
    }

    /// Adds `entity` with its tight world-space `aabb`, or refreshes it if the id is
    /// already present — insert and [`update`](Self::update) are the same operation.
    ///
    /// Forwards to [`DynamicAabbTree::insert`], including its early-out for a body
    /// that has not left the box already stored for it.
    pub fn insert(&mut self, entity: BodyHandle, aabb: Aabb) {
        self.insert_moving(entity, aabb, gizmo_math::Vec3::ZERO);
    }

    /// [`insert`](Self::insert) for a body that moved by `displacement` this step: forwards to
    /// [`DynamicAabbTree::insert_moving`], whose leaf reaches ahead along the motion.
    pub fn insert_moving(&mut self, entity: BodyHandle, aabb: Aabb, displacement: gizmo_math::Vec3) {
        if self.tree.holds(entity, &aabb) {
            return;
        }
        self.fixed.remove(entity);
        self.tree.insert_moving(entity, aabb, displacement);
    }

    /// [`insert`](Self::insert) for a body that never moves: it goes in the static tree, which
    /// is only ever walked against the moving one (see the type's documentation). Taken out of
    /// the moving tree if it was there — a body made static.
    pub fn insert_static(&mut self, entity: BodyHandle, aabb: Aabb) {
        if self.fixed.holds(entity, &aabb) {
            return;
        }
        self.tree.remove(entity);
        self.fixed.insert(entity, aabb);
    }

    /// Whether the broadphase already stores a box for `entity` that contains `aabb`, so that
    /// [`insert`](Self::insert) would change nothing. Forwards to [`DynamicAabbTree::holds`].
    pub fn holds(&self, entity: BodyHandle, aabb: &Aabb) -> bool {
        self.tree.holds(entity, aabb) || self.fixed.holds(entity, aabb)
    }

    /// Whether [`insert`](Self::insert) and [`insert_moving`](Self::insert_moving) would change
    /// nothing for `entity` with `aabb`: the moving tree already stores a box for it that contains
    /// `aabb`. Only the entity's own leaf is read, which no other entity's insertion or removal
    /// changes, so a caller may ask this for many entities at once — on several threads — and then
    /// insert only those it said no for, in any order.
    pub fn holds_moving(&self, entity: BodyHandle, aabb: &Aabb) -> bool {
        self.tree.holds(entity, aabb)
    }

    /// [`holds_moving`](Self::holds_moving) for [`insert_static`](Self::insert_static): the static
    /// tree's box.
    pub fn holds_static(&self, entity: BodyHandle, aabb: &Aabb) -> bool {
        self.fixed.holds(entity, aabb)
    }

    /// Refreshes an entity's bounds. Identical to [`insert`](Self::insert) — it is
    /// the same call underneath, and it inserts entities that are not present yet.
    pub fn update(&mut self, entity: BodyHandle, aabb: Aabb) {
        self.insert(entity, aabb);
    }

    /// Drops the entity from the broadphase. A no-op for an id that is not present.
    pub fn remove(&mut self, entity: BodyHandle) {
        self.tree.remove(entity);
        self.fixed.remove(entity);
    }

    /// Drops every entity whose id `keep` rejects, and reports how many went. Forwards to
    /// [`DynamicAabbTree::retain`] — see it for why a caller wants this.
    pub fn retain(&mut self, keep: impl Fn(u32) -> bool) -> usize {
        self.tree.retain(&keep) + self.fixed.retain(&keep)
    }

    /// Number of entities currently in the broadphase.
    pub fn entity_count(&self) -> usize {
        self.tree.entity_count() + self.fixed.entity_count()
    }

    /// Candidate collision pairs, ordered by id (`.0.id() < .1.id()`) so a tuple
    /// works as a symmetric cache key. See [`DynamicAabbTree::query_pairs`] for the
    /// guarantees in full. Two static bodies are never a pair.
    pub fn query_pairs(&self) -> Vec<(BodyHandle, BodyHandle)> {
        let mut pairs = self.tree.query_pairs();
        self.tree.query_pairs_with(&self.fixed, &mut pairs);
        pairs
    }

    /// Entities overlapping `aabb`, touching included. See
    /// [`DynamicAabbTree::query_aabb`].
    pub fn query_aabb(&self, aabb: Aabb) -> Vec<BodyHandle> {
        let mut found = self.tree.query_aabb(&aabb);
        found.extend(self.fixed.query_aabb(&aabb));
        found
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
        self.query_aabb(aabb)
    }

    /// Ray candidates, nearest first, each with the parameter at which the ray
    /// enters the stored box. The exact per-shape test that follows is
    /// [`Raycast::ray_shape`](crate::Raycast::ray_shape).
    ///
    /// See [`DynamicAabbTree::query_ray`] for what the `t` values mean, how they are
    /// scaled, and the degenerate-direction caveats.
    pub fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Vec<(BodyHandle, f32)> {
        let mut hits = self.tree.query_ray(origin, dir, max_t);
        hits.extend(self.fixed.query_ray(origin, dir, max_t));
        hits.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        hits
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec3;

    fn cube(x: f32, half: f32) -> Aabb {
        Aabb::from_center_half_extents(Vec3::new(x, 0.0, 0.0), Vec3::splat(half))
    }

    /// `holds_moving` and `holds_static` answer for their own tree, and a body's answer is not
    /// changed by other bodies going in and out — which is what lets the broad phase ask them on
    /// every thread before inserting on one.
    #[test]
    fn whether_a_box_is_held_does_not_depend_on_the_other_bodies() {
        let mut hash = SpatialHash::new(1.0);
        let (mover, wall) = (BodyHandle::from_id(1), BodyHandle::from_id(2));
        hash.insert_moving(mover, cube(0.0, 0.5), Vec3::new(0.1, 0.0, 0.0));
        hash.insert_static(wall, cube(3.0, 0.5));

        assert!(hash.holds_moving(mover, &cube(0.0, 0.5)));
        assert!(!hash.holds_moving(mover, &cube(2.0, 0.5)), "a box well outside the leaf is not held");
        assert!(!hash.holds_static(mover, &cube(0.0, 0.5)), "the mover is not in the static tree");
        assert!(hash.holds_static(wall, &cube(3.0, 0.5)));
        assert!(!hash.holds_moving(wall, &cube(3.0, 0.5)), "the wall is not in the moving tree");

        let asked: Vec<bool> = [0.0, 0.3, 0.6, 2.0].iter().map(|&x| hash.holds_moving(mover, &cube(x, 0.5))).collect();
        for i in 10..60 {
            hash.insert_moving(BodyHandle::from_id(i), cube(i as f32 * 0.37 - 9.0, 0.4), Vec3::new(0.0, 0.2, 0.0));
        }
        for i in (10..60).step_by(3) {
            hash.remove(BodyHandle::from_id(i));
        }
        for i in 10..30 {
            hash.insert_moving(BodyHandle::from_id(i), cube(i as f32 * -0.41 + 4.0, 0.4), Vec3::ZERO);
        }
        let again: Vec<bool> = [0.0, 0.3, 0.6, 2.0].iter().map(|&x| hash.holds_moving(mover, &cube(x, 0.5))).collect();
        assert_eq!(asked, again);
        assert!(hash.holds_static(wall, &cube(3.0, 0.5)));
    }
}
