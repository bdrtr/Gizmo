# Bevy parity — what a port of Bevy's examples actually hits

**Status:** measured, not estimated. **Last updated:** 2026-08-23.

## What this is

Bevy 0.19.1 ships 412 examples. Between 2026-08-22 and 2026-08-23, 46 of them were ported to
`demo/src/bin/bevy_*.rs`, one at a time, under one rule: **do not hide where the engine differs —
write the difference into the demo's header, with a measurement.** This file is the aggregate of
those headers. Every claim below names the demo that measured it, so it can be re-run rather than
believed.

The porting sweep also found **six real engine defects**; they are listed at the bottom, all fixed
or reported.

Read this next to `ENGINE.md` §3 (roadmap). It is not a wishlist: it is the list of things a Bevy
user would reach for and not find, ranked by how much of the example corpus each one blocks.

### How the corpus splits

A nine-agent inventory over all 412 examples (2026-08-22) classified them as
**73 easy · 112 medium · 132 hard · 94 impossible** (20 already covered). "Impossible" means the
engine has no path at all today, and those 94 are dominated by the four capabilities in section A.

---

## A. Whole capabilities that do not exist

These are not missing knobs. Each one closes off a whole category of the example corpus.

### A1. No 2D / sprite pipeline

There is no sprite, no 2D camera, no 2D transform stack. Bevy's entire `2d/` directory — and every
UI-adjacent example — has no path. Ports that ran into this: `bevy_texture`, `bevy_parallel_query`
(Bevy's version moves 128 sprites; the port had to become 3D cubes), `bevy_removal_detection`.

Blocks the largest single share of the "impossible" bucket.

### A2. No text / font rendering

Nothing draws a glyph in world space or screen space except `egui`, which is a debug/editor UI
layer, not a game text system: no font asset, no text mesh, no layout, no rich text, no
world-space labels. Every demo in the sweep that wanted a label used an `egui` panel, which is why
they all look like debug overlays.

**This is the gap the project owner has named as the first to close.** Scope notes are in
section E.

### A3. No user-authored shaders or materials

`MaterialType` is a fixed enum (`Pbr`, `Water`, `Unlit`, `BakedLit`, `Skybox`, `Backdrop`,
`BackdropPlaced`, `Grid`). A game cannot add a variant, cannot supply its own WGSL, and cannot
attach its own uniform block. Bevy's `Material`/`AsBindGroup` trait has no counterpart.

Consequence measured in `bevy_deferred_rendering`: choosing the forward path means *changing the
material type*, which also changes the shading — the two axes Bevy keeps separate are one axis
here. **A PBR material cannot be drawn forward at all.**

### A4. No extrusion / 2D-shape-to-mesh machinery

Measured in `bevy_3d_shapes`: of the 26 meshes that example builds, **6** have a ready-made
constructor in `AssetManager`. **15 of the remaining 20 come from this single gap** (8 extrusions +
7 ring extrusions). The other five are individually missing: `Tetrahedron`, `ConicalFrustum`,
icosphere, `Segment3d`, `Polyline3d`.

`Mesh::from_vertices` makes hand-rolling possible — the demo hand-builds three of them — but that
is a workaround, not a feature.

### A5. No environment cubemap / reflection probe

Measured in `bevy_pbr`: a metallic surface has no environment to reflect. SSR covers what is on
screen (`bevy_ssr` measures it working well) and nothing else.

### A6. No MSAA

Measured in `bevy_anti_aliasing`: every render target in the tree is `sample_count: 1`, and the
deferred G-buffer path is not MSAA-shaped anyway. FXAA and TAA both exist and both measurably work;
SMAA and CAS sharpening do not exist.

---

## B. Effects that exist but cannot be shaped

The effect is there and — where it was measured — it is correct. What is missing is the ability to
author it. Counts are Bevy's knobs vs the engine's.

| Effect | Bevy | Gizmo | Measured in |
|---|---|---|---|
| Screen-space reflections | 6 knobs | **0** (on/off only, and no `enabled` flag — turning it off *destroys* the state) | `bevy_ssr` |
| Tone mapping | 7 curves + dither | **1** curve (ACES, hard-coded) + exposure | `bevy_tonemapping` |
| Bloom | 7 knobs | **2** (`intensity`, `threshold`) | `bevy_bloom_3d` |
| Depth of field | 6 optical knobs | **3** numbers + enable | `bevy_depth_of_field` |
| SSAO | 5 quality levels + temporal denoise | **1** (`strength`) | `bevy_ssao` |
| Anti-aliasing | 5 techniques | **2** (FXAA, TAA) | `bevy_anti_aliasing` |
| Shadow bias | 2 knobs | **0** — derived from cascade texel size | `bevy_shadow_biases` |
| Distance fog | component with colour/falloff/mode | **4 baked presets** + a blend factor; the numbers live in `deferred_lighting.wgsl` | `bevy_fog` |
| Volumetric | — | **0** knobs | — |
| Cascaded shadows | configurable | **all constants** in `csm.rs` (4 cascades, 3072², 100 m, λ 0.75) | `bevy_shadow_biases` |
| Clearcoat | strength + layer roughness + 3 textures | **1** (`Material::clear_coat`), and measurably near-binary: the peak jumps 161 → 205 from 0 to 0.2 and then flattens | `bevy_clearcoat` |

Two measured caveats worth keeping:

- **Bloom's radius is not adjustable and is tight.** `bevy_bloom_3d` measured the halo contributing
  `+18.9` in the 60–80 px band and **exactly 0.00 beyond 80 px** at every intensity. Bevy's `scale`
  / `max_mip_dimension` / `low_frequency_boost` are precisely the knobs that would widen it.
- **The fog knob is not a fog knob.** `environment_preset` also selects the sky and the overall
  lighting mood, so fog cannot be changed independently of the scene's look.

---

## C. ECS and API gaps

Small individually; together they are what makes a Bevy snippet fail to compile.

| Missing | Effect | Measured in |
|---|---|---|
| `Local<T>` system param | Per-system state must become a world resource, so it is visible to every other system and to the scheduler's conflict analysis | `bevy_ecs_guide` |
| `On<Remove, T>` / `On<Replace, T>` dispatch | The markers exist; `observer.rs` says there is "no dispatch path yet". Removal is observable only through a raw `RemoveHook` | `bevy_removal_detection`, `bevy_observers` |
| Global (non-entity) observers | `add_observer` is `On<Insert, T>` only; `observe` needs a target entity. An untargeted event has no counterpart | `bevy_observers` |
| Run conditions on observers | Bevy can `.run_if` an observer | `bevy_observers` |
| A world handle inside observers/hooks | Callbacks get no world, so a chain reaction must go through captured state and advances **one ring per frame** (measured: 8 rings, 8 frames) | `bevy_observers` |
| `Visibility` component | Hiding means `ShadowCasting::Only`, which still casts a shadow — not the same thing | `bevy_infinite_grid` |
| ~~`Transform::forward/right/up/looking_at`~~ | **CLOSED 2026-08-23** — added to `Transform` with two tests (axes orthonormal and right-handed across a full turn of yaw; `looking_at` aims and survives degenerate input). `looking_at` returns the rotation rather than applying it, so it can be blended | `bevy_transform` |
| `.chain()` | Ordering is per-edge `label()` + `after()`, and an unmatched constraint **warns and is dropped** rather than failing | `bevy_transform`, `bevy_animated_transform` |
| `or_else` for run conditions | Repeated `run_if` ANDs; OR must be written inside one closure, which also makes the system exclusive | `bevy_run_conditions` |
| `OnEnter` / `OnExit` state schedules | `State<S>` and `in_state` exist but nothing drives transitions | `bevy_states` |
| `EventMutator` | `EventReader`/`EventWriter` only | `bevy_events` |
| Entity generations in `Children` | `Children` is `Vec<u32>`; a recycled id aliases. And a plain `despawn` on a child leaves a **dangling id** in the parent (measured), which `remove_child` cannot clean up afterwards because it takes an `Entity` | `bevy_hierarchy` |
| A clear/sky colour setting | Fog only tints geometry; the empty background stays black | `bevy_fog` |
| `AlphaToCoverage` | No MSAA to spread alpha across | `bevy_transparency_3d` |

---

## D. Game-path / editor-path asymmetries

`render_parity` exists precisely to catch these, and it is currently red on one. Recording the
whole set, because they are one family:

1. **`alpha_cutoff` is known to the game path only** — the editor draw path never learnt it.
   Pre-existing since `550a7df`; `render_parity` reports it.
2. **`RenderStats` (draw calls / triangles / instances) is filled by `gizmo-studio` only.**
   `default_render_pass` publishes nothing, so a *game* cannot read its own draw-call count.
   Measured in `bevy_many_cubes`.
3. **The grid pipeline is editor-only.** `MaterialType::Grid`, `grid.wgsl` and a compiled
   `grid_pipeline` all exist; only studio ever binds it. In the game path a grid material draws as
   a plain opaque PBR plane — measured in `bevy_infinite_grid`.
4. **`Material::emissive` is dead in the deferred path.** The G-buffer reads emissive from the
   *material uniform* (which only glTF loading fills), while game code writes the per-instance
   field. Only `baked_lit.wgsl` reads the instance field. Measured in `bevy_bloom_3d`.

5. ~~**`vignette` is reachable only through the `PostProcess` component.**~~ **CLOSED 2026-08-23.**
   The `Renderer` fallback struct had no such field, so a camera without the component was stuck
   with `PostProcessUniforms::default()`'s **0.25** — a vignette it never asked for and could not
   switch off. `Renderer::vignette_intensity` now exists and the fallback branch reads it; the
   default is still `0.25`, so **no pixel changed** — the value merely became reachable.

   Verified in `bevy_color_grading`, one capture batch, size-checked: component `vignette = 0`
   gives corner/centre **1.095** and fallback `renderer = 0.0` gives **1.095**; component `0.9`
   gives **0.608** and fallback `0.9` gives **0.608**. The two paths now produce the same frame for
   the same setting.

A full audit of this family was run (2026-08-23): of `Material`'s shading fields, the deferred
G-buffer honours 6 (albedo, roughness, metallic, anisotropy, clear-coat, subsurface) and ignores
three — `ambient` (**documented** as PBR-path-excluded, intentional), `alpha_cutoff` (known, item 1)
and `emissive` (item 4, the only undocumented one). No further silent fields.

---

## E. Suggested order of work

1. **Text / font rendering (A2).** Named first by the project owner. It is already tracked as
   **M7.6** in `ENGINE.md` §3 (Phase 7), and `gizmo-ui`'s own crate docs name the same gap and the
   same landing site: *"expect the component set to change when rendering lands (a `Text` component
   and a draw-list output are the obvious additions)"*.

   The ground is better prepared than the gap suggests. `gizmo-ui` already resolves boxes through
   `taffy` and publishes them as `Node` rects; it just emits no vertices. So the work splits
   cleanly:

   - **Rasteriser + atlas** (in `gizmo-renderer`): load a `ttf`, rasterise on demand into a GPU
     atlas, evict nothing at first. The dependency is the one decision that needs `ENGINE.md` §4
     review — `ab_glyph` and `fontdue` are both rasteriser-only and can be sealed behind an opaque
     atlas handle so no foreign type reaches a `pub` signature.
   - **`Text` component** (in `gizmo-ui::components`): string, font handle, size, colour, and an
     anchor. Single-line and single-font is enough to be useful; layout richness is a later axis.
   - **Two draw modes**: screen-space (a UI quad batch, consuming `Node`) and world-space (a
     camera-facing quad, so 3D labels work). Both batch by atlas texture, which fits the existing
     `BatchKey` (the atlas is just a texture bind group).
   - **Acceptance**: a golden render test, because that is the only thing that can prove a glyph
     landed where it should. The repo already has that machinery
     (`crates/gizmo/src/systems/render/mod.rs`, `golden_render_tests`).

   Two constraints the sweep already established: the wasm target is a **separate CI gate**, so the
   rasteriser must build there; and `BackgroundColor` is currently written and never read, so the
   same draw path should consume it and close that dangling component at the same time.
