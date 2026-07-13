<div align="center">
  <img src="media/logo.png" alt="Gizmo Engine Logo" width="250" />
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

- **Archetype-based ECS:** A columnar, data-driven Entity Component System powered by `mimalloc`. Built for maximum cache locality and lock-free concurrency, easily scaling to tens of thousands of entities. Component access goes through a borrow-checked query API — shared reads via `World::query` (read-only), exclusive mutation via `World::query_mut`/`borrow_mut` — so two aliasing `&mut` views can never be created from safe code.
- **Custom Vectorial Physics Engine:** Built from scratch using purely mathematical vectors. Features include:
  - Sweep and Prune (SAP) Broad-Phase with Rayon multi-threading.
  - GJK/EPA Narrow-Phase for accurate collision detection (Convex Hulls, Capsules, Polygons).
  - FEM (Finite Element Method) Soft-Body Physics for hyper-realistic deformation and stress-tensor calculations.
  - Sequential Impulse Solvers with advanced Coulomb Friction and Moment of Inertia.
- **WGPU-Based Rendering:** A robust graphics pipeline targeting Vulkan, Metal, DX12 — **and WebGPU in the browser**. Features Instanced Rendering, GLTF PBR Materials, Dynamic Shadows (CSM), SSAO, Bloom, and Deferred Shading. The engine runs on `wasm32-unknown-unknown` with a reduced web pipeline (forward-only, 4 bind groups, no shadows); try it with the [`demo-web/`](demo-web/) crate. Audio, networking and Lua scripting are still native-only — see [`RELEASING.md`](RELEASING.md) §4g.
- **In-Game Editor:** Built-in `egui` tooling with a dynamic scene hierarchy, real-time inspector, and modular prefab architecture.
- **Spatial Audio:** RAM-cached, 3D spatial audio engine with distance attenuation and Doppler effect support.

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

Gizmo's decoupled workspace architecture allows you to pick and choose exactly what you need. If you are building a headless server, simply omit the renderer plugin!

- **`gizmo-core`**: The foundational ECS, math, and scheduling architecture.
- **`gizmo-physics`**: A completely render-agnostic, zero-dependency physics engine. Can be embedded into other engines (e.g., Bevy, Macroquad).
- **`gizmo-renderer`**: The standalone, WGPU-driven rendering pipeline.
- **`gizmo-app`**: The plugin-driven app loop and phase executor.

## 📸 Showcase

<p float="left">
  <img src="media/gizmo_city_demo.jpg" width="48%" />
  <img src="media/gizmo_engine_showcase.png" width="48%" /> 
</p>
<p align="center">
  <img src="media/demo_racetrack.jpg" width="70%" />
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

> **Upgrading?** `0.8.0` is a large feature release; the whole workspace ships
> at one uniform `0.x` version and no API is promised stable yet. See the
> [`CHANGELOG`](CHANGELOG.md) (and, if coming from `0.1.x`, the
> [migration guide](docs/migration-0.1-to-0.2.md)).

## 📄 License

Gizmo Engine is free, open source, and dual-licensed under the MIT and Apache 2.0 licenses.
