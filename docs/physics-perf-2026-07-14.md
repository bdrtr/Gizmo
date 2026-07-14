# Gizmo — Physics Performance Round (2026-07-14)

> Two **algorithmic** bottlenecks (both accidentally O(N²)) found by profiling a
> large active box scene, fixed, and verified behaviour-preserving. This is the
> follow-up to the narrowphase batch-SIMD investigation, whose Step-0 gate
> ([narrowphase-batch-simd-plan.md](narrowphase-batch-simd-plan.md)) correctly
> said "don't micro-SIMD the SAT sweep" — the real wins were elsewhere.

## Method

Profiled `world.step` stage timings (broadphase / narrowphase / solver /
integration, already instrumented in `PhysicsMetrics`) across scales
(500 / 2000 boxes) on a **sustained-active** dense-stack scene, reporting the
early (all-awake) vs settled regimes and the worst single frame. Both dominant
stages grew **super-linearly** with body count — the signature of a hidden N².

## Bottleneck 1 — broadphase `query_pairs` was O(P²)

`DynamicAabbTree::query_pairs` (`crates/gizmo-physics-core/src/broadphase/aabb_tree.rs`)
ran two phases:
1. a root-seeded dual-tree descent, then
2. `collect_internal_pairs` — the standard BVH self-query (each internal node's
   left subtree vs its right).

Phase 2 alone is complete and duplicate-free (each colliding pair has a unique
lowest-common-ancestor node). Phase 1 re-produced the same pairs, and to drop the
duplicates `descent_pair` did `pairs.contains(&pair)` — a **linear scan of the
whole result vector per candidate ⇒ O(P²)** in the reported-pair count. For an
active box field P is O(N) (tens of thousands), so this was ~10⁹ comparisons per
frame — billed to the narrowphase stage.

**Fix:** delete phase 1, drop the `contains` dedup. Output pair set is unchanged
(proptest `query_pairs_exact_match_brute_force` still passes). Commit `0aaa20d`.

## Bottleneck 2 — TGS solver scratch/loops sized to the whole world

`solve_contacts_tgs` (`crates/gizmo-physics-rigid/src/solver/tgs.rs`) is called
**once per island**, but allocated `active` / `has_real` / `dp` as
`vec![…; n_bodies]` (full world) and looped `for i in 0..n_bodies` — including the
dp-integration loop inside every one of the 20 biased iterations. `solve_contacts`
also zeroed the full-length `pos_corrections` per island. For a scene of many
tiny separated stacks (2000 boxes ≈ hundreds of islands) that is
**O(n_islands × n_bodies)** work + hundreds of full-world allocations per substep.

**Fix:** thread the island's distinct body-index list (already computed at the
call site as `island_indices`) into the solver — drop the `active` array
(island_bodies *is* the active set), make `has_real` a small FxHashSet, make `dp`
a reused thread-local reset only for island_bodies (islands are disjoint), iterate
island_bodies in the loops, and zero `pos_corrections` island-locally. Per-contact
math untouched ⇒ behaviour preserved. Commit `056bbaa`.

## Bottleneck 3 — TGS re-derived per-contact constants on every sweep

The live TGS-Soft solver runs **24 sweeps per substep** (20 biased + 4 relax), and
`tgs_sweep` recomputed, for every contact on **every** sweep, the quantities that
are constant for the whole solve: `r_a`/`r_b`, the effective normal mass `k_n`, the
friction basis `t1`/`t2` (with a `normalize()`/sqrt), and the tangent effective
masses `k_t1`/`k_t2` — plus per-manifold `inv_world_inertia_tensor` (a matrix
build+rotate). That's ~24× redundant trig/matrix work.

**Fix:** precompute those constants once per island into a flat `Vec<Prepared>`;
the sweep then does only the velocity/dp-dependent work and mutates the impulse
accumulators in place (written back to the manifold once at the end). Every hoisted
value uses the **identical expression** the old sweep used, and the flat array
reversed reproduces the old (reverse-manifolds + reverse-contacts) order exactly ⇒
**bit-identical**. Verified by a state-hash oracle: a rich scene (resting stack +
tumbling high-speed impact + tilted contact + spun boxes) produced the
**same hash `0xb9dd08d61b586477`** before and after. Commit *(this round)*.

