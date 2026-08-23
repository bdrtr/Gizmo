# Capability gaps — what the engine cannot do yet, measured

**Status:** measured, not estimated. **Last updated:** 2026-08-23.

## What this is

Between 2026-08-22 and 2026-08-23 a demo suite was built in `demo/src/bin/`, one capability at a
time, under one rule: **do not hide where the engine falls short — write the shortfall into the
demo's header, with a measurement.** This file is the aggregate of those headers. Every claim
below names the demo that measured it, so it can be re-run rather than believed.

The sweep also found **eight real engine defects** and **two complete subsystems wired to nothing**;
they are listed at the bottom.

Read this next to `ENGINE.md` §3 (roadmap). It is not a wishlist: it is the list of things a game
built on this engine would reach for and not find, ranked by how much each one blocks.

### Scale of the gap

A nine-agent inventory (2026-08-22) over a 412-item reference corpus of 3D and ECS capabilities
classified them as **73 easy · 112 medium · 132 hard · 94 out of reach** (20 already covered).
"Out of reach" means the engine has no path at all today, and those 94 are dominated by the four
capabilities in section A.

---

## A. Whole capabilities that do not exist

These are not missing knobs. Each one closes off a whole category of work.

### A1. No 2D / sprite pipeline

There is no sprite, no 2D camera, no 2D transform stack. Every 2D and UI-adjacent capability has
no path. Demos that ran into this: `texture`, `parallel_query` (the reference version moves 128
sprites; the demo had to become 3D cubes), `removal_detection`.

Blocks the largest single share of the out-of-reach bucket.

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
attach its own uniform block. There is no material trait to implement.

Consequence measured in `deferred_rendering`: choosing the forward path means *changing the
material type*, which also changes the shading — two axes that ought to be separate are one axis
here. **A PBR material cannot be drawn forward at all.**

### A4. No extrusion / 2D-shape-to-mesh machinery

Measured in `3d_shapes`: of the 26 meshes that demo wants, **6** have a ready-made constructor in
`AssetManager`. **15 of the remaining 20 come from this single gap** (8 extrusions + 7 ring
extrusions). The other five are individually missing: tetrahedron, conical frustum, icosphere,
3D segment, polyline.

`Mesh::from_vertices` makes hand-rolling possible — the demo hand-builds three of them — but that
is a workaround, not a feature.

### A5. No environment cubemap / reflection probe

Measured in `pbr`: a metallic surface has no environment to reflect. SSR covers what is on
screen (`ssr` measures it working well) and nothing else.

### A5b. No split screen / multi-viewport — and the reason is three deep

`Camera` has no viewport rectangle, and the render path picks exactly one camera:
`cameras.iter().find(|(_, c)| c.primary).or_else(|| cameras.iter().next())` is the whole of
`active_camera`. Measured in `split_screen` and, more completely, in `render_to_texture`, which
found **three separate obstacles** between here and a second camera:

1. **`default_render_pass` takes no camera argument.** Selection is `active_camera`, so the
   `primary` flag has to be flipped between calls. This part works — measured `Some(5)` then
   `Some(4)`.
2. ~~**`with_simple_scene` stamps one pose onto every camera.**~~ **CLOSED 2026-08-23** (D7) —
   found by this demo and fixed in the engine. The second camera, spawned at `(0, 7.0, 0.2)` with
   pitch −1.15, used to read back as `yaw −1.57 pitch −0.21 T(0, 1.4, 6.5)` at draw time.
3. **There is one `global_uniform_buffer`, written with `queue.write_buffer`.** Those writes order
   against *submission*, not against encoder recording — so two passes recorded into one encoder
   both read the **last** camera written. This is the deepest of the three and was invisible until
   the first two were worked around.

Two remain, and `render_to_texture` shows how to get past them: flip `primary`, and record the
first pass into its own encoder submitted immediately. The proof is in the pixels — the off-screen
target's brightness bands go `54.07 / 113.55 / 83.28` to `87.95 / 95.83 / 60.11`. Neither is a
documented pattern.

**Render-to-texture itself works**: `default_render_pass` accepts any `TextureView`.

### A6. No MSAA

Measured in `anti_aliasing`: every render target in the tree is `sample_count: 1`, and the
deferred G-buffer path is not MSAA-shaped anyway. FXAA and TAA both exist and both measurably work;
SMAA and CAS sharpening do not exist.

