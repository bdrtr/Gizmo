# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Gizmo Engine — a pure-Rust, ECS-driven 3D game engine and physics simulator with **zero external physics dependencies**. Cargo workspace of ~20 crates at uniform version `0.10.0`, published to crates.io (the facade crate `crates/gizmo` is published as **`gizmo-engine`**). Aimed at large-scale, deterministic physics simulation + a WGPU renderer that also runs in the browser via WASM/WebGPU.

## Build, test, run

```bash
# Build / test the whole workspace (default features)
cargo build --workspace
cargo test --workspace --no-fail-fast   # ALWAYS --no-fail-fast — see below

# **`--no-fail-fast` is not optional here.** Cargo stops launching further test binaries after the
# first one fails, so ONE red hides everything behind it. Measured 2026-08-19, while this tree had
# a standing red (`gizmo-studio`'s icon test, which reads `media/logo.png` and fails whenever that
# file is not square): a plain `cargo test --workspace` ran **130** test binaries and
# `--no-fail-fast` ran **159** — the known red was hiding 29 of them, including every
# `gizmo-studio` integration test (`render_parity`, `studio_render_pixels`, …). That red is closed
# (2026-08-20, the logo is square again) and the rule outlives it: a gate that stops at a failure
# you have decided to ignore is a gate that covers less every time you ignore it. The whole suite
# is 159/159 green — if you see 130, you are looking at a failure, not at a shorter suite.

# Run a single test by name substring
cargo test --workspace soak_resting_stacks_stay_bounded
cargo test -p gizmo-physics-rigid <test_name>

# Feature-gated tests that CI runs separately (not covered by --workspace defaults):
cargo test -p gizmo-net --features client-server,rollback
cargo test -p gizmo-core -p gizmo-physics-core -p gizmo-physics-rigid -p gizmo-scene --features reflect
cargo test -p gizmo-core --features tracing-layer   # the tracing_subscriber bridge, off by default
cargo test -p gizmo-physics-rigid --features experimental-multibody

# Demos live in demo/src/bin/ (39 of them). ALWAYS use --release for physics demos —
# a debug build hits a severe broad/narrow-phase CPU bottleneck.
cargo run --release -p demo                      # default: 3d_scene (PBR + physics)
cargo run --release -p demo --bin car_demo
cargo run --release -p demo --bin advanced_physics

# Determinism / stability gate — 200-box tower collapse, 3 runs, hashes must match
cargo run --release -p demo --bin headless_stress_test

# Look at what the engine actually drew — any windowed binary, no display grab needed.
# (External screen capture does NOT work here: Xwayland runs rootless, so the X root window is
# empty and every `import`/ffmpeg grab comes back black. This reads the frame back from the GPU.)
GIZMO_SCREENSHOT=/tmp/frame.png GIZMO_SCREENSHOT_FRAME=180 GIZMO_SCREENSHOT_EXIT=1 \
  env -u WAYLAND_DISPLAY DISPLAY=:1 ./target/release/gizmo-studio
# FRAME defaults to 90 (early frames are unrepresentative: assets stream in, layout settles).
#
# GOOD FOR: is the frame black, does the camera point where you think, is the geometry/ground/
# debug overlay there, did a shader change do what you meant.
#
# NOT A BEFORE/AFTER DIFF on a physics demo, and the numbers are worth knowing before you try:
# a windowed demo steps physics with the REAL frame delta, so "frame 180" is a different amount
# of SIMULATED time on every run. Measured 2026-08-18, the same binary rendering the same frame
# number three times: 6.17 %, 6.49 % and 4.77 % of bytes differed between runs. A code change
# measured this way showed 7.39 % — indistinguishable from doing nothing at all.
# For a deterministic comparison use a fixed-timestep driver and compare a hash, which is what
# `headless_stress_test` is.

# Benchmarks (criterion). CI runs each one once with `--test` as a smoke gate — it catches a
# panicking or non-compiling bench, not a regression (see docs/ENGINE.md on why there is no
# timing gate). Four crates have them, and the claim "only gizmo-math and gizmo-core" was stale
# for at least two of them until 2026-08-18:
#   gizmo-core 6 · gizmo-math 5 · gizmo-physics-rigid 2 · gizmo-renderer 2
cargo bench --workspace --benches -- --test    # smoke: runs each bench once, catches runtime panics
cargo bench -p gizmo-core                       # real timing run
```

### Lint (must pass — this is the exact CI gate)

```bash
cargo clippy --workspace --all-features --all-targets -- -D warnings \
  -A clippy::too_many_arguments -A clippy::type_complexity
```

`-D warnings` is a real gate. The two `-A` exemptions are grandfathered architectural lints (a **ratchet**: new lint kinds break CI; the exempt list only shrinks). `rustfmt` is checked **report-only** in CI — the tree is not yet fully fmt-clean, so a `cargo fmt` diff will not fail CI, but don't reflow unrelated code.

