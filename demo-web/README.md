# demo-web — Gizmo Engine in the browser

The browser (WebGPU/WASM) counterpart of `demo/src/bin/3d_scene.rs`: the
same high-level `SimpleAppExt` API and the same engine core, compiled to
`wasm32-unknown-unknown`. The scene drops a stack of physics cubes onto a
ground disk so the live simulation loop is visible.

## Games in the browser

`?oyun=pinbol` opens the pinball game instead of the demo scene —
**https://bdrtr.github.io/Gizmo/?oyun=pinbol**. It is the same game the desktop
`pinbol` binary runs (`games/src/pinbol.rs`, one game, two hosts), with touch
flippers: on a phone the left and right halves of the screen are the two
flippers, and a tap also launches the ball and restarts a finished game.

The browser has no system fonts to borrow, so this crate turns on the engine's
`bundled-font` feature: Noto Sans Regular (SIL Open Font License 1.1, see
`crates/gizmo-renderer/assets/fonts/OFL.txt`) is built in, and text — Turkish
letters included — draws as on the desktop.

## Live demo

**https://bdrtr.github.io/Gizmo/** — no toolchain needed, just a WebGPU browser.

Published by `.github/workflows/pages.yml` on every push to `main` (and on
`workflow_dispatch`): it runs exactly the build below, then uploads
`index.html` + `pkg/` as a Pages artifact. The workflow pins `wasm-bindgen-cli`
to the version read out of the committed `Cargo.lock`, for the version-lock
reason described under Requirements.

> The link only serves once Pages is enabled for the repository
> (Settings → Pages → Source: **GitHub Actions**); until then the workflow
> builds fine and the deploy step fails at the API call.

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
(`gizmo_renderer::pipeline`'s `load_shader_web`). The post-process grade goes
with them: the browser runs a reduced `post_process_wasm.wgsl` with no depth of
field, chromatic aberration or film grain. Networking (UDP) and
Lua scripting are native-only for now, so this crate does not enable those two
features. **Audio is not in that group** — `demo-web` enables the `audio`
feature and the left-click beep above goes through the engine's own
`AudioManager`; what the browser adds is the autoplay policy, which is why it
takes a user gesture to start. See `docs/ENGINE.md` for the full status.

`test.html` is a headless verification harness (boots the engine, counts 90
frames, samples canvas pixels, reports to the serving process); `index.html`
is the page meant for humans.