### A7. No area lights, light textures, or contact/soft shadows

All three engine lights are punctual: `PointLight`, `DirectionalLight`, `SpotLight`. Measured in
`area_lights`:

- **No rectangular or spherical area light.** Nothing resembling an LTC path exists.
- **No light texture (cookie/gobo).**
- **No contact shadows.**
- **No percentage-closer soft shadows.** The PCF loop is a fixed 3×3 kernel stepping by a constant
  texel — `let texel_size = scene.cascade_params.y;` with `frame_uniforms.rs` asserting that equals
  `1 / SHADOW_MAP_RES`. The quantity PCSS varies, a blocker-distance search radius, is never
  computed.

**A trap worth naming:** `PointLight::radius` is **range**, not source size. Growing it widens and
dims the light rather than softening its shadow — measured scene brightness rising
`136.13 → 139.09 → 143.16` as radius goes `2 → 6 → 20`.

### A7b. No occlusion culling — and frustum culling currently buys nothing

There is no hierarchical depth buffer, no query-based culling, no reprojected visibility. Frustum
culling **does** exist (`frustum_cull.rs`, with its own tests, and it culls shadow casters against
the cascades too) and is correct — measured 0 of 60 000 objects passing when the camera looks away.

But it saves no time, for the reason set out in F1: the per-object walk happens before the cull
decision, and it is the whole cost. Adding occlusion culling would land in the same loop and buy
the same nothing. The order of work is the walk first, then a culling strategy — and the tool for
the walk is already in the tree.

Measured in `occlusion_culling`.

### A8. No render-world extraction, no runtime component definition, no immutable components, no system hot-patching

Measured in `ecs_absent_apis`:

- **No second world.** Rendering reads game components directly (measured: `set_render` sees 4
  live components with no extraction step). Cheaper and simpler; the cost is that render cannot
  run in parallel with game logic.
- **Component registration is typed.** `register_component_type::<T>()` only — the registered-type
  count moves `0 → 1 → 2` through typed registration alone. Raw access exists
  (`get_component_ptr` + `TypeId` read `Some(100)`), so *access* is dynamic but *definition* is not.
- **No immutable components.** `Mut<Frozen>` builds fine and wrote `−1` through no hook at all, so
  immutability is a preference, not a contract.
- **No system hot-patching.** Shaders *are* hot-reloadable (`demo/assets/shaders/*.wgsl`); Rust
  system bodies are not.

---

## B. Effects that exist but cannot be shaped

The effect is there and — where it was measured — it is correct. What is missing is the ability to
author it. The middle column is what a full implementation offers.

| Effect | Full capability | Gizmo | Measured in |
|---|---|---|---|
| Screen-space reflections | 6 knobs | **0** (on/off only, and no `enabled` flag — turning it off *destroys* the state) | `ssr` |
| Tone mapping | 7 curves + dither | **1** curve (ACES, hard-coded) + exposure | `tonemapping` |
| Bloom | 7 knobs | **2** (`intensity`, `threshold`) | `bloom_3d` |
| Depth of field | 6 optical knobs | **3** numbers + enable | `depth_of_field` |
| SSAO | 5 quality levels + temporal denoise | **1** (`strength`) | `ssao` |
| Anti-aliasing | 5 techniques | **2** (FXAA, TAA) | `anti_aliasing` |
| Shadow bias | 2 knobs | **0** — derived from cascade texel size | `shadow_biases` |
| Distance fog | colour / falloff / mode | **4 baked presets** + a blend factor; the numbers live in `deferred_lighting.wgsl` | `fog` |
| Volumetric | — | **0** knobs | — |
| Cascaded shadows | configurable | **all constants** in `csm.rs` (4 cascades, 3072², 100 m, λ 0.75) | `shadow_biases` |
| Level of detail | per-level distances + crossfade | **one** level, **one** rule (`distance > radius × 15`), no crossfade — and generation is gated on `vertex_count > 20000`, which **no engine primitive reaches** (`new_indexed` de-duplicates first; the largest primitive is 561 unique vertices). Measured: `create_sphere(1.0, 48, 72)` gets 0 LOD buffers at 3 575 vertices, while the same sphere via `from_vertices` gets one at 20 736. The better construction path is the one that loses LOD | `visibility_range` |
| Wireframe | global flag + default colour + per-entity include/exclude/colour | **1** — `WireframeConfig::global`, all or nothing, no colour. Measured consequence: the overlay is drawn in a fixed light colour, so on a light material it is nearly invisible (edge energy only ×1.20, 1.17 % of pixels). Also: drawn in the forward pass, skybox/backdrop excluded by design, and **inert on wasm** (`PolygonMode::Line` unsupported there) | `wireframe` |
| Clearcoat | strength + layer roughness + 3 textures | **1** (`Material::clear_coat`), and measurably near-binary: the peak jumps 161 → 205 from 0 to 0.2 and then flattens | `clearcoat` |
| Shadow caster / receiver | per-object cast **and** receive toggles | **caster only**, but richer there: `ShadowCasting::{On, Off, Only}` — `Only` casts without being drawn, which a plain cast toggle cannot express. **Receiving cannot be disabled at all.** Measured dark-pixel fractions 1.06 % / 2.07 % / 4.23 % | `shadow_caster_receiver` |
| Baked lighting | lightmap texture + second UV set | **vertex colour** via `MaterialType::BakedLit` — resolution is the mesh's tessellation, not a texture's. Still receives the sun's cascades, so mixed lighting works. Measured: the blue-minus-red balance flips sign, `+1.35` under `Pbr` and `−4.74` under `BakedLit`, because `BakedLit` does not see point lights | `lightmaps` |
| Irradiance volumes | component + GPU voxel sampling | the **maths exists and is unwired** — see F | `irradiance_volumes` |

