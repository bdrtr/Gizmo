# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Gizmo Engine — a pure-Rust, ECS-driven 3D game engine and physics simulator with **zero external physics dependencies**. Cargo workspace of ~20 crates at uniform version `0.10.0`, published to crates.io (the facade crate `crates/gizmo` is published as **`gizmo-engine`**). Aimed at large-scale, deterministic physics simulation + a WGPU renderer that also runs in the browser via WASM/WebGPU.

## Build, test, run

```bash
# Build / test the whole workspace (default features)
cargo build --workspace
cargo test --workspace

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
cargo run --release -p demo                      # default: bevy_3d_scene (PBR + physics)
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

# Benchmarks (criterion; benches only in gizmo-math and gizmo-core). CI runs once with --test:
cargo bench --workspace --benches -- --test    # smoke: runs each bench once, catches runtime panics
cargo bench -p gizmo-core                       # real timing run
```

### Lint (must pass — this is the exact CI gate)

```bash
cargo clippy --workspace --all-features --all-targets -- -D warnings \
  -A clippy::too_many_arguments -A clippy::type_complexity
```

`-D warnings` is a real gate. The two `-A` exemptions are grandfathered architectural lints (a **ratchet**: new lint kinds break CI; the exempt list only shrinks). `rustfmt` is checked **report-only** in CI — the tree is not yet fully fmt-clean, so a `cargo fmt` diff will not fail CI, but don't reflow unrelated code.

> Gotchas when reproducing CI locally: the entry crate is **`gizmo-engine`**, not `-p gizmo`. Piping cargo through `| tail` masks the exit code — check exit status separately.

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
- **`gizmo`** (facade, crate `gizmo-engine`) — `gizmo::prelude::*`, the high-level `SimpleApp`/`App<S>` API. Feature flags gate every subsystem (`render`, `audio`, `physics`, `physics-soft`, `editor`, `scene`, `scripting`, `network`, `egui`, `analysis`, …); `headless` = physics + net, no window/render. Audio, networking, and Lua scripting are **native-only** (not on WASM).

## Determinism contract (important)

The simulation state (Transform/Velocity/solver) runs entirely on **glam/f32**. Guarantee is **same-platform** replay + rollback bit-equality, verified via `state_hash` and cross-process tests. Cross-platform bit-exact determinism is explicitly **out of scope**. When you change physics: any bit-level change must be intentional — the `headless_stress_test` and `soak_*` regression tests exist to catch unintended drift. Historical hashes in docs/comments are point-in-time and have been superseded.

## Working conventions (from docs/ENGINE.md §8)

- Each change: **fix → write a regression test → build/test/clippy → done.** Verify behavior-changing physics fixes with `headless_stress_test` + focused scenarios; choose a soak-test horizon *past* the onset of instability (a too-short soak once shipped green while hiding an explosion at frame ~853).
- On bug-hunt sweeps: fan out subagents, then **verify each finding by hand** — this codebase has a documented history of false positives (see ENGINE.md §7 for the list of already-refuted "bugs" — don't re-chase them).
- Known accepted non-goals: narrowphase batch-SIMD (measured ~3% of frame, rejected); N≥48 extreme towers still buckle (`soak_extreme_tower_n48` is `#[ignore]`, game structures are ≤~12 so it doesn't matter).

## Docs & conventions

- **`docs/ENGINE.md`** is the single internal engineering doc: architecture, live roadmap, release strategy (staged 1.0), determinism/migration contracts, closed research. **Written in English** (translated 2026-08-06 — the bus-factor item D2); many inline code comments are still Turkish. `README.md` = user-facing intro, `CHANGELOG.md` = version history.
- Public API hardening for 1.0 is in progress: 96 types are `#[non_exhaustive]`, errors are enums + `Result`. **docs/ENGINE.md §4 is the authority on what may appear on a Stage A public surface — read it before adding a dependency type to any `pub` signature, field, associated type or trait impl.** In short: `glam` is a deliberate permanent public dep; `bevy_reflect` is behind the default-off `reflect` feature and `tracing-subscriber` behind the default-off `tracing-layer` feature (`tracing` 0.1 itself is unconditional — it is frozen and leaves no type in our signatures); `crossbeam-queue` is sealed out of `gizmo-core` by the opaque `asset::AssetDropQueue`, `arrayvec` out of `gizmo-physics-core` by `ContactPoints` + `collision::ContactPointsIter`, and `ron`/`web-time` out of `gizmo-scene` by `error::{ParseError, SerializeError}` + a private `SceneSnapshot::timestamp` (no `pub use ron;`, no `gizmo::ron`); `rustc-hash` is sealed out of `gizmo-physics-rigid` by the opaque `world::EntityIndexMap` (that crate's 1.0 blocker, cleared 2026-08-09 — the field stays `pub` and readable, but it is no longer a `HashMap`); `wgpu`/`winit`/`egui` leak intentionally during 0.x. **Known unsealed:** nothing on a Stage A default surface except the deliberate `glam` and `serde`/`serde_json` entries, both accepted in §4 — check there before adding a fifth.
