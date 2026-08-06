# Gizmo Engine — Engineering Document

> The **single** internal reference document for the engine: architecture, live roadmap, release
> strategy, determinism/migration contracts, closed research and the working method.
> The user-facing introduction is in `README.md`, the version history in `CHANGELOG.md`.
>
> In the 2026-07 consolidation this document merged 12 separate plan/FIXPLAN/reference files; the
> detailed narrative of finished work was pruned, the durable decisions/lessons were kept.

---

## 1. Overview

Gizmo — a lightweight, **pure-Rust**, ECS-based 3D engine + a physics simulator written from
scratch (no external physics dependency). Published on crates.io (**0.9.0**, 19 crates).

- **ECS:** Entity = id, Component = data, Systems query Archetypes. `World` is the central
  state; `Query`/`Mut`/`With`/`Without`/`Changed`/`Added` filters; `Commands` for deferred
  structural changes; Table + SparseSet storage.
- **Physics:** rigid (TGS-Soft solver), soft-body (FEM/cloth/rope), fracture, joints,
  vehicle/character dynamics, CCD, GJK/EPA narrowphase, BVH broadphase.
- **Renderer:** WGPU deferred PBR + shadows/SSAO/SSGI/volumetric/TAA; egui HUD/editor.
- **Platform:** native + WASM (the sim core + renderer + window run in the browser).

---

## 2. Architecture (20 crates — stable)

Clean bottom-up layering, NO circular dependencies:

```
gizmo-math ─┬─ gizmo-core ─┬─ gizmo-physics-{core,rigid,dynamics,soft}
            │              ├─ gizmo-renderer ─ gizmo-{window,ui,editor}
            │              ├─ gizmo-{scene,net,ai,animation,audio,scripting}
            └──────────────┴─ gizmo-app ─ gizmo (facade) ─ demo/
```

**Refactor contract (from the god-file splitting rounds, completed):** pure/verbatim moves only
(no logic edits in the same step), `pub use` re-export from the original path (call sites do not
change), every step verified with build+test+clippy. 10 mega-files were split; the determinism
hash did not change. OPEN (optional, behavior-adjacent, not a pure move): splitting the
still-large functions such as `update_vehicle` / `execute_render_pipeline` — as needed.
<!-- TRANSLATOR NOTE: "davranış-bitişik" is a coinage; read as "adjacent to behavior", i.e. such a split can alter behavior and therefore falls outside the pure-move contract above. -->

---

## 3. Roadmap (LIVE — remaining work only)

> **2026-08-04:** An independent whole-engine audit was carried out → `docs/AUDIT-2026-08.md`
> (every finding backed by `file:line` evidence; 9 claims that could not survive adversarial
> verification were removed). The resulting work is being executed in `docs/FIXPLAN.md` — once
> that campaign ends, the durable decisions move here and FIXPLAN is deleted. The Phase 6/7 items
> below remain valid; the audit added a Phase A (honesty + security) that comes **before** them.

Phases 0–5 (stabilization, tests+CI, determinism, P2P rollback netcode, physics depth,
renderer/WASM/editor) are **DONE**. Remaining:

**Phase 6 — API stability & 1.0 mechanics**
- Freeze the public API + document the `unsafe` contracts.
- Staged 1.0 (see §4): Stage A core at 1.x, Stage B graphics layer at 0.y.
- The `RigidBody::friction`/`restitution` fields are **IGNORED** by the contact solver
  (the source is the collider material) → bridge them or remove them? (An API decision;
  bridging shifts the defaults.)

**Phase 7 — Product layer (a shippable game)**
- M7.4 authoritative client-server netcode; M7.5 audio mixer/bus/DSP; M7.6 UI font/text/
  widget/z-index; M7.7 WASM feature parities, editor panels + AssetServer hot-reload.
- 1.0 CI gates: rustfmt / `missing_docs` / coverage / cargo-deny / benchmark regression.
- Optional: cross-platform determinism (as a feature — see §5), gizmo-net on WASM.
- Gated on human-eye A/B: the textured-glTF `material_demo` asset, `car_demo` driving/geometry.
<!-- TRANSLATOR NOTE: "İnsan-gözü A/B gated" is a terse fragment; read as "these items can only be signed off by a human comparing before/after side by side". -->

