use gizmo_math::{Mat3, Vec3};
use gizmo_physics_core::BodyHandle;

use crate::coupling::{accumulate_impulse, reaction_for, NodeCollision, NodeImpact, RigidImpulse, RigidReaction};

/// Represents a single vertex/node in the FEM soft body mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftBodyNode {
    /// Current world-space position, in metres.
    pub position: Vec3,

    /// Current world-space velocity, in metres per second. Explicitly stored (unlike the
    /// Verlet-style [`crate::rope::RopeNode`]), so it can be read and written directly.
    pub velocity: Vec3,

    /// Mass in kilograms; must be strictly positive for the node to take part in the
    /// simulation. [`SoftBodyMesh::step`] skips any node whose mass is `<= 0.0` or NaN,
    /// treating it as immovable rather than dividing force by it and seeding Inf/NaN.
    pub mass: f32,

    /// Pinned. [`SoftBodyMesh::step`] never integrates this node, so its position and
    /// velocity are held exactly, but it still contributes its position to the deformation
    /// gradient — and therefore to the elastic forces — of every element that references it.
    pub is_fixed: bool, // For pinning parts of the body (e.g. chassis mounts)
}

/// A 3D Tetrahedron (4 nodes) used as the fundamental finite element.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tetrahedron {
    /// Indices into the nodes array of the parent SoftBody
    pub node_indices: [u32; 4],

    /// Rest Volume (V0) - calculated once at initialization
    pub rest_volume: f32,

    /// The inverse of the reference shape matrix (Dm^-1)
    /// Used by the GPU to quickly calculate the Deformation Gradient (F = Ds * Dm^-1)
    pub inv_rest_matrix: Mat3,
}

impl Tetrahedron {
    /// Calculate the inverse rest matrix (Dm^-1) and rest volume for the tetrahedron.
    /// Needs the 4 rest positions (x0, x1, x2, x3) of the nodes.
    pub fn calculate_rest_data(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3) -> (Mat3, f32) {
        // Dm = [ (p1-p0) | (p2-p0) | (p3-p0) ]
        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let e3 = p3 - p0;

        let dm = Mat3::from_cols(e1, e2, e3);

        // V = 1/6 * det(Dm)
        let det = dm.determinant();
        let volume = (det / 6.0).abs();

        let inv_dm = dm.inverse();

        (inv_dm, volume)
    }
}

/// The main CPU-side structure for an FEM-based Soft Body.
/// It contains the geometry and material properties needed to upload to the GPU.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SoftBodyMesh {
    /// The FEM vertices. A node's position in this vector *is* its identity: it is the index
    /// [`add_node`](Self::add_node) returns and the one [`Tetrahedron::node_indices`] stores.
    /// Reordering entries silently rewires the elements, and shortening the vector past an
    /// index an element still references makes [`step`](Self::step) panic on the
    /// out-of-bounds lookup.
    pub nodes: Vec<SoftBodyNode>,

    /// The tetrahedral elements; each contributes internal elastic forces to its four nodes
    /// on every [`step`](Self::step). Rest volume and reference shape are captured when
    /// [`add_element`](Self::add_element) is called, so the nodes must already be in their
    /// rest configuration at that point — moving them afterwards reads as deformation, not
    /// as a new rest shape.
    pub elements: Vec<Tetrahedron>,

    // --- Material Properties (Elasto-Plasticity) ---
    /// Young's Modulus (E) - Stiffness of the material (e.g., higher = stiffer metal)
    pub youngs_modulus: f32,

    /// Poisson's Ratio (nu) - Incompressibility (0.0 to 0.499, e.g., 0.3 for steel, 0.49 for rubber)
    pub poissons_ratio: f32,

    /// Lame's first parameter (lambda) - derived
    pub lambda: f32,

    /// Shear Modulus (mu) - derived
    pub mu: f32,

    /// Velocity damping as a per-SECOND RETENTION FACTOR, not a rate: [`step`](Self::step)
    /// applies `velocity *= damping.powf(dt)` once per step, so `damping` is the fraction of
    /// velocity a node keeps over one second regardless of how that second is subdivided.
    /// 1.0 is undamped, 0.0 stops the body dead; the default is 0.99 (1% of the velocity bled
    /// off per second). Values outside `[0, 1]` are not validated and amplify.
    ///
    /// The `gpu_compute` shader reads this same field and **must** apply the same
    /// `pow(damping, dt)`; it used to apply `max(1 - damping*dt, 0)` instead, which reads the
    /// field as a rate in 1/s and damped ~99x harder at the default. `tests/cpu_gpu_parity.rs`
    /// pins both halves.
    pub damping: f32,
}
#[cfg(feature = "ecs")]
gizmo_core::impl_component!(SoftBodyMesh);

/// Restitution of the node↔surface response: the fraction of the approach speed that comes
/// back out along the normal. Hard-coded and non-configurable, as it has always been.
const BOUNCE: f32 = 0.5;

/// Fraction of the *relative* tangential slip removed at the contact. Not a Coulomb
/// coefficient — there is no `μ·j_n` cone here, the tangential impulse is whatever it takes
/// to remove 80% of the slip. Preserved verbatim from the pre-coupling response; putting a
/// friction cone on it is a separate, behaviour-changing change.
const FRICTION: f32 = 0.8;

