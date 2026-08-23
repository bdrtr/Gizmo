# API depth — automatic by default, no ceiling underneath

**Status:** plan, not yet executed. **Written:** 2026-08-23.

## The brief

> What should be automatic should be automatic. But if someone wants to go into the detail, there
> should be no limit — without it being punishing.

That is two promises, and they pull in opposite directions unless the API is **layered**. This
document is how they get kept at the same time.

It is a different axis from `ENGINE.md` §4, and does not compete with it. §4 governs *which
foreign types may appear on a public surface* and what each one costs. This governs *how far down
a user can go before the engine stops them*. A change can satisfy one and violate the other.

## The problem, stated precisely

The engine's convenience layer is good. `with_simple_scene` plus `spawn_cube` gets a lit,
physics-stepped scene on screen in a dozen lines, and that is worth keeping exactly as it is.

The instability is not there. It is one layer down, where the doors are locked:

| what a game wants | what stops it | measured in |
|---|---|---|
| its own system parameter | `SystemParam` is sealed — **4 impls exist**: `Res`, `ResMut`, `f32`, `Query` | `ecs_extension_points` |
| a derived query operand | `WorldQuery` / `ReadOnlyQuery` / `FetchComponent` all sealed | `ecs_extension_points` |
| ~~a schedule phase of its own~~ | **opened 2026-08-23** — `Phase::User(u16)`, positioned | `schedule_internals` |
| its own material or shader | `MaterialType` is a closed enum; `routing.rs` is exhaustive on purpose | `deferred_rendering` |
| a normal map on a hand-built material | `assemble_material_bind_group` is `pub(crate)`; `Layouts` is `pub(super)` | `parallax_mapping` |
| two camera passes in one frame | one `global_uniform_buffer`, written with `queue.write_buffer` | `render_to_texture` |
| a post-pass that reads the frame | the surface has no `TEXTURE_BINDING` | `post_processing` |
| to keep the simple scene *and* add an exclusive pass | `set_update` replaces the hook | `demo::simple_scene_update` |
| less volumetric rather than none | no `enabled` flag; the off switch destroys the state | `volumetric_fog` |

Every row is a **ceiling, not a missing feature**. In each case the engine can already do the
thing — it just will not let the game ask. That is what makes the API feel unstable: a user who
hits one of these has no move except to fork, wait, or drop the engine, and that pressure is
exactly what produces fast breaking releases.

## The rule

> **No convenience without a named hatch.**
>
> Every function, component or builder that decides something on the user's behalf must name, in
> its own documentation, what to reach for when that decision is wrong.

This is cheap, it is the whole difference between a ceiling and a staircase, and — importantly for
this repository — it is **enforceable**. `crates/gizmo/tests/doc_language.rs` and
`prose_counts.rs` already show the pattern: a rule that is tested stops being a rule that is
remembered. A `hatch_docs.rs` test can scan the convenience surface and fail on an item that
decides something and points nowhere.

Today `App::set_render`'s doc is the model — *"the hook for a pass the engine does not have"* —
and it is why `post_processing` could be written at all.

## The three layers

Naming them gives the rule something to attach to.

**L0 — the simple scene.** `SimpleApp`, `with_simple_scene`, `spawn_cube`, `spawn_ground`,
`spawn_camera`. Zero decisions asked of the user. Stays exactly as ergonomic as it is now; every
addition here must be a *shortcut for* an L1 sequence, never the only way to do something.

**L1 — components and systems.** `spawn_bundle`, the bundles, `Query`, `Res`, `add_update_system`,
`Material`, `MeshRenderer`. The surface a real game lives in. Most of this document's work lands
here: it is where the ceilings are.

**L2 — the raw seam.** `set_render` with the wgpu encoder and target view, `&mut World` hooks,
`Renderer`'s public fields. Already exists and already works — `post_processing` measures a
user-authored pass landing on 38.67 % of pixels with the untouched rows bit-identical. What it
lacks is *reach*: several L1 decisions cannot be re-taken from L2 because the pieces are private.

The failure mode to design against is not "L2 is hard". It is **"L1 decided, and L2 cannot
un-decide"**.

## The closures, one by one

Not every seal is the same, and treating them alike would be the wrong fix. Three verdicts:
**load-bearing** (the seal is the invariant — keep it, add what people actually wanted),
**incidental** (closed with no invariant behind it — open it), **architectural** (opening it is a
real design, not a visibility change).

