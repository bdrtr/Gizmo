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
> documented in [`RELEASING.md`](RELEASING.md) and remains the planned path for a
> later release.

## [0.2.0] — 2026-06-25

The first release since `0.1.7`. It gathers the entire 1.0-readiness effort
(audit + hardening rounds) and the graphics-stack upgrade, shipped as a single
breaking `0.x` bump.

### Changed (breaking)

- **Graphics stack upgraded** across the Stage B crates: `wgpu 0.20 → 29`,
  `winit 0.29 → 0.30`, `egui 0.28 → 0.34` (plus `egui-wgpu`/`egui-winit` `0.34`,
  `egui_dock 0.13 → 0.19`, `transform-gizmo-egui 0.3 → 0.9`). Public `wgpu`/
  `winit`/`egui` types in the renderer/window/editor/app/facade move to the new
  versions. See [`docs/graphics-upgrade-plan.md`](docs/graphics-upgrade-plan.md).
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
- `RELEASING.md` (staged-1.0 strategy) and this changelog.

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