/// Sweeps a single soft-body node along its velocity for `dt` seconds, resolves the first
/// surface it would hit, and reports the reaction that surface's body is owed.
///
/// # What comes back
///
/// A [`NodeCollision`]. When [`impact`](NodeCollision::impact) is `None` nothing was hit and
/// the input `position`/`velocity` come back unchanged — the function never integrates, so in
/// that case the caller must advance the node itself. When it is `Some` the returned position
/// has already been moved along the sweep (never further than the swept distance) and the
/// returned velocity is the post-impact one; use them in place of your own integrated values.
/// The [`NodeImpact`] additionally names the [`BodyHandle`] that was hit and carries the
/// **equal and opposite** impulse owed to it — the omission the previous version of this
/// comment recorded, and the reason a soft body could not push a crate.
///
/// # The sweep
///
/// The swept distance is `velocity.length() * dt` in metres; 1e-5 m or less (a resting or
/// near-resting node) short-circuits to "no collision", which also covers a zero velocity.
/// That path does no work and emits no diagnostics — a settled soft body costs nothing and
/// logs nothing, however many nodes it has.
///
/// Every entry of `rigid_colliders` is ray-tested — a linear scan, there is no broad-phase —
/// and only the single *nearest* hit within `swept distance + 0.1 m` is resolved, so a node
/// facing two adjacent surfaces (a tiled floor, a wall meeting a floor) stops at the first
/// one instead of being displaced once per collider.
///
/// This is a swept stop, not a penetration solver: the correction only ever moves the node
/// *forward* along its own velocity, by `(hit distance − 0.1).max(0.0)` metres — a fixed
/// 0.1 m skin. Nothing here pushes a node back out along the surface normal.
///
/// # The response
///
/// `reactions` supplies the mass properties and velocity of the bodies behind the colliders,
/// looked up by handle with [`reaction_for`]; anything not in the table is
/// [`RigidReaction::IMMOVABLE`]. **Passing `&[]` reproduces the pre-coupling behaviour bit for
/// bit** (see below), which is what every caller without a rigid solver does.
///
/// With `n` the outward surface normal, `m` the node's mass, `r` the lever arm from the body's
/// centre of mass to the contact point, and `v_rel = v_node − (v_body + ω × r)` the relative
/// velocity *at the contact point*, the normal impulse is the textbook two-body one:
///
/// ```text
/// k_n = 1/m + 1/M + n · ((I⁻¹ (r × n)) × r)     [RigidReaction::effective_mass]
/// j_n = −(1 + e)·(v_rel · n) / k_n              ≥ 0, applied only while v_rel·n < 0
/// ```
///
/// and the tangential one removes a fixed fraction of the *relative* slip through the same
/// effective-mass denominator taken along the slip direction `t`:
///
/// ```text
/// j_t = −f·|v_rel − n(v_rel·n)| / k_t
/// ```
///
/// The total impulse `J = j_n·n + j_t·t` is then applied to **both** sides:
/// `v_node += J/m` here, and `−J` (plus `r × −J` about the centre of mass) is handed back in
/// the [`NodeImpact`] for the caller to spend on the body. Because the two halves are the same
/// `J`, total linear momentum is conserved by construction, and so is angular momentum about
/// any fixed point — up to the 0.1 m skin, since the node is parked short of the contact point
/// whose lever arm the body's share was computed with.
///
/// This is *not* the old response plus a push. The old response reflected the node as though
/// the surface could not move; adding a body push on top of that would have **created**
/// momentum, because the node kept the velocity change of an infinitely heavy collision while
/// the body took a second, unpaid one. Here the same `J` is what the node gets and what the
/// body is owed, so the exchange nets to zero however light the body is.
///
/// # Bodies that cannot react
///
/// A static or kinematic body, or a dynamic one with `mass == 0`, reports
/// [`can_react`](RigidReaction::can_react)` == false` and is handed a **zero** impulse — it is
/// never pushed. Its velocity still enters `v_rel`, though, so a moving kinematic platform now
/// kicks a node it sweeps into; before coupling every collider was assumed to be at rest.
///
/// # Exactness of the immovable path
///
/// When the resolved reaction is **inert** — [`can_react`](RigidReaction::can_react) is false
/// *and* the body is at rest — the node update takes a separate branch that evaluates the
/// *original* expression, operation for operation. The general formula agrees with it
/// analytically (`k_n` collapses to `1/m`, so `J/m` is exactly `−(1+e)(v·n)n − f·v_t`), but not
/// to the last bit — `j·(1/m)` rounds where the direct form does not, and `v_t − f·|v_t|·t`
/// rounds differently from `v_t·(1 − f)`. Keeping the old arithmetic means existing scenes
/// standing on level geometry are bit-for-bit unchanged. `tests/soft_rigid_coupling.rs` pins
/// the two forms against each other so they cannot silently diverge.
///
/// The branch is chosen on that *behaviour*, not on equality with
/// [`RigidReaction::IMMOVABLE`]. With both `inv_mass` and `inv_inertia` zero and both
/// velocities zero, [`velocity_at`](RigidReaction::velocity_at) returns `Vec3::ZERO` and
/// [`effective_mass`](RigidReaction::effective_mass) returns exactly `1/m` whatever
/// `center_of_mass` holds, so every such reaction is numerically indistinguishable from
/// `IMMOVABLE` and gets the legacy arithmetic.
/// That is what keeps a **stationary kinematic platform** (a door, a lift parked at a floor)
/// from drifting a ULP away from the pre-coupling engine merely because the driver now reports
/// its centre of mass. A kinematic body that is actually *moving* is not inert and takes the
/// general branch, which is the whole point of feeding its velocity in.
///
/// # `node_mass`
///
/// Kilograms, and the mass the impulse is divided by. A non-positive or non-finite mass has no
/// meaningful two-body solution, so such a node falls back to the immovable (one-sided)
/// response and the body is owed nothing; [`SoftBodyMesh::step`] never integrates such a node
/// in the first place.
pub fn resolve_node_collision(
    mut position: Vec3,
    mut velocity: Vec3,
    node_mass: f32,
    dt: f32,
    rigid_colliders: &[(
        BodyHandle,
        gizmo_physics_core::Transform,
        gizmo_physics_core::Collider,
    )],
    reactions: &[(BodyHandle, RigidReaction)],
) -> NodeCollision {
    let dist = velocity.length() * dt;
    if dist <= 1e-5 {
        return NodeCollision::miss(position, velocity);
    }

    // The `Ray` is built INSIDE the gate on purpose (it used to be built before it).
    //
    // `Ray::new` `warn!`s when it cannot normalise the direction and falls back to +Z,
    // and `velocity.normalize_or_zero()` returns exactly `Vec3::ZERO` for a velocity that
    // is zero — or merely subnormal, since `length_recip()` is then `inf` and the
    // fallback kicks in. A node in force equilibrium is damped geometrically toward zero
    // every step (`velocity *= damping.powf(dt)`), so it lands in exactly that state and
    // stays there: one WARN per settled node per step, drowning the log of any scene with
    // a soft body resting on the ground.
    //
    // The warn itself is correct where it lives — a zero direction genuinely is a caller
    // bug — so it is neither downgraded nor made once-only. What was wrong is that this
    // caller asked for a ray it was never going to use. Nothing below reads `ray` when
    // `dist <= 1e-5`, so the sweep, the nearest-hit pick and the response are bit-for-bit
    // unchanged; only the spurious log line goes away. A genuinely broken velocity
    // (infinite: `dist` is then `inf`, so the gate opens) still warns, as it should.
    let ray = gizmo_physics_core::raycast::Ray::new(position, velocity.normalize_or_zero());

    // Resolve against the SINGLE nearest hit. The old loop applied the position
    // snap once PER collider within range, so a node facing two adjacent surfaces
    // (a tiled floor, a wall meeting a floor) was advanced twice — launching it
    // straight PAST the geometry instead of stopping at the first surface.
    let mut nearest: Option<(f32, Vec3, BodyHandle)> = None;
    for (handle, col_trans, col) in rigid_colliders {
        if let Some((d, n)) =
            gizmo_physics_core::raycast::Raycast::ray_shape(&ray, &col.shape, col_trans)
        {
            if d <= dist + 0.1 && nearest.is_none_or(|(nd, _, _)| d < nd) {
                nearest = Some((d, n, *handle));
            }
        }
    }

    let Some((d, n, body)) = nearest else {
        return NodeCollision::miss(position, velocity);
    };

    let reaction = reaction_for(reactions, body);
    // The contact point is on the SURFACE. The node itself is parked 0.1 m short of it (the
    // skin), but the body's lever arm has to be measured where the push actually lands.
    let point = ray.origin + ray.direction * d;

    // "Inert" = cannot take an impulse AND is not moving. Such a reaction is numerically
    // indistinguishable from `RigidReaction::IMMOVABLE` — `velocity_at` is `Vec3::ZERO` and
    // `effective_mass` is exactly `1/m` however the centre of mass is placed — so it takes the
    // legacy branch and stays bit-for-bit identical to the pre-coupling engine. Testing the
    // behaviour rather than `reaction == IMMOVABLE` is what keeps a *stationary* kinematic
    // platform from drifting a ULP just because the driver now reports its COM.
    let inert = !reaction.can_react()
        && reaction.linear == Vec3::ZERO
        && reaction.angular == Vec3::ZERO;

    // `node_mass <= 0` has no two-body solution (1/m is Inf/NaN). `SoftBodyMesh::step` never
    // feeds one — it skips such nodes entirely — but this is a public entry point.
    let two_body = !inert && node_mass > 0.0 && node_mass.is_finite();

    let mut impact = NodeImpact {
        body,
        point,
        normal: n,
        impulse: Vec3::ZERO,
        angular_impulse: Vec3::ZERO,
    };

    if two_body {
        let inv_m = 1.0 / node_mass;
        let r = point - reaction.center_of_mass;
        let v_rel = velocity - reaction.velocity_at(point);
        let vn = v_rel.dot(n);

        // Only a node genuinely CLOSING on the surface is resolved. Measured on the relative
        // velocity, not the node's own: a node drifting slower than the surface it sits on is
        // separating, and pulling it back would be the classic sticky-contact bug.
        if vn < 0.0 {
            let mut j = Vec3::ZERO;

            let k_n = reaction.effective_mass(inv_m, r, n);
            if k_n > 0.0 {
                j += n * (-(1.0 + BOUNCE) * vn / k_n);
            }

            let vt = v_rel - n * vn;
            let vt_len = vt.length();
            if vt_len > 1e-6 {
                let t = vt / vt_len;
                let k_t = reaction.effective_mass(inv_m, r, t);
                if k_t > 0.0 {
                    j += t * (-FRICTION * vt_len / k_t);
                }
            }

            velocity += j * inv_m;

            // The other half. Withheld entirely from a body that cannot take it, so
            // "static/kinematic bodies are never pushed" is a property of this function and
            // not of whoever spends the result.
            if reaction.can_react() {
                impact.impulse = -j;
                impact.angular_impulse = r.cross(-j);
            }
        }
    } else {
        // Inert surface (or an unusable node mass): the ORIGINAL expression, verbatim.
        // Analytically this is the `1/M → 0, I⁻¹ → 0, v_body → 0` limit of the branch above;
        // kept as written so the float result is bit-identical to the pre-coupling engine.
        let vn = velocity.dot(n);
        if vn < 0.0 {
            let vt = velocity - n * vn;
            velocity = vt * (1.0 - FRICTION) - n * (vn * BOUNCE);
        }
    }

    position += ray.direction * (d - 0.1).max(0.0);

    NodeCollision {
        position,
        velocity,
        impact: Some(impact),
    }
}

