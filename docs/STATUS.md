# Gizmo Engine — where it stands

**Last updated: 2026-11-01.** One page, present tense. History, measurements and the reasons
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
| ECS (`gizmo-core`) | Archetype + sparse-set storage, borrow-checked queries, commands, events, observers, hierarchy. Under an ongoing soundness sweep; each round has come back smaller. | ENGINE.md §3 (Phase 6) |
| Rigid physics | GJK/EPA + box SAT, TGS-Soft solver, CCD, joints, vehicles, character, axis locks. Deterministic on one platform. A game with its own update hook steps it at a fixed rate with `gizmo::simple::FixedPhysics`. | ENGINE.md §5, §7 |
| Soft physics | FEM soft bodies, XPBD cloth and rope, Voronoi fracture. | ENGINE.md §1 |
| 3D rendering | Deferred PBR, CSM shadows, SSAO, SSGI, SSR, bloom, volumetrics, TAA/FXAA, decals, irradiance volumes, user materials (forward). | CAPABILITY_GAPS §A, §F |
| 2D rendering | `Sprite` in world space (entity's plane, depth-tested) or screen space; sheets, flips, layers, pixel-art sampling. `ProjectionMode::OrthographicFit` keeps a playfield in view in any window shape; `gizmo::physics::sprite_collider` derives a box collider from a sprite; `SpriteAnimation` plays a sheet; `Tilemap` draws a level from one tileset in one draw call. | CAPABILITY_GAPS §A1 |
| Text and HUD | `Text` in world or screen space; screen text, screen sprites and `gizmo-ui` backgrounds are drawn **after** tone mapping, so a HUD keeps its colours. No wrapping or shaping. | CAPABILITY_GAPS §A2 |
| Browser | `wasm32` with a reduced forward pipeline (no shadows/AO/GI/TAA); audio works there, networking and Lua do not. | `demo-web/README.md` |
| Editor | `gizmo-editor` panels; `gizmo-studio` app (not published). A game's own components join the in-engine editor with `gizmo::register_editor_component`: inspected, edited, added, removed, saved. | README, CHANGELOG |
| Netcode | P2P rollback and client-server (`gizmo-net` features `rollback`, `client-server`). | ENGINE.md §1, §5 |

## What is missing — the gaps that block the most

Ranked and measured in [`CAPABILITY_GAPS.md`](CAPABILITY_GAPS.md); the short version:

- **2D game layer:** no 2D collision shapes — 3D shapes + axis locks work, and
  `gizmo::physics::tilemap_collider` turns a tilemap's solid tiles into merged static boxes (§C9).
- **Rendering:** no environment cubemap / reflection probes (§A5), no MSAA (§A6), no area lights
  or soft contact shadows (§A7), no occlusion culling (§A7b), no motion blur or auto exposure
  (§A7c), split screen half-done (§A5b).
- **Text:** no wrapping, shaping, bidi or font fallback; the engine ships no font (§A2).
- **Extensibility:** several extension points are sealed (§C0).
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