Two measured caveats worth keeping:

- **Bloom's radius is not adjustable and is tight.** `bloom_3d` measured the halo contributing
  `+18.9` in the 60–80 px band and **exactly 0.00 beyond 80 px** at every intensity. Scale,
  max-mip and low-frequency-boost controls are precisely what would widen it.
- **The fog knob is not a fog knob.** `environment_preset` also selects the sky and the overall
  lighting mood, so fog cannot be changed independently of the scene's look.

---

## C. ECS and API gaps

Small individually; together they are what makes an idiomatic ECS snippet fail to compile.

### C0. The extension points are sealed

Measured in `ecs_extension_points`. Four traits are sealed and cannot be implemented downstream:

| Trait | Consequence |
|---|---|
| `SystemParam` | A custom system parameter cannot be written. **Only four impls exist: `Res<T>`, `ResMut<T>`, `f32` (dt), `Query<Q>`** |
| `WorldQuery` | No derived query operand |
| `ReadOnlyQuery` | — |
| `FetchComponent` | No custom fetch |

And two APIs are simply absent: **`pipe`** and **`run_system`** (one-shot systems). The seal is a
deliberate design choice — the scheduler has to know statically what a parameter touches or
parallel execution is unsound — but the consequence is real: several common patterns cannot be
expressed, only approximated. Measured substitutes all work (filtered query 3/3, plain function
`−0.800`, latched system ran exactly 1 time in 120 frames); what is lost is *structure*, not
behaviour.

**A knock-on effect:** because `SystemParam` is sealed and `Option<Res<T>>` is not among the four
impls, the common "tolerate a missing resource" parameter **does not compile**. The only route is
`run_if`, which skips the system entirely — so there is nowhere to put fallback behaviour
(`error_handling`).

### C1. Individual gaps

