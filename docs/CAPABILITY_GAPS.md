# Capability gaps — what the engine cannot do yet, measured

**Status:** measured, not estimated. **Written:** 2026-08-23; items closed since are marked in
place, each with the measurement that closed it. **Last pass:** 2026-11-02.

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
"Out of reach" means the engine had no path at all, and those 94 were dominated by the
capabilities in section A.

**Section A as of 2026-11-02:** sprites (A1), text (A2), extrusion (A4), reflection probes (A5),
split screen (A5b), area lights / cookies / soft and contact shadows (A7) and motion blur (A7c) are
closed, occlusion culling (A7b) since 2026-11-05. Open: user shaders stay forward-only (A3), MSAA is
declined by decision (A6), and the ECS items of A8. Planar reflections (A7d) closed 2026-11-06. The
2026-08-22 counts above are that day's inventory and were not re-run.

---

## A. Whole capabilities that do not exist

These are not missing knobs. Each one closes off a whole category of work.

### A1. 2D / sprite pipeline — **the engine draws sprites since 2026-11-01**

Until then there was no sprite, no 2D camera, no 2D transform stack, and every 2D and
UI-adjacent capability had no path. Demos that ran into this: `texture`, `parallel_query` (the
reference version moves 128 sprites; the demo had to become 3D cubes), `removal_detection`.

**What exists now.** `gizmo-renderer::sprite` — an image library (`SpriteImageId`, `Linear` or
`Nearest` sampling) and a pass — plus a `Sprite` component, drawn by **both** hosts through one
shared `gizmo::systems::render::record_sprites`. The "2D camera" turned out not to need a new
type: the existing `ProjectionMode::Orthographic` looking down `-Z` is one, and the "2D transform
stack" is the 3D one restricted to a plane — a world sprite lies in its entity's local XY plane.

