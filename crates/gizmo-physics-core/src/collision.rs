use gizmo_core::entity::Entity;
use gizmo_math::Vec3;

// ============================================================================
//  ContactPoint
// ============================================================================

/// A single contact point between two colliding bodies.
///
/// `normal` always points **from body A toward body B** (the separating
/// direction for body A).  Both `local_point_*` fields are populated by the
/// dispatcher for warm-starting the constraint solver across frames.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct ContactPoint {
    /// World-space contact position (midpoint on the contact surface).
    pub point: Vec3,
    /// Contact normal, pointing from A to B (unit vector).
    pub normal: Vec3,
    /// Penetration depth (always ≥ 0).
    pub penetration: f32,
    /// `point` expressed in body A's local space (set by the dispatcher).
    pub local_point_a: Vec3,
    /// `point` expressed in body B's local space (set by the dispatcher).
    pub local_point_b: Vec3,
    /// Accumulated normal impulse — reused for warm-starting.
    pub normal_impulse: f32,
    /// Accumulated tangential impulse — reused for warm-starting.
    pub tangent_impulse: Vec3,
}

// ============================================================================
//  ContactManifold
// ============================================================================

/// Up to four contact points between a pair of bodies, along with the
/// combined material properties needed by the constraint solver.
///
/// # Contact limit & point selection
///
/// Physics engines conventionally cap manifolds at **4 points** because that
/// is the minimum required to fully constrain a convex face-face contact.
/// When a 5th point would be added we keep the configuration that maximises
/// the contact area while retaining the deepest point:
///
/// 1. Always keep the deepest point (most important for penetration resolution).
/// 2. Fill the remaining 3 slots by greedily maximising the minimum distance
///    to any already-selected point (farthest-point heuristic — O(n) per slot).
///
/// This gives a good approximation of the convex hull of the contact patch
/// without an expensive full hull computation.
#[derive(Debug, Clone)]
pub struct ContactManifold {
    pub entity_a: Entity,
    pub entity_b: Entity,
    /// At most 4 contact points.
    pub contacts: Vec<ContactPoint>,
    /// Combined dynamic friction coefficient (geometric mean of both materials).
    pub friction: f32,
    /// Combined static friction coefficient.
    pub static_friction: f32,
    /// Combined coefficient of restitution (max of both materials).
    pub restitution: f32,
    /// Number of consecutive physics frames this manifold has been alive.
    /// Incremented by the pipeline each frame; reset when the collision ends.
    pub lifetime: u32,
}

impl ContactManifold {
    /// Create a new manifold.  Entity order is normalised (lower id → entity_a)
    /// so that cache lookups with either ordering always hit.
    pub fn new(entity_a: Entity, entity_b: Entity) -> Self {
        let (entity_a, entity_b) = if entity_a.id() <= entity_b.id() {
            (entity_a, entity_b)
        } else {
            (entity_b, entity_a)
        };
        Self {
            entity_a,
            entity_b,
            contacts: Vec::with_capacity(4),
            // Sensible defaults; overwritten by the pipeline using
            // PhysicsMaterial::combine before the solver runs.
            friction: 0.5,
            static_friction: 0.5,
            restitution: 0.3,
            lifetime: 0,
        }
    }

    /// Add `contact` to the manifold, warm-starting from any existing point
    /// that is within `MERGE_RADIUS` in world space.
    ///
    /// If the manifold is already at capacity (4 points) and no merge occurs,
    /// the 5-point set is reduced back to 4 using the area-maximisation
    /// heuristic described in the type-level docs.
    pub fn add_contact(&mut self, contact: ContactPoint) {
        const MERGE_RADIUS_SQ: f32 = 0.02 * 0.02;

        // ── Warm-start merge ─────────────────────────────────────────────
        for existing in &mut self.contacts {
            if (existing.point - contact.point).length_squared() < MERGE_RADIUS_SQ {
                // Update geometry but preserve accumulated impulses.
                let saved_normal = existing.normal_impulse;
                let saved_tangent = existing.tangent_impulse;
                *existing = contact;
                existing.normal_impulse = saved_normal;
                existing.tangent_impulse = saved_tangent;
                return;
            }
        }

        // ── Fast path: still room ────────────────────────────────────────
        if self.contacts.len() < 4 {
            self.contacts.push(contact);
            return;
        }

        // ── Reduce 5 → 4 with area-maximisation heuristic ────────────────
        // Build a temporary 5-element array on the stack.
        let mut pool = [ContactPoint::default(); 5];
        pool[..4].copy_from_slice(&self.contacts);
        pool[4] = contact;

        self.contacts.clear();
        self.contacts.extend_from_slice(&select_4_contacts(&pool));
    }

    /// Remove all contact points (does **not** reset `lifetime`).
    pub fn clear(&mut self) {
        self.contacts.clear();
    }

    /// Returns `true` if the manifold has not been refreshed within
    /// `max_lifetime` frames — i.e. the collision pair has separated.
    pub fn is_stale(&self, max_lifetime: u32) -> bool {
        self.lifetime > max_lifetime
    }
}

