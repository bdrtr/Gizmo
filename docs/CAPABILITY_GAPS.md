# Capability gaps — what the engine cannot do yet, measured

**Status:** measured, not estimated. **Last updated:** 2026-08-23.

## What this is

Between 2026-08-22 and 2026-08-23 a demo suite was built in `demo/src/bin/`, one capability at a
time, under one rule: **do not hide where the engine falls short — write the shortfall into the
demo's header, with a measurement.** This file is the aggregate of those headers. Every claim
below names the demo that measured it, so it can be re-run rather than believed.

The sweep also found **eight real engine defects**, **three complete subsystems wired to nothing**,
and one behaviour whose own source comment described it backwards; they are listed at the bottom.

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

### A2. Text / font rendering — **the engine draws glyphs since 2026-08-24**

Until then nothing drew a glyph in world space or screen space except `egui`, which is a
debug/editor UI layer rather than a game text system: no font asset, no text mesh, no layout, no
rich text, no world-space labels. Every demo in the sweep that wanted a label used an `egui` panel,
which is why they all looked like debug overlays. This was the gap the project owner named as the
first to close.

**What exists now.** `gizmo-renderer::text` — a font library, a glyph atlas, layout — plus a `Text`
component with two spaces, nine anchors and a colour, drawn by **both** hosts through one shared
pass (`gizmo::systems::render::record_text`, which `gizmo-studio` calls rather than copying).

| | |
|---|---|
| screen space (window pixels, top-left origin) | yes |
| world space (camera-facing quad) | yes, **depth-tested** — a label behind a wall is behind it |
| anchors | nine |
| lines | `\n`, and nothing else breaks one |
| kerning | from the face's own `kern` table |
| **wrapping, shaping, bidi, font fallback, rich text** | **no** |

Measured: the same scene with and without text differs in **869/16 384** pixels; a string anchored
at (8, 8) changes **869** pixels in the top-left quadrant and **0** in the bottom-right (the
assertion a flipped y axis fails); a world label behind a wall changes **0**, and **1 589** with
`CompareFunction::Always` in the world pipeline's place.

**Three things had to exist that did not.** The rasteriser (`ab_glyph`, sealed — no foreign type on
a `pub` signature, and already in the lockfile through winit's Wayland decorations); a **font for
the tests**, since the repository ships none and a typeface is a licensing decision rather than a
technical one — so `text::synthetic` builds a real TrueType file in memory with known metrics, and
every assertion is a derivation rather than a measurement of somebody else's face; and the atlas's
shelf packer, whose placement is checked for overlap as a property rather than by sampling.

**Still missing, and named rather than discovered later.** No default font — a `Text` whose face was
never loaded draws nothing rather than substituting one. Text is drawn into the HDR target before
tone mapping, so it is exposed and bloomed with the rest of the frame: right for a world label, and
the first thing to revisit when there is a post-tonemap pass to hang UI on. And **`gizmo-ui` is not
connected to it**: `Node` still emits no vertices, `BackgroundColor` is still written and never
read, and a UI rect is not yet handed to a `Text`. That is the next piece, and it is a smaller one
than this was — see section E.

### A3. User-authored shaders — **open since 2026-08-23**, forward-only

`MaterialType` was a fixed enum: a game could not add a variant, supply its own WGSL, or attach its
own uniform block. `MaterialType::Custom(MaterialId)` closes that, with
`MaterialRegistry::register` minting the id and `CustomMaterial::from_wgsl` compiling the shader
against the engine's contract — the same composer the engine's own shaders use, so `#import` and
`#{INSTANCE_GROUP}` work in a game's WGSL too.

**Forward-only, and that is a measurement rather than a policy.** The four G-buffer targets share a
32-byte `max_color_attachment_bytes_per_sample` budget and spend 28 (albedo `Rgba8` 4 + normal,
position and tangent `Rgba16Float` 8 each). Four bytes left is one more `Rgba8` and not one
`Rgba16Float`, so a custom material bringing its own G-buffer channel would spend the last of a
shared budget on one feature.

**There is also no spare bind group**, measured the same way: native asks for
`max_bind_groups: 6` and spends 5 (`0` scene, `1` material, `2` shadow, `3` skeleton, `4`
instance); the web asks for 4 and spends 4. A design giving a custom material its own group would
work on the desktop and fail to compile a shader in the browser. The room is inside group 1 —
seven entries, four of them textures, whose meaning a custom shader redefines.

Measured in `custom_material`, three spheres rendered alone: mean and standard deviation separate
PBR / `Unlit` / custom weakly (200.97/28.89, 216.64/12.16, 192.97/31.05), but the FFT of each
sphere's vertical brightness profile does not — the custom material's dominant cycle carries
**854.5 power, 13.3 % of its spectrum, against PBR's 99.8**. It casts shadows like PBR (408 ground
pixels against 374), which took its own fix: a custom material routes as `unlit`, and the shadow
pass dropped everything `unlit && !baked_lit`, so `CustomMaterial::casts_shadows` would have been a
field that read true and did nothing. An id with nothing behind it draws **nothing** rather than
falling back to PBR; restoring the fallback leaks 5770/16384 pixels, which is what the guard test
measures.