2. **Post-process knobs (B).** Cheapest ratio of work to unblocked examples: these are uniform
   fields and shader constants that already exist, just not exposed. SSR and tone mapping are the
   two with the widest gap.
3. **`Visibility`, `Local<T>`, `or_else` (C).** Small, self-contained, and each removes a recurring
   papercut. (`Transform`'s direction helpers were the first of this group and are done.)
4. **Extrusion machinery (A4).** One feature, 15 examples.
5. **User materials (A3).** The largest and most architectural; it interacts with `routing.rs`, the
   G-buffer budget and `render_parity`.
6. **2D pipeline (A1).** Largest of all; arguably a separate project.

---

## F. Defects found and fixed during the sweep

Six, all with regression coverage or an explicit report:

| Defect | Fix |
|---|---|
| Orthographic cameras jittered TAA into the wrong matrix column — a static scene had 2.1 % of pixels moving | Fixed + unit tests + a golden render test |
| A cut-out material fell into the transparent bucket as its alpha dropped, contradicting `Material::alpha_cutoff`'s own contract | Fixed in `batching.rs` + tests |
| The forward shader ignored `alpha_cutoff` entirely | Fixed in `shader.wgsl` |
| Spatial audio's ears were **mirrored**: `listener.right` was the negation of `Camera::get_right()`, so a sound on the camera's right reached the left ear in every simple-scene game | Fixed + a regression test that sweeps a full turn of yaw |
| `Renderer::exposure` is read by nothing; the game path reads `Camera::exposure`. A demo's exposure slider was wired to the dead field | Slider rewired; dead field documented |
| A `#[cfg(test)]` module placed mid-file truncated what `render_parity` could see, mis-reporting 14 capabilities as editor-only | Module moved to the end of the file |

Reported, not fixed: deferred `alpha_cutoff` needs the z-prepass to gain a fragment stage; the
editor path never learnt `alpha_cutoff`; `Material::emissive` is dead in the deferred path (D4).