/// Picks the `(position, velocity)` to store for a node that has ALREADY been integrated,
/// resolving it against `rigid_colliders` with [`resolve_node_collision`].
///
/// This is the whole post-integration half of a soft-body step, factored out because the two
/// integrators got it *differently*. [`SoftBodyMesh::step`] swept from the node's pre-step
/// position — correct: the sweep's own correction is what advances a colliding node, so total
/// displacement stays bounded by the swept distance. The GPU readback in
/// `gpu_compute::GpuCompute::step_soft_bodies` swept from the position the compute shader had
/// *already* advanced, so a colliding node was moved twice in one step (up to `2·|v|·dt`) and
/// could even bounce off a surface it never reached. Both now call this.
///
/// - `pre_step_position` — where the node was at the START of the step. This is the sweep
///   origin, and it is the reason the result never travels further than `|integrated_velocity|
///   · dt` from it.
/// - `integrated_position` — where the integrator put the node. Returned verbatim when nothing
///   was hit, discarded when something was (the sweep supersedes it).
/// - `integrated_velocity` — the post-force, post-damping velocity. It is both the sweep
///   direction and the input to the bounce/friction response.
///
/// See [`resolve_node_collision`] for the sweep itself, its `dist + 0.1 m` search window, its
/// hard-coded response coefficients and the meaning of `node_mass` / `reactions`. The
/// [`NodeImpact`] is passed straight through, so this is also where the caller picks up the
/// reaction owed to the rigid body.
pub fn resolve_swept_step(
    pre_step_position: Vec3,
    integrated_position: Vec3,
    integrated_velocity: Vec3,
    node_mass: f32,
    dt: f32,
    rigid_colliders: &[(
        BodyHandle,
        gizmo_physics_core::Transform,
        gizmo_physics_core::Collider,
    )],
    reactions: &[(BodyHandle, RigidReaction)],
) -> NodeCollision {
    let swept = resolve_node_collision(
        pre_step_position,
        integrated_velocity,
        node_mass,
        dt,
        rigid_colliders,
        reactions,
    );

    if swept.collided() {
        swept
    } else {
        NodeCollision::miss(integrated_position, integrated_velocity)
    }
}

impl SoftBodyMesh {
    /// Creates a new (empty) FEM soft body from material parameters.
    ///
    /// # Errors
    ///
    /// Returns [`SoftBodyError::InvalidYoungsModulus`] if `youngs_modulus` is
    /// not finite and strictly positive, and [`SoftBodyError::InvalidPoissonsRatio`]
    /// if `poissons_ratio` is outside the physically valid range `[0.0, 0.5)`.
    /// `nu >= 0.5` zeroes the Lamé denominator `(1 - 2·nu)` (incompressible
    /// limit → `+inf`), `nu > 0.5` yields a negative (unstable) `lambda`, and
    /// `nu < 0.0` is unsupported. Rejecting them here keeps the derived
    /// `lambda`/`mu` finite and stable instead of silently clamping.
    pub fn new(youngs_modulus: f32, poissons_ratio: f32) -> Result<Self, crate::SoftBodyError> {
        if !youngs_modulus.is_finite() || youngs_modulus <= 0.0 {
            return Err(crate::SoftBodyError::InvalidYoungsModulus {
                value: youngs_modulus,
            });
        }
        let nu = poissons_ratio;
        if !nu.is_finite() || !(0.0..0.5).contains(&nu) {
            return Err(crate::SoftBodyError::InvalidPoissonsRatio { value: nu });
        }

        // Calculate Lame Parameters
        let mu = youngs_modulus / (2.0 * (1.0 + nu));
        let lambda = (youngs_modulus * nu) / ((1.0 + nu) * (1.0 - 2.0 * nu));

        Ok(Self {
            nodes: Vec::new(),
            elements: Vec::new(),
            youngs_modulus,
            poissons_ratio: nu,
            lambda,
            mu,
            damping: 0.99,
        })
    }

