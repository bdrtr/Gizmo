# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Gizmo Engine — a pure-Rust, ECS-driven 3D game engine and physics simulator with **zero external physics dependencies**. Cargo workspace of ~20 crates at uniform version `0.8.0`, published to crates.io (the facade crate `crates/gizmo` is published as **`gizmo-engine`**). Aimed at large-scale, deterministic physics simulation + a WGPU renderer that also runs in the browser via WASM/WebGPU.

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
cargo test -p gizmo-physics-rigid --features experimental-multibody

# Demos live in demo/src/bin/ (39 of them). ALWAYS use --release for physics demos —
# a debug build hits a severe broad/narrow-phase CPU bottleneck.
cargo run --release -p demo                      # default: bevy_3d_scene (PBR + physics)
cargo run --release -p demo --bin car_demo
cargo run --release -p demo --bin advanced_physics

# Determinism / stability gate — 200-box tower collapse, 3 runs, hashes must match
cargo run --release -p demo --bin headless_stress_test

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
```

## Environment / machine constraints

`.cargo/config.toml` caps `jobs = 4` and sets `codegen-units=4, lto=off` — this dev machine has limited RAM (~13 GB); each rustc uses 1–2 GB, so unbounded parallelism OOMs. `[profile.dev]` uses `debug = "line-tables-only"` + `split-debuginfo = "unpacked"` (demo binaries statically link all of wgpu/egui; full DWARF blew `target/` past 600 GB). These affect debug info / build only — **runtime perf is unchanged**. Don't "fix" these settings.

## Architecture

Clean bottom-up layering, **no circular dependencies**:

```
gizmo-math ─┬─ gizmo-core ─┬─ gizmo-physics-{core,rigid,dynamics,soft}
            │              ├─ gizmo-renderer ─ gizmo-{window,ui,editor}
            │              ├─ gizmo-{scene,net,ai,animation,audio,scripting}
            └──────────────┴─ gizmo-app ─ gizmo (facade) ─ demo / cradle / server
```

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

- **`docs/ENGINE.md`** is the single internal engineering doc: architecture, live roadmap, release strategy (staged 1.0), determinism/migration contracts, closed research. **Written in Turkish**, as are many inline code comments. `README.md` = user-facing intro, `CHANGELOG.md` = version history.
- Public API hardening for 1.0 is in progress: 96 types are `#[non_exhaustive]`, errors are enums + `Result`. `glam` is a deliberate permanent public dep; `bevy_reflect` is behind the default-off `reflect` feature; `wgpu`/`winit`/`egui` leak intentionally during 0.x.
