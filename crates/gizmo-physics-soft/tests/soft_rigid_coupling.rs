//! Two-way soft↔rigid coupling: conservation oracles for the impulse a soft-body node and the
//! rigid body it hits now exchange.
//!
//! # Why "the crate moves now" is not a test
//!
//! `resolve_node_collision` used to reflect a node off every collider as though the collider
//! were an immovable wall, and throw away both the identity of what it hit and the impulse it
//! had just computed. Making the crate move is easy; making it move by the *right* amount is
//! the item. Two independent things have to hold, and each one kills a different plausible
//! fake:
//!
//! * **Linear momentum is conserved.** `Σ mᵢ·Δvᵢ + Δp_body = 0`. This kills the variant that
//!   solves the two halves from inconsistent formulas — the tempting one being "keep the
//!   node's existing immovable-wall reflection, and push the body with the proper two-body
//!   impulse". [`momentum_oracle_rejects_the_inconsistent_halves_variant`] runs exactly that
//!   arithmetic and shows it loses 120 kg·m/s on a 120 kg·m/s problem.
//!
//! * **The restitution identity holds at the contact.** `v_rel′·n = −e·(v_rel·n)`. This kills
//!   the variant momentum alone cannot see: "keep the immovable-wall reflection, and push the
//!   body by `−m·Δv_node`". That one conserves linear momentum *exactly* — it is
//!   `Δp_body ≡ −Δp_node` by construction — while blowing the effective restitution out to
//!   **3.5** on a body with `e = 0.5` and multiplying the collision's kinetic energy by 4.75.
//!   See [`restitution_identity_rejects_the_momentum_conserving_fake`].
//!
//! # Making the claim testable
//!
//! Gravity and damping both act on velocities *between* impacts, and neither conserves
//! momentum (damping is a deliberate energy sink; gravity is an external force). Every
//! conservation test here therefore runs with `gravity = Vec3::ZERO` and `damping = 1.0`, over
//! a **single** step. `1.0f32.powf(dt)` is exactly `1.0`, so the damping multiply is bit-exact
//! and the only thing that changes a node's velocity in these tests is the contact.
//!
//! Most of them also use no tetrahedral elements. Element forces cancel per element by
//! construction (`f0 = −(f1+f2+f3)`), so they conserve momentum to round-off rather than
//! exactly; [`momentum_is_conserved_for_a_real_tetrahedral_body`] covers that case separately
//! with a tolerance that says so.

