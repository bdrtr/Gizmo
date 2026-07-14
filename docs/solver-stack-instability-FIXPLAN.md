# Gizmo — TGS Stack-Instability FIX PLAN (aggressive, staged, gated)

> Companion to `solver-stack-instability-2026-07-14.md` (the bug report). This is the
> actionable plan. Root cause was pinned by a deep multi-agent code analysis (with
> Box2D-v3 reference) **plus an empirical parameter sweep that REFUTED two of the
> analysis's sub-hypotheses** — read the "measured" callouts before implementing.

## Root cause — CONFIRMED (empirical + code)

A resting box stack of N≥5 spontaneously gains energy and blows up (16-stack at
~frame 853, 5-stack ~2050; peak 12–35 m/s). It is **pure solver** (isolated single
stack) and **pre-existing** (baseline `deb455f` blows too — not a regression).

**Mechanism:** the biased TGS-soft pass is **under-converged for tall stacks**. A
vertical N-stack's Delassus (effective-mass) matrix is a 1-D discrete Laplacian with
**O(N²) condition number** → full Gauss-Seidel resolution of the support-impulse
chain needs ~`(N+1)²/π²` sweeps (~29 for N=16). With only `iterations=20` biased
sweeps, steady-state **penetration stays > slop every substep**, so the soft-bias
term `bias = bias_rate·(penetration − slop)` (`tgs.rs:411`) fires forever, slightly
**over-pushing** the stack. With effectively no dissipation at rest
(`linear_damping=0.01` ≈ 4e-5 bleed/substep) the resulting position/velocity
resonance **pumps energy** frame-over-frame until nonlinear blow-up.

### Measured (parameter sweep, isolated 16-stack, 1600 frames)

| knob | result | reading |
|---|---|---|
| `iterations` 8 / 20 / 25 / **30** | blow 482 / 855 / 1493 / **never (peak 0.16)** | **biased-iteration depth IS the lever** |
| `relax_iterations` 4 / 8 / 12 / 16 / 20 / 24 | blow 855 / 851 / 639 / 642 / 683 / 654 | **relax does NOT help — REFUTES "relax too shallow to drain"** |
| `contact_hertz` 10 / 60, `damping` 20, `max_bias_velocity` 0.5, `slop` 0.001/0.02 | only shift the frame, never prevent | **REFUTES "soft-coefficient timestep mismatch" as the primary lever** |
| `warm_start_factor` 0 | blows at frame **24** (instant) | warm-start is **stabilizing**, not the cause |
| residual KE/substep | iters20 ≈ 1e-3, iters30 ≈ 6e-5 (**30× lower**) | the residual is the accumulator |

