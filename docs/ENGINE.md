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
- CI: `cargo clippy --all-features --all-targets -- -D warnings -A too_many_arguments
  -A type_complexity` (the two grandfathered architectural lints). The entry crate is
  `gizmo-engine` (NOT `-p gizmo`); `| tail` masks cargo's exit code — check the exit status
  separately.

## Nerede kaldık (2026-08-14)

Bir günde bulunup düzeltilen kusurlar aşağıda kendi bölümlerinde duruyor. Bu bölüm yalnız **açık
olanı** ve **bir daha kovalanmaması gerekeni** taşır.

### Devam eden: iki render yolu

Kök kayıtlı ("The root the sweep could not see"). İlk kesim yapıldı — `gizmo-renderer::routing`,
malzeme tipinin anlamı tek ve tüketici bir `match`'te. Kalan iki adım, incelemenin fiyatlandırdığı
hâliyle:

- **Tek uniform kurucu (küçük, ~1 gün).** `SceneUniforms` ve `PostProcessUniforms` iki yerde elle
  doldurulan struct literalleri. Studio `exposure: 1.0`, `point_shadows_enabled: 0` gibi değerleri
  sabit yazıyor, motor ise aktif kameradan. Ortak kurucu + editörün açık geçersiz kılmaları, "her
  alan dolduruldu mu" denetimini "hangi alan neden farklı" farkına çevirir.
- **Studio'ya lib hedefi + parite testi (orta, ~2-3 gün, satır başına en yüksek değer).**
  `gizmo-studio` bugün `publish = false` ve **lib hedefi yok**, yani render yolu hiçbir test
  koşumundan erişilemiyor; iki yol arasındaki tek çapraz denetim insan gözü. Doğrulayıcı bu adımın
  fiyatını "türde doğru, ölçekte iyimser" buldu: mekanik kısım kolay, `StudioState`/`EditorState`'i
  headless kurmak değil.

Daha ucuz alternatif de kayıtta: iki çizim döngüsünü **metin olarak** okuyup bir isim envanterinin
ikisinde de geçtiğini (ya da gerekçeli bir istisna listesinde olduğunu) iddia eden ~60 satırlık bir
test. Mimariye dokunmadan bu haftaki her örneği yazıldığı gün yakalardı.

### Doğrulanmış ama henüz el atılmamış kökler

- **Sözleşmeler (çare sağlam, küçük).** `common.wgsl` kendini sahne uniform düzeninin tek doğruluk
  kaynağı ilan ediyor; **27 tüketicinin 20'si** için doğru. Ayrımı yapan hiçbir şey yok, çünkü
  paylaşım `load_shader_composed` çağrılıp çağrılmamasına bağlı. Ayna testleri doğru desen ama her
  biri **öznelerini elle sayıyor**, üçünden ikisi bayat — yani yeni bir shader, onu denetlemek için
  var olan teste görünmez.
- **Shader boru hattı.** `compose_wgsl` bir `naga::Module` kurup doğruluyor ve **atıyor**, `String`
  döndürüyor. Rust, kendi struct'larını shader'ın gördüğü düzenle karşılaştıramıyor.
- **Determinizm çevresi (çare sağlam).** `snapshot.rs` iki elle yazılmış 9 alanlık liste taşıyor,
  oysa `PhysicsWorld`'ün 28 alanı var; yeni bir alan sessizce anlık görüntünün dışında kalır.
- **Crate grafiği / Stage A.** `gizmo-animation` Stage A listesinde ama pratikte öyle değil.

### Scripting

32 doğrulanmış kusurdan üçü düzeltildi: script sırası (`HashMap` → `BTreeMap`, proses başına
rastgeleydi), on altı komutun sessizce yutulması, ve bir script'in hatasının ötekileri iptal etmesi.
Açık kalanlar: yürütme/bellek limiti yok (sonsuz Lua döngüsü prosesi asar), NaN/sonsuz doğrulaması
yok, tek yönlü sandbox (`_G`), tuş haritası her girdisinde yanlış.

**Teşhis sondası köprüsünün önündeki asıl engel:** `send` özelliğiyle `create_function`'ın kapanışı
`Send + 'static` olmak zorunda, yani bir Lua callback'i **`&World` yakalayamaz**. Bütün okuma yolu bu
yüzden kare başına anlık görüntü. Parametreli sorgu (şu (x,z)'de zemin kotu) anlık görüntüyle ifade
edilemez; çağrı-anı erişim mekanizması gerekiyor (`Lua::scope` ya da ömrü belgelenmiş userdata). Bu,
ince bir köprü değil gerçek bir tasarım kararı.

### Bir daha kovalanmasın

- Animasyonun zamanlanmamasının sebebi **imza değildi** — studio onu tam o imzayla zaten çağırıyordu.
- Süpürme sayısını kesmek yakınsamayı iyileştirmez: 9 kat az süpürme %17 kazandırıyor ve tavanı o.
- Varyans kırpması TAA kalıntısını düzeltmiyor; iki kez ölçüldü, ikisinde de biraz kötüleşti.