```bash
# Feature-pair gate (CI job "Feature powerset"). RUN THIS after touching any `#[cfg(feature)]`,
# any module declaration, or any `pub use` — it is the ONLY gate that catches a `#[cfg]` that got
# detached from its item, and an insertion anchored on a `pub mod` line steals the attributes above
# it. That mistake reached CI twice on 2026-08-17 (see docs/ENGINE.md §8).
#
# It runs CLIPPY, not `check` (since 2026-08-18): the `--all-features` lint job cannot see an arm
# that a smaller feature set removes, so 66 of the facade's 150 combinations were warning with
# nothing looking. Add `--keep-going` when fixing: without it the run stops at the first failing
# combination and tells you nothing about the size of what you are fixing.
# Slow (~1 min per crate warm, longer cold) — run it in the background, not in a foreground shell.
cargo hack clippy -p gizmo-app --feature-powerset --depth 2 --no-dev-deps \
  -- -D warnings -A clippy::too_many_arguments -A clippy::type_complexity
cargo hack clippy -p gizmo-engine --feature-powerset --depth 2 --no-dev-deps \
  -- -D warnings -A clippy::too_many_arguments -A clippy::type_complexity
```

> Gotchas when reproducing CI locally: the entry crate is **`gizmo-engine`**, not `-p gizmo`. Piping cargo through `| tail` masks the exit code — check exit status separately.
>
> **`cargo hack --no-dev-deps` rewrites the manifests in place while it runs**, and restores them
> when it finishes. So a second cargo command started in parallel — in another terminal, another
> agent, a background job — compiles against a tree whose `[dev-dependencies]` have been deleted,
> and fails with an error that describes nothing real (`cannot find crate serde_json`, in a crate
> whose manifest plainly has it). Run the powerset gate alone, and if a build fails with a
> missing dev-dependency, check whether a powerset run is in flight before believing it.

### WASM (browser) build

```bash
cargo build -p demo-web --target wasm32-unknown-unknown --release
# then wasm-bindgen (CLI version must EXACTLY match the resolved wasm-bindgen crate — it is
# version-locked) + `python3 -m http.server -d demo-web 8080`. See demo-web/README.md.

# The wasm target is LINTED too (CI gate, added 2026-08-15). It is a separate gate because which
# code exists is per-target: every `#[cfg(not(target_arch = "x86_64"))]` arm and every wasm-only
# branch is invisible to the native clippy job, and two real violations were living there.
cargo clippy --target wasm32-unknown-unknown -p demo-web \
  -- -D warnings -A clippy::too_many_arguments -A clippy::type_complexity

# `gizmo-editor` is linted for wasm too (added 2026-08-16) even though the browser build does not
# ship an editor: the crate declares wasm intent (target-gated `gizmo-scripting`, `web-time`,
# cfg'd items), nothing ever compiled that arm, and it had stopped building for wasm entirely.
cargo clippy --target wasm32-unknown-unknown -p gizmo-editor \
  -- -D warnings -A clippy::too_many_arguments -A clippy::type_complexity
