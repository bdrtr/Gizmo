# Changelog

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
  that crate's default binary — `bevy_3d_scene`, a fixed floor/cube/light/camera that opens no
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
  `Option`/`Result` keep the prefix, following the Bevy convention.
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
