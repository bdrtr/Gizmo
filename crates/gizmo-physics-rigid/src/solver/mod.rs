use crate::components::{RigidBody, Velocity};
use gizmo_math::Vec3;
use gizmo_physics_core::components::Transform;
/// AAA Constraint Solver — Sequential Impulses (SI) with:
/// - Warm-starting (re-apply the previous frame's impulses)
/// - Accumulated-impulse clamping (no negative normal impulse)
/// - Coulomb friction cone (static + dynamic)
/// - Speculative contacts (contact resolved before penetration happens)
/// - 2-dimensional friction (two tangent directions)
/// - Restitution threshold (suppresses micro-bounce)
/// - Configurable solver iteration count
use gizmo_physics_core::ContactManifold;

mod block;
mod parallel;
mod standalone;
mod tgs;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
#[cfg(target_arch = "wasm32")]
use crate::parallel_compat::*;

/// The lever a contact's rolling resistance acts through: how far the rolling body's centre of
/// mass stands off the contact along its normal — a ball's radius, a wheel's — averaged over the
/// moving sides of the pair.
pub(super) fn rolling_lever(r_a: Vec3, r_b: Vec3, normal: Vec3, (dyn_a, dyn_b): (bool, bool)) -> f32 {
    let (a, b) = (r_a.dot(normal).abs(), r_b.dot(normal).abs());
    match (dyn_a, dyn_b) {
        (true, true) => 0.5 * (a + b),
        (true, false) => a,
        (false, true) => b,
        (false, false) => 0.0,
    }
}

/// One pass of the rolling-resistance row for a contact (see `tgs::apply_rolling` for why it
/// exists): oppose the pair's relative spin with a torque impulse, bounded like friction by the
/// contact's normal impulse `acc_n` times `rolling` — through `lever`, the rolling body's radius
/// ([`rolling_lever`]), so `rolling` is the textbook coefficient: a resisting force `rolling · N`
/// at the contact, a torque `rolling · N · r`. `acc_r` is the row's impulse accumulated so far
/// this sweep; the new total is returned. Both contact solvers run it — the TGS path and the
/// split-impulse path an island with a CCD body takes.
///
/// Until 2026-11-23 the bound had no lever: a force-sized impulse spent as a torque, as though
/// every ball were a metre across. The resistance went as `1/r` — a billiard ball (28.6 mm) was
/// braked 35 times harder than its coefficient said, its spin killed at once and the ball left
/// sliding to a halt at the sliding friction's `μ·g`; a 0.5 m sphere twice as hard, which is the
/// "about 1.5× `c·g`" `tests/rolling_and_settling.rs` measured (a rolling solid sphere's is
/// `c·g / 1.4`).
pub(super) fn rolling_row(
    velocities: &mut [Velocity],
    (idx_a, idx_b): (usize, usize),
    (dyn_a, dyn_b): (bool, bool),
    (inv_i_a, inv_i_b): (gizmo_math::Mat3, gizmo_math::Mat3),
    (rolling, lever): (f32, f32),
    acc_n: f32,
    acc_r: Vec3,
) -> Vec3 {
    if rolling <= 0.0 || acc_n <= 0.0 {
        return acc_r;
    }
    let w_rel = velocities[idx_b].angular - velocities[idx_a].angular;
    let len = w_rel.length();
    if len < 1e-8 {
        return acc_r;
    }
    let axis = w_rel / len;
    // Effective angular mass along the spin axis, the rotational twin of `k_n`.
    let k = axis.dot(inv_i_a.mul_vec3(axis)) + axis.dot(inv_i_b.mul_vec3(axis));
    if k < 1e-12 {
        return acc_r;
    }
    let mut acc = acc_r - axis * (len / k);
    let max = rolling * acc_n * lever;
    if acc.length() > max {
        acc = acc.normalize_or_zero() * max;
    }
    let applied = acc - acc_r;
    if dyn_a {
        velocities[idx_a].angular -= inv_i_a.mul_vec3(applied);
    }
    if dyn_b {
        velocities[idx_b].angular += inv_i_b.mul_vec3(applied);
    }
    acc
}

// ─────────────────────────────────────────────────────────────────────────────
// Support-order (topological) contact ordering.
// ─────────────────────────────────────────────────────────────────────────────
//
// Sorts an island's contacts into SUPPORT order — closest to a static/kinematic
// anchor first, propagating outward. Support depth = graph distance (in contacts)
// from the nearest anchor, via a multi-source BFS over the island's contact graph.
// For a ground-anchored vertical stack this is exactly bottom-up: the ground contact,
// then box1↔box2, then box2↔box3, … It generalises to trees/piles (no fragile "is
// this a 1-D chain?" test).
//
// The value it DOES deliver: a DETERMINISTIC TOTAL ORDER keyed on
// (max_depth, min_depth, canonical entity pair) that is INDEPENDENT of broadphase
// pair-emission order → the island solve becomes pair-order-invariant, which is the
// property that unblocks incremental broadphase (docs/ENGINE.md §7).
//
// What it was HOPED to deliver but does NOT: fixing the tall-stack instability. The
// theory was that a vertical N-stack's Delassus matrix is a 1-D Laplacian with O(N²)
// condition number, so support-order GS would converge it in ~O(N) sweeps instead of
// O(N²). Empirically REFUTED (2026-07-14): ordering barely moves the blow-up frame,
// and the blow-up is chaotic (not monotonic) in iteration count — the instability is a
// metastable normal-channel resonance, not under-convergence. See the root-cause note
// in crates/gizmo-physics-rigid/tests/soak_and_golden.rs. Hence default OFF.
/// Returns the island's maximum support depth (graph distance in contacts from the
/// nearest anchor) — used to scale the block solver's iteration count so a tall stack
/// gets enough sweeps for support to propagate up the column. When `reorder` is true,
/// also permutes `manifolds` into support order in place.
// The bare `FxHashMap` here is deliberate and is NOT a leak: this fn is private to the
// module, so `rustc-hash` never reaches the public surface through it (docs/ENGINE.md §4).
// `ConstraintSolver::solve_contacts` is the public entry point and takes the opaque
// `EntityIndexMap`, unwrapping it with the `pub(crate)` `raw()` on the way in.
/// Scratch buffers for [`support_order_manifolds`], kept per thread.
///
/// The function allocated **one `Vec` per body in the island, plus seven** on every call —
/// and it is called once per island per substep, so a thousand-body island cost about four
/// thousand allocations a frame. An allocation profile put it at **32 % of all
/// allocations** in an awake scene, the single largest source, and well ahead of the
/// contact plumbing that looked like the obvious suspect.
///
/// Thread-local rather than a field on the solver because `solve_contacts` takes `&self`
/// (islands are solved in parallel from a shared configuration value, deliberately), so
/// there is nowhere on the solver to put mutable scratch without changing that contract.
///
/// The buffers never shrink: a scratch buffer's whole job is to keep the high-water mark.
#[derive(Default)]
struct OrderScratch {
    local: rustc_hash::FxHashMap<usize, u32>,
    global: Vec<usize>,
    is_anchor: Vec<bool>,
    medges: Vec<Option<(u32, u32)>>,
    adj: Vec<Vec<u32>>,
    depth: Vec<u32>,
    queue: std::collections::VecDeque<u32>,
    order: Vec<usize>,
    pos: Vec<usize>,
}

thread_local! {
    static ORDER_SCRATCH: std::cell::RefCell<OrderScratch> =
        std::cell::RefCell::new(OrderScratch::default());
}

