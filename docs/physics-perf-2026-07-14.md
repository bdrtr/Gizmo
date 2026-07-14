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

## Results (2000-box active scene)

| Regime | metric | before | after | speedup |
|---|---|---:|---:|---:|
| Worst frame | narrowphase | 170 ms | 9 ms | **~18×** |
| Worst frame | **total frame** | **262 ms** | **57 ms** | **~4.6×** |
| Active (avg) | solver | 10.8 ms | 4.9 ms | ~2.2× |
| Active (avg) | total frame | 24.3 ms | 13.4 ms | ~1.8× |

## Verification

- Broadphase: 3 proptests (incl. exact-match-vs-brute-force) + 12 unit tests.
- Solver: full 122-test `gizmo-physics-rigid` suite incl. `soak_tall_stack_n16`,
  `soak_falling_stack_survives_impact`, CCD, joints, and the determinism suite.
- 85 `gizmo-physics-core` tests + full-workspace release build.
- No golden-hash re-baseline needed: determinism tests are self-consistency
  checks and the soak/golden tests are behavioural tolerances — all green.

## Remaining opportunities (not done — constant-factor, not algorithmic)

Ranked by expected value; each needs its own measure-verify pass:
1. **Hoist per-contact solver constants.** `tgs_sweep` recomputes `r_a`/`r_b`,
   `k_n`, the friction basis `t1`/`t2` (a `normalize()`/sqrt) and `k_t1`/`k_t2`
   for every contact on **every one of the 24 sweeps** — they're constant across
   sweeps. Precompute once into a flat array ⇒ est. ~1.5–2× on the solver's real
   work (now the dominant cost once bodies stay awake). Can be made bit-identical.
2. **Incremental broadphase.** `broadphase_step` does `clear()` + full re-insert
   every frame (O(N log N)), defeating the DBVT's fat-AABB early-out (already
   implemented at `aabb_tree.rs`). Drive `update()` for movers instead ⇒ large win
   in the common case where most bodies sleep; no help when everything moves.
3. Batch tiny islands into single rayon tasks (`with_min_len`) to cut per-island
   task-dispatch overhead.
