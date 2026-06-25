# Releasing Gizmo — crates.io 1.0 Strategy

Status: **NOT 1.0-ready, but the Stage A core blockers are now sealed.** The
engine completed a multi-round 1.0-readiness audit (205 findings, 26 blockers)
and the hardening rounds. As of the Stage-A-sealing pass (2026-06-25), the core
no longer leaks `bevy_reflect` (§4a) or `arrayvec` (§4b) through its public API,
`glam` is documented as an official public dependency (§4e), and a verified MSRV
is declared and CI-gated (§4f). What remains for Stage A is the mechanical
`get_`-getter rename (§4d); Stage B (graphics) still re-exports pre-1.0 `wgpu`/
`winit`/`egui` by design (§3) until the upgrade in §4c. This document defines a
**staged** path to 1.0 instead of a single lock-step release.

> **Progress (2026-06-25 Stage-A-sealing pass):** §4(a) reflect-gating,
> §4(b) arrayvec newtype, §4(e) glam public-dep doc, §4(f) MSRV, and the §4(d)
> **getter rename** are **done** and verified (full workspace: 552 tests,
> `--all-features`, both feature configs, CI clippy on stable, determinism 3/3
> unchanged, MSRV 1.89 build). §4(d)'s rename was deliberately scoped by the Bevy
> `get_`-=-fallible convention (most `get_*` are intentional; see §4d). §4(d)'s
> visibility-narrowing of `gizmo-animation` is **deferred** (load-bearing
> `gizmo_app::Plugin`/`App` dep — needs an architectural extraction). Remaining
> Stage B blocker: §4(c) graphics upgrade.

This is a strategy document, not a promise of dates. It records *why* the version
plan is what it is, *which* crates can ship 1.0 first, *what* the external-type
contract is, and *what work remains* in priority order.

---

## 1. Versioning strategy: STAGED 1.0

### Why lock-step 1.0 was abandoned

The original plan (`publish_all.sh`) ships all 19 publishable crates in one
topological pass at a single shared version (currently `0.1.7`, inherited from
`[workspace.package]`). That model is fine for 0.x but is **wrong for 1.0**.

A `1.0.0` on a crate is a hard semver promise: no breaking change to its public
API without a `2.0`. Several Gizmo crates **cannot** make that promise yet,
because their public API re-exports types from dependencies that are themselves
`0.x` and will break:

- `wgpu 0.20`, `winit 0.29`, `egui 0.28` appear directly in the public surface of
  `gizmo-renderer`, `gizmo-window`, `gizmo-editor`, `gizmo-app`, `gizmo-ui`,
  `gizmo-scripting`, and the `gizmo` facade.
- `bevy_reflect 0.15` is load-bearing in `gizmo-core`, `gizmo-physics-core`,
  `gizmo-physics-rigid`, and `gizmo-scene` (the `Reflect` derives on `Transform`,
  `RigidBody`, `Velocity`, etc. drive scene serialization).

If we tagged those crates `1.0` today, the next routine upgrade of `wgpu` or
`bevy_reflect` would force a `2.0` — i.e. the `1.0` would be a lie. Lock-step
1.0 would either freeze the whole engine on ancient `wgpu 0.20`/`egui 0.28`
forever, or burn the `1.0` promise on the first dependency bump.

### The staged model

We decouple the version of the **external-dependency-light core** from the
**graphics/integration layer**:

- **Stage A — Core to 1.0.** Ship `gizmo-math`, `gizmo-core`, the `physics-*`
  crates, `gizmo-scene`, and the other dependency-light crates as `1.0` once the
  `bevy_reflect` leak (and the `arrayvec` leak in `physics-core`) is sealed behind
  our own API. These crates have a stable, owned public surface and *can* honor
  the 1.0 promise.
- **Stage B — Graphics/integration to 1.0.** `gizmo-renderer`, `gizmo-window`,
  `gizmo-editor`, `gizmo-ui`, `gizmo-app`, `gizmo-scripting`, and the `gizmo`
  facade stay on `0.x` until `wgpu`/`winit`/`egui` are upgraded to versions we
  are willing to pin into a 1.0 contract (ideally each of those crates' own
  `1.0`, or a version we wrap behind an opaque API). They keep moving on `0.y`
  while Stage A is frozen at `1.0`.