---

## 4. Release Strategy — Staged 1.0

1.0 = the hard promise of "no breaking change without a 2.0". A crate that re-exports a 0.x
dependency (wgpu/winit/egui, bevy_reflect) in its public API cannot make that promise → a
lock-step 1.0 either freezes the engine on old deps or burns the 1.0 at the first dep bump.
The solution is **staged**:

- **Stage A (may go 1.x):** dependency-light crates whose surface we own —
  gizmo-math, -core, -physics-{core,rigid,dynamics,soft}, -scene, -net, -audio, -ai,
  -animation.
- **Stage B (stays 0.y):** graphics/integration — gizmo-renderer, -window, -editor, -ui,
  -app, -scripting + the `gizmo` facade (until wgpu/winit/egui are pinned to 1.0).
- **Consequence:** once staging begins the crates no longer share a SINGLE workspace version
  (`publish_all.sh` + the version-inheritance assumption must be updated).

**External-type contract (permanent):** `glam` = a permanent, DELIBERATE public dep (gizmo-math
re-exports it). `bevy_reflect` = sealed behind the default-OFF `reflect` feature (with a serde
fallback). `wgpu`/`winit`/`egui` = a deliberate leak that carries no semver cost during 0.x.
`ron` = a public dep of gizmo-scene (the RON file format + the SceneError API).
96 public types are `#[non_exhaustive]`; 13 Error enums + fn→Result conversions; `arrayvec` was
removed from the public API (opaque `ContactPoints`).

---

## 5. Determinism (reference)

- The simulation state (Transform/Velocity/solver) runs entirely on **glam/f32**.
- **Target:** same-platform replay + rollback bit-equality. Verified with `state_hash` +
  a cross-process test.
- **OUT of scope:** cross-platform bit-equal determinism — it requires an Fp32/softfloat
  migration (the Q16.16 `Fp32` type EXISTS in gizmo-math but the sim does not use it,
  experimental). It may become an optional feature after 1.0.
- The historical hashes appearing in this document (AAC365945335779E etc.) are point-in-time;
  they were superseded by later fixes — historical.

---

## 6. Migration & Graphics Upgrade (0.1 → 0.2, completed)

The "1.0-readiness hardening + graphics upgrade" breaking release (2026-06-25):

