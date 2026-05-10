<div align="center">
  <img src="media/logo.png" alt="Gizmo Engine Logo" width="250" />
  <h1>Gizmo Engine</h1>
  <p><strong>A lightweight, ECS-driven 3D game engine and physics simulator written entirely in Rust.</strong></p>

  [![Crates.io](https://img.shields.io/crates/v/gizmo-engine.svg)](https://crates.io/crates/gizmo-engine)
  [![Docs.rs](https://img.shields.io/docsrs/gizmo-engine.svg)](https://docs.rs/gizmo-engine)
  [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
  [![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
</div>

<br/>

Gizmo Engine is a high-performance, data-driven, and fully modular game development framework. Designed specifically for large-scale physics simulations, advanced vehicular dynamics, and modern 3D rendering, it provides an industry-standard workflow with zero external physics API dependencies.

---

## ✨ Features

- **Archetype-based ECS:** A columnar, data-driven Entity Component System powered by `mimalloc`. Built for maximum cache locality and lock-free concurrency, easily scaling to tens of thousands of entities.
- **Custom Vectorial Physics Engine:** Built from scratch using purely mathematical vectors. Features include:
  - Sweep and Prune (SAP) Broad-Phase with Rayon multi-threading.
  - GJK/EPA Narrow-Phase for accurate collision detection (Convex Hulls, Capsules, Polygons).
  - FEM (Finite Element Method) Soft-Body Physics for hyper-realistic deformation and stress-tensor calculations.
  - Sequential Impulse Solvers with advanced Coulomb Friction and Moment of Inertia.
- **WGPU-Based Rendering:** A robust graphics pipeline supporting Vulkan, Metal, DX12, and **WebAssembly (WASM)**. Features Instanced Rendering, GLTF PBR Materials, Dynamic Shadows (CSM), SSAO, Bloom, and Deferred Shading.
- **In-Game Editor:** Built-in `egui` tooling with a dynamic scene hierarchy, real-time inspector, and modular prefab architecture.
- **Spatial Audio:** RAM-cached, 3D spatial audio engine with distance attenuation and Doppler effect support.

## 🚀 Quickstart

Gizmo Engine is designed to be highly modular and ergonomic. Here is a minimal "Hello World" example demonstrating how to spawn a rotating 3D cube.

```rust
use gizmo::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_startup_system(setup_scene)
        .add_system(rotate_cube)
        .run();
}

fn setup_scene(
    mut commands: Commands, 
    mut meshes: ResMut<Assets<Mesh>>, 
    mut materials: ResMut<Assets<StandardMaterial>>
) {
    // Spawn Camera
    commands.spawn(Camera3dBundle {
        transform: Transform::from_xyz(0.0, 5.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
        ..Default::default()
    });

    // Spawn Light
    commands.spawn(DirectionalLightBundle {
        transform: Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4)),
        ..Default::default()
    });

    // Spawn Cube
    commands.spawn((
        PbrBundle {
            mesh: meshes.add(Mesh::from(shape::Cube { size: 1.0 })),
            material: materials.add(Color::rgb(0.8, 0.2, 0.3).into()),
            transform: Transform::from_xyz(0.0, 0.0, 0.0),
            ..Default::default()
        },
        RotatingComponent,
    ));
}

#[derive(Component)]
struct RotatingComponent;

fn rotate_cube(time: Res<Time>, mut query: Query<&mut Transform, With<RotatingComponent>>) {
    for mut transform in query.iter_mut() {
        transform.rotate_y(1.0 * time.delta_seconds());
    }
}
```

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

To compile the engine and test the showcase map with advanced physics and rendering:

```bash
cargo run --release --bin showcase
```

> **Note:** Due to the extreme scale of the broad-phase and narrow-phase physics computations, compiling without `--release` will cause a severe CPU bottleneck. Always use the release profile for optimal performance.

## 📄 License

Gizmo Engine is free, open source, and dual-licensed under the MIT and Apache 2.0 licenses.