```

> **The wasm gate is per-crate, and checking a subset is not checking it.** CI builds
> `-p gizmo-renderer` and `-p gizmo-app` on their own, not only through `-p demo-web`. Feature
> unification makes that a real difference: `demo-web`'s graph can enable a feature (getrandom's
> `wasm_js`, say) that the same crate built alone does not get, so a local check on `demo-web`
> passes while CI's standalone build fails with `compile_error!`. That happened on 2026-08-16.
> To reproduce the gate, run the crate list from `.github/workflows/ci.yml`'s `wasm` job, not a
> convenient subset of it.

### Testing the editor UI without a window or a GPU

`egui::Context::run_ui` drives a **real** editor frame headlessly — no window, no wgpu, no display.
That makes UI facts assertable instead of screenshot-guessable, and it is fast enough for ordinary
unit tests — measured here, a module driving several full editor frames finishes in 0.04–0.22 s,
most of which is egui building its font atlas once:

```rust
let ctx = egui::Context::default();
let output = ctx.run_ui(egui::RawInput::default(), |ui| {
    draw_editor(ui, &world, &mut state);       // or one panel: ui_console(ui, &mut state)
});
// …inspect `output.shapes`…
output.drop_without_applying_deltas();          // REQUIRED: TexturesDelta panics if just dropped
```

`output.shapes` is what the frame actually painted, and it answers the questions screenshots
can't:

- **Was a texture painted?** `Shape::Mesh(m) => m.texture_id == id` — used to prove the Game panel
  displays its render target (`the_game_panel_paints_the_texture_it_was_given`).
- **What colour was that row?** `Shape::Text(t)` → `t.override_text_color` or the first section's
  `format.color` (`the_console_paints_warnings_and_errors_in_those_colours`).
- **How wide is the content really?** `shape.visual_bounding_rect().max.x` against the panel width.
  Set `ui.set_clip_rect(Rect::EVERYTHING)` first — clipping is what *hides* overflow on screen, so
  measuring unclipped is what turns "looks bitten off" into a number (`inspector_width_tests`).
- **What text is on screen?** collect `Shape::Text` and read `t.galley.text()`
  (`hierarchy_count_tests` reads the header's digits back out).

`Shape::Vec` nests, so every scan needs to recurse or it silently misses most of the frame.

Two things to know when building the state for such a test:

- `EditorState::default()` calls `EditorPrefs::load()` **and** `load_layout()`, which read
  `editor_prefs.toml` and `editor_layout.json` from the config dir and the working directory. A
  test that cares about the dock should overwrite `state.dock_state` with
  `editor_state::create_default_dock_state()` rather than inherit whatever is on disk; a test that
  cares about prefs I/O should use the `*_to(path)` variants instead of pointing `XDG_CONFIG_HOME`
  somewhere (that is process-global, and other tests in the same binary read it in parallel).
- To choose which viewport is on top: `dock_state.find_tab(&tab)` then `set_active_tab(..)`.

### Shader hot-reload (works, and is not discoverable)

Every renderer shader is compiled in with `include_str!`, but each one first tries to read a
**disk override** and falls back to the embedded copy:

```rust
let source = std::fs::read_to_string("demo/assets/shaders/post_process.wgsl")
    .unwrap_or_else(|_| include_str!("shaders/post_process.wgsl").to_string());
```

So: copy a shader out of `crates/gizmo-renderer/src/shaders/` into `demo/assets/shaders/`, edit it,
and the studio recompiles the pipelines while it runs — the studio watches `demo/assets`
recursively and calls `Renderer::rebuild_shaders()` on any `.wgsl` change. Verified end to end on
2026-08-16 by tinting `post_process.wgsl` mid-run and watching the frame turn green.

`demo/assets/shaders/` is deliberately **not** in the repository. Committing copies there would
mean two versions of every shader free to drift, with the disk one silently winning.

## Environment / machine constraints

`.cargo/config.toml` caps `jobs = 4` and sets `codegen-units=4, lto=off` — this dev machine has limited RAM (~13 GB); each rustc uses 1–2 GB, so unbounded parallelism OOMs. `[profile.dev]` uses `debug = "line-tables-only"` + `split-debuginfo = "unpacked"` (demo binaries statically link all of wgpu/egui; full DWARF blew `target/` past 600 GB). These affect debug info / build only — **runtime perf is unchanged**. Don't "fix" these settings.

## Architecture

Clean bottom-up layering, **no circular dependencies**:

```
gizmo-math ─┬─ gizmo-core ─┬─ gizmo-physics-{core,rigid,dynamics,soft}
            │              ├─ gizmo-audio, gizmo-animation
            │              ├─ gizmo-{scene,net,ai}       (over physics-{core,rigid})
            │              ├─ gizmo-renderer             (over gizmo-animation)
            │              ├─ gizmo-scripting            (over ai + animation + physics-rigid)
            │              └─ gizmo-editor               (over renderer + scene + scripting + ai)
            └── gizmo-app (over renderer/editor/scene/scripting/audio/net/physics)
                  ├─ gizmo-ui, gizmo-analysis
                  └─ gizmo (facade, published as `gizmo-engine`) ─ demo / cradle / studio