| | |
|---|---|
| world space (entity's XY plane, follows `GlobalTransform`) | yes, **depth-tested**, no depth write |
| screen space (window pixels, top-left origin) | yes |
| draw order | `layer`, then back to front (world), then entity order |
| sprite sheets | `uv_rect`, `with_frame` / `set_frame` for a regular grid |
| flip, tint, fractional anchor | yes |
| `IsHidden` (and inherited) | yes |
| batching | one draw call per run of consecutive same-image sprites |
| screen sprites / text / UI backgrounds after tone mapping | yes, since the same day — see below |
| ~~mipmaps, atlas packing~~ | **closed 2026-11-07:** a `SpriteFilter::Linear` image has mipmaps, built weighted by alpha and in linear light; `Renderer::load_sprite_atlas` packs a set into one texture with their edges repeated into a margin, and `Sprite::from_region` draws one. A `Linear` sheet not loaded as an atlas can bleed between cells when shrunk far below its size. |
| sprite-sheet animation (`SpriteAnimation`, advanced by both draw paths) | yes, since 2026-11-01 |
| `Tilemap` (one tileset, one entity, one draw call; both draw paths) | yes, since 2026-11-01 |
| collision from a tilemap (`tilemap_collider`: solid tiles → merged static boxes) | yes, since 2026-11-01 |
| 2D shapes (`Collider::{polygon_2d, polyline_2d, circle_2d, rect_2d}`) on planar bodies (`RigidBody::planar_xy`) | yes, since 2026-11-01 — 3D shapes confined to the plane, one solver |
| one-way (jump-through) platforms (`Collider::one_way`) | yes, since 2026-11-01 |
| a side-on character on one-way platforms (`CharacterController`: jump through, land, `drop_through_timer`) | yes, since 2026-11-01 |
| **a separate 2D solver** | **no, by decision** — see §C9 |

Measured (`sprites_reach_the_frame_in_place_in_order_and_behind_walls`): a world sprite behind a
wall changes **0** pixels, and **784** with `CompareFunction::Always` in the world pipeline; two
overlapping world sprites spawned near-first come out right, and **890** pixels wrong with the
depth key reversed. Demo: `sprite_2d` — parallax, a sprite sheet, collection via `IsHidden`, a HUD.

**The HUD is drawn after tone mapping** (`gizmo-renderer::overlay`, same day). Screen sprites,
screen text and UI backgrounds used to go into the HDR target with the world, so they were
exposed, bloomed, tone-mapped and vignetted: an opaque `(0, 1, 0)` screen sprite read back as
`(0, 219, 0)`. They are now drawn onto the final image inside `Renderer::run_post_processing` —
the one function every output of both hosts goes through — and read back as `(0, 255, 0)`. World
sprites and labels stay in the HDR pass, lit and occluded with the scene.

A correction to the first version of this entry, which blamed tone mapping for the demo's
energy bar coming out mint: most of that was the colour itself. `Sprite::color` is **linear**, and
`(0.35, 0.9, 0.45)` is sRGB `(160, 243, 179)` — mint, whatever the pipeline does with it.

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
| lines | `\n`, and `Text::max_width` wraps where Unicode allows (spaces, hyphens, between ideographs, between Thai, Lao, Khmer and Burmese words), or between characters for a stretch too long to fit — since 2026-11-07 |
| kerning | from the face's `GPOS`, or its `kern` table if it has only that, since 2026-11-06 |
| per-line alignment (`TextAlign`) | yes, since 2026-11-01; by default each line on the side it starts on, since 2026-11-06 |
| shaping and bidi | yes, since 2026-11-06 (`harfrust`, `unicode-bidi`); see below |
| rich text | colour, size and face per span, `Text::rich` and `Span`, since 2026-11-07 |
| vertical text | `Text::in_columns`, `FontLibrary::layout_vertical`: top to bottom, columns from the right, `vert` forms; Latin on its side and Han, kana and Hangul standing, by range, or `.upright()` for all standing. Since 2026-11-07 |
| font fallback | a chain of faces tried per character (`Renderer::add_font_fallback`), since 2026-11-03 |
| finding a font | `Renderer::load_system_font` — `GIZMO_FONT`, then the platform's usual sans faces (`SYSTEM_FONT_PATHS`, per OS), since 2026-11-03; still no bundled face (a licensing call) |

**Shaping and bidi, 2026-11-06.** `text/shape.rs` runs each line through `unicode-bidi`, cuts it
wherever the face or the script changes, and shapes each piece with `harfrust`. Arabic letters join,
Devanagari vowels move and conjuncts form, right-to-left and mixed lines are ordered, marks sit where
`GPOS` puts them, and kerning comes from `GPOS`, which is the only kerning Noto Sans has: the bundled
font was drawn unkerned until now, so existing text changes width slightly.

Latin text gets no ligatures. Noto Sans's `fi` hides the dot of the i, which in Turkish is the
difference between two letters, and it has no Turkish entry a language setting could select. There is
no language setting, so `locl` lookups see a face's defaults. Vertical text is `Text::in_columns`: Latin lies on its side, unless `.upright()`. Bidi and shaping are checked by glyph
ids and positions against subsets of Noto faces in `text/fixtures`, and Arabic, Hebrew, Devanagari, Thai
and Turkish were looked at once in a desktop frame, not in a browser or on a phone.

Looking caught what the glyph tests could not: **a fallback face came out too small.** A size means a
face's height from ascent to descent, Noto Sans Arabic is 2.1 em tall against Noto Sans's 1.4, so at the
same number its letters were two thirds as big beside Latin. `PositionedGlyph::px` now carries the size
that gives a fallback the em of the text's own face.

Measured in release, on one machine: 200 short HUD strings take 1.44 ms the first time and 15 µs once
remembered, a wrapped 510-character paragraph 0.97 ms and 0.5 µs. That is why `FontLibrary` keeps up
to 256 recent layouts per style. **The cost is size:** `demo-web` went from 16.58 to 17.36 MB
(4.94 → 5.18 MB gzipped, +4.9 %) before `wasm-bindgen` and `wasm-opt`.

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

**`gizmo-ui` is connected to it (same day, second commit).** The crate computed boxes and drew
nothing from the day it was written — its own docs said *"this crate emits no vertices and no draw
calls"* and *"`BackgroundColor` is written and never read"*. Both are now false, and the bridge is
small because both halves already spoke the same units: `Node` publishes absolute window-pixel
rects and `TextSpace::Screen` takes them. `BackgroundColor` is painted as a solid quad — through
the *text* pipeline, because the atlas carries a fully-opaque patch, so a background costs no
pipeline and no second buffer — and a `Text` on the same entity is placed in that node's box, its
own anchor choosing the corner. The bridge lives in the facade and cannot live anywhere else:
`gizmo-ui` sits above `gizmo-app`, so the renderer cannot see a `Node`.

Measured: a 60×30 node with a background changes **1 978/16 384** pixels (1 800 is the rectangle,
the rest is bloom) and **0** with the quad not queued; a label authored at (100, 100) and given a
box at (10, 10) changes **221** pixels inside the box and **0** outside it — and drawing at its
authored position instead is what the outside count catches.

**Still missing, and named rather than discovered later.** No default font — a `Text` whose face was
never loaded draws nothing rather than substituting one. Text is drawn into the HDR target before
tone mapping, so it is exposed and bloomed with the rest of the frame: right for a world label, and
the first thing to revisit when there is a post-tonemap pass to hang UI on. **No z-order**: the
paint order was global — every background, then every glyph — so a label was always above every
panel (**closed 2026-11-02:** `ZIndex`, inherited down the tree, paints and hit-tests layer by layer;
only the front layer under the cursor is hovered; an absolutely-positioned root now honours its
`inset`, which it silently ignored, so a dialog can be placed at all); and no widget beyond "a box with a colour and a label". (Keyboard and gamepad focus
exist since 2026-11-02 — `UiNavigation` — and so does clipping: a node with `ClipContent` cuts its
own label and every descendant's background and label to its box, trimming glyph quads and their
texture coordinates on the CPU, so it costs no draw call. Tested on the quad arithmetic, on a
three-glyph string cut to one, and on nested clippers intersecting; `ui_layout`'s fourth button is
cut at its edge. Scrolling followed the same day: `Scroll` offsets a node's children, clamps to
their extent and clips them; the wheel drives it, and a clipped-out button takes no clicks. No
scrollbar is drawn and nothing drags it.) (Click events exist since 2026-11-02:
`ButtonBundle`'s `Clicked` is `true` for the one frame a press on the button is released on it.)

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

**Choosing the forward path for a built-in material: tried and removed, 2026-11-06.** Measured in
`deferred_rendering`, it meant changing the material type. `Material::with_forward` was built and removed
the same day, before any release. A PBR material fits the G-buffer entirely, and TAA, motion blur, SSR,
SSGI and SSAO all need it there, so it had to stay in. The forward pass then only drew a lesser copy over
the deferred result: no decals, no SSAO on its own surface, no PCSS, and its pixels shaded twice. Bringing
those three across would have made it the same picture at twice the cost. A material goes forward when the
G-buffer cannot hold it: blended, unlit, baked-lit, water, custom.

What the attempt uncovered stayed, because the browser, Android, transparent surfaces and the editor
viewport are all drawn by `shader.wgsl`.

**The forward shader reads the whole material, same day.** It read the base colour and nothing else, so
normal, metallic-roughness, emissive and occlusion maps, the UV transform, anisotropy, clear coat and
subsurface never reached a browser, a phone, the editor viewport or a transparent surface. `surface.wgsl`
now evaluates a material for the G-buffer and the forward shader alike, the lighting terms for the extra
lobes are shared functions in `pbr_ext.wgsl`, and on the desktop path the forward shader samples
irradiance volumes too. One panel under a bright sky, tilted and head on, per knob and map: at most 0.8
of 255 apart when measured, under 2 asserted.

**The first version took a phone from 62 fps to 20,** an Adreno 618 drawing a cube and a floor, and only
the phone could say so. Measured by taking pieces out: the extra samples cost nothing visible, the
anisotropy, clear coat and subsurface branches took it from 43 to 20 though no material used them, and
the UV transform and tangent basis per fragment from 69 to 44. Both have a right place to go. The UV
transform and the tangent basis are linear, so they are computed per vertex now, on both paths: 64 fps
with every map still read. The three lobes moved to a second set of lit pipelines on the forward-only
path, which batching picks for a material that uses one; `the_plain_web_shader_leaves_the_lobes_out`
holds that in place, since no test without a phone can measure it. **What is left is a real limit:** a
clear coat on a floor filling the screen reads 20 fps on that phone, because that surface really does
pay for all three.

**A night of features took the same phone back down, and tiers put it right, 2026-11-07.** Coat roughness,
reflectance, LOD crossfades, back faces lit as back faces and fog modes each went into `shader.wgsl`
behind a branch nothing in Nişancı takes, and every gate stayed green: 63.7 fps became 53.6 with the phone
upright and the floor filling the screen, twenty readings each in one sitting, and 102 became 89 on its side. The lit shader has three tiers now on the forward-only path. The plain one is
single-sided, opaque and has no discard, and it is what most of a frame is. `DETAILS` adds what a draw has
to ask for: back faces, cut-outs, crossfades, its own reflectance, the blend modes, the debug views.
`LOBES` adds the three lobes. `routing::needs_details` and `SceneState::lit_pipeline` choose, the desktop
path has everything always, and `the_plain_web_shader_leaves_the_lobes_out_and_the_variant_has_them` holds
it in place. `unlit.wgsl` got the same split with `KEEPS`. Which fog and which `FogMode` stopped being
decided per fragment altogether: `frame_uniforms::frame_fog` settles them once a frame for every path,
and `height_fog` is arithmetic. The plain tier reads 63.1 upright and 103 on its side. **What it costs:** a floor that takes
`DETAILS` read 84 where the plain one read about 97 in that sitting, so a cut-out filling the screen pays
for being one. And an opaque material's texel alpha no longer shows through on these paths: with no
discard it would have vanished while still writing depth, so the plain tier draws it solid, as the
G-buffer always has. A surface that shows through says so with `is_transparent` or `alpha_cutoff`.

**Measuring it taught more than the fix.** `Time::fps` is one frame's `1/dt`, so a few readings are noise:
twenty per build is the least that separates them, and only in one sitting, since the same old build read
102 and then 82 an hour later as the phone warmed. Frames counted at SurfaceFlinger are a different
number, because frames that land in one vsync are dropped there. And less code is not always faster on
this driver: an exponential-only fog for the plain tier, with strictly less to do, read 85 against 103
by both counts, for no reason found, which `height_fog` says next to the code. The driver is a black box, so
measure rather than reason. Not measured: a browser.

**Then the environment, which was most of the frame, 2026-11-08.** With the tiers in, the presets and
`Renderer::sky` were still worked out at every fragment, twice, for the light and for the reflection: a
constant in their place took the same phone from 63 fps to about 140. They depend on a direction, a
roughness and what the frame says about the sky and the sun, so on the forward-only path they are drawn
into a cube now (`env_cube`), 128 texels at its sharpest and mip `k` at roughness `k / 5`, only when
what they depend on changes, and the lit shader samples it (`ENV_CUBE`). One function,
`scene_environment` in `common.wgsl`, is what the bake, the desktop's forward shader and the deferred
pass all call. In one sitting, alternating, twenty readings each: 52.9 fps before and 92.9 after, as the
phone warmed through the afternoon; the floor's colour moved by at most 1 of 255. A sun that moves every
frame draws the cube every frame, 36 small passes, which was not measured. The desktop path keeps
working it out per pixel, next to its reflection probes.

An environment map reaches that path the same day, through the same cube. It goes through the
reflection probes' GGX prefilter at its own resolution, is copied into the cube mip for mip, and is drawn
behind the scene from mip 0. For that the cube's mips go down to 4 texels, as a probe's do, so mip `k`
is roughness `k / 5`; the floor's colour moved by 2 to 3 of 255 with it. `capture_reflection_probes`
returns at once on that path now, since nothing there reads a probe: a probe in a web or phone scene drew
the world six times for nothing.

Unlit and baked-lit surfaces do not fill the G-buffer, so those effects still see through them, and a
moving one gets no motion blur. Baked-lit exists to skip that cost, so changing this is its own
decision.

**The environment presets moved to the forward shader, 2026-11-06 (decided and applied).** They lived
in the deferred shader alone, so `shader.wgsl` lit everything with a fixed warm hemisphere: a red cube
with no sun read red 182 forward and 104 deferred. Both shaders now call `preset_environment` in
`common.wgsl` and use the same environment BRDF, and on one red cube, under all four presets and a set
sky, with and without a sun, metal included, they are under 8 of 255 apart. This **changed the look**
of every transparent surface, of the editor viewport, and of every browser and Android game, all of
which are drawn by that shader.

**Height fog reaches every material path, same day.** It was the deferred shader's alone, so
`Renderer::fog` did nothing on the web or on a phone, and a glass pane, a baked-lit level, water or a
mirror stood outside the fog around them. `height_fog` is in `common.wgsl` now. `shader.wgsl` has one
fragment entry point per blend mode, because fog means something different under each: alpha and opaque
draws fade to its colour, additive ones fade out, multiplied ones fade to white, and premultiplied ones
do both by their alpha. **Baked-lit surfaces are fogged now**, by the default preset's fog too, which
a game with its own lighting will see at a distance until it sets `Renderer::fog`. Unlit materials stay
outside it on purpose: their contract is the exact colour asked for, and editor furniture relies on it.

Particles, smoke and the fluid surface and its spray joined it the same day, in the way each blends: smoke
replaces what it shows by how much of the view it blocks, spray fades out. GPU physics bodies have the same
fog, but nothing in the workspace turns that system on, so no test draws them, and spray is not exercised
either, since a settled pool classifies as all liquid. `fem_render.wgsl` belongs to the `beamng` demo, with
uniforms of its own, and is not an engine path.

**Adding fog to the water found that it was drawn over the whole frame.** The depth pass and the blur had
come to keep the world normal in xyz and the depth in w, and `fluid_composite.wgsl` still read x as the
depth, so no background pixel counted as empty. It also had no depth of its own to test, so water showed
through anything in front of it. It reads w and writes `frag_depth` now, and takes the real normal and view
vector instead of screen-space stand-ins, so **the fluid looks different**: `fluid`, `fluid_rigid` and
`ocean_scene` were not looked at afterwards, because they open a window. The test uses a pool of 10 000 in one
renderer: a new renderer per frame with the default 100 000 particles reset this machine's GPU ring eight times.

Still open: a custom material cannot be drawn deferred.

### A4. Extrusion / 2D-shape-to-mesh machinery — **landed 2026-08-24**

Measured in `3d_shapes`: of the 26 meshes that demo wants, **6** had a ready-made constructor in
`AssetManager`, and **15 of the remaining 20 came from this single gap** (8 extrusions + 7 ring
extrusions). The other five are individually missing: tetrahedron, conical frustum, icosphere,
3D segment, polyline — and they still are.

`asset::primitives::extrude` is the machine. Two entry points, because a prism, a pentagonal prism,
a rounded slab and a torus of square section are not four features but four **outlines**:

- `AssetManager::create_extrusion(outline, depth)` — a closed 2-D outline along Z: two caps and one
  quad per edge.
- `AssetManager::create_sweep(profile, radius, segments)` — the same outline carried around the Y
  axis. A circular profile is a torus, a rectangle a square-section ring.

Outlines live in `extrude::outline` — `circle` (and `circle(r, 5)` *is* the pentagon; there is no
separate regular-polygon builder because there is no separate shape), `ellipse`, `rectangle`,
`stadium`, `star`. A new shape is a function returning `Vec<Vec2>`, not another `create_*`.

**The caps are the half with a decision in it.** Walls are one quad per edge. Caps need the outline
triangulated, and a **fan** — the obvious answer, and what the demo's own hand-rolled pentagon
used — is correct only for a *convex* outline; a star, an L or an arrow fanned from one vertex puts
triangles outside the shape. `triangulate` is ear clipping, so concave outlines come out right.
Tested by **area**, not by triangle count: a wrong triangulation still returns `n − 2` triangles,
and the shoelace sum is what notices that they do not cover the outline. A self-intersecting
outline is refused rather than approximated, and the loop that refuses it is bounded — an ear
clipper with no ear available otherwise spins forever inside a mesh builder.

What it does not do: holes (an annulus is a sweep here, not an extrusion), bevels, self-intersecting
outlines, or smooth normals — every face is flat, matching `create_cube` and the rest of the
directory.

### A5. Reflection probes — **up to four, box parallax, blended — CLOSED 2026-11-01**

Measured in `pbr`: a metallic surface has no environment to reflect. SSR covers what is on
screen (`ssr` measures it working well) and nothing else.

**`ReflectionProbe` (component) + `gizmo_renderer::reflection`.** The game path renders the scene
from the probe's position six times — 90°, square, stopped at the HDR target — copies each into a
`Rgba16Float` cube, and prefilters the mips by roughness (GGX, filtered importance sampling). The
deferred lighting pass's specular term then reads `cube(R, roughness × max_mip)` instead of the
procedural sky (group 3, bindings 2–4, alongside the irradiance volume). Captured on the first frame
and again after `request_capture()`; each face in its own submission; TAA, SSGI and motion-blur
history set aside during it. With no probe the term reads the sky exactly as before.

Tested: the cube is the right way round — six axis directions read their wall, and white quarters
on a side wall and on the ceiling land where they are (the cube convention is left-handed, so a
right-handed render of each face is copied mirrored; dropping the mirror or flipping the ceiling's
up vector fails it); a mirror in a coloured room reflects the wall behind the camera with the probe
(1 200+ of 1 536 pixels) and the sky without; removing the probe restores the no-probe frame to the
pixel. Five mutations red. `pbr` with `GIZMO_PROBE=1` shows the smooth metals reflecting their
neighbours.

**Box-projected parallax, same day:** `ReflectionProbe::influence` (half-extents of a world-aligned
box around the probe, usually the room). The lighting pass traces each reflected ray from the
surface to the box's far wall and looks the cube up toward that point as seen from the probe.
Measured with a mirror 3 m beside the probe, facing a wall half white on its side: by direction
1 008 magenta / 528 white, with the box 1 536 white / 0 magenta. The scene sits off the world
origin, so forgetting to subtract the probe's position or centre the box on it fails the test;
four mutations red.

**Clear coat and the rough mips, same day.** The clear-coat layer reflects the probe too (at its own
0.08 roughness, through the same parallax lookup): a dark, rough cube's face gains 1.79 of magenta
excess with a coat against 0.16 bare — and 0.17 with the coat still reading the sky. The rough mips
are tested: at the roughest mip the +X wall's saturation and the gap to the −X wall both fall; a
prefilter that only copied mip 0 down keeps them (0.71 against 0.74).

**Several probes, blended, same day.** Up to four, in one cube array (the lighting pass has no bind
group left for more textures). Boxed probes are local: each weighs in only inside its box, fading
in over `blend_distance` from the walls, normalised where boxes overlap; whatever they leave is the
background — the first probe without a box, else the sky. Tested with two closed rooms 30 m apart:
each mirror reflects its own room (only-the-first-probe leaves room B's mirror on the sky,
every-probe-reads-slot-0 gives it room A's magenta). And an edit after the capture —
`blend_distance` 0.5 → 20 — reaches the lighting pass without a recapture: room A's magenta excess
39.17 → 23.14. Before that it did not: parameters went up only with a capture, so `intensity`,
`influence` and `blend_distance` changed nothing until the next one. Positions stay where each slot
was captured. Behaviour change: a single boxed probe now leaves surfaces outside its box to the sky
instead of reflecting itself by direction there.

**Transparent surfaces, same day.** The forward pass — transparent and double-sided lit
materials — read neither the probes nor even the procedural sky: its specular environment was a
hemispheric stand-in. It now reads the probes through the same `probe_environment`, now a shared
shader module (`gizmo::reflection_probes`, at `#{REFLECTION_GROUP}`), with the irradiance volume's
group bound at 5 — the native device's sixth, which nothing used. With no probe it still returns the
stand-in, unchanged (since 2026-11-06 the presets instead, see A3). Measured then: a half-transparent metal cube's magenta excess 0.38 → 4.80 with a
probe, 0.39 with the forward shader ignoring it. Native only: the web's four groups are all spent.

**The studio captures too, same day.** `gizmo-studio` calls the game path's
`capture_reflection_probes` before its own passes, so an editor viewport reflects the same probes a
game does. The game path's draw list now skips `EditorOnly` furniture (a no-op in a game, which has
none); without that the grid and light icons would be baked into every probe the editor captured.
Tested in the studio's own pipeline with green furniture covering the wall a mirror faces: magenta
excess 15.33; −14.12 with the furniture skip removed; 1.63 with the studio not capturing.
`render_parity` dropped both the `ReflectionProbe` and the `EditorOnly` exceptions — each type is
now known to both paths.

Not done for probes: real-time recapture (a probe is a level-load cost), probes on the web.

### A5b. Split screen / multi-viewport — **CLOSED 2026-11-01**

**`Camera::viewport` + `Camera::order`, drawn by `render_viewports`** — which `default_render_pass`
calls by itself as soon as any camera has a viewport, so every host that already renders a scene
gets it. Each camera with a viewport draws into its own rectangle (fractions of the target, top-left
origin; `Viewport::{left_half, right_half, top_half, bottom_half, quarter, FULL, new}`), lower
`order` first, and the HUD is drawn once over the whole target. Demo: `split_screen` (B cycles side
by side, stacked, picture-in-picture, single camera).

What the remaining layer turned out to be — not the one this section predicted. Temporal state was
the expected gap; the larger one was that **the draw path uploads camera-dependent data with
`queue.write_buffer`** beyond the uniforms `SceneView` separated: sprite vertices (built with the
camera's view-projection), the culled instance list, the post-process grade. Those writes order
against submission, so two views in one encoder both draw what the last one wrote. Measured: with
both views in one submission, the left view of `split_screen_draws_each_camera_into_its_own_half`
shows **0** red pixels — it draws with the instance list the right view culled. Separating every
such buffer per view would be a rewrite of the draw path; **one submission per view** fixes all of
them at once and costs a submit per camera. The caller's encoder receives only the HUD.

The rest, each pinned by a test that goes red when it is undone:
- **the region**: the last post pass (composite, or FXAA when on) writes only the view's rectangle
  and loads the rest; without it the last view covers the target;
- **the aspect**: the projection is built for the rectangle; with the target's instead, a face-on
  cube measured **22×44** px in a half;
- **once per frame, not per view**: transforms, materials, animation, GPU physics, fluid, particles
  (`prepare_frame`, `step_gpu_simulations`) — per view, a sprite animation stepped to the wrong
  frame; the HUD is drawn once (per view it read **241** for half-white over black, against ~188);
- **TAA and SSGI history per view** (`Renderer::with_view_history`): views 1.. get their own,
  swapped in for their draw. TAA's neighbourhood clamp hides most of the cross-blending a shared
  history causes (the left half's red count 2067 → 1933), so the test pins the cause: view 0's TAA
  steps once per frame (shared, it stepped 8 times in 4 frames).

Not covered: the studio's viewport and Game panel draw one camera (render_parity lists `Viewport`
as game-only, with the reason); the scene is rendered at the renderer's full internal size per view,
so a four-way split costs four full renders.

**History of this section, kept for the measurements:**

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

### A6. No MSAA — **by decision**; CAS sharpening added 2026-11-01

Measured in `anti_aliasing`: every render target in the tree is `sample_count: 1`, and the
deferred G-buffer path is not MSAA-shaped anyway. FXAA and TAA both exist and both measurably work.

**MSAA stays out, deliberately.** On a deferred path it means every G-buffer target multisampled
and lighting run per sample (or edge-detected and run per sample there) — at ×4, four times the
G-buffer memory — to fix the one thing TAA + FXAA already measurably fix: the hardest step falls
191 → 126 with both on. What TAA costs is softness, and that is what was missing.

**CAS — contrast-adaptive sharpening — is now the last post pass** (`gizmo-renderer::cas`,
`renderer.cas`, off by default, `set_sharpness(0..=1)`). Composite → FXAA → CAS → output, so it
sharpens the final image, writes a split-screen view's rectangle, and the HUD is drawn after it.
Measured in `anti_aliasing` (frame 240): TAA + CAS raises the mean gradient TAA lowered (2.60 →
3.24, above the 2.90 of no AA at all) and gives back part of the hardest step (145 → 184) — detail
and stair-stepping are the two ends of one axis, and sharpness picks the point. Tests: a soft edge
is steepened and a flat area left alone, sharpness reaches the shader (each of three shader
mutations red); in a frame, CAS present but disabled changes **0** pixels, enabled it raises the
edge sum 36 830 → 95 540 at sharpness 1; with CAS on, split screen still writes only each view's
rectangle.

**FXAA quality presets, same day:** `FxaaQuality::{Low, Medium, High, Ultra}` through
`FxaaState::set_quality` — the classic presets' contrast thresholds and search lengths; `High`, the
default, is exactly the previous pass. Over stripes of rising edge contrast: 396 / 564 / 672 pixels
changed at Low / High / Ultra.

Still missing: **SMAA**.

### A7. Area lights, light cookies, contact and soft shadows — **CLOSED 2026-11-02**

Sphere, tube and rectangle lights exist, spot lights project cookies, and the sun has soft and
contact shadows. The `area_lights` demo shows all of it. What is left is narrower than the heading
ever was: no cube cookies for point lights, no cookies on web, and rectangle lights cast no
shadows. Each item:

- **Spherical area lights: CLOSED 2026-11-01** — `PointLight::source_radius` / `SpotLight::source_radius`
  (metres, 0 = a point, the default), through `compute_sphere_lighting` in `common.wgsl`, both
  draw paths. Specular takes the representative point (the point on the sphere nearest the
  reflected ray), the lobe is widened and renormalised by (α/α')². Measured, a glossy metal floor
  under a lamp 1 m up: the saturated highlight 186 → 464 px as a 0.3 m sphere while the frame's
  mean holds (88.20 → 87.79); without the renormalisation, 829 px and a mean of 98.25 — energy
  added.
- **Tube lights: CLOSED 2026-11-01** — `PointLight::source_length` (metres along the light's local
  +X, 0 = off, the default), with `source_radius` as the capsule's radius, through
  `compute_tube_lighting`. The representative point is the point on the segment nearest the
  reflected ray, then the sphere step around it; the lobe is renormalised by (α/α_line)·(α/α')².
  Measured, a 1.2 m tube against a 0.3 m sphere over the same glossy floor: the highlight's
  width-per-height 0.72 → 3.36 while the frame's mean holds (85.82 → 87.74); without the line
  renormalisation the mean reads 95.38 — energy added; with the axis never uploaded, 0.30.
- **Rectangle lights: CLOSED 2026-11-01** — `RectLight` (`width` × `height` metres in the
  entity's local XY, lighting the side its forward axis, local −Z, faces), light type 3, through
  `compute_rect_lighting`. **Not LTC**, deliberately: LTC needs two fitted 64×64 tables shipped as
  data, and what it buys over this is an exact *specular* lobe. Here diffuse is exact before horizon
  clipping (Lambert's polygon formula: ½·Σ θ_e n̂_e · N over the four edges) and specular takes the
  representative point (the reflected ray's hit on the light's plane, clamped into the rectangle)
  renormalised across each extent. `intensity` is the on-axis intensity (radiance = intensity /
  area), so a small panel face on lights like a point light of the same intensity. One-sided —
  in the surface terms, in the anisotropic and clear-coat terms (the deferred loop zeroes its
  attenuation behind the panel) and in the volumetric pass, which lit the air behind it until the
  test caught it. Measured over a matte floor from straight above: a 5 cm panel 226.71 under it
  against a point light's 228.45; on edge, the half behind it 61.41 against 61.39 unlit; a 1.5 m
  panel 5258 → 6018 pixels over 150 while the spot under it dims (226.71 → 202.64). Over the glossy
  floor, a 1.2 m × 5 cm panel's highlight 0.80 → 4.44 wide per height, the frame's mean 80.89 →
  83.65. Each was confirmed by breaking it: orientation never uploaded, diffuse from the centre,
  the area halved, the volumetric gate removed, the renormalisation removed — all five fail. Not
  shadow casters; the volumetric pass still scatters every light as a point otherwise (spot cones
  included, as before).
- ~~**No light texture (cookie/gobo).**~~ **CLOSED 2026-11-02:** `SpotLight::cookie` — a name
  registered with `Renderer::insert_light_cookie`, or on native a path loaded on first use. Eight
  256² layers in one array beside the shadow maps; the outer cone fills the image, its up is world
  up seen down the cone (the packed spot has no roll). Measured with a black-left/white-right
  cookie on a spot aimed straight down: the floor's halves 65.56 / 88.48 against 93.82 / 93.80
  without it and 61.39 unlit; a cookie named but never found reads exactly as none. Caught when
  broken: the image mirrored (88.50 / 65.56), the layer off by one (61.39 both — an empty layer is
  black), the cookie sampled but not applied, and never packed. **Spot only, native only** (the web
  forward pipeline has no shadow group); point lights have no cube cookie.
- ~~**No contact shadows.**~~ **CLOSED 2026-11-01:** `DirectionalLight::contact_shadow_length`
  (metres, 0 = off, the default). The deferred lighting pass marches 16 steps from each pixel toward
  the sun in screen space and compares each with the G-buffer surface there. Measured on a 3 cm
  step under a 15° sun — a shadow shorter than the cascades' 4.2 cm depth bias, which they lose:
  363 dark pixels without (the step's own unlit side), 697 with, and not one byte of open floor
  changed. A march pointed away from the sun dives into the floor and self-shadows it everywhere
  (28 416 of 40 960 open-floor bytes); the test pins that, and a length never uploaded. Sun only;
  the forward pass has none.
- ~~**No percentage-closer soft shadows.**~~ **Corrected and closed 2026-11-01.** This line
  described the *forward* path's fixed 3×3 PCF. The deferred path did have PCSS — a blocker search
  and a penumbra estimate — but with the light size a constant (`0.004`) and the filter clamped to
  0.6–1.2 texels, so every edge came out about one texel wide: PCSS in name. Now
  `DirectionalLight::angular_radius` (radians, 0 = that fixed edge, the default) drives a physical
  penumbra `2·d·tan θ`, converted to shadow-map UV through the cascade matrix's row lengths, up to
  128 texels, its 25 taps turned per texel against banding. Measured, a 3 m pillar's 4 m shadow:
  18 / 91 / 196 penumbra pixels at 0 / 0.02 / 0.06 rad — sharp at the base, soft at the tip. The
  first version read the cascade's scale from `m[0][0]` alone, near zero under a tilted sun, and
  softened nothing; the test pins that. The forward pass keeps its fixed PCF.

**A trap worth naming:** `PointLight::radius` is **range**, not source size. Growing it widens and
dims the light rather than softening its shadow — measured scene brightness rising
`136.13 → 139.09 → 143.16` as radius goes `2 → 6 → 20`.

### A7b. Occlusion culling — **landed 2026-11-05, opt-in**

**`Occluder`** on a big opaque entity: what is completely behind it, from the camera, draws nothing.
Each view rasterises the occluders' boxes on the CPU into a 36 864-pixel depth buffer
(`gizmo_renderer::occlusion`), and each object the frustum let through is tested against it. Chosen
over a GPU depth readback because it has no latency (nothing pops in from behind a corner), needs no
readback, and runs the same in the browser and on a phone. It leans one way on both sides: an
occluder writes only cells it wholly covers at its farthest depth there, an object reads every cell
its box touches at its nearest. A hidden caster still casts. The studio path does not cull
(`render_parity` records why). `Renderer::occluded_objects` says how many it hid.

Tested three ways: **the frame is byte-identical** with and without, shadows included (a mutation that
drops hidden casters' shadows changes 117 of 65 536 bytes); against ground truth, random walls and
objects where every ray to what it calls hidden must hit a wall; and the buffer's cells read directly.

**Measured, 1280 × 720, a wall hiding everything else:**

| scene | no occluder | occluder |
|---|---|---|
| 2 026 spheres of 8 064 triangles, not casting | 23.0 ms | **8.0 ms** |
| the same, casting | 34.2 ms | **19.4 ms** |
| 19 882 cubes, not casting | 16.3 ms | **13.4 ms** |
| the same, casting | 17.3 ms | 17.9 ms |

**What it does not buy:** cheap geometry that casts. A hidden caster keeps its instance and its shadow
draws, so all it saves is the camera's passes, which for a cube is less than the 55 ns the test costs.
Each occluder is rasterised every view, so tens, not thousands. Occluders do not hide each other's
shadow casters, and nothing is automatic: a game has to mark them, because the box must be inside the
object and only a box mesh's bounds are.

Measuring it found a defect of its own: an object off screen that casts nothing went on to the
shadow-only list anyway (the classifier asks the material whether it can cast, not the entity whether
it does) and was built and uploaded for a pass that skipped it. Fixed; that alone took a camera turned
away from 19 882 such cubes from 19 888 instances to 7.

The rest of this section is the older question of the per-object walk.

### A7b (history). No occlusion culling — and frustum culling currently buys nothing

There is no hierarchical depth buffer, no query-based culling, no reprojected visibility. Frustum
culling **does** exist (`frustum_cull.rs`, with its own tests, and it culls shadow casters against
the cascades too) and is correct — measured 0 of 60 000 objects passing when the camera looks away.

But it saves no time, for the reason set out in F1: the per-object walk happens before the cull
decision, and it is the whole cost. Adding occlusion culling would land in the same loop and buy
the same nothing. The order of work is the walk first, then a culling strategy — and the tool for
the walk is already in the tree.

Measured in `occlusion_culling`.

**Re-measured 2026-11-03, and the walk was not where the time had gone.** The windowed demo now
read ~31 ms whether the cubes were drawn or culled, against 18.9 in August — a regression. Splitting
the walk with hard-coded early exits put almost nothing before the visibility decision (lookups and
the matrix maths: ~1.7 ms for 60 000) and ~19 ms after it — but in that scene the cubes behind the
camera are inside the sun's cascades, so they are not culled, they are *shadow casters*, and the
19 ms was the per-drawn-object work. With the sun off and everything culled the walk is ~2.6 ms
(43 ns per object). A headless probe timing `default_render_pass`'s CPU side (no compositor, which
at 1 a.m. had started throttling the window to ~48 Hz and flattened every windowed number to
20.8 ms) gave the split for 60 000 drawn cubes:

| change | CPU ms |
|---|---|
| before | 25.0 |
| `MotionHistory` a `Vec` indexed by entity id, not a `HashMap` swept with `retain` every frame (added 2026-11-01 with object motion blur) | 16.4 |
| the batcher stops cloning three `Arc`s per drawn object (only a new batch needs its own) | 15.9 |
| the batch map hashed with FxHash, not SipHash (keys are engine-made pointers and flags) | 14.1 |
| the transform sync rebuilds a `Transform`'s cached matrix only when it moved, without a change mark; propagation's visited set is a bitset, not a `HashSet` | **11.9** |

−52 %, every drawn object, every game. Everything culled: 7.4 → 5.8 ms, the transform pass alone
2.69 → 0.86 ms on a static scene. The sync had been writing every `Transform` through the
change-tracked path every frame — so `Changed<Transform>` matched the whole world, moving or not —
the same mistake already fixed for `GlobalTransform` (`set_global_matrix`), left standing one
component over. `Frustum::intersects_aabb` also stopped testing the n-vertex it
discards (half the plane work, the same decision bit for bit); not a measured win on its own.

### A7c. Motion blur (camera and object) and auto exposure — **CLOSED 2026-11-01**

**Camera motion blur: `PostProcess::motion_blur`** (shutter fraction, 0 = off, the default). The
composite pass reprojects each pixel through `previous_view_proj · inverse(view_proj)` (both
unjittered, per split-screen view, kept in `Renderer::previous_view_proj`) and averages the HDR
frame along that path with 12 samples, capped at 5 % of the screen. Tested: a turning camera loses
edge energy and a half shutter loses less than a full one; a still camera and the first frame are
bit-identical to the shutter closed. Four mutations red. In `motion_blur` it showed something else
first: **that demo's camera had never moved** — the simple scene's fly camera overwrote the pose
`drive` wrote, so the "sixteen times the speed, bit-identical" table below measured a still camera.
Re-measured with the camera actually orbiting: edge energy 1.000 / 0.994 / 0.964 at 1, 4 and 16
turns with the shutter at 0.5, and flat with it at 0.

**Object velocity, same day — the data layer this entry said was closed.** `gizmo_renderer::motion`:
`Renderer::motion_history` keeps each entity's model matrix frame to frame (advanced once per frame,
so a split screen's second view gets the same answer); the game path uploads it in step with the
instance buffer as the G-buffer pass's group 5, with both unjittered view-projections; and the
G-buffer gained RT4, `velocity` (`Rg16Float`, screen-UV motion — object and camera together). Two
cubes, one moved 0.3 m between frames under a still camera: the still one reads exactly 0, the moved
one 0.047 of the screen (= its 6 px at 20 px a metre); the first frame is zero everywhere. Three
mutations red. That took the G-buffer to 36 bytes per sample — WebGPU counts `Rgba8UnormSrgb` as 8,
so it was already at the default 32, not at the 28 its shader comment said — and native devices
now ask for up to 64.

**Object motion blur, same day.** The composite reads that target (group 4, native): geometry blurs
along its G-buffer velocity — object and camera motion — and the sky along the camera's reprojection
as before. Velocity exists only where an object *is*, so the background it just uncovered would read
zero and the smear would stay inside the silhouette; a neighbour-max search (McGuire et al. 2012,
without the tiles: eight taps along both axes out to the blur's reach) carries it out. Measured under
a still camera, one cube moved 0.3 m: its strip's edge energy 1 122 066 shut, 313 386 open (700 320
without the neighbour max), its trail reaching the background beside it (12 of 72 pixels without),
and the still cube's strip byte-identical. Transparent (forward) objects are not in the G-buffer and
take the velocity of what is behind them.

**Auto exposure is built in** (`renderer.auto_exposure`, off by default, native). The loop this entry
asked for, closed without the CPU: `LuminanceReduce` measures, one compute invocation moves the
exposure toward `key / mean` in stops at `1 − e^(−dt·speed)` (separate `adapt_to_bright` /
`adapt_to_dark`, clamped to `min_exposure..=max_exposure`, snapping on the first enabled frame),
and the composite pass reads it from a storage buffer as group 3 — `Camera::exposure` becomes
compensation. Off, group 3 holds exactly 1.0 and frames are bit-identical (0 pixels, tested). Per
split-screen view, like TAA. Measured in `auto_exposure` (`GIZMO_AE_SENSOR=engine`): the two
stations' gap 131.0 → 35.8, frame time 0.52 → 0.54 ms (noise; the CPU-readback loop was 0.82). A
sunlit floor under a ten-times-brighter sun: 232 vs 119 off, 171 vs 135 on. Six mutations red — a
linear instead of log2 blend, swapped speeds, no clamp, no snap, the step never recorded, the
composite ignoring it. Not built: a histogram (it meters the mean), metering masks, physical camera
units.

The history below is how it got here.

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

### A7d. Planar reflections — **landed 2026-11-06**

**`PlanarReflection`** on a flat unlit mesh: each frame the scene is drawn again from the camera
reflected in its plane, with an oblique near plane on the mirror so what stands behind it is clipped
in every pipeline, and copied into the mirror's own texture. The mirror's pipeline looks that up by
screen position with a Schlick rise (`reflectance` 1 is a mirror, 0.02 is water or glass). The capture
is HDR from before the post chain, so it is exposed and tone-mapped once, with the frame.

Tested: the cube behind the camera shows in the mirror on its own side of the picture and what is
behind the mirror does not (red when the oblique plane or the left-right flip is removed); pure maths
that the flipped capture lands exactly on the mirror image and that the near plane lies on the mirror;
a mirror out of view or seen from behind captures nothing; a second mirror is not in the first one's
picture. Looked at too: positions match the arithmetic by hand.

**What it costs:** one more full draw of the scene per mirror in view, shadows included. On a nearly
empty scene at 1280 × 720, 5.4 → 7.7 ms; it scales with the scene. `Renderer::planar.captures` says how
many ran.

**On water, the same day:** `PlanarReflection::water()` on a `Material::with_water` surface. The Gerstner
normal pushes the look-up so the waves break the picture up, falling off with distance, and the shader's
Fresnel mixes the real reflection in where it used to add a constant sky colour (a pillar standing in
the water: 1 401 red pixels below its foot, 185 different two seconds later, none without the
component). Water without the component is the same shader as before, not one byte different. The
water's own texture is not read while it reflects, and seen from underwater it has no capture and draws
plain. The plane is the surface at rest: what touches the water sits slightly off a tall wave.

**What it does not do:** no blur (every mirror is perfectly sharp, roughness is ignored); mirrors do
not appear in each other or in a reflection probe, so no mirror-in-a-mirror; it draws plain in split
screen and in the studio
(`render_parity` records why). The reduced path has it and its shader is validated under the web
schema, but **it has not been run in a browser or on a phone.**

Before this entry, and what the demo `mirror` still records: no reflection-plane concept, no oblique frustum, no stencil buffer, and a render target cannot be
bound back onto a material (same `pub(crate)` wall as C1). What remains is SSR, and
`mirror` puts a number on what it cannot do: a red column facing a smooth metallic floor
contributes +7.91 of red−blue to it while on screen and **exactly nothing** once it moves behind
the camera. Showing what the camera cannot see is the case a mirror exists for.

### A8. No render-world extraction, no runtime component definition, no system hot-patching

Measured in `ecs_absent_apis`:

- **No second world.** Rendering reads game components directly (measured: `set_render` sees 4
  live components with no extraction step). Cheaper and simpler; the cost is that render cannot
  run in parallel with game logic.
- **Component registration is typed.** `register_component_type::<T>()` only — the registered-type
  count moves `0 → 1 → 2` through typed registration alone. Raw access exists
  (`get_component_ptr` + `TypeId` read `Some(100)`), so *access* is dynamic but *definition* is not.
- ~~**No immutable components.**~~ **Closed 2026-11-07:** `impl_component!(T; immutable)`, and `&mut T` in a query is
  a compile error.
- **No system hot-patching.** Shaders *are* hot-reloadable (`demo/assets/shaders/*.wgsl`); Rust
  system bodies are not.

---

## B. Effects that exist but cannot be shaped

The effect is there and — where it was measured — it is correct. What is missing is the ability to
author it. The middle column is what a full implementation offers.

| Effect | Full capability | Gizmo | Measured in |
|---|---|---|---|
| Screen-space reflections | 6 knobs | **8 shaping fields since 2026-08-24** — `SsrParams::{roughness_cutoff, fade_start, fade_end, step_size, max_steps, thickness, start_offset, edge_fade}`, one 32-byte uniform per frame, defaults pixel-identical to the shader literals they replaced. Plus `enabled` (2026-08-23). **What it finds replaces the environment since 2026-11-07**, weighted as the lighting pass weights it and in a metal's colour: a red cube on a polished metal floor under a blue sky moves red up 150 and blue down 207; it was added on top at a dielectric's Fresnel, so metals barely showed it. Measured: five of them move 11–19 % of the frame; `max_steps` moves nothing on that scene because the reflection is found on the first step, which is a property of the fixture rather than of the field. Still absent, and never written: step exponent, binary refinement, secant search | `ssr` |
| Tone mapping | 7 curves + dither | **6 curves** — `TonemapCurve::{None, Reinhard, ReinhardExtended, Aces}` since 2026-08-24, and **`AgX`** and **`PbrNeutral`** (Khronos) since 2026-11-02, via `Renderer::tonemap_curve`, **default still ACES** and locked as such in three places, because every existing scene was authored under it. Measured on one scene: the first three differ from ACES by 4.91 / 7.20 / 5.41 % of pixels; on a bright frame the means are ACES 183.2, None 185.1, Reinhard 164.8, AgX 174.5, PBR Neutral 175.1, and AgX vs Neutral differ on 16368 of 16384 pixels; routing AgX to ACES fails the test. **Deband dither since 2026-11-02** (`Renderer::deband_dither`, off by default): half an 8-bit step of noise in the display encoding — on a dim gradient 80.3 % of neighbouring pixels were equal, 59.3 % with it, the mean unmoved (56.143 → 56.145). `TonemapCurve::ReinhardLuminance` since 2026-11-07, so seven. **The browser and a phone had none of it until 2026-11-08:** their composite applied fixed ACES to the frame as it came, so the curve, `Camera::exposure`, the dither and the vignette did nothing there. The curves, the vignette and the dither are one file now, `post_common.wgsl`, in front of both composites, and the reduced one takes the curve and whether there is a dither, vignette or grain as pipeline constants, built again when they change (`post_process::sync_composite`): a phone compiles one curve and nothing it does not use. A GPU test runs that composite on the desktop against each formula; taking the exposure or the vignette out of it fails the test. The vignette's 0 is no vignette now, as `PostProcess::vignette` always said: the shader's formula took the corners to 73 % at 0. **Looks change:** at the renderer's default 0.25 a desktop frame's corners are lighter (about 75 % where they were 52 %), and a browser or phone frame gains the vignette it lacked. **What it costs a phone:** Nişancı upright, twenty readings each, alternating: 113.5 and 112.4 fps before, 106.8 and 107.9 after, so 3 to 5 % at the defaults; with the vignette at 0, 111.4 and 107.8 against 108.8 and 107.3 with it, so the vignette is about 1.5 % of that and inside the noise. A game that wants the frames back sets `Renderer::vignette_intensity` to 0, which compiles it out. SBDT, Tony McMapface and Blender Filmic are each defined by a look-up table, and **since 2026-11-08 the engine reads one** (`color_lut::ColorLut`, from a `.cube` file, through `Renderer::set_color_lut`): as a grade after the curve (`LutDomain::Grade`, what a grading tool writes) or in the curve's place, fed `x / (1 + x)` (`LutDomain::Reinhard`, how Tony McMapface and SBDT are made) or `log2` between two exposures (`LutDomain::Log2`, how Filmic and AgX looks are made), on both composites through one shared function, the forward-only one taking the mode as a pipeline constant. A table that swaps red and blue turns a red wall (225, 99, 59) blue; an identity grade moves the frame by at most 2 of 255; an identity table fed `x / (1 + x)` gives what the Reinhard curve gives; taking the grade out of the shader fails the test. The tables themselves are not shipped: each has its own licence. On the phone the table costs nothing while none is set (107.1 and 108.9 fps before, 108.6 and 107.0 after) | `tonemapping` |
| Bloom | 7 knobs | **6** — `intensity`, `threshold`, since 2026-11-07 `Renderer::bloom_softness` and `bloom_energy_conserving`, and since 2026-11-02 `Renderer::bloom_radius` (tap spacing, 1 = the old blur) and `Renderer::bloom_iterations` (rounds of the separable blur, 1–8). Defaults write exactly the old steps. The last two are renderer-wide rather than per-camera `PostProcess` fields, since the blur passes read their own buffers. Measured, a ring 8–24 px around a small saturated cube (horizontal / vertical arm): no bloom 61.29 / 61.31, default 67.99 / 67.99, radius 3 132.80 / 132.82, four rounds 101.02 / 101.03; either axis's step left unwritten, or the rounds ignored, fails the test. Still absent: a mip-chain (dual-filter) bloom, a low-frequency boost, per-camera radius | `bloom_3d` |
| Depth of field | 6 optical knobs | **Since 2026-11-07, all but the bokeh mode** — `PostProcess::with_lens(focus, f_stops, max_blur)` is a thin lens: the focal length comes from `dof_sensor_height` (Super 35 unless set) and the camera's field of view, and a 50 mm f/2 at 5 m gives a radius of 3.65 texels at 10 m on 1080 lines, worked by hand in the test. `dof_max_depth` limits what a sky counts as. The focus band stays beside it. Blur radius is in texels of the frame (it was fractions of a 1920×1080 one), 24 taps on the golden angle, and a sharp thing no longer glows on the blurred background behind it (11944 → 2315 in the frame's middle third). A blurred foreground spreads over what is in focus behind it, which stays sharp: each pixel looks for it in 12 depth taps, with no pass of its own, at that cost per pixel while depth of field is on | `depth_of_field` |
| SSAO | 5 quality levels + temporal denoise | **3** — `strength`, and since 2026-11-02 the sampling radius and depth bias (`SsaoState::set_shape(queue, radius, bias)`, defaults `SSAO_DEFAULT_RADIUS` 0.5 m / `SSAO_DEFAULT_BIAS` 0.015, the shader's old literals — setting them explicitly changes no byte). Measured, a cube on a floor, darkening summed over a band of floor below it: 237 at 0.5 m, 2584 at 2 m (rows only the wide radius reaches: 0 → 842); the shader keeping its literal fails. **Closed 2026-11-07:** a flat floor no longer darkens at all (0 pixels moved of 16384): a sample must find its occluder above the surface's plane, off-screen samples leave the denominator, and the blur's loads past the frame are clamped, which were a band 15 levels dark along the top and left. `SsaoState::set_quality(SsaoQuality::{Low, Medium, High, Ultra})`, 8 to 64 samples: away from 64, 8 reads 4472, 16 reads 2630, 32 reads 2567. No temporal stage: the noise tile is two half-resolution pixels and the blur is five, so the noise is gone before the frame sees it | `ssao` |
| Anti-aliasing | 5 techniques | **2** (FXAA, TAA) | `anti_aliasing` |
| Shadow bias | 2 knobs | **2 since 2026-11-02** — `ShadowSettings::with_bias(depth, normal)`, multipliers on each path's own calibration (depth 4.2 cm deferred/forward and 16 cm baked-lit, normal offset 2 cascade texels), so 1/1 is bit-identical to before. Carried in `SceneUniforms::shadow_bias`, appended (fixed part now 608 B). Measured, a cube 10 cm over a floor: 76 shadow pixels by default, 52 with ×50 depth, 12 with ×50 normal; either multiplier ignored fails. A zero bias shows no acne in that scene — the shadow pass culls front faces, the real acne cure — and with the cube 40 cm up even ×50 depth changed nothing (the slope floor makes it 21 cm) | `shadow_biases` |
| Distance fog | colour / falloff / mode | **4 baked presets** + a blend factor, and since 2026-11-02 **`Renderer::fog`** (`Fog::new(color, density).with_height(falloff, base)`), which replaces the preset's fog outright — colour, density, height falloff, base height — so fog no longer has to come with a preset's sky and mood. Two vec4s appended to `SceneUniforms` (fixed part 560 → 592 B, light ceiling 1015 → 1014). Measured, a white wall 40 m off (centre RGB): preset 213 214 215, red fog at 0.1 → 232 44 45, density 0 → 225 225 226; the override ignored fails. **Found while building it:** the height integral divided by `falloff × dir_y` and only guarded `dir_y`, so a fog that does not thin with height gave NaN and vanished; the guard is on the product now (blue fog with falloff 0 → 99 100 231). `FogMode` since 2026-11-07: exponential, squared, linear (a wall at 40 m in black fog, thin 184 and squared 198, thick 76 and squared 11, a ramp 206, 165, 0 as it closes). The sky is not fogged, by the same choice as Bevy's: it has no distance | `fog` |
| Sky / ambient light | sky colours, ambient colour and intensity, from code | **4 baked presets** + a blend, and since 2026-11-03 **`Renderer::sky`** (`Sky::gradient(zenith, horizon, ground)`, `Sky::uniform`, `with_intensity`), which replaces the preset's sky as what lights the scene — the ambient term around each normal and the reflection along each mirror ray — on **both** shading paths (the forward one had its own baked hemisphere, a sunset, which it now also gives up). Not the background: that is `Camera::clear_color`. Three vec4s appended to `SceneUniforms` (608 → 656 B, light ceiling 1014 → 1013). Measured, a matte and a metal cube with no light at all: preset grey (103 106 113 / 83 86 91), green sky green (31 230 38 / 6 224 7; forward 0 229 0 / 0 251 0), intensity 0 dark; either shader's branch removed fails. **An image since 2026-11-07:** `Renderer::set_environment_map`, an equirectangular HDR that lights the scene, is reflected and stands behind it, in the probes' cube array. A panorama red to +X and blue to −X: behind the scene left 0 0 236, right 234 0 0; a matte ball 129 24 185 and 184 24 132 on its two sides; a mirror ball 3 0 243 and 243 0 0. **On the browser and on a phone since 2026-11-08**, through the cube that path reads its sky from (`env_cube`): the probes' prefilter at the map's own resolution, copied in mip for mip, behind the scene from mip 0. Nişancı with a panorama orange above and green below lit its floor warm and stood green behind it, at 92 fps against 94 without. Probes themselves are still desktop only | `kosucu` |
| Cascaded shadows | configurable | **Reach and split configurable since 2026-11-02** — `Renderer::shadow_settings` (`ShadowSettings::{distance, lambda}`, defaults the old 100 m and λ 0.75), read by both paths through the shared setup; the shaders fade over the last 15 % of whatever range results, since they read it from the splits. Tested through `collect_scene_setup` in both paths: 30 m ends the range at 30 m, λ 0 moves the first split out, and the defaults are bit-identical to the old constants; a setup that ignores the setting fails. **Still constants:** 4 cascades and 3072² texels | `shadow_biases` |
| Level of detail | per-level distances + crossfade | **All of it** — `LodGroup` has had per-level distances on both draw loops, and the row said "one level, one rule" because it measured only the automatic LOD1; `LodGroup::with_crossfade` (2026-11-07) dissolves over a band, by a screen-space pattern two levels split between them, including out to nothing past the last. The automatic LOD1 (`distance > radius × 15`, `LOD_MIN_CORNERS`, generated from a mesh's own indices) stays as it was: it switches hard | `visibility_range` |
| Wireframe | global flag + default colour + per-entity include/exclude/colour | **All of it since 2026-11-07** — `Wireframe`, `NoWireframe`, `WireframeColor` per entity (measured on two spheres: marked only (480, 0) green pixels left/right; global with one out (0, 516); two colours 478 green, 457 red), and the studio viewport honours them and `global`. Before: **2** — `WireframeConfig::global`, all or nothing, and since 2026-11-02 `WireframeConfig::color` (one colour for every line; the wireframe pipeline compiles `shader.wgsl` with `WIREFRAME`, which returns the colour packed into `SceneUniforms::wireframe_color`). Measured on a pale sphere: 0 strongly green pixels with the lit colour, 594 with green; the shader branch removed reads 0.  Before the colour: Measured consequence: the overlay is drawn in a fixed light colour, so on a light material it is nearly invisible (edge energy only ×1.20, 1.17 % of pixels). Also: drawn in the forward pass, skybox/backdrop excluded by design, and **inert on wasm** (`PolygonMode::Line` unsupported there) | `wireframe` |
| Clearcoat | strength + layer roughness + 3 textures | **2 since 2026-11-07** — `clear_coat`, `clear_coat_roughness`, 32 steps each in one half float in the G-buffer. The "near-binary" reading was the tone curve: a highlight at roughness 0.08 saturates it. Measured dim, on a dark surface: 0.2 adds 72, 0.8 adds 217, 0.8 at roughness 0.6 adds 53, forward and deferred within 3. No textures: nothing in the material bind group holds them, and glTF's clearcoat extension is not in the `gltf` crate | `clearcoat` |
| Shadow caster / receiver | per-object cast **and** receive toggles | **both since 2026-11-02**: casting as `ShadowCasting::{On, Off, Only}` — `Only` casts without being drawn, which a plain cast toggle cannot express — and receiving as `MeshRenderer::receives_shadows` (`with_receives_shadows(false)`, an inspector checkbox), which skips the sun's cascades, contact shadows and the point-light cube on that surface. It rides the instance's `emissive.w` into RT5's alpha for the deferred pass; the forward and baked-lit shaders read it directly. Measured, a cube over a floor: 171 shadow pixels on an opaque floor and 174 on a transparent one disappear when it refuses; the lighting pass ignoring RT5's alpha, the forward shader ignoring the flag, and the batcher never writing it each fail. Casting fractions measured earlier: 1.06 % / 2.07 % / 4.23 % | `shadow_caster_receiver` |
| Baked lighting | lightmap texture + second UV set | **vertex colour** via `MaterialType::BakedLit` — resolution is the mesh's tessellation, not a texture's. Still receives the sun's cascades, so mixed lighting works. Measured: the blue-minus-red balance flips sign, `+1.35` under `Pbr` and `−4.74` under `BakedLit`, because `BakedLit` does not see point lights | `lightmaps` |
| Irradiance volumes | component + GPU voxel sampling | **wired 2026-08-24** — `deferred_lighting.wgsl` imports `gi_sample` from `irradiance.wgsl` and the baked grid reaches the lighting pass; F2 has the correlation against the CPU path. **Authored since 2026-11-02:** `IrradianceVolume::new(min, max, resolution).with_ambient(..)`, a component (saved in scenes) that both paths keep baked from the scene's directional and point lights — rebaked and uploaded only when a fingerprint of the volume and the lights changes. Tested: no volume uploads nothing; a 4×3×4 volume uploads 48 probes and its red ambient moves the frame's mean red−blue −12.26 → 30.40; a second still frame keeps the fingerprint; moving a light changes it; dropping the sync call fails. The bake is direct light only (no occlusion, no bounce) and spot lights are not in it | `irradiance_volumes` |
| Volumetric / god rays | scattering, steps, phase, distance | **All six, since 2026-08-23** — `VolumetricParams::{phase_g, steps, max_distance, sun_scatter, bulb_scatter, shadow_bias}`, one 32-byte uniform uploaded per frame, and the defaults are pixel-identical to the shader literals they replaced. Plus `enabled`. Measured working: 19.55 % of pixels, max 134 | `volumetric_fog` |
| Blend modes | alpha / additive / multiply / premultiplied | **All four since 2026-11-02** — `BlendMode::{Alpha, Additive, Multiply, Premultiplied}` via `Material::with_blend_mode` (and the inspector), saved in `MaterialDesc`. Any mode but `Alpha` makes the material transparent. PBR and unlit routes, both draw paths; baked-lit, water, sky and backdrop keep their own blending. `Multiply` ignores alpha (fixed-function blending cannot weigh a product without a premultiplied output) — fade it toward white. Measured, a half-transparent grey unlit panel over a blue-grey wall (R, G, B at the centre): wall 148.8 186.4 214.1, alpha 197.5 201.8 208.1, additive 236.0 240.1 244.5, multiply 59.1 90.7 128.0, premultiplied 238.7 239.6 241.0; lit additive 241.5 243.0 246.4 (191.0 201.8 220.6 before the forward shader honoured the panel's emissive, the same day). The mode dropped from the batch, the lit variant never chosen, multiply built as additive, and the editor's old single-pipeline transparent loop all fail a test. Fixed on the way: the editor shaded **unlit** transparent batches with the lit shader; it uses the unlit pipeline now, as the game does | `blend_modes` |
| Specular tint | a colour F0 | **The strength since 2026-11-07, not the colour.** `Material::reflectance` / `with_ior` set a non-metal's F0 on both paths and in SSR (none 126, default 176, diamond 302, three channels added, on a dark surface; forward and deferred within 1). A tinted F0 on a non-metal needs three numbers and the G-buffer has one spare: RT5's alpha now holds this, RT4 is two channels, RT2 and RT3 are full. A seventh target for a rare look is separate work and is not done; `metallic` is the coloured F0 we have. A glTF `specularColorFactor` contributes its luminance | `transmission` |
| Transmission / thickness | per-material | **Since 2026-11-07** — `Material::with_transmission`, `with_thickness`, on both paths and in the studio viewport (`crate::transmission`). Screen-space, as Bevy's is at one step: only what is on screen shows through, one pane does not show through another, blended surfaces and particles are drawn after the copy. **Attenuation colour over distance since 2026-11-08** (`Material::with_attenuation(color, distance)`, glTF's `KHR_materials_volume`): through its thickness the light behind keeps `color ^ (thickness / distance)`, worked out per instance on the CPU and carried at 8 bits a channel in the instance's route flag below -0.5, where every reader of the route sees PBR; the per-material uniform is shared by every material on one texture, and growing `InstanceRaw` would have silently broken every custom shader that declares it. A cube behind a pane keeping a quarter of the red per half metre reads 243.5, 215.7 and 137.4 through none, half a metre and one; taking the factor out of the shader fails the test | `transmission` |

Two measured caveats worth keeping:

- ~~**Bloom's radius is not adjustable and is tight.**~~ **Adjustable since 2026-11-02**
  (`bloom_radius`, `bloom_iterations` — see the row above). `bloom_3d` had measured the halo
  contributing `+18.9` in the 60–80 px band and **exactly 0.00 beyond 80 px** at every intensity;
  the default is still that blur, and the default's taps are one full-frame texel apart, so on a
  small source its halo barely leaves the source (a ring at 25 px read the same with bloom on and
  off).
- **The fog knob is not a fog knob.** `environment_preset` also selects the sky and the overall
  lighting mood, so fog cannot be changed independently of the scene's look.

---

## C. ECS and API gaps

Small individually; together they are what makes an idiomatic ECS snippet fail to compile.

### C0. The extension points are sealed

Measured in `ecs_extension_points`. Four traits are sealed and cannot be implemented downstream:

| Trait | Consequence |
|---|---|
| `SystemParam` | **A primitive parameter can be written downstream since 2026-11-08**: the seal is gone and the trait is `unsafe trait SystemParam`, as in Bevy, with the contract on it. `get_access_info` has to declare everything `fetch` and its value can touch, because an undeclared read can be co-scheduled with a `query_unchecked` writer, and that is a data race even through the safe `World::query`. `crates/gizmo-core/tests/downstream_param.rs`, a separate crate, counts entities through a parameter of its own in a schedule, and its declared read reaches the system. Before: a *primitive* parameter could not be written **downstream**. **Nine** impls (2026-08-24; this line said six and omitted `Commands`, `EventReader` and `EventWriter`, then gained `Local`): `Res<T>`, `ResMut<T>`, `f32` (dt), `Query<Q>`, `Option<P>`, `Commands`, `EventReader<T>`, `EventWriter<T>`, `Local<T>` — plus anything `system_param!` declares, which is a **composite**, so grouping existing parameters no longer needs the seal opened. `Local` is the first primitive added since the seal was written, and it needed the trait to grow a `type State`: `fetch` had no per-system identity, so there was nowhere for a value to live |
| `WorldQuery` | ~~No derived query operand~~ **Closed 2026-11-07:** `query_data!` declares a named one, forwarding every method to the tuple of its fields, as `system_param!` does for parameters. A *primitive* operand still cannot be written downstream, and that is separate work: `WorldQuery`'s methods index an archetype's raw columns, so opening it means making the storage layout public API, not removing a seal |
| `ReadOnlyQuery` | — |
| `FetchComponent` | No custom fetch |

~~`run_system`~~ **closed 2026-11-05:** `World::run_system(system)` runs a system once, now, inside
the frame's change window, its `Commands` applied before it returns. ~~`pipe`~~ **closed 2026-11-07:**
`a.pipe(b)` hands what `a` returns to `b`'s first parameter, `In<T>`. What used to be called `pipe`, which handed
nothing over, is `then`. The seal is a
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
| ~~`Local<T>` system param~~ | **CLOSED 2026-08-24.** Per-system state had to be a world resource, which made it visible to every other system *and to the scheduler*: two systems each keeping their own tally in a `ResMut<T>` declare a write of the same type, so the batcher keeps them apart and they never run in parallel. `Local<T>` declares nothing — measured: two systems holding a `Local<u32>` land in **1** batch, the same two written with a resource land in **2**. The price §E under-counted was real and was paid: `SystemParam` grew a `type State`, `fetch` a third argument, and that reached nine impls, the `system_param!` macro (composites hold their fields' states as a tuple, so a composite may contain a `Local`), the 1..12 `IntoSystem` macros, the 1..6 `IntoCondition` macros, and four call sites outside `gizmo-core`. `fetch_stateless` is the provided method a caller with no state uses, because `&'w mut ()` cannot be conjured from a temporary. One API consequence: a **`pub`** composite now needs its field types at least as public as itself, since `State` names them | `ecs_guide` |
| ~~`On<Remove, T>` / `On<Replace, T>` dispatch~~ | **CLOSED 2026-08-24.** `add_observer` is generic over the marker and the marker comes from the closure's own signature — `add_observer(\|e: On<Remove, Hp>\| …)` — so the ordinary call needs no turbofish and the phase is readable at the registration. The three now **partition** a component's life: a write fires `Insert` **xor** `Replace`, never both and never neither; a detachment fires `Remove`. `Replace` needed a fourth hook list rather than a reuse of `on_set`, because `on_set` fires on the first write too and a hook that is handed only an entity cannot tell the cases apart — the dispatcher can, so it does it once instead of every caller keeping a per-entity ledger. **The gap understated itself in one place:** giving `Remove` a dispatch path turned an internal quirk into a broken promise, because `remove_bundle` fired `on_remove` for a bundle's sparse components and detached its Table components in silence — the same component, removed two ways, answering differently. Fixed in the same change and re-measured in the demo, whose header table had recorded the silence as a fact: five removal paths, **five** hook calls now, four before | `removal_detection`, `observers` |
| ~~Global (non-entity) observers~~ | **CLOSED 2026-08-24.** `World::observe_global` / `trigger_global`, keyed only by the event's own type — the third door, next to `add_observer`'s component type and `observe`'s target entity. Synchronous: the listeners run inside the `trigger_global` call, in registration order, and see the world before it returns. **No `Clone` bound**, unlike the entity path — there is no hierarchy walk, so no second recipient needs a second copy, and the listener takes the event by reference; an event carrying a `String` costs nothing to publish. `Events<T>` remains the right choice when the reaction wants to be a scheduled system, and the docs say so rather than leaving it implied. Demonstrated with two *independent* subscribers that do not know about each other and a publisher that knows about neither: `observers` announces `ChainSettled` when its cascade stops, measured at "26 mines · 8 rings" queued and "26 mines · 1 ring" immediate | `observers` |
| ~~Optional components in a query~~ | **CLOSED 2026-11-02:** `Option<&T>` / `Option<&mut T>` as query operands — every entity the rest of the query matches, `Some` where `T` is present. Before, "the bonus if there is one" was two queries joined by id, or a `borrow::<T>()` a scheduled system cannot make. The access is declared whether or not any entity has `T`, so `Option<Mut<T>>` serialises like a `Mut<T>` writer and `(Mut<T>, Option<&T>)` is the aliasing panic; over sparse storage the per-row presence test is the same one `&T` uses, and chunk iteration is refused. Tested for table and sparse presence, change marking, aliasing and the declared write; removing the presence test or the declaration fails them; Miri-clean (Tree Borrows). `gizmo-ui`'s click signal is its first engine user | `ui_layout` |
| ~~Run conditions on observers~~ | **CLOSED 2026-11-02:** `World::add_observer_if(condition, system)`, with `condition` in the `run_if` shape (`FnMut(&World) -> bool`), so `in_state(..)` and the rest plug in unchanged; asked at every firing, against the world after the write. Only `add_observer` needed it — `observe` and `observe_global` hand their listener `&mut World` already. Tested with `in_state`: quiet in the menu, fires while playing; dropping the gate fails | `observers` |
| ~~A world handle inside observers/hooks~~ | **CLOSED 2026-08-24 — and the row was half wrong when written.** All five *hook* types have taken a `&mut World` all along (`AddHook`, `SetHook`, `ReplaceHook`, `RemoveHook`, `DespawnHook`); what had no world was the *observer* end, and only one of its two forms had no layer to drop down to: a lifecycle observer's `On` is built by a hook that holds a world, so `register_on_add` and its siblings were always the route, but `World::observe`'s listener was handed the `On` and nothing else, with nothing underneath it. It now takes `(&mut World, On<E>)`. The measured consequence is the one the row named, inverted: `observers` runs the same chain both ways and reaches the **same 26 of 70 mines with the same 26 bubbled events** — in **8 frames** through the captured queue, and in **1** when the listener drives it, calling `trigger` from inside the dispatch. Re-entrancy terminates structurally: the notified entity's listeners are owned by the dispatch for the length of the call, so an event sent back at its own target finds nothing, while another entity's listeners run nested. One measurement moved while writing this: the immediate path first reported **73** bubbled rather than 26, because it collected its neighbour list as a snapshot and triggered entries the cascade had already reached — an event bubbles even when its target's listener returns early | `observers` |
| ~~`Visibility` component~~ | **THE ENTRY WAS WRONG, and stale when written.** It said hiding meant `ShadowCasting::Only`, "which still casts a shadow". The engine's hide is `gizmo_core::component::IsHidden`, both draw loops have honoured it since 2026-08-19, and it is a *full* hide — the entity is dropped before it becomes a draw item. Measured 2026-08-24: an `IsHidden` cube gives a frame **0 pixels** different from having no cube in the world, while `ShadowCasting::Only` leaves its shadow, which is what that variant is for. What WAS missing is **inheritance**, and that is the part nobody had written down: both loops asked `contains(entity)` per entity, so hiding a parent left its children drawn — **1 886 of a pair's 2 946** pixels survived. `collect_hidden` walks `Children` down from every marked entity now, shared by both paths, `O(hidden subtree)` so a frame with nothing hidden pays nothing | `infinite_grid` |
| ~~`Transform::forward/right/up/looking_at`~~ | **CLOSED 2026-08-23** — added with two tests (axes orthonormal and right-handed across a full turn of yaw; `looking_at` aims and survives degenerate input). `looking_at` returns the rotation rather than applying it, so it can be blended | `transform` |
| ~~`.chain()`~~ | **CLOSED 2026-11-02.** `(a, b, c).chain()` (`IntoSystemChain`, 2–8 systems, bare or configured, in the prelude) gives each a generated label and an `after` on the previous one; `Schedule::add_chain`, `App::add_update_chain` / `add_system_chain`, and `SystemChain::in_phase`. A system keeps its own labels, so others can still order against it. Generated labels cannot be misspelt — the hand-written per-edge form still can, and an unmatched constraint still **warns and is dropped** rather than failing. Measured: 8 systems touching nothing share **1** batch loose and get **8** chained, in the listed order on every one of 50 runs (dropping the links fails the test); `schedule_internals`, whose chain mode now uses it, shows `ABCDEFGH ×650`, 0 flips, against **78 distinct orderings** in 600 frames unconstrained | `transform`, `schedule_internals` |
| ~~`or_else` for run conditions~~ | **CLOSED 2026-08-24** — `or`, `and` and `not`, on the *typed* path, so the combined condition keeps its access declaration and the system stays parallelisable. That was the whole cost: an OR had to go in one `run_if(\|world\| …)` closure, and an opaque predicate marks the system exclusive. Measured in the test: a typed OR and an unrelated system land in **1** batch, the same OR as a closure gives **2**. Uncovered a second thing — the typed path could not be *called* with a parameter closure at all (`E0283`, and the annotation is unspellable), because `IntoCondition` was missing the second `FnMut` bound `IntoSystem` has always carried. A function *returning* a condition must say `impl IntoCondition<..>`, not `impl FnMut(..)`, for the same reason | `run_conditions` |
| ~~`OnEnter` / `OnExit` state schedules~~ | **CLOSED 2026-11-02 — as hooks, not schedules.** `App::add_state(initial)` (or `gizmo_core::state::add_state`) makes a driven machine: the app loop (`frame::run_fixed_and_update`, both runtimes) runs `state::run_state_drivers` once a frame **before** the fixed and update schedules, so every system of a frame sees one state. `App::on_enter` / `on_exit` take `FnMut(&mut World)` — the world mutably, which no scheduled system can have (C2). **The initial state is reported** on the first frame, which closes the trap this row measured (0 entities for 120 frames). One switch a frame: a `set` inside a hook lands next frame. Tests: the order exit → scoped despawn → enter, the initial entry exactly once, a hook queueing a switch and adding hooks while the hooks are out of the world, and an update system seeing the new state (and the hook already run) on the frame of the switch. A bare `State` is still undriven, as before; nothing changes for code that does not call `add_state` | `states`, `state_scoped` |
| ~~`DespawnOnExit` / `DespawnOnEnter`~~ | **`DespawnOnExit` CLOSED 2026-11-02** — `DespawnOnExit(S::X)`, despawned with its children when a driven machine leaves `X`, between that state's `on_exit` and the next one's `on_enter` (in the prelude). **`DespawnOnEnter` closed 2026-11-07:** despawned, with its children, when `X` is entered, ahead of `X`'s `on_enter`. The hand-built version `state_scoped` measured leak-free stays in that demo as the record of what this replaced | `state_scoped` |
| ~~`EventMutator`~~ | **CLOSED 2026-11-02:** `EventMutator<T>` (and `Events::iter_mut`) changes the readable batch in place, before the readers ordered after it. Its access is a resource write, so it never shares a batch with a reader or writer of `T`; which side of it a reader lands on is ordering (`.chain()`). Tested with a damage clamp read back by a chained reader, `[40, 250] → [40, 100]`; pointing it at the write buffer instead fails | `events` |
| `iter_combinations` | **Present, both forms, for any `K`.** `iter_combinations::<K>()` yields `[Item; K]`. `iter_combinations_mut` is lending since 2026-11-06, `while let Some([a, b]) = c.fetch_next()`: as an `Iterator` it could hand out two live `&mut` to one row in safe code, and that form is gone along with its `unsafe`. | `iter_combinations` |
| ~~Entity generations in `Children`~~ | **CLOSED 2026-11-06.** `Parent` and `Children` hold whole `Entity` values, not raw ids, so a link to a dead entity stays a link to the dead entity whoever gets its id next. Every walker in the workspace checks `is_alive` (`Query::live` for a system with no `World`). Before it, a list written directly, or a handle kept past a despawn, could hide, move or despawn a stranger on the recycled id: `collect_hidden` hid one. Scene files still store plain ids on `parent_id`, which load remaps | `hierarchy` |
| ~~A typed single-entity reader on `World`~~ | **CLOSED 2026-11-02:** `World::get::<T>(entity) -> Option<&T>`, either storage. It checks the generation, so a stale handle whose id was recycled reads `None`, not the new entity's value — the case a raw-id `borrow::<T>().get(id)` gets wrong. Read-only on purpose: a write must mark the change for `Changed<T>`, which `query_mut` does and a raw `&mut T` could not. A lifecycle hook reads the value it fired for with it (tested: `11`); removing the liveness check fails the stale-handle test; Miri-clean under the CI flags | `component_hooks` |
| ~~`Entity` from a query~~ | **CLOSED 2026-11-06:** `Entity` is a query term, `Query<(Entity, &T)>`, and `iter()` yields only the item. Before that, 2026-11-02: `Query::entity(id) -> Option<Entity>`, the generation-checked lookup through the world the query already holds, so a scheduled system turns an id into a handle for `Commands` without ever having `&World`. Safe in parallel: it reads the allocator under its lock, as `Commands` itself does. Queries still *yield* raw `u32` ids. Tested on a recycled id (generation ≠ 0) despawned through `Commands` from a system; forging `Entity::new(id, 0)` instead fails it | `state_scoped`, `entity_disabling` |
| ~~A clear/sky colour setting~~ | **CLOSED 2026-11-02:** `Camera::clear_color` (`with_clear_color`, an inspector colour), per camera so split screen can differ. The deferred pass writes it where no geometry landed, the forward-only pipeline clears to it, and the editor's game view uses the game camera's; `None` keeps each path's own (deferred dark grey, forward light blue). It rides `SceneUniforms`' old `_align_pad`, now `clear_color`, so no offset moved. Tested: a red background reads (167, 0, 0) at a corner with the cube still drawn over it. **Building it found a defect:** the G-buffer's position target was cleared with `Color::BLACK`, whose alpha is 1, so the "no geometry" flag every reader tests (`w < 0.5`) was never set — sky pixels were lit as a surface at the camera, and the volumetric pass gave every sky ray zero length. Cleared to `TRANSPARENT` now; see CHANGELOG | `fog` |
| `AlphaToCoverage` | No MSAA to spread alpha across | `transparency_3d` |
| Binding a normal / MR / emissive / AO map | **Open since 2026-08-23** — `AssetManager::material()` is a builder over the seven-entry material bind group, every slot optional with a neutral default (base colour and sampler included), so `material().normal(&view).build(..)` is a complete material. `params` is nameable too. Before it, `assemble_material_bind_group` was `pub(crate)` and the public surface gave base colour only, with the glTF loader as the sole route to the other four. Cost measured then and now: the same height field as geometry is 55 296 verts at shading σ 39.27; as a normal map it is **6 verts at σ 38.98**, 99.3 % of the variation for 1/9216 of the geometry | `parallax_mapping` |
| A user post-pass that reads the frame | `set_render` really does let a game add its own full-screen pass (measured: 38.67 % of pixels, and the untouched rows stay bit-identical, so `LoadOp::Load` preserves the engine's frame). The *surface* has no `TEXTURE_BINDING`, so a pass cannot sample the swapchain — but it does not have to: `PostProcessState::hdr_texture` is `TEXTURE_BINDING | COPY_SRC` and its texture, view and bind group are public, which is the frame the chain itself reads. A user FXAA or radial blur is writable through it; what is missing is a **compute reduction**, priced at 10 ms a CPU readback in `auto_exposure` | `post_processing`, `auto_exposure` |
| ~~Fallible system params~~ | **CLOSED — and the row was stale when re-read on 2026-11-03.** `Option<P>` has been a system parameter since 2026-08-23 (`027d0ef9`): `Option<Res<T>>`, `Option<ResMut<T>>`, `Option<Query<Q>>` read `None` for an absent resource and the system still runs, which is the `else` branch this row said was missing. Only absence becomes `None`; a borrow conflict still panics, because it is a scheduling bug. Without the `Option` a missing parameter still panics by design. The neighbouring single-entity case closed the same day: `Query::single` / `single_mut` return the one match or `QuerySingleError::{NoEntities, MultipleEntities}` | `fallible_params`, `error_handling` |
| ~~Systems returning `Result`, `?`, an error handler~~ | **CLOSED 2026-11-02.** A system function may return `()` or `Result<(), E: Display>` (`SystemOutput`), so `?` works in its body. An `Err` goes to the world's `SystemErrorHandler` resource — a plain `fn(system, error)`, read without synchronisation by systems in parallel, so it has no state to race on — and by default to one `tracing::error!` naming the system; the system runs again next frame. Every system form takes it (0–12 parameters, closures included), and every existing system still compiles unchanged — the whole workspace and its demos built on the first try. Tested: `Ok` reaches no handler, an `Err` reaches it every frame with the system's name and the message, the default does not panic; dropping the forwarding fails the test. Run conditions still return `bool` | `error_handling` |
| ~~Default query filters~~ | **CLOSED 2026-08-24.** `World::default_query_filters_mut().add::<T>()`, and an entity carrying `T` is invisible to every query a system **declares as a parameter**. The hatch is the `IgnoreDefaultFilters` operand, in the signature. **The pricing was wrong twice, in opposite directions.** It predicted "seven entry points, each needing an opt-out": there are seventeen public `Query` methods and they all read one archetype list, so the filter is one predicate applied at construction — nothing per row, `has_row_filter` untouched, chunk iteration intact, and `entity_count`/`len` become exact rather than over-counts. But it also missed the only hard question, *which constructor*, and the first answer was **wrong**: filtering `World::query_unchecked` looked like "the system path" (the parameter impl goes through it) and is in fact the engine's mutable-from-shared hatch — **98 sites in 37 files, nine of them editor panels and seven of those the inspector**. Review traced four consequences before it shipped: the inspector went blank for a disabled entity with no UI left to re-enable it; transform propagation froze whole subtrees, stranding *enabled* descendants; `sync_bodies` **destroyed** a disabled rigid body rather than pausing it, leaving joints dangling and rollback snapshots invalid; and netcode captured unfiltered while restoring filtered, i.e. a desync. The boundary is therefore the parameter itself — `SystemParam for Query`'s fetch, one function — covering the **44** demo files and **3** engine files that declare a `Query` parameter. **Named limitation:** disabling hides an entity from *your systems*; it does not pause physics, transform propagation, audio or rollback, all of which read the world directly, and the four consequences above are why they must keep doing so | `entity_disabling` |
| ~~Custom relationships~~ | **CLOSED 2026-11-07.** `relationship!(struct Likes -> LikedBy;)` declares both sides as immutable components; `World::register_relationship` keeps the reverse index through insert, overwrite, remove, despawn and clone, and `OnTargetDespawn` says whether a target takes its sources with it. | `crates/gizmo-core/src/relationship.rs` |
| ~~Observer propagation control~~ (cancel) | **CLOSED 2026-08-24.** `World::stop_propagation()` from inside a listener ends the walk once that listener returns. This row had already diagnosed itself correctly — *"what is missing is not access but a return channel"* — and the fix is that diagnosis taken literally: handing the listener a `&mut World` earlier the same day **was** the channel, so the listener's return type never changed. Measured on the same five-link chain the row was written from: `[4, 3, 2, 1, 0]` with **2** links visited after a listener tries to veto, and `[4, 3, 2]` with **0** when it can. The remaining listeners on the vetoing entity still run — the flag is read once that entity is finished, not between its listeners — and `trigger` saves and restores the flag around its own walk, so a nested dispatch cancelling itself cannot truncate the walk that called it. Both halves of that save/restore are load-bearing and a different test catches each. **Still open in this row's other half:** the walk's *path* is always the `Parent` chain, and there is no way to choose another | `observer_propagation` |

### C2. `.exclusive()` is a concurrency barrier, not a mutability upgrade

`System::run` receives `&World`, never `&mut World`, and `.exclusive()` does not change that — it
only guarantees the system runs alone in its batch. **No scheduled system can hold `&mut World`.**
Anything that needs one — despawning what a query found, running a stored closure — must live in the
app-level `set_update` / `set_render` hook. The
engine's own `LifetimeSystem` takes the same route (`query_unchecked` + `world.entity(id)`).

### C2b. `Changed` and `Added` lost writes — **closed 2026-11-07**

One tick per schedule run with a strict `>` meant a system saw nothing written after it in that run, by commands after it, or between
runs. `auto_collider` had written this down as a trap to work around. A `Query` parameter now keeps the tick its system last ran at and
the tick moves between batches, so every system sees every write once: tested with readers on both sides of a writer, with commands,
and with a write between runs. A query built from the world (`impl System`, hooks) still reads the schedule's window.

### C3. Component hooks reach every write — **closed 2026-11-07**

`register_on_add` / `on_set` / `on_replace` / `on_remove`, plus a global despawn hook. Every path now fires them:

| path | add | set | replace | remove | despawn |
|---|---|---|---|---|---|
| `spawn_bundle` | 1 | 1 | 0 | 0 | 0 |
| `add_component` (new) | 1 | 1 | 0 | 0 | 0 |
| `add_component` (overwrite) | 0 | 1 | 1 | 0 | 0 |
| `remove_component` | 0 | 0 | 0 | 1 | 0 |
| `add_bundle` (all-Table) | 1 | 1 | 0 | 0 | 0 |
| `remove_bundle` (Table) | 0 | 0 | 0 | 1 | 0 |
| `insert_batch` (3) | 3 | 3 | 0 | 0 | 0 |
| `remove_batch` (3) | 0 | 0 | 0 | 3 | 0 |
| `spawn_batch` (4) | 4 | 4 | 0 | 0 | 0 |
| `clone_entity` (3) | 3 | 3 | 0 | 0 | 0 |
| `despawn` | 0 | 0 | 0 | 1 | 1 |

`add_bundle`, `spawn_batch` and `clone_entity` wrote columns directly and fired nothing, or once for four entities,
which made an index kept by hooks a guess: `relationships` measured 10 links and 7 in its index. They now take the
per-component route for a type with hooks and keep their fast path for one without, so a type nothing listens to
pays nothing. The one write hooks cannot hear is through `&mut T`; `impl_component!(T; immutable)` makes that a compile
error, and `relationship!` is the index, kept by the engine.

### C4. No custom executor or stepping — but phases of your own now exist

Measured in `schedule_internals`. **Closed 2026-11-07:** `Schedule::set_executor(ExecutorKind::Sequential)` runs a batch one
system at a time in added order, and a `Stepping` resource holds every schedule for a frame or a system at a time, keeping its
change-detection window. A wholly custom executor is still not a thing a game can write. There is
~~also **no `Startup` phase**~~ **closed 2026-11-07:** `add_startup_system` runs an ordinary system once, after the initial state's
`on_enter` and before the first fixed step, on every app loop.

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

### C9. What a finished game reached for (2026-11-01, `kule_yikici`)

The demos test one capability each; `demo/src/bin/kule_yikici.rs` is a small **complete** game —
a catapult, a tower of crates on real rigid bodies, five shots, win/lose, restart, a HUD — built
on the sprite path and the post-tonemap overlay the same day. It works: the tower stands at rest
(400 frames, no drift visible), collapses plausibly under a hit, and the HUD counts what fell.
What it had to build for itself, each recorded in the file's header:

- ~~**No 2D collision shapes.**~~ **Closed 2026-11-01, as shapes rather than a second solver.**
  Boxes were 3D `Collider::box_collider`s with depth 1, the stone a 3D sphere, kept on the plane
  by hand-written axis locks. Now: `RigidBody::planar_xy()` for the locks, and
  `Collider::{polygon_2d, polyline_2d, circle_2d, rect_2d}` build the shapes a 2D game asks for
  (ramps, wedges, hilly ground) from points in the plane, extruded along Z. A separate 2D solver
  was considered and **not** built: it would be a second physics engine beside a deterministic
  one, with its own contract to keep, and 2D bodies could not touch 3D ones. Writing the
  `planar_xy` test also found that the locks leaked through two position paths — fixed the same
  day (CHANGELOG, "Axis locks leaked…").
- ~~**A sprite and its collider are sized separately.**~~ **Closed 2026-11-01.** Nothing linked
  `Sprite::size` to the collider's half-extents, so the game's `spawn_box` derived both from one
  number. `gizmo::physics::sprite_collider(&sprite, depth)` (and `sprite_collider_scaled` for an
  entity with a `Transform::scale`, which the renderer applies and the physics does not) derives
  the box from the sprite — size and anchor; the game uses it.
- ~~**No level-reset helper.**~~ **Not a gap — a discoverability miss (2026-11-01).** The game
  wrote a despawn loop over a `Level` marker by hand; `World::despawn_all_with::<C>()` already did
  exactly that, with a doc example naming this very case. The game now calls it.
- ~~**No fixed-step driver for a game that owns its camera.**~~ **Closed 2026-11-01.**
  `simple_scene_update` steps physics but also flies the camera with WASD/mouse, so a game with a
  fixed camera could not use it, and the accumulator lived in the game (`FIXED_DT`, `MAX_STEPS`).
  `gizmo::simple::FixedPhysics` is that accumulator (fixed step, per-call ceiling, `alpha`,
  `reset`) and `gizmo::simple::sync_transforms` is `simple_scene_update`'s last job on its own;
  the game uses both.
- ~~**An orthographic camera does not fit the window's shape.**~~ **Closed 2026-11-01.**
  `Orthographic { height }` fixes the vertical extent; in a portrait window (948×1028, measured)
  the visible width was ±7.4 units and the catapult at x = −11 was off screen, so the game
  recomputed the height from the aspect ratio every frame. `ProjectionMode::OrthographicFit
  { width, height }` now shows at least that box in any window shape, and
  `ProjectionMode::view_extent(aspect)` says what is actually visible; the game uses both.

### C10. The second game: a 3D one (2026-11-02, `kristal_avcisi`)

`demo/src/bin/kristal_avcisi.rs` is the 3D counterpart to C9: a top-down arena, a player sphere,
emissive additive crystals, hunters that chase, three lives and a clock, and Menu → Playing → Over
on the driven state machine with `DespawnOnExit`. It works on the first build; what it ran into:

- ~~**Screen text is absolute window pixels.**~~ **Closed 2026-11-02.** The menu title was authored
  at `(640, 240)` — the middle of the 1280×720 the game asks for. A tiling window manager opened it
  at 472×492 and the title was off screen: the first screenshot had **no text at all**. `text.rs`
  and `kule_yikici` had both worked around the same thing by rewriting `TextSpace::Screen` from
  `WindowInfo` every frame. `Text::pinned(.., anchor, offset)` / `TextSpace::Pinned { offset }`
  pins the text's anchor to the **same** point of the window (centre, a corner, an edge's middle)
  plus a pixel offset, and follows resizes. `Text::pinned_at(.., at, offset)` pins to any
  fraction of the window (`kule_yikici`'s banner at 30 % of the height); `TextSpace::Pinned` is
  `{ at, offset }`. Both workarounds are gone — `text.rs` lost its `reposition` system entirely.
- ~~**No deferred bundle spawn.**~~ **Closed 2026-11-02.** `World::spawn_bundle` had no `Commands`
  twin, so the system that spawns a hunter wrote seven `insert`s where the enter hook wrote one
  tuple — and each `insert` moves the entity to another archetype. `Commands::spawn_bundle` and
  `EntityCommands::insert_bundle` queue one command that spawns and fills in one move; the game's
  hook and system now share one `hunter(..)` bundle.
- ~~**`on_enter` hooks see no GPU.**~~ **Half a discoverability miss, half a real gap — closed
  2026-11-02.** A hook takes `&mut World` and a mesh needs the `Device`, so the first version
  built everything in setup and kept it in a `Kit` resource. The GPU-free path already existed:
  `MeshSource("sphere")` is a key the asset loader uploads, and `MaterialDesc` is a material the
  draw pass resolves, both the next frame. What made it unusable: `MaterialDesc` had **no
  constructor and no builders** (thirteen fields by hand), neither was in the prelude, and the
  loader uploaded a **separate** mesh for every `"sphere"` entity — `garbage_collect` even kept a
  `primitive/` cache prefix alive that nothing ever filled. Now `MaterialDesc::new()` carries every
  `with_*` builder `Material` has (one macro expanded on both, and `Material::new` is
  `MaterialDesc::default()` over a bind group), `AssetManager::primitive_mesh` uploads each
  primitive key once, and both types are in the prelude. A GPU test draws an entity spawned with
  only `MeshSource` + `MaterialDesc` + `MeshRenderer`; the game's `Kit` is gone. **Geometry made in
  code closed the same day:** `MeshData` — an extrusion, a sweep, a sphere of any radius, a terrain
  chunk — is a triangle list the loader uploads like a `MeshSource`, clones sharing one upload
  (GPU-tested: drawn after the loader, not before; clones share a buffer; collection drops
  unused uploads). The game's crystals are an extruded star built in its enter hook.
- ~~**A tuple is not a system parameter.**~~ **Closed 2026-11-02.** The game wrote
  `(time, state): (Res<Time>, Res<State<Screen>>)` — what a Bevy user writes first, and the way
  past the nine-parameter limit — and it did not compile; the only grouping was the named
  `system_param!` struct. Tuples of two to eight parameters, nested freely, are now parameters:
  the state is the tuple of the members' states (a `Local` inside keeps its value) and the access
  declaration is forwarded member by member, so a write inside a tuple still serialises against a
  reader — tested, and an emptied declaration fails it.
- ~~**No sound without sound files.**~~ **Closed 2026-11-02.** `AudioManager` played files or a
  file's bytes and nothing else, the repository carries no audio assets, and the only WAV encoder
  in it was a test helper — so the first version of the game was silent. `gizmo_audio::synth`:
  `wav_from_samples` (16-bit mono PCM, clamped, NaN as silence), `AudioManager::load_samples` for
  generated PCM, and `Blip` — one oscillator (sine, square, triangle, saw, seeded noise), an
  exponential pitch slide, an attack and a linear decay to silence. Generated sound goes through
  the same decoder as a file; a test decodes the encoder's output with rodio and compares it
  sample for sample. The game makes its four effects this way.
- ~~**No RNG.**~~ **Closed 2026-11-02.** The game carried a four-line xorshift, as did
  `spotlight`, `parallel_query`, `change_detection` and the particle emitter; `showcase`,
  `rpg_demo` and `hill_climb` used `rand` directly. `gizmo_math::Rng` (in the prelude) is a seeded
  SplitMix64 — `range(a..b)` over `f32` and the integer types, `bool`, `chance`, `pick`,
  `shuffle`, `unit_vec2/3`, `in_unit_sphere`, `in_box`, `fork`, `with_stream(seed, stream)` — and
  an ordinary value, so a game keeps it in a resource. The xorshifts are gone. The first draft
  also had `f64` ranges, and every converted call site failed to compile: two float impls make
  `range(-5.0..5.0)` fall back to `f64`. `f64` ranges were dropped and a test pins the inference.
- ~~**No way to colour the whole screen.**~~ **Closed 2026-11-02.** A hit flash or a fade to black
  between levels meant a window-sized UI node faded by hand — `ColorTween` drives sprites, text and
  materials but not a UI background, and a node sits on one layer, not above all of them.
  `gizmo::renderer::screen_fade::ScreenFade` (a resource): `fade_out` / `fade_in` / `fade_to`,
  `flash(color, alpha, secs)`, `just_finished` for the frame to swap a level behind black. Both
  draw paths advance it on the **real** frame time (a fade into a pause menu does not freeze) and
  paint it as one rect on the highest overlay layer. GPU-tested over a red box on `ZIndex(1000)`:
  covered at full alpha, unchanged when clear, half-way at half; painting it on layer 0 or not at
  all fails it. `kristal_avcisi` flashes red when the player is caught.

### C11. The third game: physics at the centre (2026-11-02, `lobut`)

`demo/src/bin/lobut.rs` is a bowling game on real rigid bodies — ten cylinder pins, a CCD ball,
two rolls a frame, collision events for sound. Its first run had every body in free fall from
frame one. What it found:

- ~~**A `RigidBody` without a `Velocity` was not simulated, silently.**~~ **Closed 2026-11-02.**
  The ECS bridge gathered `(RigidBody, Transform, Velocity)`, so a floor written as
  `RigidBody::new_static()` + a collider — no velocity, a floor does not move — was not in the
  world, with no warning. A static body now joins the step with zero velocity; a moving one is
  given a `Velocity` through the command queue and simulated from the next frame. Scenes whose
  bodies all carry one take neither branch. Tested both ways; removing the branch fails both.
- ~~**Inertia was a placeholder unless set by hand.**~~ **Closed 2026-11-02.** `RigidBody::new`
  stores `(1,1,1)` and the documented rule was "call `update_inertia_from_collider`" — 28 of the
  36 `RigidBody::new` calls in `demo/` did not. A 20 cm ball turned ten times too hard, never
  rolled, and lost 80 % of its speed to friction in 1.5 s. The ECS step now replaces the
  placeholder (`RigidBody::UNSET_INERTIA`) with the collider's tensor the first time and writes
  it back; an inertia set by hand is kept. Measured: the ball settles into rolling at 6.77 m/s,
  the textbook 5/7 × 9.52 = 6.80. Bodies added to a `PhysicsWorld` directly (the
  `headless_stress_test` tower) are unchanged.
- ~~**No cylinder primitive.**~~ **Closed 2026-11-02.** `MeshSource("cylinder")` and `"cone"`,
  sized like their colliders (radius 1, half-height 1); `AssetManager::PRIMITIVE_MESH_KEYS` is
  the one list of primitive keys — the studio's missing-asset check kept a second copy of five.
- ~~**`with_simple_scene` overwrote a `PhysicsWorld` inserted during setup**~~, so a game could
  not set gravity there. **Closed 2026-11-02:** the default is inserted only when the setup did
  not insert one; tested with a Moon-gravity world, and an unconditional insert fails it.
- *Not a gap:* `EventReader::iter` is Bevy's `read`, with the same one-frame semantics.
- ~~**A moving platform did not carry the character on it.**~~ **Closed 2026-11-02** (found
  reading `platformer.rs` while choosing the next game). The character controller saw a snapshot
  of transforms and colliders with no velocities, so a platform moving sideways or turning slid
  out from under its passenger. `character_controller_system` now finds each grounded
  character's floor with the step's own ray and moves the character by the floor's `Velocity` at
  the contact point (`v + ω × r`) before the step. Vertical motion was already followed by the
  ground snap — measured, the lift test passes without the carry — and the carry is pinned not to
  add to it. A platform carries only through its `Velocity`; moved by `Transform` alone it has no
  motion to give.

### C12. The fourth game: AI, UI and the mouse (2026-11-02, `kale`)

`demo/src/bin/kale.rs` is a tower defence: enemies path-find across a grid (`NavAgent` on a
`NavGrid` the game builds), towers are placed with the mouse and are walls the enemies re-route
around (a tower that would close the path is refused), and a `gizmo-ui` button starts each wave.
None of the earlier games touched `gizmo-ai`, `gizmo-ui` or picking. What it found:

- ~~**No world point under the cursor.**~~ **Closed 2026-11-02.** `Camera::screen_to_ray` wants
  the window size, the camera's world position and the cursor, each from a different place, and
  `Ray` had no plane intersection: fifteen lines in the game. `gizmo::pointer::cursor_ray(world)`
  and `cursor_on_plane(world, point, normal)` — in split screen through the camera whose viewport
  is under the cursor, at that rectangle's aspect — and `Ray::intersect_plane`. Tested at two
  window sizes, by direction, above the horizon and in split screen; casting through the whole
  window instead of the viewport fails the last.
- ~~**No "is the cursor over the UI".**~~ **Closed 2026-11-02.** The obvious answer — the buttons'
  `Interaction` — is wrong: a click on the empty part of the game's button bar placed a tower on
  the ground underneath. `gizmo::ui::pointer_over_ui(world)` tests every `Node`.
- ~~**No click event; no optional query component.**~~ Both closed the same day for this game
  (`Clicked`, `Option<&T>`/`Option<Mut<T>>` — §C1).
- ~~**No particles in any `SimpleApp` game; no bursts anywhere.**~~ **Closed 2026-11-02.** The
  simple scene's render closure set `gpu_particles`, `ssr`, `ssgi`, `gpu_physics` and `gpu_fluid`
  to `None` **every frame** — so `ParticleEmitter` emitted nothing in every `SimpleApp` game, and a
  game that turned SSR on in its setup had it turned off again a frame later. The particle system
  was switched off because it dispatches and draws its whole ring (100 000) every frame, alive or
  not; it now skips both while nothing it ever spawned can still be alive
  (`GpuParticleSystem::is_idle`), so idle costs nothing and it stays on. The simple scene sets
  its other defaults once, before the game's setup, which may undo them. An emitter only streamed
  at `spawn_rate`; `ParticleEmitter::one_shot(n)` / `burst(n)` put `n` out at once (capped at
  4096), and the rule lives in one function (`gizmo::systems::render::emit`) that the studio now
  calls instead of its own copy of the loop. Tested: the clock (idle before any spawn, busy while
  anything spawned lives, idle after), a GPU test going busy → idle as an emitter is removed, the
  burst arithmetic, and a source guard on the simple scene's render closure. `kale` sparks on
  hits and bursts on kills.
- ~~**A grid change did not make agents replan.**~~ **Closed 2026-11-02.** They picked it up at
  their next recalculation (0.5 s by default) and walked into the new wall until then, so the
  game cleared every agent's path by hand. `NavGrid` now keeps a revision —
  `add_obstacle_world`, `remove_obstacle_world`, a physics rebuild and `mark_changed` (for direct
  writes to `obstacles`) bump it — and the navigation system replans every agent on the frame it
  changes. Tested with a wall dropped on an agent's route inside its replan interval; ignoring the
  revision fails it.

### C13. The fifth game: first person (2026-11-02, `poligon`)

`demo/src/bin/poligon.rs` is a first-person shooting range: mouse-look on a `CharacterController`,
a physics ray from the centre of the view, an impulse on what it hits, crate stacks to knock off
their pedestals. What it found:

- ~~**The cursor could not be held.**~~ **Closed 2026-11-02.** Nothing in the engine called
  `set_cursor_grab` or `set_cursor_visible`, so a mouse-look game's cursor left the window and
  sat over the scene. `gizmo::core::window::CursorOptions` (`mouse_look()`, `free()`,
  `with_grab`/`with_visible`) is a resource the windowed app applies to the window on the frame it
  changes; `Locked` falls back to `Confined` where the platform cannot lock (X11). Look itself
  already came from raw device motion. *Not covered by a test* — applying it needs a window;
  checked by hand on Linux the same day (cursor held and hidden, Esc releases and re-locks).
- ~~**No way to push an ECS body.**~~ **Closed 2026-11-02.** The impulse formula lived in
  `Integrator`, which the facade does not export, the three components had to be borrowed by hand
  and a sleeping body woken. `gizmo::physics::apply_impulse(world, id, impulse, point)` takes the
  id a raycast hit names; tested through the centre (pure linear), off it (spin), on a sleeper
  (woken) and refusing a static body and a missing entity.
- ~~**No screen shake.**~~ **Closed 2026-11-02.** A game that shook its own camera shook its aim ray
  and its look angles with it, and had to undo the offset itself — exactly, or the camera drifted.
  `CameraShake` (on the camera; in the prelude): `add(trauma)`, a smooth noise scaled by trauma²
  across the view (right/up offset, yaw, pitch), worn off at `decay` per second on the animation
  clock. Both draw paths call `apply_camera_shake` before the transform pass and
  `undo_camera_shake` after drawing, restoring the remembered `Transform`, angles and
  `GlobalTransform` bit for bit rather than subtracting. Tested: capped trauma and wear-off,
  trauma² bounded and continuous, the camera exactly back after fifty shaken frames, no motion
  along the look axis; `render_parity` requires both calls in both paths. `poligon` kicks on every
  shot and hit.

### C14. The sixth game: things that move smoothly (2026-11-02, `kaydirmaca`)

`demo/src/bin/kaydirmaca.rs` is a 4 × 4 sliding-tile puzzle: click a tile beside the gap and it
slides into it. The engine had **no tween and no easing** — a game wanting "there in 0.15 s,
slowing down" kept its own start, duration, clock and curve per moving thing.

- ~~**No tweens.**~~ **Closed 2026-11-02.** `Tween` (`to_position` / `to_rotation` /
  `to_scale` / `to_transform`, `and_*` to combine, `Repeat::{Once, Loop, PingPong}`) and `Ease`
  (eleven curves, every one 0→1, `OutBack`/`OutElastic` overshooting), in `gizmo-animation`,
  re-exported as `gizmo::renderer::tween` and in the prelude. The start is taken from the entity
  the first frame, so replacing a running tween starts from where it got to. The frame driver
  advances them on the animation clock (pause and time scale apply) **before** transform
  propagation, in both draw paths — `render_parity` now lists `tween_system(` among the drivers
  both must run. Tested: curve endpoints and shapes, exact landing, untouched parts, loop and
  ping-pong, zero duration, and a paused clock moving nothing.

### C14b. A 2D game with all of it (2026-11-02, `balon`)

`demo/src/bin/balon.rs`: balloons rise on an orthographic 2D view; click one and it grows, fades
and bursts into particles. `cursor_on_plane` worked through the orthographic camera unchanged.

- ~~**Tweens drove `Transform` only.**~~ **Closed 2026-11-02.** A popping balloon could grow but
  not fade; the first version lowered its sprite's alpha by hand with its own timer.
  `ColorTween` (`to(colour, s)`, `fade_to(alpha, s)`) eases the colour of a `Sprite`, `Text` and
  `Material` (whichever the entity has) with the same curves and repeats as `Tween` — the pass
  arithmetic is now one function, `Repeat::position`, shared by both. Both draw paths advance it
  beside `tween_system`, and `render_parity` lists it. Tested: easing and exact landing, a fade
  moving only alpha, the system writing both a sprite and a text, a paused clock writing nothing.
- *Not a gap:* `Sprite::color` is linear, so `0.15` shows as about `0.4` in sRGB; the first
  palette looked washed out and measured exactly as that predicts.

### C15. The menu every game has (2026-11-02, `menu`)

`demo/src/bin/menu.rs` is a title menu — Start, Fullscreen, Volume, Quit — usable with the
mouse, the keyboard and a gamepad. Three of its four buttons were impossible:

- ~~**No keyboard or gamepad navigation.**~~ **Closed 2026-11-02.** `gizmo-ui` had no focus, so
  every menu was mouse-only. `UiNavigation` (a resource, **off by default** because the keys are a
  game's too): arrows / Tab / d-pad move the focus in reading order and wrap, Enter / Space / the
  south button click the focused element through `Clicked`, the mouse moving onto an element
  takes the focus, and the focused element reads as `Interaction::Hovered` so a game's hover
  colour is its focus highlight. Tested: reading order over a spawn order that is not it, both
  wraps, Tab, one-frame activation of the focused element only, off leaves the keys alone, the
  mouse taking the focus; removing the sort or the activation fails them.
- ~~**No fullscreen at run time.**~~ **Closed 2026-11-02.** `WindowMode::{Windowed, Fullscreen}`,
  a resource the windowed app applies when it changes (borderless, current monitor). Not
  unit-testable — it needs a window.
- ~~**A game could not quit.**~~ **Closed 2026-11-02.** The windowed loop ended only when the
  window closed and the headless loop never. `AppExit` (a resource) ends both after the frame that
  inserts it, through the path closing the window takes (`shut_down`: the input recording is
  written, then the event loop exits). The headless half is tested — `run` returns after the
  third frame when the game asks on its third.
- ~~**No text input.**~~ **Closed 2026-11-02.** The input layer reported physical keys only — a
  game could read `KeyA` but not tell `a` from `A`, and could not read `ş` at all — so there was
  no way to ask for a name. `Input::typed_text()` is this frame's text as the keyboard layout
  wrote it (winit's `KeyEvent::text`, control characters dropped), and `gizmo::ui::TextInput` is
  a field: a click focuses it (a click elsewhere, Escape or Enter leaves), it takes the typed
  text up to `max_chars`, Backspace deletes, Enter marks it submitted for a frame; its `Text`'s
  own content is the placeholder, and the draw path shows the value, with a caret while focused.
  While a field has the keyboard, `UiNavigation` leaves the keys to it — otherwise Space and Enter
  typed into a name would also press the focused button. Tested on the edit rules, focus through
  clicks, and the navigation hand-off. The menu asks for a name and remembers it.
- ~~**Nothing survived a restart.**~~ **Closed 2026-11-02.** Menu settings and every game's best
  score were lost on exit; the engine kept scenes and the editor's own preferences, nothing for a
  game. `gizmo::app::save::SaveData` — `open(game)`, `get::<T>` / `set` / `remove`, `save()` —
  under the platform data dir (`localStorage` in the browser). A write is atomic (temporary
  file, then rename), and a save it cannot read is **moved aside** (`save.json.bad`, `.bad.1`, …)
  rather than overwritten by the next save. Tested: a round trip of numbers, strings and a
  struct, a wrong-typed read, a corrupt file kept twice over, safe names; removing the move-aside
  fails it. The menu remembers its volume and fullscreen, `kristal_avcisi` its best score.

- ~~**Nothing could be dragged.**~~ **Closed 2026-11-02.** The volume control was a button that
  stepped by 25 % because a slider needs a drag, and `Interaction` is a per-frame state with no
  notion of one. `gizmo::ui::Drag` (opt-in on any element): it starts on a press that **begins**
  on the element — sliding onto it with the button held starts nothing — follows the pointer
  **off** the element until the release (captured), and reports the drop for one frame
  (`just_ended`), with `start`, `position` and `delta`. Under a finger it works the same, the first
  finger being the mouse. Tested on a press-drag-off-release-slide-on sequence; dropping the
  capture or starting on a slide fails it.
- ~~**No slider.**~~ **Closed 2026-11-02.** Built from a `Drag` by hand it took the menu about forty
  lines — a fill child, two `ZIndex` layers to keep it under the label — and was unreachable from
  the keyboard. `gizmo::ui::Slider` on a `ButtonBundle`: a drag across it sets `value` from the
  pointer's place on the track (captured, clamped), and while it has the `UiNavigation` focus
  ←/→ and the d-pad step it instead of moving the focus (↑/↓/Tab still do). `just_changed`,
  `just_released`. The engine paints its `fill` over the background, under the label. Writing the
  keyboard test found a real double action: the right arrow that moved the focus onto a slider
  also stepped it, in the same frame — `UiNavigation::moved_this_frame` now keeps the two apart.
  Tested on drag, capture, release and both key paths; ignoring the focus move, letting ←/→ move
  the focus, or setting the value only when a drag starts each fails them. `menu`'s volume is one.

- ~~**One language.**~~ **Closed 2026-11-02.** Every demo wrote its strings in Turkish in the
  source; a game shipping a second language had no table to put them in and no way to switch at
  run time. `gizmo::app::locale::Locale` (a resource, in the prelude): JSON tables per language
  (nested objects become dotted keys; a table with a non-string value is refused whole), `get`
  falling back to the first language loaded and then to the key itself — a missing string shows
  as its key, where it is noticed — and `missing()` listing every key no table had; `format` fills
  `{name}` placeholders; `system_language()` reads `LC_ALL`/`LC_MESSAGES`/`LANG` or the browser's
  `navigator.language`. `LocalizedText` beside a `Text` is resolved **at draw time** (after a
  `TextInput`'s value, so a field's placeholder is translated too), so `set_language` relabels
  everything on the next frame. Tested on the lookup chain, placeholders, a refused table, tag
  normalisation and the draw-time resolution; a guard fails if the draw loop stops asking. `menu`
  is bilingual with a language button, and opens in the system's language (checked by screenshot
  under `LANG=en_US` and `tr_TR`). Plurals since 2026-11-03: `format_count(key, n, args)` and
  `LocalizedText::with_count` pick `key.one` for 1 and `key.other` otherwise, the forms always from
  **one** language (the current one if it has either, else the fallback — mixing them turned a
  Turkish 1 into the English "1 coin" in the first draft); Turkish writes only `other`, since the
  noun stays singular after a number. CLDR's full rules since 2026-11-07 (`icu_plurals`), Russian, Arabic
  and Polish included. Right-to-left text is in `text/shape.rs`: bidi, then shaping.

- ~~**No typed-out text.**~~ **Closed 2026-11-03.** Dialogue and announcements had no way to
  appear a character at a time. `Typewriter` (in the prelude) beside a `Text`: the label draws the
  first `visible()` characters of what it would otherwise show — its content, a `LocalizedText`'s
  string, a field's value — and a new string types from the top; `skip()`, `just_finished()`.
  Advanced by both draw paths on the animation clock (`render_parity` lists it). Tested on typing,
  finishing once, a new line, a skip, and a guard on the draw loop's cut; not restarting on a new
  line, or holding `just_finished` past its frame, fails it. `menu`'s announcements type out.

- ~~**No progress bar, no switch.**~~ **Closed 2026-11-03.** A health bar was two nested nodes
  with a percentage width kept in step by hand, and "Fullscreen: Off" a button relabelled on every
  click. `ProgressBar { value, fill }` (the engine paints the fill, like a `Slider`'s) and
  `Toggle { on }` on a `ButtonBundle` — every `Clicked` flips it, so mouse, finger, Enter and the
  gamepad all do — with a check box drawn at its left edge. Tested on one flip per click (a press is
  not a click) and on the drawn parts; flipping every frame fails it. `menu`'s fullscreen is a
  `Toggle`; `ritim` shows the song's progress in a `ProgressBar`.

- ~~**No reverb.**~~ **Closed 2026-11-03.** The only effect in the audio path was the underwater
  low-pass; a gunshot in a hall and one in a field sounded the same. `AudioManager::set_reverb(
  Option<Reverb>)` — `Reverb::room()` / `hall()` / `cave()` or any `room` / `damping` / `wet` —
  is Freeverb (eight damped feedback combs and four all-passes per channel, the second channel's
  delays spread), built as a stage inside every source the way the muffle is, so one store turns
  it on for every sound already playing. Off is bit-exact and runs no filter; a reverberated
  sound keeps playing its tail after the input ends, until a frame stays below 1e-4 or three
  seconds pass; channels reverberate apart; a seek clears the filter memory. Tested device-free:
  off = input, a click rings and decays and the tail ends, a bigger room rings longer, a silent
  channel stays silent, switching off is heard on the next sample; cutting the tail with the input
  or sharing one channel's state fails them. `poligon` plays in a hall, **V** toggles it.

- ~~**Nothing kept the mix under 1.0.**~~ **Closed 2026-11-03.** Every sound was added into the
  device's mixer and the sum had no ceiling: rapid fire in `poligon` (noise at 0.35 plus its reverb
  tail, several at once) went past 1.0 and the device clipped it. `AudioManager::set_limiter(
  Some(Limiter::default()))` puts a brick-wall limiter on the sum — instant attack, a release
  (100 ms default), one gain per frame — bit-exact under the ceiling, off by default;
  `take_limiter_gain()` reports how far it went. The players connect to a bus the manager owns now,
  which needs a silent keep-alive input: without it the first silence unplugged the bus on a real
  device (measured). `poligon` turns it on. **Still open:** an EQ and a general (ratio/threshold)
  compressor.

- ~~**A list could only be scrolled with a wheel.**~~ **Closed 2026-11-03.** A touchpad without a
  wheel, or a finger, could not reach the end of a `Scroll` list. Its thumb is drawn now (its
  length the visible share of the content, its place the offset's share of the travel) and can be
  dragged — captured until the release, clamped; a press on the track jumps there. The strip along
  the right edge belongs to the bar: a press there is not a press on the list item under it, or
  dragging the bar would click whatever it was released over. Tested on the thumb's geometry, a
  drag past the box, the clamp, the release and the strip; dropping the strip rule, the grab
  offset or the capture each fails it.

- ~~**No select list.**~~ **Closed 2026-11-03.** A quality or difficulty setting was a button
  cycling its label. `gizmo::ui::Dropdown { options, selected }` on a `ButtonBundle`: a click opens
  a list below it, drawn by the engine above every UI layer in the button's own text style; a press
  on an option picks it and closes the list, a press elsewhere closes it unchanged; **while open
  the list takes the pointer** — nothing under it is hovered, pressed or clicked, and
  `pointer_over_ui` counts it; with the navigation focus ←/→ and the d-pad step through the
  options (they do not move the focus off it). `just_changed`, `set_open`. Tested on open, hover,
  pick, the button underneath, close-without-change and the keyboard; letting the pointer through
  the open list, or letting ←/→ move the focus, fails it. `menu` has a difficulty list whose options
  follow the language.

- ~~**A list could not be flicked.**~~ **Closed 2026-11-03.** On a phone a list moves under the
  finger and keeps sliding when let go. A finger that comes down on a `Scroll`'s content and
  travels past an 8 px slop now scrolls it with the finger — and from then on the press is a
  scroll, not a tap: the item under it is neither pressed nor clicked when the finger lifts, or
  every scroll would end by selecting whatever it was let go over. Released, it slides on at the
  finger's last speed, decaying (×0.05 a second), and stops at the edges. A tap in place still
  clicks the item; a mouse dragging across a list does not scroll it. Tested on all four, a fling
  that must stop short of the edge; dropping the click cancellation, the slop or the finger-only
  rule, or the decay, each fails it.

- ~~**Holding a key did nothing more than tapping it.**~~ **Closed 2026-11-03.** `just_pressed`
  fires on the press only (the OS's own repeats are not new presses), so a held ↓ moved a menu's
  focus one button and a held → stepped a slider once — and a grid game's held arrow walked one
  tile. `gizmo::core::input::HoldRepeat` (in the prelude): fires on the press, then after `delay`,
  then every `interval`, catching up on a long frame. `UiNavigation` reads ↑↓←→ and the d-pad
  through it (0.4 s, then ten a second), and sliders and dropdowns now take their steps from the
  navigation (`sideways()`), so all of them repeat. Writing it found that a repeat on its own
  swallows the second of two taps in consecutive frames (it looks like one hold); a new press now
  restarts it. Tested on the timing in exact steps, release, a held ↓ walking the list, and the
  quick double tap; dropping the restart fails it.

- ~~**The keyboard walked the focus out of sight.**~~ **Closed 2026-11-03.** In a long menu
  inside a `Scroll`, a held ↓ moved the focus past the bottom of the box and kept going, unseen.
  When the keys (or the d-pad) move the focus, the nearest `Scroll` around the focused element
  now scrolls just enough to show it — its bottom meeting the box's when it is below, its top when
  it is above. A focus moved by the mouse does not scroll anything. Tested both ways and with the
  mouse; reversing the upward shift fails it.

### C16. Every game wrote the same three lines (2026-11-02)

~~**No timer.**~~ **Closed 2026-11-02.** `balon`, `kale` and `yikim_ustasi` each counted down by
hand — `spawn_timer -= dt; if spawn_timer <= 0.0 { spawn_timer = 0.9; … }`, a `cooldown: f32` on a
tower — and that shape throws away the time past zero, so a 1 s spawner at 7.5 fps fires every
8 frames (1.07 s) and a long frame fires once instead of twice. `gizmo::core::Timer` (in the
prelude): `once` / `repeating`, `tick(dt)` returns how many periods the step crossed and keeps the
remainder, `just_finished` / `finished` / `fraction` / `remaining`, `reset`, `set_duration`,
`pause`. A plain value — a resource field, a component, a local — ticked by the game, so it runs
on game time. Tested: catch-up and remainder, sixty fires a minute at 30/60/144/7.5/45.5 fps,
once-stays-finished, paused/negative/NaN/zero/infinite inputs; dropping the remainder, firing
once per tick or ignoring pause each fails them. `balon`'s spawner and `kale`'s spawner and tower
reload use it.

### C16b. The seventh game: many things born and killed (2026-11-02, `nisanci`)

`demo/src/bin/nisanci.rs` is a twin-stick arena shooter — WASD or the left stick to move, the
mouse or the right stick to aim, eleven bullets a second, sparks, waves of enemies. Nearly all of
it was already there (`move_axis`, `right_stick`, `cursor_on_plane`, `Timer`, `one_shot` +
`DespawnAfter`, `CameraShake`, `ScreenFade`); one thing was not:

- ~~**A mistyped mesh key failed again for every entity.**~~ **Closed 2026-11-02.**
  `MeshSource("standard_sphere")` (the key is `"sphere"`) is taken for a file path; the read fails,
  and the failure was not cached — so every bullet retried the disk, logged another
  `OBJ load failed` and uploaded another empty buffer, eleven times a second. The empty mesh is now
  cached under the key (collection drops it once nobody holds it, so a file that appears later
  still loads), and for a name without an extension the error lists the primitive keys. GPU-tested:
  two requests share one buffer, collection forgets it; not inserting into the cache fails it.

### C16aj. A game as an Android app (2026-11-05, `demo-android`)

The browser ran the games on a phone; a game a player installs is an APK. `scripts/android-apk.sh`
builds one (`pinbol`), and on a Redmi Note 12 Pro it installs, opens full screen, takes the touch
flippers, and survives being sent to the background. What stood in the way, all closed (CHANGELOG):

- ~~**No way in.**~~ An Android app starts in `android_main(AndroidApp)` and winit builds its loop
  only from that handle; `gizmo_app::android::set_app` hands it to `App::run`.
- ~~**The C++ runtime was not linked**~~ (`meshopt`): the library did not load.
- ~~**Six bind groups asked of a driver that allows four.**~~ The render path a target takes is a
  cfg of its own now, `gizmo_forward_only`, and Android takes the browser's.
- ~~**Back from the background to a black screen.**~~ The surface is let go on `suspended` and
  made again on `resumed`.
- ~~**A HUD the wrong size on a phone; sticks under the navigation bar.**~~ **Closed.** `DisplayScale`
  makes screen text, UI pixels and stick radii logical pixels; `SafeArea` reports the covered edge.
  A game has to read `SafeArea` itself: the layout does not apply it (only `nisanci` does).
- **Open:** one APK per game (`pinbol`, `nisanci`), each a host crate; `aarch64` only. (The CI `android` job builds, lints and uploads the APK; the render budget
  is 1280 × 720 pixels' worth, four times the browser's.)

### C16ai. A game on a phone, in the browser (2026-11-05, `pinbol` in `demo-web`)

The first game to run in the browser: `demo-web/?oyun=pinbol` runs `games::pinbol::app()`, the
same game the desktop binary runs, with touch flippers (two `VirtualButton`s over the halves of
the screen). Its `--test` lays a finger on the left half headlessly and checks the left flipper
rises. What getting there found:

- ~~**An absolute root UI node was not sized by its insets.**~~ **Closed.** `top: 0, bottom: 0`
  and no height gave a root 0 px tall (taffy lays a root out with no containing block); the
  touch buttons were 640 × 0. `stretched_root` sizes it between its insets.
- **A game could not run in the browser without moving house.** `demo-web` is a library and
  cannot reach a binary in `demo/`; a game that is to run in both lives in `games/` now, with an
  `app()` both hosts call. `pinbol` moved first; `nisanci` followed on the same day
  (`?oyun=nisanci`, two touch sticks).
- ~~**A touch control made on the first touch missed that touch.**~~ **Closed 2026-11-05.** A
  `VirtualStick`/`VirtualButton` claimed only a finger landing on it, and one made in answer to a
  touch has no box until layout runs, so `nisanci`'s sticks never got the first touch. On its first
  frame with a box a control now also takes a finger already down inside it that nothing holds;
  `nisanci --test` lays a first finger on the left stick and checks the player moves.
- ~~**No font in the browser.**~~ **Closed** (the project's call, made 2026-11-05: carry one).
  The web has no system fonts to borrow (`load_system_font` is always `NotFound` there), so text
  fell back to the synthetic face, ASCII only — every Turkish letter in a HUD missing on a phone.
  `gizmo-renderer`'s default-off `bundled-font` feature carries **Noto Sans Regular** (SIL Open
  Font License 1.1, `assets/fonts/OFL.txt`, 621 KB) and `default_font` uses it when the machine
  lends none; `demo-web` turns it on. Every letter of the Turkish alphabet has a glyph in it
  (`text::bundled` test).
- **Then run on a real phone** (Redmi Note 12 Pro, Android 13, Chrome 156, over `adb`: the page
  served locally through `adb reverse`, screenshots with `screencap`, the console through Chrome's
  DevTools protocol, touches with `input tap/swipe`). It drew nothing at first; five engine gaps
  stood between it and a game, all closed (CHANGELOG): the canvas held at 1280 × 720 CSS pixels,
  the first `Resized` lost to the async renderer init, a landscape-only render cap, pointer
  positions in the wrong pixels (every touch missed), and a console flooded at TRACE. Plus the
  game's own camera looking the wrong way. Now the table fills an upright screen, Turkish text
  draws, a tap launches the ball and a held left half raises the left flipper.
- ~~**A HUD line ran off a narrow screen.**~~ **Closed 2026-11-05.** Screen and pinned text wrapped
  only at `\n` or a fixed `max_width`; it now wraps at the room the window leaves it (CHANGELOG).
- ~~**Speckled rendering on the phone.**~~ **Closed 2026-11-05.** Rainbow noise on the bumpers,
  a quad of it on the table, rails patched in other objects' colours, flicker while playing; the
  desktop browser drew the same build clean. `gizmo_cluster_index` took the whole
  `SceneUniforms` by value (17 040 bytes) and Adreno 618's compiler (driver V@0502) miscompiled
  the lit fragment shader around the copy; it takes the five fields it reads now (CHANGELOG). A
  test fails any shader function that takes more than 1 KB by value.

### C16ah. State keyed by a raw entity id (2026-11-05, an audit after `sarmasik`)

The ECS hands a despawned entity's id to a later spawn, and much of the engine keeps state by the
raw `u32` (or `BodyHandle`, which is one). `sarmasik` found the first case (C16af: a dead body's
joints tied to the next body on its id); an audit of everything else that keeps state by id
across frames found four more. No production code registers a despawn hook, so nothing outside
the ECS is told an entity went. The free list is first in, first out, so these show in quiet
scenes or when it has run dry.

- ~~**Contacts and trigger overlaps.**~~ **Closed.** The contact cache is keyed by the pair's ids:
  a body born in a trigger on a dead body's id was told `Persisting` and never `Started`, and in a
  solid contact took the dead body's warm-start impulses. A removed body's pairs leave the cache
  and are reported `Ended` before the next step's own events (`tests/joint_recycled_id.rs`).
- ~~**Streamed textures.**~~ **Closed.** A finished decode was applied to whatever material held
  the id; it now goes only to a material whose `texture_source` is still the requested path.
- ~~**Object motion (TAA, motion blur).**~~ **Closed.** `MotionHistory` handed a new entity the
  previous matrix of the dead one on its id — a frame of motion from there to here. Its slot
  records the generation.
- ~~**UI focus.**~~ **Closed 2026-11-05, with no change to its API.** A menu rebuilt without calling
  `focus` again left the focus on whichever new widget took the old id (highlighted, and Enter
  pressed it). This was priced as needing an `Entity`-based API because the system has no world to
  read generations from; `Query::entity`, added since, is exactly that read. `UiNavigation` keeps the
  handle it saw, and a game focusing a widget it just spawned is still believed.
- ~~**`FighterController::already_hit`.**~~ **Closed 2026-11-05.** A victim on a recycled id was immune
  for the rest of the attacker's move; the list holds handles now (a public field's type changed).
- ~~**Hierarchy links written directly.**~~ **Closed 2026-11-06.** `Parent` and `Children` hold handles
  now, and every walker checks them (C1). `collect_hidden` was hiding a stranger on a dead child's id.
- Checked and safe: character-controller triggers
  (rewritten every step), kinematic targets and soft/cloth/rope coupling (rebuilt every frame),
  drag/drop and text-input focus, rollback restore (generation-checked), editor selection
  (`Entity`, generation included), AI targets (positions), audio (sink ids).

### C16ag. The games as a gate (2026-11-04, `scripts/game-tests.py`)

Fifteen games had a headless `--test` mode and no gate ran one; a CCD fix that day moved
`robot`'s balls onto a path that exposed a 3 cm reach error, found only because the game was run
by hand. `scripts/game-tests.py` now plays every demo with the mode — release, four at a time,
no display (a binary that ignores the flag fails instead of opening a window), a fresh empty
save directory each — in CI's determinism job and the local gate. The games without the mode are
being given one, on `gizmo::simple::HeadlessFrames`: the game's own `setup` on a plain `World`,
its own `update` with an empty `Input`, the deferred commands applied after every frame and the
simple scene's defaults added, as the windowed loop does. Since 2026-11-05 **all 36 games** have
one (86 s for the lot, four at a time). What turning them into tests found:

- ~~**A kinematic body driven once a frame moved in jerks.**~~ **Closed.** `KinematicTarget`
  is reached in the next step, so a four-step frame made the whole move in its first step at four
  times the speed. A ball on a board starting to tilt (surface rising at 1.8 m/s) was thrown up at
  5.2 m/s; `FixedPhysics::advance` now spreads the target over the frame's steps: 1.74 m/s.
- ~~**Two of `labirent`'s three levels could not be won.**~~ **Closed.** A hole is a whole
  missing tile and a ball cannot roll across one; in levels 2 and 3 a hole cut every way to the
  goal. Found by the autopilot's path search; the test now checks every level has a path.
- ~~**`pinbol`'s ball could be stuck in the launch lane for good**~~ — rolled back in, it was
  still counted as launched. **Closed.**
- ~~**Tweens, typewriters and the screen fade moved only when a frame was drawn.**~~ **Closed.**
  They were advanced inside the draw path's `prepare_frame`, and game logic waits on them (`zindan`
  takes the stairs when its fade has gone dark, `mucevher` moves on when its gems' tweens end).
  `systems::render::advance_frame_clocks` is the one function the draw path and `HeadlessFrames`
  both call.
- ~~**A 2D scene could not be built without a renderer.**~~ **Closed.** A `SpriteImageId` came only
  from `load_sprite_rgba`, which needs a GPU. `SpriteImageId::placeholder(n)` names no image and
  draws nothing (`zindan`).
- ~~**`poligon`'s player walked at twice his speed**~~ (10 m/s for a 5 m/s controller): the game
  still drove the character controller once a frame after `FixedPhysics` had started driving it
  every step. **Closed.**
- ~~**`pist`'s AI could be held against a barrier for the rest of the race**~~ — it backed off
  0.6 m and drove back in. **Closed** (2 s at full throttle). The three-car race itself passed
  here and failed on CI, a car rolled over: cars hitting cars diverge from machine to machine
  (same-platform determinism), so the test drives one car, at three frame rates.
- Not engine gaps, but what a headless loop must do, all in `HeadlessFrames` now so no game's test
  can forget one: apply the deferred commands (fracture despawns through them — `vinc` reported 0
  pieces where 9 broke); insert the scene's physics world (`labirent`'s ball never moved); add the
  game's plugins and run their schedules (`nisanci`'s sparks piled up to 537 without
  `LifetimePlugin`); put `Input` in the world and advance `Time` before them (`UiPlugin`'s systems
  stop the frame without `Input`); end the frame with `Input::begin_frame`.
- Measured and not a gap: `kopru`'s single row of hinged beams carries the ball, as a sagging
  chain (86 % of the joint threshold); `komutan`'s twelve charging at fourteen equals lose, the
  game wants tactics; `zindan`'s autopilot dies on level 2, it attacks everything it sees.

### C16af. The thirty-sixth game: ropes taken and let go (2026-11-04, `sarmasik`)

`demo/src/bin/sarmasik.rs` crosses a chasm on four ropes hanging from the ceiling: hang, pump,
let go, catch the next in flight. Each grab pushes a `Joint::rope` into `PhysicsWorld::joints`
at the length the player is from the anchor, each release takes it out — the first game to make
and break joints while it plays. `sarmasik --test` opens no window: the autopilot pumps and lets
go when the flight lands within reach of the next rope; across in 14 s, three grabs, the rope
never more than 2 mm past its length. One gap, closed:

- ~~**A body on a recycled id inherited the dead body's joints.**~~ **Closed 2026-11-04.** Physics
  knows a body by its raw ECS id, and the ECS hands a despawned id to the very next spawn. Despawn
  a roped body and spawn another before the next step — what restarting after a fall does — and
  the new body took over the old row, joints and all: spawned 15 m away, it was pulled 5 m toward
  the dead body's anchor in a second. The ECS bridge now records each id's generation; an id that
  comes back under a new one is a new body, and the old one is removed with its joints.
  `tests/joint_recycled_id.rs` (red with the bridge's check taken out).

### C16ae. The thirty-fifth game: a fast ball and flippers (2026-11-04, `pinbol`)

`demo/src/bin/pinbol.rs` is a pinball table — gravity tilted 12°, a 5 cm CCD ball, two kinematic
flippers swung by `KinematicTarget` at 25 rad/s, three bumpers that kick. `pinbol --test` opens
no window: an autopilot flips for a minute (11 flipper hits, 1100 points, the ball never leaves
the table through a wall; 6 hits before the kinematic fix in C16ag, which also showed that a ball
rolling back into the launch lane was stuck there for good). One gap, closed:

- ~~**A CCD body touching a wall was held by it.**~~ **Closed 2026-11-04.** Under 1 mm the
  speculative contact took its normal from the approach direction itself, so a ball resting a
  hair from the rail and rolling *along* it under gravity got a contact pointing straight back up
  the slope, and stopped: the first ball hung on the launch-lane rail, normal (0, 0.978, −0.208).
  A lone CCD ball on a slope rolled 9 % short the same way, its own floor taken as being in the
  way. The normal is now the surface's own, measured a little past it along the approach.
  `tests/ccd_touching.rs` (0.025 m and 0.665 m before, against ~1 m free).

The same change moved `robot`'s balls, and its autopilot for the first time asked for a tip
inside the arm's minimum reach (|upper − lower| = 0.2 m, folded over the root) — 3 cm missed.
`solve_two_bone_ik` documents that it folds to that reach; the game's `clamp_reach` clamped only
the outer one, and now clamps both (0.000 m again).

### C16ad. The thirty-fourth game: inverse kinematics (2026-11-04, `robot`)

`demo/src/bin/robot.rs` is a two-bone arm on a pedestal pushing balls into a pocket — the arm's
two capsules are kinematic bodies driven by `KinematicTarget`, posed every frame by
`solve_two_bone_ik` (elbow up, so the arm passes over the balls and only the tip comes down).
`robot --test` opens no window: the autopilot sinks all five in 24 s, the tip never further than
0.000 m from where it was asked to be. The engine's IK had no user at all before this.
**No gap found.** One note made where the next person will read it: `rolling_friction` is
combined by geometric mean like the sliding coefficients, so a ball with it on a table without it
rolls forever.

### C16ac. The thirty-third game: a ragdoll in flight (2026-11-04, `topcu`)

`demo/src/bin/topcu.rs` is a cannon that throws a ragdoll (`RagdollBuilder::create_humanoid`) at
rings on the ground; where it comes to rest scores. `topcu --test` opens no window: one ballistic
shot at 40°, 14.9 m/s — lands 1.3 m short of the 25 m target, tumbles 8.6 m on, at rest
(0.07 m/s) within eight seconds; joints at most 6 mm apart in flight and impact. One gap, closed:

- ~~**The stock humanoid was born apart**~~ — nine of ten joints open at spawn (the shoulders by
  18 cm), the lower arms inside the torso: every ragdoll snapped and kicked on its first step, and
  the one in the barrel shook before it flew. Each bone now sits where its joint's anchors meet;
  `spawn_ragdoll` warns about one that does not (`tests/ragdoll_spawn.rs`).

### C16ab. The thirty-second game: soft solids (2026-11-04, `jole`)

`demo/src/bin/jole.rs` is a tower of jelly blocks dropped from a swinging crane — `SoftBodyMesh`
blocks on a rigid platform and on each other. `jole --test` opens no window: four blocks dropped
when the crane is centred, five seconds to settle — 2.83 m tall (2.80 m upright), none fallen,
the fastest node 0.12 m/s at the end. Six gaps, all closed:

- ~~**A dropped block exploded**~~ — one explicit 1/60 s step for a material that needs thirty.
  `SoftBodyMesh::stable_substeps`; the system sub-steps every body together.
- ~~**A resting block sank into the floor**~~ (0.33 m in 10 s) — the sweep ignores moves under
  1e-5 m and nothing put a node that was inside back out. It is put on the surface now.
- ~~**A node leaving the floor was held on it**~~ while its velocity climbed — a surface the node
  moves away from was taken as a hit.
- ~~**A jelly fell through a jelly**~~ — `soft_contact`: out through the face it is falling
  through, Coulomb friction (without it a stacked block slid 0.8 m in 5 s and fell off), the move
  and the impulse shared by inverse mass. Choosing "the nearest point" or "the shallowest face"
  failed on blocks built on one grid (nodes on corners and in side planes); a push made in one go
  blew an off-centre stack apart — at most 5 mm a call.
- ~~**No way to build a block, and the hand-built one was non-conforming**~~ —
  `SoftBodyMesh::block` (Kuhn split); `cpu_physics` used five tetrahedra a cube, which leaves
  half the inner faces unmatched, and `damping = 5.0`, which adds speed.
- ~~**A soft body was not drawn**~~ — `soft_body_mesh_system`, both paths.

*Observation, not a gap:* jelly towers are wobbly. Four blocks dropped from 0.6–1.2 m stand; from
1.6 m they topple; six can lean over within a second. The game drops from 1 m.

### C16aa. The thirty-first game: what water carries (2026-11-04, `kayik`)

`demo/src/bin/kayik.rs` is a motorboat race through six buoy gates on a lake — the boat, the
buoys and the lake a `FluidZone`, a side gust every few seconds rolling the boat. `kayik --test`
opens no window: three minutes of autopilot with a gust every four seconds — all six gates, the
hull 35 % under (Archimedes says 35 %), never tipped past 6°, the buoys at the surface. Two gaps,
both closed:

- ~~**Water carried a body at the wrong depth and never righted it**~~ — the push acted only
  while the body's origin was inside the zone, as one force at the centre of mass: a density-300
  box floated half under instead of 30 %, a tipped hull stayed tipped, a spinning one barely
  slowed. Buoyancy and drag are now summed over a 3×3×3 grid of cells, each at its own point
  (`tests/buoyancy.rs`).
- ~~**A steady push was a different push at every frame rate**~~ — the force accumulators are
  emptied by each step and a game writes them once a frame; under `FixedPhysics`, 10 N for a
  simulated second gave 5.0 m/s at 30 fps and (with `+=`) 24.0 at 144. `ExternalForce`, applied
  on every step while it is on the body (`tests/external_force.rs`).

Not a gap, a limit the game works around: a zone's drag is the same in every direction and has
no term of its own for turning, so a hull slides sideways as easily as forward and its roll dies
slowly (~8 % a second). The game adds a keel (sideways damping) and bilge keels (roll damping)
through the same `ExternalForce` — what a hull's shape does, which a zone does not know.

### C16z. The thirtieth game: a car on terrain (2026-11-04, `arazi`)

`demo/src/bin/arazi.rs` is a checkpoint race over generated hills — `VehicleController` (wheel
rays through the broadphase) on the heightfield collider `golf` brought in, the first time the two
were driven together. `arazi --test` opens no window: two minutes of autopilot, all five
checkpoints, the body's centre 0.66–1.50 m above the ground throughout. **No gap found.**

### C16y. The twenty-ninth game: an authoritative server (2026-11-04, `kovala`)

`demo/src/bin/kovala.rs` is tag on the client-server netcode: the server moves everyone and
chooses who is "it", three bots run on it, a client predicts its own runner, reconciles with the
server and interpolates the others. `kovala --test` opens no window: a server and two clients over
127.0.0.1 for five seconds — ~300 reconciliations each, largest correction exactly 0, the other
runner seen at most 0.58 m behind (100 ms of interpolation delay at 5 m/s). The first game on that
half of `gizmo-net`. Five gaps, all closed:

- ~~**The facade could not reach `client_server`**~~ — `network` enabled rollback only. Feature
  `network-client-server`.
- ~~**No wire format, no send/receive**~~ — a game needed `renet` and a serializer of its own.
  `encode`/`decode`, `NetworkServer::{clients, send, broadcast, receive}`,
  `NetworkClient::{is_connected, send, receive}`.
- ~~**No message carried what reconciliation needs**~~ — the ack and the state came apart, and
  the state was keyed by entity index. `ServerMessage::PlayerState`.
- ~~**A game could not send its own data**~~ — `ClientMessage::Custom`, `ServerMessage::Custom`.
- ~~**`UdpTransport` took anyone's datagrams**~~ (rollback, found on the way) — only the peer's.

Not changed: the reference `server/` binary still sends `InputAck` and an entity-keyed
`WorldStateUpdate`; it moves no players, so there is no per-client state for it to send.

### C16x. The twenty-eighth game: a net that holds things (2026-11-04, `file`)

`demo/src/bin/file.rs` is fruit falling into a net you slide left and right — a `Cloth` pinned on
its edges, moved by moving its pinned nodes, holding rigid bodies through the two-way coupling.
`file --test` drops ten fruit two seconds apart onto a still net: all ten caught. One gap, closed,
and one left open:

- ~~**Cloth damping was hard-coded and next to none**~~ (1 % a second) — a fruit kept bouncing at
  its arrival speed. `Cloth::damping` (the net uses `0.05`).
- ~~**Open, measured:** a fruit lying in the damped net still rises and falls ~5 cm — "the
  coupling's own limit cycle"~~ — **misdiagnosed, measured 2026-11-04.** The 5 cm was the fourth
  second only. Over fifteen seconds the ball's per-second height range on the test hammock at
  `0.05` goes 19.4, 12.2, 4.6, 3.2, 1.6 … 0.2 (tenth second), 0.0 cm (thirteenth): a slowly
  damped ~2 Hz bounce, not a cycle, and at `0.99` it decays too (19 → 7 cm by the fifteenth).
  Two fixes were tried for a cycle that was not there, and neither is kept: paying the body for
  the projection's displacement (`−m·Δx/sub_dt`, counts the node's momentum twice), and letting
  each node's impulse update the body's velocity inside the cloth sub-steps (made it *worse*:
  4.6 → 7.5 cm in the fourth second at `0.05`, because the collider still does not move within
  the step, so a body seen rising keeps being met by nodes the projection pushes back down).
  `a_damped_hammock_settles_the_ball` now asserts rest in the tenth second.

### C16w. The twenty-seventh game: cloth (2026-11-04, `makas`)

`demo/src/bin/makas.rs` hangs a curtain — a `Cloth` component, swung by the engine's physics step
— with a red region to cut out by dragging the mouse; cuts remove the links the blade crosses, and
whatever is no longer connected to the rod falls. `makas --test` opens no window: an octagonal cut
around the red region, three seconds — all of it detached and 1.5 m down, 95 % of the rest still
hanging. One gap, closed:

- ~~**A `Cloth` in the world was never drawn**~~ — no draw path knew the type, and the cloth kept
  no grid width. `surface_triangles` + `cloth_mesh_system` in both draw paths.

~~Still open, and the reason this is a cutting game rather than a net or a trampoline: cloth is
**one-way** — it drapes over rigid bodies and pushes nothing back.~~ **Closed the same night**
(`Cloth::step_coupled`): a hammock holds a 1 kg and a 5 kg ball. What is open now is damping —
the cloth has next to none, so a ball keeps bouncing on it at about its arrival speed.

### C16v. The twenty-sixth game: a crowd (2026-11-04, `karinca`)

`demo/src/bin/karinca.rs` is an ant colony: 600 `NavAgent`s walk between a nest and food piles;
the player drops walls (every agent re-routes on that frame) and new piles. `karinca --test` opens
no window: 40 s with a wall across the colony from 10 s to 25 s — 748 pieces carried, the longest
AI frame 1.6 ms. Two gaps, both closed, measured with a 100 × 100 crowd probe before the game was
written:

- ~~**A crowd's route searches ran on one thread**~~ — 1000 agents given a target on one frame
  cost 44 ms; in parallel, 8.4 ms.
- ~~**An unreachable target was searched for every frame**~~ — 4000 agents on a cluttered grid
  put ~950 searches into every frame. A failed search waits for the schedule now, and
  `NavAgent::route_failed()` lets the game pick another goal (this one does).
- ~~**Routes were staircases of cell centres**~~ — found later the same night; straightened
  (`find_smooth_path`), the colony's 40 s haul went from 748 to 1198.

### C16u. The twenty-fifth game: rules written in Lua (2026-11-04, `yilan`)

`demo/src/bin/yilan.rs` is snake with every rule in `demo/assets/scripts/yilan.lua`; the Rust side
builds the board and the HUD and calls the facade's own `PlayLoop::step` — the step the editor's
▶ and an exported game take. `yilan --test` opens no window: the script plays itself and the board
is checked (score 17, exactly 3 + score segments, all with a mesh and a colour). The first game
whose logic is a script. Four gaps, all closed:

- ~~**`entity.spawn` returned nothing**~~ — a script could not name what it had made. It returns
  the reserved id during `on_update`.
- ~~**Nothing a script spawned could be seen**~~ — no mesh or colour call. `set_mesh`, `set_color`.
- ~~**The host could not read a script's values**~~ — `get_global` / `set_global`.
- ~~**A script could only come from a file**~~ — `load_script_source`.

~~Left: `on_entity_update` ran without the world, so `spawn` there returned `nil`~~ — closed the
same night: `update_entity_in` gives it the world, and the play loop calls that.

### C16t. The twenty-fourth game: a landscape (2026-11-04, `golf`)

`demo/src/bin/golf.rs` is three holes of golf on a generated hilly course: aim with the mouse,
hold the button for power. The course is built in memory — `AssetManager::terrain_triangles`
draws it, `gizmo::systems::terrain_collider` collides it — and `golf --test` opens no window: an
AI plays the three holes (10 strokes) while every step measures how far the ball sinks into the
ground (at most 6 mm). The first game on terrain; no demo used `Terrain` before. Two gaps, both
closed:

- ~~**A terrain collided as a flat box**~~ at half its tallest hill — the code called it "a crude
  stand-in". A heightfield of the same samples now, and the mesh split along its diagonal (the
  other diagonal was used, so even the right collider and the drawn surface disagreed inside
  every sloped cell).
- ~~**A generated terrain needed a heightmap file**~~ — `Terrain` takes a path and the mesh was
  built inside a function that needs the GPU device. The triangle list and the collider can now
  be built from samples in memory.

### C16s. The twenty-third game: saving what you built (2026-11-03, `atolye`)

`demo/src/bin/atolye.rs` is a building sandbox — click a column to stack a block, F5 writes the
level, F9 brings it back — and the first game to use the engine's scene format as its save.
`atolye --test` opens no window: it builds 30 blocks, saves, clears, loads and compares each
block's place, colour, collider and mesh; all 30 come back. One gap, closed:

- ~~**No way to save part of a world**~~ — `SceneData::save` writes everything, camera and HUD
  included (each load would add another camera), and the struct is `#[non_exhaustive]`, so a
  game holding `serialize_entities`' output could not build a `SceneData` from it except by
  assigning a `default()`'s fields one by one, joints left out. `of_entities` (with the joints
  inside the set) and `instantiate` (re-pointing them) now.

Observation, not a gap: a block is ~1.7 KB of RON — every component writes every field,
defaults and `RigidBody`'s force accumulators included.

### C16r. The twenty-second game: two players over a network (2026-11-03, `itis`)

`demo/src/bin/itis.rs` is a two-player shove match on `RollbackSession`: two pucks on a square
arena, three falls wins. It runs locally (two sessions in one process over a `LoopbackTransport`
with 4 ticks of lag), over UDP (`--port/--peer/--player`), or as `itis --test`, which opens no
window: two sessions over a lossy, laggy link, one starting 40 frames late, 1500 ticks of random
play — both end on the hash and score of the same match played in one world. The first game on
`gizmo-net`. Four gaps, all closed:

- ~~**The peer that started first stayed ahead for good**~~ — the session never waited, so a late
  start put every remote input past the rollback window. `try_advance` holds the leader.
- ~~**Game state did not roll back**~~ — only the `PhysicsWorld` did; a goal scored on a
  prediction stayed on the board. `with_state` snapshots the game's state with the world, and a
  `StepRule` runs after every step, re-simulated ones included.
- ~~**Inputs were applied in opposite orders on the two peers**~~ — local first, so any rule that
  depended on order came out differently. Player-id order on both now.
- ~~**A session over `LoopbackTransport` could not be a `World` resource**~~ — its queues were
  `Rc<RefCell<_>>`. `Arc<Mutex<_>>` now.

Also found: `cargo test -p gizmo-net` with default features did not compile (an ungated test).

### C16q. The twenty-first game: breaking things (2026-11-03, `vinc`)

`demo/src/bin/vinc.rs` is a demolition game seen from the side: a crane hook driven left and
right (`KinematicTarget`), a 300 kg ball on a 6.5 m `Joint::rope`, and a building of 32
`Breakable` blocks to bring down to a fifth in 45 s. `VINC_OTO=1` pumps the swing unaided and
wins at about 31 s with 27 of 32 down, 7 of them shattered. Three gaps, all closed:

- ~~**A `Breakable` never broke in a game**~~ — nothing ran `physics_fracture_system`, neither
  `FixedPhysics` nor `PhysicsPlugin`. The facade's physics step runs it now.
- ~~**Debris was invisible, immortal and scaled twice**~~ — bare sphere-collider bodies with no
  geometry, the block's `scale` copied on, `debris_lifetime` read by nothing. Each piece carries
  `Debris` (its Voronoi cell), is drawn in the broken body's material, has scale one and expires.
- ~~**`draw_line` went nowhere in a simple scene**~~ — there was no `Gizmos` resource, and no word
  about it; the rope was not drawn. The simple scene inserts an empty one.

Tuning note, not a gap: the ball's hit on a block is about 160 N·s, so a 250 N·s threshold only
toppled blocks; the game uses 90.

### C16p. The twentieth game: pushing things (2026-11-03, `kapi`)

`demo/src/bin/kapi.rs` is a top-down box puzzle: three rooms, each door a kinematic body
(`KinematicTarget`) that sinks into the floor while every pressure plate of its group is pressed —
by a crate pushed onto it (`PhysicsWorld::overlap_shape` on the plate's box) or by the player
standing on it. `KAPI_OTO=1` solves it unaided, three crates onto three plates and out. Two gaps,
both closed:

- ~~**A character could not push anything**~~ — every collider was level geometry to it; a 10 kg
  crate stopped it like a wall. `CharacterController::push_force` (400 N) pushes the dynamic
  bodies its sweep hits, capped at walking speed.
- ~~**A character stopped at a trigger, and no trigger saw a character**~~ — the sweep treated
  `is_trigger` colliders as solid, and a character without a `RigidBody` is not in the physics
  world, so `trigger_events()` never named it. Triggers are out of the sweep now, and
  `CharacterController::triggers` lists the ones it is inside.

### C16o. The nineteenth game: being seen (2026-11-03, `gizli`)

`demo/src/bin/gizli.rs` is a top-down stealth game: guards patrol a warehouse on `NavGrid`, each
casting a vision cone that is rebuilt every frame from rays against the walls (a `MeshData`), and
suspicion fills while the player is inside one with nothing in between. Four gaps, all closed:

- ~~**A replaced `MeshData` was never drawn**~~ — read once, on the first frame without a `Mesh`.
  Re-uploaded on change now, the old upload released (the game path never garbage-collects).
- ~~**An agent sent to its own cell never arrived**~~ — an empty route skipped the arrival check.
- ~~**A `RigidBody` without a `Collider` stood still silently**~~ — not simulated; warned now.
- ~~**Two `Ray` types, no conversion**~~ — `From` both ways.

### C16n. The eighteenth game: time while closed (2026-11-03, `bostan`)

`demo/src/bin/bostan.rs` is a garden idle game: plant a crop on a plot, wait, harvest, sell, buy
dearer seeds and a watering upgrade. Planting times are wall-clock and saved, so closing the game
does not stop the garden and the next launch reports what ripened. Two gaps, both closed:

- ~~**No wall clock.**~~ **Closed.** `gizmo_core::time::unix_time()`; `SystemTime::now` panics on
  `wasm32-unknown-unknown`, so the obvious code worked natively and crashed in the browser.
- ~~**Colours from a palette came out washed.**~~ **Closed.** The engine's colour slots are linear;
  `Color::hex` / `rgb8` take numbers literally. `Color::srgb` / `srgb_u8` / `srgb_hex` decode.

`SaveData` with a `serde` struct holding a vector of enums, `Display::Grid`, `ProgressBar` and
`Clicked` worked on the first try.

### C16m. The seventeenth game: a structure of joints (2026-11-03, `kopru`)

`demo/src/bin/kopru.rs` is a bridge builder: beams drawn between grid nodes become bodies, nodes
become breakable ball sockets, a 120 kg ball tries to cross, and each beam is coloured by its
joints' load. Three gaps, all closed:

- ~~**A joint's load could not be read.**~~ **Closed.** `Joint::last_force` / `last_torque` /
  `load()`, the numbers the thresholds are compared with (a sleeping pair keeps its last value).
- ~~**Fixed, hinge and ball-socket joints broke at half their `break_force`.**~~ **Fixed.** Their
  point constraint skipped its linear rows below 0.1 mm of error; a hanging 10 kg read 196 N.
- ~~**A despawned body's joints caught the next owner of its id.**~~ **Fixed.** Joints go with
  their body.

*Open, measured:* the truss's joint forces run above a hand calculation — ~5 kN under its own
weight where the top chord should carry roughly 2.6 kN, and ~16 kN with the 1.2 kN ball at
midspan. Neither more joint iterations (10 → 40) nor beam damping changes it. **Separated:** a
statically determinate two-bar wall bracket (5 kg bars, 50 kg at the tip) reads within ~3–6 % of
the member forces statics gives (`joint_load.rs` asserts 8 %), so neither the measurement nor a
determinate solve is off — the excess belongs to this truss's four pinned supports, which make it
indeterminate. Two things measured on the way and not asserted: with 0.1 kg bars (500:1 against the
weight) the bracket reads ~30 % low and never settles, the sequential-impulse mass-ratio limit; and
its readings swing about ±15–18 % around statics at every iteration count from 2 to 80 over a
2.5 s window, while the tip's sway falls with iterations (2.2 → 0.5 mm) — an earlier note here
said 40 iterations swung more (±25 %) than 10 (±3 %), which was ten steps' worth of samples, not a
property. The game sets its threshold to the forces it sees (30 kN).

### C16l. The sixteenth game: cars (2026-11-03, `pist`)

`demo/src/bin/pist.rs` is a three-car, three-lap race on a stadium track: the engine's own vehicle
model (`VehicleController` — suspension, Pacejka tyres, automatic gears), static barriers, a chase
camera, and a top-down minimap in the corner. Three gaps, all closed:

- ~~**Cars did not move under `FixedPhysics`.**~~ **Fixed 2026-11-03.** The fixed-step driver ran
  the rigid step only; vehicle, character and fighter controllers ran only in
  `GameplayPhysicsPlugin`, which nothing adds. A throttled car moved 8·10⁻¹² m in 2.5 s. Each step
  runs the controllers first now.
- ~~**A minimap blacked out the main view.**~~ **Fixed.** Any camera with a viewport switched the
  frame to the split-screen path, which skipped cameras without one — the primary included. The
  primary camera without a viewport is the full-target base now; a higher-order viewport draws
  over it.
- ~~**A car was four hand-written corners.**~~ **Closed.** `VehicleController::four_wheeled(track,
  wheelbase, radius)`, which also sets the tuning's track and wheelbase to where the wheels are —
  this game had left them at the defaults.

Not engine gaps, recorded so they are not re-chased: the opponents' driving is simple (aim at the
next point, slow for the sharpness two to four points ahead, back off a wall) and loses time on
the barriers; a 28 m turn holds about 60 km/h before the tyres let go, which is physics.

### C16k. The fifteenth game: units under a box (2026-11-03, `komutan`)

`demo/src/bin/komutan.rs` is a small real-time strategy skirmish: drag a box to select units,
right-click the ground to march them (a square formation around the point), and they fire at
enemies in range. Twelve blue against fourteen red; `NavGrid` routes all twenty-six round the
rocks. Three gaps, all closed:

- ~~**No world-to-screen projection.**~~ **Closed 2026-11-03.** The cursor had a ray into the
  world, the world had no way back to the screen — and a box selection has to project the units,
  a health bar has to know the pixel over its unit. `pointer::world_to_screen` /
  `Camera::world_to_screen` (`None` behind the camera; split-screen viewports honoured).
- ~~**The cursor ray came from a stale camera position.**~~ **Fixed.** It read the camera's
  `GlobalTransform`, a frame behind in game code (the origin, for a camera spawned this frame);
  found when `world_to_screen`'s example projected nothing. It reads `world_position` now.
- ~~**Agent separation was hard-coded.**~~ **Closed.** 1.5 m and a weight of 1.5 for every
  agent; a 1.2 m formation was pushed apart on arrival. `NavAgent::with_separation(radius,
  weight)`.

### C16j. The fourteenth game: a road that never ends (2026-11-03, `kosucu`)

`demo/src/bin/kosucu.rs` is a three-lane endless runner: switch lanes, jump the low barriers, slide
under the beams, take the coins; the speed climbs from 43 to 122 km/h and the best distance is
saved. The road is spawned ahead in 30 m chunks (a parent with ~25 children) and despawned behind
with `despawn_recursive` — the autopilot run spawns and kills ~75 entities a second, and the
process held 89 MB for three minutes. Swipes (`Input::swipe`), `Fog`, `SaveData`, `ScreenFade` and
synthesised sounds worked on the first try. Two gaps:

- ~~**The sky that lights the scene could not be set.**~~ **Closed 2026-11-03.** The game made the
  background and the fog a pale blue, and its gold coins came out grey: what a surface reflects and
  the ambient it gets came from the preset's baked sky, which no setting reached. `Renderer::sky`
  (row in §B).
- ~~**A child spawned this frame has no world position yet.**~~ **Closed 2026-11-03.** Its
  `GlobalTransform` is the default until propagation runs after the update, so a collision test
  against fresh obstacles would read them at the origin — where the player starts (caught while
  writing it; the first draft read local `Transform`s and kept its chunk roots at the origin).
  `world_matrix(world, entity)` / `world_position` walk `Parent` and compose this frame's
  `Transform` fields; the chunks sit at their own z now and the game reads them through it.

### C16i. The thirteenth game: a board that tilts (2026-11-03, `labirent`)

`demo/src/bin/labirent.rs` is a marble maze: tilt the board (keys or stick) and roll a real
dynamic ball between the walls, past the holes, to the green square; three levels, best times
saved. The board is one kinematic body whose collider is all its floor tiles and walls. Four
things, one of them a crash:

- ~~**A kinematic body could not be driven to a pose.**~~ **Closed 2026-11-03.** A game knows where
  the board should be, the body moves by its velocity, and under a fixed-step driver a velocity
  computed from the frame time turns it by the wrong amount (a frame runs 0–N steps).
  `KinematicTarget::new(position, rotation)`: each step sets the velocity that arrives in that step
  — what rests on the body is carried — and lands the body on the target exactly.
- ~~**No multi-part collider constructor.**~~ **Closed.** `Collider::compound([(Transform,
  Collider), …])`.
- ~~**A compound crashed the physics next to a CCD body.**~~ **Fixed.** The speculative contact
  passed it to GJK whole (debug panic; a zero support point in release). Kinematic bodies turn CCD
  on, so this board hit it on its first step.
- ~~**A ball on tile seams** lost half its speed~~ **Closed.** A box part's edge contact on a plane
  another part holds with a face contact is dropped (1.01 → 2.03 m of the slab's 2.21 m, CCD ball,
  two seconds). The last 8 % turned out not to be the seams: it was the **landing** — one
  speculative contact whose zero gap was written as `-0.0`, which the solver's `< 0.0` test reads
  as a resting contact, so it got friction at the ball's centre and stopped its slide down the
  slope dead. Strictly negative now, and speculative contacts carry no friction: 2.23 m. Two wrong
  guesses on the way, both recorded: "the speculative contact near a seam" (it fired once, at the
  start) and "its below-1 mm normal fallback" (changing it changed nothing). The game turns CCD
  off anyway, which its speeds allow.

### C16h. The twelfth game: things on screen that move (2026-11-03, `duello`)

`demo/src/bin/duello.rs` is a card duel against the computer: mana that grows each turn, a hand
refilled to five, cards dragged onto a portrait to play them (attacks on the opponent, shields
and heals on yourself), an opponent that plays back one card at a time. Everything is UI, and
everything moves (and a symbol needed a second font): cards slide from the deck into the hand one after another, follow the pointer
while dragged, fly to their target and fade when played, spring back when dropped in the wrong
place, and the rest of the hand slides over to close the gap; a hit portrait shakes. Two things:

- ~~**UI boxes could not be animated.**~~ **Closed 2026-11-03.** `Tween` moves a `Transform`; a
  UI box's place is its `Style`, recomputed every frame. `UiTransform { translation, opacity }`
  (after layout, children follow, hit testing follows, opacity inherited) and `UiTween` to ease
  it, on real time, with `Tween`'s own curves (`Ease`, moved to `gizmo-math` for it). The hand's
  slide is the FLIP pattern: set the new `inset`, set the translation to old − new, tween it to
  zero.
- ~~**A character the font lacks drew as a box.**~~ **Closed 2026-11-03.** The shield ⛨ is not in
  DejaVu Sans. `Renderer::add_font_fallback(face)`: a chain of faces tried per character; `duello`
  adds Noto Sans Symbols.

### C16g. The eleventh game: drag and drop (2026-11-03, `birlestir`)

`demo/src/bin/birlestir.rs` is a merge garden made of UI alone: a 6×5 field, **Üret** plants a
seed (energy that refills on a `Timer`), two equal things dragged together become the next stage,
orders on the right pay for a stage dropped on them, a bin takes what is not wanted, and the field
is saved. Four things:

- ~~**A drop could not be found.**~~ **Closed 2026-11-03.** The dragged item follows the pointer,
  so it is what is under the pointer; everything beneath reads `Interaction::None`, and the game
  had to hit-test every slot's box itself, minding layers, clipping and the item. `DropTarget`
  (`hovered_by`, `dropped`) and `Drag::over` / `dropped_on`: front layer first, then the innermost
  of nested targets; clipped, sizeless, and the dragged subtree never count. Four mutations, four
  failing tests.
- ~~**A slot painted over its own item.**~~ **Fixed 2026-11-03.** Boxes on one layer came out in
  archetype order; slot and item are in different tables. Tree order now (parent, then children
  in `Children` order).
- ~~**The UI does not scale with the window.**~~ **Closed 2026-11-03.** A field laid out for
  1280×720 did not fit the 472×492 window a tiling window manager gave it: `Val::Px` was always a
  window pixel. `UiScale::fit(w, h)` (or `fixed(k)`), a resource: layout in layout pixels, `Node`
  in window pixels, labels on boxes scaled with them. Four conversions, four failing mutations.
- ~~**No tooltip.**~~ **Closed 2026-11-03.** The game built its own — an absolute node moved to
  the pointer every frame, `Display::None` when nothing was hovered, the pointer divided by the
  `UiScale` by hand. `Tooltip::new(text)`: shown after resting `delay` seconds, drawn by the facade
  in the element's own font, kept in the window; focus alone, a press, leaving or empty text hide
  it. Tested in logic and on the GPU.

- ~~**No grid layout.**~~ **Closed 2026-11-03.** The six-column field was a wrapping row whose
  width was six cells and five gaps — the column count lived in that arithmetic. `Display::Grid`
  with `GridTracks::repeat(6, GridTrack::Px(84.0))`; taffy's grid had been compiled in and
  unreachable. Not modelled: named lines and areas, `minmax()`, auto-fill, `grid_auto_flow`.

### C16f. The tenth game: a world made of tiles (2026-11-03, `zindan`)

`demo/src/bin/zindan.rs` is a turn-based dungeon crawler: rooms and corridors generated per level,
a step per input (held, it walks on — `move_axis` + `HoldRepeat`), monsters that close in once they
see you, gold, stairs down behind a `ScreenFade`, health in a `ProgressBar`. Its world is one
`Tilemap`, drawn in one call. One thing was missing:

- ~~**A tile could not be tinted.**~~ **Closed 2026-11-03.** Fog of war — lit where you see,
  dim where you have been, dark elsewhere — is the first thing a tile game wants, and a `Tilemap`
  had one colour for the whole map: the game's first version drew a dimmed copy of every tile into
  the tileset. `Tilemap::set_tint(x, y, colour)` / `tint` / `clear_tints`, multiplied on top of the
  map's colour; a map that never tints allocates nothing for it, and a white tint on an untinted
  map does not either. Tested on the multiply, an untinted neighbour, the tint staying with its
  cell when the tile changes, and the clear; not applying it fails it.

(It also first hid out-of-sight monsters by moving them far away; `IsHidden` already hides
sprites, so it uses that.)

### C16e. The ninth game: time from the music (2026-11-03, `ritim`)

`demo/src/bin/ritim.rs` is a four-lane rhythm game: notes fall to a line, press the lane's key (or
its on-screen `VirtualButton`) as they cross it. The music is synthesised (120 BPM: kick, hi-hat,
bass) and the notes sit on its beats — so the game has to know where the *music* is, not where the
frame clock is: notes timed by the frame clock drift from what the speaker plays by the output
latency, every dropped frame and every stall, and the drift adds up.

- ~~**The playing position of a sound could not be read or moved.**~~ **Closed 2026-11-03.**
  rodio's players know it; the engine did not expose it. `AudioManager::position(id)` (seconds of
  the sound), `seek(id, secs)` and `duration(name)`.
- ~~**No sound in the engine could seek — and nothing said so.**~~ **Closed 2026-11-03.** The
  hardware test for `seek` went down three layers. The `Muffle` low-pass every sound is wrapped in
  did not forward `try_seek`; under looped sounds, rodio's `repeat_infinite` buffers its input and
  the buffer cannot seek; and decoders built with `Decoder::new` on a cursor do not know their
  length, so symphonia refuses random access. And rodio's `Player::try_seek` returns `Ok` when its
  feedback channel closes — the first hardware run "passed" with nothing moved. Now `Muffle`
  forwards the seek and clears its filter memory (no click from the old history), loops are the
  engine's own `Looping` source (decodes the same `Arc`ed bytes again at the end, wraps a seek into
  the sound's length), and every decoder is built through one `decode` that states the length and
  seekability. The proof is device-free — the decoder itself seeking in `Looping`'s test, the
  filter's memory in `Muffle`'s — because the hardware test cannot tell a real seek from rodio's
  `Ok`; it stays as the end-to-end check. Dropping the wrap or the memory reset fails them.

### C16d. The eighth game: animations in sequence (2026-11-03, `mucevher`)

`demo/src/bin/mucevher.rs` is a match-3 puzzle: swap two neighbours (drag, or click then click),
runs of three clear, the rest fall, new gems drop in, cascades chain. Its whole flow is
animations played one after another — swap, swap back or clear, fall, look again — and that needed
nothing new: a small state machine waiting on `Tween::is_finished`. One thing was missing:

- ~~**A tween could not wait.**~~ **Closed 2026-11-03.** Gems falling column by column, a moment
  apart, meant counting the start by hand. `Tween::with_delay` and `ColorTween::with_delay`: they
  hold, take their start **when they start** (not when added — something may have moved the entity
  meanwhile), and spend the part of the frame past the delay on themselves. Tested on both; taking
  the start at once, or dropping the remainder, fails them.

### C16c. Touch (2026-11-02)

~~**A finger did nothing.**~~ **Closed 2026-11-02.** The windowed loop had no `WindowEvent::Touch`
arm, so on a phone — the browser build runs on one — or a touchscreen laptop nothing could be
pressed at all, and `demo-web`'s canvas let the browser take a finger for a pan or a pinch anyway.
`Input::on_touch` records each finger (`Input::touches`: id, position, start, this frame's delta,
`just_started` / `just_ended`), and the **first finger down drives the mouse**: it places the cursor
without a flick in `mouse_delta`, holds the left button, drags the cursor and releases on lift — so
`Clicked`, `pointer_over_ui`, `gizmo::pointer` and every game written for the mouse work under a
finger unchanged. A second finger never takes the cursor over mid-gesture; focus loss lifts every
finger. `demo-web`'s canvas sets `touch-action: none`. Tested in `gizmo-core` without a device
(tap = click, a one-frame tap is not lost, drags, only the first finger is the mouse, focus loss,
an old recording without touches loads); removing the emulated press, letting every finger take
the mouse or skipping the per-frame roll each fails them. **Not yet tried on a real touchscreen.**
~~Still missing: gestures (pinch, swipe).~~ **Closed 2026-11-03:** `Input::pinch()` (the distance
between the first two fingers now over a frame ago), `two_finger_pan()` (their average delta) and
`swipe(min_px)` (a finger lifting at least that far from where it landed — distance, not speed).
Tested on spread, close, one finger left, a pan, a swipe, a short one, and a drag that came back;
measuring the pinch against the current distance, counting a lifting finger as down, or a finger
still down as a swipe each fails them. `kale` zooms with the wheel or a pinch and pans with two
fingers, and now places a tower on the **release** — on the press, every two-finger gesture began
by dropping one, since the first finger is the mouse.

~~**Two thumbs could not play.**~~ **Closed 2026-11-03.** Only the first finger drives the mouse, so
nothing built on `Interaction` could follow a second one — and a twin-stick game needs two.
`gizmo::ui::VirtualStick` and `VirtualButton` read `Input::touches` directly: each **claims the
finger that lands on it** and follows it until it lifts, even out of its box; a finger already
holding one control is never claimed by another; a tap inside one frame is a press and a release.
A stick is floating (centred where the thumb lands), `y` up like a gamepad's, clamped to the unit
circle; the engine draws its knob. With no fingers on the screen they answer to the mouse.
Tested with two fingers at once, landings outside and slides over, and the mouse; dropping the
`y` flip, releasing on leaving the box, or ignoring the fingers each fails them. `nisanci` grows
two sticks on its first touch (move; aim and fire) and stops taking the emulated mouse as fire.

### C17. Music that changes (2026-11-02, `menu`)

~~**No fades, and a game's own `AudioManager` was never ticked.**~~ **Closed 2026-11-02.**
`AudioManager::update` — which collects finished sounds and applies a moved bus or master gain to
the ones still playing — had one caller, the opt-in 3D audio system. A game that inserted its own
manager (every demo with sound) leaked a sink and a mixer route per sound played, and the menu's
volume button reached only sounds started after it. And there was no way to change a sound's
volume over time, so a menu-to-level music switch was a cut. Now `gizmo-app`'s frame loop calls
`AudioManager::tick(frame_dt)` on the resource every frame; `fade_to(id, factor, secs)`,
`fade_out(id, secs)` (stops at the end) and `crossfade(from, name, secs)` (the new track looping
on the music bus from silence). The fade is its own factor in the mixer's composition, so the 3D
system rewriting a spatial sound's volume each frame, or a slider moving mid-fade, does not undo
it. Tested on the mixer without a device (linear, lands exactly, survives volume writes, stops
once, reverses without a jump); the loop's half on hardware. `menu` has music, crossfaded by
Start.

---

## D. Game-path / editor-path asymmetries

`render_parity` exists precisely to catch these. It was **red on one for two days** and is green
again as of 2026-08-24. Recording the whole set, because they are one family:

1. ~~**`alpha_cutoff` is known to the game path only.**~~ **CLOSED 2026-08-24.** The editor draw
   path never learnt it: it built its instances without the threshold and chose its bucket from
   `is_transparent` alone, so a pierced fence was solid in the viewport and pierced in the game.
   Measured before the fix — **11 654 of 16 384** pixels that every texel's alpha said should have
   been discarded — by `a_cut_out_material_cuts_in_the_editor_viewport`, which is binary by
   construction: a white texture and albedo alpha 0.3 against a 0.5 cutoff means *everything* is
   under the threshold, so the correct frame is empty. The fix is the threshold on the instance and
   `draws_blended` made `pub`, because "which bucket" is a decision and two loops answering it
   separately is exactly what `gizmo-renderer::routing` exists to prevent. `render_parity`'s
   inventory went green with it, and the workspace suite with it: **244 binaries, 2 470 tests, 0
   failures** — the first fully green run since `05fcbff7` on 2026-08-22.
2. ~~**`RenderStats` (draw calls / triangles / instances) is filled by `gizmo-studio` only.**~~
   **CLOSED 2026-11-02**, with its consumer: `default_render_pass` publishes it per view, counted
   as the editor counts (one draw call per batch with camera-visible instances, triangles × their
   instances, shadow-only casters excluded; GPU memory sampled about once a second), and
   `many_cubes` — which had to infer its batch count from how it built the scene — now shows the
   engine's: `1 draw call · 7443 instances · 89316 triangles` at frame 120. `render_parity` had
   declined this as "stats with no consumer"; the demo is the consumer, and the entry is gone.
   Tested: three cubes sharing a mesh and material read `(1, 3, 36)`, and counting the
   shadow-only fourth fails the test. Under split screen it holds the last view drawn.
3. ~~**The grid pipeline is editor-only.**~~ **CLOSED 2026-11-02.** `route(Grid)` is forward
   now (`instance_flag` 1.0, `skips_deferred`), `is_grid` rides the game path's batch key and draw
   item, and the forward pass binds `grid_pipeline` for it ahead of the unlit branch. Before, a
   grid material drew in the game as a plain opaque PBR plane — measured in `infinite_grid`.
   Tested from above: the lower half of the frame has 0 sharp edges for a PBR plane and 166 for
   the grid; dropping the forward branch sends the grid to the unlit pipeline and back to 0. The
   editor still hides its grid in play mode; a game that puts one in its world gets it drawn.
4. ~~**`Material::emissive` is dead in the deferred path.**~~ **CLOSED 2026-11-02.** The
   G-buffer read emissive from the *material uniform* (which only glTF loading fills) and added it
   to the 8-bit albedo — lit, and clamped at 1 — while game code writes the per-instance field,
   which only `baked_lit.wgsl` read; the forward shader read neither. Now the G-buffer has an RT5
   (`emissive`, `Rgba16Float`, 44 bytes per sample in all) carrying the instance's emissive plus
   glTF's factor × texture, and the lighting pass adds it unlit, in HDR, before fog; the forward
   shader adds the instance's too. Measured on a black cube with no light in the scene (centre
   red, emissive 0 / 1 / 4): opaque 6.0 / 231.3 / 251.8, transparent 0.0 / 231.6 / 251.9 — the two
   paths agree, and 4 reads brighter than 1. The lighting pass ignoring RT5, the forward shader
   ignoring the field, and the emission clamped to 1 each fail the test. A glTF emissive is
   therefore brighter than before (no longer multiplied down by lighting, no longer clamped).
5. ~~**`vignette` is reachable only through the `PostProcess` component.**~~ **CLOSED 2026-08-23.**
   The `Renderer` fallback struct had no such field, so a camera without the component was stuck
   with `PostProcessUniforms::default()`'s **0.25** — a vignette it never asked for and could not
   switch off. `Renderer::vignette_intensity` now exists and the fallback branch reads it; the
   default is still `0.25`, so **no pixel changed** — the value merely became reachable.

   Verified in `color_grading`, one capture batch, size-checked: component `vignette = 0`
   gives corner/centre **1.095** and fallback `renderer = 0.0` gives **1.095**; component `0.9`
   gives **0.608** and fallback `0.9` gives **0.608**.
6. ~~**Vertex colour is read only by `baked_lit.wgsl`.**~~ **CLOSED 2026-11-02:** the G-buffer
   multiplies albedo by it at face value — `baked_lit.wgsl`'s rule, and since 2026-11-02 the
   forward shader's too, which used to rewrite a near-black colour to white (un-painting a surface
   authored black).
   Every engine primitive and uncoloured import is white already, so only meshes that colour their
   vertices change — and they now look in the game as they did in the editor. Tested on a
   red/green/blue triangle under a white PBR material (red corner 216.7 146.7 179.5, green 146.3
   217.1 179.6, blue 159.2 159.6 223.5; ignoring the attribute gives three greys of 244). The
   original finding: the attribute is always present — the one
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
   attaches, nothing reads it, the entity never dies, and ~~**nothing warns**~~ — **it warns since
   2026-11-02:** `default_render_pass` calls `lifetime::warn_if_inert` every frame, which logs once
   per world when a `DespawnAfter` / `DespawnBelowY` is attached and the plugin's `LifetimeActive`
   marker is absent. The plugin stays opt-in on purpose: installing it behind a user's back would
   run the system twice for anyone who also adds it. Found in
   `delayed_commands` only because the demo ran the engine's path beside a hand-built one and saw
   the engine side grow 5 → 10 → 15 while the hand-built side drained correctly.

10. ~~**`MaterialType::Skybox` draws nothing in the game path — cause unknown.**~~ **No longer
    reproduces (2026-11-02)**, so it is closed without a named cause: whichever later change fixed
    it did so without knowing. A sky cube around the camera now repaints every colour byte of a
    128² frame (`a_skybox_draws_in_the_game_path`, centre (59, 61, 64) → (243, 249, 252)), and
    `atmosphere` with and without its cube differs in 10066 pixels at frame 90 — all of them sky,
    the light blue of `sky.wgsl` against the deferred environment's orange. The test pins it.
    The original record, measured in
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
three — `ambient` (**documented** as PBR-path-excluded, intentional), `alpha_cutoff` (item 1, and
its editor half closed 2026-08-24)
and `emissive` (item 4, the only undocumented one). No further silent fields.

---

## E. Suggested order of work

> Sequenced by capability. For the *shape* the surface should take as these land — layered access,
> and the rule that every convenience must name its escape hatch — see `docs/API_DEPTH.md`. Several
> entries below are one unlock away from each other there. Per-pass uniforms — expected to close
> multi-viewport, planar reflections and reflection probes on its own — landed 2026-08-23 as
> `SceneView` and closed the camera half of all three; the shadow cascades and cluster table are
> still shared per encoder, and that is what the three now wait on (A5b, item 3). Multi-viewport
> itself closed 2026-11-01 by a submission per view instead (A5b).

1. ~~**Text / font rendering (A2).**~~ **DONE 2026-08-24, both halves.** `gizmo-renderer::text` —
   fonts, a glyph atlas, layout — a `Text` component in two spaces drawn by one pass both hosts
   call, and `gizmo-ui`'s `Node` + `BackgroundColor` painted from it. Details and the measurements
   are in A2.

   Three things the plan got wrong, kept because being wrong in writing is the useful part:

   - **The component belongs in `gizmo-renderer::components`, not `gizmo-ui`.** `gizmo-ui` sits
     *above* `gizmo-app`, so a `Text` there is invisible to both draw paths. The direction the
     layering allows is the other one — `gizmo-ui` can see renderer types — which is why the
     *bridge* is in the facade and the *component* is in the renderer.
   - **The draw modes did not want the existing `BatchKey`.** A glyph is an instance, not a mesh:
     text is one instance buffer and two draw calls, not a batch per atlas texture.
   - **`BackgroundColor` needed no draw path of its own.** The atlas carries a fully-opaque patch,
     so a solid quad is a glyph quad whose coverage is 1 — same pipeline, same buffer.

   Two constraints the sweep named did hold: the wasm target is a separate CI gate and the
   rasteriser builds there, and the golden render test is the acceptance, because a picture is the
   only thing that can say a glyph landed where it should.

   *Its landing sites were right:* tracked as **M7.6** in `ENGINE.md` §3 (Phase 7) — whose z-index
   and widget halves are still open — and `gizmo-ui`'s own crate docs named the same site,
   *"expect the component set to change when rendering lands"*.

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
3. ~~**`Visibility`, `Local<T>`, `or_else`, default query filters (C).**~~ **ALL FOUR DONE
   2026-08-24.** Filed as "small, self-contained, each removes a recurring papercut". **Half of
   that held**: two were papercuts, `Local<T>` was a change to the core of the ECS, and the
   `Visibility` entry was simply wrong about what the engine already did. (`Transform`'s direction
   helpers were the first of this group and are done.)

   - ~~**`or_else`**~~ **DONE 2026-08-24** — `or`/`and`/`not` on the typed path, so a combined
     condition keeps its access declaration. Building them found that the typed path had never been
     *callable* with a parameter closure; see C1.
   - ~~**`Visibility`**~~ **The entry was wrong.** `IsHidden` was already the engine's hide, on both
     paths since 2026-08-19, and already a full one. What was missing was inheritance, and that
     landed 2026-08-24. See C1 for the measurements that corrected it.
   - ~~**`Local<T>`**~~ **DONE 2026-08-24** — and it *was* a change to the core of the ECS rather
     than a papercut, exactly as the pricing above said: `SystemParam` grew a `type State` and
     `fetch` a third argument, reaching nine impls, three macro families and four call sites
     outside `gizmo-core`. It is in its own commit for that reason.
   - ~~**Default query filters**~~ **DONE 2026-08-24 — and worth reading C1 for how.** The
     pricing ("seven entry points and an opt-out each") described the surface rather than the
     machinery: seventeen public `Query` methods share one archetype list, so the filter is one
     predicate. The hard question was *which constructor*, the first answer was wrong, and an
     adversarial review caught it before it shipped — filtering `query_unchecked` would have
     blanked the editor inspector, frozen transform subtrees and made physics **destroy** a
     disabled body. C1 carries the four traced consequences, because they are now the justification
     for where the boundary sits.

   With this, **§E's list is closed down to item 6** — the 2-D pipeline, which the list itself calls
   arguably a separate project. Item 2 is closed *as scoped*: everything that existed and was
   unwired is wired, and what B still lists is the narrower end of each table, which was never
   written and so was never in item 2's scope.
4. ~~**Extrusion machinery (A4).**~~ **DONE 2026-08-24** — one feature, 15 shapes, and the count
   held: `create_extrusion` + `create_sweep` over a shared outline module. The part §E did not say
   is where the work was: cap triangulation, which is a fan for a convex outline and ear clipping
   for anything else. See A4.
5. ~~**User materials (A3).**~~ **DONE 2026-08-23** — `MaterialType::Custom(MaterialId)`,
   forward-only. The G-buffer budget it was said to depend on settled the question instead of
   blocking it: 28 of 32 bytes are spent, so a custom material cannot bring a target and declares
   itself forward. `routing.rs` stayed exhaustive and produced exactly one compile error.
6. ~~**2D pipeline (A1).**~~ **The drawing half DONE 2026-11-01** — "largest of all; arguably a
   separate project" turned out to be one component and one pass, because the 2D camera and the
   2D transform stack already existed as an orthographic camera and a plane. What remains is the
   2D *game* layer around it, measured by building one (§C9) — and every item that game listed
   was closed 2026-11-01: the window-fitting orthographic camera, sprite↔collider sizing, the
   sprite-animation component, the tilemap (drawing, culling, collision) and 2D shapes on planar
   bodies.

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
| The **editor** draw path ignored it too — no threshold on its instances, and its bucket chosen from `is_transparent` alone. Found by `render_parity`'s inventory on 2026-08-22 and left standing; measured 2026-08-24 at 11 654/16 384 pixels that should have been discarded | Threshold on the instance + `draws_blended` shared between the two loops, and a studio pixel test that is empty when it works |
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

Reported, then closed 2026-11-07: deferred `alpha_cutoff` needed the z-prepass to gain a fragment stage, and it did, for a draw that discards;
`Material::emissive` is dead in the deferred path (D4). ~~the editor path never learnt
`alpha_cutoff`~~ — **closed 2026-08-24, see D1.** The two were one sentence and are not one
problem: the editor draws *forward*, where the threshold only had to reach the instance, while the
deferred route needed a fragment stage.