Still open: choosing the forward path for a *built-in* material. Measured in
`deferred_rendering` — doing so means changing the material type, which also changes the shading,
so two axes that ought to be separate are one axis. **A PBR material cannot be drawn forward at
all**, and a custom material cannot be drawn deferred.

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

### A5b. No split screen / multi-viewport — two of the three obstacles are closed

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
3. ~~**There is one `global_uniform_buffer`, written with `queue.write_buffer`.**~~ **CLOSED
   2026-08-23** — as `SceneView`. Those writes order against *submission*, not against encoder
   recording, so two passes recorded into one encoder both read the **last** camera written; this
   was the deepest of the three and was invisible until the first two were worked around. A
   `SceneView` owns its own uniform buffer and group-0 bind group, `SceneState` gained
   `views` + `active_view`, and every pass now binds through `view_bind_group()` and writes through
   `view_uniform_buffer()`. Two views, one encoder, one submit.

One remains — flipping `primary` between calls — and the encoder-per-camera workaround is gone.
Measured on the off-screen target's three brightness bands: the trap gives `54.42 / 139.18 / 84.39`,
per-camera-submit gives `88.80 / 101.40 / 63.07`, and `SceneView` in one encoder gives
`89.70 / 99.02 / 86.14`.

**The third band was a real remainder, measured rather than reasoned about — and closed
2026-08-24.** Replacing the sun with a shadowless `Generic` light made the two agree to within 1.5
on all three bands, so the whole difference was the shadow cascades: *derived* from the camera into
`shadow_cascade_uniform_buffers`, and shared, along with the cluster table `upload_clusters` fills.
Obstacle 3 again, one level down. Both now live on `SceneView`, reached through
`view_shadow_cascade_buffer`, `view_point_shadow_buffer` and their bind-group pairs, and the lower
band reads **62.82** against per-camera-submit's 63.07.

**The shadow textures did not need copying**, which is the measurement that kept this cheap: render
passes execute in recording order, so each view's shadow pass redraws the same cascade array
immediately before that view's main pass samples it. Only the uniforms could not be shared, because
`queue.write_buffer` orders against submission. A second view costs about **460 KB**, not the
144 MB a second 3072²×4 cascade array would.

`two_cameras_in_one_encoder_render_two_different_frames` guards it, and had to be rewritten to do
so. Its original claim — "two passes without a view are pixel-identical" — was measuring the wrong
half: the *second* pass reads the last camera written, so it is right by accident, and the **first**
is the one rendering someone else's camera. It now compares the first pass against that camera
rendered alone: 0 pixels of drift with per-view state, and reverting either separation is visible —
**283 pixels** for the cascades, **2912** for the cluster table.

**Still shared: temporal state.** TAA history and SSGI accumulation are per-renderer, not per-view.
The test disables both, because with them on a single frame carries a 500-pixel noise floor that
buries the camera's own difference. That is the next layer of this same gap.

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

### A7c. No motion blur, and no auto exposure

**Motion blur.** Measured in `motion_blur`: three camera speeds captured at the same angle give
edge energy `9.977 / 9.977 / 9.977`, and the frames differ only in the HUD text — sixteen times
the speed, bit-identical pixels. The frame is a zero-duration sample. Object motion blur is closed
at the data layer: `InstanceRaw` carries only `model`, no previous `Transform` is kept anywhere,
and none of the G-buffer's four targets is velocity.

What exists is most of the camera half: TAA already keeps the previous unjittered view-projection
and the G-buffer already writes `world_position` — the two inputs a camera-blur shader needs.

**Auto exposure.** The actuator is live and has authority. Measured in `auto_exposure`: a bright
and a dark station differ 2.64× in mean luminance and neither drifts over 580 frames (+0.128 and
+0.015 — capture noise). Sweeping `Camera::exposure` 0.5 → 4.0 moves screen luminance 31.8 → 130.4,
more than enough to close that gap.

**The sensor is writable, and that corrects what this entry used to say.** It read "the sensor does
not exist"; what does not exist is a *built-in* one. `PostProcessState::hdr_texture` is
`RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC` with texture, view and bind group all public, so a
game can read the frame it is about to tone-map. The demo now does: read back, sample every eighth
pixel, drive `Camera::exposure` toward `0.18 / measured`. It lands exactly where the arithmetic
says (0.3656 → 0.492, 0.0534 → 3.370) and closes 82 % of the gap between the two stations
(118.18 → 20.93 in rendered brightness).