Consequence: crates will **no longer share a single workspace version**. Stage A
crates advance on the `1.x` line; Stage B crates stay on `0.y`. The
`[workspace.package] version` inheritance is dropped for these two groups (see
§5). The `gizmo` facade, because it re-exports the graphics layer, is a **Stage B**
crate and stays `0.x` until Stage B completes — even though most of what it pulls
in is already 1.0-grade.

---

## 2. Per-crate 1.0 readiness

Legend: **Ready?** = can honor a 1.0 semver promise after the listed blocker is
resolved. **Ext-type leak** = pre-1.0 external types in the *public* API.

| Crate | Stage | 1.0-ready? | Blocker | Ext-type in public API |
|---|---|---|---|---|
| `gizmo-math` | A | ✅ Yes (now) | none — `glam` is an intentional public dep (§3) | `glam 0.29` (pinned `=0.29.3`, documented public dep) |
| `gizmo-core` | A | ⚠️ After reflect-gating | `bevy_reflect 0.15` in derives | `bevy_reflect` |
| `gizmo-physics-core` | A | ⚠️ After reflect + arrayvec | `bevy_reflect` + `arrayvec` leak | `bevy_reflect`, `arrayvec` (`CollisionEvent.contact_points`) |
| `gizmo-physics-rigid` | A | ⚠️ After reflect + arrayvec | `bevy_reflect` + `arrayvec` (via physics-core) | `bevy_reflect`, `arrayvec` |
| `gizmo-physics-dynamics` | A | ✅ After core stages | none of its own (no ext leak) | none |
| `gizmo-physics-soft` | A | ✅ After core stages | `wgpu` only behind opt-in `gpu_physics` feature (off by default) | `wgpu 0.20` (feature-gated, not default) |
| `gizmo-scene` | A | ⚠️ After reflect-gating | `bevy_reflect 0.15` (serialization) | `bevy_reflect` |
| `gizmo-net` | A | ✅ After core stages | none of its own | none |
| `gizmo-audio` | A | ✅ After core stages | none of its own | none |
| `gizmo-ai` | A | ✅ After core stages | none of its own | none |
| `gizmo-animation` | A* | ⚠️ depends on `gizmo-app` | inherits Stage B via `gizmo-app` | (transitive) |
| `gizmo-ui` | B | ❌ No | depends on `gizmo-app` (Stage B) | (transitive `wgpu`/`winit`/`egui`) |
| `gizmo-renderer` | B | ❌ No | `wgpu`/`winit` upgrade | `wgpu 0.20`, `winit 0.29` |
| `gizmo-window` | B | ❌ No | `winit` upgrade | `winit 0.29` |
| `gizmo-scripting` | B | ❌ No | depends on `gizmo-renderer` | `wgpu 0.20` (transitive) |
| `gizmo-editor` | B | ❌ No | `wgpu`/`winit`/`egui` upgrade | `wgpu 0.20`, `winit 0.29`, `egui 0.28`, `egui_dock 0.13`, `transform-gizmo-egui 0.3` |
| `gizmo-app` | B | ❌ No | `wgpu`/`winit`/`egui` upgrade (feature-gated but public) | `wgpu`, `winit`, `egui` (opt-in features) |
| `gizmo` (facade) | B | ❌ No | re-exports the whole graphics layer | `wgpu`, `winit`, `egui` (opt-in features) |
| `gizmo-studio` | — | n/a | `publish = false` (binary/app, never published) | n/a |

\* `gizmo-animation` is dependency-light in itself but currently depends on
`gizmo-app` (Stage B). To make it Stage A, that dependency must be narrowed to
the core crates it actually needs (`gizmo-core`, `gizmo-math`,
`gizmo-physics-core`) — see §4(d) visibility/dependency narrowing. Until then it
is pinned to Stage B by transitivity.

### Reading the table

- **Stage A 1.0 candidates (post reflect/arrayvec gating):** `gizmo-math`,
  `gizmo-core`, `gizmo-physics-core`, `gizmo-physics-rigid`,
  `gizmo-physics-dynamics`, `gizmo-physics-soft`, `gizmo-scene`, `gizmo-net`,
  `gizmo-audio`, `gizmo-ai`. This is the engine's stable, ownable core: ECS,
  math, physics, serialization, netcode, audio, AI.
