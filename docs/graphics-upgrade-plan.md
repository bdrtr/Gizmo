# Graphics stack upgrade — `wgpu`/`winit`/`egui` (RELEASING §4c)

**Status: IN PROGRESS on branch `upgrade/graphics-stack` (not on `main`).** This is
the single Stage B 1.0 blocker (RELEASING.md §4c) — an **XL, multi-session** effort.
This doc records the resolved version matrix, the quantified break surface, the
execution order, and the per-dependency migration notes, so the work can proceed
crate-by-crate without re-discovering the landscape each time.

`main` stays green at the Stage A milestone (`c457be4`); all upgrade work lives on
the `upgrade/graphics-stack` branch until the whole stack compiles + runs.

---

## 1. Resolved version matrix (verified compatible by the resolver)

Bumping the pins below and running `cargo update` resolved **with no conflicts** —
this is the target matrix:

| Crate | From | To |
|---|---|---|
| `wgpu` | 0.20.1 | **29.0.3** |
| `winit` | 0.29.15 | **0.30.13** |
| `egui` | 0.28.1 | **0.34.3** |
| `egui-wgpu` | 0.28.1 | **0.34.3** |
| `egui-winit` | 0.28.1 | **0.34.3** |
| `egui_dock` | 0.13.0 | **0.19.1** |
| `transform-gizmo-egui` | 0.3.0 | **0.9.0** |

Notes:
- `winit 0.31` exists only as `0.31.0-beta.2`; we target stable **0.30.13**.
- `egui-wgpu 0.34` pulls `wgpu 29` and `egui-winit 0.34` pulls `winit 0.30`, so the
  egui ecosystem, wgpu, and winit versions are mutually consistent at these pins.
- The old transitive `wgpu 23.0.1` (from `transform-gizmo` 0.3) collapses into the
  single `wgpu 29.0.3` after the bump.

## 2. Break surface (quantified, `cargo check` per crate)

| Crate | Errors | Dominant cause |
|---|---|---|
| `gizmo-window` | 2 → **0 ✅** | winit `WindowBuilder`→`ApplicationHandler` (DONE) |
| `gizmo-physics-soft` (`gpu_physics`) | 9 | wgpu compute pipeline/buffer descriptors |
| `gizmo-renderer` | 283+ | wgpu 0.20→29 (155 mismatched types, 67 removed struct fields, 44 new required fields, 11 moved types) |
| `gizmo-app` | 283+ | winit `ApplicationHandler` event loop + wgpu surface/device |
| `gizmo-editor` | 283+ | egui 0.28→0.34 + egui-wgpu/egui-winit + egui_dock 0.19 + transform-gizmo 0.9 + wgpu |
| `gizmo` / `demo` / `cradle` / `gizmo-studio` | (transitive) | depend on the above |

Total ≈ **850+ errors**. `283` is cargo's per-crate error cap, so renderer/app/editor
each have *at least* that many. This is the bulk of the work.

## 3. Execution order (dependency-topological, smallest blast radius first)

1. **`gizmo-window`** — winit only. ✅ **DONE** (reference for the winit 0.30 pattern).
2. **`gizmo-physics-soft` (`gpu_physics`)** — wgpu only, feature-gated, small (9). Good
   second step to nail the wgpu 29 compute/buffer pattern in isolation.
3. **`gizmo-renderer`** — the wgpu core (283+). The largest single piece; everything
   visual depends on it. Migrate device/queue/surface/pipeline/bind-group/render-pass.
4. **`gizmo-app`** — winit `ApplicationHandler` event loop + wgpu surface acquisition.
   Depends on the renderer's new API.
5. **`gizmo-editor`** — egui 0.34 + egui-wgpu/egui-winit 0.34 + egui_dock 0.19 +
   transform-gizmo-egui 0.9. Depends on renderer + app.
6. **`gizmo` (facade)**, then **`demo` / `cradle` / `gizmo-studio`** — re-exports +
   binaries; mostly fall out once 3–5 compile, plus any direct winit/wgpu calls.

## 4. Per-dependency migration notes

### winit 0.29 → 0.30 (pattern established in `gizmo-window`)
- `WindowBuilder` is **gone**. Use `Window::default_attributes()` → `WindowAttributes`,
  then `active_event_loop.create_window(attributes) -> Result<Window, OsError>`.