    /// Appends a node at `position` (world-space metres) with mass `mass` (kilograms) and
    /// returns its index — the handle [`add_element`](Self::add_element) expects.
    ///
    /// Indices are handed out sequentially from `0` and are plain offsets into
    /// [`nodes`](Self::nodes), so mutating that vector directly invalidates them. The node
    /// starts at rest (zero velocity) and unpinned, and is treated as being in its rest
    /// configuration by any element added after it.
    ///
    /// `mass` is stored verbatim and is *not* validated here; a non-positive or NaN mass
    /// leaves the node inert in [`step`](Self::step) instead of failing loudly.
    pub fn add_node(&mut self, position: Vec3, mass: f32) -> u32 {
        let idx = self.nodes.len() as u32;
        self.nodes.push(SoftBodyNode {
            position,
            velocity: Vec3::ZERO,
            mass,
            is_fixed: false,
        });
        idx
    }

    /// Adds a tetrahedral element referencing four existing node indices.
    ///
    /// # Errors
    ///
    /// Returns [`SoftBodyError::NodeIndexOutOfBounds`] if any of the four
    /// indices refers to a node that has not been added yet. On success the
    /// element is appended exactly as before.
    pub fn add_element(
        &mut self,
        i0: u32,
        i1: u32,
        i2: u32,
        i3: u32,
    ) -> Result<(), crate::SoftBodyError> {
        let n = self.nodes.len() as u32;
        for index in [i0, i1, i2, i3] {
            if index >= n {
                return Err(crate::SoftBodyError::NodeIndexOutOfBounds {
                    index,
                    node_count: n,
                });
            }
        }

        let p0 = self.nodes[i0 as usize].position;
        let p1 = self.nodes[i1 as usize].position;
        let p2 = self.nodes[i2 as usize].position;
        let p3 = self.nodes[i3 as usize].position;

        let (inv_rest_matrix, rest_volume) = Tetrahedron::calculate_rest_data(p0, p1, p2, p3);

        // Reject (near-)degenerate tetrahedra at construction (fail-fast): a
        // near-zero rest volume means the four nodes are (nearly) coplanar, so
        // `Dm` is singular, `inv_rest_matrix` is undefined, and the derived
        // elastic forces would be near-zero / NaN — the element would never
        // recover from compression. Only well-conditioned elements are stored.
        const MIN_REST_VOLUME: f32 = 1e-6;
        if rest_volume <= MIN_REST_VOLUME || rest_volume.is_nan() || !inv_rest_matrix.is_finite() {
            return Err(crate::SoftBodyError::DegenerateTetrahedron {
                volume: rest_volume,
            });
        }

        self.elements.push(Tetrahedron {
            node_indices: [i0, i1, i2, i3],
            rest_volume,
            inv_rest_matrix,
        });
        Ok(())
    }

    /// Advances the FEM simulation by one timestep using a Neo-Hookean hyperelastic model.
    ///
    /// `dt` is in seconds and `gravity` is a world-space acceleration in m/s². Each node is
    /// loaded with `gravity * mass` plus the internal elastic force of every element that
    /// references it, then integrated with one semi-implicit (symplectic) Euler step —
    /// velocity first, damped by [`damping`](Self::damping)`.powf(dt)`, and the position
    /// advanced with that *new* velocity. `dt` is deliberately **not** clamped here: an
    /// explicit integrator on a stiff material goes unstable for large steps, so bounding
    /// the timestep is the caller's responsibility.
    ///
    /// Nodes that are [`is_fixed`](SoftBodyNode::is_fixed), or whose mass is `<= 0.0` or NaN,
    /// are skipped outright and stay put. An element is skipped for that step if the
    /// determinant of its deformation gradient is `<= 1e-4` (inverted or fully collapsed) or
    /// if it produces a non-finite force; every other element keeps its full stiffness no
    /// matter how compressed it is.
    ///
    /// Each moving node is then swept against `rigid_colliders` with
    /// [`resolve_node_collision`], and on a hit the swept position and velocity replace the
    /// integrated ones. Soft-vs-soft and self-collision are not handled at all.
    ///
    /// Element forces are computed in parallel (sequentially on `wasm32`) but accumulated in
    /// element order, so the result is deterministic and independent of thread scheduling.
    ///
    /// **Every collider is treated as immovable and at rest**, and the reaction the bodies
    /// behind them are owed is discarded. That is what this method has always done and it is
    /// left exactly so, bit for bit; [`step_coupled`](Self::step_coupled) is the two-way
    /// version.
    pub fn step(
        &mut self,
        dt: f32,
        gravity: Vec3,
        rigid_colliders: &[(
            BodyHandle,
            gizmo_physics_core::Transform,
            gizmo_physics_core::Collider,
        )],
    ) {
        let mut sink = Vec::new();
        self.step_coupled(dt, gravity, rigid_colliders, &[], &mut sink);
        debug_assert!(
            sink.is_empty(),
            "an empty reaction table can only produce immovable contacts"
        );
    }