### Load-bearing — keep sealed, widen instead

**`WorldQuery` / `ReadOnlyQuery` / `FetchComponent`.** The seal *is* the soundness argument: a
wrong impl re-opens dual-`&mut` aliasing, and `World::query`'s `Q: ReadOnlyQuery` bound is what
makes building a mutable query from `&World` impossible in safe code. Opening this trades a
compile error for undefined behaviour.

What people actually reach for, and can be given without opening it:

- ~~**`Option<Res<T>>`**~~ — **done 2026-08-23.** `Option<P>` for any `P: SystemParam`, added
  inside `gizmo-core` rather than by opening the seal. Measured in `error_handling`: `Res<T>`
  panics, the `run_if` guard runs 0 times, `Option<Res<T>>` runs 90 times seeing `None` in all 90.
  Only absence becomes `None` — a borrow conflict still panics, because it is a bug, not a state.
- **a derive for composite params** — `#[derive(SystemParam)] struct Ctx<'w> { a: Res<'w, A>, b: Query<'w, &B> }`. The derive can emit the access declaration mechanically, which is exactly the
  information the seal exists to protect. This is the honest way to open `SystemParam` (below).
- **`iter_combinations`** — currently impossible to hand-roll in one pass, because a query holding
  `Mut<T>` cannot be iterated read-only at all. Measured in `iter_combinations`: one call becomes
  three passes plus a `Vec`.

### `SystemParam` — load-bearing, but openable with the invariant made explicit

The scheduler must know statically what a parameter touches, or parallel batching is unsound.
That is real. But the seal is not the only way to hold it: the invariant can be **stated by the
implementor** instead of assumed by the crate.

```rust
/// # Safety
/// `access` must declare every component and resource `fetch` touches. The scheduler batches
/// systems on this declaration alone; under-declaring is undefined behaviour, not a bug.
pub unsafe trait SystemParam { … fn access(info: &mut AccessInfo); … }
```

`unsafe trait` puts the obligation where it belongs and keeps safe code safe. The derive above
then becomes the route 95 % of users take, with the raw impl available underneath — which is the
staircase, applied to the ECS.

### Incidental — open these

**`Phase`.** ✅ **Done, 2026-08-23.** It was a closed 5-variant enum with no invariant behind it;
the scheduler sorted phases by their ordinal. `Phase::User(u16)` now carries a *position* on a
scale where the built-ins sit at round thousands, so a user phase can be placed *between* two
built-ins rather than only after them — and `Ord` is hand-written over `position()`, because a
derived one would have sorted every `User` behind every built-in and defeated the point. The enum
is `#[non_exhaustive]` so a sixth built-in phase is not another breaking change. Measured: 599 of
600 frames held the expected ten-probe sequence, 0 deviated.

**`assemble_material_bind_group` (`pub(crate)`) and `Layouts` (`pub(super)`).** The material bind
group has seven entries, four of them detail maps — normal, MR, emissive, AO — filled with
defaults. A game outside `gizmo-renderer` cannot supply any of them; the only route is the glTF
loader. `parallax_mapping` prices the workaround at **6 verts → 55 296** for 2.87× the shading
variation, because geometry is the only surface detail a hand-built scene can get. Making the
assembler public behind a builder is a visibility change with a documented layout, not a design.

**`set_update` replacing rather than composing.** Half-fixed already: it warns on overwrite and
`gizmo::simple::simple_scene_update` is public. The rest is an additive form —
`add_update_hook(f)` — with `set_update` kept as the deliberate replace.

**Effect on/off flags.** `renderer.volumetric = None` destroys the state; turning it back on means
`VolumetricState::new(device, scene, deferred, w, h)`. Same shape in SSR. An `enabled: bool`
beside each is a field, not a redesign.

**Effect parameters.** `VolumetricState` exposes textures, views, pipelines and dimensions — not
one shaping field. Phase 0.55, 16 steps, a 100 m cap, sun scatter 0.0015, bulb 0.0008, bias 0.16
are all shader literals. A `VolumetricParams` uniform is mechanical work with a measured payoff:
the effect already moves 19.55 % of pixels, max 134, and none of it is reachable.

### Architectural — real design, sequence them deliberately

