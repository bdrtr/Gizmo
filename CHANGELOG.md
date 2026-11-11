# Changelog

## crates.io — three names from the previous repository closed out (2026-08-23)

`gizmo-physics`, `gizmo-network` and `gizmo-studio` were published from the old self-hosted
repository and were left behind by the move here. All three still carried
`repository = git.bedirkaraabali.com/bdrtr/Yelbegen-engine` and — word for word — the same
description as the live crates, so `cargo add gizmo-physics` handed out a 0.1.2 from May 2026
with nothing to say it was the wrong line.

The cleanup after the move *was* done for the names that survived it: `gizmo-engine` and
`gizmo-core` both have their pre-move versions yanked. These three were simply missed, and the
repository kept no record of the old generation at all — "Yelbegen" appears nowhere in it, which
is why nobody noticed. `docs/AUDIT-2026-08.md` had caught half of it, flagging a README claim
about a `gizmo-physics` crate as wrong because no such crate is in the workspace; what it did not
check was whether the *name* was still live on crates.io.

Each got a final release that says where it went, and every earlier version was yanked:

| name | tombstone | points at |
|---|---|---|
| `gizmo-physics` | 0.1.3 | `gizmo-physics-core`, `-rigid`, `-dynamics`, `-soft` |
| `gizmo-network` | 0.1.8 | `gizmo-net` |
| `gizmo-studio` | 0.1.8 | nothing — the editor ships with the repository, `publish = false` |

The tombstones stay unyanked on purpose. A crate with every version yanked still shows up in
search but answers nothing; this way the page answers the question the searcher actually has.
Sources are in `deprecated/`, excluded from the workspace so they can never enter the engine's
dependency graph.

Verified end to end: `cargo add gizmo-physics` now resolves to 0.1.3, building against it emits
`use of deprecated crate gizmo_physics: DEPRECATED — split into …`, and pinning `=0.1.2` fails
with `version 0.1.2 is yanked`.

All notable changes to the Gizmo engine are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims
to follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Versioning note.** `0.2.0` ships the whole workspace at one uniform `0.x`
> version on purpose: it bundles the large 1.0-readiness effort and the breaking
> graphics-stack upgrade, but **defers the hard `1.0` promise** to gain soak time
> on the new `wgpu`/`winit`/`egui` stack. The *staged* `1.0` model — promoting the
> dependency-light **Stage A** core (`gizmo-math`, `gizmo-core`, the
> `gizmo-physics-*` crates, `gizmo-scene`, `gizmo-net`, `gizmo-audio`, `gizmo-ai`)
> to `1.x` while the graphics/integration **Stage B** crates stay on `0.y` — is
> documented in [`docs/ENGINE.md`](docs/ENGINE.md) and remains the planned path for a
> later release.

## [Unreleased]

### Added

- **A local light's shadow is as soft as its source is large.** A sphere-sourced point or spot light (`source_radius`),
  a tube and a rectangle light throw a penumbra as wide as their size and the gap between caster and receiver make it —
  percentage-closer soft shadows, as the sun's angular radius gives the sun's. A point source keeps its sharp edge. The
  local layers draw every caster by both faces for it (a closed mesh's back faces put the blocker at its far side), and
  each filter tap compares against the receiver's own plane, taken from the pixel's neighbours.
- **A shadow pass draws the casters in its own frustum**, not every caster of the frame (`gizmo_renderer::shadow_culling`):
  a batch's instances are put in spatial order and cut into runs of 64 with a box each, and each cascade and local layer
  draws the runs it meets — in the game and the editor viewport alike. On 2 500 objects under 16 shadowed spots and the
  sun, shadow instances 50 020 → 13 607 and the frame 23.0 → 14.1 ms, every frame byte for byte the one drawn without it.
- **How many shadow passes a frame records, and what they draw**: `RenderStats::shadow_passes` (the sun's four cascades
  when there is a sun, one per local shadow layer) and `shadow_instances`, each with a row in the editor's viewport
  overlay.

- **Every local light casts shadows, when it asks to.** `PointLight`, `SpotLight` and `RectLight` have
  `casts_shadows` (and `with_shadows()`; off by default, `#[serde(default)]`), and so do `PointLightBundle` and
  `SpotLightBundle`. The lights that cast share one depth texture array (`gizmo_renderer::local_shadows`): a spot whose
  cone is no wider than 80° takes one layer down it, a point or rectangle light — or a wider spot — six, a cube around it.
  Layers go to the lights in the frame's rank order until 32 are spent, and the array grows to what a frame asks for. Both
  lit shaders look a light's shadow up through the matrix it was drawn with (`local_shadow_visibility` in `common.wgsl`),
  so it falls in the deferred pass, on glass and in the editor viewport alike; off-screen casters inside a light's layers
  are kept for them. Only the sun and one point light could cast before. The editor's light sections have a checkbox for
  it (and the point light's the range row it lacked), and the `lighting` and `spotlight` demos' lamps cast (the latter's
  documentation said its sixteen cones did).
  Desktop only, as every shadow is.

- **A game's own surface, drawn deferred.** `Renderer::add_surface_material(label, wgsl)` composes a game's
  `fn material_surface(input: SurfaceInput) -> Surface` into the engine's G-buffer and lit forward shaders, in place of the
  engine's material, and `MaterialType::CustomSurface(id)` draws it: written to the G-buffer as `Pbr` is — so the deferred
  lighting, SSAO, SSR, SSGI, decals and the velocity target all see it — and lit by the forward shader in the browser, on a
  phone, in the editor viewport, transparent and as glass. `gizmo::surface` holds `Surface` and `SurfaceInput` (position, UV,
  vertex colour, the geometric frame, the instance's material values, the face, the pixel, the clock, the camera), and
  `gizmo::material::standard_surface` is what `Pbr` draws, to start from. A WGSL error is `SurfaceError::Compose`. Measured: a
  surface returning `standard_surface(input)` draws `Pbr`'s frame to the byte. A material of the game's own could only be a
  forward pipeline (`MaterialType::Custom`) until now. **Breaking, mildly:** `pipeline::LitLobes` is `LitSet`; the engine's
  material moved from `gizmo::surface` to `gizmo::material` (`evaluate_surface`, `surface_uv`, the group-1 bindings) for a
  shader that imported it.

- **A system's body changed while the game runs.** The default-off `hotpatching` feature (`gizmo-engine`, `gizmo-app`,
  `gizmo-core`) calls every system, run condition, `pipe` stage, observer, hook and `DynamicSystem` body through subsecond's jump
  table, and connects the app to the Dioxus CLI; `dx serve --hot-patch` then swaps a body without a restart. Verified with
  `demo/src/bin/hot_systems.rs` and `dioxus-cli` 0.7.10: the patch took 1.7 s and the world kept its state. Debug builds, the
  game's own crate, bodies but not signatures — subsecond's limits, in `gizmo::core::hotpatch`. **Breaking, mildly:**
  `World::register_on_add`, `register_on_remove`, `register_on_set`, `register_on_replace` and `register_despawn_hook` take the
  closure (`impl FnMut(&mut World, Entity)`) instead of a `Box<dyn …>`, so the closure keeps the type a patch needs; a
  `Box::new(|w, e| …)` written out still compiles only where its parameters are annotated — drop the `Box::new`.

- **A query term can be written outside `gizmo-core`.** `WorldQuery` and `ReadOnlyQuery` were sealed; they are `unsafe trait`s
  now, as in Bevy, with the contract on them, `query::declare_access` for a term's `check_aliasing`, and `World::sparse_set(key)`
  for one that reads a sparse component. `tests/downstream_query.rs` writes Bevy's `Has<T>`. `FetchComponent` stays sealed: it is
  the shared half of `&T`, `&mut T` and `Option<..>`. **Breaking** for nothing outside the crate, which could not implement them.

- **Components defined while the program runs.** `World::register_component(ComponentDescriptor::new("Health", layout))` returns a
  `ComponentKey`; `insert_by_key` / `remove_by_key` put a value on an entity and take it off, through the same path
  `add_component` / `remove_component` take; `DynamicQuery::builder().read(a).write(b).without(c).changed(d)` iterates by key, types
  and runtime components mixed; `DynamicSystem` runs queries in a schedule with their access declared. Drop and clone glue, sparse
  storage and immutability are the descriptor's to choose, and hooks reach one through `World::component_hooks_mut(key)`.
  **Breaking:** `World::clone_entity` returns `Result<Vec<Entity>, CloneEntityError>` — it refuses an entity carrying a component
  that owns something and cannot be cloned, and says which; `ComponentInfo::of_type_id`, which made up a zero-sized info for a
  type it did not know, is gone, and `ComponentInfo` has a `mutable` field.

- **Latin lies on its side in a column.** `Text::in_columns` sets Han, kana and Hangul standing and everything else a quarter turn
  clockwise, as CSS's `text-orientation: mixed` does, on the screen and in the world. `.upright()` stands everything, which is what
  columns did until now. **Looks change** for Latin in a column.

- **`Schedule::set_executor(ExecutorKind::Sequential)` and `Stepping`.** A batch's systems one at a time in the order they were
  added, for a frame that must repeat exactly; and a resource that holds every schedule until told to go on, a frame or a system at
  a time, without losing a change made during the hold.

- **Plurals by CLDR's rules.** `Locale::format_count` and `LocalizedText::with_count` pick `zero`, `one`, `two`, `few`, `many` or
  `other` as the language asks: Russian's 1, 21 and 2, 22 and 5, 11, Arabic's six, Polish's three. A table need only write the
  forms it uses. The browser build grows by what the rules weigh; see the commit.

- **An app reads as a Bevy one does.** `App::add_systems(Startup, setup)`, `add_systems(Update, (a, b).chain())`,
  `add_systems(FixedUpdate, (c, d))`, `insert_resource`, `init_resource::<T>()`, `add_plugins((a, b))`, on both `App`s, with
  `Startup`, `Update` and `FixedUpdate` in the prelude. A tuple of systems may now hold configured ones (`a.after(..)`). The older
  names remain as the one-at-a-time form.

- **An image as the sky: `Renderer::set_environment_map`.** `EnvironmentMap::from_hdr` reads a Radiance `.hdr` panorama,
  `from_equirectangular` takes pixels, `with_intensity` and `with_rotation` shape it. It lights the scene, is reflected in it, and
  stands behind it, on the desktop path and in the studio viewport. It takes a slot of the reflection probes' array, so a scene
  has three probes beside one. Not on the web, as probes are not.

- **`FogMode`.** `Fog::squared()` closes in as the square of the thickness, `Fog::linear(color, start, end)` is a straight ramp.
- **`Renderer::bloom_softness` and `bloom_energy_conserving`.** How far below its threshold bloom starts, and whether the glow
  comes out of the light that made it rather than adding to it.
- **`TonemapCurve::ReinhardLuminance`.** Bright saturated light keeps its hue.

- **`query_data!`: a named query term.** `query_data! { pub mut struct Target -> TargetItem { pub health: &'static mut Health, .. } }`
  and then `Query<Target>`, `target.health`. What Bevy's `#[derive(QueryData)]` does; the item's name is written out because a
  declarative macro cannot make one. Without `mut` it is read-only, and a `&'static mut` term in it does not compile. Everything
  goes through the tuple of its fields, so the scheduler sees the same access.

- **`add_startup_system`, on both `App`s and `gizmo_core::system`.** A system that runs once before the first frame, with a `Query`
  and `Commands` and the rest, which a setup closure cannot be. After the initial state's `on_enter`, before the first fixed step.

- **`LodGroup::with_crossfade(metres)`.** A level change dissolves over a band around its far edge instead of popping: the two
  levels split the pixels between them by a screen-space pattern, so no pixel gets both or neither and nothing blends. After the
  last level, the entity fades out and then is gone. Both draw loops, lit and unlit.

- **`Material::with_clear_coat_roughness`.** From the hard lacquer that every coat has been (0.08) up to 1: worn paint, satin.
  Both paths, with no new render target. A coat was never near-binary, as the docs said: that was a highlight saturating the tone
  curve. Dim enough to read, 0.2 adds 72 and 0.8 adds 217.

- **Depth of field through a lens: `PostProcess::with_lens(focus, f_stops, max_blur)`.** A thin lens, with the focal length from
  the sensor (`dof_sensor_height`) and the camera's field of view, so a long lens is shallower, as is a wide aperture.
  `dof_max_depth` limits what a sky counts as. The focus band stays. **Looks change** for any camera with a blur: the radius is
  now in texels of the frame rather than fractions of a 1920×1080 one, 24 taps replace 8, a sharp thing no longer glows on
  the blurred background behind it, and a blurred foreground has a soft edge over what is in focus behind it.

- **`SsaoState::set_quality`.** 8, 16 (the default), 32 or 64 samples a pixel.

- **`DespawnOnEnter(S::X)`.** The other half of `DespawnOnExit`: the entity goes, with its children, when `X` is entered, ahead
  of `X`'s `on_enter` hooks. A loading screen, whichever state it was spawned in.

- **Wireframes per entity: `Wireframe`, `NoWireframe`, `WireframeColor`.** The marker draws one entity's lines with
  `WireframeConfig::global` off, the opt-out keeps one out with it on, and the colour is that entity's, over
  `WireframeConfig::color`. The studio viewport draws them too, and honours `global`, which it had not. A line in the lit colour no
  longer carries reflection-probe light: the pipeline's group 5 is the colour now.

- **Glass: `Material::with_transmission` and `with_thickness`.** A surface lets light through instead of scattering it: tinted
  by its albedo, blurred by its roughness, bent by its index of refraction over its thickness, with the highlight and the
  reflection still on top. It works on both render paths and in the studio viewport. With glass in view the frame is drawn in
  two passes and copied between them, with a mip chain for the blur; without any, there is one pass, no copy and no texture.
  `KHR_materials_transmission` and `KHR_materials_volume` reach it, without their textures. It is screen-space: only what is
  on screen shows through, one pane does not show through another (the nearest wins), and blended surfaces and particles
  behind glass are not seen through it. The attenuation colour over distance is not there; albedo is the tint.

- **`Material::reflectance` and `with_ior`.** How much a non-metal mirrors head on: 0.04 unless a material says otherwise,
  `with_ior(1.33)` for water, `with_ior(2.42)` for diamond, `with_ior(1.0)` for no highlight at all. It moves the highlight
  and the environment reflection on both render paths, and the Fresnel of screen-space reflections, which read a literal
  0.04. A glTF file's `KHR_materials_ior` and `KHR_materials_specular` reach it; the specular colour contributes its
  luminance and its hue is lost, and the extension's textures are not read. No new render target: the magnitude of the
  channel that held `receives_shadows`, whose sign now holds it. A material that sets nothing draws as before.

- **Sprites have mipmaps and can be packed.** A `SpriteFilter::Linear` image has mipmaps, so a large image drawn small
  stops aliasing, built weighted by alpha and in linear light so a soft edge does not go dark and a checker keeps its
  brightness. `Renderer::load_sprite_atlas` packs a set into one texture with edges repeated into a margin, and sprites of
  it share a draw call however they are ordered: red, blue, red cost 3 apart and 1 packed. **Looks change:** a `Linear`
  sprite shrunk below its size is now smoother, and a `Linear` sheet shrunk far below its size can bleed between cells;
  `Nearest` has no mipmaps.

- **`a.pipe(b)` hands a value over.** What `a` returns arrives as `b`'s first parameter, `In<T>`; a pipe is one system to
  the scheduler, with the union of what its halves touch, each half keeping its own `Local`s, and it can be piped again.
  **Breaks:** what was called `pipe`, which handed nothing over, is now `then`.

- **Custom relationships.** `relationship!(struct Likes -> LikedBy;)` and `World::register_relationship::<Likes>()`: the
  reverse index stays right through insert, overwrite, remove, despawn and clone. `register_relationship_with` and
  `OnTargetDespawn::Despawn` make a target take its sources with it.

- **Immutable components.** `impl_component!(T; immutable)`, and `&mut T` in a query does not compile.

- **A system parameter can be written outside `gizmo-core`.** `SystemParam` was sealed; it is `unsafe trait SystemParam` now, as
  in Bevy, with the contract on the trait: `get_access_info` declares everything `fetch` touches. **Breaking** for nobody outside
  the crate, since nobody could implement it; `system_param!` still needs no `unsafe`.

### Changed

- **The sun's cascades draw a solid by its front faces** — what the sun meets — where they drew its back faces, so a
  soft shadow measures its penumbra from the side the light meets: a 1.2 m cube and a 5 cm slab with the same top cast
  the same soft edge (by back faces the cube's was half as wide), a 3 m pillar's penumbra reads 18 / 103 / 236 pixels at
  0 / 0.02 / 0.06 rad (18 / 91 / 196), and the cascades hold a 3 cm step's whole shadow (660 dark pixels against 363).
  The lookup's biases and the receiver's plane keep a lit face off its own depth: with the sun 17°–72° off overhead,
  sharp or soft, no floor shadows itself by 2 levels of 255 — a slab on every lit path, a plane, a normal-mapped plane —
  and no cube does.
  `SceneState::shadow_pipeline_for(front_faces, cuts)` and `SurfaceMaterial::shadow_pipeline(front_faces)` take the
  closed-mesh choice by its new name. A local light's layers still draw every caster by both faces.
- **A frame with no sun draws no cascades.** Every lookup of the sun's cascades asks for the sun first, and a sunless
  frame drew all four anyway, every caster in each, and kept objects off screen for them. `SceneSetup::draws_cascades`
  and `SceneSetup::shadow_view_projs` decide it for both paths, and `SceneSetupInputs::shadow_maps` (was
  `local_shadows`) says whether a path draws shadow maps at all: the forward-only path — a browser, a phone — kept
  off-screen objects for the sun's cascades it never draws. `ShadowCaster` and `SceneSetupInputs::shadow_caster` are
  gone: the editor fitted the cascades to its first light when there was no sun, and nothing ever read them — a light
  that should cast says so with `casts_shadows`, in the viewport as in the game.
- **`Renderer::point_shadows_enabled` is gone**, with `ShadowFrame::{point_caster, point_shadows_enabled}` and
  `SceneSetupInputs::point_shadows_enabled` (now `local_shadows`, whether a path draws the local shadow layers): a light
  says for itself whether it casts (`casts_shadows`). `SceneUniforms` carries the local shadow layers and each light's entry
  in them (3 216 bytes fixed and 68 a light, 20 624 in all; the uniform floor now caps the light array at 916), and
  `cascade_params.w` is unused. The shadow group's binding 2 is the local shadow array (`texture_depth_2d_array`), not a
  cube. `SceneState::{point_shadow_*}` became `local_shadow_maps`, `local_shadow_uniform_buffers` and
  `local_shadow_pass_bind_groups`, with `ensure_local_shadow_layers`.

### Fixed

- **A rough surface on the web and on a phone took the wrong environment.** The forward-only path reads the presets
  from a cube (`env_cube`) whose mip for a roughness of 1 was 4 texels across, where the presets keep lobes about 20°
  wide: a matte floor lit by the environment alone read 88.76 against the desktop's 66.23, Sunset Gold's metal one 94.96
  against 135.95, and every roughness between was off by up to 40. The sky's cube stops at 16 texels (`sky_levels`) and
  the shader takes the roughness across the cube's own mips: within 3.1 of the desktop under every preset, matte and
  metal. The memory is what it was and the bake is 24 passes where it was 36; a map's cube still goes down to 4 as a
  probe's slot does.
- **A moving unlit or baked-lit object had no motion of its own for TAA and motion blur.** Both read the G-buffer's
  velocity, which a surface drawn forward does not write, so its pixels kept the motion of what stood behind: an unlit cube
  moved 30 cm read 0 where a deferred one reads −0.0764, TAA reprojected it as if it stood still and motion blur left it
  sharp. After the forward pass each opaque surface drawn forward is drawn again through the G-buffer's vertex stage
  into the velocity's `xy` alone, where the forward pass left the depth (`fs_motion`, `DeferredState::motion_pipelines`):
  −0.0764 unlit and baked-lit, and nothing from one behind a still wall. Not water, a mirror or a game's own pipeline.
- **A pane let through all of the volumetric air behind it.** A surface that blends writes neither the G-buffer nor
  depth, so the march ran on through it and its air was added over everything: a lamp a metre behind a pane 98 % opaque
  moved 3 926 242 of the frame against 2 381 984 through an opaque pane. The nearest surface that blends is drawn alone,
  depth only, ahead of them (`VolumetricState::veil_depth_view`, through the G-buffer's prepass), and the march splits
  the air there (`VolumetricState::behind_view`): the air behind is added ahead of what blends, which shows it as much as
  it lets through (2 345 837), the air in front over everything (3 720 388 against 3 729 351, the lamp in front). The
  point-light falloff test draws its volumetric light again: a transparent floor and an opaque one under a lamp read
  227.94 and 228.45 with it (229.55 before).
- **SSR, SSGI and the volumetric march reached through what was drawn forward.** All three describe the G-buffer's
  surface and are added after the forward pass, and an unlit or baked-lit surface, or a game's own pipeline, writes depth
  and no G-buffer: an opaque unlit panel in front of a polished red cube on a floor took 12 137 and 18 569 of the 96 591
  and 100 681 SSR and SSGI move on those pixels (the cube's reflection, the floor's and the cube's bounce light), and the volumetric march went on through an unlit wall to
  the floor and sky behind it (1 275.0 against 479.9 for the same wall drawn deferred). A pass after the forward one
  writes how much of the G-buffer's surface the frame's depth shows (`DeferredState::seen_view`, `seen.wgsl`), SSR and
  SSGI are added in proportion to it, and the march stops where the depth says: none over the panel, 480.0 both ways.
  `DeferredState::depth_view` keeps the frame's depth for the passes that read it. And SSR and SSGI are added between the
  opaque world and what blends over it — glass, transparent surfaces, particles, the fluid — which shows them as much as
  it lets through: a pane 98 % opaque took 5 853 and 14 895 of that reflection and bounce light and takes 24 and
  67. A reflection no longer shows what blends; glass refracts a frame that has them. The forward pass is split there
  only when SSR, SSGI or glass needs it.
- **Contact shadows missed everything under about 2 cm.** The march read the f16 G-buffer, started a centimetre off the
  surface, wanted a 5 mm gap and stepped a sixteenth of its length at a time — a coin, which
  `DirectionalLight::contact_shadow_length` promised, cast nothing, nor did a 1 cm step. It reads the frame's depth now (the
  G-buffer group's binding 7), pixel by pixel: compared where it enters each pixel and where it leaves it, with the
  receiver's own depth run across a pixel as its tolerance. Under a 15° sun at about 1.1 cm a pixel, a 2 mm step darkens
  112 pixels, a 5 mm step 224 and a 1 cm step 336 (0, 0 and 0 before); a 3 cm step's 11.2 cm shadow 1 122 of its ~1 150
  (332 before, and the cascades hold 660 at their own bias). Open ground, flat or tilted 20°, seen steeply or grazing
  from three sides, changes by not one byte; at 1080p the march costs what it did. `DeferredState::new` and
  `DeferredState::resize` take the frame's depth view.
- **A lamp lit the air behind a wall, and a spot the air all round it.** The volumetric march gave every local light a
  bulb's sphere of glow and asked no light's shadow: a casting lamp behind a wall lit the air in front of it as if the wall
  were not there, a spot lit the air beside and behind its cone, and its cookie was no part of its shaft. Each light reaches
  the air now as it reaches a surface there — inside a spot's cone and through its cookie, and past its shadow when it
  `casts_shadows`. Measured on the march's own target: the air in front of a 10 cm wall with a casting lamp a metre behind
  it, 703.9 without the shadow and 1.18 with it (769.3 either way before); beside a 0.15 rad spot's cone, 0 (211 either
  side, against 260 in it). The march's samples were each stepped
  before they were taken, so the last landed up to a step past the surface its ray ends on — through a thin wall, into the
  light behind it; they end short of it now. The spot's cone is one function in `common.wgsl` (`spot_cone`), where the
  deferred and forward passes had a copy each. `VolumetricState::volumetric_texture` is `COPY_SRC`, so the scattering term
  can be read back on its own, and `VolumetricState::{width, height}` say they are the frame's size, which they always were.
- **A soft sun cast a sharp shadow on glass, in the editor and on every baked-lit surface.** The forward and baked-lit
  shaders filtered the sun's shadow 3×3 whatever `DirectionalLight::angular_radius` said. Every lit path takes one lookup
  now (`sun_shadow_visibility`, in `common.wgsl`): a cube's penumbra under a 0.05 rad sun is 406 pixels deferred, 415
  forward and 568 baked-lit (164 / 179 / 222 with no size), and 925 in the editor's viewport against 371. With it, the
  forward shader's shadows fade out over the end of the covered range (they ended in a line), and take
  `ShadowSettings::normal_offset`; the baked-lit one gives up its flat 16 cm depth bias for the shared slope-scaled one;
  the volumetric pass takes `ShadowSettings::depth_bias`; and the shadow fade is one function, where the deferred and
  baked-lit shaders each had a copy.
- **Two views in one encoder drew the first with the second's instances.** `SceneView` gave a view its own uniforms,
  clusters and shadow matrices, and left the instance buffer shared — and what a camera draws is what it sees. The guard
  test passed while its two cameras laid their instances out alike. Each view has its own instance buffer now
  (`SceneState::view_instance_buffer`, `view_instance_bind_group`, `view_instance_capacity`), and the guard's cameras see
  different objects.
- **A point light was half as bright on a forward-drawn surface.** The forward shader divided its falloff by `d² + 1`
  and the deferred pass by `d² + 0.01`, so a light a metre from a transparent surface — and from anything in the editor
  viewport, a browser or a phone — gave about half: a matte floor read 195.5 where the G-buffer's read 228.5. Both passes
  call `light_falloff` in `common.wgsl` now (the deferred pass's formula, which a rectangle light's integral agrees with).
- **A game with no background of its own was dark on the desktop and sky blue on a phone.** `Camera::clear_color`'s
  `None` meant each path's own: the deferred path's dark grey, the forward-only path's light blue. It is
  `frame_uniforms::DEFAULT_BACKGROUND`, the grey, on every path now. **Changes the look** of a browser or phone game
  that set no background, to the desktop's; set `Camera::clear_color` for another.
- **An opaque draw was blended where it was drawn forward.** The forward lit pipelines on the desktop, the forward-only
  path's `DETAILS` and `LOBES` tiers, the opaque unlit pipelines and a game surface's opaque set blended a texel by its
  alpha while writing its depth, so a cut-out's kept texels under 1 and an opaque material's texture alpha showed what was
  drawn first — the editor's frame changed between identical renders, by up to 1 577 pixels. None of them blends now
  (`pipelines::OPAQUE`), as the G-buffer never did; a transparent unlit draw takes `SceneState::unlit_blended_pipeline`,
  and `SceneState::unlit_for` takes whether the draw is transparent. **Changes the look** of cut-out edges in the editor
  and on the forward-only path, to the game's.
- **A cut-out cast its whole quad's shadow.** The shadow pass had no fragment stage. `shadow.wgsl`'s cut-out stage now
  asks the material where it is, for the engine's cut-outs and LOD crossfades and for every game surface (with its own
  `material_surface`, handed the frame's camera and clock through `ShadowVsUniform`). A cut-out cut away everywhere casts
  nothing, where it cast the plane's 184 pixels. Every shadow pipeline binds the same four groups — light, material,
  skeleton, instances — and `SceneState::shadow_pipeline_for` takes whether the draw cuts.
- **A point light cast an indexed mesh as its deduplicated corners.** Its six faces were drawn with `draw(0..vertex_count)`
  and no index buffer, so every engine primitive and import cast the wrong triangles — a plane only its first. They are
  drawn through the index buffer now: the same triangles indexed and flat cast the same 431 pixels.
- **A shadow-only object was drawn whenever it was drawn forward.** `ShadowCasting::Only` casts and is not drawn, and
  only the G-buffer passes asked: a blended, unlit, baked-lit or game-material `Only` object stood in the picture, and on the
  forward-only path (browser, phone), where every draw is forward, every one did — a transparent cube 5 658 pixels of it. The
  forward pass skips it now, and so does every draw loop of the editor viewport; its double-sided, transparent, glass, sky,
  backdrop and grid loops drew it too.
- **A plane cast no shadow.** The shadow pass drew every mesh by its back faces, which keeps a solid from shadowing
  itself and leaves a plane, a card or a sheet of cloth facing the light with nothing to cast: 0 shadow pixels against a
  cube's 236, single- or double-sided, from the sun and from a point light. `Mesh::closed` says whether a mesh closes over
  a volume (from its own triangles), and one that does not is drawn into the shadow maps by both faces
  (`SceneState::shadow_open_pipeline`, `shadow_pipeline_for`), in the game and in the editor. The plane casts 185, and does
  not shadow its own lit face.
- **The capsule mesh did not close.** Its equator ring was placed at 0 for both hemispheres, so each hemisphere's last band
  slanted into the middle of the tube and the tube's rims met nothing; an odd latitude count left no equator ring. Each
  band now moves with its hemisphere, and the count is rounded up to even.
- **The editor viewport drew a game's own pipeline with the engine's lit shader.** Its draw loops never read
  `Routing::custom`, so a `MaterialType::Custom` material showed in `gizmo-studio` as the engine's material with the lights
  skipped. Both kinds of game material are drawn from the renderer's registry there now, and an unregistered id draws nothing,
  as on the game path.
- **A hot reload never reached the deferred path.** `rebuild_pipelines` (`Renderer::rebuild_shaders`) rebuilt every pipeline
  but the G-buffer, prepass, environment-sky and lighting ones, so a reloaded `gbuffer.wgsl` or `deferred_lighting.wgsl`
  changed nothing in a running studio. `DeferredState::rebuild_pipelines` does it, and registered surfaces are rebuilt after.
- **A material's params buffer smaller than a `MaterialParams` failed far from its cause**, at a wireframe or debug-view draw
  that reads the group with the engine's shaders. `AssetManager::material().build` says so instead.
- **A reflection probe cost a phone 29 % of its frame rate.** The forward-only lit shader asked the probe uniform on every pixel
  which kinds of probe the scene had, and worked out the sky that a background probe then replaced. Which kinds there are is the
  same for a whole frame, so the pipelines are now built for it (`ProbeShape`: `PROBE_BACKGROUND`, `PROBE_BOXES`), and a scene with
  a background probe never samples the sky. Nişancı with a mirror floor on an Adreno 618, upright, one sitting: no probe 106.4 fps,
  a probe before 75.3, after 106.2; the reflection is unchanged. The desktop decides per pixel as before.

- **Inserting over a table component counted as adding it.** An overwrite fires `on_set` and `on_replace` and never `on_add`, but a
  table column stamped both ticks, so `Added<T>` reported the entity again — a sparse component's overwrite did not, so a re-insert
  was an add or not depending on where the component lived. Only `changed` is stamped now, in both.

- **`Changed<Children>` missed a second child, and a dead entity's handle read its successor.** `World::get_component_mut_ptr`
  stamped no change tick, so a write through it (the hierarchy, relationships, the inspector, scenes) was invisible to `Changed<T>`;
  asking for it now counts as a write, as `Mut<T>` does. And neither raw pointer checked the entity's generation, so a handle kept
  past a despawn read, or wrote, whatever entity reused the id. Both found while planning dynamic components.

- **A phone lost 13 % of its frame rate to features nothing in the game used.** The lit shader has three tiers on the forward-only
  path now: plain, `DETAILS` (back faces, cut-outs, crossfades, a material's own reflectance, blend modes, debug views) and `LOBES`.
  Batching picks one per draw (`routing::needs_details`, `SceneState::lit_pipeline`), `unlit.wgsl` splits the same way with `KEEPS`,
  and the desktop path has everything always. On an Adreno 618 in Nişancı, the phone upright and a floor filling the screen: 63.7 fps
  before the features, 53.6 after them, 63.1 now; on its side, 102, 89 and 103. A floor that takes `DETAILS` reads 84 where the plain
  tier read about 97. **Looks change** on the browser and on a phone: an opaque material's texel alpha is drawn solid,
  as the G-buffer always has. A surface that shows through says so with `is_transparent` or `alpha_cutoff`.

- **A phone draws 1.75 times as many frames: the environment comes from a cube.** On the browser and on a phone the presets and
  `Renderer::sky` were worked out at every fragment, twice, and that was most of the frame. They are drawn into a cube now, mip by
  roughness, when what they depend on changes (`env_cube`), and the lit shader samples it. Nişancı on an Adreno 618, upright: 52.9 fps
  before, 92.9 after, in one sitting; the floor's colour within 1 of 255. `scene_environment` in `common.wgsl` is the one function
  every path calls. The desktop path is unchanged.

- **The browser and a phone honour the tone curve, the exposure, the dither, the vignette and the grain.** Their composite applied
  fixed ACES and nothing else. `post_common.wgsl` holds the curves, the vignette and the dither for both composites; the reduced one
  takes the curve and which of the rest are on as pipeline constants (`post_process::sync_composite`). Bloom, depth of field, motion
  blur and chromatic aberration are still desktop only.

- **Colour look-up tables: `color_lut::ColorLut` and `Renderer::set_color_lut`.** A `.cube` file as a grade after the tone curve, or
  in its place fed `x / (1 + x)` or `log2` between two exposures: how Tony McMapface, SBDT and Blender's Filmic and AgX looks are
  made. On every path. `PostProcessUniforms::_bloom_pad` is `color_lut` now.

- **Glass absorbs over its thickness: `Material::with_attenuation(color, distance)`.** The light behind keeps
  `color ^ (thickness / distance)` a channel at a time, as glTF's `KHR_materials_volume` says, which the loader now reads.
  `Material::volume_transmittance` is the number. A glass instance carries it in its route flag, below -0.5.

- **A vignette of 0 is no vignette.** `PostProcess::vignette` said so, and the shader still took the corners to 73 %. **Looks change:**
  at the renderer's default 0.25 the corners are lighter, about 75 % where they were 52 %.

- **SMAA 1x: `Renderer::set_smaa(Some(SmaaQuality::High))`.** Edges, blending weights and neighbourhood blending, in FXAA's
  place in the chain, at the reference's four presets. Bevy's WGSL port of the reference, with its area and search tables under
  their MIT licence (`crates/gizmo-renderer/src/smaa/LICENSE`). Built the first time it is asked for.

- **A coloured specular: `Material::with_specular_tint`.** A non-metal's F0 a channel at a time, glTF's `specularColorFactor`, on
  both paths and in SSR. **Breaking for custom shaders:** `InstanceRaw` grew a `optics` vec4 (144 bytes), which also carries the
  glass volume that sat in the route flag; a custom shader declaring the instance struct must declare it, and
  `CustomMaterial::from_wgsl` panics with both sizes if it does not. The G-buffer's RT4 is `Rgba16Float` (48 bytes a sample).

- **Reflection probes on the browser and on a phone.** Group 0's bindings 6–8 carry them, and the lit pipelines are built with
  `PROBES` only while a scene has one (`SceneState::set_forward_reflections`). `reflection_probes.wgsl` takes its binding numbers as
  defs, and the irradiance grid's import has its own def, `IRRADIANCE`.

- **`Renderer::set_environment_map` works on the browser and on a phone.** The map goes through the reflection probes' prefilter
  into the cube those paths read their sky from, at its own resolution, and stands behind the scene. A `ReflectionProbe` is still
  desktop only, and those paths no longer capture one for nothing. `ReflectionProbeGpu::write_environment` takes a scale.

- **Fog is settled once a frame.** `frame_uniforms::frame_fog` picks the preset, the blend and the `FogMode`, and `height_fog` is
  arithmetic on every path. `SceneUniforms::fog` is always the fog in force, where its alpha used to be negative for "none set".

- **`Changed<T>` and `Added<T>` lost writes.** One tick per schedule run and a strict comparison meant a system saw nothing written
  after it in the same run, by a batch's commands after it, or between runs: a game's update hook, a physics step's hook. A query
  parameter now compares against the tick its own system last ran at, and the tick moves on between batches, so every system sees
  every write exactly once. **Behaviour changes:** a system that was missing changes now gets them. A marker spawned in the update
  hook now resolves (`AutoBoxCollider`, which documented the trap). The physics hash is unchanged. A query built from the world, in
  an `impl System` or a hook, still reads the world's window: it sees what came after the schedule's last run, and not what a later
  system of that run wrote.

- **The studio viewport ignored `Renderer::tonemap_curve`, its white point and `deband_dither`**, always drawing ACES.

- **A metal floor did not reflect the scene.** Screen-space reflections were added on top of the environment reflection and held down
  to a dielectric's Fresnel, so a polished metal showed the sky with a faint trace of what stood on it. What the ray finds now
  replaces the environment, weighted as the lighting pass weights it, in the metal's own colour: a red cube on a polished floor
  under a blue sky moves red up by 150 and blue down by 207 of 255. **Looks change** on anything shiny.

- **A cut-out's holes showed the sky on the deferred path, whatever stood behind them.** The depth prepass has no fragment stage,
  so it wrote depth over the holes, and the G-buffer pass then rejected what was behind them. A draw that discards has a prepass
  that discards. `Material::alpha_cutoff` also now works on that path at all: the G-buffer read only a glTF material's threshold,
  and a glTF `Mask` material now sets the field too. **Looks change:** foliage and fences show the world through them.

- **The back of a double-sided or transparent surface was lit with the front's normal**, on both render paths, so it read as turned
  away from a light shining straight on it (a pale panel: 195 from the front, far less from behind; 195 from both now). Found
  because SSAO's own fix let a test stop passing by accident: it had been seeing the inside of a cube as different only because AO
  had blackened it. **Looks change:** cloth, leaves and panes seen from behind are lit correctly.

- **SSAO darkened flat floors and marked the frame's edges.** A floor with nothing on it lost up to four levels over a quarter of
  a frame, and the frame had bands: dark along the top and left, where the blur read past the edge and got zero, and bright where
  samples fell off the screen and counted as unoccluded. All gone, and a flat floor is now untouched. **Looks change**: every
  scene with SSAO is a little brighter on open ground.

- **TAA let black into its neighbourhood along the frame's edge**, so what appeared there came in over many frames (186 against
  223, the frame after a white wall appears). The same read past the edge.

- **The studio viewport ignored `Renderer::sky` and `Renderer::fog`**, and its Game panel ignored the environment preset as
  well: both built their frame from defaults. A scene that set a sky by hand was lit one way in the game and another in the
  editor. **Looks change** in the Game panel of a project that sets any of these, to what the game draws.

- **Hooks reach every write.** `add_bundle` and `clone_entity` fired nothing and `spawn_batch` fired once in four, so an
  index kept by hooks was a guess. They take the per-component route for a type with hooks, and keep their fast path for
  one without.

### Added

- **Lines break where each script allows.** `Text::max_width` used to break only at spaces, so Chinese, Japanese and
  Thai, which have none, were cut between arbitrary characters, a full stop could start a line, and a Thai word was
  torn in half. `icu_segmenter` now gives the opportunities: UAX #14 for every script and the LSTM models Thai, Lao,
  Khmer and Burmese need. **Latin wraps differently too:** a line can now break after a hyphen.

- **Several sizes and faces in one text.** `Text::rich` still takes `("Can: ", WHITE)`, and now also
  `Span::new("5").color(RED).size(32.0).font(bold)`. Each line is as tall as the tallest thing on it, everything on a
  line shares a baseline, and wrapping measures each part at its own size. `FontLibrary::layout_styled` is the layout
  underneath. **Breaks:** `Text::spans` is `Vec<TextSpan>`, not `Vec<(u32, Vec4)>`.

- **Vertical text.** `Text::in_columns` sets a text top to bottom with the first column on the right, as Chinese,
  Japanese and Korean can be. `max_width` is then a column's length. Upright only: Latin is not turned on its side.

- **The line breaker costs the browser build 0.32 MB gzipped** (5.20 → 5.52 MB, +6.2 %; +0.40 MB raw). It carries the
  models for the scripts that write no spaces, and there is no way to choose a smaller set yet.

### Changed

- **`Query` has the shape of Bevy's.** `iter()` and `iter_mut()` yield the row's item and nothing else;
  whoever wants the entity asks for it in the query. All 625 call sites, 36 games and every demo moved in
  the same change, with no compatibility path.

  ```rust
  // before
  for (id, (mut t, v)) in q.iter_mut() { /* id: u32 */ }
  let q = world.query::<&Enemy>().unwrap();
  for (id, e) in q.iter() { /* ... */ }

  // after
  for (entity, mut t, v) in q.iter_mut() { /* with Query<(Entity, Mut<T>, &V)>, entity: Entity */ }
  let q = world.query::<(Entity, &Enemy)>().unwrap();
  for (entity, e) in q.iter() { /* ... */ }
  ```

  | Before | Now |
  |---|---|
  | `q.iter()` → `(u32, Item)` | `q.iter()` → `Item` |
  | `q.single()` → `Result<(u32, Item), _>` | `q.single()` → `Result<Item, _>` |
  | `par_for_each(\|(id, item)\| ..)` | `par_for_each(\|item\| ..)` |
  | `iter_chunks()` → `(&[Entity], Slice)` | `iter_chunks()` → `Slice`; put `Entity` in the query for the handles |
  | `iter_combinations()` → `((u32, Item), (u32, Item))` | `iter_combinations::<K>()` → `[Item; K]`; `for [a, b] in q.iter_combinations()` |
  | `for (a, b) in q.iter_combinations_mut()` | `while let Some([a, b]) = c.fetch_next()` |
  | `q.entities()` | `(Entity, ...)` in the query; `q.is_empty()` to ask whether there is anything |
  | `q.get_entity(e)`, `q.get_mut_entity(e)` | `q.get(e)`, `q.get_mut(e)` |

  `iter_combinations` now takes any `K`, not only pairs.

- **A filter is the query's second parameter.** `Query<&T, With<U>>` yields `&T`; the old
  `Query<(&T, With<U>)>` yielded `(&T, ())`, and every loop carried a `_` for it. A filter in the first place no
  longer compiles (`QueryData` and `QueryFilter` are the two sides). From a world, `query_filtered`,
  `query_filtered_mut` and `query_filtered_unchecked`. Entities without data: `Query<Entity, With<U>>` and
  `Query<(), With<U>>`.

  ```rust
  // before
  fn sys(q: Query<(Entity, &mut T, With<U>, Without<V>)>) { for (e, mut t, _, _) in q.iter_mut() {} }
  // after
  fn sys(mut q: Query<(Entity, &mut T), (With<U>, Without<V>)>) { for (e, mut t) in &mut q {} }
  ```

- **`for x in &q` and `for x in &mut q`** work, as they do on any Rust collection.

- **`World::query_entity` and `query_entity_mut` take an `Entity`** and check it, like `Query::get`. They were the
  last query entry points with a raw id, and answered for whoever held the slot.

- **A writable query term is `&mut T`, and there is one way to ask.** `Query<&mut T>`, `world.query_mut::<&mut T>()`,
  `Option<&mut T>`; the row's item is still [`Mut<T>`], which is what records the change. `Mut<T>` is no longer a
  query term.

  | Before | Now |
  |---|---|
  | `world.borrow::<T>()` | `world.query::<&T>()` |
  | `world.borrow_mut::<T>()` | `world.query_mut::<&mut T>()` |
  | `world.borrow_mut_unchecked::<T>()` | `world.query_unchecked::<&mut T>()` |
  | `Query<Mut<T>>`, `Query<(Mut<A>, &B)>` | `Query<&mut T>`, `Query<(&mut A, &B)>` |
  | `StorageView<T>`, `StorageViewMut<T>` | `Query<&T>`, `Query<&mut T>` |

- **`world.query::<Q>()` returns the query, not an `Option`.** Building one never failed: `Query::new` always
  said `Some`, so the type asked about 330 callers for an `.unwrap()` or `.expect()` and 130-odd for an
  `if let Some`, whose `else` branches could not run. Two of those logged a failure that did not exist. `query`, `query_mut`, `query_unchecked` and
  `query_cached` all changed. `SystemParamFetchError::QueryError`, documented as unreachable, is gone.

  ```rust
  // before
  if let Some(q) = world.query::<&Enemy>() { for e in q.iter() { /* .. */ } }
  let n = world.query::<&Enemy>().unwrap().iter().count();

  // after
  for e in world.query::<&Enemy>().iter() { /* .. */ }
  let n = world.query::<&Enemy>().iter().count();
  ```

- **`Query::get`, `get_mut` and `contains` take an `Entity` and nothing else,** so a lookup can no longer
  reach whoever got a dead entity's id. What held raw ids holds handles now, and code that passed `x.id()`
  around passes `x`:

  | Type or function | Was | Is |
  |---|---|---|
  | `HitEvent::{attacker, attacker_hitbox, victim, victim_hurtbox}` | `u32` | `Entity` |
  | `UiNavigation::{focused, focus}`, `Drag::{over, dropped_on}`, `DropTarget::{hovered_by, dropped}` | `Option<u32>` | `Option<Entity>` |
  | `HierarchyExt::{is_ancestor, descendants_inclusive}` | `u32` | `Entity` |
  | `active_camera`, `SceneData::{find_entity_by_name, entity_names, of_entities, save_prefab, load_prefab}`, `instantiate` | `u32` | `Entity` |
  | `collect_hidden` | `HashSet<u32>`, and a `hidden` parameter | `HashSet<Entity>`, no parameter: it asks the world |
  | `Action`/`Condition` behaviour-tree callbacks | `FnMut(u32, ..)` | `FnMut(Entity, ..)` |
  | `Query::live`, new `World::live` | `Option<u32>` | `Option<Entity>` |

  `World::reconstruct_entity` is gone. **Raw ids remain where they are the format, not a shortcut:**
  `BodyHandle` and the physics solver (its state hash keys on them), Lua numbers and `ScriptCommand`, scene
  files' `original_id`, renderer caches. `World::entity(u32)` is the one way across, and it no longer takes
  the allocator's lock: it reads the handle from the entity's row.

  `Entity` gained `Ord`. It sorts by generation first, so a sorted list of handles is not in id order.

  Measured against the previous commit, alternating the two: row iteration is within noise
  (`for_each_iter` +3 %). `get` by `Entity` pays for the generation check, over 50 000 entities in order:
  sparse 368 → 636 µs (7.4 → 12.7 ns a call), table 566 → 632 µs (11.3 → 12.6 ns), wide queries +1–3 %.
  The commit message of this change gives 26 ns for 10 000 table rows; that is wrong and this is the
  measurement.

### Fixed

- **A stale handle reached a new entity through `world.get_entity(x.id())`.** About 120 callers that went
  through the id now ask `world.live(x)`, which says `None` once `x` has died. `UiNavigation` loses its
  `focused_handle`, a second copy kept only to detect exactly this.

- **`iter_combinations_mut` could give two live `&mut` to one row, in safe code.** It was an `Iterator`, whose
  items outlive the call that made them, and row 0 is in the pairs `(0, 1)` and `(0, 2)` alike:
  `.collect::<Vec<_>>()` held both. `fetch_next` ties each combination to a borrow, so the previous one is
  gone before the next exists, and a `compile_fail` doctest holds that. It also removed the raw pointer and
  the `unsafe` block the old form needed.

- **Fluid was drawn over the whole frame, and through what stood in front of it.** `fluid_composite.wgsl`
  read the depth from the normal's channel, so no background pixel counted as empty, and it wrote no depth
  to test. It reads the right channel, writes `frag_depth`, and uses the real normal and view vector, so **the
  fluid looks different**. The demos that draw it open a window and were not looked at afterwards.

- **An unseen object that casts nothing was still built and uploaded.** `ShadowCasting::Off` kept an
  off-screen object from the shadow pass but not from the shadow-only instance list: a camera turned
  away from 19 882 such cubes still uploaded all of them. Now it uploads none.

- **The UI focus passed to a new widget on a dead one's id.** A menu rebuilt without calling `focus`
  again left the focus on whichever new widget took the id: highlighted, and Enter pressed it.
  `UiNavigation` now remembers the handle it saw and drops the focus when the id's entity is another
  one; a game focusing a widget it just spawned keeps it.
- **A fighter's new victim on a dead one's id was immune.** `FighterController::already_hit` is
  `Vec<Entity>` now, not `Vec<u32>`: a new entity on an id the move had already hit gets hit.

- **Every game in the browser was too dark.** The reduced path's composite left sRGB encoding to the
  surface, and a browser canvas has no sRGB format: on the web the surface is `bgra8unorm`, so the
  linear colour reached the screen as it was. `pinbol`'s table (`#1B4F72`) measured (1, 22, 63) in
  headless Chrome; it is (41, 89, 125) now, against (40, 89, 125) in the Android APK, whose surface
  is sRGB. The composite encodes when the surface will not (`ENCODE_SRGB`, a pipeline constant set
  from the surface's format).

- **A touch control made in answer to a touch never got that touch.** `VirtualStick` and
  `VirtualButton` claimed only a finger *landing* on them, and a control spawned on the first
  touch has no box until layout runs — by the next frame the finger is no longer landing. On a
  phone `nisanci`'s sticks appeared under the thumb and ignored it until it lifted and came down
  again. On its first frame with a box a control now also takes a finger already down inside it
  that nothing else holds (a stick centres where that finger is); after that frame only a landing
  finger counts, so one sliding onto a control that was already there still claims nothing, and a
  finger lifting over a brand-new button is not a tap. `nisanci --test` now lays a first finger on
  the left stick and checks the player moves (+0.00 before, +3.50 after).

- **Screen text ran off the side of a narrow window.** A pinned or screen-positioned `Text`
  wrapped only at `\n` or its own `max_width`, so a HUD line wider than the window lost its end
  past the edge — `pinbol`'s controls line on an upright phone (350 frame pixels across) lost its
  last third. It now wraps at the room the window leaves it, measured from its anchor point by how
  its box hangs off it: to the right edge for a top-left box, twice the nearer edge for a centred
  one, to the left edge for a right-anchored one; a narrower `max_width` still wins. Text placed
  by a host (a UI box, a tooltip) and world labels are unchanged.

- **The lit forward shader came out garbage on an Adreno phone.** On a Redmi Note 12 Pro
  (Adreno 618, driver V@0502, Chrome 156) `pinbol` drew speckles, rainbow noise on the bumpers,
  rails patched with other objects' colours and a quad of noise on an empty stretch of table —
  and flickered as it played. The desktop browser drew the same build clean. The cause was
  `gizmo_cluster_index(scene_: SceneUniforms, …)`: it took the whole scene uniform **by value**,
  17 040 bytes (nearly all of it the 256-light array), for five fields. That driver's compiler
  miscompiled the fragment shader around the copy: everything that depended on a fragment input
  (varyings, the instance id, even the fragment's own depth) was garbage in places, while a
  constant colour drew clean, and it did so with the light loop never running (the scene has no
  point lights). It now takes the five fields. Found by bisecting `fs_main` on the phone. Ruled
  out on the way, each by a build on the phone: the instance storage buffer (fed as vertex
  attributes instead), `firstInstance`, the depth format, depth precision, the HDR format's
  compression, blending, `@invariant`, skinning. A test fails any shader function in the forward,
  deferred or web schema that takes more than 1 KB by value (red on the old signature, 17 040 B).

- **The web build on a phone: the canvas, its size, the pointer, the console.** Found by running
  `pinbol` on a phone over `adb` (Android 13, Chrome 156):
  - the canvas was given the app's 1280 × 720 as its CSS size, over the page's 100 % — a phone
    392 px wide showed the canvas's top-left corner. In the browser no size is asked for now;
  - the `Resized` that reports the canvas's real size came while the renderer was still being
    built asynchronously, found none, and was dropped: the frame stayed at 640 × 360. The size is
    applied once the renderer is ready;
  - the web render cap was "640 wide or 360 tall": an upright phone would have got 192 × 360. It
    is a pixel budget now (640 × 360 of them, any shape): 350 × 657;
  - pointer and touch positions arrive in the canvas's pixels and the frame is rendered smaller,
    so every touch control and UI click landed off to the right or off the frame. They are scaled
    into the frame's pixels (exactly 1 on the desktop);
  - the console got every `tracing` event down to TRACE, each also put in the performance
    timeline — hundreds of lines a frame, 50 ms frames. INFO and up now, no timeline marks.
- **An absolutely positioned root UI node was never sized by its insets.** taffy lays a root out
  with no containing block, so `top: 0, bottom: 0` (or `left` and `right`) with an `Auto` size
  gave a box **0 px** on that axis — a touch button meant to cover half the screen covered none
  of it. Such a root now spans the window between its insets, as an absolute box inside a parent
  always did.
- **A new entity on a dead one's id was drawn with the dead one's motion.** `MotionHistory`
  (object motion for TAA and motion blur) keeps a slot per raw id; a new entity drawn on an id
  another was drawn on last frame got that one's matrix as its previous — one frame of motion
  vectors from the dead entity's place to its own, a streak or a ghost. The slot records the
  entity's generation now; a different one is a newcomer, with no motion.
- **A streamed texture could land on the wrong entity's material.** A finished decode names its
  waiting entities by raw id; a material despawned while its texture decoded had its id taken by
  a new entity, whose material got the old texture and kept it. The texture goes only to a
  material whose `texture_source` is still the path it asked for. `TextureReloadCompletion` has
  a `request_path` (the cache key is its canonical form, not always the same string).
- **A body on a dead body's id was told it had been in that body's contacts all along.** The
  contact cache is keyed by the pair's raw ids; despawn a body inside a trigger and spawn another
  there on the same id, and the new one got `Persisting` on every substep and never `Started` —
  an "on enter" never ran — and, in a solid contact, took the dead body's warm-start impulses.
  Removing a body now takes its cached contacts out, and the next step reports them `Ended`
  first, before its own events, so the order on the shared id is "left, then entered".
  `tests/joint_recycled_id.rs`.
- **A kinematic body driven once a frame moved in jerks (`FixedPhysics::advance`).** A
  `KinematicTarget` is reached "in the next step", so a frame that ran four steps made the whole
  move in the first at four times the speed asked, then stood still. A ball at rest on a board
  starting to tilt at 0.6 rad/s, 3 m from the axis (a surface rising at 1.8 m/s), was thrown up
  at 5.2 m/s; in `labirent` it hopped onto the walls. `advance` now cuts each target into one
  piece per step — 1.74 m/s. The same change sent `pinbol`'s ball back into the launch lane,
  where it sat forever, counted as launched; a ball resting at the lane's foot can be launched
  again.
- **A body spawned on a dead body's id inherited its joints (ECS physics bridge).** Physics
  knows a body by its raw ECS id, and a despawned id goes to the very next spawn: despawn a body
  held by a joint and spawn another before the next step, and the new one took over the old
  body's row and was tied to its partner — spawned 15 m away, pulled 5 m in a second. The bridge
  now tracks each id's generation and removes the old body, with its joints, when the id comes
  back as someone else. `tests/joint_recycled_id.rs`.
- **A CCD body a hair from a wall stopped moving along it (`Gjk::speculative_contact`).** Inside
  1 mm the contact's normal was the approach direction itself, so a ball rolling along a rail
  under gravity was given a contact pointing back against gravity — in `pinbol` it hung on the
  launch-lane rail, normal (0, 0.978, −0.208) — and a lone CCD ball on a slope rolled 9 % short,
  its own floor bounces taken the same way. The normal is now the surface's own, measured just
  past it along the approach. `tests/ccd_touching.rs`: 0.025 m and 0.665 m rolled before,
  against about a metre free; both within a few per cent of a body without CCD now.

- **A stock humanoid ragdoll was born apart (`RagdollBuilder::create_humanoid`).** Nine of its ten
  joints were open at spawn — each bone's `local_pos` disagreed with the anchors of the joint that
  holds it, the shoulders by 18 cm — and the lower arms were born inside the torso they do not hang
  from, so every ragdoll snapped and kicked itself on its first step (in zero gravity, still moving
  at 0.11 rad/s a second after spawning from rest). Each bone now sits where its joint's anchors
  meet, the shoulders clear the torso and the legs clear the hands; `spawn_ragdoll` warns about a
  bone authored off its joint. `tests/ragdoll_spawn.rs`.

### Changed

- **`Parent` and `Children` hold `Entity`, not `u32` (breaking).** A link to a dead entity stays a link to
  the dead entity, whoever gets its id next. A list written directly, or a handle kept past a despawn, could
  hide, move or despawn the stranger on the recycled id, and `collect_hidden` did hide one. Walkers check
  `World::is_alive`, or the new `Query::live` where a system has no `World`. `collect_hidden` takes the `World`.
  `descendants_inclusive` leaves dead children out. Scene files are unchanged: they store plain ids and load
  remaps them. To migrate, `Parent(e.id())` becomes `Parent(e)`, and a walk uses `child.id()`.

- **The forward shader uses the environment presets: transparent surfaces, the editor viewport and every
  browser and Android game look different.** The presets lived in the deferred shader alone, and
  `shader.wgsl` lit everything with a fixed warm hemisphere, so one red cube with no sun read red 182
  forward and 104 deferred. Both now share `preset_environment` and the environment BRDF, and come out
  under 8 of 255 apart under every preset. Under the default preset, forward surfaces with little
  direct light get noticeably darker and lose the warm cast.
  `pinbol`'s chrome ball went nearly black on its dark table for the same reason, so the game now sets
  a `Renderer::sky` matching the pale background behind it, and it reads again.
- **The forward shader reads the whole material: textured surfaces look different in the browser, on
  Android, in the editor viewport and wherever they are transparent.** It read the base colour and nothing
  else, so normal, metallic-roughness, emissive and occlusion maps, the UV transform, anisotropy, clear coat
  and subsurface never reached those paths. `surface.wgsl` now evaluates a material for both shaders, and
  each knob and map comes out under 2 of 255 apart forward and deferred. Irradiance volumes reach forward
  surfaces on the desktop path.
  On Android and in the browser a material with anisotropy, clear coat or subsurface takes a second set of
  lit pipelines, because an Adreno 618 pays for those branches even untaken: a cube and a floor hold 64 fps,
  and a clear coat on a floor filling the screen reads 20.
- **Height fog covers every material path: transparent, forward, baked-lit, water and mirrors, and
  `Renderer::fog` now works in the browser and on Android.** It was the deferred shader's alone. Each
  blend mode treats it as it needs: alpha and opaque fade to its colour, additive fades out, multiply
  fades to white. **Baked-lit surfaces are now fogged, by the default preset's fog too**: a game with
  its own lighting sets `Renderer::fog` to choose. Unlit materials keep their exact colour. Particles,
  smoke, GPU physics bodies and the fluid surface and spray stand in it too.

### Added

- **`Entity` is a query term, and `Query::get` takes one.** `Query<(Entity, &Health)>` hands out each row's
  whole handle. `get`, `get_mut` and `contains` take an `Entity` or a raw `u32`: an `Entity` is checked against
  the row, so a dead handle returns `None` even after its id went to someone else, with no lock. Archetypes store
  `Entity` per row, so `Archetype::entities` and the first slice of `iter_chunks` are `&[Entity]`. A table's
  `for_each` got 28 % faster (14.3 to 10.7 µs over 50 000 rows), since a query that never looks at the id no
  longer walks the list. `get` with a raw id and the physics step are unchanged, measured against the old code
  in alternation because a loaded machine moved those numbers by 60 %.

- **`Text::rich`: several colours in one text, and lines that start on their own side.** A span colours the
  glyphs drawn from its bytes, across lines, wraps and right-to-left runs, and `Text::color` still fades the
  whole string. `TextAlign::Start` is the new default: a right-to-left line lines up on the right, and
  left-to-right text stays where it was.

- **Text shaping and bidi.** Arabic letters join, Devanagari vowels reorder and conjuncts form,
  right-to-left and mixed lines are ordered, and marks sit where the face puts them, given a face that
  has those scripts (`add_font_fallback`). `GPOS` kerning now applies, and it is the only kerning Noto
  Sans has, so **existing text changes width slightly**. Latin text gets no ligatures, because Noto
  Sans's `fi` hides the dot that tells i from ı. Recent layouts are remembered: a wrapped paragraph
  costs 0.97 ms the first frame and 0.5 µs after. Adds `harfrust` and `unicode-bidi`; the web build grows
  4.9 % gzipped. Fallback faces are now drawn at the em size of the text's own face (`PositionedGlyph::px`):
  Arabic beside Latin was two thirds as big. Checked by glyph ids and positions and looked at once in a
  desktop frame; not yet in a browser or on a phone.

- **Planar reflections (`PlanarReflection`).** A flat unlit mesh with this component reflects the scene,
  including what the camera cannot see: the scene is drawn again from the camera reflected in its
  plane, clipped at the mirror by an oblique near plane. `reflectance` runs from a mirror to water's
  grazing-angle Fresnel. **It costs one more full draw of the scene per mirror in view** (5.4 → 7.7 ms
  on a nearly empty scene at 1280 × 720, and it scales with the scene); a mirror out of view or seen from
  behind costs nothing. Perfectly sharp; mirrors do not show in each other; plain in split screen and
  in the editor. Not yet run in a browser or on a phone. **On water** (`PlanarReflection::water()` with
  `Material::with_water`), the waves break the reflection up and Fresnel mixes it in where a constant
  sky colour used to be; water without the component is unchanged.
- **Occlusion culling (`Occluder`).** Put it on a wall or a building, and what is completely behind it
  draws nothing, shadows kept. Rasterised on the CPU each view, so nothing pops in late and it runs
  the same in the browser. The frame is byte-identical with and without. On 2 026 spheres of 8 064
  triangles behind a wall: 23.0 → 8.0 ms not casting, 34.2 → 19.4 ms casting. It buys nothing for
  cheap geometry that casts (19 882 cubes: 17.3 → 17.9 ms). A scene with no `Occluder` pays nothing.

- **`World::run_system`: a system, once, now.** From an `on_enter` hook, an exclusive system or a
  test. It runs inside the frame's change window, its `Commands` are applied before it returns, and
  a `Result` goes to the error handler. (A `Schedule` built for it and `run` reopens the window from
  tick 0, and every later system of that frame sees the whole world as changed.)

- **Logical pixels: a HUD is the same size on every screen (`DisplayScale`).** The engine works in
  the frame's pixels, and those are different sizes: a 22 px HUD line was 13 logical pixels on a
  phone, and its size in the browser depended on the render budget. The windowed app keeps a
  `DisplayScale` resource (frame pixels per logical pixel; 1.67 on a Redmi Note 12 Pro, exactly 1
  on an ordinary desktop window), and a screen `Text`'s font, wrap width and pinned margin, every
  `Val::Px` of a layout and a `VirtualStick`'s radius are now logical pixels. A `UiScale::fixed`
  multiplies it, `UiScale::fit` already measures the window, and a `Text::screen` position stays
  a place in the frame. **A high-DPI desktop changes:** its HUDs and layouts grow to their
  logical size, as the platform's scale asks.
- **`SafeArea`: what the system covers.** A resource with each window edge's covered length in frame
  pixels, from Android's content rectangle (the phone's navigation bar: 130 physical pixels,
  measured). `nisanci`'s sticks sit above it, where a thumb reaches them.

- **Android: the games as an installable app.** `scripts/android-apk.sh` builds an APK
  (`demo-android`, cargo-apk) that installs on a phone and opens full screen from its icon —
  `pinbol` today, touch flippers and all, measured on a Redmi Note 12 Pro (Adreno 618, Android 13).
  What it took in the engine:
  - `gizmo_app::android::set_app`: an Android app starts in `android_main(AndroidApp)`, and winit
    builds its event loop only from that handle; `App::run` takes it from there. winit's
    `android-native-activity` feature is on for Android.
  - **The render path is the browser's** there. An Adreno 618's Vulkan driver allows four bind
    groups and the desktop path asks for six (the first APK died in `request_device`). Which path
    a target takes is now its own cfg, `gizmo_forward_only` (wasm32 or Android, set by
    `gizmo-renderer`'s and `gizmo`'s build scripts), instead of `target_arch = "wasm32"`: Android
    takes the browser's pipelines and keeps the desktop's threads, files and timer.
  - **The surface survives the app leaving the screen.** Android destroys the window then; the
    app now lets the surface go on `suspended` and makes another on `resumed`
    (`Renderer::suspend_surface`, `resume_surface`) — before, a game sent to the background came
    back to a black screen. A frame with no surface is skipped instead of panicking.
  - **`nisanci` as an APK too** (`scripts/android-apk.sh nisanci`, `demo-android-nisanci`): 91 fps
    on the phone, both touch sticks, sound through AAudio. One APK per game; both carry the
    project's icon (`media/logo.png`), and CI's `android` job builds both and keeps them.
  - **An app renders at 1280 × 720 pixels' worth**, four times the browser's 640 × 360: an app has
    the GPU to itself, and at the browser's budget an upright phone stretched a 328 × 701 frame 3.3
    times.
  - The C++ runtime is linked on Android (`meshopt` is C++; the first APK could not load:
    `cannot locate symbol "__gxx_personality_v0"`), and the desktop crash dialog (`rfd`), which
    does not build there, is off.

- **`nisanci` in the browser.** `demo-web/?oyun=nisanci` runs the twin-stick arena shooter, the same
  game the desktop binary runs: it moved to `games/` (`games::nisanci::app()`, `self_test()`), as
  `pinbol` did, and `demo/src/bin/nisanci.rs` is now three lines that host it. Its font is
  `Renderer::default_font` (the bundled Noto Sans in the browser) rather than a loader of its own,
  and `games` turns on `audio` for its synthesised shots. On a narrow screen (a phone held upright, about 34° across)
  the camera follows the player sideways, stopping at the arena's edge, where before half the
  arena was off the sides; in a wide window it does not move. A phone has no R or Start, so a touch restarts a
  finished game, once a second has passed since it ended (a thumb still pressing in panic does not
  start the next one unseen); the banner says `dokun: yeniden` once the touch sticks are up.
- **Soft bodies you can drop, stand and stack (`SoftBodyMesh`).** Measured on a 1 m, 10 kg block
  (E = 20 000 Pa) before this: dropped 1.5 m onto the floor it **exploded** inside a second (one
  explicit 1/60 s step is far past what such a material is stable for — E = 1000 did too); resting
  on the floor through short steps it **sank** 0.33 m in ten seconds; and a block dropped on
  another fell **straight through** it, soft bodies meeting only rigid ones. Now:
  - `SoftBodyMesh::stable_substeps(dt)` estimates the body's fastest vibration from its material
    and elements, and `soft_body_step_system` cuts the frame into that many steps (8 at E = 1000,
    32 at 20 000 for that block);
  - a node inside a collider after the sweep is put back on its surface, its inward velocity
    taken out and paid to the body; and a surface the node is moving away from no longer counts
    as a hit (a node leaving the floor was held on it while its velocity climbed);
  - `gizmo_physics_soft::soft_contact`: every node of one body inside another goes back out
    through the face it is falling through, with Coulomb friction, the two sharing the move and
    the impulse by inverse mass — momentum conserved exactly. Without the friction, a block on a
    block slid 0.8 m sideways in five seconds and fell off;
  - `SoftBodyMesh::block(min, size, cells, mass, E, ν)` builds a solid box from cubes cut into six
    tetrahedra the same way each time, so the mesh is conforming. The five-tetrahedra split that
    blocks were built with by hand (`cpu_physics`) is not — half its inner faces read as surface.
    `cpu_physics`'s jelly uses it now, and its `damping = 5.0` (a retention factor: above 1 it
    *adds* speed) is 0.5.
  `tests/soft_solid.rs` — reverting each of the five turns a test red.
- **A soft body is drawn (`soft_body_mesh_system`).** A `SoftBodyMesh` in the world was stepped
  and drawn as nothing but debug lines. Both draw paths now give it the mesh of its surface,
  outward, normals smoothed over the nodes, and a renderer and material the first time.
- **`SpriteImageId::placeholder(n)`** — an image id no renderer hands out, for building a 2D
  scene where there is no renderer: a sprite or tilemap naming one draws nothing. `load_sprite_rgba`
  needs a GPU and the id had no other way in, so a sprite game could not be built headlessly.
- **A `SimpleApp` game plays without a window (`gizmo::simple::HeadlessFrames`).** Hand it a
  world holding the game's scene and call `frame(dt, update)` with the game's own update: the
  hook runs with nothing pressed, then the deferred commands are applied, as in the windowed
  loop. `HeadlessFrames::font()` gives HUD text a font with no renderer. The first headless test
  written as a bare loop skipped the command flush, and `vinc`'s shattered blocks were latched
  broken and never despawned — 0 pieces reported where 9 broke. `SimpleSceneState` is `Default`. It also adds the
  simple scene's defaults (its physics world, `Gizmos`, an asset manager), takes the game's
  plugins (`add_plugin`) and runs their fixed-step and per-frame systems before the hook, as the
  windowed loop does. After the hook it advances the per-frame clocks the draw path advances —
  tweens, colour tweens, typewriters, the screen fade (`systems::render::advance_frame_clocks`, one
  function both call) — because game logic waits on them: `zindan` takes the stairs when its fade
  has gone dark.
- **Pinbol in the browser, with touch flippers (`demo-web/?oyun=pinbol`).** The game moved to a
  new `games/` crate (`games::pinbol::app()`), which the desktop `pinbol` binary and `demo-web`
  both run; on a phone the halves of the screen are the flippers.
- **A font the engine carries (`bundled-font`, default off).** Noto Sans Regular, SIL Open Font
  License 1.1 (`crates/gizmo-renderer/assets/fonts/OFL.txt`), 621 KB. `Renderer::default_font`
  uses it when the machine lends no font — always in the browser, where text had been ASCII-only
  (every Turkish letter blank). `demo-web` turns it on; `text::bundled::{noto_sans, NOTO_SANS,
  NOTO_SANS_LICENSE}`.
- **A chaos test (`gizmo-physics-rigid/tests/chaos.rs`).** From a seed, through the ECS bridge a
  game uses: bodies spawned and despawned every frame (ids recycled constantly), ropes tied
  between them and cut, kicks, CCD switched on and off, in a closed box. After every step nothing
  may be non-finite, outside the box (only a body through a wall could be) or absurdly fast, and
  no joint may name a body that is gone; the same seed twice must give the same world. One storm
  runs in the suite (~50 s unoptimised); three long ones with `--ignored`. Green on its first run;
  it catches the joint cleanup of `remove_body_at` taken out at frame 10.
- **Every game plays itself headlessly (`--test`), all 36.** What it found is under Fixed and in
  `docs/CAPABILITY_GAPS.md` C16ag; besides the engine fixes, `poligon`'s player walked at twice
  his speed (the game drove its character controller a second time), `pist`'s AI stayed stuck on a
  barrier, `kristal_avcisi`'s states and systems are a plugin both runtimes add.
- **`kapi`, `vinc` and `labirent` play themselves headlessly (`--test`).** `labirent`'s
  autopilot now solves the maze — a path search around walls and holes, tilting toward the next
  tile — and found that levels 2 and 3 could never be won: a hole cut every way to the goal. Each
  lost one hole; the test also checks every level has a path. The autopilot's times are not saved
  as records.
- **The games are a gate (`scripts/game-tests.py`).** Every demo with a headless `--test` mode
  plays itself in release, four at a time, with no display in its environment — a binary that
  ignores the flag fails rather than opening a window. CI runs it in the determinism job; 15
  games, 33 s. Until now no gate ran a single one, so a physics change could break a game unseen.
- **`sarmasik`, the thirty-sixth game** — swing across a chasm on ropes: hang, pump, let go,
  catch the next in flight; each grab and release makes and breaks a `Joint::rope` while the game
  runs. `sarmasik --test` lets an autopilot cross headlessly and checks the rope never stretches
  past its length. It found the recycled-id joint under Fixed.
- **`pinbol`, the thirty-fifth game** — a pinball table: gravity tilted 12°, a small fast CCD
  ball, two kinematic flippers swung at 25 rad/s, three bumpers that kick and score.
  `pinbol --test` lets an autopilot flip for a minute headlessly and checks the flippers hit the
  ball and that it never leaves the table through a wall. It found the CCD contact under Fixed.
- **`robot`, the thirty-fourth game** — a two-bone robot arm on a table pushing balls into a
  pocket, its tip following the cursor through `solve_two_bone_ik`; the first use of the engine's
  IK anywhere. `robot --test` lets an autopilot sink all five headlessly (24 s) and checks the tip
  is where it was asked to be (it never strays: 0.000 m). No gap found; `PhysicsMaterial::rolling_friction`
  now says that under the default combine rule one surface at zero makes the pair zero.
- **`topcu`, the thirty-third game** — a cannon that throws a ragdoll at rings on the ground; the
  first game of ragdolls. `topcu --test` fires one ballistic shot headlessly and checks the
  ragdoll is born whole, holds together through the flight and the landing (joints at most 6 mm
  apart), lands near the target and comes to rest. It found the humanoid item under Fixed.
- **`jole`, the thirty-second game** — a tower of jelly blocks dropped from a swinging crane; the
  first game of soft bodies. `jole --test` drops four headlessly and checks the tower stands. It
  found the soft-body items above, and one more: three jellies a few centimetres off centre blew
  the stack apart (nodes at thousands of m/s) because a contact pushed an overlapping node out in
  one go; a push is at most 5 mm now, the rest over the next sub-steps.

- **`ExternalForce` — a push that lasts (`gizmo::physics::components::ExternalForce`).** A force
  and torque applied on every physics step while the component is on the body: an engine, a
  thruster, a steady wind. The body's own accumulators are emptied by each step and a game writes
  them once a frame, so with a fixed-step driver the same push was a different acceleration at
  every frame rate — measured, 10 N on 1 kg for a simulated second gave 5.0 / 10.0 / 10.0 m/s at
  30 / 60 / 144 fps written with `=`, and 5.0 / 10.0 / 24.0 written with `+=`. The component gives
  10.0 at all three (`tests/external_force.rs`). `ExternalForce::at_point` is a force at a point
  as force plus torque.
- **`kayik`, the thirty-first game** — a motorboat race through buoy gates on a lake, side gusts
  rolling the boat; the first game carried by water. `kayik --test` lets an autopilot run the six
  gates headlessly through a gust every four seconds and checks the hull floats at the depth
  Archimedes gives and stays upright. It found the buoyancy item below and `ExternalForce` above.

### Fixed

- **What floats, floats as it should (`FluidZone`).** A zone pushed only on a body whose *origin*
  was inside it, with one force at the centre of mass worked out from the vertical extent alone.
  Measured: a box of density 300 floated **half** under, not at the 30 % Archimedes gives (its
  origin at the surface, where the push switched off); a hull tipped 30° stayed at 30.0° for good;
  a hull spun at 3 rad/s still turned at 2.46 rad/s after 4 s. A non-spherical body is now a
  3×3×3 grid of cells through its local bounds, each displacing its share of the water and
  dragged by its own velocity, **at the cell** — the density-300 box floats 30 % under, the hull
  rights itself (largest tilt 9.8° in the tenth second, 2.2° in the thirtieth) and spinning meets
  the water. A ball is one cell, as before. `tests/buoyancy.rs`; dropping the torque or going back
  to the origin test each turns it red.

## [0.11.0] — 2026-11-04

Eleven weeks of work since `0.10.0`, most of it found by writing games on the engine — thirty of
them now live in `demo/src/bin/`, each with a headless `--test` mode, and each entry below that
came from one says which. The MSRV is unchanged (**1.96**; CI's MSRV job builds the workspace on
it). The minor moves because public signatures did, as they may under `0.x`: read the
**Changed** and **Removed** sections before upgrading.

What arrived, in short:

- **Game layer:** tweens and easing, timers, screen shake and fades, a seeded RNG, save data,
  localisation with plurals, driven state machines (`App::add_state`), deferred bundles,
  `Query::single`, optional components in queries, systems that return `Result`, system chains.
- **UI (`gizmo-ui`):** clicks, scrolling with flicks and a scrollbar, text input, sliders,
  toggles, progress bars, dropdowns, drag and drop, tooltips, z-layers, clipping, grids,
  `UiScale`, tweened boxes, on-screen touch controls, and touch gestures.
- **Rendering:** reflection probes (box-projected, blended, captured by the studio), area and
  sphere/tube lights, light cookies, soft and contact shadows, object motion blur, irradiance
  volumes, a hand-set sky and fog, AgX and PBR Neutral tone mapping, blend modes, font fallback,
  per-tile tints; drawing many objects 52 % cheaper on the CPU.
- **Physics:** a `Breakable` breaks in a game and its debris is drawn and cleared; characters
  push crates and walk through triggers; moving platforms carry them; `KinematicTarget`;
  compound colliders; terrain collides as the surface it draws; joints report their load;
  cloth and rope are drawn, cloth pushes rigid bodies back and rope lies on what it hits.
- **AI:** crowd separation, routes searched in parallel, straight (smoothed) paths, no
  re-search of an unreachable target every frame.
- **Networking:** rollback with game state, time sync and a UDP transport that only takes the
  peer's datagrams; the client-server half is reachable from the facade
  (`network-client-server`) with a wire format, send/receive and the messages reconciliation
  needs.
- **Audio:** fades, reverb, a master limiter, positions/seek, and synthesised sounds.
- **Scripting:** Lua can spawn what it then moves, recolour and re-mesh it, and read the world.

### Added

- **`Renderer::sky` — the sky that lights the scene, by hand.** The light a surface gets from its
  surroundings — ambient and reflections — came from four procedural skies baked into the
  shaders; `Camera::clear_color` and `Renderer::fog` could make the background and the haze any
  colour, but a shiny thing still reflected the preset's sunset. `Sky::gradient(zenith, horizon,
  ground)` (and `Sky::uniform`, `with_intensity`) replaces the preset's sky outright on both
  shading paths: the gradient around a surface's normal lights it, the one along its mirror ray
  (softened toward the average as it gets rougher) is what it reflects; reflection probes still win
  inside their boxes. It is not the background — the docs say to pair it with the clear colour
  and fog. Three vec4s appended to `SceneUniforms` (fixed part 608 → 656 B, light ceiling
  1014 → 1013; every size pin and ENGINE.md updated). Measured with a matte and a metal cube under
  no light at all: the preset leaves them grey (103 106 113 / 83 86 91 deferred), a green sky
  turns them green (31 230 38 / 6 224 7; forward 0 229 0 / 0 251 0), intensity 0 leaves them dark;
  dropping either shader's branch fails the test. Found by `kosucu`, whose gold coins were grey
  under a pale blue sky.
- **`NavAgent::separation_radius` / `separation_weight` (`with_separation`).** How far other
  agents push this one, and how hard, were hard-coded (1.5 m, 1.5): a formation spaced tighter
  than 1.5 m was pushed apart on arrival. Per agent now, defaults unchanged (the step is
  bit-identical for them), in the inspector, and a scene row written before the fields loads with
  the defaults. Tested with two agents walking 1 m apart: the default pushes them apart, a 0.8 m
  radius or a weight of 0 leaves them at 1 m. `komutan` uses 1.0 m for its 1.2 m formation.
- **`Joint::last_force()` / `last_torque()` / `load()`** — what a joint carried in the last step,
  the numbers its `break_force` / `break_torque` are compared with, and the larger of the two as
  a fraction of its threshold (`1.0` breaks). They were `pub(crate)`, so a game could not show how
  close a beam was to giving way. A joint whose bodies are both asleep keeps the last value
  instead of reading 0. Measuring them found the bug below.
- **`Renderer::load_system_font()`** — a font the machine already has: the file `GIZMO_FONT` names
  (with a warning, and on to the next, if it cannot be read), else the first readable one of the
  platform's usual sans faces (`SYSTEM_FONT_PATHS`: DejaVu, Noto, Liberation, Cantarell on Linux;
  Helvetica, SF, Arial on macOS; Segoe UI, Arial, Tahoma on Windows); `FontError::NotFound`
  otherwise, and always on the web. Still not a bundled font — that stays the project's licensing
  call. Twenty demos carried their own copy of the search (Linux paths plus one guess each for
  macOS and Windows, and `duello` a symbol face besides); they call this now. GPU-tested: the
  named file is taken (told apart by the synthetic face's ascent), a bad one falls through to the
  system's; ignoring `GIZMO_FONT` fails it.
- **`VehicleController::four_wheeled(track_width, wheelbase, radius)`** — a car in one call: a
  wheel at each corner, front axle toward −Z (the controller's forward), in the order the wheel
  vector's docs call load-bearing, and `tuning.track_width` / `wheelbase` set to where the wheels
  are (the anti-roll bar and Ackermann read them; a hand-built car that left them at the defaults
  steered as some other car — `pist` did). Every car in the tree had written the same four-corner
  loop. Tested driving: straight is −Z, a positive steering input turns left; putting the front
  axle at +Z fails it. `pist` uses it.
- **`gizmo_core::time::unix_time()`** — wall-clock seconds since the Unix epoch, for what happens
  while a game is closed (crops that grew, energy that refilled) or a save's timestamp. `Time` is
  the frame clock and starts at zero every run; `std::time::SystemTime::now()` works natively but
  panics on `wasm32-unknown-unknown`, so a game calling it directly crashed in the browser. This
  reads the browser's clock there. Doctest.
- **`Color::srgb` / `srgb_u8` / `srgb_hex`** — a colour as a picker, a paint program or CSS gives
  it (sRGB), converted to the linear values the engine's colour slots read. `Color::hex` and
  `rgb8` take their numbers literally, so a palette's dark green came out grey-green; their docs
  now say so and point here. Tested at the fixed points, the linear toe, the power segment and
  the alpha (which passes through).
- **`From` between `gizmo::math::Ray` and `gizmo::physics::Ray`** — the cursor's ray and the
  physics query's were two types with no conversion; `world.raycast(&cursor_ray.into(), …)` works.
- **A `Breakable` breaks in a game, and its pieces are drawn and cleared away.** Nothing ran
  `physics_fracture_system` — not `FixedPhysics`, not `PhysicsPlugin` — so a breakable wall in a
  game never broke unless the game called it by hand; when it did, the debris was bare rigid
  bodies with sphere colliders and no geometry (the wall vanished and nothing fell;
  `advanced_physics` gave every mesh-less body a sphere), each inheriting the wall's `scale`, and
  `Breakable::debris_lifetime` was metadata nobody read, so every piece stayed in the solver for
  good. Now the facade's physics step (`cpu_physics_step_system`, which both drivers run) runs
  fracture; each piece carries a new `Debris` component — its Voronoi cell in its own frame, the
  source id, its age — and is drawn as that cell in the broken body's own `MaterialDesc` from
  the frame it appears (read before the body's despawn lands); pieces have a scale of one and
  are despawned at `debris_lifetime`. Tested with a 50 kg ball dropped on a red box: the box is
  gone, every piece is red with 12+ vertices; with a 0.5 s lifetime none is left a second
  later. Removing the fracture call, the material copy, the scale reset or the ageing each fails
  a test.
- **A character pushes the crates it walks into (`CharacterController::push_force`, 400 N by
  default; `0` turns it off).** The controller treated every collider as level geometry: walking
  into a 10 kg crate stopped the character at it as at a wall (player x 1.20, crate x 2.00 after
  3 s). Now the bodies that stop its sweep are reported (`update_character_with_hits`;
  `update_character` keeps its signature) and `character_controller_system` gives each dynamic one
  `push_force · dt / mass` along the walking direction, never past the character's own speed —
  a light crate goes at walking pace (x 9.57 after the same 3 s, the player at its face), a 500 kg
  one stays put because its friction is more than the push, and kinematic or static bodies are
  never touched. A scene file written before the field loads it with the default. Removing the
  push, the speed cap (the crate reaches 3.55 m/s ahead of a 3 m/s walk) or the serde default
  each fails a test.
- **A character walks through trigger colliders, and says which it is inside
  (`CharacterController::triggers`).** The controller swept against every collider alike, so a
  door sensor (`Collider::with_trigger(true)`) stopped a character as a wall would (x 1.20 in
  front of a sensor at x 2), a sensor slab under its feet was ground, and a character — not in the
  physics world without a `RigidBody` — was never named by `trigger_events()` (0 events in 2 s).
  Triggers are now left out of the sweep, the ground probe and the platform carry, and the ones
  the capsule overlaps after each step are listed: measured, from x 1.20 to 2.80, exactly
  2 ± (0.5 + 0.3). Removing the filter or the record fails a test.
- **Rollback game state: `RollbackSession::with_state`, `advance_with`, `try_advance_with`.** A
  session kept only the `PhysicsWorld`, so a game's score lived outside it, and a goal scored on
  a prediction that a late input then undid stayed counted. `RollbackSession<T, S = ()>` now
  carries `state: S`, snapshotted with the world each tick and restored with it; `apply` may
  change it, and a `StepRule` runs after every step, re-simulated ones included. Tested with a
  push the remote stops after 4 ticks: the prediction crosses the goal line, the real push does
  not, and the peer ends with no goal; not restoring the state fails it. `new` and `advance` are
  unchanged.
- **Rollback inputs are applied in player-id order on both peers.** They were applied local
  first — opposite orders on the two peers — so any `apply` that depended on order (one player's
  restart clearing what the other's input reads) came out differently. Tested by logging the
  calls; local-first fails it.
- **`LoopbackTransport` is `Send + Sync`** (its queues were `Rc<RefCell<_>>`), so a session over it
  can be a `World` resource — a local two-player game over a simulated link.
- **A rope lies on what it falls on (`Rope::step_with_colliders`).** A rope knew the floor at
  `y = 0` and nothing else, so one dropped across a crate fell through it. Free nodes are now
  projected out of spheres, boxes and capsules (the shapes cloth drapes over) by half the rope's
  thickness, losing their velocity into the surface; `rope_step_system` passes the world's
  colliders. One-way, like cloth before it. Tested: a rope dropped across a crate rests on its top
  with its ends hanging; without the colliders it lies on the floor.
- **A `Rope` component is drawn** (`rope_mesh_system`, `Rope::thickness`) — the same gap `Cloth`
  had: the physics step stepped every `Rope` in the world and no draw path knew the type
  (`advanced_physics` kept its rope outside the ECS and put a sphere on each node by hand). Both
  draw paths now give it a square tube `thickness` across along every link, in the entity's own
  frame. Tested (four links → 96 vertices, all within the rope's width of its line).
- **The reference `server/` speaks the protocol a client can reconcile against.** It applied no
  input, sent `InputAck` reliably and an entity-keyed `WorldStateUpdate` unreliably — the shape
  `kovala` showed a client cannot use. It now moves players by their inputs (a documented
  reference step), sends each client `PlayerState`, keys `WorldStateUpdate` by client id, and goes
  through `gizmo-net`'s own codec and `send`/`receive` (its private bincode copy is gone). The
  netcode docs that said nothing in the repository drove the client half now point at `kovala`.
- **`arazi`, the thirtieth game** — a checkpoint race over generated hills: the vehicle model on
  the heightfield terrain, the first time the two met. `arazi --test` lets an autopilot drive two
  minutes headlessly — all five checkpoints, the body never closer than 0.66 m to the ground. It
  found nothing missing.
- **`Renderer::default_font()` and `gizmo::simple::set_text`** — the two helpers every game wrote.
  33 demos carry the same `load_font` (the system font, else the synthetic face) and 16 the same
  `set_text` (write a `Text`'s content only when it changed). Now one call each: `default_font`
  cannot fail the setup, `set_text` returns whether it wrote. The demos keep their copies; new
  games use these. Tested (the font on the GPU path).
- **Agents walk straight lines, not staircases (`NavGrid::find_smooth_path`, `smooth_path`).**
  Routes were raw 8-connected cell centres, so an agent sent anywhere off a row, a column or an
  exact diagonal walked a staircase: on an open grid from (0.5, 0.5) to (9.5, 3.5), 9 waypoints and
  10.3 m against one waypoint and 9.5 m. The route is now straightened — each waypoint a line of
  sight can skip is dropped, the line checked along itself and 0.3 cell to each side so the agent
  keeps room from walls — and `NavAgent` routes come from it. `find_path` is unchanged. Measured
  in a game: `karinca`'s colony carries 1198 pieces in 40 s, up from 748. Tested on an open grid
  and round a wall (the straightened route never crosses a blocked cell). The straightening first drops
  the waypoints in the middle of straight runs, so its line-of-sight scan sees only corners:
  measured on 1000 routes of a cluttered 100 × 100 grid, its cost per route fell from ~33 µs to
  ~9 µs (1000 agents' first frame 13.4 → 10.5 ms).
- **Client-server reconciliation has the pair it needs (`ServerMessage::PlayerState`), a game can
  send its own data (`ClientMessage::Custom` / `ServerMessage::Custom`), and the facade reaches
  this half at all (feature `network-client-server`).** Reconciling replays the inputs after the
  server's ack on top of the state the server had *after that input* — and the protocol only had
  the two apart: `InputAck` (the reference server sends it reliably) and `WorldStateUpdate`
  (unreliably, keyed by entity index, so a client could not find itself). Paired from different
  ticks, the replay starts from the wrong base: an ack for tick 10 with tick 8's state pulls a
  perfect prediction back two steps (a new test in `prediction`). `PlayerState` carries the ack,
  the position and the velocity in one message to that client. The two message enums were fixed,
  so a game could not send a score or whose turn it was; and `gizmo::net` exposed only rollback,
  `client_server` was configured out of every facade build. `kovala` plays on all three: two
  clients reconcile ~300 times each in its headless test with a largest correction of exactly 0.
- **`kovala`, the twenty-ninth game** — tag on an authoritative server with three server-side
  bots; clients predict themselves and interpolate the rest. `kovala --test` runs a server and two
  clients over 127.0.0.1 without a window. It found the items above and below.
- **Client-server messages have a wire format, and the two ends a send/receive pair.**
  `ClientMessage` / `ServerMessage` were serde types with no encoding: a game talking to a client
  had to depend on `renet` and a serializer itself, choose channel numbers and a byte format, and
  keep both ends agreeing by hand. `encode` / `decode` now (bincode `legacy()`, the rollback
  packets' bytes; trailing bytes refused), `NetworkServer::{clients, send, broadcast, receive}`
  and `NetworkClient::{is_connected, send, receive}`, and `NetError::Codec`. No test had connected
  the two ends: `tests/client_server_link.rs` does it over 127.0.0.1 — connect, an input up, a
  world state down — through these calls only.
- **`UdpTransport` takes only the peer's datagrams, and says which port it got
  (`UdpTransport::local_addr`).** Every datagram reaching the port was handed to the session as
  the remote player's input, so anyone who could reach it — or a peer left over from the last match
  on it — could play for the other side. Once the peer's address is known (set, or learned from
  the first packet) others are dropped. And `bind(0)` lets the OS pick a port, but nothing said
  which, so the peer had nowhere to send. The real-UDP path had never been run by a test or a game:
  `tests/udp_session.rs` now plays a session over two sockets on 127.0.0.1 to the hash of the
  same match in one world, and shows a third socket's forged input ignored (it was taken before
  the fix); `itis --test` adds the same match over real UDP to its loopback run.
- **`Cloth::damping`** — the share of its velocity a node keeps over a second; `0.99` (the old
  hard-coded value, still the default, so nothing changes) is next to none. With the cloth pushing
  bodies back an undamped cloth is a trampoline; measured, a ball's height in the fourth second on
  a hammock ranges over 19 cm at `0.99` and 4.6 cm at `0.05`. At `0.05` the ball is at rest by the
  tenth second (this entry said ~5 cm remained for good, "a limit cycle of the coupling" — that was
  the fourth second, a slow bounce still decaying; measured over fifteen seconds 2026-11-04, and the
  test now asserts rest in the tenth). Tested; hard-coding `0.99` again fails it.
- **`file`, the twenty-eighth game** — catch falling fruit in a net you slide along; the net is a
  `Cloth`, the fruit rigid bodies it holds. `file --test` drops ten on a still net headlessly and
  checks all ten are caught. It found the damping item above.
- **Cloth pushes back: a hammock holds a ball (`Cloth::step_coupled`).** Cloth was strictly
  one-way — it draped over rigid bodies and pushed none back — so a 1 kg ball dropped 0.8 m onto a
  hammock went through it to the floor, the cloth parting around it. `cloth_step_system` now does
  what `soft_body_step_system` did: reads the bodies' mass and motion (`collect_rigid_reactions`),
  resolves each node contact against the body's velocity at the contact instead of a wall at
  rest, and pays the body `−m_node · Δv` (with the angular part) through
  `apply_reaction_impulses`. Static colliders keep the exact old arithmetic, and `Cloth::step` is
  the one-way step as before. Tested with 1 kg and 5 kg balls on a pinned hammock; dropping the
  payment puts both on the floor. Not done: the cloth has next to no internal damping, so a ball
  keeps bouncing on it at about the speed it arrived with — energy kept, not gained (measured).
- **A `Cloth` component is drawn.** The physics step simulated every `Cloth` in the world and no
  draw path knew the type; the cloth did not even keep the grid width that turns its nodes back
  into a surface (`cloth_demo` kept its cloth outside the ECS and built its own GPU mesh in a
  render hook). `Cloth::grid_size` and `Cloth::surface_triangles` (whole cells only, so a tear is
  a hole) now, and `gizmo::systems::cloth_mesh_system` — called by both draw paths — writes both
  faces of it as the entity's `MeshData` every frame, adding a renderer and a white material the
  first time. Tested in the soft crate and the facade; drawing torn cells fails both.
- **`makas`, the twenty-seventh game** — cut a marked region out of a hanging curtain with the
  mouse; `makas --test` cuts an octagon around it headlessly and checks it fell and the rest
  still hangs. It found the item above.
- **`karinca`, the twenty-sixth game** — an ant colony of 600 `NavAgent`s; walls the player drops
  re-route the whole colony on one frame. `karinca --test` runs 40 s headless. It found the two
  AI items below.
- **A crowd's routes are searched in parallel, and an unreachable one is not searched for every
  frame.** Every route search ran on one thread inside the steering loop, so a crowd that got a
  target on one frame paid them all at once; and an agent whose target was walled off cleared its
  path on the failed search, which counted as "route used up" and forced another search on the
  next frame, for ever. The navigation pass now decides (in agent order), searches (in parallel
  on native, `rayon`; in sequence on wasm and for fewer than 16 requests) and then steers — the
  routes are the ones a search of each gives, the same on every run — and a failed search waits
  for the schedule, a wall change or a target move (`NavAgent::route_failed()` tells a game).
  Measured on a 100 × 100 grid with 1500 walls, the frame every agent gets a new target / the
  median frame: 1000 agents 44 → 8.4 ms / 2.9 → 0.88 ms; 2000 agents 86 → 17 ms / 4.8 → 1.8 ms;
  4000 agents 168 → 106 ms / 10.4 → 7.5 ms. Tested for the routes and their determinism, and for
  the retry (the old rule shows 29 searches in 30 frames).
- **An entity script has the world too (`ScriptEngine::update_entity_in`).** `on_entity_update`
  ran without it, so `entity.spawn` returned `nil` there and `physics.ground_at` did not exist —
  only the global `on_update` had them. The play loop now calls `update_entity_in`, which runs the
  hook inside the same two windows; `update_entity` is unchanged. Tested both ways; dropping the
  windows fails it.
- **`yilan`, the twenty-fifth game** — snake with every rule in Lua, run through the facade's
  `PlayLoop`; `yilan --test` lets the script play itself and checks the board. It found the
  scripting items below.
- **Lua can make what it then moves: `entity.spawn` returns the new id; `set_mesh`, `set_color`;
  the host reads and sets a script's globals; a script can be loaded from a string.** During
  `on_update`, `entity.spawn` / `spawn_prefab` reserve the entity from the world's allocator and
  return its id (they returned nothing, so a script could not move, colour or destroy what it had
  made); commands naming that id in the same frame land on it (`ScriptCommand::SpawnReserved`).
  `entity.set_mesh(id, key)` gives it a `MeshSource`; `entity.set_color(id, r, g, b[, a])` is handed
  to the host, and the play loop makes it the entity's `MaterialDesc` — a script-spawned entity with
  both is drawn. `ScriptEngine::get_global` / `set_global` read and write a script's own globals as
  `ScriptValue`s (the host had no way to read a script's score), and `load_script_source` loads a
  script from text (`include_str!`). Tested in the scripting crate and the play loop; running the
  scripts outside the reserving window makes the id tests fail.
- **Editing a `MaterialDesc` at run time recolours the object.** The per-frame sync rebuilt every
  description from its live `Material`, so a game that recoloured through the description — `kapi`'s
  plate lamps, `kopru`'s beams going red under load — had the edit undone before it was drawn: those
  two features never showed, and nothing said so. The sync now tells which side was edited (a
  hidden copy of the description as it last left it) and carries the edit the other way: an edited
  description is drawn, an edited material is saved. A description naming a new texture drops the
  material so it is rebuilt with that texture. GPU-tested both ways and for the texture; the edited-
  description test failed before the fix.
- **`golf`, the twenty-fourth game** — three holes on a generated hilly course, built in memory
  from `terrain_triangles` and `terrain_collider`; `golf --test` plays them with an AI and checks
  the ball never sinks into the ground. It found the terrain item below.
- **A terrain collides as the surface it draws.** Its collider was a box, `width/2 ×
  max_height/2 × depth/2` about the entity, while the mesh runs from 0 to `max_height` — every
  body on a terrain rested on a flat lid at half its tallest hill, floating over the valleys and
  sunk into the peaks. It is a `Collider::heightfield` of the same samples now
  (`gizmo::systems::terrain_collider`, used by the game's frame and by the studio alike), and the
  mesh is split along the heightfield's own cell diagonal — it used the other one, so even with
  the right collider a sloped cell was drawn and hit as different triangles. The triangle list
  is `AssetManager::terrain_triangles`, buildable without a GPU. A scene saved with the old box
  gets the heightfield on load; a box the author chose stays. Tested: every drawn triangle is a
  collider triangle, and a ray down inside each one lands at the drawn height; the old diagonal
  or a wrong height scale fails both.
- **`SceneData::of_entities` / `SceneData::instantiate` — part of a world as a scene.**
  `SceneData::save` writes the whole world, camera and HUD included, so a game saving its level
  that way gained a camera on every load; `serialize_entities` took a chosen set, but
  `SceneData` is `#[non_exhaustive]` and a game could not build one from it except field by
  field on a `default()`, with the joints left out. `of_entities(world, ids, registry)` keeps the
  joints whose two bodies are in the set; `instantiate(world, registry)` spawns it and re-points
  them at the new ids (shared with `load_into`). Tested with four bodies, two joints and a
  three-body part; dropping the filter or the joint re-pointing fails it.
- **`atolye`, the twenty-third game** — a building sandbox that saves and loads its level as a
  scene; `atolye --test` round-trips 30 blocks without a window. It found the item above.
- **`itis`, the twenty-second game** — a two-player shove match over `RollbackSession`: local,
  UDP, or `itis --test`, which plays 1500 ticks over a lossy, laggy link with a late start and
  checks both peers end on the real match. The first game on the netcode; it found the items
  above and the time sync below.
- **Rollback time sync: `RollbackSession::frames_ahead` / `try_advance` / `poll_remote`.** The
  session never waited for its peer — it predicts — so nothing kept two peers' clocks
  together: one that started 50 frames earlier stayed 50 ticks ahead, every remote input
  reached it past the rollback window, and the two never agreed again (measured: different
  hashes after a 50-frame late start against a 32-tick window). `frames_ahead()` is how far
  this peer has simulated past the newest remote input; `try_advance(input, apply, max_lead)`
  advances only while that is within `max_lead`, otherwise polls and returns `false`. With it
  the same late start ends on the same hash as both peers' real run. Skipping the hold, or not
  tracking the remote tick, fails the tests.
- **A simple scene has `Gizmos`.** `with_simple_scene` inserted no `Gizmos`, so a game's
  `get_resource_mut::<Gizmos>()` came back empty and every `draw_line` went nowhere without a
  word. An empty one is inserted now unless the setup brought its own; nothing draws into it on
  its own, so no picture changes. Tested, and removing the insert fails it.
- **`vinc`, the twenty-first game** — demolition with a crane: drive the hook, swing a 300 kg
  ball on a rope into a building of breakable blocks, bring four fifths of it down in 45 s. It
  found the fracture and `Gizmos` items above.
- **`kapi`, the twentieth game** — a box puzzle: push crates onto pressure plates and the doors
  of their group sink into the floor; stand on a plate and it opens only while you stay. It found
  the two character items above (pushing, triggers).
- **`gizli`, the nineteenth game** — a stealth game: guards patrol, their vision cones are rebuilt
  every frame from rays against the walls, suspicion fills while you are seen. It found the four
  items above.
- **`bostan`, the eighteenth game** — a garden that grows while the game is closed: plant, wait,
  harvest, buy dearer seeds and watering; planting times are wall-clock and saved, so the next
  launch counts what ripened. It found the two items above.
- **`kopru`, the seventeenth game** — bridge building: click beams between grid nodes, then a
  120 kg ball tries to cross; beams turn red with their joints' `load()` and joints past their
  threshold snap. Its three gaps — the unreadable joint load, point joints breaking at half their
  rating, a despawned body's joints catching the next owner of its id — are fixed (see above).
- **`pist`, the sixteenth game** — a three-car, three-lap race on a stadium track with barriers,
  driven by the engine's own vehicle model (`VehicleController`), a chase camera and a minimap in
  the corner. Its two gaps — cars that did not move under `FixedPhysics`, and the minimap
  blacking out the main view — are fixed (see Fixed).
- **`komutan`, the fifteenth game** — a small real-time strategy skirmish: box-select units with
  the mouse, right-click to march them in a square formation, they fire at what comes into range;
  twelve against fourteen. Health bars are UI nodes placed over each unit with
  `world_to_screen` every frame.
- **`pointer::world_to_screen(world, point)` and `Camera::world_to_screen`** — the pixel a world
  point is drawn at, `cursor_ray`'s inverse: what a box selection tests units against and where a
  health bar goes over a 3D thing. Top-left origin like `Input::mouse_position`; `None` behind the
  camera; offset into the camera's viewport in split screen. Tested as the inverse of the cursor
  ray at three cursors in two window shapes, behind the camera, and in a right-half viewport;
  dropping the behind-camera test, the y flip or the viewport offset fails it.
- **`Display::Grid` in `gizmo-ui`.** Explicit tracks (`GridTracks` of `GridTrack::{Px, Percent,
  Fr, Auto}`, up to 16 per axis, held inline so `Style` stays `Copy`), one size for implicit rows
  and columns (`grid_auto_rows` / `grid_auto_columns`), and placement by line, span, or both
  (`GridPlacement`); the gaps are `row_gap` / `column_gap`. taffy's grid algorithm had been
  compiled in all along and unreachable. `birlestir`'s six-column field was a *wrapping row* whose
  width was set to six cells and five gaps — the column count hidden in an arithmetic that a
  changed cell size would have broken silently; it is a grid now. Tested through the layout
  system (pixel, `fr` and percentage tracks, gaps, implicit rows, a span, a line, a line with a
  span, explicit rows); breaking any one of eight conversions fails a test. **Breaking for a
  `Style { .. }` literal written without `..Default::default()`** — six fields were added.
- **`world_matrix(world, entity)` / `world_position` — where an entity is now.** In game code
  `GlobalTransform` is a frame behind: propagation runs after the update, so an entity spawned
  this frame still holds the default (the origin) and the child of something that moved this
  frame is where it was. A collision test against freshly spawned obstacles reads them all at the
  origin. These walk up `Parent` and compose this frame's `Transform` fields; `None` where
  propagation would not reach (no `Transform` up the chain, a dead parent id, a cycle). In the
  prelude. Tested against propagation on a moved, turned, scaled chain (equal after it runs, ahead
  of it before), and on each broken chain; composing in the wrong order or treating a dead parent
  as a root fails them. `kosucu`'s road chunks sit at their own z now and it reads its obstacles
  through it.
- **`kosucu`, the fourteenth game** — a three-lane endless runner: the road is spawned ahead and
  despawned behind in 30 m chunks, the speed climbs, swipes work on a touch screen. Memory held at
  89 MB over three minutes of the autopilot spawning and despawning ~75 entities a second.
- **`KinematicTarget` — a kinematic body driven to a pose.** A game knows where a tilting board,
  a door or a lift should be; turning that into a `Velocity` was its job, and with a fixed-step
  driver it was wrong, because a frame runs zero, one or several steps. Now the physics step sets
  the velocity that reaches the target *in that step* (so what rests on the body is carried by
  the motion — the centre-of-mass offset of a turn included), and puts the body on the target
  exactly afterwards; an unchanged target holds it still. Ignored on a body that is not kinematic.
  Tested: one step to arrive, held exactly for sixty, the step's velocity being the motion, a
  dynamic body ignoring it, a tilting board rolling the ball on it downhill and keeping it on;
  dropping the velocity or the landing each fails a test. Found writing `labirent`, the
  thirteenth game (a marble maze).
- **`Collider::compound([(Transform, Collider), …])`** — one collider from several shapes. Writing
  the `Compound` variant by hand meant boxing every shape (`tilemap_collider` said so in a comment
  and now uses this).
- **Font fallback: `Renderer::add_font_fallback(font)`** (`FontLibrary::add_fallback`). A
  character a text's face does not cover was drawn as that face's "missing" glyph — `duello`'s
  shield ⛨ came out as DejaVu's empty box, and the first version wrote "Kalkan" instead. Faces
  added to the chain are tried in order for each such character; the text's own face keeps every
  character it has, line height stays its own, and kerning applies only within one face.
  `PositionedGlyph` carries the face its glyph comes from (`font`), which the atlas is keyed by.
  Tested with two synthetic faces covering different characters (`synthetic_face_drawing`): the
  missing one comes from the fallback at the same advance, the chain rejects unknown and repeated
  ids, a face does not fall back to itself; inverting the coverage test fails it.
- **Moving and fading UI boxes: `gizmo::ui::UiTransform` and `UiTween`.** A box's place comes
  from its `Style`, recomputed by layout every frame, so sliding a card from the deck to the hand,
  letting a dragged item follow the pointer or sending it back meant rewriting `inset`s by hand
  (`birlestir` turned its dragged item `Absolute` for it). `UiTransform { translation, opacity }`
  is applied after layout, like a CSS `transform`: the box and its children are drawn **and
  hit-tested** where it moves them, siblings do not reflow, and the translation is in layout
  pixels (scaled by `UiScale`); the opacity multiplies the alpha of the box's background, its
  extra parts, its text and everything under it. `UiTween::translation(to, s)` / `opacity(to, s)`
  (`and_…`, `with_ease`, `with_delay`, `is_finished`) eases it on real time from wherever it is
  when it starts, landing exactly; an element without a `UiTransform` is given one. Tested on the
  delay and its overrun, starting from a value moved meanwhile, exact landing, an overshooting
  curve keeping opacity in range, the layout (children follow, siblings stay, ×2 scale), the
  inserted transform, and inherited opacity; four mutations, four failures. Found writing
  `duello`, the twelfth game (a card duel).
- **`Ease` lives in `gizmo-math` now** (`gizmo_math::Ease`), so `gizmo-ui` eases with the same
  curves without depending on `gizmo-animation`; `gizmo_animation::tween::Ease` re-exports it and
  every existing path still compiles.
- **`Query::entity` on every query**, not only read-only ones: it reads no component.
- **`gizmo::ui::Tooltip`.** `Tooltip::new("…")` on any element with an `Interaction`: after the
  pointer rests on it for `delay` seconds (0.5 by default, real time) with no button held, the
  facade draws the text beside the pointer — light on a dark box, in the element's own `Text`
  font and size (scaled with `UiScale`), `\n` for lines, wrapped at 320 layout pixels, flipped to
  stay in the window, above every UI layer and under an open dropdown list. Keyboard focus alone
  does not show it; leaving, a press (either button) or an empty text hides it. `birlestir` built
  its own first — an absolute node moved to the pointer each frame — and uses this now. Tested on
  the delay, following, leaving, both buttons, empty text and focus (five mutations, five
  failures), the box placement, and on the GPU (a dark box with light glyphs beside the pointer,
  nothing changed elsewhere).
- **`gizmo::ui::UiScale` — a layout that fits the window.** `Val::Px` was always a window pixel,
  so `birlestir`, laid out for 1280×720, did not fit the 472×492 window a tiling window manager
  gave it. Insert `UiScale::fit(1280.0, 720.0)` and the layout is scaled to the window every frame
  (the tighter axis decides, up or down), or `UiScale::fixed(k)` for a plain factor. The layout
  is computed in layout pixels (window ÷ factor) and every `Node` is written in window pixels, so
  hit testing, drags, drop targets and drawing need nothing; a `Scroll` offset stays in window
  pixels, its extent scaled. A `Text` on a UI node is drawn at the factor too (font and wrap
  width). Tested: sizes, padding, a percentage width, a `fit` following a resize, a scrolled list
  at ×2; dropping any of the four conversions fails it.
- **Drag and drop: `gizmo::ui::DropTarget`.** What a dragged item was let go over could not be
  found by asking what is under the pointer — the item follows the pointer, so it is what is
  under it, and every slot beneath reads `Interaction::None`. `ui_drop_system` (after
  `ui_drag_system`) finds the target holding the drag's position and answers on both ends:
  `DropTarget::hovered_by()` / `dropped()`, `Drag::over()` / `dropped_on()`, the drop for the one
  frame of the release. The highest `ZIndex` layer wins, then the smallest of nested targets; a
  target clipped out of its `ClipContent` / `Scroll`, one with no size, and the dragged element
  and its subtree never count. Tested on hover, drop, one-frame drop and letting go over nothing,
  and on each tie-break; dropping the subtree exclusion, the area order, the layer order or the
  clip check each fails a test. Found writing `birlestir`, the eleventh game (a merge garden).

- **A master limiter: `AudioManager::set_limiter(Some(Limiter::default()))`.** Every sound is
  summed by one mixer and the sum had no ceiling: eight sounds at 0.6 peak near 4.8, and the device
  clips it into a crackle. A brick-wall limiter now sits on the sum (instant attack, so no sample
  passes the ceiling; a release, 100 ms by default, so it does not buzz; one gain per frame, so the
  stereo image holds). Bit-exact while the mix stays under the ceiling, and off by default.
  `take_limiter_gain()` reports the deepest reduction since the last call. To get a sum it could
  sit on, every player now connects to a mixer the manager owns, fed into the device's. **That bus
  needed a silent infinite input:** an idle rodio mixer reads as a finished source and is dropped
  for good, so on a real device the first silence unplugged it and every later sound played
  nowhere (measured: removing the keep-alive fails the device test). Tested offline (eight in-phase
  voices limited to ≤ 0.89 with L/R balance exact, under-ceiling and off both bit-exact, the bus
  passing a two-sound sum unchanged after 4 800 idle samples, the release holding then letting go)
  and on hardware; each of three mutations fails a test. `poligon` turns it on for rapid fire.
- **`Query::single` / `Query::single_mut`** — the one matching row, or a
  `QuerySingleError::{NoEntities, MultipleEntities}` saying why there is not exactly one. For *the*
  player or *the* camera, a `for` loop silently runs no body before the spawn and lets the last of
  two matches win; this names both cases. Per-row filters apply as in `iter`, at most two rows are
  walked, and a refused `single_mut` marks neither row changed (tested; dropping the second-row
  check fails both tests).
- **Text pinned to the window: `Text::pinned` / `TextSpace::Pinned`.** `TextSpace::Screen` is
  absolute window pixels, so a title centred for 1280×720 is off screen in a smaller window — the
  new game `kristal_avcisi` opened at 472×492 under a tiling window manager and its menu had no
  text. `Text::pinned(content, font, size, anchor, offset)` puts the text's `anchor` on the same
  point of the window (`Center` centres it, `TopRight` hangs it from that corner) plus `offset`
  pixels, and follows resizes; `Text::pinned_at(content, font, size, at, offset)` pins to any
  fraction of the window (`TextSpace::Pinned { at, offset }`), and
  `TextSpace::screen_point(viewport)` is where it lands. A UI `Node` on the same entity still wins.
  `text.rs` and `kule_yikici` drop their per-frame repositioning from `WindowInfo`. Tested at two window sizes — an absolute reading passes at
  one and fails at the other (mutation-checked).
- **Deferred bundles: `Commands::spawn_bundle` and `EntityCommands::insert_bundle`.** The
  `Commands` twins of `World::spawn_bundle` / `World::add_bundle`: one queued command that spawns
  and fills the entity in one archetype move, instead of one `insert` (and one move) per
  component. Tested for "nothing before the flush, everything after" and for a dead entity being
  left alone; dropping the bundle in the queued command fails the first.
- **A seeded RNG: `gizmo_math::Rng` (in the prelude).** SplitMix64 — every seed valid, the same
  sequence on every platform — with `range(a..b | a..=b)` over `f32`, `i32`, `u32`, `i64`, `u64`
  and `usize` (integer ranges by Lemire's multiply-high with rejection, so no modulo bias),
  `f32`/`f64` in `[0, 1)`, `signed`, `bool`, `chance(p)`, `pick`, `shuffle` (Fisher–Yates),
  `unit_vec2`, `unit_vec3` (uniform in area), `in_unit_sphere`, `in_box`, `fork`, and
  `with_stream(seed, stream)` for one stream per (frame, entity) without the `seed + stream`
  collisions. No `f64` ranges on purpose: a second float impl makes a literal range fall back to
  `f64`, which broke all four call sites converted to it. Pinned to the reference SplitMix64
  outputs for seed 0; uniformity, half-open bounds and sphere uniformity are tested and
  mutation-checked. The private xorshifts in `spotlight`, `parallel_query`, `change_detection`,
  `kristal_avcisi` and the particle emitter are gone — the emitter's sparks draw different (equally
  jittered) values than before, same frame still replays the same.
- **Materials without a GPU: `MaterialDesc::new()` and every `with_*` builder.** A description
  had no constructor or builder, so making one where there is no device — an `on_enter` hook, a
  system — meant writing thirteen fields. The builders are now one macro expanded on both
  `Material` and `MaterialDesc`, and `Material::new` is `MaterialDesc::default()` over a bind
  group, so the two cannot drift. Spawned beside `MeshSource` and `MeshRenderer`, the draw path
  builds the mesh and the material the next frame — a GPU test draws such an entity and fails
  when the description is not resolved. `MaterialDesc` and `MeshSource` are in the prelude.
- **Geometry made in code, without a GPU: `MeshData`.** A triangle list (`MeshData::new`, or
  `extrude`, `sweep`, `sphere`, `cylinder`, `cone`, `capsule`) the asset loader uploads the first
  frame its entity has no `Mesh` — so an `on_enter` hook or a scheduled system can spawn an
  extrusion or a generated chunk, not only a `MeshSource` primitive. Clones share one upload (keyed
  on the allocation; `AssetManager::mesh_from_data`), and `garbage_collect` drops uploads nobody
  draws. GPU-tested end to end — not drawn before the loader, drawn after; disabling the upload
  fails it — and for sharing. In the prelude. `kristal_avcisi`'s crystals are an extruded star.
- **Built-in primitive meshes are uploaded once: `AssetManager::primitive_mesh`.** The asset
  loader built a fresh mesh for every `MeshSource("sphere")` (or cube, plane, …) entity, so a
  hundred described spheres were a hundred identical vertex buffers. Each primitive key is now
  cached under `primitive/<key>` — the prefix `garbage_collect` already kept alive for a cache
  nothing filled. GPU-tested (same buffer twice, survives collection); disabling the cache fails
  it.
- **`lobut` — a bowling game on real rigid bodies**, and what its first run found (all three
  closed; `docs/CAPABILITY_GAPS.md` §C11):
  - **A `RigidBody` without a `Velocity` is simulated.** The ECS bridge gathered
    `(RigidBody, Transform, Velocity)`, so a static floor written without a velocity was not in
    the world — silently; every body above it fell. A static body now joins with zero velocity; a
    moving one is given a `Velocity` via the command queue and simulated from the next frame.
  - **Inertia comes from the collider.** `RigidBody::new` stores a `(1,1,1)` placeholder
    (`RigidBody::UNSET_INERTIA`) that the ECS step now replaces with the collider's tensor on the
    first step and writes back; a hand-set inertia is kept. Before, a 20 cm ball turned ten times
    too hard, never rolled and lost 80 % of its speed in 1.5 s; now it rolls at 6.77 m/s against
    the textbook's 6.80. **Behaviour change** for every ECS body that relied on the placeholder;
    bodies added to a `PhysicsWorld` directly are unchanged, as are the determinism gate's hashes.
  - **`with_simple_scene` keeps a `PhysicsWorld` the setup inserted** — it used to insert its own
    afterwards, unconditionally, so a game's own gravity was silently replaced.
  - **`MeshSource("cylinder")` and `"cone"`**, sized like their colliders, and
    `AssetManager::PRIMITIVE_MESH_KEYS` as the one list of primitive keys (the studio's
    missing-asset check now uses it instead of its own copy).
- **Keyboard focus scrolls into view**: moving the focus with the keys or the d-pad onto an item
  outside its `Scroll` scrolls the list just enough to show it.
- **Per-tile tints: `Tilemap::set_tint`** — fog of war without a dimmed copy of every tile. Found
  by `zindan`, a turn-based dungeon crawler (new demo) built on one `Tilemap`.
- **Held keys repeat in menus: `HoldRepeat`** (`gizmo::core::input`) — fires on the press, then
  after a delay, then at an interval. `UiNavigation`'s arrows and d-pad repeat through it, and so
  do sliders and dropdowns stepped from the keyboard.
- **Flicking lists**: a finger on a `Scroll`'s content scrolls it past an 8 px slop (cancelling the
  tap on the item under it) and flings it, decaying; a tap still taps; the mouse is unaffected.
- **Select lists: `gizmo::ui::Dropdown`** — opens below its button, drawn above every UI layer,
  takes the pointer while open, steps with ←/→ while focused. `menu` has a difficulty list.
- **`Scroll` has a scrollbar** — drawn while something overflows, its thumb dragged by the mouse
  or a finger; its strip does not pass presses to the list items under it.
- **Reverb: `AudioManager::set_reverb`** (`gizmo::audio::reverb::Reverb::{room, hall, cave}`) —
  Freeverb inside every source, live on sounds already playing, bit-exact when off, tails that
  ring past the sound's end. `poligon` plays in a hall (V toggles).
- **`ProgressBar` and `Toggle`** (`gizmo::ui`): a bar filled to a value, and an on/off switch
  flipped by every click (mouse, finger, Enter, gamepad), both drawn by the engine.
- **Drawing many objects is 52 % cheaper on the CPU.** 60 000 drawn cubes: `default_render_pass`
  25.0 → 11.9 ms. The transform sync no longer stamps every `Transform` changed every frame
  (`Changed<Transform>` now means something), and propagation's visited set is a bitset. The per-object motion history (object motion blur) was a `HashMap` swept every
  frame — now a slot per entity id; the batch map uses FxHash instead of SipHash; the batcher no
  longer clones three `Arc`s per drawn object. Frustum tests skip the n-vertex they discarded.
- **Plurals: `Locale::format_count` and `LocalizedText::with_count`** — `key.one` / `key.other` by
  the count, the forms from one language only.
- **`Typewriter`**: a `Text` shown a character at a time (its localized string too), restarting on
  a new string; `skip()`, `just_finished()`. `menu`'s announcements use it.
- **Touch gestures: `Input::pinch`, `two_finger_pan`, `swipe`.** `kale` zooms with a pinch (or the
  wheel), pans with two fingers, and places towers on release so a gesture does not drop one.
- **Where a sound is: `AudioManager::position`, `seek`, `duration`** — and sounds can seek at
  all: the `Muffle` filter now forwards seeks, loops use the engine's own seekable `Looping` source
  instead of rodio's `repeat_infinite` (whose buffer cannot seek), and decoders are built knowing
  their length (symphonia refused random access without it). Found by `ritim`, a rhythm game
  (new demo) that times its notes by the music's position.
- **On-screen touch controls: `gizmo::ui::VirtualStick` and `VirtualButton`.** Each claims the
  finger that lands on it and follows it until it lifts, so two thumbs play at once; the mouse
  stands in without fingers. `nisanci` is playable on a touchscreen.
- **Tween delays.** `Tween::with_delay` / `ColorTween::with_delay`: wait, then start from where
  the entity is at that moment. Found by `mucevher`, a match-3 puzzle (new demo), whose board fills
  column by column.
- **Localisation: `Locale` and `LocalizedText`** (`gizmo::app::locale`, in the prelude). JSON
  string tables per language, fallback to the first language then the key, `{name}`
  placeholders, `missing()`, `system_language()`; a `LocalizedText` beside a `Text` shows the
  current language's string, resolved at draw time so a switch relabels everything at once.
  `menu` is bilingual.
- **UI sliders: `gizmo::ui::Slider`.** On a `ButtonBundle`: dragged (captured) or stepped with
  ←/→ and the d-pad while focused, `just_changed` / `just_released`; the engine paints its fill
  under the label. The menu's volume is one.
- **UI drags: `gizmo::ui::Drag`.** Opt-in on any element: starts on a press that begins on it,
  follows the pointer anywhere until the release (captured), reports the drop for one frame.
- **Touch input.** `WindowEvent::Touch` was dropped, so nothing could be pressed on a phone or a
  touchscreen. `Input::on_touch` / `touches()` / `touch(id)` / `primary_touch()`; the first finger
  drives the mouse (cursor + left button), so UI clicks and mouse-written games work under a
  finger unchanged. `demo-web`'s canvas sets `touch-action: none` so the browser does not take the
  finger for a pan.
- **Building a renderer is ten times cheaper after the first.** Composing the eighteen shaders
  (naga_oil `#import`/`#ifdef`) was 0.99 s of a 1.1 s headless construction; the result is now
  cached for the process, keyed by source, label and defs (a second renderer: 1.1 s → 0.11 s),
  and `naga_oil` + `regex` build optimised in the dev profile (the first: 2.4 s → 1.8 s). The
  ignored GPU tests of `gizmo` went 288 s → 100 s, its regular ones 82 s → 35 s, and the whole
  suite's run 423 s → 312 s. Debug builds of every demo start faster for the same reason.
- **A missing mesh fails once.** An unknown `MeshSource` key (a typo for a primitive, a file that
  is not there) used to retry the disk, log and upload an empty buffer for every entity naming
  it; the failure is now cached until nothing holds it, and the error lists the primitive keys.
  Found by `nisanci`, a twin-stick shooter (new demo).
- **`ScreenFade`** (`gizmo::renderer::screen_fade`): a colour over the whole frame, above every
  UI layer — `fade_out`/`fade_in` for scene changes, `flash` for hits, `just_finished` to swap a
  level behind black. Advanced on the real frame time by both draw paths.
- **Screen shake: `CameraShake`.** On a camera, `add(trauma)`; the shake is a smooth noise scaled
  by trauma² and wears off at `decay` per second. The draw path applies it to the frame being
  drawn and puts the camera back exactly afterwards, so game code, aim rays and the audio
  listener never see it. `poligon` uses it.
- **Audio fades, and a game's `AudioManager` is ticked.** `fade_to`, `fade_out` and
  `crossfade` (menu → level music); `AudioManager::tick(dt)` advances them. `gizmo-app`'s frame
  loop now ticks an `AudioManager` resource every frame — before, nothing did unless the opt-in
  3D audio system was registered, so finished sounds were never collected (a sink and a route
  leaked per sound played) and a volume change never reached a sound already playing.
- **`Timer`** (`gizmo::core::Timer`, in the prelude). A once or repeating countdown ticked with
  the frame's `dt`: `tick` returns how many periods the step crossed and keeps the remainder, so
  a spawner's pace no longer depends on the frame rate the way the hand-written
  `t -= dt; if t <= 0 { t = period }` it replaces in `balon` and `kale` did.
- **UI scrolling: `gizmo::ui::Scroll`.** A node carrying it moves its children by its `offset`,
  clamped by the layout to how far they reach past its box (padding kept at both ends), and
  clips them like `ClipContent`; the mouse wheel scrolls the innermost one under the cursor by
  its `speed`. Its direct children do not flex-shrink, so a long list stays long. A button
  scrolled — or clipped — out of its panel is no longer hovered or clicked where its box would
  be; that applies to `ClipContent` too. `ui_layout` has a twelve-row list in a six-row box.
- **Text input: `Input::typed_text()` and `gizmo::ui::TextInput`.** The characters typed this
  frame as the layout wrote them (`ş`, capitals, repeats; control characters dropped), and a text
  field widget — click to focus, type, Backspace, Enter to submit, Escape or a click elsewhere to
  leave — whose `Text` content is its placeholder and whose value the draw path shows, with a
  caret while focused. A focused field keeps the keyboard from `UiNavigation`. The menu demo
  asks for a name and saves it.
- **UI layers: `gizmo::ui::ZIndex`.** A node's layer, inherited by its descendants. The overlay
  paints layer by layer — backgrounds then labels — so a panel in front hides what is behind it;
  before, every background was under every label. Only the front layer under the cursor is
  hovered and clicked. Screen sprites keep their place between layer 0's backgrounds and labels,
  so a UI without `ZIndex` draws exactly as before. GPU-tested with two overlapping boxes in both
  orders (a first version of the test passed with layers disabled, through archetype order, and
  was rebuilt to fail then).
- **Absolutely-positioned UI roots honour their `inset`.** Taffy puts the root it lays out at
  the origin, so an `Absolute` root ignored `left`/`top`/`right`/`bottom` and every dialog sat in
  the top-left corner. `left`/`top` (px or % of the window) or, when `Auto`, `right`/`bottom`
  from the far edge now place it; static roots are unchanged.
- **UI clipping: `gizmo::ui::ClipContent`.** A node carrying it cuts its own label and every
  descendant's background and label to its box (nested clippers intersect). Glyph quads are
  trimmed on the CPU together with their texture coordinates, so a cut letter is cut, not
  squashed, and it costs no draw call. `TextRenderer::queue_clipped` / `queue_rect_clipped` are
  the renderer half. `ui_layout`'s overflowing fourth button is now cut at its edge.
- **Colour tweens: `ColorTween`.** `ColorTween::to(colour, s)` and `fade_to(alpha, s)` ease the
  colour of an entity's `Sprite`, `Text` and `Material`, with `Tween`'s curves and repeats (the
  pass arithmetic is now one function, `Repeat::position`). Both draw paths advance it on the
  animation clock. `balon`, a 2D balloon-popping game, fades its popped balloons with it.
- **Particles work in `SimpleApp` games, and emitters can burst.** The simple scene switched the
  GPU particle system (and SSR, SSGI, GPU physics and fluid) off **every frame**, so
  `ParticleEmitter` did nothing in any `SimpleApp` game and a game could not turn SSR back on.
  The particle system now skips its dispatch and draw while nothing alive is in it
  (`GpuParticleSystem::is_idle`) and stays on; the other defaults are set once, before the game's
  setup. `ParticleEmitter::one_shot(n)`, `burst(n)` and `with_burst(n)` emit `n` at once; the
  emission rule is one function (`gizmo::systems::render::emit`) both draw paths call — the
  studio carried a copy of the loop.
- **Saving between runs: `gizmo::app::save::SaveData`.** A game's settings, scores and progress
  as typed key/values (`get::<T>`, `set`, `remove`, `save`), in `<data dir>/<game>/save.json` on
  the desktop and `localStorage` in the browser. Writes are atomic (temporary file + rename); a
  save that cannot be read is moved aside to `save.json.bad` instead of being overwritten. The
  menu demo remembers volume and fullscreen; `kristal_avcisi` remembers its best score.
- **A menu's three missing buttons** (`menu`, `docs/CAPABILITY_GAPS.md` §C15):
  - **Keyboard and gamepad navigation: `gizmo::ui::UiNavigation`.** A resource, off by default;
    on, the arrows / Tab / d-pad move a focus in reading order (wrapping), Enter / Space / the
    south button click the focused element via `Clicked`, the mouse takes the focus with it, and
    the focused element reads as `Interaction::Hovered`.
  - **Fullscreen at run time: `gizmo_core::window::WindowMode`** — borderless fullscreen or
    windowed, applied by the windowed app when the resource changes.
  - **Quitting: `gizmo_core::window::AppExit`.** Inserting it ends the windowed loop (through the
    same path as closing the window, now `shut_down`) and the headless loop, which before had no
    way out at all.
- **Tweens: `Tween` and `Ease`** (in `gizmo-animation`, re-exported as
  `gizmo::renderer::tween`, in the prelude). `Tween::to_position(p, 0.15).with_ease(Ease::OutCubic)`
  on an entity eases its `Transform` there — position, rotation (slerp) and scale, alone or
  combined, once, looping or ping-pong, eleven curves. It starts from wherever the entity is when
  it first runs, so a new tween replacing a running one does not jump. Both draw paths advance
  tweens on the animation clock before transform propagation (the render-parity driver list
  includes it). `kaydirmaca`, a sliding-tile puzzle, is the first user (§C14).
- **`poligon` — a first-person shooting range** (`docs/CAPABILITY_GAPS.md` §C13), and what it
  needed:
  - **Cursor grab and visibility: `gizmo::core::window::CursorOptions`.** A resource
    (`mouse_look()` = locked and hidden, `free()`) the windowed app applies to the window when it
    changes; `Locked` falls back to `Confined` where the platform cannot lock. Before, nothing in
    the engine could hold or hide the cursor.
  - **`gizmo::physics::apply_impulse(world, id, impulse, point)`** — push an ECS body at a point
    (linear and angular, waking it), with the id a raycast hit names. Before, a game borrowed the
    three components and reached into `Integrator`, which the facade does not export.
- **`kale` — a tower defence on `gizmo-ai`, `gizmo-ui` and the mouse** (`docs/CAPABILITY_GAPS.md`
  §C12), and what it needed:
  - **`gizmo::pointer::cursor_ray` / `cursor_on_plane`** — the world under the mouse, through the
    camera under the cursor (split screen included), and **`Ray::intersect_plane`**.
  - **Agents replan when the grid changes.** `NavGrid` keeps a revision that its obstacle
    methods and a rebuild bump (`mark_changed` for direct writes to `obstacles`); the navigation
    system replans every agent on the frame it changes, instead of at the next 0.5 s
    recalculation — until then they walked through the tower just placed.
  - **`gizmo::ui::pointer_over_ui(world)`** — whether the cursor is over any UI node, so a click
    on a panel is not also a click on the world (the game's first version placed towers through
    its own button bar).
- **Optional components in queries: `Option<&T>` and `Option<Mut<T>>`.** Every entity the rest
  of the query matches, `Some` where `T` is present. The access is declared regardless (an
  optional writer serialises like a writer; `(Mut<T>, Option<&T>)` is the aliasing panic); sparse
  storage is tested per row and refuses chunk iteration. Tested for table and sparse presence,
  change marking, aliasing and the declared write; Miri-clean under the CI flags.
- **UI clicks: `gizmo_ui::Clicked`, in `ButtonBundle`.** `true` for exactly the frame the primary
  button is released over the element it was pressed on; released off it, the click is cancelled.
  `Interaction` is a per-frame state and every game had to diff two frames to see a click. The
  interaction system updates it through `Option<Mut<Clicked>>`, so a node without one works as
  before. `ui_layout` counts clicks on its buttons. **`ButtonBundle` gained a field** — a literal
  without `..Default::default()` needs `clicked: Clicked::default()`.
- **Moving platforms carry the character standing on them.** `character_controller_system`
  moves each grounded character by its floor's `Velocity` at the contact point (`v + ω × r`)
  before the step, so a sideways platform takes its passenger along and a turntable carries one
  round. Tested sideways, turning and up/down (the vertical case was already right through the
  ground snap and is pinned so the carry cannot double it); removing the carry fails the first
  two.
- **Tuples are system parameters.** `fn f((time, state): (Res<Time>, Res<State<S>>))` — two to
  eight members, nesting freely — the anonymous form of `system_param!` and the way past the
  nine-parameter limit. Members' states are kept per member (a `Local` in a tuple keeps its value)
  and their access declarations are forwarded one by one; a test compares a nested tuple's
  declaration with the flat list's, and dropping the forwarding fails it.
- **Sound without sound files: `gizmo_audio::synth`.** `wav_from_samples(samples, rate)` encodes
  mono PCM as a 16-bit WAV (clamped, non-finite as silence); `AudioManager::load_samples` registers
  generated PCM; `Blip::new(wave, hz, seconds).with_slide(end_hz).with_volume(v)` renders a short
  effect — sine, square, triangle, saw or seeded noise, an exponential pitch slide, an attack and a
  linear decay to silence, so it neither starts nor ends on a click — and `.wav()` hands it to
  `load_sound_bytes`. It plays through the same decoder as a file. Tested by decoding the encoder's
  output with rodio sample for sample, and on length, peak, clicks, the slide's pitch ratio and
  noise reproducibility; five mutations (no slide, no decay, stuck noise, no clamp, wrong rate in
  the header) each fail a test. `kristal_avcisi` makes its four effects this way.
- **`kristal_avcisi` — a complete 3D game.** Top-down arena, emissive additive crystals, chasing
  hunters, lives, a clock, Menu → Playing → Over on `add_state` / `on_enter` / `DespawnOnExit`,
  gameplay as one `.chain()`, rect lights, hand-set fog, a camera clear colour. What it ran into
  is in its header and in `docs/CAPABILITY_GAPS.md` §C10; the two above came out of it.

- **Object motion blur.** `PostProcess::motion_blur` now blurs moving objects too, not only camera
  motion: the composite reads the G-buffer's velocity (group 4 on native, `VelocityBinding`) for
  geometry, with a neighbour-max search so a moving object's trail reaches the background it just
  uncovered; the sky keeps the camera's reprojection. Under a still camera a cube moved 0.3 m loses
  72 % of its edge energy while a still cube's pixels do not change. Transparent objects are not in
  the G-buffer and blur with what is behind them.

- **Per-object velocity in the G-buffer: `gizmo_renderer::motion`, `Renderer::motion_history`,
  `DeferredState::velocity_view`.** Each entity's previous model matrix is kept and uploaded beside
  the instance buffer; the G-buffer pass writes RT4 `velocity` (`Rg16Float`, screen-UV motion, object
  and camera together; exactly zero for a still object under a still camera). The G-buffer is now 36
  bytes per sample, so native devices request `max_color_attachment_bytes_per_sample` up to 64.
  Groundwork for object motion blur; nothing reads the target yet.

- **`EventMutator<T>`** — a system parameter that changes the readable events in place (a damage
  clamp, a tag) before the readers ordered after it see them; `Events::iter_mut` underneath. In
  the prelude.

- **`Query::entity(id)`** — the generation-checked `Entity` for an id a query yielded, so a
  scheduled system (which never has `&World`) can hand a real handle to `Commands` instead of
  forging `Entity::new(id, 0)`, which is wrong for every recycled id.

- **`World::get::<T>(entity)`** — one entity's component without building a query, from either
  storage. Checks the entity's generation, so a stale handle reads `None` rather than whatever
  entity now has its id. Read-only; writes still go through `query_mut`, which marks changes.

- **Systems may return `Result`.** A system function returning `Result<(), E>` (`E: Display`)
  can use `?`; an `Err` goes to the world's `SystemErrorHandler` resource (a `fn(system, error)`),
  or by default to a `tracing::error!` naming the system, and the system runs again next frame.
  `SystemOutput` is the trait behind it; `()` systems are unchanged.

- **Irradiance volumes you place: `gizmo_renderer::irradiance::IrradianceVolume`.** A component
  with a box, a resolution and an ambient; both render paths bake it from the scene's directional
  and point lights and upload it (`systems::render::sync_irradiance_volume`), and rebake only when
  the volume or a light changes. Saved in scenes. `IrradianceState` gained `baked_from`.

- **Deband dither: `Renderer::deband_dither`.** Half an 8-bit step of noise before the output,
  against banding in dark gradients. Off by default. `PostProcessUniforms::_tonemap_pad` became
  `deband_dither` + a one-word pad.

- **Two more tone-mapping curves: `TonemapCurve::AgX` and `TonemapCurve::PbrNeutral`.** AgX
  (log encode, sigmoid, inset/outset — highlights roll toward white rather than skewing hue) and
  Khronos PBR Neutral (identity below 0.76, so base colours read as authored; a shoulder above).
  ACES stays the default; the enum's existing values are unchanged.

- **Shadow bias multipliers: `ShadowSettings::with_bias(depth, normal)`** (`depth_bias`,
  `normal_offset`, both 1 by default — the previous bias exactly). They scale each path's own
  calibrated depth bias and the cascade normal offset; `SceneUniforms` gained `shadow_bias` at its
  tail (fixed part 608 B) and `ShadowFrame` gained `bias_scale`.

- **Hand-set fog: `Renderer::fog`** (`Fog::new(color, density).with_height(falloff, base)`). Replaces
  the environment preset's height fog outright, so fog can be set without taking a preset's whole
  look. `SceneUniforms` gained `fog` and `fog_shape` at its tail (fixed part 560 → 592 B; the light
  ceiling is 1014), `EnvironmentFrame` gained `fog`. Fixed on the way: a fog with zero height
  falloff divided by zero and vanished.

- **Wireframe colour: `WireframeConfig::color`.** One colour for every wireframe line, readable
  on any material (the lit lines nearly vanished on a pale one). Native only. `WireframeConfig`
  gained the field (a struct literal needs it; it is no longer `Eq`), `SceneUniforms`'
  `_pre_align_pad` became `wireframe_color` + a one-word pad, and `EnvironmentFrame` gained
  `wireframe_color`.

- **Gated observers: `World::add_observer_if(condition, system)`.** A lifecycle observer that runs
  only while a `run_if`-shaped condition holds (`in_state(..)` works as is). The plain
  `add_observer` closure is never shown the world, so it could not gate itself.

- **Background colour: `Camera::clear_color`** (`with_clear_color`, an inspector colour). Fills
  the frame where nothing is drawn — the deferred pass's empty pixels, the forward-only
  pipeline's clear, the editor's game view — per camera. `None` keeps each path's own.
  `SceneUniforms::_align_pad` became `clear_color` (same size and offset);
  `EnvironmentFrame` gained `clear_color`.

- **SSAO radius and bias: `SsaoState::set_shape(queue, radius, bias)`.** The sampling radius
  (0.5 m) and depth bias (0.015) were literals in `ssao.wgsl`; they ride the kernel uniform now,
  with the old values as `SSAO_DEFAULT_RADIUS` / `SSAO_DEFAULT_BIAS`.

- **Surfaces can refuse shadows: `MeshRenderer::receives_shadows`** (`with_receives_shadows`,
  and an inspector checkbox). `false` keeps the sun's cascades, contact shadows and the
  point-light cube off that surface, in the deferred, forward and baked-lit paths; it rides the
  instance's `emissive.w` (`InstanceRaw::with_shadow_receiving`) into the G-buffer's RT5 alpha.
  Scenes saved before it load as receiving.

- **Shadow reach: `Renderer::shadow_settings`** (`ShadowSettings::{distance, lambda}`, with
  `with_distance` / `with_lambda`). How far the sun's cascades reach (100 m by default) and how the
  four split it (λ 0.75) were constants; both paths read the setting now.
  `csm::compute_directional_cascades_with` is the parameterised form, and `SceneSetupInputs`
  gained `shadow_settings`.

- **The game path publishes `RenderStats`** (draw calls, instances, triangles, sampled GPU
  memory), counted as the editor counts them, so a game can read its own draw-call count; the
  `many_cubes` demo shows it. Under split screen it holds the last view drawn.

- **An inert `DespawnAfter` is reported.** With a `DespawnAfter` or `DespawnBelowY` attached and no
  `LifetimePlugin` installed, `default_render_pass` logs one warning per world
  (`lifetime::warn_if_inert`); the plugin inserts `LifetimeActive`, which silences it. A custom
  loop running `LifetimeSystem` itself should insert the marker too.

- **System chains: `(a, b, c).chain()`.** `IntoSystemChain` (in the prelude) turns a tuple of 2–8
  systems, bare or configured, into a `SystemChain` that runs them in the listed order: each gets a
  generated label and an `after` on the one before. `Schedule::add_chain`,
  `App::add_update_chain` / `add_system_chain`, `SystemChain::in_phase`. The `schedule_internals`
  demo's chain mode uses it (one ordering in 650 frames).

- **Driven state machines: `App::add_state`, `App::on_enter`, `App::on_exit`, `DespawnOnExit`.**
  `add_state(initial)` makes `State<S>` a machine the app loop drives: once a frame, before the
  fixed and update schedules, a queued `State::set` is applied and the hooks run (`on_exit` of the
  old state, its `DespawnOnExit` entities despawned with their children, `on_enter` of the new).
  Hooks get `&mut World`. **The initial state's `on_enter` runs on the first frame** — a bare
  `State` never reports it. Also as free functions in `gizmo_core::state` (`add_state`, `on_enter`,
  `on_exit`, `run_state_drivers`) for custom loops. The `states` demo now uses it, without the
  render-closure request resource it needed before.

- **Bloom reach: `Renderer::bloom_radius` and `Renderer::bloom_iterations`.** The radius spreads
  the blur's taps (1 = the previous blur, clamped to 0..=16); the iterations run the separable
  blur again (1–8). Defaults are bit-for-bit the old steps. Native only, as bloom is.
  `gizmo_renderer::post_process::blur_steps` is the step arithmetic.

- **Blend modes: `BlendMode` and `Material::with_blend_mode`.** `Alpha` (the default and the
  previous behaviour), `Additive` (fire, sparks, glows), `Multiply` (tinted glass, stains — alpha
  is not consulted; fade toward white) and `Premultiplied`. Any mode but `Alpha` makes the
  material transparent. Honoured by the PBR and unlit routes in both draw paths through six new
  pipelines (`SceneState::blend_variant`), saved in `MaterialDesc::blend_mode`
  (`#[serde(default)]`), editable in the material inspector, and in the prelude. The
  `blend_modes` demo opens on the four side by side.

- **Light cookies: `SpotLight::cookie` / `with_cookie`, `Renderer::insert_light_cookie`,
  `gizmo_renderer::cookies`.** A spot projects an image through its cone (a window frame, leaves,
  a coloured gel): its colour multiplies the light's. Up to eight cookies resident at once, 256²
  each, in one texture array bound beside the shadow maps (shadow group, bindings 3 and 4). A
  cookie is a name registered from memory, or on native a path read from disk the first frame a
  spot names it (a missing file is logged once and the spot stays a plain cone). The image's up is
  world up as seen down the cone. Native only: the web forward pipeline has no shadow group. The
  `area_lights` demo projects a procedural window.

- **Rectangle lights: `RectLight`.** A one-sided panel (`width` × `height` in the entity's local XY,
  lighting along its forward axis, local −Z): diffuse is the exact cosine-weighted solid angle of the
  rectangle (Lambert's polygon formula), specular the representative point on it, renormalised
  across each extent. `intensity` is on-axis, so a small panel face on lights like a `PointLight`.
  Light type 3 in `LightData` (orientation quaternion in `direction`, half extents in `params.zw`);
  `compute_rect_lighting` in `common.wgsl`, both draw paths, and the volumetric pass keeps the air
  behind it dark. Saved in scenes, editable in the inspector, addable in the studio. Not a shadow
  caster. The `area_lights` demo now shows the sphere, tube and rectangle lights over a glossy floor.

- **Tube lights: `PointLight::source_length` (`with_source_length`).** A point light whose source is
  a capsule along the light's local +X: specular takes the representative point on the segment
  nearest the reflected ray (Karis 2013), the lobe is renormalised for the line as well as for the
  sphere, so a long fluorescent tube shows a long highlight without adding energy. Combines with
  `source_radius` (the capsule's radius). 0 (the default) is the previous light. The axis rides in
  `LightData::direction` and the length in `params.w`; `compute_tube_lighting` in `common.wgsl`, both
  draw paths.

- **Sphere lights: `PointLight::source_radius`, `SpotLight::source_radius` (`with_source_radius`).**
  A light's source as a sphere: specular uses the representative point on it nearest the reflected
  ray, renormalised so a bigger source spreads its energy rather than adding to it — a glossy
  surface shows a highlight the size of the bulb's reflection. 0 (the default) is the previous point
  source. Carried in `LightData::params.z` (previously unused); `compute_sphere_lighting` in
  `common.wgsl`, both draw paths.

- **FXAA quality presets: `FxaaQuality` and `FxaaState::set_quality`.** Low / Medium / High / Ultra,
  after the classic FXAA presets (contrast threshold, minimum threshold, search steps). `High` is
  the default and exactly the previous pass. `FxaaParams::_padding` is now `quality`.

- **Contact shadows: `DirectionalLight::contact_shadow_length` / `with_contact_shadows`.** A
  16-step screen-space march toward the sun in the deferred lighting pass, for what the shadow map
  misses: anything shorter than its depth bias or finer than its texels — a step, the line where an
  object meets the floor. 0 (the default) is off. Carried in `SceneUniforms::cluster_depth.z`
  (previously unused); `SunFrame` gained `contact_shadow_length`.

- **Soft sun shadows: `DirectionalLight::angular_radius` / `with_angular_radius`.** The sun's
  angular radius (radians) sets a physical PCSS penumbra, `2·d·tan θ` — sharp at contact, soft
  with distance from the caster. 0 (the default) keeps the previous near-crisp edge: the deferred
  path's PCSS had a constant light size and a filter clamped to about one texel, so it never
  softened anything. Carried in `SceneUniforms::camera_forward.w` (previously unused).
  `SunFrame` gained `angular_radius`: a struct literal needs it, or `..Default::default()`.

- **The studio captures reflection probes.** It calls the now-public
  `gizmo::systems::render::capture_reflection_probes` before its passes, so the editor viewport
  reflects what the game will. The game path's draw list skips `EditorOnly` entities, so editor
  furniture never reaches a probe (and never reached a game frame anyway).

- **Transparent surfaces reflect reflection probes (native).** The forward pass's specular
  environment was a hemispheric stand-in; it now reads the probes through `probe_environment`,
  moved into a shared shader module (`gizmo::reflection_probes`). The lit forward pipelines gained
  group 5 (the irradiance volume's group), bound once per forward pass in both the game path and the
  studio. Without probes the stand-in is returned unchanged.

- **Several reflection probes, blended: `ReflectionProbe::blend_distance`.** Up to four probes in
  one cube array. Boxed probes apply inside their boxes, fading in from the walls and normalised
  where they overlap; the first unboxed probe (else the sky) fills the rest. Probe parameters edited
  after a capture now reach the lighting pass without a recapture (they did not). A single boxed
  probe no longer reflects itself outside its box — the sky does. `IrradianceState::set_reflection`
  takes a `ReflectionSet`; binding 2 is a `texture_cube_array`.

- **The clear-coat layer reflects the reflection probe.** It read the procedural sky even with a
  probe in the scene; now it reads the probe at its own 0.08 roughness, through the parallax lookup.
  The probe's prefiltered rough mips are tested (saturation and the gap between opposite walls fall
  at the roughest mip).

- **Box-projected parallax for reflection probes: `ReflectionProbe::influence` /
  `with_influence(half_extents)`.** Reflected rays are traced to the probe's box (the room) before
  the cube lookup, so a mirror away from the probe reflects the part of the room it faces. Measured:
  a mirror 3 m off the probe goes from 1 008 magenta / 528 white (direction only) to 1 536 white.
  `IrradianceState::set_reflection` now takes a `ReflectionSource`.

- **Reflection probes: `ReflectionProbe` component, `gizmo_renderer::reflection`.** The scene
  captured from the probe's position into a prefiltered HDR cube (six 90° renders, GGX mips by
  roughness), read by the deferred lighting pass's specular term in place of the procedural sky —
  a chrome sphere in a red room now reflects red. Captured on the first frame and after
  `request_capture()`. No probe: the sky, bit-identical. One probe at a time, by direction only (no
  parallax). `IrradianceState` gained bindings 2–4 and `set_reflection`. Demo: `pbr` with
  `GIZMO_PROBE=1`.

- **Camera motion blur: `PostProcess::motion_blur`.** A shutter fraction on the camera's grade
  (0 = off, the default; 0.5 = 180°). The composite pass reprojects each pixel with the last frame's
  unjittered view-projection (`Renderer::previous_view_proj`, per split-screen view) and averages
  the HDR frame along the motion, capped at 5 % of the screen. A still camera and the first frame
  are bit-identical to the shutter closed. Editor: a "Motion Blur" slider in the camera's grade.
  `PostProcessUniforms` grew to 160 bytes (`motion_blur`, `motion_blur_max`, `reproject`). Only the
  camera's motion: there is no per-object velocity.

- **Auto exposure: `renderer.auto_exposure` (`gizmo_renderer::auto_exposure`).** The luminance
  sensor (`LuminanceReduce`) fed back into tone mapping entirely on the GPU: one compute step moves
  the exposure toward `key / mean` in stops at a frame-rate-independent rate
  (`AutoExposureSettings { key, adapt_to_bright, adapt_to_dark, min_exposure, max_exposure }`), and
  the composite pass reads it as group 3 on top of `Camera::exposure`. Off by default — then group 3
  holds 1.0 and frames are bit-identical. Native only. Per split-screen view. Measured in
  `auto_exposure` (`GIZMO_AE_SENSOR=engine`): bright/dark station gap 131.0 → 35.8 at no measurable
  frame cost.

- **CAS sharpening: `renderer.cas` (`gizmo_renderer::cas::CasState`).** Contrast-adaptive
  sharpening as the last post pass — composite → FXAA → CAS → output — to give back the detail TAA
  and FXAA blur away. Off by default (it changes every frame's look); `cas.enabled = true` and
  `set_sharpness(queue, 0..=1)`. Flat areas are left exactly as they are; split-screen views and
  the HUD are handled like FXAA's. Measured in `anti_aliasing`: TAA + CAS mean gradient 2.60 →
  3.24, hardest step 145 → 184 (key 5 / `GIZMO_AA=taa_cas`). MSAA stays out by decision (deferred
  G-buffer; see CAPABILITY_GAPS §A6).

- **Split screen: `Camera::viewport` and `Camera::order`, drawn by `render_viewports`.** Give a
  camera a rectangle of the target (`Viewport::left_half()`, `quarter(i)`, `new(x, y, w, h)` — fractions,
  top-left origin) and `default_render_pass` draws every such camera into its own rectangle, lower
  `order` first (a picture-in-picture map is a small viewport with a higher order), then the HUD
  once over the whole target. Each view is projected at its rectangle's aspect, submitted on its own
  (the draw path's `queue.write_buffer` uploads — sprite vertices, culled instances, the grade —
  order against submission, and in one encoder the first view drew the last view's instance list:
  0 of its pixels right), and keeps its own TAA/SSGI history (`Renderer::with_view_history`).
  Animation, GPU physics, fluid and particles still advance once per frame. New renderer methods:
  `run_post_processing_in` (region + optional HUD) and `record_overlay`. Six mutations, each red.
  Demo: `split_screen`. `Camera` gained two fields (`#[serde(default)]`; struct literals need
  `viewport: None, order: 0`).

- **The character controller honours one-way platforms; `CharacterController::drop_through_timer`.**
  The kinematic `update_character` ignored `Collider::one_way`, so a character's upward sweep hit
  the underside and it could not jump through. It now applies the pipeline's own rule
  (`Collider::one_way_keeps`) at every ray, with two additions a capsule needs: only the **feet**
  are caught (the body's middle and head rays pass through, or a character sinking through would
  snag on its chest), and only feet that are on or above the top — a jump that leaves the feet
  inside the slab falls back instead of being snapped up by the ground ray, which starts `radius`
  above the feet. `drop_through_timer` (written by the game, counted down, `#[serde(skip)]`) makes
  one-way colliders ignored while positive: down + jump. Tests compare each run with a solid
  platform; four mutations of the rule (solid, no depth check, no drop, no feet-only) each break
  at least one. A side-on character on this controller stays at `z == 0.0`. Demo: `platformer`'s
  blue ledge, S + SPACE to drop.

- **One-way platforms: `Collider::one_way(up)` (field `Collider::one_way: Option<Vec3>`).** A
  collider that stops only bodies arriving from its `up` side (collider-local, turns with the
  body): jump up through it from below, land on it from above. The pipeline drops a pair's
  contacts unless the normal points out of the platform within 60° of `up` and the other body is
  not rising through it faster than 0.1 m/s — the rule is `Collider::one_way_keeps`, in one place.
  Pairs without a one-way collider pay nothing; `headless_stress_test`'s hash is unchanged.
  `#[serde(default)]`, so scenes saved before load as solid. Tests pair each run with a solid
  platform (which sends the ball back down) and an upside-down one (which blocks from below).

- **2D shapes: `Collider::polygon_2d`, `polyline_2d`, `circle_2d`, `rect_2d`.** Built from points
  in the XY plane and extruded `depth` along Z, for bodies locked with `RigidBody::planar_xy`: a
  convex polygon becomes a convex-hull prism (ramps, wedges, triangles), a polyline a static strip
  of quads (hilly ground). No solver change — the engine's 2D physics stays 3D shapes on the
  plane, so 2D and 3D bodies collide with each other. Tested end to end: a triangle settles on its
  base, a ball rolls down a polyline slope and stays on it (a strip with no area lets it fall
  through, which the test catches).

- **Text wrapping: `Text::max_width` / `with_max_width`, and `FontLibrary::layout_wrapped`.**
  Lines wider than the width break at spaces (the space is dropped); a word too wide on its own
  is cut between characters rather than overflowing. In glyph pixels in both spaces. `None`, the
  default, keeps the old behaviour of breaking only at `\n`. A `gizmo-ui` `Node` does not wrap its
  text on its own — `gizmo-ui` never measures text, so its box can be narrower than any word.
  Lines of a multi-line text can be aligned: `Text::align` / `with_align(TextAlign::Center)`
  (`Left` default, `Right`), backed by `TextLayout::align` and `TextLayout::line_widths`; the
  `text` demo shows a wrapped, centred paragraph.

- **`RigidBody::planar_xy()` — the 2-D setup in one call.** Locks Z translation and X/Y rotation,
  the three lines `kule_yikici` and `hill_climb` (twice) wrote by hand; all three use it now.

- **`gizmo::physics::tilemap_collider(&map, depth, solid)` — collision for a tile level.** The
  tiles `solid` accepts become one static compound collider in the map's own frame, with cells
  merged into as few boxes as the grid allows (row runs, then equal runs in consecutive rows): a
  flat ground of any size is one box, which also leaves no seams for a sliding body to catch on.
  Tested end to end — a ball dropped on a tile floor rests on its top edge.

- **`Tilemap` — a grid of tiles from one tileset image, as one component.** Built empty
  (`new`), from text rows with a legend (`from_rows`), or edited with `set` / `fill`; `get`,
  `tiles`, `cell_at` (the cell under a local point) and `cell_center`. Tile `(0, 0)` is the
  top-left and rows go down, from the entity's origin. Drawn through `record_sprites` on both
  draw paths, so a map sorts on its `layer` with world sprites and costs one draw call — a GPU
  test holds it pixel-identical to the same tiles drawn as sprites. `sprite_2d`'s ground is one
  `Tilemap` instead of 240 sprite entities. Only the cells the camera can see are queued
  (`visible_cells`, conservative — a view past the horizon gets the whole map): a 2000 × 2000 map
  queues 100 tiles in the GPU test and draws the same picture as a 40 × 40 patch. Collision:
  `tilemap_collider`.

- **`SpriteAnimation` — sprite-sheet animation as a component.** Plays cells `first..=last` of a
  `columns × rows` sheet at `fps`, looping or once (`once()`, `finished()`), forwards or
  backwards, pausable (`playing`). `set_run(first, last)` switches runs and restarts only on a
  change, so a game can pick the run from its state every frame. Both draw paths advance it with
  the clock's delta (`sprite_animation_system`, beside the skeletal animation drivers), so
  `set_time_scale` and pause apply; `render_parity` holds both paths to it. `sprite_2d`'s player
  uses it in place of its hand-computed frame.

- **`gizmo::physics::sprite_collider` / `sprite_collider_scaled` — a box collider derived from a
  world-space `Sprite`.** Size and anchor both carry over: a centred sprite gets a
  `Collider::box_collider`, any other anchor an `offset_box` placed where the renderer puts the
  quad. The scaled variant applies `Transform::scale`, which the renderer applies to a sprite and
  the physics does not apply to a collider. `None` for a screen sprite or one with no area.
  (Requires the `render` and `physics` features.)

- **`gizmo::simple::FixedPhysics` and `gizmo::simple::sync_transforms` — fixed-step physics for a
  game that owns its camera.** `simple_scene_update` steps physics but also flies the camera with
  WASD and the mouse, and slices the frame delta into ≤16 ms pieces, so how many steps run depends
  on the frame rate. A game that needed neither wrote its own accumulator (`kule_yikici` did).
  `FixedPhysics::new(hz)` steps at exactly `1/hz`, runs at most 8 steps per call (adjustable with
  `with_max_steps`) and drops the debt past that, and exposes `alpha()` and `reset()`.
  `sync_transforms` is the transform sync + propagate that `simple_scene_update` ends with. The
  game owns the driver, so it does not share the windowed runtime's `PhysicsTime`.

- **`ProjectionMode::OrthographicFit { width, height }` — an orthographic camera that fits the
  window.** Plain `Orthographic { height }` fixes only the vertical extent, so a portrait window
  cut the sides off a 2D playfield (measured in `kule_yikici`: 948×1028 showed ±7.4 units, the
  catapult at x = −11 was off screen). The new mode shows at least `width × height` in any window
  shape. Also `ProjectionMode::view_extent(aspect)` (what an orthographic view actually shows) and
  `ProjectionMode::is_orthographic()`. **Breaking for exhaustive matches** on `ProjectionMode`
  inside this workspace only — the enum is `#[non_exhaustive]`, so downstream code already has a
  wildcard arm.

- **Five vehicle setup knobs that were fixed constants.** Real car data gives each of these
  separately, and a downstream racing project had to approximate all five:
  `Wheel::suspension_rebound_damping` (was always 2.5 × bump; data can give rebound ≈ bump),
  `VehicleTuning::brake_bias_front` / `brake_bias_rear` (was 60 / 40),
  `VehicleTuning::anti_roll_front` / `anti_roll_rear` (one rate served both axles — the main
  under/oversteer balance knob), `Wheel::sliding_grip_floor` (the 0.35 a sliding tyre keeps of
  its other axis — now also `pacejka_combined_with_floor` and `DEFAULT_SLIDING_GRIP_FLOOR`), and
  `AeroPackage::downforce_speed_exponent` (downforce grew only with v²; `1.0` makes it linear).
  **Every default is the old behaviour bit for bit**, measured by an A/B probe — a car driven
  3600 steps through throttle, braking, steering and speed, its state hashed per step — run on
  the old and new code: `0f0f276ca1978a09` both times. The probe is sensitive: a 1e-7 change
  to any of three knobs changes the hash. That is why the brake bias is two fields and not
  `bias` and `1 − bias`: in `f32`, `1.0 − 0.6` is `0.39999998`, which alone changed the hash.
  New fields on `Wheel`, `VehicleTuning` and `AeroPackage`: a struct literal without
  `..Default::default()` needs them added.

- **`kule_yikici` — a small, complete game on the engine.** A catapult, a tower of crates on real
  rigid bodies kept to the XY plane by axis locks, five shots, win/lose, restart, a ballistic aim
  line and a HUD (sprites and text over the tone-mapped image). Every image is generated in code.
  What it had to build for itself — no 2D collision shapes, sprite and collider sized separately,
  no level-reset helper, no fixed-step driver for a game that owns its camera, and an orthographic
  camera that does not fit a portrait window — is in its header and `docs/CAPABILITY_GAPS.md` §C9.

- **A game's own components in the editor, end to end.** One call —
  `gizmo::register_editor_component::<T>(&mut world, "Name")`, i.e.
  `ComponentRegistry::register_editable` on the world's registry, for any
  `Component + Serialize + DeserializeOwned + Default` — and the in-engine editor shows the
  component with a section of its own, edits its fields, adds it from ➕ (as `T::default()`),
  removes it with 🗑, and writes it when it saves. Every link of that chain was broken before:
  - **Nothing applied the edits in the in-engine editor.** It drew the generic section, accepted
    the drag, queued `pending_json_updates` — and only `gizmo-studio` ever drained that queue.
    `gizmo_editor::component_requests` is now the drain both hosts share
    (`gizmo_app::editor_runtime::process_component_requests` runs it every frame).
  - **Integer fields could not be edited.** `draw_json_value` asked `as_f64` first, which every
    number answers, so a `u32` was shown as a float drag and written back as `4.0` — which
    `serde_json` refuses for an integer. Integers are now drawn and written as integers (unsigned
    stays non-negative); `null` (`Option::None`) is shown as absent instead of "unsupported type".
  - **➕ and 🗑 only knew names spelled out in a `match`.** The registry now carries a
    default-insert and a remove for editable types (`insert_default_fn` / `remove_fn`, kept in a
    private map so `TypeRegistration` — a public literal — gains no field), and both hosts fall
    back to them.
  - **An editor save left the component out of the file.** Every save, load, play snapshot and
    duplicate built `full_scene_registry()` from nothing. They now use
    `gizmo::scene_registry_for(world)` (= `scene_registry_for_world` in `gizmo-app`), the engine's
    registry plus the world's serializable registrations via the new
    `ComponentRegistry::extend_serializable_from` — which only adds, never overrides a built-in,
    and skips menu-only registrations (`gizmo-studio` lists `Material` that way, and each would
    have made every save warn once per entity). Play → Stop now restores game components too.
  `gizmo-studio` itself is a separate binary and cannot see a game's types; this is for the
  editor embedded in a game (`editor` feature) and for a studio build that registers them.

- **Sprites — the engine's first 2D path (`docs/CAPABILITY_GAPS.md` §A1).** A `Sprite` component
  (`gizmo_renderer::components`) draws a textured quad either in the entity's local XY plane,
  placed by its `GlobalTransform` (`SpriteSpace::World` — so it moves, rotates and scales with the
  entity, and a 2D game is an orthographic camera looking down `-Z`), or in window pixels
  (`SpriteSpace::Screen`, a HUD). Tint, sub-rectangle (`uv_rect`, `with_frame`/`set_frame` for a
  regular sprite sheet), `flip_x`/`flip_y`, a fractional anchor and an `i32` `layer`.
  Images load through `Renderer::load_sprite_image` / `_file` / `_rgba`, each with a
  `SpriteFilter` (`Linear`, or `Nearest` for pixel art); the decoder's error type does not leak.
  Order is layer, then — for world sprites — back to front by clip-space depth (a key that works
  under orthographic projection, where clip `w` is constant), then entity order; consecutive
  sprites sharing an image are one draw call. World sprites test the scene depth and write none.
  `IsHidden` hides a sprite exactly as it hides a mesh, ancestors included.
  Both hosts draw them through one shared pass, `gizmo::systems::render::record_sprites`, just
  before `record_text`. Corners are projected on the CPU with the **unjittered** view-projection,
  because the pass runs after TAA and the jittered matrix would make pixel art shimmer.
  Guarded by `sprites_reach_the_frame_in_place_in_order_and_behind_walls` (GPU): it fails with
  **890** px when the depth key is reversed and **784** px when the world pipeline's depth test is
  `Always`. Demo: `sprite_2d` — every image procedurally generated, nothing added to the repo.
  Not yet: mipmaps, atlas packing, 2D physics, a tilemap and a sprite-animation component.

- **The HUD is drawn after tone mapping.** Screen-space sprites, screen text and `gizmo-ui`
  backgrounds are now drawn onto the final image by a post-tonemap overlay
  (`gizmo-renderer`'s private `overlay` module), from inside `Renderer::run_post_processing` — the
  one function every output of both hosts goes through, FXAA path included — in the order
  backgrounds, sprites, glyphs. They used to go into the HDR target with the world, so a HUD was
  exposed, bloomed, tone-mapped and vignetted: an opaque `(0, 1, 0)` screen sprite read back as
  `(0, 219, 0)`, and now reads `(0, 255, 0)`. World sprites and labels stay in the HDR pass.
  Overlay pipelines are built per output format on first use; on a non-sRGB target the shader
  encodes to sRGB itself (linear 0.5 reads 188 on both `Rgba8UnormSrgb` and `Rgba8Unorm`, 128
  with the encode bypassed). `TextRenderer::begin_frame` now also forgets the upload counts and
  `record_text`'s early return calls it — otherwise the overlay, which draws from those counts,
  repainted a removed HUD forever (468 px measured). `TextRenderer::render` and
  `SpriteRenderer::render` draw world-space content only.
  The previous entry blamed tone mapping for the `sprite_2d` energy bar coming out mint; most of
  that was the colour: `Sprite::color` is linear, and `(0.35, 0.9, 0.45)` is sRGB
  `(160, 243, 179)`. The demo now uses saturated colours and says why.

- **`CommandQueue::clear`** — drops every pending command without running any, returning how many
  went. The teardown counterpart to `apply`, and what `World::clear_entities` now calls (see
  *Fixed*). A caller that wants the pending work to happen should `apply_commands()` *before* a
  clear, while the entities those commands name still exist.

- **One guarded hierarchy walk: `HierarchyExt::descendants_inclusive`.** Returns every entity
  reachable from a root down the `Children` links — the root first, then breadth-first —
  **cycle-safe and duplicate-free**. It exists because the same child-list descent was being
  hand-written at every call site that needed it, and a sweep found **six** unguarded copies (see
  *Fixed*, below — a seventh site is in that entry too, but it is a *mirror* into another
  library's tree rather than a copy of this descent, and only three of the six could be replaced
  by a shared walker at all). Having one guarded walker is still the point: the next person to
  need this should not have to remember the guard. Ids are returned rather than `Entity` handles
  and are not checked for liveness — a `Children` list may name a despawned id, so resolve with
  `World::entity` and expect `None`.

- **Untargeted events: `World::observe_global` / `trigger_global`.**
  The third door. `add_observer` is keyed to a component type and `observe` to a target entity; an
  event that belongs to neither — "the level loaded", "the round ended", "the save finished" — had
  no synchronous route at all and had to be an `Events<T>` queue read by a system a frame later.

  Dispatch is synchronous and in registration order: the listeners run inside the `trigger_global`
  call and can see and change the world before it returns. `Events<T>` is still the right choice
  when the reaction wants to be a *scheduled system* — batched, parallelisable, able to declare its
  access — and the documentation says which to reach for rather than leaving it implied.

  **The event needs no `Clone`.** The entity path requires it because a bubbling walk hands the
  same value to several entities; there is no walk here, so the listener takes the event by
  reference and an event carrying a `String` or a `Vec` costs nothing to publish.

  Re-entrancy follows the rule the rest of the module already had: the listener list is owned by
  the dispatch for its duration, so republishing the **same** type from inside terminates while a
  **different** type runs nested. A listener registered from inside a dispatch is appended after
  the ones already there and runs from the next publish — the merge-back is load-bearing and a
  mutation that overwrote instead of appending is caught by
  `a_listener_registered_during_a_dispatch_is_kept`.

  `observers` demonstrates it with two **independent** subscribers that do not know about each
  other, and a publisher that knows about neither: one records the result for the HUD, one logs.
  The cascade announces `ChainSettled` when it stops — measured at "26 mines · 8 rings" through the
  queue and "26 mines · 1 ring" when the listener drives it. Changing the mode changes when the
  announcement happens, not the announcement.

  Still open in that area: an observer cannot be gated by a run condition.

- **A listener can end a propagating walk: `World::stop_propagation()`.**
  Call it from inside a `World::observe` listener and the walk stops once that listener returns,
  instead of continuing up the `Parent` chain.

  **The gap had already diagnosed itself, and the fix is that diagnosis taken literally.** The
  entry read: *"what is missing is not access but a return channel — `On` arrives by value and the
  listener returns `()`, so the dispatch loop has nowhere to read an answer."* Handing the listener
  a `&mut World` earlier the same day **is** that channel. `stop_propagation` sets a flag on the
  world and `trigger` reads it after each listener returns, so the listener's return type never had
  to change — which also means the `observe` signature changed once this week rather than twice.

  Measured on the same five-link chain the gap entry was written from. Bubbling from the leaf
  visits `[4, 3, 2, 1, 0]`, and with a listener at link 2 *trying* to veto, **2** more links are
  still visited — that was the number in the entry. With the veto able to land, the same event
  visits `[4, 3, 2]` and **0** more. `observer_propagation` keeps both on `V`, because the size of
  a closed gap is only visible as a comparison.

  Two decisions the tests pin rather than leave to reading:

  - the vetoing entity's **other listeners still run** — the flag is read once that entity is
    finished, not between its listeners;
  - `trigger` **saves and restores** the flag around its own walk, so a nested dispatch that
    cancels itself cannot truncate the walk that called it. Both halves of that save/restore are
    load-bearing, and a different test catches each: dropping the reset-on-entry breaks
    `stopping_outside_a_dispatch_is_inert`, dropping the restore-on-exit breaks
    `a_nested_walk_does_not_cancel_the_outer_one`.

  Still missing from that row, and it stays open: the walk's *path* is always the `Parent` chain.

- **An entity-event listener is handed the world, so a chain reaction resolves in one frame.**
  `World::observe`'s listener now takes `(&mut World, On<E>)` — the same world the dispatch is
  running inside — so it can spawn, despawn, write components and call `trigger` again. Until now
  it received the `On` and nothing else, and anything a notification wanted to *do* had to be
  written to captured state and picked up by a later system.

  **The gap this closes was half wrong when it was written.** It read "callbacks get no world",
  naming observers *and* hooks. The hook types have taken a `&mut World` all along — `AddHook`,
  `SetHook`, `RemoveHook`, `DespawnHook`, and `ReplaceHook` was born with one earlier the same
  day — and a lifecycle observer's `On` is built by one of them, so `register_on_add` and its
  siblings were always the layer underneath. `observe` was the one route with nothing under it.

  **Measured, both ways, on the same chain.** `observers` drops a mine field of 70 and detonates
  the centre. Through the captured queue it reaches 26 mines in **8 frames**, one ring per frame.
  With the listener driving it — `trigger` called from inside the dispatch — it reaches **the same
  26 mines with the same 26 bubbled events in 1 frame**. The demo keeps both, on `M`, because
  watching the chain propagate is half of what it is for; it is now a choice rather than a
  constraint.

  One number moved while measuring, and it is worth keeping. The immediate path first reported
  **73** bubbled events instead of 26: it collects its neighbour list as a snapshot, and by the
  time it triggers the third entry the cascade from the first has already reached it. An event
  bubbles to its ancestors even when the target's own listener returns early, so the redundant
  triggers were not free. Re-checking before triggering brought it to 26 — the queued path had
  been doing that de-duplication in its drain all along.

  **Re-entrancy terminates structurally, not by a check.** The notified entity's listeners are
  owned by the dispatch for the length of the call, so an event sent back at its own target finds
  nothing to run, while another entity's listeners are live and run nested. That is the same shape
  component hooks already had. Five tests cover it, including that a listener survives the dispatch
  that detached it — the detachment lasts one dispatch, it is not a de-registration.

  `EntityListener<E>` names the boxed closure type, because three places have to spell it and a
  mismatch there means a `downcast_mut` that quietly returns `None`: a listener that is never
  called, with no error anywhere.

- **Lifecycle observers cover a component's whole life: `Insert`, `Replace`, `Remove`.**
  `World::add_observer` is generic over the marker, and the marker comes from the closure's own
  signature — `world.add_observer(|e: On<Remove, Hp>| …)` — so the ordinary call needs no turbofish
  and the phase is readable at the registration rather than inferred from the function name. Until
  now it built an `Insert` and nothing else; `Remove` and `Replace` were markers whose own
  documentation said "no dispatch path yet", and detachment was reachable only through a raw
  `RemoveHook`.

  The three **partition** a component's life. A write fires `Insert` **xor** `Replace` — never both,
  never neither — and a detachment fires `Remove`.

  `Replace` needed a fourth hook list rather than a reuse of `on_set`. `on_set` fires on the first
  write as well as every later one, and a hook is handed the entity and nothing else, so it cannot
  tell the two apart; subtracting the inserts means every caller keeping a per-entity ledger. The
  dispatcher already knows which branch it took — it is the only thing that does — so it says so
  once. `register_on_replace` is the raw hatch, and the sealed `Lifecycle` trait is what lets one
  closure's argument type pick a list.

  **Closing the gap turned an internal quirk into a broken promise, so that was fixed too.**
  `remove_bundle` fired `on_remove` for a bundle's sparse components and let its Table components
  out through `move_entity_to` in silence: the same component, removed two ways, answered
  differently. Nobody had to care while `Remove` was unreachable; `On<Remove, T>` makes it a
  contract. The bundle's Table members are now notified after the migration, which is where
  `remove_component` notifies for the same storage, and only for members the entity actually held.

  `removal_detection` had **measured** that silence and recorded it in its header as a fact — five
  removal paths, four hook calls. It now grows a `GIZMO_REMOVAL_SELFTEST=1` mode that prints the
  table instead of leaving it to be read off a HUD, and the re-measurement is five paths, **five**
  hook calls. `observers` drops its raw removal hook for the observer, counts all three phases, and
  ties `Replace` to something checkable: a mine is overwritten when it explodes, so the replace
  count must track the explosion count.

  Nine tests, and the two load-bearing ones were mutation-checked. Wiring `Remove` to the
  `on_add` list turned `remove_reaches_an_observer_by_both_routes` green **for the wrong reason** —
  the spawn filled the ledger and the end-state assertion could not tell that from a working remove
  path. It now asserts the ledger is empty before each detachment, and fails under that mutation.

- **Default query filters: an entity can be disabled instead of destroyed.**
  `World::default_query_filters_mut().add::<Disabled>()` and an entity carrying `Disabled` is
  invisible to every query a system **declares as a parameter**. The marker is yours — the engine
  imposes none, because the mechanism takes any `Table`-stored type, several if you like. The hatch
  is a query operand: `Query<(&Hp, With<Disabled>, IgnoreDefaultFilters)>` is how the system that
  re-enables entities is written, and it is in the signature rather than in a comment.

  **The gap's pricing was wrong twice, in opposite directions, and the second one nearly shipped.**

  It predicted "seven entry points in `query/`, each needing an opt-out". There are seventeen public
  `Query` methods and they all read one archetype list, so the filter is one predicate applied at
  construction: nothing per row, `has_row_filter` untouched — which is why chunk iteration still
  serves a filtered query — and `entity_count`/`len` become exact rather than over-counts. That
  shape is only available because a filter type must be `Table`-stored; `add` panics on a sparse
  one rather than filtering nothing silently.

  But it missed the only hard question — *which constructor* — and the first answer was **wrong**.
  Filtering `World::query_unchecked` looked like "the system path", because the parameter impl goes
  through it. It is also the engine's mutable-from-shared hatch: **98 call sites in 37 files, nine
  of them editor panels and seven of those the inspector** — egui draw functions taking `&World`,
  with no system near them. An adversarial review traced four consequences in the code before it shipped:

  - the **editor inspector went blank** for a disabled entity, and the scene-view manipulator with
    it, leaving no UI to re-enable it from;
  - **transform propagation froze whole subtrees**: its BFS enqueues children inside the `if let`
    that fetches the parent's `GlobalTransform`, so one disabled node stranded every *enabled*
    descendant in stale world space;
  - `sync_bodies` **destroyed** a disabled rigid body rather than pausing it — a body absent from
    the incoming list is removed, which permutes the component arrays, leaves joints dangling and
    invalidates earlier rollback snapshots. "Disabled" is supposed to mean the data is kept;
  - netcode rollback captured **unfiltered** and restored **filtered**, which is a desync.

  So the boundary is the parameter itself: `SystemParam for Query`'s fetch, one function. It covers
  the **44** demo files and **3** engine files that declare a `Query` parameter — game code writes
  systems, the engine reads the world directly.

  **Named limitation, because it is the price of that boundary.** Disabling hides an entity from
  *your systems' queries*. It does not pause the engine: physics still simulates it, transform
  propagation still updates it, audio still plays it, rollback still captures it. The four
  consequences above are why they must keep reading the whole world. The marker is a **view**, not
  a **state**.

  `nothing_but_a_system_parameter_is_filtered` asserts every constructor, `unsafe` ones included,
  and goes red if the rejected boundary comes back. Review also found `Or` silently swallowing the
  opt-out — it forwards `check_aliasing` and `has_row_filter` explicitly and had not been given the
  third, so `Or<IgnoreDefaultFilters, _>` compiled, read as an escape hatch and was filtered anyway.
  Fixed, with a test.

  `entity_disabling`'s old measurement — two systems, one remembering `Without<Disabled>` and one
  forgetting, diverging by 543 extra updates by frame 240 — is now an engine test asserting the
  divergence is zero over the same 240 frames, because the forgetful system can no longer be
  written. The demo's own replacement counter was rejected too and rewritten: it compared a
  filtered query against a filtered query, so it would have read zero even if the filter did
  nothing.

- **`Local<T>`: per-system state that the scheduler cannot see.** Until now a counter, a "did I
  already do this", a scratch buffer — anything a system wanted to keep between runs — had to be a
  world resource. That made it visible to every other system, and worse, visible to the *batcher*:
  two systems each keeping their own tally in a `ResMut<T>` declare a write of the same type, so
  they are kept apart and never run in parallel.

  `Local<T>` declares nothing at all. Measured: two systems holding a `Local<u32>` land in **1**
  batch, and the same two written with a resource land in **2** — the test asserts both halves,
  because the resource form is what the comparison is against.

  **It cost what `docs/CAPABILITY_GAPS.md` priced it at, and that is why it is its own commit.**
  `SystemParam::fetch` was a *static* function handed no per-system identity, so there was nowhere
  for a value to live and `Local` could not be added as another impl the way `Option<P>` was. The
  trait grew a `type State`, `fetch` a third argument, and the change reached nine impls, the
  `system_param!` macro, the 1..12 `IntoSystem` macros, the 1..6 `IntoCondition` macros and four
  call sites outside `gizmo-core`.

  Details worth knowing before meeting them:

  - **Per *built* system, not per function.** Registering the same function twice gives two
    independent values, which is also the answer to "why did my counter reset".
  - **Composites can hold one.** `system_param!`'s `State` is the tuple of its fields', so a
    `Local` inside a composite keeps its own value. Had that line been left as `()` the composite
    would have compiled and its counter would have reset every frame.
  - **Conditions can hold one.** A condition's state lives in the closure `into_condition` builds,
    so it is per condition — the same rule `distributive_run_if` already documents.
  - **`fetch_stateless`** is the provided method a caller with no state to hand over uses: `fetch`
    takes `&'w mut Self::State` because `Local` hands that reference to the system body, and a
    temporary `&mut ()` does not live long enough. It costs nothing — `()` is zero-sized.
  - One API consequence: a **`pub`** composite now needs its field types at least as public as
    itself, because `State` names them. Private composites — most of them — never meet it.

  `ecs_guide`'s header said "there is no per-system local state" and hand-rolled a `RoundLog`
  resource for its round counter. Both are gone.

- **The editor viewport ignored `alpha_cutoff` — the workspace's one standing red, closed.**
  `render_parity`'s capability inventory had reported `alpha_cutoff` as game-path-only since
  `05fcbff7` on 2026-08-22, and that assertion had been failing ever since: a gate left red is a
  gate that covers less every time it is ignored.

  It was a real defect, not a scanner artefact. The editor built its instances without the
  threshold and chose its bucket from `is_transparent` alone, so a pierced fence was solid in the
  viewport and pierced in the game. Measured before the fix at **11 654 of 16 384** pixels that
  every texel's alpha said should have been discarded. The test is binary by construction — a white
  texture and albedo alpha 0.3 against a 0.5 cutoff put *everything* under the threshold, so the
  correct frame is empty and "it faded a bit less" cannot pass.

  Two changes. The threshold now rides on the editor's instances (`ambient.w`, which `shader.wgsl`
  already discards on). And `draws_blended` is `pub`, shared by both loops, for the reason
  `gizmo-renderer::routing` exists: which bucket a material draws in is a *decision*, and a
  cut-out's contract is that discarding keeps the draw in the **opaque** pass — reading
  `is_transparent` alone moved it into the sorted blended bucket the moment its alpha fell under
  the threshold it was supposed to be tested against.

  `cargo test --workspace --no-fail-fast` is **244 binaries, 2 470 tests, 0 failures** — the first
  fully green run since the red appeared. The other half of that old sentence stays open and is not
  the same problem: deferred `alpha_cutoff` still needs the z-prepass to gain a fragment stage,
  while the editor draws forward, where the threshold only had to reach the instance.

- **Extrusion, and the sweep around an axis: one machine for fifteen shapes.** `3d_shapes` counted a
  26-mesh catalogue against the engine's six ready-made builders and found that **15 of the 20
  missing ones were one absent capability** — eight extrusions and seven ring extrusions. A prism, a
  pentagonal prism, a rounded slab and a torus of square section are not four features; they are
  four *outlines* handed to one machine.

  `AssetManager::create_extrusion(outline, depth)` takes a closed 2-D outline along Z (two caps, one
  quad per edge); `create_sweep(profile, radius, segments)` carries the same outline around the Y
  axis. Outlines are in `asset::primitives::extrude::outline` — `circle`, `ellipse`, `rectangle`,
  `stadium`, `star` — and `circle(r, 5)` *is* the pentagon, because a segment count is the only
  thing that separates them and two names for it would drift.

  **The caps are the half with a decision in them.** A triangle fan — the obvious answer, and what
  this demo's own hand-rolled pentagon used — is correct only for a *convex* outline; a star, an L
  or an arrow fanned from one vertex puts triangles outside the shape. `triangulate` is ear
  clipping, so a concave outline comes out right, and it is tested by **area** rather than triangle
  count: a wrong triangulation still returns `n − 2` triangles, and only the shoelace sum notices
  they do not cover the outline. A self-intersecting outline is refused rather than approximated,
  and the refusing loop is bounded — an ear clipper with no ear available otherwise spins forever
  inside a mesh builder.

  Winding is normalised rather than demanded: a clockwise outline extrudes to the same solid, which
  the existing `winding_tests` harness now checks for every generated shape in both directions —
  the extruder's winding is *derived* (from the outline, through the triangulation, reversed once
  for the back cap), so there are three places to get a sign wrong and each produces a solid that
  looks fine from one side.

  Not done: holes (an annulus is a sweep here, not an extrusion), bevels, smooth normals, and the
  five shapes that were never part of this gap — tetrahedron, conical frustum, icosphere, 3D
  segment, polyline. `3d_shapes` keeps hand-building the first two and says why.

- **Hiding is inherited — and the entry that asked for it had the facts backwards.**
  `docs/CAPABILITY_GAPS.md` listed a missing `Visibility` component whose consequence was *"hiding
  means `ShadowCasting::Only`, which still casts a shadow"*, and `infinite_grid` said the same and
  built its P key on it. Both were false, and already four days stale when the demo was written:
  `IsHidden` reached the game path on 2026-08-19 and it is a **full** hide — the entity is dropped
  before it becomes a draw item, so it is neither drawn nor cast. Measured now rather than argued:
  an `IsHidden` cube gives a frame **0 pixels** different from having no cube in the world at all,
  while `ShadowCasting::Only` leaves its shadow behind, which is what that variant is *for*.

  What was actually missing is the part nobody had written down. Both draw loops asked
  `hidden.contains(entity)` per entity, so hiding a parent left its children on screen — **1 886 of
  a parent/child pair's 2 946** pixels survived. `gizmo::systems::render::collect_hidden` walks
  `Children` down from every marked entity, both paths share it (two answers to "is this drawn" is
  what the 2026-08-19 fix was about), and it is `O(hidden subtree)`: a frame with nothing hidden
  visits nothing. A hierarchy cycle terminates rather than hanging the pass, which the unit tests
  build by hand because `add_child` should make one impossible.

  `infinite_grid`'s P key hides with `IsHidden` now, and its header says what the old one got
  wrong instead of quietly dropping it.

- **Run conditions combine: `or`, `and`, `not` — and the typed path could not be called at all.**
  Repeated `run_if` already ANDs (each call wraps the previous), but an OR had to be written inside
  one `run_if(|world| …)` closure, and that form is opaque: the scheduler cannot see what the
  predicate touches, so it marks the system **exclusive** and runs it alone in its batch. That was
  the cost, and it is now measured rather than asserted — a typed `or` and an unrelated system land
  in **1** batch; the same condition as a closure gives **2**.

  The combinators are typed, so the combined access reaches the scheduler from both sides: a system
  guarded by `or(reads Score, reads Paused)` is kept out of a batch with a writer of *either*. The
  union is declared even though `or` short-circuits — batches are decided before the step, so
  "might read" has to mean "reads".

  **Building them found that the typed path had never been usable.** `IntoCondition` was bounded
  only on `FnMut(P::Item<'_>) -> bool`, and `P::Item<'_>` is a projection the compiler cannot run
  backwards: given `|s: Res<Score>| …` it could not work out that `P1` is `Res<'static, Score>`, so
  every call was `E0283 type annotations needed` with no annotation a caller could write. The
  parallelisable half of run conditions existed, was documented as the way to keep a system out of
  its own batch, and could not be reached with anything taking a parameter. Fixed by the second
  `FnMut` bound that `IntoSystem` has carried all along.

  A consequence worth knowing before hitting it: a function that *returns* a condition must be
  written `-> impl IntoCondition<(Res<'static, T>,)>`, not `-> impl FnMut(Res<T>) -> bool`, or the
  second bound disappears behind the opaque type. Both `IntoCondition`'s docs and the
  `run_conditions` demo say so; the demo now drives two of its three cubes through the typed path
  and deliberately leaves the third on the closure form so the cost stays visible.

- **`gizmo-ui` draws.** The crate computed flexbox boxes and hover state from the day it was
  written and emitted nothing: its own docs said *"this crate emits no vertices and no draw calls"*
  and *"`BackgroundColor` is written and never read"*. Both were true, and both are now false.

  The bridge is small because both halves already spoke the same units — `Node` publishes absolute
  window-pixel rects and `TextSpace::Screen` takes them. `BackgroundColor` becomes a solid quad and
  a `Text` on the same entity is placed in that node's box, its own anchor choosing the corner
  (`Center` centres the label in the button; `TopLeft` puts it in the corner). The `Text`'s authored
  position is ignored when a `Node` is present, which is the only thing putting both on one entity
  can sensibly mean.

  **A background costs no pipeline.** The glyph atlas carries a fully-opaque patch, so a solid quad
  is a glyph quad whose coverage happens to be 1 — same shader, same instance buffer, one more
  instance. Quads are painted under every glyph in the frame whatever order they were queued in.

  Measured, with controls: a 60×30 node with a background changes **1 978/16 384** pixels (1 800 is
  the rectangle, the rest bloom) and the test fails with the quad not queued; a label authored at
  (100, 100) and given a box at (10, 10) changes **221** pixels inside the box and **0** outside —
  and drawing at the authored position instead is exactly what that 0 catches.

  The bridge lives in the facade and cannot live anywhere else: `gizmo-ui` sits *above* `gizmo-app`,
  so `gizmo-renderer` cannot see a `Node` and never will. `gizmo-studio` calls the same function, so
  the editor viewport shows the UI the game draws.

  `demo/src/bin/ui_layout.rs` is a taffy-laid row of buttons with labels and hover recolouring,
  including one deliberately narrow button whose label overflows — because there is still no
  clipping. Still missing and now named on the crate's own page rather than only in the repository:
  no z-order (the paint order is global — every background, then every glyph — so a label is always
  above every panel), no clipping, no click events, no keyboard focus, and one widget shape.
  `gizmo-ui`'s crates.io description said "there is no text rendering yet"; it does not now.

- **`Gizmos::depth_test` did nothing — the two pipelines it chose between were identical.**
  `GizmoRendererSystem` builds a "depth-tested" pipeline and a "no-depth" one, and both were
  `CompareFunction::Always`: the flag selected between two copies of the same state, and the
  field's own documentation — *"whether the lines are occluded by the scene"* — was false. Found
  while building the text overlay pass, which is the same pipeline pattern one slot earlier in the
  frame.

  **No picture the engine draws changes.** `Gizmos` derives `Default` (so `depth_test` is `false`)
  and `gizmo-studio` sets it to `false` explicitly, so the broken half was never selected — which
  is also why nothing caught it. What changes is that the flag now means what it says.
  `a_depth_tested_gizmo_is_hidden_by_geometry_and_an_untested_one_is_not` asserts the pair rather
  than either half: a line behind a wall leaks **115 of 16 384** pixels with the old state and
  **0** with the new, and the same line still draws when the flag is off — because "hidden" is
  something a line that is never drawn also satisfies.

- **The engine draws text.** Until now nothing drew a glyph except `egui`, a debug/editor UI layer
  — which is why every demo that wanted a label looked like a debug overlay. `docs/CAPABILITY_GAPS.md`
  §A2 was that gap and §E named it the first to close.

  `gizmo-renderer::text` is the font library, the glyph atlas and the layout; `Text` is the
  component, with two spaces (window pixels, or a camera-facing quad in the world), nine anchors, a
  colour and `\n`. **Both hosts draw it through one shared pass** — `gizmo-studio` calls
  `gizmo::systems::render::record_text` rather than keeping a copy, for the reason
  `render_parity.rs` exists.

  Measured, with the control in the same test: the same scene with and without text differs in
  **869/16 384** pixels and **0** with the pass not called; a string anchored at (8, 8) changes
  **869** pixels in the top-left quadrant and **0** in the bottom-right, which is the assertion a
  flipped y axis fails; a world label behind a wall changes **0**, and **1 589** with
  `CompareFunction::Always` in the world pipeline's place.

  **The tests build their own font.** The repository ships no typeface — that is a licensing
  decision belonging to the project rather than to a renderer — and a test reaching for a system
  font asserts against whichever face the machine happened to have. So `text::synthetic` emits a
  real TrueType file in memory: known em size, known ascent and descent, a known advance, one glyph
  that is a filled box of known side. Every number in the text tests is then a derivation rather
  than a measurement, and they run in CI and on wasm.

  **`ab_glyph`, sealed.** No `ab_glyph` type reaches a `pub` signature: the surface is `FontId`,
  `LineMetrics`, `PositionedGlyph`, `GlyphEntry` and `FontError`, all ours. It was **already in the
  lockfile** (winit's Wayland decorations pull it in through `sctk-adwaita`) and its whole tree is
  four small pure-Rust crates that build for wasm. Recorded in `docs/ENGINE.md` §4.

  **What it does not do, named rather than discovered later.** No default font — a `Text` whose face
  was never loaded draws nothing rather than substituting one. No wrapping, no shaping (so no Arabic
  joining, no Indic reordering, no ligatures), no bidi, no font fallback, no rich text; lines break
  on `\n` and nowhere else. Nothing is evicted from the atlas — it reports `is_full` instead of
  quietly dropping glyphs. Text is drawn into the HDR target before tone mapping, so it is exposed
  and bloomed with the frame: right for a world label, and the first thing to revisit when there is
  a post-tonemap pass to hang UI on. And **`gizmo-ui` is not connected to it yet** — `Node` still
  emits no vertices and `BackgroundColor` is still written and read by nothing. That is the next
  piece, and a smaller one: `Node` already publishes absolute window-pixel rects and
  `TextSpace::Screen` already takes them.

  `Renderer::load_font` / `load_font_file` are the entry points, and `demo/src/bin/text.rs` shows
  the lot: eight anchors on the window's edges, a size sweep, a multi-line note, and two world
  labels of which one sits behind a wall. It takes a face from `GIZMO_FONT`, falls back to a few
  system paths, and falls back again to the synthetic one — saying on screen that it did, because
  someone seeing boxes blames the engine first.

- **Water draws through its own pipeline, and the waves move.** `water.wgsl` was written,
  `water_pipeline` compiled every run and was stored in both `renderer.scene` and `Renderer`, and
  **no pass had ever bound any of it**. The cause was one line: `route(MaterialType::Water)`
  returned exactly `route(Pbr)`'s answer, so a water material was a PBR material with a different
  name. `Routing::is_water` now sends it forward — not for the reason the other forward types are
  forward, but because water displaces its own vertices, so a G-buffer filled by a shader that does
  not would record the flat plane's depth and normals while the visible surface is elsewhere. It
  joins `BatchKey` too: water and unlit both set `skips_deferred`, and an untextured plane of each
  shares the cached white-texture bind group, so without it the two keys are identical and one
  batch draws with the other's pipeline. Measured: the same surface as water and as PBR differs in
  **10 568 of 16 384** pixels, and **0** with the routing put back — the two frames were
  bit-identical.

  **Wiring it uncovered a second dead thing inside the first.** The shader read elapsed time from
  `scene.camera_pos.w`, a slot `frame_uniforms.rs` fills with a constant `1.0` and `gpu_types.rs`
  documents as unused; time is `cascade_params.z`. So even once bound, the ocean would have been
  frozen at t = 1.0 — entirely plausible in a still frame, and invisible to any single-frame test.
  Two seconds of elapsed time now move **3 730 of 16 384** pixels, and **0** with the old slot
  restored. `the_water_surface_is_drawn_by_its_own_pipeline_and_moves` renders three frames rather
  than two for exactly this reason.

  **And a third thing the route needed that the engine did not have.** The displacement is per
  vertex and `AssetManager::create_plane` is four of them, so a Gerstner ocean on it is a quad with
  four moving corners. `AssetManager::create_plane_subdivided(size, segments)` is the new
  primitive — same winding, same normals, same world-unit UVs, and checked against `plane_data`
  vertex for vertex at `segments = 1`. `water_demo` uses it (400 units / 200 segments) and is the
  first place the waves are visible at all.

  **The editor got a water pass in the same change, and had to.** `gizmo-studio` picks pipelines in
  its own passes and had no water branch — the same shape as its missing custom-material branch —
  but here silence was not neutral: water's instance flag went from `0.0` to `1.0`, and
  `shader.wgsl` reads `1.0` as *skip the lights and return the albedo*, so the editor's opaque pass
  would have turned the ocean into a flat rectangle, strictly worse than the accidental PBR it had
  before. Its test is worth a note because the obvious version passes while broken: brightness does
  not separate the two states (σ **8.99** broken against **11.62** wired — post-processing and the
  editor grid put variation in both), so the assertion is about *time*. Two seconds apart the
  editor's water differs by **4 633/16 384** pixels and **0** without the pass, with a water-free
  scene carried alongside as the control that the clock moves nothing by itself.

  Limits, recorded rather than discovered later: the water pipeline alpha-blends **and** writes
  depth (a surface, not a pane) and culls back faces, so `with_double_sided` is load-bearing here
  unlike on the transparent pipeline — hence `water_double_sided_pipeline`, for the swimmer looking
  up. Water casts no shadow, which also took it out of `classify_visibility_world`'s caster
  predicate — an off-screen water surface was being kept and uploaded for a shadow map nothing
  writes. It does not refract.

  This closes section F of `docs/CAPABILITY_GAPS.md` — the three subsystems that were finished,
  tested, exported and reachable by nothing. `deferred_rendering`'s measured table was re-run
  against the change: `Water`'s shadow column went from **−26.51** to **−2.50** against a fit noise
  floor of ±2.15 (it no longer casts), and it is now the only row that moves between runs, because
  it is the only one whose geometry depends on elapsed time.

- **The events had no reader: `PlayLoop` now rotates its event queues, and Lua can see the hits.**
  `Events<T>` is double-buffered — `send` writes this frame, `iter` reads the last one — and
  **something must call `update()` once a frame** or nothing ever becomes readable. Nothing did on
  this path: `App::add_event` is the windowed runtime's pump, and neither the editor's ▶ nor an
  exported game goes through it. So `physics_step_system` had been sending collision and trigger
  events into queues that never rotated, and the new hit events would have joined them.

  `PlayLoop` rotates them at the end of its frame, and **creates** the `HitEvent` queue if it is
  missing — the way `run_fixed_and_update` creates a `PhysicsTime` — because it is the only one of
  the three with no other way in. The other two are rotated only if the game asked for them, and
  that asymmetry is measured rather than tidy: `physics_step_system` clones every collision event
  into its queue, which in a 200-box tower is thousands of contact lists a frame.

  On top of that a script can now fight: `fighter.hits()` mirrors each resolved `HitEvent`
  (attacker, victim, the boxes involved, damage, hitstun, hitstop, move name), and
  `fighter.set_health(id, value)` is the write that spends one — an assignment, because clamping,
  armour, a block that halves it and death are the game's rules. The end-to-end test drives the
  whole chain from a `.lua` file: the script throws a jab, the engine resolves the hit and reports
  it, the script takes the health off — 92, not 100 (the event arrived) and not 84 (one hit per
  move).

- **A reliable network message was sent exactly once, ever.** renet splits the per-frame drive in
  two — the transport advances the netcode (encryption, keepalive, timeouts) and
  `RenetServer`/`RenetClient::update` advances each connection (reliability, RTT, the resend
  timer) — and Gizmo's wrappers called only the transport. Every connection's clock stayed at zero,
  so the reliable channel's resend gate never opened. Because that channel is *ordered*, the first
  dropped datagram stopped the peer receiving any further reliable message at all, the
  undeliverable backlog grew, and renet eventually disconnected it. Two lines, one per wrapper.

- **An object deleted during Play could fail to come back on Stop.** `SceneSnapshot::restore`
  walked the snapshot row by row and called `world.spawn()` for a dead id. The entity allocator's
  free list is FIFO, so that spawn could hand back the id of a row not yet processed; when its turn
  came, that row found its id alive — the entity another row had just been given — and wrote over
  it. Two rows, one entity, one object silently gone. Play-time entities make it easy to reach,
  since Stop despawns them first and their ids join the free list.

  Restore now assigns every row a target before writing anything: live ids are claimed, the rest
  get fresh entities, and the second pass never looks anything up by id. (A dead slot can still
  come back under a different id — that was already documented and is unchanged.)

- **Every rollback re-simulated one tick too many.** `RollbackManager::end_frame` labels a snapshot
  with the tick *just stepped*, so the state called "tick T" is the state at the **end** of T.
  `begin_frame` restored it and set `current_tick = T`, leaving the world one step ahead of what
  the counter claimed — and the host loop takes `latest_tick - current_tick` catch-up steps. A
  30-tick world rewound to tick 24 came back at 31 steps: a divergence that never reconverges, and
  that compounds per rollback, because the snapshots written during catch-up are labelled from the
  inflated counter.

  Fixed in `begin_frame` rather than at the call site, because the same public surface was lying to
  both callers — and the giveaway was that the crate's own integration test wrote the missing `+1`
  by hand (`ROLLBACK_AT - REWIND + 1`, under a comment explaining exactly why) while the engine's
  loop did not. That test now asks the manager where it is instead of recomputing it.

- **Every frame-time and FPS number the engine showed was a fragment of the real frame.**
  `Schedule::run` ended with `FrameProfiler::end_frame()`, which made the profiler's "frame" a
  schedule run. A windowed frame runs the fixed schedule `0..N` times and the update schedule once,
  and the loop then ends the frame itself — so `end_frame` fired **`fixed_steps + 2` times per
  rendered frame**, each one resetting the clock. The studio's status bar, the scene-view HUD and
  the Profiler panel all showed roughly half the true frame time and roughly double the true FPS,
  which is precisely the number a person opens that panel to trust.

  The same reset clears `active_scopes`, so a scope spanning a schedule run was destroyed by the
  run — including the windowed loop's own `"physics"` scope, which wrapped the whole fixed-step
  drain and could therefore never be recorded. Measured: the frame's scope list came back empty.

  The boundary now belongs to whoever owns the frame — the windowed loop (which already ended it),
  the headless loop and the reference server (which now do). A schedule run is not a frame.

- **A scene can hold a material at last: `MaterialDesc` + `gizmo_renderer::material_sync`.** The
  known exception written down that morning — "`Material` owns a live wgpu bind group, so a round
  trip loses PBR maps" — turned out to name its own fix. Everything else on `Material` is data,
  **including `texture_source`, the path the albedo came from**, so the description can be written
  and the bind group rebuilt.

  `MaterialDesc` is every field but the handle, and two passes keep the pair in step, both run by
  both draw paths: `resolve_material_descriptions` builds a material for every description that has
  none (a freshly loaded scene), and `sync_material_descriptions` writes a description back for
  every live material (so the next save has something to write). Materials built in code — the glTF
  loader, `Material::new`, the editor's ➕ menu — become saveable without knowing any of this.

  `From<&Material> for MaterialDesc` destructures the material **exhaustively, with no `..`**: a
  field added to `Material` and forgotten there is a compile error rather than a value that
  silently stops being saved. That is a guarantee no test could give, since building a `Material`
  needs a live GPU. A description with no texture, or one naming a file that will not load,
  resolves to the renderer's white 1×1 — a warning and a white surface rather than an entity that
  fails to appear. `Renderer::white_material_bind_group` is new and is what both cases use.

- **Three components the editor could add and a save silently threw away.** `ParticleEmitter`,
  `Terrain` and `BoneAttachment` were offered by the ➕ menu, added by the request handler, and
  absent from the scene registry — so authoring one, saving and reopening lost it without a word,
  because a component the registry does not know is simply not written. All three already derived
  `Serialize`/`Deserialize`; nothing structural had kept them out.

  Three lists have to agree here — what the menu offers, what "add" does, what a save keeps — and a
  new test in `gizmo-studio` holds the first to the third, reading the menu's list from the studio's
  own setup rather than restating it. `Material` is the one declared exception, with the reason: it
  owns a live wgpu bind group, so there is nothing to write, which is why a round trip still loses
  PBR maps.

- **GPU fluid rendered in a shipped game and showed nothing in the editor.** The last of the
  draw-path asymmetries, and the only one that was not a call away: the game path stepped the fluid
  and reconstructed its surface, the studio's pipeline did not mention `renderer.gpu_fluid` at all.
  Both halves are shared entry points now — `gizmo::systems::render::step_gpu_fluid` and
  `record_fluid_surface`, the shape `record_forward_decals` already had — and both paths call them.

  Two things fell out of doing it. **The viewport's fluid had to be gated on
  `Renderer::fluid_enabled`**, not on the fluid existing: `gpu_fluid` is `Some` on every native
  renderer (a 100 000-particle allocation made up front) and `fluid_enabled` is what says a scene
  wants it. Reading only `num_particles` would have run an SPH solve and a screen-space surface
  pass in every editor frame of every scene. And **`GpuFluidSystem::render_pass` was an empty
  public method** — "Fallback for compatibility, not used directly by SSFR loop" — called every
  frame from the game's forward pass, which therefore has never drawn a drop of fluid. The method
  and both call sites are gone; `render_ssfr` is what draws.

- **The two draw paths ran different per-frame drivers, in both directions.** Enumerating what each
  one actually calls turned up two more of the same defect on top of the state machine:

  - **`BoneAttachmentSystem` ran only in the editor.** It places every entity parented to a joint —
    a sword in a hand, a hat on a head — onto the pose animation just wrote. So an attachment
    followed the animation in the viewport and stayed frozen in an exported game: the mirror image
    of the state machine, which was engine-only. The game path runs it now, immediately after the
    animation systems and before anything reads a transform.
  - **`gpu_physics_submit_system` / `gpu_physics_readback_system` ran only in the game.** A scene
    using GPU rigid bodies simulated when shipped and sat still in the editor — whose own passes
    were already drawing from `renderer.gpu_physics`. Both are no-ops unless the renderer has a GPU
    physics world, so a scene that never asked for one pays nothing.

  The capability inventory could not see either: `BoneAttachment` and the GPU physics link are
  reached through systems and named in neither path's source, and the inventory only reports a
  capability known to exactly **one** path — one known to neither is silence. The driver list is
  guarded directly now, comments cut, both directions.

- **The editor's viewport ignored `time_scale` and kept animating through ⏸.** The engine's draw
  path advances skeletons by `Time::dt()` — the frame delta scaled and clamped — and skips at zero;
  the studio passed its own raw frame delta. Since `current_time += dt * speed` is the whole of the
  animation clock, that delta *is* the playback rate: a script calling `set_time_scale(0.5)` halved
  a game's animation and left the viewport running at full speed, and `0.0` stopped physics while
  the skeletons kept walking.

  `systems::simulation::animation_delta` decides it now, in three cases rather than the two the
  physics gate needed: ⏸ returns **zero**, because pausing stops `PlayLoop` while `Time` keeps
  advancing regardless — reading the clock alone would leave a paused editor animating. Editing
  returns the clock too, so previewing a clip still plays, now with the clamp that stops a two-second
  hitch jumping the pose two seconds forward.

- **An entity driven by an `AnimationStateMachine` animated in an exported game and stood still in
  the editor.** There are two skeletal drivers — `animation_update_system` for `AnimationPlayer`,
  `animation_state_machine_update_system` for the state machine — and the studio's pipeline called
  only the first. The engine's draw path called both.

  The guard that exists to catch exactly this could not see it. `render_parity.rs`'s capability
  inventory scans for `pub struct`/`pub enum` **defined in** `gizmo-renderer/src/components`, and
  the entire skeletal family is *re-exported* there from `gizmo-animation` — so none of those eight
  types was ever a subject. The scanner now collects cross-crate re-exports too, which is what
  turned this up. A second test pins the calls themselves with comments cut, because the inventory
  deliberately counts a name in a comment as knowing a type (a component touched only through a
  system cannot be named any other way) — and that is precisely how a comment about the drivers
  could satisfy it while one path never ran one.

- **A headless game had no clock at all.** The windowed runtime creates and updates a
  `gizmo_core::time::Time` every frame and feeds the fixed loop `Time::dt()` — the raw delta scaled
  by `time_scale` and clamped to `max_dt`. The headless runtime never touched `Time`: it handed the
  raw wall-clock delta straight to the fixed loop. So a server had no `Res<Time>` — no `dt()`, no
  `elapsed()`, no `frame()` — and `set_time_scale` did nothing there, `0.0` included, which is
  supposed to be pause. That is the same defect the headless loop's own comment records having
  fixed one layer up ("a plugin could not be written once and behave the same in both"), left
  standing one layer down.

  Both runtimes now take the simulated delta from one place, `gizmo_app::frame::advance_time`,
  which also removes the duplicate: the windowed loop had this inline. A source-shape guard pins
  both call sites, because neither loop can be driven from a test — one needs a window, the other
  never returns.

- **The editor ran the simulation at twice the rate an exported game does, and ⏸ did not stop
  anything falling.** `gizmo-studio`'s update hook called `cpu_physics_step_system(world, dt)`
  unconditionally, and then `handle_simulation` ran `PlayLoop::step`, which spends its own 60 Hz
  accumulator — so with ▶ down the world was stepped **twice per rendered frame**. Measured over
  60 frames (one second of wall clock) a falling body dropped **4.909 m** on the correct path and
  **19.530 m** on the ungated one: 3.98× the distance, which is 2× the simulated time through
  ½gt². The game you watched in the editor was not the game you shipped, which is exactly the
  drift `PlayLoop` was extracted to make impossible.

  The same line was why pausing did nothing to the world: ⏸ stops `PlayLoop` and nothing else ever
  sets `PhysicsWorld::is_paused`, so the "⏸ DURAKLATILDI" overlay was painted over bodies still
  falling under the editor's own step. Both are one gate:
  `systems::simulation::editor_owns_the_physics_step`, true only outside a play session. Edit mode
  is unchanged — stepping there is how a designer watches a stack settle.

  (The trace that led here claimed edit mode used a *raw variable* delta. It does not: the delta is
  clamped at `windowed/event.rs`, and `PhysicsWorld::step` never hands a variable step to the
  solver — it banks the delta and spends it in exact 1/240 s sub-steps. The premise was wrong and
  the defect underneath it was bigger.)

- **Lua can animate: the new `animation` API.** `ScriptCommand::PlayAnimation` and
  `SetAnimationSpeed` have had working handlers for as long as they have existed —
  `flush_commands` applies both to real `AnimationPlayer`s — and **nothing anywhere pushed
  either one**: no Lua binding, no Rust caller, no test. The engine could animate on request and
  had no way to be asked; `gizmo-animation`'s own docs name "the Lua `PlayAnimation` command"
  while explaining why a bad clip name is logged.

  `animation.play(id, name, blend?, loop?)` fills in the ordinary case (a 0.2 s cross-fade, and
  looping) so a locomotion clip is one call, and takes an override for a one-shot;
  `animation.set_speed(id, speed)` is the other half. The read side comes with it —
  `animation.state(id)` gives `clip`, `time`, `duration`, `speed`, `looping` and the full `clips`
  list, with `animation.clip` and `animation.is_playing` on top. The clip list matters: a name that
  matches nothing is ignored by the engine with a warning, so a script that wants to be sure checks
  rather than hopes.

- **Hits are resolved and reported: `gizmo_physics_dynamics::hit_detection_system` and
  `HitEvent`.** The fight subsystem's last missing link. A move now reaches its active window (the
  clock, above), and this turns that into "these two volumes overlap, once": it drives every
  fighter-owned `Hitbox::active` from the active window, tests the live boxes against everyone
  else's `Hurtbox`es, and reports each connection as a `HitEvent { attacker, attacker_hitbox,
  victim, victim_hurtbox, damage, hitstun, hitstop, move_name }` on `Events<HitEvent>`.

  **The engine reports; the game decides what a hit costs.** Nothing here subtracts health, applies
  hitstun or ends a round — death, armour, counter-hits and friendly fire are the rules of a
  particular fighting game. `damage` is the move's `FrameData::damage` scaled by the region's
  `Hurtbox::damage_multiplier`, so a head hurtbox with a 1.5× multiplier reports 12 for an 8-damage
  jab and the game spends it.

  Three decisions worth knowing, each with a test and a measured negative control:

  - **`Hitbox::move_name`** (new, `Option<String>`, `#[serde(default)]`) says which move a box
    belongs to; `None` means every move. The fight system deleted in `592bd6f` drove *every* box in
    a fighter's subtree from the active window, which is right for a fighter with one hitbox and
    wrong the moment a jab's fist and a kick's foot are both there.
  - **A move connects with a given victim once.** `FighterController::already_hit` records them,
    and the new `FighterController::start_move` (and the end of a move) clears it. Without it a
    three-frame active window reports the same hit three times, one per frame.
  - **Ownership is the nearest `FighterController` up the `Parent` chain**, so a fist parented to a
    fighter attacks for it, and a fighter's own hurtboxes are never targets.

  Geometry is the new `NarrowPhase::box_box_overlap` — the boolean sibling of `box_box`, sharing
  its axis construction and penetration test rather than copying them, stopping at the first
  separating axis and building no manifold. Verified to agree with `box_box` over 120
  configurations. Box poses are composed from the local `Transform` chain, so hit detection needs
  no transform-propagation pass; scale is ignored, which is what the engine's own debug gizmo
  already does.

- **A Lua script can read the fight, not just write to it: `fighter.state(id)`.** The read half of
  the fighting API used to be one boolean wide (`is_locked`), so a script could ask the engine to
  throw a jab and then had no way to learn what frame it was on, whether it was hitting, or when it
  ended — the whole point of frame data. `update_fighter_read_api` now mirrors the entire component
  each frame, in the same `table[entity_id] = value` shape as `entity._positions`:

  ```lua
  local s = fighter.state(id)   -- nil if that entity is not a fighter
  s.health, s.max_health, s.player_id, s.blocking, s.crouching
  s.hitstop, s.hitstun, s.locked
  s.move        -- nil when neutral: name, frame, total, startup, active, recovery,
                -- attacking (is_in_active_window), damage, hitstun_on_hit, hitstop_on_hit
  ```

  With helpers `fighter.is_locked`, `fighter.is_attacking` and `fighter.move_frame`. The one-bit
  `_is_locked` table is gone — it was a second source for a fact `_state` now carries. The snapshot
  is deliberately one frame old: the script pass mirrors at the top of the frame and the fight
  clock is spent at the bottom of it, which is the value a script reacting to its own move wants.

  `fighter.set_move` also gained two **optional** trailing arguments, `hitstun` and `hitstop` — the
  frames the move inflicts when it lands. Every Lua-authored move used to inherit `FrameData`'s
  20/5 silently, with no way to set or read them; a call that names neither still gets exactly
  that. And the input-buffer mirror now carries `just_released`, without which charge moves and
  negative-edge specials were invisible to Lua however carefully the buffer was filled.

- **The fight subsystem has a clock: `FighterController::tick`, `FrameData::total_frames` and
  `gizmo_physics_dynamics::fighter_frame_system`.** `tick` spends one fixed frame on one fighter —
  it counts `hitstop_frames`/`hitstun_frames` down and advances `current_move_frame`, ending the
  move after `total_frames()` of them, recovery included. `fighter_frame_system` is the thin ECS
  driver, registered by `GameplayPhysicsPlugin` alongside the vehicle and character controllers and
  called by `gizmo::systems::PlayLoop` on its fixed step — so the editor's ▶ and every exported
  game have it. Both are no-ops on a world with no fighters.

  It deliberately stops there: it does not fill `input_buffer` (which action names to record is the
  game's), does not pick a move from a button, and does not drive `Hitbox::active`. Those are the
  three halves of the system deleted in `592bd6f` that were game policy rather than engine clock.

- **A real low-pass for the underwater muffle, retunable while sounds are playing**
  (`gizmo_audio::filter::Muffle`, internal). rodio's `Player` hands out no way to reach the source
  it is playing, so a live filter has to be *inside* the source: `Muffle` wraps each decoder and
  reads an `Arc<AtomicU32>` cutoff per sample, making `set_underwater` one atomic store that every
  sound hears at once — including sounds started afterwards.

  Measured (Butterworth Q = 1/√2, 48 kHz, corner 800 Hz): 100 Hz passes within 1 dB, the corner is
  −3 dB, 6.4 kHz is below −30 dB, and bypass is sample-for-sample identical. The biquad keeps
  **per-channel** state, unlike rodio's own `BltFilter`, which runs an interleaved stream through
  one set of memories and so filters the right channel with the left channel's history.

  **Behaviour change:** the underwater muffle no longer slows playback to 0.85×. That was a stand-in
  for the filter rodio could not run live, and slowing playback is a pitch shift — it detuned music
  and dropped looped engine sounds a tone. Volume still drops to 0.4; the character now comes from
  the 700 Hz corner.

- **A mixer with buses: `gizmo_audio::Mixer`.** Named buses (`Mixer::MUSIC`, `SFX`, `UI`, `VOICE`,
  and any name a game invents — they are created on mention), a master gain, and mute flags that
  keep their gains so unmuting restores exactly what was there. A bus gain applies to sounds that
  are **already playing**, which is what makes it a settings-menu slider rather than a value read
  once at spawn. Reached through `AudioManager::mixer()` / `mixer_mut()`; a live sound is routed
  with `AudioManager::set_sink_bus`, and a scene-authored one through the new `AudioSource::bus`
  field (`#[serde(default)]`, so scenes saved before buses load onto `Mixer::DEFAULT_BUS`).

  Every volume and speed that reaches rodio is now **composed** in one place —
  `route.volume × bus × master × environment` — instead of being multiplied into the player by
  whoever ran last. See *Fixed* below for what that changed.

- **`Input::move_axis` / `blend_move_axis` — one movement read for the keyboard and the stick.**
  Returns `(x, y)` in the closed unit disc, `x` right and `y` forward, basis-free so the caller
  multiplies it by whatever right/forward vectors it already has. `MoveKeys::WASD` and
  `MoveKeys::ARROWS` are presets; `move_axis_with` takes any four key codes.

  It exists because **20 places** had written their own and they did not agree: five of them
  (`showcase`, `cpu_physics`, `ocean_scene`, `advanced_physics`, `demo/src/main.rs`) added a
  full-speed step per key from four independent `if`s, so **holding W and D moved 41 % faster than
  either key alone**, and nineteen of the twenty had no gamepad support at all. Three of those
  nineteen are engine code: **`SimpleApp`'s built-in fly camera, the studio's editor camera and
  `FpsLook` can now all be driven with a stick**, so a game built on any of them gets that without
  doing anything.

  `car_demo` deliberately still reads its keys directly, because a vehicle's throttle and steering
  are independent axes rather than a movement vector.

  `SimpleSceneState::fly_step` is new and public: the fly camera's per-frame step, lifted out of an
  `set_update` closure that could not be reached without a window. It is why the engine's own
  camera has tests at all.

- **`FpsLook::apply_stick_look` and `FpsLook::stick_sensitivity` — right-stick look for the
  engine's first-person controller.** `stick_sensitivity` is in **radians per second**, not the
  radians-per-pixel that `sensitivity` is, because the two inputs are different kinds of thing: a
  mouse reports how far it *has moved* (the frame's time is already in the number, so `apply_look`
  takes no `dt` and must not), a stick reports how far it *is held* (a rate, so `apply_stick_look`
  takes `dt` and must). Getting either backwards makes look speed frame-rate dependent.

  Keyboard behaviour is unchanged for the demos that were already correct — checked against their
  previous expression over all 81 key combinations, not asserted.

- **Input bindings as text, and a rebinding panel.** `ActionMap::to_named` / `apply_named` turn a
  map into `action → ["key:w", "pad:south", "axis:left_stick_x+0.5"]` and back, which is what the
  `NAMED_KEYS` / `NAMED_GAMEPAD_BUTTONS` tables were built for and what nothing could do until now.
  `apply_named` returns every entry it could not parse rather than dropping it — a typo'd binding
  is otherwise a control that silently does nothing.

  `InputBinding::captured_from(&Input)` is the "press a control now" read: press edges only, a
  default threshold for axes rather than however far the stick happened to be, and a fixed
  precedence so a key beats a resting stick. `ActionMap::add_binding` / `remove_binding_named` are
  the two edits a rebinding UI makes.

  `gizmo_editor::rebinding::ui_rebinding_panel` draws the rows. It takes the map and the input
  rather than owning them, so a game's settings screen can use the same panel.

- **A mesh inside a moved `.glb` is found again.** Scene asset identity covered files but not the
  meshes inside them: a sub-mesh is referenced by `gltf_mesh_<path>_<node>_p<n>`, which is not a
  path, so the registry answered nothing and the reference gained no identity. Stamping now uses
  the path inside the key, and repair **rewrites** the key around the file's new location rather
  than replacing it with the path — which would turn "the Body mesh in car.glb" into "car.glb".

  `MeshSource::split_gltf_key` / `gltf_key_with_path` are the shared parse, in `gizmo-core` below
  both consumers; the renderer's loader had its own copy and now uses this one.

- **`FpsLook` gained a look gate, sprint and configurable vertical keys — and its first caller.**
  `look_button` makes looking require a held mouse button (the mouse only; a stick is not gated),
  `sprint_key`/`sprint_multiplier` add sprinting, and `up_key`/`down_key` are configurable because
  ShiftLeft already meant descend and shift-to-sprint would have meant both. All three default to
  the previous behaviour.

  The three exist because the controller had **no callers at all**, and the first attempt to give
  it one showed why. `cpu_physics` is that caller now: −57 lines, and its state struct lost all
  four of its camera fields.

- **Gamepads in the browser.** The `gamepad` feature is no longer native-only: gilrs's wasm
  backend reads the Web Gamepad API, and the engine's side is identical on both targets.
  `demo-web` enables it explicitly, and CI lints the wasm arm with the feature on.

  Two browser-only facts, documented at the backend: the page must be a secure context, and the
  gamepad list stays empty until the player presses a button — so "no pad connected" is the normal
  first state on the web rather than an error.

- **Gamepad rumble.** `Input::rumble(weak, strong, duration_secs)` and `Input::rumble_pad` queue a
  request; `GamepadBackend::apply_rumble` hands it to the driver. The two motors are separate and
  named for the feeling — `weak` buzzes, `strong` thumps — because a game that sets only one is
  asking for a qualitatively different sensation, not a quieter version of the same one.

  It is a **queue** rather than state, which is what keeps three things from going wrong: a replay
  does not shake the controller (the queue is `#[serde(skip)]`), a dropped frame does not repeat a
  rumble (requests are drained), and losing focus clears pending ones and asks the driver to stop.

  **Known limitation, measured:** gilrs reports no force-feedback support for a Linux uinput device
  even when it advertises every capability bit gilrs's own test reads, so the device-level path
  could not be verified here — see docs/ENGINE.md §3. The engine's side (queue, clamps, focus-loss
  stop, one effect per pad, requests consumed even when unsatisfiable) is covered by tests.

- **`ColliderShape::Cone`.** Base at `-half_height`, apex at `+half_height`, axis local +Y —
  `Collider::cone(radius, half_height)`, the same convention as the cylinder. Convex, so it rides
  the existing GJK/EPA route.

  What is new is the support function, and it is the one part that must not be copied from the
  cylinder: a cone's extreme point in any direction is **either the apex or a point on the base
  rim**, never in between, because its radius shrinks to nothing at the tip. Everything derived
  from the shape got its own arm — volume (exactly a third of the bounding cylinder), an analytic
  inertia tensor (`I_y = (3/10)·m·r²`, and the transverse term taken about the collider origin
  rather than the centroid, which a cone's is not), an exact AABB (the union of a point and a
  disc), a ray test whose lateral quadratic is genuinely a cone's, the cloth pusher and the debug
  wireframe.

  Also fixed with it: `SimpleApp::spawn_textured_cone` gave a cone mesh a **sphere** collider under
  an `// approximation` comment, and `spawn_textured_cylinder` still did the same a day after
  `Collider::cylinder` shipped. Both now use their own shape.

- **`ColliderShape::Cylinder`.** Flat circular ends, axis along local +Y, built with
  `Collider::cylinder(radius, half_height)` — where `half_height` is half the *whole* solid,
  unlike `Collider::capsule`'s, which excludes the caps. The shape a wheel, a barrel or a column
  needs: a capsule of the same numbers rests a radius higher and rocks on its rounded end, and a
  box squares off a silhouette that is round.

  It is convex, so it goes through the existing GJK/EPA route; what is new is a support function
  whose radial and axial parts are chosen independently, which is what puts support points on the
  **rim** and lets a resting contact spread around the edge instead of balancing on a point.
  Everything a shape feeds got its own arm rather than a fallback: an analytic inertia tensor
  (`I_y = ½·m·r²`, `I_x = I_z = m(3r² + h²)/12`), an exact AABB, `πr²h` for volume, a ray test
  against the wall and both caps, cloth collision, the character controller's size derivation, an
  editable inspector row and a debug wireframe.

  `gizmo-studio`'s cylinder primitive now spawns one. It used to spawn `Collider::convex_hull` of
  the mesh's 24 ring points — a prism standing in for a circle, with an AABB-derived inertia.

- **`ColliderShape::Heightfield`** — terrain. `Collider::heightfield(heights, rows, cols, scale)`
  takes a row-major lattice of height samples centred on the collider's origin, with `scale` as
  (cell size in X, height multiplier, cell size in Z). Each cell is two triangles, dispatched
  per cell by the narrowphase and walked cell by cell (a 2-D DDA) by the raycast, so cost follows
  what a body or a ray actually overlaps rather than the size of the terrain.

  It is genuinely concave, which is the point: a body dropped into a valley rests on the valley
  floor. Any convex treatment — the shape's support function, a hull of its samples, its AABB —
  puts a lid across the dip instead, which is the "the car floats over the ditch" bug. The
  support function therefore refuses a heightfield the way it refuses a plane.

  Malformed input is inert rather than fatal: a sample count that disagrees with `rows * cols`,
  or a lattice too small for one cell, logs a warning and yields a field that collides with
  nothing. The cached bounds are `serde(skip)` and measured again on load, like the triangle
  mesh's BVH.

  Not covered: `Cone`, the third shape the 2026-08 audit listed.

- **Gamepads.** `Input` now carries connected controllers alongside the keyboard and mouse:
  `input.gamepad()` for the pad in use, `input.gamepads()` for local multiplayer, with named
  `GamepadButton`/`GamepadAxis` enums rather than opaque codes. `ActionMap` gained
  `bind_gamepad_button` and `bind_gamepad_axis` (an axis past a threshold, with real press and
  release *edges*), plus `watch_gamepad` so one map per player reads one pad. The Lua API gained
  `input.gamepad_pressed/just_pressed/just_released/axis/connected/name`, whose button names come
  from `gizmo_core::input::NAMED_GAMEPAD_BUTTONS` rather than a second transcription.

  The device side is `gizmo_app::gamepad::GamepadBackend` over [gilrs], behind the default-on
  `gamepad` feature (native only — the browser's gamepad API is polled, not evented, and is not
  wired). The windowed loop drives it: it pumps once per frame *before* the replay logic, so a
  recording captures the pad and a replay is not steerable, and it restores pad state on focus
  regain because a driver reading the device directly never stopped sending while the window was
  away and never announces what is still held on the way back. That restore replays the backend's
  own mirror of the event stream: gilrs reads nothing from a device when it opens it (measured —
  a pad holding its trigger at maximum reports no value at all until something moves), so a
  control already held when the game *launches* is invisible until it moves. Once seen, it stays
  known.

  Sticks are read through a **radial** deadzone with the remaining travel rescaled to a unit disc
  (`apply_stick_deadzone`, default 0.15) — a per-axis deadzone leaves a square dead region, so a
  stick pushed to `(0.14, 0.14)` reads as centred and one pushed to `(0.16, 0.14)` snaps to pure
  horizontal, and square-gated hardware reporting `(1, 1)` at full diagonal is a 41 % speed bonus
  for running diagonally. Triggers get their own smaller deadzone (0.05) because every hundredth
  there is throttle resolution.

  `car_demo` and `platformer` are playable on a controller: analog throttle and brake on the
  triggers, steering and movement on the left stick with the magnitude preserved (a light push
  walks), camera on the right stick, gears on the bumpers. The fighting-game motion buffer reads
  its frames through `ActionMap`, so a quarter-circle is enterable on a d-pad with no new code
  there at all — `a_quarter_circle_motion_can_be_entered_on_a_dpad` is the proof.

  Held state survives what it should and not what it shouldn't: focus loss releases the pad (an
  Alt-Tabbed game must not keep driving), and a controller unplugged mid-hold emits its release
  edges and centres its axes before disappearing, so a charge never sticks.

  [gilrs]: https://gitlab.com/gilrs-project/gilrs

- **Clustered light culling, and a light ceiling of 256** (`MAX_LIGHTS` 32 → 256). The view volume is
  cut into 16×9×24 clusters; each light is assigned to the clusters its sphere of influence touches;
  the deferred and forward lighting loops walk their own fragment's cluster list instead of every
  light in the frame. Per-fragment work is bounded by `clustered::MAX_LIGHTS_PER_CLUSTER` = 32
  regardless of scene light count, so the cap stopped being a frame-time budget in disguise.

  The assignment runs on the CPU (`gizmo_renderer::clustered::assign_lights`, a pure function with
  unit tests): measured 0.047 ms at 8 lights, 0.106 at 32, 0.201 at 64, 0.469 at 128, 0.764 at 256.
  The two cluster buffers live on bind group 0 because the web forward pipeline already binds four
  groups and WebGPU's baseline `maxBindGroups` is 4 — clustering therefore works in the browser too.
  `SceneUniforms` gained `cluster_dims` and `cluster_depth`, both derived from the camera.

- **Scenes can find an asset that moved (asset identity, path-authoritative).** `EntityData` and
  `MaterialData` gained `mesh_uuid` / `texture_uuid` (`serde(default)`, so every existing scene
  loads unchanged), and `SceneData::save_with_identity` / `load_into_with_identity` write and use
  them. The path is still what a scene names and what loads it; the UUID only speaks up when the
  registry reports a *different* current location for the same asset — i.e. the file moved with its
  `.meta` sidecar, which is exactly what a path reference cannot survive. A rename that leaves the
  sidecar behind orphans the identity and is not covered.

  The resolver is supplied by the caller through the new `gizmo_scene::AssetIdentity` trait, because
  `gizmo-scene` sits below `gizmo-renderer` and must not learn what an asset is;
  `gizmo_app::asset_identity::ManagerIdentity` is the implementation over `AssetManager`. **This is
  also the release where anything populates that registry at all** — no code anywhere called
  `AssetManager::scan_assets_directory`, so the scanner, the UUID resolver in `load_obj` /
  `load_material_texture` and the `.meta` sidecars on disk had never been connected. The studio
  scans its asset-browser root at startup (read-only — minting stays an explicit import), the
  exported runtime scans its own layouts, and an application that scans nothing behaves exactly as
  before.

### Fixed

- **A debug line drawn every frame was drawn twice on most frames.** The renderer cleared
  `Gizmos` only on frames that had stepped physics, so at 144 Hz over a 60 Hz step a line from an
  update system was drawn twice on more than half the frames (a moving one with a ghost a frame
  behind), and a line from a fixed-step system twice on a frame with two steps. `Gizmos` now keeps
  two lifetimes: `fixed_lines`, what the last physics step drew, kept until the next step (the
  frame loop calls `Gizmos::end_fixed_step`), and `lines`, cleared after every rendered frame.
  `vertices()` / `vertex_count()` give both. `physics_debug_system` also stopped logging at `info`
  on every call.
- **A replaced `MeshData` was never drawn.** The asset loader read an entity's `MeshData` only on
  the first frame it had no `Mesh`, so geometry rebuilt in code (a vision cone each frame) stayed
  as it was first uploaded. It now tracks which allocation each entity's mesh came from and uploads
  again when the data changes, releasing the upload no entity draws any more (the game path never
  ran `garbage_collect`, so replacing data every frame would have kept every frame's buffer), and
  emptied data removes the mesh. GPU-tested over fifty replacements: the mesh is the latest data's
  and the cache holds one upload; never re-uploading, or not releasing, fails it.
- **A `NavAgent` sent to the cell it stands in never arrived.** Its route is empty, and only the
  route-following branch checked for arrival: it stayed `Moving` with its target set, so a patrol
  whose first point was the guard's own spot never went on. A route-less agent arrives too now.
- **A `RigidBody` without a `Collider` stood still without a word.** The physics sync gathers bodies
  with colliders only, so such a body — a guard steered by its `NavAgent`'s velocity — is never
  simulated. It warns when the count of such bodies grows (once, not every frame).

- **Fixed, hinge and ball-socket joints broke at half their `break_force`.** Their shared point
  constraint skipped its linear rows while the anchors were within 0.1 mm — so a perfectly
  placed joint let its body fall a step before it pushed back, and then pushed back for the fall
  and the sag together. Measured on a 10 kg weight hanging from a ball socket: 196 N reported
  against the 98 N it carries, every step, independent of the iteration count; the threshold is
  compared with that number, so a joint rated for 150 N gave way under a 75 N load. The weight
  never fell asleep either. The rows run whatever the error now: 98 N, and asleep, and the
  first-step transient drops from 260 N to 100 N. (Rope and distance joints are one-sided limits
  and keep their behaviour — a rope that becomes taut still catches a falling body.)

- **A despawned body's joints tied its id's next owner to the partner.** Removing a body from the
  physics world left the joints attached to it in place, and joints find their bodies by raw id —
  so the next body to recycle the id was held by the dead body's joint. Measured: a body spawned
  15 m from the dead one's partner was pulled 2.5 m toward it in a second (and the partner toward
  it). Joints naming a removed body are removed with it now. Found sizing up a bridge-building
  game; a regression test spawns onto the recycled id and fails without the change.

- **The engine's warnings never reached a game.** No game set up a `tracing` subscriber (the full
  one is behind the default-off `tracing-layer` feature), so every `tracing::warn!` and `error!`
  in the engine — a texture that would not load, a navigation grid clipping the level, a route
  that hit the search cap — went nowhere. The windowed and headless runtimes now install
  `gizmo_core::logger::install_console_subscriber()` when nothing else has: events at `warn` and
  `error` (or the level `GIZMO_LOG` / `RUST_LOG` names; `off` installs nothing) go to stderr and
  to the log the editor console reads, with their fields appended. No dependency added — it is a
  forty-line `tracing::Subscriber`. Tested in its own binary.

  The first thing it showed: all eighteen games printed the same warning, `set_update replaced an
  existing update hook`, because each replaces `with_simple_scene`'s fly camera and variable-step
  physics with its own on purpose. `App::replace_update` says so and does not warn; `set_update`
  still warns (it was added for demos that lost their physics by accident) and its message now
  names `replace_update`. The games use it, and run 900 frames each with nothing printed.

  Then every demo, 300 frames each (136 of them, none crashed): eight more printed the same
  warning, all deliberately — seven call `simple_scene_update` at the top of their own hook, the
  way the warning advises, and `sprite_2d` drives its own camera and says why; they use
  `replace_update` now. Two (`system_forms`, `error_handling`) printed "`after('frames')` does not
  match" for a constraint that was being kept: the label lived in `PreUpdate` and the constraint in
  `Update`, and the scheduler only looked inside the phase. A constraint on a label of another
  phase is now told apart — `after` an earlier phase (or `before` a later one) holds by phase order
  and is silent; the reverse can never hold and is reported as such; a label nowhere is still
  "does not match". Tested both ways round, each wrong branch fails it. What remained were assets
  the repository does not ship (`AnisotropyBarnLamp.glb`, `scenes/main.scene`) and egui's note on
  an sRGB framebuffer.

- **A small viewport blacked out the main view.** As soon as any camera had a `viewport`, the
  frame went down the split-screen path, which drew only cameras with one — the primary camera
  without a viewport was skipped, and a minimap in a corner left the rest of the screen black.
  The primary camera without a viewport is now the whole target, drawn at its own `order`, so a
  small viewport with a higher order lands on top: picture in picture. GPU-tested (full view red,
  small view blue in its quarter, nothing outside it); removing the base view fails it. Found by
  `pist`'s minimap.

- **A car driven by `FixedPhysics` never moved.** `FixedPhysics::advance` — the fixed-step
  driver `SimpleApp` games use — stepped the rigid world only; the gameplay controllers
  (`VehicleController`, `CharacterController`, the fight clock and hit detection) run only in
  `GameplayPhysicsPlugin`, which nothing adds by default. So a car on this driver sat still with
  its throttle down (measured: z moved 8·10⁻¹² m in 2.5 s). Each step now runs them first, in the
  plugin's order, then the rigid step they feed. Found while sizing up a racing game; a test
  drives a throttled car through `FixedPhysics` and fails without the change.

- **`NavGrid::find_path` was 3.4× slower than it needed to be.** Every query filled three
  HashMaps (scores, parents, closed set), allocated a `Vec` per expanded cell for its neighbours,
  and hashed each neighbour's cell into `obstacles` up to three times. The search state is now
  dense per-cell arrays reused across queries (a stamp invalidates them), neighbours are generated
  in place, and walkability is looked up once per cell per query: 400 long routes on a cluttered
  200×200 grid, 726 → 211 µs each. The routes are identical — same cells, same order, same
  failures: a test runs 369 queries (open and cluttered grids, same cell, into a wall, off the
  grid, another layer) against the old search, kept beside it for that; a wrong walkability
  cache or a stale stamp fails it. A destination on another layer now returns `None` at once
  instead of after exhausting the layer.

- **A crowd replanned all at once, every half second.** Agents spawned or ordered together had
  their replan timers in step, so every `recalc.interval` all of them ran A* on one frame: 1000
  agents, 58 ms on every 30th frame against a 0.27 ms median. After a forced replan (new target,
  route used up, wall moved, first plan) the next scheduled one now waits for the agent's own
  slot of the interval, so the crowd's replans spread: the same 1000 agents run ~2.4 ms every
  frame and no spike after the first. A test counts replans per frame for 300 agents (≤ 30;
  without the slot, 300).

- **Agent separation scanned every pair.** `ai_navigation_system` found each agent's neighbours
  by checking every other agent: 0.83 / 13.1 / 53.2 ms a frame for 250 / 1000 / 2000 agents. They
  come from XZ buckets now — 0.07 / 0.29 / 0.58 ms (steering only; path replans are separate) —
  visited in agent order, so the result is bit-identical. A test steps a 120-agent crowd of mixed
  radii both ways and compares every velocity's bits; visiting only the agent's own bucket, or
  the buckets unsorted, fails it.

- **The cursor ray came from a stale camera position.** `pointer::cursor_ray` (and so
  `cursor_on_plane`) read the camera's `GlobalTransform` first, which in game code is a frame
  behind — a camera spawned this frame cast from the origin, and one following a player cast from
  where it was. It reads this frame's place through its parents now (`world_position`). Found
  writing `world_to_screen`'s example, whose freshly spawned camera projected nothing; a test pins
  it and the old lookup fails it.

- **A multi-part compound collider crashed the physics near any CCD body.** The CCD path's
  speculative contact handed a `Compound` to GJK whole, which violates `support_point`'s contract:
  a debug panic, and in release a zero support point — a wrong contact — on the first step a CCD
  body came near it. Every kinematic body turns CCD on, so a kinematic board made of parts
  (`labirent`) hit it on frame one. `Gjk::speculative_contact` now splits a compound into its parts,
  as the manifold path always has, and keeps the closest part's contact.
- **A ball caught on the seams of a tiled floor.** On a compound of boxes laid edge to edge, a body
  resting a hair inside the surface also touches the next box's top edge, whose normal leans back
  against the motion. Measured with a CCD ball down seventeen tiles: 1.01 m in two seconds against
  2.21 m on one slab (the analytic value). The narrowphase now drops a box part's edge contact when
  another part holds the body with a face contact on the same plane — a real edge (a step) is off
  that plane and stays: 2.03 m. Without CCD the tiles cost nothing either way (2.22 m). The last
  8 % was not the seams — see the next entry.
- **A speculative contact with nothing left to close was taken for a resting one.** When a CCD
  body arrives within the 1 cm skin, `Gjk::speculative_contact` may let it close nothing more, and
  wrote that as a penetration of `-0.0` — which is not `< 0.0`, the solver's test for a
  speculative contact. So the contact was solved as a real one: restitution allowed, and friction
  applied at its anchor, the dynamic body's **centre**, where an impulse has no lever arm — it
  does not spin the body, it takes its tangential speed. A CCD ball dropped 5 cm onto a tilted
  floor lost the 0.156 m/s it carried down the slope on landing and covered 2.03 m in two seconds
  instead of 2.23 m (the slab's 2.21 m; the slab run lands through a real contact and never saw
  it). Both halves are fixed: the gap is strictly negative, and a speculative contact gets no
  friction at all — the bodies have not touched, and the next step's real contact rolls it. The
  first suspect, the contact's below-1 mm normal fallback, was tried and changed nothing; tracing
  the step found one contact and a velocity set to exactly zero. Undoing either half fails a test.

- **UI boxes on one layer were painted in storage (archetype) order, not tree order.** A slot
  (`NodeBundle` + `DropTarget`) and the item inside it (`ButtonBundle` + `Drag`) live in different
  tables, and the table that came first painted first: in `birlestir` every slot covered its own
  item, leaving only its label visible. The facade's `ui_boxes` now walks the hierarchy — a
  parent before its children, siblings in `Children` order — and the renderer's stable layer sort
  keeps that order. Tested with the child's table created first (storage order gave
  `[3, 2, 1, 0]`).

- **Large imported meshes no longer draw a nonsense LOD.** `Mesh::new_indexed` (every glTF and
  OBJ import, every primitive) passed its de-duplicated vertex array to `Mesh::new`, whose LOD
  step reads its input as a flat triangle list — so a mesh with more than 20 000 unique vertices
  got a LOD1 made of every three consecutive vertices, drawn whenever it was far away. LOD1 now
  comes from the mesh's own indices, and the gate counts triangle corners on both paths
  (`LOD_MIN_CORNERS`), so indexed meshes past it get a real LOD: `create_sphere(1.0, 48, 72)`
  has one now (it had none — the gate counted its 3 575 unique vertices). LOD1 keeps 99.8 % of a
  sphere's surface area; the old reading covered 2 %.

- **Sky pixels are no longer lit as geometry.** The G-buffer's position target was cleared with
  `Color::BLACK`, whose alpha is 1 — and that alpha is the "something was drawn here" flag every
  reader tests. So no pixel was ever empty: the background was a surface at the camera with a
  zero normal, lit like one (it happened to come out near the intended dark grey), the empty-pixel
  branch never ran, and the volumetric pass gave every sky ray zero length, so god rays never
  reached the sky. Cleared to `TRANSPARENT` now. The background is the intended 0.05 grey
  (corner 41/43/46 → 42/42/42 under the vignette), and sky rays march `max_distance` — the
  volumetric step-count test re-measured 2755 → 16276 differing pixels, largest 6 → 32. **Visible
  change:** smooth metal that used to reflect that dark fake surface where it saw sky now
  reflects the environment and is brighter; the orthographic TAA shimmer probe re-measured
  33 → 50 working (131 broken) and its bar moved from 38 to 80.

- **A `Grid` material draws a grid in the game**, as it always did in the editor: `route(Grid)` is
  forward, and the game's forward pass binds `grid_pipeline`. It drew as a plain opaque PBR plane.
- **The game-path skybox draws** — `CAPABILITY_GAPS` §D10 had it drawing nothing, cause unknown.
  It no longer reproduces (a sky cube repaints the frame; `atmosphere` differs in 10066 sky
  pixels with and without its cube); `a_skybox_draws_in_the_game_path` pins it.

- **The deferred PBR path reads vertex colour**, at face value as `baked_lit.wgsl` does. A mesh
  with coloured vertices drew uncoloured in the game and coloured in the editor. The forward
  shader's rule that a near-black vertex colour counts as white is gone too — it turned a surface
  painted black white, and an uncoloured source is already white at import. Engine primitives
  are white and unchanged.

- **`Material::emissive` glows.** The deferred path never read the per-instance emissive that
  `with_emissive` sets (only `baked_lit.wgsl` did), and the forward shader read none at all; a
  glTF emissive was added to the 8-bit albedo, lit and clamped at 1. The G-buffer gained an RT5
  (`emissive`, `Rgba16Float`; `DeferredState::emissive_view`, 44 bytes per sample) that the
  lighting pass adds unlit and in HDR, and the forward shader adds the instance's emissive too.
  **Visible change:** glTF emissive surfaces are brighter than before, and PBR materials with
  `with_emissive` glow for the first time.

- **A plain `despawn` no longer leaves dangling hierarchy links.** The despawned entity is taken
  out of its parent's `Children`, and its direct children lose their `Parent` link and become
  roots. Before, both kept naming the raw id, so once it was recycled a parent listed a stranger
  as its child and orphans hung off — and inherited the transform of — whatever entity got it.

- **`motion_blur` demo: the camera never moved.** It was posed in PreUpdate and the simple scene's
  fly camera overwrote it every frame, so its "same pixels at sixteen times the speed" measurement
  was of a still camera. Posed in `set_render` now; re-measured.

- **Axis locks leaked through two position paths; a body locked to a plane drifted off it.**
  Locks are velocity-level, and the transform was moved past them twice: Heun's position step
  averages `pre_linear`/`pre_angular`, copied from the solver's output before locks run, and the
  contact solver's positional correction is applied to the transform directly. A box with
  `planar_xy()` dropped off-centre onto a sphere ended 4·10⁻⁴ off z = 0; it now stays at exactly
  0 (`gizmo-physics-rigid/tests/planar_xy.rs`, which fails with either fix removed). New
  `RigidBody::mask_locked` does the masking for displacements. Bodies without locks are
  bit-identical (`headless_stress_test` hash unchanged); locked bodies change slightly, by design.

- **A `PhysicsMaterial` component on an entity now sets its surface — for the contact solver and
  for vehicle tyres.** It is an ECS component, `world.add_component(ground,
  PhysicsMaterial::ASPHALT)` compiled, and nothing read it: the solver and the wheels both took
  `Collider::material`, which `Collider::trimesh` leaves at the default (dynamic friction 0.5). A
  downstream racing project measured the cost: every tyre held 0.5 / 0.65 = 77 % of its asphalt
  grip, and a car's lateral limit read 0.62 g instead of 0.84 g with the asphalt component on the
  road. The rule is now `Collider::material_with(component)` — the component wins — applied by both
  ECS→physics bridges (`physics_step_system`, and the wheels' first-frame ECS fallback), so a body
  cannot have one surface for collisions and another for tyres. Without a component nothing
  changes: `headless_stress_test` still hashes `A462C9EB8A09D5CA`.

- **`World::compact` promoted an invalid `EntityLocation` into a valid one pointing at row
  `u32::MAX`.** Reclaiming an empty archetype swap-removes it and moves the last archetype down
  into the freed slot, then renumbers the locations of every entity that archetype lists.
  `ArchetypeIndex::gc_empty_archetypes` did that one field at a time — `loc.archetype_id =
  target_id` — over whatever the slot happened to hold, and `EntityLocation::is_valid` inspects
  `archetype_id` alone. So a slot holding `INVALID` came back as a real archetype with `row`
  still `u32::MAX`, and `World::query_entity`, `World::get_component_ptr` and `Query::get` turn a
  row into an unchecked byte offset in both build profiles.

  The slot holds `INVALID` for an *orphaned row* — an entity listed in an archetype whose
  location does not name it. `World::despawn` builds one whenever an `on_remove` hook re-lists
  the dying entity; it names that in a `debug_assert` and leaves release behaviour unchanged, on
  the stated ground that the row is unreachable *because* its location is invalid, and
  `compact`'s own documentation called such rows harmless for the same reason. This write was
  what falsified that ground, from ordinary maintenance. The renumbering now touches only
  locations that actually name the archetype being moved. `entity_archetype` keeps its
  unconditional update, which is correct rather than an oversight: every id in `entities()` *is*
  held by that archetype whatever its location says.

- **A zero-sized component with an alignment was undefined behaviour on the query path.**
  `#[repr(align(16))] struct Marker;` is a legal component. `BlobVec` gave every column that owns
  no allocation the same base pointer regardless of its element — `NonNull::<u8>::dangling()`,
  address 1 — and this was documented as safe on the ground that the pointer "is never
  dereferenced". `query::fetch`'s `get_item` dereferences it for every row of a zero-sized
  column, and `get_slice` hands it to `slice::from_raw_parts`, which requires an aligned pointer
  even for a length of zero. A reference must be aligned for its type whether or not the type has
  bytes to read, so a plain `world.query::<&Marker>()` handed safe code a misaligned reference; a
  debug build aborts on it with `misaligned pointer dereference: address must be a multiple of
  0x10 but is 0x1`. The pointer is now the element's own alignment as an address, at all three
  places that install one — construction, the zero-sized branch, and the deallocation
  `shrink_to_fit` performs (which `compact` runs over every column on every GC tick).

  The same root had a fourth site: `ComponentSparseSet::clone_entry` passed address 1 to the
  component's cloner as a *destination* for a zero-sized type, and `ptr::write` requires an
  aligned destination too — so an over-aligned sparse-stored marker was undefined behaviour when
  cloned, which is what prefab instantiation and `ObjectPool` do.

- **`World::query_cached` silently missed every entity in an archetype created after its first
  call.** The archetype index memoises which archetype indices match a query type and invalidated
  that cache only when an archetype was *removed* or the world cleared. Creating one — which
  every first `spawn_bundle` of a new component combination does, and every `add_component` that
  migrates an entity into a shape no entity held yet — set nothing, across five creation sites,
  two of which push onto the archetype vector from `World` rather than through the index. No
  panic, no assertion, no `unsafe`: a public query returning fewer entities than it has.
  Creation now goes through one `ArchetypeIndex::push_archetype`, and a debug build re-derives
  the match list and compares rather than trusting it, so a sixth creation site added later
  cannot reintroduce this without a named failure at the query that suffered it.

- **The three component removals left two stale `EntityLocation`s behind when a component
  destructor panicked.** `World::remove_component`, `World::remove_bundle` and
  `World::remove_batch` migrate the entity to a smaller archetype and then repair three tables:
  the row of the entity that was swap-moved into the vacated row, the migrated entity's own
  location, and the archetype map. The removed components' destructors — the migration's only
  user code — ran *inside* `Archetype::move_entity_to`, before any of the three. A panicking
  `Drop` therefore escaped with the migrated entity still naming the row it had just left, which
  the swapped entity now occupies, and the swapped entity still naming a row the source archetype
  no longer has. `World::query_entity` and `World::get_component_ptr` index both with no bounds
  check in either build profile.

  This is the other direction of the invariant the entry below fixes, and it is invisible to that
  entry's check: every column already matches its entity list. `Archetype::move_entity_to_deferred`
  now returns a `#[must_use]` token instead of running the destructors, and each of the three
  callers consumes it after all three writes — so the drop glue runs over a world in which no
  table names a row that is not there. `move_entity_to` keeps the immediate disposal for
  `add_component` and `insert_batch`, whose target archetype is a superset of their source: they
  detach nothing and run no user code in the migration at all.

- **Six composite operations left the ECS indexable past the end of a column when a component
  destructor panicked.** `Archetype::len()` is its entity count, every query bounds itself by
  that, and the fetch path turns a row into a raw offset with no bounds check in either build
  profile — so an operation that shortened a column while the entity list still counted the row
  handed safe code a reference past the end of an allocation. All six ran their destructors, or
  a user `Clone`, while their parallel arrays still disagreed.

  The rule they now follow is that the entity list is the visibility switch: a removal shortens
  everything *before* any destructor runs, an addition pushes the id *after* every column is
  filled, and the removed values wait above the length where nothing can reach them.
  `Archetype::swap_remove_entity` splits into a detach and a drop; `Archetype::clear` and
  `ArchetypeIndex::clear_entities` empty every archetype before dropping anything in any of
  them; `ComponentSparseSet::remove` and `::clear` repair their four arrays first;
  `World::spawn_batch` stages its row and commits it last; `Archetype::batch_clone_row` abandons
  a half-cloned batch rather than leaving its columns torn against each other; and
  `World::clear_entities` performs every infallible reset before the two clears that run
  destructors, so an unwind can no longer skip the location table, the id allocator and the
  pending command queue.

  A panic on any of these paths now leaks whatever it did not reach — safe, and the only sound
  answer — instead of leaving something reachable and wrong. `ComponentSparseSet` also gained an `O(1)`
  debug-only length check across `dense`, `ticks` and `entities` that `insert` and `remove` run.

- **The last three sites where a panicking component `Drop` could corrupt the world — two of
  them real.** `add_bundle`'s *same-archetype* branch dropped every component the bundle names
  and refilled the slots one statement later, so a panic out of a `Drop` or out of a
  `Bundle` impl left slots that `entities` and the column both still counted holding a destroyed
  or uninitialised value. The old values are now duplicated bytewise into a row above `entities`
  — which no query can reach — before the write, and dropped after it, so the two possible
  outcomes are "the bundle did not land and every old value is intact" and "the bundle landed and
  what was not reached is leaked". `ComponentSparseSet::insert`'s overwrite branch had the same
  shape and is fixed the same way, using the dense array's own tail as the temporary rather than
  the allocation the engineering notes had assumed it would need; the incoming value's ownership
  now transfers on *entry*, which `World::add_component` had to be changed to respect.

  **The third was not a defect.** The `*ptr = component` assignments in `add_component` and
  `insert_batch` had been recorded as drop-then-move. Measured on rustc 1.98: they are
  move-out/write/drop, so the slot is never left destroyed. What *was* wrong is that the change
  tick was stamped after the user code, so a panicking `Drop` left the row holding a new value
  under its old timestamp — permanently invisible to `Changed<T>` and `Added<T>`. Both sites now
  write and stamp in one infallible step.

  Under it all, `Column::swap_remove_and_drop` now shortens its tick array before it runs the
  element's drop glue rather than after, so a panicking `Drop` can no longer leave a column's two
  halves disagreeing about how long it is.

- **A panicking component `Drop` could leave the engine running drop glue over uninitialised
  memory.** `World::add_bundle`'s migration branch extended every target column by a row, left
  the columns the bundle was adding uninitialised, and only then called user code — the `Drop` of
  each component carried across, and `Bundle::write_to_archetype`. A panic out of either unwound
  past the fill, leaving a row that `entities` counted and whose new columns held garbage; the
  world's teardown then ran the component's drop glue over it. Worse than a double drop, and
  reachable with no `unsafe` from the caller: `catch_unwind` is safe std and nothing promised a
  component's `Drop` would not panic.

  The row is now **staged rather than committed**. `Archetype::len` is `entities.len()` and every
  query bounds itself by it, so a column longer than `entities` is a row nothing can reach —
  only the drop paths follow a column's own length. The bundle therefore writes into a row no
  query can see, and a single store makes it exist. If anything panics first, every column's
  length goes back and the bytes above it are abandoned: that recovery is the same whatever was
  written, which is what makes it possible at all. It leaks what those rows owned, which on a
  panic path is safe and is the only sound answer for memory that is part live and part
  uninitialised with no way to tell which.

  Confirmed with Miri, not argued: the old ordering reports
  `Undefined Behavior: … memory is uninitialized`, and the new one is clean.

- **`World::reset_epoch` is new** — how many times `World::clear_entities` has run, and the one
  thing a cache keyed by `Entity` can check itself against. A clear resets the generation counter
  as well as the id counter, so every handle it destroys comes back **bit-identical** attached to
  an unrelated entity: `is_alive`, the usual defence against a stale handle, cannot tell the two
  apart by construction. That is why `clear_entities` has always had to warn about "a selection
  list, a cache keyed by `Entity`" in prose — there was nothing for such a cache to compare.
  Record it beside whatever holds handles and discard that when the two differ.

  `PoolManager` is the worked example and now closes its own case: it drops every pool when the
  epoch moves. `PoolManager::register_pool` takes a `&World` for this reason and no other — a
  pool registered *after* a clear is current rather than stale, and telling those apart needs the
  epoch recorded at registration.

- **An object pool could clone a stranger, or hand out a corpse.** Two independent stale-handle
  defects in `PoolManager`, both reachable without destroying the world:

  `ObjectPool` stored its prefab as a raw `u32`. An id carries no generation, so once the prefab
  was despawned and its slot recycled — and the allocator drains its free list first, so the very
  next spawn lands there — `instantiate` cloned whatever entity had taken the slot and handed the
  copy out as a pooled object. The struct's own documentation described this rather than
  preventing it. It stores an `Entity` now, and a pool whose prefab is gone reports `None`
  instead of producing something from a stranger.

  And a parked entity that died *while* pooled was still handed out. `destroy` refuses to park a
  dead entity, but nothing keeps a parked one alive afterwards, so the queue could produce a
  corpse — on which every later component insert silently does nothing, which is the exact
  failure the parking guard was added to prevent. Dead entries are now discarded on the way out,
  falling through to a clone if the whole queue is stale.

  `ObjectPool::prefab_id: u32` is now `ObjectPool::prefab: Entity`, and `ObjectPool::new` takes
  an `Entity`. Both are `pub`; the crate is `0.x`.

- **`WorldStats::sparse_component_bytes` is new** — the memory a `SparseSet` component actually costs,
  which the existing `component_bytes` could never show because it sums archetype columns and
  sparse storage lives outside them. It reports both halves: `dense`, which scales with the
  number of entries, and the reverse index, which is **four bytes per id up to the largest ever
  inserted** whether or not those ids still carry the component. The second is the one that
  surprises — a set with one entry can hold megabytes — and it is what `World::compact` truncates,
  so that reclamation is now visible in the numbers the engine prints about itself rather than
  only in a debugger.

- **`BlobVec::shrink_to_fit` contradicted its own zero-sized-type sentinel.** `new` sets a ZST
  vector's capacity to `usize::MAX` to mean "never needs to grow", and `grow` relies on it by
  returning early without updating it; `shrink_to_fit` wrote `capacity = len` instead. Nothing
  broke — `push` short-circuits on ZSTs before consulting capacity — but `World::compact` runs
  this over every ZST marker column on every garbage-collection tick, so the invariant was false
  almost everywhere rather than in some corner.

- **A component whose `Drop` panicked could be dropped a second time.** Dropping an
  already-dropped value is undefined behaviour, and two primitives in the archetype storage
  allowed it: `BlobVec::clear` set its length to zero *after* its drop loop, so a panic left the
  length untouched and the vector's own `Drop` re-dropped everything before the panicking
  element; `BlobVec::swap_remove_and_drop` dropped the victim before decrementing, so a panic
  left a destroyed value still counted. `clear`'s SAFETY comment asserted the opposite in so many
  words — true on the normal path, false on the unwind path.

  Both now do the bookkeeping first, so a panic **leaks** the remainder instead of double-dropping
  it, which is safe and is what `Vec` does for the same reason. Nothing was added: no guard, no
  `unsafe`, no cost on the normal path. It is the root under `Column::clear`, `Archetype::clear`,
  `ComponentSparseSet::clear`, `Column::swap_remove_and_drop`, `Archetype::swap_remove_entity`,
  `Archetype::move_entity_to` and `ComponentSparseSet::remove`, none of which needed touching.

  Reachable without `unsafe`: `catch_unwind` is safe std and nothing here promised a component's
  `Drop` would not panic. Three narrower shapes remain open and are listed in `docs/ENGINE.md`
  §3, along with a separate hazard the same sweep turned up — composite operations update their
  parallel arrays one at a time, so an unwind in the middle leaves them disagreeing.

- **Per-entity event listeners outlived their entity.** `World::observe` files a listener under
  an `Entity`, and nothing removed it when that entity was despawned — only `clear_entities`,
  dropping every listener for every event type. A game that pairs `observe` with destroying
  things leaked one entry per kill, for the life of the world. Sixty-four despawns left
  sixty-four entries.

  It was a leak and not a correctness bug: the key carries a generation and a despawn bumps it,
  so a listener filed under a dead handle could never match the id's next occupant. What blocked
  the fix was expressiveness — the listener map's values are per-event-type, so `despawn`, which
  has an `Entity` and no event type to downcast with, had nothing it could call. The map now
  holds a small trait object instead of `Box<dyn Any>`, recording the removal at registration
  where the type is still known; the same shape `ComponentInfo` already uses for drop glue.

- **`add_bundle` could strand an entity between two archetypes if a component's `Drop` panicked.**
  Its migration branch put the entity in the target archetype, then ran two pieces of user code —
  the `Drop` of every component carried across, and `Bundle::write_to_archetype` — and only then
  recorded the new location. A panic out of either unwinds past that record, leaving the entity
  listed in the target while its location still names the source, at a row that by then may not
  exist. The next structural touch reads that row.

  Nothing about this needs `unsafe` or breaks a documented contract: `catch_unwind` is safe std,
  and the crate nowhere promises a component's `Drop` will not panic. Measured with the old
  ordering: `entity 1 at archetype 2 row 0, location says archetype 1 row 1`. The location writes
  happen before the user code now — the same reordering `despawn` and `insert_batch` received,
  and the third and last place with that shape.

- **`insert_batch` cloned the component in the middle of a migration.** `T::clone` is user code,
  and it was called between the archetype move and the location write — the one moment where the
  entity has a row in the target archetype and a location still naming the source. Anything that
  re-entered the world from there saw the same inconsistency a `despawn` hook used to see. It
  needs the caller's own `unsafe` to reach a `&mut World` from `Clone::clone`, so this is not a
  soundness fix; the clone simply happens before the migration now, so the window does not exist
  rather than being hard to reach. Still one clone per entity.

- **A `SparseSet` `on_remove` hook saw the dying entity pointing at somebody else's row.**
  `despawn` swap-removes the archetype row, then runs the sparse `on_remove` hooks with a
  `&mut World`, and only cleared the location at the very end of the function. For the whole of
  that window the entity's location still named the row it had just vacated — a row that now
  belongs to whichever entity was last in the archetype. A hook doing anything that routes
  through the location, and `add_component` is enough, handed that row to a migration, which
  moves whoever is *in* a row rather than whoever the caller meant, and dragged an unrelated
  entity into the target archetype under the dying entity's id.

  The location and the archetype-map entry are now cleared as soon as the row is gone, so the
  hooks see an entity with no row and no location and the paths that ask either question agree.
  A hook that puts the entity *back* — `flush_spawn` on a deferred spawn does, and that is its
  documented purpose — cannot be caught by liveness, because the id is not retired until after
  the hooks; that one is a `debug_assert` at the despawn, so release behaviour is unchanged and a
  debug build names it at the cause.

- **A deferred spawn whose entity was destroyed before the queue ran still took a row.**
  `Commands::spawn` reserves the id and hands the handle out immediately, queuing the storage
  commit for later; despawning that handle before the queue is applied is ordinary use, and the
  queued `flush_spawn` then ran anyway. It consults neither the allocator nor the existing
  location, so it appended a row for a dead id — and because the despawn had returned that id to
  the allocator, the next `spawn` took it back and appended a **second** row, leaving the empty
  archetype listing one id twice with only the later row recorded. Anything reaching that entity
  afterwards reads or writes the wrong row. A flush for an id that is no longer alive is a no-op
  now; a second flush for the same live id — which the function's contract already forbade — is a
  `debug_assert` rather than silent corruption.

- **`spawn_batch` could write a batch into an archetype that has no columns for it.** It spawns
  its first bundle normally to discover the archetype the rest are appended into, and that
  spawn's `on_add` hooks run in between. A hook that *removes* the component it was just given
  migrates the entity to a different archetype — a perfectly valid location naming the wrong
  place — and the append loop then wrote into columns that do not exist there. It panicked with a
  message asserting the cause was SparseSet storage, which had nothing to do with it. The
  archetype is now checked against the bundle's own component set rather than merely for
  existence, and that panic message names both ways of reaching it.

- **A migration that moves the wrong entity now says so, at the line that caused it.**
  `Archetype::move_entity_to` does not take an entity — it takes a *row*, and moves whoever is
  sitting in it. Every caller has an entity in mind and derives that row from the entity's
  recorded location, so the two agree only while the location is fresh; when it is stale the
  migration moves a stranger and the caller files the new row under its own id, leaving two
  entities wrong and nothing to notice. That is not hypothetical — it is exactly what the
  duplicated-batch defect below did.

  It now returns a `Moved` naming the entity it actually took, and every call site
  `debug_assert_eq!`s that against the entity it meant. Nothing changes in release; in tests,
  under Miri and in CI a stale row is reported as `migration moved entity 3 while the caller
  meant 0: a stale row` at the migration itself, rather than surfacing later as whatever trips
  over the corruption first.

- **A duplicated entity in an `insert_batch` or `remove_batch` slice silently corrupted the
  world.** Neither grouping loop deduplicated, so one entity named twice in the caller's slice
  landed in its group twice. The migration loop reads each member's location fresh — and the
  first pass has just moved that location to the **target** archetype — so the second pass handed
  a target row to `move_entity_to` on the **source** archetype. That drags whichever entity now
  occupies that source row into the target and records the new row under the duplicate's id.

  Two entities end up wrong: the dragged one is listed in an archetype its own location does not
  name, and the duplicate's location points at a row it does not occupy. Neither panics. Every
  later read and write through those locations lands on a different entity's data — measured as
  `entity 0 sits at archetype 2 row 0 but its location says archetype 2 row 1`. Nothing in either
  function's documentation forbids duplicates, and a slice collected from two overlapping sources
  is an ordinary way to produce one. Duplicates are now collapsed rather than rejected.

- **Five places indexed the entity-location table raw, and each of them could panic.** Three
  share one cause, the fourth is its mirror image, and the fifth is a live crash in `spawn_batch`.

  **`remove_component`, `insert_batch` and `remove_batch` panicked on an entity that had been
  reserved but not yet flushed.** `Commands::spawn` hands out an id before the `flush_spawn` it
  queues has run. Between those two moments the entity is `is_alive` — it has an id and a
  generation — but owns no archetype row, and if its id is past the end of the location table it
  has no slot there at all. All three functions checked `is_alive` and then indexed
  `entity_locations[id]` **raw**, so the call died with `index out of bounds` rather than doing
  nothing.

  The two batch APIs are the likelier way to meet it: their grouping loops read the location raw
  *between* `is_alive` and the `loc.is_valid()` test that was meant to catch exactly this, so the
  guard ran one line too late, and a caller who collects a slice of entities needs only one
  unflushed member in it to take the whole call down.

  **`despawn` panicked when one of its own hooks cleared the world.** Its re-fetch of the
  location *after* the `on_remove` hooks — the read whose entire reason for existing is that a
  hook may have changed things, and which is commented "Re-fetch location safely" — was the raw
  one. `run_hooks` hands each hook a `&mut World` and `hooks.rs` documents them as free to spawn,
  despawn and mutate; a hook that reaches `clear_entities` empties the table, and the re-fetch
  then indexed a vector of length 0. The entry read forty lines above it is bounds-checked and
  carries a comment explaining why. The one added because state may have changed was not.

  **`spawn_batch` crashed when a hook despawned its first entity.** It spawns the first bundle
  normally to discover the archetype the rest are appended into. That spawn fires `on_add`/
  `on_set`, and a hook may despawn the entity it was handed — leaving the batch with no
  archetype. The location then reads `INVALID`, whose `archetype_id` is `u32::MAX`, and the
  append loop indexed the archetype vector with it: `index out of bounds` for any batch of two or
  more. A one-element batch hides it, because the loop body never runs. It now falls back to the
  per-entity path, which is the same answer the function already gives when it cannot use its
  fast path.

  All five now read through `World::entity_location`, the bounds-checked accessor that answers
  `INVALID` for an id past the end — which every branch below them already handled.

  **The class was already known**, which is the part worth recording: `add_bundle` carries a
  comment describing this exact entity and guards against it, `despawn` handles it, and both have
  regression tests. Three sites were simply missed. Found while auditing `entity_locations`'
  indexers for a *different* change — the one below, which reclaims the sparse index and
  deliberately leaves the location table for its own commit.

- **`World::compact` walked past one of the two largest allocations in the world.** Its
  documentation
  promised RAM "back towards the initial defragmented state"; it shrank the archetypes, the
  location table and the id allocator, and never touched sparse storage. A `SparseSet` component's
  reverse index is indexed *directly by entity id*, so its length is `largest id ever inserted + 1`
  rather than the number of entries — `remove` only writes an absent-sentinel into it, and nothing
  but `clear_entities`, which destroys every entity, ever gave it back. A world that had once
  spawned a million entities carrying such a component kept four megabytes **per component type**
  across every `compact`, and the studio calls `compact` from its garbage collector every three
  seconds.

  Shrinking the arrays would not have fixed it: a `Vec`'s doubling slack is bounded by the number
  of entries, so it reclaims a constant factor while the id-sized index stays exactly as long.
  What reclaims it is **truncating** past the last non-sentinel entry — a trailing sentinel and an
  id past the end of the vector already mean the same thing, absent — and every reader
  bounds-checks, so a short index reads as absence rather than as an error.

  **`World::entity_locations` is the same defect one size larger and is deliberately left for its
  own change.** It too is indexed directly by entity id, at 8 bytes per id against a sparse
  index's 4, and `compact` still only reclaims its slack. Truncating it is the same argument;
  it is not the same audit, because 29 of the 33 places that index it in code carry no bounds
  check, against the one that `sparse` has. `compact`'s rustdoc now says so rather than leaving the
  omission to be rediscovered.

- **A command queued before `World::clear_entities` was delivered to an unrelated entity after
  it.** This is sharper than an ordinary use-after-despawn. `despawn` bumps the slot's generation,
  so a stale handle stays dead and a queued command misses harmlessly; `clear_entities` resets the
  generations *and* the id counter, so the first entity spawned afterwards is `Entity(0, gen 0)` —
  the same 64 bits the closure captured. It does not miss. It gives a stranger a component nobody
  asked for, or despawns it.

  The queue is now discarded at the clear, not applied: applying would run a queued `despawn` and
  with it the `on_remove` hooks that `clear_entities` documents itself as not running. The
  function's own rustdoc had named "a queued `Commands` closure" as an example of the hazard while
  leaving the queue full.

  **One case is named rather than closed:** an `Events<T>` queue has the same defect and cannot be
  fixed at that layer. `CollisionEvent`, `TriggerEvent` and `HitEvent` all carry `Entity` handles,
  but `Events<T>` is an ordinary resource with no registry of which resource types are event
  queues, so there is nothing generic to drain — and unlike the per-entity observer map, whose
  whole contents could go, the resource map also holds `Time`, `Input` and the world's machinery.
  Both `clear_entities` and `ENGINE.md` now say so.

- **A malformed glTF file could abort the editor on import, or turn a few hundred bytes into an
  exponential number of GPU uploads.** The glTF spec says a document's nodes form a disjoint
  union of strict trees. Nothing enforces it: `gltf` 1.4.1 validates index bounds and vocabulary
  only — its whole error vocabulary is `IndexOutOfBounds | Invalid | Missing | Oversize |
  Unsupported`, the word "cycle" appears nowhere in `gltf-json`, and children are stored as plain
  indices resolved lazily. So `nodes: [{children:[1]}, {children:[0]}]` imports `Ok`.

  `parse_gltf_node` then recursed over `node.children()` with no visited set, no depth cap and no
  de-duplication, and the two malformed shapes broke it two ways:

  - a **cycle** recursed until the stack was gone — measured with the guard removed:
    `fatal runtime error: stack overflow, aborting`, SIGABRT. `gltf::import` runs on a worker
    thread, but this runs on the **main** one, so it takes the editor and its unsaved scene down.
  - a **DAG** — the same node index named as a child twice — terminates, and is the quieter and
    more expensive half: each path materialised a separate subtree, re-running the primitive loop
    and re-uploading its vertex buffers (`mesh_cache` is written at the end of that loop and never
    consulted before it). A file whose node `i` names node `i+1` twice turns N nodes into 2^N
    materialisations.

  One `HashSet<usize>` of node indices, shared across every scene root of an import, is both
  guards. For a conforming file it never fires, so nothing about loading a valid model changes.

  A second, independent walk in the same loader had the same gap in the other direction:
  `compute_armature_root_transform` climbs the node-parent map from a skin's first joint, and its
  only exits were reaching a bone of that skin or a node with no parent. A parent loop among
  non-joint nodes never left it — and this one *grows* while it spins, pushing a `Mat4` per step,
  so the failure is unbounded allocation rather than a stall. It now stops at the back-edge and
  keeps the ancestors gathered so far, which is the degradation `gizmo-animation`'s
  `SkeletonHierarchy::calculate_global_matrices` already documents for the equivalent bone-level
  walk: a partial transform, not a file that refuses to load.

  Both were found by the audit of the `Children`-cycle sweep below, looking for the walks that
  sweep's six-site list never named. They are the eighth and ninth.

- **The UI's clickable rectangle could stop matching its visible one, and an unstyled entity
  could take a child out of the layout tree entirely.** Both are consequences of the cycle guard
  added to `gizmo-ui`'s layout mirror in the entry below, found by the audit of that change.

  `ui_layout_system` mirrors the ECS hierarchy into taffy (step 3), lets taffy lay the tree out
  (step 4), then walks the tree again accumulating ancestor offsets so `Node.position` ends up in
  absolute window coordinates (step 5) — the field `ui_interaction_system` hit-tests against.
  Once step 3 started **pruning** the graph it mirrored, the two walks stopped agreeing:

  - step 3 claims a shared child at *push* time, so the shallowest reacher wins; step 5 read
    `Children` and deduped at *pop* time, so it could reach that child through a different
    ancestor. `local` is measured by taffy against the parent it actually gave the node, so
    adding it to some other ancestor's origin puts `Node.position` where the box is not. On
    `R.Children = [B, A]` with `A.Children = [B]`, `B` was laid out at the top of `R` but
    hit-tested over `A` — both siblings answering to `A`'s rectangle, and `B` unclickable. Step 5
    now walks the same pruned adjacency step 3 handed taffy, so the two cannot disagree; its own
    cycle guard is gone because that tree is a forest by construction.
  - `set_children` is a no-op for a parent with no layout node, but the pruning descent still let
    such a parent *claim* a child. An unstyled entity whose `Children` names a UI entity — which
    a scene file is free to say — took the child from the styled parent that could have held it,
    leaving it attached to nothing, never laid out, and its `Node` overwritten from taffy's zero
    default: a zero-size box at the window origin. Only tracked entities claim children now.
    Before the descent existed this could not happen, because step 3 iterated styled entities.

- **A cycle in the `Children` graph froze, exhausted or aborted the engine at seven different
  places — and the seven failed in three different ways.** `Parent`/`Children` can loop:
  `HierarchyExt::add_child` refuses to build a cycle, but `Children` is an ordinary component that
  `add_component` writes directly, and `SceneData::instantiate_entities` writes a scene file's
  parent edges verbatim with no cycle rejection anywhere on that path. A rig or a UI tree loaded
  from a file is exactly where one arrives.

  Which walk you hit decided what happened, so no single symptom describes the class:

  - **the two per-frame walks** — `gizmo-physics-rigid`'s compound-collider gather and
    `gizmo-animation`'s target resolve — *pop before they push*, so on the simple cycle A→B→C→A
    their **stack** holds one id for the whole infinite run. Whether *memory* is flat is a
    separate question the loop body answers: the physics one appends a sub-shape per child it
    visits, so the body being assembled swells while the step spins, and the animation one
    queues a boxed command per name-matched visit against a queue nothing drains until the
    schedule flushes — which a walk that never returns never reaches. Measured with the guard
    removed, ten seconds each: a cycle containing a track-named entity peaked at **523 MB and
    climbing**, the same cycle built from nameless entities at **5.4 MB, flat**. And the
    animation one is not a load-path risk that passes: resolution re-runs every frame while
    `target_entities` is empty, and a clip whose track names match nothing keeps it empty
    forever.
  - **the three `gizmo-studio` cascades** — delete, GC, selection highlight — each stepped an
    index cursor along a vector they never drained, so a cycle grew that vector until the process
    was killed. The delete cascade runs on a **Delete keypress** and the selection highlight runs
    **every frame per selected entity**, so selecting the wrong entity was enough.
  - **the two recursive sites** — `gizmo-editor`'s hierarchy panel, and taffy's `compute_root_layout`
    under `gizmo-ui`, which received the cycle because the layout system mirrored every `Children`
    list into the layout tree verbatim — end the process outright: no unwind, no `catch_unwind`,
    no chance to save the scene. In the editor. They do **not** end it the same way, which is
    worth knowing before writing a test for either: run with their guards removed under a 1 GB
    cap, the taffy one printed `fatal runtime error: stack overflow, aborting` and died on
    SIGABRT, while the editor panel never overflowed anything — egui allocates per recursion
    level far faster than it consumes stack, so the allocator killed it first.

  A **diamond** — the same id named by two `Children` lists — is the quieter half of the same
  defect. It terminates on its own, so nothing hangs; it just visits the shared child twice, which
  double-counted the delete cascade's report and added the same child collider to a compound
  rigid body twice, giving the body a doubled sub-shape nobody authored.

  Three of the sites now call one guarded walker, the new
  **`HierarchyExt::descendants_inclusive`** (root first, then breadth-first, cycle-safe and
  duplicate-free); the other four carry an inline `visited` set, because each needs something the
  shared walker cannot give — per-node work interleaved with the descent, a set threaded through a
  recursion, or a mirror rather than a walk. `gizmo-ui`'s mirror now descends from the roots with
  one shared set, which drops the back-edge instead of following it, and clears the child list of
  any styled entity no root reaches (layout nodes outlive a frame, so a list mirrored before the
  cycle appeared would otherwise put the cycle straight back).

  Regression tests at every site. The ones whose pre-fix failure is a hang rather than a wrong
  answer run their body on a worker thread behind a ten-second deadline, because a walk that never
  ends has no assertion to disagree with: the natural test does not fail, it *hangs*, and a suite
  that hangs covers less than one that goes red.

- **A breakable whose entity id had been recycled survived its own destruction — and an explosion
  in the same position re-detonated every frame.** ECS query iteration yields a bare `u32` with no
  generation, and both physics systems rebuilt an `Entity` from it as `Entity::new(id, 0)`. That
  is the right handle only while an id is on its *first* life. `World::despawn` bumps the
  generation and `Entities::reserve_entity` drains the free list before growing the id space, so
  the very next spawn after any despawn lands on a recycled slot at generation 1 — ordinary in any
  game that pools or destroys anything, not an edge case.

  Six sites, three symptoms, and they fail in opposite directions:

  - **the explosion-driven shatter** despawned through the stale handle. `World::despawn` skips a
    handle that fails its liveness check *silently*, so the original stayed in the scene while the
    debris still spawned and `is_broken` latched on it. Every damage path is gated on
    `!is_broken` and nothing ever clears it, so the survivor is **permanently undamageable**: one
    blast leaves a duplicate that gameplay can never remove.
  - **the explosion entity itself** was collected and despawned the same way, so a recycled one
    stayed `is_active` and **re-detonated on every subsequent frame**, re-impulsing bodies and
    re-damaging whatever survived the first pass — against `Explosion`'s own documentation, which
    promises it applies exactly once and then despawns.
  - **the collision path** fails the other way: there the stale handle was rejected by
    `get_mut_entity`, so a recycled-id breakable took **no contact damage at all**. That direction
    corrupts nothing, which is why it could sit unnoticed — an invulnerable crate reads as a
    tuning problem. The comment at that call site claimed the generation check was preventing a
    write into a reused slot; with the generation pinned to 0 it was rejecting every reused slot.

  All six now resolve through `World::entity`, which returns the id's current generation and
  `None` for a dead id — the documented way to turn a raw id into a handle, whose own rustdoc
  warns against fabricating one and notes that this class was already the root of several audit
  bugs. Four regression tests, one per site including the copy-pasted `entity_b` twin, each
  verified by reverting its own site and confirming it is the only test that goes red.

- **A component hook that despawned during `insert_batch`/`remove_batch` corrupted a later group.**
  Both functions compute *every* group up front and then process them one at a time, running each
  group's hooks before the next group starts — and `run_hooks` hands every hook a `&mut World`
  that the hook documentation explicitly describes as free to spawn, despawn and mutate. Nothing
  re-checked the snapshot, so a hook fired for one group could despawn a member of a group that
  had not been touched yet. A despawned entity's location is `EntityLocation::INVALID`, whose row
  is `u32::MAX`.

  Three symptoms, and **only two of them crashed**:

  - the migration branch fed `u32::MAX` to `move_entity_to`, which indexes `self.entities[row]`
    safely — a panic, and the mildest outcome;
  - the same-archetype overwrite branch fed it to `Column::get_ptr`, whose bounds check is a
    `debug_assert` and therefore **absent in release**: a write through a pointer
    `u32::MAX * size_of::<T>()` bytes past the column;
  - and with no crash at all, the target archetype is looked up from `group_entities[0]`, so a
    despawned *first* member made that lookup return `None` and the whole group was `continue`d —
    every live entity in it silently never receiving the component.

  Both functions now re-filter each group against liveness and current archetype before using it,
  which is also the only one of the three that a bounds check at the use site would not have
  fixed. Within a group there was never a window: its hooks run only after all of its entities
  have migrated.

  **The silent symptom is why the tests assert a survivor's write instead of watching for a
  crash.** Group order comes from a `HashMap` and differs run to run, so the first version of the
  overwrite test graded the fix on a coin flip: it passed **7 runs in 20** with the fix removed,
  because in that order the group was skipped rather than corrupted and nothing in the test looked
  at the entity that lost its write. Restructured so both orders fail, all three regression tests
  now fail 20/20 with their fix reverted and pass 20/20 with it in place.

- **`add_bundle` leaked the component it replaced — and the obvious fix was worse than the leak.**
  `Bundle::write_to_archetype` copies raw bytes into the archetype column, so it never dropped
  what it overwrote: every `add_bundle` that re-asserted a component the entity already carried
  leaked whatever the old value owned. A bundle used as a "set these fields" call — the common
  case — leaked once per call.

  Making the write an assignment, so that it drops before it writes, is the fix that suggests
  itself, and it is **unsound**. `Archetype::move_entity_to` extends *every* column of the target
  archetype by one row and fills only the ones the source also had; the rest it leaves
  **uninitialised below `len`**, for the bundle write to fill. So `row < col.len()` does not mean
  "this slot holds a live value" — it means "this slot is inside the column", and from inside the
  write a hole and a live value are the same thing. Assignment drops the hole, which frees a
  garbage pointer: `world.spawn()` followed by `add_bundle(e, (T,))` for any `T` owning a heap
  allocation aborted with `free(): invalid pointer`.

  **The whole suite stayed green under that**, because every component the tests hand to
  `add_bundle` is plain data whose drop glue is a no-op. What caught it was writing a test with a
  `String` in it.

  The distinction is recoverable, but only *before* the migration and only by the caller: a
  bundle component arrives at the new row alive exactly when the old archetype had a column for
  it. `add_bundle` now computes that set first and drops those rows itself — `drop_live_bundle_rows`,
  deduplicated by type, because `BundleExt::with` appends rather than substitutes and a bundle may
  legitimately name one component twice, where dropping one slot twice would be a double free.
  `write_to_archetype` keeps its `ptr::write` and now states the contract it was always relying
  on, and `move_entity_to`'s documentation says that it returns with a hole in the target — the
  omission that made the wrong fix look right.

  One regression test covers all three write shapes, because a fix that satisfies any two of them
  is exactly the fix that was wrong: filling a hole must not drop, replacing in place must, and
  replacing *across a migration* must. Each of the three has its own mutation that turns it red.
  A second test pins the one case that still leaks — a `with`-composed bundle naming the same
  component twice, which `DynamicBundle`'s own rustdoc has always documented as leaking — so that
  closing it later is a decision taken with a red test and an updated doc, rather than a silent
  side effect.

- **`clear_entities` left both of the world's per-entity maps behind, so the next entity spawned
  inherited a stranger's state.** It reset the archetypes, the entity locations and the id
  allocator, and touched neither the sparse-set component storage nor the entity-event listeners.

  Both are keyed by the entity, and `Entities::clear` resets the **generation** counter as well as
  the id counter — so the first entity spawned afterwards is `Entity(0, gen 0)`, the same 64 bits
  as the entity 0 that was just destroyed. That is what separates this from an ordinary `despawn`,
  where the generation bump is precisely what keeps a stale key dead forever.

  The sparse half was invisible in the worst way: nothing dangles and nothing panics, the new
  entity simply *has* a component it was never given. It corrupted the write path too, since
  `add_component`'s sparse branch reads "is this entity already in the set?" to choose
  add-versus-overwrite — so a genuine first attach looked like an overwrite and swallowed
  `on_add`. The listener half is louder: a `World::observe` listener registered against the old
  entity 0 runs for the new one, which the second half of this fix closes.

  `clear_entities` also gained the documentation it never had (its entire doc comment was "Clears
  all entities."). It runs **no hooks** — not `on_remove`, not the despawn hooks — where every
  other destruction path in the crate does, so a game that releases a physics body or a GPU buffer
  from `on_remove` leaks every one of them across a level change. That is a defensible choice
  about teardown cost; what it was not, until now, was a written one.

  Found by the sweep the sparse-set fix asked for. Two siblings from that sweep are recorded in
  `docs/ENGINE.md` §3 rather than fixed here: `World::compact` never shrinks sparse storage (its
  rustdoc, which promised otherwise, is corrected), and a queued `Commands` closure that captured
  an entity handle survives a clear and then lands on that handle's bit-identical replacement
  instead of missing harmlessly.

- **A `Parent` cycle hung the frame instead of ending the walk.** `World::trigger` followed the
  `Parent` chain with no visited set, so a cycle looped forever inside the dispatch. `add_child`
  refuses to build one, but `Parent` is a bare id that `add_component` will write directly, and
  `SceneData::instantiate_entities` writes the parent edges out of a RON file verbatim with no
  cycle rejection anywhere on that path.

  The guard is a `HashSet` allocated per walk and touched only when the event bubbles at all, so a
  targeted event pays for an empty `HashSet::new()` and nothing more.

  **The test fuses itself**, because the failure it guards against is not a wrong answer but the
  absence of one: an unbounded loop has no assertion to disagree with, so the plain version of
  this test does not fail, it hangs — measured, past 60 s before it had to be killed. The listener
  cuts the walk with the engine's own `stop_propagation` once the visit count passes what a
  two-entity cycle can honestly produce, and the overrun then arrives as a number in the assertion
  instead of as a stopped clock. A suite that hangs covers less than one that goes red, which is
  the argument `CLAUDE.md` already makes for `--no-fail-fast`.

  **The comment this fix first carried claimed too much, and is corrected here.** The three
  walkers next door do carry guards — `despawn_recursive`, `collect_hidden`, transform propagation
  — but they are not all of them, and writing "every other walker has one" would have told the
  next reader that an invariant is enforced somewhere when it is not. A sweep found **six**
  unguarded hierarchy walks elsewhere in the workspace: gizmo-physics-rigid's compound-collider
  gather and gizmo-animation's target resolve, both inside per-frame work; three studio cascades;
  and the editor's hierarchy panel, which recurses and so overflows the stack rather than hanging.
  Those are not fixed here; they are recorded in `docs/ENGINE.md` §3, together with two further
  findings from the same sweep — `gizmo-physics-rigid` despawning through fabricated
  generation-0 handles, and `insert_batch`/`remove_batch` not revalidating a group after a hook
  has despawned into it.

- **A nested `trigger` re-ran listeners that had already finished — the guarantee three documents
  promised was false.** `World::trigger` detached an entity's whole listener list once, then put
  each listener *back* as soon as it returned, inside the loop. So by the time the second listener
  ran, the first was live in the map again: a nested `trigger` at that same entity — precisely the
  case the termination guarantee is about — re-entered it.

  With two listeners where the second re-triggers at its own entity, the first ran **twice** for
  one logical event. With *k* such listeners the dispatch performs `2^k − 1` invocations instead of
  `k`; at k = 25 that is 33 million calls, a frame hang from a construct documented as terminating.

  **A test with one listener could never see it**, which is exactly why
  `a_listener_retriggering_its_own_entity_terminates` had been green since the day it was written.
  The put-back now happens after the loop, the way `run_hooks` and `trigger_global` already did it,
  which also fixes a second defect nobody had noticed: a listener registered from inside a dispatch
  used to land *in front of* the listeners that had already run, breaking the documented
  registration order. Two regression tests, one per defect.

  Found by an adversarial review of the observer arc before any of it was released; four of five
  independent review lenses converged on it.

- **`trigger_global` did not save the propagation flag**, so a global listener calling
  `stop_propagation` truncated whatever entity walk it happened to be nested inside. `trigger` had
  the save/restore from the start; the global path was simply left out of it. Same review.

- **⏸ did not reach the audio device.** `PlayLoop::step` is what drives audio and a paused editor
  does not call it, so pausing froze the frame and left the level's ambience playing over it.
  `AudioManager::set_all_paused` is the pause counterpart of `stop_all`, and the studio calls it
  every frame with the editor's own pause state — only a *change* is pushed to the sinks, which is
  what keeps a game's own `pause(id)` from being undone on the next frame.

- **`AudioSource::max_distance` was off by an order of magnitude, because rodio was attenuating
  underneath us.** The engine wrote a linear taper (`1 - d/max`) as the sink's volume, and rodio's
  `Spatial` source multiplies each ear by `min(1/d², 1)` on top of that. The curve anyone actually
  heard was the product, so a source authored to carry 100 m was at **0.9 % of its volume by
  10 m** — and turning `max_distance` up barely moved that, because the term shaping the falloff
  did not contain it.

  `spatial_gain` now cancels rodio's distance term and leaves the engine's taper as the curve, so
  the field means what it has always said it means. Cancelling can only undo an attenuation rodio
  is about to apply, so nothing gets louder than the volume it asked for — pinned by a test that
  sweeps 0–100 m. What survives from rodio is the part that is its job: the left/right difference,
  i.e. the panning, which is untouched because both ears scale by the same factor.

  **Existing games will hear 3D sounds carry further**, which is the fix rather than a side effect;
  a game that tuned around the old curve wants its `max_distance` values divided down.

- **Prefabs recorded no asset identity, so a model that moved broke every prefab built on it.**
  Scenes have been saved and loaded *with identity* since it existed — each asset reference also
  records the UUID of the file it names, so a later load can find that file if it has since moved
  — and the prefab path had none of it: `save_prefab` stamped nothing and `load_prefab` repaired
  nothing. A prefab is the **reusable** unit, so one moved model breaks it everywhere it is
  instantiated; it is the case identity exists for and it was the case that did not get it.

  `save_prefab_with_identity` / `load_prefab_with_identity` are the identity-aware halves, and the
  existing two now delegate to them with `NoAssetIdentity` — the same "one code path, two
  resolvers" shape the scene functions already use, so the two cannot drift. The stamping and
  repairing themselves are now free functions over a slice of entity records, shared by both file
  types for the same reason. A prefab written before today carries no UUIDs and loads exactly as it
  did.

- **Two of `AnalysisConfig`'s four fields were documented as live and were read once.** The type
  said "read every frame, so changing it mid-run takes effect on the next `collect`";
  `metric_history` was read exactly once, when the `MetricStore` was built, and
  `detailed_archetypes` once, when the built-in `EcsCollector` was registered. Turning either up
  in a running game did nothing and reported nothing.

  `metric_history` is live now (`MetricStore::set_capacity`, applied at the top of `collect`), and
  `detailed_archetypes` is documented as the construction-time exception it is — a
  `Box<dyn Collector>` cannot be reached back into, so the collector owns its copy.

  Both ring buffers also trimmed on an `==` boundary, which a ring that is *already* longer than
  its new limit never reaches: **lowering** either number would have stopped nothing and freed
  nothing, and the frame history would have grown without bound from then on. Measured, with the
  fix reverted: 41 snapshots kept past a limit of 5.

- **A `NavAgent` could not be added in the editor and could not be saved — while the inspector
  drew a full section for it.** "AI NavAgent" with max speed, steering force, arrival radius and
  live state has been in the inspector for as long as the component has existed, and there was no
  way to put one on an entity: not in the ➕ menu, not in the add handler, and not in the scene
  registry, so even a scripted agent vanished on save. It is in all three now, with a 🗑 in its
  section like every other addable component.

  What a file keeps is what an author sets — `max_speed`, `steering_force`, `arrival_radius` and
  the target. The path, the cursor into it, the state, the stall detector and the replan schedule
  are `#[serde(skip)]`: they describe a moment in a running simulation, and a file carrying them
  would load an agent that believes it is halfway along a route through a level that has just been
  rebuilt. `NavAgentRecalcState`'s `Default` is written out rather than derived for the same
  reason a derived one would be wrong: `interval: 0.0` means *replan every update*, i.e. a full A*
  query per agent per frame.

- **A scene's music never played: `is_3d = false` meant "not played at all".** The flag was
  meant to choose *how* a source plays — spatial, with a position and a Doppler shift, or flat.
  The spatial system's auto-start requires `is_3d` (correctly: a flat sound has no position to
  update), and nothing else started anything, so authoring music or an ambience track into a level
  produced silence in ▶ and in an exported game. `gizmo_audio::host::play_flat_sources` starts
  them now, and pushes `volume`/`pitch` to a live sink each frame so turning either has an effect
  on a sound that is already playing — the promise the spatial path already kept.

  It lives beside the device rather than beside the spatial system because a flat sound needs no
  camera and no transform: this is the half of scene audio a headless or render-less build gets.
  Proven on hardware by a test whose world contains **no camera at all**.

- **A scene could hold audio and never make a sound.** The editor offers an `AudioSource`, the
  inspector draws it, the scene format saves it — and `AudioManager` was constructed in exactly
  three places in the tree, all of them demos. Neither the editor's ▶ nor an exported game built
  one, so the system that reads those components returned on its first line, and Lua's
  `audio.play` took the same exit. The API existed end to end except for the end.

  `PlayLoop::step` now runs `systems::audio::audio_frame`, and the device half lives in
  `gizmo_audio::host` so the script path — which exists in builds with no renderer — shares it.
  The device is opened **only when the world has something to play** (an `AudioSource`, or a
  script that just asked), because opening one is audible on some backends and a game without
  audio should not hold it; and **only once**, because a machine with no device still has none
  next frame. That failure is latched in `AudioLoadState`, which also latches per-name load
  failures — otherwise a missing file is a warning per frame at 60 Hz.

  `AudioSource::sound_name` is now resolved **as a path** when nothing has registered that name,
  exactly as `MeshSource` is a path: a source saying `demo/assets/audio/engine.wav` plays that
  file with no game code at all. A name the game loaded itself is never re-read — embedded bytes,
  a wasm `fetch`, a differently-named file all win over anything on disk.

  Two more in the same corner. **⏹ now stops the sounds**: a snapshot restores entities, and the
  sinks are not entities — they live on the device behind a resource, so stopping a game used to
  leave its looping ambience playing over the editor for the rest of the session
  (`AudioManager::stop_all`). And **`AudioSource::_internal_sink_id` is no longer serialised**:
  saving a scene while playing wrote a live sink id into the file, and a reloaded *looping* source
  carrying one never started again — the spatial system only clears a stale id for a one-shot.

  Verified on real hardware, not only in the type system: two `#[ignore]`d tests play
  `demo/assets/audio/engine.wav` through the whole path and assert a live sink, and that ⏹
  silences a looping one.

- **No `NavAgent` the engine ever created moved, and the one path that could have moved them hung
  on a ground plane.** Two gaps stacked: `PlayLoop` — the editor's ▶ and every exported game —
  never ran the navigation systems, so agents were steered only by `gizmo-studio`'s update hook,
  i.e. only while the editor sat *stopped*; and underneath that, **nothing in the workspace ever
  constructed a `NavGrid`**, which `ai_navigation_system` returns on its first line without. Lua
  could ask for navigation (`ai.add_nav_agent`, `ai.set_target`) and the grid those need had no
  way into a world at all.

  `system::ai_frame` is now the one entry point — behaviour trees, then the grid, then the
  navmesh rebuild, then steering, in the order that works — and both hosts call it, with the
  editor's call on the same `editor_owns_the_physics_step` gate as its physics step so ▶ does not
  steer every agent twice.

  Three supporting changes, each measured rather than guessed. **A half-space `Plane` is no longer
  rasterised into obstacles:** its AABB is a ±10 km broadphase sentinel, so a rebuild that met a
  ground plane did ~8·10¹² cell insertions (measured: the regression test does not fail against the
  old code, it does not return — 90 s timeout, `EXIT=124`), and had it finished, every cell of
  every layer would have been an obstacle. The floor is not an obstacle; a half-space used as a
  *wall* is not one either, so model those as boxes. **Rasterisation is clipped to the grid**, which
  bounds a rebuild by the grid's size instead of the geometry's. And **`NavGrid` gained an
  `origin`** (`centred_on`, `fitted_to`): the grid used to start at the world origin, so the
  navigable area was the positive quadrant only and a scene laid out around the origin — the
  editor's default scene, every demo — had agents permanently out of bounds. `NavGrid::new` still
  leaves it at zero, so nothing existing moves.

  The studio's *Rebuild NavMesh* button used to answer "NavGrid bulunamadı! AI aktif mi?"; the
  honest answer was "no, nowhere". It now builds one fitted to the static geometry and says so.

- **The analyzer counted physics steps as frames.** `AnalysisPlugin` registered its collector on
  `AppParts::schedule` — the **fixed-timestep** schedule, which a rendered frame runs `0..N` times.
  So a frame that stepped twice produced two samples and a frame that stepped none produced zero,
  and every per-frame number the analysis panel and the JSON/CSV export reported was a count of
  *steps*. The collector is on `update_schedule` now, which runs exactly once per frame — what
  "collect every frame" always meant. The docs on the way in were stale too: they said
  `Schedule::run` calls `FrameProfiler::end_frame`, which stopped being true when the frame
  boundary moved out to the runtime loops that own a frame.

  The guard is a source-shape one (a behavioural version needs a runtime loop), and it exposed a
  second hole while being written: **`gizmo-analysis/app` is enabled by nothing in a workspace
  build**, so `cargo test --workspace` never compiled `plugin.rs`'s tests at all, and the `lint`
  job's `--all-features` only type-checks them. CI now runs `cargo test -p gizmo-analysis
  --features app` as its own step, next to the other default-off feature seals.

- **The studio could add a `FighterController` but not remove one, and never showed what the fight
  clock was doing to it.** Four inspector sections draw a 🗑 button; `scene_ops` had removal arms
  for three, so pressing Delete on a fighter logged "Component turu silinemiyor" at the user and
  left the component in place. The four are now pinned by one test together — the defect was an
  asymmetry between a list of buttons and a list of match arms, which is exactly what a
  per-component test misses.

  Two more in the same corner. **Adding two fighters produced two player 1s** (`default()` is
  always slot 1), and the studio's own fight HUD needs a p1 *and* a p2 before it draws anything —
  so authoring a versus scene the obvious way produced a HUD that never appeared; the add path now
  hands out the lowest free slot and reuses one that is freed. And **⏸ wiped the HUD**: the sync
  block asked `is_playing()`, which is false while paused, so every paused frame fell into the
  reset branch and the health bars vanished and the round timer snapped back to 99 under a pause
  overlay that was still being drawn. It asks `is_in_play_session()` now; only the countdown stops.

  The inspector's fighter section also draws the live half at last — the move in flight with its
  frame index, total and phase, the hitstop/hitstun counters and the lock — read-only, because
  `fighter_frame_system` rewrites them every fixed step. It had been drawing six authoring fields
  and not one of them, while its own documentation promised "the frame data of the current move",
  and it never drew `max_health` either: the denominator of the bar the fight HUD paints.

- **`PhysicsTime::alpha` promised an interpolation the engine cannot do, and now says so.**
  `gizmo_app::frame`'s documentation said an update system "can read `PhysicsTime::alpha` to
  interpolate between the last two simulated states" — the engine keeps **one**. There is no
  previous `Transform` anywhere in the workspace, so the renderer draws the pose the last fixed
  step left behind and holds it. Both alphas' docs now say what they are for, and cross-reference
  each other: `PhysicsTime::alpha` is the leftover of the caller's fixed step (60 Hz on the
  `App`/`PlayLoop` path), `PhysicsWorld::render_alpha` the leftover of `gizmo-physics-rigid`'s own
  240 Hz sub-step accumulator — two embedding levels, not two copies.

  What that costs is now measured and pinned in `gizmo-core`'s tests rather than argued about: at
  60 Hz physics on a 60 Hz display, nothing (every pose drawn exactly once, alpha never leaves
  0.0). At 144 Hz each pose is held for **two or three** frames in an irregular 3,2,3,2,3,2,2…
  pattern, so a body at constant speed appears to advance by different amounts on neighbouring
  frames; at 300 Hz the hold is a uniform 5 and there is no beat. And `alpha * fixed_dt` is how
  stale the drawn pose is — up to 16.7 ms at 60 Hz, which at 10 m/s is 16.7 cm, sawtoothing sixty
  times a second. Nothing was deleted and no behaviour changed; engine-side interpolation is a
  scoped follow-up with its trigger written down in `docs/ENGINE.md`.

- **A hitstop applied from Lua lasted forever, and no attack ever reached its hitting frames.**
  `FighterController` is a frame-counting state machine and **nothing in the engine counted its
  frames**: every write to `hitstop_frames`, `hitstun_frames` and `current_move_frame` anywhere in
  the workspace was a wholesale assignment — not one `+=`, `-=` or `saturating_*` existed. So
  `fighter.apply_hitstop(id, 6)` froze that fighter for the rest of the process rather than for six
  frames, `fighter.set_move` started a move that stayed on frame 0, and `is_in_active_window` — the
  function that says "this attack is hitting right now" — had never once answered `true` in this
  engine (it had no callers at all).

  The subsystem had every other half: the studio adds the component and draws a fight HUD from it,
  the inspector edits it, the scene format serialises it, and three Lua calls write it. The clock
  was written once (`334d6ed`) and deleted two days later by a refactor (`592bd6f`); its call site
  survived as a comment until `9cbdddf` swept that too. The engine's own default path — `PlayLoop`,
  which is both the editor's ▶ and every exported game — therefore ran a fighting API over state
  nothing produced.

  Regression tests drive the whole chain from a `.lua` file: a script's three-frame hitstop now
  ends after exactly three frames, and a 5/3/2 jab hits on exactly frames 5, 6 and 7 and is over
  after 10. Without the clock those two assertions read `[1,2,3,4,5]` (locked forever) and `[]`.

- **The underwater muffle did nothing to 3D sound, and made everything 2.5× too loud on the way
  out.** `AudioManager::set_underwater` multiplied every sink's volume by `0.4` and undid it with
  `2.5`, while `audio_spatial_system` overwrote that same volume with `attenuation × source.volume`
  every frame. Measured on a real device: a 3D sound went `1.00 → 0.40` on submerging and back to
  `1.00` **on the next frame** — so the muffle was unreachable for every 3D sound in the engine —
  and surfacing then multiplied by 2.5 regardless, i.e. 250 % of the volume the game asked for, out
  of a speaker. A 2D sound whose volume was set to `0.22` while submerged surfaced at `0.55`. The
  known symptom had been recorded as "a slight drift on surfacing".

  Volume and speed are composed from the mixer now rather than accumulated in the sink, so the
  order the modifiers arrive in cannot matter. Behaviour is otherwise unchanged: the same `0.4`
  turn-down and `0.85` slow-down, still idempotent, still applied to sounds that start while
  submerged.

- **A pitch of zero could still assert on the audio thread.** `sanitize_playback_speed` clamps to
  `0.01`, but the underwater slow-down multiplied *after* it: `0.01 × 0.85 = 0.0085`, small enough
  for rodio's `SampleRateConverter` to compute a source rate of 0 and trip its `from >= 1` assert —
  on the cpal callback thread, which takes playback with it. The clamp is applied after the
  environment multiply now, where the final number is.

- **Playing a sound leaked a routing entry per sound, forever.** The sink garbage collector dropped
  dead sinks but nothing dropped what the mixer knew about them; `clean_dead_sinks` now retires
  both together.

- **A Lua script could not drive a car, and could not set a field of view.** The same scan that
  found the silent audio API enumerated the whole command vocabulary: of `ScriptCommand`'s 42
  variants, 22 are applied inside the scripting crate and 20 came back to a host that dropped them.
  `PlayLoop` now answers seven — the three audio calls, the three vehicle calls
  (`vehicle.set_engine_force` / `set_steering` / `set_brake`) and `SetCameraFov`. The remaining
  thirteen are listed in docs/ENGINE.md with what each is waiting for.

  Two of the new ones carried **unit traps** that a straight assignment would have failed silently:
  `SetVehicleEngineForce` documents "negative drives it backwards" while `VehicleController::
  throttle_input` documents that a negative value is *not* reverse — so the naive mapping would have
  driven the car forwards at full throttle. It engages reverse instead. And `SetCameraFov` is
  documented in degrees while `Camera::fov` is radians, so a script asking for 60 would have got 60
  radians, unremarked.

- **A Lua script's `audio.play` made no sound — anywhere.** `audio.play` / `play_3d` / `stop` queue
  a `ScriptCommand`, and `ScriptEngine::flush_commands` returns the ones it cannot apply itself (the
  scripting crate does not depend on the audio subsystem). Both call sites in `PlayLoop` discarded
  that return value with `let _unhandled = …`, and no other consumer existed in the workspace — so
  the whole Lua audio API was a no-op in the editor's Play mode and in every exported game, while a
  unit test asserted that the command reached the queue.

  `PlayLoop` now answers them: `play`, `play_3d` against the same listener the spatial audio system
  uses, and `stop` through the new `AudioManager::stop_by_name`, which stops every live sound
  started from a given name — a sink id is what engine code holds, but a *name* is all a script has.
  Scene, dialogue, race and camera commands are still handed back deliberately: the editor must not
  switch scenes under the author.

- **A force or torque applied to a sleeping body did nothing at all.** `RigidBody::force_accumulator`
  and `torque_accumulator` are the public fields a game writes for a continuous push — a thruster, a
  wind volume, a conveyor — and the integrator returns early on `is_sleeping`, so the accumulator was
  never read; nothing woke the body either, because the sleep test looks only at velocity and a force
  is not one. Measured: a 1 kg box settled on a plate, given 50 N sideways for two seconds, moved
  **0.0000 m**; with a manual `wake_up()` it moved 92.7 m. So a continuous force stopped working the
  moment its target came to rest.

  `PhysicsWorld::step` now wakes any dynamic sleeping body whose force or torque accumulator is
  non-zero, which is the contract `apply_impulse`, `apply_force` and the explosion system already
  kept. Determinism unchanged (`A462C9EB8A09D5CA`).

- **Ten unused imports, a dead static and an unused binding, all in feature combinations nothing
  linted.** CI's lint job runs `--all-features`, so it never sees code a *smaller* feature set
  removes, and the feature-powerset job compiled those combinations with `cargo hack check`, which
  does not deny warnings. Asked with `-D warnings` for the first time, **66 of the facade's 150
  feature combinations failed** — ten defects in `crates/gizmo/src/bundles.rs` and
  `crates/gizmo/src/systems/physics.rs`, each repeated across the combinations that expose it, one
  of them a `#[cfg]` attached to the line below the import it was meant to gate. All fixed, and
  the powerset job now runs `clippy -- -D warnings` so the configuration cannot rot again (+20 %:
  49 s → 59 s over the 150 combinations, dependencies cached).

- **`gizmo-audio` was the one crate not on the `missing_docs` ratchet** (19 of 20 carried
  `#![warn(missing_docs)]`), while `docs/ENGINE.md` said the backlog was closed. The crate was
  already at zero, so the fix is one line and no documentation — but a crate at zero with no lint
  is a state, not a ratchet.

- **`PhysicsWorld::raycast_all` ignored `max_distance`.** It bounded nothing, so a body whose
  bounding box reached into range while its actual shape sat outside was returned anyway — the
  broadphase's limit leaking out as if it were the query's. A 1×1×20 box turned 45° and centred
  15 m away, queried at 10 m: `raycast` correctly answered `None`, `raycast_all` answered a hit at
  15.29 m. Now bounded by the surface distance, inclusively.

- **Applied forces were frame-rate dependent, and by a large factor.** The physics world runs fixed
  1/240 s substeps; the integrator drained `RigidBody::force_accumulator` on the **first substep**
  of each frame, so a body received `F·(1/240)` of impulse per frame instead of `F·frame_dt`. The
  same 10 N push on a 1 kg body for one second reached 9.95 m/s at 240 fps, 4.97 at 120, 2.49 at 60
  and 1.24 at 30 — the acceleration halving with the frame rate. Forces are drained once per frame
  now; every substep integrates `F·substep_dt` and the sum is `F·frame_dt`.

  `PhysicsWorld::apply_force` was never affected (it writes the velocity directly with a
  caller-supplied `dt`), and nothing in the workspace wrote to the accumulator — which is why this
  survived. The determinism hash is unchanged.

  It also removed most of the friction creep documented in docs/ENGINE.md §3: a box held at 99 % of
  its static limit drifted 15.2 mm over 200 s before and **0.010 mm** after, because the old drain
  delivered each frame's force as a kick that tripped the sliding branch of the friction cone four
  times a second.

- **`AssetManager::normalize_path` doubled the leading separator of an absolute path**
  (`/tmp/x` → `//tmp/x`). Lookups matched anyway, since both sides normalise, so the damage was to
  the path handed *out* by `get_path` — tolerated on Linux, a UNC prefix on Windows. `./demo/x` and
  `demo/x` also keyed the same file twice, which would have given one asset two identities.

### Removed

- **`Camera2D`, a second camera type for a capability `Camera` already has.** No draw path read
  it, and the trace that flagged it assumed wiring it meant writing a 2D pipeline. It does not:
  `Camera` has `ProjectionMode::Orthographic { height }`, builds a real `Mat4::orthographic_rh`,
  and the studio has a shortcut that toggles it — orthographic viewing works today. So this was not
  a type waiting for a feature but a second, unread implementation of one that exists, with a zoom
  where the other has a height.

  Safe to remove: four references (the scene registry, two re-export lists, half of one test), no
  reader anywhere, and the scene loader is infallible by design — an unknown component name is
  logged and skipped, so a scene saved with one still loads, without it. **Trigger to bring it
  back:** a real 2D pipeline — sprites, z-ordering, pixel snapping. A dedicated camera earns its
  place then; today `Camera` in orthographic mode draws the same picture.

- **`ScriptCommand::SetFightCamera`, dead at both ends.** Unlike the other open commands — a script
  can ask for those and the engine has not answered yet — this one could not be asked for either:
  no `api_*.rs` pushed it, and no arm matched it (it fell into `flush_commands`' catch-all, and the
  host's camera pass answers only `SetCameraFov`). Wiring it would have meant writing the
  camera-follow subsystem the engine does not ship, and a live replacement already runs in
  `gizmo-studio`, whose auto fight camera derives all four of the command's fields itself.

  **Trigger to bring it back:** the engine shipping a camera-follow system a script can point at,
  plus a game that wants to parameterise the framing. The first step that day is moving the
  studio's framing maths (midpoint, `separation.max(2.0)`, `(separation * 1.2).max(4.0)`, height
  1.8) into the engine — then the command drives something that exists, and the studio's copy goes
  with it.

- **`ScriptEngine::get_pending_audio_scene_commands` — a public method that returned an empty
  `Vec` and could not return anything else.** Its body was `Vec::new()` under two comments
  wondering when it should be called; its doc said it "returns the audio/scene commands pending at
  runtime". Nothing in the workspace called it. The commands it promised are the ones
  `flush_commands` hands back, and those now have a real consumer in `PlayLoop`.

- **`ScriptEngine::run_entity_update`, `ScriptContext` and `ScriptResult` — the second per-entity
  script protocol, whose last caller was deleted in `0de4bee`.** It marshalled a `ctx` table
  (position, velocity and nine hard-coded key flags) into Lua and lifted a `{position, velocity}`
  table back out, leaving the caller to apply it. The live protocol is `update_entity` →
  `on_entity_update(entity_id, dt, props)` with effects going through the command queue, which is
  what the editor's Play mode and every exported game run. Two protocols meant the next wiring
  could pick the dead one; the hard-coded `key_w`/`key_a`/… also duplicated the `input` table with
  a fixed set of keys, and *its* caller would have had to fill them by hand.

- **`gizmo_scripting::dummy_engine` and the crate's whole `cfg(target_arch = "wasm32")` arm.** It
  could not be compiled in any configuration: `cargo check -p gizmo-scripting --target
  wasm32-unknown-unknown` fails inside mlua-sys's build script ("don't know how to build Lua for
  wasm32-unknown-unknown"), and both consumers (`gizmo-app`, `gizmo-editor`) list this crate under
  `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`, so `cargo tree -p demo-web --target
  wasm32-unknown-unknown` shows it zero times. The drift proves it never compiled: the stub's
  `update_entity` took three arguments where the real one takes four, under a comment promising
  that identical signatures let calling code compile unchanged on both targets. Web builds are
  unaffected — the fallback that actually runs there is `gizmo-editor`'s per-item one, which *is*
  linted for wasm.

### Changed

- **`post_process::create_blur_buffers` returns the vertical step's buffer too** —
  `(h_buffer, v_buffer, h_group, v_group)` — and `PostProcessState` keeps it as
  `blur_v_params_buffer`. `blur_params_buffer`'s documentation said it held both steps; it only
  ever held the horizontal one, which is why nothing could rewrite the vertical step.

- **`Material` and `MaterialDesc` gained `blend_mode`**: a struct literal of either needs the
  field (`BlendMode::Alpha` is the old behaviour; `Material::new` and `..Default` paths are
  unaffected).

- **The editor draws unlit transparent materials with the unlit pipeline**, as the game always
  has. Its transparent loop bound the lit `transparent_pipeline` for every transparent batch, so
  an unlit, half-transparent material was shaded by lights in the viewport and not in the build.

- **`SpotLight` is no longer `Copy`**: it can name a cookie (`cookie: Option<String>`). `.clone()`
  where a copy was taken; nothing in the workspace needed one. `SceneSetupInputs` gained
  `light_cookies` and `collect_scene_lights` a `&CookieTable` — pass
  `renderer.light_cookies.table()`, or `Default::default()` for no cookies.

- **`FighterInputBuffer::check_combo_strict` matches a press edge only — holding the buttons is no
  longer performing the motion.** It used to accept `just_pressed || pressed`, which is not a
  looser reading of a motion but the end of order checking: a player merely *holding* Down, Right
  and Punch has all three in `pressed` on every frame of the buffer, so any order of them completes
  within three consecutive frames — the quarter circle, its reverse, anything. `max_gap` stopped
  meaning anything too, since a held button matches on every frame it is held. The function's own
  comment already said the safe rule was `just_pressed`; the code did the other thing.

  Real motions are unaffected — they are made of press edges, and the d-pad quarter-circle test
  that drives a real `ActionMap` through `update` still passes untouched. What is lost is a step
  whose edge fell outside the buffer's window, i.e. a button already held before recording began.

  This also closes a divergence nobody could see: the Lua `fighter.check_combo` has always matched
  press edges only, so the two implementations of one concept answered differently, and every combo
  test in the workspace drove one or the other, never both. A new cross-check test in
  `gizmo-scripting` runs seven scenarios through **both** and asserts the same answer.

- **Four source-shape guards were vacuous and now are not.** A positive `contains` over source
  text is satisfied by a comment, and each of these tests exists because a line was once deleted or
  made inert — the one state a comment cannot be told apart from. Commenting the guarded line out
  left `both_paths_size_the_instance_buffer_for_the_frame`,
  `the_gizmo_config_still_assigns_snapping`, `the_descriptor_is_sized_from_the_target_not_the_window`
  and `shader_shadow_fade_matches_the_rust_mirror` all green, with the defect each names reachable
  underneath. They cut comments before matching now, and each was re-broken to watch it fail.
- **A source ratchet keeps movement going through one function.**
  `crates/gizmo/tests/movement_input.rs` fails the build if anything under `crates/*/src` or
  `demo/src` reads a movement key without the shared blend. Its exception list documents the nine
  files that read movement-named keys for something else: throttle, turret aim, a dial, editor tool
  modes, a fighting-game binding table, and the platform layer's logical-key fallback.
- **Demos with a vertical movement axis (Q/E, Space/Ctrl) now clamp their movement vector to
  length 1 instead of normalising it.** Identical for keys — any non-empty key combination is
  already at least unit length — and a normalise would push a half-tilted stick back up to full
  speed, which removes the one thing a stick adds over a key.
- **The light ceiling is 32, and off-screen lights no longer take slots (`MAX_LIGHTS` 10 → 32).**
  The scene block grows to 2576 bytes (the uniform-binding floor is 64 KiB everywhere WebGPU runs)
  and costs nothing per frame — both lighting loops run to `num_lights`, so a scene with three
  lights pays for three. Lights whose sphere of influence misses the camera frustum are now culled
  before the cap is applied, so a level's worth of lights behind the player cannot crowd out the
  ones lighting the view. Going much past 32 needs clustered/tiled culling, which is not in this
  release.

### Fixed

- **A crate resting on a slope no longer runs away — the friction cone was choosing its
  coefficient from the wrong quantity.** With the default material (`μ_s` 0.6, `μ_d` 0.5) the
  friction angle is `atan(0.6)` = 30.96°, so a body at rest on anything shallower must stay put.
  It did not: measured on a 1 kg crate over 10 s, a 28° slope produced **26 m of slide and the
  crate leaving the plate**, and a 30° slope **77 m, still accelerating at 27 m/s**. Below
  `atan(μ_d)` = 26.57° the same fault showed as creep instead (2.8 mm at 25°, and 107 mm over
  200 s for a box held at 99 % of its static limit on the flat).

  The coefficients were never the problem: setting `μ_d = μ_s` made every slope hold, which is
  what identified the *transition* as the fault. The cone clamped to `μ_d·λ_n` whenever the
  **demanded** impulse exceeded `μ_s·λ_n` on a sweep — but `λ_n` fluctuates between sweeps and
  substeps, so a contact standing perfectly still was intermittently charged the dynamic rate and
  lost the `(μ_s − μ_d)·λ_n` of hold it was entitled to. Above the dynamic friction angle that
  slip feeds itself.

  The budget now comes from the contact's **actual tangential speed** (the model PhysX uses):
  static below `ConstraintSolver::static_friction_velocity_threshold` (new, default 1 cm/s),
  dynamic above it. All five friction solves in the crate — TGS sweep, block sweep, island sweep,
  the SI path and the standalone one-shot — went through five copies of the same four lines and
  now call one `friction_limit` helper.

  After: 28° slides 6.7 mm and stops, 30° slides 9.6 mm and stops, and flat-ground creep improves
  44× at 90 % of the limit and 7.5× at 99 %. Gated by
  `soak_and_golden::a_crate_holds_on_a_slope_below_the_friction_angle`. **Determinism is
  unchanged** (`headless_stress_test`, three matching hashes, `A462C9EB8A09D5CA`): its scenario is
  a 2000-box collapse whose contacts slide far above the threshold, so it runs the dynamic branch
  throughout. No golden scene needed re-blessing.

### Changed

- **Joints warm-start by default (`JointSolver::warm_start_factor` 0.0 → 0.5) — a physics
  behaviour change.** The mechanism (a λ-injection sweep before the iteration loop) already
  existed and was shipped disabled while the number was an open question. Measured on a 16-link
  chain at 10 iterations, settled: tip constraint error 0.01249 → 0.00881 m at a 20 kg tip and
  0.10359 → 0.06254 m at 200 kg. The injection sweep costs about one iteration and buys roughly
  four times what an extra plain iteration buys. It is a half rather than a whole because past
  that it buys jitter: at 1.0 the 200 kg chain's residual `max|v|` reaches 0.19 m/s while the
  error stops improving. Ordinary mass ratios pay nothing — residual velocity unchanged at 1e-4.

  Two golden joint scenes were re-blessed; the sharpest instrument in them moved the right way
  (arm error ×1000: 0.415 → 0.320 on the soft path, 0.847 → 0.487 on the legacy Baumgarte lever).
  Old → new values are recorded line by line in `golden_state.rs`. Determinism is unaffected
  (`headless_stress_test`: three matching hashes).

### Added

- **`Renderer::headless_device()` and `Renderer::new_headless_with_device(..)`** — build several
  headless renderers on one `wgpu::Device`. `new_headless` is now a thin wrapper over the pair and
  behaves exactly as before. This is what the test suites needed: a renderer per test (they carry
  TAA/GI history between frames, so sharing one breaks the "single frame from a clean state" the
  golden tests are written against), but *not* a device per test — one device per renderer is what
  made the GPU tests flaky at workspace scale and what makes a long sweep die outright
  (`radv/amdgpu: Not enough memory for command submission`, measured at ~17 devices).

### Removed

- **The GPU mesh-culling subsystem, which never ran.** `gizmo_renderer::gpu_cull` — `GpuCullState`,
  `MeshBoundsRaw`, `DrawIndirectArgs`, `WebProfile::gpu_cull_enabled` and `mesh_cull.wgsl` — was
  constructed for every renderer (a compute pipeline, three buffers, a bind group) and called by
  nothing: the render path had removed the pass itself, with the reason left in a comment there
  (`// GPU cull pass removed since we use CPU instancing`). CPU frustum culling is the live path
  and is unchanged. Bringing GPU culling back needs the *draw* side — indirect draws in the batch
  path, bounds uploaded per frame — and, by this project's own rule for optimisations, a
  measurement of what CPU culling actually costs first. The code is in the history.

### Changed

- **`Plugin` no longer names a runtime, so both runtimes can take plugins.** `Plugin::build` took
  `&mut App<State>`, and `App` is whichever runtime the feature flags selected — but the two
  runtimes coexist, so in any `window` + `render` build `headless::App::add_plugin` was `#[cfg]`-ed
  out of existence and a headless simulation inside a graphical application had to register every
  system by hand. `build` now takes `&mut dyn AppLike`, whose single method hands back the three
  things every plugin in the tree actually touched — world, fixed schedule, update schedule — as
  disjoint borrows (`AppParts`), which is what a plugin had with direct field access.

  `Plugin`'s `State` parameter is gone with it: no `build` body used it, so thirteen impls went
  from `impl<State: 'static> Plugin<State> for X` to `impl Plugin for X`. A plugin that needs more
  than `AppLike` — replacing the runner, say — is not a plugin: take the concrete `App` in an
  ordinary function (`cradle`'s headless server now does).

- **Studio's Build/Export ships your scene, not the engine's sample.** It built `demo` and copied
  that crate's default binary — `3d_scene`, a fixed floor/cube/light/camera that opens no
  scene file and runs no script — then copied the project's `scenes/` and `scripts/` next to it,
  where nothing ever read them, and said "Oyununuz hazır". It now builds `gizmo_runtime` (new, in
  `demo/src/bin/`), writes the world the editor is showing to `export/gizmo_game/scenes/main.scene`,
  and only claims the game is ready when the scene actually landed.

  The runtime's contract is the editor's Play mode, and not by resemblance: both drive the same
  `gizmo::systems::PlayLoop` — one accumulator, one step size, one script order — with only the
  reporting injected, because a broken script is a console line in the editor and a stderr line in
  a shipped game. "The exported game does what the editor showed you" is therefore not a promise
  to keep but a thing with nothing to drift from. (`PlayLoop` is new public API on the facade,
  behind the existing `physics` feature.) An exported directory is self-contained, so the
  runtime makes it the working directory and the editor's relative paths keep meaning what they
  meant; with no argument it opens `scenes/main.scene`.

### Fixed

- **A decal placed in the editor is visible in the editor.** Decals blend into the deferred
  G-buffer's albedo target, and the editor draws forward and fills no G-buffer, so a decal was
  invisible until the game ran — the author saw an empty floor and the shipped game showed the
  splatter. There is a forward decal pass now (`gizmo::systems::render::record_forward_decals`,
  `decal_forward.wgsl`): it reconstructs the surface position from the depth buffer the forward
  pass already writes and alpha-blends into the lit image, with the same volume test, projection
  and fade as the deferred one. Both passes read the world through one shared collector, so they
  cannot disagree about where a decal is.

- **Two of the engine's own assets said the wrong thing when they failed to load.** A scene from
  before components were stored as strings (`perfect_car.scene`) failed at the parser with
  `10:28: Expected string` — before the version machinery could say anything — and now fails as
  `SceneError::LegacyComponentEncoding`, which names the format and says what to do. A prefab that
  was not text at all (`prefab_8.prefab`, binary) reported "scene file I/O error" for a file that
  was present and readable; the not-UTF-8 case is now reported as the wrong-format failure it is,
  and that prefab — which every export copied — is deleted.

- **A per-entity script's commands land in the frame that issued them.** `entity.set_position` and
  its neighbours queue a command rather than writing the world, and the play loop flushed that
  queue *before* running the per-entity `on_entity_update` hooks — so everything those hooks asked
  for waited for the next frame's flush. Measured: an entity whose script set its position on
  frame 1 was still at the origin at the end of frame 1 and moved at the end of frame 2. Every
  per-entity script in the engine carried a frame of latency, in the editor and in exported games
  alike, and nothing caught it because the movement did happen — just late.

- **A windowed app no longer dies on a frame it could not present.** The egui frame runs at the
  top of the loop; when the swapchain image could not be acquired afterwards — an outdated
  surface, a resize in flight, and the first frame of a freshly mapped window — the loop returned
  early and dropped that frame's `egui::FullOutput`. egui hands each texture delta over exactly
  once, so this was not a skipped frame but a lost upload: debug builds died on epaint's
  `debug_assert!` (`cargo run -p demo --bin advanced_physics` panicked about a second after
  launch, and so did every other windowed binary), and release builds lost whatever that frame
  carried — a font atlas rebuilt on a DPI change never reaching the GPU means glyphs paint as
  blank boxes for the rest of the run. `EguiContext::render` had the same defect on the painted
  path: it applied the deltas by reference and never cleared them.

  A skipped frame now skips the pixels, not the uploads (`EguiContext::absorb_unpainted_frame`).

- **Screen-space reflections and screen-space GI reach the frame at all.** Both shaders tested the
  G-buffer's written-flag with a strict `> 0.5`, while `gbuffer.wgsl` packs that flag as
  `(0.5 + 0.49·anisotropy) + floor(100·subsurface)` — exactly `0.5` for an ordinary material, and
  exactly representable in the `Rgba16Float` target it is stored in. Every ray-march hit candidate
  was therefore rejected, both passes returned black, and their additive apply added nothing: a
  frame rendered with SSR and SSGI running was byte-identical to one rendered with the passes
  removed, on every scene measured. The entry gates in the same two shaders had always used the
  inclusive form, as do the other six readers of that flag; only the inner hit test disagreed.

  Games drawing through `default_render_pass` now get both effects (measured on a mirror floor at
  128×128: SSGI moves 12.2 % of pixels, SSR 2.6 %). `SimpleApp` and `with_scene_render` still
  switch both off deliberately, and the editor viewport never recorded them. Guarded by
  `screen_space_reflections_and_gi_reach_the_frame`.

### Changed

- **BREAKING — `PostProcessSettings` keeps only the editor's own two viewport toggles.** The nine
  graded fields (`bloom_intensity`, `bloom_threshold`, `exposure`, `vignette`,
  `chromatic_aberration`, `dof_focus_dist`, `dof_focus_range`, `dof_blur_size`, `film_grain`) are
  gone from `gizmo_editor::editor_state::PostProcessSettings`; what remains is `fxaa_enabled`,
  `ssao_enabled` and `ssao_strength`.

  They had to move because the struct is *editor state*: nothing wrote it to a file, its only
  reader was the editor viewport, and the engine's frame read a second unrelated copy off the
  `Renderer`. A look an author tuned was therefore gone at the next launch and absent from every
  exported build — while the panel editing it said in as many words that these were the *scene's*
  settings. The look is now `gizmo_renderer::components::PostProcess`, a component on the camera,
  and exposure stayed on `Camera::exposure` where it already round-tripped. Leaving the fields
  behind would have left sliders writing values nobody reads, which is the defect itself.

  **Migration:** read and write the camera's `PostProcess` (add one if the camera has none — its
  default is the engine's no-component behaviour, so adding it changes nothing) and use
  `Camera::exposure` for exposure. `PostProcess::clamped` carries the ranges
  `EditorState::validate_post_process` used to enforce.

- **BREAKING — `inspector::environment::draw_environment_settings` takes a `&World`.** It edits the
  active camera's components now rather than editor state, so it needs the world the camera is in.

- **BREAKING — `EditorState::validate_post_process` clamps only `ssao_strength`.** The rest of the
  clamping moved to `PostProcess::clamped` with the fields it applies to. It had to move rather
  than stay: a scene file has no slider bounds, so a rule that only ran in a panel would not cover
  a hand-written or generated scene.

- **An export ships `demo/assets/` under its own name.** It used to be copied to `assets/`, which
  broke every reference into it (see *Fixed*). Packages built before this change have their asset
  tree in the old place; rebuild rather than moving it by hand, because the scene inside the
  package names the new one.

- **A collider authored in the ECS reaches the solver whole.** `is_trigger` and `collision_layer`
  now survive the per-frame rebuild in `physics_step_system` (see *Fixed*). A game that set either
  field and had learned to work around it being ignored will see different physics: triggers stop
  pushing and start reporting, and layer filtering starts filtering.

- **`IsHidden` hides on the engine's draw path too**, not only in the editor viewport (see
  *Fixed*). A game that added the marker and still expected the entity drawn will now see it
  disappear — which is what the marker's documentation has always described.

### Added

- **`gizmo_renderer::components::PostProcess`** — the authored look (bloom, vignette, chromatic
  aberration, depth of field, film grain) as a component on the camera that renders it, registered
  in `full_scene_registry` so it round-trips with the scene. Its default is exactly what a camera
  without it gets, `dof_blur_size` included (0.0, because the renderer's `dof_enabled` is false and
  zeroes the blur), so adding one from the ➕ menu changes nothing until a slider moves.

- **`gizmo_renderer::components::active_camera`** — the one implementation of "which camera is the
  scene's": the one flagged `primary`, else the first, so the answer never depends on ECS iteration
  order. The engine's render path, the editor's viewport and the environment panel all ask it now,
  because a panel editing "the scene's camera" has to mean the camera the frame renders from.

- **`SpotLight` and `DirectionalLight` can be created in the editor.** Both were drawn by the
  engine and carried by the scene file, and neither was in the studio's `ComponentRegistry` — so
  neither appeared in the ➕ menu and the add handler had no arm for either. `SpotLight` also had no
  inspector section at all and could not fall through to the generic one, which looks a component
  up in that same registry. A scene that arrived without a sun could never gain one.

- **The export names the assets it could not package, and packages the ones it can.**
  `audit_scene_assets` walks the live world's reference fields — mesh sources in all three
  encodings, material and particle textures, terrain heightmaps, scripts, sounds — and every file
  outside the four shipped trees is either copied into the package **at its own relative path** or
  reported in the build log with the reason it could not be. Nothing is rewritten, which is what
  makes it safe: rewriting is where the hazards live (a glTF sub-mesh key with the path buried in
  its middle, an asset UUID that would drag the path back to the development tree). An absolute
  path and a `.gltf` with sidecars are refused rather than half-done, and a path containing `..` is
  refused outright — it would write outside the export directory. The final log line is
  `⚠ BUILD TAMAMLANDI — N varlık eksik` when anything is missing, instead of the unqualified
  celebration that used to follow an incomplete package.

- **`gizmo::systems::prefab::prefab_request_system`** — the resolver `gizmo-core` says the
  application must supply for `PrefabRequest`, run from `PlayLoop::step` (see *Fixed*).

- **`gizmo::systems::terrain::terrain_mesh_system`** — turns a `Terrain` recipe into the mesh a
  draw path can draw, on both hosts (see *Fixed*).

### Fixed

- **The inspector's "Trigger" checkbox never reached the physics world — and it was the third field
  lost by one line.** `physics_step_system` rebuilds each body's collider from the ECS every frame
  and did it with `Collider::from_shape(shape)`, which takes everything but the shape from
  `Default`. The `material` half of this was found and fixed earlier by appending
  `.with_material(...)`; the two nobody appended were `is_trigger` and `collision_layer`. A
  collider with the box ticked was rebuilt **solid** every frame, so the player hit the door sensor
  instead of walking through it — and because the pipeline chooses between a contact manifold and a
  `TriggerEvent` on that same flag, **no ECS body could emit a trigger event at all**, which is
  also why Lua's `physics.triggers` was structurally always empty. The gather now clones the
  authored collider and replaces only its shape, which closes the class rather than its third
  instance. No determinism change: for a collider with the default flags the rebuilt struct is
  bit-identical, and `headless_stress_test` agrees across three runs.

- **Texture streaming was deleting the paths the user had assigned.** The request stage marked a
  material as "already asked for" by clearing `Material::texture_source`, and nothing put it back —
  the apply stage writes only `bind_group`. `material_sync` copies the material into a
  `MaterialDesc` every frame, that is what a scene file carries, and on load it overrides
  `MaterialSource`. So opening a textured scene, waiting a few seconds and pressing Ctrl+S wrote
  away every albedo path in the file, and those objects reopened white. Nothing looked wrong while
  the editor was open, because the bind group was still installed. The dedup marker is now a set of
  `(entity, path)` pairs on `AssetServer`, keyed by path so re-assigning a texture is a new
  question rather than a permanently suppressed one.

- **Every texture assigned in the editor was missing from the exported game.** The export shipped
  `demo/assets` under the name `assets` and nothing rewrote the references — while the asset
  browser's workspace root *is* `demo/assets`, so a dragged texture is stored as
  `demo/assets/foo.png`. The shipped binary makes its own directory the working directory, looks
  there, and finds nothing. The package therefore opened untextured **on the machine that built
  it**. `every_export_dir_ships_to_the_path_the_scene_names` now holds the whole list to
  destination == source, because the same mistake is available to the next directory anyone adds
  and is invisible unless someone runs the exported binary.

- **The authored look was written to no file at all.** See *Changed* for what moved and how to
  migrate. The editor's viewport and the shipped frame also disagreed about *exposure* — the
  viewport drove it from the editor's own slider while the game read `Camera::exposure` — which the
  same change ends.

- **`Terrain` was a recipe nothing converted outside the editor.** The only code that turned one
  into a `Mesh` was `gizmo-studio`'s render file, driven by a queue of editor edits. Nothing pushed
  on **load**, and `Mesh` owns GPU buffers so no file can carry one — so a saved level reopened as
  an entity that says it is a terrain and draws nothing, and an exported game never had one. The
  new `terrain_mesh_system` is triggered by **presence** (`Terrain` and no `Mesh`), which is true
  on load, on export and after an add, where a queue of edits is true for none of them.

- **`IsHidden` was honoured only by the editor's draw loop.** The marker's own documentation says
  "do not display this entity", the hierarchy's 👁 and the H shortcut are built on it, and the
  engine's shared `collect_draw_items` never asked — so an object hidden in the editor came back in
  the Game panel and in the exported game, and a game calling `world.add_component(e, IsHidden)`
  got nothing.

- **The inspector's "Mesh Renderer" section reset on every save and reopen.** `MeshRenderer` was in
  no registry, so a LOD bias and a `Cast shadow` mode an artist set were simply not written. It
  came in by a different route than the components fixed earlier — `asset_loading` adds one beside
  every mesh it loads, so it is not in the ➕ menu and the menu-driven guard test could not see it.
  A second test now scans the inspector's own section calls, where every section must name the
  component it edits or say why the registry is not where its answer lives.

- **`entity.spawn_prefab` was swallowed rather than refused.** `flush_commands` matched the command
  and spawned an entity carrying a `PrefabRequest`, which had one producer and **zero readers**;
  the call looked applied and produced a named empty transform with no mesh, no collider and no
  warning. Unlike the script commands this engine drops on purpose, that one was not handed back to
  the caller and so was invisible. The key is read as a path — the one loader in the workspace takes
  one and there is no prefab catalogue to turn a name into anything else — and the prefab loads as a
  child of the requesting entity, keeping the name and position the request already carried.

- **Single-clicking a scene with unsaved changes did nothing at all.** The asset browser set
  `scene.load_confirm_dialog`, which had no reader anywhere: no modal, no load, not even a status
  line. It compounded — `has_unsaved_changes` is raised by the animation panel and the studio's own
  save path never lowered it, so one dragged keyframe disabled single-click loading for the rest of
  the session. Both halves are fixed.

- **The README badge asked for a Rust the workspace rejects.** It said 1.92+ against a 1.96 floor
  the same file states a hundred lines later.

- **The README understated the audio layer and `demo-web/README` contradicted itself.** The README
  said "there is no mixer, bus routing or DSP yet" while all three exist and run on the engine's own
  audio pass (a `Mixer` with `music`/`sfx`/`ui`/`voice` buses and a master gain, and a live low-pass
  biquad driving the underwater muffle). `demo-web/README` said audio was not enabled by that crate,
  ten lines under its own description of the beep that proves it is. Audio is **not** native-only:
  `gizmo-audio` has a wasm backend and `demo-web` enables the feature.

## [0.10.0] — 2026-08-16

### Changed

- **Graphics stack upgraded to `wgpu` 30 / `egui` 0.36 — and the MSRV rises with it,
  `1.92` → `1.96`.** This is why the release is `0.10.0` and not `0.9.2`: raising the minimum
  supported Rust version is a semver-minor change, and under `0.x` the minor position is the
  breaking one.

  The floor was measured across every dependency's declared `rust-version`, not guessed:
  `transform-gizmo-egui 0.10` asks for 1.96, the `egui` 0.36 family and `bevy_ecs` for 1.95,
  and `wgpu` 30 for only 1.87. The previous floor of 1.92 came from `egui` 0.34 the same way.

  Full set: `wgpu` 29 → 30, `naga` 29 → 30, `naga_oil` 0.22 → 0.23, `egui`/`egui-winit`/
  `egui-wgpu` 0.34 → 0.36, `egui_dock` 0.19 → 0.21, `transform-gizmo-egui` 0.9 → 0.10.

  The automated bump this replaces was internally inconsistent and could not build: it left
  `egui` at 0.35 while moving `transform-gizmo-egui` to 0.10 (which needs 0.36), and moved
  `wgpu` to 30 while leaving `egui-wgpu` at 0.35 (which needs 29) — two `egui`s and two `wgpu`s
  in one graph, whose types do not interconvert.

  Nothing about rendering changed on purpose. `SurfaceConfiguration::color_space` is set to
  `Auto`, the variant documented as reproducing wgpu's pre-30 behaviour, because a dependency
  upgrade is the wrong place to change how colour reaches the screen. The determinism hash is
  unmoved (`A462C9EB8A09D5CA`, 3/3).

### Fixed

- **`egui` texture deltas were being dropped.** `textures_delta.set` now carries a *list* of
  deltas per texture — a font atlas that grows in several regions in one frame arrives as
  several partial updates. Uploading only the first would have left the rest of the atlas
  stale: glyphs rendering as blank boxes a frame later.

- **`gizmo-animation` shipped to crates.io and docs.rs with no README.** It was the only one of
  the 19 published crates missing `readme.workspace = true`.

- **README images were broken on crates.io.** They were relative paths, which GitHub resolves
  against the repository and crates.io does not — it serves the README from its own domain, so
  `media/logo.png` became a 404. Now absolute.

- **The MSRV CI gate could have stopped guarding anything.** Its toolchain was pinned as the
  action ref (`dtolnay/rust-toolchain@1.92.0`), which Dependabot cannot tell from an action
  version; it opened a bump to `@1.100.0`, a Rust that does not exist. The toolchain is now
  derived from `rust-version` in `Cargo.toml`, so there is no constant left to drift.

## [0.9.1] — 2026-08-16

> **`0.9.0` never reached crates.io.** It was a local version bump — no `v0.9.0` tag, no
> registry upload — so the last published release is `0.8.0` and this is what follows it there.
> The `[0.9.0]` section below is kept as written: it documents real work, and rewriting history
> to pretend it was `0.9.1` would make the dated record wrong. Under 0.x semver `0.8 → 0.9` is
> the breaking-allowed step, which is what the `Sprite` removal needs, so nobody pinned to
> `^0.8` is upgraded into it.

### Removed

- **`gizmo_renderer::components::Sprite` — BREAKING.** An exported component that nothing could
  draw. Nothing in the workspace referenced it: not `default_render_pass`, not any engine system,
  not `gizmo-studio`, not the editor, not a demo, not even `gizmo-app`'s scene registry, which
  registers its sibling `Camera2D`. Attaching one to an entity rendered nothing and always had.
  Wiring it would have meant writing a 2D sprite pipeline — billboarding, layer sorting, atlas UVs,
  transparency — which is a feature rather than a fix, and one nothing in the engine or its
  consumers asked for. An exported component that cannot be drawn is a promise the API does not
  keep, and the 1.0 surface is the wrong place to keep it. Found by the sweep recorded in
  `docs/ENGINE.md` ("Written, exported, and not wired to anything").

### Added

- **`JointSolver::warm_start_factor`, default `0.0` — OFF, and whether to ship it on is not
  yet decided.** It is committed inert so the measurement phase can sweep it without a
  throwaway patch. A non-zero factor injects that fraction of the previous substep's λ in a
  separate sweep before iteration 0 (in-place injection is an exact algebraic no-op in this
  solver) and makes λ carried simulation state, which `WorldSnapshot` already covers. Do not
  raise it in library code.

- **A per-frame `Update` schedule.** `App` now carries two: `schedule` still runs `0..N`
  times per rendered frame at a constant `dt` (physics, unchanged — nothing moved out of
  it), and the new `update_schedule` runs **exactly once** per frame with the real frame
  delta. Register on it with `App::add_update_system`.

  This closes a defect that was hard to see and easy to blame on hardware. The single
  schedule ran only inside the fixed-timestep loop, so with the renderer's default
  `PresentMode::AutoNoVsync` pushing hundreds of frames per second against a 60 Hz
  accumulator, most rendered frames ran no systems at all. `Input` is captured once per
  rendered frame and its edges cleared once per rendered frame, but were consumed 0..N
  times — so keypresses and mouse motion on those frames were written and discarded with
  nothing observing them. Taps went missing; mouse-look arrived in fragments.

  `FpsLookPlugin` moves to the per-frame schedule for exactly that reason. Its placement
  is locked by a test: running the fixed schedule must not move the camera, running the
  update schedule must.

  The sequencing lives in `gizmo_app::frame::run_fixed_and_update` with seven tests that
  need neither a window nor a GPU — including the two that state the contract directly:
  nine short frames run the fixed schedule zero times and update nine times, and one long
  frame runs fixed four times and update once.

  `add_system` still targets the fixed schedule. Changing its default would have silently
  moved existing users' systems onto a variable `dt`.

- **Scene queries.** `QueryFilter` (layer mask, multi-body exclusion, trigger opt-in) plus
  `PhysicsWorld::raycast_filtered`, `overlap_shape`, `point_query`, `cast_shape` and
  `cast_body`, all broadphase-accelerated.

  The previous public surface was three unfilterable raycasts. A ray has no volume, so a
  character controller could not sweep its own capsule; without a layer mask every gameplay
  ray hit triggers, debris and the caster itself. The engine's own character and vehicle
  code shows what that cost: both bypass the broadphase and scan every collider in the
  world, every frame, because there was nothing to call.

  `cast_shape` marches over `NarrowPhase::test_collision` and bisects, rather than using
  `Gjk::conservative_advancement`. That routine is present in the crate but only correct
  for a head-on approach — with any lateral offset its step carries the shape into overlap,
  and `Gjk::distance` reports a positive distance for overlapping shapes instead of
  signalling penetration, so it concludes nothing was hit. Fixing it is tracked separately;
  the query layer does not wait on it.

- **`Material::ambient` and `Material::emissive` — the two knobs `MaterialType::BakedLit` was
  missing.** It was a bare multiply chain (vertex colour × instance albedo × texture, plus the
  sun's shadow term) with nothing in it that adds, so content authored dark could not be lifted
  from anywhere: the material had no term for it, and the ACES toe downstream
  (`aces(x) ≈ 0.214·x` as `x → 0`) takes back most of what a small lift would have gained.

  `ambient` is incident light and joins the baked term **before** the albedo/texture multiply,
  so a lifted surface keeps its own colour instead of flooding toward grey. `emissive` is the
  surface emitting and is added **after**, so a black wall can still carry a lit window — the
  relationship glTF's `emissiveFactor` has to base colour. The full expression is

  ```text
  rgb = (baked · (1 − 0.45 + 0.45 · sun_visibility) + ambient) · albedo · texture + emissive
  ```

  Both default to zero and both are floored at zero on the way to the GPU (the fields are
  `pub`, so the builders are not the enforcement point). With the defaults the expression is
  `baked · shade · albedo · texture` — the same bits as before, checked by
  `zero_knobs_reproduce_the_old_expression_exactly` rather than assumed.

  Exposure was already reachable and did not need a new knob: `Camera::exposure` is the single
  exposure applied over the whole composited HDR (the `1.15` in `post_process.rs` is only the
  buffer's initial content, overwritten every frame from the active camera; `Camera`'s default
  is `1.0`). `camera_exposure_brightens_the_frame` covers that end to end.

  Neither knob is scene-serializable, because `MaterialType::BakedLit` itself has no
  representation in `SceneData` — a pre-existing gap, documented on `MaterialSource::unlit`.

- **`MaterialType::Backdrop` — the scene's OWN painted sky/panorama geometry.** The engine had
  two materials that each got half of it. `Skybox` gets the depth right and then throws the mesh
  away: `sky.wgsl` contains no `textureSample` at all and never reads the vertex colour, so it
  paints a procedural gradient over whatever was drawn. `Unlit` gets the pixels right but is
  ordinary world geometry — it writes depth and does not follow the camera, so a backdrop panel
  stands *in front of* the world it is meant to sit behind, and a backdrop authored near the
  origin is left behind (and frustum-culled) once the camera drives away from it.

  `Material::with_backdrop(tint)` selects it. Four things travel together and are not separately
  settable, because seven of the eight combinations of them are bugs:

  * drawn **before** the world — `DrawLayer::Backdrop` in the game batcher's draw-order sort, a
    dedicated first loop in the studio pass. Not redundant with the depth pin: a transparent draw
    writes no depth, so a backdrop drawn after one would paint over it.
  * **locked to the camera** — `backdrop.wgsl` adds `scene.camera_pos` before the view transform,
    which cancels the view's translation exactly and keeps its rotation (`V·(x+c) = R·x`).
  * **never writes depth**, and every vertex is pinned to NDC `0.99999`, so it also cannot win
    the depth test against geometry the deferred pass has already put in the depth buffer.
  * the **mesh's own pixels**: `vertex colour × instance albedo × texture`, alpha included, with
    no lighting and nothing second-guessing a black vertex colour.

  Because the authored transform is not where the triangles land, culling and LOD go through
  `gizmo_renderer::backdrop::camera_locked_model` first; `classify_visibility`'s doc now says so,
  and a `Backdrop` never casts a shadow. Rendered evidence, not just state assertions:
  `a_backdrop_shows_the_meshs_own_pixels_and_stays_behind_the_world`,
  `a_backdrops_texture_reaches_the_screen` and `a_backdrop_is_locked_to_the_camera` drive the
  real `default_render_pass` into an offscreen target and read the pixels back, each with an
  `Unlit`/`Skybox` arm that reproduces the old behaviour.

  `SceneState` gains `backdrop_pipeline` (and `baked_lit_transparent_pipeline`); like
  `BakedLit`, `Water` and `Grid`, `Backdrop` has **no representation in a scene file** — the
  `MaterialData::unlit` encoding has no free slot — so a backdrop must be rebuilt by code after
  a scene loads.

### Changed

- **Vertex colour is RGBA.** *Breaking for code that builds `gpu_types::Vertex` by hand:
  `color` is `[f32; 4]`, not `[f32; 3]`, and `Vertex`'s stride is 96 bytes (was 92).*

  The alpha channel existed nowhere: the attribute was `Float32x3`, so a decal or skid-mark
  layer's soft edge never reached the GPU and the layer drew as opaque geometry over the road.
  Widening the attribute is half the fix — the other half is that `BakedLit` now has a
  **transparent pipeline variant** (alpha blending, no depth write, no culling) chosen by
  `Material::is_transparent`. Every other stage already routed such a material as transparent
  (sorted back-to-front, skipped by the z-prepass and the shadow pass); only the pipeline
  disagreed, and `blend: None` discarded the alpha the shader had been computing all along.
  Opaque baked-lit geometry keeps exactly the state it had.

  Shaders that do not want the alpha still declare `@location(1) color: vec3<f32>` — a vertex
  format may supply more components than the shader consumes (wgpu checks only the scalar
  *kind* for a vertex input) — so every shader reading the attribute except `baked_lit.wgsl`
  and the new `backdrop.wgsl` is untouched.

- **`BakedLit` takes the vertex colour at face value.** It used to rewrite a near-zero colour
  to white (`if length(baked) < 0.0001`), so that an importer which never set the attribute
  could not black out a model. Whether the attribute *exists* is a property of the vertex
  layout, not of a pixel value, and this engine has one layout that always has it: a source
  file with no colours is normalised to opaque white **at import**, which is the only place
  that knows it was absent. The consequence of the old test was that a surface an author
  painted black came out white. No in-tree mesh source produces a zero vertex colour, so no
  shipped scene changes; a project that relied on the rewrite must set white explicitly.

- **`InstanceRaw` carries `ambient` and `emissive`** — 128 bytes, was 96, and the instance
  storage buffer grows with it (~1.0 MB at the default 8192-instance capacity, was 786 KB).
  All ten shaders that index the buffer declare the two new slots; a missed one is a wrong
  element stride, so `every_instance_shader_declares_the_full_struct` checks all ten.
  `InstanceRaw::new` is the one place a `Material` becomes instance bytes, shared by the game
  and studio render paths.

- **Vehicle suspension rays go through the broadphase.** *Behavioural: a wheel can now only rest
  on a body the rigid pipeline simulates. Read the second half of this entry before upgrading a
  scene whose drivable surfaces are bare `Collider`s.*

  `vehicle_controller_system` used to clone **every** `(Transform, Collider)` entity in the ECS
  into an owned `Vec`, every step, and then scan all of it once per wheel. It now casts one
  `PhysicsWorld::raycast_filtered` per wheel, excluding the chassis — the scene-query layer added
  above, which existed precisely because this code had nothing to call.

  Measured on 4 098 colliders and one car, `--release`
  (`wheel_query_cost_scan_vs_broadphase`, `#[ignore]`d):

  | per step | linear scan | broadphase |
  | --- | --- | --- |
  | `Collider` clones | 4 098 | 0 |
  | colliders visited (4 rays) | 16 392 | 8 |
  | `vehicle_controller_system` | 0.434 ms | 0.002 ms |

  What a wheel can rest on changed, and the change is not cosmetic:

  * **Only bodies the solver knows about.** A `PhysicsWorld` holds what `physics_step_system`
    syncs into it — `RigidBody` + `Transform` + `Velocity` + `Collider`, minus `Pooled` and
    `IsDeleted`. A `Collider` + `Transform` entity with no `RigidBody` used to hold a car up
    while the chassis fell straight through it; now the wheels agree with the solver and ignore
    it. Every vehicle demo in this repo already gives its ground a `RigidBody::new_static()` and
    a `Velocity`, so none of them move. If yours does not, add them — or keep calling
    `update_vehicle` (unchanged, and still the ECS-free entry point) with your own collider list.
  * **One step of latency on newly spawned geometry**, which becomes visible to wheels after the
    next `physics_step_system`. Before the world's first step the system falls back to the old
    ECS scan, so "spawn a car, read `is_grounded`, then start stepping" is unaffected.
  * **The wheel sees the solver's collider**: the entity's shape merged with its children into a
    `Compound`, carrying its `PhysicsMaterial`. Consequently an ECS trigger volume that also has
    a `RigidBody` is now drivable, because `physics_step_system` drops `is_trigger` (and
    `collision_layer`) when it rebuilds the collider for the solver — a pre-existing defect in
    that bridge, not in the query filter, which does exclude triggers. For the solver that volume
    was already solid; the wheels have stopped disagreeing with it.
  * **Unchanged**: the chassis never hits itself, no layer mask is applied, dynamic bodies are
    still drivable, the nearest hit wins, and surface friction still comes from the hit collider.

  New public API in `gizmo-physics-dynamics`: `WheelGroundQuery` (one ray, one nearest surface),
  `WheelGroundHit`, `ColliderListQuery` (the old linear scan, as an implementation) and
  `update_vehicle_with_query`. `PhysicsWorld` implements `WheelGroundQuery`. On a scene both
  implementations can see, they are asserted bit-identical down to the resulting `Velocity`.

  **`character_controller_system` still has this defect and is deliberately not fixed here.** It
  is the twin the scene-query changelog entry named, but it is not the same fix: the KCC wants a
  *capsule sweep* (`cast_shape`) rather than a ray — its three centre-line rays walk through
  anything that misses the centre line — and it is routinely run against `Collider`-only entities
  that never enter a `PhysicsWorld`, including in its own tests. It also does not skip triggers
  today, so a character can stand on one. Follow-up.

- **Rigid joint rows are soft constraints now, parameterised by FREQUENCY.** *Behavioural: every
  joint scene moves. New `JointSolver::rigid_hertz` (default 200 Hz); `0` restores the previous
  Baumgarte behaviour exactly.*

  A joint row with `compliance == 0` used to be solved as a hard equality with a Baumgarte push
  (`β·C/dt`, velocity-clamped) and **no `−impulse_scale·λ` feedback term**. That missing term is
  a measured defect, not a stylistic one — though not for the reason first written down here: a
  hard row does not accumulate its own residual (with `mass_scale = 1, impulse_scale = 0` the
  coefficient on λ in its update is already zero), the memory lives in the Gauss–Seidel coupling
  between the rows of an ill-conditioned chain. What the term buys is a genuine compliance
  proportional to the row's own effective mass. Its absence is what destroyed
  joint warm start at its natural factor of 1.0 — the value the contact solver runs its own warm
  start at — while the identical scene at `compliance = 1e-6` was stable (`docs/FIXPLAN.md`, B4
  commit 5). Rigid rows now take the same Box2D-v3 soft formulation the contact solver uses,
  with ω from a frequency rather than from `√(k/α)`, which is undefined at zero compliance.

  The observable contract is a closed form: **static constraint error = `a/ω²`**, `ω = 2π ·
  min(rigid_hertz, 1/dt)` — 6.2 µm per `g`, and **independent of the mass, the iteration count
  and (below the clamp) the substep rate**. Neither Baumgarte nor a compliance can express any
  of the three. `tests/joint_rigid_stiffness.rs` asserts each one.

  Deliberately NOT the contact solver's numbers: 200 Hz not 30, ζ = 1 not 10. A contact carries
  roughly its own body's weight, a joint up to 400× that, and the naive mirror sags a 1 kg rope
  by 2.8e-4 m and a 16-link chain by 1.8 m.

  **What it costs.** Baumgarte converged to zero error given enough iterations; a spring does
  not. On a 16-link chain with a 200 kg tip the converged floor rises from 11 mm to 39 mm of
  constraint error and no iteration count removes it — though the *shipped* 10-iteration answer
  improves, from 154 mm to 95 mm.

  **One row class is deliberately NOT softened.** A soft row leaves `impulse_scale · v` behind
  and what takes it back is the row's position term, so a row that has none would drift linearly
  and without bound — measured at 7.3° in 40 s on a weld under a sustained 10 rad/s². The only
  such row in the crate is `JointData::Fixed`'s 3-axis angular lock (it passes a literal zero
  error, because a `Fixed` joint carries no reference pose to servo toward), and it stays a hard
  velocity constraint on every path. An offset anchor does not mitigate this and never did:
  rotating a body about a pinned anchor leaves that anchor coincident, so the linear rows see
  nothing. Every other angular row — the slider's quaternion lock, D6 `Locked`, the hinge's axis
  alignment — carries a real error and gets the bounded `a/ω²` droop instead.

  Also changed: `max_correction_speed`/`max_angular_speed` bind at 2.9 cm / 1.65° of error
  instead of 6.9 cm / 4.0°, since the bias rate went from 72 s⁻¹ to 173.7 s⁻¹.
  `compliance_damping_ratio` now governs rigid rows too. `position_bias` keeps two live roles —
  the legacy path selected by `rigid_hertz = 0`, and the hinge/slider servo P-gain, unchanged on
  both paths.

- **`gizmo-core` no longer exposes `crossbeam-queue` or `tracing-subscriber`.** *Breaking for
  anyone who called the asset-handle constructors or used the tracing bridge; nothing
  behavioural.*

  Two seals on a Stage A crate, both impossible after 1.0 because the seal is itself the
  breaking change:

  - `asset::AssetDropQueue` is a new opaque newtype around the concurrent queue that strong
    `Handle`s report their death to — no public constructor, no accessor, no public field.
    `Handle::new` and `Handle::make_strong` take it instead of `Arc<SegQueue<usize>>`, and
    `HandleIdTracker::drop_queue` is private, which also closes the transitive path through
    the public `Handle::tracker`. Because the newtype cannot be constructed from outside the
    crate, those two constructors are effectively in-crate only now.
  - `GizmoTracingLayer` and `logger::init_tracing()` moved behind the new default-OFF
    `tracing-layer` feature (mirrored on the `gizmo-engine` facade). `tracing` itself stays an
    unconditional dependency — 0.1 is frozen and leaves no type in our signatures; it is the
    `Layer` impl, a foreign trait on a public type of ours, that carries the semver cost.

  **Migration:** build handles with `Assets::add`, which mints the id and binds the queue —
  there was never a supported way to obtain the queue anyway, and a handle bound to a
  privately-built one leaks its asset instead of collecting it. For the tracing bridge, add
  `features = ["tracing-layer"]` to your `gizmo-engine` (or `gizmo-core`) dependency;
  `gizmo-studio` does. `gizmo_log!` fills the in-memory buffer either way.

- **`gizmo-physics-core` no longer exposes `arrayvec` — and the earlier claim that it had
  stopped was wrong.** *Breaking only for code that named the by-value iterator type.*

  0.2.0 made the `ArrayVec` storage field of `ContactPoints` private and the changelog recorded
  that accurately, but `<ContactPoints as IntoIterator>::IntoIter` went on reading
  `arrayvec::IntoIter<ContactPoint, 4>` — a 0.7.x type on the default surface of a Stage A
  crate, and an associated type nobody has to write down to be bound by. It is the opaque
  `collision::ContactPointsIter` now (private field; `Iterator`, `DoubleEndedIterator`,
  `ExactSizeIterator` forwarded, `FusedIterator` asserted by us because arrayvec 0.7 omits it),
  re-exported at the crate root beside `ContactPoints`. The by-ref impl already yielded
  `std::slice::Iter` and is unchanged. Both associated types are now pinned by always-on
  `const _` fn-pointer coercions, so a future widening fails the build rather than the review.

  **Migration:** nothing, unless you spelled `arrayvec::IntoIter<ContactPoint, 4>` in a
  signature or a struct field — write `ContactPointsIter` there instead. Iterating, collecting
  and `for` loops are unaffected.

- **`gizmo-scene` no longer exposes `ron` or `web-time`.** *Breaking for anyone who reached
  through them; nothing behavioural.*

  `gizmo-scene` is a Stage A crate — it goes to `1.x`, where 1.0 means no breaking change
  without a 2.0 — so a `0.x` dependency's type on its public surface would have handed that
  dependency the power to force our 2.0. Three seals, all of which are impossible after 1.0
  because the seal is itself the breaking change:

  - `pub use ron;` is gone. It re-exported the parser's *entire* surface as
    `gizmo_scene::ron`, so a `ron` 0.13 broke us whether or not our own types moved.
  - `SceneError::Parse` / `::Serialize` now carry `error::ParseError` / `error::SerializeError`
    — types we own, with private payloads, forwarding `Display` and `Error::source` verbatim
    so a printed error chain is unchanged. The one thing the transparent payload gave you,
    the failure position, comes back as `ParseError::line()` / `::column()`. The two `From`
    impls stay, so `?` on a parser call still lands in the right variant.
  - `SceneSnapshot::timestamp` is private. On `wasm32` that field was a `web_time::Instant`
    on the *default* surface — target-selected, not feature-gated, so no caller could opt
    out. `age() -> std::time::Duration` was already its only reader anywhere in the workspace.

  **Migration:** `ron::from_str::<SceneData>(s)` → `SceneData::from_ron_str(s)`, and
  `ron::ser::to_string_pretty(&scene, …)` → `scene.to_ron_string()`. Unlike a bare parse,
  `from_ron_str` also runs `migrate`, so a string from a newer engine is refused instead of
  silently truncated. Reading `err.position.line` off a parse error → `p.line()` / `p.column()`.
  Reading `snapshot.timestamp` → `snapshot.age()`. The facade's `gizmo::ron` re-export went
  with it; `gizmo::scene::SceneData::from_ron_str` replaces it.

  Separately, the `web-time` pin in the seven crates that use it (`scene`, `core`,
  `physics-rigid`, `ai`, `app`, `editor`, `renderer`) moves `0.2` → `1`. The lock already
  carried `web-time` 1.1.0 by way of `winit`/`egui`/`bevy_platform`, so the old pin was
  forcing a second, duplicate build of a dead major; 0.2.4 is now gone from the lock.

- **`gizmo-physics-rigid` no longer exposes `rustc-hash`.** *Breaking for anyone who read or
  wrote `PhysicsWorld::entity_index_map` as a `HashMap`, or who called `solve_contacts` /
  `solve_joints` directly; nothing behavioural — not one simulated value changes.*

  This was the last thing blocking `gizmo-physics-rigid` from `1.x`, and like the three seals
  above it is impossible after 1.0 because the seal is itself the breaking change. It also
  survived the surface audit that produced those three, for an instructive reason:
  `FxHashMap<u32, usize>` is a type *alias* for `HashMap<u32, usize, FxBuildHasher>`, so the
  foreign hasher rode along on our public surface without anyone ever writing its name — and
  `rustc-hash` 1.x → 2.0 changed precisely that alias, so a `rustc-hash` 3.0 forcing our 2.0 was
  a repeat, not a hypothetical.

  `world::EntityIndexMap` is a new opaque newtype over the handle→row-index map: private field,
  no `Deref`, no `AsRef`, no accessor, and a hand-written `Debug` that prints only the length
  (a derived one would have printed every entry in hash order). Three sites now name it —
  the `PhysicsWorld::entity_index_map` field, `ConstraintSolver::solve_contacts` and
  `JointSolver::solve_joints`. The field stays `pub`: `get`, `contains_key`, `len` and
  `is_empty` mirror the `HashMap` methods exactly, so **the four common reads need no change**.
  Serialization is `transparent`, so `trigger_snapshot`'s JSON is byte-identical.

  **Migration.** Reading — `world.entity_index_map.get(&id)`, `.contains_key(&id)`, `.len()`,
  `.is_empty()` are unchanged, references and all. The rest of the `HashMap` read surface is
  *not* carried over and does need a change: `map[&id]` (no `Index`), `==` between two maps (no
  `PartialEq`), `get_key_value`, `keys`/`values`, `capacity` and `hasher` are all gone. Say so if
  you need one back — every one of them is an additive impl, so it can be restored after 1.0
  without a major. Writing — `insert`, `remove` and `clear` are
  now crate-private; use `add_body`, `remove_body_at`, `sync_bodies` or `clear_bodies`, which is
  what you wanted anyway, since the map has to stay in lockstep with the parallel arrays and a
  stale entry silently points at whichever body now occupies that row. (The field is still `pub`,
  so assigning a whole new map over it compiles — that is not a supported route, it just is not
  something a newtype can prevent without privatising the field.) Iterating — there is no
  iterator, on purpose: hash order is not part of the determinism contract. Iterate
  `PhysicsWorld::entities` (a `Vec`, so it has a defined order) and look each handle up.
  Building one for a bare `solve_joints` / `solve_contacts` call that has no `PhysicsWorld`
  behind it — `let map: EntityIndexMap = pairs.into_iter().collect();` (`FromIterator<(u32,
  usize)>`), which replaces `let map: FxHashMap<u32, usize> = pairs.into_iter().collect();`
  and needs no `rustc-hash` dependency of your own.

- **The joint solver no longer writes into a sleeping mechanism.** *Behavioural for any jointed
  mechanism that falls asleep; bit-identical for everything else.*

  `JointSolver::solve_joints` now skips a joint whose two ends are both inert — a sleeping
  dynamic body, or a static/kinematic one that is not moving. There was previously not a single
  `is_sleeping` reference anywhere in the joint solver: it wrote velocities into sleeping bodies
  that position integration then discarded. `PhysicsWorld` runs its joint-graph wake pass
  immediately before the solve, so any component containing a mover is already awake when the
  gate is evaluated and nothing that should be simulated is skipped
  (`tests/joint_sleep.rs::waking_a_component_is_solved_on_the_same_step_it_wakes` pins that
  ordering). Two user-visible consequences: an embedder calling `solve_joints` without a wake
  pass of its own now gets nothing solved for an all-asleep mechanism, and a joint whose bodies
  fall asleep stops reporting load and can therefore no longer break — which is what already
  happens when a contact island sleeps.

  `state_hash` also mixes the joint array now (endpoint pair, `is_broken`, and the solver's λ
  slots), so a rollback desync in joint state is caught directly instead of only once it has
  bled into velocities. `tests/rollback.rs` gains a 16-link chain with a 200 kg tip — the
  hardest-loaded joint scene in the suite — as a snapshot-completeness probe.

  **Warm start was built, measured, and NOT shipped.** The mechanism works as an accelerator:
  ten warm iterations land far closer to the converged answer than ten cold ones on that chain.
  But at the natural factor of 1.0 — the value the contact solver's own warm start uses — it
  destroys the mechanism, and not only at high mass ratios: a 16-link chain with a *1 kg* tip
  ends up at 17.54 m and 30 m/s against a converged 16.000. The cause is structural. A rigid row
  (`compliance == 0`) has no `−α̃·λ` feedback term, so an under-converged pass integrates the
  Baumgarte residual into λ substep after substep; the identical chain at `compliance = 1e-6`
  is stable at factor 1.0 and better than cold, which is also why the contact solver — TGS-soft
  throughout — can run its own warm start at 1.0. The only working setting was a tuned 0.5 with
  a cliff 1.75× above it, measured on three chain scenes and never on a ragdoll, vehicle or D6,
  so it was reverted rather than shipped behind a public knob defaulting to inert. The
  measurements and the named blocker are in `docs/FIXPLAN.md`; the sequencing is now: give the
  rigid rows the soft reformulation `soft_coefficients` already records as open, then the warm
  start needs no tuned constant at all.

- **Every bounded collider shape now shatters, and the ones that cannot no longer disable the
  entity.** *Behavioural for any non-box `Breakable`.*

  `shatter_entity` only ever read box half-extents and returned early on everything else — but
  all three call sites had already set `Breakable::is_broken`. A sphere, capsule or hull
  breakable that ran out of health therefore spawned no debris, was never despawned, and,
  because every damage path is gated on `!is_broken` and nothing in the engine clears that
  flag, could never be damaged or broken again. It stayed in the scene at zero health,
  permanently inert. The bail-out did not mean "unsupported shape does nothing"; it meant
  "unsupported shape destroys the entity in place".

  Sphere, capsule, convex hull and compound now shatter through the collider's **local
  bounding box**, so a sphere breaks like the cube around it. That is the approximation the
  debris already carried — each Voronoi cell is replaced by a volume-matched sphere whatever
  its real geometry — so the bound is not the weak link. `Plane` (an infinite half-space,
  whose AABB is a ±10 km cube that would have shattered the floor into kilometre-wide
  boulders) and `TriMesh` (static and concave, which convex debris cannot represent) still do
  not shatter; `shatter_entity` reports that to its callers, which now latch `is_broken` only
  on a real break, leaving such a body damageable instead of frozen.

  **The box path is unchanged, and that was measured rather than argued:** the debris field of
  a 0.5 m box is pinned bit-for-bit in `tests/breakable_shatter.rs`, and those numbers were
  read off the pre-fix build. Running the new test file against pre-fix `system.rs` turns five
  of its six tests red and leaves exactly that one green.

- **`gizmo-ui`'s `Style` no longer holds a `taffy::Style`.** *Breaking for `gizmo-ui`, which is
  marked experimental.*

  The component is now a plain `Copy` POD built on a crate-owned `Val { Auto, Px, Percent }`,
  and taffy is reached only inside `UiContext` at layout time. That deletes both
  `unsafe impl Send/Sync` — the component derives them now — whose soundness had rested on
  taffy's `calc` feature staying off, and closes the `pub use taffy::style::*` /
  `pub use taffy::geometry::*` leak, so no taffy type appears in a public signature, field or
  re-export any more. Every taffy field the new type does not model is named in its docs, the
  CSS Grid family included, so the omissions are stated rather than silent.

- **Debris patterns differ per object.** *Behavioural (visual) for anything with `Breakable`.*

  `shatter_entity` seeded the Voronoi cut with a literal `42`, so every object in a scene broke
  into the identical pattern. The seed is now derived from the entity's ECS id through a
  SplitMix64 finalizer.

  Deliberately **not** mixed with a frame counter, though the plan called for one: the engine
  has no rollback-safe counter to use — `PhysicsWorld` carries no tick, neither snapshot
  restores one, and `Time::frame_count` counts wall-clock-driven render frames. Any of them
  would make a rolled-back-and-resimulated break produce different debris than the original.
  Nothing is lost by leaving it out, since `is_broken` latches on the first shatter and the seed
  therefore never has to tell two occasions apart, only two entities.

- **The Stage A crates' public documentation is in English.** *Documentation only — no
  behavioural change.*

  `gizmo-core`, `gizmo` (the facade), `gizmo-physics-core`, `gizmo-math`, `gizmo-ai`,
  `gizmo-audio`, `gizmo-app` and `gizmo-scene` had 1286 Turkish `///` / `//!` lines between
  them; 8 remain, of which 7 are false positives of the detector (Plücker, Möller–Trumbore, and
  a line of English *about* Turkish casing) and the eighth is a doc quoting a Turkish log format
  string that a test asserts on. Translated sentence for sentence, with hedges left as hedges
  and emphasis left where it was — the same standard `docs/ENGINE.md` was held to. No
  intra-doc link broke: the rustdoc warning count is unchanged at 71.

  Comments inside doc examples were translated too, including the assertions that quote the
  messages they check. Plain `//` inline comments are still Turkish across the workspace — a
  larger surface, tracked separately.

- **Every doc example now compiles and runs.** *Documentation only — no behavioural change.*

  The workspace had 30 `​```ignore` fences, i.e. 31 doc-tests that rustdoc collected and then
  skipped. There are now **zero**: `cargo test --workspace --doc` goes from 17 passing / 31
  ignored to **45 passing / 0 ignored**. None of them needed `no_run` and none turned out to be
  irreducible pseudo-code, so all 30 became real tests — and they assert the documented
  contract rather than merely linking.

  Un-ignoring them surfaced defects the fences had been hiding, all now fixed:
  `World::spawn_bundle`'s example used `MeshBundle` / `Material::pbr` / `Color::BLUE`, none of
  which `gizmo-core` can even name; a doc comment in `component_ops.rs` was attached to the
  wrong function (it documented `query_entity_mut` while sitting on `insert_batch`, whose own
  summary was stranded at the bottom of the block); `web_profile.rs`'s module blurb was written
  with `///` and so documented the next item instead, and its example passed a `bool` where
  `with_shadows` takes a `ShadowQuality`; `gizmo-renderer`'s crate docs described the frustum
  matrix as `view * projection` when `camera.rs` builds `projection * view`; and
  `resource_scope`'s example could not compile at all, since its turbofish supplied two of
  three generic arguments and the third is an unnameable closure type.

- **The contact solver's soft-constraint penalty term no longer carries a mass factor.**
  *Behavioural only for `block_solver = false`; the default path never reaches this line.*

  `impulse_scale·λ` relaxes the accumulated impulse — it is not a velocity error to be
  converted into one — so it belongs outside the effective-mass division, as in Box2D v3's
  `-normalMass·massScale·(vn + bias) − impulseScale·λ`. It had been moved inside, making the
  update `λ_{n+1} = λ_n·(1 − impulse_scale/k_n) + …`, whose factor leaves the unit disc once
  `m_eff > 2/impulse_scale` (≈34.6 at the shipped `hertz = 30`, `ζ = 10`).

  Contacts never blew up the way the joint rows did — `max(0.0)` truncates the negative half of
  every cycle — so the symptom was quieter: penetration recovery stopped being mass-invariant,
  which is precisely what a constraint parameterised by hertz and damping must be. A box
  resting in 0.2 m of penetration finished at `1 kg 0.4733 · 100 kg 0.4092 · 300 kg 0.4084 ·
  1000 kg 0.4872` — 0.06 m of spread, and not even monotonic. Undivided it is 0.471068 at every
  mass from 1 kg to 5000.

  This reverses a change that had been landed as a bug fix with an arithmetic test showing only
  that the two orderings *differ*, never which one is right. That test is replaced by one
  asserting the property that decides it, plus a recursion test that exhibits the divergence
  and shows the clamp containing it. Determinism is unchanged (`A462C9EB8A09D5CA`) because the
  block solver, on by default, discards `impulse_scale` outright — asserted by a test so the
  scope claim cannot rot.

- **`NavMeshConfig::agent_radius` now erodes the walkable area.** *Behavioural for every
  navmesh built with a non-zero radius, i.e. the default.*

  The voxeliser computed a `ceil(radius / cell_size)` margin and used it only to widen the
  loop bounds — `blocked.insert` stayed gated on the obstacle's real AABB, so the setting
  produced no clearance at all and polygons ran right up to the wall. Each obstacle's blocked
  cells are now grown by that margin, so an agent that stays inside polygon interiors keeps
  its body clear. Clearance is quantised upwards to whole cells: at least the radius, up to
  one cell more.

  The floor-height sampling band moves outward with it rather than being swallowed by the new
  skirt. That band is the only writer of `walkable_y`, and blocking it in place drops every
  polygon to the `0.0` fallback — measured on that variant, not assumed. It keeps its old
  width and now sits just outside the skirt, so polygons near an obstacle still take their Y
  from its top surface.

  The three pre-existing navmesh build tests pass unchanged: they assert structure, not
  polygon coordinates, so nothing needed re-blessing. Three tests were added; the erosion one
  fails on the old build.

- **A floating-base articulated tree feels gravity.** *Behavioural for
  `ArticulatedTree { is_fixed_base: false }`, behind `experimental-multibody`.*

  In pass 3 of the ABA the root's parent acceleration is where gravity enters, as the
  fictitious `a_grav = (0, -gravity)`. The floating-base branch used `base_acceleration`
  *instead of* that term rather than in addition to it, so the `gravity` argument was accepted
  and discarded for the whole tree: with the default zero base acceleration a pendulum hung in
  mid-air at `q̈ = 0`. Both branches now share one formula — gravity, plus whatever base
  acceleration the caller prescribed — so a base at rest gives exactly the fixed-base answer,
  and a base falling at g (`base_acceleration = (0, +g)`) gives the weightless one.

  Fed a non-zero base acceleration the old branch did worse than drop gravity: it inverted the
  response. The free-fall input above produced `+4.9049997` where the correct answer is `0`,
  the exact negation of the `-4.905` fall. Nothing in the workspace sets `is_fixed_base = false`
  — every test and every default is fixed-base — which is why this survived property testing.

- **`glam` 0.29 → 0.32, `bevy_reflect` 0.15 → 0.19.** *No behavioural change — see below.*

  `glam` is the one deliberate permanent public dependency, so its major version is part of
  the 1.0 promise: shipping 1.0 on 0.29 would have made the upgrade a 2.0-level break for
  every downstream crate. It was three majors behind.

  The blocker was real but narrower than recorded: nothing in the workspace pinned 0.29
  except `gizmo-math`'s own manifest. What broke was the default-off `reflect` feature —
  `bevy_reflect 0.15` implements `Reflect` for `glam 0.29`'s types, so with the engine on
  0.32 those impls no longer applied. `bevy_reflect 0.16` is on glam 0.29 as well; 0.19 is
  the first release on 0.32, which is why the jump is four minors.

  The physics did not move: `state_hash` is unchanged at `EF6E4AC3644BF3BA` and every
  committed value in `tests/golden_state.rs` holds without re-blessing. That is worth
  stating explicitly — a maths-library major bump is exactly where silent numerical drift
  would hide, and the golden fixtures exist to answer that question rather than assume it.

  Benchmark-only follow-on: `bevy_math` / `bevy_picking` / `bevy_mesh` dev-dependencies moved
  to 0.19 too, so `glam` now resolves to a single version across the whole graph. Their APIs
  shifted — `CubicSegment::new_bezier` split off `new_bezier_easing`, `VectorSpace` gained a
  `Scalar` associated type, `ray_mesh_intersection` takes `Affine3A` plus a `uvs` argument,
  and `bevy_reflect`'s `clone_dynamic` became `to_dynamic_map` / `to_dynamic_list` /
  `to_dynamic_struct` with `Map` / `List` / `Struct` moved out of the crate root.


- **`compliance` is now an inverse stiffness.** *Behavioural for every joint with
  `compliance > 0` — ragdoll limits, elastic ropes, soft D6 locks.*

  The field is public, persisted, and documented as "0 = hard stop; larger = a soft, springy
  limit that gives under load". It did not behave like one. The implementation added
  `compliance / dt²` to the row's effective mass (CFM regularisation) and stopped there — but
  enlarging `k` only shrinks each iteration's step, so the sequential-impulse series still
  converges to the RIGID solution. All the observed softness came from `iterations` being
  finite. `compliance` was a relaxation factor for the solver loop, and doubling the
  iteration count halved its effect: the same rope stretched 0.0194 m at 5 iterations and
  0.0096 m at 10.

  Joints now use the same soft-constraint formulation the contact solver has always used
  (`bias_rate` / `mass_scale` / `impulse_scale`, Box2D v3), with each row's frequency derived
  from its compliance and effective mass as `ω = √(k/α)`. The result obeys Hooke's law:
  hanging 1 kg from a rope with `compliance = 0.03` settles `0.03 · 1 · 9.81 = 0.294 m` past
  its rest length, measured within 0.2% across two orders of magnitude of compliance and one
  of mass, and identical at 5, 10, 20 and 40 iterations.

  `compliance == 0` keeps the original rigid path unchanged, so nothing that did not opt into
  softness moves. `JointSolver` gains `compliance_damping_ratio` (default 1.0, critically
  damped) for the soft rows.

  If you tuned a ragdoll or a rope against the old numbers, re-tune: the value is now a
  physical spring constant rather than a solver artefact, and it no longer drifts when you
  change `iterations`.

- **`break_force` / `break_torque` now measure the joint's net reaction.** *Behavioural —
  finite thresholds already calibrated against the old numbers will need re-tuning.*

  Each joint type used to run its own break check from **inside** the solver's iteration
  loop, comparing `Σ|λᵢ|` — the L1 sum of its rows' impulse magnitudes — against the
  threshold. Summing magnitudes of rows that are not collinear does not give the force the
  joint carries. A weld's three linear rows are the world X/Y/Z axes, so the same 9.81 N
  load reported 9.81 N when gravity pointed down one axis and 17 N when it pointed
  diagonally: the reported force depended on the arbitrary orientation of the load relative
  to the world axes, and nothing else. On a ball-socket, whose cone/twist/swing rows are not
  even orthogonal, there was no bound on the overstatement.

  There is now one check, once per solver pass, against `‖Σ λᵢ·nᵢ‖ / dt` — the magnitude of
  the net impulse the joint actually applied. Three further consequences:

  - `Joint::check_break` was public API with **zero callers**. It is now the one code path.
  - A `Fixed` joint whose anchors were exactly coincident skipped its linear break check
    entirely, because the whole linear block sat behind an `err_len >= 1e-4` gate.
  - Slider suspension springs and hinge torsional springs carry real load and were invisible
    to the break check — a "breakable" shock absorber could hold any load forever. They now
    report into the same total. Motors and D6 drives deliberately do **not**: they are
    actuators, not external load.

  A joint that breaks now does so at the end of the pass rather than mid-iteration, so it
  transfers one extra step's worth of impulse before letting go. `world.joint_solver
  .iterations` no longer participates in the calculation at all.

- **`UiPlugin` and `TransformPlugin` now register on the per-frame schedule.** Layout,
  hit-testing and transform propagation are read once per rendered frame; running them
  `0..N` times in the fixed loop was both wasted work and, with vsync off, a hover that
  registered on roughly one frame in ten.

  Moving transform propagation also turns an intention into a guarantee. `PhysicsPlugin`
  labels its step `physics_step` with a comment saying transform systems "can order
  themselves after it", but no such edge was ever wired. The update schedule runs after
  every fixed step of the frame, so "transforms propagate after physics" is now structural —
  and it happens after the per-frame update systems too, which is what a camera moved by
  `FpsLookSystem` needs.

- **The headless runtime has a fixed timestep.** It used to run its single schedule once per
  loop iteration with the real elapsed `dt`; with the loop's 1 ms sleep that is roughly a
  thousand ticks a second, so a server registering `PhysicsPlugin` stepped physics ~1000
  times per second while the same plugin in the windowed runtime stepped at 60 Hz. Both
  runtimes now use the same sequencing, so a plugin behaves the same in either. Simulation
  determinism was never at risk — `PhysicsWorld::step` substeps internally at 240 Hz — but
  the cadence and the wasted work were real.

### Fixed

- **Scripting: script execution order was randomised per process.** `ScriptEngine::loaded_scripts`
  was a `std::collections::HashMap`, whose `RandomState` is seeded per process, so `update` ran
  scripts in a different order on every run. Two scripts touching the same entity therefore resolved
  in an order the allocator chose — under an engine whose headline contract is same-platform
  bit-identical replay. It is a `BTreeMap` now, so the order is a property of the scripts' paths.
- **Scripting: sixteen commands were accepted from Lua and silently discarded.** Thirteen scene,
  dialogue, race and camera variants were matched by an arm whose body was empty and whose comment
  said they would "already appear in `unhandled`" — they could not, because that arm consumed them
  before the catch-all could see them. The three vehicle commands had empty bodies of their own.
  All sixteen now fall through and are returned to the host, which is what the comment always
  claimed. Applying the vehicle ones needs `VehicleController` from `gizmo-physics-dynamics`, which
  this crate deliberately does not depend on; the host that flushes them does.
- **Scripting: one script's runtime error cancelled every script after it.** The update loop
  propagated the first `Err` with `?`, so a single throwing script silently stopped the rest for
  that frame — and now that the order is stable, "the rest" is a stable and therefore reliably
  silent set. Failures are collected and reported together; a broken script loses its own frame and
  nobody else's.

- **Cascaded shadow maps: texel snapping did not stabilise anything, and an overhead sun produced
  NaN.** Two defects in `directional_cascade_view_projs`, both found by measuring rather than
  reading. The snap worked on an **AABB of the frustum-slice corners in light space**, whose extent
  changes as the camera turns — so the texel size changed every frame and the grid moved out from
  under the snap — and it was expressed in a light basis rebuilt per cascade around a
  camera-following centre, so the grid translated with the camera as well. Measured on a static
  world point: its sub-texel phase changed on every frame (`.188`, `.872`, `.554`, `.234`, …) and
  it moved in fractional steps (−2.3162, −2.3179, …), which is shadow edges crawling. The fit is
  now a bounding **sphere** (rotation-invariant, so the texel size is constant) in a single
  world-fixed light basis; the same measurement now shows a constant phase and whole-texel steps.
  Separately, `Mat4::look_at_rh` was given `Vec3::Y` as its up vector unconditionally, so a light
  direction of `(0, -1, 0)` — noon — made the basis degenerate and produced a cascade matrix of NaN
  end to end. Both are pinned by tests in `csm.rs`, together with a third that pins the agreement
  between the two halves of cascade selection — the split comparison lives in the shader and the
  fit lives in Rust, and a fragment landing outside its own cascade does not error, it silently
  reads as fully lit. The sphere fit costs resolution and the figure is recorded next to it:
  texels grow **1.44-1.52x** (4.3 mm/texel on the nearest cascade, 59 mm on the farthest), which
  is the right way round — a crawling edge is far more visible than a texel half again as wide.
- **Full anisotropy read back as none whenever subsurface was above 0.16.** The three PBR extras
  ride in one `f32` as three-digit decimal fields, and nine digits do not fit: an `f32` is exact on
  integers only to 2^24 = 16 777 216. Past that the step exceeds 1, so anisotropy's clamped 999
  rounded to 1000 and **carried into the clear-coat field** — anisotropy 0.0 plus a phantom
  clear-coat, and with clear-coat also at its endpoint the carry reached subsurface. That is the
  exact overflow the existing `.min()` clamps and their regression test were written to stop; f32
  rounding reintroduced it higher up the range, where the test — which only ever tried subsurface
  0 — was not looking. The fields are two digits now: packed values top out at 999 999, seventeen
  times under the limit, exact for every combination, at the 1 % resolution subsurface always had.
- **The G-buffer's albedo target was linear 8-bit, so dark materials collapsed together.** Linear
  `Rgba8Unorm` spends its codes where the eye has least: the perceptual range 0–32/255 gets **4
  codes** against an sRGB target's 32, and the first linear code already sits at a perceptual
  12.7/255. Measured through the real pipeline, albedo 0.004 and 0.0045 rendered **byte-identically**
  — the same material as far as the renderer was concerned. The target is `Rgba8UnormSrgb` now:
  same four bytes, hardware transfer function on write and read, alpha (metallic) untouched.
- **Subsurface and anisotropy were not separable where they share a G-buffer channel.** `.w` of the
  world-position target carries subsurface in its integer part and anisotropy in its fraction, and
  the pack omitted the `floor` — so unless `100 · subsurface` happened to be an integer, subsurface's
  own fraction landed in anisotropy's slot. Not noise but inversion: subsurface 0.234 with
  anisotropy **0** decoded as **0.82**, and with anisotropy **1** as **0**. A material with no
  anisotropy rendered as though it were fully anisotropic.
- **Three shaders read the world-position G-buffer without putting it back into world space** —
  TAA's reprojection, SSAO's kernel projection and distance comparisons, and SSGI's temporal
  reprojection. All three bound that target as `t_position` rather than `t_world_position`, so none
  was among the readers the camera-relative change updated. The binding is now called
  `t_position_rel_camera` in every shader that takes it, so the convention is in the name and the
  next change to it can be found by grep rather than by luck.
  Introduced by the camera-relative change above and caught by a new convergence test rather than by
  eye: `taa.wgsl` binds that target as `t_position`, not `t_world_position`, so it was not among the
  readers the change updated. The history was sampled from wherever the camera stood relative to the
  origin, never matched, and the neighbourhood clamp dragged it back to the jittered current frame
  every frame — a completely still scene swung **33–35/255** between frames, against 18 with the
  reprojection correct and **0** with TAA switched off.
- **The G-buffer held absolute world positions in half floats, so rendering degraded with distance
  from the world origin.** f16 quantises to 6 cm at 100 m, 50 cm at 1 km and a full metre at 2 km,
  against a nearest-cascade shadow texel of 4.3 mm — a city-sized level sampled its shadows, view
  vector and height fog from a position rounded to the nearest half-metre. Measured: translating an
  entire scene 2 km from the origin changed **9.7 %** of the frame. The target cannot be widened
  (the four attachments share a 32-byte-per-sample budget and are at 28), but the budget is about
  the bytes, not what goes in them: the position is stored **relative to the camera** now and every
  reader adds it back, which keeps the same eight bytes and puts the values at view scale, where
  f16 is good to centimetres anywhere in the world. No uniform changed — the decal pass folds the
  camera translation into its inverse-model matrix instead.
- **Point-light shadow bias grew as the square of the distance from the light.** The cube lookup
  compares a *perspective* depth, whose derivative falls off as 1/d², so the fixed `0.0005` NDC
  bias was 5 mm a metre from the light, 50 cm at the edge of a 10 m light, **1.99 m** at the edge
  of a 20 m one and **12.5 m** at the edge of a 50 m one — surfaces out near a large light's range
  received no shadow at all, because the comparison sat metres past every caster. Now expressed in
  metres and converted with the local derivative, using the same constant as the cascade path:
  it is the same physical question and one engine should not answer it two ways. (Point shadows
  are off by default, so this changes nothing until `point_shadows_enabled` is set.)
- **Two more shadow paths still carried a raw NDC bias, and widening the caster reach quadrupled
  what it meant.** `baked_lit.wgsl` — the path the flagship city is drawn with — and
  `volumetric.wgsl` both sampled the cascades with a flat `0.0015`. Raising `CASTER_REACH` from
  60 m to 500 m took the projection's depth range from ~105 m to ~545 m, and with it that bias
  from **0.16 m of peter-panning to 0.82 m**. Both now express it in metres and convert with the
  cascade matrix's z gradient, restoring what the constant was worth at the range it was chosen
  under. All four shadow-sampling paths are on the same footing.
- **Tall shadow casters stopped casting as the sun climbed.** The cascade projection reserved 60 m
  of depth in front of its slice, so a caster rising above a receiver was clipped by the shadow
  projection's near plane and never reached the map — measured at **65 m with the sun 75° up**,
  90 m at 45°, 188 m at a low 20°. A building losing its shadow at noon is exactly backwards. The
  reserve is now a named `CASTER_REACH` of 500 m, which an orthographic projection can afford:
  shadow depth is linear, and `Depth32Float` over a kilometre still resolves a tenth of a
  millimetre. **The shaders' depth bias moved to metres in the same change and for the same
  reason** — it was an NDC constant, so it meant a different world distance in every cascade and
  would have grown from 4.2 cm to 22 cm of peter-panning as the range widened. It is now converted
  with the cascade matrix's own z gradient, exactly as the normal offset already used its x
  gradient.
- **Skeletal animation is advanced by the engine now.** `animation_update_system` and
  `animation_state_machine_update_system` existed in `gizmo-renderer` and nothing ever called
  them: `current_time += dt · speed` appeared nowhere else in the workspace, no schedule, plugin,
  app or demo invoked either, and `collect_draw_items` meanwhile read `Skeleton` for its skinning
  matrices every frame. A skinned mesh therefore rendered its bind pose for ever. `default_render_pass`
  now calls both, guarded by a golden render test that fails if the call is removed. The reason
  they were never scheduled is almost certainly their signature: both need a `wgpu::Queue` to
  upload skin matrices, which no ordinary system has.
- **`ParticleEmitter` entities emit.** The rest of the particle path was already wired —
  `default_render_pass` ran `update_params` and `compute_pass`, `passes/forward` drew the result —
  but the step that reads emitters and spawns from them lived only in `gizmo-studio`, so the engine
  stepped and drew a particle set that no scene ever populated. Now `systems::render::spawn_from_emitters`,
  called from the pass. Its jitter is a private seeded xorshift rather than `rand`, so replays
  reproduce their particles as exactly as their physics and the facade gains no dependency.
- **`LodGroup` is honoured by the engine's own render pass.** `collect_draw_items` consults it in
  both its component and asset-handle queries, with `gizmo-studio`'s semantics: the group overrides
  the entity's `Mesh`, and a distance past the last level culls rather than falling back to the
  coarsest. Previously only studio's pipeline looked, so a scene carrying three detail levels drew
  all three at every distance.

- **The hard brightness step at the end of the shadow cascades.** Past `SHADOW_DISTANCE`
  (100 m) there is no cascade to sample, so the shadow term snapped to "fully lit". With the
  baked-lit path flooring a shadowed fragment at `1 − 0.45 = 0.55`, crossing that boundary was
  a **1.82× brightness jump** — a bright band across the world, a couple of degrees below the
  horizon from a chase camera, with no distance falloff anywhere before it.

  The term now fades to unshadowed over the last `SHADOW_FADE_FRACTION` (15%) of the covered
  range instead of stepping, in both `baked_lit.wgsl` and `deferred_lighting.wgsl`. The maths
  lives in `csm::shadow_distance_fade` so it can be tested without a GPU, and the shader
  copies are pinned to the same constant. Cost: one `smoothstep` and one `mix` per shadowed
  fragment, no memory. The alternative — pushing `SHADOW_DISTANCE` out past anything the
  camera can see — spreads the same 4 × 3072² texels over a longer range and blurs exactly the
  near-camera contact shadows the cap exists to keep crisp.

  *Behavioural:* a fully shadowed fragment between 85 m and 100 m is now lighter than it was.
  That is the fix, not a side effect.

- **`baked_lit.wgsl` measured cascade depth radially.** It used `length(world_pos − camera_pos)`
  where `cascade_splits` and `select_cascade` are both defined along the camera's forward axis,
  as `deferred_lighting.wgsl` already had it. Radial distance overstates the depth of anything
  off-axis, so the shader picked a too-far cascade at the screen edges and put the end of the
  shadow range on a sphere rather than a plane — which would also have made the new fade band
  bend away from the cascade boundary it is supposed to hide.

## [0.9.0] — 2026-08-04

A correctness and honesty release. It carries no new features: it closes two paths to
undefined behaviour, one determinism hole, and the reason the facade only ever compiled
in a single configuration — all found by an external audit
([`docs/AUDIT-2026-08.md`](docs/AUDIT-2026-08.md)), and all with the evidence written
down rather than summarised.

**It is a minor bump rather than a patch because three public signatures changed.** Two
of them changed because their previous shape was the bug: a fracture API seeded from
thread-local entropy cannot be reproduced, and a `&self` method that mutates a Lua VM
cannot be `Sync`. Keeping the old signatures would have meant keeping the defects.

Upgrading from `0.8.0`:

- `generate_fracture_chunks(..)` takes a trailing `seed: u64`. Pass anything
  reproducible — an entity id, a frame counter — not `rand::random()`.
- `ScriptEngine::has_function` and `::run_entity_update` take `&mut self`. Callers
  reaching them through a `World` resource need `ResMut`, not `Res`.
- `gizmo_app::headless::App` and `gizmo_app::windowed::App` now coexist. `gizmo_app::App`
  still resolves to the windowed runtime when it is compiled in, so most code is
  unaffected; `headless::App::add_plugin` is unavailable when both are present.
- Several facade items are now behind the feature that actually provides them. A default
  build is unchanged; a `--no-default-features` build now compiles at all, which it
  previously did not.
- The library is now genuinely named `gizmo`. Previously the package was `gizmo-engine`
  with no `[lib] name`, so the real path was `gizmo_engine::` — and `use gizmo::prelude::*`,
  which is what the README, the crate docs and every example show, only compiled for
  someone who renamed the dependency in their own manifest. Copying the quickstart after
  `cargo add gizmo-engine` failed on line one. If you were using `gizmo_engine::` paths
  directly, either switch them to `gizmo::` or rename in your manifest
  (`gizmo_engine = { package = "gizmo-engine", version = "0.9" }`).

### Fixed — soundness

- **`unsafe impl Sync for ScriptEngine` was unsound and published.** mlua's `send`
  feature makes `Lua` `Send`; it never makes it `Sync`, because the `lua_State` is
  mutated through `&Lua`. `has_function` and `run_entity_update` did exactly that
  behind `&self`, and `ScriptEngine` is stored as a `World` resource — so two
  systems holding a shared reference could race on the interpreter. Both methods
  now take `&mut self`, which makes concurrent VM access unrepresentable rather
  than merely discouraged. **Breaking** for direct callers; there were none outside
  the crate.
- **`World::query_entity_mut` skipped its aliasing check.** Every other query entry
  point runs `check_aliasing` first; this one did not, so
  `query_entity_mut::<(Mut<T>, Mut<T>)>(id)` was safe code that handed out two live
  `&mut T` to the same row — undefined behaviour with no panic and no compile error.
  It now panics like the other paths, as does `query_entity` for symmetry.

### Fixed — animation

- **`Track::sample` panicked on a single-keyframe track with a `NaN`.** `idx.clamp(1, len - 1)`
  becomes `0.clamp(1, 0)` when the track holds one keyframe, and `Ord::clamp` asserts
  `min <= max`. It is reachable because a `NaN` — in the sampled time or in the track's own
  timestamps — makes both of `sample`'s early-return comparisons false, so control falls
  through to the clamp. The inline comment above that line described the clamp as the NaN
  guard; it was the thing that panicked.

  `Track::new` rejects non-finite timestamps, but `Track`'s fields are public, so a
  hand-built or deserialized track bypasses it. Found while documenting the crate for D4.

### Fixed — sleeping

- **Waking now travels a jointed mechanism in one step.** A sleeping body does not integrate,
  so a joint pulling on one has its correction silently swallowed and the mechanism looks
  broken. The wake propagation existed, but as a single pass over `world.joints` in array
  order: disturbing the deep end of a 12-link chain woke only 5 links in one step — one per
  substep — while the seven above kept absorbing joint corrections they never integrated, so
  the chain behaved as if pinned partway down.

  Physically this is not a subtlety: an inextensible chain loads every link the instant the
  bottom one is disturbed. Contacts have had the right answer all along — `island.rs`
  union-finds manifolds so a whole pile wakes together — but joints were never part of island
  construction. Wake propagation now runs over the joint-connected COMPONENT: one mover
  anywhere in it wakes all of it, and a component with no mover is left alone. Cost scales
  with the joint count, not the body count.

### Fixed — rollback

- **`WorldSnapshot` now carries `PhysicsWorld::weather`.** Same omission as the joint state
  below, one field further along.

  The rigid pipeline never reads `weather`, which is why it was easy to leave out — but the
  vehicle tyre model scales its friction-circle limit from it, and it cannot be recomputed
  from transforms or velocities. Gameplay switching weather inside a rollback window therefore
  left the re-simulation running vehicles under the grip of a weather it had already rolled
  back past, invisibly: `state_hash` covers only transform, velocity and sleep state.

  Found by applying the inclusion rule written on `WorldSnapshot` when the joint state was
  added — the criterion is derivability, not size.

### Fixed — rollback

- **`WorldSnapshot` now carries joint state.** Rollback could not un-break a joint.

  `Joint::is_broken` is a one-way latch — nothing outside scene load ever sets it back to
  `false` — and `joints` was not in the snapshot at all. A joint that snapped inside a
  rollback window stayed snapped through the restore, so the re-simulation ran without a
  joint the continuous simulation still had, permanently. The same applied to
  `initial_relative_rotation`, the reference pose latched on a joint's first solve, against
  which every cone/twist/swing limit is measured: a stale one silently redefines the joint's
  rest pose.

  Neither is visible to `state_hash`, which hashes only transform/velocity/sleep — so the
  desync stayed invisible until it bled into velocities. `tests/rollback.rs` gains two cases
  that fail on the old code, using a rope that goes taut (and breaks) at tick 38, inside a
  snapshot window opened at 20.

  The criterion for what belongs in `WorldSnapshot` is not size but derivability: anything
  that cannot be recomputed from `transforms`/`velocities` has to be in it, and its absence
  cannot be caught by the hash. That rule is now written on the type.

### Fixed — determinism

- **`generate_fracture_chunks` seeded itself from thread-local entropy**, so
  identical inputs produced different chunk geometry, masses and spins. Any scene
  that fractured diverged between replays and desynced under rollback. It now takes
  an explicit `seed: u64`. **Breaking**: the parameter is required, because keeping
  the old signature would mean keeping the bug. The ECS fracture path
  (`shatter_entity`) was already deterministic and is unaffected.

### Fixed — feature composition

- **`gizmo-engine` compiled in exactly one configuration.** Its own advertised
  `headless` feature failed with 37 errors, and `--no-default-features` plus every
  single-feature build failed too — so the README's headless-server story was dead
  code. The facade's modules are now gated on what they actually use.
- **`gizmo-physics-core` is now a mandatory dependency of the facade.** It defines
  `Transform`, `GlobalTransform` and `Collider`, so gating it behind `physics` also
  broke `--features render` and `--features audio`: nothing can be drawn or
  spatialised without a transform. `physics` now gates the *simulation*
  (`gizmo-physics-rigid`), which is the honest split.
- **`gizmo-app`'s `window` feature was non-additive** — enabling it *deleted*
  `headless::App`. Since Cargo unifies features across the whole graph, an unrelated
  dependency turning `window` on could silently swap a simulation server's `App`
  type out from under it. Both runtimes now coexist and are reachable by path; the
  root re-export still prefers the windowed one, so existing code is unaffected.
  `headless::App::add_plugin` is unavailable when both are compiled in, because
  `Plugin::build` is typed against the root `App`.
- A `feature-powerset` CI job (`cargo hack --depth 2`) now covers both entry crates.

### Added — supply chain and process

- `deny.toml` and a `cargo deny` CI job covering advisories, licences, sources and
  duplicate versions. Every exception carries a written justification. The first run
  fixed one advisory (`crossbeam-epoch` → 0.9.20) and documented five, including
  `bincode 1.x`, which is a direct dependency of `gizmo-net` and is tracked for
  migration.
- MPL-2.0 (via `rodio` → `symphonia`, on the default `audio` feature) is now
  disclosed explicitly rather than sitting unremarked under a flat "MIT OR
  Apache-2.0".
- `CONTRIBUTING.md`, `SECURITY.md`, `.github/dependabot.yml`.
- `docs/AUDIT-2026-08.md` — an external review with every finding pinned to
  `file:line` — and `docs/FIXPLAN.md`, which tracks the work it opened.

### Fixed — rendering

- **Six point-light shadow passes were recorded every frame into a cubemap nothing
  sampled.** `Renderer::point_shadows_enabled` defaults to false and
  `deferred_lighting.wgsl` already gated its lookup on the uniform written from that same
  bool, but `record_shadow_passes` did not — so a lit batch spent twelve of its
  twenty-three draws filling a 1024×1024×6 depth cubemap for nothing. Both sides now read
  the one flag. A golden-image test renders the scene with it on and off and demands
  byte-identical frames, which is both the claim (the skipped work was unobserved) and the
  guard against gating a pass the shader really does sample.

### Fixed — other

- The two golden-image GPU tests are serialised behind a mutex. Each requested its
  own wgpu device, and `cargo test` runs a binary's tests in parallel — concurrent
  device creation surfaced as an intermittent `SIGSEGV` inside the driver that took
  down the whole workspace run.
- `car_demo` and `wind_tunnel` loaded models from absolute paths into the original
  author's home directory and unwrapped the result, so the demo the README tells
  people to run panicked for everyone else. Optional assets now resolve through
  `demo::assets` and fall back to procedural geometry.
- README feature claims corrected: there is no Sweep-and-Prune broadphase (it is a
  dynamic AABB tree, and single-threaded), no `gizmo-physics` crate, no mimalloc in
  `gizmo-core`, and no Doppler in `gizmo-audio`. Determinism — the one property here
  with no equivalent in Rapier or Avian — is now stated, having previously gone
  unmentioned.

### Added

- **Ergonomics (DX).** `Prefab` — a define-once / spawn-many blueprint (mesh +
  material + optional `RigidBodyBundle`) with `spawn` / `spawn_at` /
  `spawn_with_mass` + per-instance `with_pbr`. `AutoBoxCollider` — derive a box
  collider from an entity's `Transform.scale` so the size is authored once
  (opt-in marker + a synchronous `Prefab` path). Auto-despawn lifetime
  components (`DespawnAfter` / `DespawnBelowY` + `LifetimePlugin`), `FpsLook`
  mouse-look camera controller, `World::despawn_all_with::<C>()` bulk despawn.
- **Tooling.** Broad unit-test sweep (~1376 tests across the workspace);
  structured `tracing` logging (instrument spans + fields) across the value
  crates, with silent error-swallows promoted to `warn!` / `error!`.

### Changed

- **Docs.** Consolidated 12 planning / fix-plan documents (roadmap, releasing,
  determinism, migration, architecture, and the finished FIX-PLANs) into a
  single [`docs/ENGINE.md`](docs/ENGINE.md); `README` / `CHANGELOG` /
  `demo-web/README` stay standalone.

### Fixed

- **Physics — resting-stack stability.** A settled box stack that spontaneously
  gained energy and blew up (lateral buckling) is fixed by a manifold **block
  solver** (coplanar normals solved jointly + Tikhonov regularization) plus
  **full warm-start** (`warm_start_factor` 0.85 → 1.0) — stable to N≤32 (was
  ~N≤16); N≥48 towers remain open. See [`docs/ENGINE.md`](docs/ENGINE.md) §7.
- **Rendering — 6 latent bugs.** World tangent (plain model 3×3, not
  inverse-transpose); PBR param-packing overflow at 1.0; ECS query
  `get` / `contains` now honour table-storage `With` / `Without` filters
  (matched `iter`); shadow-caster instance ordering (two-region layout); glTF
  `AlphaMode::Mask` cutout (alpha-cutoff discard).
- **Physics — perf.** Quadratic costs removed (broadphase pair dedup
  O(P²)→O(P); per-island TGS scratch sized to the island; per-contact constants
  hoisted out of the sweep loop): worst frame 262→46 ms on a 2000-box scene.
- **App — GPU robustness.** Surface `Outdated` / `Lost` now reconfigures the
  swapchain and backs off (rate-limited) instead of freezing or busy-spinning;
  `CloseRequested` shuts down gracefully (runs `Drop` → clean wgpu teardown)
  instead of `process::exit(0)`.

## [0.8.0] — 2026-07-12

A large feature release gathering ~205 commits since `0.2.0`. The whole
workspace continues to ship at one uniform `0.x` version (the staged `1.0`
model in [`docs/ENGINE.md`](docs/ENGINE.md) remains the planned later path). No
crate-level API is promised stable yet; treat any change as potentially
breaking and pin an exact `=0.8.0` if you need reproducibility.

### Added

- **Physics — joints.** First-class `Distance`/`Rope` joint; a generic 6-DoF
  (`D6`) joint with per-axis motors + springs; cone-twist, slider suspension,
  and hinge torsional-spring joints; per-joint compliance, asymmetric cone
  limits, distance reachability, spring-break, and servo motors.
- **Physics — bodies & vehicles.** Consolidated vehicle simulation in
  `gizmo-physics-dynamics` (dynamics is now canonical; the dead rigid vehicle
  path was removed); ECS systems for vehicle/character + ragdoll runtime;
  opt-in aerodynamic drag (½ρCdAv²) for rigid bodies; CCD exposed via bundle
  builders with analytic test ladders; `RigidBodyBundle` derives rotational
  inertia from its collider.
- **Physics — soft bodies & water.** Hardened cloth ↔ rigid-body collision
  (capsule, per-segment edge, averaged push) plus cloth tearing; a Subnautica-
  style water system (`water_at` query, swimming controller, Gerstner waves,
  underwater camera fog) and character oxygen.
- **Physics — ergonomics.** Fluent builders for materials, colliders, bodies
  and bundles; `PhysicsPlugin` auto-steps at the app's fixed timestep;
  `GameplayPhysicsPlugin` registers vehicle/character systems.
- **Rendering.** Textured PBR (normal / metallic-roughness / emissive / AO
  maps); distance-based texture streaming wired end-to-end; AAA smoke VFX
  (soft particles, flipbook, curl-noise, lit) with volumetric ray-marched
  smoke; headless/offscreen renderer (no window/surface); HighPerformance GPU
  adapter preference.
- **Web / WASM.** The deterministic simulation core compiles to `wasm32`, and
  the full engine runs in the browser (WebGPU/WASM) with an audio backend and
  a hardened web surface.
- **Animation & glTF.** Two-bone IK + FABRIK, cubic-Hermite scale tracks;
  `KHR_texture_transform`, `KHR_materials_emissive_strength`, and glTF sampler
  settings honoured.
- **Camera.** Orthographic projection mode (Numpad5 toggle) and
  `screen_to_ray` screen→world picking.
- **CI.** Run-once benchmark gate (and the engine bug it caught).

## [0.2.0] — 2026-06-25

The first release since `0.1.7`. It gathers the entire 1.0-readiness effort
(audit + hardening rounds) and the graphics-stack upgrade, shipped as a single
breaking `0.x` bump. **Upgrading from `0.1.x`? See the
[migration guide](docs/ENGINE.md).**

### Changed (breaking)

- **ECS query API split along the safe/unsafe boundary (closes a soundness hole).**
  `World::query::<Q>(&self)` previously accepted a *mutable* query (`Q = Mut<T>`)
  from a shared `&World`, so two live `Mut<T>` queries (or `Mut<T>` + `&T`) could
  alias the same storage — reachable from **safe code**, with no panic. The query
  surface is now:
  - `World::query::<Q: ReadOnlyQuery>(&self)` — **read-only** (`&T`, `With`/
    `Without`/`Changed`/`Added`, `Or`, and tuples of those).
  - `World::query_mut::<Q>(&mut self)` / `World::borrow_mut::<T>(&mut self)` —
    safe **mutable** access (requires `&mut World`).
  - `unsafe World::query_unchecked::<Q>(&self)` / `borrow_mut_unchecked::<T>` —
    escape hatch for code that only holds `&World` (e.g. inside the parallel
    scheduler's `System::run(&World)`), with a documented `# Safety` contract.

  Migrate by replacing `world.query::<Mut<T>>()` with `world.query_mut::<Mut<T>>()`
  (`borrow_mut` now needs `&mut World`); pure-read call sites are unchanged. On a
  `Query`, `iter`/`get`/`iter_chunks`/`par_for_each`/`entities`/`contains` are
  read-only; use `iter_mut`/`get_mut`/`iter_chunks_mut`/`par_for_each_mut` for
  mutation. Behavior is unchanged (determinism hash identical).
- **`RigidBody` lost its `friction` and `restitution` fields**, and
  `RigidBody::new` is now `new(mass, use_gravity)` (was
  `new(mass, restitution, friction, use_gravity)`). These fields were **dead**:
  the contact solver always sourced friction/restitution from the colliders'
  `PhysicsMaterial` (combined per contact), so setting them on the body did
  nothing — the editor inspector even exposed two no-op sliders. Configure
  contact friction/restitution on the collider material instead. Determinism is
  unchanged (proof the fields never affected the simulation). The scripting layer
  followed suit: the Lua `physics.add_rigidbody(id, mass, use_gravity)` binding
  and `ScriptCommand::AddRigidBody` dropped their (ignored) `restitution`/
  `friction` parameters.
- **Graphics stack upgraded** across the Stage B crates: `wgpu 0.20 → 29`,
  `winit 0.29 → 0.30`, `egui 0.28 → 0.34` (plus `egui-wgpu`/`egui-winit` `0.34`,
  `egui_dock 0.13 → 0.19`, `transform-gizmo-egui 0.3 → 0.9`). Public `wgpu`/
  `winit`/`egui` types in the renderer/window/editor/app/facade move to the new
  versions. See [`docs/ENGINE.md`](docs/ENGINE.md) (§6).
- **`bevy_reflect` is now gated behind an off-by-default `reflect` feature** on
  `gizmo-core`, `gizmo-physics-core`, `gizmo-physics-rigid`, and `gizmo-scene`.
  With default features, scene save/load + snapshots fall back to plain `serde`
  (every reflected component also derives `Serialize`/`Deserialize`), and
  `bevy_reflect` no longer appears in the default public API or — after the
  `gizmo-math` dependency-hygiene fix below — in the Stage A dependency tree.
- **`CollisionEvent.contact_points`** is now an opaque `ContactPoints` newtype
  (`gizmo_physics_core::collision::ContactPoints`) instead of leaking
  `arrayvec::ArrayVec`.
- **96+ public enums/structs marked `#[non_exhaustive]`** (error/shape/event
  enums and `Default`/constructor-guaranteed config structs) so future variants
  and fields are not breaking. Closed leaf math/config types are intentionally
  exempt.
- **Many constructors/loaders now return `Result`/`Option`** instead of
  panicking (`spawn_gltf`, `ComponentRegistry::register`, `SceneData::save/load*`,
  `AudioManager::new/play*`, `NetworkClient/Server::new`, `AppWindow::new`,
  `App::run`, renderer `load_*`, …), and 13 concrete error enums were added.
- **Infallible plain-value getters dropped the `get_` prefix** (`get_neighbors →
  neighbors`, `get_entity_component_types → entity_component_types`,
  `get_log_version → log_version`, `get_engine_torque → engine_torque`,
  `get_entity_names → entity_names`). Fallible `get_*` accessors that return
  `Option`/`Result` keep the prefix, following the usual Rust convention.
- **MSRV raised to `1.92`** (floor set by `egui 0.34`), up from `1.89`. Enforced
  by a CI `msrv` job. Earlier in the cycle the MSRV was empirically set to `1.89`
  (1.82/1.85 fail on transitive `crypto-common`/`wide`/`safe_arch`).
- **`glam` is now re-exported directly** (`pub use glam::{…}` in `gizmo-math`)
  and documented as an official public dependency, rather than via `bevy_math`.

### Added

- **The engine now runs in the browser (WebGPU/WASM).** `gizmo-renderer`,
  `gizmo-app` and the facade build for `wasm32-unknown-unknown` with a web
  feature subset, using a reduced 4-bind-group forward pipeline (browser
  `maxBindGroups = 4`; shadows/deferred/compute disabled on wasm). The new
  `demo-web/` crate (wasm-bindgen + `index.html`) shows a live physics scene in
  the browser and was verified end-to-end in headless Chrome. `gizmo-app`'s wasm
  `resumed` implements the async WebGPU init via `spawn_local`; `gizmo-scripting`
  (mlua) is target-gated to native, and the CI `wasm` job now also builds the
  graphics stack. Audio/networking/scripting remain native-only (RELEASING §4g).
- Deterministic same-platform **rollback netcode** (`gizmo-net`, `rollback`
  feature): `PhysicsWorld::snapshot`/`restore_snapshot` (full internal state incl.
  contact warm-start), a `Transport` trait with real-UDP and loopback impls, and
  a GGPO-style `RollbackSession` that converges under lag + packet loss.
- `PhysicsWorld::state_hash()` sync-hash API (process-stable) for desync
  detection and replay, plus a cross-process determinism oracle.
- **TGS Soft constraint solver** (Box2D-v3-style) for stable tall/high-energy
  stacks, with dormant-pair narrow-phase skipping for wide settled scenes.
- Continuous collision detection (CCD) hardening (no tunnelling), full joint
  library behavioural coverage, island-aware sleeping, and a phase-timed
  `PhysicsMetrics` profiler.
- Property-based and differential test suites across ECS, collision, raycast,
  SAT, ABA/multibody, joints, soft-body, and fracture; a CI matrix
  (ubuntu/macos/windows), a ratcheted `clippy -D warnings` gate, and a headless
  determinism gate.
- `docs/ENGINE.md` (§4 staged-1.0 strategy) and this changelog.

### Fixed

- **`gizmo-math` dependency hygiene:** removed an unused regular `bevy_math`
  dependency that transitively pulled `bevy_reflect` into the Stage A *production*
  dependency tree even with the `reflect` feature off. `bevy_reflect` is now
  absent from the default Stage A tree.
- Numerous correctness fixes across the EPA/GJK contact pipeline, integrator
  (body-space inertia), split-impulse leakage, joint effective-mass, renderer
  mesh winding + skin-weight normalisation + skinned-normal inverse-transpose,
  and post-process depth linearisation (see git history for the per-round audit
  detail).
- **egui 0.34 / winit 0.30 deprecations migrated** off the crate-level
  `#![allow(deprecated)]` bridges left by the graphics upgrade: all mechanical
  egui renames, plus the top-level panel `show(ctx)` pattern migrated to egui
  0.34's root-`Ui` composition (`show_inside`). The only remaining (scoped,
  documented) deprecation is winit's closure `EventLoop::run`/`create_window`
  bridge in `gizmo-app`, whose `ApplicationHandler` migration is deferred.

## [0.1.7] — earlier

Initial published series (`0.1.x`) on crates.io: the ECS, math, physics
(rigid/soft/dynamics), renderer, editor/studio, audio, AI, scripting, and
client-server netcode that make up the engine. See the git history for details.

[0.2.0]: https://github.com/bdrtr/Gizmo/compare/v0.1.7...v0.2.0
[0.1.7]: https://github.com/bdrtr/Gizmo/releases/tag/v0.1.7
