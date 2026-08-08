# Changelog

All notable changes to the Gizmo engine are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims
to follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Versioning note.** `0.2.0` ships the whole workspace at one uniform `0.x`
> version on purpose: it bundles the large 1.0-readiness effort and the breaking
> graphics-stack upgrade, but **defers the hard `1.0` promise** to gain soak time
> on the new `wgpu`/`winit`/`egui` stack. The *staged* `1.0` model — promoting the
> dependency-light **Stage A** core (`gizmo-math`, `gizmo-core`, the
> `gizmo-physics-*` crates, `gizmo-scene`, `gizmo-net`, `gizmo-audio`, `gizmo-ai`)
> to `1.x` while the graphics/integration **Stage B** crates stay on `0.y` — is
> documented in [`docs/ENGINE.md`](docs/ENGINE.md) and remains the planned path for a
> later release.

## [Unreleased]

### Added

- **A per-frame `Update` schedule.** `App` now carries two: `schedule` still runs `0..N`
  times per rendered frame at a constant `dt` (physics, unchanged — nothing moved out of
  it), and the new `update_schedule` runs **exactly once** per frame with the real frame
  delta. Register on it with `App::add_update_system`.

  This closes a defect that was hard to see and easy to blame on hardware. The single
  schedule ran only inside the fixed-timestep loop, so with the renderer's default
  `PresentMode::AutoNoVsync` pushing hundreds of frames per second against a 60 Hz
  accumulator, most rendered frames ran no systems at all. `Input` is captured once per
  rendered frame and its edges cleared once per rendered frame, but were consumed 0..N
  times — so keypresses and mouse motion on those frames were written and discarded with
  nothing observing them. Taps went missing; mouse-look arrived in fragments.

  `FpsLookPlugin` moves to the per-frame schedule for exactly that reason. Its placement
  is locked by a test: running the fixed schedule must not move the camera, running the
  update schedule must.

  The sequencing lives in `gizmo_app::frame::run_fixed_and_update` with seven tests that
  need neither a window nor a GPU — including the two that state the contract directly:
  nine short frames run the fixed schedule zero times and update nine times, and one long
  frame runs fixed four times and update once.

  `add_system` still targets the fixed schedule. Changing its default would have silently
  moved existing users' systems onto a variable `dt`.

- **Scene queries.** `QueryFilter` (layer mask, multi-body exclusion, trigger opt-in) plus
  `PhysicsWorld::raycast_filtered`, `overlap_shape`, `point_query`, `cast_shape` and
  `cast_body`, all broadphase-accelerated.

  The previous public surface was three unfilterable raycasts. A ray has no volume, so a
  character controller could not sweep its own capsule; without a layer mask every gameplay
  ray hit triggers, debris and the caster itself. The engine's own character and vehicle
  code shows what that cost: both bypass the broadphase and scan every collider in the
  world, every frame, because there was nothing to call.

  `cast_shape` marches over `NarrowPhase::test_collision` and bisects, rather than using
  `Gjk::conservative_advancement`. That routine is present in the crate but only correct
  for a head-on approach — with any lateral offset its step carries the shape into overlap,
  and `Gjk::distance` reports a positive distance for overlapping shapes instead of
  signalling penetration, so it concludes nothing was hit. Fixing it is tracked separately;
  the query layer does not wait on it.

### Changed

- **Every bounded collider shape now shatters, and the ones that cannot no longer disable the
  entity.** *Behavioural for any non-box `Breakable`.*

  `shatter_entity` only ever read box half-extents and returned early on everything else — but
  all three call sites had already set `Breakable::is_broken`. A sphere, capsule or hull
  breakable that ran out of health therefore spawned no debris, was never despawned, and,
  because every damage path is gated on `!is_broken` and nothing in the engine clears that
  flag, could never be damaged or broken again. It stayed in the scene at zero health,
  permanently inert. The bail-out did not mean "unsupported shape does nothing"; it meant
  "unsupported shape destroys the entity in place".

  Sphere, capsule, convex hull and compound now shatter through the collider's **local
  bounding box**, so a sphere breaks like the cube around it. That is the approximation the
  debris already carried — each Voronoi cell is replaced by a volume-matched sphere whatever
  its real geometry — so the bound is not the weak link. `Plane` (an infinite half-space,
  whose AABB is a ±10 km cube that would have shattered the floor into kilometre-wide
  boulders) and `TriMesh` (static and concave, which convex debris cannot represent) still do
  not shatter; `shatter_entity` reports that to its callers, which now latch `is_broken` only
  on a real break, leaving such a body damageable instead of frozen.

  **The box path is unchanged, and that was measured rather than argued:** the debris field of
  a 0.5 m box is pinned bit-for-bit in `tests/breakable_shatter.rs`, and those numbers were
  read off the pre-fix build. Running the new test file against pre-fix `system.rs` turns five
  of its six tests red and leaves exactly that one green.

- **`gizmo-ui`'s `Style` no longer holds a `taffy::Style`.** *Breaking for `gizmo-ui`, which is
  marked experimental.*

  The component is now a plain `Copy` POD built on a crate-owned `Val { Auto, Px, Percent }`,
  and taffy is reached only inside `UiContext` at layout time. That deletes both
  `unsafe impl Send/Sync` — the component derives them now — whose soundness had rested on
  taffy's `calc` feature staying off, and closes the `pub use taffy::style::*` /
  `pub use taffy::geometry::*` leak, so no taffy type appears in a public signature, field or
  re-export any more. Every taffy field the new type does not model is named in its docs, the
  CSS Grid family included, so the omissions are stated rather than silent.