use gizmo_math::{Mat3, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_soft::{resolve_node_collision, RigidImpulse, RigidReaction, SoftBodyMesh};

const DT: f32 = 1.0 / 60.0;
/// The response's hard-coded restitution. Not configurable; mirrored here so the tests state
/// what they are checking against rather than re-deriving it.
const E: f32 = 0.5;

/// A dynamic rigid body as these tests model it: enough to build a [`RigidReaction`], spend a
/// [`RigidImpulse`] and read a momentum back out.
#[derive(Debug, Clone, Copy)]
struct Body {
    mass: f32,
    inv_inertia: Mat3,
    com: Vec3,
    linear: Vec3,
    angular: Vec3,
}

impl Body {
    /// A point mass (no angular response) of `mass` kg whose centre of mass is `com`.
    fn point(mass: f32, com: Vec3) -> Self {
        Self {
            mass,
            inv_inertia: Mat3::ZERO,
            com,
            linear: Vec3::ZERO,
            angular: Vec3::ZERO,
        }
    }

    /// A solid box of `mass` kg with FULL extents `extents`, centred (and COM'd) at `com`.
    fn solid_box(mass: f32, extents: Vec3, com: Vec3) -> Self {
        // I = m/12 · (h² + d²) and cyclic — the same tensor `RigidBody::calculate_box_inertia`
        // derives, written out so this file does not depend on the rigid crate.
        let i = Vec3::new(
            mass / 12.0 * (extents.y * extents.y + extents.z * extents.z),
            mass / 12.0 * (extents.x * extents.x + extents.z * extents.z),
            mass / 12.0 * (extents.x * extents.x + extents.y * extents.y),
        );
        Self {
            mass,
            inv_inertia: Mat3::from_diagonal(Vec3::new(1.0 / i.x, 1.0 / i.y, 1.0 / i.z)),
            com,
            linear: Vec3::ZERO,
            angular: Vec3::ZERO,
        }
    }

    /// A body with an ISOTROPIC inertia `i` kg·m² picked by hand rather than derived from a
    /// shape.
    ///
    /// The angular oracle re-multiplies by the forward tensor (`L = I·ω`) after the solver
    /// divided by the inverse one, so `i` and `1/i` must round-trip exactly in `f32` or the
    /// tolerance would be measuring the tensor instead of the contact. Powers of two do; the
    /// `m/12·(h²+d²)` of a box generally does not.
    fn with_inertia(mass: f32, i: f32, com: Vec3) -> Self {
        Self {
            mass,
            inv_inertia: Mat3::from_diagonal(Vec3::splat(1.0 / i)),
            com,
            linear: Vec3::ZERO,
            angular: Vec3::ZERO,
        }
    }

    fn moving(mut self, linear: Vec3, angular: Vec3) -> Self {
        self.linear = linear;
        self.angular = angular;
        self
    }

    fn reaction(&self) -> RigidReaction {
        RigidReaction::new(
            1.0 / self.mass,
            self.inv_inertia,
            self.com,
            self.linear,
            self.angular,
        )
    }

    /// Spends a reaction exactly as the ECS driver does: `v += J/m`, `ω += I⁻¹·(r × J)`.
    fn apply(&mut self, impulse: &RigidImpulse) {
        self.linear += impulse.linear / self.mass;
        self.angular += self.inv_inertia * impulse.angular;
    }

    fn momentum(&self) -> Vec3 {
        self.linear * self.mass
    }
}

/// An axis-aligned box collider under `handle`.
fn collider(handle: u32, center: Vec3, half_extents: Vec3) -> (BodyHandle, Transform, Collider) {
    (
        BodyHandle::from_id(handle),
        Transform::new(center),
        Collider::box_collider(half_extents),
    )
}

/// A soft body that is pure particles: no elements, no gravity coupling, no damping. Its
/// nodes only ever change velocity through a contact, which is what makes the conservation
/// statement a statement about the contact.
fn particles(nodes: &[(Vec3, Vec3, f32)]) -> SoftBodyMesh {
    let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid material params");
    sb.damping = 1.0; // exactly 1.0 → `damping.powf(dt)` is exactly 1.0 → no velocity is lost
    for (position, velocity, mass) in nodes {
        let idx = sb.add_node(*position, *mass) as usize;
        sb.nodes[idx].velocity = *velocity;
    }
    sb
}

fn total_node_momentum(sb: &SoftBodyMesh) -> Vec3 {
    sb.nodes
        .iter()
        .fold(Vec3::ZERO, |acc, n| acc + n.velocity * n.mass)
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// 1. The hand-computable impact. Every number below is checked against the closed-form
//    one-dimensional two-body result, so this test is anchored to physics and not to the
//    implementation's own arithmetic.
// ─────────────────────────────────────────────────────────────────────────────────────────

/// A 2 kg node at 60 m/s hits a free 1 kg crate head-on, `e = 0.5`.
///
/// The textbook answer (`v₁′ = ((m₁ − e·m₂)v₁ + (1+e)m₂v₂)/(m₁+m₂)` and its mirror) is
/// `v_node = 30 m/s`, `v_crate = 60 m/s`, and the exchanged impulse is 60 N·s. Momentum before
/// and after is 120 kg·m/s.
///
/// Geometry: node at the origin sweeping +X for `60·(1/60) = 1 m`; the crate is a unit box
/// centred at `x = 1.05`, so its −X face — and the contact — is at `x = 0.55`, and the node
/// parks 0.1 m short of it at `x = 0.45`. The contact is dead through the centre of mass, so
/// `r × n = 0` and no angular term or friction enters: `k = 1/2 + 1/1 = 3/2` exactly.
#[test]
fn head_on_impact_matches_the_closed_form_two_body_result() {
    let mut sb = particles(&[(Vec3::ZERO, Vec3::new(60.0, 0.0, 0.0), 2.0)]);
    let mut body = Body::solid_box(1.0, Vec3::splat(1.0), Vec3::new(1.05, 0.0, 0.0));

    let colliders = [collider(1, Vec3::new(1.05, 0.0, 0.0), Vec3::splat(0.5))];
    let reactions = [(BodyHandle::from_id(1), body.reaction())];

    let p_before = total_node_momentum(&sb) + body.momentum();
    assert!(
        (p_before - Vec3::new(120.0, 0.0, 0.0)).length() < 1e-5,
        "scenario setup: total momentum should be 120 kg·m/s, got {p_before:?}"
    );

    let mut impulses = Vec::new();
    sb.step_coupled(DT, Vec3::ZERO, &colliders, &reactions, &mut impulses);

    assert_eq!(impulses.len(), 1, "exactly one body was hit: {impulses:?}");
    assert_eq!(
        impulses[0].body,
        BodyHandle::from_id(1),
        "the reaction must name the body that was hit"
    );
    assert!(
        (impulses[0].linear - Vec3::new(60.0, 0.0, 0.0)).length() < 1e-3,
        "j = (1+e)·Δv_approach / (1/m + 1/M) = 1.5·60/1.5 = 60 N·s, got {:?}",
        impulses[0].linear
    );
    assert!(
        impulses[0].angular.length() < 1e-4,
        "a hit straight through the centre of mass exerts no torque, got {:?}",
        impulses[0].angular
    );

    body.apply(&impulses[0]);

    assert!(
        (sb.nodes[0].velocity - Vec3::new(30.0, 0.0, 0.0)).length() < 1e-3,
        "node: ((2 − 0.5·1)·60)/3 = 30 m/s, got {:?}",
        sb.nodes[0].velocity
    );
    assert!(
        (body.linear - Vec3::new(60.0, 0.0, 0.0)).length() < 1e-3,
        "crate: ((1 + 0.5)·2·60)/3 = 60 m/s, got {:?}",
        body.linear
    );
    assert!(
        (sb.nodes[0].position.x - 0.45).abs() < 1e-3,
        "the node still parks 0.1 m short of the x = 0.55 face, got {}",
        sb.nodes[0].position.x
    );

    let p_after = total_node_momentum(&sb) + body.momentum();
    assert!(
        (p_after - p_before).length() < 1e-3,
        "momentum {p_before:?} -> {p_after:?}"
    );
}

/// The oracle is not vacuous: the "inconsistent halves" implementation fails it loudly.
///
/// That variant is the natural way to get this wrong — leave `resolve_node_collision`'s
/// existing immovable-wall reflection alone (the node comes back at `−e·v = −30 m/s`) and
/// additionally hand the crate the proper two-body impulse (60 N·s). Each half is defensible
/// on its own; together they destroy 120 kg·m/s, the entire momentum of the problem.
#[test]
fn momentum_oracle_rejects_the_inconsistent_halves_variant() {
    let (m, big_m, v0) = (2.0_f32, 1.0_f32, 60.0_f32);
    let p_before = m * v0;

    // Half A, as the pre-coupling engine computed it: reflect off an immovable surface.
    let node_after_one_sided = -E * v0; // −30 m/s
    // Half B, the correct two-body impulse for the crate.
    let j = (1.0 + E) * v0 / (1.0 / m + 1.0 / big_m); // 60 N·s
    let crate_after = j / big_m; // 60 m/s

    let p_after = m * node_after_one_sided + big_m * crate_after;
    assert!(
        (p_after - p_before).abs() > 100.0,
        "control: mixing the one-sided node reflection with a two-body body impulse must \
         break conservation badly — {p_before} -> {p_after}"
    );
    assert!(
        p_after.abs() < 1e-4,
        "control: the arithmetic above should land on exactly 0 kg·m/s (2·−30 + 1·60), got \
         {p_after}"
    );

    // …and the real implementation lands on the conserved answer instead. (Same scenario as
    // the test above; repeated here so the control and the subject sit side by side.)
    let mut sb = particles(&[(Vec3::ZERO, Vec3::new(v0, 0.0, 0.0), m)]);
    let body = Body::solid_box(big_m, Vec3::splat(1.0), Vec3::new(1.05, 0.0, 0.0));
    let mut impulses = Vec::new();
    sb.step_coupled(
        DT,
        Vec3::ZERO,
        &[collider(1, Vec3::new(1.05, 0.0, 0.0), Vec3::splat(0.5))],
        &[(BodyHandle::from_id(1), body.reaction())],
        &mut impulses,
    );
    let real = m * sb.nodes[0].velocity.x + impulses[0].linear.x;
    assert!(
        (real - p_before).abs() < 1e-3,
        "the implementation must conserve: {p_before} -> {real}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// 2. The restitution identity — the property linear momentum cannot see.
// ─────────────────────────────────────────────────────────────────────────────────────────

/// The relative normal velocity must reverse and scale by `e`: `v_rel′·n = −e·(v_rel·n)`.
///
/// This is the assertion that rejects the fake momentum conservation alone accepts: keep the
/// immovable-wall reflection for the node and push the crate by `−m·Δv_node`. Because that
/// hands the body exactly minus the node's momentum change, it conserves linear momentum by
/// construction — and yet it separates the pair at 210 m/s after a 60 m/s approach, an
/// effective restitution of **3.5**, multiplying the kinetic energy of the collision by 4.75.
/// The arithmetic for it is spelled out below so the failure mode is on the record.
#[test]
fn restitution_identity_rejects_the_momentum_conserving_fake() {
    let (m, big_m, v0) = (2.0_f32, 1.0_f32, 60.0_f32);

    // The fake: node reflects off an "immovable" wall, crate absorbs −m·Δv_node.
    let fake_node = -E * v0; // −30 m/s
    let fake_crate = -m * (fake_node - v0) / big_m; // +180 m/s
    let fake_momentum = m * fake_node + big_m * fake_crate;
    assert!(
        (fake_momentum - m * v0).abs() < 1e-3,
        "the fake really does conserve linear momentum — that is the whole point of this test"
    );
    let fake_restitution = (fake_crate - fake_node) / v0;
    assert!(
        (fake_restitution - 3.5).abs() < 1e-3,
        "the fake's effective restitution should be 3.5, got {fake_restitution}"
    );

    // The implementation.
    let mut sb = particles(&[(Vec3::ZERO, Vec3::new(v0, 0.0, 0.0), m)]);
    let mut body = Body::solid_box(big_m, Vec3::splat(1.0), Vec3::new(1.05, 0.0, 0.0));
    let mut impulses = Vec::new();
    sb.step_coupled(
        DT,
        Vec3::ZERO,
        &[collider(1, Vec3::new(1.05, 0.0, 0.0), Vec3::splat(0.5))],
        &[(BodyHandle::from_id(1), body.reaction())],
        &mut impulses,
    );
    body.apply(&impulses[0]);

    // n = (−1, 0, 0): the crate's −X face looks back at the incoming node.
    let n = Vec3::new(-1.0, 0.0, 0.0);
    let approach = (Vec3::new(v0, 0.0, 0.0) - Vec3::ZERO).dot(n); // −60
    let separate = (sb.nodes[0].velocity - body.linear).dot(n); // must be +30
    assert!(
        (separate + E * approach).abs() < 1e-3,
        "v_rel′·n must equal −e·(v_rel·n) = {}, got {separate}",
        -E * approach
    );
    let effective = separate / -approach;
    assert!(
        (effective - E).abs() < 1e-4,
        "effective restitution must be {E}, got {effective} (3.5 means the momentum-conserving \
         fake is in place)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// 3. The general oracle: an awkward impact, four nodes, one spinning body with an offset COM.
// ─────────────────────────────────────────────────────────────────────────────────────────

/// Linear momentum of (four soft nodes + one rigid body) is conserved across a single impact
/// step in which every complication is switched on at once: four different node masses,
/// oblique approach velocities (so the tangential/friction impulse is live), a body that is
/// already translating *and* spinning (so the contact-point velocity `v + ω × r` matters), and
/// a centre of mass displaced from the collider's centre (so the lever arms are non-zero and
/// the angular term enters the effective mass).
///
/// # Tolerance
///
/// `|Δp| ≤ 1e-5 · scale`, where `scale = Σ mᵢ|vᵢ| + M|V|` is the total momentum *traffic*
/// through the step (≈ 260 kg·m/s here), i.e. an absolute bar of ≈ 2.6e-3 kg·m/s.
///
/// What justifies the number: the identity is exact in ℝ — the node is given `+J/m` and the
/// body is owed `−J`, the same `J` — so everything that survives is `f32` round-off.
/// `f32` carries 1.19e-7 relative precision; each node's share passes through roughly ten
/// rounded operations (the impulse solve, `J·(1/m)`, the velocity add, and the `m·v`
/// remultiply in the oracle itself), and the sum is a *cancelling* one — the nodes lose ≈ 180
/// kg·m/s and the body gains ≈ 180 kg·m/s to leave ≈ 120 — so the error floor scales with the
/// traffic, not with the residual. 1e-5 relative is ≈ 84 ULP, roughly an order of magnitude
/// above that floor and four to five orders of magnitude *below* the smallest defect the test
/// exists to catch (the inconsistent-halves variant misses by ≈ 100% of the body's momentum).
#[test]
fn momentum_is_conserved_for_an_awkward_multi_node_impact() {
    let mut sb = particles(&[
        (Vec3::new(0.0, -0.6, 0.1), Vec3::new(50.0, 3.0, -2.0), 0.7),
        (Vec3::new(0.0, -0.2, -0.1), Vec3::new(55.0, -4.0, 1.0), 1.3),
        (Vec3::new(0.0, 0.2, 0.3), Vec3::new(60.0, 0.0, 5.0), 2.1),
        (Vec3::new(0.0, 0.6, -0.3), Vec3::new(45.0, 2.0, 2.0), 0.5),
    ]);

    // A 1 × 6 × 6 slab centred at x = 1.2 (near face at x = 0.7), 3 kg, with its centre of
    // mass deliberately NOT at the collider's centre.
    let mut body = Body::solid_box(3.0, Vec3::new(1.0, 6.0, 6.0), Vec3::new(1.2, 0.5, 0.0))
        .moving(Vec3::new(-1.5, 0.4, 0.2), Vec3::new(0.2, -0.3, 0.5));

    let colliders = [collider(42, Vec3::new(1.2, 0.0, 0.0), Vec3::new(0.5, 3.0, 3.0))];
    let reactions = [(BodyHandle::from_id(42), body.reaction())];

    let scale = sb
        .nodes
        .iter()
        .fold(0.0_f32, |acc, n| acc + n.mass * n.velocity.length())
        + body.mass * body.linear.length();
    let p_before = total_node_momentum(&sb) + body.momentum();

    let mut impulses = Vec::new();
    sb.step_coupled(DT, Vec3::ZERO, &colliders, &reactions, &mut impulses);

    // Scenario validity: if the geometry drifts and the nodes stop hitting anything, the
    // conservation assertion below becomes trivially true. Refuse to be vacuous.
    assert_eq!(
        impulses.len(),
        1,
        "all four nodes must land on the one body: {impulses:?}"
    );
    assert!(
        impulses[0].linear.length() > 1.0,
        "the exchange must be substantial, got {:?}",
        impulses[0].linear
    );

    body.apply(&impulses[0]);
    let p_after = total_node_momentum(&sb) + body.momentum();

    let err = (p_after - p_before).length();
    assert!(
        err <= 1e-5 * scale,
        "linear momentum must be conserved: {p_before:?} -> {p_after:?} (error {err}, \
         tolerance {})",
        1e-5 * scale
    );
}

/// The same oracle for a *real* tetrahedral soft body — four nodes bound by one element,
/// deformed so the internal elastic forces are large and non-trivial.
///
/// The tolerance is looser (`1e-4` relative) for one reason, and it is not the contact: an
/// element distributes `f0 = −(f1+f2+f3)` over its nodes, so the internal forces cancel by
/// *construction* in ℝ but only to round-off in `f32`, and that residual is multiplied by
/// `dt/mᵢ`, re-multiplied by `mᵢ` here, and summed. Elements are not what this item changed;
/// the test exists to show the contact does not add anything on top of that floor.
#[test]
fn momentum_is_conserved_for_a_real_tetrahedral_body() {
    let mut sb = SoftBodyMesh::new(2.0e4, 0.3).expect("valid material params");
    sb.damping = 1.0;
    for p in [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(-0.4, 0.0, 0.0),
        Vec3::new(-0.2, 0.5, 0.0),
        Vec3::new(-0.2, 0.2, 0.5),
    ] {
        sb.add_node(p, 1.0);
    }
    sb.add_element(0, 1, 2, 3).expect("well-conditioned tet");
    // Deform it (J = 1.3) so the element pushes hard, and launch the whole thing at the crate.
    sb.nodes[1].position *= 1.3;
    for n in &mut sb.nodes {
        n.velocity = Vec3::new(50.0, 0.0, 0.0);
    }

    // Near face at x = 0.35 — close enough that the elastic forces, which redistribute several
    // m/s between the nodes before the sweep runs, cannot push any node out of reach and make
    // the test quietly stop exercising the contact.
    let mut body = Body::solid_box(2.0, Vec3::new(1.0, 4.0, 4.0), Vec3::new(0.85, 0.0, 0.0));
    let colliders = [collider(3, Vec3::new(0.85, 0.0, 0.0), Vec3::new(0.5, 2.0, 2.0))];
    let reactions = [(BodyHandle::from_id(3), body.reaction())];

    let scale = sb
        .nodes
        .iter()
        .fold(0.0_f32, |acc, n| acc + n.mass * n.velocity.length());
    let p_before = total_node_momentum(&sb) + body.momentum();

    let mut impulses = Vec::new();
    sb.step_coupled(DT, Vec3::ZERO, &colliders, &reactions, &mut impulses);
    assert_eq!(impulses.len(), 1, "the tet must reach the crate: {impulses:?}");
    body.apply(&impulses[0]);

    let err = (total_node_momentum(&sb) + body.momentum() - p_before).length();
    assert!(
        err <= 1e-4 * scale,
        "momentum error {err} exceeds {} (elastic-force cancellation floor)",
        1e-4 * scale
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// 3b. The ANGULAR oracle — the half linear momentum cannot see.
// ─────────────────────────────────────────────────────────────────────────────────────────

/// Total **angular** momentum about the world origin is conserved across the impact, with the
/// node's share booked at the contact point the impulse was computed for.
///
/// # Why this is a separate oracle
///
/// `Σ mᵢ·Δvᵢ + Δp_body = 0` is blind to the angular half: it holds for *any*
/// `angular_impulse`, including zero, including one taken about the collider's centre instead
/// of the body's centre of mass, and including one derived from a different `J` than the linear
/// half. Every one of those is a plausible way to write this code and none of them shows up in
/// section 3. This test pins all three at once, because the identity below only closes when the
/// angular impulse is exactly `r × (−J)` with `r` measured from the **centre of mass** and `J`
/// the **same vector** the node was given.
///
/// # The identity
///
/// With `P` the contact point, `X` the centre of mass, `r = P − X` and `J` the impulse the node
/// receives, the body is handed `−J` at `P`. About the origin:
///
/// ```text
/// ΔL = P × J  +  [ r × (−J) ]  +  X × (−J)
///    = P × J  +  (P − X) × (−J) + X × (−J)
///    = P × J  −  P × J  =  0
/// ```
///
/// — exact in ℝ, for every mass ratio and every lever arm. The three brackets are, in order:
/// the node, the body's spin (`I·Δω = I·I⁻¹·angular_impulse`) and the body's orbital term
/// (`M·(X × ΔV)`).
///
/// # The scenario
///
/// Everything that can be switched on is: an oblique approach (so the friction impulse is live
/// and `J` is not parallel to `n`), a contact well off the centre of mass in two axes, a body
/// that is already translating *and* spinning, and a centre of mass displaced from the
/// collider's centre. The body's inertia is isotropic and a power of two so that `I·(I⁻¹·A)`
/// round-trips exactly in `f32` — otherwise the tolerance would be measuring the tensor rather
/// than the contact.
///
/// # Tolerance
///
/// `1e-4 · scale`, with `scale` the angular-momentum *traffic* through the step (the sum of the
/// magnitudes of the six terms above, ≈ 3.6e2 kg·m²/s here) — an absolute bar of ≈ 3.6e-2.
/// What justifies it: the identity is exact in ℝ, so the whole residual is `f32` round-off on
/// terms of magnitude ~2e2 that then cancel to ~0; at 1.19e-7 relative precision and ~10 rounded
/// operations per term that floor is ~2e-4, three orders below the bar. And the bar is in turn
/// more than an order of magnitude below the *smallest* defect the test exists to catch — the
/// second half of this test measures that defect directly and refuses to pass unless it is at
/// least 10× the tolerance: booking the node's share 0.1 m away (at the position it parks at
/// rather than at the contact) already costs ≈ 0.8 kg·m²/s. A wrong lever-arm origin, a
/// mismatched `J` or a dropped angular half all cost far more than that.
#[test]
fn angular_momentum_is_conserved_about_the_contact_point() {
    // I = 2 kg·m², isotropic → I⁻¹ = 0.5 and `2·0.5 == 1` exactly. Mass 4 kg for the same
    // reason (`(x/4)·4` is exact). COM deliberately off the collider's centre, and the body is
    // already moving and spinning.
    const INERTIA: f32 = 2.0;
    let before = Body::with_inertia(4.0, INERTIA, Vec3::new(1.7, 0.5, 0.0))
        .moving(Vec3::new(-1.0, 0.5, 0.25), Vec3::new(0.3, -0.2, 0.4));
    let mut body = before;

    // A 2 × 2 × 2 collider centred at (1.7, 0, 0): its −X face is at x = 0.7.
    let colliders = [collider(5, Vec3::new(1.7, 0.0, 0.0), Vec3::splat(1.0))];

    let node_mass = 2.0_f32;
    let x0 = Vec3::new(0.0, 0.4, -0.3);
    let v0 = Vec3::new(60.0, 5.0, -4.0); // oblique: the tangential impulse is live

    let out = resolve_node_collision(
        x0,
        v0,
        node_mass,
        DT,
        &colliders,
        &[(BodyHandle::from_id(5), body.reaction())],
    );
    let impact = out.impact.expect("the node must reach the slab");

    // Non-vacuity: both halves of the exchange must be substantial, and the contact must be
    // genuinely off-centre in more than one axis (otherwise the angular term is trivial).
    assert!(
        impact.impulse.length() > 10.0,
        "the exchange must be substantial, got {:?}",
        impact.impulse
    );
    assert!(
        impact.angular_impulse.length() > 1.0,
        "the hit must be off-centre enough to torque the body, got {:?}",
        impact.angular_impulse
    );
    let r = impact.point - before.com;
    assert!(
        r.x.abs() > 0.1 && r.z.abs() > 0.1,
        "the lever arm must be off-centre in more than one axis, got {r:?}"
    );

    let mut owed = RigidImpulse::zero(impact.body);
    owed.linear = impact.impulse;
    owed.angular = impact.angular_impulse;
    body.apply(&owed);

    // L about the world origin, with the node's share taken at the CONTACT POINT — the point
    // the impulse was solved for, and the point the node physically touched.
    let point = impact.point;
    // m·(x × v)  +  I·ω  +  M·(X × V)
    let angular_momentum = |node_x: Vec3, node_v: Vec3, b: &Body| {
        node_x.cross(node_v) * node_mass + b.angular * INERTIA + b.com.cross(b.linear) * b.mass
    };
    // Both sides are booked at the SAME fixed point, so the difference isolates the velocity
    // change — which is the whole content of the identity.
    let l_before = angular_momentum(point, v0, &before);
    let l_after = angular_momentum(point, out.velocity, &body);

    let scale = node_mass * point.length() * (v0.length() + out.velocity.length())
        + INERTIA * (before.angular.length() + body.angular.length())
        + before.mass * before.com.length() * (before.linear.length() + body.linear.length());
    let err = (l_after - l_before).length();
    assert!(
        err <= 1e-4 * scale,
        "angular momentum must be conserved about the contact point: {l_before:?} -> \
         {l_after:?} (error {err}, tolerance {})",
        1e-4 * scale
    );

    // …and the SKIN is the only thing that spoils it. The node parks 0.1 m short of the
    // contact, so booking its share there instead shifts L by exactly `(x_parked − P) × J`.
    // Asserting the shift rather than merely bounding it does two jobs: it states the doc
    // comment's "conserved up to the 0.1 m skin" as a checked number, and it proves the
    // assertion above is not vacuous — the oracle really can tell two booking points apart.
    let offset = out.position - point;
    assert!(
        (offset.length() - 0.1).abs() < 1e-4,
        "the node must park exactly 0.1 m short of the contact, got {}",
        offset.length()
    );
    let j_node = -impact.impulse; // what the NODE received; the body was owed its negative
    let skin_shift = angular_momentum(out.position, out.velocity, &body)
        - angular_momentum(out.position, v0, &before);
    assert!(
        (skin_shift - offset.cross(j_node)).length() <= 1e-4 * scale,
        "the skin residual must be exactly (x_parked − P) × J: {skin_shift:?} vs {:?}",
        offset.cross(j_node)
    );
    assert!(
        skin_shift.length() > 10.0 * (1e-4 * scale),
        "the skin residual must be far larger than the tolerance, or the oracle above cannot \
         tell one booking point from another; got {} vs {}",
        skin_shift.length(),
        1e-4 * scale
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// 4. Bodies that must not be pushed.
// ─────────────────────────────────────────────────────────────────────────────────────────

/// A collider with no reaction entry (how the ECS driver reports a **static** body) is
/// immovable, and the node's response must come back **bit-for-bit identical** to the
/// pre-coupling engine — not merely close. That exactness is why `resolve_node_collision`
/// keeps a separate branch for it: level geometry must not drift a single ULP because
/// two-way coupling was added.
#[test]
fn a_static_surface_is_never_pushed_and_the_node_response_is_unchanged() {
    let colliders = [collider(1, Vec3::new(1.05, 0.0, 0.0), Vec3::splat(0.5))];
    let v = Vec3::new(60.0, 12.0, -7.0); // oblique, so the friction branch runs too

    let out = resolve_node_collision(Vec3::ZERO, v, 2.0, DT, &colliders, &[]);
    let impact = out.impact.expect("the node must hit the box");
    assert_eq!(impact.impulse, Vec3::ZERO, "a static body takes no impulse");
    assert_eq!(impact.angular_impulse, Vec3::ZERO);
    assert_eq!(impact.body, BodyHandle::from_id(1));

    // The pre-coupling expression, transcribed: reflect the normal component by `e`, scale the
    // tangential one by `1 − f`.
    let n = impact.normal;
    let vn = v.dot(n);
    let vt = v - n * vn;
    let legacy = vt * (1.0 - 0.8) - n * (vn * E);
    assert_eq!(
        out.velocity, legacy,
        "the immovable path must be bit-identical to the original expression"
    );

    // And an explicit `IMMOVABLE` entry in the table is the same thing as no entry at all.
    let explicit = resolve_node_collision(
        Vec3::ZERO,
        v,
        2.0,
        DT,
        &colliders,
        &[(BodyHandle::from_id(1), RigidReaction::IMMOVABLE)],
    );
    assert_eq!(explicit.velocity, out.velocity);
    assert_eq!(explicit.position, out.position);
}

/// A **kinematic** body takes no impulse either — it reports zero inverse mass and zero
/// inverse inertia, so [`RigidReaction::can_react`] is false and the response withholds the
/// reaction at the source rather than relying on the consumer to drop it.
///
/// Its *velocity* does enter the response, though, which is a deliberate behaviour change: a
/// moving platform now kicks a node it sweeps into. Before coupling every collider was assumed
/// to be at rest. With the surface receding at 10 m/s the approach speed is 50, not 60, so the
/// node leaves at `60 − (1+e)·50 = −15 m/s` rather than the −30 it would get off a static
/// wall.
#[test]
fn a_kinematic_body_is_never_pushed_but_its_motion_is_felt() {
    let colliders = [collider(1, Vec3::new(1.05, 0.0, 0.0), Vec3::splat(0.5))];
    let platform = RigidReaction::new(
        0.0,       // kinematic: infinite mass
        Mat3::ZERO, // …and no angular response
        Vec3::new(1.05, 0.0, 0.0),
        Vec3::new(10.0, 0.0, 0.0), // receding along +X at 10 m/s
        Vec3::ZERO,
    );
    assert!(!platform.can_react());

    let out = resolve_node_collision(
        Vec3::ZERO,
        Vec3::new(60.0, 0.0, 0.0),
        2.0,
        DT,
        &colliders,
        &[(BodyHandle::from_id(1), platform)],
    );
    let impact = out.impact.expect("the node must hit the platform");
    assert_eq!(
        impact.impulse,
        Vec3::ZERO,
        "a kinematic body must not be pushed"
    );
    assert!(
        (out.velocity.x - (-15.0)).abs() < 1e-3,
        "the platform's own motion must enter v_rel: expected −15 m/s, got {}",
        out.velocity.x
    );
}

/// A kinematic body that is **parked** is bit-for-bit the immovable path, centre of mass and
/// all.
///
/// The legacy branch is selected on *behaviour* — `can_react()` false and both velocities zero
/// — and not on `reaction == RigidReaction::IMMOVABLE`, precisely so this holds. A parked lift
/// or a closed door is reported by the ECS driver with its real centre of mass, which is not
/// `Vec3::ZERO`, so an identity test would have sent it down the general branch. That branch is
/// analytically the same thing but not bit-identical: it evaluates `v_t − f·|v_t|·t` where the
/// legacy form evaluates `v_t·(1 − f)`, and `j·(1/m)` where the legacy form never divides at
/// all. A scene full of parked kinematic geometry would have drifted a ULP per contact for no
/// physical reason.
///
/// Both sub-cases below are checked with `assert_eq!` on the raw `f32`s, not a tolerance.
#[test]
fn a_parked_kinematic_body_is_bit_identical_to_the_immovable_path() {
    let colliders = [collider(1, Vec3::new(1.05, 0.0, 0.0), Vec3::splat(0.5))];
    let v = Vec3::new(60.0, 12.0, -7.0); // oblique, so the friction branch runs too

    let legacy = resolve_node_collision(Vec3::ZERO, v, 2.0, DT, &colliders, &[]);

    // Kinematic: zero inverse mass, zero inverse inertia, at rest — but with a real, non-zero
    // centre of mass, which is exactly what `collect_rigid_reactions` reports for it.
    let parked = RigidReaction::new(
        0.0,
        Mat3::ZERO,
        Vec3::new(1.05, 0.0, 0.0),
        Vec3::ZERO,
        Vec3::ZERO,
    );
    assert!(!parked.can_react());
    assert_ne!(
        parked,
        RigidReaction::IMMOVABLE,
        "the point of this test is that it is NOT equal to IMMOVABLE"
    );

    let out = resolve_node_collision(
        Vec3::ZERO,
        v,
        2.0,
        DT,
        &colliders,
        &[(BodyHandle::from_id(1), parked)],
    );
    assert_eq!(
        out.velocity, legacy.velocity,
        "a parked kinematic body must not move the node by a single ULP"
    );
    assert_eq!(out.position, legacy.position);
    let impact = out.impact.expect("the node must still report the surface");
    assert_eq!(impact.impulse, Vec3::ZERO, "…and must not be pushed");
    assert_eq!(impact.angular_impulse, Vec3::ZERO);

    // The centre of mass genuinely drops out: with both inverse mass and inverse inertia zero
    // and the body at rest, `velocity_at` is zero and `effective_mass` is exactly `1/m` for any
    // lever arm, so an absurd COM must land on the same bits.
    let absurd_com = RigidReaction::new(
        0.0,
        Mat3::ZERO,
        Vec3::new(-300.0, 7.5, 2.25),
        Vec3::ZERO,
        Vec3::ZERO,
    );
    let out2 = resolve_node_collision(
        Vec3::ZERO,
        v,
        2.0,
        DT,
        &colliders,
        &[(BodyHandle::from_id(1), absurd_com)],
    );
    assert_eq!(
        out2.velocity, legacy.velocity,
        "the lever arm cannot matter when neither inverse mass nor inverse inertia can use it"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// 5. The angular half, and the heavy-body limit.
// ─────────────────────────────────────────────────────────────────────────────────────────

/// An off-centre hit torques the body: the reported angular impulse is `r × J` about the
/// centre of mass, and it spins the crate the way a shove above its middle should.
///
/// Node at `y = +0.3` moving +X into a unit crate centred at `(1.05, 0, 0)`: the contact is at
/// `(0.55, 0.3, 0)`, so `r = (−0.5, 0.3, 0)` and `n = (−1, 0, 0)`. The angular term lifts the
/// effective mass from `1/2 + 1/1 = 1.5` to `2.04` (`I⁻¹ = 6·I` for a 1 kg unit cube), so
/// `j = 1.5·60/2.04 ≈ 44.12 N·s` — a *weaker* exchange than the head-on case, because part of
/// the compliance goes into spin. The torque is `r × J ≈ (0, 0, −13.24) N·m·s`: pushed above
/// its middle, the crate tips forward.
///
/// Note angular momentum is conserved here only up to the 0.1 m skin: the body's share is
/// measured about the contact point, while the node ends up parked 0.1 m short of it.
#[test]
fn an_off_centre_hit_torques_the_body_about_its_centre_of_mass() {
    let colliders = [collider(1, Vec3::new(1.05, 0.0, 0.0), Vec3::splat(0.5))];
    let crate_body = Body::solid_box(1.0, Vec3::splat(1.0), Vec3::new(1.05, 0.0, 0.0));

    let out = resolve_node_collision(
        Vec3::new(0.0, 0.3, 0.0),
        Vec3::new(60.0, 0.0, 0.0),
        2.0,
        DT,
        &colliders,
        &[(BodyHandle::from_id(1), crate_body.reaction())],
    );
    let impact = out.impact.expect("the node must hit the crate");

    assert!(
        (impact.point - Vec3::new(0.55, 0.3, 0.0)).length() < 1e-3,
        "contact point on the −X face at the node's height, got {:?}",
        impact.point
    );
    assert!(
        (impact.impulse.x - 44.1176).abs() < 1e-2,
        "j = 1.5·60/2.04 ≈ 44.12 N·s (the angular term makes the contact stiffer to shove), \
         got {:?}",
        impact.impulse
    );
    assert!(
        impact.impulse.x < 60.0,
        "an off-centre hit must transfer LESS linear impulse than the head-on 60 N·s"
    );

    // The torque is exactly the lever arm crossed into the impulse.
    let r = impact.point - crate_body.com;
    assert!(
        (impact.angular_impulse - r.cross(impact.impulse)).length() < 1e-4,
        "angular impulse must be r × J: {:?} vs {:?}",
        impact.angular_impulse,
        r.cross(impact.impulse)
    );
    assert!(
        impact.angular_impulse.z < -10.0,
        "a shove above the centre of mass tips the crate forward (−Z spin), got {:?}",
        impact.angular_impulse
    );
}

/// The exchange scales with the mass ratio, and the infinite-mass limit is the old behaviour.
///
/// A heavier crate takes a larger impulse but a smaller velocity change, and the node's rebound
/// walks monotonically from "barely slowed" toward the immovable-wall reflection of −30 m/s.
/// At 1e9 kg the general two-body formula and the legacy branch agree to within 1e-3 m/s —
/// which is the numerical statement that the legacy response *is* the `1/M → 0` limit of the
/// new one, and not a separate model bolted alongside it.
#[test]
fn the_heavy_body_limit_reproduces_the_immovable_response() {
    let colliders = [collider(1, Vec3::new(1.05, 0.0, 0.0), Vec3::splat(0.5))];
    let v0 = Vec3::new(60.0, 0.0, 0.0);

    let legacy = resolve_node_collision(Vec3::ZERO, v0, 2.0, DT, &colliders, &[]);
    assert!(
        (legacy.velocity.x - (-30.0)).abs() < 1e-4,
        "immovable reflection is −e·v = −30 m/s, got {}",
        legacy.velocity.x
    );

    let mut previous_dv = f32::INFINITY;
    let mut previous_node_vx = f32::INFINITY;
    for mass in [1.0_f32, 10.0, 1000.0, 1.0e9] {
        // A point mass: no angular response, so the comparison isolates the mass ratio.
        let body = Body::point(mass, Vec3::new(1.05, 0.0, 0.0));
        let out = resolve_node_collision(
            Vec3::ZERO,
            v0,
            2.0,
            DT,
            &colliders,
            &[(BodyHandle::from_id(1), body.reaction())],
        );
        let impact = out.impact.expect("hit");
        let dv = impact.impulse.x / mass;

        assert!(
            dv < previous_dv,
            "a heavier crate must be moved LESS: {mass} kg -> Δv {dv}, previous {previous_dv}"
        );
        assert!(
            out.velocity.x < previous_node_vx,
            "…and the node must rebound harder as the crate gets heavier: {mass} kg -> {}",
            out.velocity.x
        );
        previous_dv = dv;
        previous_node_vx = out.velocity.x;
    }

    // The limit itself.
    let huge = Body::point(1.0e9, Vec3::new(1.05, 0.0, 0.0));
    let out = resolve_node_collision(
        Vec3::ZERO,
        v0,
        2.0,
        DT,
        &colliders,
        &[(BodyHandle::from_id(1), huge.reaction())],
    );
    assert!(
        (out.velocity.x - legacy.velocity.x).abs() < 1e-3,
        "1e9 kg must reproduce the immovable response ({}), got {}",
        legacy.velocity.x,
        out.velocity.x
    );
}

/// A node that is *separating* from the surface it is sweeping past is not resolved, and owes
/// nothing — measured on the RELATIVE velocity, so a node riding a platform that is running
/// away faster than the node is closing does not get yanked back.
#[test]
fn a_separating_contact_exchanges_nothing() {
    let colliders = [collider(1, Vec3::new(1.05, 0.0, 0.0), Vec3::splat(0.5))];
    // Node closing at 60 m/s, surface running away at 90 m/s → v_rel·n = +30, separating.
    let runaway = Body::point(4.0, Vec3::new(1.05, 0.0, 0.0)).moving(
        Vec3::new(90.0, 0.0, 0.0),
        Vec3::ZERO,
    );
    let out = resolve_node_collision(
        Vec3::ZERO,
        Vec3::new(60.0, 0.0, 0.0),
        2.0,
        DT,
        &colliders,
        &[(BodyHandle::from_id(1), runaway.reaction())],
    );
    let impact = out.impact.expect("the sweep still reports the surface it reached");
    assert_eq!(
        impact.impulse,
        Vec3::ZERO,
        "a separating contact must not exchange an impulse"
    );
    assert_eq!(
        out.velocity,
        Vec3::new(60.0, 0.0, 0.0),
        "…and must leave the node's velocity alone"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// 6. Determinism.
// ─────────────────────────────────────────────────────────────────────────────────────────

/// Coupling must not cost replay determinism: the impulses are accumulated in node order into
/// a first-touch-ordered list, so two identical worlds produce bit-identical soft bodies AND
/// bit-identical reaction lists.
#[test]
fn coupled_stepping_is_bit_deterministic() {
    let build = || {
        let mut sb = particles(&[
            (Vec3::new(0.0, -0.3, 0.2), Vec3::new(50.0, 4.0, -3.0), 0.9),
            (Vec3::new(0.0, 0.3, -0.2), Vec3::new(58.0, -2.0, 1.0), 1.7),
        ]);
        sb.damping = 0.99; // back to the default: determinism must hold with damping on
        sb
    };
    let body = Body::solid_box(2.5, Vec3::new(1.0, 4.0, 4.0), Vec3::new(1.2, 0.3, 0.0))
        .moving(Vec3::new(-0.7, 0.1, 0.0), Vec3::new(0.0, 0.2, -0.1));
    let colliders = [collider(9, Vec3::new(1.2, 0.0, 0.0), Vec3::new(0.5, 2.0, 2.0))];
    let reactions = [(BodyHandle::from_id(9), body.reaction())];

    let (mut a, mut b) = (build(), build());
    let (mut ia, mut ib) = (Vec::new(), Vec::new());
    a.step_coupled(DT, Vec3::new(0.0, -9.81, 0.0), &colliders, &reactions, &mut ia);
    b.step_coupled(DT, Vec3::new(0.0, -9.81, 0.0), &colliders, &reactions, &mut ib);

    assert_eq!(a, b, "identical soft bodies must evolve identically");
    assert_eq!(ia, ib, "…and owe identical impulses");
    assert!(!ia.is_empty(), "the scenario must actually produce a contact");
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// 7. The ECS driver: the user-visible symptom.
// ─────────────────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "rigid-coupling")]
mod ecs {
    use super::*;
    use gizmo_core::world::World;
    use gizmo_physics_rigid::components::{RigidBody, Velocity};
    use gizmo_physics_soft::system::soft_body_step_system;

    /// A crate that has settled and fallen asleep beside a soft body: parked exactly as
    /// `update_sleep_state` would have left it.
    fn sleeping_crate(mass: f32) -> RigidBody {
        let mut rb = RigidBody::new(mass, false); // gravity off: this test is about the impulse
        rb.update_inertia_from_collider(&Collider::box_collider(Vec3::splat(0.5)));
        rb.is_sleeping = true;
        rb.sleep_counter = RigidBody::SLEEP_FRAMES_REQUIRED + 12;
        rb
    }

    fn spawn_soft(world: &mut World, sb: SoftBodyMesh) -> gizmo_core::entity::Entity {
        let e = world.spawn();
        world.add_component(e, sb);
        e
    }

    fn spawn_rigid(
        world: &mut World,
        rb: RigidBody,
        shape: Collider,
        pos: Vec3,
    ) -> gizmo_core::entity::Entity {
        let e = world.spawn();
        world.add_component(e, rb);
        world.add_component(e, shape);
        world.add_component(e, Transform::new(pos));
        world.add_component(e, Velocity::default());
        e
    }

    /// End-to-end, through the real driver: a 2 kg soft node at 60 m/s hits a sleeping 1 kg
    /// crate. The crate must end up at 60 m/s **and awake**.
    ///
    /// The wake is not decoration. A sleeping body is skipped by both integration stages and
    /// its island is never solved, so the velocity would sit in the component unspent and move
    /// nothing — the exact failure `physics_explosion_system` was fixed for. Asserting on
    /// `Velocity` alone would pass either way, which is why `is_sleeping` is asserted too.
    #[test]
    fn a_soft_body_wakes_and_shoves_a_sleeping_crate() {
        let mut world = World::new();

        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        sb.damping = 1.0;
        let idx = sb.add_node(Vec3::ZERO, 2.0) as usize;
        sb.nodes[idx].velocity = Vec3::new(60.0, 0.0, 0.0);
        spawn_soft(&mut world, sb);

        let target = spawn_rigid(
            &mut world,
            sleeping_crate(1.0),
            Collider::box_collider(Vec3::splat(0.5)),
            Vec3::new(1.05, 0.0, 0.0),
        );

        soft_body_step_system(&world, DT, Vec3::ZERO);

        let bodies = world.borrow::<RigidBody>();
        let vels = world.borrow::<Velocity>();
        let rb = bodies.get(target.id()).expect("crate");
        let vel = vels.get(target.id()).expect("crate velocity");

        assert!(
            (vel.linear.x - 60.0).abs() < 1e-2,
            "the crate must take the full 60 N·s / 1 kg, got {:?}",
            vel.linear
        );
        assert!(
            !rb.is_sleeping,
            "a pushed body MUST be woken or the integrator swallows the impulse ({:?})",
            vel.linear
        );
        assert_eq!(rb.sleep_counter, 0, "waking must reset the sleep counter");
    }

    /// The scope of the push, which is the obvious way to over-correct this: a **static**
    /// floor takes nothing and is not woken, however hard a soft body lands on it.
    ///
    /// The driver leaves static bodies out of the reaction table entirely, so they resolve to
    /// `RigidReaction::IMMOVABLE` and the node takes the unchanged legacy response — here, a
    /// clean `−e·v` bounce back up at +30 m/s.
    #[test]
    fn a_static_floor_is_neither_pushed_nor_woken() {
        let mut world = World::new();

        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        sb.damping = 1.0;
        let idx = sb.add_node(Vec3::ZERO, 2.0) as usize;
        sb.nodes[idx].velocity = Vec3::new(0.0, -60.0, 0.0);
        let soft = spawn_soft(&mut world, sb);

        // Top face at y = −0.1, straight below the node.
        let floor = spawn_rigid(
            &mut world,
            RigidBody::new_static(),
            Collider::box_collider(Vec3::new(5.0, 0.5, 5.0)),
            Vec3::new(0.0, -0.6, 0.0),
        );

        soft_body_step_system(&world, DT, Vec3::ZERO);

        {
            let bodies = world.borrow::<RigidBody>();
            let vels = world.borrow::<Velocity>();
            assert_eq!(
                vels.get(floor.id()).expect("floor velocity").linear,
                Vec3::ZERO,
                "a static floor must never be pushed"
            );
            assert!(
                bodies.get(floor.id()).expect("floor").is_sleeping,
                "…and must not be woken by the contact either"
            );
        }

        let meshes = world.borrow::<SoftBodyMesh>();
        let sb = meshes.get(soft.id()).expect("soft body");
        assert!(
            (sb.nodes[0].velocity.y - 30.0).abs() < 1e-2,
            "the node must still bounce at −e·v = +30 m/s off the static floor, got {:?}",
            sb.nodes[0].velocity
        );
    }

    /// Nothing in the world to hit: the driver must not manufacture a reaction, and must not
    /// disturb a sleeping body that was never touched.
    #[test]
    fn an_untouched_body_is_left_alone() {
        let mut world = World::new();

        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        sb.damping = 1.0;
        let idx = sb.add_node(Vec3::ZERO, 2.0) as usize;
        sb.nodes[idx].velocity = Vec3::new(60.0, 0.0, 0.0);
        spawn_soft(&mut world, sb);

        // 40 m away — far outside the 1 m sweep.
        let far = spawn_rigid(
            &mut world,
            sleeping_crate(1.0),
            Collider::box_collider(Vec3::splat(0.5)),
            Vec3::new(40.0, 0.0, 0.0),
        );

        soft_body_step_system(&world, DT, Vec3::ZERO);

        let bodies = world.borrow::<RigidBody>();
        let vels = world.borrow::<Velocity>();
        assert_eq!(
            vels.get(far.id()).expect("far velocity").linear,
            Vec3::ZERO,
            "a body the soft body never reached must not be pushed"
        );
        assert!(
            bodies.get(far.id()).expect("far").is_sleeping,
            "…nor woken — the wake is scoped to bodies that were actually pushed"
        );
    }

    /// The conservation oracle **through the real driver**, and the only test that touches the
    /// angular write-back at all.
    ///
    /// Sections 1–3 verify the solver as a pure function; they say nothing about how
    /// `apply_reaction_impulses` spends the result. That step is where a whole family of
    /// plausible mistakes lives, and none of them is visible to "the crate moved":
    ///
    /// * spending `impulse.linear` as a *velocity* instead of `impulse.linear · 1/M` (the crate
    ///   ends up `M`× too fast — here 5× — and the sum is 5× the truth);
    /// * spending it twice, or once per node instead of once per body;
    /// * dropping the angular half, or feeding `impulse.angular` to `inv_mass` / the linear
    ///   velocity by transposition.
    ///
    /// Two soft-body nodes at 60 and 55 m/s, **both above the centre line**, hit a 5 kg crate.
    /// Total linear momentum of the two nodes plus the crate must come out where it went in,
    /// and the crate must acquire a −Z spin (a shove above the middle tips it forward): the
    /// signs of the two nodes' torques agree by construction, so the assertion cannot be
    /// satisfied by cancellation.
    ///
    /// # Tolerance
    ///
    /// `1e-4 · scale`, `scale = Σ mᵢ|vᵢ|` ≈ 2.0e2 kg·m/s, i.e. ≈ 2.0e-2 kg·m/s absolute. Looser
    /// than section 3's `1e-5` for one specific reason: the driver rounds the momentum an extra
    /// time on each side of the exchange, spending `J·(1/M)` where the oracle then re-multiplies
    /// by `M`, and `1/5` is not representable in binary. That round-trip is worth ~1e-7 relative
    /// on a ~2e2 quantity, so the true floor is ~2e-5 — three orders under the bar — while every
    /// defect in the list above is off by a *factor*, i.e. by ~1e2 kg·m/s, four orders over it.
    #[test]
    fn the_driver_conserves_momentum_end_to_end() {
        let mut world = World::new();

        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        sb.damping = 1.0; // no velocity is lost between the integrate and the contact
        for (pos, vel, mass) in [
            (Vec3::new(0.0, 0.25, 0.0), Vec3::new(60.0, 0.0, 0.0), 2.0),
            (Vec3::new(0.0, 0.15, 0.0), Vec3::new(55.0, 3.0, -2.0), 1.5),
        ] {
            let idx = sb.add_node(pos, mass) as usize;
            sb.nodes[idx].velocity = vel;
        }
        let p_before = total_node_momentum(&sb);
        let scale = sb
            .nodes
            .iter()
            .fold(0.0_f32, |acc, n| acc + n.mass * n.velocity.length());
        let soft = spawn_soft(&mut world, sb);

        // A 5 kg unit crate, awake. Its −X face is at x = 0.55, so both nodes reach it inside
        // their swept distance and both contacts sit above the centre of mass at y = 0.
        const CRATE_MASS: f32 = 5.0;
        let mut rb = RigidBody::new(CRATE_MASS, false);
        rb.update_inertia_from_collider(&Collider::box_collider(Vec3::splat(0.5)));
        let target = spawn_rigid(
            &mut world,
            rb,
            Collider::box_collider(Vec3::splat(0.5)),
            Vec3::new(1.05, 0.0, 0.0),
        );

        // Zero gravity and a single step: nothing but the contact can change a velocity, which
        // is what makes the conservation claim a claim about the contact.
        soft_body_step_system(&world, DT, Vec3::ZERO);

        let meshes = world.borrow::<SoftBodyMesh>();
        let vels = world.borrow::<Velocity>();
        let stepped = meshes.get(soft.id()).expect("soft body");
        let vel = vels.get(target.id()).expect("crate velocity");

        // Non-vacuity: if the geometry ever drifts so the nodes stop reaching the crate, the
        // conservation assertion below becomes trivially true.
        assert!(
            vel.linear.x > 1.0,
            "the crate must actually have been shoved, got {:?}",
            vel.linear
        );
        assert!(
            vel.angular.z < -1.0,
            "both contacts are ABOVE the centre of mass, so the crate must tip forward \
             (−Z spin) — this is the only assertion in the suite that reads the angular \
             write-back at all; got {:?}",
            vel.angular
        );

        let p_after = total_node_momentum(stepped) + vel.linear * CRATE_MASS;
        let err = (p_after - p_before).length();
        assert!(
            err <= 1e-4 * scale,
            "the driver must conserve linear momentum: {p_before:?} -> {p_after:?} \
             (error {err}, tolerance {})",
            1e-4 * scale
        );
    }
}