**The reduction landed 2026-08-24** as `gizmo_renderer::luminance::LuminanceReduce` — two
dispatches, tile partials then a final sum, `sums[0]` holding the mean. Four unit tests pin it
against known inputs: a flat frame, a half-black/half-white split (which is what catches a tree
reduction dropping tiles), Rec. 709 weights rather than a plain average, and an infinite texel not
poisoning the mean.

**And measuring it corrected the estimate above.** The demo now runs four modes as four separate
runs and times whole frames rather than one call:

| mode | mean frame (two runs) | sensor call |
|---|---|---|
| no sensor | 0.168 · 0.200 ms | — |
| CPU frame readback | 0.825 · 0.822 ms | — |
| GPU reduce + 4-byte readback | 0.785 · 0.787 ms | 7.1–7.2 ms |
| GPU reduce, **no readback** | 0.185 · 0.199 ms | **0.002 ms** |

Recording the reduction costs 2 µs, as expected. What was wrong was the conclusion drawn from it:
adding a reduction buys nothing on its own. Reducing on the GPU and then reading the answer back
(0.785) beats pulling the whole frame (0.825) by **5 %**, because both do the same thing — stall on
`poll(Wait)` — and whether the copy is 4 bytes or 4 MB barely matters. The win is in **not reading
it back at all**, and its size should not be quoted from one run: `gpu-only` measured 0.017 ms above
the baseline in the first and 0.001 ms *below* it in the second, so it sits inside the noise.

`LuminanceReduce::result_buffer()` is where the number stays. Feeding it to the next frame's
post-process — closing the loop without the CPU ever seeing it — is the remaining work, and needs
`post_process.wgsl` to read exposure from that buffer.

### A7d. No planar reflections, and SSR's limit priced

No reflection-plane concept, no oblique frustum, no stencil buffer, and a render target cannot be
bound back onto a material (same `pub(crate)` wall as C1). What remains is SSR, and
`mirror` puts a number on what it cannot do: a red column facing a smooth metallic floor
contributes +7.91 of red−blue to it while on screen and **exactly nothing** once it moves behind
the camera. Showing what the camera cannot see is the case a mirror exists for.

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
| Screen-space reflections | 6 knobs | **8 shaping fields since 2026-08-24** — `SsrParams::{roughness_cutoff, fade_start, fade_end, step_size, max_steps, thickness, start_offset, edge_fade}`, one 32-byte uniform per frame, defaults pixel-identical to the shader literals they replaced. Plus `enabled` (2026-08-23). Measured: five of them move 11–19 % of the frame; `max_steps` moves nothing on that scene because the reflection is found on the first step, which is a property of the fixture rather than of the field. Still absent, and never written: step exponent, binary refinement, secant search | `ssr` |
| Tone mapping | 7 curves + dither | **4 curves since 2026-08-24** — `TonemapCurve::{None, Reinhard, ReinhardExtended, Aces}` via `Renderer::tonemap_curve`, **default still ACES** and locked as such in three places, because every existing scene was authored under it. Measured on one scene: they differ from ACES by 4.91 / 7.20 / 5.41 % of pixels. Still absent: luminance-Reinhard, AgX, SBDT, Tony McMapface, Blender Filmic, deband dither | `tonemapping` |
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
| Irradiance volumes | component + GPU voxel sampling | the **maths exists and is unwired** — see F2 | `irradiance_volumes` |
| Volumetric / god rays | scattering, steps, phase, distance | **All six, since 2026-08-23** — `VolumetricParams::{phase_g, steps, max_distance, sun_scatter, bulb_scatter, shadow_bias}`, one 32-byte uniform uploaded per frame, and the defaults are pixel-identical to the shader literals they replaced. Plus `enabled`. Measured working: 19.55 % of pixels, max 134 | `volumetric_fog` |
| Blend modes | alpha / additive / multiply / premultiplied | **1** — `ALPHA_BLENDING`, and no `BlendMode` type to select another | `blend_modes` |
| Specular tint | a colour F0 | **0** — F0 is the literal 0.04; the only lever is `metallic`, which kills the diffuse (luminance −24 %) and cannot deliver the colour anyway for want of an environment (measured: red−blue +9.04 → −0.17) | `transmission` |
| Transmission / thickness / IOR | per-material | **0 fields** | `transmission` |

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
| `SystemParam` | A *primitive* parameter still cannot be written. Six impls now: `Res<T>`, `ResMut<T>`, `f32` (dt), `Query<Q>`, `Option<P>`, and anything `system_param!` declares — the last being a **composite**, so grouping existing parameters no longer needs the seal opened |
| `WorldQuery` | No derived query operand |
| `ReadOnlyQuery` | — |
| `FetchComponent` | No custom fetch |