- **Debris patterns differ per object.** *Behavioural (visual) for anything with `Breakable`.*

  `shatter_entity` seeded the Voronoi cut with a literal `42`, so every object in a scene broke
  into the identical pattern. The seed is now derived from the entity's ECS id through a
  SplitMix64 finalizer.

  Deliberately **not** mixed with a frame counter, though the plan called for one: the engine
  has no rollback-safe counter to use — `PhysicsWorld` carries no tick, neither snapshot
  restores one, and `Time::frame_count` counts wall-clock-driven render frames. Any of them
  would make a rolled-back-and-resimulated break produce different debris than the original.
  Nothing is lost by leaving it out, since `is_broken` latches on the first shatter and the seed
  therefore never has to tell two occasions apart, only two entities.

- **The Stage A crates' public documentation is in English.** *Documentation only — no
  behavioural change.*

  `gizmo-core`, `gizmo` (the facade), `gizmo-physics-core`, `gizmo-math`, `gizmo-ai`,
  `gizmo-audio`, `gizmo-app` and `gizmo-scene` had 1286 Turkish `///` / `//!` lines between
  them; 8 remain, of which 7 are false positives of the detector (Plücker, Möller–Trumbore, and
  a line of English *about* Turkish casing) and the eighth is a doc quoting a Turkish log format
  string that a test asserts on. Translated sentence for sentence, with hedges left as hedges
  and emphasis left where it was — the same standard `docs/ENGINE.md` was held to. No
  intra-doc link broke: the rustdoc warning count is unchanged at 71.

  Comments inside doc examples were translated too, including the assertions that quote the
  messages they check. Plain `//` inline comments are still Turkish across the workspace — a
  larger surface, tracked separately.

- **Every doc example now compiles and runs.** *Documentation only — no behavioural change.*

  The workspace had 30 `​```ignore` fences, i.e. 31 doc-tests that rustdoc collected and then
  skipped. There are now **zero**: `cargo test --workspace --doc` goes from 17 passing / 31
  ignored to **45 passing / 0 ignored**. None of them needed `no_run` and none turned out to be
  irreducible pseudo-code, so all 30 became real tests — and they assert the documented
  contract rather than merely linking.

  Un-ignoring them surfaced defects the fences had been hiding, all now fixed:
  `World::spawn_bundle`'s example used `MeshBundle` / `Material::pbr` / `Color::BLUE`, none of
  which `gizmo-core` can even name; a doc comment in `component_ops.rs` was attached to the
  wrong function (it documented `query_entity_mut` while sitting on `insert_batch`, whose own
  summary was stranded at the bottom of the block); `web_profile.rs`'s module blurb was written
  with `///` and so documented the next item instead, and its example passed a `bool` where
  `with_shadows` takes a `ShadowQuality`; `gizmo-renderer`'s crate docs described the frustum
  matrix as `view * projection` when `camera.rs` builds `projection * view`; and
  `resource_scope`'s example could not compile at all, since its turbofish supplied two of
  three generic arguments and the third is an unnameable closure type.

- **The contact solver's soft-constraint penalty term no longer carries a mass factor.**
  *Behavioural only for `block_solver = false`; the default path never reaches this line.*

  `impulse_scale·λ` relaxes the accumulated impulse — it is not a velocity error to be
  converted into one — so it belongs outside the effective-mass division, as in Box2D v3's
  `-normalMass·massScale·(vn + bias) − impulseScale·λ`. It had been moved inside, making the
  update `λ_{n+1} = λ_n·(1 − impulse_scale/k_n) + …`, whose factor leaves the unit disc once
  `m_eff > 2/impulse_scale` (≈34.6 at the shipped `hertz = 30`, `ζ = 10`).

  Contacts never blew up the way the joint rows did — `max(0.0)` truncates the negative half of
  every cycle — so the symptom was quieter: penetration recovery stopped being mass-invariant,
  which is precisely what a constraint parameterised by hertz and damping must be. A box
  resting in 0.2 m of penetration finished at `1 kg 0.4733 · 100 kg 0.4092 · 300 kg 0.4084 ·
  1000 kg 0.4872` — 0.06 m of spread, and not even monotonic. Undivided it is 0.471068 at every
  mass from 1 kg to 5000.

  This reverses a change that had been landed as a bug fix with an arithmetic test showing only
  that the two orderings *differ*, never which one is right. That test is replaced by one
  asserting the property that decides it, plus a recursion test that exhibits the divergence
  and shows the clamp containing it. Determinism is unchanged (`A462C9EB8A09D5CA`) because the
  block solver, on by default, discards `impulse_scale` outright — asserted by a test so the
  scope claim cannot rot.

