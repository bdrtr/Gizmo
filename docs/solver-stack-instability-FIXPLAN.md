# Gizmo — TGS Stack-Instability FIX PLAN (aggressive, staged, gated)

> Companion to `solver-stack-instability-2026-07-14.md` (the bug report). This is the
> actionable plan. Root cause was pinned by a deep multi-agent code analysis (with
> Box2D-v3 reference) **plus an empirical parameter sweep that REFUTED two of the
> analysis's sub-hypotheses** — read the "measured" callouts before implementing.

---

> ## ✅ FIX SHIPPED (2026-07-15) — manifold BLOCK solver (partial: fixes N≤16)
>
> **Root cause (final): lateral BUCKLING.** A resting column leans (its lean/tilt grow
> exponentially while |v| stays tiny) then topples — an inverted-pendulum instability whose
> critical height was far too low (~N=5) because the contact solve gave too weak a
> lateral/rotational restoring torque. Confirmed by `probe_buckling_vs_pump`. It is shared by
> BOTH solvers and is NOT under-convergence / warm-start / soft-params (all refuted below).
>
> **The fix — manifold block solver** (`crates/gizmo-physics-rigid/src/solver/block.rs` +
> `tgs.rs::tgs_sweep_block`): solve each contact manifold's up-to-4 **coplanar normal impulses
> JOINTLY** (a regularised active-set LCP) instead of sequential Gauss-Seidel. The joint solve
> realises the impulse *redistribution* (deeper corner carries more, lifting corner separates)
> that produces the tilt-resisting torque — the restoring stiffness GS under-resolved. Two
> details were essential: (1) a 4-coplanar-contact block is **rank-deficient** (4 contacts,
> 3 DOF) → **Tikhonov regularisation** (`block_regularization`, default 0.1) or it produces a
> huge garbage impulse (10-box tower flew to 18 m); (2) it stays **rigid** (soft mass-scaling
> measurably weakens the fix). Plus **support-ordering + adaptive iterations** (sweeps scale
> with island support-depth) so support propagates up the column. Defaults: `block_solver=true`,
> `support_ordering=true`.
>
> **Result:** buckling critical height raised ~5 → ≥16; residual energy cut ~100× (20 m/s
> blow-ups → ~0.2 peak). Realistic stacks (the field bug was 5-box columns; now stable well
> beyond, to 16) are fixed. Full suite green in debug AND release, no determinism re-bless.
> Live regression test: `soak_resting_stacks_stay_bounded` (N∈{2,5,16}).
>
> **Whole-chain DIRECT solve — IMPLEMENTED, opt-in** (`direct_chain_solve`, default OFF;
> `solver/block.rs::solve_island_normals` + `tgs.rs::tgs_sweep_island`): for a tall bucklable
> chain (support-depth ≥5, ≤256 contacts) it solves ALL the island's normal impulses JOINTLY
> each sweep (dense active-set LCP over the full island Delassus matrix), resolving the
> inter-manifold support coupling exactly. It **raises the stable-tower boundary** (e.g. N24 and
> N40 stay bounded at 3000 frames where block-only failed) and roughly doubles survival time for
> the rest — a real improvement. Default OFF because it costs **O(n³)** per chain (a 16-stack
> soak went 10 s → 46 s in debug) for an **incomplete** benefit: it does NOT robustly fix N32+
> (still chaotic — e.g. N32 blows @1420, N40 OK). Reason: it resolves only the NORMAL
> constraints; the lean is also mediated by **friction** (solved sequentially) and frozen
> anchors, so complete elimination needs a joint **normal+friction** nonlinear complementarity
> solver (research-grade). Turn it on where you have tall towers and can afford the cost.
>
> **REMAINING (open):** robust stability for extreme towers (N≥32) — needs the normal+friction
> joint solver above. Acceptance test: `soak_extreme_tower_n32_stays_bounded` (#[ignore]d).

---

> ## ⛔ CORRECTION (2026-07-14, execution session) — the root cause below is REFUTED
>
> Stage 0 (the RED test) and Stage 1 (support-order solve) were implemented. Then an
> empirical **iteration ladder + targeted sweeps refuted the "under-converged Gauss-
> Seidel" root cause this plan is built on.** Do NOT implement Stages 1–3 as written;
> they target a mechanism that isn't the driver. Evidence:
>
> | test | result | conclusion |
> |---|---|---|
> | **iteration ladder** N=32, iters 30…110, ordering on/off | blow-up frame CHAOTIC / non-monotonic (iters=35→@63, iters=60→@1458); never stable | under-convergence would be **monotonic** — REFUTED |
> | doc claim "iters=30 → never (N16)" | does **not** reproduce — N16 iters=30 blows @1417 | the original convergence evidence is not robust |
> | **support ordering** (Stage 1) on vs off | blow-up frame barely moves (868→860) | not an O(N²)→O(N) GS-order problem for this SOFT solver — REFUTED as an instability fix |
> | **frictionless** stack | CATASTROPHICALLY worse (blows ~frame 2–3) | friction is **stabilising**; injection is in the NORMAL/position channel, not friction |
> | hertz / damping / relax / timestep-consistent-`h` (Stage 2b) | only shift the frame, or make it worse | soft-param tuning is a dead end — REFUTED |
> | total energy (KE+PE) over time | ~conserved through a long plateau, then SUDDEN escape | **metastable**, not a slow exponential pump |
>
> **Corrected diagnosis:** a **metastable normal-channel resonance** seeded by float
> asymmetry in the 4-point manifold. The residual max|v| hovers right at the 0.05 sleep
> threshold, so the stack **never sleeps**, and eventually a fluctuation escapes
> nonlinearly. It is NOT under-convergence and NOT fixable by iteration/soft-param tuning.
>
> **What shipped this session:** the RED test (`soak_resting_stacks_never_gain_energy`,
> `#[ignore]`d — it is the acceptance test for the eventual fix) and the support-order
> solver (kept, **default OFF**, unit-tested for pair-order-invariance — its one real,
> independent win: it unblocks incremental broadphase; it does NOT fix the instability).
>
> ### Stage 4 executed (2026-07-15) — also does NOT fix it, but found the real redirect
>
> Implemented both Stage-4 components behind flags and measured:
> - **Interleaved relax** (biased→integrate→relax per iteration, Box2D-style): WORSE
>   (N16 868→418). The "accumulated bias velocity" hypothesis is refuted. Removed.
> - **Rotating anchors** (`ConstraintSolver.rotating_anchors`, kept, default OFF): mild —
>   delays N16 (868→1114) but does NOT fix it and does not help N32.
>
> **⭐ Pivotal finding:** the **SI (split-impulse) solver also blows up the resting stack,
> even earlier** (N16@360, N32@67) with `use_tgs_soft=false`. So the instability is **NOT
> TGS-specific** — two independent solvers both destabilise resting stacks. The root is
> **upstream / shared**, i.e. in what both paths have in common: **warm-starting** (both
> re-apply last frame's impulses; `warm_start=0` → instant blow, so it's load-bearing but
> mis-applied?) and/or the **narrowphase box-box 4-point manifold** (contact-point
> identity/positions shifting frame-to-frame as the stack micro-wobbles → warm-start
> impulses land on slightly-wrong points → torque error → injection). NOT a solver-inner
> issue.
>
> ### RESOLVED DIAGNOSIS (2026-07-15) — it is lateral BUCKLING, a linear instability
>
> A buckling-vs-pump probe + a 10-agent investigation workflow + a signed-drift probe settled it:
>
> - **It is LATERAL BUCKLING, not a vertical energy pump.** `probe_buckling_vs_pump`: the column's
>   lean (max box |x|+|z|) and tilt grow EXPONENTIALLY (τ≈100 frames) for hundreds of frames while
>   max|v| stays <0.05; then the column topples and PE→KE gives the sudden escape (at N16 blow-up,
>   vy≈0.015 vs |v|=0.50 → lateral/rotational, not a bounce). Explains every constraint: height-
>   scaling = lower buckling critical load; friction-stabilises = friction resists the lean
>   (frictionless→frame 2); both solvers; metastable; never-sleeps.
> - **It is a GENUINE LINEAR INSTABILITY, not a discrete bug.** Signed-drift probe: the lean
>   direction meanders randomly early, then locks into a NOISE-selected direction (−x here) near
>   frame ~400 and grows exponentially → the equilibrium eigenvalue is just above 1 for N≥5. Not a
>   fixed solve-order bias, so symmetrisation cannot cancel it.
> - **The workflow's top candidate (warm-start carry-over) was REFUTED by experiment.** Added a
>   configurable `warm_start_match_tolerance` (default 0.02) and swept it: tightening 0.02→1e-3→1e-4
>   makes the blow-up EARLIER (N16 868→635→22), and warm_start=0 blows at frame 24. **Warm-start is
>   STABILISING, not the injection channel.** The box-box manifold is per-corner correct
>   (contacts.rs `clip_box_box`), so it is not a manifold bug either. Only rotating anchors mildly
>   help (868→1114), consistent with better torque-arm accuracy.
>
> **CONCLUSION:** the iterative contact solver's effective lateral/rotational restoring stiffness for
> a stacked column is slightly below the buckling-critical value, so tall stacks (N≥5) are linearly
> unstable. This is a solver-STRUCTURE limitation, not a small discrete bug — no
> iteration/soft-param/warm-start knob fixes it.
>
> **Recommended fix (structural — needs a dedicated pass, user's call on approach):**
> 1. **Contact-manifold BLOCK solver** — solve a box-box manifold's up-to-4 coplanar normal
>    constraints JOINTLY (coupled) instead of sequential Gauss-Seidel, so the tilt-resisting impulse
>    redistribution is realised exactly → raises restoring stiffness above critical. Most targeted;
>    Box2D uses a 2-point block solver for exactly this.
> 2. **Shock propagation** — an extra post-solve pass, bottom-up along the (already-built) support
>    order, treating each contact's lower body as infinite-mass → rigidifies the support chain.
>    Reuses `support_ordering`.
> 3. **Sleep-before-buckle** — force a near-quiescent stack to sleep in the first ~100 frames
>    (before the lean grows past linear), robust to the solver's velocity-jitter spikes. Smaller/
>    heuristic; freezes the stack near-vertical.
>
> Shipped this session (all default-preserving, suite green): the buckling/signed-drift probes, the
> `warm_start_match_tolerance` config (default 0.02 = historic behaviour), and the corrected diagnosis.

---

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