And two APIs are simply absent: **`pipe`** and **`run_system`** (one-shot systems). The seal is a
deliberate design choice — the scheduler has to know statically what a parameter touches or
parallel execution is unsound — but the consequence is real: several common patterns cannot be
expressed, only approximated.

**Composite parameters no longer among them (2026-08-23).** `system_param!` declares a struct that
is a single parameter, forwarding `get_access_info` field by field — which is precisely the
information the seal protects, produced mechanically rather than by hand. `sealed::Sealed` became
`#[doc(hidden)] pub` so the macro can name it from the calling crate; implementing it by hand still
compiles and still means writing the access declaration by hand, so the seal means what it meant.
Guarded at the scheduler, not only at the declaration: a composite writing `ResMut<T>` and a plain
`ResMut<T>` writer must land in two batches, and that test goes red — "two writers of the same
resource were put in one batch" — the moment the forwarding is removed. Measured in `system_forms`:
one parameter carrying three, 89 runs, `health 240 · mana 120`. Measured substitutes all work (filtered query 3/3, plain function
`−0.800`, latched system ran exactly 1 time in 120 frames); what is lost is *structure*, not
behaviour.

~~**A knock-on effect:** because `SystemParam` is sealed and `Option<Res<T>>` is not among the four
impls, the common "tolerate a missing resource" parameter does not compile.~~ **CLOSED 2026-08-23.**
`Option<P>` is now a parameter for any `P: SystemParam`, so there are five impls. The seal stays —
this was added inside `gizmo-core` rather than by opening the trait, which is the first item of
`docs/API_DEPTH.md`'s sequence.

