use gizmo_math::{Mat3, Vec3};
use gizmo_physics_core::BodyHandle;

/// Everything the soft-body collision response needs to know about the rigid body a node
/// hit, so it can solve the **two-body** impulse instead of bouncing off an assumed-immovable
/// surface.
///
/// This is a snapshot, not a handle: the caller reads it off the rigid body once per step and
/// hands it down. That is deliberate — this crate does not know what a rigid body *is*, and
/// the pure solver ([`crate::soft_body::resolve_node_collision`]) stays a function of its
/// arguments, which is what makes the conservation oracle in `tests/soft_rigid_coupling.rs`
/// testable without an ECS.
///
/// Units are SI: `inv_mass` in kg⁻¹, `inv_inertia` in kg⁻¹·m⁻², positions in metres,
/// velocities in m/s and rad/s. `inv_inertia` and `center_of_mass` are **world**-space —
/// the rigid crate's `RigidBody::inv_world_inertia_tensor(rotation)` and
/// `transform.position + transform.rotation * rb.center_of_mass` respectively.
///
/// [`IMMOVABLE`](Self::IMMOVABLE) is the neutral element, and it is what a lookup that finds
/// nothing falls back to. Feeding it reproduces the pre-coupling behaviour **bit for bit** —
/// see [`crate::soft_body::resolve_node_collision`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct RigidReaction {
    /// `1/m` in kg⁻¹, or `0.0` for infinite mass. Zero is the *only* thing that stops the
    /// linear half of the reaction being applied, which is why static and kinematic bodies
    /// must report zero here (the rigid crate's `RigidBody::inv_mass()` already does).
    pub inv_mass: f32,

    /// World-space inverse inertia tensor `R·I⁻¹·Rᵀ`, kg⁻¹·m⁻². [`Mat3::ZERO`] means "no
    /// angular response", which is also what the rigid crate reports for a non-dynamic body
    /// and for one whose rotation axes are all locked.
    pub inv_inertia: Mat3,

    /// World-space centre of mass, metres. Lever arms are measured from **this**, not from
    /// the transform origin; the two agree only while the body's COM offset is zero.
    pub center_of_mass: Vec3,

    /// World-space linear velocity of the centre of mass, m/s.
    pub linear: Vec3,

    /// World-space angular velocity, rad/s, in scaled-axis form.
    pub angular: Vec3,
}

impl RigidReaction {
    /// A body that cannot move and is not moving: infinite mass, no angular response, at
    /// rest. This is the implicit reaction for every collider the caller supplies no data
    /// for, and it is the *exact* limit the pre-coupling one-sided reflection assumed for
    /// every collider in the world.
    ///
    /// Equality with this constant is **sufficient** but not necessary to get the legacy
    /// arithmetic: [`crate::soft_body::resolve_node_collision`] tests the behaviour instead —
    /// [`can_react`](Self::can_react) false plus both velocities zero — so a parked kinematic
    /// body reported with its real, non-zero [`center_of_mass`](Self::center_of_mass) takes the
    /// same bit-exact path.
    pub const IMMOVABLE: Self = Self {
        inv_mass: 0.0,
        inv_inertia: Mat3::ZERO,
        center_of_mass: Vec3::ZERO,
        linear: Vec3::ZERO,
        angular: Vec3::ZERO,
    };

    /// Builds a reaction from the five quantities above. Nothing is validated or clamped: a
    /// negative `inv_mass` or a non-finite tensor propagates into the impulse.
    pub const fn new(
        inv_mass: f32,
        inv_inertia: Mat3,
        center_of_mass: Vec3,
        linear: Vec3,
        angular: Vec3,
    ) -> Self {
        Self {
            inv_mass,
            inv_inertia,
            center_of_mass,
            linear,
            angular,
        }
    }