// ============================================================================
//  4-point selection
// ============================================================================

/// Reduce `pool` (exactly 5 elements) to the 4 points that maximise the
/// contact area:
///
/// 1. Pick the deepest point (index of maximum `penetration`).
/// 2. Pick the point farthest from #1.
/// 3. Pick the point farthest from the line #1–#2.
/// 4. Pick the point that maximises the triangle area of the remaining set.
///
/// This is equivalent to a greedy farthest-point sampling and runs in O(1)
/// (fixed pool size of 5).
fn select_4_contacts(pool: &[ContactPoint; 5]) -> [ContactPoint; 4] {
    // Step 1 — deepest point.
    let i0 = (0..5)
        .max_by(|&a, &b| pool[a].penetration.total_cmp(&pool[b].penetration))
        .unwrap();

    // Step 2 — farthest from i0.
    let p0 = pool[i0].point;
    let i1 = (0..5)
        .filter(|&i| i != i0)
        .max_by(|&a, &b| {
            (pool[a].point - p0)
                .length_squared()
                .total_cmp(&(pool[b].point - p0).length_squared())
        })
        .unwrap();

    // Step 3 — farthest from the line p0–p1.
    let p1 = pool[i1].point;
    let seg = (p1 - p0).normalize_or_zero();
    let i2 = (0..5)
        .filter(|&i| i != i0 && i != i1)
        .max_by(|&a, &b| {
            dist_sq_to_line(pool[a].point, p0, seg).total_cmp(&dist_sq_to_line(
                pool[b].point,
                p0,
                seg,
            ))
        })
        .unwrap();

    // Step 4 — the remaining point that maximises the area of the
    // quadrilateral formed by the 4 selected points.
    //
    // When the contact patch is coplanar (common case: face-on-face) the
    // volume-based heuristic degenerates to zero.  Instead, compute the
    // sum of triangle areas from the candidate to every pair of
    // already-selected points.  This always picks the point that keeps
    // the contact patch as spread-out as possible.
    let p2 = pool[i2].point;
    let i3 = (0..5)
        .filter(|&i| i != i0 && i != i1 && i != i2)
        .max_by(|&a, &b| {
            let score = |idx: usize| -> f32 {
                let q = pool[idx].point;
                // Sum of cross-product magnitudes gives a good proxy for
                // how much area the candidate adds to the patch.
                (q - p0).cross(q - p1).length_squared()
                    + (q - p1).cross(q - p2).length_squared()
                    + (q - p2).cross(q - p0).length_squared()
            };
            score(a).total_cmp(&score(b))
        })
        .unwrap();

    [pool[i0], pool[i1], pool[i2], pool[i3]]
}

/// Squared distance from `point` to the infinite line through `origin` along
/// unit direction `dir`.
#[inline]
fn dist_sq_to_line(point: Vec3, origin: Vec3, dir: Vec3) -> f32 {
    let d = point - origin;
    let along = dir * d.dot(dir);
    (d - along).length_squared()
}

// ============================================================================
//  Event types
// ============================================================================

/// Whether a collision pair has just begun, is ongoing, or has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollisionEventType {
    /// First frame the pair is in contact.
    Started,
    /// Pair was already in contact last frame.
    Persisting,
    /// Pair is no longer in contact.
    Ended,
}

/// Emitted every physics step for each solid collision pair.
#[derive(Debug, Clone)]
pub struct CollisionEvent {
    pub entity_a: Entity,
    pub entity_b: Entity,
    pub event_type: CollisionEventType,
    /// Solved contact points (populated after constraint resolution).
    pub contact_points: arrayvec::ArrayVec<ContactPoint, 4>,
}

/// Emitted for trigger (non-solid) collider overlaps.
#[derive(Debug, Clone)]
pub struct TriggerEvent {
    /// The entity whose collider has `is_trigger = true`.
    pub trigger_entity: Entity,
    pub other_entity: Entity,
    pub event_type: CollisionEventType,
}

/// Emitted when a rigid body's fracture threshold is exceeded.
#[derive(Debug, Clone, Copy)]
pub struct FractureEvent {
    pub entity: Entity,
    pub impact_point: Vec3,
    pub impact_force: f32,
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entity(id: u32) -> Entity {
        Entity::new(id, 0)
    }

    fn pt(x: f32, y: f32, pen: f32) -> ContactPoint {
        ContactPoint {
            point: Vec3::new(x, y, 0.0),
            normal: Vec3::Y,
            penetration: pen,
            ..Default::default()
        }
    }

    // ── Entity ordering ───────────────────────────────────────────────────

    #[test]
    fn manifold_normalises_entity_order() {
        let e_high = make_entity(10);
        let e_low = make_entity(5);
        let m = ContactManifold::new(e_high, e_low);
        assert_eq!(m.entity_a.id(), 5);
        assert_eq!(m.entity_b.id(), 10);
    }