Measured in `error_handling`, same missing resource in one frame: `Res<T>` panics, the
`run_if`-guarded system runs **0** times, and `Option<Res<T>>` runs **90** times seeing `None` in
all 90 — so there is now somewhere to put fallback behaviour. Only *absence* becomes `None`; a
borrow conflict still panics, because it is a scheduling bug rather than a state.

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
| `iter_combinations` | **Present, both forms.** Read-only since 2026-08-23; `iter_combinations_mut` since 2026-08-24, handing out both halves writable. That one needs `unsafe` — two `&mut` into one storage, sound only because `i != j`, which the borrow checker cannot see — and the invariant is carried by the iterator's structure rather than a type. Measured three ways: the read-only loop against a hand-rolled one gives **66 pairs each and a largest acceleration difference of exactly 0.000000000** over 1500 frames; the single-pass mutable version reproduces the three-pass version's kinetic energy digit for digit (55.905 / 64.584 / 40.142) with momentum under 1e-4 in both. **Miri clean**, and that mattered: the first version passed all six unit tests and Miri rejected it — `transmute_copy` + `mem::forget`, where the forget's move retagged and invalidated the copy | `iter_combinations` |
| Entity generations in `Children` | `Children` is `Vec<u32>`; a recycled id aliases. And a plain `despawn` on a child leaves a **dangling id** in the parent (measured), which `remove_child` cannot clean up afterwards because it takes an `Entity` | `hierarchy` |
| A typed single-entity reader on `World` | There is none — reading one component means building a whole `Query`. A lifecycle hook is handed only the `Entity`, so seeing the value it fired for costs a query (measured working: it reads back `11`) | `component_hooks` |
| `Entity` from a query | Queries yield **raw `u32` ids**. Converting one safely needs `World::entity(id)`, which needs `&World` — and a scheduled system can never hold it (see C2). `Entity::new(id, 0)` compiles and is wrong for a recycled id | `state_scoped`, `entity_disabling` |
| A clear/sky colour setting | Fog only tints geometry; the empty background stays black | `fog` |
| `AlphaToCoverage` | No MSAA to spread alpha across | `transparency_3d` |
| Binding a normal / MR / emissive / AO map | **Open since 2026-08-23** — `AssetManager::material()` is a builder over the seven-entry material bind group, every slot optional with a neutral default (base colour and sampler included), so `material().normal(&view).build(..)` is a complete material. `params` is nameable too. Before it, `assemble_material_bind_group` was `pub(crate)` and the public surface gave base colour only, with the glTF loader as the sole route to the other four. Cost measured then and now: the same height field as geometry is 55 296 verts at shading σ 39.27; as a normal map it is **6 verts at σ 38.98**, 99.3 % of the variation for 1/9216 of the geometry | `parallax_mapping` |
| A user post-pass that reads the frame | `set_render` really does let a game add its own full-screen pass (measured: 38.67 % of pixels, and the untouched rows stay bit-identical, so `LoadOp::Load` preserves the engine's frame). The *surface* has no `TEXTURE_BINDING`, so a pass cannot sample the swapchain — but it does not have to: `PostProcessState::hdr_texture` is `TEXTURE_BINDING | COPY_SRC` and its texture, view and bind group are public, which is the frame the chain itself reads. A user FXAA or radial blur is writable through it; what is missing is a **compute reduction**, priced at 10 ms a CPU readback in `auto_exposure` | `post_processing`, `auto_exposure` |
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

### C4. No custom executor or stepping — but phases of your own now exist

Measured in `schedule_internals`. `run_batches` is private and there is no stepping API. There is
also **no `Startup` phase**: the two substitutes run at
different times (setup closure at frame 0, a latched system at frame 1), and nothing can be both
"takes a `Query`" and "runs before the schedule's first turn" (`system_forms`).

**Closed (2026-08-23): `Phase::User(u16)`.** `Phase` used to be a sealed 5-variant enum, which is
what made "a schedule phase of its own" a locked door. It now carries a *position* on the scale the
built-ins sit on — `PreUpdate`=1000 … `Render`=5000 — so `User(3500)` means "after physics settles,
before transforms propagate", and `User(5001)` means after `Render`, which nothing could express
before. `Ord` is hand-written over `Phase::position()`; a derived one would sort every `User` behind
every built-in and the whole point would be lost. Measured in `schedule_internals` over 600 frames:
ten probes, expected sequence `0P1U2F3O4R`, **599 frames held it, 0 deviated** — and the system that
reads the sequence is itself in `User(5001)`.

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
8. **`App::set_update` replaces the update hook; it does not chain.** The builder stored
   `self.update_fn = Some(f)`. So a demo or game that writes
   `.with_simple_scene(..).set_update(..)` — the obvious thing to write when it needs an exclusive
   `&mut World` — silently discards *all four* of the simple scene's per-frame jobs. Found
   2026-08-23 in six of this repo's own demos.

   **Fixed 2026-08-23:** `App::add_update_hook` installs beside the existing hooks (`update_fn`
   became `update_fns: Vec<_>`); `set_update` still clears the list and still warns.

   **And the cost was not what this item said it was.** The `update_hooks` demo measures all three
   forms over 300 frames. CPU physics really stops — a free-falling body descends **0.000** units
   under the trap against ~6.9 either working way. But `TransformSyncSystem` and
   `TransformPropagateSystem` do *not* stop: propagation also runs in `TransformPlugin`'s scheduled
   system and in `ensure_global_transforms` before each draw, so `set_update` swallows one of
   three. What is lost is the hook seeing a current `GlobalTransform` **in its own frame**, and the
   measured signature is a single 2.5-unit step on frame one — exactly the child's distance from
   its parent — as `Mat4::IDENTITY` is replaced by the real pose. The camera is the one job with no
   second home, and it is the one that could not be measured windowless, since `fly_step` reads
   input.
9. **`DespawnAfter` is inert without `LifetimePlugin`**, which is not on by default. The component
   attaches, nothing reads it, the entity never dies, and **nothing warns**. Found in
   `delayed_commands` only because the demo ran the engine's path beside a hand-built one and saw
   the engine side grow 5 → 10 → 15 while the hand-built side drained correctly.

10. **`MaterialType::Skybox` draws nothing in the game path — cause unknown.** Measured in
    `atmosphere`: removing the skybox entity changes **0 pixels, max 0**. Every sky pixel comes
    from the deferred environment term. Eliminated as causes: scale (400/100/30/12 all identical),
    `GlobalTransform` present or absent, `Skybox` vs `Unlit` material, inverted vs ordinary cube,
    the far plane (1500, cube at ±200), mesh validity (24 indexed verts, bounds ±1), pipeline
    binding (`forward.rs:115`), face culling (sky pipeline is `cull_mode: None`), alpha (returns
    1.0), depth (`Clear(1.0)` with `LessEqual`) and pass order (forward runs after deferred
    lighting). Other cubes in the same scene draw. **Open question**, recorded with its elimination
    list rather than closed with a guess.

11. ~~**`Material::double_sided` is ignored on a transparent surface.**~~ **Not a defect — the
    source comment was.** `passes/forward.rs` claimed "a transparent double-sided surface is
    single-sided in both paths". Measured in `blend_modes`: a back-facing transparent plane
    renders, and toggling `with_double_sided` gives a bit-identical frame. The `transparent`
    pipeline is built with `cull_mode: None` (and `baked_lit_state(true)` agrees), so transparent
    geometry is never back-face culled — the flag is redundant, not ignored. Comment corrected
    2026-08-23.

A full audit of this family was run (2026-08-23): of `Material`'s shading fields, the deferred
G-buffer honours 6 (albedo, roughness, metallic, anisotropy, clear-coat, subsurface) and ignores
three — `ambient` (**documented** as PBR-path-excluded, intentional), `alpha_cutoff` (known, item 1)
and `emissive` (item 4, the only undocumented one). No further silent fields.

---

## E. Suggested order of work

> Sequenced by capability. For the *shape* the surface should take as these land — layered access,
> and the rule that every convenience must name its escape hatch — see `docs/API_DEPTH.md`. Several
> entries below are one unlock away from each other there. Per-pass uniforms — expected to close
> multi-viewport, planar reflections and reflection probes on its own — landed 2026-08-23 as
> `SceneView` and closed the camera half of all three; the shadow cascades and cluster table are
> still shared per encoder, and that is what the three now wait on (A5b, item 3).

1. ~~**Text / font rendering (A2).**~~ **The engine half landed 2026-08-24** — `gizmo-renderer::text`,
   a `Text` component in two spaces, one shared pass both hosts call. What is left of this item is
   the **`gizmo-ui` half**, and it is smaller than what landed: `Node` publishes absolute
   window-pixel rects already, and `TextSpace::Screen` takes absolute window pixels, so connecting
   them is a system that copies one into the other — plus consuming `BackgroundColor`, which is
   still written and never read. The layering allows it: `gizmo-ui` sits *above* `gizmo-app`, so it
   can see the renderer's types, while the renderer cannot see `Node`. That is also why the
   component landed in `gizmo-renderer::components` rather than in `gizmo-ui` as the note below
   assumed — a `Text` in `gizmo-ui` would have been invisible to both draw paths.

   Two things the sweep listed as constraints held: the wasm target is a separate CI gate and the
   rasteriser builds there, and the golden render test is the acceptance, because a picture is the
   only thing that can say a glyph landed where it should.

   *The original note, kept because its landing sites were right:* it was tracked as **M7.6** in
   `ENGINE.md` §3 (Phase 7), and `gizmo-ui`'s own crate docs named the same gap and the same site —
   *"expect the component set to change when rendering lands (a `Text` component and a draw-list
   output are the obvious additions)"*.

   Of the four pieces the plan listed, three are done: the **rasteriser + atlas** (`ab_glyph`,
   sealed; nothing evicted, and the atlas says so through `is_full` rather than dropping glyphs
   quietly), the **`Text` component** — in `gizmo-renderer::components`, not `gizmo-ui`, for the
   layering reason above — and **both draw modes**, which turned out not to want the existing
   `BatchKey`: a glyph is an instance rather than a mesh, so text is one instance buffer and two
   draw calls, not a batch per atlas texture.

   What is left is the **`gizmo-ui` connection**: a system that copies each `Node` rect into a
   `Text`'s `TextSpace::Screen` position, and a draw of `BackgroundColor`, which is still written
   and read by nothing.
