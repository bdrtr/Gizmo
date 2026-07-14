# Gizmo — Narrowphase Box–Box Batch-SoA SIMD Plan

> **Status: STOPPED at Step 0 — the gate FAILED (measured 2026-07-14).** The plan
> below is retained for the record and in case the profile changes materially.
> A companion to the `*_FIXPLAN.md` convention: a staged plan with explicit
> **decision gates** so we stop the moment the data says the win isn't there.
> Step 0 is a measurement gate — no production code changes until it passes.

## ⛔ Step 0 result — gate FAILED, do not proceed

Temporary `Instant` instrumentation split `narrowphase_and_collision_step` into
parallel SAT **compute** vs sequential **post-processing**, profiled on the
box–box-heavy `wide_scene` (2000 boxes) at HEAD `deb455f`. Worst-case **active**
frame (~32k contacts, ~30 ms/frame), two consistent runs:

| Piece | ms/frame | % of narrowphase | **% of frame** |
|---|---:|---:|---:|
| **Box–box SAT compute** (the batch-SoA target) | ~1.0 | 27% | **~3.3%** |
| Post-processing (cache/manifold/trigger — *not* batchable) | ~2.77 | 73% | ~9.2% |
| Pair enumeration + dormant filter (metric − above) | ~4.3 | — | ~14% |
| Frame total | ~30 | | 100% |

**Why this kills it:**
- The batch-SoA target is **~3.3% of the worst-case frame**, and the measured
  "compute" figure *includes* clipping + GJK fallbacks — the pure SAT sweep that
  batching would actually accelerate is **< 1 ms/frame**.
- SAT compute is **already rayon-parallel**, so its wall-clock ceiling is ~1 ms.
  Even a hypothetical *free* SAT sweep saves < 1 ms in the worst frame and ~0 in
  typical/settled frames.
- Post-processing (which batch-SoA cannot touch) is **3× larger** than compute,
  and pair-enumeration overhead is **4× larger**.
- The stale **"~82%"** figure in the code comment (`pipeline.rs`, dormant-skip
  justification) and in earlier notes is **obsolete**: after dormant-skip +
  pre-size + FxHash + parallel narrowphase, narrowphase is now ~34% of stage time
  and its *SAT compute* sub-slice is ~3% of the frame — not 82%.

**Decision:** **STOP.** Do not do Steps 1–4. The multi-day batch-SoA investment
(plus a determinism re-baseline) cannot move the frame. If this scene's profile
ever shifts (e.g. a much larger active box count with post-processing already
optimized away), re-run Step 0 before reconsidering. The real optimization
targets in this scene, if any, are elsewhere: pair enumeration (`query_pairs`) and
the narrowphase post-processing (contact-cache build / manifold cloning).

*The instrumentation was temporary and has been fully reverted — HEAD is clean.*

## Why batch-SoA (and not per-pair SIMD)

The [2026-07-14 perf-hasher round](../) tried to SIMD-accelerate the box–box
narrowphase **per pair** — twice:

1. `ArrayVec` allocation elimination on the SAT axis buffer, and
2. `glam` `Mat3A` (SIMD 3×3) for the axis projections.

**Both regressed.** The takeaway: the scalar narrowphase is already well
auto-vectorized by the compiler, and there is no headroom to squeeze inside a
single pair. Per-pair SIMD only adds setup/shuffle cost that the compiler was
already avoiding.

The **only** remaining path to a real SIMD win is **batch-SoA**: process **8
box–box pairs at once** across `wide::f32x8` lanes — one independent pair per
lane — so the SIMD setup cost is amortized over eight pairs instead of paid
per-pair. (`wide` is already a dependency of `gizmo-physics-rigid`; it has been
unused to date.)

## Target

Move the box–box **SAT sweep** (the 15-axis separation test — today
`crates/gizmo-physics-core/src/narrowphase/mod.rs:186-202`, driven by
`sat_penetration` in `narrowphase/contacts.rs`) onto 8-wide SoA SIMD.

**Clipping stays scalar.** Sutherland–Hodgman clipping (`clip_box_box`) has
data-dependent, variable-length output — a poor SIMD fit — and only runs for the
subset of pairs that actually overlap. Only the *uniform* SAT sweep is batched.

## Where it plugs in

`crates/gizmo-physics-rigid/src/pipeline.rs :: narrowphase_and_collision_step`
has two clearly separated halves:

- **Parallel SAT compute** — `active_pairs.par_iter() … test_collision_manifold …
  .collect()` (today `pipeline.rs:260-374`). **This is the batch-SoA target.**
- **Sequential post-processing** — contact-cache build, manifold assembly,
  trigger events, soft-body handling (today `pipeline.rs:376+`). **Batch-SoA
  cannot touch this**; if it dominates, the whole effort is moot (see Step 0).

## Steps (with decision gates)

### Step 0 — MEASURE FIRST (gate)
Micro-profile: separately time the **parallel SAT compute** vs the **sequential
post-processing** inside `narrowphase_and_collision_step` (temporary `Instant`
instrumentation in `pipeline.rs`; do **not** commit it). Also confirm the
narrowphase stage is still a meaningful fraction of the frame *after* the
`deb455f` pre-size change.

**Gate:** proceed only if the SAT compute is genuinely dominant. If
post-processing or another stage dominates, **STOP** — box–box SAT SIMD cannot
move the frame. (The ArrayVec + micro-SIMD regressions from this round already
hint the compute path is tight; a batch win is *not* guaranteed either.)

Measure on an **active** box–box-heavy scene, not just the settled/dormant one —
the dormant-pair skip zeroes narrowphase once bodies sleep, which would understate
the load the optimization actually targets (worst-case active frames).

### Step 1 — Separate SAT from clipping
Extract `box_box_sat(pair) -> Option<(min_pen, axis, flip)>` as a pure scalar
function (the logic currently at `mod.rs:186-202`). Clipping stays a separate
call. No behavior change; pure refactor, keep tests green.

### Step 2 — 8-wide SAT + differential test (gate)
Implement `box_box_sat_x8(8 pairs, SoA) -> [Option<(pen, axis)>; 8]` — the 15-axis
projection + penetration across `wide::f32x8` lanes. **Differential test:** for
random batches of 8 pairs, assert `x8` result == scalar `box_box_sat` within an
f32 tolerance.

**Gate:** synthetic batch micro-bench. If `x8` SAT is not *clearly* faster than
scalar (SoA gather overhead can eat the win), **STOP — do not touch the
pipeline.** This de-risks the large investment before it's made.

### Step 3 — Restructure the pipeline for batching
Split candidate pairs by shape type; batch box–box pairs into groups of 8 (SoA
gather of 8× pos/rot/half-extents), call the batched SAT, and run **scalar**
clipping for the lanes that overlap. `rayon` parallelizes **across** batches;
SIMD works **within** each batch — the two compose. Sphere/compound pairs stay
scalar.

### Step 4 — Determinism re-baseline + verify + measure
Re-baseline golden hashes (see Risks), run the full differential + determinism
suite, and measure end-to-end with the interleaved profiler.

## Risks

- **Determinism.** Each lane is one pair with the *same* op order, so it *may*
  stay bit-identical to scalar. **But** if `wide` emits FMA (fused multiply-add),
  the rounding differs and the hash **changes**. Expect a re-baseline:
  update the golden hashes in `crates/gizmo-physics-rigid/tests/determinism.rs`
  and the soak/golden suite, and document it. Persisted replays become invalid —
  acceptable in dev (no durable replays). **User has accepted this risk.**
- **SoA gather overhead.** Cost of gathering 8 pairs into SoA layout could eat
  the batch win (Step 2's gate tests exactly this).
- **Divergent control flow.** No SAT early-out: compute **all 15 axes on all
  lanes** and mask, instead of branching per lane (uniform work). Clipping stays
  scalar. Only box–box is batched (sphere/compound stay scalar).
- **Lesson from this round:** *don't assume without measuring* — both the
  allocation and micro-SIMD intuitions turned out wrong. Measure every step with
  the interleaved `wide_scene_profile` (git-stash baseline binary + alternate the
  measured build to cancel thermal drift).

## Effort & tooling

A multi-day, standalone project. Verification tooling is ready:
`demo/src/bin/wide_scene_profile.rs` (interleaved), the cross-process determinism
oracle (`demo/tests/cross_process_determinism.rs` +
`crates/gizmo-physics-rigid/tests/determinism.rs`), and
`crates/gizmo-physics-core/src/narrowphase/tests.rs`.

When ready, start at **Step 0**. Do not skip the gate.
