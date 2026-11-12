# Gizmo Engine — where it stands

**Last updated: 2026-11-03.** One page, present tense. History, measurements and the reasons
behind decisions live in [`ENGINE.md`](ENGINE.md); this file only says what is true now and where
to read more. When something here stops being true, fix this page in the same commit.

Counts are deliberately absent (demos, tests, `#[non_exhaustive]` types…): every one this
repository has written down went stale. Count with the command, not from the page.

## What it is

A pure-Rust, ECS-driven 3D engine with its own physics (no third-party physics crate), a WGPU
renderer that also runs in the browser, and an egui editor. Same-platform deterministic: a world
hash replays bit-for-bit, which is what rollback netcode and the regression soaks stand on.

## What works today

| Area | State | Read |
|---|---|---|
| ECS (`gizmo-core`) | Archetype + sparse-set storage, borrow-checked queries, commands, events (`EventMutator` too), observers, hierarchy. Bevy's shape: `Query<D, F>`, `query_data!`, `add_systems(Startup, ..)`, `Changed` per system; `Stepping` and a sequential executor for debugging. Driven state machines (`App::add_state` / `on_enter` / `on_exit`, `DespawnOnExit`), system chains (`(a, b).chain()`), systems returning `Result`. Under an ongoing soundness sweep; each round has come back smaller. | ENGINE.md §3 (Phase 6), CAPABILITY_GAPS §C1 |
| Rigid physics | GJK/EPA + box SAT, TGS-Soft solver, CCD, joints, vehicles, character (pushes dynamic bodies, walks through triggers and lists the ones it is in), axis locks. Deterministic on one platform. A game with its own update hook steps it at a fixed rate with `gizmo::simple::FixedPhysics`; a steady push is an `ExternalForce`. Water (`FluidZone`) floats a body at its density's depth and rights a tipped hull. 2D: `RigidBody::planar_xy` + `Collider::{polygon_2d, polyline_2d, circle_2d, rect_2d}`; tile levels: `tilemap_collider`. | ENGINE.md §5, §7 |
| Soft physics | FEM soft bodies (`SoftBodyMesh::block`; sub-stepped to the material's stable step; they rest on the floor, stack on each other and are drawn), XPBD cloth and rope, Voronoi fracture (a `Breakable` breaks in the physics step; its debris is drawn in its material and expires). Cloth is drawn and two-way — it pushes rigid bodies back, and `Cloth::damping` brings what it holds to rest. | ENGINE.md §1 |
| 3D rendering | Deferred PBR, CSM shadows, SSAO, SSGI, SSR, bloom, volumetrics, TAA/FXAA/SMAA, decals, irradiance volumes (`IrradianceVolume`, the light the scene bounces, captured from each probe; one a scene), a game's own surfaces lit through the G-buffer (`MaterialType::CustomSurface`) and pipelines of its own (forward); shadows of solids, sheets and cut-outs from the sun and from every point, spot and rectangle light that casts (`casts_shadows`, one shared array); glass (`Material::with_transmission`, screen-space), a non-metal's reflectance (`with_ior`), an HDR panorama as the sky (`Renderer::set_environment_map`, desktop); hand-set fog (`Renderer::fog`) and the sky that lights the scene (`Renderer::sky`) beside the four presets. | CAPABILITY_GAPS §A, §F |
| 2D rendering | `Sprite` in world space (entity's plane, depth-tested) or screen space; sheets, flips, layers, pixel-art sampling. `ProjectionMode::OrthographicFit` keeps a playfield in view in any window shape; `gizmo::physics::sprite_collider` derives a box collider from a sprite; `SpriteAnimation` plays a sheet; `Tilemap` draws a level from one tileset in one draw call. | CAPABILITY_GAPS §A1 |
| Text and HUD | `Text` in world or screen space — absolute pixels or `Text::pinned` / `pinned_at` to a point of the window that follows resizes —, wrapping at `max_width`; screen text, screen sprites and `gizmo-ui` backgrounds are drawn **after** tone mapping, so a HUD keeps its colours. Font fallback (`add_font_fallback`); shaping and bidi, so Arabic, Hebrew and Indic scripts draw correctly given a face that has them. | CAPABILITY_GAPS §A2 |
| Game layer | What a finished game reaches for, each found by writing one (`kristal_avcisi`, `lobut`, `kale`, `poligon`, `kaydirmaca`, `menu`, `nisanci`, `mucevher`, `ritim`, `zindan`, `birlestir`, `duello`, `labirent`, `kosucu`, `komutan`, `pist`, `kopru`, `bostan`, `gizli`, and on 2026-11-03/04 `kapi`, `vinc`, `itis`, `atolye`, `golf`, `yilan`, `karinca`, `makas`, `file`, `kovala`, `arazi`, `kayik`, `jole`, `topcu`, `robot`, `pinbol`, `sarmasik` — every one with a headless `--test` mode, played by `scripts/game-tests.py` in CI): seeded `Rng`; `Timer`; held keys that repeat (`HoldRepeat`); strings in several languages (`Locale`, `LocalizedText`); typed-out text (`Typewriter`); screen shake (`CameraShake`); fades and flashes (`ScreenFade`); music fades (`AudioManager::crossfade`) and the playing position (`position`, `seek`); `Tween`/`Ease` (with delays); `SaveData` between runs; the world under the mouse and the pixel over a world point (`gizmo::pointer`, `world_to_screen`); UI clicks, keyboard/gamepad focus (`UiNavigation`), clipping (`ClipContent`), layers (`ZIndex`), scrolling (`Scroll`), drags (`Drag`), grids (`Display::Grid`), sliders (`Slider`), progress bars (`ProgressBar`), switches (`Toggle`), select lists (`Dropdown`), touch sticks and buttons (`VirtualStick`, `VirtualButton`), text fields (`TextInput`, `Input::typed_text`) and `pointer_over_ui`; cursor grab, fullscreen and quitting (`CursorOptions`, `WindowMode`, `AppExit`); spawning without a GPU (`MeshSource` primitives, `MaterialDesc`, `MeshData`); `Commands::spawn_bundle`, tuple and `Option` system/query params; playing a game headlessly for its `--test` (`HeadlessFrames`). | CAPABILITY_GAPS §C9–§C17 |
| Audio | `AudioManager`: flat and 3D sinks, buses through `Mixer`, an underwater muffle, reverb (`set_reverb`), and on the whole mix an equalizer (`set_equalizer`), a compressor (`set_compressor`) and a limiter (`set_limiter`); sounds from files, embedded bytes, or generated in code (`audio::synth` — a WAV encoder and the `Blip` effect generator). | `gizmo-audio` crate docs |
| Browser | `wasm32` with a reduced forward pipeline (the sun's shadow, at 1024² a cascade, and no other; no AO/GI/TAA; its post pass has the exposure, the seven tone curves, dither, vignette and grain, and no bloom, depth of field, motion blur or chromatic aberration); audio works there, networking and Lua do not. Touch works there and on touchscreens: the first finger is the mouse, every finger is in `Input::touches`, and pinch, two-finger pan and swipe are read from them. | `demo-web/README.md` |
| Android | An installable APK (`scripts/android-apk.sh`, `demo-android`): a game from `games/` in a `NativeActivity`, on the browser's reduced render path (`gizmo_forward_only`), with touch, logical-pixel HUDs (`DisplayScale`), the covered screen edges (`SafeArea`), and the surface remade when the app comes back to the screen. `aarch64` only; one game per APK (`pinbol`, `nisanci`). | `demo-android/README.md` |
| Editor | `gizmo-editor` panels; `gizmo-studio` app (not published). A game's own components join the in-engine editor with `gizmo::register_editor_component`: inspected, edited, added, removed, saved. | README, CHANGELOG |
| Netcode | P2P rollback and client-server (`gizmo-net` features `rollback`, `client-server`). Rollback holds the peer that is ahead (`try_advance`) and rolls the game's own state back with the physics (`with_state`); `itis` is a two-player game on it, tested over real UDP. Client-server (facade feature `network-client-server`) has a wire format, send/receive calls, `PlayerState` for reconciliation and `Custom` messages; `kovala` plays on it. | ENGINE.md §1, §5 |
| Scenes | RON scene files with the game's own components (`full_scene_registry` + `register_serializable`); a whole world (`SceneData::save`/`load_into`) or part of one (`of_entities` → `to_ron_string`, `from_ron_str` → `instantiate`), joints included. `atolye` saves its level this way. | ENGINE.md §5 |

## What is missing — the gaps that block the most

Ranked and measured in [`CAPABILITY_GAPS.md`](CAPABILITY_GAPS.md); the short version:

- **2D game layer:** 2D physics is 3D shapes on bodies locked to the XY plane — by decision, no
  separate 2D solver (§C9); one-way platforms with `Collider::one_way`. A side-on character is the
  kinematic `CharacterController` with `target_velocity.x` — it jumps up through one-way platforms,
  lands on them and drops off them (`drop_through_timer`), and stays in its plane.
- **Rendering:** planar mirrors and reflecting water are `PlanarReflection`, one extra scene draw each per
  view that sees them (split screen and texture cameras included) and sharp apart from the waves (§A7d); reflection probes — up to four, box parallax, blended, transparent surfaces
  included on native (§A5) — no MSAA — by decision, TAA +
  FXAA (four quality presets) or SMAA 1x, + CAS sharpening instead (§A6), area lights
  are sphere, tube and rectangle, and spot lights take cookies (§A7: `source_radius` /
  `source_length` / `RectLight` / `SpotLight::cookie`, cookies native-only. Soft sun shadows (on
  every lit path) and contact shadows (deferred, every light) are `DirectionalLight::angular_radius` /
  `contact_shadow_length` (each light has its own); a local light's
  shadow is `casts_shadows`, as soft as its source is large), occlusion culling is opt-in (`Occluder`, §A7b), motion blur
  covers camera and objects but not transparent ones (§A7c, `PostProcess::motion_blur`; auto exposure
  is `renderer.auto_exposure`). Split screen works on the game path (`Camera::viewport`, §A5b), and a camera draws into a texture of the game's own with `CameraTarget` (on both paths, §A5b); the studio previews a
  split-screen game through its primary camera. Transparent materials blend by
  `Material::blend_mode` — alpha, additive, multiply, premultiplied (§B).
- **Text:** in vertical text Latin lies on its side (`.upright()` stands it); `Renderer::load_system_font` finds the machine's font (`GIZMO_FONT`, then the platform's usual faces); the one font the engine carries is Noto Sans (OFL 1.1) behind the default-off `bundled-font` feature, for the browser, which lends none; and `add_font_fallback` chains faces per character (§A2).
- **Extensibility:** a game's own material is a surface the engine lights (`Renderer::add_surface_material`,
  `MaterialType::CustomSurface`: its WGSL `material_surface` in the engine's G-buffer and lit shaders, so drawn deferred with
  SSAO, SSR and decals, and forward wherever a draw is forward) or a pipeline of its own (`MaterialType::Custom`, forward; §A3).
  Composites and named query terms go through `system_param!` and `query_data!`; a primitive parameter is an
  `unsafe impl SystemParam` and a primitive query term an `unsafe impl WorldQuery`, both written downstream as in Bevy (§C0). Components can be defined at runtime
  (`World::register_component`, `DynamicQuery`, `DynamicSystem` — §A8), and system bodies hot-patched while the game runs
  (`hotpatching` + `dx serve --hot-patch`, §A8). `SimpleApp` and `with_scene_render` draw from a render world (`RenderWorld`): the game path's
  draw (`draw_frame`) reads only a copy, after `prepare_frame` has done the frame's game-visible work, kept by `render_extractor` — components,
  resources (copied when written; resources carry change ticks) and asset maps. The `Renderer` is not a game-world resource — the game side has a `GpuContext` that creates and never writes what the draw reads. A render hook can be a `RenderStage` — `prepare` with the world at the sync point, `draw` without it — and `with_scene_render`/`SimpleApp` are; a stage draws on a render thread by default on native targets, overlapping the next update, with the loop pacing itself so the overlap does not age the input (`App::with_render_thread(false)` keeps the loop's thread; §A8). What a game adds to the frame goes on the stage: `RenderWorld::with_settings` sets the renderer from the game at the sync point, `with_pass` draws a pass of its own over the finished frame; every demo but `fluid`, a renderer of its own, draws this way (§A8).
- **Editor:** `gizmo-studio` is a separate binary and cannot see a game's types; the in-engine
  editor can.

## Where things are written down

| Need | Document |
|---|---|
| Build, test, lint gates, machine limits | [`CLAUDE.md`](../CLAUDE.md) |
| Architecture, crate layering | ENGINE.md §2 |
| Open work, with measured state per item | ENGINE.md §3 |
| What may appear on a public API, and what is sealed | ENGINE.md §4 |
| Determinism contract | ENGINE.md §5 |
| Refuted bugs and rejected ideas — **read before chasing one** | ENGINE.md §7 |
| How work is done here (fix → regression test → gates) | ENGINE.md §8 |
| What a game reaches for and does not find | [`CAPABILITY_GAPS.md`](CAPABILITY_GAPS.md) |
| The shape the public API should grow into | [`API_DEPTH.md`](API_DEPTH.md) |
| The 2026-08 independent audit (dated evidence, not a plan) | [`AUDIT-2026-08.md`](AUDIT-2026-08.md) |
| What changed, release by release | [`CHANGELOG.md`](../CHANGELOG.md) |

## A first hour

```bash
cargo run --release -p demo                     # 3D scene: PBR + physics
cargo run --release -p demo --bin kule_yikici   # a small complete 2D physics game
cargo run --release -p demo --bin sprite_2d     # the sprite path on its own
cargo run --release -p demo --bin headless_stress_test   # the determinism gate
```

Then `demo/src/bin/` — each demo's header says what it shows and, measured, where the engine
falls short.
