# assets/

Runtime assets for the demos. Only small, redistributable files are committed here.

## What is and is not in the repository

`.gitignore` excludes `*.glb`, so the large glTF models are **not** part of a clone.
What you get is the small committed set (`suzanne.obj`, `brick.jpg`, `grass.jpg`, the
KTX2 environment maps) plus the `.meta` sidecars that describe import settings for
models that are not shipped.

Every demo that wants a missing model degrades instead of crashing: it prints a
one-line note and falls back to procedural geometry. `cargo run --release -p demo
--bin car_demo` works on a fresh clone — you just get a box chassis rather than the
Kenney race car, and the vehicle dynamics the demo exists to show are unaffected.

## Supplying your own models

Drop a file here, or point `GIZMO_ASSETS` at a directory containing it:

```sh
export GIZMO_ASSETS=~/my-assets
cargo run --release -p demo --bin car_demo
```

Resolution order is `$GIZMO_ASSETS/<name>`, then `<repo>/assets/<name>`, then
`./assets/<name>` — see `demo::assets::find`.

Models the demos will pick up if present:

| File | Demo | Source |
|---|---|---|
| `raceCarRed.glb` | `car_demo` | [Kenney Racing Kit](https://kenney.nl/assets/racing-kit) (CC0) |
| `mercedes_amg_gt4__www.vecarz.com.glb` | `wind_tunnel` | third-party, not redistributable |

## Third-party content

The `.meta` sidecars in this directory reference models that were used during
development and are **not** covered by this repository's MIT/Apache-2.0 license —
several are third-party or brand-licensed content (vehicle models from vecarz.com,
game character rips). They are not distributed here, and none of them is required to
build or run anything. If you supply such a file locally, its own license governs it.

For assets you intend to redistribute with a project built on Gizmo, prefer CC0
sources such as [Kenney](https://kenney.nl) or [Poly Haven](https://polyhaven.com).