| Missing | Effect | Measured in |
|---|---|---|
| `Local<T>` system param | Per-system state must become a world resource, so it is visible to every other system and to the scheduler's conflict analysis | `ecs_guide` |
| `On<Remove, T>` / `On<Replace, T>` dispatch | The markers exist; `observer.rs` says there is "no dispatch path yet". Removal is observable only through a raw `RemoveHook` | `removal_detection`, `observers` |
| Global (non-entity) observers | `add_observer` is `On<Insert, T>` only; `observe` needs a target entity. An untargeted event has no counterpart | `observers` |
| Run conditions on observers | An observer cannot be gated | `observers` |
| A world handle inside observers/hooks | Callbacks get no world, so a chain reaction must go through captured state and advances **one ring per frame** (measured: 8 rings, 8 frames) | `observers` |
| `Visibility` component | Hiding means `ShadowCasting::Only`, which still casts a shadow — not the same thing | `infinite_grid` |
| ~~`Transform::forward/right/up/looking_at`~~ | **CLOSED 2026-08-23** — added with two tests (axes orthonormal and right-handed across a full turn of yaw; `looking_at` aims and survives degenerate input). `looking_at` returns the rotation rather than applying it, so it can be blended | `transform` |
| `.chain()` | Ordering is per-edge `label()` + `after()`, and an unmatched constraint **warns and is dropped** rather than failing. The manual form does work: 8 unconstrained systems produced **78 distinct orderings** in 600 frames, and the same 8 chained produced **1** | `transform`, `schedule_internals` |
| `or_else` for run conditions | Repeated `run_if` ANDs; OR must be written inside one closure, which also makes the system exclusive | `run_conditions` |
| `OnEnter` / `OnExit` state schedules | `State<S>` and `in_state` exist but **nothing drives transitions** — the application owns the `apply_transitions` call. And entering the *initial* state is never reported, so setup hung off "did a transition happen" leaves the opening state **empty** (measured: 0 entities for 120 frames) | `states`, `state_scoped` |
| `DespawnOnExit` / `DespawnOnEnter` | State-scoped entities must be despawned by hand. Hand-built and measured leak-free (spawned − despawned == 3 at every sample) | `state_scoped` |
| `EventMutator` | `EventReader`/`EventWriter` only | `events` |
| `iter_combinations` | No pairwise iteration, and the borrow rules make a one-pass hand-rolled version impossible: a query holding `Mut<T>` cannot even be iterated read-only (`iter` wants `ReadOnlyQuery`), so "read then write" is two `iter_mut` passes plus a `Vec`. One call becomes three passes. Correctness verified by conservation: total momentum stayed under 1e-5 over 1500 frames of a 12-body sim | `iter_combinations` |
| Entity generations in `Children` | `Children` is `Vec<u32>`; a recycled id aliases. And a plain `despawn` on a child leaves a **dangling id** in the parent (measured), which `remove_child` cannot clean up afterwards because it takes an `Entity` | `hierarchy` |
| A typed single-entity reader on `World` | There is none — reading one component means building a whole `Query`. A lifecycle hook is handed only the `Entity`, so seeing the value it fired for costs a query (measured working: it reads back `11`) | `component_hooks` |
| `Entity` from a query | Queries yield **raw `u32` ids**. Converting one safely needs `World::entity(id)`, which needs `&World` — and a scheduled system can never hold it (see C2). `Entity::new(id, 0)` compiles and is wrong for a recycled id | `state_scoped`, `entity_disabling` |
| A clear/sky colour setting | Fog only tints geometry; the empty background stays black | `fog` |
| `AlphaToCoverage` | No MSAA to spread alpha across | `transparency_3d` |
| Fallible system params | A parameter that cannot be fetched **panics** (measured: `❌ FATAL ECS ERROR ❌`). That is deliberate — the message says so. The only guard is `run_if`, measured working (guarded system ran 0 times, no panic). Cost: the system does not run at all, so there is no `else` branch | `fallible_params`, `error_handling` |
| Systems returning `Result`, `?`, an error handler | None of the three exist. Business-logic errors must be written to a resource | `error_handling` |
| Default query filters | There is no way to add an implicit filter to every query. This is the whole of "entity disabling": the marker component is trivial, the implicit filter is the feature. Measured cost of forgetting it once: **543 extra updates over 180 frames** | `entity_disabling` |
| Custom relationships | No relationship trait or derive; only the fixed `Parent`/`Children`. A reverse index can be hand-built from hooks — but see C3 for the hole that leaves | `relationships` |
| Observer propagation control | Bubbling works (measured `[4, 3, 2, 1, 0]` from leaf to root), but **no listener can cancel the walk** — measured 2 more links visited after one tried — and the path is always the `Parent` chain | `observer_propagation` |

### C2. `.exclusive()` is a concurrency barrier, not a mutability upgrade

`System::run` receives `&World`, never `&mut World`, and `.exclusive()` does not change that — it
only guarantees the system runs alone in its batch. **No scheduled system can hold `&mut World`.**
Anything that needs one — resolving a raw query id to an `Entity`, despawning what a query found,
running a stored closure — must live in the app-level `set_update` / `set_render` hook. The
engine's own `LifetimeSystem` takes the same route (`query_unchecked` + `world.entity(id)`).

### C3. Component hooks are not an audit trail

