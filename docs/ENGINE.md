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
> verification were removed). The resulting work was executed in `docs/FIXPLAN.md`, a deliberately
> temporary document. **That campaign closed on 2026-08-17 and the file is deleted, as its own
> header always promised.** Its durable content is here: unfinished work below, measurements and
> refuted candidates in §7, method in §8. The audit report itself stays — it is dated evidence,
> not a plan.

Phases 0–5 (stabilization, tests+CI, determinism, P2P rollback netcode, physics depth,
renderer/WASM/editor) are **DONE**. Remaining:

**Phase 6 — API hygiene (CONTINUOUS, not a milestone; restated 2026-08-17)**

This used to read "API stability & 1.0 mechanics" and hold three items waiting for a 1.0. Waiting
was the problem: two of the three had already been done and the list did not know it, which is what
a milestone does to work — it turns "not done yet" into "not looked at". None of it needs a release
to happen, so none of it is scheduled against one any more. Each item below carries its measured
state instead.

- **`unsafe` contracts — CLOSED (2026-08-17).** Every `unsafe` block in the workspace states why
  it is sound, and the lint that says so is a ratchet in **all 20 crates**:
  `#![deny(clippy::undocumented_unsafe_blocks)]`. `gizmo-core` was the last and the real one — the
  ECS holds most of the workspace's `unsafe` — and the arguments there are the three the storage
  actually rests on: the `Component: Send + Sync` bound behind the `Send`/`Sync` impls on
  `BlobVec` / `Archetype` / `ComponentSparseSet`, the caller-side no-aliasing contract behind the
  `UnsafeCell` column access, and row liveness behind every raw pointer.
- **"Freeze the public API" — dropped.** There is no freeze event to schedule. What that item was
  reaching for is §4's external-type contract, and that is enforced continuously: every dependency
  on a public surface is listed there with its cost, and `crates/gizmo/tests/crate_staging.rs`
  fails the build if the crate graph drifts from it.
- **Staged 1.0 — not being pursued as a milestone.** The crates keep one workspace version and
  stay on `0.x`. §4 stays exactly as it is, because its *rules* are what keep the surface honest;
  what is dropped is the release event those rules were being saved for. A 1.0 can be declared
  later, from a surface that is already clean, which is the only way it was ever going to be
  worth declaring.
- **`RigidBody::friction`/`restitution` — closed; the item was stale.** The fields do not exist:
  friction and restitution come from the collider's material, and `RigidBody`'s own docs say so at
  the type and at `RigidBody::new`. Verified 2026-08-17.

**Phase 7 — Product layer (a shippable game)**
- M7.4 authoritative client-server netcode; M7.6 UI font/text/widget/z-index; M7.7 WASM feature
  parities, editor panels + AssetServer hot-reload.

- **M7.5 — the mixer landed 2026-08-18; the DSP half has not.** `gizmo_audio::Mixer` holds named
  buses (`music`/`sfx`/`ui`/`voice`, created on demand so a game's own names need no engine
  change), a master gain, mute flags that keep their gains, and the environment modifier. Buses
  are the half a game ships: two sliders in a settings menu, applied to sounds that are *already
  playing*. `AudioSource::bus` carries the choice into scene files, with `#[serde(default)]` so a
  scene written before buses loads onto the default one.

  **What made it a fix rather than a feature is what the buses displaced.** Every modifier used to
  write *into* the player's volume: the underwater muffle multiplied by `0.4` and undid itself with
  `2.5`, while `audio_spatial_system` overwrote the same field with `attenuation × source.volume`
  every frame. Two writers, one field, no memory of what the game had asked for. Measured on real
  hardware before the change:

  | step | volume | speed |
  |---|---|---|
  | 3D sound playing | 1.00 | 1.00 |
  | `set_underwater(true)` | 0.40 | 0.85 |
  | **one frame of `audio_spatial_system`** | **1.00** | **1.00** |
  | `set_underwater(false)` | **2.50** | 1.00 |
  | 2D sound set to 0.22 while submerged, then surfaced | **0.55** | 1.00 |

  So the muffle was unreachable for every 3D sound in the engine — it lasted until the next frame,
  which is to say it never happened — and surfacing multiplied by 2.5 regardless, which a `Player`
  obeys: 250 % of the volume asked for, out of a speaker. The documented symptom was "a slight
  drift on surfacing".

  Volume is **composed** now, never accumulated: `route.volume × bus × master × environment`, and
  `speed = route.pitch × environment`, recomputed whenever any input moves. The order the modifiers
  arrive in stops mattering, which is the property that was broken — and it is asserted both in the
  mixer's own tests and, because the composition is worthless if a call site still writes its own
  level, on a real device (`the_device_gets_the_mixers_numbers_and_nothing_accumulates`, `#[ignore]`d
  like every hardware test here).

  Three smaller things fell out of the same shape. The pitch clamp that keeps rodio's sample-rate
  converter out of its `from >= 1` assert had to move **after** the environment multiply — an
  already-clamped `0.01` times the underwater `0.85` is `0.0085`, i.e. the assert, on the audio
  callback thread. Gains are clamped where they are written (negative is a phase flip, `NaN` reaches
  the device as `NaN` samples), and the sink garbage collector now drops routes with their sinks,
  which it must: one leaked route per sound ever played is one per footstep.

  **The first piece of the DSP half landed the same day: a real low-pass** (`gizmo_audio::filter`).
  The muffle's stand-in was a 0.85× playback speed, and the reason it was a stand-in is written into
  rodio's API: once a source is `append`ed the `Player` owns it, so `BltFilter::to_low_pass` cannot
  be reached from outside. But a slow-down is a **pitch shift** — a looped engine drops a tone, a
  music track detunes — and none of that is what water does.

  The way past it is not to reach into the player but to hand it a source that reads its own
  parameter: `Muffle` wraps the decoder and loads an `Arc<AtomicU32>` cutoff per sample, so
  `set_underwater` is **one atomic store** heard by every sound at once, including ones started
  later. The playback-speed term is gone from the composition; the corner is 700 Hz.

  **The biquad's state is per channel, and that is the part not borrowed from rodio.** rodio's
  `BltFilter` keeps a single `x_n1/x_n2/y_n1/y_n2` set and runs an *interleaved* stream through it,
  so on stereo the left channel's history filters the right channel's samples. The test that pins
  the difference is `a_silent_channel_stays_silent`: signal left, silence right, and the right
  channel must come out *exactly* zero — which it cannot on a shared-state filter.

  Measured rather than asserted by construction (Butterworth, Q = 1/√2, 48 kHz, corner 800 Hz):
  **100 Hz passes within 1 dB, the corner is −3 dB, and 6.4 kHz — three octaves up — is below
  −30 dB.** Bypass (`cutoff = 0`) is *sample-for-sample identical*, since a game is bypassed for
  its whole run except while submerged. And an absurd cutoff cannot poison the stream: unclamped,
  a corner at or above Nyquist makes the bilinear transform produce a `NaN`, which then lives in
  the filter's own feedback path and turns **every** later sample into `NaN` — silence on the
  device, from a number a game is allowed to ask for.

  **Still not done:** sends/returns (a reverb bus fed from other buses) and a per-bus opt-out from
  the environment modifier, since today the menu music muffles when the camera goes under water.
  **Trigger for the opt-out:** a game with non-diegetic sound that dives. **Trigger for starting a
  sound already on its bus** (rather than `play` + `set_sink_bus`): a game that can hear the gap,
  which is one mixer buffer and audible only into a muted bus.
- Optional: cross-platform determinism (as a feature — see §5), gizmo-net on WASM.
- Gated on human-eye A/B: the textured-glTF `material_demo` asset, `car_demo` driving/geometry.

**The CI gates that were filed under "1.0" — decided, 2026-08-17.** They were one line
("rustfmt / `missing_docs` / coverage / cargo-deny / benchmark regression"), and two of them had
already shipped. Each is now either on, or off with a reason:

| Gate | State |
|---|---|
| **cargo-deny** | **ON** — a CI job since before this list was written (`Supply chain (cargo-deny)`, advisories + licenses + bans + sources). The roadmap line was stale. |
| **`missing_docs`** | **ON per crate, as a ratchet.** All 10 Stage A crates plus `gizmo-window`, `gizmo-ui` and — since 2026-08-17 — `gizmo-app`, `gizmo-analysis`, `gizmo-studio`, `gizmo-scripting`, the facade, `gizmo-editor` and `gizmo-renderer` are at zero and warn (CI's `-D warnings` makes that a deny). **The backlog is closed: every crate in the workspace is on the ratchet** — true since 2026-08-18, and this row claimed it a day early. `gizmo-audio` had never been given the attribute: it was *at* zero, so nothing was undocumented, but nothing was stopping the next `pub fn` either, which is the whole difference between a state and a ratchet. Counted, not remembered: 19 of 20 crates carried the line, and the sentence naming the sixteen that did was accurate — the crate it did not name was simply missing. Re-measured 2026-08-17, and the table was accurate at every step. `gizmo-app`'s 23 were the windowed and headless builders — `new`, `set_setup`, `set_update`, `add_system` and the rest of what every demo calls — so they were also the crate's whole docs.rs front page. `gizmo-analysis`'s 33 were mostly the public fields of `Stats`, `SpanSample` and `TraceRecord`, i.e. the units of every number it exports; `gizmo-studio`'s 40 were its module list, its per-frame systems and the `PrimitiveSize` table — which is also where a stale claim was found and removed, since that table still said the engine had no cylinder shape. `gizmo-scripting`'s 102 were three quarters `ScriptCommand`: 41 variants and 35 fields making up the entire vocabulary a Lua script has for changing the world, which is the crate's actual API and had no units written anywhere. `gizmo-editor`'s 281 were two thirds `EditorState` and its nested state types — 63 fields, 107 more in `state_types.rs` — which is where the editor's whole request protocol lives: a panel cannot mutate the world while egui is drawing it, so every action it offers is recorded as a field and drained by a system afterwards, and a request nobody drains is a menu item that silently does nothing. Documenting it also turned up a doc comment that had been on the wrong field since `ffd8c29`: `history` was described as "whether FXAA anti-aliasing is on", left behind when the FXAA flag moved into `PostProcessSettings`. `gizmo-renderer`'s 874 — the largest, and the last — split into two halves that wanted different things written. The GPU-facing structs (`gpu_types`, and the physics, fluid and particle type modules) needed the *reasons*, not the field names: every `_pad` field is the Rust half of an alignment gap naga inserts whether or not the struct does, so omitting one shifts every field after it in every shader; the render fields sitting inside simulation structs like `GpuBox` and `GpuParticle` are there because those buffers are also the instance buffers, with no CPU in between. The pass states (SSAO, SSGI, SSR, TAA, volumetric, the fluid's screen-space surface) needed the *shape*: which passes exist, which textures are rebuilt on resize, and why each split is a split — a blur that reads what it writes smears in one direction, a separable blur costs `2n` samples against `n²`, a composite pass cannot sample the target it is writing. **The renderer is also the crate that proved the ratchet is per-target**: see the note in §8. The facade's 168 were the front door itself — the bundles, `SimpleApp`'s scene builder, the `gizmo::physics` re-export tree and the colour constants — i.e. the crate a user actually types, and the one whose docs.rs page is the engine's. A crate joins the ratchet when it reaches zero; that rule is what stops the number growing while the backlog is worked off. |
| **benchmarks** | **ON as a smoke gate** (`cargo bench --benches -- --test` runs every criterion bench once, catching panics and broken bench asserts). A *regression* gate is **not** adopted: shared CI runners have no timing floor, so the threshold that avoids false alarms is wide enough to miss real regressions. Performance work here is measured deliberately instead (§7 has the record of what was measured and rejected). |
| **rustfmt** | **report-only, and staying that way.** Making it a gate means reformatting the tree first: **2794 hunks across every crate** (re-measured 2026-08-18; it was 2660 when this row was written, and it drifts upward as the tree grows — which makes the argument stronger over time, not weaker). That is one commit of pure churn against git blame, a conflict with anything in flight, and it buys a property clippy does not already give — the lint gate is the one that catches defects. The existing rule stands: don't reflow unrelated code, and leave the report on so drift stays visible. |
| **coverage** | **not adopted.** A percentage gate rewards tests that execute lines; this codebase's tests are written to fail when behaviour is wrong (the pixel readbacks, the soak horizons, the source-shape guards), and several of the defects found this month were in code with coverage and no assertion. The number would go up and mean nothing. |
<!-- TRANSLATOR NOTE: "İnsan-gözü A/B gated" is a terse fragment; read as "these items can only be signed off by a human comparing before/after side by side". -->

**Carried from the FIXPLAN campaign (retired 2026-08-17).** `docs/FIXPLAN.md` was the temporary
document that executed the 2026-08 audit; it is gone, as its own header always said it would be.
What it still tracked as unfinished is the list below — verified against the code on the day it
moved, not copied. What it *knew* is in §7 (measurements, refuted candidates, non-goals) and §8
(method).

- **Gamepad input — CLOSED (2026-08-17), native.** `car_demo`, `beamng`, `platformer` and a
  complete fighting-game input module existed and not one of them could be played with a stick.
  Now: pads live in `Input` next to the keyboard and mouse (`input.gamepad()`,
  `input.gamepads()`, named `GamepadButton`/`GamepadAxis`), `ActionMap` binds buttons and
  axes-past-a-threshold, and `gizmo_app::gamepad::GamepadBackend` (gilrs, default-on `gamepad`
  feature, native only) feeds them from the windowed loop.

  **Putting the state inside `Input` rather than beside it is the decision the rest follows
  from.** `Input` is what a replay records, what the loop clones into the world each frame, what
  focus loss clears and what `begin_frame` rolls — a separate `Gamepads` resource would have
  needed its own copy of every one of those, and the first one forgotten would have been the
  replay. The cost is a serialisation change, paid with `#[serde(default)]` and a test that loads
  a pre-gamepad recording.

  **Sticks are read through a radial deadzone, not a per-axis one** (`apply_stick_deadzone`,
  0.15 default, rescaled to a unit disc). A per-axis threshold leaves a *square* dead region:
  `(0.14, 0.14)` is a clear diagonal push that reads as dead centre, and `(0.16, 0.14)` snaps to
  pure horizontal. The magnitude clamp is the other half — square-gated hardware reports `(1, 1)`
  at full diagonal, magnitude 1.41, i.e. 41 % extra speed for running diagonally. Triggers get a
  separate, smaller deadzone (0.05); every hundredth there is throttle resolution.

  **Three findings from the device test, and none would have come from reading documentation.**
  gilrs does *not* emit `ButtonChanged` when an analog trigger crosses its press threshold — a
  trigger pulled from rest to the floor produces `ButtonPressed(RightTrigger2)` and nothing else,
  so the backend read a pressed trigger whose travel was still 0.0 and a driving game bound to
  `right_trigger()` got no throttle at all. The fix reads the value out of gilrs's own state on
  the button edge. And `Gamepads::first()` originally returned only *connected* pads, which
  quietly emptied the promise that an unplugged controller releases what it held: the release
  edges exist on the frame `input.gamepad()` had already gone `None`, so
  `if let Some(pad) = input.gamepad()` — the shape every game writes — could not see them. It now
  falls back to a pad living out its disconnect frame, and a still-connected pad always outranks
  one that is going away.

  The third came from driving `car_demo` with the virtual pad and screenshotting the frame: the
  car did not move, and `Gaz: 0.00` was on screen while the trigger was held. **gilrs reads
  nothing from a device when it opens it** — measured directly: immediately after `Gilrs::new()`,
  a pad holding its right trigger at maximum reports `button_data(RightTrigger2) == None` and
  `value(RightZ) == 0.0`, and the value appears only once an event moves it. Its state is built
  from the event stream, so "ask gilrs what the pad is doing" is not a thing that can be done.
  The backend therefore keeps its **own mirror** of everything it has seen, and `resync` replays
  that instead — which is what makes the focus round-trip work (Alt-Tab away with the throttle
  held, come back, still held). What it cannot fix is the launch case: a control already held
  when the game starts stays invisible until it moves, because nothing anywhere saw it happen.
  With the wobble that a held analog control needs on Linux anyway (the kernel drops an `ABS`
  write that repeats the current value), the same run read `Gaz: 0.98 | Fren: 0.00 |
  Direksiyon: -0.89`, 18.6 km/h — against `0.00 | 0.00 | 0.00`, 0.0 km/h, 800 RPM with no pad
  present. The whole chain, in pixels: uinput → gilrs → backend → `Input` → vehicle → HUD. (The
  control run is not a formality: the *first* one was taken while a virtual pad from the previous
  run was still alive and read exactly like the pad run, which would have "confirmed" the feature
  by measuring nothing. A control that reproduces the treatment is a control that was not run.)

  Also: gilrs's `Button::LeftTrigger` is the *bumper* and `LeftTrigger2` is the analog trigger;
  this engine calls those `LeftBumper` and `LeftTrigger`, and the one `match` that crosses the two
  namings has a test, because a silent swap gives every player a handbrake where they expected a
  gear change.

  **Not covered, with triggers.** *A control held while the game launches* — see above; it needs
  reading the device's state directly, which is the backend's job and not ours. **Trigger:** a
  gilrs release that reads initial state, or a complaint. *The browser — WIRED (2026-08-18).* gilrs's wasm backend reads
  the Web Gamepad API through web-sys and turns the browser's polled snapshot into the same event
  stream the native backends produce, so `pump`, `resync` and the `KnownPad` mirror are unchanged
  and nothing above the backend knows which target it is on. The `gamepad` feature is no longer
  target-gated, `demo-web` opts into it explicitly (it takes `default-features = false`, so
  without the entry the web build would be the one configuration where a plugged-in controller
  does nothing), and CI lints the arm in its own invocation — because a target that is *built* and
  never *linted* is exactly how the previous wasm arm in this workspace rotted.

  **Two browser facts with no native counterpart**, both of which a game meets before this code
  does: the page must be a **secure context** (https or localhost) or the API is simply absent,
  and **the gamepad list stays empty until the player presses a button** — browsers hide connected
  pads from a page nobody has interacted with, as a fingerprinting defence. So the "control held
  at launch" gap above is not merely still open on the web; it is the *normal* first state there,
  and a game that waits for `input.gamepad()` before showing "press a button to start" has it
  backwards.

  **Not verified against a real browser** — nothing here can drive one. What is verified is that
  the arm builds and lints for wasm32 with the feature on. **Trigger:** a browser with a pad in
  front of someone.

  Worth recording separately, because it cost a build: gilrs's `compile_error!` demanding
  `xinput` or `wgi` is **not gated on Windows**, so `default-features = false` fails on wasm32
  too. The Cargo.toml comment already said so; trying it anyway is how that was re-confirmed. *Rumble — WIRED (2026-08-18), with a
  third-party limitation measured rather than guessed.* `Input::rumble(weak, strong, secs)` and
  `rumble_pad`; `gizmo_app::gamepad::GamepadBackend::apply_rumble` hands them to gilrs's `ff`
  after the frame's systems have run.

  **A queue, not a field, and the reasons are each a line of code.** Rumble is the one thing on a
  pad that travels outwards, so the three properties `Input` gets for free as *state* all become
  wrong: a replay would shake the controller (the queue is `#[serde(skip)]` — a recording is what
  the player did, not what the game answered), a dropped frame would repeat it (it is *drained*,
  not read), and Alt-Tab would fire it late (`release_all` clears pending requests **and** queues
  a stop, because a rumble already running lives in the driver and outlives the frame that started
  it). The two motors are named `weak`/`strong` after the feeling rather than `left`/`right` after
  one controller's plastic. One effect per pad, replaced in place: a gilrs `Effect` owns a driver
  slot and a typical pad has sixteen, so building a new one per explosion stops working after a
  dozen — with an error a game cannot act on.

  **What could not be verified here, precisely.** A virtual `/dev/uinput` pad declaring `FF_RUMBLE`
  plus every bit gilrs's `test_ff` reads — `FF_SQUARE`, `FF_TRIANGLE`, `FF_SINE`, `FF_GAIN`,
  checked by reading the device's capabilities back — still reports `is_ff_supported() == false`
  through gilrs, so no effect ever reaches the kernel's upload protocol and the magnitudes cannot
  be read back. The blocker is inside gilrs-core's Linux path. `gamepad_rumble_device.rs` keeps the
  whole harness (it *does* read uploaded effects: `strong=`, `weak=`, `length=` straight out of
  `UI_FF_UPLOAD`) and **skips loudly with that measurement** rather than failing, because a red
  test there would be blaming this engine for someone else's behaviour. Worth knowing separately:
  gilrs's test wants the periodic-waveform set of a force-feedback *wheel*, and most console pads
  on Linux advertise only `FF_RUMBLE` — so gilrs reports no force feedback for them either.
  **Trigger:** a gilrs release that fixes this, or a wheel-class device to try it on.

  *A rebinding UI — CLOSED (2026-08-18), and the missing piece was not the
  panel.* The item read "`NAMED_GAMEPAD_BUTTONS` exists so a config file can name controls, but no
  editor panel consumes it". What actually blocked it was that **`ActionMap` had no way out to
  text and no way back**, so the names named things nobody could save; a panel would have had
  nowhere to put its answer.

  `gizmo_core::input::binding_names` is that half. One binding is one string —
  `key:w`, `mouse:left`, `pad:south`, `axis:left_stick_x+0.5` — and the `pad:`/`axis:` prefix
  earns its place: `left_trigger` is a name in **both** tables, the same control read as a digital
  press and as analog travel, and a config that could not tell them apart would be ambiguous
  exactly where a driving game cares. `to_named` returns a `BTreeMap` so a settings file does not
  produce a diff on every save; `apply_named` **replaces** rather than merges, or a control removed
  from the file keeps firing until the next fresh start; and it **returns what it could not parse**
  instead of skipping it, because a typo is otherwise a control that silently does nothing, which
  is the least diagnosable bug a game can have.

  **The capture read is the part a naive version gets wrong**, so it is in the engine rather than
  the panel: `InputBinding::captured_from` reads press *edges* (or the click that opened the dialog
  binds itself), stores a **default** threshold rather than however far the stick happened to be
  pushed (0.83 of travel would bind an action that only fires past 0.83), and answers in a fixed
  order so a key beats a stick resting past its deadzone. A test asserts that everything capture
  can return is also nameable — otherwise a panel shows a blank row for a control just pressed.

  `gizmo_editor::rebinding` is the panel, and it takes the map and the `Input` rather than owning
  them, so a game's own settings screen can draw the same rows. Its one interesting state is
  *listening*, one action at a time, with a **one-frame grace** — the click that starts listening
  is in the same `Input` the panel is about to read. Escape cancels and is checked before capture,
  or the key that means "stop" is the key that gets bound. All of it drives headlessly through
  `egui::Context::run_ui`, which is what makes it testable at all. *Demos beyond `car_demo` and `platformer` — CLOSED (2026-08-17), and the
  three lines turned out to be the wrong shape.* See the movement-axis item below.
- **Movement input — one axis instead of nineteen copies (2026-08-17).** The gamepad item above
  left a line reading "the remaining 37 demos still read the keyboard only; the path is three
  lines each". Writing those three lines nineteen times is what turned out to be wrong, because
  the nineteen copies did not already agree with each other.

  **Measured before touching anything — and then re-measured twice, because each measurement was
  short.** A first `grep` said 17 demos. A source ratchet over a *written* subject list said 18. A
  ratchet over a **scanned** one — every crate's `src` plus the demo crate's — said **20 places
  computing a movement direction from the keyboard**: 16 demos in `demo/src/bin`, `demo/src/main.rs`,
  `SimpleApp`'s built-in fly camera, the studio's editor camera, and
  `gizmo::systems::fps_look` — the engine's own first-person controller. Fifteen accumulated a
  direction and normalised it.
  **Five — `showcase`, `cpu_physics`, `ocean_scene`, `advanced_physics` and `demo/src/main.rs` —
  add a full-speed step per key from four independent `if`s**, so holding W and D moved them at
  √2 ≈ 1.41 × the speed of either key alone. **Nineteen of the twenty had no stick at all**, and
  three of those nineteen are engine code rather than demos: every game built on `SimpleApp`, and
  every camera driven by `FpsLook`, had movement a pad could not touch. That is the part of this
  that was never a demo problem.

  `gizmo_core::input::blend_move_axis` and [`Input::move_axis`] are the two rules in one place, and
  all 18 now read movement through them. `car_demo` deliberately does not: a vehicle's controls are
  not a movement vector — throttle and steering are independent axes, and folding them into a unit
  disc would take away throttle for steering.

  **Two API levels, and the studio is why.** `move_axis` reads keys and stick together, which is
  what a demo wants. The studio cannot use it: its fly keys are gated behind the right mouse
  button, because W/E/Q collide head-on with the editor's Translate/Rotate/Select shortcuts — but a
  **pad has no tool shortcuts**, so requiring that gesture of a stick would be a restriction with
  nothing behind it. It therefore calls `blend_move_axis` with the key half zeroed when the gate is
  closed, and the stick flies the viewport ungated (still not during Play — there the pad is the
  game's).

  **`SimpleSceneState::fly_step` exists because the engine's own camera was untestable.** It lived
  inside a `set_update` closure, which needs a window, a renderer and an event loop to reach, so
  nothing had ever asserted on the mover every `SimpleApp` user gets. It is a method now, and six
  tests cover it — including the diagonal it did *not* have, pinned so it cannot acquire it.

  **The two rules do different jobs, and the obvious explanation of the second one is wrong.**
  The radial clamp on the sum is what bounds the speed, diagonal keys included. Normalising the
  key direction *first* changes no magnitude at all — deleting it left every speed assertion in
  the file green, which is how the wrong explanation was caught. What it actually buys is that
  keys and stick are **comparable**: a stick is at most 1 long, an un-normalised diagonal key push
  is √2, so without it a player pushing the stick hard against a held W+D cancels only 71 % of it
  and keeps drifting at 0.414 of full speed. That number is the failure message of the one test
  that notices — `a_full_stick_can_cancel_a_diagonal_key_push_too` — and both mistakes were
  reintroduced to watch their own guard go red.

  **The conversion's promise was that keyboard play is untouched, so that is a test rather than a
  claim:** `the_keyboard_half_reproduces_what_the_demos_already_computed` checks the new function
  against the shape the thirteen correct demos had, over all 81 key combinations. The demos with a
  vertical axis (Q/E, Space/Ctrl) had their `normalize` turned into a **clamp** — for keys the two
  are identical (any non-empty combination is at least unit length), and a normalise would have
  pushed a half-tilted stick back up to full speed, destroying the one thing the stick adds.

  Verified on a device, not only in unit tests: the virtual-pad test now also asserts that a stick
  pushed right and up arrives at `move_axis` as `(1.00, 0.00)` and `(0.00, 1.00)`.

  **And there is now something watching, because the divergence was never a hard problem — it was
  an unwatched one.** `crates/gizmo/tests/movement_input.rs` scans for files that read a movement
  key without going through the shared blend.

  **Its first subject list was written, and it was wrong within the hour** — which is §8's rule
  about scanned subject lists failing in exactly the documented way: silently, by covering less
  than it appeared to. It named `demo/src/bin`, `SimpleApp` and the studio camera, and so missed
  `demo/src/main.rs` (which had the 41 % diagonal) and `fps_look.rs` (the engine's own FP
  controller, whose module doc *opens* by complaining that "the demos write it out BY HAND every
  frame"). The subjects are scanned now: `crates/*/src` plus `demo/src`, so a crate added tomorrow
  is covered. The detector also learned to cut **trailing** comments — `gizmo-core`'s `NAMED_KEYS`
  is a column of `("a", 19), // KeyCode::KeyA`, and the key table was on its own offender list. It is a
  ratchet, and its exception list is the other thing it produced: eight files read movement-*named*
  keys for something else entirely — throttle (`car_demo`, `hill_climb`), turret aim (`yikim`,
  `yikim_ustasi`), a dial (`cloth_demo`, `wind_tunnel`), editor tool modes, and a fighting-game
  `ActionMap` table where directions are digital by design. Each entry says which, so the list read
  end to end answers "what does W mean in this repository". The detector is a **proxy** — it
  matches "reads a movement key", not "rolls its own blend" — and says so in its own docs; the
  choice was deliberate, because a scan clever enough to tell a good `KeyCode::KeyW` from a bad one
  by its neighbours is a scan that fails silently.

  **Look/aim — resolved where it belonged, not as a free-floating helper.** The deferral said a
  shared helper would have to invent a sensitivity convention. `FpsLook` already owns one
  (`sensitivity`, rad/pixel), so that is where stick look went: `FpsLook::apply_stick_look` and a
  `stick_sensitivity` in **rad/second**. The different unit is the whole point, and the pair is
  written down at both functions: **a mouse reports how far it has moved, so the frame's time is
  already inside the number and `dt` must not be applied; a stick reports how far it is held, so
  what it describes is a rate and `dt` must be.** Getting either backwards makes look speed
  frame-rate dependent, which is invisible on the machine it was written on. `apply_look` takes no
  `dt` at all — the signature is the guard — and the stick's half is pinned by a test that holds it
  for twice as long and expects twice the turn.

  **`FpsLook` had no callers — and the trigger fired on the first attempt to give it one
  (2026-08-18).** The recorded trigger was "the next demo that wants a first-person camera; if
  `FpsLook` does not fit it either, the controller is the thing to change". Trying `cpu_physics`
  showed it did not fit, in two specific ways that every real caller has:

  * **Looking was ungated.** A demo or a tool with a cursor cannot have the camera swing on every
    mouse move; `cpu_physics` and `gizmo-studio`'s editor camera had each hand-rolled a
    right-mouse gate. `FpsLook::look_button` is that gate, and it covers the **mouse only** — a
    stick has no cursor to fight with, the same conclusion the studio reached about its fly keys.
  * **No sprint.** `sprint_key` + `sprint_multiplier`, and the collision that forced the vertical
    keys to become fields too: ShiftLeft already meant *descend*, so shift-to-sprint would have
    meant both. `up_key`/`down_key` are configurable and default to what they were, so a `FpsLook`
    written before this behaves exactly as it did.

  `cpu_physics` is the first caller, and the conversion is the evidence: **−57 lines, and its
  `DemoState` lost every camera field it had** (`camera_speed`, `camera_pitch`, `camera_yaw`,
  `camera_pos` — all four dead once the controller owned them; the struct is down to `time`).

  The other 21 hand-rolled cameras stay hand-rolled: some are orbit cameras, some are turrets, and
  converting a camera nobody asked about is churn against a demo that works. **Trigger:** the next
  one that is a plain first-person or fly camera.
- **The ten-light ceiling — CLOSED (2026-08-17), both halves. Read the item as history.**
  The heading said "the ceiling is not" for a day after clustering raised it to 256 and the
  paragraphs below recorded that; corrected 2026-08-18. `MAX_LIGHTS` is **256** in
  `frame_uniforms.rs`, the per-fragment loop is per-cluster, and the remaining hard limit is
  computed by a test rather than argued (all below). The item is kept because the *selection*
  reasoning is still what the code does.

  **The selection half, first.** What changed before the ceiling did was which lights get kept.
  It used to be whichever ones ECS iteration reached first, and that had three separate
  consequences: distance never entered into it (a light behind the camera took a slot from one
  lighting the wall in front), point lights were collected in their own loop first and so
  **starved every spotlight** once the array filled with points anywhere in the level, and the
  chosen set shifted whenever the archetype set changed — spawn, despawn or add a component and a still scene changed its lighting, i.e.
  flicker. `collect_scene_lights` now scores every light against the camera and keeps the best
  ten, points and spots in one pool, ordered by distance to the light's **sphere of influence**
  (exact, not a heuristic: the shaders window attenuation with `clamp(1 - (d/r)^4, 0, 1)`, so a
  light contributes precisely nothing past `radius`), tie-broken by `intensity * radius` and then
  by entity id — which is what makes the selection a pure function of world state rather than of
  iteration order. Four tests in `shared.rs` hold it, including the archetype-reorder one that is
  the flicker regression.
  **The cap and the frustum cull closed the same day.** `MAX_LIGHTS` went 10 → **32** — read this
  paragraph as history, because clustering raised it to 256 the same day (below) — and lights whose sphere of
  influence misses the camera frustum are dropped before the ranking runs, so a level's worth of
  lights behind the player no longer compete for the frame's slots. Raising the cap was the cheap
  part *because* the layout is guarded: `shader_contract`'s
  `every_scene_uniform_declaration_matches_the_bytes_rust_uploads` parses every WGSL declaration
  against the Rust struct, and it named the one file a `grep` had missed (`gbuffer.wgsl`, which
  flattens the array to `array<vec4<f32>, 4·MAX_LIGHTS>`). The three hardcoded `1168`-byte asserts
  are now written against `MAX_LIGHTS`, so the next raise does not need a hand-recomputed literal
  in three files — which is how one of them would have been forgotten.

  **The block's fixed part is 560 bytes, not the 528 this paragraph used to say.** It grew by 32
  when `cluster_dims` and `cluster_depth` were appended, and only the ceiling arithmetic below was
  updated — so the same struct had two different sizes in adjacent paragraphs until 2026-08-18.
  Measured, not recomputed: `SceneUniforms` is **16 944 B** at `MAX_LIGHTS` = 256, of which 560 B
  is fixed.

  **Clustered culling landed the same day, and the cap is now 256.** `MAX_LIGHTS` was never a
  hardware limit — it was the per-fragment loop over *every* light in the frame wearing a constant's
  clothing. `gizmo-renderer::clustered` cuts the view volume into 16×9×24 clusters, assigns each
  light to the clusters its sphere touches, and both lighting loops (deferred and forward) now walk
  their own fragment's list, bounded by `MAX_LIGHTS_PER_CLUSTER` = 32 whatever the scene holds. The
  hard ceiling left is the uniform block: `(65536 − 560) / 64` = 1015 lights, and the honest move
  past a few hundred is to put the light array in a storage buffer rather than inch toward that
  cliff.

  **That ceiling is computed now, not argued.**
  `the_scene_block_fits_the_uniform_binding_floor_every_webgpu_target_guarantees` asserts the block
  is inside the 64 KiB `maxUniformBufferBindingSize` every WebGPU target guarantees, and that the
  560 / 1015 figures quoted here are this layout's. Before it, raising `MAX_LIGHTS` past the cliff
  would have compiled, passed every test, and failed at **run time** on the first machine that
  enforces the floor — as a bind-group validation error, which is the least informative place to
  learn about arithmetic done in a header. Verified red at `MAX_LIGHTS` = 1200: 77 360 B, with the
  real ceiling in the message.

  **Assignment is on the CPU, deliberately**, and the cost is measured. A compute-shader build is
  the follow-up and its **trigger is a profile** showing this — not a preference for compute. What
  the CPU version buys is that the whole assignment is a pure function with nine unit tests, which
  is how the rest of this engine's correctness is held.

  **The trigger could not be pulled until 2026-08-18**, which is the part worth recording: the
  numbers below were taken by hand and nothing reproduced them, so finding out whether assignment
  had *become* the frame's problem meant redoing the experiment from scratch and hoping it was set
  up the same way. `crates/gizmo-renderer/benches/clustered_bench.rs` is that experiment written
  down, at the same light counts so a run is a comparison rather than a fresh baseline. It also
  joins CI's `cargo bench -- --test` smoke gate, so the harness cannot quietly stop compiling
  between the times anyone looks at it.

  | lights | recorded by hand | benchmark (2026-08-18) |
  |---|---|---|
  | 8 | 0.047 ms | 0.027 ms |
  | 32 | 0.106 ms | 0.091 ms |
  | 64 | 0.201 ms | 0.177 ms |
  | 128 | 0.469 ms | 0.346 ms |
  | 256 | 0.764 ms | 0.718 ms |

  Same order, same scaling — linear in light count — and consistently a little lower, which is
  what a different light distribution and criterion's median rather than a wall-clock stopwatch
  would produce. Neither column is the other's correction. What matters for the decision is the
  last row: **0.72 ms at 256 lights is 4.3 % of a 60 fps frame**, so the CPU version is still the
  right side of the trade, and a profile that says otherwise is now something anyone can produce
  by running one command.

  **The guard finding is the part worth remembering.** Clustering is correct only if
  `assign_lights` (CPU) and `gizmo_cluster_index` (WGSL) agree about which cluster a point is in,
  and *no pixel test could see a disagreement*: adding `+ 1u` to the shader's slice left **every**
  frame guard green, including one written specifically to catch it, because a light big enough to
  measure is a light the CPU also assigned to the neighbouring slices. What guards it is a **GPU
  mirror test** — the shader's own function, composed from the shipping module, evaluated over
  sample points and compared against the CPU's `cluster_of_point`. It fails on 93 of 97 points for a
  one-slice error and 73 of 97 for a one-tile error. Two implementations of one rule need a test
  that runs *both*, not a test of what they are for.
  (Boundary points are deliberately not sampled: glam's matrix multiply and the GPU's are both
  correct and not bit-identical, so a point exactly on a tile edge may land either side. That is
  harmless because cluster bounds are corner AABBs and therefore overlap — a light on the seam is
  assigned to both.)

  Two findings worth keeping from doing it. The GPU guard
  (`the_last_light_slot_reaches_the_frame`, a light in slot 31 must change the frame) only went red
  after clamping **three** shaders back to ten — `deferred_lighting`, `shader` and `volumetric` all
  light this frame, and either one alone kept the light visible; a guard that measures the frame
  rather than a shader is what covers consumers the next person does not know about. And
  `render_parity`'s capability scan read `#[cfg(test)]` code as production: adding that guard made
  it report `PointLight` as "game path only" although light collection is entirely in the shared
  module. It now cuts each file at its test module, and asserts that the cut left none behind.
- **Friction has no positional term** — a modelling gap. Narrowed twice on 2026-08-17, and worth
  reading in that order. The normal channel carries `pen0` into the bias in all three sweeps
  (`solver/tgs.rs`); the tangent channel has no counterpart — every friction solve is pure
  `acc_t − rel·t/k_t`, velocity-level only, so friction resists tangential *velocity* and never
  tangential *displacement*.
  - It was listed as the standing candidate for the tall-stack growth rate. That growth rate is
    now measured at exactly zero (§7, N1/N2 closed: peak lean 0.0000 for N=16/32/48 at every
    ground half-extent from 20 to 200), so that motivation is gone.
  - Chasing the trigger this item was left with — "a measured creep" — found one, and it was not a
    missing positional term: the friction **cone** was picking its coefficient from the demanded
    impulse rather than from whether the contact was sliding, and a crate at rest on a 28° slope
    left the plate. Fixed (§7 "The slope a crate could not stand on"), and the residual creep fell
    44× at 90 % of the static limit and 7.5× at 99 %.
  - **Chasing it a third time (2026-08-18) turned up something larger again — this time not in
    friction at all.** The item asked for a measured creep; writing the measurement as a test
    (`friction_hold.rs`, which had never existed — the 14 mm lived in this paragraph and nothing
    would have noticed it changing) meant pushing a box with `RigidBody::force_accumulator`, and
    the box would not move until roughly **four times** the force it should have taken.

    The force channel was frame-rate dependent. The world runs fixed 1/240 s substeps; the
    integrator drained the accumulator on the **first substep** of a frame and the rest got
    nothing, so a body received `F·(1/240)` of impulse per frame instead of `F·frame_dt`.
    Measured, 10 N on 1 kg for one second:

    | frame rate | v after 1 s, before | after |
    |---|---|---|
    | 240 fps | 9.95 m/s | 9.95 m/s |
    | 120 fps | 4.97 m/s | 9.95 m/s |
    | 60 fps | 2.49 m/s | 9.95 m/s |
    | 30 fps | 1.24 m/s | 9.95 m/s |

    The acceleration halved every time the frame rate halved — a thruster, a wind volume or a
    custom gravity field behaving differently on every machine. Forces are drained once per
    **frame** now, so each substep integrates `F·substep_dt` and the sum is `F·frame_dt`.

    **Nothing in the workspace wrote to that field**, which is exactly why it survived: the
    engine's own `PhysicsWorld::apply_force` takes a `dt` and writes the velocity directly, so the
    accumulator was a public API with no in-tree consumer, and no test, demo or golden image
    touched it. The determinism hash is unchanged (`A462C9EB8A09D5CA`) for the same reason — the
    stress test falls under gravity.

  - **And the residual creep this item is named for went with it.** Same harness, same 99 % of the
    static limit: **15.2 mm over 200 s before, 0.010 mm after** — a factor of 1500. The mechanism
    is the one §7 already described: the old drain delivered the frame's force as a *kick* on one
    substep, and a kick is tangential velocity that has already happened, so `friction_limit` saw
    a sliding contact and charged the dynamic coefficient — losing `(μ_s − μ_d)·λ_n` of hold, four
    times a second. A steady per-substep push is what the static branch can actually hold.

    That does not prove the tangent channel now has a positional term — it does not, and a
    velocity row still cannot undo a displacement that already happened. What it does mean is that
    the *motivation* recorded here was measured through a defective force channel, and the honest
    residual is 10 µm over 200 s rather than 14 mm. **Trigger:** unchanged in kind and weaker in
    force — a game that needs sub-millimetre hold at the friction limit. Friction anchors remain
    the fix if one turns up.

  - **A third defect in the same field, found the same way (2026-08-18): a force applied to a
    sleeping body did nothing at all.** Measured — a 1 kg box settled on a plate, then given
    **50 N sideways for two seconds** through `force_accumulator`, moved **0.0000 m**. The same
    box with `wake_up()` called alongside the push moved **92.7 m**. Not less: nothing, silently,
    for as long as it stayed asleep.

    The mechanism is two correct decisions meeting: `Integrator::integrate_velocities` returns
    early on `is_sleeping`, so the accumulator is never read — and the sleep test looks only at
    *velocity*, so a body with a force but no motion is a body that qualifies to keep sleeping.
    A thruster, a wind volume or a conveyor therefore stopped working the moment its target came
    to rest, which is the moment it is most obviously meant to work.

    **Every sibling path already kept the contract.** `PhysicsWorld::apply_impulse` and
    `apply_force` take `&mut RigidBody` precisely so they can wake it, each with a comment saying
    that otherwise the effect is silently swallowed; the explosion system wakes what it moves,
    after a bug where "a settled stack beside a blast simply did not react". The accumulators are
    **public fields**, so they were the one way in with nothing standing there. `PhysicsWorld::step`
    now wakes any dynamic sleeping body whose force or torque accumulator is non-zero, before the
    substeps run.

    It was in front of us the whole afternoon: `friction_hold.rs`, written hours earlier, calls
    `wake_up()` next to its push with a comment explaining that sleeping "would make this measure
    nothing at all". A workaround in a test is a defect report nobody filed. That call is gone now,
    and the file exercises the path instead. Determinism unchanged (`A462C9EB8A09D5CA`) — a body
    with an empty accumulator is not touched.

    **And the angular channel had no guard at all.** Both accumulators are drained by the same
    line, so the frame-rate defect above was on both, and only the linear half was pinned by a
    test — an edit could have restored it on the angular half with everything still green.
    `applied_forces.rs` now covers the torque twin (bit-identical ω at 240/120/60/30 fps), the
    wake contract on both channels, and the one property `Integrator::apply_torque` rests on: the
    **world-space** inverse inertia tensor. That test uses a rod laid on its side, where the
    body-space shortcut answers 0.0150 rad/s against the correct 3.0 — 200×, so it cannot pass by
    accident.
- **The doc-language rule — CLOSED (2026-08-17), and the claim it replaces was false.**
  This item used to read "Stage A plus the facade went from 1286 Turkish `///` lines to 8, and
  seven of those eight are measurement error." Scanning for it found **462 Turkish doc lines in
  Stage A's `src/`**, 476 of them in `gizmo-physics-rigid` alone — the solver's and the joint
  solver's parameter documentation, which is some of the densest engineering prose in the repo.
  So the marker had been false for as long as anyone had believed it, and the earlier
  measurement is the thing to distrust, not the crates.

  All 462 are now English, `gizmo-physics-rigid` and `gizmo-net` included, and the translation
  commit is comment-only by construction — `git diff -U0 | grep '^[+-]' | grep -v '^[+-]\s*//'`
  came back empty at every step, which is how the one accidental *code* change (a renamed struct
  field) was caught while translating the doc block above it.

  **What replaces the marker is a ratchet**, `crates/gizmo/tests/doc_language.rs`: it scans
  `crates/*/src/**/*.rs`, counts Turkish doc lines by crate, and compares against a table that
  may only go down. A crate absent from the table must be at zero, a crate in it must match
  **exactly** (so cleaning without lowering the budget also fails, which is what stops the table
  becoming fiction), and the subject list is scanned rather than written, so a new crate is
  covered the moment it exists. Verified red: putting one Turkish line back into
  `gizmo-physics-rigid` fails it with the file and line. One exception is recorded with a
  reason — `gizmo-core/src/cvar.rs` documents how `to_lowercase` handles `İ`, i.e. the Turkish
  letter is the subject of an English sentence — and an exception that stops applying fails too.

  **The "clean" claim needed one correction, and it is the interesting part.** The detector's
  letter set was the six that appear in no other language likely to show up here (`ğşı` and their
  capitals), which made it precise and left a tail: a Turkish line marked only by `ç`, `ö` or `ü`
  — `/// (isim, seri) çiftleri.` — sailed straight through. **52 such lines were still in `src/`
  after the pass that reported the surface clean.** The three letters are in now, at the price of
  a small allowlist of proper nouns spelled with them (`Möller`, `Plücker`, `Bézier`…), checked
  word by word rather than as a whole-file exception. What the detector still cannot see is a
  Turkish sentence with no marked letter and fewer than two of its listed function words; that is
  the residual, and it is written down here rather than implied by a green test.

  **Stage B went the same way in the same session**, so the table is now **empty**: the renderer
  (327), scripting (138), analysis (137), editor (100) and studio (53) are English too, 755 lines
  on top of Stage A's 462. Along the way the detector learned to ignore citations — a code span or
  a quoted string is something a sentence *mentions*, not the language it is written in — which
  removed the only exception the ratchet had (`cvar.rs` documents how `to_lowercase` treats `İ`)
  and dropped 15 false positives across four crates. `EXCEPTIONS` stays in the file, empty, for
  the case citation-stripping cannot answer.

  Test files, benches and plain `//` comments are deliberately out of scope (the rule is about the
  documentation surface; CLAUDE.md already records that inline comments are still Turkish in
  places). Measured remainder there, by the same detector: **221 lines** in `tests/` and `benches/`
  across the workspace. (An earlier count said 524; that figure predated citation-stripping and
  counted the `#[cfg(test)]` modules that live inside `src/` — which this session translated.)
- **Asset identity — decided and wired (2026-08-17); one of the three stays open, with a trigger.**
  The blocking question was *do scenes address assets by identity?* The answer is **the path stays
  authoritative and identity is the fallback**, because the two failure modes are not symmetric: a
  missing path is a message anyone can act on ("no file at `demo/assets/tree.obj`"), while a
  dangling UUID is opaque, and a scene that names its assets by UUID stops being readable or
  hand-editable for a gain that only materialises when a file moves. So a scene records both, and
  `SceneData::repair_asset_paths` prefers the registry's current path only when it *disagrees* with
  the stored one.

  What that closed:
  - **`demo/assets` is scanned.** This was the linchpin and it was not a small gap: *nothing
    anywhere* called `scan_assets_directory` or `import_assets_directory`, so `path_to_uuid` was
    always empty, and the scanner, the resolver (`resolve_path_from_meta_source`, which `load_obj`
    and `load_material_texture` have always called) and the 22 sidecars on disk were wired to
    nothing at all. The studio now scans its asset-browser root at startup, read-only, and the
    exported runtime scans its own two layouts.
  - **Scenes carry identity.** `EntityData::mesh_uuid` and `MaterialData::texture_uuid`, both
    `serde(default)` so every older scene loads unchanged. `save_with_identity` /
    `load_into_with_identity` are the identity-aware pair; `save` / `load_into` are those functions
    with a `NoAssetIdentity`, so there is one code path rather than two that drift. The resolver is
    passed in as `AssetIdentity` because `gizmo-scene` sits below `gizmo-renderer` and must not
    learn what an asset is; the impl lives in `gizmo-app`, the first layer holding both halves,
    which is also why the app's own initial-scene load gets repair for free.
  - **What it survives, exactly:** a file or folder that MOVED with its `.meta` sidecar — dragging
    a folder, `git mv`, reorganising `demo/assets` — which is the case a path cannot survive. Not a
    rename that leaves the sidecar behind: the sidecar is named after its file, so the identity is
    orphaned and a later import mints a new one. That is a property of sidecar identity, and
    overclaiming it would be worse than the gap.
  - Two defects fell out of writing the tests: `normalize_path` gave absolute paths a **doubled
    leading slash** (`//tmp/x`, harmless in a lookup because both sides normalise, wrong in the
    value handed to a loader), and `./demo/x` vs `demo/x` were two registry keys for one file.
  - **glTF sub-mesh identity — CLOSED (2026-08-18), and it needed no second id.** The item said a
    sub-asset needs "a second id (Unity's fileID)". It already had one: the key is
    `gltf_mesh_<path>_<node>_p<n>`, and the suffix after the path **is** the fileID — what was
    missing was anyone using it. `stamp_asset_identity` asked the registry about the whole key,
    which is not a path, so a sub-mesh reference gained no identity at all and a moved `.glb`
    took its meshes with it.

    Two halves, and the second is the one a naive fix gets wrong. Stamping now looks up the
    **path inside the key**. Repair **rewrites** the key around the new path instead of replacing
    it: overwriting `gltf_mesh_old/car.glb_Body_p0` with `vehicles/car.glb` would turn "the Body
    mesh in car.glb" into "car.glb", which the loader reads as an OBJ path and fails to load.

    The split is made at the file **extension**, not at a separator, because underscores are
    normal on both sides — `wheel_front_left` inside `sports_car.glb` is the ordinary case. That
    rule now lives once, in `MeshSource::split_gltf_key` in `gizmo-core` (below both consumers),
    and the renderer's loader was rewritten onto it: it had its own copy, and two parses of one
    key shape is how the loader and the saver come to disagree about where a file name ends.
  - **Writing identity is on for the editor and the app, and that is the whole rollout.** There is
    no asset rename/move action in the editor yet (the browser is read-only), so nothing else needs
    it. **Trigger for more:** an editor move/rename action — which should carry the sidecar, and at
    that point is also what makes the orphan case above worth fixing.
- **Collider shapes — `Cylinder` and `Heightfield` landed (2026-08-17), closing an audit item
  that was never carried onto this list.** The 2026-08 audit found three shape gaps (§Y1,
  "Cylinder, Cone, Heightfield yok") and its phase B carried them as B7, *"Cylinder + Heightfield
  collider — wheels and terrain, both common"*. When FIXPLAN was retired the item was not carried
  into this section, so the roadmap has been silent about it since; the code was not. That is the
  second stale marker found this week by checking the code instead of the list.

  **What `Cylinder` is, and what it is not.** Flat circular ends, axis along local +Y, the same
  convention as the capsule — except that `half_height` here means half the *whole* solid, where
  a capsule's excludes its caps. It is convex, so it rides the existing GJK/EPA route: the only
  new geometry is a support function whose radial and axial parts are chosen independently, which
  is exactly what puts the support point on the **rim** and lets a manifold spread around the
  edge. That is the property that makes a cylinder stand where a capsule rocks.

  Everything derived from a shape got its own arm rather than a fallback: an analytic inertia
  tensor (`I_y = ½·m·r²`, `I_x = I_z = m(3r² + h²)/12` — *not* the capsule's, whose hemispheres
  put mass furthest from the axis), an exact AABB, `πr²h` for volume, a ray test against wall and
  both caps, the cloth pusher, the character controller's size, the editor inspector and the debug
  wireframe.

  **The AABB has a measured numerical wrinkle worth knowing.** Its radial term is
  `r·sqrt(1 - a_i²)`, and when the axis lands on a world axis that square root sits on a
  singularity: a quaternion's last bits (ε ≈ 6e-8) come out as ≈ `r·sqrt(2ε)`, measured at
  1.2e-4 m for a 0.35 m wheel on its side. It always errs *outwards*, which is the direction a
  broadphase bound may err in, and the test's tolerance says so rather than hiding it.

  **The studio stopped lying in the same commit.** Its cylinder primitive spawned
  `Collider::convex_hull` of the mesh's 24 ring points under a comment reading "the engine has no
  cylinder shape" — faithful to the silhouette, and still wrong twice over: inertia came from an
  AABB rather than from `½·m·r²`, and every contact resolved against a facet, so a cylinder on its
  side settled onto whichever flat it landed on.

  **`Heightfield` landed the same day, and B7 is closed.** Terrain is a lattice of height
  samples centred on the collider's origin, `heights[row * cols + col]`, with `scale` as (cell
  size in X, height multiplier, cell size in Z). Each cell is two triangles, and that is what the
  narrowphase and the raycast test against — the same per-cell idea as `TriMesh`, with the tree
  replaced by arithmetic: which cells a shape overlaps is a division on its bounds, not a
  traversal, and the data is one float per sample instead of three vertices and three indices per
  triangle.

  **What the shape is for is the concavity, so that is what the test measures.** Any convex
  treatment — GJK on a support function, a hull of the samples, the field's own AABB — puts a lid
  across every valley, and a body dropped into a dip rests on the lid. Verified red:
  flattening every cell to the field's highest sample made
  `a_box_dropped_into_a_valley_rests_on_its_floor_not_on_a_lid` stop the box at 4.25 m, the rim's
  height, with the message that names the failure. The support function refuses a heightfield the
  way it refuses a plane, and the narrowphase arms sit below the plane arms and above the GJK
  fallback for the same two reasons the mesh arms do.

  Raycasting walks the lattice cell by cell (a 2-D DDA in XZ) and stops as soon as the nearest hit
  is closer than the next cell boundary, so a ray at the ground under a character tests one or two
  cells whatever the terrain's size — with a hard step bound, because a raycast that never returns
  is worse than one that misses. Four inline copies of Möller–Trumbore became one shared
  `Raycast::ray_triangle` on the way past.

  **`Cone` landed 2026-08-18, and the shape gap is closed.** Base at `-half_height`, apex at
  `+half_height`, axis local +Y — the same convention as the cylinder, deliberately, so the two are
  interchangeable in a scene file.

  **The support function is the one thing that must not be copied from the cylinder.** A cylinder
  picks its radial and axial extremes independently, because its radius is the same at every
  height. A cone's is not: its extreme point in any direction is **either the apex or a point on
  the base rim**, never in between. Verified red by writing the cylinder's version instead — a cone
  balanced apex-down then sat there indefinitely on a flat top that does not exist, where the real
  one falls over. That test (`a_cone_balanced_on_its_apex_falls_over_where_a_cylinder_stands`)
  measures the cylinder in the same pose as its own control, so it fails if the two ever agree.

  Every derived quantity got its own arm rather than a fallback, and each differs from the
  cylinder's by a number a wrong copy shows up as: volume is exactly a third; `I_y = (3/10)·m·r²`
  against the cylinder's `½·m·r²`, a ratio of 3/5; the AABB is the union of a **point and a disc**
  rather than of two discs; the ray test's lateral quadratic carries `y` in all three coefficients
  (a cylinder's does not), and each root is range-checked because the infinite double cone above
  the apex satisfies the equation exactly and is not part of the solid. The cloth pusher measures
  the radius **at the node's own height**, and the debug wireframe draws one rim and the lines to
  the apex — the fallback box would have drawn a cone and a cylinder identically, which is the
  confusion the shape exists to remove.

  **On the centre of mass, stated rather than assumed.** A solid cone's centroid is a quarter of
  the way up from its base, i.e. `-half_height/2` here, so the origin this shape is centred on is
  *not* its balance point. The engine leaves `RigidBody::center_of_mass` alone for every primitive,
  so the tensor is taken about the **origin** to match — `I_xz = (3/20)·m·r² + (1/10)·m·H²`, which
  is the textbook centroid form with the parallel-axis shift folded in and the two `H²` terms
  collapsed. Using the quoted centroid number directly would understate a cone's resistance to
  tipping, because it is the tensor about a point the engine does not spin it around. A body that
  wants the real balance point sets `center_of_mass` itself; the field docs say what to use.

  **Two call sites were lying and are not now.** `SimpleApp::spawn_textured_cone` gave a cone mesh
  a **sphere** collider under a `// approximation` comment — and `spawn_textured_cylinder` did the
  same, still, a day after `Collider::cylinder` landed. Both now carry their own shape. Nothing in
  the studio needed fixing: it has no cone primitive to be wrong about.

- **Shader Graph.** A node editor plus WGSL generation. The largest item on the editor list and a
  project on its own; the prototype has a tab for it, the engine has nothing.


---

## 4. Public-surface contract (was: "Release Strategy — Staged 1.0")

> **Reframed 2026-08-17.** Nothing in this section is waiting for a release any more (§3). The
> staging *plan* — Stage A crates going to `1.x` on their own version line — is **not being
> pursued**; the workspace keeps one `0.x` version. Everything else here is kept word for word,
> because the rules are what they always were: they describe which foreign types may appear on a
> public surface and what each one costs, and they are true whether or not a 1.0 is ever declared.
> Read "may go 1.x" below as "would be defensible to promise", and the criterion as the test a
> change to the public API has to pass **today**. That is also how it is enforced today, by
> `crates/gizmo/tests/crate_staging.rs` and by the seals listed further down.

1.0 = the hard promise of "no breaking change without a 2.0". What decides whether a crate can
make that promise is not our own code but our loudest dependency: if a foreign type is reachable
from our public API, that dependency's next major is our next major. A lock-step 1.0 across all
crates would therefore either freeze the engine on old deps or burn the 1.0 at the first dep bump.
The solution is **staged**.

**The Stage A criterion — RESTATED 2026-08-09.** This used to read: "a crate that re-exports a 0.x
dependency (wgpu/winit/egui, bevy_reflect) in its public API cannot make that promise." That
literal test is wrong in three ways, and the eleven-crate surface audit of 2026-08-09 found what
it had been letting through:

1. **It is the CADENCE that disqualifies, not the version number.** The property being tested for
   is *a history of frequent majors*; `0.x` is the common case, not the definition. `wgpu` is the
   counter-example the old sentence named as its own archetype and then failed to catch: it
   resolves at **29.0.3**, so it passes a literal "is it 0.x?" test, and it has historically
   shipped on the order of a major per quarter — this engine's own 0.2.0 upgrade crossed
   **0.20 → 29** in one release (§6). A version number ≥ 1.0 buys nothing when the next integer is
   a quarter away. The converse holds too: a frozen 0.x costs nothing, which is why `tracing` 0.1
   is an unconditional dependency below.
2. **"Re-exports" is too narrow — the test is REACHABILITY.** A foreign type is on the surface if
   a downstream crate can be handed it or forced to name it: `pub use`, yes, but equally a public
   field, a parameter or return type, an associated type in one of our trait impls, or a foreign
   trait implemented on one of our public types. `arrayvec` was declared clean in 0.2.0 on the
   strength of the storage field having been made private, while
   `<ContactPoints as IntoIterator>::IntoIter` went on reading `arrayvec::IntoIter<ContactPoint, 4>`
   — an associated type nobody had to write down, and one that could not be changed after 1.0
   either. That is the leak the old wording let through, and it survived until 2026-08-09.
3. **The promise is per FEATURE CONFIGURATION — the old rule did not mention features at all.**
   1.0 covers the DEFAULT feature set. A feature may put a fast-moving dependency back on the
   surface, but only if it is default-OFF *and* named in the contract below as an explicit opt-out
   of the stability promise for that crate. Today those are `reflect`, `tracing-layer`,
   `gpu_physics` and `client-server`.

The staging itself:

- **Stage A (may go 1.x):** dependency-light crates whose surface we own —
  gizmo-math, -core, -physics-{core,rigid,dynamics,soft}, -scene, -net, -audio, -ai,
  -animation. "May go" is a candidacy, not a clearance. **gizmo-physics-rigid was BLOCKED on the
  `rustc-hash` leak; that block was CLEARED 2026-08-09** by the `EntityIndexMap` seal in the
  contract below, so the crate is now a candidate on the same footing as the rest — which still
  means the accepted `glam` cost, not a clean surface.
- **Stage B (stays 0.y):** graphics/integration — gizmo-renderer, -window, -editor, -ui,
  -app, -scripting + the `gizmo` facade (until wgpu/winit/egui settle — by criterion 1 that means
  a slow major cadence, NOT merely a version number ≥ 1.0; wgpu is already at 29).
- **Consequence:** once staging begins the crates no longer share a SINGLE workspace version
  (`publish_all.sh` + the version-inheritance assumption must be updated).

**External-type contract (permanent).** One entry per dependency that is, was, or could be on a
Stage A public surface. Where an entry corrects an earlier claim of this document, the earlier
claim is quoted and marked wrong rather than quietly overwritten — the value of this section is
its precision, and a section that silently rewrites itself cannot be checked.

- **`glam` — a permanent, DELIBERATE public dep, and the largest semver liability in Stage A.**
  `gizmo-math` re-exports it; resolved version **0.32.1**, i.e. 0.x, and the `.32` says plainly
  how often it breaks. The 2026-08-09 audit put it on the DEFAULT public surface of **8 of the 11
  Stage A crates** — all of them except `gizmo-core`, `gizmo-audio` and `gizmo-net` (audio does
  not depend on `gizmo-math` at all; net reaches it only through the optional, default-off
  `rollback` feature). Stated as the cost rather than as the intent: **a glam 0.33 is a breaking
  change for eight Stage A crates at once**, and under the 1.0 promise that means eight 2.0s
  released together. We accept it — the alternative is our own vector/quat types plus a conversion
  layer at every boundary, which is worse for users and for the determinism contract (§5) — but it
  is an accepted cost, not a neutral one, and it is the one item in this list that sealing cannot
  fix.

- **`arrayvec` — sealed 2026-08-09. THE EARLIER CLAIM HERE WAS FALSE.** This section used to say
  "`arrayvec` was removed from the public API (opaque `ContactPoints`)", and §6 listed "`arrayvec`
  left the public surface" among the 0.2.0 API breakers. Neither was true. 0.2.0 made the
  `ArrayVec` storage field of `ContactPoints` private — that is what those sentences were written
  from — but `impl IntoIterator for ContactPoints` still declared
  `type IntoIter = arrayvec::IntoIter<ContactPoint, 4>`: a 0.7.x type on the default surface of a
  Stage A crate, reached by criterion 2 above and not by criterion "re-export". What is true now:
  the by-value iterator is the opaque `collision::ContactPointsIter` (private field; `Iterator`,
  `DoubleEndedIterator`, `ExactSizeIterator` forwarded, `FusedIterator` asserted by us because
  arrayvec 0.7 omits it), re-exported at the crate root beside `ContactPoints`. The by-ref impl
  was verified rather than assumed — it already yielded `std::slice::Iter`. Both associated types
  are pinned by always-on `const _` fn-pointer coercions in `collision.rs`, so a future widening
  fails the build rather than the review. `arrayvec` itself stays as a private implementation
  detail (the fixed-capacity storage).

- **`ron` — sealed out of gizmo-scene's public API (2026-08-09); the earlier entry UNDERSTATED the
  leak.** It was recorded here as "the RON file format + the `SceneError` API". In fact
  `pub use ron;` re-exported the parser's ENTIRE surface as `gizmo_scene::ron`, and the facade
  re-exported that again as `gizmo::ron` — so a ron 0.13 broke us whether or not `SceneError`
  moved. Now the two RON-shaped failures are the opaque `error::ParseError` /
  `error::SerializeError` (private payloads, `Display` and `Error::source` forwarded verbatim,
  `line()`/`column()` handed back), and `SceneData::from_ron_str` /
  `to_ron_string` replace the "call `ron::from_str` yourself" workflow. **"No capability is lost"
  was the claim here and it is too strong — corrected 2026-08-09 by review:** the position comes
  back, but the parser's error *kind* (its `code` enum, matchable before), the *end* of the error
  span, and `Clone`/`PartialEq` on the payload do not, and `source().downcast_ref::<ron::…>()`
  stops matching. All four are accepted costs — every replacement for them would put a parser
  type back on the surface — and they are now spelled out on `ParseError` itself rather than
  glossed. `gizmo::ron` went with it
  (Stage B, but keeping it would have meant a second ron pin to hold in lock-step). The only
  remaining mention of ron in the surface is the two `From` impls on `SceneError` that make `?`
  work inside `scene.rs` — deliberately kept, because they force no ron type into any caller's own
  signature: the cost of a ron major is rewriting two `from` bodies here, not everyone's error
  handling. Resolved at 0.12.2.
  **The condition the seal was accepted under, and it is a promise rather than a note:** *1.0
  freezes the Rust API; it does NOT freeze the RON scene format.* The file format is versioned
  separately, through the existing mechanism — `scene::CURRENT_SCENE_VERSION` plus
  `SceneData::migrate`, which brings an older file forward and REFUSES a file written by a newer
  engine with `SceneError::UnsupportedVersion` instead of silently truncating it. Without this
  sentence the seal would buy a 1.0 that quietly reserves the right to change its own on-disk
  format underneath a frozen API.

- **`web-time` — removed from gizmo-scene's public API 2026-08-09; it was MISSING from this
  contract entirely** and nobody had noticed it before the audit. `SceneSnapshot::timestamp` was
  `pub`, and on `wasm32` the type of that field is `web_time::Instant`, not `std::time::Instant`.
  Because the substitution is target-gated rather than feature-gated it sat on the default wasm
  surface with no way to opt out — precisely the shape the old rule was blind to, since nothing
  was re-exported and nothing looked 0.x at the call site. The field is now private;
  `age() -> std::time::Duration` was already its only reader anywhere in the workspace, so no
  capability was lost. In the same pass the pin went `0.2` → `1` in all seven crates that declare
  it, which also dropped the duplicate 0.2.4 build; **1.1.0** is now the only version resolved.

- **`crossbeam-queue` — sealed out of gizmo-core's public API (2026-08-09).** Opaque
  `asset::AssetDropQueue` newtype: no public constructor, no accessor, no public field, and a
  hand-written `Debug` so the wrapped type does not surface in rendered docs or in any containing
  `{:?}`. `HandleIdTracker::drop_queue` is private and typed as the newtype, which also closes the
  transitive path through the public `Handle::tracker`. The dependency itself stays — it is
  genuinely used by `asset.rs` and `commands.rs`; the seal hides it, it does not remove it.

- **`rustc-hash` — sealed out of `gizmo-physics-rigid`'s public API (2026-08-09). THE EARLIER
  ENTRY HERE IS SUPERSEDED, and it is worth keeping why it existed.** It read: "STILL ON THE
  DEFAULT PUBLIC SURFACE of `gizmo-physics-rigid`. NOT sealed, and it was missed by the
  2026-08-09 audit entirely… Until this is sealed, `gizmo-physics-rigid` cannot honestly go 1.0."
  That was accurate when written — the leak was found by the adversarial review of the audit's
  own change sets, not by the audit. `FxHashMap<K, V>` is a type *alias*,
  `HashMap<K, V, FxBuildHasher>`, so the hasher — a `rustc-hash` type, resolved at **2.1.2** —
  travelled with every use of it, and a type alias hides nothing: it is exactly criterion 2's
  "an associated type nobody had to write down", one level of indirection further out. The
  cadence argument is what made it urgent rather than theoretical: rustc-hash 1.x → 2.0 **changed
  exactly this type**, from `HashMap<K, V, BuildHasherDefault<FxHasher>>` to
  `HashMap<K, V, FxBuildHasher>`, so the precedent for a rustc-hash 3.0 breaking us was the last
  major it shipped. Both 1.1.0 and 2.1.2 are still in `Cargo.lock` (something else in the graph
  wants 1.x).

  What is true now: `world::EntityIndexMap` is an opaque newtype over the map — private field, no
  `Deref`/`DerefMut`/`AsRef`/`Borrow`/`From`/`Into`, no public accessor, and a hand-written
  `Debug` (the `AssetDropQueue` precedent above; a derive would have printed the entries in hash
  order, which is non-deterministic output from a type inside a determinism-critical struct). It
  is `#[serde(transparent)]`, so `PhysicsWorld`'s snapshot JSON is byte-identical to before. All
  three reachable sites now name it instead:
  - `world/mod.rs` — `PhysicsWorld::entity_index_map` stays a **public field**, retyped. Read
    access was deliberately kept rather than privatised: `get`, `contains_key`, `len` and
    `is_empty` mirror the `HashMap` methods exactly, so the ~57 in-crate call sites and any
    downstream reader compile unchanged.
  - `solver/mod.rs` — `ConstraintSolver::solve_contacts(.., &world::EntityIndexMap, ..)`.
  - `joints/solver/mod.rs` — `JointSolver::solve_joints(.., &world::EntityIndexMap, ..)`.

  Two sites the audit list also flagged, `solver/mod.rs:48` (`support_order_manifolds`) and
  `solver/tgs.rs:115` (`solve_contacts_tgs`), were checked and are **private** and `pub(super)`
  respectively — not reachable, so they still take the bare `FxHashMap` and are handed it by a
  `pub(crate) raw()` on the newtype. `rustc-hash` itself stays as the implementation detail it
  always should have been.
  **Capabilities, stated as costs rather than glossed:** *editing a world's map entry by entry*
  from outside the crate is gone (`insert`/`remove`/`clear` are `pub(crate)`) — it was never safe,
  since the map has to stay in lockstep with the SoA arrays, and
  `add_body`/`remove_body_at`/`sync_bodies`/`clear_bodies` are the supported routes. Note the
  narrowness, because an earlier draft of this entry overclaimed it: the field stays `pub` and
  `Default`/`Clone`/`FromIterator`/`Deserialize` are public, so `world.entity_index_map = …` still
  replaces the map wholesale and still breaks lockstep. The invariant is a convention, exactly as
  it was before the seal; only the hasher was sealed. Enforcing it would mean privatising the
  field, which this entry deliberately does not do. *Iterating* it is gone too, and deliberately: hash
  order is not in the determinism contract (§5) and nothing in the crate iterates it, so the
  omission is free — iterate the `Vec` `PhysicsWorld::entities` and look each handle up.
  *Constructing* one is NOT gone: `impl FromIterator<(u32, usize)>` keeps the bare
  `solve_joints`/`solve_contacts` embedding path (no `PhysicsWorld` involved) open without naming
  the hasher. This is a **breaking change** for anyone who read the field as a `HashMap`; see the
  CHANGELOG migration note.
  The seal is guarded by five `compile_fail` doc-tests compiled as an external consumer
  (`src/world/entity_index_map.rs`): no `insert`, no `iter`, no coercion to
  `&HashMap<u32, usize, _>` — that third one is the `Deref` trap — no `AsRef`, and no `raw()`.
  Note that this
  toolchain's rustdoc **ignores the expected error code** on a `compile_fail` fence (verified
  2026-08-09 on rustc 1.97.1: `compile_fail,E0999` against an actual E0624 still passes), so the
  fences are bare and the observed diagnostic is recorded in prose next to each of the first four
  (the fifth is marked as read off the signature, not observed). Three of the
  five compile under the pre-seal shape and so are proofs; `AsRef` failed before the seal too, and
  `raw()` did not exist before it, so those two are forward guards only. The `raw()` fence was
  added by the review pass afterwards: `raw()` is `pub(crate)` and widening it to `pub` is the
  likeliest future regression, and none of the other four fire on it — `insert` stays private,
  `iter` stays absent, and the wrapper still does not coerce.
  `gizmo-physics-core` uses `FxHashMap` too but only behind `pub(crate)`
  (`broadphase/aabb_tree.rs:59`), and the transitive path through
  `PhysicsWorld::spatial_hash` → `SpatialHash` → `DynamicAabbTree::entity_map` was re-checked
  and is closed at both hops, so that crate stays clean.

- **`serde` / `serde_json` — on Stage A public surfaces, accepted under criterion 1, listed here
  because the contract claims one entry per dependency that is on such a surface.** `serde` is
  everywhere (derives on public types; a derive of a foreign trait is criterion-2 reachable).
  `serde_json::Value` is named outright by `gizmo-core`'s `pub type GetJsonFn` / `SetJsonFn` and
  by the public `ComponentRegistry::{get_json_fn, set_json_fn}` fields — which `gizmo-scene`
  re-exports as `SceneRegistry`. `gizmo-physics-rigid`'s `SnapshotError::Serialize(serde_json::Error)`
  is a public variant payload: the identical shape to `SceneError::Parse(ron::…)` that was sealed
  above, left unsealed on purpose — and, found by the 2026-08-09 rustc-hash sweep and added here
  because this list claims to be complete, so is the `impl From<serde_json::Error> for
  SnapshotError` beside it, the same shape as the two ron `From` impls kept on `SceneError`.
  Both crates are at 1.0 with a years-long major cadence, which
  is the whole content of criterion 1 — but note the asymmetry is a *judgement about cadence*, not
  a difference in shape, and it should be re-checked rather than assumed at 1.0 time.

- **Verified clean, recorded so the next audit does not re-derive it (2026-08-09 review).**
  `rodio` 0.17 in `gizmo-audio` — 0.x and fast-moving, but every `Sink`/`OutputStream`/`SpatialSink`
  is a private field and `AudioError` carries only `String`/`io::Error`, so nothing is reachable.
  `bincode` 2 in `gizmo-net` — the `Configuration` const is private and both `encode_packet` /
  `decode_packet` return `std::io::Result`. `rand` 0.10 and `chrono` 0.4 — locals inside function
  bodies only. `web-time` in `gizmo-core` (`profiler.rs`) and `gizmo-ai`/`gizmo-physics-rigid` —
  private fields and locals; the `SceneSnapshot` field was the only public one. `wide`, `dashmap`
  and `crossbeam-queue` in `gizmo-physics-rigid`, and `uuid` in `gizmo-core` (declared twice) and
  `getrandom` in `gizmo-math`, have **zero** source references — dead dependencies, not leaks, but
  they should be dropped in a separate build-graph pass. (Re-confirmed for `gizmo-physics-rigid`
  on 2026-08-09 during the rustc-hash seal.)

- **`gizmo-physics-rigid` swept for the other two shapes of criterion 2, 2026-08-09** — the two
  the audit demonstrably nearly missed, done while the rustc-hash seal was open. **Nothing found.**
  The crate declares **no `pub type` alias at all** (so the `FxHashMap` shape had no siblings) and
  **no associated type in any trait impl** (so no repeat of `<ContactPoints as IntoIterator>::IntoIter`;
  the only `IntoIterator`-ish code in the crate is the wasm-only `parallel_compat` module, which is
  private). Every trait implemented on a public type is either `std` (`Default`, `From`, `Debug`,
  `Display`, `Error`, and the new `FromIterator` on `EntityIndexMap`) or already contracted above
  (`serde` derives; `bevy_reflect::Reflect` behind default-off `reflect`). Method used, recorded so
  it can be repeated: `cargo doc --no-deps` then grep the rendered HTML for every dependency name —
  the only foreign hits on the default surface are `glam`, `serde_json` in `SnapshotError`, and
  prose.

- **`tracing-subscriber` vs `tracing` — the same project, weighted differently on purpose; this is
  criterion 1 in both directions inside one crate.** `tracing-subscriber` (0.3.23) is behind
  gizmo-core's default-OFF `tracing-layer` feature, because `impl Layer<S> for GizmoTracingLayer`
  is a foreign trait on a public type of ours and a trait impl cannot be hidden — it constrains us
  exactly as a public field would, so a tracing-subscriber 0.4 would force a gizmo-core 2.0.
  `tracing` itself (0.1.44) stays an UNCONDITIONAL dep: 0.1 has been frozen for years, and neither
  `info!` nor `#[instrument]` leaves a `tracing` type in any signature of ours, so the low version
  number costs the contract nothing. `gizmo-studio` opts the feature in so `init_tracing()` still
  exists for the editor console; CI runs `cargo test -p gizmo-core --features tracing-layer`.

- **`bevy_reflect` — sealed behind the default-OFF `reflect` feature** (with a serde fallback).
  Unchanged; the archetype for criterion 3.

- **`wgpu` in `gizmo-physics-soft` — an explicit CARVE-OUT from the 1.0 promise.**
  `gizmo-physics-soft` is a Stage A crate, but its default-OFF `gpu_physics` feature compiles
  `pub mod gpu_compute`, whose `GpuCompute` holds `wgpu::Device` / `Queue` / `ComputePipeline` /
  `BindGroupLayout` / `Buffer` in public fields. Nothing in this workspace enables it. Therefore,
  said explicitly rather than left to be inferred: **enabling `gpu_physics` opts that crate out of
  the 1.0 stability promise.** With the feature on, a wgpu major is a breaking change for
  `gizmo-physics-soft` and will ship as a minor bump, not as a 2.0. This is the weakest of the
  carve-outs — default-off, unused in-tree, and the module is a compute path rather than an API
  anyone builds on — but the promise has to name it, because a 1.0 that is silently
  conditional on features is not a promise. (The feature also pulls `bytemuck` and `pollster`;
  wgpu is the one with the cadence.)

- **`renet` in `gizmo-net` — a second feature carve-out, NOT part of the 2026-08-09 audit.**
  Found while writing this section up, so it is recorded as unverified by that audit and needs its
  own confirmation pass. `gizmo-net` is Stage A with `default = []`; its default-off
  `client-server` feature compiles `pub mod client_server`, and there `NetworkClient`/
  `NetworkServer` carry `renet::RenetClient` / `RenetServer` and the two `renet_netcode`
  transports in public fields, with `protocol::connection_config() -> renet::ConnectionConfig` on
  top. renet is at 2.0.0 — major-versioned, and by criterion 1 that settles nothing on its own.
  Same treatment as `gpu_physics` therefore: **enabling `client-server` opts `gizmo-net` out of
  the 1.0 stability promise.** The `rollback` feature is a different matter — it pulls only our
  own crates (and `gizmo-math`, i.e. glam) and stays inside the promise.

- **`wgpu`/`winit`/`egui` in Stage B — a deliberate leak that carries no semver cost while the
  crates that leak them stay at 0.y themselves.** This is the entire reason staging exists; see
  the Stage B list above. Note that only `winit` (0.30.13) and `egui` (0.34.3) are 0.x here —
  `wgpu` is 29.0.3, and it is on the Stage B surface not because of its version number but because
  of its cadence. The leak turns into a cost only if Stage B is ever taken to 1.0, which is gated
  on all three of them settling in the criterion-1 sense.

96 public types are `#[non_exhaustive]`; 13 Error enums + fn→Result conversions.

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
- **What the snapshot carries is a compile-time decision (2026-08-14).** `snapshot()` and
  `restore_snapshot()` were two hand-written nine-field lists against a 28-field `PhysicsWorld`,
  and every field in `WorldSnapshot` carries a comment naming the divergence that earned it a
  place — gravity fields, joints and weather were each added *after* a resimulation ran under
  state the continuous run no longer had. Omitting a field is not an error, just a rollback that
  restores less than it claims, and the symptom surfaces somewhere else. Both directions now
  destructure exhaustively with **no `..` arm**, so a new `PhysicsWorld` (or `WorldSnapshot`)
  field fails to compile there until someone answers "carried state, or not — and why"; the
  `_`-bound names are that answer written down. Behaviour is unchanged: the same nine fields
  travel, `headless_stress_test` gives three matching hashes, and the guard was verified by
  adding a field and watching `E0027` land on the pattern.

---

## 6. Migration & Graphics Upgrade (0.1 → 0.2, completed)

The "1.0-readiness hardening + graphics upgrade" breaking release (2026-06-25):

- **MSRV → Rust 1.92** (egui 0.34's floor; previously 1.89).
- **Graphics stack:** wgpu 0.20→**29.0.3**, winit 0.29→**0.30.13**, egui→**0.34.3**
  (+ egui-wgpu/winit 0.34.3, egui_dock 0.19.1, transform-gizmo-egui 0.9.0), naga 29.
  The determinism hash (598E315D0E7499FF) did not change across the whole upgrade.
- **API breakers:** the `glam` re-export was made official; `bevy_reflect` was moved behind the
  `reflect` feature; ~~`arrayvec` left the public surface~~ (**wrong — corrected below**);
  96 types became `#[non_exhaustive]`; Error enums + Result returns. For the detailed 11-item
  migration steps, see the git history (the 0.2.0 commits).
  - **CORRECTION (2026-08-09), kept here rather than deleted:** arrayvec did NOT leave the public
    surface in 0.2.0. That release made the `ArrayVec` storage field of `ContactPoints` private,
    which is all anyone checked, but `<ContactPoints as IntoIterator>::IntoIter` went on reading
    `arrayvec::IntoIter<ContactPoint, 4>`. The by-value iterator was sealed only later, by the
    opaque `ContactPointsIter` — see the arrayvec entry in §4, and criterion 2 there for why the
    rule in force at the time could not see it.
- **Code decision (explains the current code):** winit 0.30 still offers the deprecated
  `EventLoop::run(closure)` → gizmo-app's ~600-line closure event loop was moved to
  `ApplicationHandler` DELIBERATELY (see `crates/gizmo-app/src/windowed/`).

---

## 7. Closed Research & Non-Goals

**Metal draws less of a double-sided interior than Vulkan — OPEN, measured, do not re-chase as a
threshold bug** *(2026-08-16)*. `a_double_sided_material_is_drawn_from_behind` fails only on the
macOS runner. It reads like a mis-calibrated pixel threshold and it is not: the assertion was
rewritten to sample the CENTRE of the frame — a camera inside a cube is looking at a wall there
under any projection — and Metal changes **43.8%** of those pixels where Vulkan changes over 90%.
A framing difference cannot leave half the centre of a wall untouched, so the amount of interior
actually drawn differs on that backend. The test is `#[ignore]`d on macOS with the number written
into its doc comment rather than loosened, because loosening it would hide the difference on every
platform. Diagnosing it needs a Mac to run on. Two things measured along the way and worth keeping:
the old assertion compared BYTES including a constant alpha channel, so its ratio was capped at
0.75 before any geometry was drawn; and the test's claim to guard "either pipeline selection" was
false — reverting the z-prepass arm leaves it green (the prepass writes only depth, and the sole
occluder here is the surface under test), so that arm is unguarded.

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
- **CLOSED 2026-08-17 (was: "OPEN — the extreme N≥48 tower still buckles").** It stands, and has
  since `be46e01` closed the one-point-manifold seed; `soak_extreme_tower_n48_stays_bounded` is
  un-ignored and green. The friction-aware whole-chain direct solver this waited for
  (`direct_chain_solve` + `solve_island_normals`, normals only, O(n³)) is not needed for it and is
  not planned. See §7.

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

**Sub-phase timers (2026-08-13).** `PhysicsMetrics` now carries six finer fields —
`solver_{order,prepare,sweep,relax}_ms` and `narrowphase_{dispatch,manifold}_ms` — fed by
module-level atomics in `profile.rs`. Globals rather than fields because `solve_contacts`
takes `&self` so islands can be solved in parallel from one `Copy` config; every scope wraps
per-island or per-substep work, never per-contact. Determinism-neutral by the same argument
the existing phase timers make: written and read, never branched on — hash `A462C9EB8A09D5CA`
is unchanged with them in. **Islands are solved in parallel, so these are CPU-time shares and
can sum past wall-clock; read them as proportions.**

What they say, and it is the same answer in both scenes:

    1000 boxes, settled   solver 3.95 CPU-ms   order 2%  prepare 4%  SWEEP 79%  relax 16%
                          narrow 0.51          maths 55%  plumbing 45%
    1000 spheres, awake   solver 8.63          order 7%  prepare 5%  SWEEP 74%  relax 14%
                          narrow 2.56          maths 75%  plumbing 25%

**Three quarters of the solver is the sweep** — the constraint iteration itself, not the
scaffolding around it.

That reading led to a conclusion recorded here for a day and **since refuted by measurement**:
that the gap is a convergence-per-iteration difference and closing it means a better-converging
sweep. See "What the iteration budget is actually worth" below. Three quarters of the *solver*
is not three quarters of the frame, and the sweep turns out to be the smallest of the three
factors in the Rapier gap rather than the whole of it.

**What the iteration budget is actually worth (2026-08-14, `benchmarks/vs-rapier` with
`ITERCURVE=1`).** Rapier's default is `num_solver_iterations: 4` per island per frame with no
substepping; we run four substeps of `iterations` (20), with `adaptive_iterations` raising deep
islands to `max(28, 1.5·D)` regardless of the base. So we run about **20× the constraint
iteration per island per frame** — and, at 9× the total frame cost, each of our iterations is
therefore about **2× cheaper than theirs**. The gap was never in what an iteration costs.

Nor is it in how many we run. Sweeping the count on the thousand-box pile, `adaptive_iterations`
off so the setting is actually honoured, and reading **only the frames where bodies are still
moving** (see the trap below):

    iterations   awake ms/frame   sweeps/frame   99% asleep at
    32           5.029            2346           frame 74
    20 (default) 4.431            1586           frame 75
     8           3.627             712           frame 77
     2           3.644             176           frame 78
     1           3.636              97           frame 84

**The entire variable cost of the constraint iteration is 0.795 ms of a 4.431 ms awake frame —
18%**, and below eight sweeps it is flat. A solver that converged perfectly in a single sweep
would make this scene 18% faster and nothing more. It cannot close a 9× gap, and neither can any
amount of work on how well the sweep converges.

Two traps this measurement had to get past, both of which had already produced a wrong number
once. **The 300-frame average is mostly sleep** — the pile settles around frame 75, so three
quarters of it measures a sleeping scene and says nothing about the solver; the first version of
this table read a "floor" of 86% that was very largely sleep. And **the base count is overridden
for deep islands**, so with `adaptive_iterations` on, taking `iterations` from 20 to 4 cuts sweeps
only 40% and buys 1% of frame — the first curve looked flat for that reason, and it was the sweep
*count* column, not the milliseconds, that gave it away.

So the honest decomposition of the gap, measured on awake frames only (Gizmo 4.340 ms × 75 awake
frames against Rapier 0.504 ms × 116 — they stay awake longer, so on total work done the gap is
**5.6×**, not the 9.2× the all-frames average reports):

| factor | measured | is it optional? |
|---|---|---|
| substep multiplier | **4×** | deliberate — it is what survives 320 m/s without tunnelling |
| per-substep pipeline outside the sweep | **1.8×** | open — broadphase, narrowphase, prepare, integrate, run four times |
| the constraint iteration itself | **1.2×** | 18% ceiling, measured above |

The lever is the middle row, or the top one at a price. It is not the sweep.

**Where the middle row is, phase by phase (2026-08-14, awake frames only).** The recorded phase
table was also a 300-frame average and so also three quarters sleep; read over the frames that
have work in them, and divided by four so a substep is compared against Rapier's step:

| phase | Gizmo / awake frame | per substep | Rapier / step | ratio |
|---|---|---|---|---|
| broadphase | 0.439 | 0.110 | 0.046 | 2.4× |
| **narrowphase** | **1.032** | **0.258** | **0.064** | **4.2×** |
| solver | 2.479 | 0.620 | 0.297 | 2.1× |
| integration | 0.114 | 0.029 | 0.028 | 1.0× |

**The narrowphase is the worst ratio in the engine** — 4.2× per substep, a quarter of the awake
frame — and it splits into a parallel section (the collision maths, 0.80 ms) and a **sequential**
assembly of manifolds, contact cache and events (0.36 ms, 8% of the awake frame, on one thread
while the rest idle). That sequential 8% is the concrete target the "per-substep pipeline" row
resolves to.

**But it has no single lever, measured.** The warm-start pass was pure per-pair work over
immutable state and has been moved onto the worker that produced the contacts (behaviour-identical
— hash `A462C9EB8A09D5CA` unchanged, since the per-pair computation is order-independent). It was
**10%** of the sequential loop. The contact-cache inserts, the next obvious suspect, measure
**0.02 ms — 5%**. The remainder is manifold construction and copying, spread thin. This is the
same shape as the allocation investigation: a real cost with no concentrated win in it, and it
should be treated the same way — do not spend more here without a measurement that names a
specific 30%+ of that 0.36 ms.

Two instrumentation corrections came out of the same work, both worth knowing about because both
had already produced a wrong number. The `dispatch` scope was **per-pair** — ~7300 `Instant::now()`
plus a `fetch_add` on one shared counter per frame, violating `profile.rs`'s own stated rule and
costing 3% of the phase it measured; it now wraps the whole parallel section and reports wall
time. And every phase timer in the benchmark now accumulates on awake frames only, per engine,
because the two engines sleep at different frames (Rapier 116, us 75) and a shared divisor
mis-scales one of them.

**The solver carries no fat (2026-08-13, `benchmarks/vs-rapier` with `ABLATION=1`).** With
no profiler available, the solver's own switches were used as one: turn each off, measure
the thousand-box pile, and the difference is what that feature costs. **Every switch is
load-bearing, and turning any of them off makes the frame slower, not faster.**

    all on (default)              2.065 ms/frame   1/1000 awake   mean y 3.98
    support_ordering off          2.481            1/1000         3.98
    adaptive_iterations off       2.381            1/1000         3.98
    iterations 20 -> 8            2.438            1/1000         3.98
    block_solver off             17.418         1000/1000         1.54
    use_tgs_soft off             36.135         1000/1000         1.56

The last two columns are the explanation: without the block solver or TGS-soft the pile
never settles — a thousand bodies stay awake and the stack collapses from a mean height of
3.98 to 1.54 — and an unsettled pile costs far more than the feature that settles it. The
block solver repays itself fifteen times over, TGS-soft thirty-four. Even the small knobs
go the same way: *fewer* solver iterations is **more expensive**, because convergence gets
worse and the pile takes longer to come to rest.

So nothing in this table is waste to be reclaimed. But note what it does **not** say, because it
was once read as saying it: that the iteration count is load-bearing. One point cannot tell a
floor from a slope, and the curve measured in "What the iteration budget is actually worth" above
shows this scene is insensitive to the count between 1 and 8. `iterations 20 → 8` costing more
here is a real effect of a different kind — it is the *adaptive* path and the settle time moving,
not convergence failing.

**Written, exported, and not wired to anything (2026-08-14 survey).** `LodGroup` turned out to be
a capability the engine's own default path could not reach — the components existed, `select_mesh`
existed, and only `gizmo-studio` ever looked. That was the second one (`Frustum::test_aabb_masked`
sat at zero callers until `gizmo-renderer::visibility` found it), so the class was worth sweeping
for rather than meeting one at a time. What the sweep found:

| what | state | shape |
|---|---|---|
| **skeletal animation** | **FIXED 2026-08-14.** `animation_update_system` and `animation_state_machine_update_system` lived in `gizmo-renderer` with `current_time += dt · speed` appearing nowhere else in the workspace, and nothing in the facade, in `gizmo-app` or in any demo ever called either. `default_render_pass` now calls both, before the draw path reads `Skeleton` | the draw path *consumes* `Skeleton` (skinning matrices in `collect_draw_items`), so the engine rendered a pose it never advanced — a clock with no hands, and everything downstream of it looked wired |
| **`ParticleEmitter`** | **FIXED 2026-08-14.** The component, the GPU pipeline and the draw call were all present — `default_render_pass` already ran `update_params` and `compute_pass`, and `passes/forward` already drew the result. Nothing ever *put anything in it*: the emitter→GPU bridge lived only in `gizmo-studio`. It is now `systems::render::spawn_from_emitters`, called from the pass | exactly `LodGroup`'s shape: the whole path present, one link missing, and the link living in studio |
| **`Sprite`** | **DELETED 2026-08-14.** Referenced by nothing in the workspace — not studio, not the editor, not a demo, not even `gizmo-app`'s scene registry, which registers its sibling `Camera2D` | dead, and unlike the other three there was no link to restore: wiring it meant writing a 2D pipeline (billboarding, layer sort, atlas UVs, transparency), which is a feature nothing asked for. An exported component that cannot be drawn is a promise the API does not keep, and the 1.0 surface is the wrong place to keep it |
| **`gizmo-ai`'s systems** | `behavior_tree_system`, `ai_navigation_system`, `ai_navmesh_rebuild_system` are re-exported and never scheduled | the same shape, but plausibly deliberate: when AI ticks is a game's decision, not an engine's. Left alone, and named here so the next sweep does not re-find it as news |

**Why it was never wired — and the first explanation here was wrong.** This section originally said
both systems take `(&mut World, dt, &wgpu::Queue)`, that no ordinary system has a queue, and that
"there was no schedule they *could* be added to — that is very likely the whole story". It is not.
`gizmo-studio/src/render_pipeline/mod.rs:20` was **already calling `animation_update_system` with
exactly that signature**, from exactly the position the fix now uses: holding the world and the
queue, before the draw. The signature was never the obstacle.

The real reason is structural and is the same one behind `LodGroup` and `ParticleEmitter`: **render
wiring happened in `gizmo-studio`, and the engine's own `default_render_pass` is a second,
independently-maintained copy of the same job.** A capability wired into studio still has an in-tree
consumer, so the obvious check — "does anything read this component, does anything call this
system?" — answers *yes*, and the gap is invisible to precisely the sweep you would run to find it.
That is why the sweep below, which looked for zero callers, found `Sprite` and missed these.

The drift runs both ways, which is the proof it is structural rather than a backlog: the fix landed
on the engine path only, so `animation_state_machine_update_system` is now engine-only exactly as
`BoneAttachmentSystem` is studio-only. `default_render_pass` is the one place holding the world,
the queue, and a position before `collect_draw_items` reads the result, so they are called from
there, with `dt` from the `Time` resource.

The guard is `golden_render_tests::default_render_pass_advances_skeletal_animation`, which drives
the real pass over a one-joint rig and asserts the clock moved — and it was checked against a
build with the call removed, where it fails. It deliberately asserts the *wiring* and not the
arithmetic: `normalize_anim_time` already had tests for looping, clamping and zero duration, and
every one of them passed throughout the years the feature did not run. A test of a policy is not a
test that the policy is reached.

**Two notes from the particle fix.** The bridge sits in the facade rather than in `gizmo-renderer`
because it needs both halves and they are in different crates — `GpuParticleSystem` is in the
renderer and `Transform` is in `gizmo-physics-core`, which the renderer does not depend on. And it
does its spawn jitter with a private four-line xorshift seeded from `Time::frame()` and the entity
id, where the code it replaces called `rand::rng()`: the facade would otherwise gain a dependency
that everyone building `gizmo-engine` pays for, and a deterministic engine emitting
nondeterministic sparks is a small lie. Neither version ever touched simulation state, so this was
never a contract violation — only an avoidable inconsistency.

`gizmo-studio` still carries its own copy of the loop and **that is deliberate, not an oversight**:
`spawn_from_emitters` takes `&mut World`, which is a signature with no borrowing precondition,
while studio's pipeline holds a live read borrow across the whole block. A `&World` variant would
serve both, but its soundness would depend on what borrows the caller happens to hold — a
conditionally-safe public API is a worse trade than one duplicated loop. The duplication is
written down at both ends instead of hidden.

**Where the sweep ended.** Three of the four were the same defect — a link missing from an
otherwise complete path — and all three are fixed. The fourth was not a missing link but a missing
feature, and it was removed instead. The lesson worth carrying is the one animation taught: every
policy in that path had tests, and every one of them passed for as long as the policy was
unreachable. **A test of a policy is not a test that the policy is reached**, and the three fixes
are each guarded accordingly — `default_render_pass_advances_skeletal_animation` fails with the
call removed, and it was checked that way rather than assumed.

**The root the sweep could not see (2026-08-14, architectural review).** The sweep above looked for
*zero callers* and found `Sprite`. It could not have found `LodGroup`, `ParticleEmitter` or the
animation systems, and the reason is structural: **`gizmo-studio` is a workspace member, so a
capability wired only into its render pipeline still has an in-tree consumer.** The obvious check —
does any system read this component? — answers *yes*, and the gap is invisible to precisely the
question you would ask.

Underneath that is the real shape: `collect_draw_items` and `gizmo-studio`'s
`execute_render_pipeline` are two independent implementations of *world → draw list*, and only the
pass recording after that genuinely needs to differ. `shared.rs` says as much in its own header —
light collection and cascades were single-sourced *after* each had to be fixed twice. Everything
that has not yet caused a visible bug is still duplicated, so the default state of any new
capability is "lives in exactly one path". The drift runs both ways, which is what makes it
structural: `animation_state_machine_update_system` is now engine-only exactly as
`BoneAttachmentSystem` is studio-only.

**First cut taken: `gizmo-renderer::routing`.** Both loops decided what every `MaterialType` means
with a `match … { _ => 0.0 }`, and the two wildcards disagreed — `BakedLit` was routed by the engine
path and defaulted by studio's, so a baked-lit level shaded one way in the game and another in the
editor; `Grid` was the reverse. `MaterialType` is `#[non_exhaustive]`, so a wildcard is *obligatory*
in any downstream crate and a ninth variant could never have been a compile error in either file.
The decision now lives in the crate that defines the enum, where the match is exhaustive: one
compile error there instead of two silent misroutes out here. The engine path is behaviour-identical
(12 golden render tests, hash unchanged); studio's `BakedLit` flag goes 0.0 → 1.0, which is the fix
and is a visible change to a viewport nothing can test.

What this deliberately does **not** do is merge the two paths. The deferred and editor-forward
recorders genuinely differ, that difference is the part with no automated coverage, and §3 gates it
on human-eye A/B. Single-sourcing the *semantics* is the half that pays.

**Second cut taken: `gizmo-renderer::frame_uniforms`.** The two uniform blocks — `SceneUniforms`
(18 fields) and `PostProcessUniforms` (16) — were hand-filled as exhaustive struct literals at
**six** sites: both draw loops, the renderer's two initial buffers, and three demos with custom
render callbacks. An exhaustive literal *looks* like the safe form, because omitting a field does
not compile; what it actually checks is "was every field filled", and all six passed that. The
question nobody was asking is "which field differs, and why":

- `PostProcessUniforms::cam_near`/`cam_far` exist because DoF linearises depth with them, and the
  field comment says a hardcoded range miscalibrates the circle of confusion. **Five of six sites
  hardcoded `0.1`/`2000.0` anyway** — including studio, so the editor viewport's DoF was
  miscalibrated for exactly the cameras the field was added for. This is the one live bug the cut
  fixes, and it is a viewport change nothing can test.
- `SceneUniforms::cascade_params.x` is documented in `common.wgsl` as the camera's z-near. Studio
  sent `cam_near`, the engine sent a literal `0.1`. They had disagreed for as long as both paths
  existed **and it cost nothing**, because no shader reads `.x` today. `SceneUniforms::exposure` is
  dead in the same way (the post composite owns exposure; `deferred_lighting.wgsl` says so next to
  the field it no longer reads) and had drifted the same way: camera's in the engine, `1.0` in
  studio.

Two dead fields drifting is not the finding. The finding is that a *live* field drifts by the same
mechanism and is just as invisible — `cam_near` is the same class of mistake, and it was live.

Derived work (the inverse view-projection, the packed `cascade_params` slots, the `w` flags, the
padding, the shadow texel size) now happens once in `SceneUniforms::new(&SceneFrame)`; what the two
paths legitimately disagree about is arguments at a call site with the reason next to them —
studio's identity cascades when nothing casts, its point-shadow lookup left off because it records
no cube pass, its exposure from the editor slider. The engine path is byte-identical (12 golden
render tests, hashes unchanged) except for the two dead fields.

The guard is `no_hand_filled_uniform_literals_outside_the_constructor`, and it **walks the
workspace** instead of naming the files it knows about — deliberately, because that is the flaw in
the shader mirror tests below: each hand-counts its subjects, so a new shader is invisible to the
test that exists to police it. A partial literal (`..Default::default()`) is the sanctioned escape;
an exhaustive one is a test failure the day it is written. Verified by reintroducing one.

**The same flaw, on the shader side: `gizmo-renderer::shader_contract` (2026-08-14).** `common.wgsl`
opens by declaring itself the single source of truth for the scene uniform layout. It is — for the
shaders that `#import` it, and **seven still declare their own `SceneUniforms`**, because whether a
shader shares the definition depends on whether its Rust call site reached for
`load_shader_composed` rather than `load_shader`. Nothing in the shader says which it is. The seven
are legitimate — each is a *prefix* of the block, truncated after the last field it reads, which is
how a shader avoids declaring 1168 bytes to read a view-projection — but nothing checked the
prefix, and the tests that looked like they did could not: one pins Rust's struct sizes while
calling itself "a contract with the WGSL side" without opening a `.wgsl` file, and the other reads
ten shaders **from a hand-written list** and counts `vec4<f32>` occurrences rather than checking
where the fields land.

The replacement takes its subjects from the shader directory and its answer from naga: every shader
declaring `SceneUniforms`, `InstanceRaw`/`InstanceData` or `LightData` is parsed, and each named
field's byte offset is compared against `offset_of!` on the Rust struct that fills it. Padding is
ignored by name — its effect is entirely visible in where the named fields land — and types are not
compared, which is what lets `gbuffer.wgsl` keep its deliberate `array<vec4<f32>, 40>` in place of
ten `LightData` while a field that *moved* still fails. No drift was found; the eight declarations
agree today. Verified by reordering two fields in one copy and dropping one from another, and
watching the two tests name the file, the field and both offsets.

This also closes the "`compose_wgsl` builds a `naga::Module`, validates it and throws it away"
entry, and closes it honestly rather than by refactoring for its own sake: `compose_module` now
returns the validated module, `compose_wgsl` is the thin text wrapper over it, and the contract
tests are the consumer that needed it — the composed shaders substitute bind-group indices inline
(`@group(#{INSTANCE_GROUP})`), so naga alone cannot read them and only the real composition path
can.

Two things this is **not** saying. A public function with no in-tree caller is not a defect — a
library's surface exists to be called from outside, and a sweep on that basis returns most of
`gizmo-core`. And a system a game is meant to schedule itself is not unwired. The class that
matters is narrower: **the engine's own default path depends on state that the engine's own loop
never produces**, or a component the engine exports and nothing anywhere can draw. Animation and
`Sprite` are those. `ParticleEmitter` is one step out — nothing is broken, but the feature is only
reachable by reimplementing studio's bridge.

**NON-GOAL: cutting contact-path allocations.** Investigated, REJECTED (2026-08-13) — and
rejected for the same reason as the SIMD item below, in a different currency. A comparison
against Rapier3D (`benchmarks/vs-rapier`) showed us allocating **8158 heap allocations per
frame against Rapier's 60** on a settled thousand-box pile, which looked like the obvious
lever. Three changes cut it 42 % — dormant cache entries moved instead of cloned, the cached
manifold's buffer reused, and `support_order_manifolds`' per-body `Vec`s moved to a
thread-local scratch (that last one alone was 32 % of all allocations, found with a sampling
allocation profile, `PROFILE=1` in that benchmark). **Frame time did not move.** The reason,
measured: a small `Vec` allocate-and-drop costs **8 ns**, so 22 833 allocations a frame are
**0.183 ms of a 9.5 ms frame — 1.9 %.** Removing every one would buy 2 %. The three changes
are kept (they are correct, and behaviour-identical: hash `A462C9EB8A09D5CA` throughout), but
do not spend more here. An allocation *count* is not a frame time.

**Where the Rapier gap actually is (2026-08-13).** Same harness, both engines multi-threaded,
identical scenes and `dt`. Quality is level — the 20-box tower drifts 0.000 m against their
0.010, an analytic elastic collision comes out 4.950 against their 5.000 — and the thousand-box
pile costs 1.7 ms against 0.17. That splits in two: **~4× is the substep multiplier**
(`PHYSICS_HZ = 240` runs the whole pipeline, collision detection included, four times per
rendered frame — deliberate, and it is what makes 0.5 m spheres survive 320 m/s without
tunnelling, measured), and **~2.5× is genuine per-substep cost spread across every phase**
rather than concentrated in one. Neither is allocations. That next step was taken — sub-phase timers, then the iteration curve
above — and it moved the answer: of the ~2.5×, the constraint iteration is 1.2× and the rest of
the per-substep pipeline is 1.8×.

**NON-GOAL: narrowphase batch-SIMD.** Investigated, REJECTED (2026-07-14). Measurement
(wide_scene, 2000 boxes, ~30ms frame): box-box SAT compute is only **~3.3%**; narrowphase
post-processing cannot be batched; both per-pair SIMD attempts regressed (the scalar code is
already auto-vectorized). DO NOT RETRY without passing the step-0 gate again. (The
"~82% narrowphase" figure is OBSOLETE.)

---

### Physics performance baseline (2026-08-05, `benches/step_bench.rs`)

Carried out of the campaign because a baseline nobody can find is a baseline nobody uses. Five
scenario groups in `gizmo-physics-rigid` — broadphase (a contact-free lattice), narrowphase
(overlapped), solver (a settled tower), joints (a hanging chain), full_step (mixed). Every
iteration **rebuilds the scene**: `iter_batched`'s setup stays out of the measurement, and
stepping one world a thousand times measures a different simulation each time (bodies settle,
sleep, the cost collapses, and that reads as a speed-up).

| scenario | 64/8 | 256/24 | 1024/48 |
|---|---|---|---|
| broadphase | 226 µs | 609 µs | 1.73 ms |
| dense_contacts (solver-bound) | 6.96 ms | 27.66 ms | **151.00 ms** |
| solver (tower) | 532 µs | 1.20 ms | 4.05 ms |
| joints (chain 8/32/128) | 161 µs | 317 µs | 755 µs |
| full_step (128/512) | 635 µs | 2.43 ms | — |

**These are a floor, not a ceiling** — measured with this repo's `lto=off` / `codegen-units=4`
dev profile — and they are machine-specific. Use them the only way they are valid: same machine,
before and after.

### The tall-stack sensitivity (N1/N2) — CLOSED 2026-08-17, and the five candidates that died proving what it is

> **Both are gone, and had been for eleven days.** Re-running the measurements on 2026-08-17 found
> N1's ground-size sensitivity and N2's 2 cm-gap collapse both absent, so the cause was bisected
> instead of theorised further (a `git worktree`, not a checkout — the tree is shared):
>
> | commit | date | effect on the 2×12×2 block, gaps 0 → 0.2 m |
> |---|---|---|
> | `947a830~1` | 2026-08-06 | broad: gaps 0.005 (f1640), 0.020 (f70) fall on ground 20; 0.000, 0.005, 0.020, 0.030, 0.050, 0.100 fall on ground 200 |
> | `947a830` sleep whole contact islands | 2026-08-06 | **one cell left**: gap 0.020 only, at frame 17 (ground 20) and 136 (ground 200) |
> | `be46e01` give the depth test the tolerance the slab test already had | 2026-08-06 | **nothing falls**, at any gap, on either ground |
> | today (HEAD) | 2026-08-17 | nothing falls; peak \|v\| 0.030–0.032, peak lean 0.0000–0.0001 |
>
> `be46e01` is the same root as the recorded *seed* (`43c51b0`): `clip_box_box` rejected corners at
> `signed_depth <= 0.0`, two boxes at exact contact have all four corners at exactly `0.0`, so
> Sutherland-Hodgman returned empty, the swapped-reference retry too, and the pair fell through to
> GJK/EPA — **one** contact point. A one-point manifold carries no tilt-resisting torque, which is
> exactly the margin a marginally-stable stack has none of. Nothing about warm start, ground-face
> float detail or friction was ever the mechanism.
>
> Two things follow. **The N≥48 non-goal is retired**: `soak_extreme_tower_n48_stays_bounded` is
> un-ignored and green (peak \|v\| 0.379, resting penetration 0.0013, 1500 frames, 0.4 s release /
> 3.5 s debug), and the "friction-aware whole-chain direct solve" it was waiting for is not needed
> and not planned. **And the ground-extent effect is gone with it**: N=16/32/48 at half-extents 20,
> 140, 150 and 200 all report identical peak \|v\| to four decimals and peak lean 0.0000 — 140 and
> 150 are the two sizes recorded below as falling.
>
> The 2 cm cell is now watched by a real gate, `a_block_with_a_two_centimetre_lateral_gap_stays_standing`,
> verified to fail at `be46e01~1` ("collapsed at frame 17, peak |v| 0.648") and pass at HEAD. The
> record below is kept as-written because the refuted candidates are still refuted — a future
> instability must not re-chase them — but its open threads are answered above.

**N1 — ground size changes tower stability.** Mechanism found: it is the **sleep path**. Forcing
every body awake makes the whole effect vanish; ground size only moves *when* the stack sleeps.
What size really does is raise the **resting amplitude** — peak lean is ~0.011 up to half-size 21
and saturates at ~0.018–0.024 from 50 on — and collapses appear only in that upper band. *Which*
size collapses inside the band is chaotic: 140 and 150 fall, 160/175/190 stand, 200 falls. What
raises the amplitude is still unidentified, and that is the thread to pull.

Refuted on the way, and none of them worth re-testing:

| candidate | measurement | verdict |
|---|---|---|
| birth-kick magnitude scales with ground | `does_a_bigger_ground_deliver_a_bigger_birth_kick` | **dead** — 0.58 % across 20→200 |
| tangential ratchet (accumulation) | `is_the_lean_slip_or_rotation` | **dead** — slip oscillates, path/net ≈ 21 |
| accumulated slip discriminates survivors | same | **dead** — survivors accumulate MORE (0.521 vs 0.462) |
| contact jitter grows with ground | `does_a_settled_contact_jitter_more_on_a_bigger_ground` | **dead** — a settled contact is bit-stable at every size, 0 jitter over 600 frames |
| "no mechanism, just a different sample" | ground 20.000 → 20.001 → 20.01 → 20.1 → 21.0 | **dead** — a millimetre on a 40 m box does not scatter the result; peak leans stay within 0.004 |

### The slope a crate could not stand on (2026-08-17, FIXED)

Found by chasing the trigger left on the "friction has no positional term" item (§3) — the item
asked for "a measured creep", and looking for one turned up something much larger.

The default material is `μ_s` 0.6 / `μ_d` 0.5, so the friction angle is `atan(0.6)` = 30.96° and a
body at rest on anything shallower must not move. Measured, 1 kg crate, 10 s after settling:

| slope | tan θ / μ_s | before | after |
|---|---|---|---|
| 25° | 0.78 | 2.8 mm | 2.8 mm |
| 28° | 0.89 | **26 m — left the plate** | 6.7 mm, at rest |
| 30° | 0.96 | **77 m, still accelerating at 27 m/s** | 9.6 mm, at rest |

And on the flat, a box pushed at a constant fraction of its static limit crept **linearly, forever**
— 0.000533 m/s at 99 % (107 mm over 200 s), 0.000030 m/s at 90 %. After the fix: 0.000071 and
0.000001 m/s (7.5× and 44×).

**The coefficients were never the fault.** Setting `μ_d = μ_s` = 0.6 made all three slopes hold;
so did 1.2/1.2, identically — the magnitude changed nothing, the *gap* changed everything. That is
what pointed at the transition:

```rust
if mag > max_static { scale to max_dynamic }   // the old test, in five places
```

`mag` is the impulse *demanded on this sweep* and `max_static` is `μ_s·λ_n` — but `λ_n` fluctuates
between sweeps and substeps, so a contact standing perfectly still occasionally demands more than
the static cap and is charged the dynamic rate for it, losing `(μ_s − μ_d)·λ_n` of hold it was
entitled to. Below `atan(μ_d)` = 26.57° the loss shows up as creep; above it, gravity outruns
dynamic friction and the slip feeds itself, which is exactly the band 26.57°–30.96° where the crate
ran away. The sharp cliff between 25° (2.8 mm) and 28° (26 m) is that boundary, not a threshold in
the code.

The fix is to decide the budget from the contact's **actual tangential speed**, which is what
"sliding" means and what PhysX's `PxMaterial` does: static below
`ConstraintSolver::static_friction_velocity_threshold` (new, 1 cm/s), dynamic above. The five
copies of the clamp — TGS sweep, block sweep, island sweep, SI path, standalone one-shot — now call
one `friction_limit` helper, because five copies is how they would have come to disagree about it.

Gate: `soak_and_golden::a_crate_holds_on_a_slope_below_the_friction_angle` (28° and 30°, both
inside the band). **Determinism unchanged** — `headless_stress_test` gives three matching
`A462C9EB8A09D5CA`, the same value as before the change, because its 2000-box collapse slides far
above the threshold and never takes the static branch; no golden scene moved either. That is worth
stating precisely rather than as reassurance: this change is invisible to the determinism gate, so
the *slope* gate is the one that guards it.

### Do not re-chase (carried from the campaign)

- **`gizmo-audio`'s cfg-gated `unsafe impl Send/Sync`.** Correct, and justified in place: a
  single-threaded wasm build with `atomics` off, and the impls disappear if `atomics` is enabled.
- **The nine audit claims that failed adversarial verification.** They were removed from
  `docs/AUDIT-2026-08.md` when it was written; they are not a backlog.
- **bincode.** Staying. The advisory covers every version of the crate (development stopped; "no
  safe upgrade is available"), 2.0.1 is the maintained line, and the alternatives are different
  serializers, not upgrades. Leaving it breaks the wire format and needs its own benchmarks. The
  trigger to reopen: a measured serialization bottleneck, or a defect 2.x does not fix.
- **Moving `Transform` into `gizmo-core`.** Measured 2026-08-17 and declined: depending on
  `gizmo-physics-core` costs nothing a consumer would notice (no solver, no rayon — arrayvec,
  core, math, rustc-hash, serde, tracing), `gizmo-core` is one of only three Stage A crates with
  no glam on its surface, and physics-core's `gizmo-core` dependency is *optional*, so `Transform`
  works without an ECS today — a move would end that. If it is ever paid for, the two shapes are
  (a) core takes glam, or (b) a separate spatial crate in the shape of `bevy_transform`; the
  trigger is D1 (packaging the physics crates independently) actually happening, or a consumer who
  wants the type without the physics vocabulary.
- **The prototype's four egui-0.34 limits** (left-aligned button labels, Lucide icons, a 2 px
  separator between sections, an offset focus ring). Not defects, toolkit limits — recorded in
  code as `gizmo_editor::theme::UNIMPLEMENTED_SYSTEM_RULES` so they are not rediscovered as bugs.

## 8. Working Method

- Every item: **fix → write a regression test → build/test/clippy → tick it off.**
- **Measure the noise floor before reading a difference.** After a day of physics and renderer
  changes, nothing had confirmed a demo still *drew* anything, so one was screenshotted through
  `GIZMO_SCREENSHOT` — a good check, and it passed: geometry, ground, debug overlay, camera where
  expected. Then the same demo was rendered from its pre-change source and diffed: **7.39 %** of
  bytes differed, which read as "the change altered the simulation".

  It did not. A build failure left a stale binary in place, so two renders of the *same* code got
  compared by accident — and they differed by 6.93 %. Measured properly, the same binary at the
  same frame number three times: **6.17 %, 6.49 %, 4.77 %**. A windowed demo steps physics with
  the real frame delta, so "frame 180" is a different amount of simulated time on every run. The
  7.39 % was the machine, not the code.

  Two things follow. The screenshot path is for "is it black, is the camera right, did the shader
  do what I meant" — it is not a regression diff, and CLAUDE.md now says so with the numbers. And
  the accident was the useful part: without the stale binary the false conclusion would have been
  written down. A difference is only evidence once you know what no difference looks like.
- **A configuration that is *built* and never *linted* is outside every gate the project has —
  and the target axis was only the first one.** The wasm lint job exists because that arm was
  compiled by CI and never linted (2026-08-16). The same argument holds one axis over, and nobody
  made it until 2026-08-18: the lint gate runs `--all-features`, so it cannot see code that a
  *smaller* feature set removes, and the feature-powerset job that does build those sets ran
  `cargo hack check` — which compiles the combinations and throws their warnings away.

  Asked properly for the first time — `cargo hack clippy --feature-powerset --depth 2` with the
  same `-D warnings` CI applies everywhere else — **66 of the facade's 150 combinations failed.**
  Not 66 defects: **ten, in two files**, each repeated across every combination that exposes it.
  The first run only reported the first three, because `cargo hack` stops at the first failing
  combination; `--keep-going` is what turns "a warning" into the size of the thing.

  What they were, and none of them is exotic: imports at the top of `systems/physics.rs` naming
  types that only the `render`-gated debug view and GPU-physics systems use; a static counter read
  by one of those systems; a `soft_color` binding sitting one line above the
  `#[cfg(feature = "physics-soft")]` block that is its only reader; and in `bundles.rs`, six
  imports whose users are `render`-only or `all(render, physics)` — including a
  `use gizmo_physics_core::{BoxShape, Collider, ColliderShape};` line directly above a
  `#[cfg(feature = "physics")]` that belonged to it. That is the same detached-`#[cfg]` shape this
  section already records twice from 2026-08-17, and it had been sitting in the tree the whole
  time, in the one configuration nothing looked at.

  **The gate is flipped rather than the tree merely cleaned**, because cleaning without a gate is
  what produced this: `check` → `clippy -- -D warnings` in the powerset job. Measured cost, with
  dependencies cached and only the facade rebuilt: **49 s → 59 s** for 150 combinations, i.e. 20 %
  on a job that already compiles every one of them. `gizmo-app`'s 49 combinations were clean
  already, which is the other half of the number: the debt is where the feature graph is widest.

  **The CI cost, measured on CI rather than guessed from here: 7 min 3 s.** That is the whole
  powerset job — both crates, ~200 combinations, cold clippy artifacts — on the very run that
  introduced the flip (`c693d4e`), against a 45-minute ceiling. The next run took 7 min 19 s.

  **And a misdiagnosis worth keeping, because it is the more useful half.** Two later runs sat at
  `in_progress` for 35+ minutes and the obvious reading was "the clippy flip is what is costing
  this" — a conclusion that fits, follows from the change just made, and is wrong. Looking at
  which *step* was running answered it in one command: both stuck jobs were parked in
  `Install Linux dependencies`, an `apt-get` that never returned, and the two jobs that hang are
  exactly the two that install packages. The change under suspicion had already been measured at
  seven minutes twice.

  The rule that generalises: **a slow job is not evidence about the step you changed.** Ask which
  step is running before believing a story about the one you touched — the same discipline as
  measuring the noise floor before reading a difference, one layer up.

  The job is a two-entry matrix now (one runner per crate, `fail-fast: false`), and that stands on
  its own smaller claim rather than the one above: sequentially, `gizmo-app` failing stops the run
  before `gizmo-engine` is ever linted, which is the `--keep-going` lesson one level up.

- **A count written into prose is a count the code will walk away from.** Three were found stale
  on the same afternoon (2026-08-18), each written once and never re-measured: the rustfmt churn
  behind a standing decision (2660 → **2794**), the scene block's fixed part quoted two different
  ways in adjacent paragraphs (528 vs **560**), and CLAUDE.md's present-tense "96 types are
  `#[non_exhaustive]`" (**122**). None of the three was wrong when written.

  The fix is not diligence. Where the number carries a decision, **compute it** — the uniform
  ceiling is a test now, and the cluster-assignment cost is a benchmark. Where it is background,
  give it a date and say which way it drifts, so a reader can tell a measurement from a constant.
  A count with neither is a claim nobody can check and everybody will quote.
- **A remembered rationale is not evidence, and the cheapest way to find out is to remove the
  thing it defends.** The solver skips its modern TGS-soft path for any island containing a CCD
  body, with a comment giving the reason: TGS's dp/relax flow conflicts with the speculative
  clamp on high-speed angled impacts. That gate has a *known cost*, pinned as an ignored test —
  `ccd_makes_a_bounce_a_thud`: with CCD on, an equal-mass head-on hit at restitution 1 shares the
  momentum evenly instead of transferring it, so a Newton's cradle stops bouncing.

  Tried on 2026-08-18, by deleting `&& !has_ccd` and running the CCD suites:

  - The pinned limitation **passes** — so the cost really does come from that gate and nowhere
    else, which no amount of reading would have established.
  - `ccd_analytical`'s nine tests stay green, including the angled-impact one the rationale names.
  - But `prop_ccd_never_tunnels` found and shrank a counterexample **inside one run**: speed
    1753.91 m/s, half-thickness 0.2967, radius 0.3940 → tunnelled at x = 0.70. A bullet through a
    0.59 m wall.

  So the trade is real and the gate stays. What changed is what defends it: a sentence became a
  counterexample, saved in `ccd.proptest-regressions`. The shipped solver passes that case, so
  keeping it costs nothing and covers a genuinely hard shot. The other route out — restitution on
  speculative contacts in the split-impulse path — is untouched by this experiment and stays the
  open candidate.

  The general point: an ignored test that documents a limitation is worth more than a comment,
  because it can be *un-ignored experimentally*. That is what turned this from folklore into a
  number in an afternoon.
- **"No production caller and no test" is a defect class, not a tidiness question.** The
  frame-rate-dependent force channel (§3) was found by writing a measurement for something else,
  and it had survived because nothing in the workspace wrote to `force_accumulator`. Sweeping the
  four physics crates for the same shape on 2026-08-18: **247 public functions, 56 with no
  production caller, 27 with neither a caller nor a test.** Most of those 27 are builders and
  back-compat aliases that are correct by inspection. Two were not:

  - **`PhysicsWorld::raycast_all` did not bound by `max_distance` at all.** It filtered nothing,
    so a body whose AABB reached into range while its solid sat outside came back anyway — the
    broadphase's bound leaking out as though it were the query's. Measured: a 1×1×20 box turned
    45° and centred 15 m away, queried at `max_distance = 10`, gave `raycast → None` and
    `raycast_all → a hit at 15.29 m`. One ray, one world, two answers. For a game that is "what
    is within 10 m of me" returning things that are not, and only sometimes — depending on how
    loose each body's bounding box happens to be.
  - **`aabb_overlaps_simd4` has two `cfg`-exclusive bodies and no caller.** SSE on x86_64, a
    scalar loop on wasm32; no build compiles both, so nothing could ever compare them. It now
    has a test against the rule written out longhand — the third-implementation trick for when
    the language will not let two be compared. Writing it taught its own lesson: the rule is
    **six** comparisons, a case only exercises the one it sits exactly on the boundary of, and
    the first version had a touching box on one side of one axis — so five of the six could be
    turned strict without it noticing. Each of the six is now covered and each was verified to
    fail on its own.

  **The sweep now lives in the repository**, as `crates/gizmo/tests/unmentioned_api.rs` —
  `#[ignore]`d, in the house style for a measurement rather than a gate. It was a scratch
  script when this paragraph was first written, which made the paragraph describe something
  that did not exist: the same defect class this whole section is about, produced while
  writing about it. It scans `pub fn` declarations in each crate's `src` against mentions
  everywhere, cut at `#[cfg(test)]`. Whole-workspace figures: **107 of 1385 public functions
  unmentioned in production, 81 of those with no test either**. **Its first run lied**, and in the direction that
  wastes time rather than hides bugs: the call regex did not allow a turbofish, so
  `world.query_mut::<(A, B)>()` did not count as calling `query_mut`, and `get_resource`,
  `remove_component` and a dozen other things every file uses came back as "never called".
  A detector that reports the whole ECS as dead code is one nobody reads twice.

  Run over the rest of Stage A afterwards, the honest figure is 21 with neither caller nor
  test — and two of those are **documented behaviour with nothing checking it**, which is
  the same class one layer along:

  - `gizmo_core::state` — `State<S>` is deliberately undriven (the application owns the
    `apply_transitions` call, and the module docs say so), but five behaviours rested on
    prose alone. One is genuinely surprising: `set` compares against the **current** state
    only, so `state.set(current)` does **not** cancel a queued switch — a reader who used it
    as a cancel would get the transition anyway, one frame later, with nothing to point at.
  - `gizmo_ai::behavior_tree_system` — its five neighbouring tests tick nodes; nothing ticked
    the system. It moves each root *out* of the component for the duration of a tick, so a
    node that reaches its own tree sees `None` there, and a node that removes its own
    component loses the tree entirely. Both are documented; both are now checked, the second
    **pinned rather than fixed** — restoring the root would resurrect a component the node
    deliberately removed, which is the worse surprise.

  **The detector lied twice more before it was trustworthy**, and the second time in the class
  it exists to find: a function that is *passed* rather than called. `schedule.add_di_system(
  ui_layout_system.into_config())` mentions the name with no parenthesis after it, so both of
  `gizmo-ui`'s systems came back as "never called" — and an ECS system passed by value is exactly
  the shape the `animation_update_system` bug had. The check is now "is the name mentioned at
  all, outside a comment", not "is it called". Corrected figures: 25 in the physics crates, 17
  across the rest of Stage A, 5 in the small crates, 25 in renderer/editor/app/studio — and most
  of those are builders a *user* calls, which is what a public API is for.

  A third finding came out of the last sweep. `gizmo_math::Ray::intersect_triangle` is public,
  documented at length, mentioned nowhere, and had no test — while `gizmo_physics_core::Raycast::
  ray_triangle` is a second Möller–Trumbore that *is* used. Both now have tests, including a
  cross-check that they agree on hit and distance. Two places they differ deliberately:

  - **The self-intersection band.** Both reject a hit at exactly `t = 0`; between there and
    `1e-8` the math one refuses (a renderer's shadow and bounce rays start ON surfaces) and the
    collider one accepts (a hit a nanometre away is one body touching another). Measured at
    offsets of 0, 1e-8 and 1e-7.
  - **The parallel epsilon** — `1e-6` against `1e-8` — which turns out **not to be reachable**.
    For this shape `|det| = |dir.z|`, so a determinant inside the gap means a ray travelling
    almost entirely in the triangle's plane, and such a ray crosses it millions of units away,
    outside any real triangle. It is rejected on the barycentrics, not on the epsilon. The first
    test written for it asserted a one-way implication that stayed true when the epsilons were
    made equal — measured, and replaced with the conclusion instead of a test that passes for the
    wrong reason.

  Two more came out of running it over everything at once. `gizmo::systems::audio_spatial_system`
  is opt-in by design — `DefaultPlugins` deliberately does not register it — which explains
  the missing caller and not the missing test: its documented precondition is an
  `AudioManager` resource, and nothing checked that a game without audio runs it harmlessly
  instead of panicking. (Its helper `should_autostart` was already covered, latch and all;
  the sweep pointed at the system beside it.) And `Aabb::distance_to_point` was untested
  while `distance_sq_to_point` next to it was not — a one-line wrapper is exactly where a
  squared value gets returned from a function that promises a distance, which reads correct
  at every call site and is wrong at all of them.

  Worth re-running when a crate grows a public surface.
- **A positive `contains` over source text is satisfied by a comment — and four of this
  repository's guards were.** Audited 2026-08-18, after three separate false guards turned up in
  two days by accident: the fly-gate assertion that stayed green with the call wrapped in the very
  gate it forbade, the `missing_docs` ratchet that was per-target, and a written subject list that
  covered less than it looked like. The systematic pass — comment out the guarded line, re-run —
  found four more, all one shape:

  | Guard | With the line commented out |
  |---|---|
  | `render_parity::both_paths_size_the_instance_buffer_for_the_frame` | **green** — both paths carry long comments *about* `ensure_instance_capacity` |
  | `scene_view::the_gizmo_config_still_assigns_snapping` | **green** — the inert field plus the old line as a comment |
  | `egui_ctx::the_descriptor_is_sized_from_the_target_not_the_window` | **green** |
  | `csm::shader_shadow_fade_matches_the_rust_mirror` | **green** — WGSL comments are `//` too |

  Every one of them exists **because a line was once deleted or made inert**, which is the state a
  comment is indistinguishable from. They now cut comments before matching, and each was
  re-broken to watch it go red. Five guards audited the same way held: the draw-loop
  `select_mesh` scan, `the_editor_only_rule_is_written_once`,
  `no_stage_a_crate_depends_on_a_stage_b_crate`, and both `shader_contract` layout checks —
  and `editor_runtime`'s texture check, which survived only because of its **negative** half
  (`!contains("&render_view")`). That is the pattern worth keeping: a positive assertion says a
  string is present somewhere, a negative one says the wrong thing is absent, and the second is
  much harder to satisfy by accident.
- **The audit tool needs the same scepticism as the thing it audits.** The first harness reported
  "green" for a run where the filter matched **no test at all**, and for one where the injected
  mistake did not compile. Both look identical to "the guard did not fire". It now reports those
  separately, which changed two verdicts.
- **A crate-wide lint is only crate-wide on the target you ran it on.** `gizmo-editor` was put on
  the `missing_docs` ratchet after clippy reported zero, and CI went red: the crate has a
  `#[cfg(target_arch = "wasm32")]` arm — an empty `draw_script_section`, since a browser has no
  filesystem to browse — that the native compiler never sees. The lint config is crate-wide, but
  *which code exists* is per-target, so "the crate is clean" was a statement about one target that
  read as a statement about the crate. CI's wasm job lints the same crate with `-D warnings`,
  where a `warn` is a deny. Anything ratcheted must be measured on every target CI lints it on —
  which for the renderer and the eight simulation crates means wasm as well as native. This is the
  same shape as the gate that found it: the wasm job exists because the target was *built* and
  never *linted*.
- **A plan's own status markers rot, and they rot in the direction that costs most.** Retiring
  FIXPLAN found *four* items marked in progress or not started whose work had been finished weeks
  earlier — including one where the code, its tests and its Cargo.toml comment all said so. A
  marker is a claim about the code: verify it against the code before planning around it. "Not
  done" about finished work is worse than no plan, because it is the one kind of error that never
  gets a bug report.
- **An `#[ignore]`d measurement is not a watchman, and "verified against the code" is not the same
  as re-run.** N1/N2 (§7) were the fifth and sixth stale items, and they cost the most so far: two
  physics defects, an accepted non-goal (N≥48) and a queued solver feature (positional friction)
  all rested on numbers that had been false for eleven days. The measurement that would have said
  so existed and was `#[ignore]`d, so nothing ran it — and when the item was moved into this
  document it was checked against the *code shape* (the parameters still exist, the scene still
  builds), which is exactly the check that cannot notice a behaviour change. Two rules follow: an
  open defect gets a **real gate at the exact failing cell**, not an ignored table, and re-running
  the measurement is part of moving an item, not a follow-up. Bisecting the cause afterwards took
  four `git worktree` builds — cheaper than the eleven days, and the only reason the fix can be
  named instead of hoped for.
- **Re-blessing a golden scene is a paragraph, not a number.** Record old → new for every value
  *in the file*, name the cause, and say which direction the sharpest instrument moved. The two
  joint scenes carry two such blocks and they are what makes a later reader able to tell a fix
  from a regression.
- Verify behavior-changing physics fixes with `headless_stress_test` + focused scenarios;
  choose the soak horizon beyond the onset of instability.
- On bug-hunt rounds use subagent fan-out, then verify every finding BY HAND
  (sieve out the false positives).
- **A guard that has never failed is not known to work.** When a change lands a test whose job is
  to catch a class of mistake — a scan, a ratchet, an exhaustive destructure — reintroduce the
  mistake, watch it go red, and put that in the commit message. Two of this repo's mirror tests
  passed for months while checking nothing, and both looked exactly like tests that worked.
- **Inserting an item before a `pub mod` or `pub fn` steals its attributes.** Anchor an insertion on
  the item's own line and it lands *between* the item and the `#[cfg]` / `#[allow]` / doc block
  above it — which now belongs to whatever was inserted. This happened three times on 2026-08-17:
  twice displacing a doc comment (caught immediately by `missing_docs`) and once taking
  `#[cfg(feature = "egui")]` off `pub mod dev_console`, which compiled fine natively and produced
  17 unresolved-`egui` errors in the wasm job — a gate that only exists because the same reasoning
  was applied to the wasm target a day earlier. Then it happened a **fourth** time, in the facade,
  taking `#[cfg(feature = "render")]` off `pub mod asset_server` — and that one reached CI, because
  no local gate I was running compiles a feature *subset*. The gate that catches it is
  `cargo hack check --feature-powerset`, now written into CLAUDE.md's local command list. Walk
  backwards over the attributes *and* the docs before inserting, and run the powerset check after
  touching any module declaration.
- **When the subject is a device, make one.** Gamepad support could be unit-tested down to the
  last edge and still be wrong about the only questions that mattered: does a button arrive as the
  button we named it, is a stick pushed *up* positive when the kernel calls that direction
  negative, does a controller yanked from the port release what it held. `/dev/uinput` answers all
  three — `crates/gizmo-app/tests/virtual_pad.py` creates a virtual Xbox 360 pad and drives a
  scripted sequence into it while the test reads the engine's side, and it found two real defects
  in the first run (§3, gamepad). It is `#[ignore]`d because CI has no `/dev/uinput` ACL, and it is
  worth far more than the mock it replaced would have been: a mock of gilrs would have encoded my
  belief about gilrs, which is exactly what was wrong. The same trick is available for anything
  the kernel can pretend to be — pads, tablets, joysticks, keyboards.
- **Prefer a scanned subject list to a written one.** A test that names the ten files it polices
  cannot see the eleventh, and that is the file the bug will be in. Take subjects from the
  directory, the component modules, the workspace; keep only the *exceptions* by hand, and fail on
  a stale exception too, or the list rots the same way.
- CI: `cargo clippy --all-features --all-targets -- -D warnings -A too_many_arguments
  -A type_complexity` (the two grandfathered architectural lints). The entry crate is
  `gizmo-engine` (NOT `-p gizmo`); `| tail` masks cargo's exit code — check the exit status
  separately.
- **A GPU test must refuse a software adapter** unless it says why not. The Windows runner's
  adapter is WARP; a deferred frame there software-rasterises a 3072² × 4 shadow-map array, and
  the job that first let those tests render was still going at 5.5 hours against ubuntu's six
  minutes. Every CI job carries `timeout-minutes: 45` for that, but a timeout is a report, not a
  fix — the runner still burns the 45 minutes. `every_gpu_test_refuses_a_software_adapter` reads
  the test file and fails on any test that opens an adapter without the
  `headless_adapter_is_software` guard; the one deliberate exception (pipeline compilation, whose
  whole subject is the backend) is named there with its reason, and a stale exception fails too.
  Added after three tests written on 2026-08-15 checked only that an adapter existed.
- **The same gate runs against `wasm32-unknown-unknown`** (2026-08-15). Not redundant: the lint
  config is crate-wide but which code *exists* is per-target, so every
  `#[cfg(not(target_arch = "x86_64"))]` arm and every wasm-only branch sat outside every gate the
  project had. CI built that target and never linted it, and it was holding an undocumented public
  function in a crate that denies `missing_docs`, plus a `return` that is needless once the other
  arm is stripped. A green native lint says nothing about the arms it did not compile.
  - **It caught its first real break the next day** (2026-08-16). Per-object shadow casting added
    `DrawItem::casts_shadows`, whose only reader is `passes::shadow` — and that module is
    `#[cfg(not(target_arch = "wasm32"))]`, because the browser pipeline is forward-only and has no
    shadow pass. So the field is written on every target and read on one, which is `dead_code` on
    wasm and invisible to the native gate. The field stays (it is part of `BatchKey`; dropping it
    on wasm would merge objects that disagree about shadows into one batch), so the fix is a
    targeted `#[cfg_attr(target_arch = "wasm32", allow(dead_code))]` with that reason written
    down. Worth noting *how* it surfaced: nothing in the native workspace was red — the break sat
    at HEAD for three commits, and the wasm gate is the only thing that sees this class of defect.
  - **`gizmo-editor` joined the gate** (2026-08-16), and it had already rotted. The crate declares
    wasm intent — `gizmo-scripting` is a `cfg(not(target_arch = "wasm32"))` dependency, `web-time`
    a wasm one, and items are cfg'd to match — but nothing ever compiled that arm, because
    `demo-web` does not enable the facade's editor feature. It had stopped building for wasm
    outright: a scripting-only inspector function was added without the `cfg` its neighbour
    carries. Beside it, a `let mut initial_dir` that is dead once the native arm is stripped, and
    a `std::thread::spawn` on the wasm path — which on `wasm32-unknown-unknown` does not run the
    closure, it panics, so Save/Load in a browser build would have panicked on click. Three
    defects in one crate, none visible to any native job. The editor is still **not shipped** in
    the browser; it is compiled there so its cfg arms stay honest.

## Nerede kaldık (2026-08-14 → 15)

Bulunup düzeltilen kusurlar aşağıda kendi bölümlerinde duruyor. Bu bölüm yalnız **açık olanı** ve
**bir daha kovalanmaması gerekeni** taşır.

**Kayıtlı kusur listesi boş.** 15 Ağustos sonunda açık kalan hiçbir madde bir hata ya da engel
değil; ikisi bilinçli karar (aşağıda), gerisi sıradan ürün işi (§3 Faz 6/7).

**Bulmayı sağlayan soru — bir sonraki oturum için asıl aktarılan şey bu:** *aynı karar nerede iki
kez veriliyor?* Bugünün on bir bulgusunun tamamı bu sorudan çıktı ve hiçbiri bir hata raporundan
gelmedi. Sıralı olarak bakılan eksenler: iki render yolu (sekiz sürüklenme), üçüncü hedef olarak
wasm (hiç lint edilmiyormuş), scripting (dört kusur), crate grafiği (belge bayat, söz sınanmıyor),
rollback (iki uygulama, biri eksik). Tükendiğinde belirti şu oluyor: aday aramak, bulguları
üretmekten uzun sürüyor.

### İki render yolu — kayıtlı adımların hepsi kapandı

Kök kayıtlı ("The root the sweep could not see"). Beş kesim:

1. `gizmo-renderer::routing` — malzeme tipinin anlamı tek ve tüketici bir `match`'te.
2. `gizmo-renderer::frame_uniforms` (**2026-08-14**) — iki uniform bloğunun kurucusu. Literaller
   tahmin edilenden çoktu: iki çizim döngüsü değil **altı** yer (renderer'ın iki başlangıç tamponu
   ve kendi render callback'i olan üç demo da elle dolduruyordu). Ayrıntısı §7'de; özeti şu:
   sürüklenen üç alandan ikisi ölü (`cascade_params.x`, `scene.exposure` — kimse okumuyor), biri
   canlıydı. Canlı olan **studio'nun DoF'u**: `cam_near`/`cam_far` altı yerin beşinde `0.1`/`2000`
   sabitiydi, yani editör viewport'u tam da bu alanın eklenme sebebi olan kameralarda yanlış
   kalibreydi. Motor yolu (ölü iki alan dışında) bayt-aynı; 12 golden render testi değişmedi.
   Bekçi `no_hand_filled_uniform_literals_outside_the_constructor`: dosya listesi tutmuyor,
   **workspace'i tarıyor** — yani yedinci çağrı yeri yazıldığı gün kırmızı. Kırılabildiği
   doğrulandı (literal geri konup test kırmızıya düşürüldü, sonra geri alındı).

3. **Studio lib hedefi + parite testi (2026-08-15).** `gizmo-studio` artık lib hedefi taşıyor
   (`publish = false` aynen duruyor — amaç yayımlamak değil, editörün render yoluna testten
   erişebilmek) ve `tests/render_parity.rs` iki yolun ilk otomatik çapraz denetimi.
   Doğrulayıcının "ölçekte iyimser" dediği yer doğruydu: `StudioState`/`EditorState`'i headless
   kurmak hâlâ pahalı. O yüzden test **kurulumu** hedefliyor, pass kaydını değil — asıl iş
   `collect_scene_setup`: ışık toplama ve cascade orkestrasyonu zaten ortaktı, ama etraflarındaki
   kırk satır (güneş-var bayrağı, cascade'ın hangi ışığı takip edeceği, ışıksız sahnede identity
   geri düşüşü, point-shadow caster indeksi) iki dosyada ayrı ayrı yazılıydı. İki yol arasındaki
   fark artık **adlandırılmış bir politika**: `ShadowCaster::SunOnly` (oyun) ve `SunOrFirstLight`
   (editör). Test aynı dünya için iki argüman setini yan yana koyup bloğun **dünyanın karar
   verdiği** her alanında eşitlik, yalnız her yolun ilan ettiği üç alanda fark arıyor. Yanına iki
   bekçi: hiçbir render yolu `collect_scene_lights`/`compute_directional_cascades`'i doğrudan
   çağıramaz (çağırırsa parite testi o yolu kapsamıyor demektir), ve her yolun hangi politikayı
   geçtiği kaynakta sabitli — yoksa test kimsenin kullanmadığı iki argüman setini karşılaştırıp
   sonsuza kadar yeşil kalırdı. Motor yolu bayt-aynı (12 golden render testi), determinizm hash'i
   değişmedi. Kırılabildiği doğrulandı: "dünyanın kararı" bir politikaya bağlandı ve studio'ya
   doğrudan bir çağrı kondu; ikisi de kırmızıya düştü.

4. **Yetenek envanteri (2026-08-15).** Kayıttaki "~60 satırlık isim envanteri" testi de yazıldı,
   aynı dosyada. Özneler `gizmo-renderer/src/components`'ten **taranıyor** (yarın eklenen bir
   bileşen aynı gün envanterde), yalnız istisnalar elle — ve **bayat istisna da kırmızı**: iki yol
   da tanımaya başlamışsa kayıt silinmek zorunda, yoksa liste tam da yerine geçtiği çürüyen elle
   sayıma dönüşür. Bugünkü asimetri ölçüldü, tahmin edilmedi: **`Decal` yalnız oyun yolunda**
   (decal geçişi G-buffer'a harmanlıyor, editörün yolu forward — yapısal, ama sonucu gerçek:
   editörde yerleştirilen bir decal oyun koşana kadar görünmüyor), `EditorRenderTarget` ve
   `GameRenderTarget` yalnız editörde (tanımı gereği). Kırılabildiği doğrulandı: yeni bir bileşen
   eklenip tek yola bağlandı, bir de bayat istisna kondu; ikisi de kırmızıya düştü.

5. **Çizim listesindeki iki ortak karar (2026-08-15).** Envanter "hangi yetenek hangi yolda"yı
   ölçüyor; asıl ayrışma ise iki döngünün **aynı soruya ayrı ayrı cevap vermesi**. İkisi bulundu,
   biri gerçekten kopmuştu:
   - **PBR paketleme — canlı sürüklenme.** Anizotropi/clear-coat/subsurface üçlüsü tek bir f32
     yuvasına paketleniyor. Motor alan başına **iki** ondalık hane kullanıyor; studio hâlâ **üç**
     kullanıyordu — yani motorun *terk ettiği* düzen: dokuz hane `f32`'nin tam-tamsayı sınırını
     (2^24) aşıyor ve alt alan komşusuna taşıyor. `gbuffer.wgsl` iki haneyi çözüyor, dolayısıyla
     studio'nun instance'ları **başka bir malzeme** olarak çözülürdü. Bugün etkisiz, çünkü
     editörün forward hattı o shader'a hiç uğramıyor — `cascade_params.x` ile aynı sınıf: gerçek
     sürüklenme, bedeli şimdilik sıfır. Paketleme artık `InstanceRaw::new`'un içinde ve fonksiyon
     **private**; çağrı yerinde yazılamıyor. Yuvanın adı da düzeltildi: `_padding` → 
     `packed_pbr_params` ("padding" diye anılan bir yuva, kimsenin doğru yapmak zorunda olmadığı
     bir yuva gibi okunuyor — iki yolun ayrı paketlemesinin bir sebebi bu).
   - **LOD seçimi.** "Bu entity hangi mesh'i çiziyor" üç durumlu bir cevap (grup entity'nin kendi
     mesh'ini *ezer*; son seviyeyi geçen mesafe **cull** demek, en kaba seviyeyi çizmek değil) ve
     iki yerde satır içi yazılıydı. Uyuşuyorlardı — iyi durum, ama kalıcı değil: `LodGroup` uzun
     süre yalnız editörde onurlandırıldı, motorun kopyası özellikten genç. Artık
     `LodGroup::pick`; mesafenin nereden ölçüldüğü (biri `GlobalTransform`, öteki birleştirilmiş
     model matrisi) her yolun kendi işi olarak kalıyor. Motordaki "studio mesh merkezine ölçüyor"
     yorumu da yanlıştı, silindi.

   Bekçi: iki çizim döngüsü `select_mesh`'i doğrudan çağıramaz ve `packed_pbr_params`'ı elle
   yazamaz. Kırılabildiği doğrulandı.

6. **Çift-yüzlü malzeme — motorda hiç bağlı değilmiş (2026-08-15).** Batch anahtarlarını yan yana
   koyunca çıktı: studio'nun anahtarında `is_double_sided` var, motorunkinde yok — çünkü motor bu
   alanı **hiç okumuyordu**. `Material::with_double_sided` her zaman public'ti, ama yalnız
   editörün forward hattı ona bakıyordu; motorun Z-prepass'i ve G-buffer'ı koşulsuz arka yüz
   kırpıyordu. Yani çift-yüzlü yazılmış bir kumaş/yaprak **editörde iki yüzlü, oyunda tek yüzlü**
   görünüyordu. İncelemenin adını koyduğu sınıf: motorun dışa verdiği bir yetenek, motorun kendi
   varsayılan yolunun okumadığı bir durum (`Sprite`, `LodGroup`, `ParticleEmitter` ile aynı raf).
   Bağlandı: iki yeni boru hattı varyantı (`gbuffer_double_sided`, `z_prepass_double_sided` —
   kırpma modu wgpu'da boru hattına gömülü), batch anahtarına bayrak, üç geçişte seçim. Forward
   yolda zaten kullanılmadan duran `render_double_sided_pipeline` da bağlandı.
   **Kanıt:** yeni golden render testi — kamera bir küpün içinde, yani her görünür üçgen arka yüz;
   tek-yüzlüyle kare boş, çift-yüzlüyle iç yüzey çiziliyor. Düzeltme geri alınınca test "karenin
   %0.0'ı değişti" diyerek kırılıyor.
   **Bağlanmayan, kayda geçen:** saydam çift-yüzlü yüzey iki yolda da tek yüzlü (harmanlı boru
   hattının iki-yüzlü varyantı yok, editörde de yok) ve gölge geçişi ön-yüz kırpmayı sürdürüyor.

   **Bekçinin kendi boşluğu da kapandı.** Dünkü yetenek envanteri bunu göremezdi: özneleri bileşen
   *tipleri*ydi, bu ise `Material`'ın bir *alanı* — ve `Material` iki yolda da her renk okuyan
   satırda geçiyor. Envanter artık `Material` ve `Mesh`'in public alanlarını da özne sayıyor, ve
   alanları **erişim** olarak eşleştiriyor (`.ad`) çünkü çıplak ad `radius`/`color` gibi yerel
   değişkenlerle çakışıyor. Neden yalnız bu iki struct: entity başına yetenek malzemede ya da
   mesh'te oturuyor; `Camera::primary` ya da `PointLight::color` ortak toplayıcılardan okunuyor ve
   yalnızca iki farklı struct'ın aynı alan adını taşıması yüzünden işaretlenirdi — daha da
   genişletmek, tarayıcı sınırını tasarım kararıymış gibi kaydeden istisnalar üretirdi. Ölçüldü:
   bütün bileşenlere açılsa 13 asimetrinin ~4'ü bu türden yanlış pozitif. Kırılabildiği doğrulandı
   (paylaşılan bir alan tek yola indirildi + bir istisnanın yönü ters çevrildi; ikisi de kırmızı).

8. **Instans tamponu: motor kırpıyordu, editör büyütüyordu (2026-08-15).** Gölge-caster
   yerleşimini karşılaştırırken çıktı. Motor instans yüklemesini `instance_capacity`'ye kırpıp
   kaç tanesinin GPU'ya ulaştığını döndürüyor; **`Renderer::ensure_instance_capacity` ise var,
   birim testi de var, ve tek çağıranı studio.** Yani 8 192 instans'ı aşan bir sahne oyunda
   geometri kaybediyor, aynı sahne editörde tam çiziliyordu. `is_double_sided` ile birebir aynı
   sınıf: motorun dışa verdiği bir yetenek, motorun kendi yolunun kullanmadığı.
   İki bölgeli yerleşim (A = bütün batch'lerin kamera instansları, B = bütün gölge-caster'lar)
   bu kırpmanın **zarif bozulması** için yapılmıştı — artık bozulacak bir şey yok, ama tampon
   büyütmeyi reddederse diye bekçi olarak duruyor. Studio'nun batch-başına `[kamera][gölge]`
   yerleşimi motorun terk ettiği düzen ama zararsız, çünkü tam da tamponu büyüttüğü için hiç
   kırpmıyor — kayda geçti.
   **Kanıt:** 9 000 küp, tek batch; GPU'ya ulaşan sayı ölçülüyor. Büyütme çağrısı kaldırılınca
   test "9 000'in 8 192'si ulaştı" diyor — motorun düne kadarki davranışı. Büyüme kasıtlı olarak
   sınırsız ve editörle aynı: 200 000 instans isteyen bir kare 25 MB tampon alır; HashMap'in en
   sona koyduğu mesh'lerin kaybolduğu bir resim yerine.

9. **Harmanlama sırası: motor batch içini sıralamıyordu (2026-08-15).** `gizmo-renderer::draw_order`.
   Saydam boru hattı derinlik yazmıyor, yani harmanlanan geometride **çizim sırası sonucun
   kendisi**. Motor batch'leri sıralıyordu ama her batch'in instanslarını toplandıkları sırayla
   ekliyordu; editör ikisini de sıralıyordu. Aynı malzemeden iki üst üste saydam yüzey — bir
   binadaki pencere sırası, üst üste camlar, birden çok kez instans'lanan herhangi bir saydam prop
   — oyunda **ECS gezinme sırasına** göre harmanlanıyordu, editörde doğru. Bir de ikiz vardı:
   batch'in temsili derinliği motorda `batch_sort_depth`, studio'da `batch_centroid_depth` —
   harfi harfine aynı hesap, iki isim, iki test. İkisi de tek modüle indi.
   **Kanıt:** iki üst üste saydam pane, **farklı renklerde** ama tek batch'te (batch anahtarı
   malzemenin *doku* bind group'u, ikisi de aynı dokudan); sahne bir kez yakın-önce bir kez
   uzak-önce kuruluyor ve iki kare bayt bayt eşit olmak zorunda. Düzeltme kaldırılınca 7 143 bayt
   farklı.
   **Not:** bu testin ilk hâli **boştu** — iki pane aynı renkti, `c over (c over bg)` iki sırada da
   aynı ifade, yani düzeltme kaldırıldığında da geçiyordu. §8'deki "bekçinin kırılabildiğini
   gör" alışkanlığı tam olarak bunu yakaladı; olmasa yeşil ama hiçbir şey ölçmeyen bir test
   commit edilmişti.

10. **Yerleştirilmiş backdrop editörde kameraya yapışıyormuş (2026-08-15).** Studio
    `backdrop::instance_model`'ı hiç çağırmıyordu. Backdrop boru hattının vertex shader'ı kamera
    konumunu ekliyor; `MaterialType::BackdropPlaced` için bu eklemenin CPU'da geri alınması gerek
    ki ikisi sadeleşsin ve backdrop seviyenin koyduğu yerde kalsın. Motor bunu hep yapıyordu,
    studio ham matrisi yüklüyordu — yani seviyeye yerleştirilen bir backdrop **editörde kamerayla
    sürükleniyor, oyunda duruyordu.** Kuralın kendisi zaten `gizmo-renderer::backdrop`'ta yazılı ve
    testli (`a_placed_backdrop_lands_where_it_was_authored`); eksik olan tek şey çağrıydı.
    Bekçi: iki çizim döngüsü de yazdığı matrisi `instance_model`'dan geçirmek zorunda — ve bekçi
    **yorumu saymıyor**, yoksa çağrıyı silip açıklamayı bırakmak testi geçerdi.

11. **Frustum culling — sürüklenme yok, kayda geçsin.** Sıradaki aday buydu; bakınca iki yol da
   zaten `classify_visibility_world`'ü paylaşıyor, AABB'yi bir kez dönüştürüyor ve farklarını
   gerekçesiyle yazmış: editör oyun kamerasının frustum'una göre kırpıyor (edit modunda culling'i
   sınayabilmek için) ve kameraya-kilitli backdrop'u testten muaf tutuyor — çünkü onun shader'ı
   *aktif* kameraya kilitlenirken culling frustum'u *oyun* kamerasınınki, yani testi anlamlı kılan
   bir matris yok. Motor bunun yerine kilitli matrise göre kırpıyor. Bu iyi durum; bir daha
   "bakılacak" listesine girmesin diye yazıldı.

Açık kalan: çizim listesi hâlâ iki ayrı uygulama. Tek tek ortak kararlar (routing, uniform bloğu,
kurulum, LOD, paketleme, çift-yüzlülük) tek kaynağa indi; `collect_draw_items` ile studio'nun
batching'i **tek koda** inmedi ve inmesi de kendi başına bir karar — pass kaydı gerçekten ayrı,
ayıran çizgi "dünyayı okuyan kısım ortak, komut kaydeden kısım ayrı" olarak tutuluyor. Anahtarların
kalan farkı da kayıtta: studio `is_grid`'i anahtarlıyor (motorda grid boru hattı yok), motor
`is_transparent`/`baked_lit`'i anahtarlıyor (studio bunları ayrı HashMap'lerle ayırıyor) — ikisi de
mekanizma farkı, sürüklenme değil.

**İnsan gözü isteyen iki yer:** (1) editör viewport'unda DoF artık kameranın gerçek near/far'ıyla
lineerleşiyor — varsayılan kamerada (0.1/2000) fark yok, farklı far düzlemli sahnede odak doğru
yere kayacak. (2) Editörün sahne bloğunda `cascade_params.w` artık 0 yerine gerçek caster indeksi
taşıyor; editörün forward shader'ı bu yuvayı okumuyor (`point_shadows_enabled` de 0), yani
görünür bir etki beklenmiyor — ama ikisini de test edecek koşum yok, §3'ün A/B kapısına düşer.

### Doğrulanmış ama henüz el atılmamış kökler

(Bu listedeki son madde de kapandı — aşağıya bakın.)

Kapatılanlar (**2026-08-14**), ayrıntısı §7'de:

- **Sözleşmeler** → `gizmo-renderer::shader_contract`. Kendi `SceneUniforms`'unu bildiren yedi
  shader'ın hepsi bloğun bir **ön eki**; meşru, ama ön eki kimse denetlemiyordu. Yeni testler
  öznelerini shader dizininden, cevaplarını **naga**'dan alıyor: her adlandırılmış alanın bayt
  ofseti Rust'taki `offset_of!` ile karşılaştırılıyor. Bugün sürüklenme yok — sekiz bildirim de
  uyuşuyor. Elle özne sayan eski ayna testi silindi (kapsamı yenisinde), yanlış iddia eden
  yorumu düzeltildi. Kırılabildiği doğrulandı: bir kopyada iki alan yer değiştirdi, bir başkasında
  kuyruk alanı silindi; ikisi de dosya/alan/ofset vererek kırmızıya düştü.
- **Shader boru hattı** → `compose_module` artık doğrulanmış modülü döndürüyor, `compose_wgsl` onun
  üstünde ince bir metin sarmalayıcı. Refaktör kendisi için değil: derlenen shader'lar bind-group
  indekslerini satır içi yerleştiriyor (`@group(#{INSTANCE_GROUP})`), yani onları yalnız naga
  okuyamıyor — sözleşme testinin gerçek kompozisyon yoluna ihtiyacı vardı.
- **Crate grafiği / Stage A** → ölçüldü. `gizmo-animation` bağımlılık olarak zaten temiz
  (`gizmo-core`/`math`/`physics-core`, `default = []`, `wgpu` yok) — yani Stage A ölçütünü
  karşılıyor; bayat olan **grafik belgesiydi**. Gerçek grafik `cargo metadata`'dan alındı ve
  CLAUDE.md'deki diyagram üç yerde yanlıştı: `gizmo-ui` aslında `gizmo-app`'in **üstünde** (ona
  bağlı), `gizmo-window`'un hiçbir workspace bağımlılığı yok, ve `gizmo-animation` renderer ile
  scripting'in **altında** — yanlarındaki bir yaprak değil. Diyagram düzeltildi.
  Asıl kazanç ölçüm değil, artık sınanan değişmez: `crates/gizmo/tests/crate_staging.rs`
  manifestlerden grafiği okuyup **hiçbir Stage A crate'inin Stage B'ye bağlanmadığını** doğruluyor
  (bağlansaydı o crate, listelendiği sürümü hızlı katmanın her kırıcı değişikliğiyle birlikte
  yapmak zorunda kalırdı), her crate'in tam bir aşamada sınıflandırıldığını (yeni crate = karar
  anı) ve `gizmo-core`/`gizmo-math`'in taban olarak kaldığını. Kırılabildiği doğrulandı
  (`gizmo-audio → gizmo-scripting` kenarı eklendi, kırmızıya düştü).
- **Determinizm çevresi** → `snapshot()`/`restore_snapshot()` artık `..` içermeyen **tam yıkım**
  (destructure) yapıyor: `PhysicsWorld`'e (ya da `WorldSnapshot`'a) eklenen bir alan orada derleme
  hatası. `_` ile bağlanan 19 alanın her biri gerekçesiyle yazılı — yapılandırma, türetilmiş,
  çıktı, yapı, kontrol bayrakları. Davranış aynı: aynı dokuz alan taşınıyor,
  `headless_stress_test` üç eşleşen hash veriyor. Bekçi doğrulandı (alan eklendi, `E0027` desenin
  üstüne düştü). Ayrıntısı §5'te.

### Scripting

Bu bölüm bir zamanlar "32 doğrulanmış kusurdan yedisi düzeltildi" diye başlıyordu. **Kalan 25'in
arkasında yazılı tek bir madde yok** — bunu bir sonraki okuyanın keşfetmesine bırakmamak için burada
söylüyor: 32 sayısı, madde listesi saklanmamış bir taramadan geliyor. Aşağıdaki yedi maddenin
yanındaki not ("kayıtta adı geçen bütün açık maddeler") o taramanın yazıya geçmiş kısmının tükendiğini
söylüyor. Dolayısıyla "25 kalan kusur" bir iş kalemi değil; ne kovalanabilir ne kapatılabilir.
Scripting'de bir sonraki adım, sayıyı azaltmak değil **taramayı yeniden koşturup bu kez maddeleri
yazmak**. Sayı bu yüzden buradan kaldırıldı: arkasında iş olmayan bir sayaç, ilerleme ölçüsü gibi
görünüp değil.

**Tarama yeniden koşturuldu ve ilk maddesi yazıldı (2026-08-18): Lua'nın ses API'si uçtan uca
çalışıyordu, ucu hariç.** `audio.play("jump")` bir `ScriptCommand::PlaySound` kuyruğa atıyor;
`ScriptEngine::flush_commands` bunu uygulayamıyor — scripting crate'i ses altsistemine bağlı değil —
ve komutu **çağırana geri veriyor**. `PlayLoop`'taki iki çağrı yerinin ikisi de o dönüş değerini
`let _unhandled = …` ile atıyordu, ve workspace'te başka tüketici yoktu. Yani üç çağrılık ses API'si
(play / play_3d / stop) editörün Play modunda da, ihraç edilmiş her oyunda da **hiçbir ses
çıkarmıyordu** — üstelik `api_audio.rs`'te komutun kuyruğa atıldığını doğrulayan bir birim testiyle
birlikte. Kuyruğu ölçen bir test, etkisi olmayan bir API'yi onaylar.

Bulma yöntemi kaydedilmeye değer: kusur okuyarak değil, **bir dönüş değerini takip ederek** çıktı —
`flush_commands`'in `Vec<ScriptCommand>` döndürdüğünü görüp "bunu kim alıyor?" diye sormak yetti.
Cevap: kimse. Aynı soru §8'in "üretimde çağrısı olmayan public fn" taramasının bir üst katmanı.

Ses tarafı artık `PlayLoop`'ta karşılanıyor (`apply_script_audio`): `play`, `play_3d` (spatial
sistemin kullandığı **aynı** dinleyiciyle — yoksa ses ikinci karesinde zıplar) ve `stop`.
`stop` için `AudioManager::stop_by_name` eklendi: sink id motorun elindeki şey, ama bir script'in
elinde yalnız **isim** var. Sahne/diyalog/yarış/kamera komutları bilerek geri verilmeye devam
ediyor — editör yazarın altından sahne değiştiremez.

Uçtan uca test gerçek donanımda (`demo/tests/the_runtime_runs_scripts.rs`), ve negatif kontrolü
koşturuldu: `apply_script_audio` çağrısı kaldırılınca test kırmızıya düşüyor (0 ses, 1 beklenirken).
Cihazsız yarısı ayrı: komutların **tanınması** — asıl kırık olan yarı — `split_audio_actions` ile
birim testinde.

**Ve aynı soru bütün komut vokabülerine soruldu — LİSTE BU (2026-08-18).** `ScriptCommand`'ın
**43 varyantı** var; `flush_commands` bunların **23'ünü** kendi içinde uyguluyor, **20'si** geri
dönüyordu ve hiçbirine bakan yoktu. `PlayLoop` şimdi yedisini karşılıyor. *(Sayılar 2026-08-19'da
düzeltildi: `SetFighterHealth` aynı gün eklendi ve iki yerde yazılı sayıyı bayatlattı — §8'in
"düzyazıya yazılan sayı, kodun geride bıraktığı sayıdır" kuralının bu oturumdaki örneği. Ölçüm:
enum gövdesindeki varyant adları sayıldı, 43.)*

| komut | durum |
|---|---|
| `PlaySound` · `PlaySound3D` · `StopSound` | **karşılandı** — yukarıdaki madde |
| `SetVehicleEngineForce` · `SetVehicleSteering` · `SetVehicleBrake` | **karşılandı** — Lua'nın araç API'si de sessizdi; `VehicleController` `gizmo-physics-dynamics`'te, yani scripting crate'inin ulaşamayacağı yerde |
| `SetCameraFov` | **karşılandı** |
| `LoadScene` · `SaveScene` | açık, **bilerek**: editör yazarın altından sahne değiştiremez. Çalışma zamanı için meşru; tetikleyicisi sahne geçişi isteyen bir oyun |
| `SetCameraTarget` · `SetFightCamera` | açık: bunlar bir **değer** değil, zaman içinde bir **davranış** istiyor ve motor script'in gösterebileceği bir takip sistemi göndermiyor. Tetikleyici: öyle bir sistem |
| `ShowDialogue` · `HideDialogue` | açık: diyalog altsistemi yok |
| `TriggerCutscene` · `EndCutscene` | açık: ara sahne altsistemi yok |
| `StartRace` · `FinishRace` · `ResetRace` · `AddCheckpoint` · `ActivateCheckpoint` | açık: yarış altsistemi yok |

**Araç tarafında iki BİRİM tuzağı vardı ve ikisi de canlıydı** — düz atama ikisini de sessizce
bozardı:

- `SetVehicleEngineForce`'un belgesi "negatif değer geri sürer" diyor;
  `VehicleController::throttle_input`'un belgesi tam tersini söylüyor: *"yalnız büyüklüğü kullanılır,
  negatif değer geri vites DEĞİLDİR — onun için `set_reverse`"*. Alanı doğrudan yazsaydık
  `vehicle.set_engine_force(id, -1)` arabayı **tam gazla ileri** sürerdi, ki bu "geri"nin
  olabilecek en kötü okuması. Eşleme geri vitesi takıyor (idempotent, her kare çağrılabilir).
- `SetCameraFov`'un belgesi **derece**, `Camera::fov`'unki **radyan**. 60 isteyen bir script
  60 radyan alırdı ve `Camera::new` yalnız alttan kelepçeliyor, yani hiçbir şey itiraz etmezdi.
  Bu, bölümün zaten kayıtlı tuş-haritası kusuruyla aynı şekil: iki gerçek birim, aynı birim değil,
  ve ikisini karşılaştıran hiçbir şey yok.

Üçünün de testi var ve üçü de cihazsız — ses komutlarının aksine bunlar saf ECS yazımı.

**Yapısal olarak açık kalan bir şey daha.** `PlayLoop::step` karşılayamadığı komutları artık
yutmuyor ama **dışarı da vermiyor**: gömen bir oyunun kendi diyalog/yarış katmanını Rust'ta yazıp
kalanları alması mümkün değil. Yani yukarıdaki on üç madde "motor göndermiyor" olmaktan çıkıp
"kimse gönderemez"e dönüyor. Tetikleyici: bu altsistemlerden birini kendi yazmak isteyen bir oyun;
çaresi muhtemelen `PlayReport`'a bir varyant, çünkü raporlama zaten enjekte edilen kanal.

**Aynı soru öteki dönüş değerlerine soruldu ve üç ÖLÜ KOL çıktı (2026-08-18).** Bu sefer araç
tahmin değil, deponun kendi taraması: `crates/gizmo/tests/unmentioned_api.rs` yeniden koşuldu
(**1406 public fn'in 109'u üretimde anılmıyor**) ve scripting crate'inin sırası okundu. Üçü de
"kimse çağırmıyor" değil — üçü de **çağrılsa bile doğru cevabı veremeyecek** kod:

- **`get_pending_audio_scene_commands` her zaman boş `Vec` döndürüyordu.** Gövdesi tek satır
  `Vec::new()`, üstünde ne zaman çağrılması gerektiğini soran iki yorum; belgesi ise "çalışma
  zamanında bekleyen ses/sahne komutlarını döndürür" diyordu. Yani bir önceki maddede düzeltilen
  kusurun **taslağı**: ses komutlarını "demo tarafında" bekleyen kanal, hiç bağlanmamış. Gerçek
  kanal `flush_commands`'in dönüş değeri, ve onun artık tüketicisi var. Silindi.
- **`run_entity_update` + `ScriptContext` + `ScriptResult`: ikinci bir varlık-başı protokol.**
  Lua'ya bir `ctx` tablosu (pozisyon, hız ve **dokuz sabit tuş bayrağı**) veriyor, geri dönen
  `{position, velocity}` tablosunu çıkarıyor ve uygulamayı **çağırana** bırakıyordu. Canlı protokol
  bu değil: `update_entity` → `on_entity_update(entity_id, dt, props)`, etkiler komut kuyruğundan.
  Son çağıranı `0de4bee`'de (silinen gizmo-fight tümleştirmesi) gitmiş; o günden beri motorda
  uygulayanı olmayan bir sonuç türü duruyordu. İki protokolün maliyeti soyut değil: bir sonraki
  bağlama ölü olanı seçebilir, ve sabit tuş listesi `input` tablosunun sınırlı bir kopyası — üstelik
  onu dolduracak olan da çağıranın kendisi. Silindi; `ScriptContext`'in belgesi zaten "geriye dönük
  uyumluluk için tutuluyor" diyordu, ki 0.x'te ve dışarıda kullanıcısı olmayan bir tür için bu
  cümlenin kendisi ipucu.
- **Crate'in bütün `cfg(target_arch = "wasm32")` kolu — hiçbir yapılandırmada derlenemez.**
  `dummy_engine.rs` (104 satır), wasm için `register_script_components` no-op'u ve `Dummy*` takma
  adları. Ölçüm: `cargo check -p gizmo-scripting --target wasm32-unknown-unknown` mlua-sys'in build
  script'inde ölüyor — *"don't know how to build Lua for wasm32-unknown-unknown"*. Yani o kolun var
  olma sebebi olan hedef, crate'in **hiç kurulamadığı** hedef. Tüketiciler bunu zaten biliyor:
  `gizmo-app` ve `gizmo-editor` bu crate'i `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`
  altında listeliyor, ve `cargo tree -p demo-web --target wasm32-unknown-unknown` içinde
  `gizmo-scripting` **sıfır** kez geçiyor. Hiç derlenmediğinin kanıtı sürüklenmenin kendisi:
  stub'ın `update_entity`'si **üç** argüman alıyordu, gerçeği dört — üstelik hemen üstünde
  "imzaları aynı tutmak çağıran kodun iki hedefte de değişmeden derlenmesini sağlar" diyen bir
  yorumla. Gerçekten yük taşıyan desen crate düzeyinde değil, **tüketicide ve öğe başına**:
  `gizmo-editor`'ün `draw_script_section`'ı iki gövdeli ve script denetçisi cfg'li — ve o kol
  wasm'da **lint'leniyor**, farkı yapan da bu.
  **Tetikleyici** (bir stub'ı geri getirmek için): tarayıcı yapısında `Script` bileşenlerinin sahne
  gidiş-dönüşünde hayatta kalmasını isteyen bir oyun, ya da wasm32'de derlenen bir Lua backend'i.
  İlk adım her iki durumda da manifestte `mlua`'yı hedefe kapılamak — yoksa stub, crate'in
  kurulamadığı bir hedef için yazılmış olmayı sürdürür.

Yöntem notu, çünkü aynı tuzağa yine düşüldü: wasm ölçümü ilk kez `cargo check … | tail -30` ile
alındı ve **kabuk 0 döndürdü** — CLAUDE.md'nin "boruya sokmak çıkış kodunu maskeler" uyarısının
tam olarak kendisi. Build'in çöktüğü ancak çıktı okunduğunda görüldü. Boruya sokulan bir cargo
komutunun yeşil görünmesi bir kanıt değildir.

Bir önceki oturumun bıraktığı **iki iz de kapandı** — biri kod, biri ölçüm ve karar olarak:

- **Dövüş altsisteminin saati yoktu — 2026-08-18'de yazıldı.** Bırakılan iz "sözleşmenin *ya da
  bir script* yarısı Lua'dan ulaşılamıyor" diyordu ve iki yol öneriyordu: sayaçları sayı olarak
  Lua'ya açıp ilerletme yazımını eklemek, ya da belgeden "(ya da bir script)"i silip tick'i Rust
  host'un işi ilan etmek. **Ölçüm ikisini de geçersiz kıldı: tick'i yapan host da yoktu.** Depoyu
  taradığımızda `current_move_frame`, `hitstop_frames` ve `hitstun_frames`'e yapılan HER yazım bir
  atamaydı — bütün ağaçta tek bir `+=`, `-=` ya da `saturating_*` yok. Yani seçenek "kim sayacak"
  değildi; **kimse saymıyordu.**

  Somut sonuç, artık regresyon testi olarak sabit: `fighter.apply_hitstop(id, 3)` çağıran bir
  script dövüşçüsünü üç kare değil **sonsuza kadar** donduruyordu (test kaldırıldığında ölçülen
  dizi `[1,2,3,4,5]`), ve `fighter.set_move` ile başlatılan hareket 0. karesinde kalıyordu, yani
  `is_in_active_window` — "bu saldırı şu an vuruyor" diyen fonksiyon — bu motorda **hiç `true`
  dönmemişti** (zaten tek bir çağıranı da yoktu).

  Tarih önemli, çünkü bu bir eksik özellik değil bir **kayıp**: saat `334d6ed`'de yazılmıştı
  (`physics_fighter_system`, 415 satır, kendi testiyle), iki gün sonra `592bd6f`'in ayrıştırma
  refactor'ünde silindi, yorum satırına düşmüş çağrısı da `9cbdddf`'de süpürüldü. Geriye
  altsistemin bütün öteki yarıları kaldı: stüdyo bileşeni ekliyor ve ondan bir dövüş HUD'ı
  çiziyor, denetçi düzenliyor, sahne biçimi serileştiriyor, üç Lua çağrısı yazıyor. §7'nin
  "oyunun kendi zamanlaması gereken bir sistem *bağlanmamış* sayılmaz" istisnası bunu kapsamıyor;
  kapsayan, hemen altındaki dar sınıf: **motorun kendi varsayılan yolu, motorun kendi döngüsünün
  hiç üretmediği bir duruma dayanıyordu** — ve o yol `PlayLoop`, yani hem editörün ▶'si hem
  ihraç edilen her oyun.

  Yazılan: `FighterController::tick` (bir sabit kare), `FrameData::total_frames` (hareketin tek
  uzunluk tanımı), ve `fighter_frame_system` (ince ECS sürücüsü) — `GameplayPhysicsPlugin`
  kaydediyor, `PlayLoop::physics_pass` sabit adımda çağırıyor. Kilit **harcanmadan önce
  okunuyor**, yani `apply_hitstop(n)` tam `n` kare donduruyor; hitstop hareketi donduruyor
  (hitstun zaten iptal ediyor); hareket `total_frames()` karede bitiyor, recovery dahil.

  **Bilerek yapılmayan üç şey**, çünkü üçü de motorun tahmin edeceği oyun politikası:
  `input_buffer`'ı beslemek (hangi eylem adlarının kaydedileceği oyunun), bir tuştan hareket
  seçmek, ve `Hitbox::active`'i aktif pencereden sürmek. Silinen 415 satırın motor-saati olmayan
  kısmı tam olarak bu üçüydü; stüdyodaki varsayılan `ActionMap` iskelesi de onların fosili.

- **Kare enterpolasyonu: `alpha` her kare hesaplanıyor, kimse okumuyor — ÖLÇÜLDÜ, karar (b).**
  Buradaki ilk şüphe (`windowed/event.rs`'te `let _ = steps;`) bakınca erimişti — `FrameSteps`
  bir **kopya**, asıl değer `PhysicsTime` kaynağında. Bir kat aşağıdaki soru gerçekti:
  `compute_alpha()` her karede çağrılıyor, `alpha()` belgesinde `lerp(prev, curr, alpha)` diyor,
  ve workspace'te onu okuyan **hiçbir render yolu yok**.

  Tarama üç şey ekledi: (1) `alpha()`'nın iki tüketicisi var, ikisi de render değil —
  `frame.rs:126`'daki `tracing::debug!` alanı ve `FrameSteps.alpha`, onun da tek üretim okuyucusu
  `let _ = steps;`. (2) **İKİNCİ bir alpha var**: `PhysicsWorld::render_alpha`
  (`world/step.rs:172`), kendi **240 Hz** alt-adım akümülatörüne göre, belgesi zaten *"Output
  only"* diyor. (3) **Eksik olan katsayı değil, saklama**: ağaçta sabit adımlar arası
  enterpolasyon için tutulan hiçbir önceki-`Transform` yok. (`TaaState::prev_vp` /
  `SsgiState::prev_vp` var ama onlar KAMERANIN önceki view-projection'ı, temporal reprojection
  için; `gizmo-renderer`'ın `animation_system`'i ise gerçekten `blend_poses(prev, cur, alpha)`
  yapıyor — yani "önceki durum + alpha" deseni motorda zaten var, sadece `Transform` için yok.)

  **Titreme ölçüldü** ve `time.rs`'in testlerine sabitlendi (`hold_lengths`): 60 Hz fizik karşısında
  bir renderer'ın aynı pozu üst üste kaç kare çizdiği —

  | render | fizik | poz başına kare | alpha |
  |---|---|---|---|
  | 60 Hz | 60 Hz | hep 1 | hep 0.0 |
  | 144 Hz | 60 Hz | **2 ya da 3**, düzensiz: 3,2,3,2,3,2,2… (240 karede 59×2, 40×3) | 0.08 – 1.0 |
  | 165 Hz | 60 Hz | 3,3,3,3,2… | 0.09 – 1.0 |
  | 300 Hz | 60 Hz | hep 5 | 0.0 – 0.8 |

  Yani kusur ortak durumda **görünmez** (60/60'ta alpha hiç 0'dan ayrılmıyor) ve tam katta da
  görünmez (300 Hz'de tutuş düzgün — slayt gösterisi ama pürüzsüz). Görünür olduğu yer vuruşun
  kendisi: 144 Hz'de sabit hızla giden bir cismin kare başına yer değiştirmesi komşu kareler
  arasında **%50 salınıyor**. İkinci sayı: `alpha * fixed_dt` çizilen pozun bayatlığı — 60 Hz'de
  16.7 ms'ye kadar, 10 m/s giden bir şey için **16.7 cm**, saniyede altmış kez testere dişi.

  **Karar: (b) şimdi, (a) tetikleyicisiyle ertelendi.** Yapılan: `alpha()`'nın, `frame.rs`'in ve
  `render_alpha`'nın belgeleri dürüst hale getirildi (özellikle `frame.rs`'in *"son iki simüle
  edilmiş durum arasında"* cümlesi — motor **bir** tane tutuyor), iki alpha birbirine
  çapraz-referanslandı (hangisi hangi gömme seviyesinde doğru), ve maliyet yukarıdaki iki testle
  depoya sabitlendi. Silinen bir şey yok: `PhysicsTime::alpha` sabit-adım sözleşmesinin standart
  yarısı ve gömen oyun onu kullanabilir; `render_alpha` da `PhysicsWorld::step`'i doğrudan
  değişken dt ile süren (ECS'siz) gömücü için doğru olan katsayı. İkisi kopya değil, iki seviye.

  **(a) neden yazılmadı ve tetikleyicisi ne.** Motor tarafı enterpolasyon tasarım boyutunda bir iş,
  üç yerde birden karar istiyor: (i) neyin saklanacağı — `Transform`'u enterpole etmek onu
  *oyunun okuduğu* değer yapar, yani ayrı bir render-only poz gerekiyor; (ii) hiyerarşi —
  `GlobalTransform` yerelden türetiliyor, dolayısıyla enterpole edilmiş yereller için ikinci bir
  propagate geçişi gerekir; (iii) rollback/net anlık görüntüleri ve determinizm kâhini — enterpole
  edilen değer simülasyona geri sızarsa hash değişir. Artı çizilen varlık başına kare başına bir
  lerp. Tetikleyici: 60 Hz'in katı olmayan bir ekranda (144/165 Hz) titremenin şikâyet konusu
  olması, ya da motorun varsayılan `PresentMode::AutoNoVsync`'inin vsync'e çekilmesi kararı —
  ikisinden biri olduğunda (i)-(iii) sırayla yanıtlanıp opt-in bir özellik olarak yazılır.

Saat yazılırken aynı taramanın çıkardığı, **ölçülmüş ama düzeltilmemiş** yedi şey — hepsi dövüş
altsisteminin geri kalanı, ve hiçbiri saatin kapsamında değil:

1. **Bir düzeltme, bir önceki oturumun cümlesine.** "`check_combo` bu motorda hiç dolu bir tampon
   göremez" doğru değil: `fighter._buffers` bir Lua tablosu, ve salt-okunur API vekili yalnız DIŞ
   tablonun `__newindex`'ini tutuyor — `fighter._buffers` okuması `__index` üzerinden **gerçek iç
   tabloyu** döndürüyor, dolayısıyla `fighter._buffers[1] = {...}` sıradan bir tablo yazımı
   (`api_table.rs`'in kendi belgesi bu boşluğu zaten söylüyor: *"a table the engine hands out by
   value … is unaffected"*), ve crate'in beş kombo testi tam olarak bunu yapıyor. Doğru cümle:
   **motor tamponu hiç doldurmuyor**, script kendisi doldurabiliyor.
2. ~~`SetFighterMove` hitstun ve hitstop karelerini taşıyamıyor~~ **— düzeltildi.** İkisi de
   `fighter.set_move`'a **isteğe bağlı** son iki argüman oldu; verilmezse `FrameData::default()`'ın
   20/5'i, yani daha önce yazılmış her çağrı aynen davranıyor. Öncesinde Lua'dan başlatılan her
   hareket o 20/5'i sessizce miras alıyordu ve script ne okuyabiliyor ne değiştirebiliyordu.
3. ~~Lua'ya hiçbir dövüşçü sayısı okunmuyor~~ **— düzeltildi.** `update_fighter_read_api` artık
   bileşenin tamamını aynalıyor: `fighter.state(id)` → `health`, `max_health`, `player_id`,
   `blocking`, `crouching`, `hitstop`, `hitstun`, `locked`, ve nötr değilse
   `move = { name, frame, total, startup, active, recovery, attacking, damage, hitstun_on_hit,
   hitstop_on_hit }`. Yardımcılar: `fighter.is_locked`, `fighter.is_attacking`,
   `fighter.move_frame`. Şekil `entity._positions` deseninin aynısı. Tek bitlik `_is_locked`
   tablosu silindi — aynı olgunun ikinci kaynağıydı. **Aynanın bir kare geriden geldiği bilinerek
   bırakıldı**: script geçişi kareyi başında aynalıyor, saat ise sonunda harcanıyor, ki kendi
   hareketine tepki veren bir script'in bakması gereken değer zaten o.
4. ~~Lua aynası `just_released`'ı düşürüyor~~ **— düzeltildi.** Tampon aynası artık üç kümeyi de
   taşıyor, yani şarjlı/negatif-kenar hareketler script'e görünür.
5. ~~Lua'nın `check_combo`'su ile Rust'ın `check_combo_strict`'i farklı algoritmalar~~
   **— düzeltildi, ve ayrışan taraf Rust'tı.** Fark tek bir şeydeydi: Rust `just_pressed VEYA
   pressed` eşliyordu. Bu daha hoşgörülü bir okuma değil, **sıra denetiminin sonu**: üç tuşu
   birlikte TUTAN bir oyuncuda üçü de her karede `pressed` içinde, dolayısıyla herhangi bir
   sıralaması üç ardışık karede tamamlanıyor — çeyrek daire de, tersi de. `max_gap` de anlamını
   yitiriyor, çünkü tutulan tuş her karede eşleşiyor. Fonksiyonun kendi yorumu zaten "en güvenlisi
   just_pressed" diyordu, kod öteki şeyi yapıyordu. Artık ikisi de yalnız basma kenarını eşliyor.
   Gerçek hareketler etkilenmiyor (d-pad çeyrek daire testi gerçek bir `ActionMap` sürüyor ve
   dokunulmadan geçiyor); kaybedilen tek şey basma kenarı pencerenin dışında kalmış bir adım, yani
   tampon başlamadan önce basılı olan bir tuş.

   Ayrışmanın görünmez olmasının sebebi de kayda geçti: workspace'teki her kombo testi ikisinden
   BİRİNİ sürüyordu. Yeni bir çapraz test (`gizmo-scripting`) yedi senaryoyu **ikisine birden**
   soruyor ve aynı cevabı istiyor; eski Rust semantiği geri konduğunda "üçünü birlikte tutmak"
   senaryosunda Rust true, Lua false diyerek kırılıyor.
6. ~~Hasar hiç uygulanmıyor~~ **— vuruş algılama yazıldı, KARAR: motor bildirir, oyun harcar.**
   Dört tasarım sorusu kullanıcıya soruldu ve *"olay yayınla, hasarı oyun uygulasın"* seçildi.
   Yazılan: `hit_detection_system` (gizmo-physics-dynamics), her sabit adımda saatten SONRA
   koşuyor, ve bağladığı her çakışmayı bir `HitEvent` olarak bildiriyor. Motor `health`'e
   dokunmuyor; ölüm, zırh, savuşturma, takım ateşi oyunun kuralları olarak kalıyor.

   Dört sorunun yanıtları, hepsi testli:
   - **Hangi hitbox hangi hareketin:** `Hitbox::move_name: Option<String>` eklendi. `None` (ve
     varsayılan) "her hareket" demek — tek hitbox'lı bir dövüşçü için doğru olan. İki kutu olduğu
     anda etiket gerekiyor, ve `592bd6f`'de silinen sürümün kusuru tam buydu: dövüşçünün ALT
     AĞACINDAKİ her kutuyu sürüyordu.
   - **Hareket başına tek vuruş:** `FighterController::already_hit`, `start_move` ve hareketin
     bitişi temizliyor. Olmadığında üç karelik pencere aynı vuruşu üç kez bildiriyor (negatif
     kontrolde ölçüldü: `[(2,1)]` yerine `[(2,1),(3,1)]`).
   - **Kim kimi vurabilir:** sahiplik `Parent` zinciri yukarı en yakın `FighterController`;
     saldıranın kendi alt ağacındaki hurtbox'lar hedef değil.
   - **Etki:** olay. `HitEvent { attacker, attacker_hitbox, victim, victim_hurtbox, damage,
     hitstun, hitstop, move_name }`; `damage` = hareketin `FrameData::damage`'ı × bölgenin
     `Hurtbox::damage_multiplier`'ı.

   Geometri `NarrowPhase::box_box_overlap` — `box_box`'ın on beş eksenli SAT süpürmesinin boolean
   kardeşi, aynı eksen kurulumunu ve `sat_penetration`'ı PAYLAŞIYOR (iki kopya sürüklenirdi), ilk
   ayıran eksende duruyor ve manifold kurmuyor. 120 yapılandırmada `box_box` ile aynı cevabı
   verdiği test edildi. Kutuların dünya pozu yerel `Transform` zinciri kompoze edilerek bulunuyor,
   yani propagate geçişine bağlı değil; ölçek yok sayılıyor (motorun kendi hata-ayıklama gizmo'su
   da öyle yapıyor — çizilen kutu ile sınanan kutu aynı olsun diye).

   **Tüketici tarafı da kapandı (ayrı commit).** Üç eksik vardı ve üçü de "olay var, okuyanı yok"
   biçimindeydi: (a) `Events<T>` çift tamponlu ve `update()` çağıran yoktu — `App::add_event`
   pencereli çalışma zamanının pompası, editörün ▶'si de ihraç edilen oyun da oradan geçmiyor,
   yani `physics_step_system`'in gönderdiği çarpışma/tetik olayları da yıllardır dönmeyen bir
   kuyruğa gidiyormuş. `PlayLoop` artık karesinin sonunda pompalıyor; `HitEvent` kuyruğunu yoksa
   YARATIYOR da (`run_fixed_and_update`'in `PhysicsTime`'ı yaratması gibi), ötekileri yalnız oyun
   istemişse döndürüyor — asimetri ölçülmüş: `physics_step_system` ürettiği her çarpışma olayını o
   kuyruğa klonluyor, 200 kutuluk kulede bu kare başına binlerce temas listesi eder ve kimse
   istemediği bir kaynak için bunu ödememeli. (b) Lua'nın olayları göreceği yüzey yoktu:
   `fighter.hits()` eklendi, aynası öteki okuma API'leriyle aynı şekilde bir kare geriden geliyor.
   (c) Script'in hasarı harcayacak yazma yolu yoktu: `fighter.set_health(id, değer)` — atama,
   çünkü tavan, taban, zırh, bloğun yarıya indirmesi ve ölüm oyunun kararları.

   Uçtan uca test zincirin tamamını sürüyor (`demo/tests/the_runtime_runs_scripts.rs`): script bir
   jab atıyor, motor vuruşu çözüp bildiriyor, script `fighter.hits()`'ten okuyup canı düşürüyor →
   **92**, 100 değil (olay ulaştı) ve 84 değil (hareket başına tek vuruş). Negatif kontrol: pompa
   kaldırılınca 100.
7. ~~Stüdyo tarafında dört kusur~~ **— düzeltildi (aynı gün, ayrı commit).** Saati yazdıktan
   sonra stüdyoda görülecek yüzeyi yoktu: `scene_ops` `FighterController`'ı ekleyebiliyor ama
   **silemiyordu** (denetçide dört sil düğmesi var, üçünün kolu vardı), iki dövüşçü de varsayılan
   `player_id = 1` ile geliyordu (HUD `p1` VE `p2` isteyince hiç açılmıyordu), ⏸ duraklatma HUD'ı
   sıfırlıyordu (`is_playing()` yalnız Play; doğrusu `is_in_play_session()`, sayaç ise yalnız
   Play'de akmalı), ve denetçi `active_move` / `current_move_frame` / `hitstop_frames` /
   `hitstun_frames`'in hiçbirini göstermiyordu — üstelik kendi belgesi "mevcut hareketin frame
   data'sı" diyordu. Dördü de testli: dört sil düğmesi **birlikte** sınanıyor (kusur bir düğme
   listesiyle bir kol listesi arasındaki asimetriydi, tek tek test tam bunu kaçırır), slot dağıtımı
   silinen slotu geri veriyor, HUD ▶/⏸/⏹ üçlüsünde sürülüyor, ve denetçi başsız bir egui
   karesinde boyadığı metinden okunuyor.

**Bir sonraki oturuma, bu oturumda ölçülüp düzeltilmeyenler** (dövüş zinciri kapandı, bunlar
komşu köşeler):

- ~~`PlayAnimation` ve `SetAnimationSpeed`'in üreticisi yok~~ **— bağlandı.** İkisinin de
  `flush_commands`'te çalışan işleyicisi vardı ve hiçbir `api_*.rs` onları itmiyordu: motor istek
  üzerine animasyon oynatabiliyordu ama istenmesinin yolu yoktu. Yeni `api_animation.rs`:
  `animation.play(id, ad, blend?, loop?)` (varsayılan 0.2 s çapraz geçiş + döngü, çünkü
  script'in `animation.play(id, "run")` demesi sıradan olanı istemektir) ve
  `animation.set_speed(id, hız)`; okuma yarısı da `animation.state(id)` →
  `{ clip, time, duration, speed, looping, clips }`, artı `animation.clip` / `animation.is_playing`
  yardımcıları. `clips` listesi olmadan bir script hangi klibi seçebileceğini bilemez — var olmayan
  bir ad motor tarafında uyarıyla yok sayılıyor.
  Uçtan uca test yaz-oku-yaz turunu sürüyor: script `run`'a geçiyor, hızı ayarlıyor, sonra
  `is_playing(id, "run")` okuyup `idle`'a dönüyor. Ayna kaldırılınca ikinci geçiş hiç olmuyor
  (`Some("run")`, `Some("idle")` yerine).
- **`SetFightCamera`'nın ne üreticisi ne işleyicisi var** (`commands.rs:151`; `play.rs`'te yalnız
  bir yorumda anılıyor). Stüdyonun kendi otomatik dövüş kamerası (`simulation.rs`) aynı işi
  yapıyor ama sayıları gömülü ve yalnız editörde koşuyor — ihraç edilen oyunda dövüş kamerası yok.
  Yani seçenek "sil" ile "stüdyodaki çerçeveleme matematiğini motora taşı, komut onu sürsün"
  arasında; ikincisi bir kopyayı da ortadan kaldırır.
- ~~Editör modunda fizik DEĞİŞKEN dt ile adımlanıyor~~ **— iz yanlıştı, altındaki kusur çok daha
  büyüktü; düzeltildi.** Ölçüm önce izi çürüttü: `main.rs`'in gördüğü dt zaten kırpılmış
  (`windowed/event.rs:575`, `dt.min(0.05)`) ve `PhysicsWorld::step` çözücüye asla değişken bir
  adım vermiyor — deltayı kendi akümülatörüne bankalayıp tam 1/240 s alt-adımlarda harcıyor
  (`world/step.rs:67,105`). Yani editör modu hem sabit adımlı hem de 60 ve 300 fps'te aynı
  duvar-saati hızında koşuyordu.

  **Asıl kusur o satırın KAPISIZ olmasıydı.** `main.rs`'in `set_update` kancası fiziği koşulsuz
  adımlıyordu, sonra `update_studio` → `handle_simulation` → `PlayLoop::step` bir daha adımlıyordu:
  ▶ basılıyken dünya kare başına İKİ kez adımlanıyordu. Ölçüldü — 60 karede (bir saniye) düşen bir
  cisim: doğru yolda **4.909 m**, kapısız hâlde **19.530 m**, yani **3.98 kat** (2× simüle edilmiş
  zamanın ½gt² imzası). Editörde gördüğün oyun ihraç ettiğin oyun değildi, ki `PlayLoop` tam olarak
  bunu imkânsız kılmak için (`9cbdddf`) çıkarılmıştı. İkinci sonuç: ⏸ yalnız `PlayLoop`'u
  durduruyor, `PhysicsWorld::is_paused`'u kimse yazmıyor — yani "⏸ DURAKLATILDI" katmanı hâlâ düşen
  cisimlerin üstüne çiziliyordu.

  Düzeltme: `systems::simulation::editor_owns_the_physics_step(world)` — yalnız oyun oturumu dışında
  true. Editör modu değişmiyor (bir yığının oturmasını izlemek onunla oluyor). Karar birim testli,
  ve `main.rs`'in kapıyı gerçekten sorduğu bir kaynak-biçimi testiyle sabit (kapı kaldırılınca
  kırmızı). **Yan bulgu:** o kaynak testinin komşusu olan `the_play_frame_is_the_shared_step...`
  NEGATİF bir `contains` guard'ı ve yorumları kesmiyordu — koruduğu satırın üstüne yazılan bir
  açıklama onu kırmızıya düşürdü. Pozitif guard'lar için 2026-08-18'de öğrenilen dersin ayna
  görüntüsü; artık o da yorumları kesiyor.
- **`PhysicsTime`'ın 8 adımlık ölüm-sarmalı tavanına ulaşılamıyor.** `Time::update` dt'yi 0.05'e
  kırpıyor (`time.rs:62,87`) ve `windowed/event.rs:575` bir kez daha kırpıyor; akümülatörde kalan
  da her zaman < 1/60. Yani birikim en fazla ~0.0667 s = 4 adım, tavan ise 8·(1/60) = 0.133.
  Tavan ya ölü ya da yanlış yerde; hangisi olduğu ölçülmeli.
- **§4'ün "96 public type `#[non_exhaustive]`" sayısı bayat** (`ENGINE.md:1103`); CLAUDE.md
  2026-08-18'de 122'ye düzeltilmişti. §8'in kendi kuralı bu satırı adıyla anıyor.

Düzeltilmiş ve kaydı tutulan yedi kusur. İlk üçü: script sırası (`HashMap` → `BTreeMap`,
proses başına rastgeleydi), on altı komutun sessizce yutulması, bir script'in hatasının ötekileri
iptal etmesi. **2026-08-15'te dördü daha — kayıtta adı geçen bütün açık maddeler:**

- **Tuş haritası her girdisinde yanlıştı.** Lua tablosu **USB HID** kullanım kodları taşıyordu
  (`w = 17`, `space = 44`), motor ise `winit::KeyCode as u32` saklıyor (KeyW = 41, Space = 62).
  İkisi de gerçek numaralandırma; aynı numaralandırma değil ve ikisini karşılaştıran hiçbir şey
  yoktu. En kötüsü: `down = 81` ve `right = 79`, winit'in ArrowRight ve ArrowDown'ı — yani ok
  tuşlarını okuyan bir script, oyuncu aşağı bastığında sağa gidiyordu. `up`/`left` ise kendi
  kodlarına denk gelmişti, yani girdi yarı çalışıyor gibi görünüyordu. Test de yakalayamazdı:
  aynı transkripsiyonu kullanıyordu (17'ye basıp "w" iddia ediyordu) — aynası kopya olan bir ayna
  testi. Sayılar artık `gizmo_core::input::keys`'te bir kez, winit'in bildirim sırasından
  üretilmiş; doğrulama winit'i gören crate'te (`gizmo-app`), hiçbir girdi denetimsiz kalmayacak
  bir kapsam iddiasıyla.
- **Sonsuz döngü prosesi bitiriyordu.** Ne komut sayacı ne bellek tavanı vardı: `while true do end`
  yield etmiyor, `call` dönmüyor, kare bitmiyor, pencere kapanma olayını bile işlemiyor. Yakalanacak
  sinyal ve yardımı dokunacak bir watchdog yok — VM'i ancak VM kesebilir. 10 000 komutta bir çalışan
  hook **çağrı başına** bütçe harcıyor (kare başına değil: `update` bütün scriptleri koşturur ve
  ortak bütçe, ilk hatalı script'in herkesinkini harcaması demekti — hata izolasyonundan kaldırılan
  aynı kusurun başka para birimindeki hâli). Bellek tavanı da taşan ayırmayı yakalanabilir Lua
  hatasına çeviriyor. Negatif kontrolü farklı: hook kaldırılınca test kırmızıya düşmüyor, **asılıyor**.
- **`_G` paylaşımlı tabloydu.** Her script kendi env'inde koşuyordu, yani örtük `FOO = 1` yereldi —
  ama `_G`, env'in `__index`'i üzerinden motorun globals'ına çözülüyordu, dolayısıyla her Lua
  öğreticisinin "global yapmanın açık yolu" diye yazdığı `_G.FOO = 1` paylaşılan tabloya yazıyordu.
  Ölçüldü: A `_G.LEAK` yazdı, B `from-a` okudu. Yükleme sırası alfabetik olduğu için bu, sırayla
  yarışan bir paylaşım. `_G` artık script'in kendi tablosu; okumalar hâlâ düşüyor, yani API ve
  standart kütüphane aynen görünür.
- **NaN doğrulaması on bir varyanttan birinde vardı.** `sanitize_dim` yalnız collider ölçülerini
  kapsıyordu; pozisyon, kuvvet, impuls, hız, kamera fov'u, animasyon hızı ve hasar denetimsizdi.
  Denetim artık her komutun geçtiği tek yerde — kuyrukta — ve match'i **tam**: `_` kolu yok, yani
  float taşıyan yeni bir varyant orada derleme hatası (doğrulandı: `E0004`). Komutlar kırpılmıyor,
  düşürülüyor: kırpılmış bir kuvvet karenin sessizce kabul ettiği yanlış cevaptır.

- **API tabloları paylaşılan nesnelerdi** (aynı gün kapatıldı). `_G` yalıtımı bir script'in
  *global*lerini kendine ait yaptı; API tablolarını yapmadı, çünkü `input.is_pressed = f` bir
  global yazımı değil, her script'in elinde tuttuğu bir **nesnenin alanına** yazma. Ölçüldü: A
  `input.is_pressed`'i değiştirdi, B A'nın sürümünü çağırdı.
  Çözüm çıplak bir `__newindex` **değil** — o metametot yalnız tabloda *olmayan* anahtarlar için
  tetikleniyor ve ezilmeye değer her anahtar zaten var. Bu yüzden script'in gördüğü global boş bir
  **proxy**: her okuma ıskalayıp `__index` üzerinden gerçek tabloya gidiyor, her yazma —yeni
  anahtar olsun olmasın— `__newindex`'e düşüp reddediliyor. Gerçek tablo Lua'nın adlandıramadığı
  **registry**'de duruyor; `__metatable = false` de `getmetatable` ile çıkarılmasını engelliyor.
  Motor kendi kare-başı yazımlarını `raw_set` ile yapıyor: yazan motor, okuyan script.
  Dokuz modülün on dördü tablo bu desene geçti. Yan kazanç: `entity` ve `scene`'in Lua
  yardımcıları **her frame yeniden tanımlanıyormuş** (kendi yorumu "idempotent" diyordu) — artık
  kayıt sırasında bir kez.
  Kırılabildiği doğrulandı: proxy yerine klasik yanlış çözüm (`__newindex`'li gerçek tablo)
  konunca test kırmızıya düşüyor.

**Hâlâ açık:** bir script kendi env'inde adı gölgeleyebilir (`input = başka_şey`) — ki `_G`
yalıtımı bunu zararsız kılıyor, gölge o script'e özel. Ve motorun global olarak sunmak yerine
*değer olarak* verdiği tablolar bu kapsamda değil.

**Teşhis sondası köprüsü — engel diye kaydedilen şey engel değilmiş (2026-08-15).** Kayıt şöyle
diyordu: `send` özelliğiyle `create_function`'ın kapanışı `Send + 'static` olmak zorunda, yani bir
Lua callback'i `&World` yakalayamaz; bütün okuma yolu bu yüzden kare-başı anlık görüntü, ve
parametreli sorgu (şu (x,z)'de zemin kotu) anlık görüntüyle ifade edilemez.

Muhakemenin ilk yarısı doğru, sonucu yanlış. O sınır `Lua::create_function`'ın; **`Scope::create_function`'ın
sınırı `F: Fn(..) + 'scope` — ne `Send` ne `'static`.** Yani kapsamlı (scoped) bir kapanış dünyayı
ödünç alabiliyor, ve ödünç kapsam bitince bitiyor: tam da karenin ömrü.

Çalışan hâli: `api_physics::with_call_time_queries` kareyi bir `lua.scope` içine alıyor, `&World`
tutan `physics.ground_at(x, z)`'yi kuruyor, scriptleri koşturuyor, çıkarken adı geri siliyor —
böylece adı saklayıp sonra çağıran bir script "destructed callback" yerine sade bir nil alıyor.
Testi kayıttaki örneğin kendisi: üstü y=2'de olan bir zemin plakası; script `ground_at(0,0)` için
2.0, plakanın dışında `nil` alıyor (0.0 değil — zemin yok ile zemin sıfırda aynı cevap değil).
Kırılabildiği doğrulandı: fonksiyon kurulmayınca test kırmızı.

Yani sonda köprüsü artık bir tasarım kararı beklemiyor; mekanizma kurulu ve tek tüketicisi var.
Kalanı sıradan iş: hangi sorguların sunulacağı.

### Rollback: iki uygulama, biri eksik (2026-08-15)

`gizmo-net`'te iki rollback var. `RollbackSession` `PhysicsWorld`'ü otoriter sayıp tam
`WorldSnapshot` ile yedekliyor; **windowed app'in bağladığı** `RollbackManager` ise ECS'i otoriter
sayıp entity başına altı sayı tutuyor: konum, dönüş, iki hız, uyku bayrağı.

Aradaki fark, fizik crate'inin "transform ve hızdan **türetilemez**" diye belgelediği her şey:
substep accumulator, contact cache'in warm-start impulse'ları, eklemlerin tek yönlü `is_broken`
mandalı ve mandallanmış referans pozları, oyunun kare içinde değiştirebildiği kuvvet alanları ve
sıvı hacimleri. Üstelik app, rollback'ten sonra `pw.clear_bodies()` çağırıp fizik dünyasını ECS'ten
yeniden kurduruyordu.

**Önce yanlış ölçtüm, kayda o da geçsin.** İlk testim "ıraksıyor" dedi (C9C742C6… ≠ 992A5D82…) ama
ıraksamayı üreten şey testimdeki bir fazla adımdı: `end_frame` tick'i adımdan *sonra* kaydediyor,
yani tick T'ye dönmek "T+1 adım atılmış" duruma dönmek. Hesap düzeltilince o senaryo — dört düşen
kutu — **düzeltmeyle de düzeltmesiz de** geçiyor. İki uygulamayı ayırt edemeyen bir test hiçbiri
hakkında kanıt değil.

Ayırt eden senaryo: **pencere içinde kopan bir eklem.** `is_broken` yalnız `PhysicsWorld`'de yaşayan
tek yönlü bir mandal, yani bileşen geri yüklemesiyle geri alınamaz — re-simülasyon, kesintisiz
koşunun hâlâ sahip olduğu bir eklem olmadan devam ediyordu. Test tick 12'de kopacak şekilde
ayarlı (hedef 8, geri sarma 20) ve kopmanın pencere içinde olduğunu ayrıca doğruluyor; düzeltme
kapatılınca "eklem hâlâ kopuk" diyerek kırılıyor.

Düzeltme, denetlenmiş uygulamayı yeniden kullanıyor: `RollbackManager` artık tick başına
`PhysicsWorld::snapshot()`'ı da **yerel** olarak tutuyor (tel formatı `PhysicsStateSnapshot` aynen
kalıyor — `WorldSnapshot` contact manifold ve eklem taşıyor, ağa gitmez). Geri yükleme fizik
dünyasını kurup satırlarını ECS'e de yazıyor, çünkü bir sonraki adımda `sync_bodies` ECS'ten
kopyalayıp düzeltilen satırları ezerdi — fonksiyonun kendi dokümanının uyardığı şey. App'in
`clear_bodies()`'i kalktı (artık geri yüklenen durumu atardı) ve fast-forward döngüsü
`record_resimulated_tick`'i çağırıyor; o döngü `end_frame`'i satır içi tekrar yazdığı için fizik
yarısı oradan da eksikti.

### Açık kalan iki karar (kusur değil)

- **Çizim listesi birleştirmesi.** `collect_draw_items` (~950 satır) ile studio'nun batching'i hâlâ
  iki uygulama. Ölçüye dayanarak yapılmadı: bugün bulunan sekiz sürüklenmenin hepsi ortak bir
  *karardaydı* ve hepsi tek kaynağa indi, her biri bekçiyle. Kalan ikizlik döngü **yapısı**, ve onu
  birleştirmek pass kaydını birleştirmek demek — otomatik kapsamı olmayan ve insan-gözü kapısına
  bağlı olan yarı. Çizgi: "dünyayı okuyan ortak, komut kaydeden ayrı".
- **Sonda köprüsünde hangi sorgular sunulacak.** Mekanizma kuruldu ve tek tüketicisi var
  (`physics.ground_at`); geri kalanı API tasarımı.

### "1.0 için" bekleyen işler normal işe çevrildi (2026-08-17)

Roadmap'te bir sürüme kilitli üç madde ve beş "1.0 CI kapısı" duruyordu. Kilidi kaldırınca çıkan
ilk şey şu oldu: **maddelerin çoğu ya çoktan bitmişti ya da hiç bakılmamıştı** — bir milestone'un
işe yaptığı tam olarak bu, "henüz yapılmadı"yı "bakılmadı"ya çeviriyor.

Ölçülen durum ve alınan kararlar §3'te tablo hâlinde; buradaki kayıt, yapılan işin kendisi:

- **`unsafe` sözleşmeleri — KAPANDI.** Lint 106 yer bildiriyordu ama çoğunda gerekçe **zaten
  yazılıydı**: clippy'nin aradığı yerde değil, insanın okuduğu yerde (bir grup `unsafe`'in üstünde
  tek yorum, ya da araya girmiş bir `let`). Yani madde göründüğünden çok daha yakınmış. Önce
  `gizmo-core` dışındaki 25 yer kapatıldı; sonra `gizmo-core`'un kendisi — deponun `unsafe`'inin
  çoğunun yaşadığı yer — ve orada yazılan şey üç gerçek değişmez: depolama `Send`/`Sync`
  impl'lerinin arkasındaki `Component: Send + Sync` sınırı, `UnsafeCell` sütun erişiminin
  arkasındaki *çağıran-tarafı* aliasing sözleşmesi, ve her ham pointer'ın arkasındaki satır
  canlılığı. **20 crate'in 20'si mandalda**, workspace sıfırda.

  Yol boyunca düzeltilen bir yanlış kayıt: `BlobVec`'in `Send`/`Sync` gerekçesi "erişim &self/&mut
  self üzerinden, RefCell guard'ları koruyor" diye yazılıydı. O, *aliasing* argümanı; thread'ler
  arası taşımayı meşru kılan şey `Component: Send + Sync` sınırı. Gerekçe düzeltildi.
- **`missing_docs`.** `gizmo-window` (0) ve `gizmo-ui` (5 madde yazıldı) mandala eklendi. Geri
  kalanın sayıları §3'te; kural şu: bir crate sıfıra inince mandala girer, böylece birikim
  eritilirken yeniden büyümez.
- **Ve wasm kapısı işini yaptı.** Mandal her crate'e takılınca `gizmo-audio`'nun **yalnız wasm
  kolunda** derlenen ikinci `unsafe impl`'i (`Sync`) açıkta kaldı: gerekçe ilk impl'in üstünde tek
  blok hâlindeydi. Native işin göremeyeceği bir yer — ve bu, CLAUDE.md'nin "wasm kapısını alt
  kümeyle koşturma" uyarısının aynı hafta ikinci kez haklı çıkışı.

  Üçüncü kez de aynı gün geldi ve bu sefer ders farklıydı: `-p gizmo-app`'in wasm derlemesi
  yerelde `egui-winit` yüzünden düşüyordu ve bu "yerele özgü bir tuhaflık" diye kaydedilmişti.
  Değildi — CI o adımı `--no-default-features --features render,physics,scene` ile koşuyor, ben
  varsayılan özelliklerle koşmuştum. Yani kapıyı değil, kendi uydurduğum bir komutu koşturmuşum.
  Doğru bayraklarla yerelde de yeşil. **Kapıyı koştururken komutu workflow'dan kopyala.**

### Decal editörde görünmüyordu (2026-08-17, DÜZELTİLDİ)

Bir decal bir **projektör**dür: zaten orada olan yüzeyi boyar, dolayısıyla o yüzeyin konumuna
ihtiyacı vardır. Motorun deferred geçişi bunu G-buffer'dan okur — oyun için çalışır, editör için
hiç çalışmaz: stüdyo forward çiziyor ve G-buffer doldurmuyor. Sonuç, editörün WYSIWYG olma iddiası
için en kötüsü: kullanıcı decal'ı yerleştiriyor, boş zemin görüyor, "bozuk" diye kaydediyor —
oysa gönderilen oyunda splatter oradaydı. `render_parity.rs` bunu "gerçek ve **düzeltilmemiş**"
diye kaydetmişti; bu, o kaydı kapatıyor.

**Düzeltme:** forward bir decal geçişi (`decal_forward.wgsl` + `record_forward_decals`), yüzey
konumunu **derinlik tamponundan** geri kuruyor — forward hattın zaten yazdığı tampon — ve ışıklı
HDR görüntüsüne alfa harmanlıyor. Projeksiyon matematiği, hacim testi ve dairesel solma deferred
sürümün birebir aynısı: editörde bir türlü, oyunda başka türlü görünen bir decal, hiç
görünmeyeninden kötüdür.

İki ayrıntı kayda değer:

- **Dünyayı okuyan taraf ortak.** `collect_decals` (facade'da, çünkü `DecalState` renderer'da ve
  `Transform` physics-core'da) her iki geçişi de besliyor. İkisinin anlaşmak zorunda olduğu şey
  decal'ın *nerede* olduğu; nasıl kaydedildiği değil.
- **Tek uniform, iki shader.** CPU tarafı `inv_model`'i `model⁻¹ · T(kamera)` olarak katlıyor,
  çünkü deferred okuyucu kameraya göreli konum veriyor. Forward shader da geri kurduğu mutlak
  konumdan kamerayı çıkarıp aynı matrisi kullanıyor — böylece iki hat aynı tamponu paylaşıyor ve
  "decal nerede" sorusunda ayrışamıyorlar.

Ayrıca derinlik tamponu bu geçişte **örnekleniyor**, attachment değil (wgpu ikisine birden izin
vermez) — parçacık geçişinin kalıbı. Ön yüzler kırpılıyor: derinlik testi olmadan kutunun iki yüzü
de rasterize olur ve her piksel iki kez harmanlanırdı.

Doğrulama, stüdyonun kendi piksel koşumunda: `a_decal_is_visible_in_the_editor_viewport`. Geçiş
kapatılıp koşuldu — **0/16384 piksel**; açıkken geçiyor. Ve `render_parity.rs`'teki `Decal`
istisnası kaldırıldı: artık iki hat da tanıyor.

### Ölü biçimdeki iki varlık: artık ne oldukları söyleniyor (2026-08-17, KAPANDI)

Depoda iki dosya, ikisi de motorun kendi varlıkları, ikisi de yüklenemiyordu — ve ikisinin de
hatası yanlış şeyi söylüyordu. Denetim (2026-08-04) bunu zaten kaydetmişti; kapatan bu.

**1. `perfect_car.scene` — reflection çağından.** Bileşenleri alan-adı anahtarlı iç içe map olarak,
enum'ları iç etiketli (`"shape": {"type": "Aabb", …}`) yazıyor; bugünkü biçim her bileşeni bir RON
**string**'i olarak yazıyor. Ayrıştırma tipli deserialize'da düşüyor — yani `migrate` dosyanın
sürümüne bakamadan — ve kullanıcıya kalan mesaj `10:28: Expected string` oluyor. Sürüm alanının
tam da bu yüzden var olduğu bir dosyada, sürüm mekanizması devreye giremiyor.

Yeni davranış: ayrıştırma düştüğünde dosya bir kez de `ron::Value` olarak okunuyor ve doğrudan
sorulan soru şu — **bileşen yükü string mi, map mi?** Map'se hata `SceneError::LegacyComponentEncoding`
ve mesaj ne olduğunu ve ne yapılacağını söylüyor. Bu ikinci ayrıştırma yalnız zaten başarısız olmuş
yolda koşuyor.

Dosyanın kendisi silinmedi, **fixture oldu**: `crates/gizmo-scene/tests/fixtures/legacy_reflection.scene`.
Sentetik bir örnek değil, gerçek relik — ve testi o çalıştırıyor. Üç bekçi: eski dosya adıyla
anılıyor mu, sıradan bozuk bir dosya hâlâ `Parse` mı (yoksa her hata "sahneniz eski" olur), ve
bugünkü biçim hâlâ yükleniyor mu.

**2. `prefab_8.prefab` — ikili çöp.** RON değil, metin bile değil; `read_to_string` UTF-8'de
düşüyor ve kullanıcının gördüğü şey **"scene file I/O error"** — yani yerinde duran, okunabilir bir
dosya için "dosya açılamadı" diyen bir mesaj. Dosya silindi (export her oyuna kopyalıyordu), ve
`InvalidData` durumu artık ayrı raporlanıyor: "bu dosya metin değil, başka bir (muhtemelen ikili,
eski) biçimde yazılmış". Yanındaki `Default_Cube.prefab` ölçüldü, **yükleniyor** — o sağlam.

### Kalan GPU alt sistemleri tarandı: biri hiç koşmuyormuş (2026-08-16)

Aynı alet (geçişi kaldır, pikseli say) hiç ölçülmemiş dört isteğe bağlı sisteme tutuldu.

| Sistem | Varsayılan | Sonuç |
|---|---|---|
| `gpu_particles` | **açık** | **çalışıyor** — emitter'lı sahnede 10 karede 197/16384 px, 60 karede 355, max delta 142 |
| `gpu_particles` + `gpu_fluid` | **açık** | parçacığı/sıvısı olmayan sahnede **0/65536 bayt** — boşta duruyorlar, kirletmiyorlar |
| `smoke` | kapalı | ölçülmedi; varsayılan `None`, demo `Some(SmokeVolume::new(..))` veriyor |
| `gpu_fluid` (kullanıldığında) | — | ölçülmedi; SPH parçacığı olan bir fixture gerekiyor (`fluid_rigid` demosu bu yolu koşturuyor) |
| **`gpu_cull`** | kurulu | **hiç koşmuyor — kaldırıldı** |

**`gpu_cull`'un hikâyesi** kayda değer, çünkü SSR/SSGI'den farklı bir ölü türü: geçiş yanlış
çalışmıyordu, **hiç çağrılmıyordu**. `GpuCullState` her renderer'da kuruluyordu (compute pipeline,
üç tampon, bind group), `prepare()` ve `cull_pass()` eksiksiz yazılmıştı — kapasite taşmasını
uyaran bir `warn!`'ı ve `clamped_draw_count` için kendi birim testi bile vardı — ama depoda tek bir
çağıranı yoktu. Gerekçe de koddaydı, `default_render_pass`'in içinde:

```rust
// GPU cull pass removed since we use CPU instancing
```

Yani karar zaten verilmiş, yalnız kurulum geride kalmış. Silinen: modül (256 satır), `mesh_cull.wgsl`,
`Renderer::gpu_cull` alanı, kurulumu, ve `WebProfile::gpu_cull_enabled` — o bayrağın da okuyanı
yoktu, yalnız iki testte doğrulanıyordu. CPU tarafındaki `frustum_cull` çalışan yol ve öyle kalıyor.

**Geri istenirse ne gerekir:** eksik olan yarı `prepare`/`cull_pass` değil, **çizim** tarafı —
batch'lerin `draw_indirect`'e taşınması ve mesh sınırlarının her kare yüklenmesi. Ve deponun kendi
kuralına göre önce bir ölçüm: CPU culling'in karede ne kadar tuttuğu. (Aynı kural narrowphase
batch-SIMD'i %3'te reddetmişti.) Kod git geçmişinde duruyor.

Bekçi: `particles_from_an_emitter_reach_the_frame`. Böylece "kareye ulaşıyor mu" ailesi dörde
çıktı — SSR/SSGI, volumetric, decal, parçacıklar.

### Pencereli her uygulama ilk karede ölüyordu (2026-08-16, DÜZELTİLDİ)

`cargo run -p demo --bin advanced_physics` — CLAUDE.md'nin belgelediği komut — açılıştan ~1 saniye
sonra `Dropped TexturesDelta with 1 unapplied deltas` ile ölüyordu. Export çalışma-zamanı için ilk
sahne denemesinde çıktı ve kusur yeni binary'de değil, motorun pencere döngüsündeydi: deponun
kendi demosu da aynı biçimde ölüyor.

Kare şöyle akıyor: egui karesi en başta koşuyor (`event.rs:574`), sonra swapchain görüntüsü
alınıyor. Alınamazsa — outdated surface, uçuşta bir resize, timeout; ve **yeni haritalanmış bir
pencerenin ilk karesi tam olarak budur** — tek bir epilog erken dönüyor ve o `FullOutput`
uygulanmadan düşüyordu.

Maliyeti iki katmanlı, ve sessiz olan yarısı daha kötüsü:

- **Debug**: `TexturesDelta::drop` bir `debug_assert!`. Süreç ölüyor — yani hiçbir pencereli demo
  debug'da açılamıyordu.
- **Release**: assert sessiz. Ama egui her deltayı **bir kez** verir; düşen çıktı o karenin
  taşıdığı yüklemeleri kalıcı olarak götürür. Atlas o karede yeniden kurulmuşsa (ölçek/DPI
  değişimi, font değişimi) yazı bir daha geri gelmez, glyph'ler boş kutu olarak çizilir.

Aynı kusurun ikinci yüzü `EguiContext::render`'ın kendisindeydi: deltaları referansla uygulayıp
listeyi `clear()` etmiyordu, dolayısıyla **çizilen** karede de aynı assert'e düşülüyordu.

Düzeltme `absorb_unpainted_frame`: çizilmeyecek karenin dokularını yükleyip listeyi temizler,
platform çıktısını da uygular (UI koştu — imleç/pano onun sonucu). Atlanan kare **pikselleri**
atlar, yüklemeleri değil. `render` de uyguladıktan sonra temizliyor.

Doğrulandı: düzeltmeden önce `advanced_physics` 1 s içinde exit 101; sonra 25 s boyunca ayakta ve
`GIZMO_SCREENSHOT` ile kare üretiyor. Bekçiler `egui_frame_ownership_tests` — kaynak-şekli
testleri, çünkü "surface outdated" durumu gerçek bir swapchain olmadan birim testine sokulamıyor.

Neden bugüne kadar görülmedi: demolar `--release` ile koşuluyor (CLAUDE.md fizik demoları için
bunu şart koşuyor), release'te panik yok, ve kalan zarar hem sessiz hem koşullu.

### Post-process kontrolleri taraması: SSAO dışında hepsi çalışıyor (2026-08-16)

Gölgeleme çiplerinin üçünün ölü çıkması üzerine, editörün TÜM render kontrolleri aynı yöntemle
ölçüldü — headless 128×128 render, kontrolü kıpırdatıp piksel farkı say:

| Kontrol | Fark | |
|---|---|---|
| bloom eşiği · exposure · vignette · film grain | %66–75 | çalışıyor |
| dof blur · fxaa | %1.5–2.0 | çalışıyor |
| dof odak aralığı · chromatic aberration | %0.3–0.7 | çalışıyor |
| bloom yoğunluğu | %75 (eşik 0'da) | çalışıyor |
| dof odak mesafesi | %1.6 (aralık ≥20'de) | çalışıyor |
| **SSAO** | %0 | **ölü — zaten belgeli, widget'ları kapalı** |

Yeni kusur yok. Ama sonuca varmak **iki kötü deney** gerektirdi ve ikisi de önce "bu kontrol ölü"
diye okundu; bir daha aynı tuzağa düşülmesin:

- **Bloom yoğunluğu**, varsayılan eşikte 0 fark verir. Sahnede eşiği aşan hiçbir şey yoksa
  yoğunluğun büyütecek bir şeyi yoktur. Eşiği 0'a indirmeden ölçmek, kontrolü değil fixture'ı
  ölçer.
- **DoF odak mesafesi**, dar aralıkta 0 fark verir. `coc = clamp(|view_dist - focus| / range, 0, 1)`
  — `range` küçükken her iki odak değeri de coc'u 1.0'a doyurur, iki resim de eşit derecede
  bulanık olur. Aralığı ≥20 tutmadan ölçmek yine deneyi ölçer.

Genel kural: bir kontrolün ölü olduğunu ancak **etkili olabileceği** bir zeminde ölçtükten sonra
söyle. Gölgeleme çipleri gerçekten ölüydü (her zeminde 0/65536); bu ikisi değildi.

### SSR ve SSGI iki karakter yüzünden ölüydü (2026-08-16, DÜZELTİLDİ)

Aynı mercek bir seviye aşağı tutuldu: kontrolü değil **geçişin kendisini** kaldır
(`renderer.ssgi = None`) ve kareyi baytına kadar karşılaştır. Dört sahne, beş ekran-uzayı geçişi:

| Geçiş | Kaldırılınca fark | |
|---|---|---|
| SSAO | %10–15 | çalışıyor — *kontrolü* ölü, o ayrı konu (yukarıdaki tablo) |
| TAA | %0,4–1,6 | çalışıyor |
| volumetric | bu sahnelerde eşik altı (max delta 5–8) | **fixture'dı** — doğru zeminde %14,5, aşağıya bak |
| **SSGI** | **0/65536 bayt** | **ölü** |
| **SSR** | **0/65536 bayt** | **ölü** |

Sıfır "az" demek değil, **birebir aynı kare**: durum kuruluyor, pass kaydediliyor, pipeline
derleniyor, her karede koşuyor — ve toplamsal apply'ı kareye hiçbir şey eklemiyor.

Sebep tek eşik. `gbuffer.wgsl` "bu piksel yazıldı" bayrağını world_position'ın alfasına paketliyor:

```wgsl
let packed_ss_aniso = (0.5 + 0.49 * anisotropy_raw) + floor(100.0 * subsurface_raw);
```

Sıradan bir malzemede (anizotropi 0, subsurface 0) bu **tam 0,5**, ve 0,5 Rgba16Float'ta tam
temsil edilir — yuvarlama payı yok. Bayrağı okuyan on yerden sekizi kapsayıcı yazılmış
(`< 0.5` → atla, ya da `>= 0.5` → geçerli): ssao'nun ikisi, deferred_lighting, volumetric, taa,
ssgi_temporal ve SSR/SSGI'nin **kendi giriş kapıları**. Işın yürüyüşünün isabet testi ise iki
shader'da `> 0.5` idi — `ssr.wgsl:71` ve `ssgi.wgsl:107` — yani sıradan her piksel için yanlış.
Işın 20 (SSR) / 8 (SSGI) adımını sonuna kadar koşup hiçbir isabet kaydedemiyor, siyah dönüyordu.

`>= 0.5` yapıldı. Ölçüm (ayna zemin, 128×128, pass açık vs. kaldırılmış):

| | önce | sonra |
|---|---|---|
| SSGI | 0 px · 0 bayt | 2005/16384 px (%12,2) · 7959 bayt · max delta 64 |
| SSR | 0 px · 0 bayt | 426/16384 px (%2,6) · 1908 bayt · max delta 26 |

Bekçi: `screen_space_reflections_and_gi_reach_the_frame` (`golden_render_tests`). Eşikler ölçülenin
çok altında — korunan şey "geçiş kareye ulaşıyor mu", ayarı değil. Düzeltme geri alınıp test
koşuldu: `removing SSGI changed 0/16384 pixels` ile kırmızı, yani kusuru gerçekten tutuyor.

**Düzeltme nereye ulaşıyor.** `default_render_pass` — yani oyun yolu; `platformer`, `vehicle_scene`,
`cloth_demo` gibi geçişleri açık bırakan demolar bugünden itibaren gerçekten SSR/SSGI görüyor.
Editör viewport'u **değişmez**: stüdyonun kendi hattı bu geçişleri hiç kaydetmiyor (`gizmo-studio`
içinde `ssr`/`ssgi` geçmiyor, "iki render yolu" bölümüyle tutarlı), `SimpleApp` ile
`with_scene_render` ise ikisini bilerek `None`'a çekiyor. Değişikliği stüdyoda arayan biri
"düzelmemiş" sonucuna varır.

**Neden yıllarca hiçbir şey yakalamadı.** Kurulum, kayıt, bind group, shader derlemesi — her yan
sağlıklı görünüyordu; yalnız resim biliyordu. Ve zemin seçimi burada da belirleyici: SSGI ayna
zeminde %12,2 verirken kırmızı duvarlı "bounce" sahnesinde eşiğin altında kaldı (max delta 6).
Toplayacak parlak komşu yoksa GI'yi ölçmek yine fixture'ı ölçer.

**Ve aynı tarama volumetric'i az kalsın yanlış mahkûm ediyordu.** Yukarıdaki tabloda "max delta
5–8" satırı, bu belgenin bir bölüm yukarıda yazdığı kuralın ihlaliydi: ölçüldüğü dört sahnenin
hiçbirinde kamera güneşe bakmıyordu. Katkının tamamı `sun_intensity · faz · yürüyüş boyu`, ve
kamera güneşe dönmediğinde üçü birden çöküyor — Henyey-Greenstein lobu `g = 0,55`'te güneşe
doğru 0,61, tersine 0,015 (kırk kat), ve yakın geometriye çarpan ışın gökyüzünün 100 birimi
yerine 6 birim yürüyor. Kamera güneşe çevrilince (`yaw = π/2, pitch = π/4`, çünkü
`DirectionalLightBundle::default()` güneşi (0, +0,707, +0,707) yönüne koyuyor) ve ışını kesen bir
levha konunca: **2376/16384 piksel (%14,5), max delta 22**. Geçiş sağlam.

Bekçi: `volumetric_god_rays_reach_the_frame`. Bu test aynı zamanda kuralın kendisinin kaydı — tek
sahnenin sıfırı, geçişin ölü olduğunu göstermez.

**Aynı bayrağın üçüncü yazımı: decal.** `decal.wgsl` bayrağı `world_pos_val.w == 0.0` ile, yani
tam kayan-nokta eşitliğiyle okuyordu. Bugün doğru çalışıyor — temizleme değeri tam 0,0 ve tam
temsil ediliyor — ama `(0, 0,5)` aralığındaki her şeyi de içeri alırdı; on okuyucu içindeki en
kırılgan biçim. `< 0.5`'e hizalandı. Bu yeniden yazım bugün tek pikseli değiştiremez (kodlayıcı o
aralıkta değer üretmiyor), tam da bu yüzden arkasına tartışma değil ölçüm konuldu.

Ve decal geçişinin **hiçbir testi yoktu**: beyaz zemine kırmızı projektör → **1133/16384 piksel
(%6,9), max delta 75**. Oyun yolunda çalışıyor; bekçi `decals_reach_the_frame`. Editörde hâlâ
görünmüyor ve bu ayrı bir kusur: decal G-buffer'ın albedo hedefine karışıyor, editörün hattı ise
forward — kayıt `gizmo-studio/tests/render_parity.rs`'te duruyor ve **açık**.

Ölçüm notu: her render için yeni bir `Renderer::new_headless` kurmak GPU belleğini bitiriyor —
4 zemin × 6 render = 24 cihazın 17.'sinde `radv/amdgpu: Not enough memory for command submission`
ve cihaz kaybı geldi (tek başına gölge dizisi 3072²×4). Süpürmeyi gerçekten okunacak satırlarla
sınırlı tut; bu makinede sınır ~16 headless renderer.

### Gölgeleme modları: forward hat deferred'ın numaralandırmasını TEKRARLIYOR (2026-08-16)

Toolbar'ın Lit/Normals/Albedo/Wire çipleri tek bir `shading_mode` uniform'u yazıyor. Modlar
başlangıçta yalnız `deferred_lighting.wgsl`'de vardı; stüdyo ise forward hattan (`shader.wgsl`)
çiziyor, yani üç çip ölçülebilir biçimde hiçbir şey yapmıyordu — Lit'e karşı 0/65536 bayt.

`shader.wgsl` artık 1 (Normals) ve 2 (Albedo) modlarını **aynı numaralarla ve aynı kodlamayla**
uyguluyor. Bu bir kopya, ve bilerek: tek uniform, hangi hat koşarsa koşsun tek anlam. Deferred
tarafına yeni bir mod eklenirse (bugün 3–6: Roughness/Metallic, Shadows, Tangents, ClearCoat) ve
stüdyoda görünmesi isteniyorsa, forward'a da eklenmesi gerekir — `every_shading_mode_draws_a_different_picture`
yalnız toolbar'ın gösterdiği dördünü tutar.

**Mod 3 iki hatta iki farklı şey.** Deferred'da Roughness/Metallic; stüdyoda toolbar'ın dediği şey,
yani **wireframe** — ve wireframe bir shading terimi değil, bir pipeline: `wireframe_pipeline`
aynı shader'dan `PolygonMode::Line` ile kurulu, ve depoda onu seçen hiçbir şey yoktu. Stüdyo o
modda uniform'u 0'da bırakıp pipeline'ı değiştiriyor, tam da bu çakışma yüzünden.

Not: hata ayıklama görünümleri HDR tamponuna yazılıp post-process'ten geçiyor, yani ekrandaki
değerler ham normal/albedo değil. Deferred hattın davranışı da aynı; ayrı bir kusur değil.

### Editör kamera tuşları sağ tuşa kapılandı (2026-08-16, davranış değişikliği)

Araç kısayolları (Q/W/E/R → Seç/Taşı/Döndür/Ölçek, `draw_editor`'da GENEL) ile serbest uçuş
tuşları (W/A/S/D + Q/E, `gizmo-studio` kamera sistemi) üç harfi paylaşıyordu ve uçuş hiçbir
değiştirici istemiyordu. Sonuç: W ile öne uçmak aracı Taşı yapıyordu, kaçışı yoktu.

Uçuş artık **viewport üzerinde sağ tuş basılı** olmasını istiyor — bakışı zaten kapılayan jest, ve
Game panelinin yardım metninin zaten tarif ettiği şey. Bu bir davranış değişikliği: eskiden sağ
tuşsuz uçulabiliyordu.

Bayrak `dragged_by` ile değil `is_pointer_button_down_on() && pointer.secondary_down()` ile
üretiliyor. Sürükleme egui'nin eşiğini geçmeden başlamaz; sağ tuşu basılı tutup fareyi
kıpırdatmadan WASD ile uçmak normal kullanım, drag'e bağlansaydı fare durunca kamera da dururdu.

Geri alınacaksa bilinsin diye yazıldı: kapıyı kaldırmak çakışmayı geri getirir, ve
`free_flight_is_gated_on_the_right_mouse_button` bunu kırmızıya çevirir.

### "Sonucu at, başarıyı yaz" taraması (2026-08-16)

Aynı kalıbın dört örneği bir günde çıkınca depo geneli tarandı. Kalıp şu: bir işlemin `Result`'ı
`let _ =` ile atılıyor, hemen ardından koşulsuz bir başarı satırı basılıyor. Sonuç: log, olmamış
bir şeyi olmuş gibi söylüyor.

Bulunan ve kapatılanlar:

| Yer | Ne diyordu | Gerçek |
|---|---|---|
| `gc.rs` auto-save | `💾 Auto-Save: <yol>` | kayıt başarısız olabilir, dosya yok |
| `build.rs` export | `Kopyalandı -> scripts/` (×4) | ikisinin kaynağı hiç yoktu |
| `render.rs` prefab kaydet | `Prefab kaydedildi.` | yazma başarısız olabilir |
| `render.rs` Ctrl+D | `Obje çoğaltıldı.` | okuma sonucuna bakılmadan |
| `prefs.rs` tercihler | (hiçbir şey demiyordu) | ayarlar sessizce kaybolur |
| `simulation.rs` script | (hiçbir şey demiyordu) | script hiç çalışmaz |

En ağırı auto-save'di: insanın işinin diskte olduğuna inanmak için baktığı satır tam da o.

Tarama sonrası `crates/` altında bu kalıptan **kalmadı** (üretici + iddia dörder satır içinde,
koşulsuz). İki bekçi duruyor: `no_save_call_discards_its_result` (`render.rs`, `gc.rs` kaynağını
okur) ve her düzeltmenin kendi davranış testi.

Sık düşülen tuzak, düzeltmenin ikinci yarısı: **koşulsuz bildirmek de yanlış.** Auto-save, script
reload ve tercih yazımı kare başına koşuyor; düz bir log satırı saniyede altmış kopya demek.
Üçünde de karar "giriş/çıkış anında bir kez" — hafızası olan saf bir fonksiyonda, döngüde değil.

### Panel genişliği taraması: sekiz panelden biri taşıyordu (2026-08-16)

Inspector'ın kendi içeriğini kırptığı bulununca (ayrıntısı commit'te) aynı kusur sınıfı için
**bütün editör panelleri ölçüldü** — göz kararı değil: `Context::run_ui` ile gerçek bir kare
sürülüp karenin boyadığı şekillerin en sağ kenarı okundu, clip rect açık bırakılarak. Kırpma
kusuru ekranda gizleyen şeyin ta kendisi; kırpmadan ölçmek onu sayıya çeviriyor.

Varsayılan yerleşimdeki genişliklerde (1600 px pencere, `create_default_dock_state` bölmeleri):

| Panel | Genişlik | İçerik | Sonuç |
|---|---|---|---|
| Inspector (environment) | 400 | **422.7** | taşıyordu — düzeltildi |
| Inspector (bileşenler) | 400 | 400.5 | sığıyor |
| Hierarchy | 320 | 320.5 | sığıyor |
| Assets · Console · Console (400 karakterlik satırla) | 880 | 880.5 | sığıyor |
| Profiler | 880 | 224.0 | sığıyor |
| Animation | 880 | 304.1 | sığıyor |
| Ayarlar · Script Editor | 400 | 400.5 | sığıyor |
| Toolbar | 1600 / 1280 / 1024 / 900 | +0.5 | sığıyor |

0.5 px kenarlık kalınlığı, taşma değil.

**Ölçümün kör olmadığı doğrulandı** — "temiz" demenin ön koşulu bu: toolbar saçma genişliklerde
tekrar ölçüldü ve 687 px'in altında taşıdığı görüldü (500 px'te 186.7 px taşma). Yani araç
taşmayı görüyor; diğer paneller gerçekten taşmıyor. Toolbar'ın gerçek alt sınırı **687 px**,
stüdyonun varsayılan penceresinin çok altında.

Bir daha taranmasına gerek yok; bekçi `inspector_width_tests` olarak duruyor ve iki yolu da
(seçim yok / seçili nesne) üç genişlikte tutuyor.

### Export bir oyunu değil, sabit bir demoyu paketliyordu (2026-08-16, DÜZELTİLDİ)

Stüdyodaki "Build / Export", `cargo build --release -p demo` koşup çıkan binary'yi
`export/gizmo_game/` altına kopyalıyor ve "Oyununuz hazır" diyor. `demo`'nun varsayılan binary'si
`bevy_3d_scene` — zemin, küp, ışık ve kameradan ibaret **sabit** bir sahne. Hiçbir sahne dosyası
okumuyor, hiçbir script çalıştırmıyor.

Yani export kullanıcının sahnesini değil, motorun örneğini paketliyor. Kopyalanan `scenes/` ve
`scripts/` dizinlerini açan bir şey yok: `Scene::load_into` var ama tek çağıranı editör; `demo`,
`cradle` ve `server` altındaki hiçbir binary sahne dosyası yüklemiyor.

Bugün kapatılan yarısı, kopyalamanın **dürüstlüğüydü** — dört dizin `let _ =` ile kopyalanıp
dördü için de koşulsuz "Kopyalandı" basılıyordu ve kaynakların ikisi (`demo/scenes`,
`demo/scripts`) hiç var olmayan yollardı. Artık ne olduğunu söylüyor ve çalışma anında kullanılan
yollardan (`scripts/`, `scenes/`) okuyor.

Kalan yarısı bir **özellik**: başlangıçta bir sahne dosyası yükleyen, script'leri bağlayan ve
fiziği/render'ı süren bir çalışma-zamanı binary'si. Onsuz export'un kopyaladığı veriyi okuyan
kimse yok. Kapsamı bir hata düzeltmesinin ötesinde olduğu için bilerek açık bırakıldı; kararı
verilmeden yapılmamalı, çünkü "oyun çalışma-zamanı" motorun ne kadarını varsayılan açacağı
sorusudur (fizik? scripting? ağ?).

#### Çalışma-zamanı yazıldı: `gizmo_runtime` (2026-08-16)

Kapsam sorusu **referansla** cevaplandı, zevkle değil: çalışma-zamanı, editörün Play modunun
sürdüğünü sürer — script motoru (`update` → `flush_commands` → entity başına `update_entity`),
1/60 sn sabit adımlı fizik akümülatörü (kare başına en çok 16 adım, borç da 16 adımla sınırlı) ve
`default_render_pass`. Ne fazlası ne eksiği. Bunun değeri şu: "export edilen oyun editörde
gördüğünle aynısını yapar" bir söz olmaktan çıkıp **karşılaştırmaya** dönüşüyor; ikisi ayrıştığı an
biri hatalıdır, tasarım tartışması değil.

Bilerek bırakılan iki fark, ikisi de eksik olan şeyin editör olması yüzünden: asset watcher yok
(hot-reload bir yazarlık aracı) ve script logları editör konsoluna değil stdout'a gidiyor.

Veri nereden geliyor: export edilen dizin binary'nin yanında `scenes/`, `scripts/`, `assets/`
taşıyan **kendi kendine yeten** bir dizin, dolayısıyla runtime o dizini çalışma dizini yapıyor ve
editörün yazdığı göreli yollar anlamını koruyor. Geliştirme ağacında (binary `target/release`'te)
bu kural kendiliğinden kapanıyor — kuralın kendisi testli.

Beklenen şey, **parçaların zaten var olmasıydı** ve öyle çıktı: `App::load_scene` açılışta
`SceneData::load_into`'yu `full_scene_registry` ile zaten çağırıyor. Yazılan şey alt sistem değil,
kablolama.

Doğrulandı: `demo/assets/sample.scene` (aşağıya bak) verilip `GIZMO_SCREENSHOT` ile kare alındı —
zemin, kırmızı küp, mavi küre, güneş ve gölgesi. Yani dosyadan yüklenen sahne gerçekten çiziliyor.

**Yolda çıkan iki kusur:**

1. Pencereli her uygulamanın ilk karede ölmesi (ayrı bölüm, düzeltildi). Runtime'ın ilk denemesi
   onu ortaya çıkardı; kusur runtime'da değildi.
2. **Deponun tek `.scene` dosyası yüklenmiyor.** `demo/assets/perfect_car.scene` eski biçimden:
   `components` alanını iç içe map olarak yazıyor, bugünkü biçim ise her bileşeni **string** olarak
   yazıyor (`"Transform": "(position:(...))"`). Ayrıştırma `10:28 Expected string` ile düşüyor —
   yani sürüm göçü (`migrate`) çalışamadan. Editörün varlık tarayıcısı da bu dosyayı açamaz.
   Dönüştürücü yazılmadı; yerine güncel biçimde `demo/assets/sample.scene` motorun kendi
   `SceneData::save` yolundan üretildi ve commit'lendi. Bekçi: `scene_round_trip.rs` — kaydedilen
   sahne mesh kaynakları, ışığı, birincil kamerası ve değerleriyle geri geliyor mu.
   **(2026-08-17'de kapatıldı — aşağıya bak.)**

#### Export artık onu paketliyor (2026-08-16)

`cargo build --release -p demo` → `cargo build --release -p demo --bin gizmo_runtime`, ve kopyalanan
dosya `demo` değil `gizmo_runtime`. Çapraz derleme hedefleri dahil: Windows export'unun `demo.exe`
göndermesi aynı kusurun uzantısı değişmiş hâliydi.

**Açık sahne de gidiyor.** Build isteği geldiğinde, iş parçacığı başlamadan önce ana iş
parçacığında canlı dünya geçici bir dosyaya kaydediliyor; cargo başarılı olursa
`export/gizmo_game/scenes/main.scene` olarak kopyalanıyor — runtime'ın argümansız açtığı ad. İki
karar yazılı: **canlı dünya**, çünkü diskteki son kayıt kullanıcının baktığı şey değildir ve eski
bir sahneyi sessizce paketlemek bu yolun düzeltildiği yalanın aynısıdır; **geçici dosya**, çünkü
export kullanıcının kaynak ağacına, hele `scenes/main.scene` üstüne yazmamalı.

Bitiş satırı da artık koşullu: sahne gitmediyse "Oyununuz hazır" değil, "build bitti ama sahne
gitmedi, boş pencere gelir" yazıyor.

Bekçiler (`export_copy_tests`): her hedefin runtime'ı kurup gönderdiği (`--bin` olmadan cargo
demo'nun varsayılanını kurar), export'un yazdığı sahne adının runtime'ın aradığı adla aynı olduğu
(test öteki ucun kaynağını okuyor, tekrar etmiyor), ve staging'in canlı dünyayı proje ağacının
dışına yazdığı.

Uçtan uca doğrulandı: `gizmo_runtime` + `scenes/main.scene`'den ibaret bir dizin kurulup binary
**başka bir çalışma dizininden, argümansız** çalıştırıldı — kendi dizinini bulup sahneyi açtı ve
çizdi.

#### Sözleşme iddia değil, tek kod oldu: `PlayLoop` (2026-08-16)

Son adım "iki yolun davranış eşitliğini test et" diye planlanmıştı. Doğrusu test değildi:
**eşitliği sınamak yerine ortadan kaldırmak.** Koşan bir oyunun karesi artık
`gizmo::systems::PlayLoop::step` — editörün ▶'si de, export edilen oyun da onu çağırıyor. Tek
akümülatör, tek adım boyu, tek script sırası.

İkisinin meşru olarak ayrıldığı tek şey **kimin duyduğu**: bozuk bir script editörde kırmızı bir
konsol satırı, gönderilmiş oyunda stderr satırı. O yüzden raporlama enjekte ediliyor
(`PlayReport`), kararların hiçbiri değil.

Bu, üç kopyayı bire indirdi — ve üçüncüsü testin kendisiydi: stüdyonun testleri koruduğu pump'ın
**elle yazılmış bir aynasını** çalıştırıyordu ("Mirror of the fixed-timestep pump"), yani
korudukları şey değiştiğinde kırılamayan bir test. Aritmetik artık gerçek kodun üstünde
sınanıyor; iki tarafta kalan testler ise düzeni koruyor: `the_play_frame_is_the_shared_step_not_a_copy_of_it`
(stüdyo) ve `the_frame_is_the_shared_play_step_not_a_copy_of_it` (runtime) — birinde
`physics_accumulator` ya da `update_entity` yeniden belirirse kırmızıya düşüyorlar.

Ölçü ve sınırlar yazılı hâlde taşındı: `MAX_STEPS`'in hem kare başına adımı hem **borcu** birden
sınırlaması (yalnız adım sayısını sınırlamak, kırk saniye boyunca hızlı ileri sarma demek) artık
`a_long_stall_does_not_buy_hundreds_of_steps_or_leave_them_owed` ile tutuluyor.

Editörde kalan tek fark bilerek: varsayılan `ActionMap` iskelesi. İki satır altında çağrısı yorum
satırına alınmış bir dövüş sistemi için yazılmıştı, yani koşan bir oyunun parçası değil; bir oyunun
gerçekten ihtiyaç duyduğu tuş eşlemesi sahnede durmalı, o zaman iki yol da alır.

Refactor'dan sonra runtime aynı sahneyi birebir aynı kareyle çiziyor.

#### Ve ortak adım ilk gerçek script'inde bir kusur verdi: bir kare gecikme (2026-08-16, DÜZELTİLDİ)

Sözleşmenin render yarısı doğrulanmıştı, **scripting yarısı hiç** — üstelik depoda tek bir `.lua`
dosyası yok, scripting testlerinin hepsi kaynağını geçici dosyaya kendi yazıyor. İlk gerçek script
koşturulduğunda çıktı:

`entity.set_position` dünyayı yazmaz, **komut kuyruğuna atar**; kuyruğu `flush_commands` uygular.
Adımın sırası ise `update → flush → entity başına on_entity_update` idi. Yani bir entity script'inin
istediği her şey, o karenin flush'ını **kaçırıp** bir sonraki kareyi bekliyordu. Ölçüldü: 1. karede
konumunu (5,0,0) yapan script'in entity'si 1. karenin sonunda hâlâ orijinde, 2. karenin sonunda
yerinde.

Yani motordaki **her** entity script'i bir kare gecikmeli çalışıyordu — editörde ve gönderilen her
oyunda — ve kimse fark etmedi çünkü hareket *oluyordu*, sadece geç.

Düzeltme: entity döngüsünden sonra ikinci bir flush. İlki duruyor (entity hook'ları paylaşılan
pass'in komutlarını görmüş bir dünya okusun diye); boş kuyruğu boşaltmak bir `Vec` takası, yani
komut üretmeyen kare hiçbir şey ödemiyor. Ve düzeltme tek yerde: `PlayLoop` paylaşıldığı için
editör de export edilen oyun da aynı anda düzeldi — refactor'ın ilk temettüsü.

Bekçiler `demo/tests/the_runtime_runs_scripts.rs`: script'in istediği karede indiği, eksik bir
script'in otuz karede bir kez bildirilip kareyi düşürmediği, ve **zincirin tamamı** — sahne dosyası
→ dünya → `Script` bileşeni dosyadan sağ çıkıyor → koşuyor → entity kıpırdıyor. Halkaların her
birinin testi vardı, zincirin yoktu; export'un okunmayan bir binary'yi bunca zaman
paketleyebilmesinin sebebi de tam olarak buydu.

### God fonksiyon taraması (2026-08-15)

Uzunluk tek başına sinyal değil; iç içelik derinliği ve dallanma sayısıyla birlikte ölçüldü. Bölünen
beş yer ve **bölünmeyenlerin gerekçesi** — ikincisi daha önemli, çünkü tekrar tartışılmasın:

- `handle_event` (gizmo-app) 708 → 450, derinlik 10 → 9. Asıl kazanç uzunluk değildi: dört surface
  hata kolunun üçü, `self`'ten çıkarılan altı şeyi geri koyan sekiz satırlık epilogun kendi
  kopyasını taşıyordu. Tek çağıran-tarafı epiloga indi.
- `constraint_solve_step` (gizmo-physics-rigid) 459 → 438, derinlik **10 → 8**. Crate'in en derin
  noktası solver aritmetiği değil, satır içi cevaplanan bir kırılma kontrolüymüş.
- `execute_render_pipeline` (studio) 595 → 541 + editörün kamera kuralları ilk kez testli.
- Kutu-seçim ve reparent: kod bölünmedi, **testleri yazıldı** / çekirdek yardımcıya indirildi.

**Bölünmeyenler ve neden:**

| Fonksiyon | Gerekçe |
|---|---|
| `update_vehicle_with_query` (544) | Ayrılabilir kararlar **zaten ayrılmış**: `ackermann_steering_angle`, `anti_roll_force`, `ground_effect_factor`, `weather_grip_factor`, `apply_force_at_point` — hepsi ayrı fonksiyon ve 19 testli. Kalan kütle 256 satırlık tekerlek döngüsü; dışarıdan ~12 değer okuyor, çıkarmak 12 alanlı bir bağlam struct'ı demek. Fonksiyon kısalır, karmaşıklık kalır. |
| `create_fluid_pipelines` (529) | **0 dal** — bildirimsel wgpu descriptor'u. Çözülecek akış yok. |
| `default_render_pass` (420) | **Derinlik 4** — uzun ama düz; zaten faz dizisi olarak okunuyor. |
| `ui_scene_view` (433) | egui tesisatı; kararları başka yerde çözülüyor (kutu-seçim isteği burada kaydedilip `studio_input`'ta işleniyor — okunarak doğrulandı). |
| `solve_contacts` / `_tgs` / `narrowphase_*` | Sıcak yol. Belgeler belirli *optimizasyonları* ölçüp reddetmiş; yapı hakkında bir şey söylemiyor. Bölünecekse şartı `headless_stress_test` hash'inin değişmemesi — `collect_fracture_events` bunun yapılabildiğini gösterdi. |

**Ölçüt olarak ayakta kalan tek şey:** uzunluğun sebebi **düğüm mü** (iç içe durum + dallanma → böl)
yoksa **dizi/bildirim mi** (→ bırak). Yol boyunca kullanılıp çürütülen üç sahte ölçüt: "iyi test
edilmiş" (kapsam bölmeyi *güvenli* kılar, gereksiz değil), "GPU'ya bağlı" (test edilebilirlik
hakkında, bölünebilirlik hakkında değil), "bilinçli yoğun" (perf kararı, yapı kararı değil).

### Bir daha kovalanmasın

- Animasyonun zamanlanmamasının sebebi **imza değildi** — studio onu tam o imzayla zaten çağırıyordu.
- Süpürme sayısını kesmek yakınsamayı iyileştirmez: 9 kat az süpürme %17 kazandırıyor ve tavanı o.
- Varyans kırpması TAA kalıntısını düzeltmiyor; iki kez ölçüldü, ikisinde de biraz kötüleşti.
- **`gizmo-animation` Stage A'ya uygun değil** diye kaydedilmişti; bağımlılıkları ölçüldü, uygun.
  Bayat olan diyagramdı.
- **Sonda köprüsü `send` yüzünden imkânsız** diye kaydedilmişti; `Scope::create_function`'ın
  imzasına bakılmamıştı, o sınırı taşımıyor.
- **ECS-only rollback dört düşen kutuda ıraksıyor** diye ölçmüştüm; ıraksamayı üreten şey testteki
  fazladan bir adımdı. O senaryo iki uygulamayı ayırt edemiyor; ayırt eden şey pencere içinde
  kopan bir eklem.


### Editör viewport'u: "çirkin ve kalitesiz" neydi (2026-08-15)

Kullanıcı editörü açıp *"çok çirkin ve kalitesiz görünüyo"* dedi ve dört belirtiyi birden
işaretledi: bulanık, tırtıklı, ışık düz, renkler yıkanmış. Dördü birden tek bir üst-akım sebebe
işaret ediyordu ve öyleydi.

**Kök: editörün 3B görüntüsü ekrana bir sRGB kodlaması eksik düşüyordu.** egui'nin shader'ı
sözleşmesini kendi yorumunda yazıyor — *"We expect 'normal' textures that are NOT sRGB-aware"* —
ama her iki viewport RTT'si de `config.format` ile, yani sRGB olarak yaratılıyordu. İki çözme,
bir kodlama. Ayrıntı ve düzeltme `editor_runtime::create_viewport_target`'ta.

Bu kökü bulmanın yolu, tahmin listesini eleyip **ölçmek** oldu; sırayla düşen hipotezler:

| Hipotez | Nasıl düştü |
|---|---|
| Bugünkü DoF kalibrasyonu | Hesap: hiperbolik derinlikte 20 m'de fark %1. Görünür değil. |
| PBR paketleme değişikliği | Forward shader `inst_pbr.w`'yi hiç okumuyor. |
| `..Default::default()` bloom'u düşürdü | Devredilen alanların hepsi aynı değere sahip. |
| Ortak ışık kurulumuna geçiş | Uniform'a giden değerler bastırıldı: güneş yön/renk/yoğunluk doğru. |
| Gölge haritası her yeri gölgede sanıyor | Gölge araması devre dışı bırakıldı, piksel bire bir aynı. |
| Mesh hiç çizilmiyor | Batch'ler bastırıldı: küp 24 vert/36 index ile lit batch'te. |
| FXAA | Kapatıldı, piksel değişmedi. |

Ayırt eden ölçüm, post zincirine **bilinen bir sabit** basmak oldu: composite'e lineer 0.5
yazdırıldı, ekranda 188 yerine 128 çıktı. Bir gamma adımı, tam olarak. Kalibrasyon için egui
panel zemini kullanıldı (28 ölçüldü, egui koyu temasının 27'si) — yüzey kodlamasının doğru
olduğunu, kaybın yalnızca viewport yolunda olduğunu bu gösterdi.

**Bunu mümkün kılan şey yeni `gizmo_renderer::capture`.** Bu makinede dışarıdan ekran yakalama
çalışmıyor (Xwayland rootless → X kökünde içerik yok, `import` da ffmpeg de siyah döndürür), ve
bu değişmeyecek. Kare artık GPU'dan geri okunuyor. Motorun kendi çıktısına bakabilmesi teşhis
süresini "kullanıcıya sor" döngüsünden ~40 saniyelik bir deneye indirdi.

Sahne içeriği hakkında, kusur olmayan iki gözlem: studio'nun varsayılan sahnesinde gökyüzü/zemin
yok (`setup.rs`'teki "Custom Skybox or proper horizon color" yorumu kalıntı) ve Default Cube'ün
materyali kasten %21 gri. Karanlık görünmesinin bir kısmı buradan; ikisi de bilinçli seçim
olabilir, o yüzden dokunulmadı.

#### Denendi ve reddedildi: editör viewport'una prosedürel gökyüzü

Gamma düzeltmesinden sonra geriye kalan "sahne boş bir karanlık" hissini gidermek için skybox
denendi. `sky.wgsl` zaten tam bir atmosfer çiziyor (zenit/ufuk/zemin gradyanı, güneş halesi ve
diski) ve `sky_pipeline` kurulu — studio'nun hiç skybox varlığı yaratmaması yüzünden editörde hiç
çalışmamış. Bir varlık eklemek yetiyor; `gizmo_root` altında, grid ile aynı krom statüsünde.

**Sonuç ölçüldü ve daha kötü:** gökyüzü açıldığında grid tamamen kayboluyor. Grid materyali açık
renkli ve alfa-harmanlı; koyu arka plan için tasarlanmış. Ufuk rengi (0.5, 0.7, 0.9) sahnenin
güneş yoğunluğuyla çarpıldığı için (studio'da 1.5) beyaza kırpılıyor; yoğunluk 1.0'a çekilince de
viewport 182–211 aralığında düz bir soluk yıkama oluyor ve grid yine görünmüyor. Kamera pitch'i
-23°, FOV 60° — görünen alanın neredeyse tamamı en parlak ufuk bandı.

Gökyüzünü editörde kullanılabilir kılmak grid ve gizmo renklerinin de yeniden tasarlanmasını
gerektirir. Blender ve Unity'nin editör arka planını nötr-koyu tutmasının sebebi bu. Kayıt için:
skybox varlığını eklemek 20 satır, sorun orada değil.

### Editör hattının ilk piksel testleri, ve bulduğu kusur (2026-08-15)

Motorun deferred yolunda on yedi golden piksel testi var; editörün 600 satırlık forward yolunda
hiç yoktu. `render_parity.rs` ortak *kurulumu* örtüyor (kurulum saf fonksiyon), ama pass kaydı
gözlemsizdi. Bedeli bu oturumda görüldü: bir aylık açılış çökmesi ve aylarca süren gamma hatası,
ikisi de ekran görüntüsünde apaçık, ikisi de piksele bakmayan bir test paketine görünmez.

`tests/studio_render_pixels.rs` bu boşluğu kapatıyor: headless renderer, gerçek
`execute_render_pipeline`, geri okuma. Kaba iddialar bilinçli — golden görüntü editörün *görünüşünü*
sabitler, oysa görünüş değişmeli; testler yalnızca kaybı "bozuk" demek olan özellikleri tutuyor
(hiçbir şey çizilmemiş, her şey tek renk, aydınlatılan nesne arkasındaki boşluktan ayırt edilemez).

**Kurulur kurulmaz bir kusur buldu.** Motor yolu `ensure_global_transforms` ile `Transform`'u olup
`GlobalTransform`'u olmayan mesh'lere bileşeni ekliyor — kendi yorumunda "empty screen footgun".
Editör yolunda bu adım yoktu; studio'nun koşturduğu sync/propagate sistemleri mevcut bileşeni
günceller, olmayanı eklemez. Yani `spawn((Transform, Mesh, Material))` oyunda çiziliyor, editörde
sessizce çizilmiyordu (ölçüm: küp merkezi 44.0, arka plan 34.0 — saf arka plan). Farkın görünür
bedeli `setup.rs`'te duruyordu: dokuz varlığa elle eklenmiş `GlobalTransform::default()`.

İki kayıt notu daha çürüdü:

- *"Bu testler için önce `StudioState` headless kurulabilir olmalı."* Değilmiş: struct skalarlardan
  ve bir `Option`'dan ibaret, hat da üç opsiyonel kaynağa bakıyor. İlk denemede koştu. Bu, ölçülmeden
  kaydedilmiş engellerin bu kod tabanındaki **üçüncüsü** ve üçü de temasta dağıldı (öncekiler:
  `Scope::create_function`'ın `Send` sınırı, animasyonun imzası).
- Testin ilk hâli kırmızıydı ve suç renderer'da değildi: küpe tam cepheden bakan kamera tek yüz
  gösteriyor, tek normal tek gölge veriyor — 153.6..153.6, doğru çıktı, ama "aydınlatma ulaşmıyor"
  ile birebir aynı görünüyor. Küp döndürüldü; gerekçe testin içinde, yoksa biri "gereksiz" diye
  siler.

#### Editör hattında taranan öteki özellikler (2026-08-15)

Piksel koşumu kurulunca aynı mercek motorun golden testlerinin kapsadığı öteki özelliklere
tutuldu. Sonuçlar, negatifler dahil:

| Özellik | Sonuç |
|---|---|
| Instance kapasitesi | **Sorun yok.** Studio `ensure_instance_capacity` çağırıyor; pass'lerdeki beş `instance_capacity` kırpması bu yüzden ölü savunma, mesh düşmüyor. |
| Gölge dökme | **Çalışıyor**, artık testli. Bkz. `the_editor_casts_a_shadow_onto_the_ground`. |
| `GlobalTransform` doldurma | **Kusurluydu**, düzeltildi (yukarıdaki bölüm). |
| SSAO onay kutusu + şiddet kaydırıcısı | **Ölüydü**, kontrol kapatıldı. |
| Inspector'daki post kaydırıcıları (bloom, grain, exposure, vignette, aberration, DoF) | Hepsi `post_params`'a akıyor, canlı. |

Gölge yolu az kalsın yanlışlıkla "ölü" diye kaydediliyordu: forward shader'da `shadow_visibility`
1.0'a zorlandığında örneklenen piksel sıfır değişmişti. Sebep gölgenin çalışmaması değil, örnek
noktanın aydınlık bir yüzde olmasıydı. Tek pikselden çıkarılan olumsuz sonuç, sahne o soruyu
soracak biçimde kurulmadıkça hiçbir şey kanıtlamıyor.

#### Game paneli editör kamerasını gösteriyordu (2026-08-15, KAPANDI)

Studio karede **tek** sahne çizimi yapıp iki çıktı üretiyor: `run_post_processing` önce editör
hedefine, sonra oyun hedefine yazıyor, ikisi de aynı `renderer.post.hdr_texture_view`'i okuyarak.
O doku edit modunda editör kamerasından çizilmiş oluyor — gizmo'lar, grid ve "oyun kamerası burayı
görüyor" tel kutusu dahil. Yani Game sekmesi Scene sekmesinin kopyası, hem de tam işe yarayacağı
anda. Play modunda iki kamera aynı olduğu için soru doğmuyor.

Ölçüldü: iki kamera zıt yönlere bakarken game hedefi sahne hedefiyle bayt bayt aynı (65536 baytın
0'ı farklı). Kayıt prosa değil, `#[ignore]`'lu bir teste kondu —
`the_game_view_shows_the_game_camera_not_the_editor_camera` — çünkü çalıştırılabilir bir kayıt
bayatlayamaz ve düzeltildiği gün kendiliğinden yeşile döner.

**Düzeltildi.** Üç parça: `EditorOnly` işaretleyici bileşeni (gizmo-core'a değil
`gizmo-renderer`'a — ECS tabanı Stage A'nın en alt yüzeyi ve bu bir render kaygısı, §4;
`EditorRenderTarget` zaten orada), ana geçişe `draw_chrome` bayrağı, ve oyun kamerasından ikinci
bir çizim. İşaretleyici batch anahtarına da girdi: bir ikonla aynı batch'e düşen sahne mesh'i aksi
hâlde oyun görüntüsünden onunla birlikte silinirdi.

**Ve buradaki tuzak kaydedilmeye değer.** İlk deneme "kare ortasında uniform'u yeniden yaz, ikinci
geçişi aynı encoder'a kaydet" idi ve çalışmadı — üstelik *hiçbir şey olmamış gibi* görünen bir
başarısızlıkla: `Queue::write_buffer` komutlarla değil **submit'lerle** sıralı. Bir submit'ten
önceki her yazım o submit'teki bütün geçişler için geçerli, dolayısıyla ikinci yazım ilk çizimi de
değiştirdi ve iki panel yine aynı görüntüyü verdi. Doğrusu bir submit sınırı: oyun görünümü kendi
encoder'ında çizilip, editörün uniform'ları geri yazılmadan önce submit ediliyor. Bu sayede ikinci
bir uniform tamponu ya da bind group çoğaltması gerekmedi.

Cascade'ler oyun kamerasına yeniden oturtuluyor — shader cascade'i görüş derinliğinden seçtiği için
split'ler bakan kameraya ait olmak zorunda. Bedel ölçüldü: 481 → 453 FPS (~%6); culling ve instance
tamponu paylaşıldığı için iki kez ödenen şey yalnızca pass kaydı ve rasterizasyon. Görünürlük
kapısı bilinçli olarak konmadı: editör durumunda panelin görünür olup olmadığını bildiren bir şey
yok ve canlı beklenen bir önizlemede %6 için görünürlük protokolü icat etmek yanlış takas.

Bu iş sırasında yazdığım bir yan kazanç iddiası **yanlıştı** ve düzeltildi: "ışık ikonları artık
play modunda da sızmıyor" demiştim; `systems/gizmos.rs` play modunda o nesnelere zaten `IsHidden`
ekliyormuş. İşaretleyicinin gerçek kazancı edit modundaki Game görünümü.


### "Editör nesnesi mi" kararı: sekiz kopyadan bire (2026-08-15)

Game view işi bir işaretleyici bileşen gerektirdi, ve onu ararken kuralın zaten var olduğu ortaya
çıktı — isim öneki olarak, `starts_with("Editor ") || == "Highlight Box"`, **sekiz** yerde ayrı
ayrı yazılmış: hiyerarşi paneli (iki kez), `gizmo-app`'in editör runtime'ı, `gizmo-scene`'in
snapshot filtreleri (iki) ve sahne yazıcısı, studio'nun korunan-nesne kümesi, silme koruması,
tümünü-seç kısayolu, play-modu gizlemesi. Sekizinin de o boşluk karakteri üzerinde anlaşması
gerekiyordu.

Karar artık `gizmo_core::component::is_editor_only`; yanında `EditorOnly` bileşeni. İsim kuralı
korunuyor çünkü bileşenden önce yazılmış sahneler yalnızca isim taşıyor — geçiş, tasarım değil.

**Bileşenin yeri ölçüldü, tercih edilmedi.** Önce `gizmo-renderer`'a konmuştu ("editör kavramı ECS
tabanına ait değil"). Tüketicileri sayınca yanlış olduğu görüldü: `gizmo-scene` de bu kararı
veriyor ve renderer'ı göremiyor — grafikte yanında duruyor, üstünde değil. Yani kavram bir render
kaygısı değil, bir *dünya* kavramı: "bu nesne alet, içerik değil". Çekirdek zaten `IsHidden` ve
`IsDeleted`'ı barındırıyor.

İsim kuralının bilinen yarası testte belge olarak duruyor: sahnesinde "Editor Desk" adlı masası
olan kullanıcı onu hiyerarşide göremez ve hiçbir kayıtta bulamaz. İşaretleyici tam da bunun için
var.

Tarayıcı test yalnızca **üretim kodunu** okuyor: bir testin "bu filtrelendi" derken dizeyi anması
sonucu denetlemektir, kararı yeniden vermek değil — `gizmo-scene`'in kayıt testi tam olarak bunu
yapıyor ve haklı.
### Editör kontrolleri taraması: beş şüpheli, ikisi yanlış teşhis (2026-08-15)

SSAO bulgusundan sonra arayüzün yazdığı **30 alanın** hepsi tarandı: her birinin gerçek bir
tüketicisi var mı? Beşi şüpheli çıktı. Paralel bir inceleme koşturuldu ve dört soruşturmacıdan
biri **iddiayı çürütmekle** görevlendirildi. İyi ki öyle yapıldı:

| Kontrol | İlk teşhis | Gerçek |
|---|---|---|
| `snap_translate` | ölü | **YANLIŞ — canlı.** `scene_view.rs:204` varlık sürükle-bırakta okuyup yuvarlıyor. |
| `snap_rotate_deg` | ölü | **YANLIŞ — atıl.** Config'e geçiyor, `config.snapping` kapalı olduğu için kütüphane okumuyor. |
| `show_grid` | ölü | Doğru. |
| `snap_scale` | ölü | Doğru (config'e hiç geçmiyordu). |
| `gizmo_size` | ölü | Doğru. |

**Denetimin kusuru neydi:** okuyucuları sayarken `gizmo-editor`'ın kendi dosyaları elenmişti
("arayüzün kendisi yazıyor" varsayımıyla), oysa `scene_view.rs` o crate'in içinde ve gerçek bir
tüketici. Filtre, aradığı şeyi tanımıyla dışarıda bırakıyordu. Bu, tarama testleri yazarken tekrar
edilebilecek bir hata: kapsam dışı bırakılan yer, aranan şeyin yaşadığı yer olabilir.

`snapping` alanı ise tek satırlık ama üç ayarı birden ölü gösteren cinsten: `transform-gizmo`
`snap_distance`/`snap_angle`/`snap_scale`'i **yalnız** `if config.snapping` içinde okuyor, o alan
hiç atanmamıştı, ve `..Default::default()` `false` veriyordu. Ctrl modifier'ı bile yıllardır
kuruluymuş ve kimse fark etmemiş — çünkü hepsi hesaplanıp atılıyordu.

`gizmo_size` uçtan uca **görsel olarak** doğrulandı: küp geçici olarak seçili hâle getirilip
75 ve 220 değerleriyle iki kare alındı; tutamaklar belirgin biçimde büyüdü. Piksel testi yazılmadı,
çünkü transform gizmo'su yalnız bir seçim varken çiziliyor ve seçim egui girdisiyle kuruluyor —
headless koşumda simüle edilemez. Geçici düzenlemeler geri alındı.

Grid anahtarının testi iki şey birden tutuyor: fark sayısı **ve yönü**. Yalnız "kareler farklı"
denseydi bayrak ters dönse de test geçerdi. Snapping'de piksel testi mümkün değil (egui sürüklemesi
gerekir), o yüzden karar `snap_active` saf fonksiyonuna çıkarıldı; XOR'un asıl sınanmaya değer
satırı ikincisi: tercih açıkken Ctrl snapping'i **askıya alır**. Oraya `||` yazmak tuşu zamanın
yarısında işlevsiz bırakır ve hiçbir şey fark etmez.