    /// Whether an impulse applied to this body would change anything.
    ///
    /// `false` for static and kinematic bodies (both report zero inverse mass *and* a zero
    /// inverse inertia tensor), and for a dynamic body with `mass == 0.0`. The collision
    /// response reports a **zero** impulse for such a body rather than a token one, so
    /// "static/kinematic bodies are not pushed" holds at the source and not merely at the
    /// place the impulse is spent.
    #[inline]
    pub fn can_react(&self) -> bool {
        self.inv_mass != 0.0 || self.inv_inertia != Mat3::ZERO
    }

    /// The scalar effective mass of this body plus a node along `dir`, at a contact whose
    /// lever arm from the body's centre of mass is `r`:
    ///
    /// ```text
    /// k = 1/m_node + 1/m_body + dir · ((I⁻¹ (r × dir)) × r)
    /// ```
    ///
    /// This is the denominator of the impulse — `j = −(1+e)·v_rel·dir / k` — and it is what
    /// makes a light crate take most of the exchange and a heavy one almost none. `dir` is
    /// expected to be a unit vector (the contact normal, or the tangent of the slip).
    ///
    /// For [`IMMOVABLE`](Self::IMMOVABLE) both extra terms vanish and `k` is exactly
    /// `inv_node_mass`, so the impulse degenerates to the single-body reflection.
    #[inline]
    pub fn effective_mass(&self, inv_node_mass: f32, r: Vec3, dir: Vec3) -> f32 {
        let rxd = r.cross(dir);
        inv_node_mass + self.inv_mass + dir.dot((self.inv_inertia * rxd).cross(r))
    }

    /// Velocity of the body's material point at world position `point`, m/s:
    /// `v + ω × (point − com)`.
    #[inline]
    pub fn velocity_at(&self, point: Vec3) -> Vec3 {
        self.linear + self.angular.cross(point - self.center_of_mass)
    }
}

impl Default for RigidReaction {
    fn default() -> Self {
        Self::IMMOVABLE
    }
}

/// One soft-body node hitting one rigid collider: which body, where, and the reaction the
/// body is owed.
///
/// The node's own half of the exchange has already been applied to the velocity that comes
/// back in [`NodeCollision::velocity`]. This is the *other* half — equal and opposite by
/// construction, which is the whole point of the item this type exists for.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct NodeImpact {
    /// The collider that was hit. Before this existed the sweep resolved the node and threw
    /// the identity of the surface away, so the reaction had nowhere to go.
    pub body: BodyHandle,

    /// World-space contact point: where the sweep ray crossed the surface. Note this is
    /// **not** where the node is parked — the node stops `0.1 m` short of it (the fixed skin
    /// thickness), so lever arms taken about the node position and about this point differ
    /// by up to that much.
    pub point: Vec3,

    /// Outward unit surface normal at [`point`](Self::point), pointing back towards the side
    /// the node came from.
    pub normal: Vec3,

    /// Linear impulse owed to the **rigid body**, N·s, world-space. The node received
    /// exactly `-impulse`; summing the two gives zero, which is the conservation statement.
    ///
    /// Exactly [`Vec3::ZERO`] when the body cannot react
    /// ([`RigidReaction::can_react`] is `false`) — static, kinematic, or zero-mass dynamic.
    pub impulse: Vec3,

    /// Angular impulse owed to the rigid body about its centre of mass, N·m·s:
    /// `(point − com) × impulse`. Spend it as `ω += I⁻¹ · angular_impulse`.
    pub angular_impulse: Vec3,
}

/// What [`crate::soft_body::resolve_node_collision`] returns: the node's post-sweep state,
/// plus the impact if there was one.
///
/// Replaces the old `(position, velocity, bool)` triple. The `bool` said *that* something was
/// hit and the doc comment recorded, as a known omission, that it could not say *what* — see
/// [`NodeImpact::body`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct NodeCollision {
    /// Where the node ends up. On a miss this is the input position, untouched — the sweep
    /// never integrates, so the caller must advance the node itself in that case.
    pub position: Vec3,
    /// The node's velocity after the response, or the input velocity on a miss.
    pub velocity: Vec3,
    /// `Some` when a surface was resolved.
    pub impact: Option<NodeImpact>,
}