- **`NavMeshConfig::agent_radius` now erodes the walkable area.** *Behavioural for every
  navmesh built with a non-zero radius, i.e. the default.*

  The voxeliser computed a `ceil(radius / cell_size)` margin and used it only to widen the
  loop bounds — `blocked.insert` stayed gated on the obstacle's real AABB, so the setting
  produced no clearance at all and polygons ran right up to the wall. Each obstacle's blocked
  cells are now grown by that margin, so an agent that stays inside polygon interiors keeps
  its body clear. Clearance is quantised upwards to whole cells: at least the radius, up to
  one cell more.

  The floor-height sampling band moves outward with it rather than being swallowed by the new
  skirt. That band is the only writer of `walkable_y`, and blocking it in place drops every
  polygon to the `0.0` fallback — measured on that variant, not assumed. It keeps its old
  width and now sits just outside the skirt, so polygons near an obstacle still take their Y
  from its top surface.

  The three pre-existing navmesh build tests pass unchanged: they assert structure, not
  polygon coordinates, so nothing needed re-blessing. Three tests were added; the erosion one
  fails on the old build.

- **A floating-base articulated tree feels gravity.** *Behavioural for
  `ArticulatedTree { is_fixed_base: false }`, behind `experimental-multibody`.*

  In pass 3 of the ABA the root's parent acceleration is where gravity enters, as the
  fictitious `a_grav = (0, -gravity)`. The floating-base branch used `base_acceleration`
  *instead of* that term rather than in addition to it, so the `gravity` argument was accepted
  and discarded for the whole tree: with the default zero base acceleration a pendulum hung in
  mid-air at `q̈ = 0`. Both branches now share one formula — gravity, plus whatever base
  acceleration the caller prescribed — so a base at rest gives exactly the fixed-base answer,
  and a base falling at g (`base_acceleration = (0, +g)`) gives the weightless one.

  Fed a non-zero base acceleration the old branch did worse than drop gravity: it inverted the
  response. The free-fall input above produced `+4.9049997` where the correct answer is `0`,
  the exact negation of the `-4.905` fall. Nothing in the workspace sets `is_fixed_base = false`
  — every test and every default is fixed-base — which is why this survived property testing.

- **`glam` 0.29 → 0.32, `bevy_reflect` 0.15 → 0.19.** *No behavioural change — see below.*

  `glam` is the one deliberate permanent public dependency, so its major version is part of
  the 1.0 promise: shipping 1.0 on 0.29 would have made the upgrade a 2.0-level break for
  every downstream crate. It was three majors behind.

  The blocker was real but narrower than recorded: nothing in the workspace pinned 0.29
  except `gizmo-math`'s own manifest. What broke was the default-off `reflect` feature —
  `bevy_reflect 0.15` implements `Reflect` for `glam 0.29`'s types, so with the engine on
  0.32 those impls no longer applied. `bevy_reflect 0.16` is on glam 0.29 as well; 0.19 is
  the first release on 0.32, which is why the jump is four minors.

  The physics did not move: `state_hash` is unchanged at `EF6E4AC3644BF3BA` and every
  committed value in `tests/golden_state.rs` holds without re-blessing. That is worth
  stating explicitly — a maths-library major bump is exactly where silent numerical drift
  would hide, and the golden fixtures exist to answer that question rather than assume it.

  Benchmark-only follow-on: `bevy_math` / `bevy_picking` / `bevy_mesh` dev-dependencies moved
  to 0.19 too, so `glam` now resolves to a single version across the whole graph. Their APIs
  shifted — `CubicSegment::new_bezier` split off `new_bezier_easing`, `VectorSpace` gained a
  `Scalar` associated type, `ray_mesh_intersection` takes `Affine3A` plus a `uvs` argument,
  and `bevy_reflect`'s `clone_dynamic` became `to_dynamic_map` / `to_dynamic_list` /
  `to_dynamic_struct` with `Map` / `List` / `Struct` moved out of the crate root.


- **`compliance` is now an inverse stiffness.** *Behavioural for every joint with
  `compliance > 0` — ragdoll limits, elastic ropes, soft D6 locks.*

  The field is public, persisted, and documented as "0 = hard stop; larger = a soft, springy
  limit that gives under load". It did not behave like one. The implementation added
  `compliance / dt²` to the row's effective mass (CFM regularisation) and stopped there — but
  enlarging `k` only shrinks each iteration's step, so the sequential-impulse series still
  converges to the RIGID solution. All the observed softness came from `iterations` being
  finite. `compliance` was a relaxation factor for the solver loop, and doubling the
  iteration count halved its effect: the same rope stretched 0.0194 m at 5 iterations and
  0.0096 m at 10.

  Joints now use the same soft-constraint formulation the contact solver has always used
  (`bias_rate` / `mass_scale` / `impulse_scale`, Box2D v3), with each row's frequency derived
  from its compliance and effective mass as `ω = √(k/α)`. The result obeys Hooke's law:
  hanging 1 kg from a rope with `compliance = 0.03` settles `0.03 · 1 · 9.81 = 0.294 m` past
  its rest length, measured within 0.2% across two orders of magnitude of compliance and one
  of mass, and identical at 5, 10, 20 and 40 iterations.

  `compliance == 0` keeps the original rigid path unchanged, so nothing that did not opt into
  softness moves. `JointSolver` gains `compliance_damping_ratio` (default 1.0, critically
  damped) for the soft rows.

  If you tuned a ragdoll or a rope against the old numbers, re-tune: the value is now a
  physical spring constant rather than a solver artefact, and it no longer drifts when you
  change `iterations`.