    /// [`step`](Self::step), but the rigid bodies feel it.
    ///
    /// `reactions` describes the bodies behind `rigid_colliders` — mass, inverse world
    /// inertia, centre of mass and current velocity — keyed by the same [`BodyHandle`] the
    /// collider list carries. Anything absent from it is [`RigidReaction::IMMOVABLE`], so
    /// `&[]` is exactly [`step`](Self::step).
    ///
    /// Every impulse the nodes owe the bodies this step is **appended** to `out_impulses`,
    /// summed per body in first-touch order (see [`accumulate_impulse`]). The caller spends
    /// them:
    ///
    /// ```text
    /// v += inv_mass  · impulse.linear
    /// ω += inv_inertia · impulse.angular
    /// ```
    ///
    /// …and must WAKE any sleeping body it does so to, or the write is skipped by the
    /// integrator and swallowed — the same trap `physics_explosion_system` fell into. A body
    /// only appears in the list when it was actually pushed, which is the scope the wake
    /// should use.
    ///
    /// # Within-step staleness
    ///
    /// Every node of every soft body sees the *same* rigid-body velocity — the one in
    /// `reactions`, sampled before the step. This is a Jacobi-style exchange, not
    /// Gauss-Seidel: the second node to hit a crate does not see the velocity the first node
    /// just gave it. That costs accuracy for a many-node impact (the exchange is slightly
    /// over-counted, so the contact is a little too bouncy) but **not** conservation: each
    /// node's `+J` and the body's `−J` are the same number whatever velocity `J` was computed
    /// from, so the sum is zero regardless. The conservation oracle in
    /// `tests/soft_rigid_coupling.rs` therefore holds for a multi-node impact too.
    pub fn step_coupled(
        &mut self,
        dt: f32,
        gravity: Vec3,
        rigid_colliders: &[(
            BodyHandle,
            gizmo_physics_core::Transform,
            gizmo_physics_core::Collider,
        )],
        reactions: &[(BodyHandle, RigidReaction)],
        out_impulses: &mut Vec<RigidImpulse>,
    ) {
        let mut forces: Vec<Vec3> = self.nodes.iter().map(|n| gravity * n.mass).collect();

        // 1. Calculate and accumulate internal elastic forces from all tetrahedra in PARALLEL
        #[cfg(not(target_arch = "wasm32"))]
        use rayon::prelude::*;
        #[cfg(target_arch = "wasm32")]
        use crate::parallel_compat::*;

        let positions: Vec<Vec3> = self.nodes.iter().map(|n| n.position).collect();
        // Yalnızca gerçekten dejenere/ters (J ≤ eps) elemanlar atlanır — NaN/tekillik
        // koruması. Eskiden J < 0.1 ile GEÇERLİ ama sıkışmış elemanlar da tüm sertliğini
        // kaybediyordu (sıkışma altında çöküyor, geri toparlanamıyordu).
        const J_EPS: f32 = 1e-4;

        // Her tetrahedronun düğüm kuvvetlerini PARALEL hesapla, sonra DETERMİNİSTİK
        // (eleman sırasında) topla. (Paralel `reduce` float toplamı sırayı bozup
        // lockstep determinizmini kırıyordu — float toplaması birleşmeli değil.)
        let elem_forces: Vec<Option<([usize; 4], [Vec3; 4])>> = self
            .elements
            .par_iter()
            .map(|elem| {
                let idx = [
                    elem.node_indices[0] as usize,
                    elem.node_indices[1] as usize,
                    elem.node_indices[2] as usize,
                    elem.node_indices[3] as usize,
                ];
                let ds = Mat3::from_cols(
                    positions[idx[1]] - positions[idx[0]],
                    positions[idx[2]] - positions[idx[0]],
                    positions[idx[3]] - positions[idx[0]],
                );
                let f = ds * elem.inv_rest_matrix;
                let j = f.determinant();
                if j <= J_EPS {
                    return None;
                }
                let f_inv_t = f.inverse().transpose();
                let ln_j = j.ln();
                let p = f_inv_t * (-self.mu) + f * self.mu + f_inv_t * (self.lambda * ln_j);
                let h = p * elem.inv_rest_matrix.transpose() * elem.rest_volume;
                let f1 = -h.col(0);
                let f2 = -h.col(1);
                let f3 = -h.col(2);
                let f0 = -(f1 + f2 + f3);
                // NaN/Inf koruması: tutarsız kuvvet üretildiyse atla.
                if !(f0.is_finite() && f1.is_finite() && f2.is_finite() && f3.is_finite()) {
                    return None;
                }
                Some((idx, [f0, f1, f2, f3]))
            })
            .collect();

        for (idx, f) in elem_forces.into_iter().flatten() {
            forces[idx[0]] += f[0];
            forces[idx[1]] += f[1];
            forces[idx[2]] += f[2];
            forces[idx[3]] += f[3];
        }

        // 2. Integrate velocities and positions
        for (i, node) in self.nodes.iter_mut().enumerate() {
            // Skip pinned nodes and any node with non-positive / non-finite mass:
            // dividing `forces[i] / node.mass` by `mass <= 0` yields Inf/NaN that
            // then poisons velocity and position forever. Such a node behaves as
            // if it were fixed (immovable) rather than exploding the sim.
            if node.is_fixed || node.mass <= 0.0 || node.mass.is_nan() {
                continue;
            }

            // Explicit Euler Integration
            let acceleration = forces[i] / node.mass;
            node.velocity += acceleration * dt;

            // `damping` is a per-SECOND retention factor, so it is applied as `damping^dt` —
            // that is what makes the decay independent of how the second is subdivided. This
            // is the REFERENCE expression; `soft_body.wgsl`'s `integrate` pass must apply the
            // same `pow(damping, dt)` (it once applied `max(1 - damping*dt, 0)`, a rate
            // reading, which damped ~99x harder at the 0.99 default). Pinned on both sides by
            // `tests/cpu_gpu_parity.rs`.
            node.velocity *= self.damping.powf(dt);

            let next_pos = node.position + node.velocity * dt;

            // Same values, same order as the inlined form this replaced (sweep from the
            // PRE-step position, fall back to `next_pos` when nothing is hit, leave the
            // velocity alone in that case) — this is a pure extraction and changes no result.
            // It exists so the GPU readback path cannot drift away from it again.
            let swept = resolve_swept_step(
                node.position,
                next_pos,
                node.velocity,
                node.mass,
                dt,
                rigid_colliders,
                reactions,
            );
            node.position = swept.position;
            node.velocity = swept.velocity;

            // The half of the exchange the rigid body is owed. Accumulated in node order, so
            // the sum — and therefore the body's velocity next step — is replay-stable.
            if let Some(impact) = &swept.impact {
                accumulate_impulse(out_impulses, impact);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A VALID, moderately compressed element (J ≈ 0.4³ ≈ 0.064, below the old 0.1 threshold)
    /// must now resist and spring back. (The old code applied zero force once J < 0.1, so the
    /// element stayed collapsed.) It must also produce no NaN/Inf.
    #[test]
    fn resists_moderate_compression_and_stays_finite() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid material params");
        sb.add_node(Vec3::ZERO, 1.0);
        sb.add_node(Vec3::X, 1.0);
        sb.add_node(Vec3::Y, 1.0);
        sb.add_node(Vec3::Z, 1.0);
        sb.add_element(0, 1, 2, 3).expect("valid node indices");

        // Üniform sıkıştır: F = 0.4·I → J = 0.064 (< eski 0.1 eşiği).
        for node in &mut sb.nodes {
            node.position *= 0.4;
        }
        let d_before = (sb.nodes[1].position - sb.nodes[0].position).length();

        // Yerçekimsiz: yalnızca iç elastik kuvvetler etkili olsun.
        for _ in 0..120 {
            sb.step(1.0 / 240.0, Vec3::ZERO, &[]);
        }

        let d_after = (sb.nodes[1].position - sb.nodes[0].position).length();
        assert!(
            d_after > d_before + 1e-4,
            "sıkışmış eleman geri açılmalı (direnç): {d_before} -> {d_after}"
        );
        for node in &sb.nodes {
            assert!(node.position.is_finite(), "pozisyon NaN/Inf olmamalı");
            assert!(node.velocity.is_finite(), "hız NaN/Inf olmamalı");
        }
    }

    /// A node with mass 0 (and is_fixed=false) used to produce `forces/mass` = Inf/NaN and
    /// poison the whole simulation. Such a node is now skipped, as if it were fixed.
    #[test]
    fn zero_mass_node_does_not_poison_simulation() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid material params");
        // Kütlesiz (0.0) düğüm — pinlenmemiş.
        let zero_idx = sb.add_node(Vec3::new(0.5, 2.0, 0.5), 0.0) as usize;
        // Normal kütleli düğümler.
        sb.add_node(Vec3::X, 1.0);
        sb.add_node(Vec3::Y, 1.0);
        sb.add_node(Vec3::Z, 1.0);

        for _ in 0..30 {
            sb.step(1.0 / 240.0, Vec3::new(0.0, -9.81, 0.0), &[]);
        }

        for node in &sb.nodes {
            assert!(
                node.position.is_finite(),
                "kütlesiz düğüm Inf/NaN yaymamalı: {:?}",
                node.position
            );
            assert!(node.velocity.is_finite(), "hız Inf/NaN olmamalı");
        }
        // Kütlesiz düğüm hareket etmemeli (sabit gibi davranmalı).
        assert_eq!(sb.nodes[zero_idx].position, Vec3::new(0.5, 2.0, 0.5));
        assert_eq!(sb.nodes[zero_idx].velocity, Vec3::ZERO);
    }

    /// A degenerate tetrahedron built from four (nearly) coplanar nodes has rest_volume ≈ 0;
    /// `add_element` must now reject it.
    #[test]
    fn degenerate_tetrahedron_is_rejected() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid material params");
        // Dört düğüm de z=0 düzleminde → koplanar → hacim ≈ 0.
        sb.add_node(Vec3::new(0.0, 0.0, 0.0), 1.0);
        sb.add_node(Vec3::new(1.0, 0.0, 0.0), 1.0);
        sb.add_node(Vec3::new(0.0, 1.0, 0.0), 1.0);
        sb.add_node(Vec3::new(1.0, 1.0, 0.0), 1.0);

        let result = sb.add_element(0, 1, 2, 3);
        assert!(
            matches!(
                result,
                Err(crate::SoftBodyError::DegenerateTetrahedron { .. })
            ),
            "koplanar (dejenere) tetrahedron reddedilmeli, ama: {result:?}"
        );
        // Reddedilen eleman eklenmemeli.
        assert!(sb.elements.is_empty());
    }

