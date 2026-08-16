# Gizmo Engine — Engineering Document

> The **single** internal reference document for the engine: architecture, live roadmap, release
> strategy, determinism/migration contracts, closed research and the working method.
> The user-facing introduction is in `README.md`, the version history in `CHANGELOG.md`.
>
> In the 2026-07 consolidation this document merged 12 separate plan/FIXPLAN/reference files; the
> detailed narrative of finished work was pruned, the durable decisions/lessons were kept.

---

## 1. Overview

Gizmo — a lightweight, **pure-Rust**, ECS-based 3D engine + a physics simulator written from
scratch (no external physics dependency). Published on crates.io (**0.9.0**, 19 crates).

- **ECS:** Entity = id, Component = data, Systems query Archetypes. `World` is the central
  state; `Query`/`Mut`/`With`/`Without`/`Changed`/`Added` filters; `Commands` for deferred
  structural changes; Table + SparseSet storage.
- **Physics:** rigid (TGS-Soft solver), soft-body (FEM/cloth/rope), fracture, joints,
  vehicle/character dynamics, CCD, GJK/EPA narrowphase, BVH broadphase.
- **Renderer:** WGPU deferred PBR + shadows/SSAO/SSGI/volumetric/TAA; egui HUD/editor.
- **Platform:** native + WASM (the sim core + renderer + window run in the browser).

---

## 2. Architecture (20 crates — stable)

Clean bottom-up layering, NO circular dependencies:

```
gizmo-math ─┬─ gizmo-core ─┬─ gizmo-physics-{core,rigid,dynamics,soft}
            │              ├─ gizmo-renderer ─ gizmo-{window,ui,editor}
            │              ├─ gizmo-{scene,net,ai,animation,audio,scripting}
            └──────────────┴─ gizmo-app ─ gizmo (facade) ─ demo/
```

**Refactor contract (from the god-file splitting rounds, completed):** pure/verbatim moves only
(no logic edits in the same step), `pub use` re-export from the original path (call sites do not
change), every step verified with build+test+clippy. 10 mega-files were split; the determinism
hash did not change. OPEN (optional, behavior-adjacent, not a pure move): splitting the
still-large functions such as `update_vehicle` / `execute_render_pipeline` — as needed.
<!-- TRANSLATOR NOTE: "davranış-bitişik" is a coinage; read as "adjacent to behavior", i.e. such a split can alter behavior and therefore falls outside the pure-move contract above. -->

---

## 3. Roadmap (LIVE — remaining work only)

> **2026-08-04:** An independent whole-engine audit was carried out → `docs/AUDIT-2026-08.md`
> (every finding backed by `file:line` evidence; 9 claims that could not survive adversarial
> verification were removed). The resulting work is being executed in `docs/FIXPLAN.md` — once
> that campaign ends, the durable decisions move here and FIXPLAN is deleted. The Phase 6/7 items
> below remain valid; the audit added a Phase A (honesty + security) that comes **before** them.

Phases 0–5 (stabilization, tests+CI, determinism, P2P rollback netcode, physics depth,
renderer/WASM/editor) are **DONE**. Remaining:

**Phase 6 — API stability & 1.0 mechanics**
- Freeze the public API + document the `unsafe` contracts.
- Staged 1.0 (see §4): Stage A core at 1.x, Stage B graphics layer at 0.y.
- The `RigidBody::friction`/`restitution` fields are **IGNORED** by the contact solver
  (the source is the collider material) → bridge them or remove them? (An API decision;
  bridging shifts the defaults.)

**Phase 7 — Product layer (a shippable game)**
- M7.4 authoritative client-server netcode; M7.5 audio mixer/bus/DSP; M7.6 UI font/text/
  widget/z-index; M7.7 WASM feature parities, editor panels + AssetServer hot-reload.
- 1.0 CI gates: rustfmt / `missing_docs` / coverage / cargo-deny / benchmark regression.
- Optional: cross-platform determinism (as a feature — see §5), gizmo-net on WASM.
- Gated on human-eye A/B: the textured-glTF `material_demo` asset, `car_demo` driving/geometry.
<!-- TRANSLATOR NOTE: "İnsan-gözü A/B gated" is a terse fragment; read as "these items can only be signed off by a human comparing before/after side by side". -->

---

## 4. Release Strategy — Staged 1.0

1.0 = the hard promise of "no breaking change without a 2.0". What decides whether a crate can
make that promise is not our own code but our loudest dependency: if a foreign type is reachable
from our public API, that dependency's next major is our next major. A lock-step 1.0 across all
crates would therefore either freeze the engine on old deps or burn the 1.0 at the first dep bump.
The solution is **staged**.

**The Stage A criterion — RESTATED 2026-08-09.** This used to read: "a crate that re-exports a 0.x
dependency (wgpu/winit/egui, bevy_reflect) in its public API cannot make that promise." That
literal test is wrong in three ways, and the eleven-crate surface audit of 2026-08-09 found what
it had been letting through:

1. **It is the CADENCE that disqualifies, not the version number.** The property being tested for
   is *a history of frequent majors*; `0.x` is the common case, not the definition. `wgpu` is the
   counter-example the old sentence named as its own archetype and then failed to catch: it
   resolves at **29.0.3**, so it passes a literal "is it 0.x?" test, and it has historically
   shipped on the order of a major per quarter — this engine's own 0.2.0 upgrade crossed
   **0.20 → 29** in one release (§6). A version number ≥ 1.0 buys nothing when the next integer is
   a quarter away. The converse holds too: a frozen 0.x costs nothing, which is why `tracing` 0.1
   is an unconditional dependency below.
2. **"Re-exports" is too narrow — the test is REACHABILITY.** A foreign type is on the surface if
   a downstream crate can be handed it or forced to name it: `pub use`, yes, but equally a public
   field, a parameter or return type, an associated type in one of our trait impls, or a foreign
   trait implemented on one of our public types. `arrayvec` was declared clean in 0.2.0 on the
   strength of the storage field having been made private, while
   `<ContactPoints as IntoIterator>::IntoIter` went on reading `arrayvec::IntoIter<ContactPoint, 4>`
   — an associated type nobody had to write down, and one that could not be changed after 1.0
   either. That is the leak the old wording let through, and it survived until 2026-08-09.
3. **The promise is per FEATURE CONFIGURATION — the old rule did not mention features at all.**
   1.0 covers the DEFAULT feature set. A feature may put a fast-moving dependency back on the
   surface, but only if it is default-OFF *and* named in the contract below as an explicit opt-out
   of the stability promise for that crate. Today those are `reflect`, `tracing-layer`,
   `gpu_physics` and `client-server`.

The staging itself:

- **Stage A (may go 1.x):** dependency-light crates whose surface we own —
  gizmo-math, -core, -physics-{core,rigid,dynamics,soft}, -scene, -net, -audio, -ai,
  -animation. "May go" is a candidacy, not a clearance. **gizmo-physics-rigid was BLOCKED on the
  `rustc-hash` leak; that block was CLEARED 2026-08-09** by the `EntityIndexMap` seal in the
  contract below, so the crate is now a candidate on the same footing as the rest — which still
  means the accepted `glam` cost, not a clean surface.
- **Stage B (stays 0.y):** graphics/integration — gizmo-renderer, -window, -editor, -ui,
  -app, -scripting + the `gizmo` facade (until wgpu/winit/egui settle — by criterion 1 that means
  a slow major cadence, NOT merely a version number ≥ 1.0; wgpu is already at 29).
- **Consequence:** once staging begins the crates no longer share a SINGLE workspace version
  (`publish_all.sh` + the version-inheritance assumption must be updated).

**External-type contract (permanent).** One entry per dependency that is, was, or could be on a
Stage A public surface. Where an entry corrects an earlier claim of this document, the earlier
claim is quoted and marked wrong rather than quietly overwritten — the value of this section is
its precision, and a section that silently rewrites itself cannot be checked.

- **`glam` — a permanent, DELIBERATE public dep, and the largest semver liability in Stage A.**
  `gizmo-math` re-exports it; resolved version **0.32.1**, i.e. 0.x, and the `.32` says plainly
  how often it breaks. The 2026-08-09 audit put it on the DEFAULT public surface of **8 of the 11
  Stage A crates** — all of them except `gizmo-core`, `gizmo-audio` and `gizmo-net` (audio does
  not depend on `gizmo-math` at all; net reaches it only through the optional, default-off
  `rollback` feature). Stated as the cost rather than as the intent: **a glam 0.33 is a breaking
  change for eight Stage A crates at once**, and under the 1.0 promise that means eight 2.0s
  released together. We accept it — the alternative is our own vector/quat types plus a conversion
  layer at every boundary, which is worse for users and for the determinism contract (§5) — but it
  is an accepted cost, not a neutral one, and it is the one item in this list that sealing cannot
  fix.

- **`arrayvec` — sealed 2026-08-09. THE EARLIER CLAIM HERE WAS FALSE.** This section used to say
  "`arrayvec` was removed from the public API (opaque `ContactPoints`)", and §6 listed "`arrayvec`
  left the public surface" among the 0.2.0 API breakers. Neither was true. 0.2.0 made the
  `ArrayVec` storage field of `ContactPoints` private — that is what those sentences were written
  from — but `impl IntoIterator for ContactPoints` still declared
  `type IntoIter = arrayvec::IntoIter<ContactPoint, 4>`: a 0.7.x type on the default surface of a
  Stage A crate, reached by criterion 2 above and not by criterion "re-export". What is true now:
  the by-value iterator is the opaque `collision::ContactPointsIter` (private field; `Iterator`,
  `DoubleEndedIterator`, `ExactSizeIterator` forwarded, `FusedIterator` asserted by us because
  arrayvec 0.7 omits it), re-exported at the crate root beside `ContactPoints`. The by-ref impl
  was verified rather than assumed — it already yielded `std::slice::Iter`. Both associated types
  are pinned by always-on `const _` fn-pointer coercions in `collision.rs`, so a future widening
  fails the build rather than the review. `arrayvec` itself stays as a private implementation
  detail (the fixed-capacity storage).

- **`ron` — sealed out of gizmo-scene's public API (2026-08-09); the earlier entry UNDERSTATED the
  leak.** It was recorded here as "the RON file format + the `SceneError` API". In fact
  `pub use ron;` re-exported the parser's ENTIRE surface as `gizmo_scene::ron`, and the facade
  re-exported that again as `gizmo::ron` — so a ron 0.13 broke us whether or not `SceneError`
  moved. Now the two RON-shaped failures are the opaque `error::ParseError` /
  `error::SerializeError` (private payloads, `Display` and `Error::source` forwarded verbatim,
  `line()`/`column()` handed back), and `SceneData::from_ron_str` /
  `to_ron_string` replace the "call `ron::from_str` yourself" workflow. **"No capability is lost"
  was the claim here and it is too strong — corrected 2026-08-09 by review:** the position comes
  back, but the parser's error *kind* (its `code` enum, matchable before), the *end* of the error
  span, and `Clone`/`PartialEq` on the payload do not, and `source().downcast_ref::<ron::…>()`
  stops matching. All four are accepted costs — every replacement for them would put a parser
  type back on the surface — and they are now spelled out on `ParseError` itself rather than
  glossed. `gizmo::ron` went with it
  (Stage B, but keeping it would have meant a second ron pin to hold in lock-step). The only
  remaining mention of ron in the surface is the two `From` impls on `SceneError` that make `?`
  work inside `scene.rs` — deliberately kept, because they force no ron type into any caller's own
  signature: the cost of a ron major is rewriting two `from` bodies here, not everyone's error
  handling. Resolved at 0.12.2.
  **The condition the seal was accepted under, and it is a promise rather than a note:** *1.0
  freezes the Rust API; it does NOT freeze the RON scene format.* The file format is versioned
  separately, through the existing mechanism — `scene::CURRENT_SCENE_VERSION` plus
  `SceneData::migrate`, which brings an older file forward and REFUSES a file written by a newer
  engine with `SceneError::UnsupportedVersion` instead of silently truncating it. Without this
  sentence the seal would buy a 1.0 that quietly reserves the right to change its own on-disk
  format underneath a frozen API.

- **`web-time` — removed from gizmo-scene's public API 2026-08-09; it was MISSING from this
  contract entirely** and nobody had noticed it before the audit. `SceneSnapshot::timestamp` was
  `pub`, and on `wasm32` the type of that field is `web_time::Instant`, not `std::time::Instant`.
  Because the substitution is target-gated rather than feature-gated it sat on the default wasm
  surface with no way to opt out — precisely the shape the old rule was blind to, since nothing
  was re-exported and nothing looked 0.x at the call site. The field is now private;
  `age() -> std::time::Duration` was already its only reader anywhere in the workspace, so no
  capability was lost. In the same pass the pin went `0.2` → `1` in all seven crates that declare
  it, which also dropped the duplicate 0.2.4 build; **1.1.0** is now the only version resolved.

- **`crossbeam-queue` — sealed out of gizmo-core's public API (2026-08-09).** Opaque
  `asset::AssetDropQueue` newtype: no public constructor, no accessor, no public field, and a
  hand-written `Debug` so the wrapped type does not surface in rendered docs or in any containing
  `{:?}`. `HandleIdTracker::drop_queue` is private and typed as the newtype, which also closes the
  transitive path through the public `Handle::tracker`. The dependency itself stays — it is
  genuinely used by `asset.rs` and `commands.rs`; the seal hides it, it does not remove it.

- **`rustc-hash` — sealed out of `gizmo-physics-rigid`'s public API (2026-08-09). THE EARLIER
  ENTRY HERE IS SUPERSEDED, and it is worth keeping why it existed.** It read: "STILL ON THE
  DEFAULT PUBLIC SURFACE of `gizmo-physics-rigid`. NOT sealed, and it was missed by the
  2026-08-09 audit entirely… Until this is sealed, `gizmo-physics-rigid` cannot honestly go 1.0."
  That was accurate when written — the leak was found by the adversarial review of the audit's
  own change sets, not by the audit. `FxHashMap<K, V>` is a type *alias*,
  `HashMap<K, V, FxBuildHasher>`, so the hasher — a `rustc-hash` type, resolved at **2.1.2** —
  travelled with every use of it, and a type alias hides nothing: it is exactly criterion 2's
  "an associated type nobody had to write down", one level of indirection further out. The
  cadence argument is what made it urgent rather than theoretical: rustc-hash 1.x → 2.0 **changed
  exactly this type**, from `HashMap<K, V, BuildHasherDefault<FxHasher>>` to
  `HashMap<K, V, FxBuildHasher>`, so the precedent for a rustc-hash 3.0 breaking us was the last
  major it shipped. Both 1.1.0 and 2.1.2 are still in `Cargo.lock` (something else in the graph
  wants 1.x).

  What is true now: `world::EntityIndexMap` is an opaque newtype over the map — private field, no
  `Deref`/`DerefMut`/`AsRef`/`Borrow`/`From`/`Into`, no public accessor, and a hand-written
  `Debug` (the `AssetDropQueue` precedent above; a derive would have printed the entries in hash
  order, which is non-deterministic output from a type inside a determinism-critical struct). It
  is `#[serde(transparent)]`, so `PhysicsWorld`'s snapshot JSON is byte-identical to before. All
  three reachable sites now name it instead:
  - `world/mod.rs` — `PhysicsWorld::entity_index_map` stays a **public field**, retyped. Read
    access was deliberately kept rather than privatised: `get`, `contains_key`, `len` and
    `is_empty` mirror the `HashMap` methods exactly, so the ~57 in-crate call sites and any
    downstream reader compile unchanged.
  - `solver/mod.rs` — `ConstraintSolver::solve_contacts(.., &world::EntityIndexMap, ..)`.
  - `joints/solver/mod.rs` — `JointSolver::solve_joints(.., &world::EntityIndexMap, ..)`.

  Two sites the audit list also flagged, `solver/mod.rs:48` (`support_order_manifolds`) and
  `solver/tgs.rs:115` (`solve_contacts_tgs`), were checked and are **private** and `pub(super)`
  respectively — not reachable, so they still take the bare `FxHashMap` and are handed it by a
  `pub(crate) raw()` on the newtype. `rustc-hash` itself stays as the implementation detail it
  always should have been.
  **Capabilities, stated as costs rather than glossed:** *editing a world's map entry by entry*
  from outside the crate is gone (`insert`/`remove`/`clear` are `pub(crate)`) — it was never safe,
  since the map has to stay in lockstep with the SoA arrays, and
  `add_body`/`remove_body_at`/`sync_bodies`/`clear_bodies` are the supported routes. Note the
  narrowness, because an earlier draft of this entry overclaimed it: the field stays `pub` and
  `Default`/`Clone`/`FromIterator`/`Deserialize` are public, so `world.entity_index_map = …` still
  replaces the map wholesale and still breaks lockstep. The invariant is a convention, exactly as
  it was before the seal; only the hasher was sealed. Enforcing it would mean privatising the
  field, which this entry deliberately does not do. *Iterating* it is gone too, and deliberately: hash
  order is not in the determinism contract (§5) and nothing in the crate iterates it, so the
  omission is free — iterate the `Vec` `PhysicsWorld::entities` and look each handle up.
  *Constructing* one is NOT gone: `impl FromIterator<(u32, usize)>` keeps the bare
  `solve_joints`/`solve_contacts` embedding path (no `PhysicsWorld` involved) open without naming
  the hasher. This is a **breaking change** for anyone who read the field as a `HashMap`; see the
  CHANGELOG migration note.
  The seal is guarded by five `compile_fail` doc-tests compiled as an external consumer
  (`src/world/entity_index_map.rs`): no `insert`, no `iter`, no coercion to
  `&HashMap<u32, usize, _>` — that third one is the `Deref` trap — no `AsRef`, and no `raw()`.
  Note that this
  toolchain's rustdoc **ignores the expected error code** on a `compile_fail` fence (verified
  2026-08-09 on rustc 1.97.1: `compile_fail,E0999` against an actual E0624 still passes), so the
  fences are bare and the observed diagnostic is recorded in prose next to each of the first four
  (the fifth is marked as read off the signature, not observed). Three of the
  five compile under the pre-seal shape and so are proofs; `AsRef` failed before the seal too, and
  `raw()` did not exist before it, so those two are forward guards only. The `raw()` fence was
  added by the review pass afterwards: `raw()` is `pub(crate)` and widening it to `pub` is the
  likeliest future regression, and none of the other four fire on it — `insert` stays private,
  `iter` stays absent, and the wrapper still does not coerce.
  `gizmo-physics-core` uses `FxHashMap` too but only behind `pub(crate)`
  (`broadphase/aabb_tree.rs:59`), and the transitive path through
  `PhysicsWorld::spatial_hash` → `SpatialHash` → `DynamicAabbTree::entity_map` was re-checked
  and is closed at both hops, so that crate stays clean.

- **`serde` / `serde_json` — on Stage A public surfaces, accepted under criterion 1, listed here
  because the contract claims one entry per dependency that is on such a surface.** `serde` is
  everywhere (derives on public types; a derive of a foreign trait is criterion-2 reachable).
  `serde_json::Value` is named outright by `gizmo-core`'s `pub type GetJsonFn` / `SetJsonFn` and
  by the public `ComponentRegistry::{get_json_fn, set_json_fn}` fields — which `gizmo-scene`
  re-exports as `SceneRegistry`. `gizmo-physics-rigid`'s `SnapshotError::Serialize(serde_json::Error)`
  is a public variant payload: the identical shape to `SceneError::Parse(ron::…)` that was sealed
  above, left unsealed on purpose — and, found by the 2026-08-09 rustc-hash sweep and added here
  because this list claims to be complete, so is the `impl From<serde_json::Error> for
  SnapshotError` beside it, the same shape as the two ron `From` impls kept on `SceneError`.
  Both crates are at 1.0 with a years-long major cadence, which
  is the whole content of criterion 1 — but note the asymmetry is a *judgement about cadence*, not
  a difference in shape, and it should be re-checked rather than assumed at 1.0 time.