- **`break_force` / `break_torque` now measure the joint's net reaction.** *Behavioural —
  finite thresholds already calibrated against the old numbers will need re-tuning.*

  Each joint type used to run its own break check from **inside** the solver's iteration
  loop, comparing `Σ|λᵢ|` — the L1 sum of its rows' impulse magnitudes — against the
  threshold. Summing magnitudes of rows that are not collinear does not give the force the
  joint carries. A weld's three linear rows are the world X/Y/Z axes, so the same 9.81 N
  load reported 9.81 N when gravity pointed down one axis and 17 N when it pointed
  diagonally: the reported force depended on the arbitrary orientation of the load relative
  to the world axes, and nothing else. On a ball-socket, whose cone/twist/swing rows are not
  even orthogonal, there was no bound on the overstatement.

  There is now one check, once per solver pass, against `‖Σ λᵢ·nᵢ‖ / dt` — the magnitude of
  the net impulse the joint actually applied. Three further consequences:

  - `Joint::check_break` was public API with **zero callers**. It is now the one code path.
  - A `Fixed` joint whose anchors were exactly coincident skipped its linear break check
    entirely, because the whole linear block sat behind an `err_len >= 1e-4` gate.
  - Slider suspension springs and hinge torsional springs carry real load and were invisible
    to the break check — a "breakable" shock absorber could hold any load forever. They now
    report into the same total. Motors and D6 drives deliberately do **not**: they are
    actuators, not external load.

  A joint that breaks now does so at the end of the pass rather than mid-iteration, so it
  transfers one extra step's worth of impulse before letting go. `world.joint_solver
  .iterations` no longer participates in the calculation at all.

- **`UiPlugin` and `TransformPlugin` now register on the per-frame schedule.** Layout,
  hit-testing and transform propagation are read once per rendered frame; running them
  `0..N` times in the fixed loop was both wasted work and, with vsync off, a hover that
  registered on roughly one frame in ten.

  Moving transform propagation also turns an intention into a guarantee. `PhysicsPlugin`
  labels its step `physics_step` with a comment saying transform systems "can order
  themselves after it", but no such edge was ever wired. The update schedule runs after
  every fixed step of the frame, so "transforms propagate after physics" is now structural —
  and it happens after the per-frame update systems too, which is what a camera moved by
  `FpsLookSystem` needs.

- **The headless runtime has a fixed timestep.** It used to run its single schedule once per
  loop iteration with the real elapsed `dt`; with the loop's 1 ms sleep that is roughly a
  thousand ticks a second, so a server registering `PhysicsPlugin` stepped physics ~1000
  times per second while the same plugin in the windowed runtime stepped at 60 Hz. Both
  runtimes now use the same sequencing, so a plugin behaves the same in either. Simulation
  determinism was never at risk — `PhysicsWorld::step` substeps internally at 240 Hz — but
  the cadence and the wasted work were real.


## [0.9.0] — 2026-08-04

A correctness and honesty release. It carries no new features: it closes two paths to
undefined behaviour, one determinism hole, and the reason the facade only ever compiled
in a single configuration — all found by an external audit
([`docs/AUDIT-2026-08.md`](docs/AUDIT-2026-08.md)), and all with the evidence written
down rather than summarised.

**It is a minor bump rather than a patch because three public signatures changed.** Two
of them changed because their previous shape was the bug: a fracture API seeded from
thread-local entropy cannot be reproduced, and a `&self` method that mutates a Lua VM
cannot be `Sync`. Keeping the old signatures would have meant keeping the defects.

Upgrading from `0.8.0`:

- `generate_fracture_chunks(..)` takes a trailing `seed: u64`. Pass anything
  reproducible — an entity id, a frame counter — not `rand::random()`.
- `ScriptEngine::has_function` and `::run_entity_update` take `&mut self`. Callers
  reaching them through a `World` resource need `ResMut`, not `Res`.
- `gizmo_app::headless::App` and `gizmo_app::windowed::App` now coexist. `gizmo_app::App`
  still resolves to the windowed runtime when it is compiled in, so most code is
  unaffected; `headless::App::add_plugin` is unavailable when both are present.
- Several facade items are now behind the feature that actually provides them. A default
  build is unchanged; a `--no-default-features` build now compiles at all, which it
  previously did not.
- The library is now genuinely named `gizmo`. Previously the package was `gizmo-engine`
  with no `[lib] name`, so the real path was `gizmo_engine::` — and `use gizmo::prelude::*`,
  which is what the README, the crate docs and every example show, only compiled for
  someone who renamed the dependency in their own manifest. Copying the quickstart after
  `cargo add gizmo-engine` failed on line one. If you were using `gizmo_engine::` paths
  directly, either switch them to `gizmo::` or rename in your manifest
  (`gizmo_engine = { package = "gizmo-engine", version = "0.9" }`).

### Fixed — soundness

- **`unsafe impl Sync for ScriptEngine` was unsound and published.** mlua's `send`
  feature makes `Lua` `Send`; it never makes it `Sync`, because the `lua_State` is
  mutated through `&Lua`. `has_function` and `run_entity_update` did exactly that
  behind `&self`, and `ScriptEngine` is stored as a `World` resource — so two
  systems holding a shared reference could race on the interpreter. Both methods
  now take `&mut self`, which makes concurrent VM access unrepresentable rather
  than merely discouraged. **Breaking** for direct callers; there were none outside
  the crate.
- **`World::query_entity_mut` skipped its aliasing check.** Every other query entry
  point runs `check_aliasing` first; this one did not, so
  `query_entity_mut::<(Mut<T>, Mut<T>)>(id)` was safe code that handed out two live
  `&mut T` to the same row — undefined behaviour with no panic and no compile error.
  It now panics like the other paths, as does `query_entity` for symmetry.

### Fixed — animation

- **`Track::sample` panicked on a single-keyframe track with a `NaN`.** `idx.clamp(1, len - 1)`
  becomes `0.clamp(1, 0)` when the track holds one keyframe, and `Ord::clamp` asserts
  `min <= max`. It is reachable because a `NaN` — in the sampled time or in the track's own
  timestamps — makes both of `sample`'s early-return comparisons false, so control falls
  through to the clamp. The inline comment above that line described the clamp as the NaN
  guard; it was the thing that panicked.

  `Track::new` rejects non-finite timestamps, but `Track`'s fields are public, so a
  hand-built or deserialized track bypasses it. Found while documenting the crate for D4.

### Fixed — sleeping

- **Waking now travels a jointed mechanism in one step.** A sleeping body does not integrate,
  so a joint pulling on one has its correction silently swallowed and the mechanism looks
  broken. The wake propagation existed, but as a single pass over `world.joints` in array
  order: disturbing the deep end of a 12-link chain woke only 5 links in one step — one per
  substep — while the seven above kept absorbing joint corrections they never integrated, so
  the chain behaved as if pinned partway down.

  Physically this is not a subtlety: an inextensible chain loads every link the instant the
  bottom one is disturbed. Contacts have had the right answer all along — `island.rs`
  union-finds manifolds so a whole pile wakes together — but joints were never part of island
  construction. Wake propagation now runs over the joint-connected COMPONENT: one mover
  anywhere in it wakes all of it, and a component with no mover is left alone. Cost scales
  with the joint count, not the body count.

### Fixed — rollback

- **`WorldSnapshot` now carries `PhysicsWorld::weather`.** Same omission as the joint state
  below, one field further along.

  The rigid pipeline never reads `weather`, which is why it was easy to leave out — but the
  vehicle tyre model scales its friction-circle limit from it, and it cannot be recomputed
  from transforms or velocities. Gameplay switching weather inside a rollback window therefore
  left the re-simulation running vehicles under the grip of a weather it had already rolled
  back past, invisibly: `state_hash` covers only transform, velocity and sleep state.

  Found by applying the inclusion rule written on `WorldSnapshot` when the joint state was
  added — the criterion is derivability, not size.

### Fixed — rollback

- **`WorldSnapshot` now carries joint state.** Rollback could not un-break a joint.

  `Joint::is_broken` is a one-way latch — nothing outside scene load ever sets it back to
  `false` — and `joints` was not in the snapshot at all. A joint that snapped inside a
  rollback window stayed snapped through the restore, so the re-simulation ran without a
  joint the continuous simulation still had, permanently. The same applied to
  `initial_relative_rotation`, the reference pose latched on a joint's first solve, against
  which every cone/twist/swing limit is measured: a stale one silently redefines the joint's
  rest pose.

  Neither is visible to `state_hash`, which hashes only transform/velocity/sleep — so the
  desync stayed invisible until it bled into velocities. `tests/rollback.rs` gains two cases
  that fail on the old code, using a rope that goes taut (and breaks) at tick 38, inside a
  snapshot window opened at 20.

  The criterion for what belongs in `WorldSnapshot` is not size but derivability: anything
  that cannot be recomputed from `transforms`/`velocities` has to be in it, and its absence
  cannot be caught by the hash. That rule is now written on the type.

### Fixed — determinism

- **`generate_fracture_chunks` seeded itself from thread-local entropy**, so
  identical inputs produced different chunk geometry, masses and spins. Any scene
  that fractured diverged between replays and desynced under rollback. It now takes
  an explicit `seed: u64`. **Breaking**: the parameter is required, because keeping
  the old signature would mean keeping the bug. The ECS fracture path
  (`shatter_entity`) was already deterministic and is unaffected.

### Fixed — feature composition

- **`gizmo-engine` compiled in exactly one configuration.** Its own advertised
  `headless` feature failed with 37 errors, and `--no-default-features` plus every
  single-feature build failed too — so the README's headless-server story was dead
  code. The facade's modules are now gated on what they actually use.
- **`gizmo-physics-core` is now a mandatory dependency of the facade.** It defines
  `Transform`, `GlobalTransform` and `Collider`, so gating it behind `physics` also
  broke `--features render` and `--features audio`: nothing can be drawn or
  spatialised without a transform. `physics` now gates the *simulation*
  (`gizmo-physics-rigid`), which is the honest split.
- **`gizmo-app`'s `window` feature was non-additive** — enabling it *deleted*
  `headless::App`. Since Cargo unifies features across the whole graph, an unrelated
  dependency turning `window` on could silently swap a simulation server's `App`
  type out from under it. Both runtimes now coexist and are reachable by path; the
  root re-export still prefers the windowed one, so existing code is unaffected.
  `headless::App::add_plugin` is unavailable when both are compiled in, because
  `Plugin::build` is typed against the root `App`.
- A `feature-powerset` CI job (`cargo hack --depth 2`) now covers both entry crates.

### Added — supply chain and process

- `deny.toml` and a `cargo deny` CI job covering advisories, licences, sources and
  duplicate versions. Every exception carries a written justification. The first run
  fixed one advisory (`crossbeam-epoch` → 0.9.20) and documented five, including
  `bincode 1.x`, which is a direct dependency of `gizmo-net` and is tracked for
  migration.
- MPL-2.0 (via `rodio` → `symphonia`, on the default `audio` feature) is now
  disclosed explicitly rather than sitting unremarked under a flat "MIT OR
  Apache-2.0".
- `CONTRIBUTING.md`, `SECURITY.md`, `.github/dependabot.yml`.
- `docs/AUDIT-2026-08.md` — an external review with every finding pinned to
  `file:line` — and `docs/FIXPLAN.md`, which tracks the work it opened.

### Fixed — rendering

- **Six point-light shadow passes were recorded every frame into a cubemap nothing
  sampled.** `Renderer::point_shadows_enabled` defaults to false and
  `deferred_lighting.wgsl` already gated its lookup on the uniform written from that same
  bool, but `record_shadow_passes` did not — so a lit batch spent twelve of its
  twenty-three draws filling a 1024×1024×6 depth cubemap for nothing. Both sides now read
  the one flag. A golden-image test renders the scene with it on and off and demands
  byte-identical frames, which is both the claim (the skipped work was unobserved) and the
  guard against gating a pass the shader really does sample.

### Fixed — other

- The two golden-image GPU tests are serialised behind a mutex. Each requested its
  own wgpu device, and `cargo test` runs a binary's tests in parallel — concurrent
  device creation surfaced as an intermittent `SIGSEGV` inside the driver that took
  down the whole workspace run.
- `car_demo` and `wind_tunnel` loaded models from absolute paths into the original
  author's home directory and unwrapped the result, so the demo the README tells
  people to run panicked for everyone else. Optional assets now resolve through
  `demo::assets` and fall back to procedural geometry.
- README feature claims corrected: there is no Sweep-and-Prune broadphase (it is a
  dynamic AABB tree, and single-threaded), no `gizmo-physics` crate, no mimalloc in
  `gizmo-core`, and no Doppler in `gizmo-audio`. Determinism — the one property here
  with no equivalent in Rapier or Avian — is now stated, having previously gone
  unmentioned.

### Added

- **Ergonomics (DX).** `Prefab` — a define-once / spawn-many blueprint (mesh +
  material + optional `RigidBodyBundle`) with `spawn` / `spawn_at` /
  `spawn_with_mass` + per-instance `with_pbr`. `AutoBoxCollider` — derive a box
  collider from an entity's `Transform.scale` so the size is authored once
  (opt-in marker + a synchronous `Prefab` path). Auto-despawn lifetime
  components (`DespawnAfter` / `DespawnBelowY` + `LifetimePlugin`), `FpsLook`
  mouse-look camera controller, `World::despawn_all_with::<C>()` bulk despawn.
- **Tooling.** Broad unit-test sweep (~1376 tests across the workspace);
  structured `tracing` logging (instrument spans + fields) across the value
  crates, with silent error-swallows promoted to `warn!` / `error!`.

### Changed

- **Docs.** Consolidated 12 planning / fix-plan documents (roadmap, releasing,
  determinism, migration, architecture, and the finished FIX-PLANs) into a
  single [`docs/ENGINE.md`](docs/ENGINE.md); `README` / `CHANGELOG` /
  `demo-web/README` stay standalone.

### Fixed

- **Physics — resting-stack stability.** A settled box stack that spontaneously
  gained energy and blew up (lateral buckling) is fixed by a manifold **block
  solver** (coplanar normals solved jointly + Tikhonov regularization) plus
  **full warm-start** (`warm_start_factor` 0.85 → 1.0) — stable to N≤32 (was
  ~N≤16); N≥48 towers remain open. See [`docs/ENGINE.md`](docs/ENGINE.md) §7.
- **Rendering — 6 latent bugs.** World tangent (plain model 3×3, not
  inverse-transpose); PBR param-packing overflow at 1.0; ECS query
  `get` / `contains` now honour table-storage `With` / `Without` filters
  (matched `iter`); shadow-caster instance ordering (two-region layout); glTF
  `AlphaMode::Mask` cutout (alpha-cutoff discard).
- **Physics — perf.** Quadratic costs removed (broadphase pair dedup
  O(P²)→O(P); per-island TGS scratch sized to the island; per-contact constants
  hoisted out of the sweep loop): worst frame 262→46 ms on a 2000-box scene.
- **App — GPU robustness.** Surface `Outdated` / `Lost` now reconfigures the
  swapchain and backs off (rate-limited) instead of freezing or busy-spinning;
  `CloseRequested` shuts down gracefully (runs `Drop` → clean wgpu teardown)
  instead of `process::exit(0)`.

## [0.8.0] — 2026-07-12

A large feature release gathering ~205 commits since `0.2.0`. The whole
workspace continues to ship at one uniform `0.x` version (the staged `1.0`
model in [`docs/ENGINE.md`](docs/ENGINE.md) remains the planned later path). No
crate-level API is promised stable yet; treat any change as potentially
breaking and pin an exact `=0.8.0` if you need reproducibility.

### Added

- **Physics — joints.** First-class `Distance`/`Rope` joint; a generic 6-DoF
  (`D6`) joint with per-axis motors + springs; cone-twist, slider suspension,
  and hinge torsional-spring joints; per-joint compliance, asymmetric cone
  limits, distance reachability, spring-break, and servo motors.
- **Physics — bodies & vehicles.** Consolidated vehicle simulation in
  `gizmo-physics-dynamics` (dynamics is now canonical; the dead rigid vehicle
  path was removed); ECS systems for vehicle/character + ragdoll runtime;
  opt-in aerodynamic drag (½ρCdAv²) for rigid bodies; CCD exposed via bundle
  builders with analytic test ladders; `RigidBodyBundle` derives rotational
  inertia from its collider.
- **Physics — soft bodies & water.** Hardened cloth ↔ rigid-body collision
  (capsule, per-segment edge, averaged push) plus cloth tearing; a Subnautica-
  style water system (`water_at` query, swimming controller, Gerstner waves,
  underwater camera fog) and character oxygen.
- **Physics — ergonomics.** Fluent builders for materials, colliders, bodies
  and bundles; `PhysicsPlugin` auto-steps at the app's fixed timestep;
  `GameplayPhysicsPlugin` registers vehicle/character systems.
- **Rendering.** Textured PBR (normal / metallic-roughness / emissive / AO
  maps); distance-based texture streaming wired end-to-end; AAA smoke VFX
  (soft particles, flipbook, curl-noise, lit) with volumetric ray-marched
  smoke; headless/offscreen renderer (no window/surface); HighPerformance GPU
  adapter preference.
- **Web / WASM.** The deterministic simulation core compiles to `wasm32`, and
  the full engine runs in the browser (WebGPU/WASM) with an audio backend and
  a hardened web surface.
- **Animation & glTF.** Two-bone IK + FABRIK, cubic-Hermite scale tracks;
  `KHR_texture_transform`, `KHR_materials_emissive_strength`, and glTF sampler
  settings honoured.
- **Camera.** Orthographic projection mode (Numpad5 toggle) and
  `screen_to_ray` screen→world picking.
- **CI.** Run-once benchmark gate (and the engine bug it caught).

## [0.2.0] — 2026-06-25

The first release since `0.1.7`. It gathers the entire 1.0-readiness effort
(audit + hardening rounds) and the graphics-stack upgrade, shipped as a single
breaking `0.x` bump. **Upgrading from `0.1.x`? See the
[migration guide](docs/ENGINE.md).**

### Changed (breaking)

- **ECS query API split along the safe/unsafe boundary (closes a soundness hole).**
  `World::query::<Q>(&self)` previously accepted a *mutable* query (`Q = Mut<T>`)
  from a shared `&World`, so two live `Mut<T>` queries (or `Mut<T>` + `&T`) could
  alias the same storage — reachable from **safe code**, with no panic. The query
  surface is now:
  - `World::query::<Q: ReadOnlyQuery>(&self)` — **read-only** (`&T`, `With`/
    `Without`/`Changed`/`Added`, `Or`, and tuples of those).
  - `World::query_mut::<Q>(&mut self)` / `World::borrow_mut::<T>(&mut self)` —
    safe **mutable** access (requires `&mut World`).
  - `unsafe World::query_unchecked::<Q>(&self)` / `borrow_mut_unchecked::<T>` —
    escape hatch for code that only holds `&World` (e.g. inside the parallel
    scheduler's `System::run(&World)`), with a documented `# Safety` contract.

  Migrate by replacing `world.query::<Mut<T>>()` with `world.query_mut::<Mut<T>>()`
  (`borrow_mut` now needs `&mut World`); pure-read call sites are unchanged. On a
  `Query`, `iter`/`get`/`iter_chunks`/`par_for_each`/`entities`/`contains` are
  read-only; use `iter_mut`/`get_mut`/`iter_chunks_mut`/`par_for_each_mut` for
  mutation. Behavior is unchanged (determinism hash identical).