impl NodeCollision {
    /// A clean sweep: nothing was hit, both values come back untouched.
    #[inline]
    pub const fn miss(position: Vec3, velocity: Vec3) -> Self {
        Self {
            position,
            velocity,
            impact: None,
        }
    }

    /// Whether a surface was resolved — the old third tuple element.
    #[inline]
    pub const fn collided(&self) -> bool {
        self.impact.is_some()
    }
}

/// The reaction owed to one rigid body over a whole soft-body step, summed over every node
/// that touched it.
///
/// Kept as a sum rather than as one record per node so the consumer applies it once: the
/// order in which impulses are added to a velocity is not associative in `f32`, and one
/// deterministic accumulation (node order, then first-touch body order) is both cheaper and
/// bit-stable under replay.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct RigidImpulse {
    /// The body owed the reaction.
    pub body: BodyHandle,
    /// Σ of the linear impulses, N·s, world-space. Spend as `v += inv_mass · linear`.
    pub linear: Vec3,
    /// Σ of the angular impulses about the body's centre of mass, N·m·s, world-space.
    /// Spend as `ω += I⁻¹ · angular`.
    pub angular: Vec3,
}

impl RigidImpulse {
    /// An empty (zero) reaction for `body`.
    #[inline]
    pub const fn zero(body: BodyHandle) -> Self {
        Self {
            body,
            linear: Vec3::ZERO,
            angular: Vec3::ZERO,
        }
    }
}

/// Adds `impact`'s reaction into `out`, merging it with any impulse already owed to the same
/// body.
///
/// A no-op for an impact whose impulse is zero — a static or kinematic surface — so a body
/// that was merely *touched* never shows up in the output. Consumers rely on that: a
/// non-empty entry means "this body was actually pushed", which is the same scoping rule
/// `physics_explosion_system` uses to decide whether to wake a sleeper.
///
/// The scan is linear. One soft body touches a handful of colliders at once, so the list is
/// short, and a linear scan keeps the ordering deterministic where a hash map would not.
pub fn accumulate_impulse(out: &mut Vec<RigidImpulse>, impact: &NodeImpact) {
    if impact.impulse == Vec3::ZERO && impact.angular_impulse == Vec3::ZERO {
        return;
    }
    if let Some(existing) = out.iter_mut().find(|i| i.body == impact.body) {
        existing.linear += impact.impulse;
        existing.angular += impact.angular_impulse;
    } else {
        out.push(RigidImpulse {
            body: impact.body,
            linear: impact.impulse,
            angular: impact.angular_impulse,
        });
    }
}