    /// A node whose sweep ray hits two adjacent colliders (a tiled floor, a wall
    /// meeting a floor) must stop at the NEAREST surface, not be advanced once per
    /// collider — the old loop summed the per-collider snaps and launched the node
    /// clean through the geometry.
    #[test]
    fn resolve_node_collision_stops_at_nearest_of_adjacent_colliders() {
        use gizmo_physics_core::{BodyHandle, BoxShape, Collider, ColliderShape, Transform};

        let node = Vec3::ZERO;
        let vel = Vec3::new(60.0, 0.0, 0.0); // dist = 60 * (1/60) = 1.0
        let dt = 1.0 / 60.0;
        let thin = |cx: f32| {
            (
                BodyHandle::from_id(1),
                Transform::new(Vec3::new(cx, 0.0, 0.0)),
                Collider::from_shape(ColliderShape::Box(BoxShape {
                    half_extents: Vec3::new(0.05, 5.0, 5.0),
                })),
            )
        };
        // Near faces at x = 0.50 and x = 0.60 (both within the dist+0.1 window).
        let colliders = vec![thin(0.55), thin(0.65)];

        let out = resolve_node_collision(node, vel, 1.0, dt, &colliders, &[]);
        let pos = out.position;
        assert!(out.collided(), "the node should register a collision");
        // Nearest surface is at x=0.50 → snap to ~0.40. The old per-collider sum
        // pushed it to ~0.90, past BOTH boxes (which end at x=0.70) — a tunnel.
        assert!(
            pos.x < 0.5,
            "node must stop before the first surface, got x = {} (tunneled through?)",
            pos.x
        );
    }

    // ---------------------------------------------------------------------
    // Tetrahedron::calculate_rest_data — reference-shape math.
    // ---------------------------------------------------------------------

    /// The canonical unit tetrahedron (edges = the basis vectors) has rest volume 1/6 and an
    /// identity inverse reference matrix.
    #[test]
    fn calculate_rest_data_unit_tet() {
        let (inv_dm, vol) = Tetrahedron::calculate_rest_data(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z);
        assert!((vol - (1.0 / 6.0)).abs() < 1e-6, "unit tet volume must be 1/6, got {vol}");
        assert!(inv_dm.abs_diff_eq(Mat3::IDENTITY, 1e-6), "Dm = I → inv = I");
    }

    /// Volume is UNSIGNED (node winding must not flip its sign) and scales with the cube of a
    /// uniform edge scaling.
    #[test]
    fn calculate_rest_data_volume_is_unsigned_and_scales() {
        // Swapping two nodes negates det(Dm); volume must stay positive and equal.
        let (_, v_swapped) = Tetrahedron::calculate_rest_data(Vec3::ZERO, Vec3::Y, Vec3::X, Vec3::Z);
        assert!((v_swapped - (1.0 / 6.0)).abs() < 1e-6, "reversed winding must not flip volume");

        // Edges scaled x2 → volume x8.
        let (inv_dm, v_big) = Tetrahedron::calculate_rest_data(
            Vec3::ZERO,
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::new(0.0, 0.0, 2.0),
        );
        assert!((v_big - (8.0 / 6.0)).abs() < 1e-5, "scaled volume must be 8/6, got {v_big}");
        // inv(diag(2,2,2)) = diag(0.5,0.5,0.5).
        assert!(inv_dm.abs_diff_eq(Mat3::from_diagonal(Vec3::splat(0.5)), 1e-6));
    }

    /// The deformation gradient `F = Ds · Dm⁻¹` equals identity when the element is at its
    /// rest configuration — the fundamental FEM invariant.
    #[test]
    fn deformation_gradient_is_identity_at_rest() {
        let (p0, p1, p2, p3) = (Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z);
        let (inv_dm, _) = Tetrahedron::calculate_rest_data(p0, p1, p2, p3);
        let ds = Mat3::from_cols(p1 - p0, p2 - p0, p3 - p0); // == Dm at rest
        let f = ds * inv_dm;
        assert!(f.abs_diff_eq(Mat3::IDENTITY, 1e-5), "F must be identity at rest, got {f:?}");
        assert!((f.determinant() - 1.0).abs() < 1e-5, "J must be 1 at rest");
    }

    /// A uniaxial stretch of factor 2 along X yields `F = diag(2, 1, 1)` and `J = 2`.
    #[test]
    fn deformation_gradient_tracks_uniaxial_stretch() {
        let (p0, p1, p2, p3) = (Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z);
        let (inv_dm, _) = Tetrahedron::calculate_rest_data(p0, p1, p2, p3);
        // Deform: double every x coordinate.
        let (d0, d1, d2, d3) = (
            Vec3::ZERO,
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        );
        let ds = Mat3::from_cols(d1 - d0, d2 - d0, d3 - d0);
        let f = ds * inv_dm;
        assert!(f.abs_diff_eq(Mat3::from_diagonal(Vec3::new(2.0, 1.0, 1.0)), 1e-5), "got {f:?}");
        assert!((f.determinant() - 2.0).abs() < 1e-5, "J must equal the stretch factor");
    }

    // ---------------------------------------------------------------------
    // SoftBodyMesh::new — material validation & Lamé derivation.
    // ---------------------------------------------------------------------