- **`RigidBody` lost its `friction` and `restitution` fields**, and
  `RigidBody::new` is now `new(mass, use_gravity)` (was
  `new(mass, restitution, friction, use_gravity)`). These fields were **dead**:
  the contact solver always sourced friction/restitution from the colliders'
  `PhysicsMaterial` (combined per contact), so setting them on the body did
  nothing — the editor inspector even exposed two no-op sliders. Configure
  contact friction/restitution on the collider material instead. Determinism is
  unchanged (proof the fields never affected the simulation). The scripting layer
  followed suit: the Lua `physics.add_rigidbody(id, mass, use_gravity)` binding
  and `ScriptCommand::AddRigidBody` dropped their (ignored) `restitution`/
  `friction` parameters.
- **Graphics stack upgraded** across the Stage B crates: `wgpu 0.20 → 29`,
  `winit 0.29 → 0.30`, `egui 0.28 → 0.34` (plus `egui-wgpu`/`egui-winit` `0.34`,
  `egui_dock 0.13 → 0.19`, `transform-gizmo-egui 0.3 → 0.9`). Public `wgpu`/
  `winit`/`egui` types in the renderer/window/editor/app/facade move to the new
  versions. See [`docs/ENGINE.md`](docs/ENGINE.md) (§6).
- **`bevy_reflect` is now gated behind an off-by-default `reflect` feature** on
  `gizmo-core`, `gizmo-physics-core`, `gizmo-physics-rigid`, and `gizmo-scene`.
  With default features, scene save/load + snapshots fall back to plain `serde`
  (every reflected component also derives `Serialize`/`Deserialize`), and
  `bevy_reflect` no longer appears in the default public API or — after the
  `gizmo-math` dependency-hygiene fix below — in the Stage A dependency tree.