- **Verified clean, recorded so the next audit does not re-derive it (2026-08-09 review).**
  `rodio` 0.17 in `gizmo-audio` — 0.x and fast-moving, but every `Sink`/`OutputStream`/`SpatialSink`
  is a private field and `AudioError` carries only `String`/`io::Error`, so nothing is reachable.
  `bincode` 2 in `gizmo-net` — the `Configuration` const is private and both `encode_packet` /
  `decode_packet` return `std::io::Result`. `rand` 0.10 and `chrono` 0.4 — locals inside function
  bodies only. `web-time` in `gizmo-core` (`profiler.rs`) and `gizmo-ai`/`gizmo-physics-rigid` —
  private fields and locals; the `SceneSnapshot` field was the only public one. `wide`, `dashmap`
  and `crossbeam-queue` in `gizmo-physics-rigid`, and `uuid` in `gizmo-core` (declared twice) and
  `getrandom` in `gizmo-math`, have **zero** source references — dead dependencies, not leaks, but
  they should be dropped in a separate build-graph pass. (Re-confirmed for `gizmo-physics-rigid`
  on 2026-08-09 during the rustc-hash seal.)

- **`gizmo-physics-rigid` swept for the other two shapes of criterion 2, 2026-08-09** — the two
  the audit demonstrably nearly missed, done while the rustc-hash seal was open. **Nothing found.**
  The crate declares **no `pub type` alias at all** (so the `FxHashMap` shape had no siblings) and
  **no associated type in any trait impl** (so no repeat of `<ContactPoints as IntoIterator>::IntoIter`;
  the only `IntoIterator`-ish code in the crate is the wasm-only `parallel_compat` module, which is
  private). Every trait implemented on a public type is either `std` (`Default`, `From`, `Debug`,
  `Display`, `Error`, and the new `FromIterator` on `EntityIndexMap`) or already contracted above
  (`serde` derives; `bevy_reflect::Reflect` behind default-off `reflect`). Method used, recorded so
  it can be repeated: `cargo doc --no-deps` then grep the rendered HTML for every dependency name —
  the only foreign hits on the default surface are `glam`, `serde_json` in `SnapshotError`, and
  prose.

- **`tracing-subscriber` vs `tracing` — the same project, weighted differently on purpose; this is
  criterion 1 in both directions inside one crate.** `tracing-subscriber` (0.3.23) is behind
  gizmo-core's default-OFF `tracing-layer` feature, because `impl Layer<S> for GizmoTracingLayer`
  is a foreign trait on a public type of ours and a trait impl cannot be hidden — it constrains us
  exactly as a public field would, so a tracing-subscriber 0.4 would force a gizmo-core 2.0.
  `tracing` itself (0.1.44) stays an UNCONDITIONAL dep: 0.1 has been frozen for years, and neither
  `info!` nor `#[instrument]` leaves a `tracing` type in any signature of ours, so the low version
  number costs the contract nothing. `gizmo-studio` opts the feature in so `init_tracing()` still
  exists for the editor console; CI runs `cargo test -p gizmo-core --features tracing-layer`.

- **`bevy_reflect` — sealed behind the default-OFF `reflect` feature** (with a serde fallback).
  Unchanged; the archetype for criterion 3.

- **`wgpu` in `gizmo-physics-soft` — an explicit CARVE-OUT from the 1.0 promise.**
  `gizmo-physics-soft` is a Stage A crate, but its default-OFF `gpu_physics` feature compiles
  `pub mod gpu_compute`, whose `GpuCompute` holds `wgpu::Device` / `Queue` / `ComputePipeline` /
  `BindGroupLayout` / `Buffer` in public fields. Nothing in this workspace enables it. Therefore,
  said explicitly rather than left to be inferred: **enabling `gpu_physics` opts that crate out of
  the 1.0 stability promise.** With the feature on, a wgpu major is a breaking change for
  `gizmo-physics-soft` and will ship as a minor bump, not as a 2.0. This is the weakest of the
  carve-outs — default-off, unused in-tree, and the module is a compute path rather than an API
  anyone builds on — but the promise has to name it, because a 1.0 that is silently
  conditional on features is not a promise. (The feature also pulls `bytemuck` and `pollster`;
  wgpu is the one with the cadence.)

- **`renet` in `gizmo-net` — a second feature carve-out, NOT part of the 2026-08-09 audit.**
  Found while writing this section up, so it is recorded as unverified by that audit and needs its
  own confirmation pass. `gizmo-net` is Stage A with `default = []`; its default-off
  `client-server` feature compiles `pub mod client_server`, and there `NetworkClient`/
  `NetworkServer` carry `renet::RenetClient` / `RenetServer` and the two `renet_netcode`
  transports in public fields, with `protocol::connection_config() -> renet::ConnectionConfig` on
  top. renet is at 2.0.0 — major-versioned, and by criterion 1 that settles nothing on its own.
  Same treatment as `gpu_physics` therefore: **enabling `client-server` opts `gizmo-net` out of
  the 1.0 stability promise.** The `rollback` feature is a different matter — it pulls only our
  own crates (and `gizmo-math`, i.e. glam) and stays inside the promise.

- **`wgpu`/`winit`/`egui` in Stage B — a deliberate leak that carries no semver cost while the
  crates that leak them stay at 0.y themselves.** This is the entire reason staging exists; see
  the Stage B list above. Note that only `winit` (0.30.13) and `egui` (0.34.3) are 0.x here —
  `wgpu` is 29.0.3, and it is on the Stage B surface not because of its version number but because
  of its cadence. The leak turns into a cost only if Stage B is ever taken to 1.0, which is gated
  on all three of them settling in the criterion-1 sense.

96 public types are `#[non_exhaustive]`; 13 Error enums + fn→Result conversions.

---

## 5. Determinism (reference)

- The simulation state (Transform/Velocity/solver) runs entirely on **glam/f32**.
- **Target:** same-platform replay + rollback bit-equality. Verified with `state_hash` +
  a cross-process test.
- **OUT of scope:** cross-platform bit-equal determinism — it requires an Fp32/softfloat
  migration (the Q16.16 `Fp32` type EXISTS in gizmo-math but the sim does not use it,
  experimental). It may become an optional feature after 1.0.
- The historical hashes appearing in this document (AAC365945335779E etc.) are point-in-time;
  they were superseded by later fixes — historical.
- **What the snapshot carries is a compile-time decision (2026-08-14).** `snapshot()` and
  `restore_snapshot()` were two hand-written nine-field lists against a 28-field `PhysicsWorld`,
  and every field in `WorldSnapshot` carries a comment naming the divergence that earned it a
  place — gravity fields, joints and weather were each added *after* a resimulation ran under
  state the continuous run no longer had. Omitting a field is not an error, just a rollback that
  restores less than it claims, and the symptom surfaces somewhere else. Both directions now
  destructure exhaustively with **no `..` arm**, so a new `PhysicsWorld` (or `WorldSnapshot`)
  field fails to compile there until someone answers "carried state, or not — and why"; the
  `_`-bound names are that answer written down. Behaviour is unchanged: the same nine fields
  travel, `headless_stress_test` gives three matching hashes, and the guard was verified by
  adding a field and watching `E0027` land on the pattern.

---

## 6. Migration & Graphics Upgrade (0.1 → 0.2, completed)

The "1.0-readiness hardening + graphics upgrade" breaking release (2026-06-25):