- **Stage B (blocked on graphics upgrade):** everything that touches windowing,
  rendering, or the egui editor — plus the `gizmo` facade and `gizmo-ui`.
- `gizmo-math` is the only crate that is 1.0-ready **today**: its sole external
  public dependency, `glam`, is deliberate and documented (§3).

---

## 3. External-type contract

Gizmo deliberately exposes some third-party types in its public API. This is a
**design choice**, not an accident, and it is bounded:

### `glam` — permanent, intentional (Stage A) — **documented (§4e done)**

`gizmo-math` re-exports `glam` (`Vec3`, `Quat`, `Mat4`, …) as the engine's vector
math vocabulary. This is intentional and permanent: forcing callers through
newtype wrappers around `glam` would add zero value and break ergonomics. The
re-export now goes **directly through `glam`** (`pub use glam::{…}` in
`gizmo-math/src/lib.rs`), not via `bevy_math` — `bevy_math` re-uses the exact
same `glam` types, so this is the single source of truth. `glam` is on the
`0.29` line; a `glam` major bump is a deliberate, documented `gizmo-math` bump.
Recorded as an **official public dependency** in `gizmo-math`'s crate docs.

> **Note — `bevy_math` transitive dependency (RESOLVED 2026-06-25).** Historically
> `gizmo-math` declared a regular `bevy_math 0.15` dependency (used only as the
> re-export origin and by its benches), which transitively pulled `bevy_reflect`
> into the **production** dependency tree even with the `reflect` feature off — the
> feature gate removed it from the public *API*, not from the dependency *tree*.
> That regular dependency was **unused in `gizmo-math`'s source** (the math
> vocabulary already re-exports `glam` directly, §4e), so it has been **removed**.
> `bevy_math`/`bevy_picking`/`bevy_mesh` remain only as **dev-dependencies** (a
> comparison baseline for the benches); dev-deps do not propagate to downstream
> consumers. Verified: `cargo tree -p gizmo-physics-rigid -e no-dev -i bevy_reflect`
> now matches nothing — `bevy_reflect` is gone from the entire Stage A default
> production tree. One consequence: the `reflect` feature now enables
> `bevy_reflect`'s `glam` feature **explicitly** (it was previously satisfied
> transitively via `bevy_math`).

### `bevy_reflect` — **sealed behind the `reflect` feature (§4a done)**

`bevy_reflect 0.15` was public (via `#[derive(Reflect)]` on core components +
the `ComponentRegistry` reflect fields, used by scene serialization). It is now
gated behind a workspace-wide, **off-by-default** `reflect` feature across
`gizmo-core`, `gizmo-physics-core`, `gizmo-physics-rigid`, and `gizmo-scene`. With
the default feature set, none of these crates mention `bevy_reflect` in their
public API; scene save/load + snapshot fall back to plain `serde` (every gated
component derives `Serialize`/`Deserialize`, so the fallback is fully functional
and round-trip-tested). When `reflect` is on, the typed reflect (de)serializers
are used as before.

### `wgpu` / `winit` / `egui` — intentional, but only safe while 0.x (Stage B)

For the graphics crates (`gizmo-renderer`, `gizmo-window`, `gizmo-editor`,
`gizmo-app`, `gizmo`, `gizmo-ui`), `wgpu`, `winit`, and `egui` types **are a
conscious part of the public API**. A renderer that hides `wgpu::Device` /
`wgpu::Queue`, or a window layer that hides `winit::event::WindowEvent`, would be
hostile to real users who need to drop down to the GPU/window layer.

The contract is therefore explicit:

> While the graphics crates remain on `0.x`, re-exporting `wgpu 0.20` /
> `winit 0.29` / `egui 0.28` carries **no semver violation** — `0.x` already
> signals that breaking changes (including from these deps) may occur on any
> minor bump. The risk only materializes at `1.0`. Hence these crates **stay on
> `0.x` by design** until the upgrade in §4(c) lands and we can pin a version we
> are willing to freeze into a 1.0 promise.