- **MSRV → Rust 1.92** (egui 0.34's floor; previously 1.89).
- **Graphics stack:** wgpu 0.20→**29.0.3**, winit 0.29→**0.30.13**, egui→**0.34.3**
  (+ egui-wgpu/winit 0.34.3, egui_dock 0.19.1, transform-gizmo-egui 0.9.0), naga 29.
  The determinism hash (598E315D0E7499FF) did not change across the whole upgrade.
- **API breakers:** the `glam` re-export was made official; `bevy_reflect` was moved behind the
  `reflect` feature; `arrayvec` left the public surface; 96 types became `#[non_exhaustive]`;
  Error enums + Result returns. For the detailed 11-item migration steps, see the git history
  (the 0.2.0 commits).
- **Code decision (explains the current code):** winit 0.30 still offers the deprecated
  `EventLoop::run(closure)` → gizmo-app's ~600-line closure event loop was moved to
  `ApplicationHandler` DELIBERATELY (see `crates/gizmo-app/src/windowed/`).

---

## 7. Closed Research & Non-Goals

**Solver stack instability — SOLVED.** A resting column of N≥5 boxes was linearly unstable
(lateral BUCKLING / inverted pendulum, not a vertical energy pump): the iterative contact
solver's effective lateral restoring stiffness was below the buckling-critical value.
- **Fix (2 layers):** (1) a manifold **BLOCK solver** (`solver/block.rs` +
  `tgs.rs::tgs_sweep_block`) — it solves a manifold's ≤4 COPLANAR normal impulses TOGETHER
  (a regularized active-set LCP). Two critical details: the 4-coplanar block is RANK-DEFICIENT
  (4 contacts, 3 DOF) → **Tikhonov reg** (`block_regularization`, 0.05 today — the narrowphase
  fix below turned 0.1 into over-softening) is mandatory; and the block must stay **RIGID**
  (soft scaling weakens it). (2) **Full warm-start** (`warm_start_factor` 0.85→1.0) — a partial
  warm-start threw away 15% of the impulse every substep and injected marginal energy on
  re-convergence; a full warm-start shuts that off. **Result: a 1-wide N≤32 tower is stable**
  (3000 frames, a single ground size) — see the narrowing below; this sentence used to be written
  unconditionally. NO determinism re-bless. Regression:
  `soak_resting_stacks_stay_bounded` (N∈{2,5,16,24,32}).
- **OPEN:** the extreme N≥48 tower still buckles — it needs a friction-aware whole-chain
  direct/global solver (`direct_chain_solve` opt-in flag + `solve_island_normals` solves normals
  only, O(n³)). `soak_extreme_tower_n48` is #[ignore].

**At exact contact the manifold collapsed to a SINGLE POINT — SOLVED** *(2026-08-06)*.
The depth test in `narrowphase/contacts.rs::clip_box_box` had no tolerance (`signed_depth <= 0.0`).
At exact contact the depth of all four corners is exactly zero → all of them are culled and the
clip returns empty → the pair fell back to GJK's **single-point** fallback. A single-point manifold
carries zero tilt-restoring torque (that is precisely why the block solver exists), and the point
GJK returns is not at the center: its offset grows with the size of the opposing collider and
reaches the edge of the resting box, applying an unearned torque impulse. On supports with a
half-extent of ≲1.5 the interface never recovered at all (the centered point holds the box with no
torque → it never sinks in → the clipping path is never entered again).
- **Fix:** a tolerance of the kind the slab test already carries (`DEPTH_TOLERANCE = 1e-4`).
- **Result:** 4 corner points at every support size, both at spawn and at rest; the spawn kick
  0.03 rad/s → **0**; the lean of the 12-high tower on the small platform 0.024 → **0.0000**.
- **The convergence cost it exposed:** `block_regularization` 0.1 → **0.05**. The cost is in the
  4-point interface itself and always was (a chain overlapping by 1 mm is exactly as slow);
  the tolerance removed the degenerate point that had been hiding it. A Tikhonov term is also a
  softening, and 0.1 had been chosen in a period when the term was never actually applied. At 0.05
  the compressed chain comes to rest at frame 0 instead of frame 379, and momentum leakage
  5.4e-4 → 4e-6.
- **Determinism re-bless:** `46EB56180318E43C` → `15D4FD6845119D8B` (3/3).

**Partial sleep was corrupting stacks — SOLVED** *(2026-08-06)*. Once a body falls asleep in the
middle of a contact island it is no longer INTEGRATED but is still SOLVED: `solver/tgs.rs` reads
its mass without looking at the sleep state, the only gate being `is_dynamic()`. The awake
neighbor takes its share of the reaction, the sleeping one does not → momentum is not conserved at
that interface. That is why 12-high stacks did not reliably stand for 3000 frames, and why the
static ground's half-extent (20 vs 200) flipped the outcome.
- **Fix:** the sleep decision is per contact ISLAND rather than per body, and happens AFTER the
  solve (`RigidBody::advance_sleep_counter` + the island pass in `pipeline.rs`). A body with no
  contacts still sleeps on its own counter as before (that transition happens after the joint
  pass).
- **Result:** `wide_block_collapse_per_ground` from 10/20 collapses to **0/20**;
  `height_12_stacks_stay_standing` (6 cells, 3000 frames) **passes**; the natural-sleep lean of a
  1×12×1 column from 0.0104–10.17 to **0.000106**, i.e. identical to the force-awake value.
- **Decisive evidence:** without the fix, at 1 sweep the stack blows up at frame 193 in the
  natural run but never blows up in the force-awake arm → the blow-up comes from partial sleep,
  not from under-solving.
- **Side benefit:** because settled stacks can now sleep collectively, `headless_stress_test`
  went 1.62 s → 0.51 s.
- **Determinism re-bless:** `EF6E4AC3644BF3BA` → `46EB56180318E43C` (3/3).
  `golden_state` `settle vy` `-0.0408733` → `0.0`; that number was one substep's worth of gravity
  and turned out to be the defect's fingerprint. `settle y` did not change.
- **The cost:** if a single member of the island is moving, the island does not sleep; one
  jittering box can keep a whole stack awake. It did not happen in the scenes measured.
- ⚠️ **The statements "robustly stable at N≤32" and "game structures are ≤~12 → not needed" were
  a narrow sample even so** *(narrowed on 2026-08-05)*: a 1-WIDE tower, a SINGLE ground size,
  1500 frames. The fix above rescued 12-high stacks, but the lesson itself stands — the SAMPLE
  must be widened as much as the horizon.
- **LESSON:** choose the soak-test horizon BEYOND the onset of instability (the old `n16` test was
  600 frames, the blow-up at ~853 → it shipped green and hid the bug). **And widen the SAMPLE as
  much as the horizon:** the narrowing above was a defect missed because a test whose horizon was
  sufficient tried a single shape and a single ground size.

**Physics perf, second round — SOLVED** *(2026-08-06)*. Three items, all measured:
- **Incremental broadphase** (C2): the tree is preserved across substeps. The fat-margin AABB
  tree's `insert` already early-outs for a body that has not left its box; `clear()` was throwing
  that gain in the bin. Removals are reconciled with `DynamicAabbTree::retain`. **The determinism
  hash did not budge** — the incremental tree emits pairs in a different order, and the fact that
  this does not change the simulation is empirical evidence of the "pair-emission invariance"
  property.
- **The O(N²) writeback in the ECS bridge** (C3) → a handle→index map. (The bridge has no
  benchmark; a pure complexity fix.)
- **Rewind history opt-in** (C4a): `max_history_frames` 600 → 0. 160 B per body per frame; the old
  default held 192 MB resident in the 2000-box stress scene.

| scenario | 2026-08-05 | 2026-08-06 |
|---|---|---|
| `broadphase/1024` | 1.73 ms | **564 µs** |
| `solver_settled_stack/48` | 4.05 ms | **115 µs** |
| `full_step_mixed/512` | 2.43 ms | **1.64 ms** |
| `headless_stress_test` | 1.62 s | **392 ms** |

The large drop in the settled stack comes from island-collective sleep, the ones in broadphase and
full_step from the incremental tree.

**Physics perf (N² bottlenecks) — SOLVED.** broadphase `query_pairs` pair generation
(O(P²)→O(P)), the TGS per-island scratch sized to the island instead of the whole world, and
HOISTing the per-contact TGS constants out of the 24-sweep loop → worst frame 262→46ms (~5.7×),
bit-equal determinism.

**6 latent bugs (the 2026-07-13 hunt) — ALL FIXED.** tangent (model_mat3, not inverse-transpose),
PBR-pack overflow (`.min(999.0)`), the query get/contains table-storage With/Without gate,
batch-shadow instance-region separation, glTF `AlphaMode::Mask` cutout. **Eliminated as false
positives (do not re-chase these):** deferred_lighting f16 aniso, gbuffer bitangent-collapse,
vehicle point-velocity COM, narrowphase incident-corner. *Remaining minor:* PBR params are still
decimal-packed into a single f32 (precision drops above 2²⁴) — long term, a separate slot.
<!-- TRANSLATOR NOTE: the heading says 6 bugs but only 5 are listed; the sixth may be the "remaining minor" PBR-params item, or an entry lost in an earlier edit — unverified, please check. -->

**NON-GOAL: narrowphase batch-SIMD.** Investigated, REJECTED (2026-07-14). Measurement
(wide_scene, 2000 boxes, ~30ms frame): box-box SAT compute is only **~3.3%**; narrowphase
post-processing cannot be batched; both per-pair SIMD attempts regressed (the scalar code is
already auto-vectorized). DO NOT RETRY without passing the step-0 gate again. (The
"~82% narrowphase" figure is OBSOLETE.)

---

## 8. Working Method

- Every item: **fix → write a regression test → build/test/clippy → tick it off.**
- Verify behavior-changing physics fixes with `headless_stress_test` + focused scenarios;
  choose the soak horizon beyond the onset of instability.
- On bug-hunt rounds use subagent fan-out, then verify every finding BY HAND
  (sieve out the false positives).
- CI: `cargo clippy --all-features --all-targets -- -D warnings -A too_many_arguments
  -A type_complexity` (the two grandfathered architectural lints). The entry crate is
  `gizmo-engine` (NOT `-p gizmo`); `| tail` masks cargo's exit code — check the exit status
  separately.