- **MSRV → Rust 1.92** (egui 0.34's floor; previously 1.89).
- **Graphics stack:** wgpu 0.20→**29.0.3**, winit 0.29→**0.30.13**, egui→**0.34.3**
  (+ egui-wgpu/winit 0.34.3, egui_dock 0.19.1, transform-gizmo-egui 0.9.0), naga 29.
  The determinism hash (598E315D0E7499FF) did not change across the whole upgrade.
- **API breakers:** the `glam` re-export was made official; `bevy_reflect` was moved behind the
  `reflect` feature; ~~`arrayvec` left the public surface~~ (**wrong — corrected below**);
  96 types became `#[non_exhaustive]`; Error enums + Result returns. For the detailed 11-item
  migration steps, see the git history (the 0.2.0 commits).
  - **CORRECTION (2026-08-09), kept here rather than deleted:** arrayvec did NOT leave the public
    surface in 0.2.0. That release made the `ArrayVec` storage field of `ContactPoints` private,
    which is all anyone checked, but `<ContactPoints as IntoIterator>::IntoIter` went on reading
    `arrayvec::IntoIter<ContactPoint, 4>`. The by-value iterator was sealed only later, by the
    opaque `ContactPointsIter` — see the arrayvec entry in §4, and criterion 2 there for why the
    rule in force at the time could not see it.
- **Code decision (explains the current code):** winit 0.30 still offers the deprecated
  `EventLoop::run(closure)` → gizmo-app's ~600-line closure event loop was moved to
  `ApplicationHandler` DELIBERATELY (see `crates/gizmo-app/src/windowed/`).

---

## 7. Closed Research & Non-Goals

**Metal draws less of a double-sided interior than Vulkan — OPEN, measured, do not re-chase as a
threshold bug** *(2026-08-16)*. `a_double_sided_material_is_drawn_from_behind` fails only on the
macOS runner. It reads like a mis-calibrated pixel threshold and it is not: the assertion was
rewritten to sample the CENTRE of the frame — a camera inside a cube is looking at a wall there
under any projection — and Metal changes **43.8%** of those pixels where Vulkan changes over 90%.
A framing difference cannot leave half the centre of a wall untouched, so the amount of interior
actually drawn differs on that backend. The test is `#[ignore]`d on macOS with the number written
into its doc comment rather than loosened, because loosening it would hide the difference on every
platform. Diagnosing it needs a Mac to run on. Two things measured along the way and worth keeping:
the old assertion compared BYTES including a constant alpha channel, so its ratio was capped at
0.75 before any geometry was drawn; and the test's claim to guard "either pipeline selection" was
false — reverting the z-prepass arm leaves it green (the prepass writes only depth, and the sole
occluder here is the surface under test), so that arm is unguarded.

**Solver stack instability — SOLVED.** A resting column of N≥5 boxes was linearly unstable
(lateral BUCKLING / inverted pendulum, not a vertical energy pump): the iterative contact
solver's effective lateral restoring stiffness was below the buckling-critical value.
- **Fix (2 layers):** (1) a manifold **BLOCK solver** (`solver/block.rs` +
  `tgs.rs::tgs_sweep_block`) — it solves a manifold's ≤4 COPLANAR normal impulses TOGETHER
  (a regularized active-set LCP). Two critical details: the 4-coplanar block is RANK-DEFICIENT
  (4 contacts, 3 DOF) → **Tikhonov reg** (`block_regularization`, 0.05 today — the narrowphase
  fix below turned 0.1 into over-softening) is mandatory; and the block must stay **RIGID**
  (soft scaling weakens it). (2) **Full warm-start** (`warm_start_factor` 0.85→1.0) — a partial
  warm-start threw away 15% of the impulse every substep and injected marginal energy on
  re-convergence; a full warm-start shuts that off. **Result: a 1-wide N≤32 tower is stable**
  (3000 frames, a single ground size) — see the narrowing below; this sentence used to be written
  unconditionally. NO determinism re-bless. Regression:
  `soak_resting_stacks_stay_bounded` (N∈{2,5,16,24,32}).
- **OPEN:** the extreme N≥48 tower still buckles — it needs a friction-aware whole-chain
  direct/global solver (`direct_chain_solve` opt-in flag + `solve_island_normals` solves normals
  only, O(n³)). `soak_extreme_tower_n48` is #[ignore].

**At exact contact the manifold collapsed to a SINGLE POINT — SOLVED** *(2026-08-06)*.
The depth test in `narrowphase/contacts.rs::clip_box_box` had no tolerance (`signed_depth <= 0.0`).
At exact contact the depth of all four corners is exactly zero → all of them are culled and the
clip returns empty → the pair fell back to GJK's **single-point** fallback. A single-point manifold
carries zero tilt-restoring torque (that is precisely why the block solver exists), and the point
GJK returns is not at the center: its offset grows with the size of the opposing collider and
reaches the edge of the resting box, applying an unearned torque impulse. On supports with a
half-extent of ≲1.5 the interface never recovered at all (the centered point holds the box with no
torque → it never sinks in → the clipping path is never entered again).
- **Fix:** a tolerance of the kind the slab test already carries (`DEPTH_TOLERANCE = 1e-4`).
- **Result:** 4 corner points at every support size, both at spawn and at rest; the spawn kick
  0.03 rad/s → **0**; the lean of the 12-high tower on the small platform 0.024 → **0.0000**.
- **The convergence cost it exposed:** `block_regularization` 0.1 → **0.05**. The cost is in the
  4-point interface itself and always was (a chain overlapping by 1 mm is exactly as slow);
  the tolerance removed the degenerate point that had been hiding it. A Tikhonov term is also a
  softening, and 0.1 had been chosen in a period when the term was never actually applied. At 0.05
  the compressed chain comes to rest at frame 0 instead of frame 379, and momentum leakage
  5.4e-4 → 4e-6.
- **Determinism re-bless:** `46EB56180318E43C` → `15D4FD6845119D8B` (3/3).

**Partial sleep was corrupting stacks — SOLVED** *(2026-08-06)*. Once a body falls asleep in the
middle of a contact island it is no longer INTEGRATED but is still SOLVED: `solver/tgs.rs` reads
its mass without looking at the sleep state, the only gate being `is_dynamic()`. The awake
neighbor takes its share of the reaction, the sleeping one does not → momentum is not conserved at
that interface. That is why 12-high stacks did not reliably stand for 3000 frames, and why the
static ground's half-extent (20 vs 200) flipped the outcome.
- **Fix:** the sleep decision is per contact ISLAND rather than per body, and happens AFTER the
  solve (`RigidBody::advance_sleep_counter` + the island pass in `pipeline.rs`). A body with no
  contacts still sleeps on its own counter as before (that transition happens after the joint
  pass).
- **Result:** `wide_block_collapse_per_ground` from 10/20 collapses to **0/20**;
  `height_12_stacks_stay_standing` (6 cells, 3000 frames) **passes**; the natural-sleep lean of a
  1×12×1 column from 0.0104–10.17 to **0.000106**, i.e. identical to the force-awake value.
- **Decisive evidence:** without the fix, at 1 sweep the stack blows up at frame 193 in the
  natural run but never blows up in the force-awake arm → the blow-up comes from partial sleep,
  not from under-solving.
- **Side benefit:** because settled stacks can now sleep collectively, `headless_stress_test`
  went 1.62 s → 0.51 s.
- **Determinism re-bless:** `EF6E4AC3644BF3BA` → `46EB56180318E43C` (3/3).
  `golden_state` `settle vy` `-0.0408733` → `0.0`; that number was one substep's worth of gravity
  and turned out to be the defect's fingerprint. `settle y` did not change.
- **The cost:** if a single member of the island is moving, the island does not sleep; one
  jittering box can keep a whole stack awake. It did not happen in the scenes measured.
- ⚠️ **The statements "robustly stable at N≤32" and "game structures are ≤~12 → not needed" were
  a narrow sample even so** *(narrowed on 2026-08-05)*: a 1-WIDE tower, a SINGLE ground size,
  1500 frames. The fix above rescued 12-high stacks, but the lesson itself stands — the SAMPLE
  must be widened as much as the horizon.
- **LESSON:** choose the soak-test horizon BEYOND the onset of instability (the old `n16` test was
  600 frames, the blow-up at ~853 → it shipped green and hid the bug). **And widen the SAMPLE as
  much as the horizon:** the narrowing above was a defect missed because a test whose horizon was
  sufficient tried a single shape and a single ground size.

**Physics perf, second round — SOLVED** *(2026-08-06)*. Three items, all measured:
- **Incremental broadphase** (C2): the tree is preserved across substeps. The fat-margin AABB
  tree's `insert` already early-outs for a body that has not left its box; `clear()` was throwing
  that gain in the bin. Removals are reconciled with `DynamicAabbTree::retain`. **The determinism
  hash did not budge** — the incremental tree emits pairs in a different order, and the fact that
  this does not change the simulation is empirical evidence of the "pair-emission invariance"
  property.
- **The O(N²) writeback in the ECS bridge** (C3) → a handle→index map. (The bridge has no
  benchmark; a pure complexity fix.)
- **Rewind history opt-in** (C4a): `max_history_frames` 600 → 0. 160 B per body per frame; the old
  default held 192 MB resident in the 2000-box stress scene.

| scenario | 2026-08-05 | 2026-08-06 |
|---|---|---|
| `broadphase/1024` | 1.73 ms | **564 µs** |
| `solver_settled_stack/48` | 4.05 ms | **115 µs** |
| `full_step_mixed/512` | 2.43 ms | **1.64 ms** |
| `headless_stress_test` | 1.62 s | **392 ms** |

The large drop in the settled stack comes from island-collective sleep, the ones in broadphase and
full_step from the incremental tree.

**Physics perf (N² bottlenecks) — SOLVED.** broadphase `query_pairs` pair generation
(O(P²)→O(P)), the TGS per-island scratch sized to the island instead of the whole world, and
HOISTing the per-contact TGS constants out of the 24-sweep loop → worst frame 262→46ms (~5.7×),
bit-equal determinism.

**6 latent bugs (the 2026-07-13 hunt) — ALL FIXED.** tangent (model_mat3, not inverse-transpose),
PBR-pack overflow (`.min(999.0)`), the query get/contains table-storage With/Without gate,
batch-shadow instance-region separation, glTF `AlphaMode::Mask` cutout. **Eliminated as false
positives (do not re-chase these):** deferred_lighting f16 aniso, gbuffer bitangent-collapse,
vehicle point-velocity COM, narrowphase incident-corner. *Remaining minor:* PBR params are still
decimal-packed into a single f32 (precision drops above 2²⁴) — long term, a separate slot.
<!-- TRANSLATOR NOTE: the heading says 6 bugs but only 5 are listed; the sixth may be the "remaining minor" PBR-params item, or an entry lost in an earlier edit — unverified, please check. -->

**Sub-phase timers (2026-08-13).** `PhysicsMetrics` now carries six finer fields —
`solver_{order,prepare,sweep,relax}_ms` and `narrowphase_{dispatch,manifold}_ms` — fed by
module-level atomics in `profile.rs`. Globals rather than fields because `solve_contacts`
takes `&self` so islands can be solved in parallel from one `Copy` config; every scope wraps
per-island or per-substep work, never per-contact. Determinism-neutral by the same argument
the existing phase timers make: written and read, never branched on — hash `A462C9EB8A09D5CA`
is unchanged with them in. **Islands are solved in parallel, so these are CPU-time shares and
can sum past wall-clock; read them as proportions.**

What they say, and it is the same answer in both scenes:

    1000 boxes, settled   solver 3.95 CPU-ms   order 2%  prepare 4%  SWEEP 79%  relax 16%
                          narrow 0.51          maths 55%  plumbing 45%
    1000 spheres, awake   solver 8.63          order 7%  prepare 5%  SWEEP 74%  relax 14%
                          narrow 2.56          maths 75%  plumbing 25%

**Three quarters of the solver is the sweep** — the constraint iteration itself, not the
scaffolding around it.

That reading led to a conclusion recorded here for a day and **since refuted by measurement**:
that the gap is a convergence-per-iteration difference and closing it means a better-converging
sweep. See "What the iteration budget is actually worth" below. Three quarters of the *solver*
is not three quarters of the frame, and the sweep turns out to be the smallest of the three
factors in the Rapier gap rather than the whole of it.

**What the iteration budget is actually worth (2026-08-14, `benchmarks/vs-rapier` with
`ITERCURVE=1`).** Rapier's default is `num_solver_iterations: 4` per island per frame with no
substepping; we run four substeps of `iterations` (20), with `adaptive_iterations` raising deep
islands to `max(28, 1.5·D)` regardless of the base. So we run about **20× the constraint
iteration per island per frame** — and, at 9× the total frame cost, each of our iterations is
therefore about **2× cheaper than theirs**. The gap was never in what an iteration costs.

Nor is it in how many we run. Sweeping the count on the thousand-box pile, `adaptive_iterations`
off so the setting is actually honoured, and reading **only the frames where bodies are still
moving** (see the trap below):

    iterations   awake ms/frame   sweeps/frame   99% asleep at
    32           5.029            2346           frame 74
    20 (default) 4.431            1586           frame 75
     8           3.627             712           frame 77
     2           3.644             176           frame 78
     1           3.636              97           frame 84

**The entire variable cost of the constraint iteration is 0.795 ms of a 4.431 ms awake frame —
18%**, and below eight sweeps it is flat. A solver that converged perfectly in a single sweep
would make this scene 18% faster and nothing more. It cannot close a 9× gap, and neither can any
amount of work on how well the sweep converges.

Two traps this measurement had to get past, both of which had already produced a wrong number
once. **The 300-frame average is mostly sleep** — the pile settles around frame 75, so three
quarters of it measures a sleeping scene and says nothing about the solver; the first version of
this table read a "floor" of 86% that was very largely sleep. And **the base count is overridden
for deep islands**, so with `adaptive_iterations` on, taking `iterations` from 20 to 4 cuts sweeps
only 40% and buys 1% of frame — the first curve looked flat for that reason, and it was the sweep
*count* column, not the milliseconds, that gave it away.

So the honest decomposition of the gap, measured on awake frames only (Gizmo 4.340 ms × 75 awake
frames against Rapier 0.504 ms × 116 — they stay awake longer, so on total work done the gap is
**5.6×**, not the 9.2× the all-frames average reports):

| factor | measured | is it optional? |
|---|---|---|
| substep multiplier | **4×** | deliberate — it is what survives 320 m/s without tunnelling |
| per-substep pipeline outside the sweep | **1.8×** | open — broadphase, narrowphase, prepare, integrate, run four times |
| the constraint iteration itself | **1.2×** | 18% ceiling, measured above |

The lever is the middle row, or the top one at a price. It is not the sweep.

**Where the middle row is, phase by phase (2026-08-14, awake frames only).** The recorded phase
table was also a 300-frame average and so also three quarters sleep; read over the frames that
have work in them, and divided by four so a substep is compared against Rapier's step:

| phase | Gizmo / awake frame | per substep | Rapier / step | ratio |
|---|---|---|---|---|
| broadphase | 0.439 | 0.110 | 0.046 | 2.4× |
| **narrowphase** | **1.032** | **0.258** | **0.064** | **4.2×** |
| solver | 2.479 | 0.620 | 0.297 | 2.1× |
| integration | 0.114 | 0.029 | 0.028 | 1.0× |

**The narrowphase is the worst ratio in the engine** — 4.2× per substep, a quarter of the awake
frame — and it splits into a parallel section (the collision maths, 0.80 ms) and a **sequential**
assembly of manifolds, contact cache and events (0.36 ms, 8% of the awake frame, on one thread
while the rest idle). That sequential 8% is the concrete target the "per-substep pipeline" row
resolves to.

**But it has no single lever, measured.** The warm-start pass was pure per-pair work over
immutable state and has been moved onto the worker that produced the contacts (behaviour-identical
— hash `A462C9EB8A09D5CA` unchanged, since the per-pair computation is order-independent). It was
**10%** of the sequential loop. The contact-cache inserts, the next obvious suspect, measure
**0.02 ms — 5%**. The remainder is manifold construction and copying, spread thin. This is the
same shape as the allocation investigation: a real cost with no concentrated win in it, and it
should be treated the same way — do not spend more here without a measurement that names a
specific 30%+ of that 0.36 ms.

Two instrumentation corrections came out of the same work, both worth knowing about because both
had already produced a wrong number. The `dispatch` scope was **per-pair** — ~7300 `Instant::now()`
plus a `fetch_add` on one shared counter per frame, violating `profile.rs`'s own stated rule and
costing 3% of the phase it measured; it now wraps the whole parallel section and reports wall
time. And every phase timer in the benchmark now accumulates on awake frames only, per engine,
because the two engines sleep at different frames (Rapier 116, us 75) and a shared divisor
mis-scales one of them.

**The solver carries no fat (2026-08-13, `benchmarks/vs-rapier` with `ABLATION=1`).** With
no profiler available, the solver's own switches were used as one: turn each off, measure
the thousand-box pile, and the difference is what that feature costs. **Every switch is
load-bearing, and turning any of them off makes the frame slower, not faster.**

    all on (default)              2.065 ms/frame   1/1000 awake   mean y 3.98
    support_ordering off          2.481            1/1000         3.98
    adaptive_iterations off       2.381            1/1000         3.98
    iterations 20 -> 8            2.438            1/1000         3.98
    block_solver off             17.418         1000/1000         1.54
    use_tgs_soft off             36.135         1000/1000         1.56

The last two columns are the explanation: without the block solver or TGS-soft the pile
never settles — a thousand bodies stay awake and the stack collapses from a mean height of
3.98 to 1.54 — and an unsettled pile costs far more than the feature that settles it. The
block solver repays itself fifteen times over, TGS-soft thirty-four. Even the small knobs
go the same way: *fewer* solver iterations is **more expensive**, because convergence gets
worse and the pile takes longer to come to rest.

So nothing in this table is waste to be reclaimed. But note what it does **not** say, because it
was once read as saying it: that the iteration count is load-bearing. One point cannot tell a
floor from a slope, and the curve measured in "What the iteration budget is actually worth" above
shows this scene is insensitive to the count between 1 and 8. `iterations 20 → 8` costing more
here is a real effect of a different kind — it is the *adaptive* path and the settle time moving,
not convergence failing.

**Written, exported, and not wired to anything (2026-08-14 survey).** `LodGroup` turned out to be
a capability the engine's own default path could not reach — the components existed, `select_mesh`
existed, and only `gizmo-studio` ever looked. That was the second one (`Frustum::test_aabb_masked`
sat at zero callers until `gizmo-renderer::visibility` found it), so the class was worth sweeping
for rather than meeting one at a time. What the sweep found:

| what | state | shape |
|---|---|---|
| **skeletal animation** | **FIXED 2026-08-14.** `animation_update_system` and `animation_state_machine_update_system` lived in `gizmo-renderer` with `current_time += dt · speed` appearing nowhere else in the workspace, and nothing in the facade, in `gizmo-app` or in any demo ever called either. `default_render_pass` now calls both, before the draw path reads `Skeleton` | the draw path *consumes* `Skeleton` (skinning matrices in `collect_draw_items`), so the engine rendered a pose it never advanced — a clock with no hands, and everything downstream of it looked wired |
| **`ParticleEmitter`** | **FIXED 2026-08-14.** The component, the GPU pipeline and the draw call were all present — `default_render_pass` already ran `update_params` and `compute_pass`, and `passes/forward` already drew the result. Nothing ever *put anything in it*: the emitter→GPU bridge lived only in `gizmo-studio`. It is now `systems::render::spawn_from_emitters`, called from the pass | exactly `LodGroup`'s shape: the whole path present, one link missing, and the link living in studio |
| **`Sprite`** | **DELETED 2026-08-14.** Referenced by nothing in the workspace — not studio, not the editor, not a demo, not even `gizmo-app`'s scene registry, which registers its sibling `Camera2D` | dead, and unlike the other three there was no link to restore: wiring it meant writing a 2D pipeline (billboarding, layer sort, atlas UVs, transparency), which is a feature nothing asked for. An exported component that cannot be drawn is a promise the API does not keep, and the 1.0 surface is the wrong place to keep it |
| **`gizmo-ai`'s systems** | `behavior_tree_system`, `ai_navigation_system`, `ai_navmesh_rebuild_system` are re-exported and never scheduled | the same shape, but plausibly deliberate: when AI ticks is a game's decision, not an engine's. Left alone, and named here so the next sweep does not re-find it as news |

**Why it was never wired — and the first explanation here was wrong.** This section originally said
both systems take `(&mut World, dt, &wgpu::Queue)`, that no ordinary system has a queue, and that
"there was no schedule they *could* be added to — that is very likely the whole story". It is not.
`gizmo-studio/src/render_pipeline/mod.rs:20` was **already calling `animation_update_system` with
exactly that signature**, from exactly the position the fix now uses: holding the world and the
queue, before the draw. The signature was never the obstacle.

The real reason is structural and is the same one behind `LodGroup` and `ParticleEmitter`: **render
wiring happened in `gizmo-studio`, and the engine's own `default_render_pass` is a second,
independently-maintained copy of the same job.** A capability wired into studio still has an in-tree
consumer, so the obvious check — "does anything read this component, does anything call this
system?" — answers *yes*, and the gap is invisible to precisely the sweep you would run to find it.
That is why the sweep below, which looked for zero callers, found `Sprite` and missed these.

The drift runs both ways, which is the proof it is structural rather than a backlog: the fix landed
on the engine path only, so `animation_state_machine_update_system` is now engine-only exactly as
`BoneAttachmentSystem` is studio-only. `default_render_pass` is the one place holding the world,
the queue, and a position before `collect_draw_items` reads the result, so they are called from
there, with `dt` from the `Time` resource.

The guard is `golden_render_tests::default_render_pass_advances_skeletal_animation`, which drives
the real pass over a one-joint rig and asserts the clock moved — and it was checked against a
build with the call removed, where it fails. It deliberately asserts the *wiring* and not the
arithmetic: `normalize_anim_time` already had tests for looping, clamping and zero duration, and
every one of them passed throughout the years the feature did not run. A test of a policy is not a
test that the policy is reached.

**Two notes from the particle fix.** The bridge sits in the facade rather than in `gizmo-renderer`
because it needs both halves and they are in different crates — `GpuParticleSystem` is in the
renderer and `Transform` is in `gizmo-physics-core`, which the renderer does not depend on. And it
does its spawn jitter with a private four-line xorshift seeded from `Time::frame()` and the entity
id, where the code it replaces called `rand::rng()`: the facade would otherwise gain a dependency
that everyone building `gizmo-engine` pays for, and a deterministic engine emitting
nondeterministic sparks is a small lie. Neither version ever touched simulation state, so this was
never a contract violation — only an avoidable inconsistency.

`gizmo-studio` still carries its own copy of the loop and **that is deliberate, not an oversight**:
`spawn_from_emitters` takes `&mut World`, which is a signature with no borrowing precondition,
while studio's pipeline holds a live read borrow across the whole block. A `&World` variant would
serve both, but its soundness would depend on what borrows the caller happens to hold — a
conditionally-safe public API is a worse trade than one duplicated loop. The duplication is
written down at both ends instead of hidden.

**Where the sweep ended.** Three of the four were the same defect — a link missing from an
otherwise complete path — and all three are fixed. The fourth was not a missing link but a missing
feature, and it was removed instead. The lesson worth carrying is the one animation taught: every
policy in that path had tests, and every one of them passed for as long as the policy was
unreachable. **A test of a policy is not a test that the policy is reached**, and the three fixes
are each guarded accordingly — `default_render_pass_advances_skeletal_animation` fails with the
call removed, and it was checked that way rather than assumed.

**The root the sweep could not see (2026-08-14, architectural review).** The sweep above looked for
*zero callers* and found `Sprite`. It could not have found `LodGroup`, `ParticleEmitter` or the
animation systems, and the reason is structural: **`gizmo-studio` is a workspace member, so a
capability wired only into its render pipeline still has an in-tree consumer.** The obvious check —
does any system read this component? — answers *yes*, and the gap is invisible to precisely the
question you would ask.

Underneath that is the real shape: `collect_draw_items` and `gizmo-studio`'s
`execute_render_pipeline` are two independent implementations of *world → draw list*, and only the
pass recording after that genuinely needs to differ. `shared.rs` says as much in its own header —
light collection and cascades were single-sourced *after* each had to be fixed twice. Everything
that has not yet caused a visible bug is still duplicated, so the default state of any new
capability is "lives in exactly one path". The drift runs both ways, which is what makes it
structural: `animation_state_machine_update_system` is now engine-only exactly as
`BoneAttachmentSystem` is studio-only.

**First cut taken: `gizmo-renderer::routing`.** Both loops decided what every `MaterialType` means
with a `match … { _ => 0.0 }`, and the two wildcards disagreed — `BakedLit` was routed by the engine
path and defaulted by studio's, so a baked-lit level shaded one way in the game and another in the
editor; `Grid` was the reverse. `MaterialType` is `#[non_exhaustive]`, so a wildcard is *obligatory*
in any downstream crate and a ninth variant could never have been a compile error in either file.
The decision now lives in the crate that defines the enum, where the match is exhaustive: one
compile error there instead of two silent misroutes out here. The engine path is behaviour-identical
(12 golden render tests, hash unchanged); studio's `BakedLit` flag goes 0.0 → 1.0, which is the fix
and is a visible change to a viewport nothing can test.

What this deliberately does **not** do is merge the two paths. The deferred and editor-forward
recorders genuinely differ, that difference is the part with no automated coverage, and §3 gates it
on human-eye A/B. Single-sourcing the *semantics* is the half that pays.

**Second cut taken: `gizmo-renderer::frame_uniforms`.** The two uniform blocks — `SceneUniforms`
(18 fields) and `PostProcessUniforms` (16) — were hand-filled as exhaustive struct literals at
**six** sites: both draw loops, the renderer's two initial buffers, and three demos with custom
render callbacks. An exhaustive literal *looks* like the safe form, because omitting a field does
not compile; what it actually checks is "was every field filled", and all six passed that. The
question nobody was asking is "which field differs, and why":

- `PostProcessUniforms::cam_near`/`cam_far` exist because DoF linearises depth with them, and the
  field comment says a hardcoded range miscalibrates the circle of confusion. **Five of six sites
  hardcoded `0.1`/`2000.0` anyway** — including studio, so the editor viewport's DoF was
  miscalibrated for exactly the cameras the field was added for. This is the one live bug the cut
  fixes, and it is a viewport change nothing can test.
- `SceneUniforms::cascade_params.x` is documented in `common.wgsl` as the camera's z-near. Studio
  sent `cam_near`, the engine sent a literal `0.1`. They had disagreed for as long as both paths
  existed **and it cost nothing**, because no shader reads `.x` today. `SceneUniforms::exposure` is
  dead in the same way (the post composite owns exposure; `deferred_lighting.wgsl` says so next to
  the field it no longer reads) and had drifted the same way: camera's in the engine, `1.0` in
  studio.

Two dead fields drifting is not the finding. The finding is that a *live* field drifts by the same
mechanism and is just as invisible — `cam_near` is the same class of mistake, and it was live.

Derived work (the inverse view-projection, the packed `cascade_params` slots, the `w` flags, the
padding, the shadow texel size) now happens once in `SceneUniforms::new(&SceneFrame)`; what the two
paths legitimately disagree about is arguments at a call site with the reason next to them —
studio's identity cascades when nothing casts, its point-shadow lookup left off because it records
no cube pass, its exposure from the editor slider. The engine path is byte-identical (12 golden
render tests, hashes unchanged) except for the two dead fields.

The guard is `no_hand_filled_uniform_literals_outside_the_constructor`, and it **walks the
workspace** instead of naming the files it knows about — deliberately, because that is the flaw in
the shader mirror tests below: each hand-counts its subjects, so a new shader is invisible to the
test that exists to police it. A partial literal (`..Default::default()`) is the sanctioned escape;
an exhaustive one is a test failure the day it is written. Verified by reintroducing one.

**The same flaw, on the shader side: `gizmo-renderer::shader_contract` (2026-08-14).** `common.wgsl`
opens by declaring itself the single source of truth for the scene uniform layout. It is — for the
shaders that `#import` it, and **seven still declare their own `SceneUniforms`**, because whether a
shader shares the definition depends on whether its Rust call site reached for
`load_shader_composed` rather than `load_shader`. Nothing in the shader says which it is. The seven
are legitimate — each is a *prefix* of the block, truncated after the last field it reads, which is
how a shader avoids declaring 1168 bytes to read a view-projection — but nothing checked the
prefix, and the tests that looked like they did could not: one pins Rust's struct sizes while
calling itself "a contract with the WGSL side" without opening a `.wgsl` file, and the other reads
ten shaders **from a hand-written list** and counts `vec4<f32>` occurrences rather than checking
where the fields land.

The replacement takes its subjects from the shader directory and its answer from naga: every shader
declaring `SceneUniforms`, `InstanceRaw`/`InstanceData` or `LightData` is parsed, and each named
field's byte offset is compared against `offset_of!` on the Rust struct that fills it. Padding is
ignored by name — its effect is entirely visible in where the named fields land — and types are not
compared, which is what lets `gbuffer.wgsl` keep its deliberate `array<vec4<f32>, 40>` in place of
ten `LightData` while a field that *moved* still fails. No drift was found; the eight declarations
agree today. Verified by reordering two fields in one copy and dropping one from another, and
watching the two tests name the file, the field and both offsets.

This also closes the "`compose_wgsl` builds a `naga::Module`, validates it and throws it away"
entry, and closes it honestly rather than by refactoring for its own sake: `compose_module` now
returns the validated module, `compose_wgsl` is the thin text wrapper over it, and the contract
tests are the consumer that needed it — the composed shaders substitute bind-group indices inline
(`@group(#{INSTANCE_GROUP})`), so naga alone cannot read them and only the real composition path
can.

Two things this is **not** saying. A public function with no in-tree caller is not a defect — a
library's surface exists to be called from outside, and a sweep on that basis returns most of
`gizmo-core`. And a system a game is meant to schedule itself is not unwired. The class that
matters is narrower: **the engine's own default path depends on state that the engine's own loop
never produces**, or a component the engine exports and nothing anywhere can draw. Animation and
`Sprite` are those. `ParticleEmitter` is one step out — nothing is broken, but the feature is only
reachable by reimplementing studio's bridge.

**NON-GOAL: cutting contact-path allocations.** Investigated, REJECTED (2026-08-13) — and
rejected for the same reason as the SIMD item below, in a different currency. A comparison
against Rapier3D (`benchmarks/vs-rapier`) showed us allocating **8158 heap allocations per
frame against Rapier's 60** on a settled thousand-box pile, which looked like the obvious
lever. Three changes cut it 42 % — dormant cache entries moved instead of cloned, the cached
manifold's buffer reused, and `support_order_manifolds`' per-body `Vec`s moved to a
thread-local scratch (that last one alone was 32 % of all allocations, found with a sampling
allocation profile, `PROFILE=1` in that benchmark). **Frame time did not move.** The reason,
measured: a small `Vec` allocate-and-drop costs **8 ns**, so 22 833 allocations a frame are
**0.183 ms of a 9.5 ms frame — 1.9 %.** Removing every one would buy 2 %. The three changes
are kept (they are correct, and behaviour-identical: hash `A462C9EB8A09D5CA` throughout), but
do not spend more here. An allocation *count* is not a frame time.

**Where the Rapier gap actually is (2026-08-13).** Same harness, both engines multi-threaded,
identical scenes and `dt`. Quality is level — the 20-box tower drifts 0.000 m against their
0.010, an analytic elastic collision comes out 4.950 against their 5.000 — and the thousand-box
pile costs 1.7 ms against 0.17. That splits in two: **~4× is the substep multiplier**
(`PHYSICS_HZ = 240` runs the whole pipeline, collision detection included, four times per
rendered frame — deliberate, and it is what makes 0.5 m spheres survive 320 m/s without
tunnelling, measured), and **~2.5× is genuine per-substep cost spread across every phase**
rather than concentrated in one. Neither is allocations. That next step was taken — sub-phase timers, then the iteration curve
above — and it moved the answer: of the ~2.5×, the constraint iteration is 1.2× and the rest of
the per-substep pipeline is 1.8×.

**NON-GOAL: narrowphase batch-SIMD.** Investigated, REJECTED (2026-07-14). Measurement
(wide_scene, 2000 boxes, ~30ms frame): box-box SAT compute is only **~3.3%**; narrowphase
post-processing cannot be batched; both per-pair SIMD attempts regressed (the scalar code is
already auto-vectorized). DO NOT RETRY without passing the step-0 gate again. (The
"~82% narrowphase" figure is OBSOLETE.)

---

## 8. Working Method

- Every item: **fix → write a regression test → build/test/clippy → tick it off.**
- Verify behavior-changing physics fixes with `headless_stress_test` + focused scenarios;
  choose the soak horizon beyond the onset of instability.
- On bug-hunt rounds use subagent fan-out, then verify every finding BY HAND
  (sieve out the false positives).
- **A guard that has never failed is not known to work.** When a change lands a test whose job is
  to catch a class of mistake — a scan, a ratchet, an exhaustive destructure — reintroduce the
  mistake, watch it go red, and put that in the commit message. Two of this repo's mirror tests
  passed for months while checking nothing, and both looked exactly like tests that worked.
- **Prefer a scanned subject list to a written one.** A test that names the ten files it polices
  cannot see the eleventh, and that is the file the bug will be in. Take subjects from the
  directory, the component modules, the workspace; keep only the *exceptions* by hand, and fail on
  a stale exception too, or the list rots the same way.
- CI: `cargo clippy --all-features --all-targets -- -D warnings -A too_many_arguments
  -A type_complexity` (the two grandfathered architectural lints). The entry crate is
  `gizmo-engine` (NOT `-p gizmo`); `| tail` masks cargo's exit code — check the exit status
  separately.
- **A GPU test must refuse a software adapter** unless it says why not. The Windows runner's
  adapter is WARP; a deferred frame there software-rasterises a 3072² × 4 shadow-map array, and
  the job that first let those tests render was still going at 5.5 hours against ubuntu's six
  minutes. Every CI job carries `timeout-minutes: 45` for that, but a timeout is a report, not a
  fix — the runner still burns the 45 minutes. `every_gpu_test_refuses_a_software_adapter` reads
  the test file and fails on any test that opens an adapter without the
  `headless_adapter_is_software` guard; the one deliberate exception (pipeline compilation, whose
  whole subject is the backend) is named there with its reason, and a stale exception fails too.
  Added after three tests written on 2026-08-15 checked only that an adapter existed.
- **The same gate runs against `wasm32-unknown-unknown`** (2026-08-15). Not redundant: the lint
  config is crate-wide but which code *exists* is per-target, so every
  `#[cfg(not(target_arch = "x86_64"))]` arm and every wasm-only branch sat outside every gate the
  project had. CI built that target and never linted it, and it was holding an undocumented public
  function in a crate that denies `missing_docs`, plus a `return` that is needless once the other
  arm is stripped. A green native lint says nothing about the arms it did not compile.
  - **It caught its first real break the next day** (2026-08-16). Per-object shadow casting added
    `DrawItem::casts_shadows`, whose only reader is `passes::shadow` — and that module is
    `#[cfg(not(target_arch = "wasm32"))]`, because the browser pipeline is forward-only and has no
    shadow pass. So the field is written on every target and read on one, which is `dead_code` on
    wasm and invisible to the native gate. The field stays (it is part of `BatchKey`; dropping it
    on wasm would merge objects that disagree about shadows into one batch), so the fix is a
    targeted `#[cfg_attr(target_arch = "wasm32", allow(dead_code))]` with that reason written
    down. Worth noting *how* it surfaced: nothing in the native workspace was red — the break sat
    at HEAD for three commits, and the wasm gate is the only thing that sees this class of defect.
  - **`gizmo-editor` joined the gate** (2026-08-16), and it had already rotted. The crate declares
    wasm intent — `gizmo-scripting` is a `cfg(not(target_arch = "wasm32"))` dependency, `web-time`
    a wasm one, and items are cfg'd to match — but nothing ever compiled that arm, because
    `demo-web` does not enable the facade's editor feature. It had stopped building for wasm
    outright: a scripting-only inspector function was added without the `cfg` its neighbour
    carries. Beside it, a `let mut initial_dir` that is dead once the native arm is stripped, and
    a `std::thread::spawn` on the wasm path — which on `wasm32-unknown-unknown` does not run the
    closure, it panics, so Save/Load in a browser build would have panicked on click. Three
    defects in one crate, none visible to any native job. The editor is still **not shipped** in
    the browser; it is compiled there so its cfg arms stay honest.

## Nerede kaldık (2026-08-14 → 15)

Bulunup düzeltilen kusurlar aşağıda kendi bölümlerinde duruyor. Bu bölüm yalnız **açık olanı** ve
**bir daha kovalanmaması gerekeni** taşır.

**Kayıtlı kusur listesi boş.** 15 Ağustos sonunda açık kalan hiçbir madde bir hata ya da engel
değil; ikisi bilinçli karar (aşağıda), gerisi sıradan ürün işi (§3 Faz 6/7).

**Bulmayı sağlayan soru — bir sonraki oturum için asıl aktarılan şey bu:** *aynı karar nerede iki
kez veriliyor?* Bugünün on bir bulgusunun tamamı bu sorudan çıktı ve hiçbiri bir hata raporundan
gelmedi. Sıralı olarak bakılan eksenler: iki render yolu (sekiz sürüklenme), üçüncü hedef olarak
wasm (hiç lint edilmiyormuş), scripting (dört kusur), crate grafiği (belge bayat, söz sınanmıyor),
rollback (iki uygulama, biri eksik). Tükendiğinde belirti şu oluyor: aday aramak, bulguları
üretmekten uzun sürüyor.

### İki render yolu — kayıtlı adımların hepsi kapandı

Kök kayıtlı ("The root the sweep could not see"). Beş kesim:

1. `gizmo-renderer::routing` — malzeme tipinin anlamı tek ve tüketici bir `match`'te.
2. `gizmo-renderer::frame_uniforms` (**2026-08-14**) — iki uniform bloğunun kurucusu. Literaller
   tahmin edilenden çoktu: iki çizim döngüsü değil **altı** yer (renderer'ın iki başlangıç tamponu
   ve kendi render callback'i olan üç demo da elle dolduruyordu). Ayrıntısı §7'de; özeti şu:
   sürüklenen üç alandan ikisi ölü (`cascade_params.x`, `scene.exposure` — kimse okumuyor), biri
   canlıydı. Canlı olan **studio'nun DoF'u**: `cam_near`/`cam_far` altı yerin beşinde `0.1`/`2000`
   sabitiydi, yani editör viewport'u tam da bu alanın eklenme sebebi olan kameralarda yanlış
   kalibreydi. Motor yolu (ölü iki alan dışında) bayt-aynı; 12 golden render testi değişmedi.
   Bekçi `no_hand_filled_uniform_literals_outside_the_constructor`: dosya listesi tutmuyor,
   **workspace'i tarıyor** — yani yedinci çağrı yeri yazıldığı gün kırmızı. Kırılabildiği
   doğrulandı (literal geri konup test kırmızıya düşürüldü, sonra geri alındı).

3. **Studio lib hedefi + parite testi (2026-08-15).** `gizmo-studio` artık lib hedefi taşıyor
   (`publish = false` aynen duruyor — amaç yayımlamak değil, editörün render yoluna testten
   erişebilmek) ve `tests/render_parity.rs` iki yolun ilk otomatik çapraz denetimi.
   Doğrulayıcının "ölçekte iyimser" dediği yer doğruydu: `StudioState`/`EditorState`'i headless
   kurmak hâlâ pahalı. O yüzden test **kurulumu** hedefliyor, pass kaydını değil — asıl iş
   `collect_scene_setup`: ışık toplama ve cascade orkestrasyonu zaten ortaktı, ama etraflarındaki
   kırk satır (güneş-var bayrağı, cascade'ın hangi ışığı takip edeceği, ışıksız sahnede identity
   geri düşüşü, point-shadow caster indeksi) iki dosyada ayrı ayrı yazılıydı. İki yol arasındaki
   fark artık **adlandırılmış bir politika**: `ShadowCaster::SunOnly` (oyun) ve `SunOrFirstLight`
   (editör). Test aynı dünya için iki argüman setini yan yana koyup bloğun **dünyanın karar
   verdiği** her alanında eşitlik, yalnız her yolun ilan ettiği üç alanda fark arıyor. Yanına iki
   bekçi: hiçbir render yolu `collect_scene_lights`/`compute_directional_cascades`'i doğrudan
   çağıramaz (çağırırsa parite testi o yolu kapsamıyor demektir), ve her yolun hangi politikayı
   geçtiği kaynakta sabitli — yoksa test kimsenin kullanmadığı iki argüman setini karşılaştırıp
   sonsuza kadar yeşil kalırdı. Motor yolu bayt-aynı (12 golden render testi), determinizm hash'i
   değişmedi. Kırılabildiği doğrulandı: "dünyanın kararı" bir politikaya bağlandı ve studio'ya
   doğrudan bir çağrı kondu; ikisi de kırmızıya düştü.

4. **Yetenek envanteri (2026-08-15).** Kayıttaki "~60 satırlık isim envanteri" testi de yazıldı,
   aynı dosyada. Özneler `gizmo-renderer/src/components`'ten **taranıyor** (yarın eklenen bir
   bileşen aynı gün envanterde), yalnız istisnalar elle — ve **bayat istisna da kırmızı**: iki yol
   da tanımaya başlamışsa kayıt silinmek zorunda, yoksa liste tam da yerine geçtiği çürüyen elle
   sayıma dönüşür. Bugünkü asimetri ölçüldü, tahmin edilmedi: **`Decal` yalnız oyun yolunda**
   (decal geçişi G-buffer'a harmanlıyor, editörün yolu forward — yapısal, ama sonucu gerçek:
   editörde yerleştirilen bir decal oyun koşana kadar görünmüyor), `EditorRenderTarget` ve
   `GameRenderTarget` yalnız editörde (tanımı gereği). Kırılabildiği doğrulandı: yeni bir bileşen
   eklenip tek yola bağlandı, bir de bayat istisna kondu; ikisi de kırmızıya düştü.

5. **Çizim listesindeki iki ortak karar (2026-08-15).** Envanter "hangi yetenek hangi yolda"yı
   ölçüyor; asıl ayrışma ise iki döngünün **aynı soruya ayrı ayrı cevap vermesi**. İkisi bulundu,
   biri gerçekten kopmuştu:
   - **PBR paketleme — canlı sürüklenme.** Anizotropi/clear-coat/subsurface üçlüsü tek bir f32
     yuvasına paketleniyor. Motor alan başına **iki** ondalık hane kullanıyor; studio hâlâ **üç**
     kullanıyordu — yani motorun *terk ettiği* düzen: dokuz hane `f32`'nin tam-tamsayı sınırını
     (2^24) aşıyor ve alt alan komşusuna taşıyor. `gbuffer.wgsl` iki haneyi çözüyor, dolayısıyla
     studio'nun instance'ları **başka bir malzeme** olarak çözülürdü. Bugün etkisiz, çünkü
     editörün forward hattı o shader'a hiç uğramıyor — `cascade_params.x` ile aynı sınıf: gerçek
     sürüklenme, bedeli şimdilik sıfır. Paketleme artık `InstanceRaw::new`'un içinde ve fonksiyon
     **private**; çağrı yerinde yazılamıyor. Yuvanın adı da düzeltildi: `_padding` → 
     `packed_pbr_params` ("padding" diye anılan bir yuva, kimsenin doğru yapmak zorunda olmadığı
     bir yuva gibi okunuyor — iki yolun ayrı paketlemesinin bir sebebi bu).
   - **LOD seçimi.** "Bu entity hangi mesh'i çiziyor" üç durumlu bir cevap (grup entity'nin kendi
     mesh'ini *ezer*; son seviyeyi geçen mesafe **cull** demek, en kaba seviyeyi çizmek değil) ve
     iki yerde satır içi yazılıydı. Uyuşuyorlardı — iyi durum, ama kalıcı değil: `LodGroup` uzun
     süre yalnız editörde onurlandırıldı, motorun kopyası özellikten genç. Artık
     `LodGroup::pick`; mesafenin nereden ölçüldüğü (biri `GlobalTransform`, öteki birleştirilmiş
     model matrisi) her yolun kendi işi olarak kalıyor. Motordaki "studio mesh merkezine ölçüyor"
     yorumu da yanlıştı, silindi.

   Bekçi: iki çizim döngüsü `select_mesh`'i doğrudan çağıramaz ve `packed_pbr_params`'ı elle
   yazamaz. Kırılabildiği doğrulandı.

6. **Çift-yüzlü malzeme — motorda hiç bağlı değilmiş (2026-08-15).** Batch anahtarlarını yan yana
   koyunca çıktı: studio'nun anahtarında `is_double_sided` var, motorunkinde yok — çünkü motor bu
   alanı **hiç okumuyordu**. `Material::with_double_sided` her zaman public'ti, ama yalnız
   editörün forward hattı ona bakıyordu; motorun Z-prepass'i ve G-buffer'ı koşulsuz arka yüz
   kırpıyordu. Yani çift-yüzlü yazılmış bir kumaş/yaprak **editörde iki yüzlü, oyunda tek yüzlü**
   görünüyordu. İncelemenin adını koyduğu sınıf: motorun dışa verdiği bir yetenek, motorun kendi
   varsayılan yolunun okumadığı bir durum (`Sprite`, `LodGroup`, `ParticleEmitter` ile aynı raf).
   Bağlandı: iki yeni boru hattı varyantı (`gbuffer_double_sided`, `z_prepass_double_sided` —
   kırpma modu wgpu'da boru hattına gömülü), batch anahtarına bayrak, üç geçişte seçim. Forward
   yolda zaten kullanılmadan duran `render_double_sided_pipeline` da bağlandı.
   **Kanıt:** yeni golden render testi — kamera bir küpün içinde, yani her görünür üçgen arka yüz;
   tek-yüzlüyle kare boş, çift-yüzlüyle iç yüzey çiziliyor. Düzeltme geri alınınca test "karenin
   %0.0'ı değişti" diyerek kırılıyor.
   **Bağlanmayan, kayda geçen:** saydam çift-yüzlü yüzey iki yolda da tek yüzlü (harmanlı boru
   hattının iki-yüzlü varyantı yok, editörde de yok) ve gölge geçişi ön-yüz kırpmayı sürdürüyor.

   **Bekçinin kendi boşluğu da kapandı.** Dünkü yetenek envanteri bunu göremezdi: özneleri bileşen
   *tipleri*ydi, bu ise `Material`'ın bir *alanı* — ve `Material` iki yolda da her renk okuyan
   satırda geçiyor. Envanter artık `Material` ve `Mesh`'in public alanlarını da özne sayıyor, ve
   alanları **erişim** olarak eşleştiriyor (`.ad`) çünkü çıplak ad `radius`/`color` gibi yerel
   değişkenlerle çakışıyor. Neden yalnız bu iki struct: entity başına yetenek malzemede ya da
   mesh'te oturuyor; `Camera::primary` ya da `PointLight::color` ortak toplayıcılardan okunuyor ve
   yalnızca iki farklı struct'ın aynı alan adını taşıması yüzünden işaretlenirdi — daha da
   genişletmek, tarayıcı sınırını tasarım kararıymış gibi kaydeden istisnalar üretirdi. Ölçüldü:
   bütün bileşenlere açılsa 13 asimetrinin ~4'ü bu türden yanlış pozitif. Kırılabildiği doğrulandı
   (paylaşılan bir alan tek yola indirildi + bir istisnanın yönü ters çevrildi; ikisi de kırmızı).

8. **Instans tamponu: motor kırpıyordu, editör büyütüyordu (2026-08-15).** Gölge-caster
   yerleşimini karşılaştırırken çıktı. Motor instans yüklemesini `instance_capacity`'ye kırpıp
   kaç tanesinin GPU'ya ulaştığını döndürüyor; **`Renderer::ensure_instance_capacity` ise var,
   birim testi de var, ve tek çağıranı studio.** Yani 8 192 instans'ı aşan bir sahne oyunda
   geometri kaybediyor, aynı sahne editörde tam çiziliyordu. `is_double_sided` ile birebir aynı
   sınıf: motorun dışa verdiği bir yetenek, motorun kendi yolunun kullanmadığı.
   İki bölgeli yerleşim (A = bütün batch'lerin kamera instansları, B = bütün gölge-caster'lar)
   bu kırpmanın **zarif bozulması** için yapılmıştı — artık bozulacak bir şey yok, ama tampon
   büyütmeyi reddederse diye bekçi olarak duruyor. Studio'nun batch-başına `[kamera][gölge]`
   yerleşimi motorun terk ettiği düzen ama zararsız, çünkü tam da tamponu büyüttüğü için hiç
   kırpmıyor — kayda geçti.
   **Kanıt:** 9 000 küp, tek batch; GPU'ya ulaşan sayı ölçülüyor. Büyütme çağrısı kaldırılınca
   test "9 000'in 8 192'si ulaştı" diyor — motorun düne kadarki davranışı. Büyüme kasıtlı olarak
   sınırsız ve editörle aynı: 200 000 instans isteyen bir kare 25 MB tampon alır; HashMap'in en
   sona koyduğu mesh'lerin kaybolduğu bir resim yerine.

9. **Harmanlama sırası: motor batch içini sıralamıyordu (2026-08-15).** `gizmo-renderer::draw_order`.
   Saydam boru hattı derinlik yazmıyor, yani harmanlanan geometride **çizim sırası sonucun
   kendisi**. Motor batch'leri sıralıyordu ama her batch'in instanslarını toplandıkları sırayla
   ekliyordu; editör ikisini de sıralıyordu. Aynı malzemeden iki üst üste saydam yüzey — bir
   binadaki pencere sırası, üst üste camlar, birden çok kez instans'lanan herhangi bir saydam prop
   — oyunda **ECS gezinme sırasına** göre harmanlanıyordu, editörde doğru. Bir de ikiz vardı:
   batch'in temsili derinliği motorda `batch_sort_depth`, studio'da `batch_centroid_depth` —
   harfi harfine aynı hesap, iki isim, iki test. İkisi de tek modüle indi.
   **Kanıt:** iki üst üste saydam pane, **farklı renklerde** ama tek batch'te (batch anahtarı
   malzemenin *doku* bind group'u, ikisi de aynı dokudan); sahne bir kez yakın-önce bir kez
   uzak-önce kuruluyor ve iki kare bayt bayt eşit olmak zorunda. Düzeltme kaldırılınca 7 143 bayt
   farklı.
   **Not:** bu testin ilk hâli **boştu** — iki pane aynı renkti, `c over (c over bg)` iki sırada da
   aynı ifade, yani düzeltme kaldırıldığında da geçiyordu. §8'deki "bekçinin kırılabildiğini
   gör" alışkanlığı tam olarak bunu yakaladı; olmasa yeşil ama hiçbir şey ölçmeyen bir test
   commit edilmişti.

10. **Yerleştirilmiş backdrop editörde kameraya yapışıyormuş (2026-08-15).** Studio
    `backdrop::instance_model`'ı hiç çağırmıyordu. Backdrop boru hattının vertex shader'ı kamera
    konumunu ekliyor; `MaterialType::BackdropPlaced` için bu eklemenin CPU'da geri alınması gerek
    ki ikisi sadeleşsin ve backdrop seviyenin koyduğu yerde kalsın. Motor bunu hep yapıyordu,
    studio ham matrisi yüklüyordu — yani seviyeye yerleştirilen bir backdrop **editörde kamerayla
    sürükleniyor, oyunda duruyordu.** Kuralın kendisi zaten `gizmo-renderer::backdrop`'ta yazılı ve
    testli (`a_placed_backdrop_lands_where_it_was_authored`); eksik olan tek şey çağrıydı.
    Bekçi: iki çizim döngüsü de yazdığı matrisi `instance_model`'dan geçirmek zorunda — ve bekçi
    **yorumu saymıyor**, yoksa çağrıyı silip açıklamayı bırakmak testi geçerdi.

11. **Frustum culling — sürüklenme yok, kayda geçsin.** Sıradaki aday buydu; bakınca iki yol da
   zaten `classify_visibility_world`'ü paylaşıyor, AABB'yi bir kez dönüştürüyor ve farklarını
   gerekçesiyle yazmış: editör oyun kamerasının frustum'una göre kırpıyor (edit modunda culling'i
   sınayabilmek için) ve kameraya-kilitli backdrop'u testten muaf tutuyor — çünkü onun shader'ı
   *aktif* kameraya kilitlenirken culling frustum'u *oyun* kamerasınınki, yani testi anlamlı kılan
   bir matris yok. Motor bunun yerine kilitli matrise göre kırpıyor. Bu iyi durum; bir daha
   "bakılacak" listesine girmesin diye yazıldı.

Açık kalan: çizim listesi hâlâ iki ayrı uygulama. Tek tek ortak kararlar (routing, uniform bloğu,
kurulum, LOD, paketleme, çift-yüzlülük) tek kaynağa indi; `collect_draw_items` ile studio'nun
batching'i **tek koda** inmedi ve inmesi de kendi başına bir karar — pass kaydı gerçekten ayrı,
ayıran çizgi "dünyayı okuyan kısım ortak, komut kaydeden kısım ayrı" olarak tutuluyor. Anahtarların
kalan farkı da kayıtta: studio `is_grid`'i anahtarlıyor (motorda grid boru hattı yok), motor
`is_transparent`/`baked_lit`'i anahtarlıyor (studio bunları ayrı HashMap'lerle ayırıyor) — ikisi de
mekanizma farkı, sürüklenme değil.

**İnsan gözü isteyen iki yer:** (1) editör viewport'unda DoF artık kameranın gerçek near/far'ıyla
lineerleşiyor — varsayılan kamerada (0.1/2000) fark yok, farklı far düzlemli sahnede odak doğru
yere kayacak. (2) Editörün sahne bloğunda `cascade_params.w` artık 0 yerine gerçek caster indeksi
taşıyor; editörün forward shader'ı bu yuvayı okumuyor (`point_shadows_enabled` de 0), yani
görünür bir etki beklenmiyor — ama ikisini de test edecek koşum yok, §3'ün A/B kapısına düşer.

### Doğrulanmış ama henüz el atılmamış kökler

(Bu listedeki son madde de kapandı — aşağıya bakın.)

Kapatılanlar (**2026-08-14**), ayrıntısı §7'de:

- **Sözleşmeler** → `gizmo-renderer::shader_contract`. Kendi `SceneUniforms`'unu bildiren yedi
  shader'ın hepsi bloğun bir **ön eki**; meşru, ama ön eki kimse denetlemiyordu. Yeni testler
  öznelerini shader dizininden, cevaplarını **naga**'dan alıyor: her adlandırılmış alanın bayt
  ofseti Rust'taki `offset_of!` ile karşılaştırılıyor. Bugün sürüklenme yok — sekiz bildirim de
  uyuşuyor. Elle özne sayan eski ayna testi silindi (kapsamı yenisinde), yanlış iddia eden
  yorumu düzeltildi. Kırılabildiği doğrulandı: bir kopyada iki alan yer değiştirdi, bir başkasında
  kuyruk alanı silindi; ikisi de dosya/alan/ofset vererek kırmızıya düştü.
- **Shader boru hattı** → `compose_module` artık doğrulanmış modülü döndürüyor, `compose_wgsl` onun
  üstünde ince bir metin sarmalayıcı. Refaktör kendisi için değil: derlenen shader'lar bind-group
  indekslerini satır içi yerleştiriyor (`@group(#{INSTANCE_GROUP})`), yani onları yalnız naga
  okuyamıyor — sözleşme testinin gerçek kompozisyon yoluna ihtiyacı vardı.
- **Crate grafiği / Stage A** → ölçüldü. `gizmo-animation` bağımlılık olarak zaten temiz
  (`gizmo-core`/`math`/`physics-core`, `default = []`, `wgpu` yok) — yani Stage A ölçütünü
  karşılıyor; bayat olan **grafik belgesiydi**. Gerçek grafik `cargo metadata`'dan alındı ve
  CLAUDE.md'deki diyagram üç yerde yanlıştı: `gizmo-ui` aslında `gizmo-app`'in **üstünde** (ona
  bağlı), `gizmo-window`'un hiçbir workspace bağımlılığı yok, ve `gizmo-animation` renderer ile
  scripting'in **altında** — yanlarındaki bir yaprak değil. Diyagram düzeltildi.
  Asıl kazanç ölçüm değil, artık sınanan değişmez: `crates/gizmo/tests/crate_staging.rs`
  manifestlerden grafiği okuyup **hiçbir Stage A crate'inin Stage B'ye bağlanmadığını** doğruluyor
  (bağlansaydı o crate, listelendiği sürümü hızlı katmanın her kırıcı değişikliğiyle birlikte
  yapmak zorunda kalırdı), her crate'in tam bir aşamada sınıflandırıldığını (yeni crate = karar
  anı) ve `gizmo-core`/`gizmo-math`'in taban olarak kaldığını. Kırılabildiği doğrulandı
  (`gizmo-audio → gizmo-scripting` kenarı eklendi, kırmızıya düştü).