In short: the external-type leak in Stage B is **not a bug to be hidden**, it is
a feature whose semver cost is deferred by keeping those crates pre-1.0.

---

## 4. Remaining-work checklist (the road to 1.0)

In priority order. Effort: **S** (hours), **M** (a day), **L** (multi-day,
risky), **XL** (large, multi-day, high API churn).

### (a) Gate `bevy_reflect` behind a workspace `reflect` feature — **L** — ✅ **DONE (2026-06-25)** (audit blocker #1)

`bevy_reflect 0.15` was load-bearing in the `Reflect` derives on `Transform`,
`RigidBody`, `Velocity`, etc. in `gizmo-core`, `gizmo-physics-core`,
`gizmo-physics-rigid`, and `gizmo-scene`, and drove scene serialization. It is now
behind a single, **off-by-default** workspace `reflect` feature so the default
public API of the Stage A crates does **not** mention `bevy_reflect`.
Implementation:
- Each crate makes `bevy_reflect` an `optional` dep with a `reflect` feature that
  chains down the dependency graph (`gizmo-scene/reflect` →
  `gizmo-physics-rigid/reflect` → `gizmo-physics-core/reflect` →
  `gizmo-core/reflect`).
- Derives use `#[cfg_attr(feature = "reflect", derive(Reflect))]` /
  `#[cfg_attr(feature = "reflect", reflect(ignore))]`; the `ComponentRegistry`
  reflect fields/methods (`reflect_registry`, `get_reflect_ptr_fn`,
  `insert_reflect_fn`, `register_reflect`, `InsertReflectFn`) and the
  `pub use bevy_reflect` re-exports are `#[cfg(feature = "reflect")]`.
- Scene's `serde_bridge` module centralizes the reflect-vs-`serde` cfg in one
  place; `default_scene_registry` registers components via `register_reflect`
  (reflect on) or `register_serializable` (reflect off). Both paths are
  round-trip-tested (`scene_save_load_roundtrip…`, snapshot round-trip).

### (b) Replace the `arrayvec` leak with an opaque newtype — **M** — ✅ **DONE (2026-06-25)** (audit blocker #2)

`gizmo-physics-core` exposed `arrayvec::ArrayVec` via
`CollisionEvent.contact_points`. It is now an opaque `ContactPoints` newtype
(`gizmo_physics_core::collision::ContactPoints`) whose backing `arrayvec` is
private; callers use `len`/`is_empty`/`push`/`first`/`iter`/`as_slice`, `Index`,
`IntoIterator` (by value and by ref), and `FromIterator`. `arrayvec` is no longer
in the public API and was dropped as a direct dep of `gizmo-physics-rigid`.

### (c) Upgrade `wgpu` / `winit` / `egui` to current — **XL** — ✅ **DONE (2026-06-25, branch `upgrade/graphics-stack`)**

Done: **wgpu 0.20→29, winit 0.29→0.30, egui 0.28→0.34** (+ `egui-wgpu`/`egui-winit`
0.34, `egui_dock` 0.13→0.19, `transform-gizmo-egui` 0.3→0.9). Full workspace builds
(+`--all-features`), 552 tests pass, CI clippy green on nightly+stable, determinism
3/3 unchanged, a real windowed run works, MSRV raised 1.89→**1.92** (egui floor). See
[`docs/graphics-upgrade-plan.md`](docs/graphics-upgrade-plan.md) for the matrix,
per-dependency cheatsheet, and reusable recipes. **This unblocks the entire Stage B
1.0.** (the upgrade first used winit's deprecated `run`/`create_window` as a
bridge; that bridge has since been fully migrated to `ApplicationHandler` — see
below.)