- Windows can only be created **once the loop is active** — i.e. inside
  `ApplicationHandler::resumed(&mut self, &ActiveEventLoop)`, not from `&EventLoop`.
- `event_loop.run(closure)` is deprecated → `event_loop.run_app(&mut app)` where
  `app: impl ApplicationHandler`. The old `Event::WindowEvent{..}` / `Event::AboutToWait`
  arms become the `window_event(..)` / `about_to_wait(..)` trait methods.
- Trait methods can't return `Result`; capture errors in a field and surface them after
  `run_app` returns (see `WindowApp::deferred_error`).
- **Impact:** `gizmo-app`'s event loop is the big one — its whole run-loop becomes an
  `ApplicationHandler`, and window/surface/renderer init moves into `resumed`.

### wgpu 0.20 → 29 (nine major versions — the dominant cost)
Expect, from the error categories:
- **Descriptor field churn** (67 "no field" + 44 "missing field"): `InstanceDescriptor`,
  `RequestAdapterOptions`, `DeviceDescriptor` (e.g. `required_features`/`required_limits`,
  `memory_hints`, `trace`), `SurfaceConfiguration` (`desired_maximum_frame_latency`),
  `RenderPipelineDescriptor`/`PrimitiveState`/`TextureDescriptor` new/renamed fields.
- **`RenderPass` lifetime**: since wgpu 22 the render pass borrows the encoder and is
  `'encoder`-scoped; `RenderPassDescriptor` got `timestamp_writes`/`occlusion_query_set`.
- **`Instance::new` takes `&InstanceDescriptor`**; adapter/device request APIs changed
  (some now non-`async`/return directly in 25+).
- **155 mismatched types**: knock-on from the above (e.g. `wgpu::Color`, `TextureFormat`,
  `Buffer`/`BindGroup` typing, `SurfaceTexture` acquisition).
- naga/WGSL validation is stricter — shaders may need small fixes (the `core_shaders_compile`
  test will catch these).

### egui 0.28 → 0.34
- egui-wgpu `Renderer`/`ScreenDescriptor` and the render-pass integration changed (it now
  records into a `wgpu::RenderPass` with the new lifetime).
- egui-winit `State::new`/`on_window_event` signatures changed (winit 0.30 `WindowEvent`).
- `egui_dock 0.19`: `DockArea`/`TabViewer` API changed vs 0.13.
- `transform-gizmo-egui 0.9`: gizmo config/interaction API changed vs 0.3.
- Misc egui renames (`Context`, `Visuals`, `Frame`, `RichText`, layout helpers).

## 5. Verification gate (same as Stage A)
Per crate as it lands, then the whole workspace at the end:
`cargo build --workspace` + `--all-features` + `cargo test --workspace` +
`cargo clippy --workspace -- -D warnings -A too_many_arguments -A type_complexity`
(stable) + determinism 3/3 + **a real windowed run** (`demo`/`gizmo-studio`) since most
of this is GPU/window code that headless tests don't cover.

## 5b. wgpu 0.20 → 29 field-change cheatsheet (authoritative, from wgpu-29.0.3 source)

Derived by reading `~/.cargo/.../wgpu-29.0.3` + `wgpu-types-29.0.3`. Apply these
across all wgpu files (renderer is the bulk; gpu_physics/gpu_fluid/gpu_particles
under it; physics-soft `gpu_physics`).

**Type renames (done in renderer via sed):**
- `wgpu::ImageCopyTexture` → `wgpu::TexelCopyTextureInfo`
- `wgpu::ImageDataLayout` → `wgpu::TexelCopyBufferLayout`
- `wgpu::Maintain` → `wgpu::PollType` (variants: `Maintain::Wait` → `PollType::Wait`, etc.)

**Struct field changes:**
- `PipelineLayoutDescriptor`: drop `push_constant_ranges` → add `immediate_size: u32`
  (renderer uses no push constants → `immediate_size: 0`). **`bind_group_layouts`
  is now `&[Option<&BindGroupLayout>]`** — wrap every entry: `&[&a, &b]` → `&[Some(&a), Some(&b)]`.
  (This is the bulk of the "155 mismatched types".)
