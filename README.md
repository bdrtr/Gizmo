<div align="center">
  <!-- Görseller MUTLAK URL olmak zorunda. GitHub göreli yolları depoya göre çözer, crates.io
       çözmez: README'yi kendi alan adından sunar, yani `media/logo.png` orada
       `https://crates.io/media/logo.png` olur ve 404 verir — crates.io sayfasındaki kırık
       görsellerin sebebi buydu. `main` referansı, `v0.9.1` gibi bir tag yerine, her sürümde
       elle güncellenmek zorunda kalmasın diye. -->
  <img src="https://raw.githubusercontent.com/bdrtr/Gizmo/main/media/logo.png" alt="Gizmo Engine Logo" width="250" />
  <h1>Gizmo Engine</h1>
  <p><strong>A lightweight, ECS-driven 3D game engine and physics simulator written entirely in Rust.</strong></p>

  [![Crates.io](https://img.shields.io/crates/v/gizmo-engine.svg)](https://crates.io/crates/gizmo-engine)
  [![Docs.rs](https://img.shields.io/docsrs/gizmo-engine.svg)](https://docs.rs/gizmo-engine)
  [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
  [![Rust](https://img.shields.io/badge/Rust-1.92%2B-orange.svg)](https://www.rust-lang.org/)
</div>

<br/>

Gizmo Engine is a high-performance, data-driven, and fully modular game development framework. Designed specifically for large-scale physics simulations, advanced vehicular dynamics, and modern 3D rendering, it provides an industry-standard workflow with zero external physics API dependencies.

---

## ✨ Features

- **Deterministic simulation:** The rigid-body simulation is **bit-exactly reproducible on the same platform** — the property that makes replay, deterministic rollback netcode and reproducible bug reports possible. It is enforced, not hoped for: a `state_hash` over the world, a cross-process oracle that runs the same scene in two separate processes (different hash seeds) and compares, and long-horizon soak tests all gate CI. Cross-*platform* bit-exactness is explicitly out of scope — see [`docs/ENGINE.md`](docs/ENGINE.md) §5.
- **Archetype-based ECS:** A columnar, data-driven Entity Component System built for cache locality, easily scaling to tens of thousands of entities. Systems declare their component access and the scheduler batches non-conflicting ones onto a Rayon pool. Component access goes through a borrow-checked query API — shared reads via `World::query` (read-only), exclusive mutation via `World::query_mut`/`borrow_mut` — so two aliasing `&mut` views can never be created from safe code, a contract fenced by `compile_fail` doctests and a Miri job in CI.
- **Custom Physics Engine:** Written from scratch, with **no third-party physics dependency**. Features include:
  - Dynamic AABB tree (BVH) broad-phase with swept AABBs for CCD.
  - GJK/EPA narrow-phase (sphere, box, capsule, plane, convex hull, triangle mesh, compound), with a dedicated SAT path and full contact manifold for box–box.
  - **TGS-Soft contact solver** — temporal Gauss-Seidel with inter-iteration position integration, a per-manifold block LCP, warm starting and a true two-tangent Coulomb friction cone. Islands are solved in parallel via Rayon, in a deterministic order.
  - FEM (Finite Element Method) soft bodies — compressible Neo-Hookean first Piola–Kirchhoff stress — plus XPBD cloth, rope and Voronoi fracture.
- **WGPU-Based Rendering:** A graphics pipeline targeting Vulkan, Metal, DX12 — **and WebGPU in the browser**. Features Instanced Rendering, GLTF PBR Materials, Dynamic Shadows (CSM), SSAO, SSGI, Bloom, TAA and Deferred Shading. The engine also runs on `wasm32-unknown-unknown` with a substantially reduced web pipeline (forward-only, fixed internal resolution, no shadows/AO/GI/TAA); try it with the [`demo-web/`](demo-web/) crate. Networking and Lua scripting are native-only — see [`docs/ENGINE.md`](docs/ENGINE.md).
- **In-Game Editor:** `egui`-based tooling with a scene hierarchy, an inspector for the engine's built-in components, transform gizmos and prefabs. It is a library of panels (`gizmo-editor`); the editor *application* (`gizmo-studio`) is not published to crates.io yet, and the inspector does not yet cover user-defined components.
- **Spatial Audio:** RAM-cached 3D audio with distance attenuation and pitch-based Doppler. It is a thin, functional layer — there is no mixer, bus routing or DSP yet (see the roadmap).

## 🚀 Quickstart

Gizmo Engine is designed to be highly modular and ergonomic. Here is a minimal
example — the default `bevy_3d_scene` demo — that opens a window and renders a lit
3D scene (a ground disc, a cube, a directional light, and a camera) using the
high-level `SimpleApp` API.

```rust
use gizmo::prelude::*;
use gizmo::math::Vec3;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

fn main() {
    gizmo::app::App::<SimpleSceneState>::new("Gizmo Engine - 3D Scene", 1280, 720)
        .with_simple_scene(|scene, state| {
            // Circular ground disc.
            scene.spawn_ground(4.0);

            // A cube sitting on the ground.
            scene.spawn_cube(Vec3::new(0.0, 0.5, 0.0), 1.0, Vec3::new(0.20, 0.28, 1.0));

            // A directional light.
            let light = scene.world.spawn();
            DirectionalLightBundle {
                rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4)
                    * Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
                intensity: 1.8,
                ..Default::default()
            }
            .apply(scene.world, light);

            // A camera looking at the origin.
            scene.spawn_camera(state, Vec3::new(-2.5, 4.5, 9.0), Vec3::ZERO);
        })
        .run()
        .expect("failed to run the app");
}
```

> The full source is [`demo/src/bin/bevy_3d_scene.rs`](demo/src/bin/bevy_3d_scene.rs).
> For lower-level control, drop down to `App`, `Plugin`, `Commands`, `Query`,
> `Res`/`ResMut`, and the `*Bundle` types in [`gizmo::prelude`](crates/gizmo/src/prelude.rs).

## 📦 Workspace Architecture

Gizmo is a workspace of small crates layered bottom-up, so you can depend on the parts you need rather than the whole engine.

- **`gizmo-math`**: Vector/quaternion math (re-exports `glam`).
- **`gizmo-core`**: The foundational ECS, scheduling, events, hierarchy and input.
- **`gizmo-physics-core` / `-rigid` / `-dynamics` / `-soft`**: The physics stack — colliders and collision detection, the rigid-body world and solver, vehicle/character dynamics, and soft bodies/cloth/rope. Render-agnostic: `PhysicsWorld` is a plain structure-of-arrays container keyed by opaque `BodyHandle`s, and stepping it never touches the ECS.
- **`gizmo-renderer`**: The WGPU-driven rendering pipeline.
- **`gizmo-app`**: The plugin-driven app loop and phase executor.
- **`gizmo-engine`**: The facade crate that ties it all together (`gizmo::prelude::*`).

> **Note on embedding the physics on its own:** the simulation core is genuinely
> independent of the renderer and only lightly coupled to the ECS, but
> `gizmo-physics-rigid` still takes `gizmo-core` as a mandatory dependency today, so
> using it from another engine pulls the ECS in with it. Making that dependency
> optional (and shipping the physics crates with their own docs) is a tracked
> roadmap item — see [`docs/ENGINE.md`](docs/ENGINE.md) §3.

## 📸 Showcase

<p float="left">
  <img src="https://raw.githubusercontent.com/bdrtr/Gizmo/main/media/gizmo_city_demo.jpg" width="48%" />
  <img src="https://raw.githubusercontent.com/bdrtr/Gizmo/main/media/gizmo_engine_showcase.png" width="48%" /> 
</p>
<p align="center">
  <img src="https://raw.githubusercontent.com/bdrtr/Gizmo/main/media/demo_racetrack.jpg" width="70%" />
</p>


## 🛠️ Building and Running

To compile the engine and test the showcase scene with advanced physics and rendering:

```bash
# Default demo scene (3D PBR + physics)
cargo run --release -p demo

# Other showcase binaries:
cargo run --release -p demo --bin advanced_physics
cargo run --release -p demo --bin car_demo
cargo run --release -p demo --bin fluid_rigid
```

> **Note:** Due to the extreme scale of the broad-phase and narrow-phase physics computations, compiling without `--release` will cause a severe CPU bottleneck. Always use the release profile for optimal performance.

> **Assets:** every demo runs on a fresh clone. Large `.glb` models are not committed, so
> the couple of demos that showcase one (`car_demo`, `wind_tunnel`) fall back to
> procedural geometry and say so on stderr. Drop the model into `assets/` or point
> `GIZMO_ASSETS` at a directory containing it to get the real thing — see
> [`assets/README.md`](assets/README.md).

> **Upgrading?** `0.10.0` raises the minimum Rust to **1.96** (from 1.92) along with the
> `wgpu` 30 / `egui` 0.36 graphics stack — that is why the minor moved. `0.9.1` before it is what
> followed `0.8.0` on crates.io — `0.9.0` was documented and
> then never published, so there is nothing to upgrade *from* at that number. It carries the
> correctness work of both: two paths to undefined behaviour, one determinism hole, the reason
> the crate previously compiled in only one feature configuration, and the removal of an
> exported `Sprite` component that nothing could ever draw. Public signatures changed, because
> their old shape *was* the bug.
> The whole workspace still ships at one uniform `0.x` version and no API is promised
> stable yet. See the [`CHANGELOG`](CHANGELOG.md) (and, if coming from `0.1.x`, the
> [migration guide](docs/ENGINE.md) (§6)).

## 📄 License

Gizmo Engine is free, open source, and dual-licensed under the MIT and Apache 2.0 licenses.