> **Deprecation follow-up (2026-06-25, DONE — the codebase is now
> deprecation-clean):** the crate-level `#![allow(deprecated)]` bridges left by
> the upgrade have been removed, and **every** Gizmo-owned deprecation is
> migrated (workspace-wide `--force-warn deprecated` reports none). This covered:
> - All mechanical egui 0.34 renames (`close_menu→close`,
>   `from_id_source→from_id_salt`, `Context::{style→global_style, begin_frame→
>   begin_pass, end_frame→end_pass, screen_rect→content_rect, …}`, `Frame::none→
>   new`, `allocate_ui_at_rect→scope_builder`, …).
> - The egui top-level panel `show(ctx)` pattern → egui 0.34 root-`Ui`
>   composition (`Ui::new` + `show_inside`) across the editor and studio.
> - **winit 0.30 `EventLoop::{run,create_window}` → `ApplicationHandler`/
>   `run_app`** in `gizmo-app/src/windowed.rs`: the window is now created lazily
>   in `resumed` (`ActiveEventLoop::create_window`), the ~550-line event loop is
>   driven via `window_event`/`about_to_wait`/`device_event` (each reconstructs a
>   `winit::Event` and dispatches to a unified `handle_event`, preserving the
>   `input_fn(&Event)` hook contract), and the async GPU/editor init runs in
>   `resumed` via `pollster::block_on`. Verified: full build, 552 tests, CI clippy
>   `-D warnings`, determinism unchanged, and real windowed runs of `gizmo-studio`
>   + `bevy_3d_scene`. (The wasm `resumed` branch is a stub — it is part of the
>   separate, deferred WASM port, which does not build in this environment.)

<details><summary>Original scope notes</summary>

Move `wgpu 0.20 → current`, `winit 0.29 → current`, `egui 0.28 → current`
(and the egui ecosystem crates: `egui-winit`, `egui-wgpu`, `egui_dock 0.13`,
`transform-gizmo-egui 0.3`). `wgpu 0.20 → current` alone is a large API break
(surface/device/pipeline/bind-group API churn); `winit 0.29 → current` changes
the event-loop/`ApplicationHandler` model; `egui 0.28 → current` ripples through
the entire editor. This is the single largest piece of work and the gate for the
**entire Stage B 1.0** (renderer, window, editor, ui, app, scripting, facade).
Until it lands, Stage B stays `0.x` (which is fine — see §3).

</details>

### (d) `get_` getter rename + visibility narrowing — **M** — ⚠️ **getter rename DONE (2026-06-25); visibility narrowing assessed** (audit blocker #3)

**Getter rename — done, but scoped by the Bevy convention.** The naive reading of
C-GETTER ("drop every `get_`") is **wrong** for this engine: Gizmo deliberately
models Bevy, and Bevy keeps `get_` for **fallible** accessors (`get_resource` →
`Option`, vs `resource()` which panics). Auditing the return types shows the vast
majority of Gizmo's `get_*` (`get_resource`/`get_resource_mut` (173 call sites),
`get_entity` (41), `get_component_ptr`, `get_column`, `get_name`, `get_type_id`,
`get_registration`, `get_contact`, …) return `Option`/`Result` or are
collection-style `get`/`get_mut` — these are **idiomatic and kept**. Only the
genuinely **infallible plain-value getters** violated C-GETTER and were renamed:
`get_neighbors → neighbors`, `get_entity_component_types → entity_component_types`,
`get_log_version → log_version`, `get_engine_torque → engine_torque`,
`get_entity_names → entity_names`. (`get_or_predict` is a fallback combinator and
`get_logs` is a closure-scoped accessor — both kept.) Pure renames; verified by
552 tests + clippy + determinism (hash unchanged).