Hooks exist (`register_on_add` / `on_set` / `on_remove`, plus a global despawn hook — a superset
of the usual three in one respect, since the lists accumulate). But several paths write components
**without firing anything**. Measured in `component_hooks`:

| path | add | set | remove | despawn |
|---|---|---|---|---|
| `spawn_bundle` | 1 | 1 | 0 | 0 |
| `add_component` (new) | 1 | 1 | 0 | 0 |
| `add_component` (overwrite) | **0** | 1 | 0 | 0 |
| `remove_component` | 0 | 0 | 1 | 0 |
| `add_bundle` (all-Table) | **0** | **0** | **0** | **0** |
| `remove_bundle` (Table) | **0** | **0** | **0** | **0** |
| `insert_batch` (3) | 3 | 3 | 0 | 0 |
| `remove_batch` (3) | 0 | 0 | 3 | 0 |
| `spawn_batch` (**4**) | **1** | **1** | 0 | 0 |
| `clone_entity` (3) | **0** | **0** | **0** | **0** |
| `despawn` | 0 | 0 | 1 | 1 |

The cost is concrete. `relationships` hand-builds a reverse index from hooks, then adds four
entities with `spawn_batch`: the world gained 4 relationships, **the index gained 1**. Three
silently missing — exactly the 1-in-4 the table predicts. An index maintained by hooks is only as
complete as the code paths that reach it.

### C4. No custom schedules, executor, or stepping

Measured in `schedule_internals`. `Phase` is a **sealed 5-variant enum**, `run_batches` is private,
and there is no stepping API. There is also **no `Startup` phase**: the two substitutes run at
different times (setup closure at frame 0, a latched system at frame 1), and nothing can be both
"takes a `Query`" and "runs before the schedule's first turn" (`system_forms`).

Two things that *do* work with no engine support at all: a **closure** as a system (measured
carrying state across 89 calls, 7 → 96 characters) and a **generic** system registered per type
(`report_pool::<Health>` and `::<Mana>` both counting correctly).

**In-batch order is genuinely nondeterministic.** Batch *layout* is deterministic — insertion
order is the documented tie-break — but a batch runs concurrently under rayon, and two systems
taking `Commands` declare only a *read* of the queue, so they can share a batch and enqueue in
non-reproducible order. Measured over 600 frames: **78 distinct orderings** in one run, **54** in
another; the insertion order appeared barely half the time.

> **Measurement note, kept because it is the more useful half.** The first attempt used *two*
> unconstrained systems and got 600/600 identical across two runs. That was not evidence of a
> guarantee: rayon does not bother splitting a two-element slice, so both ran on the calling
> thread. Eight writers exposed it immediately. "I measured it and it did not vary" is not
> "it cannot vary".

---

## D. Game-path / editor-path asymmetries

`render_parity` exists precisely to catch these, and it is currently red on one. Recording the
whole set, because they are one family:

1. **`alpha_cutoff` is known to the game path only** — the editor draw path never learnt it.
   Pre-existing since `550a7df`; `render_parity` reports it.
2. **`RenderStats` (draw calls / triangles / instances) is filled by `gizmo-studio` only.**
   `default_render_pass` publishes nothing, so a *game* cannot read its own draw-call count.
   Measured in `many_cubes`.
3. **The grid pipeline is editor-only.** `MaterialType::Grid`, `grid.wgsl` and a compiled
   `grid_pipeline` all exist; only studio ever binds it. In the game path a grid material draws as
   a plain opaque PBR plane — measured in `infinite_grid`.
4. **`Material::emissive` is dead in the deferred path.** The G-buffer reads emissive from the
   *material uniform* (which only glTF loading fills), while game code writes the per-instance
   field. Only `baked_lit.wgsl` reads the instance field. Measured in `bloom_3d`.
5. ~~**`vignette` is reachable only through the `PostProcess` component.**~~ **CLOSED 2026-08-23.**
   The `Renderer` fallback struct had no such field, so a camera without the component was stuck
   with `PostProcessUniforms::default()`'s **0.25** — a vignette it never asked for and could not
   switch off. `Renderer::vignette_intensity` now exists and the fallback branch reads it; the
   default is still `0.25`, so **no pixel changed** — the value merely became reachable.

   Verified in `color_grading`, one capture batch, size-checked: component `vignette = 0`
   gives corner/centre **1.095** and fallback `renderer = 0.0` gives **1.095**; component `0.9`
   gives **0.608** and fallback `0.9` gives **0.608**.