gizmo-window: standalone, no in-workspace dependencies.
```
<!-- Measured from the manifests, not drawn from memory: `gizmo-ui` sits ABOVE `gizmo-app`
     (it depends on it), `gizmo-window` depends on nothing of ours, and `gizmo-animation` is a
     dependency of both the renderer and scripting rather than a leaf beside them. The invariants
     this shape has to keep are tested in `crates/gizmo/tests/crate_staging.rs`. -->

- **`gizmo-math`** — vector/quat math (re-exports `glam`; also has an experimental Q16.16 `Fp32` the sim does *not* use).
- **`gizmo-core`** — archetype-based ECS: `World`, `Query`/`query_mut`, `With`/`Without`/`Changed`/`Added` filters, `Commands` (deferred structural changes), `Res`/`ResMut`, Table + SparseSet storage, scheduling, events, hierarchy, input. Component access is borrow-checked — aliasing `&mut` views can't be built in safe code.
- **`gizmo-physics-{core,rigid,dynamics,soft}`** — render-agnostic, embeddable physics. BVH/SAP broadphase (Rayon), GJK/EPA narrowphase, TGS-Soft sequential-impulse solver, soft-body FEM/cloth/rope, fracture, joints, vehicle/character dynamics, CCD.
- **`gizmo-renderer`** — WGPU deferred PBR: CSM shadows, SSAO/SSGI, bloom, volumetric, TAA. Reduced forward-only pipeline on WASM (no shadows, 4 bind groups).
- **`gizmo-app`** — plugin-driven app loop + phase executor. Windowed loop uses winit 0.30 `ApplicationHandler` (`crates/gizmo-app/src/windowed/`).
- **`gizmo`** (facade, crate `gizmo-engine`) — `gizmo::prelude::*`, the high-level `SimpleApp`/`App<S>` API. Feature flags gate every subsystem (`render`, `audio`, `physics`, `physics-soft`, `editor`, `scene`, `scripting`, `network`, `egui`, `analysis`, …); `headless` = physics + net, no window/render. Networking and Lua scripting are **native-only** (not on WASM). **Audio is not** — this line said it was until 2026-08-20: `gizmo-audio` carries a `cfg(target_arch = "wasm32")` dependency block and wasm-specific `Send`/`Sync` impls, `demo-web` enables the `audio` feature, and its left-click beep plays through the engine's own `AudioManager` (CI's `wasm` job names audio in the web feature subset it builds).

## Determinism contract (important)

The simulation state (Transform/Velocity/solver) runs entirely on **glam/f32**. Guarantee is **same-platform** replay + rollback bit-equality, verified via `state_hash` and cross-process tests. Cross-platform bit-exact determinism is explicitly **out of scope**. When you change physics: any bit-level change must be intentional — the `headless_stress_test` and `soak_*` regression tests exist to catch unintended drift. Historical hashes in docs/comments are point-in-time and have been superseded.

## Working conventions (from docs/ENGINE.md §8)

- Each change: **fix → write a regression test → build/test/clippy → done.** Verify behavior-changing physics fixes with `headless_stress_test` + focused scenarios; choose a soak-test horizon *past* the onset of instability (a too-short soak once shipped green while hiding an explosion at frame ~853).
- On bug-hunt sweeps: fan out subagents, then **verify each finding by hand** — this codebase has a documented history of false positives (see ENGINE.md §7 for the list of already-refuted "bugs" — don't re-chase them).
- Known accepted non-goals: narrowphase batch-SIMD (measured ~3% of frame, rejected). **The N≥48 tower non-goal was retired 2026-08-17** — `soak_extreme_tower_n48_stays_bounded` is un-ignored and green; it, N1's ground-size sensitivity and N2's 2 cm-gap collapse were all closed on 2026-08-06 by `be46e01` (the narrowphase depth test's missing tolerance → one-point manifolds at exact contact), and the docs simply hadn't re-run the measurement. See ENGINE.md §7.

## Docs & conventions

- **`docs/ENGINE.md`** is the single internal engineering doc: architecture, live roadmap, the public-surface contract (§4), determinism/migration contracts, closed research. **Written in English** (translated 2026-08-06 — the bus-factor item D2); many inline code comments are still Turkish. `README.md` = user-facing intro, `CHANGELOG.md` = version history.
- Public API hygiene is continuous — not a 1.0 campaign (§3, restated 2026-08-17): `#[non_exhaustive]` is the default for a public type here — **no count is kept, deliberately**: this line said 96, then 122, and both were wrong when written (101 and 127 at the commits that recorded them), and the figure has since gone *down* rather than climbed. Count it if you need it, don't quote it — and grep the attribute, not the word, which overshoots by ~40 because the types document it. Errors are enums + `Result`. **docs/ENGINE.md §4 is the authority on what may appear on a Stage A public surface — read it before adding a dependency type to any `pub` signature, field, associated type or trait impl.** In short: `glam` is a deliberate permanent public dep; `bevy_reflect` is behind the default-off `reflect` feature and `tracing-subscriber` behind the default-off `tracing-layer` feature (`tracing` 0.1 itself is unconditional — it is frozen and leaves no type in our signatures); `crossbeam-queue` is sealed out of `gizmo-core` by the opaque `asset::AssetDropQueue`, `arrayvec` out of `gizmo-physics-core` by `ContactPoints` + `collision::ContactPointsIter`, and `ron`/`web-time` out of `gizmo-scene` by `error::{ParseError, SerializeError}` + a private `SceneSnapshot::timestamp` (no `pub use ron;`, no `gizmo::ron`); `rustc-hash` is sealed out of `gizmo-physics-rigid` by the opaque `world::EntityIndexMap` (that crate's last unsealed leak, cleared 2026-08-09 — the field stays `pub` and readable, but it is no longer a `HashMap`); `wgpu`/`winit`/`egui` leak intentionally during 0.x. **Known unsealed:** nothing on a Stage A default surface except the deliberate `glam` and `serde`/`serde_json` entries, both accepted in §4 — check there before adding a fifth.