/// Looks `body` up in a `(handle, reaction)` table, falling back to
/// [`RigidReaction::IMMOVABLE`].
///
/// Passing an empty table therefore means "every collider is immovable", which is exactly
/// what the engine assumed before two-way coupling existed — and is what every caller that
/// has no rigid-body data (the GPU readback, the unit tests, an embedder using this crate
/// without a rigid solver) passes.
#[inline]
pub fn reaction_for(reactions: &[(BodyHandle, RigidReaction)], body: BodyHandle) -> RigidReaction {
    reactions
        .iter()
        .find(|(h, _)| *h == body)
        .map_or(RigidReaction::IMMOVABLE, |(_, r)| *r)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The neutral element really is neutral: no linear response, no angular response, and
    /// the effective mass collapses to the node's own inverse mass *exactly* (not merely to
    /// within a tolerance — this exactness is what lets the immovable path stay bit-for-bit
    /// identical to the pre-coupling code).
    #[test]
    fn immovable_reaction_is_the_neutral_element() {
        let r = RigidReaction::IMMOVABLE;
        assert!(!r.can_react());
        assert_eq!(r.velocity_at(Vec3::new(3.0, -2.0, 1.0)), Vec3::ZERO);
        let inv_m = 1.0 / 0.7_f32;
        assert_eq!(
            r.effective_mass(inv_m, Vec3::new(0.3, -0.4, 0.5), Vec3::Y),
            inv_m,
            "k must be exactly 1/m_node for an immovable body"
        );
        assert_eq!(RigidReaction::default(), RigidReaction::IMMOVABLE);
    }

    /// A dynamic body with a real inverse mass reacts; the effective mass is strictly larger
    /// than the node's own (both bodies share the compliance), so the node bounces less.
    #[test]
    fn effective_mass_grows_with_a_reacting_body() {
        let r = RigidReaction::new(
            1.0 / 4.0,
            Mat3::from_diagonal(Vec3::splat(1.0 / 2.0)),
            Vec3::ZERO,
            Vec3::ZERO,
            Vec3::ZERO,
        );
        let inv_m = 1.0;
        // Head-on through the centre of mass: r × n = 0, so only the two inverse masses count.
        let head_on = r.effective_mass(inv_m, Vec3::ZERO, Vec3::X);
        assert!((head_on - 1.25).abs() < 1e-6, "k = 1 + 1/4, got {head_on}");
        // Off-centre: the angular term is positive-definite, so k grows further.
        let off_centre = r.effective_mass(inv_m, Vec3::new(0.0, 1.0, 0.0), Vec3::X);
        assert!(
            off_centre > head_on,
            "an off-centre hit must have a larger effective mass: {off_centre} vs {head_on}"
        );
        assert!(r.can_react());
    }

    /// The velocity of a point on a spinning body includes the `ω × r` term.
    #[test]
    fn velocity_at_includes_the_spin_term() {
        let r = RigidReaction::new(
            1.0,
            Mat3::IDENTITY,
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 2.0), // spin about +Z
        );
        // ω × r at r = +Y is (0,0,2) × (0,1,0) = (-2, 0, 0).
        let v = r.velocity_at(Vec3::Y);
        assert!((v - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-6, "{v:?}");
    }

    /// Impulses to the same body merge; a zero impulse (a static surface) never creates an
    /// entry, so "appears in the list" means "was actually pushed".
    #[test]
    fn accumulate_merges_per_body_and_skips_zero() {
        let a = BodyHandle::from_id(7);
        let b = BodyHandle::from_id(9);
        let mut out = Vec::new();

        let mk = |body, impulse: Vec3, angular: Vec3| NodeImpact {
            body,
            point: Vec3::ZERO,
            normal: Vec3::Y,
            impulse,
            angular_impulse: angular,
        };

        accumulate_impulse(&mut out, &mk(a, Vec3::X, Vec3::Y));
        accumulate_impulse(&mut out, &mk(a, Vec3::X * 2.0, Vec3::Y));
        accumulate_impulse(&mut out, &mk(b, Vec3::Z, Vec3::ZERO));
        // A static surface: nothing owed, so no entry at all.
        accumulate_impulse(&mut out, &mk(BodyHandle::from_id(11), Vec3::ZERO, Vec3::ZERO));

        assert_eq!(out.len(), 2, "one entry per pushed body: {out:?}");
        assert_eq!(out[0].body, a, "first-touch order must be preserved");
        assert_eq!(out[0].linear, Vec3::new(3.0, 0.0, 0.0));
        assert_eq!(out[0].angular, Vec3::new(0.0, 2.0, 0.0));
        assert_eq!(out[1].body, b);
        assert_eq!(RigidImpulse::zero(a).linear, Vec3::ZERO);
    }

    /// An unknown handle resolves to the immovable default — that is the compatibility hinge
    /// that lets every existing caller pass `&[]`.
    #[test]
    fn reaction_lookup_falls_back_to_immovable() {
        let table = [(
            BodyHandle::from_id(1),
            RigidReaction::new(0.5, Mat3::IDENTITY, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO),
        )];
        assert_eq!(reaction_for(&table, BodyHandle::from_id(1)).inv_mass, 0.5);
        assert_eq!(
            reaction_for(&table, BodyHandle::from_id(2)),
            RigidReaction::IMMOVABLE
        );
        assert_eq!(
            reaction_for(&[], BodyHandle::from_id(1)),
            RigidReaction::IMMOVABLE
        );
    }
}