- `RenderPipelineDescriptor`: `multiview: None` → `multiview_mask: None` **and add `cache: None`** (last field).
- `ComputePipelineDescriptor`: **add `cache: None`** (last field, after `compilation_options`).
- `RenderPassColorAttachment`: **add `depth_slice: None`** (right after `view:`).
- `RenderPassDescriptor`: **add `multiview_mask: None`** (last field).
- `TextureViewDescriptor`: **add `usage: None`**.
- `DeviceDescriptor`: **add `experimental_features: wgpu::ExperimentalFeatures::default()`,
  `memory_hints: wgpu::MemoryHints::default()`, `trace: wgpu::Trace::Off`** (and
  `required_features`/`required_limits` stay).
- `InstanceDescriptor`: no longer `Default` via `..Default::default()` in all spots
  — construct explicitly (`backends`, `flags`, `memory_budget_thresholds`,
  `backend_options`, `display`) or use `InstanceDescriptor::default()` where it impls it.
- Adapter enumeration: `instance.enumerate_adapters(..)`/`request_adapter` now returns
  a `Future<Output = Vec<Adapter>>` in places — `.await` it (renderer.rs adapter setup).

**Status:** renames + `immediate_size` applied in renderer (283 → 235 errors). Remaining
in renderer: the `cache`/`depth_slice`/`usage`/`multiview_mask` field additions (~75) and
the `bind_group_layouts` `Some(..)` wrapping (~155 mismatched), then DeviceDescriptor/
adapter/InstanceDescriptor in `renderer.rs`.

## 6. Progress log
- 2026-06-25: branch created; version matrix bumped + resolved (no conflicts); break
  surface quantified; **`gizmo-window` migrated to winit 0.30 (ApplicationHandler) —
  green.**
- 2026-06-25: **`gizmo-renderer` fully migrated to wgpu 29 — green (283 → 0 errors,
  clippy clean).** Applied the §5b cheatsheet across ~20 files: type renames, the
  `bind_group_layouts` `Some(..)` wrapping (single-line via sed, multi-line via a
  perl-slurp pass), `entry_point` → `Some(..)`, `mipmap_filter` `FilterMode` →
  `MipmapFilterMode`, `depth_write_enabled`/`depth_compare` → `Option`, the
  `cache`/`depth_slice`/`usage`/`multiview_mask` field additions (anchored seds:
  `multiview_mask` for render-pipeline cache, `compilation_options`+`})` lookahead for
  compute cache, `resolve_target` for depth_slice, `TextureViewDescriptor {`-scoped
  perl for usage), and the `renderer.rs` GPU init (`Instance::new` explicit descriptor,
  `enumerate_adapters().await`, `request_adapter` now returns `Result` → `Ok/Err`,
  `request_device` drops its 2nd arg, `DeviceDescriptor` +3 fields).
- 2026-06-25: **`gizmo-physics-soft` `gpu_physics` migrated to wgpu 29 — green (9 → 0).**
  Same patterns + `request_adapter` `.ok_or` → `.map_err`, `Maintain::Wait` →
  `PollType::Wait { submission_index: None, timeout: None }`.
- **Remaining: `gizmo-app` (winit ApplicationHandler + wgpu surface), `gizmo-editor`
  (egui 0.34 ecosystem + wgpu), facade + `demo`/`cradle`/`gizmo-studio`, then a real
  windowed run.**

### Reusable sed/perl recipes (for app/editor/binaries)
```
# type renames
ImageCopyTexture→TexelCopyTextureInfo, ImageDataLayout→TexelCopyBufferLayout, Maintain→PollType
# pipeline layout
push_constant_ranges: &[]  →  immediate_size: 0
bind_group_layouts: &[&a, &b]  →  &[Some(&a), Some(&b)]   (perl-slurp for multi-line blocks)
# shader stages / pipelines
entry_point: "x"  →  entry_point: Some("x")
+ cache: None  (render pipeline after multiview:None→multiview_mask:None; compute after compilation_options)
# depth-stencil
depth_write_enabled: X → Some(X);  depth_compare: CompareFunction::X → Some(..)
mipmap_filter: FilterMode:: → MipmapFilterMode::
# render pass / attachments / views
+ depth_slice: None (color attachment, before resolve_target)
+ multiview_mask: None (render pass, after occlusion_query_set)
+ usage: None (TextureViewDescriptor, before aspect)
# device/instance/adapter
Instance::new explicit descriptor (no Default);  request_adapter → Result (Ok/Err, .map_err);
request_device drops 2nd arg;  DeviceDescriptor += experimental_features/memory_hints/trace;
PollType::Wait { submission_index: None, timeout: None }
```