    /// Valid parameters derive the Lamé coefficients from Young's modulus and Poisson's ratio.
    #[test]
    fn new_derives_lame_parameters() {
        let sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        // mu = E / (2(1+nu)); lambda = E*nu / ((1+nu)(1-2nu)).
        assert!((sb.mu - (1000.0 / 2.6)).abs() < 1e-2, "mu = {}", sb.mu);
        assert!((sb.lambda - (300.0 / 0.52)).abs() < 1e-1, "lambda = {}", sb.lambda);
        assert!((sb.damping - 0.99).abs() < 1e-6);
        assert!(sb.nodes.is_empty() && sb.elements.is_empty());

        // nu = 0 → lambda = 0, mu = E/2 (no volumetric coupling).
        let sb0 = SoftBodyMesh::new(1000.0, 0.0).expect("valid");
        assert!((sb0.lambda - 0.0).abs() < 1e-4);
        assert!((sb0.mu - 500.0).abs() < 1e-3);
    }

    /// Young's modulus must be finite and strictly positive.
    #[test]
    fn new_rejects_invalid_youngs_modulus() {
        for bad in [0.0, -5.0, f32::NAN, f32::INFINITY] {
            let r = SoftBodyMesh::new(bad, 0.3);
            assert!(
                matches!(r, Err(crate::SoftBodyError::InvalidYoungsModulus { .. })),
                "E = {bad} must be rejected, got {r:?}"
            );
        }
    }

    /// Poisson's ratio must lie in `[0.0, 0.5)`; the incompressible limit and negatives are
    /// rejected (they blow up or destabilize the Lamé denominator).
    #[test]
    fn new_rejects_invalid_poissons_ratio() {
        for bad in [-0.1, 0.5, 0.6, f32::NAN] {
            let r = SoftBodyMesh::new(1000.0, bad);
            assert!(
                matches!(r, Err(crate::SoftBodyError::InvalidPoissonsRatio { .. })),
                "nu = {bad} must be rejected, got {r:?}"
            );
        }
    }

    // ---------------------------------------------------------------------
    // Node / element construction.
    // ---------------------------------------------------------------------