- **`CollisionEvent.contact_points`** is now an opaque `ContactPoints` newtype
  (`gizmo_physics_core::collision::ContactPoints`) instead of leaking
  `arrayvec::ArrayVec`.
- **96+ public enums/structs marked `#[non_exhaustive]`** (error/shape/event
  enums and `Default`/constructor-guaranteed config structs) so future variants
  and fields are not breaking. Closed leaf math/config types are intentionally
  exempt.
- **Many constructors/loaders now return `Result`/`Option`** instead of
  panicking (`spawn_gltf`, `ComponentRegistry::register`, `SceneData::save/load*`,
  `AudioManager::new/play*`, `NetworkClient/Server::new`, `AppWindow::new`,
  `App::run`, renderer `load_*`, …), and 13 concrete error enums were added.
- **Infallible plain-value getters dropped the `get_` prefix** (`get_neighbors →
  neighbors`, `get_entity_component_types → entity_component_types`,
  `get_log_version → log_version`, `get_engine_torque → engine_torque`,
  `get_entity_names → entity_names`). Fallible `get_*` accessors that return
  `Option`/`Result` keep the prefix, following the Bevy convention.
- **MSRV raised to `1.92`** (floor set by `egui 0.34`), up from `1.89`. Enforced
  by a CI `msrv` job. Earlier in the cycle the MSRV was empirically set to `1.89`
  (1.82/1.85 fail on transitive `crypto-common`/`wide`/`safe_arch`).