fn support_order_manifolds(
    manifolds: &mut [ContactManifold],
    rigid_bodies: &[RigidBody],
    entity_index_map: &rustc_hash::FxHashMap<u32, usize>,
    reorder: bool,
) -> u32 {
    let n = manifolds.len();
    if n < 2 {
        return n as u32; // 0 or 1 contact: trivially ordered, depth ≤ 1.
    }

    // ── 1) Intern distinct bodies → compact local indices; record anchors + edges. ──
    // `medges[i]` = local endpoints of manifold i (None if an endpoint isn't mapped —
    // those keep last, deterministically by entity pair, matching the solver's own
    // `continue` on an unmapped manifold).
    ORDER_SCRATCH.with(|sc| {
    let sc = &mut *sc.borrow_mut();
    let local = &mut sc.local;
    let global = &mut sc.global;
    let is_anchor = &mut sc.is_anchor;
    let medges = &mut sc.medges;
    local.clear();
    global.clear();
    is_anchor.clear();
    medges.clear();
    medges.reserve(n);

    for m in manifolds.iter() {
        let (ga, gb) = match (
            entity_index_map.get(&m.entity_a.id()),
            entity_index_map.get(&m.entity_b.id()),
        ) {
            (Some(&a), Some(&b)) => (a, b),
            _ => {
                medges.push(None);
                continue;
            }
        };
        let mut ends = [0u32; 2];
        for (slot, &gidx) in ends.iter_mut().zip([ga, gb].iter()) {
            *slot = match local.get(&gidx) {
                Some(&li) => li,
                None => {
                    let li = global.len() as u32;
                    local.insert(gidx, li);
                    global.push(gidx);
                    is_anchor.push(!rigid_bodies[gidx].is_dynamic());
                    li
                }
            };
        }
        medges.push(Some((ends[0], ends[1])));
    }

    let v = global.len();
    // Dış vektör asla küçültülmez ve iç vektörler `clear` edilir: kapasiteleri korunur,
    // yani kalıcı durumda düğüm başına tahsis sıfır.
    let adj = &mut sc.adj;
    if adj.len() < v {
        adj.resize_with(v, Vec::new);
    }
    for a in adj[..v].iter_mut() {
        a.clear();
    }
    for &(a, b) in medges.iter().flatten() {
        adj[a as usize].push(b);
        adj[b as usize].push(a);
    }

    // ── 2) Multi-source BFS from anchors → support depth (min contacts to an anchor). ──
    // BFS yields the min graph distance regardless of visitation order, so `depth` is a
    // deterministic function of the island's contact graph.
    const INF: u32 = u32::MAX;
    let depth = &mut sc.depth;
    depth.clear();
    depth.resize(v, INF);
    let queue = &mut sc.queue;
    queue.clear();
    for li in 0..v {
        if is_anchor[li] {
            depth[li] = 0;
            queue.push_back(li as u32);
        }
    }
    // Anchor-free island (e.g. two boxes colliding mid-air): root the BFS at the body
    // with the lowest GLOBAL index — deterministic, so the order stays pair-invariant.
    if queue.is_empty() {
        let mut root = 0u32;
        let mut best = usize::MAX;
        for (li, &g) in global.iter().enumerate() {
            if g < best {
                best = g;
                root = li as u32;
            }
        }
        depth[root as usize] = 0;
        queue.push_back(root);
    }
    let mut max_depth = 0u32;
    while let Some(u) = queue.pop_front() {
        let du = depth[u as usize];
        max_depth = max_depth.max(du);
        for &w in &adj[u as usize] {
            if depth[w as usize] == INF {
                depth[w as usize] = du + 1;
                queue.push_back(w);
            }
        }
    }

    if !reorder {
        return max_depth; // caller only wanted the depth (e.g. for adaptive iterations).
    }

    // ── 3) Sort key per manifold → deterministic total order. ──
    // (max_depth, min_depth, min_entity_id, max_entity_id): anchor-closest contact
    // first; the canonical entity pair is unique per manifold so the key is a strict
    // total order → independent of the input (emission) order.
    let key_of = |i: usize| -> (u32, u32, u32, u32) {
        let ea = manifolds[i].entity_a.id();
        let eb = manifolds[i].entity_b.id();
        let (ida, idb) = if ea <= eb { (ea, eb) } else { (eb, ea) };
        match medges[i] {
            Some((la, lb)) => {
                let (da, db) = (depth[la as usize], depth[lb as usize]);
                let (lo, hi) = if da <= db { (da, db) } else { (db, da) };
                (hi, lo, ida, idb)
            }
            None => (INF, INF, ida, idb),
        }
    };
    let order = &mut sc.order;
    order.clear();
    order.extend(0..n);
    order.sort_unstable_by_key(|&i| key_of(i));

    // ── 4) Apply the permutation to `manifolds` in place (no clone of the Vec-bearing
    // ContactManifold). `pos[orig] = destination slot`; cycle-swap into place. ──
    let pos = &mut sc.pos;
    pos.clear();
    pos.resize(n, 0);
    for (slot, &orig) in order.iter().enumerate() {
        pos[orig] = slot;
    }
    for i in 0..n {
        while pos[i] != i {
            let target = pos[i];
            manifolds.swap(i, target);
            pos.swap(i, target);
        }
    }

    max_depth
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Konfigürasyon
// ─────────────────────────────────────────────────────────────────────────────

/// Tuning for the contact constraint solver.
///
/// This is configuration, not state: it is `Copy` and carries nothing from one substep to
/// the next. The accumulated normal/tangent impulses live on the contact points inside the
/// manifolds, which is what makes it safe to hand the same solver value to every island
/// being solved in parallel.
///
/// Two solve paths are selected by these settings — the default TGS-soft path
/// ([`ConstraintSolver::use_tgs_soft`]) and the older split-impulse sequential-impulse
/// path, which also takes over for any island containing a CCD-enabled body. Several
/// fields are finished-but-gated experiments; each says so in its own documentation, so
/// read the field before flipping it because it sounded useful.
///
/// The values are simulation *inputs*: a replay or a rollback only reproduces the original
/// trajectory if the solver is configured identically. They are not captured in the
/// world's serialized state or in a rollback snapshot, so keeping them in sync is the
/// caller's job.
///
/// `#[non_exhaustive]`: start from [`Default`] or [`ConstraintSolver::new`] and adjust
/// individual fields.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ConstraintSolver {
    /// PGS iteration count (more = more stable, slower)
    pub iterations: usize,
    /// Baumgarte stabilisation factor (0.1..0.3 is the useful range).
    /// Used as the fallback while Split Impulse is off.
    pub baumgarte: f32,
    /// Penetration tolerance — this much overlap is accepted as normal
    pub slop: f32,
    /// Warm-start factor (0.8 = re-apply 80 % of the previous frame's impulse)
    pub warm_start_factor: f32,
    /// Below this closing speed restitution is forced to zero (resting contact)
    pub restitution_velocity_threshold: f32,
    /// Maximum positional correction per step (metres) — keeps corrections from exploding
    pub max_linear_correction: f32,
    /// Split Impulse (pseudo-velocity) — resolves penetration on a separate
    /// pseudo-velocity channel instead of polluting the real velocity.
    /// Buys stacking stability and removes resting-contact jitter.
    /// (Unused while TGS Soft is on; the old path stays as a fallback.)
    pub split_impulse_enabled: bool,
    /// Split Impulse penetration recovery rate (0.1..0.4 is the useful range)
    pub split_impulse_erp: f32,

    // ── TGS Soft (the modern solver) ──────────────────────────────────────
    /// Use the TGS Soft path (soft constraints + relax pass). While on, split-impulse is
    /// disabled and the positional correction is written into `pos_corrections` as the soft
    /// bias's velocity contribution, `(biased − relaxed)·dt`. It holds long stacks under
    /// high-energy impacts (n≥16) where plain SI cannot.
    pub use_tgs_soft: bool,
    /// Contact softness frequency (Hz). Higher = stiffer/less penetration; lower =
    /// softer/steadier. Box2D v3 defaults to 30 Hz. Also clamped against the substep rate.
    pub contact_hertz: f32,
    /// Contact damping ratio (ζ). >1 is over-damped (no bounce, steady stacks). ~10.
    pub contact_damping_ratio: f32,
    /// Relax (bias=0) iteration count — drains the velocity the soft bias injected.
    pub relax_iterations: usize,
    /// Maximum soft-bias velocity (m/s) — caps how fast deep penetration is pushed out
    /// (no explosions). Box2D v3 uses ≈ 3·lengthUnits.
    pub max_bias_velocity: f32,

    /// Sort contacts into a DETERMINISTIC anchor-based (bottom-up) support order before
    /// solving → an island's solution becomes INDEPENDENT of broadphase pair-emission order
    /// (pair-order-invariant). That is the sleep-determinism property incremental broadphase
    /// depends on (docs/ENGINE.md §7).
    ///
    /// **ON BY DEFAULT** (`Default` yields `true`). This comment claimed "OFF BY DEFAULT" for a
    /// long time and was wrong — corrected on 2026-08-06. The difference matters to anyone
    /// investigating stack stability: the ordering is live, so island solutions are already
    /// independent of pair-emission order and "should we try turning ordering on?" is a closed
    /// question.
    ///
    /// It was originally expected to fix tall-stack instability (the Stage 1 "linchpin"), and
    /// measurement REFUTED that: ordering barely moves the frame at which a stack explodes (see
    /// the root-cause note in soak_and_golden.rs — the instability is a metastable resonance,
    /// not under-convergence). What it does deliver is a deterministic total order independent
    /// of pair emission, which is the sleep-determinism property the incremental-broadphase
    /// work needs.
    ///
    /// Note: turning it off does NOT remove the `island_depth` computation — with `block_solver`
    /// on the BFS still runs (only the permutation is skipped), because the adaptive sweep count
    /// is derived from depth.
    pub support_ordering: bool,

    /// Rotating anchors (a Box2D v3 technique): recompute contact separation and the Jacobian
    /// arms (r_a/r_b) each sweep from the accumulated delta ROTATION, which removes the
    /// frozen-anchor linearisation drift. OFF BY DEFAULT: it does NOT fix resting-stack
    /// instability (measured: it delays the N16 blow-up by ~30 % and does not fix N32 — the
    /// injection is present on BOTH the TGS and SI paths, i.e. the root is above the solver or
    /// shared; see the root-cause note in soak_and_golden.rs). The technique is correct; it was
    /// not made the default because of its cost of 2 quaternion ops per sweep. Implemented and
    /// gated.
    pub rotating_anchors: bool,

    /// The tangential speed (m/s) at which a contact counts as *sliding* — this is what decides
    /// whether the Coulomb budget is **static or dynamic**.
    ///
    /// # Why a threshold, and not "did demand exceed it"
    ///
    /// The cone clamp used to work like this: if the demanded tangential impulse exceeded
    /// `μ_s·λ_n`, scale it down to `μ_d·λ_n`. That sounds like Coulomb but is WRONG as a state
    /// test: `λ_n` fluctuates between sweeps and substeps, so a contact that is COMPLETELY AT
    /// REST crosses the threshold now and then, drops to the dynamic budget, and loses the
    /// `(μ_s − μ_d)·λ_n` of holding force it had earned. The lost force is small; the outcome is
    /// not, because above a slope of `atan(μ_d)` the escape feeds itself. Measured (2026-08-17)
    /// with the defaults μ_s=0.6 / μ_d=0.5 and a 1 kg crate:
    ///
    /// | slope | tan θ / μ_s | old behaviour | what should happen |
    /// |---|---|---|---|
    /// | 25° | 0.78 | 2.8 mm of drift in 10 s | stays put |
    /// | 28° | 0.89 | **26 m in 10 s, slid off the plate** | stays put (μ_s holds it) |
    /// | 30° | 0.96 | **77 m in 10 s, 27 m/s** | stays put |
    ///
    /// Setting `μ_d` equal to `μ_s` stopped all three — so the problem was never the magnitude
    /// of friction, it was the static/dynamic transition firing when it should not.
    ///
    /// The budget is now chosen by the contact's ACTUAL tangential speed (PhysX's `PxMaterial`
    /// model): below the threshold static, above it dynamic. The default is 1 cm/s — everything
    /// a game would call "at rest" is below it, and nothing genuinely sliding is.
    ///
    /// **The speed the step ENDED with, decided once for the next step** (`ContactPoint::sliding`,
    /// 2026-11-24). It was the speed inside each sweep, which still holds the step's unbalanced
    /// tangential acceleration `a·dt`: after a sweep that ended on the `μ_d` budget, `g·dt·(sinθ −
    /// μ_d·cosθ)` — 0.0027 m/s on a 30° slope at 240 Hz, under the threshold, but 0.011 at 60 Hz,
    /// over it, so the contact paid `μ_d` on every sweep and slid on a slope `μ_s` holds. The
    /// state at the end of a step is a rest or a slide, whatever the step's length.
    pub static_friction_velocity_threshold: f32,

    /// Warm-start matching tolerance (m): the previous substep's contact impulse is carried to a
    /// NEW contact point only if the point is within this distance (narrowphase warm-start, in
    /// pipeline.rs). This is the INJECTION CHANNEL for resting-stack instability: re-applying an
    /// old (scalar) impulse at a point that has MOVED, with the NEW lever arm, leaves a residual
    /// torque and pumps a lateral oscillation (buckling). Lowering the tolerance (to e.g. 1e-3)
    /// warm-starts only points that really did persist → shifted points start cold → the pump is
    /// cut. Default 0.02 (historical behaviour).
    pub warm_start_match_tolerance: f32,

    /// Manifold BLOCK solver: solve a contact manifold's normal constraints (same body pair,
    /// ≤4 coplanar points) TOGETHER as a direct active-set LCP instead of SEQUENTIALLY through
    /// Gauss-Seidel. It resolves the point-to-point (tilt) coupling exactly, raising a stack
    /// column's lateral restoring stiffness ABOVE the buckling threshold — the structural fix
    /// for resting-stack instability. Friction stays sequential. Off by default (pending A/B +
    /// verification); it becomes the default if it holds up.
    pub block_solver: bool,

    /// Let the block solver pick its own sweep count from the island's support depth instead of
    /// always using [`iterations`](Self::iterations).
    ///
    /// With this on (the default) a block-solved island of support depth ≥ 5 is swept
    /// `min(96, max(iterations, max(16, 3·depth/2)))` times. The floor of 16 sits *below* the
    /// default `iterations` of 20, so on the default configuration it constrains nobody: a
    /// deep island gets more than `iterations` only once `3·depth/2` exceeds it (depth ≥ 14),
    /// and a caller who lowers `iterations` below 16 gets the floor instead.
    /// (This paragraph said 28 until 2026-08-07. The floor was measured down from 28 to 16 on
    /// 2026-08-06 — see `BLOCK_ITERS_FLOOR` in `solver/tgs.rs` for the ensembles — and the
    /// claim that a caller "cannot lower it" went stale with that change.)
    ///
    /// It is also a TGS-path policy: the gate includes
    /// [`block_solver`](Self::block_solver), which only `solver/tgs.rs` reads, so an island
    /// that falls to the split-impulse path (any island holding a CCD-enabled body, or any
    /// island at all with [`use_tgs_soft`](Self::use_tgs_soft) off) sweeps `iterations`
    /// whatever its depth. See the note at the computation site in `solve_contacts`.
    ///
    /// Turning it off makes `iterations` mean exactly what it says for every island. That is
    /// what a measurement harness needs — a sweep count it can actually set, including below
    /// the floor (see `tests/solver_quality.rs`) — and it is also the escape hatch for a caller
    /// who has measured their own scene and wants the budget back.
    ///
    /// Do not turn it off casually for a scene with tall stacks. The sweep count is not just a
    /// convergence level: the TGS path derives its inner position-integration step from it
    /// (`h = dt / n_iterations`, `solver/tgs.rs`), so fewer sweeps is also a coarser temporal
    /// discretisation inside the substep, and a stacked column needs the sweeps for support to
    /// reach its top.
    pub adaptive_iterations: bool,
    /// An island with at least this many contact blocks (a block: up to four contacts of one
    /// manifold) is swept colour by colour on several threads; `0` never does. Islands with joints
    /// and islands on the split-impulse path are swept serially whatever their size.
    ///
    /// The coloured sweep visits the blocks in a different order from the serial one, so it
    /// converges differently; which islands take it depends on their size only, so the result
    /// does not depend on the machine's thread count. See `solver/parallel.rs`.
    pub parallel_island_blocks: usize,

    /// Block-solver Tikhonov regularisation, as a fraction of the manifold's mean normal
    /// effective mass. It cures the rank deficiency of a 4-coplanar contact block, and must stay
    /// small enough to leave the physical tilt-restoring modes stiff.
    ///
    /// **Lowered from 0.1 to 0.05 (2026-08-06).** 0.1 was chosen back when most interfaces were
    /// DEGENERATE single-point ones — i.e. the rank deficiency never arose and this term was
    /// never applied. Once `clip_box_box`'s depth tolerance made the 4-point block the norm, 0.1
    /// turned out to be too much softening: a regularisation term is also a softening, and a
    /// soft interface converges slowly. Measured — a compressed free chain (n=24, default
    /// sweeps) settles at frame 379 and leaks 5.4e-4 of momentum at 0.1, versus settling at
    /// frame 0 and leaking 4e-6 at 0.05: 135× better conservation. Every value from 0.05 down to
    /// 0.002 behaves identically; the LARGEST was taken, because it leaves the most numerical
    /// margin against rank deficiency. 12-high stacks (6 cells) stand at every value.
    /// (`tests/solver_quality.rs::does_block_regularization_drive_the_convergence_cost`)
    pub block_regularization: f32,

    /// Whole-CHAIN direct solve: for tall (support depth ≥5) and small enough chain islands,
    /// solve ALL of their normal impulses TOGETHER each sweep (dense active-set LCP) → the
    /// inter-manifold support coupling is resolved exactly, which improves mid-height tower
    /// stability (N24/N40). OFF BY DEFAULT: it costs O(n³) and still does not solve extreme
    /// towers (N32+) ROBUSTLY — the remaining instability lives in the friction/geometry channel
    /// and needs a joint normal+friction solver. Correct and tested; gated for scenes that need
    /// tall stacks and can afford the cost.
    pub direct_chain_solve: bool,
}

impl Default for ConstraintSolver {
    fn default() -> Self {
        Self {
            iterations: 20,
            baumgarte: 0.15,
            slop: 0.005,
            // Full warm-start (Box2D v3 / Rapier standard). The previous 0.85 discarded 15%
            // of the accumulated impulse each substep, forcing partial re-convergence whose
            // soft-constraint bias injected a marginal amount of energy — harmless at small N
            // but compounding in tall resting stacks (blow-up at N≥24). Full warm-start closes
            // that injection and makes stacks robustly stable to N≈40 (verified: soak grid
            // N=16..40 bounded over 3000 frames). See soak_and_golden::grid_candidate_fixes.
            warm_start_factor: 1.0,
            restitution_velocity_threshold: 1.0,
            max_linear_correction: 0.02,
            split_impulse_enabled: true,
            split_impulse_erp: 0.1,
            use_tgs_soft: true,
            contact_hertz: 30.0,
            contact_damping_ratio: 10.0,
            relax_iterations: 4,
            max_bias_velocity: 4.0,
            support_ordering: true,
            rotating_anchors: false,
            static_friction_velocity_threshold: 0.01,
            warm_start_match_tolerance: 0.02,
            block_solver: true,
            adaptive_iterations: true,
            parallel_island_blocks: 256,
            block_regularization: 0.05,
            direct_chain_solve: false,
        }
    }
}

/// The two bodies of a contact manifold as the contact solvers read them: their indices, inverse
/// contact masses and world inverse inertias, whether each responds to contacts, and each centre of
/// mass in the world.
///
/// Six copies of this were written out in the solvers — warm start, the velocity iterations, the
/// restitution pass and the split-impulse pass here, the warm start and the preparation in `tgs` —
/// in four layouts. The values are pure functions of the bodies and their transforms, which no
/// solve changes, so computing them once per manifold computes the same numbers.
#[derive(Clone, Copy)]
pub(crate) struct ContactPair {
    pub(crate) idx_a: usize,
    pub(crate) idx_b: usize,
    pub(crate) inv_m_a: f32,
    pub(crate) inv_m_b: f32,
    pub(crate) inv_i_a: gizmo_math::Mat3,
    pub(crate) inv_i_b: gizmo_math::Mat3,
    pub(crate) dyn_a: bool,
    pub(crate) dyn_b: bool,
    pub(crate) com_a: Vec3,
    pub(crate) com_b: Vec3,
}

impl ContactPair {
    /// `manifold`'s two bodies, or `None` when either is not in this world's index.
    pub(crate) fn of(
        manifold: &ContactManifold,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        entity_index_map: &crate::world::EntityIndexMap,
    ) -> Option<Self> {
        let idx_a = *entity_index_map.get(&manifold.entity_a.id())?;
        let idx_b = *entity_index_map.get(&manifold.entity_b.id())?;
        let com = |i: usize| transforms[i].position + transforms[i].rotation.mul_vec3(rigid_bodies[i].center_of_mass);
        Some(Self {
            idx_a,
            idx_b,
            inv_m_a: rigid_bodies[idx_a].inv_contact_mass(),
            inv_m_b: rigid_bodies[idx_b].inv_contact_mass(),
            inv_i_a: rigid_bodies[idx_a].inv_contact_inertia_tensor(transforms[idx_a].rotation),
            inv_i_b: rigid_bodies[idx_b].inv_contact_inertia_tensor(transforms[idx_b].rotation),
            dyn_a: rigid_bodies[idx_a].responds_to_contacts(),
            dyn_b: rigid_bodies[idx_b].responds_to_contacts(),
            com_a: com(idx_a),
            com_b: com(idx_b),
        })
    }
}

impl ConstraintSolver {
    /// Solver with a custom base sweep count; every other field keeps its default, so the
    /// TGS-soft path and the manifold block solver stay enabled.
    ///
    /// `iterations` counts sweeps per substep, not per rendered frame, and it is a base
    /// count rather than an exact one: on the TGS-soft path with the block solver enabled,
    /// a deep (stacked) island instead gets a sweep count derived from its depth, which
    /// can be more than the value passed here or — past an internal ceiling on adaptive
    /// iterations — fewer. Clear [`adaptive_iterations`](Self::adaptive_iterations) to make
    /// it exact. Passing 0 is accepted — it is not a division-by-zero hazard — but it leaves
    /// ordinary islands' contacts unsolved.
    pub fn new(iterations: usize) -> Self {
        Self {
            iterations,
            ..Default::default()
        }
    }
    /// The Coulomb impulse budget for one contact point this sweep.
    ///
    /// `μ_s·λ_n` while the contact is at rest, `μ_d·λ_n` once it is genuinely sliding — `sliding`,
    /// the contact's state at the end of the last step ([`record_sliding`](Self::record_sliding)),
    /// not whether the demanded impulse happened to exceed the static cap on this sweep. See
    /// [`static_friction_velocity_threshold`](Self::static_friction_velocity_threshold) for what
    /// the old test cost (a crate on a 28° slope left the plate) and why this is the fix.
    ///
    /// All five friction solves in the crate call this: the TGS sweep, the block sweep, the
    /// island sweep, the SI path and the standalone one-shot. They had five copies of the same
    /// four lines, which is how they came to disagree about nothing and would have come to
    /// disagree about this.
    #[inline]
    pub(crate) fn friction_limit(
        &self,
        sliding: bool,
        static_friction: f32,
        dynamic_friction: f32,
        normal_impulse: f32,
    ) -> f32 {
        let mu = if sliding {
            dynamic_friction
        } else {
            static_friction
        };
        mu * normal_impulse.abs()
    }


    // ─────────────────────────────────────────────────────────────────────────
    // ANA SOLVER: Manifold listesi üzerinde PGS (Projected Gauss-Seidel)
    // ─────────────────────────────────────────────────────────────────────────

    /// `pos_corrections` (same length as `velocities`) is where the split-impulse positional
    /// correction is written OUT, per body, as (linear Δposition, angular Δ-scaled-axis). This
    /// correction used to be added straight into `velocities`, which let the positional recovery
    /// speed leak into the body's real velocity — resting jitter, and bodies that never fell
    /// asleep. The caller is responsible for applying these deltas to position.
    ///
    /// Returns what the adaptive policy decided for this island — see [`SolveStats`]. An island
    /// with no manifolds is not solved and reports zeroes.
    // İndeks-tabanlı döngüler kasıtlı: `mid`/`cid` aynı anda paralel dizileri
    // (manifolds + rigid_bodies/transforms/velocities/pseudo_vel, hepsi entity
    // indeksiyle hizalı) okuyup `manifolds[mid].contacts[cid]` impulslarını geri
    // yazıyor. iter_mut'a çevirmek split-borrow gymnastics gerektirir ve bu
    // determinizm-kritik PGS yolunda fayda sağlamaz.
    #[allow(clippy::needless_range_loop)]
    pub fn solve_contacts(
        &self,
        manifolds: &mut [ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        pos_corrections: &mut [(Vec3, Vec3)],
        // The bounces of speculative contacts, `(Δlinear, Δangular)` per body, for the caller to
        // add **after** it integrates positions — see the restitution pass. Same indexing and
        // same island-only clearing as `pos_corrections`.
        bounces: &mut [(Vec3, Vec3)],
        entity_index_map: &crate::world::EntityIndexMap,
        // Distinct GLOBAL body indices in this island. The shared `pos_corrections`
        // buffer is thread-local-reused across islands, so we clear only THIS island's
        // entries (the caller reads back only these) instead of the whole world array.
        island_bodies: &[usize],
        dt: f32,
    ) -> SolveStats {
        self.solve_contacts_with(manifolds, rigid_bodies, transforms, velocities, pos_corrections, bounces, entity_index_map, island_bodies, dt, None)
    }

    /// [`solve_contacts`](Self::solve_contacts), running `between` before each of its sweeps: an
    /// island's joints, solved in the same iterations as its contacts (`pipeline.rs`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn solve_contacts_with(
        &self,
        manifolds: &mut [ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        pos_corrections: &mut [(Vec3, Vec3)],
        bounces: &mut [(Vec3, Vec3)],
        entity_index_map: &crate::world::EntityIndexMap,
        island_bodies: &[usize],
        dt: f32,
        between: BetweenSweeps<'_>,
    ) -> SolveStats {
        // Pozisyon düzeltme buffer'ını sıfırla — yalnız bu adanın girdileri (buffer çağıran
        // tarafından adalar arası yeniden kullanılıyor; full-world sıfırlama O(n_islands×n_bodies)'ti).
        for &i in island_bodies {
            pos_corrections[i] = (Vec3::ZERO, Vec3::ZERO);
            bounces[i] = (Vec3::ZERO, Vec3::ZERO);
        }
        if manifolds.is_empty() {
            return SolveStats::default();
        }

        let island_depth = self.island_depth(manifolds, rigid_bodies, entity_index_map);
        let n_iterations = self.sweep_count(island_depth);
        let has_ccd = Self::island_has_ccd(manifolds, rigid_bodies, entity_index_map);

        // Tall, bucklable stacks (support-depth ≥ 5) are the historically unstable case
        // (resting-stack instability). Only these deep islands reach this branch, so a
        // trace here — chosen solver path, adaptive sweep count, block/direct flags — is a
        // low-frequency, high-signal window into the stack solve without touching the
        // per-contact inner loop.
        if island_depth >= 5 {
            tracing::trace!(
                island_depth,
                n_iterations,
                n_manifolds = manifolds.len(),
                solver = if self.use_tgs_soft && !has_ccd { "tgs-soft" } else { "split-impulse" },
                block_solver = self.block_solver,
                direct_chain = self.direct_chain_solve,
                "solving tall (bucklable) stack island"
            );
        }

        if self.use_tgs_soft && !has_ccd {
            self.solve_contacts_tgs(
                manifolds,
                rigid_bodies,
                transforms,
                velocities,
                pos_corrections,
                bounces,
                entity_index_map,
                island_bodies,
                island_depth,
                n_iterations,
                dt,
                between,
            );
            self.record_sliding(manifolds, rigid_bodies, transforms, velocities, entity_index_map);
            return SolveStats {
                island_depth,
                iterations: n_iterations,
            };
        }

        self.solve_contacts_split_impulse(
            manifolds,
            rigid_bodies,
            transforms,
            velocities,
            pos_corrections,
            bounces,
            entity_index_map,
            island_bodies,
            dt,
            between,
        );
        self.record_sliding(manifolds, rigid_bodies, transforms, velocities, entity_index_map);
        // The split-impulse path sweeps `self.iterations` (see the loop above) and has never
        // consumed the adaptive count, so reporting `n_iterations` here would be a lie about
        // work that was not done. An island reaches this path only by containing a CCD-enabled
        // body or by `use_tgs_soft` being off.
        SolveStats {
            island_depth,
            iterations: self.iterations,
        }
    }

    /// Each contact's state at the end of the solve, for the next step's friction
    /// ([`friction_limit`](Self::friction_limit)): sliding when the bodies' points slip past each
    /// other faster than [`static_friction_velocity_threshold`](Self::static_friction_velocity_threshold)
    /// and the contact was sliding already or static friction gave all it had this step.
    fn record_sliding(
        &self,
        manifolds: &mut [ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &[Velocity],
        entity_index_map: &crate::world::EntityIndexMap,
    ) {
        let record = |m: &mut ContactManifold| {
            let Some(pair) = ContactPair::of(m, rigid_bodies, transforms, entity_index_map) else {
                return;
            };
            let (va, vb) = (&velocities[pair.idx_a], &velocities[pair.idx_b]);
            for c in m.contacts.iter_mut() {
                let rel = (vb.linear + vb.angular.cross(c.point - pair.com_b)) - (va.linear + va.angular.cross(c.point - pair.com_a));
                let slip = rel - c.normal * rel.dot(c.normal);
                // Coulomb's hysteresis: a contact at rest starts to slide only when static friction
                // gave all it had — its tangential impulse at the `μ_s·λ_n` cone — and one sliding goes
                // on until its points stop. A resting contact whose points move a little without the
                // static budget running out (a piece rocking on its edge) stays at rest; with no such
                // test, one that slipped past 1 cm/s for a step paid `μ_d` from then on and crept.
                let gave_all = c.tangent_impulse.length() >= 0.99 * m.static_friction * c.normal_impulse.max(0.0);
                c.sliding = slip.length() > self.static_friction_velocity_threshold && (c.sliding || gave_all);
            }
        };
        // Each manifold's own points from read-only velocities: in parallel for a big island.
        if manifolds.len() >= tgs::PARALLEL_MANIFOLDS {
            manifolds.par_iter_mut().with_min_len(tgs::MANIFOLDS_PER_CHUNK).for_each(record);
        } else {
            manifolds.iter_mut().for_each(record);
        }
    }

    /// The island's support depth, its contacts support-ordered on the way when that is on.
    fn island_depth(&self, manifolds: &mut [ContactManifold], rigid_bodies: &[RigidBody], entity_index_map: &crate::world::EntityIndexMap) -> u32 {
        // ── Support order + island depth ──
        // Support-order the island's contacts (bottom-up from anchors) when enabled — a
        // deterministic, pair-emission-invariant total order that lets the block solver's
        // support front propagate up the column. The BFS also yields the island's support
        // depth, which drives ADAPTIVE ITERATIONS: a tall stack needs iterations ≥ its
        // height for support to reach the top, so with the block solver we scale the sweep
        // count with depth (short piles keep the base count → no perf cost).
        if self.support_ordering || self.block_solver {
            let _t = crate::profile::Scope::new(&crate::profile::PHASES.order);
            support_order_manifolds(
                manifolds,
                rigid_bodies,
                entity_index_map.raw(),
                self.support_ordering,
            )
        } else {
            0
        }
    }

    /// The biased sweeps the TGS path runs for an island of support depth `island_depth`.
    fn sweep_count(&self, island_depth: u32) -> usize {
        // Adaptive iterations: a stacked column of support-depth D is linearly unstable
        // (buckling) until support propagates to its top, which needs sweeps that scale
        // with D. Shallow islands (D<5, no buckling) keep the base count; bucklable stacks
        // get max(FLOOR, 1.5·D) sweeps (empirically: D=5 needs ≥24, D=32 needs ~48), capped.
        //
        // TGS PATH ONLY, and knowingly so — `n_iterations` is consumed by the TGS branch
        // below (and by the trace) and by nothing else; the split-impulse fallback sweeps
        // `self.iterations` (see its `for iter in 0..self.iterations` loop) and derives its
        // position pass from the same field. An island falls to that path by containing a
        // CCD-enabled body or by `use_tgs_soft` being off, and it then gets the base count
        // however deep it is. `SolveStats` reports that honestly rather than papering over it
        // (see the note above the `SolveStats` returned at the end of this function).
        //
        // Whether it SHOULD carry over was open. It is now MEASURED and the answer is no —
        // `tests/solver_quality.rs::sweep_ladder_tower_split_impulse` walks the same ladder as
        // `sweep_ladder_tower` with `use_tgs_soft = false`, so the two tables differ in the
        // solver and nothing else. At N = 32 (well past the depth ≥ 14 where the adaptive count
        // would change anything at all on the default config):
        //
        //     sweeps   TGS blew/3, lean   SI blew/3, lean
        //          8        0,  0.0000        0,  0.0096
        //         20        0,  0.0000        0,  0.0028   <- the default
        //         28        0,  0.0000        0,  0.0014
        //         46        0,  0.0000        0,  0.0033
        //
        // Split-impulse does not blow up at the default sweep count at ANY depth tested (16, 24,
        // 32 — three ground sizes each); it is stable from 8 sweeps upward. So the dead adaptive
        // count is not leaving this path under-solved, and extending it would buy lean, not
        // stability, at a real per-island CPU cost.
        //
        // And it would not even buy lean reliably: the SI column is NOT monotonic — 46 sweeps
        // (0.0033) is worse than 28 (0.0014). That is the suspicion in the third bullet below,
        // confirmed: on a path that corrects position through a separate pseudo-velocity pass,
        // more biased sweeps is not simply more stable.
        //
        // Left as it is, deliberately and now with evidence. The reasoning that was open:
        //   • The gate is keyed on `block_solver`, which only `solver/tgs.rs` ever reads. On
        //     the split-impulse path that flag means nothing, so the condition is currently
        //     testing a feature the path does not have.
        //   • The buckling argument itself is solver-agnostic — Gauss-Seidel needs O(D)
        //     sweeps to propagate support up a column whichever velocity law it applies —
        //     which is why extending it looks right on paper.
        //   • But the numbers behind FLOOR/CAP were measured on TGS+block only (see
        //     `BLOCK_ITERS_FLOOR` in solver/tgs.rs and tests/solver_quality.rs). Nothing has
        //     been measured for split-impulse, whose position correction is a separate
        //     pseudo-velocity pass — more biased sweeps there is not obviously more stable.
        //   • Blast radius if it were extended, on the DEFAULT config: none below depth 14.
        //     `iterations` is 20 and FLOOR is 16, so `max(20, max(16, 3D/2))` is still 20 for
        //     every D ≤ 13; only D ≥ 14 would change (D=14 → 21, saturating at 96). It is NOT
        //     a no-op for a caller who lowered `iterations` below 16 — those get the floor.
        //     Both `tests/ccd.rs` and `tests/ccd_analytical.rs` run on this path and are the
        //     regression surface that such a change has to be re-blessed against.
        if self.adaptive_iterations && self.block_solver && island_depth >= 5 {
            let target = (island_depth as usize * 3 / 2).max(Self::BLOCK_ITERS_FLOOR);
            self.iterations.max(target).min(Self::BLOCK_ITERS_CAP)
        } else {
            self.iterations
        }
    }

    /// Whether a body of the island has CCD on, which keeps the island off the TGS path.
    fn island_has_ccd(manifolds: &[ContactManifold], rigid_bodies: &[RigidBody], entity_index_map: &crate::world::EntityIndexMap) -> bool {
        // TGS Soft yolu (modern çözücü): soft constraint + relax pass.
        // İSTİSNA: CCD-etkin gövde içeren island'lar eski (split-impulse) yolu kullanır.
        // CCD speculative temasları ince ayarlıdır; TGS'in dp/relax akışı yüksek-hızlı
        // açılı çarpmalarda speculative clamp'le çatışıp tünellemeye yol açabiliyor.
        // CCD cisimleri (mermiler) nadir ve genelde izole; yığın kararlılığı TGS'te kalır.
        //
        // BU GEREKÇE 2026-08-18'DE ÖLÇÜLDÜ, çünkü hatırlanan bir gerekçe bir kanıt değildir —
        // ve `tests/cradle_passthrough.rs`'teki `ccd_makes_a_bounce_a_thud` bu kapının bedelini
        // (CCD açıkken esnek çarpışmanın esnekliğini kaybetmesi) çivilenmiş bir sınırlama olarak
        // taşıyor. Deney: `&& !has_ccd`'yi kaldır, CCD paketini koştur.
        //
        //   • `ccd_analytical` (9 test) ve `cradle_passthrough` HEPSİ GEÇTİ — çivilenmiş
        //     sınırlama dahil, yani kapının bedeli gerçekten bu kapıdan geliyor.
        //   • Ama `prop_ccd_never_tunnels` bir tur içinde karşı örnek buldu ve KÜÇÜLTTÜ:
        //     hız 1753.91 m/s, yarı-kalınlık 0.2967, yarıçap 0.3940 → x = 0.70'te TÜNELLEDİ.
        //     0.59 m kalınlıkta bir duvarı delen bir mermi.
        //
        // Yani takas gerçek ve kapı kalıyor. Fark şu: itiraz artık `ccd.proptest-regressions`'a
        // yazılmış bir karşı örnek (sevkiyattaki çözücü onu geçiyor, dolayısıyla bedava ek
        // kapsam), hatırlanan bir cümle değil. Öbür yol — split-impulse yolunda speculative
        // temaslara restitution — 2026-11-15'te açıldı: aşağıdaki "Restitution pass".
        manifolds.iter().any(|m| {
            [m.entity_a, m.entity_b].iter().any(|e| {
                entity_index_map
                    .get(&e.id())
                    .is_some_and(|&i| rigid_bodies[i].ccd_enabled)
            })
        })
    }

    /// The split-impulse sequential-impulse solve, for an island with a CCD body or with
    /// `use_tgs_soft` off: the warm start, `iterations` symmetric Gauss-Seidel sweeps, the
    /// speculative contacts' bounces (into `bounces`), and the position pass (`pos_corrections`).
    #[allow(clippy::too_many_arguments)]
    fn solve_contacts_split_impulse(
        &self,
        manifolds: &mut [ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        pos_corrections: &mut [(Vec3, Vec3)],
        bounces: &mut [(Vec3, Vec3)],
        entity_index_map: &crate::world::EntityIndexMap,
        island_bodies: &[usize],
        dt: f32,
        between: BetweenSweeps<'_>,
    ) {
        let (spec_off, spec_vn0) = Self::speculative_closing_speeds(manifolds, rigid_bodies, transforms, velocities, entity_index_map);
        self.warm_start_split_impulse(manifolds, rigid_bodies, transforms, velocities, entity_index_map);

        // ── İteratif PGS ─────────────────────────────────────────────────────
        let inv_dt = if dt > 0.0 { 1.0 / dt } else { 0.0 };
        self.pgs_sweeps(manifolds, rigid_bodies, transforms, velocities, entity_index_map, inv_dt, between);
        self.speculative_bounces(
            manifolds,
            (rigid_bodies, transforms, entity_index_map),
            velocities,
            bounces,
            island_bodies,
            (&spec_off, &spec_vn0),
            dt,
        );

        if self.split_impulse_enabled {
            self.split_impulse_positions(manifolds, rigid_bodies, transforms, entity_index_map, velocities.len(), inv_dt, dt, pos_corrections);
        }
    }

    /// The closing speed of every speculative contact of a bouncy pair before the solve, at
    /// `vn0[off[manifold] + contact]` (`NAN` for the rest).
    fn speculative_closing_speeds(
        manifolds: &[ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &[Velocity],
        entity_index_map: &crate::world::EntityIndexMap,
    ) -> (Vec<usize>, Vec<f32>) {
        // ── Restitution of speculative contacts: the closing speed before the solve ──────────
        // A speculative contact (a gap still open, `penetration < 0`) is solved without
        // restitution below — it is a limit that closes the gap, and a bounce mixed into it upsets
        // the bias (`e` is zero for it in the loop). So the body arrived touching at almost no
        // speed and the next step's real contact was under the restitution threshold: with CCD on
        // — which puts the island on this path — an elastic hit was a thud (a Newton's cradle
        // stopped dead). The bounce is given afterwards instead, as Box2D v3 does: the closing
        // speed here, before warm-starting, is kept for every speculative contact of a bouncy
        // pair, and once the velocity iterations are done each one the solve engaged (it pushed:
        // the bodies will meet this step) has its normal speed set to `−e·v₀`. That only ever
        // separates the pair, so it cannot carry a body through anything — the objection the
        // other route (TGS for CCD islands) met in `prop_ccd_never_tunnels`.
        let mut spec_off: Vec<usize> = Vec::with_capacity(manifolds.len() + 1);
        spec_off.push(0);
        for m in manifolds.iter() {
            spec_off.push(spec_off[spec_off.len() - 1] + m.contacts.len());
        }
        let mut spec_vn0 = vec![f32::NAN; spec_off[manifolds.len()]];
        for (mid, m) in manifolds.iter().enumerate() {
            if m.restitution <= 0.0 {
                continue;
            }
            let (Some(&idx_a), Some(&idx_b)) = (entity_index_map.get(&m.entity_a.id()), entity_index_map.get(&m.entity_b.id())) else {
                continue;
            };
            let com_a = transforms[idx_a].position + transforms[idx_a].rotation.mul_vec3(rigid_bodies[idx_a].center_of_mass);
            let com_b = transforms[idx_b].position + transforms[idx_b].rotation.mul_vec3(rigid_bodies[idx_b].center_of_mass);
            for (cid, c) in m.contacts.iter().enumerate() {
                if c.penetration >= 0.0 {
                    continue;
                }
                let va = velocities[idx_a].linear + velocities[idx_a].angular.cross(c.point - com_a);
                let vb = velocities[idx_b].linear + velocities[idx_b].angular.cross(c.point - com_b);
                spec_vn0[spec_off[mid] + cid] = (vb - va).dot(c.normal);
            }
        }
        (spec_off, spec_vn0)
    }

    /// The split-impulse path's warm start: last step's impulses, scaled by `warm_start_factor`.
    fn warm_start_split_impulse(
        &self,
        manifolds: &[ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        entity_index_map: &crate::world::EntityIndexMap,
    ) {
        for manifold in manifolds {
            let Some(ContactPair { idx_a, idx_b, inv_m_a, inv_m_b, inv_i_a, inv_i_b, dyn_a, dyn_b, com_a, com_b }) =
                ContactPair::of(manifold, rigid_bodies, transforms, entity_index_map)
            else {
                continue;
            };

            for contact in &manifold.contacts {
                let r_a = contact.point - com_a;
                let r_b = contact.point - com_b;

                let wn = contact.normal * (contact.normal_impulse * self.warm_start_factor);
                let wt = contact.tangent_impulse * self.warm_start_factor;

                if dyn_a {
                    velocities[idx_a].linear -= wn * inv_m_a;
                    velocities[idx_a].linear -= wt * inv_m_a;
                    velocities[idx_a].angular -= inv_i_a * (r_a.cross(wn) + r_a.cross(wt));
                }
                if dyn_b {
                    velocities[idx_b].linear += wn * inv_m_b;
                    velocities[idx_b].linear += wt * inv_m_b;
                    velocities[idx_b].angular += inv_i_b * (r_b.cross(wn) + r_b.cross(wt));
                }
            }
        }
    }

    /// The split-impulse path's `iterations` sweeps over the island's contacts, alternating
    /// direction (`pgs_contact` for each).
    fn pgs_sweeps(
        &self,
        manifolds: &mut [ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        entity_index_map: &crate::world::EntityIndexMap,
        inv_dt: f32,
        mut between: BetweenSweeps<'_>,
    ) {
        let n_manifolds = manifolds.len();
        // Each contact's rolling-resistance impulse over the sweep (`rolling_row`), at
        // `roll_at[mid] + cid`. This path had no rolling row at all, and every island with a CCD
        // body comes here: a CCD ball on any ground rolled on until damping stopped it — eight
        // seconds from 1 m/s where the same ball without CCD stops in a quarter of one.
        let mut roll_at = Vec::with_capacity(n_manifolds);
        let mut n_points = 0;
        for m in manifolds.iter() {
            roll_at.push(n_points);
            n_points += m.contacts.len();
        }
        let mut acc_roll = vec![Vec3::ZERO; n_points];
        for iter in 0..self.iterations {
            // The island's joints first, then its contacts (see the TGS path's sweeps).
            if let Some(between) = between.as_mut() {
                (between.sweep)(velocities, &[], false);
            }
            // Symmetric Gauss-Seidel: alternate the sweep direction every
            // iteration. Plain forward-only PGS applies the manifold's contact
            // points in a fixed order; each point's impulse is off-centre, so the
            // transient bias never fully cancels and a *perfectly symmetric* impact
            // (e.g. an axis-aligned box stack) picks up spurious angular velocity
            // that tips and collapses tall stacks. Reversing on odd iterations
            // cancels the directional bias and keeps such stacks upright. The
            // order is a deterministic function of `iter`, so determinism holds.
            let reverse = iter % 2 == 1;
            for mi in 0..n_manifolds {
                let mid = if reverse { n_manifolds - 1 - mi } else { mi };
                let Some(pair) = ContactPair::of(&manifolds[mid], rigid_bodies, transforms, entity_index_map) else {
                    continue;
                };
                let manifold = &mut manifolds[mid];
                let materials = (manifold.friction, manifold.static_friction, manifold.restitution, manifold.rolling_friction);
                let n_contacts = manifold.contacts.len();
                for ci in 0..n_contacts {
                    let cid = if reverse { n_contacts - 1 - ci } else { ci };
                    let roll = &mut acc_roll[roll_at[mid] + cid];
                    self.pgs_contact(&mut manifold.contacts[cid], pair, materials, velocities, inv_dt, roll);
                }
            }
        }
    }

    /// One contact of one sweep on the split-impulse path: the normal impulse (a speculative
    /// contact's gap closed by its bias, a touching one's by the position pass), then the
    /// two-tangent Coulomb cone and the rolling row. `materials` is the pair's (friction, static
    /// friction, restitution, rolling friction); `acc_roll`, the contact's rolling impulse so far.
    fn pgs_contact(
        &self,
        contact: &mut gizmo_physics_core::ContactPoint,
        pair: ContactPair,
        (friction, static_friction, restitution, rolling_friction): (f32, f32, f32, f32),
        velocities: &mut [Velocity],
        inv_dt: f32,
        acc_roll: &mut Vec3,
    ) {
        let contact_pt = contact.point;
        let normal = contact.normal;
        let penetration = contact.penetration;
        let acc_n = contact.normal_impulse;
        let acc_t = contact.tangent_impulse;

        let ContactPair { idx_a, idx_b, inv_m_a, inv_m_b, inv_i_a, inv_i_b, dyn_a, dyn_b, com_a, com_b } = pair;
        let r_a = contact_pt - com_a;
        let r_b = contact_pt - com_b;

        if !dyn_a && !dyn_b {
            return;
        }

        // Temas noktasındaki göreli hız
        let va = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
        let vb = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
        let rel_vel = vb - va;
        let vel_norm = rel_vel.dot(normal);

        // ── Normal İmpuls ────────────────────────────────────────
        let r_a_x_n = r_a.cross(normal);
        let r_b_x_n = r_b.cross(normal);
        let k_n = inv_m_a
            + inv_m_b
            + (inv_i_a.mul_vec3(r_a_x_n)).dot(r_a_x_n)
            + (inv_i_b.mul_vec3(r_b_x_n)).dot(r_b_x_n);

        if k_n < 1e-8 {
            return;
        }

        // Pozisyon düzeltme stratejisi:
        // Split Impulse: bias=0 (pozisyon düzeltmesi ayrı pseudo-velocity kanalında)
        // Fallback: Baumgarte bias velocity'ye karıştırılır
        let bias = if penetration < 0.0 {
            // Speculative contact: nesne henüz teması yapmadı
            penetration * inv_dt
        } else if self.split_impulse_enabled {
            // Split Impulse: pozisyon düzeltme tamamen pseudo-velocity pass'te
            // Velocity kanalı temiz kalır → resting jitter yok
            0.0
        } else {
            // Fallback Baumgarte
            let correction = (penetration - self.slop)
                .max(0.0)
                .min(self.max_linear_correction);
            self.baumgarte * inv_dt * correction
        };

        // Restitution: sadece yüksek hızlı GERÇEK çarpışmalarda. Speculative
        // temas (penetration < 0) bir boşluk-kapatma LİMİTİdir; ona restitution
        // uygulamak bias'ı bozar (cisim substep'ler arası tutarsız yavaşlar ve
        // son substep'te yüzeyi aşıp girer). Sekme, cisim gerçekten değdiğinde
        // (penetration ≥ 0) doğal olarak uygulanır.
        let e = if penetration < 0.0 {
            0.0
        } else if -vel_norm > self.restitution_velocity_threshold {
            restitution
        } else {
            0.0
        };

        let delta_n = (-(1.0 + e) * vel_norm + bias) / k_n;
        let new_acc_n = (acc_n + delta_n).max(0.0); // Clamp: çekme yok
        let actual_n = new_acc_n - acc_n;
        contact.normal_impulse = new_acc_n;

        let imp_n = normal * actual_n;
        if dyn_a {
            velocities[idx_a].linear -= imp_n * inv_m_a;
            velocities[idx_a].angular -= inv_i_a.mul_vec3(r_a.cross(imp_n));
        }
        if dyn_b {
            velocities[idx_b].linear += imp_n * inv_m_b;
            velocities[idx_b].angular += inv_i_b.mul_vec3(r_b.cross(imp_n));
        }

        // A speculative contact gets no friction: the bodies have not touched, and
        // its anchor is the dynamic body's centre (`Gjk::speculative_contact`), so a
        // friction impulse there has no lever arm — it does not spin the body, it only
        // takes its tangential speed. Measured on a CCD ball dropped 5 cm onto a
        // tilted floor of tiles (`tests/kinematic_target.rs`): the one speculative
        // contact on landing took the 0.156 m/s it carried down the slope, and it
        // covered 2.03 m in two seconds where it covers 2.24 m without that
        // friction. The real contact on the next step rolls it.
        if penetration < 0.0 {
            contact.tangent_impulse = Vec3::ZERO;
            return;
        }

        // ── Sürtünme İmpulsu (2-tangent Coulomb cone) ───────────
        // Normalden türetilen SABİT iki ortonormal tangent (t1,t2); birikim
        // her eksende skaler ve birlikte dairesel koniye clamp'lenir.
        // (Eski tek-tangent yöntemi tangenti her iterasyonda döndürüp birikmiş
        // impulsun dik bileşenini kaybediyordu → kayıplı/yön kayan sürtünme.)
        let (t1, t2) = {
            let a = if normal.x.abs() > 0.9 {
                Vec3::new(0.0, 1.0, 0.0).cross(normal)
            } else {
                Vec3::new(1.0, 0.0, 0.0).cross(normal)
            }
            .normalize();
            (a, normal.cross(a))
        };

        let va2 = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
        let vb2 = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
        let rel2 = vb2 - va2;

        // Eksen başına efektif kütle: k = inv_m + (r×t)·I⁻¹·(r×t).
        let eff_mass = |taxis: Vec3| -> f32 {
            let rxt_a = r_a.cross(taxis);
            let rxt_b = r_b.cross(taxis);
            inv_m_a
                + inv_m_b
                + inv_i_a.mul_vec3(rxt_a).dot(rxt_a)
                + inv_i_b.mul_vec3(rxt_b).dot(rxt_b)
        };
        let k_t1 = eff_mass(t1);
        let k_t2 = eff_mass(t2);

        // Birikmiş tangent impulsu sabit baza ayrıştır, her eksende çöz.
        let acc_t1 = acc_t.dot(t1);
        let acc_t2 = acc_t.dot(t2);
        let mut new1 = if k_t1 > 1e-8 {
            acc_t1 - rel2.dot(t1) / k_t1
        } else {
            acc_t1
        };
        let mut new2 = if k_t2 > 1e-8 {
            acc_t2 - rel2.dot(t2) / k_t2
        } else {
            acc_t2
        };

        // The circular Coulomb cone: μ_s for a contact at rest at the last step's end, μ_d for one
        // sliding — see `friction_limit`.
        let limit = self.friction_limit(
            contact.sliding,
            static_friction,
            friction,
            new_acc_n,
        );
        let mag = (new1 * new1 + new2 * new2).sqrt();
        if mag > limit && mag > 1e-12 {
            let s = limit / mag;
            new1 *= s;
            new2 *= s;
        }

        let imp_t = t1 * (new1 - acc_t1) + t2 * (new2 - acc_t2);
        contact.tangent_impulse = t1 * new1 + t2 * new2;
        if dyn_a {
            velocities[idx_a].linear -= imp_t * inv_m_a;
            velocities[idx_a].angular -= inv_i_a.mul_vec3(r_a.cross(imp_t));
        }
        if dyn_b {
            velocities[idx_b].linear += imp_t * inv_m_b;
            velocities[idx_b].angular += inv_i_b.mul_vec3(r_b.cross(imp_t));
        }

        *acc_roll = rolling_row(
            velocities,
            (idx_a, idx_b),
            (dyn_a, dyn_b),
            (inv_i_a, inv_i_b),
            (rolling_friction, rolling_lever(r_a, r_b, normal, (dyn_a, dyn_b))),
            new_acc_n,
            *acc_roll,
        );
    }

    /// The speculative contacts' bounces (see `speculative_closing_speeds`), handed back in
    /// `bounces` for after position integration; the velocities are left as the sweeps made them.
    #[allow(clippy::too_many_arguments)]
    fn speculative_bounces(
        &self,
        manifolds: &[ContactManifold],
        (rigid_bodies, transforms, entity_index_map): (&[RigidBody], &[Transform], &crate::world::EntityIndexMap),
        velocities: &mut [Velocity],
        bounces: &mut [(Vec3, Vec3)],
        island_bodies: &[usize],
        (spec_off, spec_vn0): (&[usize], &[f32]),
        dt: f32,
    ) {
        // ── Restitution pass (speculative contacts — see where `spec_vn0` is filled) ─────────
        //
        // The bounce is handed back for after position integration (`bounces`), not left in the
        // velocities: a CCD contact spans the whole sweep — a bullet 4.7 m short of a wall at
        // 1200 m/s is speculative — and a velocity reversed now would send the body back from
        // where it stands without reaching the wall (measured: it turned at x = −11 before a wall
        // at 0). Kept out of this step's velocity, the body travels at the speed the speculative
        // contact allowed and ends the step touching; the bounce is added then, and it leaves
        // from the wall. (Putting the position right through the pseudo-velocity channel instead
        // was tried: that correction lands before integration, and the CCD sweep from there
        // stopped the bullet on the wall's far side.)
        let speculative = spec_vn0.iter().any(|v| !v.is_nan());
        let before: Vec<(usize, Velocity)> =
            if speculative { island_bodies.iter().map(|&i| (i, velocities[i])).collect() } else { Vec::new() };
        // Each point's impulse over the pass, starting from what the sweeps left: a manifold of
        // several points is solved over several sweeps, as the sweeps solve it. One pass, one point
        // at a time, was enough for the one point of a CCD bullet; a box face's four, coupled through
        // the rotation, overshot (two 1200 kg cars parted at 7.2 m/s where e = 0.3 gives 4.5).
        let mut acc: Vec<f32> = manifolds.iter().flat_map(|m| m.contacts.iter().map(|c| c.normal_impulse)).collect();
        // Swept until no point changes by more than a millinewton-second per kilogram it moves, or
        // `MAX_BOUNCE_SWEEPS`: the two cars above take about twenty.
        const MAX_BOUNCE_SWEEPS: usize = 32;
        let mut sweep = 0;
        let mut changed = speculative;
        while changed && sweep < MAX_BOUNCE_SWEEPS {
            sweep += 1;
            changed = false;
        for mid in 0..manifolds.len() {
            let e = manifolds[mid].restitution;
            if e <= 0.0 {
                continue;
            }
            let Some(ContactPair { idx_a, idx_b, inv_m_a, inv_m_b, inv_i_a, inv_i_b, dyn_a, dyn_b, com_a, com_b }) =
                ContactPair::of(&manifolds[mid], rigid_bodies, transforms, entity_index_map)
            else {
                continue;
            };
            if !dyn_a && !dyn_b {
                continue;
            }
            for cid in 0..manifolds[mid].contacts.len() {
                let vn0 = spec_vn0[spec_off[mid] + cid];
                // An impact meeting within this step at the speed it had (`gap ≤ |v₀|·dt`),
                // faster than the threshold. Asking whether the solve pushed is not that question
                // — a warm-started impulse is positive before anything closes, and the bullet
                // turned 4.7 m short of the wall on it.
                //
                // A contact already touching (`gap ≤ slop`) bounces only when the bounce itself
                // clears the threshold (`e·|v₀|`): a CCD golf ball rolling over the terrain's
                // facets meets a fresh one at every crease at 1–2 m/s, and bouncing each at
                // e = 0.1 — 0.1–0.2 m/s, the micro-bounce the threshold exists to suppress — sent
                // it into another basin of the course every time (`golf --test`: 28 strokes and a
                // hole it never finished; 17 with CCD off, 13 with this rule). A cradle's touching
                // ball at 5 m/s and e = 1 does clear it, and has to bounce here: the speculative
                // contact has already taken its approach speed, so the real contact next step
                // has nothing left to bounce with.
                let gap = -manifolds[mid].contacts[cid].penetration;
                let impact = !vn0.is_nan()
                    && -vn0 > self.restitution_velocity_threshold
                    && gap <= -vn0 * dt
                    && (gap > self.slop || e * -vn0 > self.restitution_velocity_threshold);
                if !impact {
                    continue;
                }
                let acc_n = acc[spec_off[mid] + cid];
                let c = &manifolds[mid].contacts[cid];
                let (normal, r_a, r_b) = (c.normal, c.point - com_a, c.point - com_b);
                let va = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
                let vb = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
                let vn = (vb - va).dot(normal);
                let (rna, rnb) = (r_a.cross(normal), r_b.cross(normal));
                let k_n = inv_m_a + inv_m_b + inv_i_a.mul_vec3(rna).dot(rna) + inv_i_b.mul_vec3(rnb).dot(rnb);
                if k_n < 1e-8 {
                    continue;
                }
                // The bounce goes into the velocity only, never into the contact's accumulated
                // impulse: that is the warm start of the next step, and a bounce kept there was
                // pushed again at the start of every step after — a golf ball rolling over the
                // terrain's facets hopped 8 cm at each crease and ran off the course.
                let new_acc = (acc_n - (vn + e * vn0) / k_n).max(0.0);
                acc[spec_off[mid] + cid] = new_acc;
                changed |= (new_acc - acc_n).abs() * (inv_m_a + inv_m_b) > 1e-3;
                let imp_n = normal * (new_acc - acc_n);
                if dyn_a {
                    velocities[idx_a].linear -= imp_n * inv_m_a;
                    velocities[idx_a].angular -= inv_i_a.mul_vec3(r_a.cross(imp_n));
                }
                if dyn_b {
                    velocities[idx_b].linear += imp_n * inv_m_b;
                    velocities[idx_b].angular += inv_i_b.mul_vec3(r_b.cross(imp_n));
                }
            }
        }
        }
        for (i, kept) in before {
            bounces[i] = (velocities[i].linear - kept.linear, velocities[i].angular - kept.angular);
            velocities[i] = kept;
        }
    }

    /// Split impulse: the position-correction pass. It computes pseudo-velocities independent of
    /// the real velocity, so penetration is corrected outside the velocity channel, and tracking
    /// the accumulated pseudo-impulse keeps it from over-correcting.
    ///
    /// The pseudo-velocities are this pass's own, one per body of the world (`n_bodies`, as the
    /// velocities), and start at zero: what it writes out is `pos_corrections`, which the caller
    /// applies to position. They were allocated for every island the solver ran, the pass on or off.
    #[allow(clippy::too_many_arguments)]
    fn split_impulse_positions(
        &self,
        manifolds: &[ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        entity_index_map: &crate::world::EntityIndexMap,
        n_bodies: usize,
        inv_dt: f32,
        dt: f32,
        pos_corrections: &mut [(Vec3, Vec3)],
    ) {
        let mut pseudo_vel: Vec<(Vec3, Vec3)> = vec![(Vec3::ZERO, Vec3::ZERO); n_bodies];
        // Per-contact birikimli pseudo-impulse (PGS clamping için).
        //
        // Tek düz tampon + offset tablosu; `solve_contacts_tgs`'in `vn0`'ıyla aynı düzeltme
        // (C4-followup). Eskiden manifold başına bir iç `Vec` ayrılıyordu, yani ada başına
        // substep başına `1 + N_manifold` ayırma. `ArrayVec` olamaz: `m.contacts.len()`
        // capped değil ve `ArrayVec` taşmada panikler.
        let mut acc_off: Vec<usize> = Vec::with_capacity(manifolds.len() + 1);
        acc_off.push(0);
        let mut acc_total = 0usize;
        for m in manifolds.iter() {
            acc_total += m.contacts.len();
            acc_off.push(acc_total);
        }
        let mut acc_pseudo = vec![0.0f32; acc_total];

        let pos_iterations = (self.iterations / 2).max(4);
        for _ in 0..pos_iterations {
            for mid in 0..manifolds.len() {
                let Some(ContactPair { idx_a, idx_b, inv_m_a, inv_m_b, inv_i_a, inv_i_b, dyn_a, dyn_b, com_a, com_b }) =
                    ContactPair::of(&manifolds[mid], rigid_bodies, transforms, entity_index_map)
                else {
                    continue;
                };
                if !dyn_a && !dyn_b {
                    continue;
                }

                for (cid, contact) in manifolds[mid].contacts.iter().enumerate() {
                    let contact_pt = contact.point;
                    let normal = contact.normal;
                    let penetration = contact.penetration;

                    let correction = (penetration - self.slop)
                        .max(0.0)
                        .min(self.max_linear_correction);
                    if correction < 1e-6 {
                        continue;
                    }

                    let r_a = contact_pt - com_a;
                    let r_b = contact_pt - com_b;

                    let r_a_x_n = r_a.cross(normal);
                    let r_b_x_n = r_b.cross(normal);
                    let k_n = inv_m_a
                        + inv_m_b
                        + (inv_i_a.mul_vec3(r_a_x_n)).dot(r_a_x_n)
                        + (inv_i_b.mul_vec3(r_b_x_n)).dot(r_b_x_n);
                    if k_n < 1e-8 {
                        continue;
                    }

                    // Pseudo-velocity relative to contact normal
                    let pv_a = pseudo_vel[idx_a].0 + pseudo_vel[idx_a].1.cross(r_a);
                    let pv_b = pseudo_vel[idx_b].0 + pseudo_vel[idx_b].1.cross(r_b);
                    let pv_rel = pv_b.dot(normal) - pv_a.dot(normal);

                    let bias = self.split_impulse_erp * inv_dt * correction;
                    // Velocity solver ile aynı konvansiyon: delta = (-pv_rel + bias) / k
                    // pv_rel > 0 → nesneler zaten ayrılıyor → düzeltme azalır
                    // pv_rel ≈ bias → yakınsadı → delta ≈ 0
                    let delta_p = (-pv_rel + bias) / k_n;

                    // Birikimli clamp: toplam pseudo-impulse ≥ 0 (çekme yok)
                    let acc_i = acc_off[mid] + cid;
                    let old_acc = acc_pseudo[acc_i];
                    let new_acc = (old_acc + delta_p).max(0.0);
                    let actual_delta = new_acc - old_acc;
                    acc_pseudo[acc_i] = new_acc;

                    let imp_p = normal * actual_delta;
                    if dyn_a {
                        pseudo_vel[idx_a].0 -= imp_p * inv_m_a;
                        pseudo_vel[idx_a].1 -= inv_i_a.mul_vec3(r_a.cross(imp_p));
                    }
                    if dyn_b {
                        pseudo_vel[idx_b].0 += imp_p * inv_m_b;
                        pseudo_vel[idx_b].1 += inv_i_b.mul_vec3(r_b.cross(imp_p));
                    }
                }
            }
        }

        // Pseudo-velocity'yi HIZA EKLEME (eski hata buydu). Bunun yerine pozisyon
        // düzeltmesi olarak dışarı yaz: Δkonum = pseudo_vel * dt. Çağıran bunu
        // doğrudan transform'a uygular; hız kanalı temiz kalır.
        for i in 0..pseudo_vel.len() {
            pos_corrections[i] = (pseudo_vel[i].0 * dt, pseudo_vel[i].1 * dt);
        }
    }
}

/// What an island's contact solve runs before each of its sweeps: the island's joints, solved in
/// the same iterations as its contacts (`pipeline.rs`, `solve_island_contacts`). `None` for a
/// solve of contacts alone.
pub(crate) type BetweenSweeps<'a> = Option<IslandJoints<'a>>;

/// An island's joints, as its contact solve sees them.
pub(crate) struct IslandJoints<'a> {
    /// The dynamic bodies they hold. The TGS path integrates their positions through its sweeps
    /// as it does a contact's bodies, so a joint's position correction lands in the position, not
    /// in the velocity the step ends with.
    pub bodies: &'a [usize],
    /// One sweep of them, given the island's velocities and how far each body has moved since the
    /// substep began (`(linear, angular)`, indexed like the velocities — the TGS path's `dp`; empty
    /// where nothing tracks it); the flag is a relax sweep (`JointSolver::sweep_island`).
    pub sweep: &'a mut dyn FnMut(&mut [Velocity], &[(Vec3, Vec3)], bool),
}

/// What the solver decided for one island, reported back so a caller can see the sweep count
/// the adaptive policy actually chose.
///
/// The engine folds this into [`PhysicsMetrics`](crate::island::PhysicsMetrics); it is returned
/// separately because the metrics are per step and this is per island, and because the policy is
/// exactly the thing a caller tuning solver cost needs visibility into.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SolveStats {
    /// The island's support depth — BFS eccentricity of its contact graph from its anchors, or
    /// from its lowest-indexed body when it has none. Zero for an island with no manifolds.
    pub island_depth: u32,
    /// Biased sweeps run for this island. On the TGS path with the block solver on and depth
    /// ≥ 5 this is the depth-adaptive count rather than
    /// [`ConstraintSolver::iterations`](ConstraintSolver::iterations); on the split-impulse
    /// path it is always `iterations`, which is that path's real behaviour and not a rounding
    /// of it.
    pub iterations: usize,
}

#[cfg(test)]
mod support_order_tests {
    use super::*;
    use gizmo_physics_core::BodyHandle;

    // Build a manifold between two bodies (contacts irrelevant to ordering — it keys on
    // the entity pair + graph depth, never reads contact points).
    fn manifold(a: u32, b: u32) -> ContactManifold {
        ContactManifold::new(BodyHandle::from_id(a), BodyHandle::from_id(b))
    }

    fn id_pairs(ms: &[ContactManifold]) -> Vec<(u32, u32)> {
        ms.iter().map(|m| (m.entity_a.id(), m.entity_b.id())).collect()
    }

    /// The ordering is a deterministic TOTAL order independent of the input (broadphase
    /// emission) order, and for a ground-anchored chain it is bottom-up from the anchor.
    #[test]
    fn support_order_is_pair_emission_invariant_and_bottom_up() {
        // Body 0 = static ground; bodies 1..=5 = a dynamic vertical chain.
        let mut rigid_bodies = vec![RigidBody::new_static()];
        for _ in 0..5 {
            rigid_bodies.push(RigidBody::new(1.0, true));
        }
        let entity_index_map: rustc_hash::FxHashMap<u32, usize> =
            (0..rigid_bodies.len() as u32).map(|i| (i, i as usize)).collect();

        // Chain contacts: ground↔1, 1↔2, 2↔3, 3↔4, 4↔5.
        let chain = [(0u32, 1u32), (1, 2), (2, 3), (3, 4), (4, 5)];

        // Several different emission orders of the SAME contact set.
        let orders: [Vec<(u32, u32)>; 3] = [
            chain.to_vec(),
            vec![(3, 4), (0, 1), (4, 5), (1, 2), (2, 3)], // shuffled
            chain.iter().rev().copied().collect(),        // reversed
        ];

        let mut results = Vec::new();
        for order in &orders {
            let mut ms: Vec<ContactManifold> = order.iter().map(|&(a, b)| manifold(a, b)).collect();
            support_order_manifolds(&mut ms, &rigid_bodies, &entity_index_map, true);
            results.push(id_pairs(&ms));
        }

        // (1) Pair-order-invariance: every emission order yields the identical solve order.
        assert_eq!(results[0], results[1], "shuffled emission changed the solve order");
        assert_eq!(results[0], results[2], "reversed emission changed the solve order");

        // (2) Bottom-up from the anchor: the ground contact (0,1) is solved first, then
        //     the chain propagates outward.
        assert_eq!(
            results[0],
            vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)],
            "expected anchor-first bottom-up support order"
        );
    }

    /// Anchor-free island (no static body): still a deterministic total order,
    /// independent of emission order (rooted at the lowest body index).
    #[test]
    fn support_order_anchor_free_is_deterministic() {
        // Four dynamic bodies (ids 1..=4), no static anchor, chained 1-2-3-4.
        let mut rigid_bodies = vec![RigidBody::new_static()]; // id 0 unused here
        for _ in 0..4 {
            rigid_bodies.push(RigidBody::new(1.0, true));
        }
        let entity_index_map: rustc_hash::FxHashMap<u32, usize> =
            (0..rigid_bodies.len() as u32).map(|i| (i, i as usize)).collect();

        let a: Vec<(u32, u32)> = vec![(1, 2), (2, 3), (3, 4)];
        let b: Vec<(u32, u32)> = vec![(3, 4), (1, 2), (2, 3)];

        let mut ma: Vec<ContactManifold> = a.iter().map(|&(x, y)| manifold(x, y)).collect();
        let mut mb: Vec<ContactManifold> = b.iter().map(|&(x, y)| manifold(x, y)).collect();
        support_order_manifolds(&mut ma, &rigid_bodies, &entity_index_map, true);
        support_order_manifolds(&mut mb, &rigid_bodies, &entity_index_map, true);
        assert_eq!(id_pairs(&ma), id_pairs(&mb), "anchor-free order must be emission-invariant");
    }
}