**Visibility narrowing — assessed, mostly deferred.** The headline item
(narrowing `gizmo-animation`'s `gizmo-app` dependency to promote it to Stage A) is
**not feasible as polish**: `gizmo-animation` implements `gizmo_app::Plugin<State>`
on `gizmo_app::App<State>`, so the dependency is load-bearing. Promoting it would
require extracting the `Plugin`/`App` abstraction down into a core crate — a real
architectural change, out of scope here. `gizmo-animation` therefore **stays
Stage B** until that extraction happens. Broader `pub`-tightening from the audit
raw output remains as fine-grained follow-up.

### (e) Document `glam` as an official public dependency — **S** — ✅ **DONE (2026-06-25)**

`gizmo-math`'s crate docs now declare `glam` (`0.29` line) a **public**
dependency, and the re-export was switched to go directly through `glam`
(`pub use glam::{…}`) instead of `bevy_math`. See §3 (incl. the `bevy_math`
transitive-dependency note).

### (f) Set and document an MSRV — **S** — ✅ **DONE (2026-06-25)**

`rust-version = "1.89"` is set in `[workspace.package]` and gated by a new CI
`msrv` job (`dtolnay/rust-toolchain@1.89.0` → `cargo check --workspace`).
**The MSRV was determined empirically, not assumed:** 1.82 fails (a transitive
`crypto-common 0.2.1` needs `edition2024` → 1.85), 1.85 fails (locked `wide 1.2`
/ `safe_arch 1.0` need 1.89), and 1.89 builds the full workspace clean. The
naive "bevy 0.15 ⇒ 1.82" floor is **wrong** for the current lock file.

### Sequencing

1. Stage A 1.0: (e) + (a) + (b) + (d) + (f), released together as one breaking
   pass, then tag the Stage A crates `1.0.0`.
2. Stage B 1.0: (c), then (re)narrow facade re-exports, then tag the Stage B
   crates `1.0.0` (likely a later, separate release once the upgraded graphics
   stack has settled).

---

## 5. Publish order

Crates publish in **dependency (topological) order** so each crate's path-deps
already exist on crates.io. This is the order encoded in `publish_all.sh`;
the corrected topological order (foundations first, facade last) is:

```
1.  gizmo-math            (foundation; glam)
2.  gizmo-core            (ECS; depends on math)
3.  gizmo-physics-core    (core, math)
4.  gizmo-physics-rigid   (core, math, physics-core)
5.  gizmo-net             (core, math, physics-core, physics-rigid)
6.  gizmo-physics-soft    (core, math, physics-core)
7.  gizmo-physics-dynamics(core, math, physics-core, physics-rigid)
8.  gizmo-audio           (core)
9.  gizmo-ai              (core, math, physics-core, physics-rigid)
10. gizmo-renderer        (math, core)            [Stage B]
11. gizmo-window          (winit)                 [Stage B]
12. gizmo-scripting       (core, math, physics-*, ai, renderer)  [Stage B]
13. gizmo-scene           (core, math, physics-core, physics-rigid)
14. gizmo-editor          (renderer, scene, scripting, egui…)    [Stage B]
15. gizmo-app             (renderer, editor, scene, net, audio…) [Stage B]
16. gizmo-animation       (app, core, math, physics-core)
17. gizmo-ui              (app, core, math)        [Stage B]
18. gizmo                 (facade — re-exports all) [Stage B]
        gizmo-studio       NOT PUBLISHED (publish = false)
```

> **Important — staged versions break the single-version assumption.**
> `publish_all.sh` currently hard-codes one shared version (`0.1.7`) for every
> crate. Under the staged model, Stage A crates will be on `1.x` while Stage B
> crates remain on `0.y`. Before the first staged publish:
> - Drop the blanket `version` inheritance from `[workspace.package]` for the
>   two groups (or split into per-crate versions).
> - Update each crate's path-dep version requirements so a `1.x` Stage A crate is
>   referenced as `"1"` and a `0.y` Stage B crate as `"0.y"`.
> - Update `publish_all.sh` (or replace it with `cargo release` / a release-plz
>   workflow) so it no longer assumes a single uniform version string.

---

## TL;DR

Gizmo is **not 1.0-ready**, but the path is clear and staged, and most of Stage A
is now sealed:

1. ✅ Seal `bevy_reflect` (§4a), ✅ `arrayvec` (§4b), ✅ document `glam` (§4e),
   ✅ set MSRV (§4f), ✅ getter rename (§4d, Bevy-scoped) — **done 2026-06-25,
   fully verified**. Remaining Stage A polish: audit `pub`-tightening + the
   `gizmo-animation`→Stage A extraction (§4d, deferred). Then **Stage A core
   ships 1.0** (math, ECS, physics, scene, net, audio, AI).
2. Upgrade `wgpu`/`winit`/`egui` (§4c, XL) → **Stage B graphics + `gizmo` facade
   ship 1.0** later.

Until Stage B's upgrade lands, the graphics crates stay on `0.x` **by design**,
where re-exporting pre-1.0 `wgpu`/`winit`/`egui` costs no semver promise.
