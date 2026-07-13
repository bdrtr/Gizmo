# Gizmo Engine — Architecture Refactor Roadmap

> Goal: reduce the "god file / god function" maintenance pain the team feels, **without**
> a risky rewrite. The crate architecture is already sound — this roadmap is about
> decomposing a handful of oversized files, incrementally and safely.
>
> Generated from a codebase-memory-mcp index (9,767 nodes / 49,497 edges) + file metrics.
> Update the checkboxes as phases land.

## 1. Assessment (data-driven, not vibes)

**The crate layout is GOOD.** 20 crates, clean layering: `gizmo-math` / `gizmo-core` at the
bottom (high fan-in, zero fan-out — correct), `physics-*` and `gizmo-renderer` above,
`src`/`demo` as entry points. No circular dependencies or layer violations were reported.
This is **not** a spaghetti monolith.

**The pain is ~10 oversized files inside otherwise-fine crates.** They split into two kinds:

| Kind | Meaning | Refactor character |
|---|---|---|
| **A — Big but simple** | many lines, low cognitive complexity (straight-line) | easy, low-risk moves; big readability win |
| **B — Big AND complex** | many lines + high cognitive load | higher value, higher risk; extract subsystems |

### Top offenders

| File | Lines | Fns | Cognitive | Kind | Notes |
|---|---:|---:|---:|:--:|---|
| `renderer/asset/loaders.rs` | 1347 | 33 | **192** | B | glTF + image + mesh + material + tangent-gen in one file. Today's tangent bug lived here. |
| `physics-dynamics/vehicle.rs` | 1562 | 40 | **154** | B | wheel/suspension/drivetrain/steering all together |
| `core/input.rs` | 859 | 52 | 98 | B | keyboard/mouse/gamepad/mapping |
| `gizmo/spawner.rs` | 957 | 39 | 95 | B | per-primitive spawn helpers |
| `physics-core/narrowphase.rs` | 996 | 33 | 79 | B | per-collider-pair algorithms |
| `physics-rigid/joints/…/joint_types.rs` | 1236 | — | — | B | one file per joint kind is natural |
| `gizmo/systems/render/mod.rs` | 1120 | — | low | A | `default_render_pass` orchestration + uniform build |
| `gizmo/systems/render/passes.rs` | 931 | — | low | A | already has `record_*` fns — just needs splitting into files |
| `shaders/deferred_lighting.wgsl` | 664 | — | — | A | one shader doing env/IBL/shadow/lighting/debug |
| `gizmo-studio/render_pipeline.rs` | 1002 | — | — | A | tooling, not shipped runtime → low priority |

The MCP's Leiden cluster analysis flags the **render path (cluster #3)** as the lowest-cohesion
de-facto module (0.50) — the most tangled seam, and exactly where we burned hours today.

## 2. Principles (non-negotiable)

- **One file per step / commit.** Never batch multiple god-file splits.
- **Pure moves, no behavior change.** Extraction only; keep public APIs stable via re-exports.
- **Verify every step:** `cargo build` + `cargo test` (where tests exist) + **run the affected
  demo** and look at it (rendering has no golden-image net yet — human eyes are the test).
- **Physics is test-backed** (1,114 TESTS edges) → refactor there with confidence.
- **Renderer is weakly tested** → lean on demo runs; consider Phase 0 first.
- **Do not touch the paused 0.8.0 release surface** or push without explicit approval.
- **Re-index after each step** (`detect_changes`) so the graph stays truthful.

## 3. Phased plan

### Phase 0 — Safety net (recommended prereq for renderer work)
- [x] Headless render smoke test — **already present** as `golden_render_tests` in
      `systems/render/mod.rs`: `default_render_pass_draws_a_cube_distinct_from_background`
      renders a lit cube through the real `default_render_pass` via `Renderer::new_headless`,
      reads the buffer back and asserts centre≠corner + >5% coverage; `camera_exposure_
      brightens_the_frame` guards the exposure knob. GPU-guarded (`headless_adapter_available`).
      Confirmed passing on a real adapter this session. This is the net Phases 1–2 lean on.

### Phase 1 — Render path (Kind A, low-risk, highest relief)
- [x] `systems/render/passes.rs` (931) → `systems/render/passes/{shadow,geometry,ssao,forward,screen_space,taa}.rs`
      + `mod.rs` re-exporting the recorders (call sites unchanged). Pure move; build clean +
      golden render tests pass. Commit `d5fd52f`. (The sixth recorder is `forward.rs`, not the
      guessed `post.rs`.)