2. **Wire up what already exists.** ~~Cheapest ratio of work to capability in the whole list~~ —
   **and the spatial index turned out not to be** (F1, measured 2026-08-24: wiring it makes the
   frame 90 % slower on a scene with shadow cascades, because their union culls nothing). It is
   wired as an opt-in resource with its conditions documented. **F2 landed 2026-08-24** — the
   probe grid now reaches the deferred lighting pass, measured against the CPU path at correlation
   0.99887. **F3 landed 2026-08-24 too** — water draws through its own forward pipeline, and the
   line that kept it dead was `route(Water)` returning exactly `route(Pbr)`'s answer. That closes
   section F entirely. What remains here is the
   **post-process knobs** (B), which are uniform fields and shader constants that exist but are
   not exposed. **Volumetric's six left B on 2026-08-23** (`VolumetricParams`) and **SSR's eight on
   2026-08-24** (`SsrParams`); all five screen-space effects also gained a reversible `enabled`.
   **Tone mapping's curve selector landed 2026-08-24** (four curves, ACES still the default).
   What remains in B is the narrower end of each table — luminance-Reinhard and the film-emulation
   curves, SSR's step exponent and refinement passes — none of which is code that exists and is
   unexposed; they were never written.
3. **`Visibility`, `Local<T>`, `or_else`, default query filters (C).** Small, self-contained, and
   each removes a recurring papercut. (`Transform`'s direction helpers were the first of this group
   and are done.)
4. **Extrusion machinery (A4).** One feature, 15 shapes.
5. ~~**User materials (A3).**~~ **DONE 2026-08-23** — `MaterialType::Custom(MaterialId)`,
   forward-only. The G-buffer budget it was said to depend on settled the question instead of
   blocking it: 28 of 32 bytes are spent, so a custom material cannot bring a target and declares
   itself forward. `routing.rs` stayed exhaustive and produced exactly one compile error.
6. **2D pipeline (A1).** Largest of all; arguably a separate project.

---

## F. Three subsystems wired to nothing — all three closed 2026-08-24

### F1. The spatial index — wired opt-in, and the cascades decide whether it helps

`RenderAabbTree` is a complete BVH over renderable AABBs: `insert` / `remove` / `retain`,
`query_frustum` / `query_frustum_full_mask` / `query_frusta` / `query_aabb`, a `VisibleSet`
companion, a benchmark suite, an independently-written verification harness
(`tests/visibility_independent.rs`), and `differential.rs`, which exists solely to prove the
indexed path and the linear path agree entity-for-entity. Its module doc carries a correctness
argument and a measured crossover table.