- **`glam` is now re-exported directly** (`pub use glam::{…}` in `gizmo-math`)
  and documented as an official public dependency, rather than via `bevy_math`.

### Added

- **The engine now runs in the browser (WebGPU/WASM).** `gizmo-renderer`,
  `gizmo-app` and the facade build for `wasm32-unknown-unknown` with a web
  feature subset, using a reduced 4-bind-group forward pipeline (browser
  `maxBindGroups = 4`; shadows/deferred/compute disabled on wasm). The new
  `demo-web/` crate (wasm-bindgen + `index.html`) shows a live physics scene in
  the browser and was verified end-to-end in headless Chrome. `gizmo-app`'s wasm
  `resumed` implements the async WebGPU init via `spawn_local`; `gizmo-scripting`
  (mlua) is target-gated to native, and the CI `wasm` job now also builds the
  graphics stack. Audio/networking/scripting remain native-only (RELEASING §4g).
- Deterministic same-platform **rollback netcode** (`gizmo-net`, `rollback`
  feature): `PhysicsWorld::snapshot`/`restore_snapshot` (full internal state incl.
  contact warm-start), a `Transport` trait with real-UDP and loopback impls, and
  a GGPO-style `RollbackSession` that converges under lag + packet loss.
- `PhysicsWorld::state_hash()` sync-hash API (process-stable) for desync
  detection and replay, plus a cross-process determinism oracle.