    #[test]
    fn manifold_same_order_when_already_sorted() {
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        let m = ContactManifold::new(e1, e2);
        assert_eq!(m.entity_a.id(), 1);
        assert_eq!(m.entity_b.id(), 2);
    }

    // ── Warm-start merge ──────────────────────────────────────────────────

    #[test]
    fn warm_start_preserves_impulses_on_merge() {
        let mut m = ContactManifold::new(make_entity(1), make_entity(2));

        let mut first = pt(1.0, 0.0, 0.1);
        first.normal_impulse = 5.0;
        first.tangent_impulse = Vec3::new(1.0, 0.0, 0.0);
        m.add_contact(first);

        // New contact is within the merge radius with updated geometry.
        let updated = pt(1.001, 0.0, 0.2);
        m.add_contact(updated);

        assert_eq!(m.contacts.len(), 1, "near-duplicate should merge, not add");
        assert_eq!(
            m.contacts[0].normal_impulse, 5.0,
            "accumulated normal impulse must be preserved"
        );
        assert_eq!(
            m.contacts[0].tangent_impulse,
            Vec3::new(1.0, 0.0, 0.0),
            "accumulated tangent impulse must be preserved"
        );
        assert!(
            (m.contacts[0].penetration - 0.2).abs() < 1e-6,
            "geometry (penetration) must be updated"
        );
    }

    // ── Contact capacity & area maximisation ──────────────────────────────

    #[test]
    fn contact_limit_enforced_at_4() {
        let mut m = ContactManifold::new(make_entity(1), make_entity(2));
        // 4 well-separated, equal-depth contacts.
        m.add_contact(pt(0.0, 0.0, 1.0));
        m.add_contact(pt(10.0, 0.0, 1.0));
        m.add_contact(pt(0.0, 10.0, 1.0));
        m.add_contact(pt(10.0, 10.0, 1.0));
        assert_eq!(m.contacts.len(), 4);

        // 5th point — shallow, near point #0; should be the one dropped.
        m.add_contact(pt(0.5, 0.5, 0.1));
        assert_eq!(m.contacts.len(), 4, "must stay at 4 contacts");

        // The shallow interloper should not survive.
        assert!(
            !m.contacts
                .iter()
                .any(|c| (c.penetration - 0.1).abs() < 1e-6),
            "shallowest near-duplicate contact should be dropped"
        );
    }

    #[test]
    fn deepest_contact_always_retained() {
        let mut m = ContactManifold::new(make_entity(1), make_entity(2));
        m.add_contact(pt(0.0, 0.0, 0.5));
        m.add_contact(pt(1.0, 0.0, 0.5));
        m.add_contact(pt(0.0, 1.0, 0.5));
        m.add_contact(pt(1.0, 1.0, 0.5));

        // Add a new point with extreme penetration.
        m.add_contact(pt(0.5, 0.5, 99.0));

        assert!(
            m.contacts
                .iter()
                .any(|c| (c.penetration - 99.0).abs() < 1e-6),
            "deepest contact must always be retained"
        );
    }

    // ── Staleness ─────────────────────────────────────────────────────────

    #[test]
    fn is_stale_respects_lifetime() {
        let mut m = ContactManifold::new(make_entity(1), make_entity(2));
        assert!(!m.is_stale(3));
        m.lifetime = 4;
        assert!(m.is_stale(3));
        m.lifetime = 3;
        assert!(!m.is_stale(3));
    }

    // ── Clear ─────────────────────────────────────────────────────────────

    #[test]
    fn clear_removes_contacts_but_not_lifetime() {
        let mut m = ContactManifold::new(make_entity(1), make_entity(2));
        m.add_contact(pt(0.0, 0.0, 1.0));
        m.lifetime = 7;
        m.clear();
        assert!(m.contacts.is_empty(), "contacts should be cleared");
        assert_eq!(m.lifetime, 7, "lifetime must not be touched by clear()");
    }

    // ── select_4_contacts ─────────────────────────────────────────────────

    #[test]
    fn select_4_keeps_deepest_and_maximises_spread() {
        // Arrange 5 points: 4 at corners of a 10×10 square (depth 1.0)
        // and one very deep point at the centre.
        let pool = [
            pt(0.0, 0.0, 1.0),
            pt(10.0, 0.0, 1.0),
            pt(0.0, 10.0, 1.0),
            pt(10.0, 10.0, 1.0),
            pt(5.0, 5.0, 5.0), // deepest, at centre
        ];
        let result = select_4_contacts(&pool);

        // The deepest point (centre, pen=5.0) must be in the result.
        assert!(
            result.iter().any(|c| (c.penetration - 5.0).abs() < 1e-6),
            "deepest point must be selected"
        );
        // All 4 must be distinct (no duplicates).
        for i in 0..4 {
            for j in (i + 1)..4 {
                assert_ne!(
                    result[i].point, result[j].point,
                    "selected contacts must be distinct"
                );
            }
        }
    }
}