**No render path called it until 2026-08-24**, and what happened when one did is below. Verified 2026-08-23: outside `visibility/`, every mention of
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
the single biggest term.

**Wired 2026-08-24 as an opt-in resource, and measuring it corrected the paragraph that used to be
here.** That paragraph read "an index in front of it is precisely what removes that walk for culled
objects". True of the camera frustum, false of what the batcher actually queries: the union of the
camera **and the four shadow cascades**, because an off-screen caster still has to reach the shadow
maps. Traced on the same 60 000-cube scene:

| scene | candidates | query |
|---|---|---|
| sun on, all cubes in view | 60 000 | 12.3 ms |
| sun on, all cubes behind the camera | **60 000** | 5.4 ms |
| sun off, all cubes behind the camera | **0** | **0.005 ms** |

With a shadow-casting sun the index culls nothing in either case — the cascades cover the scene, so
their union accepts everything the camera rejected — and then pays to write 60 000 keys into a
`Vec` and sort them. Frame time with the index wired in: **18.84 → 35.75 ms** (all visible) and
**19.73 → 27.23 ms** (all behind the camera). Without cascades the same scene culls completely and
goes **12.72 → 10.14 ms**, a 20 % win.

So `VisibilityIndex` exists as a resource the batcher uses when present, default absent, with the
three conditions written on it: static scene, high cull rate, and cascades that do not cover
everything. `the_spatial_index_renders_the_same_frame_as_the_linear_walk` guards correctness
pixel-for-pixel and goes red on a single dropped entity (227 pixels).

The module's own table puts the crossover for cull *time alone* between 8 k and 32 k meshes
(0.141 µs/entity of walk against 0.022 µs/entity of test at 32 k) — which is the number that made
this look unconditionally worth doing, and it measures the half of the problem the cascades do not
touch.

Its docs are honest about the limits: *"Measure on your own scene before believing any of the
above"*, and it is explicitly **not** an occlusion structure.

### F2. Irradiance volumes — wired 2026-08-24

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

Written, tested, exported, unplugged — until 2026-08-24. `irradiance_volumes` drove it by hand to
show the maths was sound: 48 probes baked from the scene lights, sampled per object, applied to
albedo on the CPU because the pipeline could not. The blend was correct: red-dominant
`(0.935 0.462 0.333)` on the left, blue-dominant `(0.366 0.533 0.902)` on the right.

**Now the pipeline can.** `IrradianceState` uploads the grid — probes as `SHCoeffs::to_gpu_data`'s
existing 28-float layout, so there is one definition of it rather than two — and `irradiance.wgsl`
is a direct port of `ProbeGrid::sample` + `SHCoeffs::evaluate`: same trilinear blend over the same
eight corners, same basis constants, same clamp. Bind group 3 of the deferred lighting pipeline,
which was free. Always bound, because wgpu requires a declared group to be: a renderer with no
baked GI binds one zeroed probe and the shader returns black.

Measured by running both paths over the same grid and the same scene, with only the application
route differing. The red-minus-blue difference across the seven spheres:

| sphere | CPU | GPU |
|---|---|---|
| −3 | +17.6 | +46.9 |
| +0 | −9.0 | −8.1 |
| +3 | −48.9 | −79.7 |

Both fall monotonically, **correlation 0.99887**, scale factor 1.92. The scale difference is not an
error: the CPU path folds irradiance into albedo (0.18 base + irradiance, then multiplied by direct
light) while the GPU path leaves the sphere white and adds the indirect term separately. The
*shape* of the blend — what the probes are for — is the same curve.