- **Determinizm çevresi** → `snapshot()`/`restore_snapshot()` artık `..` içermeyen **tam yıkım**
  (destructure) yapıyor: `PhysicsWorld`'e (ya da `WorldSnapshot`'a) eklenen bir alan orada derleme
  hatası. `_` ile bağlanan 19 alanın her biri gerekçesiyle yazılı — yapılandırma, türetilmiş,
  çıktı, yapı, kontrol bayrakları. Davranış aynı: aynı dokuz alan taşınıyor,
  `headless_stress_test` üç eşleşen hash veriyor. Bekçi doğrulandı (alan eklendi, `E0027` desenin
  üstüne düştü). Ayrıntısı §5'te.

### Scripting

Bu bölüm bir zamanlar "32 doğrulanmış kusurdan yedisi düzeltildi" diye başlıyordu. **Kalan 25'in
arkasında yazılı tek bir madde yok** — bunu bir sonraki okuyanın keşfetmesine bırakmamak için burada
söylüyor: 32 sayısı, madde listesi saklanmamış bir taramadan geliyor. Aşağıdaki yedi maddenin
yanındaki not ("kayıtta adı geçen bütün açık maddeler") o taramanın yazıya geçmiş kısmının tükendiğini
söylüyor. Dolayısıyla "25 kalan kusur" bir iş kalemi değil; ne kovalanabilir ne kapatılabilir.
Scripting'de bir sonraki adım, sayıyı azaltmak değil **taramayı yeniden koşturup bu kez maddeleri
yazmak**. Sayı bu yüzden buradan kaldırıldı: arkasında iş olmayan bir sayaç, ilerleme ölçüsü gibi
görünüp değil.

