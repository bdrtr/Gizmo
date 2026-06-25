# Migrating from Gizmo `0.1.x` to `0.2.0`

`0.2.0` is the first release since `0.1.7`. It bundles the multi-round
1.0-readiness hardening and the graphics-stack upgrade into one breaking `0.x`
bump (the staged `1.0` plan is deferred — see [`RELEASING.md`](../RELEASING.md)).
This guide lists every user-visible breaking change and how to adapt. See the
[`CHANGELOG`](../CHANGELOG.md) for the full list.

> **TL;DR.** Bump your toolchain to **Rust 1.92+**, migrate `wgpu`/`winit`/`egui`
> usage to the new versions, handle the new `Result`/`Option` returns from
> constructors/loaders, set contact friction/restitution on the **collider
> material** instead of the body, and (if you relied on `bevy_reflect`) turn on
> the new `reflect` feature.

---

## 1. Toolchain: MSRV is now 1.92

The graphics upgrade pulls `egui 0.34`, whose floor is **Rust 1.92**. Update your
toolchain (`rustup update`) — `1.75`–`1.91` will no longer build the engine.

## 2. Graphics stack: `wgpu 0.20 → 29`, `winit 0.29 → 0.30`, `egui 0.28 → 0.34`

If you depend on the **Stage B** crates (`gizmo-renderer`, `gizmo-window`,
`gizmo-editor`, `gizmo-ui`, `gizmo-app`, `gizmo-scripting`, or the `gizmo`
facade), the `wgpu`/`winit`/`egui` types in their public API move to the new
versions. The big-ticket renames you will hit:

- **wgpu:** `ImageCopyTexture → TexelCopyTextureInfo`, `ImageDataLayout →
  TexelCopyBufferLayout`, `Maintain → PollType`; bind-group layouts are now
  `&[Option<&BindGroupLayout>]`; render/compute pipeline descriptors take
  `cache: None`; color attachments take `depth_slice: None`;
  `DeviceDescriptor` gained `memory_hints`/`trace`; `request_adapter` returns a
  `Result`; `set_bind_group` takes `impl Into<Option<&BindGroup>>`.
- **winit:** `WindowBuilder → WindowAttributes`; the event loop moved toward the
  `ApplicationHandler`/`run_app` model (Gizmo still uses the supported closure
  bridge internally — see [`docs/graphics-upgrade-plan.md`](graphics-upgrade-plan.md)).
- **egui:** `Frame::none() → Frame::new()`, `Ui::close_menu() → ui.close()`,
  `ComboBox::from_id_source → from_id_salt`, `Context::{begin_frame → begin_pass,
  end_frame → end_pass, style → global_style, set_style → set_global_style,
  screen_rect → content_rect}`, `Panel::exact_height → exact_size`,
  `TopBottomPanel → Panel::{top,bottom}`, and top-level `Panel::show(ctx)` →
  root-`Ui` composition via `show_inside` (see §8).

`gizmo-math`, `gizmo-core`, the `gizmo-physics-*` crates, `gizmo-scene`,
`gizmo-net`, `gizmo-audio`, and `gizmo-ai` (the **Stage A** core) do **not** pull
any of these — only the graphics layer is affected.

## 3. `bevy_reflect` is now behind an off-by-default `reflect` feature

`gizmo-core`, `gizmo-physics-core`, `gizmo-physics-rigid`, and `gizmo-scene` no
longer derive `Reflect` / expose `bevy_reflect` by default. Scene save/load and
snapshots fall back to plain `serde` (every component still derives
`Serialize`/`Deserialize`), which is fully functional and round-trip-tested.

- **If you only save/load scenes:** nothing to do — it works out of the box.
- **If you used the typed reflect (de)serializers or `ComponentRegistry`'s
  reflect methods** (`register_reflect`, `get_reflect_ptr_fn`, …): enable the
  feature:
  ```toml
  gizmo-scene = { version = "0.2", features = ["reflect"] }
  ```
  The `reflect` feature chains down the dependency graph and also enables
  `bevy_reflect`'s `glam` integration.

## 4. `CollisionEvent.contact_points` is now an opaque `ContactPoints`

