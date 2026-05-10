# Gizmo Engine Wiki

Welcome to the **Gizmo Engine Wiki**! This documentation portal is designed to help you understand the core architecture of the engine and guide you from building your first scene to deploying complex, physics-driven multiplayer games.

## 📌 Table of Contents

1. [Getting Started](#1-getting-started)
2. [Core Architecture (ECS)](#2-core-architecture-ecs)
3. [Gizmo Physics Engine](#3-gizmo-physics-engine)
4. [Rendering Pipeline](#4-rendering-pipeline)
5. [Animation System](#5-animation-system)

---

## 1. Getting Started

Gizmo Engine is built entirely in Rust. To get started, you will need the Rust toolchain installed.

### Installation
Add Gizmo Engine to your `Cargo.toml`. *(Note: Gizmo is heavily modular, so you can include only the parts you need).*

```toml
[dependencies]
gizmo-engine = { version = "0.1", features = ["renderer", "physics", "audio"] }
```

### Your First App
A basic Gizmo application runs by creating an `App`, attaching plugins, and running the executor.

```rust
use gizmo::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins) // Adds windowing, rendering, and input
        .add_system(hello_world_system)
        .run();
}

fn hello_world_system() {
    println!("Hello from Gizmo Engine!");
}
```

---

## 2. Core Architecture (ECS)

Gizmo uses a **Data-Driven Entity Component System (ECS)**. Instead of object-oriented programming (OOP), logic and data are strictly separated.

### Entities & Components
- **Entity:** A simple ID (e.g., `Entity(12)`). It holds no logic or data itself.
- **Component:** Pure data structs attached to Entities.

```rust
#[derive(Component)]
struct Health {
    pub value: f32,
}

#[derive(Component)]
struct Player;
```

### Systems & Queries
Systems are regular Rust functions that query the ECS world to mutate components. Gizmo's ECS groups identical component combinations into **Archetypes**, ensuring linear memory access and zero cache misses.

```rust
fn take_damage_system(mut query: Query<&mut Health, With<Player>>) {
    for mut health in query.iter_mut() {
        health.value -= 10.0;
        if health.value <= 0.0 {
            println!("Player died!");
        }
    }
}
```

---

## 3. Gizmo Physics Engine

Unlike many engines that rely on Rapier or Jolt, Gizmo features its own **custom-built, zero-dependency physics solver**.

### Rigid Bodies & Colliders
To make an object interact physically, attach a `RigidBody` and a `Collider`.

```rust
commands.spawn((
    Transform::from_xyz(0.0, 10.0, 0.0),
    RigidBody::Dynamic,
    Collider::cuboid(1.0, 1.0, 1.0),
    Velocity { linear: Vec3::ZERO, angular: Vec3::ZERO },
));
```

### The Physics Pipeline
1. **Sweep and Prune (Broad-Phase):** Quickly filters thousands of objects into potential collision pairs using Rayon multi-threading.
2. **GJK/EPA (Narrow-Phase):** Calculates exact penetration depths and contact points for convex shapes.
3. **Sequential Impulse Solver:** Solves overlapping constraints, friction, and joint constraints iteratively.

### Soft-Body Simulation (FEM)
Gizmo supports **Finite Element Method (FEM)** physics for realistic deformation (jelly, cloth, or vehicle crashing). Simply attach a `SoftBody` component with a volumetric tetrahedral mesh, and the engine calculates Neo-Hookean stress tensors natively.

---

## 4. Rendering Pipeline

Powered by **WGPU**, the renderer compiles to Vulkan, Metal, DX12, and WebGL2/WebGPU.

### Instanced Rendering
Gizmo automatically batches identical meshes with the same material into a single draw call. You can spawn 10,000 asteroids with minimal CPU overhead.

### PBR & Post-Processing
The engine defaults to a Physically Based Rendering (PBR) workflow. Features include:
- Screen Space Ambient Occlusion (SSAO)
- Cascaded Shadow Maps (CSM)
- Bloom & Tone Mapping

---

## 5. Animation System

Gizmo includes a skeletal animation pipeline tailored for fast-paced action games (like fighting games).

### Animation Transitions (Blending)
To prevent jerky movements, Gizmo supports dynamic crossfading between animation clips. You can programmatically set `blend_duration`:

```rust
// Snappy transition for attacks
player.blend_duration = 0.05; 
// Smooth transition for walking/idle
player.blend_duration = 0.18; 
```