**Corrections to the analysis, from the sweep:**
- ❌ "Relax pass too shallow" — refuted (more relax doesn't help).
- ❌ "Soft-coefficient timestep mismatch" as primary — refuted (soft-param tweaks don't help).
- ⚠️ A **non-injection VELOCITY guard** (clamp separating vn after relax) is **likely
  ineffective**: the relax pass already drives resting `vn→0` and raising it doesn't
  help, so the persistent channel is the bias-driven **position over-push (dp)**, not
  the velocity. Don't lead with the velocity guard.
- ✅ Confirmed: **under-converged biased GS** is the driver; more biased depth fixes it.

## Aggressive staged plan (gates in **bold**)

### Stage 0 — Lock the RED test (mandatory, zero behaviour change)
Extend `crates/gizmo-physics-rigid/tests/soak_and_golden.rs`: isolated resting
stacks N ∈ {2, 5, 16, 32} on static ground, step 1/60 for **1500 frames**, assert
`max|v| < 0.5 m/s EVERY frame` (today's `soak_tall_stack_n16` stops at ~240–600
frames, before the ~853 blow-up, so it passes and hides the bug). Add a cfg-gated,
determinism-safe trace of per-substep total island KE + deepest/mid-stack penetration.
**Gate: the new test must FAIL now (N=16 blows ~853) and penetration must sit > slop.
If penetration is ≤ slop, the mechanism is wrong — STOP and re-diagnose.**

### Stage 1 — Order-optimal deterministic contact solve  ← the linchpin
Sort each island's manifolds/contacts into **support order** before the sweep: for
stack/chain islands, bottom-up along the dominant normal (topological traversal
outward from static anchors); make it a **deterministic total order independent of
broadphase pair-emission order**. Correct GS order propagates the support front **one
contact per sweep → ~O(N) sweeps instead of O(N²)**, so `iterations=20` covers N up to
~20+ at rest.
**Why this is the linchpin:** it also makes the island solve **pair-order-invariant**,
which (a) recovers the sleep quality lost to pair-order shifts and (b) **unblocks
incremental broadphase** (docs/physics-perf-2026-07-14.md). One fix, three wins.
**Gate: N=16..32 stay bounded for 1500 frames at `iterations=20` AND a shuffled-pair
test gives an identical result hash. If tall towers still creep, go to Stage 2.**
Risk: naive height-sort assumes near-vertical; general piles need topological order —
restrict height-sort to detected 1-D chains, leave general islands on Stage-2 solve.
Determinism goldens change → re-bless deliberately (user accepted).

### Stage 2 — Box2D-v3 convergence quality (if Stage 1 leaves a gap)
(a) **Static softness**: contacts touching a static/kinematic body use
`b2MakeSoft(2·contact_hertz)` (branch on `!dyn_a||!dyn_b` at prepare) — the base-of-
stack ground contact is currently half as stiff as Box2D, letting the whole column
breathe. (b) **Timestep-consistent soft params** — the biased loop advances dp by
`dt/iterations` per sweep but builds `bias_rate/mass_scale/impulse_scale` for `h=dt`
(`tgs.rs:95-99`); recompute for the actual per-sweep h so ζ is truly critical. (c)
**Ramp `warm_start_factor`→1.0** for persisting contacts by `manifold.lifetime`.
**Gate: with the Stage-0 test green at `iterations=20`, A/B solver time must beat the
`iterations=30` band-aid. If any N still blows, go to Stage 3.**

### Stage 3 — Direct chain solve (aggressive, definitive)
For detected 1-D chain (stack) islands, replace iterative GS on the normal
constraints with a **direct tridiagonal (Thomas-algorithm) block solve** of the
Delassus system: **exact convergence in O(N), zero residual regardless of height.**
Handles N > 64 towers. Reserve for if Stages 1–2 leave a measurable gap.
Risk: chain-detection heuristic; large numeric change (re-bless); CCD fallback branch
must be re-validated.

### Stage 4 — Full Box2D-v3 substep-loop conformance (reserve)
Restructure to Box2D v3's canonical loop: Prepare once, then per substep
{IntegrateVel(h) → WarmStart → 1 biased Solve → IntegratePos(h) → 1 Relax}, one final
Restitution pass + StoreImpulses, and **recompute separation each sweep by ROTATING
stored anchors** (`b2RotateVector`) instead of the frozen anchors reused across all 24
sweeps (`tgs.rs`). Removes frozen-anchor linearization drift on wobbling towers.
Reserve for an AAA-quality gap only.

### Band-aid fallback (documented, NOT preferred)
Raise default `iterations` 20→~28, or make it **island-depth-adaptive** (deep stacks
get more sweeps). Fixes 16-stacks but costs ~40% more solver sweeps (undoes part of
the 2026-07-14 solver perf work) and still fails for tall enough towers. Use only as a
stop-gap while Stage 1 lands.

## Verification (every stage)
Stage-0 RED test (N up to 32, 1500 frames, max|v| bounded + penetration bounded) +
full `gizmo-physics-rigid` suite (soak / **soak_falling_stack** / determinism / sleep /
CCD / joints) + `gizmo-physics-core`. Where a stage changes numeric output, **re-bless
determinism goldens deliberately and document why** (no hard-coded golden hashes exist
today; the suites are self-consistency + behavioural tolerances). Add a
**pair-order-invariance** test at Stage 1 (shuffle broadphase emission → identical hash).

## Recommended entry
Stage 0 (RED test) → **Stage 1 (order-optimal solve)** — it is the highest-leverage
single change: fixes the instability at default iterations AND unblocks sleep-robustness
AND incremental broadphase.
