# Releasing Gizmo — crates.io 1.0 Strategy

Status: **NOT 1.0-ready.** The engine has completed a multi-round 1.0-readiness
audit (205 findings, 26 blockers) and four hardening rounds, but the public API
of the graphics layer still re-exports pre-1.0 external types, and the core still
leaks `bevy_reflect` through its serialization derives. This document defines a
**staged** path to 1.0 instead of a single lock-step release.

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

### `glam` — permanent, intentional (Stage A)

`gizmo-math` re-exports `glam` (`Vec3`, `Quat`, `Mat4`, …) as the engine's vector
math vocabulary. This is intentional and permanent: forcing callers through
newtype wrappers around `glam` would add zero value and break ergonomics.
`glam` is pinned to `=0.29.3` so a 1.0 of `gizmo-math` ties to an exact, audited
`glam`. A `glam` major bump becomes a deliberate, documented `gizmo-math` bump.
This is recorded as an **official public dependency** (§4(e)).

### `bevy_reflect` — to be sealed before Stage A 1.0

`bevy_reflect 0.15` is currently public (via `#[derive(Reflect)]` on core
components, used by scene serialization). It is **not** an intentional permanent
public dependency. Before Stage A goes 1.0 it must be moved behind a workspace
`reflect` feature so it is not part of the unconditional public API (§4(a)).

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

### (a) Gate `bevy_reflect` behind a workspace `reflect` feature — **L** — *Stage A 1.0 blocker* (audit blocker #1)

`bevy_reflect 0.15` is load-bearing in the `Reflect` derives on `Transform`,
`RigidBody`, `Velocity`, etc. in `gizmo-core`, `gizmo-physics-core`,
`gizmo-physics-rigid`, and `gizmo-scene`, and drives scene serialization. Move it
behind a single, workspace-wide `reflect` feature so that the default public API
of the Stage A crates does **not** mention `bevy_reflect`. Risky because the
derives are spread across four crates and scene save/load depends on them; the
feature must be coherent across the whole Stage A set (a partial gate breaks
serialization). This is the gate that unblocks the entire Stage A 1.0.

### (b) Replace the `arrayvec` leak with an opaque newtype — **M** — *Stage A 1.0 blocker* (audit blocker #2)

`gizmo-physics-core` exposes `arrayvec::ArrayVec` via
`CollisionEvent.contact_points` (and `gizmo-physics-rigid` re-exports it).
Wrap it in an opaque `ContactPoints` newtype that exposes only what callers need
(`len`, `iter`, indexing, `IntoIterator`) so `arrayvec` is an implementation
detail. Lower risk than (a) but still breaking for anyone reading
`contact_points` today.

### (c) Upgrade `wgpu` / `winit` / `egui` to current — **XL** — *Stage B 1.0 blocker*

Move `wgpu 0.20 → current`, `winit 0.29 → current`, `egui 0.28 → current`
(and the egui ecosystem crates: `egui-winit`, `egui-wgpu`, `egui_dock 0.13`,
`transform-gizmo-egui 0.3`). `wgpu 0.20 → current` alone is a large API break
(surface/device/pipeline/bind-group API churn); `winit 0.29 → current` changes
the event-loop/`ApplicationHandler` model; `egui 0.28 → current` ripples through
the entire editor. This is the single largest piece of work and the gate for the
**entire Stage B 1.0** (renderer, window, editor, ui, app, scripting, facade).
Until it lands, Stage B stays `0.x` (which is fine — see §3).

### (d) `get_` getter rename + visibility narrowing — **M** — *should-do, polish* (audit blocker #3)

Rename `get_*` accessors to drop the prefix per Rust API guideline C-GETTER, and
narrow the remaining over-broad `pub` items flagged by the audit (including
narrowing `gizmo-animation`'s dependency on `gizmo-app` down to the core crates
it actually uses, which promotes it from Stage B to Stage A). Breaking but
mechanical; do it in the same release as (a)/(b) so all Stage A breakage lands
at once.

### (e) Document `glam` as an official public dependency — **S** — *Stage A 1.0 requirement*

Record in `gizmo-math`'s crate docs and this file that `glam` (`=0.29.3`) is a
**public** dependency: a `glam` major version is part of `gizmo-math`'s semver
contract. Add the standard "public dependency" note so downstream users know a
`glam` bump may be breaking. Required before Stage A 1.0 so the contract is
explicit, not implied.

### (f) Set and document an MSRV — **S** — *1.0 requirement*

Pick a Minimum Supported Rust Version, set `rust-version` in
`[workspace.package]`, and add it to CI as a build gate. A 1.0 without a declared
MSRV is incomplete; do this before either stage tags 1.0.

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

Gizmo is **not 1.0-ready**, but the path is clear and staged:

1. Seal `bevy_reflect` (L) + `arrayvec` (M), document `glam` (S), rename getters
   (M), set MSRV (S) → **Stage A core ships 1.0** (math, ECS, physics, scene,
   net, audio, AI).
2. Upgrade `wgpu`/`winit`/`egui` (XL) → **Stage B graphics + `gizmo` facade ship
   1.0** later.

Until Stage B's upgrade lands, the graphics crates stay on `0.x` **by design**,
where re-exporting pre-1.0 `wgpu`/`winit`/`egui` costs no semver promise.