- **TGS Soft constraint solver** (Box2D-v3-style) for stable tall/high-energy
  stacks, with dormant-pair narrow-phase skipping for wide settled scenes.
- Continuous collision detection (CCD) hardening (no tunnelling), full joint
  library behavioural coverage, island-aware sleeping, and a phase-timed
  `PhysicsMetrics` profiler.
- Property-based and differential test suites across ECS, collision, raycast,
  SAT, ABA/multibody, joints, soft-body, and fracture; a CI matrix
  (ubuntu/macos/windows), a ratcheted `clippy -D warnings` gate, and a headless
  determinism gate.
- `docs/ENGINE.md` (§4 staged-1.0 strategy) and this changelog.

### Fixed

- **`gizmo-math` dependency hygiene:** removed an unused regular `bevy_math`
  dependency that transitively pulled `bevy_reflect` into the Stage A *production*
  dependency tree even with the `reflect` feature off. `bevy_reflect` is now
  absent from the default Stage A tree.
- Numerous correctness fixes across the EPA/GJK contact pipeline, integrator
  (body-space inertia), split-impulse leakage, joint effective-mass, renderer
  mesh winding + skin-weight normalisation + skinned-normal inverse-transpose,
  and post-process depth linearisation (see git history for the per-round audit
  detail).
- **egui 0.34 / winit 0.30 deprecations migrated** off the crate-level
  `#![allow(deprecated)]` bridges left by the graphics upgrade: all mechanical
  egui renames, plus the top-level panel `show(ctx)` pattern migrated to egui
  0.34's root-`Ui` composition (`show_inside`). The only remaining (scoped,
  documented) deprecation is winit's closure `EventLoop::run`/`create_window`
  bridge in `gizmo-app`, whose `ApplicationHandler` migration is deferred.

## [0.1.7] — earlier

Initial published series (`0.1.x`) on crates.io: the ECS, math, physics
(rigid/soft/dynamics), renderer, editor/studio, audio, AI, scripting, and
client-server netcode that make up the engine. See the git history for details.

[0.2.0]: https://github.com/bdrtr/Gizmo/compare/v0.1.7...v0.2.0
[0.1.7]: https://github.com/bdrtr/Gizmo/releases/tag/v0.1.7