## Results (2000-box active scene, cumulative)

| Regime | metric | original | after all 3 | speedup |
|---|---|---:|---:|---:|
| Worst frame | narrowphase | 170 ms | 9 ms | **~18×** |
| Worst frame | **total frame** | **262 ms** | **46 ms** | **~5.7×** |
| Sustained | solver | ~35 ms | ~22 ms | ~1.6× (hoist) |
| Active (avg) | solver | 10.8 ms | 3.3 ms | **~3.3×** |
| Active (avg) | total frame | 24.3 ms | 10.8 ms | **~2.25×** |

(Broadphase O(P²)→O(P) owns the narrowphase collapse; island-local scratch + the
per-contact hoist together take the solver from 10.8 ms to 3.3 ms in the active
regime.)

## Verification

- Broadphase: 3 proptests (incl. exact-match-vs-brute-force) + 12 unit tests.
- Solver: full 122-test `gizmo-physics-rigid` suite incl. `soak_tall_stack_n16`,
  `soak_falling_stack_survives_impact`, CCD, joints, and the determinism suite.
- 85 `gizmo-physics-core` tests + full-workspace release build.
- No golden-hash re-baseline needed: determinism tests are self-consistency
  checks and the soak/golden tests are behavioural tolerances — all green.

## Incremental broadphase — ATTEMPTED, reverted (blocked on solver order-robustness)

Dropping the per-substep `spatial_hash.clear()` and letting the DBVT `insert`
early-out drive an incremental update is a large, correct broadphase win on its own:
measured broadphase **~5–9 ms → ~0.6 ms at 2000 boxes (~8–16×)**, and a temporary
completeness oracle (brute-force tight-overlap pairs, asserted ⊆ the incremental
tree's `query_pairs`, run across the whole suite) confirmed **no missed pairs** —
the fat-AABB early-out only skips when the stored box still contains the tight one,
so it is provably conservative.

**But it changes the *order* of `query_pairs`** (the incremental tree's shape is
update-history-dependent, unlike the fresh-rebuild's insertion-order shape), and the
constraint solver is **Gauss-Seidel — order-sensitive**. This surfaced two competing
regressions that neither variant escapes:
- **Incremental order (unsorted):** `soak_falling_stack_survives_impact` topples —
  the metastable perfectly-aligned falling stack relies on a regular solve order for
  the symmetric-GS bias cancellation, and the irregular AVL-traversal order breaks it.
- **Canonical `(min_id,max_id)` sort** (fixes the stack): the wide settling scene
  stops sleeping — **92.5% → 1.2% sleeping bodies**, contacts 2.4k → 31.6k, solver
  jumps to ~59% — because the sorted order converges worse for resting stacks, leaving
  residual jitter above the sleep threshold. (This regression is invisible to the
  behavioural suite — soak checks stability/height, not sleep — so it would have
  shipped silently. Caught only by `wide_scene_profile`'s sleep read-out.)

**Prerequisite:** make the solver robust to contact-solve order (order-independent
convergence, or sort manifolds by a stable key AND fix the resulting stack-sleep
convergence) BEFORE re-attempting incremental broadphase. Related known effect: the
`0aaa20d` `query_pairs` phase-1 removal already shifted the order enough to move
wide-scene sleeping ~92.5% → ~73.5% (minor; the O(P²)→O(P) win dwarfs it, tests green).

## Other remaining opportunities

1. **Solver order-robustness** (the prerequisite above) — the highest-leverage next
   item, since it unblocks incremental broadphase *and* recovers the sleep quality lost
   to pair-order shifts. Needs a symmetric/Jacobi refinement or an order-canonical
   manifold sweep whose convergence still settles stacks to sleep.
2. Batch tiny islands into single rayon tasks (`with_min_len`) to cut per-island
   task-dispatch overhead.
3. Reduce the biased-iteration count (currently 20) — a quality/perf trade-off,
   NOT free: `soak_tall_stack_n16` is the stability canary that would gate it.
