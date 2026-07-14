# Gizmo — LATENT BUG: TGS solver slowly destabilizes long-resting stacks

> **Confirmed pre-existing latent defect** (found 2026-07-14 while investigating why
> broadphase pair-order affects sleep). **Not a regression** — reproduces identically
> on the pre-perf-round baseline `deb455f`. Not fixed yet; documented for a dedicated
> solver-stability pass. Analogous to `LATENT_BUGS_FIXPLAN.md` in spirit.

## Symptom

A **resting** box stack that has fully settled will, after enough frames,
spontaneously gain energy and **blow up** (bodies reach tens of m/s), then scatter
and re-settle. Taller stacks blow up sooner. Measured on isolated single stacks
(no sleep/multi-island interaction), 60 fps, default solver:

| stack | blow-up frame | peak speed |
|---|---:|---:|
| 1 box  | never | 0.04 |
| 2 boxes| never | 0.04 |
| 5 boxes| ~2050 (~34 s) | 12 m/s |
| 16 boxes| ~853 (~14 s) | 27 m/s |

Crucially, **the 16-stack blows up at frame ~853 but `soak_tall_stack_n16` only
runs 600 frames** — the instability is *just* beyond the test horizon, which is why
it ships green. In a large scene (2000 boxes / 400 columns) the aggregate blow-up
lands around frame ~1100–1900 and the scene never re-sleeps (sits agitated at
1–3 m/s), which also explains the wide-scene sleep read-out collapsing over time.

## Root cause — incomplete convergence, not a discrete bug

The energy source is **insufficient solver convergence per substep**, confirmed by a
parameter sweep on the 16-stack:

| change | blow-up frame |
|---|---:|
| default (`iterations=20`) | 853 |
| `iterations=8` | 482 (sooner) |
| **`iterations=30`** | **never (peak 0.19)** |
| `warm_start_factor=0` | **24 (instant)** — warm-start is *stabilizing*, not the cause |
| `contact_hertz` 10/60, `damping` 20, `max_bias_velocity` 0.5, `slop` 0.001/0.02 | only shift the frame, none prevent it |

The 20 biased TGS sweeps don't fully resolve a tall stack each substep, so gravity's
per-substep energy isn't completely removed and the soft-bias slightly overshoots;
the residual **accumulates** frame-over-frame (positive feedback) until it blows up.
More iterations → residual small enough to never accumulate → stable. This is the
classic Gauss-Seidel-for-tall-stacks convergence-rate limitation (convergence scales
~`iterations / stack_height`).

**This is also the root of the pair-order ⇒ sleep coupling** seen in the incremental-
broadphase attempt (`docs/physics-perf-2026-07-14.md`): contact-solve order changes
the convergence rate, which changes how fast this marginal instability grows, which
changes how many bodies are quiet enough to sleep at a given frame. The `0aaa20d`
pair-order shift moved wide-scene sleeping ~92.5% → ~73.5% at frame 300 for exactly
this reason.

## Fix options (for a dedicated pass — NOT done)

1. **Better convergence without more global iterations** (preferred): solve stack
   contacts in a convergence-optimal order (bottom-up support propagation), a
   block/graph-colouring solver for stacked islands, or a small per-island adaptive
   iteration count scaled by island depth. This fixes the instability AND makes sleep
   robust to pair order AND would unblock incremental broadphase.
2. **Raise the default `iterations`** (band-aid): 30 stabilizes 16-stacks but costs
   ~50% more solver sweeps (partially undoing the 2026-07-14 solver perf work) and
   still fails for tall enough stacks (e.g. 32+).
3. **Sleep resting stacks before the instability grows**: only works if convergence
   is good enough to drive the residual below the 0.05 sleep threshold and keep it
   there — i.e. it depends on (1) anyway.

## Repro

Isolated stack of N≥5 boxes on a static ground, `world.step(1/60)` for ~1000+ frames,
watch `max |velocity|`. Or `iterations=30` to confirm the convergence hypothesis.
(Diagnostic was a temp `demo/src/bin` binary, removed.)
