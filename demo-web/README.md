# demo-web — Gizmo Engine in the browser

The browser (WebGPU/WASM) counterpart of `demo/src/bin/bevy_3d_scene.rs`: the
same high-level `SimpleAppExt` API and the same engine core, compiled to
`wasm32-unknown-unknown`. The scene drops a stack of physics cubes onto a
ground disk so the live simulation loop is visible.

## Requirements

- The `wasm32-unknown-unknown` Rust target (`rustup target add wasm32-unknown-unknown`)
- `wasm-bindgen-cli` **at exactly the version this workspace resolves `wasm-bindgen`
  to** — the bindgen format is version-locked, so a mismatch fails with a clear
  "schema version" error. Find the resolved version and install the matching CLI:

  ```sh
  # exact version from the committed lockfile
  cargo metadata --format-version 1 \
    | grep -o '"wasm-bindgen@[0-9.]*"' | head -1
  # e.g. wasm-bindgen@0.2.126 → install the matching CLI:
  cargo install wasm-bindgen-cli --version 0.2.126
  ```
- A WebGPU-capable browser (Chrome/Edge stable, Firefox behind a flag)

## Build & run

From the repository root:

```sh
cargo build -p demo-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript \
    --out-dir demo-web/pkg \
    target/wasm32-unknown-unknown/release/demo_web.wasm
python3 -m http.server -d demo-web 8080
# → http://localhost:8080
```

Controls: hold right mouse button + move to look, `WASD` to move, `Shift` for
speed. Left-click starts the browser `AudioContext` and plays a beep (autoplay
policy requires a user gesture, so audio can only begin after the first click).

## What the web build does differently

Browser WebGPU exposes `maxBindGroups = 4`, so the web pipeline uses the
4-group scheme (`global`, `texture`, `skeleton`, `instance`) — shadows,
deferred shading, screen-space effects and GPU compute subsystems are disabled
on wasm and the forward shaders are rewritten at load
(`gizmo_renderer::pipeline`'s `load_shader_web`). Audio, networking (UDP) and
Lua scripting are native-only for now; the corresponding features are simply
not enabled by this crate. See `RELEASING.md` §4g for the full status.

`test.html` is a headless verification harness (boots the engine, counts 90
frames, samples canvas pixels, reports to the serving process); `index.html`
is the page meant for humans.