Düzeltilmiş ve kaydı tutulan yedi kusur. İlk üçü: script sırası (`HashMap` → `BTreeMap`,
proses başına rastgeleydi), on altı komutun sessizce yutulması, bir script'in hatasının ötekileri
iptal etmesi. **2026-08-15'te dördü daha — kayıtta adı geçen bütün açık maddeler:**

- **Tuş haritası her girdisinde yanlıştı.** Lua tablosu **USB HID** kullanım kodları taşıyordu
  (`w = 17`, `space = 44`), motor ise `winit::KeyCode as u32` saklıyor (KeyW = 41, Space = 62).
  İkisi de gerçek numaralandırma; aynı numaralandırma değil ve ikisini karşılaştıran hiçbir şey
  yoktu. En kötüsü: `down = 81` ve `right = 79`, winit'in ArrowRight ve ArrowDown'ı — yani ok
  tuşlarını okuyan bir script, oyuncu aşağı bastığında sağa gidiyordu. `up`/`left` ise kendi
  kodlarına denk gelmişti, yani girdi yarı çalışıyor gibi görünüyordu. Test de yakalayamazdı:
  aynı transkripsiyonu kullanıyordu (17'ye basıp "w" iddia ediyordu) — aynası kopya olan bir ayna
  testi. Sayılar artık `gizmo_core::input::keys`'te bir kez, winit'in bildirim sırasından
  üretilmiş; doğrulama winit'i gören crate'te (`gizmo-app`), hiçbir girdi denetimsiz kalmayacak
  bir kapsam iddiasıyla.
- **Sonsuz döngü prosesi bitiriyordu.** Ne komut sayacı ne bellek tavanı vardı: `while true do end`
  yield etmiyor, `call` dönmüyor, kare bitmiyor, pencere kapanma olayını bile işlemiyor. Yakalanacak
  sinyal ve yardımı dokunacak bir watchdog yok — VM'i ancak VM kesebilir. 10 000 komutta bir çalışan
  hook **çağrı başına** bütçe harcıyor (kare başına değil: `update` bütün scriptleri koşturur ve
  ortak bütçe, ilk hatalı script'in herkesinkini harcaması demekti — hata izolasyonundan kaldırılan
  aynı kusurun başka para birimindeki hâli). Bellek tavanı da taşan ayırmayı yakalanabilir Lua
  hatasına çeviriyor. Negatif kontrolü farklı: hook kaldırılınca test kırmızıya düşmüyor, **asılıyor**.
- **`_G` paylaşımlı tabloydu.** Her script kendi env'inde koşuyordu, yani örtük `FOO = 1` yereldi —
  ama `_G`, env'in `__index`'i üzerinden motorun globals'ına çözülüyordu, dolayısıyla her Lua
  öğreticisinin "global yapmanın açık yolu" diye yazdığı `_G.FOO = 1` paylaşılan tabloya yazıyordu.
  Ölçüldü: A `_G.LEAK` yazdı, B `from-a` okudu. Yükleme sırası alfabetik olduğu için bu, sırayla
  yarışan bir paylaşım. `_G` artık script'in kendi tablosu; okumalar hâlâ düşüyor, yani API ve
  standart kütüphane aynen görünür.
- **NaN doğrulaması on bir varyanttan birinde vardı.** `sanitize_dim` yalnız collider ölçülerini
  kapsıyordu; pozisyon, kuvvet, impuls, hız, kamera fov'u, animasyon hızı ve hasar denetimsizdi.
  Denetim artık her komutun geçtiği tek yerde — kuyrukta — ve match'i **tam**: `_` kolu yok, yani
  float taşıyan yeni bir varyant orada derleme hatası (doğrulandı: `E0004`). Komutlar kırpılmıyor,
  düşürülüyor: kırpılmış bir kuvvet karenin sessizce kabul ettiği yanlış cevaptır.

- **API tabloları paylaşılan nesnelerdi** (aynı gün kapatıldı). `_G` yalıtımı bir script'in
  *global*lerini kendine ait yaptı; API tablolarını yapmadı, çünkü `input.is_pressed = f` bir
  global yazımı değil, her script'in elinde tuttuğu bir **nesnenin alanına** yazma. Ölçüldü: A
  `input.is_pressed`'i değiştirdi, B A'nın sürümünü çağırdı.
  Çözüm çıplak bir `__newindex` **değil** — o metametot yalnız tabloda *olmayan* anahtarlar için
  tetikleniyor ve ezilmeye değer her anahtar zaten var. Bu yüzden script'in gördüğü global boş bir
  **proxy**: her okuma ıskalayıp `__index` üzerinden gerçek tabloya gidiyor, her yazma —yeni
  anahtar olsun olmasın— `__newindex`'e düşüp reddediliyor. Gerçek tablo Lua'nın adlandıramadığı
  **registry**'de duruyor; `__metatable = false` de `getmetatable` ile çıkarılmasını engelliyor.
  Motor kendi kare-başı yazımlarını `raw_set` ile yapıyor: yazan motor, okuyan script.
  Dokuz modülün on dördü tablo bu desene geçti. Yan kazanç: `entity` ve `scene`'in Lua
  yardımcıları **her frame yeniden tanımlanıyormuş** (kendi yorumu "idempotent" diyordu) — artık
  kayıt sırasında bir kez.
  Kırılabildiği doğrulandı: proxy yerine klasik yanlış çözüm (`__newindex`'li gerçek tablo)
  konunca test kırmızıya düşüyor.

**Hâlâ açık:** bir script kendi env'inde adı gölgeleyebilir (`input = başka_şey`) — ki `_G`
yalıtımı bunu zararsız kılıyor, gölge o script'e özel. Ve motorun global olarak sunmak yerine
*değer olarak* verdiği tablolar bu kapsamda değil.

**Teşhis sondası köprüsü — engel diye kaydedilen şey engel değilmiş (2026-08-15).** Kayıt şöyle
diyordu: `send` özelliğiyle `create_function`'ın kapanışı `Send + 'static` olmak zorunda, yani bir
Lua callback'i `&World` yakalayamaz; bütün okuma yolu bu yüzden kare-başı anlık görüntü, ve
parametreli sorgu (şu (x,z)'de zemin kotu) anlık görüntüyle ifade edilemez.

Muhakemenin ilk yarısı doğru, sonucu yanlış. O sınır `Lua::create_function`'ın; **`Scope::create_function`'ın
sınırı `F: Fn(..) + 'scope` — ne `Send` ne `'static`.** Yani kapsamlı (scoped) bir kapanış dünyayı
ödünç alabiliyor, ve ödünç kapsam bitince bitiyor: tam da karenin ömrü.

Çalışan hâli: `api_physics::with_call_time_queries` kareyi bir `lua.scope` içine alıyor, `&World`
tutan `physics.ground_at(x, z)`'yi kuruyor, scriptleri koşturuyor, çıkarken adı geri siliyor —
böylece adı saklayıp sonra çağıran bir script "destructed callback" yerine sade bir nil alıyor.
Testi kayıttaki örneğin kendisi: üstü y=2'de olan bir zemin plakası; script `ground_at(0,0)` için
2.0, plakanın dışında `nil` alıyor (0.0 değil — zemin yok ile zemin sıfırda aynı cevap değil).
Kırılabildiği doğrulandı: fonksiyon kurulmayınca test kırmızı.

Yani sonda köprüsü artık bir tasarım kararı beklemiyor; mekanizma kurulu ve tek tüketicisi var.
Kalanı sıradan iş: hangi sorguların sunulacağı.

### Rollback: iki uygulama, biri eksik (2026-08-15)

`gizmo-net`'te iki rollback var. `RollbackSession` `PhysicsWorld`'ü otoriter sayıp tam
`WorldSnapshot` ile yedekliyor; **windowed app'in bağladığı** `RollbackManager` ise ECS'i otoriter
sayıp entity başına altı sayı tutuyor: konum, dönüş, iki hız, uyku bayrağı.

Aradaki fark, fizik crate'inin "transform ve hızdan **türetilemez**" diye belgelediği her şey:
substep accumulator, contact cache'in warm-start impulse'ları, eklemlerin tek yönlü `is_broken`
mandalı ve mandallanmış referans pozları, oyunun kare içinde değiştirebildiği kuvvet alanları ve
sıvı hacimleri. Üstelik app, rollback'ten sonra `pw.clear_bodies()` çağırıp fizik dünyasını ECS'ten
yeniden kurduruyordu.

**Önce yanlış ölçtüm, kayda o da geçsin.** İlk testim "ıraksıyor" dedi (C9C742C6… ≠ 992A5D82…) ama
ıraksamayı üreten şey testimdeki bir fazla adımdı: `end_frame` tick'i adımdan *sonra* kaydediyor,
yani tick T'ye dönmek "T+1 adım atılmış" duruma dönmek. Hesap düzeltilince o senaryo — dört düşen
kutu — **düzeltmeyle de düzeltmesiz de** geçiyor. İki uygulamayı ayırt edemeyen bir test hiçbiri
hakkında kanıt değil.

Ayırt eden senaryo: **pencere içinde kopan bir eklem.** `is_broken` yalnız `PhysicsWorld`'de yaşayan
tek yönlü bir mandal, yani bileşen geri yüklemesiyle geri alınamaz — re-simülasyon, kesintisiz
koşunun hâlâ sahip olduğu bir eklem olmadan devam ediyordu. Test tick 12'de kopacak şekilde
ayarlı (hedef 8, geri sarma 20) ve kopmanın pencere içinde olduğunu ayrıca doğruluyor; düzeltme
kapatılınca "eklem hâlâ kopuk" diyerek kırılıyor.

Düzeltme, denetlenmiş uygulamayı yeniden kullanıyor: `RollbackManager` artık tick başına
`PhysicsWorld::snapshot()`'ı da **yerel** olarak tutuyor (tel formatı `PhysicsStateSnapshot` aynen
kalıyor — `WorldSnapshot` contact manifold ve eklem taşıyor, ağa gitmez). Geri yükleme fizik
dünyasını kurup satırlarını ECS'e de yazıyor, çünkü bir sonraki adımda `sync_bodies` ECS'ten
kopyalayıp düzeltilen satırları ezerdi — fonksiyonun kendi dokümanının uyardığı şey. App'in
`clear_bodies()`'i kalktı (artık geri yüklenen durumu atardı) ve fast-forward döngüsü
`record_resimulated_tick`'i çağırıyor; o döngü `end_frame`'i satır içi tekrar yazdığı için fizik
yarısı oradan da eksikti.

### Açık kalan iki karar (kusur değil)

- **Çizim listesi birleştirmesi.** `collect_draw_items` (~950 satır) ile studio'nun batching'i hâlâ
  iki uygulama. Ölçüye dayanarak yapılmadı: bugün bulunan sekiz sürüklenmenin hepsi ortak bir
  *karardaydı* ve hepsi tek kaynağa indi, her biri bekçiyle. Kalan ikizlik döngü **yapısı**, ve onu
  birleştirmek pass kaydını birleştirmek demek — otomatik kapsamı olmayan ve insan-gözü kapısına
  bağlı olan yarı. Çizgi: "dünyayı okuyan ortak, komut kaydeden ayrı".
- **Sonda köprüsünde hangi sorgular sunulacak.** Mekanizma kuruldu ve tek tüketicisi var
  (`physics.ground_at`); geri kalanı API tasarımı.

### Decal editörde görünmüyordu (2026-08-17, DÜZELTİLDİ)

Bir decal bir **projektör**dür: zaten orada olan yüzeyi boyar, dolayısıyla o yüzeyin konumuna
ihtiyacı vardır. Motorun deferred geçişi bunu G-buffer'dan okur — oyun için çalışır, editör için
hiç çalışmaz: stüdyo forward çiziyor ve G-buffer doldurmuyor. Sonuç, editörün WYSIWYG olma iddiası
için en kötüsü: kullanıcı decal'ı yerleştiriyor, boş zemin görüyor, "bozuk" diye kaydediyor —
oysa gönderilen oyunda splatter oradaydı. `render_parity.rs` bunu "gerçek ve **düzeltilmemiş**"
diye kaydetmişti; bu, o kaydı kapatıyor.

**Düzeltme:** forward bir decal geçişi (`decal_forward.wgsl` + `record_forward_decals`), yüzey
konumunu **derinlik tamponundan** geri kuruyor — forward hattın zaten yazdığı tampon — ve ışıklı
HDR görüntüsüne alfa harmanlıyor. Projeksiyon matematiği, hacim testi ve dairesel solma deferred
sürümün birebir aynısı: editörde bir türlü, oyunda başka türlü görünen bir decal, hiç
görünmeyeninden kötüdür.

İki ayrıntı kayda değer:

- **Dünyayı okuyan taraf ortak.** `collect_decals` (facade'da, çünkü `DecalState` renderer'da ve
  `Transform` physics-core'da) her iki geçişi de besliyor. İkisinin anlaşmak zorunda olduğu şey
  decal'ın *nerede* olduğu; nasıl kaydedildiği değil.
- **Tek uniform, iki shader.** CPU tarafı `inv_model`'i `model⁻¹ · T(kamera)` olarak katlıyor,
  çünkü deferred okuyucu kameraya göreli konum veriyor. Forward shader da geri kurduğu mutlak
  konumdan kamerayı çıkarıp aynı matrisi kullanıyor — böylece iki hat aynı tamponu paylaşıyor ve
  "decal nerede" sorusunda ayrışamıyorlar.

Ayrıca derinlik tamponu bu geçişte **örnekleniyor**, attachment değil (wgpu ikisine birden izin
vermez) — parçacık geçişinin kalıbı. Ön yüzler kırpılıyor: derinlik testi olmadan kutunun iki yüzü
de rasterize olur ve her piksel iki kez harmanlanırdı.

Doğrulama, stüdyonun kendi piksel koşumunda: `a_decal_is_visible_in_the_editor_viewport`. Geçiş
kapatılıp koşuldu — **0/16384 piksel**; açıkken geçiyor. Ve `render_parity.rs`'teki `Decal`
istisnası kaldırıldı: artık iki hat da tanıyor.

### Ölü biçimdeki iki varlık: artık ne oldukları söyleniyor (2026-08-17, KAPANDI)

Depoda iki dosya, ikisi de motorun kendi varlıkları, ikisi de yüklenemiyordu — ve ikisinin de
hatası yanlış şeyi söylüyordu. Denetim (2026-08-04) bunu zaten kaydetmişti; kapatan bu.

**1. `perfect_car.scene` — reflection çağından.** Bileşenleri alan-adı anahtarlı iç içe map olarak,
enum'ları iç etiketli (`"shape": {"type": "Aabb", …}`) yazıyor; bugünkü biçim her bileşeni bir RON
**string**'i olarak yazıyor. Ayrıştırma tipli deserialize'da düşüyor — yani `migrate` dosyanın
sürümüne bakamadan — ve kullanıcıya kalan mesaj `10:28: Expected string` oluyor. Sürüm alanının
tam da bu yüzden var olduğu bir dosyada, sürüm mekanizması devreye giremiyor.

Yeni davranış: ayrıştırma düştüğünde dosya bir kez de `ron::Value` olarak okunuyor ve doğrudan
sorulan soru şu — **bileşen yükü string mi, map mi?** Map'se hata `SceneError::LegacyComponentEncoding`
ve mesaj ne olduğunu ve ne yapılacağını söylüyor. Bu ikinci ayrıştırma yalnız zaten başarısız olmuş
yolda koşuyor.

Dosyanın kendisi silinmedi, **fixture oldu**: `crates/gizmo-scene/tests/fixtures/legacy_reflection.scene`.
Sentetik bir örnek değil, gerçek relik — ve testi o çalıştırıyor. Üç bekçi: eski dosya adıyla
anılıyor mu, sıradan bozuk bir dosya hâlâ `Parse` mı (yoksa her hata "sahneniz eski" olur), ve
bugünkü biçim hâlâ yükleniyor mu.

**2. `prefab_8.prefab` — ikili çöp.** RON değil, metin bile değil; `read_to_string` UTF-8'de
düşüyor ve kullanıcının gördüğü şey **"scene file I/O error"** — yani yerinde duran, okunabilir bir
dosya için "dosya açılamadı" diyen bir mesaj. Dosya silindi (export her oyuna kopyalıyordu), ve
`InvalidData` durumu artık ayrı raporlanıyor: "bu dosya metin değil, başka bir (muhtemelen ikili,
eski) biçimde yazılmış". Yanındaki `Default_Cube.prefab` ölçüldü, **yükleniyor** — o sağlam.

### Kalan GPU alt sistemleri tarandı: biri hiç koşmuyormuş (2026-08-16)

Aynı alet (geçişi kaldır, pikseli say) hiç ölçülmemiş dört isteğe bağlı sisteme tutuldu.

| Sistem | Varsayılan | Sonuç |
|---|---|---|
| `gpu_particles` | **açık** | **çalışıyor** — emitter'lı sahnede 10 karede 197/16384 px, 60 karede 355, max delta 142 |
| `gpu_particles` + `gpu_fluid` | **açık** | parçacığı/sıvısı olmayan sahnede **0/65536 bayt** — boşta duruyorlar, kirletmiyorlar |
| `smoke` | kapalı | ölçülmedi; varsayılan `None`, demo `Some(SmokeVolume::new(..))` veriyor |
| `gpu_fluid` (kullanıldığında) | — | ölçülmedi; SPH parçacığı olan bir fixture gerekiyor (`fluid_rigid` demosu bu yolu koşturuyor) |
| **`gpu_cull`** | kurulu | **hiç koşmuyor — kaldırıldı** |

**`gpu_cull`'un hikâyesi** kayda değer, çünkü SSR/SSGI'den farklı bir ölü türü: geçiş yanlış
çalışmıyordu, **hiç çağrılmıyordu**. `GpuCullState` her renderer'da kuruluyordu (compute pipeline,
üç tampon, bind group), `prepare()` ve `cull_pass()` eksiksiz yazılmıştı — kapasite taşmasını
uyaran bir `warn!`'ı ve `clamped_draw_count` için kendi birim testi bile vardı — ama depoda tek bir
çağıranı yoktu. Gerekçe de koddaydı, `default_render_pass`'in içinde:

```rust
// GPU cull pass removed since we use CPU instancing
```

Yani karar zaten verilmiş, yalnız kurulum geride kalmış. Silinen: modül (256 satır), `mesh_cull.wgsl`,
`Renderer::gpu_cull` alanı, kurulumu, ve `WebProfile::gpu_cull_enabled` — o bayrağın da okuyanı
yoktu, yalnız iki testte doğrulanıyordu. CPU tarafındaki `frustum_cull` çalışan yol ve öyle kalıyor.

**Geri istenirse ne gerekir:** eksik olan yarı `prepare`/`cull_pass` değil, **çizim** tarafı —
batch'lerin `draw_indirect`'e taşınması ve mesh sınırlarının her kare yüklenmesi. Ve deponun kendi
kuralına göre önce bir ölçüm: CPU culling'in karede ne kadar tuttuğu. (Aynı kural narrowphase
batch-SIMD'i %3'te reddetmişti.) Kod git geçmişinde duruyor.

Bekçi: `particles_from_an_emitter_reach_the_frame`. Böylece "kareye ulaşıyor mu" ailesi dörde
çıktı — SSR/SSGI, volumetric, decal, parçacıklar.

### Pencereli her uygulama ilk karede ölüyordu (2026-08-16, DÜZELTİLDİ)

`cargo run -p demo --bin advanced_physics` — CLAUDE.md'nin belgelediği komut — açılıştan ~1 saniye
sonra `Dropped TexturesDelta with 1 unapplied deltas` ile ölüyordu. Export çalışma-zamanı için ilk
sahne denemesinde çıktı ve kusur yeni binary'de değil, motorun pencere döngüsündeydi: deponun
kendi demosu da aynı biçimde ölüyor.

Kare şöyle akıyor: egui karesi en başta koşuyor (`event.rs:574`), sonra swapchain görüntüsü
alınıyor. Alınamazsa — outdated surface, uçuşta bir resize, timeout; ve **yeni haritalanmış bir
pencerenin ilk karesi tam olarak budur** — tek bir epilog erken dönüyor ve o `FullOutput`
uygulanmadan düşüyordu.

Maliyeti iki katmanlı, ve sessiz olan yarısı daha kötüsü:

- **Debug**: `TexturesDelta::drop` bir `debug_assert!`. Süreç ölüyor — yani hiçbir pencereli demo
  debug'da açılamıyordu.
- **Release**: assert sessiz. Ama egui her deltayı **bir kez** verir; düşen çıktı o karenin
  taşıdığı yüklemeleri kalıcı olarak götürür. Atlas o karede yeniden kurulmuşsa (ölçek/DPI
  değişimi, font değişimi) yazı bir daha geri gelmez, glyph'ler boş kutu olarak çizilir.

Aynı kusurun ikinci yüzü `EguiContext::render`'ın kendisindeydi: deltaları referansla uygulayıp
listeyi `clear()` etmiyordu, dolayısıyla **çizilen** karede de aynı assert'e düşülüyordu.

Düzeltme `absorb_unpainted_frame`: çizilmeyecek karenin dokularını yükleyip listeyi temizler,
platform çıktısını da uygular (UI koştu — imleç/pano onun sonucu). Atlanan kare **pikselleri**
atlar, yüklemeleri değil. `render` de uyguladıktan sonra temizliyor.

Doğrulandı: düzeltmeden önce `advanced_physics` 1 s içinde exit 101; sonra 25 s boyunca ayakta ve
`GIZMO_SCREENSHOT` ile kare üretiyor. Bekçiler `egui_frame_ownership_tests` — kaynak-şekli
testleri, çünkü "surface outdated" durumu gerçek bir swapchain olmadan birim testine sokulamıyor.

Neden bugüne kadar görülmedi: demolar `--release` ile koşuluyor (CLAUDE.md fizik demoları için
bunu şart koşuyor), release'te panik yok, ve kalan zarar hem sessiz hem koşullu.

### Post-process kontrolleri taraması: SSAO dışında hepsi çalışıyor (2026-08-16)

Gölgeleme çiplerinin üçünün ölü çıkması üzerine, editörün TÜM render kontrolleri aynı yöntemle
ölçüldü — headless 128×128 render, kontrolü kıpırdatıp piksel farkı say:

| Kontrol | Fark | |
|---|---|---|
| bloom eşiği · exposure · vignette · film grain | %66–75 | çalışıyor |
| dof blur · fxaa | %1.5–2.0 | çalışıyor |
| dof odak aralığı · chromatic aberration | %0.3–0.7 | çalışıyor |
| bloom yoğunluğu | %75 (eşik 0'da) | çalışıyor |
| dof odak mesafesi | %1.6 (aralık ≥20'de) | çalışıyor |
| **SSAO** | %0 | **ölü — zaten belgeli, widget'ları kapalı** |

Yeni kusur yok. Ama sonuca varmak **iki kötü deney** gerektirdi ve ikisi de önce "bu kontrol ölü"
diye okundu; bir daha aynı tuzağa düşülmesin:

- **Bloom yoğunluğu**, varsayılan eşikte 0 fark verir. Sahnede eşiği aşan hiçbir şey yoksa
  yoğunluğun büyütecek bir şeyi yoktur. Eşiği 0'a indirmeden ölçmek, kontrolü değil fixture'ı
  ölçer.
- **DoF odak mesafesi**, dar aralıkta 0 fark verir. `coc = clamp(|view_dist - focus| / range, 0, 1)`
  — `range` küçükken her iki odak değeri de coc'u 1.0'a doyurur, iki resim de eşit derecede
  bulanık olur. Aralığı ≥20 tutmadan ölçmek yine deneyi ölçer.

Genel kural: bir kontrolün ölü olduğunu ancak **etkili olabileceği** bir zeminde ölçtükten sonra
söyle. Gölgeleme çipleri gerçekten ölüydü (her zeminde 0/65536); bu ikisi değildi.

### SSR ve SSGI iki karakter yüzünden ölüydü (2026-08-16, DÜZELTİLDİ)

Aynı mercek bir seviye aşağı tutuldu: kontrolü değil **geçişin kendisini** kaldır
(`renderer.ssgi = None`) ve kareyi baytına kadar karşılaştır. Dört sahne, beş ekran-uzayı geçişi:

| Geçiş | Kaldırılınca fark | |
|---|---|---|
| SSAO | %10–15 | çalışıyor — *kontrolü* ölü, o ayrı konu (yukarıdaki tablo) |
| TAA | %0,4–1,6 | çalışıyor |
| volumetric | bu sahnelerde eşik altı (max delta 5–8) | **fixture'dı** — doğru zeminde %14,5, aşağıya bak |
| **SSGI** | **0/65536 bayt** | **ölü** |
| **SSR** | **0/65536 bayt** | **ölü** |

Sıfır "az" demek değil, **birebir aynı kare**: durum kuruluyor, pass kaydediliyor, pipeline
derleniyor, her karede koşuyor — ve toplamsal apply'ı kareye hiçbir şey eklemiyor.

Sebep tek eşik. `gbuffer.wgsl` "bu piksel yazıldı" bayrağını world_position'ın alfasına paketliyor:

```wgsl
let packed_ss_aniso = (0.5 + 0.49 * anisotropy_raw) + floor(100.0 * subsurface_raw);
```

Sıradan bir malzemede (anizotropi 0, subsurface 0) bu **tam 0,5**, ve 0,5 Rgba16Float'ta tam
temsil edilir — yuvarlama payı yok. Bayrağı okuyan on yerden sekizi kapsayıcı yazılmış
(`< 0.5` → atla, ya da `>= 0.5` → geçerli): ssao'nun ikisi, deferred_lighting, volumetric, taa,
ssgi_temporal ve SSR/SSGI'nin **kendi giriş kapıları**. Işın yürüyüşünün isabet testi ise iki
shader'da `> 0.5` idi — `ssr.wgsl:71` ve `ssgi.wgsl:107` — yani sıradan her piksel için yanlış.
Işın 20 (SSR) / 8 (SSGI) adımını sonuna kadar koşup hiçbir isabet kaydedemiyor, siyah dönüyordu.

`>= 0.5` yapıldı. Ölçüm (ayna zemin, 128×128, pass açık vs. kaldırılmış):

| | önce | sonra |
|---|---|---|
| SSGI | 0 px · 0 bayt | 2005/16384 px (%12,2) · 7959 bayt · max delta 64 |
| SSR | 0 px · 0 bayt | 426/16384 px (%2,6) · 1908 bayt · max delta 26 |

Bekçi: `screen_space_reflections_and_gi_reach_the_frame` (`golden_render_tests`). Eşikler ölçülenin
çok altında — korunan şey "geçiş kareye ulaşıyor mu", ayarı değil. Düzeltme geri alınıp test
koşuldu: `removing SSGI changed 0/16384 pixels` ile kırmızı, yani kusuru gerçekten tutuyor.

**Düzeltme nereye ulaşıyor.** `default_render_pass` — yani oyun yolu; `platformer`, `vehicle_scene`,
`cloth_demo` gibi geçişleri açık bırakan demolar bugünden itibaren gerçekten SSR/SSGI görüyor.
Editör viewport'u **değişmez**: stüdyonun kendi hattı bu geçişleri hiç kaydetmiyor (`gizmo-studio`
içinde `ssr`/`ssgi` geçmiyor, "iki render yolu" bölümüyle tutarlı), `SimpleApp` ile
`with_scene_render` ise ikisini bilerek `None`'a çekiyor. Değişikliği stüdyoda arayan biri
"düzelmemiş" sonucuna varır.

**Neden yıllarca hiçbir şey yakalamadı.** Kurulum, kayıt, bind group, shader derlemesi — her yan
sağlıklı görünüyordu; yalnız resim biliyordu. Ve zemin seçimi burada da belirleyici: SSGI ayna
zeminde %12,2 verirken kırmızı duvarlı "bounce" sahnesinde eşiğin altında kaldı (max delta 6).
Toplayacak parlak komşu yoksa GI'yi ölçmek yine fixture'ı ölçer.

**Ve aynı tarama volumetric'i az kalsın yanlış mahkûm ediyordu.** Yukarıdaki tabloda "max delta
5–8" satırı, bu belgenin bir bölüm yukarıda yazdığı kuralın ihlaliydi: ölçüldüğü dört sahnenin
hiçbirinde kamera güneşe bakmıyordu. Katkının tamamı `sun_intensity · faz · yürüyüş boyu`, ve
kamera güneşe dönmediğinde üçü birden çöküyor — Henyey-Greenstein lobu `g = 0,55`'te güneşe
doğru 0,61, tersine 0,015 (kırk kat), ve yakın geometriye çarpan ışın gökyüzünün 100 birimi
yerine 6 birim yürüyor. Kamera güneşe çevrilince (`yaw = π/2, pitch = π/4`, çünkü
`DirectionalLightBundle::default()` güneşi (0, +0,707, +0,707) yönüne koyuyor) ve ışını kesen bir
levha konunca: **2376/16384 piksel (%14,5), max delta 22**. Geçiş sağlam.

Bekçi: `volumetric_god_rays_reach_the_frame`. Bu test aynı zamanda kuralın kendisinin kaydı — tek
sahnenin sıfırı, geçişin ölü olduğunu göstermez.

**Aynı bayrağın üçüncü yazımı: decal.** `decal.wgsl` bayrağı `world_pos_val.w == 0.0` ile, yani
tam kayan-nokta eşitliğiyle okuyordu. Bugün doğru çalışıyor — temizleme değeri tam 0,0 ve tam
temsil ediliyor — ama `(0, 0,5)` aralığındaki her şeyi de içeri alırdı; on okuyucu içindeki en
kırılgan biçim. `< 0.5`'e hizalandı. Bu yeniden yazım bugün tek pikseli değiştiremez (kodlayıcı o
aralıkta değer üretmiyor), tam da bu yüzden arkasına tartışma değil ölçüm konuldu.

Ve decal geçişinin **hiçbir testi yoktu**: beyaz zemine kırmızı projektör → **1133/16384 piksel
(%6,9), max delta 75**. Oyun yolunda çalışıyor; bekçi `decals_reach_the_frame`. Editörde hâlâ
görünmüyor ve bu ayrı bir kusur: decal G-buffer'ın albedo hedefine karışıyor, editörün hattı ise
forward — kayıt `gizmo-studio/tests/render_parity.rs`'te duruyor ve **açık**.

Ölçüm notu: her render için yeni bir `Renderer::new_headless` kurmak GPU belleğini bitiriyor —
4 zemin × 6 render = 24 cihazın 17.'sinde `radv/amdgpu: Not enough memory for command submission`
ve cihaz kaybı geldi (tek başına gölge dizisi 3072²×4). Süpürmeyi gerçekten okunacak satırlarla
sınırlı tut; bu makinede sınır ~16 headless renderer.

### Gölgeleme modları: forward hat deferred'ın numaralandırmasını TEKRARLIYOR (2026-08-16)

Toolbar'ın Lit/Normals/Albedo/Wire çipleri tek bir `shading_mode` uniform'u yazıyor. Modlar
başlangıçta yalnız `deferred_lighting.wgsl`'de vardı; stüdyo ise forward hattan (`shader.wgsl`)
çiziyor, yani üç çip ölçülebilir biçimde hiçbir şey yapmıyordu — Lit'e karşı 0/65536 bayt.

`shader.wgsl` artık 1 (Normals) ve 2 (Albedo) modlarını **aynı numaralarla ve aynı kodlamayla**
uyguluyor. Bu bir kopya, ve bilerek: tek uniform, hangi hat koşarsa koşsun tek anlam. Deferred
tarafına yeni bir mod eklenirse (bugün 3–6: Roughness/Metallic, Shadows, Tangents, ClearCoat) ve
stüdyoda görünmesi isteniyorsa, forward'a da eklenmesi gerekir — `every_shading_mode_draws_a_different_picture`
yalnız toolbar'ın gösterdiği dördünü tutar.

**Mod 3 iki hatta iki farklı şey.** Deferred'da Roughness/Metallic; stüdyoda toolbar'ın dediği şey,
yani **wireframe** — ve wireframe bir shading terimi değil, bir pipeline: `wireframe_pipeline`
aynı shader'dan `PolygonMode::Line` ile kurulu, ve depoda onu seçen hiçbir şey yoktu. Stüdyo o
modda uniform'u 0'da bırakıp pipeline'ı değiştiriyor, tam da bu çakışma yüzünden.

Not: hata ayıklama görünümleri HDR tamponuna yazılıp post-process'ten geçiyor, yani ekrandaki
değerler ham normal/albedo değil. Deferred hattın davranışı da aynı; ayrı bir kusur değil.

### Editör kamera tuşları sağ tuşa kapılandı (2026-08-16, davranış değişikliği)

Araç kısayolları (Q/W/E/R → Seç/Taşı/Döndür/Ölçek, `draw_editor`'da GENEL) ile serbest uçuş
tuşları (W/A/S/D + Q/E, `gizmo-studio` kamera sistemi) üç harfi paylaşıyordu ve uçuş hiçbir
değiştirici istemiyordu. Sonuç: W ile öne uçmak aracı Taşı yapıyordu, kaçışı yoktu.

Uçuş artık **viewport üzerinde sağ tuş basılı** olmasını istiyor — bakışı zaten kapılayan jest, ve
Game panelinin yardım metninin zaten tarif ettiği şey. Bu bir davranış değişikliği: eskiden sağ
tuşsuz uçulabiliyordu.

Bayrak `dragged_by` ile değil `is_pointer_button_down_on() && pointer.secondary_down()` ile
üretiliyor. Sürükleme egui'nin eşiğini geçmeden başlamaz; sağ tuşu basılı tutup fareyi
kıpırdatmadan WASD ile uçmak normal kullanım, drag'e bağlansaydı fare durunca kamera da dururdu.

Geri alınacaksa bilinsin diye yazıldı: kapıyı kaldırmak çakışmayı geri getirir, ve
`free_flight_is_gated_on_the_right_mouse_button` bunu kırmızıya çevirir.

### "Sonucu at, başarıyı yaz" taraması (2026-08-16)

Aynı kalıbın dört örneği bir günde çıkınca depo geneli tarandı. Kalıp şu: bir işlemin `Result`'ı
`let _ =` ile atılıyor, hemen ardından koşulsuz bir başarı satırı basılıyor. Sonuç: log, olmamış
bir şeyi olmuş gibi söylüyor.

Bulunan ve kapatılanlar:

| Yer | Ne diyordu | Gerçek |
|---|---|---|
| `gc.rs` auto-save | `💾 Auto-Save: <yol>` | kayıt başarısız olabilir, dosya yok |
| `build.rs` export | `Kopyalandı -> scripts/` (×4) | ikisinin kaynağı hiç yoktu |
| `render.rs` prefab kaydet | `Prefab kaydedildi.` | yazma başarısız olabilir |
| `render.rs` Ctrl+D | `Obje çoğaltıldı.` | okuma sonucuna bakılmadan |
| `prefs.rs` tercihler | (hiçbir şey demiyordu) | ayarlar sessizce kaybolur |
| `simulation.rs` script | (hiçbir şey demiyordu) | script hiç çalışmaz |

En ağırı auto-save'di: insanın işinin diskte olduğuna inanmak için baktığı satır tam da o.

Tarama sonrası `crates/` altında bu kalıptan **kalmadı** (üretici + iddia dörder satır içinde,
koşulsuz). İki bekçi duruyor: `no_save_call_discards_its_result` (`render.rs`, `gc.rs` kaynağını
okur) ve her düzeltmenin kendi davranış testi.

Sık düşülen tuzak, düzeltmenin ikinci yarısı: **koşulsuz bildirmek de yanlış.** Auto-save, script
reload ve tercih yazımı kare başına koşuyor; düz bir log satırı saniyede altmış kopya demek.
Üçünde de karar "giriş/çıkış anında bir kez" — hafızası olan saf bir fonksiyonda, döngüde değil.

### Panel genişliği taraması: sekiz panelden biri taşıyordu (2026-08-16)

Inspector'ın kendi içeriğini kırptığı bulununca (ayrıntısı commit'te) aynı kusur sınıfı için
**bütün editör panelleri ölçüldü** — göz kararı değil: `Context::run_ui` ile gerçek bir kare
sürülüp karenin boyadığı şekillerin en sağ kenarı okundu, clip rect açık bırakılarak. Kırpma
kusuru ekranda gizleyen şeyin ta kendisi; kırpmadan ölçmek onu sayıya çeviriyor.

Varsayılan yerleşimdeki genişliklerde (1600 px pencere, `create_default_dock_state` bölmeleri):

| Panel | Genişlik | İçerik | Sonuç |
|---|---|---|---|
| Inspector (environment) | 400 | **422.7** | taşıyordu — düzeltildi |
| Inspector (bileşenler) | 400 | 400.5 | sığıyor |
| Hierarchy | 320 | 320.5 | sığıyor |
| Assets · Console · Console (400 karakterlik satırla) | 880 | 880.5 | sığıyor |
| Profiler | 880 | 224.0 | sığıyor |
| Animation | 880 | 304.1 | sığıyor |
| Ayarlar · Script Editor | 400 | 400.5 | sığıyor |
| Toolbar | 1600 / 1280 / 1024 / 900 | +0.5 | sığıyor |

0.5 px kenarlık kalınlığı, taşma değil.

**Ölçümün kör olmadığı doğrulandı** — "temiz" demenin ön koşulu bu: toolbar saçma genişliklerde
tekrar ölçüldü ve 687 px'in altında taşıdığı görüldü (500 px'te 186.7 px taşma). Yani araç
taşmayı görüyor; diğer paneller gerçekten taşmıyor. Toolbar'ın gerçek alt sınırı **687 px**,
stüdyonun varsayılan penceresinin çok altında.

Bir daha taranmasına gerek yok; bekçi `inspector_width_tests` olarak duruyor ve iki yolu da
(seçim yok / seçili nesne) üç genişlikte tutuyor.

### Export bir oyunu değil, sabit bir demoyu paketliyordu (2026-08-16, DÜZELTİLDİ)

Stüdyodaki "Build / Export", `cargo build --release -p demo` koşup çıkan binary'yi
`export/gizmo_game/` altına kopyalıyor ve "Oyununuz hazır" diyor. `demo`'nun varsayılan binary'si
`bevy_3d_scene` — zemin, küp, ışık ve kameradan ibaret **sabit** bir sahne. Hiçbir sahne dosyası
okumuyor, hiçbir script çalıştırmıyor.

Yani export kullanıcının sahnesini değil, motorun örneğini paketliyor. Kopyalanan `scenes/` ve
`scripts/` dizinlerini açan bir şey yok: `Scene::load_into` var ama tek çağıranı editör; `demo`,
`cradle` ve `server` altındaki hiçbir binary sahne dosyası yüklemiyor.

Bugün kapatılan yarısı, kopyalamanın **dürüstlüğüydü** — dört dizin `let _ =` ile kopyalanıp
dördü için de koşulsuz "Kopyalandı" basılıyordu ve kaynakların ikisi (`demo/scenes`,
`demo/scripts`) hiç var olmayan yollardı. Artık ne olduğunu söylüyor ve çalışma anında kullanılan
yollardan (`scripts/`, `scenes/`) okuyor.

Kalan yarısı bir **özellik**: başlangıçta bir sahne dosyası yükleyen, script'leri bağlayan ve
fiziği/render'ı süren bir çalışma-zamanı binary'si. Onsuz export'un kopyaladığı veriyi okuyan
kimse yok. Kapsamı bir hata düzeltmesinin ötesinde olduğu için bilerek açık bırakıldı; kararı
verilmeden yapılmamalı, çünkü "oyun çalışma-zamanı" motorun ne kadarını varsayılan açacağı
sorusudur (fizik? scripting? ağ?).

#### Çalışma-zamanı yazıldı: `gizmo_runtime` (2026-08-16)

Kapsam sorusu **referansla** cevaplandı, zevkle değil: çalışma-zamanı, editörün Play modunun
sürdüğünü sürer — script motoru (`update` → `flush_commands` → entity başına `update_entity`),
1/60 sn sabit adımlı fizik akümülatörü (kare başına en çok 16 adım, borç da 16 adımla sınırlı) ve
`default_render_pass`. Ne fazlası ne eksiği. Bunun değeri şu: "export edilen oyun editörde
gördüğünle aynısını yapar" bir söz olmaktan çıkıp **karşılaştırmaya** dönüşüyor; ikisi ayrıştığı an
biri hatalıdır, tasarım tartışması değil.

Bilerek bırakılan iki fark, ikisi de eksik olan şeyin editör olması yüzünden: asset watcher yok
(hot-reload bir yazarlık aracı) ve script logları editör konsoluna değil stdout'a gidiyor.

Veri nereden geliyor: export edilen dizin binary'nin yanında `scenes/`, `scripts/`, `assets/`
taşıyan **kendi kendine yeten** bir dizin, dolayısıyla runtime o dizini çalışma dizini yapıyor ve
editörün yazdığı göreli yollar anlamını koruyor. Geliştirme ağacında (binary `target/release`'te)
bu kural kendiliğinden kapanıyor — kuralın kendisi testli.

Beklenen şey, **parçaların zaten var olmasıydı** ve öyle çıktı: `App::load_scene` açılışta
`SceneData::load_into`'yu `full_scene_registry` ile zaten çağırıyor. Yazılan şey alt sistem değil,
kablolama.

Doğrulandı: `demo/assets/sample.scene` (aşağıya bak) verilip `GIZMO_SCREENSHOT` ile kare alındı —
zemin, kırmızı küp, mavi küre, güneş ve gölgesi. Yani dosyadan yüklenen sahne gerçekten çiziliyor.

**Yolda çıkan iki kusur:**

1. Pencereli her uygulamanın ilk karede ölmesi (ayrı bölüm, düzeltildi). Runtime'ın ilk denemesi
   onu ortaya çıkardı; kusur runtime'da değildi.
2. **Deponun tek `.scene` dosyası yüklenmiyor.** `demo/assets/perfect_car.scene` eski biçimden:
   `components` alanını iç içe map olarak yazıyor, bugünkü biçim ise her bileşeni **string** olarak
   yazıyor (`"Transform": "(position:(...))"`). Ayrıştırma `10:28 Expected string` ile düşüyor —
   yani sürüm göçü (`migrate`) çalışamadan. Editörün varlık tarayıcısı da bu dosyayı açamaz.
   Dönüştürücü yazılmadı; yerine güncel biçimde `demo/assets/sample.scene` motorun kendi
   `SceneData::save` yolundan üretildi ve commit'lendi. Bekçi: `scene_round_trip.rs` — kaydedilen
   sahne mesh kaynakları, ışığı, birincil kamerası ve değerleriyle geri geliyor mu.
   **(2026-08-17'de kapatıldı — aşağıya bak.)**

#### Export artık onu paketliyor (2026-08-16)

`cargo build --release -p demo` → `cargo build --release -p demo --bin gizmo_runtime`, ve kopyalanan
dosya `demo` değil `gizmo_runtime`. Çapraz derleme hedefleri dahil: Windows export'unun `demo.exe`
göndermesi aynı kusurun uzantısı değişmiş hâliydi.

**Açık sahne de gidiyor.** Build isteği geldiğinde, iş parçacığı başlamadan önce ana iş
parçacığında canlı dünya geçici bir dosyaya kaydediliyor; cargo başarılı olursa
`export/gizmo_game/scenes/main.scene` olarak kopyalanıyor — runtime'ın argümansız açtığı ad. İki
karar yazılı: **canlı dünya**, çünkü diskteki son kayıt kullanıcının baktığı şey değildir ve eski
bir sahneyi sessizce paketlemek bu yolun düzeltildiği yalanın aynısıdır; **geçici dosya**, çünkü
export kullanıcının kaynak ağacına, hele `scenes/main.scene` üstüne yazmamalı.

Bitiş satırı da artık koşullu: sahne gitmediyse "Oyununuz hazır" değil, "build bitti ama sahne
gitmedi, boş pencere gelir" yazıyor.

Bekçiler (`export_copy_tests`): her hedefin runtime'ı kurup gönderdiği (`--bin` olmadan cargo
demo'nun varsayılanını kurar), export'un yazdığı sahne adının runtime'ın aradığı adla aynı olduğu
(test öteki ucun kaynağını okuyor, tekrar etmiyor), ve staging'in canlı dünyayı proje ağacının
dışına yazdığı.

Uçtan uca doğrulandı: `gizmo_runtime` + `scenes/main.scene`'den ibaret bir dizin kurulup binary
**başka bir çalışma dizininden, argümansız** çalıştırıldı — kendi dizinini bulup sahneyi açtı ve
çizdi.

#### Sözleşme iddia değil, tek kod oldu: `PlayLoop` (2026-08-16)

Son adım "iki yolun davranış eşitliğini test et" diye planlanmıştı. Doğrusu test değildi:
**eşitliği sınamak yerine ortadan kaldırmak.** Koşan bir oyunun karesi artık
`gizmo::systems::PlayLoop::step` — editörün ▶'si de, export edilen oyun da onu çağırıyor. Tek
akümülatör, tek adım boyu, tek script sırası.

İkisinin meşru olarak ayrıldığı tek şey **kimin duyduğu**: bozuk bir script editörde kırmızı bir
konsol satırı, gönderilmiş oyunda stderr satırı. O yüzden raporlama enjekte ediliyor
(`PlayReport`), kararların hiçbiri değil.

Bu, üç kopyayı bire indirdi — ve üçüncüsü testin kendisiydi: stüdyonun testleri koruduğu pump'ın
**elle yazılmış bir aynasını** çalıştırıyordu ("Mirror of the fixed-timestep pump"), yani
korudukları şey değiştiğinde kırılamayan bir test. Aritmetik artık gerçek kodun üstünde
sınanıyor; iki tarafta kalan testler ise düzeni koruyor: `the_play_frame_is_the_shared_step_not_a_copy_of_it`
(stüdyo) ve `the_frame_is_the_shared_play_step_not_a_copy_of_it` (runtime) — birinde
`physics_accumulator` ya da `update_entity` yeniden belirirse kırmızıya düşüyorlar.

Ölçü ve sınırlar yazılı hâlde taşındı: `MAX_STEPS`'in hem kare başına adımı hem **borcu** birden
sınırlaması (yalnız adım sayısını sınırlamak, kırk saniye boyunca hızlı ileri sarma demek) artık
`a_long_stall_does_not_buy_hundreds_of_steps_or_leave_them_owed` ile tutuluyor.

Editörde kalan tek fark bilerek: varsayılan `ActionMap` iskelesi. İki satır altında çağrısı yorum
satırına alınmış bir dövüş sistemi için yazılmıştı, yani koşan bir oyunun parçası değil; bir oyunun
gerçekten ihtiyaç duyduğu tuş eşlemesi sahnede durmalı, o zaman iki yol da alır.

Refactor'dan sonra runtime aynı sahneyi birebir aynı kareyle çiziyor.

#### Ve ortak adım ilk gerçek script'inde bir kusur verdi: bir kare gecikme (2026-08-16, DÜZELTİLDİ)

Sözleşmenin render yarısı doğrulanmıştı, **scripting yarısı hiç** — üstelik depoda tek bir `.lua`
dosyası yok, scripting testlerinin hepsi kaynağını geçici dosyaya kendi yazıyor. İlk gerçek script
koşturulduğunda çıktı:

`entity.set_position` dünyayı yazmaz, **komut kuyruğuna atar**; kuyruğu `flush_commands` uygular.
Adımın sırası ise `update → flush → entity başına on_entity_update` idi. Yani bir entity script'inin
istediği her şey, o karenin flush'ını **kaçırıp** bir sonraki kareyi bekliyordu. Ölçüldü: 1. karede
konumunu (5,0,0) yapan script'in entity'si 1. karenin sonunda hâlâ orijinde, 2. karenin sonunda
yerinde.

Yani motordaki **her** entity script'i bir kare gecikmeli çalışıyordu — editörde ve gönderilen her
oyunda — ve kimse fark etmedi çünkü hareket *oluyordu*, sadece geç.

Düzeltme: entity döngüsünden sonra ikinci bir flush. İlki duruyor (entity hook'ları paylaşılan
pass'in komutlarını görmüş bir dünya okusun diye); boş kuyruğu boşaltmak bir `Vec` takası, yani
komut üretmeyen kare hiçbir şey ödemiyor. Ve düzeltme tek yerde: `PlayLoop` paylaşıldığı için
editör de export edilen oyun da aynı anda düzeldi — refactor'ın ilk temettüsü.

Bekçiler `demo/tests/the_runtime_runs_scripts.rs`: script'in istediği karede indiği, eksik bir
script'in otuz karede bir kez bildirilip kareyi düşürmediği, ve **zincirin tamamı** — sahne dosyası
→ dünya → `Script` bileşeni dosyadan sağ çıkıyor → koşuyor → entity kıpırdıyor. Halkaların her
birinin testi vardı, zincirin yoktu; export'un okunmayan bir binary'yi bunca zaman
paketleyebilmesinin sebebi de tam olarak buydu.

### God fonksiyon taraması (2026-08-15)

Uzunluk tek başına sinyal değil; iç içelik derinliği ve dallanma sayısıyla birlikte ölçüldü. Bölünen
beş yer ve **bölünmeyenlerin gerekçesi** — ikincisi daha önemli, çünkü tekrar tartışılmasın:

- `handle_event` (gizmo-app) 708 → 450, derinlik 10 → 9. Asıl kazanç uzunluk değildi: dört surface
  hata kolunun üçü, `self`'ten çıkarılan altı şeyi geri koyan sekiz satırlık epilogun kendi
  kopyasını taşıyordu. Tek çağıran-tarafı epiloga indi.
- `constraint_solve_step` (gizmo-physics-rigid) 459 → 438, derinlik **10 → 8**. Crate'in en derin
  noktası solver aritmetiği değil, satır içi cevaplanan bir kırılma kontrolüymüş.
- `execute_render_pipeline` (studio) 595 → 541 + editörün kamera kuralları ilk kez testli.
- Kutu-seçim ve reparent: kod bölünmedi, **testleri yazıldı** / çekirdek yardımcıya indirildi.

**Bölünmeyenler ve neden:**

| Fonksiyon | Gerekçe |
|---|---|
| `update_vehicle_with_query` (544) | Ayrılabilir kararlar **zaten ayrılmış**: `ackermann_steering_angle`, `anti_roll_force`, `ground_effect_factor`, `weather_grip_factor`, `apply_force_at_point` — hepsi ayrı fonksiyon ve 19 testli. Kalan kütle 256 satırlık tekerlek döngüsü; dışarıdan ~12 değer okuyor, çıkarmak 12 alanlı bir bağlam struct'ı demek. Fonksiyon kısalır, karmaşıklık kalır. |
| `create_fluid_pipelines` (529) | **0 dal** — bildirimsel wgpu descriptor'u. Çözülecek akış yok. |
| `default_render_pass` (420) | **Derinlik 4** — uzun ama düz; zaten faz dizisi olarak okunuyor. |
| `ui_scene_view` (433) | egui tesisatı; kararları başka yerde çözülüyor (kutu-seçim isteği burada kaydedilip `studio_input`'ta işleniyor — okunarak doğrulandı). |
| `solve_contacts` / `_tgs` / `narrowphase_*` | Sıcak yol. Belgeler belirli *optimizasyonları* ölçüp reddetmiş; yapı hakkında bir şey söylemiyor. Bölünecekse şartı `headless_stress_test` hash'inin değişmemesi — `collect_fracture_events` bunun yapılabildiğini gösterdi. |

**Ölçüt olarak ayakta kalan tek şey:** uzunluğun sebebi **düğüm mü** (iç içe durum + dallanma → böl)
yoksa **dizi/bildirim mi** (→ bırak). Yol boyunca kullanılıp çürütülen üç sahte ölçüt: "iyi test
edilmiş" (kapsam bölmeyi *güvenli* kılar, gereksiz değil), "GPU'ya bağlı" (test edilebilirlik
hakkında, bölünebilirlik hakkında değil), "bilinçli yoğun" (perf kararı, yapı kararı değil).

### Bir daha kovalanmasın

- Animasyonun zamanlanmamasının sebebi **imza değildi** — studio onu tam o imzayla zaten çağırıyordu.
- Süpürme sayısını kesmek yakınsamayı iyileştirmez: 9 kat az süpürme %17 kazandırıyor ve tavanı o.
- Varyans kırpması TAA kalıntısını düzeltmiyor; iki kez ölçüldü, ikisinde de biraz kötüleşti.
- **`gizmo-animation` Stage A'ya uygun değil** diye kaydedilmişti; bağımlılıkları ölçüldü, uygun.
  Bayat olan diyagramdı.
- **Sonda köprüsü `send` yüzünden imkânsız** diye kaydedilmişti; `Scope::create_function`'ın
  imzasına bakılmamıştı, o sınırı taşımıyor.
- **ECS-only rollback dört düşen kutuda ıraksıyor** diye ölçmüştüm; ıraksamayı üreten şey testteki
  fazladan bir adımdı. O senaryo iki uygulamayı ayırt edemiyor; ayırt eden şey pencere içinde
  kopan bir eklem.


### Editör viewport'u: "çirkin ve kalitesiz" neydi (2026-08-15)

Kullanıcı editörü açıp *"çok çirkin ve kalitesiz görünüyo"* dedi ve dört belirtiyi birden
işaretledi: bulanık, tırtıklı, ışık düz, renkler yıkanmış. Dördü birden tek bir üst-akım sebebe
işaret ediyordu ve öyleydi.

**Kök: editörün 3B görüntüsü ekrana bir sRGB kodlaması eksik düşüyordu.** egui'nin shader'ı
sözleşmesini kendi yorumunda yazıyor — *"We expect 'normal' textures that are NOT sRGB-aware"* —
ama her iki viewport RTT'si de `config.format` ile, yani sRGB olarak yaratılıyordu. İki çözme,
bir kodlama. Ayrıntı ve düzeltme `editor_runtime::create_viewport_target`'ta.

Bu kökü bulmanın yolu, tahmin listesini eleyip **ölçmek** oldu; sırayla düşen hipotezler:

| Hipotez | Nasıl düştü |
|---|---|
| Bugünkü DoF kalibrasyonu | Hesap: hiperbolik derinlikte 20 m'de fark %1. Görünür değil. |
| PBR paketleme değişikliği | Forward shader `inst_pbr.w`'yi hiç okumuyor. |
| `..Default::default()` bloom'u düşürdü | Devredilen alanların hepsi aynı değere sahip. |
| Ortak ışık kurulumuna geçiş | Uniform'a giden değerler bastırıldı: güneş yön/renk/yoğunluk doğru. |
| Gölge haritası her yeri gölgede sanıyor | Gölge araması devre dışı bırakıldı, piksel bire bir aynı. |
| Mesh hiç çizilmiyor | Batch'ler bastırıldı: küp 24 vert/36 index ile lit batch'te. |
| FXAA | Kapatıldı, piksel değişmedi. |

Ayırt eden ölçüm, post zincirine **bilinen bir sabit** basmak oldu: composite'e lineer 0.5
yazdırıldı, ekranda 188 yerine 128 çıktı. Bir gamma adımı, tam olarak. Kalibrasyon için egui
panel zemini kullanıldı (28 ölçüldü, egui koyu temasının 27'si) — yüzey kodlamasının doğru
olduğunu, kaybın yalnızca viewport yolunda olduğunu bu gösterdi.

**Bunu mümkün kılan şey yeni `gizmo_renderer::capture`.** Bu makinede dışarıdan ekran yakalama
çalışmıyor (Xwayland rootless → X kökünde içerik yok, `import` da ffmpeg de siyah döndürür), ve
bu değişmeyecek. Kare artık GPU'dan geri okunuyor. Motorun kendi çıktısına bakabilmesi teşhis
süresini "kullanıcıya sor" döngüsünden ~40 saniyelik bir deneye indirdi.

Sahne içeriği hakkında, kusur olmayan iki gözlem: studio'nun varsayılan sahnesinde gökyüzü/zemin
yok (`setup.rs`'teki "Custom Skybox or proper horizon color" yorumu kalıntı) ve Default Cube'ün
materyali kasten %21 gri. Karanlık görünmesinin bir kısmı buradan; ikisi de bilinçli seçim
olabilir, o yüzden dokunulmadı.

#### Denendi ve reddedildi: editör viewport'una prosedürel gökyüzü

Gamma düzeltmesinden sonra geriye kalan "sahne boş bir karanlık" hissini gidermek için skybox
denendi. `sky.wgsl` zaten tam bir atmosfer çiziyor (zenit/ufuk/zemin gradyanı, güneş halesi ve
diski) ve `sky_pipeline` kurulu — studio'nun hiç skybox varlığı yaratmaması yüzünden editörde hiç
çalışmamış. Bir varlık eklemek yetiyor; `gizmo_root` altında, grid ile aynı krom statüsünde.

**Sonuç ölçüldü ve daha kötü:** gökyüzü açıldığında grid tamamen kayboluyor. Grid materyali açık
renkli ve alfa-harmanlı; koyu arka plan için tasarlanmış. Ufuk rengi (0.5, 0.7, 0.9) sahnenin
güneş yoğunluğuyla çarpıldığı için (studio'da 1.5) beyaza kırpılıyor; yoğunluk 1.0'a çekilince de
viewport 182–211 aralığında düz bir soluk yıkama oluyor ve grid yine görünmüyor. Kamera pitch'i
-23°, FOV 60° — görünen alanın neredeyse tamamı en parlak ufuk bandı.

Gökyüzünü editörde kullanılabilir kılmak grid ve gizmo renklerinin de yeniden tasarlanmasını
gerektirir. Blender ve Unity'nin editör arka planını nötr-koyu tutmasının sebebi bu. Kayıt için:
skybox varlığını eklemek 20 satır, sorun orada değil.

### Editör hattının ilk piksel testleri, ve bulduğu kusur (2026-08-15)

Motorun deferred yolunda on yedi golden piksel testi var; editörün 600 satırlık forward yolunda
hiç yoktu. `render_parity.rs` ortak *kurulumu* örtüyor (kurulum saf fonksiyon), ama pass kaydı
gözlemsizdi. Bedeli bu oturumda görüldü: bir aylık açılış çökmesi ve aylarca süren gamma hatası,
ikisi de ekran görüntüsünde apaçık, ikisi de piksele bakmayan bir test paketine görünmez.

`tests/studio_render_pixels.rs` bu boşluğu kapatıyor: headless renderer, gerçek
`execute_render_pipeline`, geri okuma. Kaba iddialar bilinçli — golden görüntü editörün *görünüşünü*
sabitler, oysa görünüş değişmeli; testler yalnızca kaybı "bozuk" demek olan özellikleri tutuyor
(hiçbir şey çizilmemiş, her şey tek renk, aydınlatılan nesne arkasındaki boşluktan ayırt edilemez).

**Kurulur kurulmaz bir kusur buldu.** Motor yolu `ensure_global_transforms` ile `Transform`'u olup
`GlobalTransform`'u olmayan mesh'lere bileşeni ekliyor — kendi yorumunda "empty screen footgun".
Editör yolunda bu adım yoktu; studio'nun koşturduğu sync/propagate sistemleri mevcut bileşeni
günceller, olmayanı eklemez. Yani `spawn((Transform, Mesh, Material))` oyunda çiziliyor, editörde
sessizce çizilmiyordu (ölçüm: küp merkezi 44.0, arka plan 34.0 — saf arka plan). Farkın görünür
bedeli `setup.rs`'te duruyordu: dokuz varlığa elle eklenmiş `GlobalTransform::default()`.

İki kayıt notu daha çürüdü:

- *"Bu testler için önce `StudioState` headless kurulabilir olmalı."* Değilmiş: struct skalarlardan
  ve bir `Option`'dan ibaret, hat da üç opsiyonel kaynağa bakıyor. İlk denemede koştu. Bu, ölçülmeden
  kaydedilmiş engellerin bu kod tabanındaki **üçüncüsü** ve üçü de temasta dağıldı (öncekiler:
  `Scope::create_function`'ın `Send` sınırı, animasyonun imzası).
- Testin ilk hâli kırmızıydı ve suç renderer'da değildi: küpe tam cepheden bakan kamera tek yüz
  gösteriyor, tek normal tek gölge veriyor — 153.6..153.6, doğru çıktı, ama "aydınlatma ulaşmıyor"
  ile birebir aynı görünüyor. Küp döndürüldü; gerekçe testin içinde, yoksa biri "gereksiz" diye
  siler.

#### Editör hattında taranan öteki özellikler (2026-08-15)

Piksel koşumu kurulunca aynı mercek motorun golden testlerinin kapsadığı öteki özelliklere
tutuldu. Sonuçlar, negatifler dahil:

| Özellik | Sonuç |
|---|---|
| Instance kapasitesi | **Sorun yok.** Studio `ensure_instance_capacity` çağırıyor; pass'lerdeki beş `instance_capacity` kırpması bu yüzden ölü savunma, mesh düşmüyor. |
| Gölge dökme | **Çalışıyor**, artık testli. Bkz. `the_editor_casts_a_shadow_onto_the_ground`. |
| `GlobalTransform` doldurma | **Kusurluydu**, düzeltildi (yukarıdaki bölüm). |
| SSAO onay kutusu + şiddet kaydırıcısı | **Ölüydü**, kontrol kapatıldı. |
| Inspector'daki post kaydırıcıları (bloom, grain, exposure, vignette, aberration, DoF) | Hepsi `post_params`'a akıyor, canlı. |

Gölge yolu az kalsın yanlışlıkla "ölü" diye kaydediliyordu: forward shader'da `shadow_visibility`
1.0'a zorlandığında örneklenen piksel sıfır değişmişti. Sebep gölgenin çalışmaması değil, örnek
noktanın aydınlık bir yüzde olmasıydı. Tek pikselden çıkarılan olumsuz sonuç, sahne o soruyu
soracak biçimde kurulmadıkça hiçbir şey kanıtlamıyor.

#### Game paneli editör kamerasını gösteriyordu (2026-08-15, KAPANDI)

Studio karede **tek** sahne çizimi yapıp iki çıktı üretiyor: `run_post_processing` önce editör
hedefine, sonra oyun hedefine yazıyor, ikisi de aynı `renderer.post.hdr_texture_view`'i okuyarak.
O doku edit modunda editör kamerasından çizilmiş oluyor — gizmo'lar, grid ve "oyun kamerası burayı
görüyor" tel kutusu dahil. Yani Game sekmesi Scene sekmesinin kopyası, hem de tam işe yarayacağı
anda. Play modunda iki kamera aynı olduğu için soru doğmuyor.

Ölçüldü: iki kamera zıt yönlere bakarken game hedefi sahne hedefiyle bayt bayt aynı (65536 baytın
0'ı farklı). Kayıt prosa değil, `#[ignore]`'lu bir teste kondu —
`the_game_view_shows_the_game_camera_not_the_editor_camera` — çünkü çalıştırılabilir bir kayıt
bayatlayamaz ve düzeltildiği gün kendiliğinden yeşile döner.

**Düzeltildi.** Üç parça: `EditorOnly` işaretleyici bileşeni (gizmo-core'a değil
`gizmo-renderer`'a — ECS tabanı Stage A'nın en alt yüzeyi ve bu bir render kaygısı, §4;
`EditorRenderTarget` zaten orada), ana geçişe `draw_chrome` bayrağı, ve oyun kamerasından ikinci
bir çizim. İşaretleyici batch anahtarına da girdi: bir ikonla aynı batch'e düşen sahne mesh'i aksi
hâlde oyun görüntüsünden onunla birlikte silinirdi.

**Ve buradaki tuzak kaydedilmeye değer.** İlk deneme "kare ortasında uniform'u yeniden yaz, ikinci
geçişi aynı encoder'a kaydet" idi ve çalışmadı — üstelik *hiçbir şey olmamış gibi* görünen bir
başarısızlıkla: `Queue::write_buffer` komutlarla değil **submit'lerle** sıralı. Bir submit'ten
önceki her yazım o submit'teki bütün geçişler için geçerli, dolayısıyla ikinci yazım ilk çizimi de
değiştirdi ve iki panel yine aynı görüntüyü verdi. Doğrusu bir submit sınırı: oyun görünümü kendi
encoder'ında çizilip, editörün uniform'ları geri yazılmadan önce submit ediliyor. Bu sayede ikinci
bir uniform tamponu ya da bind group çoğaltması gerekmedi.

Cascade'ler oyun kamerasına yeniden oturtuluyor — shader cascade'i görüş derinliğinden seçtiği için
split'ler bakan kameraya ait olmak zorunda. Bedel ölçüldü: 481 → 453 FPS (~%6); culling ve instance
tamponu paylaşıldığı için iki kez ödenen şey yalnızca pass kaydı ve rasterizasyon. Görünürlük
kapısı bilinçli olarak konmadı: editör durumunda panelin görünür olup olmadığını bildiren bir şey
yok ve canlı beklenen bir önizlemede %6 için görünürlük protokolü icat etmek yanlış takas.

Bu iş sırasında yazdığım bir yan kazanç iddiası **yanlıştı** ve düzeltildi: "ışık ikonları artık
play modunda da sızmıyor" demiştim; `systems/gizmos.rs` play modunda o nesnelere zaten `IsHidden`
ekliyormuş. İşaretleyicinin gerçek kazancı edit modundaki Game görünümü.


### "Editör nesnesi mi" kararı: sekiz kopyadan bire (2026-08-15)

Game view işi bir işaretleyici bileşen gerektirdi, ve onu ararken kuralın zaten var olduğu ortaya
çıktı — isim öneki olarak, `starts_with("Editor ") || == "Highlight Box"`, **sekiz** yerde ayrı
ayrı yazılmış: hiyerarşi paneli (iki kez), `gizmo-app`'in editör runtime'ı, `gizmo-scene`'in
snapshot filtreleri (iki) ve sahne yazıcısı, studio'nun korunan-nesne kümesi, silme koruması,
tümünü-seç kısayolu, play-modu gizlemesi. Sekizinin de o boşluk karakteri üzerinde anlaşması
gerekiyordu.

Karar artık `gizmo_core::component::is_editor_only`; yanında `EditorOnly` bileşeni. İsim kuralı
korunuyor çünkü bileşenden önce yazılmış sahneler yalnızca isim taşıyor — geçiş, tasarım değil.

**Bileşenin yeri ölçüldü, tercih edilmedi.** Önce `gizmo-renderer`'a konmuştu ("editör kavramı ECS
tabanına ait değil"). Tüketicileri sayınca yanlış olduğu görüldü: `gizmo-scene` de bu kararı
veriyor ve renderer'ı göremiyor — grafikte yanında duruyor, üstünde değil. Yani kavram bir render
kaygısı değil, bir *dünya* kavramı: "bu nesne alet, içerik değil". Çekirdek zaten `IsHidden` ve
`IsDeleted`'ı barındırıyor.

İsim kuralının bilinen yarası testte belge olarak duruyor: sahnesinde "Editor Desk" adlı masası
olan kullanıcı onu hiyerarşide göremez ve hiçbir kayıtta bulamaz. İşaretleyici tam da bunun için
var.

Tarayıcı test yalnızca **üretim kodunu** okuyor: bir testin "bu filtrelendi" derken dizeyi anması
sonucu denetlemektir, kararı yeniden vermek değil — `gizmo-scene`'in kayıt testi tam olarak bunu
yapıyor ve haklı.
### Editör kontrolleri taraması: beş şüpheli, ikisi yanlış teşhis (2026-08-15)

SSAO bulgusundan sonra arayüzün yazdığı **30 alanın** hepsi tarandı: her birinin gerçek bir
tüketicisi var mı? Beşi şüpheli çıktı. Paralel bir inceleme koşturuldu ve dört soruşturmacıdan
biri **iddiayı çürütmekle** görevlendirildi. İyi ki öyle yapıldı:

| Kontrol | İlk teşhis | Gerçek |
|---|---|---|
| `snap_translate` | ölü | **YANLIŞ — canlı.** `scene_view.rs:204` varlık sürükle-bırakta okuyup yuvarlıyor. |
| `snap_rotate_deg` | ölü | **YANLIŞ — atıl.** Config'e geçiyor, `config.snapping` kapalı olduğu için kütüphane okumuyor. |
| `show_grid` | ölü | Doğru. |
| `snap_scale` | ölü | Doğru (config'e hiç geçmiyordu). |
| `gizmo_size` | ölü | Doğru. |

**Denetimin kusuru neydi:** okuyucuları sayarken `gizmo-editor`'ın kendi dosyaları elenmişti
("arayüzün kendisi yazıyor" varsayımıyla), oysa `scene_view.rs` o crate'in içinde ve gerçek bir
tüketici. Filtre, aradığı şeyi tanımıyla dışarıda bırakıyordu. Bu, tarama testleri yazarken tekrar
edilebilecek bir hata: kapsam dışı bırakılan yer, aranan şeyin yaşadığı yer olabilir.

`snapping` alanı ise tek satırlık ama üç ayarı birden ölü gösteren cinsten: `transform-gizmo`
`snap_distance`/`snap_angle`/`snap_scale`'i **yalnız** `if config.snapping` içinde okuyor, o alan
hiç atanmamıştı, ve `..Default::default()` `false` veriyordu. Ctrl modifier'ı bile yıllardır
kuruluymuş ve kimse fark etmemiş — çünkü hepsi hesaplanıp atılıyordu.

`gizmo_size` uçtan uca **görsel olarak** doğrulandı: küp geçici olarak seçili hâle getirilip
75 ve 220 değerleriyle iki kare alındı; tutamaklar belirgin biçimde büyüdü. Piksel testi yazılmadı,
çünkü transform gizmo'su yalnız bir seçim varken çiziliyor ve seçim egui girdisiyle kuruluyor —
headless koşumda simüle edilemez. Geçici düzenlemeler geri alındı.

Grid anahtarının testi iki şey birden tutuyor: fark sayısı **ve yönü**. Yalnız "kareler farklı"
denseydi bayrak ters dönse de test geçerdi. Snapping'de piksel testi mümkün değil (egui sürüklemesi
gerekir), o yüzden karar `snap_active` saf fonksiyonuna çıkarıldı; XOR'un asıl sınanmaya değer
satırı ikincisi: tercih açıkken Ctrl snapping'i **askıya alır**. Oraya `||` yazmak tuşu zamanın
yarısında işlevsiz bırakır ve hiçbir şey fark etmez.