    /// `add_node` returns sequential indices and initializes velocity to zero / unpinned.
    #[test]
    fn add_node_returns_sequential_indices_and_defaults() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        assert_eq!(sb.add_node(Vec3::X, 1.0), 0);
        assert_eq!(sb.add_node(Vec3::Y, 2.0), 1);
        assert_eq!(sb.add_node(Vec3::Z, 3.0), 2);
        assert_eq!(sb.nodes.len(), 3);
        let n = sb.nodes[1];
        assert!(n.position.abs_diff_eq(Vec3::Y, 1e-6));
        assert_eq!(n.mass, 2.0);
        assert_eq!(n.velocity, Vec3::ZERO);
        assert!(!n.is_fixed);
    }

    /// An element referencing a node index that does not exist yet is rejected and not stored.
    #[test]
    fn add_element_out_of_bounds_is_rejected() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        sb.add_node(Vec3::ZERO, 1.0);
        sb.add_node(Vec3::X, 1.0);
        sb.add_node(Vec3::Y, 1.0);
        // Index 3 is out of bounds (only 0..=2 exist).
        let r = sb.add_element(0, 1, 2, 3);
        assert_eq!(
            r,
            Err(crate::SoftBodyError::NodeIndexOutOfBounds { index: 3, node_count: 3 })
        );
        assert!(sb.elements.is_empty(), "rejected element must not be stored");
    }

    /// A well-conditioned tetrahedron is accepted and stored with the correct rest volume.
    #[test]
    fn add_element_accepts_valid_tet_and_stores_rest_volume() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        sb.add_node(Vec3::ZERO, 1.0);
        sb.add_node(Vec3::X, 1.0);
        sb.add_node(Vec3::Y, 1.0);
        sb.add_node(Vec3::Z, 1.0);
        sb.add_element(0, 1, 2, 3).expect("valid tet");
        assert_eq!(sb.elements.len(), 1);
        assert!((sb.elements[0].rest_volume - (1.0 / 6.0)).abs() < 1e-6);
        assert_eq!(sb.elements[0].node_indices, [0, 1, 2, 3]);
    }

    // ---------------------------------------------------------------------
    // resolve_node_collision — single-hit sweep resolution.
    // ---------------------------------------------------------------------

    /// A (near-)stationary node travels no distance, so collision resolution is skipped and
    /// state is returned unchanged.
    #[test]
    fn resolve_node_collision_zero_velocity_is_noop() {
        use gizmo_physics_core::{BodyHandle, Collider, Transform};
        let colliders = vec![(
            BodyHandle::from_id(1),
            Transform::new(Vec3::new(0.5, 0.0, 0.0)),
            Collider::sphere(1.0),
        )];
        let pos = Vec3::new(0.0, 1.0, 0.0);
        let out = resolve_node_collision(pos, Vec3::ZERO, 1.0, 1.0 / 60.0, &colliders, &[]);
        assert!(!out.collided());
        assert_eq!(out.position, pos);
        assert_eq!(out.velocity, Vec3::ZERO);
    }

    /// A surface beyond the node's swept distance (plus the small margin) is ignored: no
    /// collision is registered and the position is left for the caller to advance.
    #[test]
    fn resolve_node_collision_out_of_range_is_noop() {
        use gizmo_physics_core::{BodyHandle, Collider, Transform};
        let colliders = vec![(
            BodyHandle::from_id(1),
            Transform::new(Vec3::new(2.0, 0.0, 0.0)), // sphere surface at x = 1.5
            Collider::sphere(0.5),
        )];
        // Slow node: dist = 1 * (1/60) ≈ 0.017, far short of the x=1.5 surface.
        let out = resolve_node_collision(
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            1.0,
            1.0 / 60.0,
            &colliders,
            &[],
        );
        assert!(!out.collided(), "surface out of sweep range must not collide");
        assert_eq!(
            out.position,
            Vec3::ZERO,
            "position must be untouched when nothing is hit"
        );
    }

    /// A node sweeping into a sphere stops just short of the surface and reflects its inward
    /// velocity component (bounce), leaving the state finite.
    #[test]
    fn resolve_node_collision_bounces_off_sphere() {
        use gizmo_physics_core::{BodyHandle, Collider, Transform};
        let colliders = vec![(
            BodyHandle::from_id(1),
            Transform::new(Vec3::new(2.0, 0.0, 0.0)),
            Collider::sphere(0.5), // near surface at x = 1.5
        )];
        // dist = 120 * (1/60) = 2 > 1.5 → the ray reaches the surface.
        let out = resolve_node_collision(
            Vec3::ZERO,
            Vec3::new(120.0, 0.0, 0.0),
            1.0,
            1.0 / 60.0,
            &colliders,
            &[],
        );
        let (p, v) = (out.position, out.velocity);
        let impact = out.impact.expect("the node must register the sphere hit");
        assert!(p.x < 1.5, "node must stop before the surface, got x = {}", p.x);
        assert!(v.x < 0.0, "inward velocity must reflect (bounce), got vx = {}", v.x);
        assert!(p.is_finite() && v.is_finite());
        // The sweep now names the collider it hit — the omission this function's doc comment
        // used to record. With no reaction data the surface is immovable, so it is owed
        // nothing.
        assert_eq!(impact.body, BodyHandle::from_id(1));
        assert_eq!(impact.impulse, Vec3::ZERO);
        assert!(
            (impact.point.x - 1.5).abs() < 1e-4,
            "the contact point is on the SURFACE (x = 1.5), not where the node parked; got {}",
            impact.point.x
        );
    }

    /// Counts the WARN-level `tracing` events dispatched on the current thread.
    ///
    /// Hand-rolled because this crate has no `tracing-subscriber` dev-dependency and the test
    /// below needs nothing more than a tally. `tracing::subscriber::with_default` installs it
    /// per-thread, so a concurrently running test cannot contribute to the count.
    struct WarnCounter(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl tracing::Subscriber for WarnCounter {
        fn enabled(&self, _meta: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            if *event.metadata().level() == tracing::Level::WARN {
                self.0
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        fn enter(&self, _span: &tracing::span::Id) {}
        fn exit(&self, _span: &tracing::span::Id) {}
    }

    /// A node that is not going anywhere must resolve **silently**.
    ///
    /// `Ray::new` warns (correctly) when it cannot normalise a direction. This function used
    /// to build its `Ray` before the `dist > 1e-5` short-circuit, so every settled node fed it
    /// `Vec3::ZERO` and drew one WARN per node per step — the log of any scene with a soft
    /// body lying on the ground was unreadable. The ray is now built inside the gate.
    ///
    /// The positive control below is deliberate: it fires the exact callsite under test, so a
    /// harness that silently observed nothing (a mis-wired subscriber, a filtered callsite)
    /// fails here instead of turning the real assertion into a vacuous one.
    #[test]
    fn resting_node_sweep_logs_nothing() {
        use gizmo_physics_core::{BodyHandle, Collider, Transform};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        // A floor right under the node — the collider list is non-empty so the test exercises
        // the same shape of call a real resting soft body makes.
        let colliders = vec![(
            BodyHandle::from_id(1),
            Transform::new(Vec3::new(0.0, -1.0, 0.0)),
            Collider::sphere(0.5),
        )];
        let rest_pos = Vec3::new(0.0, 0.4, 0.0);
        let dt = 1.0 / 60.0;

        let seen = Arc::new(AtomicUsize::new(0));
        let counter = WarnCounter(Arc::clone(&seen));

        tracing::subscriber::with_default(counter, || {
            // Positive control: this IS the callsite the regression is about.
            let fallback = gizmo_physics_core::raycast::Ray::new(Vec3::ZERO, Vec3::ZERO);
            assert_eq!(
                fallback.direction,
                Vec3::Z,
                "Ray::new must still fall back to +Z on a zero direction"
            );
            assert_eq!(
                seen.swap(0, Ordering::Relaxed),
                1,
                "the counter must observe Ray::new's warn, or the assertion below proves nothing"
            );

            // Exactly zero, and subnormal — the two velocities `normalize_or_zero` maps to
            // `Vec3::ZERO`. Geometric damping walks an equilibrium node through both.
            for velocity in [Vec3::ZERO, Vec3::new(0.0, -1e-40, 0.0)] {
                for _ in 0..64 {
                    let out = resolve_node_collision(rest_pos, velocity, 1.0, dt, &colliders, &[]);
                    assert!(!out.collided(), "a resting node must not register a collision");
                    assert_eq!(out.position, rest_pos, "position must come back untouched");
                    assert_eq!(out.velocity, velocity, "velocity must come back untouched");
                }
            }
        });

        let warns = seen.load(Ordering::Relaxed);
        assert_eq!(
            warns, 0,
            "a resting node must resolve silently; got {warns} WARN(s) over 128 sweeps"
        );
    }

    // ---------------------------------------------------------------------
    // SoftBodyMesh::step — integration behaviour.
    // ---------------------------------------------------------------------

    /// With no elements the step is pure gravity integration: a free node accelerates downward.
    #[test]
    fn step_free_node_falls_under_gravity() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        sb.add_node(Vec3::new(0.0, 10.0, 0.0), 1.0);
        sb.step(0.1, Vec3::new(0.0, -10.0, 0.0), &[]);
        assert!(sb.nodes[0].velocity.y < 0.0, "gravity must accelerate downward");
        assert!(sb.nodes[0].position.y < 10.0, "node must descend");
        assert!(sb.nodes[0].position.is_finite() && sb.nodes[0].velocity.is_finite());
    }

    /// Fixed nodes and nodes with non-positive mass are skipped entirely (they never move and
    /// never inject NaN/Inf), while a normal free node in the same body still integrates.
    #[test]
    fn step_fixed_and_negative_mass_nodes_do_not_move() {
        let mut sb = SoftBodyMesh::new(1000.0, 0.3).expect("valid");
        let fixed = sb.add_node(Vec3::new(0.0, 5.0, 0.0), 1.0) as usize;
        sb.nodes[fixed].is_fixed = true;
        let neg = sb.add_node(Vec3::new(1.0, 5.0, 0.0), -2.0) as usize; // negative mass
        let free = sb.add_node(Vec3::new(2.0, 5.0, 0.0), 1.0) as usize;

        let (fixed0, neg0) = (sb.nodes[fixed].position, sb.nodes[neg].position);
        for _ in 0..20 {
            sb.step(1.0 / 60.0, Vec3::new(0.0, -9.81, 0.0), &[]);
        }
        assert_eq!(sb.nodes[fixed].position, fixed0, "fixed node must not move");
        assert_eq!(sb.nodes[fixed].velocity, Vec3::ZERO);
        assert_eq!(sb.nodes[neg].position, neg0, "negative-mass node must be inert");
        assert_eq!(sb.nodes[neg].velocity, Vec3::ZERO);
        assert!(sb.nodes[free].position.y < 5.0, "the healthy free node must still fall");
        for n in &sb.nodes {
            assert!(n.position.is_finite() && n.velocity.is_finite());
        }
    }

    /// The FEM step is deterministic: parallel element force accumulation is summed in a fixed
    /// (element) order, so two identical bodies stay bit-identical.
    #[test]
    fn softbody_step_is_deterministic() {
        let build = || {
            let mut sb = SoftBodyMesh::new(1.0e5, 0.3).expect("valid");
            for p in [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z] {
                sb.add_node(p, 1.0);
            }
            sb.add_element(0, 1, 2, 3).expect("valid");
            // Perturb so internal forces are non-trivial.
            sb.nodes[1].position *= 1.3;
            sb
        };
        let (mut a, mut b) = (build(), build());
        for _ in 0..120 {
            a.step(1.0 / 120.0, Vec3::new(0.0, -9.81, 0.0), &[]);
            b.step(1.0 / 120.0, Vec3::new(0.0, -9.81, 0.0), &[]);
        }
        assert_eq!(a, b, "identical soft bodies must evolve identically");
    }
}