**`MaterialType` → user materials.** The largest one, and `ENGINE.md` §4 has a stake in it: a
material trait that hands out a `wgpu::RenderPipeline` puts wgpu on a Stage-A-adjacent surface.
The shape that survives that constraint is an opaque handle:

```rust
pub enum MaterialKind { BuiltIn(MaterialType), Custom(MaterialId) }
```

with `MaterialId` minted by a registration call that takes WGSL plus a declared bind-group layout.
`routing.rs` stays exhaustive over `MaterialType` — its "one compile error beats two silent
misroutes" property is worth keeping — and gains a single `Custom` arm that defers to the
registry. The four-target G-buffer budget is the real constraint here, and it is why this is
architectural rather than incidental: a custom material either fits the existing G-buffer
contract or declares itself forward-only.

**One `global_uniform_buffer`.** `render_to_texture` measured three obstacles between here and a
second camera and found the third decisive: uniform writes order against *submission*, not against
encoder recording, so two passes in one encoder both read the last camera written. The fix is a
per-pass uniform slice (or an explicit `RenderView` the pass carries), and it is the prerequisite
for split-screen, planar mirrors, and reflection probes — three separate capability gaps that all
terminate here.

**A frame the user's pass can read.** The surface is `RENDER_ATTACHMENT` (+`COPY_SRC`) with no
`TEXTURE_BINDING`, so a user pass can blend but never sample. Position-only effects work; a user
FXAA, radial blur or frame histogram cannot be written at all — and that last one is also why
`auto_exposure` has an actuator and no sensor. An intermediate HDR target the user can bind
closes all of it.

## The version half

The criticism that the jumps are fast and breaking (0.1.7 → 0.8.0 → 0.10.0) is fair, and the
remedy is not slower releases — it is removing the *reason* for them.

**Most breaking changes here were ceilings being raised.** A user who hits a locked door cannot
work around it, so the engine has to move for them, and moving means breaking. Each hatch this
document opens is one fewer future break, because the escape route stops being "change the
engine".

Three commitments that cost nothing to adopt now:

1. **A deprecation window.** Nothing is removed without one minor release carrying
   `#[deprecated]` with the replacement named. The `deprecated/` tombstones published on
   2026-08-23 are the same discipline applied to crate names.
2. **The hatch rule is a gate.** A new convenience that names no hatch does not land. Testable,
   like `doc_language` and `prose_counts` already are.
3. **Say which layer a change touches.** An L0 addition is additive by construction. An L1 change
   is where semver actually lives. An L2 change is where wgpu's version leaks through, and §4
   already governs that.

## Sequence

Ordered by unlocked-capability per unit of work, not by size.

1. **The hatch rule and its test.** Costs nothing, changes how everything after it is reviewed,
   and immediately documents the L2 routes that already exist and nobody knows about.
2. **`Option<Res<T>>` — done — and the `SystemParam` derive.** The most-missed thing, and the
   derive is what makes opening the trait honest later.
3. **`Phase::User`, the `enabled` flags, `add_update_hook`, `VolumetricParams`.** A batch of
   incidental opens. Each is small; together they close `custom_schedule`, the SSR/volumetric
   on-off destruction, the `set_update` trap and a whole row of section B.
4. **The material bind-group builder.** Unlocks normal/MR/emissive/AO maps for hand-built scenes,
   which is the difference between 6 and 55 296 vertices for the same surface detail.
5. **Per-pass uniforms.** One change, three capabilities: multi-viewport, planar reflections,
   reflection probes.
6. **A bindable HDR target.** Unlocks user post-processing that reads the frame, and with it
   auto-exposure's missing sensor.
7. **User materials.** Largest and most architectural; wants the G-buffer budget question settled
   first, and interacts with `routing.rs` and `render_parity`.

Items 5, 6 and 7 are also the three that would let the two unwired subsystems
(`gizmo-renderer::visibility` and `::gi`, `CAPABILITY_GAPS.md` §F) be plugged in without a user
having to reimplement them.

## What this document is not

It is not a promise of 1.0. `ENGINE.md` §4 is explicit that the staged-1.0 plan is not being
pursued and the workspace keeps one `0.x` line. Nothing here changes that. What it changes is the
shape of the surface underneath, so that when a 1.0 *is* defensible, it is defensible because the
engine stopped needing to break — not because it stopped moving.