6. **Vertex colour is read only by `baked_lit.wgsl`.** The attribute is always present — the one
   vertex layout carries it and an uncoloured import is normalised to opaque white — but the
   deferred PBR path never reads it. Measured in `vertex_colors`: on a mesh with red/green/blue
   corners and a white material, `BakedLit` shows a channel separation of **60.6** and `Pbr` shows
   **0.9**. This also closes the loop left open in `deferred_rendering`, where `BakedLit` and
   `Unlit` rendered identically for want of vertex colours.
7. ~~**`with_simple_scene` overwrites every camera in the world.**~~ **CLOSED 2026-08-23.** Its
   update closure wrote the fly camera's position, rotation, yaw and pitch to every entity
   matching `(Transform, Camera)` — no `primary` filter — so a second camera was silently stomped
   each frame. Measured in `split_screen` and again in `render_to_texture`.

   The pose now goes only to the camera `active_camera` names, which is the renderer's own rule
   rather than a second copy of it: "the camera the player flies" and "the camera the frame is
   drawn from" are now the same sentence. The closure was also lifted out as the public
   `gizmo::simple::simple_scene_update` (see item 8), and locked by
   `the_fly_camera_writes_only_to_the_rendered_camera`, which goes red if the filter is removed.
8. **`App::set_update` replaces the update hook; it does not chain.** The builder stores
   `self.update_fn = Some(f)`. So a demo or game that writes
   `.with_simple_scene(..).set_update(..)` — the obvious thing to write when it needs an exclusive
   `&mut World` — silently discards *all four* of the simple scene's per-frame jobs: camera
   control, CPU physics stepping, `TransformSyncSystem` and `TransformPropagateSystem`. Nothing
   warns and nothing fails to compile; the camera simply stops responding.

   Found 2026-08-23 in six of this repo's own demos. `demo::simple_scene_update` now exists as the
   shared fix, and its doc comment carries the warning.
9. **`DespawnAfter` is inert without `LifetimePlugin`**, which is not on by default. The component
   attaches, nothing reads it, the entity never dies, and **nothing warns**. Found in
   `delayed_commands` only because the demo ran the engine's path beside a hand-built one and saw
   the engine side grow 5 → 10 → 15 while the hand-built side drained correctly.

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
2. **Wire up what already exists.** Cheapest ratio of work to capability in the whole list, and it
   is three items: the **spatial index** (F1 — and `occlusion_culling` measures the 8.46 ms/frame
   at 60 k entities it would attack), the **irradiance-volume subsystem** (F2), and the
   **post-process knobs** (B), which are uniform fields and shader constants that exist but are
   not exposed. SSR and tone mapping have the widest gap.
3. **`Visibility`, `Local<T>`, `or_else`, default query filters (C).** Small, self-contained, and
   each removes a recurring papercut. (`Transform`'s direction helpers were the first of this group
   and are done.)
4. **Extrusion machinery (A4).** One feature, 15 shapes.
5. **User materials (A3).** The largest and most architectural; it interacts with `routing.rs`, the
   G-buffer budget and `render_parity`.
6. **2D pipeline (A1).** Largest of all; arguably a separate project.

---

## F. Two subsystems wired to nothing

### F1. The spatial index — `gizmo_renderer::visibility`

`RenderAabbTree` is a complete BVH over renderable AABBs: `insert` / `remove` / `retain`,
`query_frustum` / `query_frustum_full_mask` / `query_frusta` / `query_aabb`, a `VisibleSet`
companion, a benchmark suite, an independently-written verification harness
(`tests/visibility_independent.rs`), and `differential.rs`, which exists solely to prove the
indexed path and the linear path agree entity-for-entity. Its module doc carries a correctness
argument and a measured crossover table.

**No render path calls it.** Verified 2026-08-23: outside `visibility/`, every mention of
`RenderAabbTree` is the `pub use` in `lib.rs`, the doc example there, the benchmark, or its own
test — and that test *prints the fact* at line 1411. Both draw paths
(`batching.rs:424`, `gizmo-studio/src/render_pipeline/mod.rs:255`) call
`classify_visibility_world` linearly on every mesh with no index in front of it.

The framing that makes this "unwired" rather than "unfinished": `lib.rs` positions the tree as
game-facing API — *the renderer does not iterate entities, so the cull is yours* — but the engine
ships two paths that **do** iterate entities, and neither uses it.

