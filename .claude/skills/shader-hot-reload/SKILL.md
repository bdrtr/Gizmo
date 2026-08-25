---
name: shader-hot-reload
description: Edit a WGSL shader and see it recompile in the running gizmo-studio, without restarting. Use when iterating on any renderer shader (post_process, deferred_lighting, csm, ssao, volumetric, irradiance, …) or when a shader change needs to be seen rather than reasoned about. Explains the disk-override mechanism, which is not discoverable from the shader source.
---

# Shader hot-reload (works, and is not discoverable)

Every renderer shader is compiled in with `include_str!`, but each one first tries to read a
**disk override** and falls back to the embedded copy:

```rust
let source = std::fs::read_to_string("demo/assets/shaders/post_process.wgsl")
    .unwrap_or_else(|_| include_str!("shaders/post_process.wgsl").to_string());
```

So: copy a shader out of `crates/gizmo-renderer/src/shaders/` into `demo/assets/shaders/`, edit it,
and the studio recompiles the pipelines while it runs — the studio watches `demo/assets`
recursively and calls `Renderer::rebuild_shaders()` on any `.wgsl` change. Verified end to end on
2026-08-16 by tinting `post_process.wgsl` mid-run and watching the frame turn green.

`demo/assets/shaders/` is deliberately **not** in the repository — see the note in `CLAUDE.md`.
Committing copies there would mean two versions of every shader free to drift, with the disk one
silently winning. Delete your override when you are done, or fold the change back into
`crates/gizmo-renderer/src/shaders/`.
