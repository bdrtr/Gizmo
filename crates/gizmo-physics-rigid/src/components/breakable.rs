/// A destructible body: a health pool that impacts and blasts drain, plus the recipe for the
/// debris that replaces the entity when it empties.
///
/// Damage arrives from two places — collisions hard enough to clear
/// [`threshold`](Self::threshold), and [`Explosion`](super::Explosion)s — and both are
/// gated on that threshold, so a breakable is immune to anything below it however often it
/// is hit.
///
/// Every **bounded** collider shatters — sphere, box, capsule, convex hull and compound. The
/// Voronoi cells are cut out of the collider's local bounding box, so the debris field has the
/// silhouette of that box rather than of the shape itself: a sphere breaks like the cube around
/// it. That is the same order of approximation as the debris already carries, since each cell is
/// then replaced by a sphere of matching volume whatever its real geometry.
///
/// [`Plane`](gizmo_physics_core::ColliderShape::Plane) and
/// [`TriMesh`](gizmo_physics_core::ColliderShape::TriMesh) do **not** shatter: the first is an
/// infinite half-space with no finite body to cut up, the second is the static concave variant,
/// which convex debris cannot represent. Such a body keeps taking damage and its
/// [`current_health`](Self::current_health) keeps falling — possibly far below zero — but
/// [`is_broken`](Self::is_broken) never latches and the entity stays whole.
///
/// The debris **layout** is seeded from the entity's own ECS id, so two objects broken under
/// identical conditions come apart differently while any one object comes apart the same way on
/// every run — which is what lets a fractured scene replay. Two consequences worth knowing: the
/// pattern is a property of the id, not of the break, so a body restored from a save
/// (`is_broken` is `#[serde(skip)]`) re-shatters exactly as it did before; and a breakable
/// spawned into a recycled id slot inherits the previous occupant's pattern.
///
/// Determinism is necessary for rollback but **not sufficient, and fracture does not have the
/// rest**: rollback restores component values, not structural changes, so a shatter that
/// despawned the body and spawned debris cannot be undone today — the body is not resurrected
/// and the debris is not removed. Treat a break as a point of no return across a rollback
/// window.
///
/// Through 0.9.0 only boxes shattered, and the other shapes latched `is_broken` anyway: they hit
/// zero health, spawned no debris, were never despawned, and — because every damage path is
/// gated on `!is_broken` — could never be damaged or broken again.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Breakable {
    /// How many Voronoi seed points to cut the body into — an *upper bound* on the debris
    /// count, not an exact one, since cells that fail to close into a solid are discarded.
    /// The shattering cost climbs steeply with this number (see
    /// [`voronoi_shatter`](crate::fracture::voronoi_shatter)), so it is the first knob to
    /// turn down if a break hitches.
    pub max_pieces: u32,
    /// Damage gate, in impulse units (N·s): an impact must exceed this before *any* health
    /// is lost. Below it the hit is discarded whole rather than scaled down, so a wall with
    /// a high threshold cannot be worn away by repeated small collisions.
    ///
    /// For blast damage the comparison is against the explosion's falloff-scaled force
    /// rather than a contact impulse — the same number is doing double duty for two
    /// differently-derived quantities.
    pub threshold: f32, // Minimum impulse to deal damage
    /// Remaining health, in whatever unit the incoming damage is expressed in — note the two
    /// damage sources do not agree on one: a qualifying collision subtracts the contact
    /// impulse itself, while a blast subtracts the explosion's falloff-scaled `damage`. The
    /// body shatters as soon as this reaches zero or less. Never clamped, so it can end up
    /// negative, and nothing ever refills it.
    pub current_health: f32,
    /// Reference value for a full health pool, for gameplay code that wants to display a
    /// ratio. Inert as far as physics is concerned: no built-in system reads it, and
    /// [`current_health`](Self::current_health) is neither clamped to it nor reset from it.
    pub max_health: f32,
    /// Intended lifetime of each debris piece in seconds, after which a game would despawn
    /// it. Metadata only: the built-in fracture path spawns chunks as ordinary permanent
    /// bodies and never expires them, so the timer has to be implemented by the caller.
    pub debris_lifetime: f32,
    /// Asset identifier for the sound to play when the body breaks. Carried for a game's
    /// audio layer to resolve — this crate has no audio dependency and never plays it.
    /// `None` means the caller has nothing to play.
    pub break_sound: Option<String>,
    /// Asset identifier for a prefab to spawn per piece instead of the procedurally
    /// shattered geometry. Metadata only — the built-in path always spawns procedural
    /// chunks and never consults this.
    pub piece_prefab: Option<String>,
    /// Latched `true` once the body has shattered, so later contacts cannot shatter it a
    /// second time. Nothing in the engine clears it again.
    ///
    /// Deliberately not serialized: a deserialized or snapshot-restored breakable comes back
    /// `false` while [`current_health`](Self::current_health) comes back at its saved value.
    /// An entity saved after breaking therefore returns "unbroken" with non-positive health
    /// and shatters again on the first impact that clears
    /// [`threshold`](Self::threshold).
    #[serde(skip)]
    pub is_broken: bool,
}

impl Default for Breakable {
    fn default() -> Self {
        Self {
            max_pieces: 10,
            threshold: 100.0,
            current_health: 100.0,
            max_health: 100.0,
            debris_lifetime: 5.0,
            break_sound: None,
            piece_prefab: None,
            is_broken: false,
        }
    }
}

#[cfg(feature = "ecs")]
gizmo_core::impl_component!(Breakable);