`arrayvec::ArrayVec` no longer leaks through the public API.
`CollisionEvent.contact_points` is a `gizmo_physics_core::collision::ContactPoints`
newtype — use `len`/`is_empty`/`first`/`iter`/`as_slice`, indexing, or
`IntoIterator` (by value/ref) instead of `ArrayVec` methods.

## 5. Constructors and loaders now return `Result`/`Option`

These no longer panic on failure — handle the result (`?`, `.expect(...)`, or a
match):

- `App::run`, `AppWindow::new`
- `ComponentRegistry::register`
- `SceneData::save` / `SceneData::load*`
- `AudioManager::new` / `play*`
- `NetworkClient::new` / `NetworkServer::new`
- `spawn_gltf`, renderer `load_*`, `trigger_snapshot`

```rust
// before
App::new(...).run();
// after
App::new(...).run().expect("failed to run the app");   // or `?`
```

## 6. Infallible `get_` getters dropped the prefix

Per the Bevy convention (`get_` is kept for *fallible* accessors that return
`Option`/`Result`), only the genuinely infallible plain-value getters were
renamed:

| before | after |
|---|---|
| `get_neighbors` | `neighbors` |
| `get_entity_component_types` | `entity_component_types` |
| `get_log_version` | `log_version` |
| `get_engine_torque` | `engine_torque` |
| `get_entity_names` | `entity_names` |

`get_resource`, `get_entity`, `get_component_ptr`, etc. (which return
`Option`/`Result`) are unchanged.

## 7. `RigidBody`: friction/restitution moved to the collider material

`RigidBody` no longer has `friction` / `restitution` fields, and the constructor
is now `RigidBody::new(mass, use_gravity)`:

```rust
// before
RigidBody::new(mass, restitution, friction, use_gravity);
// after
RigidBody::new(mass, use_gravity);
```

These fields were **never read** by the contact solver — friction/restitution
have always come from the colliders' `PhysicsMaterial` (combined per contact).
Set them there:

```rust
let material = PhysicsMaterial { dynamic_friction: 0.8, restitution: 0.2, ..Default::default() };
let collider = Collider::box_collider(half_extents).with_material(material);
```

**Scripting:** the Lua binding changed to
`physics.add_rigidbody(id, mass, use_gravity)` (dropped the ignored
`restitution`/`friction` arguments).

## 8. Embedding the editor: `draw_editor` now takes a root `Ui`

If you drive `gizmo-editor` yourself (instead of using `gizmo-studio`), the
top-level panel functions moved to egui 0.34's root-`Ui` composition model:

```rust
// before
gizmo::editor::draw_editor(ctx, world, &mut editor_state);
// after — build a full-viewport background Ui, then compose into it
let mut root = egui::Ui::new(
    ctx.clone(),
    egui::Id::new("gizmo_editor_root"),
    egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.content_rect()),
);
root.set_clip_rect(ctx.content_rect());
gizmo::editor::draw_editor(&mut root, world, &mut editor_state);
```

`draw_toolbar` likewise takes `&mut egui::Ui` instead of `&egui::Context`.

## 9. `#[non_exhaustive]` on public enums/structs

Many error/shape/event enums and `Default`-able config structs are now
`#[non_exhaustive]`. You can no longer match them without a `_ => …` arm or
construct the structs with a bare literal — use the provided constructors /
`..Default::default()`.

## 10. `glam` is the math vocabulary (unchanged types)

`gizmo-math` now re-exports `glam` directly (it previously went through
`bevy_math`, which re-used the *same* `glam` types). `Vec3`/`Quat`/`Mat4`/… are
unchanged; `glam` is documented as an official public dependency on the `0.29`
line. As a bonus, `bevy_reflect` no longer compiles in the Stage A dependency
tree by default.

---

## Still on `0.1.x`?

The `0.1.x` series stays available on crates.io. `0.2.0` is a clean break; there
is no compatibility shim. If you only use the **Stage A** core (ECS + math +
physics + scene + net + audio + AI), most of the above (§2, §8) does not apply to
you — the main changes are §3 (reflect feature), §5 (Result returns), §6 (getter
names), and §7 (RigidBody).