`a_baked_probe_grid_reaches_the_frame_with_the_right_colour` guards both halves: the grid changes
the frame (0 pixels when the shader's contribution is removed) and changes it in the direction the
coefficients say (red-baked raises red more than blue — which fails when `evaluate` is replaced by
its magnitude).

Still missing: a **component**. The grid is uploaded by hand; there is no `IrradianceVolume` an
entity carries and no automatic selection between overlapping grids.

### F3. The water material route — wired 2026-08-24

`water.wgsl` was written, `water_pipeline` compiled and was stored in both `renderer.scene` and
`Renderer`, and **no pass ever bound it**. `MaterialType::Water` appeared only in tests, benches and
routing tables; no game code produced it and no draw path selected it. Verified 2026-08-23 in
`transmission`.

The cause was one line of `routing.rs`: `MaterialType::Water` returned **exactly** `Pbr`'s answer —
instance flag `0.0`, `skips_deferred` false. So a water material was a PBR material that happened to
carry a different name, and the pipeline named after it compiled every run for nothing.

**What wiring it required.** `Routing::is_water`, and forward-only. Not for the reason the other
forward types are forward: water displaces its own vertices in the vertex stage, so a G-buffer
filled by a shader that does not would record the *flat plane's* depth and normals while the visible
surface is somewhere else — the difference between the passes is geometry, not shading. That also
takes it out of the z-prepass and both shadow passes, which is the same exemption `unlit` already
carries. `is_water` then joins `BatchKey`, because water and unlit both set `unlit` and an
untextured plane of each shares the cached white-texture bind group: without it the two batches
have an identical key and one of them draws with the other's pipeline.

| | measured |
|---|---|
| same surface, `with_water` vs `with_pbr` at the same albedo/roughness/metallic | **10 568 / 16 384** pixels differ |
| the same comparison with the `Water` arm of `route` put back | **0** — bit-identical frames |
| two seconds of elapsed time, water vs water | **3 730 / 16 384** pixels differ |
| the same comparison with the shader's old clock restored | **0** |

**Wiring it uncovered a second dead thing inside the first.** `water.wgsl` read elapsed time from
`scene.camera_pos.w` — a slot `frame_uniforms.rs` fills with a constant `1.0` and `gpu_types.rs`
documents as unused. Time lives in `cascade_params.z`. So even once bound, the ocean would have been
a *fixed* displaced surface, frozen at t = 1.0, which is entirely plausible in a still frame and
invisible to any single-frame test. Only comparing two times catches it, which is why
`the_water_surface_is_drawn_by_its_own_pipeline_and_moves` renders three frames rather than two.

**And a third thing the route needed and the engine did not have: geometry.** The displacement is
per vertex and `AssetManager::create_plane` is four vertices, so a Gerstner ocean built on it is a
quad with four moving corners. `create_plane_subdivided(size, segments)` is the missing primitive —
same winding, same normals, same world-unit UVs, checked against `plane_data` exactly at
`segments = 1`.

**Limits, written down rather than discovered later.** The water pipeline alpha-blends *and* writes
depth (a surface, not a pane), and it culls back faces — so `Material::with_double_sided` is
load-bearing here, unlike on the transparent pipeline which is `cull_mode: None` already; the
`water_double_sided_pipeline` variant exists for the swimmer looking up. Water casts no shadow,
which also took it out of `classify_visibility_world`'s caster predicate — an off-screen water
surface was being kept and uploaded for a shadow map nothing writes. It does not refract: the
Fresnel term adds a sky reflection and the alpha blends, but the scene behind it is not bent, so the
transmission gap in `demo/src/bin/transmission.rs` is untouched by this.

**The editor path was wired in the same change, and had to be.** `gizmo-studio` picks pipelines in
its own passes and had no water branch — the same shape as its missing custom-material branch — but
here silence was not neutral. Water's instance flag went from `0.0` to `1.0`, and `shader.wgsl`
reads `1.0` as *skip the lights and return the albedo*, so the editor's opaque pass would have
turned the ocean into a flat rectangle: strictly worse than the accidental PBR it had before. It now
has its own water pass, and skips water in the opaque, double-sided, transparent and shadow loops.

That test is worth one note, because the obvious version of it passes while broken. Brightness does
not separate the two states — with the water pass removed the frame's σ was **8.99** against
**11.62** wired, since post-processing and the editor grid put variation in both. Displacement is
the one thing only `water.wgsl` does, so the assertion is about *time*: the same editor scene two
seconds apart differs by **4 633/16 384** pixels wired and **0** without, and a water-free scene is
carried alongside as the control that shows the clock moves nothing by itself.

---

All three are now plugged in — the spatial index opt-in with its conditions, the probe grid into the
deferred lighting pass, and water into the forward pass. They were the largest ratio of existing
work to delivered capability in the engine: all finished, all tested, and none reachable.

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
| `Gizmos::depth_test` selected between two pipelines built from **identical** state — both `CompareFunction::Always` — so the field's own doc ("whether the lines are occluded by the scene") was false. Found 2026-08-24 while building the text overlay pass, which is the same pipeline pattern | The depth-tested one is `LessEqual` now + a golden test that goes red on revert (a line behind a wall leaks 115 px with the old state, 0 with the new). **No picture the engine draws changes today**: `Gizmos` derives `Default` (false) and `gizmo-studio` writes `false` explicitly, so the broken half was never selected |

Reported and now documented rather than changed: `set_update` still replaces the simple scene's
hook (D8), but it warns when it overwrites one and its docs say so, and
`gizmo::simple::simple_scene_update` is public so the four jobs can be kept; `DespawnAfter` is
still inert without `LifetimePlugin` (D9), but its own docs now say so.

Reported, not fixed: deferred `alpha_cutoff` needs the z-prepass to gain a fragment stage; the
editor path never learnt `alpha_cutoff`; `Material::emissive` is dead in the deferred path (D4).