- [~] `systems/render/mod.rs` (1120 → 782) → **largest chunk extracted.** The per-frame render
      cache + draw-item collection/culling/instancing (the biggest self-contained block of the
      ~620-line `default_render_pass`) moved verbatim into `batching.rs` — `RenderCache`,
      `DrawItem`/`BatchKey`/`BatchData`, `collect_draw_items` (+ the `process_mesh!` macro) and
      the routing-flags test. Commit `d518c48`; build clean + golden + batch tests pass.
      The roadmap also named `lights.rs`/`shadows.rs` — those were **already** single-sourced in
      `shared.rs` (`collect_scene_lights`) + the renderer crate (`compute_directional_cascades`),
      so they didn't need extracting. Still inline (small now, optional follow-ups): the camera
      resolve, the `SceneUniforms` assembly (`scene_uniforms.rs`), and the G-buffer/SSAO/…/TAA
      `resize` block (`resize.rs`).
- [~] `shaders/deferred_lighting.wgsl` (664 → 572) → **partly split.** The pure PBR lobes
      (anisotropic GGX, clear-coat, Lazarov env-BRDF LUT) moved into a new composable module
      `gizmo::pbr_ext` (`shaders/pbr_ext.wgsl`), `#import`ed by deferred_lighting. Commit
      `d749ee4`; `core_shaders_compile` + compose tests + golden render tests pass.
      **Deliberately NOT extracted:** the procedural `environment`/IBL presets and the PCSS
      `shadows` filter — they read the `scene` uniform and shadow textures, and `common.wgsl`'s
      convention keeps composable modules PURE (no binding refs). Moving them needs `scene`
      threaded through signatures (behaviour-adjacent) → its own verified step, not a pure move.

### Phase 2 — asset/loaders.rs (Kind B, highest cognitive load: 192) ✅ `4d76c9e`
- [x] Split the 1347-line `loaders.rs` into a `loaders/` module, one concern per file
      (verbatim moves): `mod.rs` (public scene types + the load_gltf_scene/load_gltf_from_import
      orchestration), `obj.rs`, `images.rs` (RGBA8/sRGB/upload + GpuImage), `material.rs`
      (samplers + build_gltf_materials + tests), `mesh.rs` (parse_gltf_node + tangent fallback +
      normal/skin helpers + tests), `animation.rs`, `skeleton.rs`. Cross-module helpers gained
      `pub(super)`; public paths preserved (`asset::loaders::GltfSceneAsset`, `asset::GltfNodeData`).
      Named `loaders/` not `asset/gltf/` because it also owns OBJ; `texture.rs`/`procedural.rs`
      were already separate modules.
- Verify: ✅ gizmo-renderer builds clean; 9 loaders CPU tests pass; full glTF demo chain compiles;
      **headless load of a real `.glb`** (airbus_a310_mrtt) through the moved path → `roots=1,
      prims=10` (mesh + material + image path exercised end-to-end).
- Follow-up (optional, behaviour-adjacent → separate step): extract the inline per-vertex tangent
      fallback in `parse_gltf_node` into a named, unit-tested `generate_tangent()` — the roadmap's
      "TBN gen as its own testable unit" goal. Left out of this pure move on purpose.

### Phase 3 — Physics god files (Kind B, but TEST-BACKED → safer)
- [ ] `vehicle.rs` (1562) → `vehicle/{wheel,suspension,drivetrain,steering,dynamics}.rs`
- [ ] `joints/…/joint_types.rs` (1236) → one file per joint type
- [ ] `narrowphase.rs` (996) → per-collider-pair modules
- Verify: `cargo test -p gizmo-physics-*` after each.

### Phase 4 — Core & gameplay glue
- [ ] `input.rs` (859) → `input/{keyboard,mouse,gamepad,mapping}.rs`
- [ ] `query/mod.rs` (975) → builder / iteration / filters
- [ ] `spawner.rs` (957) → per-primitive spawn helpers

### Phase 5 — Tooling (lowest priority)
- [ ] `gizmo-studio/render_pipeline.rs` (1002) — not shipped runtime.

## 4. Per-file execution protocol
1. Read the file; map functions into cohesive groups. Use `search_graph` / `trace_path` to
   confirm no hidden cross-coupling before moving anything.
2. Create submodule files; move code **verbatim** (no logic edits in the same step).
3. Re-export from the original path so callers don't change (`pub use`).
4. `cargo build` + `cargo test` + run the affected demo and look at it.
5. Commit as one focused `refactor: split <file>` commit.
6. `detect_changes` to refresh the graph.

## 5. Explicitly NOT doing
- No rewrite of the crate structure (it's healthy).
- No algorithm/behavior changes bundled into a move.
- No merges to the 0.8.0 release surface without approval.

---
*Suggested start: **Phase 1 → `passes.rs`** (mechanical, low-risk, immediate readability win),
after optionally landing the Phase 0 headless smoke test.*