Why this matters is measured in `occlusion_culling`. At 60 000 entities the frame costs 18.94 ms
with **every object culled** and 18.54 ms with every object drawn — culling saves nothing, because
the per-object walk that precedes the cull decision is the whole cost. Decomposed:

| | ms | attributable to |
|---|----|-----------------|
| 0 entities | 5.82 | baseline |
| 60 000, transforms only (no `MeshRenderer`) | 10.48 | **+4.66** transform systems |
| 60 000, all frustum-culled | 18.94 | **+8.46** batcher walk |
| 60 000, all drawn | 18.54 | **−0.40** the actual drawing — inside noise |

Drawing 60 000 cubes is unmeasurable next to deciding whether to draw them. The batcher's walk is
the single biggest term, and an index in front of it is precisely what removes that walk for
culled objects. The module's own table puts the crossover for cull *time alone* between 8 k and
32 k meshes; the walk it would skip is a larger term than the test it would accelerate
(0.141 µs/entity of walk against 0.022 µs/entity of test at 32 k).

Its docs are honest about the limits: *"Measure on your own scene before believing any of the
above"*, and it is explicitly **not** an occlusion structure.

### F2. Irradiance volumes — `gizmo_renderer::gi`

This one contains a complete irradiance-volume implementation: `SHCoeffs` with
`add_directional_light` / `evaluate` / `lerp` / `to_gpu_data`, `LightProbe`, and `ProbeGrid` with
analytic baking and trilinear sampling. It has **seven tests**. It is `pub use`d from `lib.rs`.

And nothing uses it. Measured 2026-08-23:

| search | result |
|---|---|
| files mentioning `ProbeGrid` outside `gi.rs` | only the `pub use` in `lib.rs` |
| callers of `to_gpu_data` | **only its own test** |
| shaders reading SH coefficients | **none** |
| demos using it (before `irradiance_volumes`) | **none** |

Written, tested, exported, unplugged. `irradiance_volumes` drives it by hand to show the maths is
sound — 48 probes baked from the scene lights, sampled per object, applied to albedo on the CPU
because the pipeline cannot. The blend is correct: red-dominant `(0.935 0.462 0.333)` on the left,
blue-dominant `(0.366 0.533 0.902)` on the right, smooth through the middle.

These two are the largest ratio of existing work to delivered capability in the engine: both are
finished, both are tested, and neither is plugged in.

---

## G. Defects found and fixed during the sweep

Eight, all with regression coverage or an explicit report:

| Defect | Fix |
|---|---|
| Orthographic cameras jittered TAA into the wrong matrix column — a static scene had 2.1 % of pixels moving | Fixed + unit tests + a golden render test |
| A cut-out material fell into the transparent bucket as its alpha dropped, contradicting `Material::alpha_cutoff`'s own contract | Fixed in `batching.rs` + tests |
| The forward shader ignored `alpha_cutoff` entirely | Fixed in `shader.wgsl` |
| Spatial audio's ears were **mirrored**: `listener.right` was the negation of `Camera::get_right()`, so a sound on the camera's right reached the left ear in every simple-scene game | Fixed + a regression test that sweeps a full turn of yaw |
| `Renderer::exposure` is read by nothing; the game path reads `Camera::exposure`. A demo's exposure slider was wired to the dead field | Slider rewired; dead field documented |
| `vignette` was unreachable without the `PostProcess` component (D5) | `Renderer::vignette_intensity` added; no pixel changed |
| A `#[cfg(test)]` module placed mid-file truncated what `render_parity` could see, mis-reporting 14 capabilities as editor-only | Module moved to the end of the file |
| The fly camera wrote its pose to **every** camera in the world, so a second camera could not hold its own (D7) | Filtered to the camera `active_camera` names + a regression test that goes red on revert |

Reported and now documented rather than changed: `set_update` still replaces the simple scene's
hook (D8), but it warns when it overwrites one and its docs say so, and
`gizmo::simple::simple_scene_update` is public so the four jobs can be kept; `DespawnAfter` is
still inert without `LifetimePlugin` (D9), but its own docs now say so.

Reported, not fixed: deferred `alpha_cutoff` needs the z-prepass to gain a fragment stage; the
editor path never learnt `alpha_cutoff`; `Material::emissive` is dead in the deferred path (D4).
