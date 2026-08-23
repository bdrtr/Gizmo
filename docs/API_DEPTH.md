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
| ~~its own material or shader~~ | **opened 2026-08-23** — `MaterialType::Custom(MaterialId)` | `custom_material` |
| ~~a normal map on a hand-built material~~ | **opened 2026-08-23** — `AssetManager::material()` | `parallax_mapping` |
| two camera passes in one frame | one `global_uniform_buffer`, written with `queue.write_buffer` | `render_to_texture` |
| a post-pass that reads the frame | the surface has no `TEXTURE_BINDING` | `post_processing` |
| ~~to keep the simple scene *and* add an exclusive pass~~ | **opened 2026-08-23** — `add_update_hook` | `update_hooks` |
| ~~less volumetric rather than none~~ | **opened 2026-08-23** — `VolumetricParams`, six fields | `volumetric_fog` |

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
remembered.

✅ **`hatch_docs.rs` exists as of 2026-08-23**, and checks two things a document would otherwise be
trusted for: every API this file names still exists somewhere in the tree, and every hatch's **own
doc block** names the convenience it is a hatch for. It does not try to detect "a convenience"
automatically — every heuristic for that either floods or misses, and a test that cries wolf gets
ignored. The pairs are written down; adding one costs a line.

It found three things on its first run. A demo this document called custom_schedule had been
renamed to `schedule_internals` — a route offered here that the tree did not have. (That sentence
names the dead one in plain text on purpose: code font is a claim that something exists, and the
test reads this file too, so writing the old name in backticks fails the check it just passed.) `Option<P>`'s forty
lines of documentation sat above `impl … sealed::Sealed for Option<P>`, a private trait, so rustdoc
rendered none of it: the engine's only optional parameter was explained where nobody could read it.
And the test's own first version could not be made to fail — it searched whole files, so deleting
the sentence pointing `SceneView` at `global_uniform_buffer` still passed on the six other
mentions in that file. Narrowed to the doc block above each declaration, it goes red on exactly
that edit.

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
- ~~**a derive for composite params**~~ — **done 2026-08-23**, as `system_param!` rather than a
  derive. A `macro_rules!` macro needs no proc-macro crate and no new workspace member, and it does
  the one thing that mattered: forwards `get_access_info` field by field, so the declaration cannot
  drift from what the struct fetches. That is exactly the information the seal exists to protect,
  produced by construction rather than by care. `sealed::Sealed` is now `#[doc(hidden)] pub` so the
  macro can name it — the seal still means what it meant, since implementing it by hand still means
  writing the access declaration by hand.

  Guarded at two levels: the declaration must equal the sum of the fields' own, and the
  **scheduler** must separate a composite writer from a plain `ResMut` writer of the same resource.
  Both go red when the forwarding is removed, the second with "two writers of the same resource
  were put in one batch". Measured in `system_forms`: one parameter carrying three, 89 runs,
  reading `health 240 · mana 120` — the same numbers the generic systems report separately.
- ~~**`iter_combinations`**~~ — **done 2026-08-23** read-only, **and `_mut` 2026-08-24**. Every
  unordered pair once; the mutable form hands out both halves writable, which is two `&mut` into the
  same storage and sound only because `i != j`. The borrow checker cannot see that, so the invariant
  lives in the iterator's structure — `j` starts at `i + 1` and only advances, and the ids come from
  one scan that yields each row once.

  **Miri is what made that claim worth anything.** The first version passed all six unit tests and
  Miri rejected it: the lifetime was extended with `transmute_copy` + `mem::forget`, and `forget`
  *moves*, a move is a retag, and the retag invalidated the copy's own tag. Going through
  `get_inner(&self)` — the route `iter_mut` already takes — needs no transmute and is clean under
  Miri. A test suite passing is not evidence that an `unsafe` block is sound.

  Measured in `iter_combinations`, which now runs both loops side by side in a `Phase::User(2500)`
  auditor: **66 pairs each, and a largest acceleration difference of exactly 0.000000000** over
  1500 frames. Not rounding-level — zero, meaning the two loops visit the same pairs in the same
  order and do the same float operations in the same order. What is gained is not lines but the
  intermediate: the hand-rolled path collects positions into a `Vec` and runs two index loops, and
  index arithmetic is where a pair gets counted twice.

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

**`assemble_material_bind_group` (`pub(crate)`) and `Layouts` (`pub(super)`).** ✅ **Done,
2026-08-23** — as `AssetManager::material()`, a builder rather than the raw 10-argument function.
Every one of the seven slots is optional and takes a neutral default, base colour and sampler
included, so `AssetManager::material().normal(&view).build(..)` is a complete material and the
automatic part stays automatic. `params` is nameable too, so a game can own the buffer the shader
reads its per-material constants from — that is the "no floor" half.

`Layouts` stayed `pub(super)` and did not need opening: `renderer.scene.texture_bind_group_layout`
was already public, which is what `build` takes.

Measured in `parallax_mapping`, now three slabs from one height field, each rendered alone
(`GIZMO_PX_SLAB`): flat 6 verts / shading σ 3.03, geometric 55 296 verts / σ 39.27, normal-mapped
**6 verts / σ 38.98**. The map reproduces **99.3 %** of the geometry's shading variation for
1/9216 of the vertices. Guarded by
`a_normal_map_bound_through_the_builder_changes_the_shading`, verified red when the `.normal()`
call is dropped.

*(the plan as written)* The material bind
group has seven entries, four of them detail maps — normal, MR, emissive, AO — filled with
defaults. A game outside `gizmo-renderer` cannot supply any of them; the only route is the glTF
loader. `parallax_mapping` prices the workaround at **6 verts → 55 296** for 2.87× the shading
variation, because geometry is the only surface detail a hand-built scene can get. Making the
assembler public behind a builder is a visibility change with a documented layout, not a design.

**`set_update` replacing rather than composing.** ✅ **Done, 2026-08-23.** `App::add_update_hook`
installs beside the existing hooks; `set_update` still clears the list, because "set" has to keep
meaning what its warning says. The single `Option` behind it became a `Vec`.

Writing the demo for it also **falsified a claim this repo had been making**. `set_update`'s docs,
`demo::simple_scene_update`'s docs and item 8 of `CAPABILITY_GAPS.md` §G all said the trap stopped
`GlobalTransform` from being propagated. Measured over 300 frames in `update_hooks`: it does not.
`TransformPropagateSystem` runs in three places and `set_update` swallows one, so what is lost is
the hook seeing a current transform *in its own frame* — a one-frame lag, whose signature is a
single 2.5-unit step on frame one as `Mat4::IDENTITY` is replaced. CPU physics genuinely does stop
(a falling body descends 0.000 units against ~6.9). All three notes are corrected. The lesson is
the ordinary one: a consequence that follows obviously from reading the code is still a guess until
something measures it.

**Effect on/off flags.** ✅ **Done, 2026-08-23**, and wider than the plan asked: `ssr`, `ssgi`,
`volumetric`, `ssao` and `decal` all carry `enabled: bool` now, matching the flag TAA and FXAA
already had. Off, the pass is skipped rather than drawing a neutral result — safe because each
state's textures are read only by its own apply pass, which composites with `LoadOp::Load`, so a
skipped effect leaves the frame as it found it. Resize still runs while off, so switching back on
costs no rebuild. Verified the strong way, not the plausible way: a golden test renders the same
scene with `enabled = false` and with `= None` and requires them **pixel-identical**, plus a
separate assertion that clearing the flag changes the frame at all — without which an all-equal
result would pass for the wrong reason. Measured in `ssr` over 420 frames: 4 switch-offs and
**3 switch-back-ons** in one run; with `= None` the second number is structurally 0.

*(historical)* `renderer.volumetric = None` destroys the state; turning it back on means
`VolumetricState::new(device, scene, deferred, w, h)`. Same shape in SSR. An `enabled: bool`
beside each is a field, not a redesign.

**Effect parameters.** ✅ **Done for volumetric, 2026-08-23.** Six shader literals became
`VolumetricParams` — one 32-byte uniform written each frame, so `vol.params.steps = 8.0` is the
whole gesture. The defaults *are* the literals, locked pixel-for-pixel by
`the_defaults_are_the_shader_literals_they_replaced`, so no existing scene changed.

Proving each field reaches the shader took more care than adding them. Two need a scene that can
show them: `bulb_scatter` scales a loop that runs only for point and spot lights (0 pixels without
a lamp, 3590 with one), and `shadow_bias` has to be pushed *negative*, because raising it in a
scene with no volumetric shadow asks for "even more lit", which is not a state (0 at +8, 2379 at
−40). And `steps` produced a confident zero for the wrong reason: scattering sums as
`Σ contribution × step_size` with `step_size = max_distance / steps`, so where the contribution is
constant the sum is step-count-independent by construction. Read without the 8-per-channel
threshold, 4 steps against 64 differ on 2755 pixels with a largest delta of 6 — it is a cost knob,
and the test now asserts that shape instead of a difference it will never have.

*(the plan as written)* **Effect parameters.** `VolumetricState` exposes textures, views, pipelines and dimensions — not
one shaping field. Phase 0.55, 16 steps, a 100 m cap, sun scatter 0.0015, bulb 0.0008, bias 0.16
are all shader literals. A `VolumetricParams` uniform is mechanical work with a measured payoff:
the effect already moves 19.55 % of pixels, max 134, and none of it is reachable.

### Architectural — real design, sequence them deliberately

**`MaterialType` → user materials.** ✅ **Done, 2026-08-23**, and the G-buffer question the plan
said to settle first turned out to settle itself. The four targets share a 32-byte
`max_color_attachment_bytes_per_sample` budget and already spend 28 (4 + 8 + 8 + 8). Four bytes
left is one more `Rgba8` and not one `Rgba16Float`, so a custom material bringing its own channel
would spend the last of a shared budget on one feature. It declares itself forward instead.

The shape also differs from the sketch below, deliberately. `MaterialType::Custom(MaterialId)` is a
variant rather than a `MaterialKind { BuiltIn, Custom }` wrapper, and the variant costs nothing the
wrapper would have bought: `Material::material_type` keeps its type, all 56 existing uses keep
compiling, and `routing.rs` stays exhaustive — which it proved by producing exactly one compile
error when the variant landed, in exactly the place the module exists to put it.

Two things had to open behind it, and both were walls the door would otherwise have faced:

* **`CustomMaterial::from_wgsl`.** Asking a game for a `wgpu::RenderPipeline` means asking it to
  know the bind-group order *per platform*, the vertex layout, the target format and the depth
  state — all engine contract, and each one wrong is a validation error naming none of it. The
  WGSL goes through the same composer the engine's own shaders use, so `#import` and
  `#{INSTANCE_GROUP}` work in a game's shader too.
* **`set_setup` now hands out `&mut Renderer`** (and with it `SceneBuilder::renderer`). It was
  `&Renderer`, which put the registry out of reach at scene-build time — a game could register a
  material only from inside the frame loop, one frame after spawning the entities using it. 24
  files carried an explicit signature; the rest inferred.

**There is no spare bind group**, which is the other measured constraint: native asks for 6 and
spends 5, the web asks for 4 and spends 4. A design giving custom materials their own group would
work on the desktop and fail to compile a shader in the browser. The room is inside group 1 —
four textures and a uniform buffer, whose meaning a custom shader is free to redefine.

Measured in `custom_material`: three spheres, PBR / `Unlit` / registered. Mean and standard
deviation separate them weakly (200.97/28.89, 216.64/12.16, 192.97/31.05); the banding does not.
FFT of each sphere's vertical brightness profile puts the custom material's dominant cycle at
**854.5 power, 13.3 % of its spectrum, against PBR's 99.8** — 8.5×. It also casts shadows (408
ground pixels against PBR's 374), which needed its own fix: a custom material routes as `unlit`,
and the shadow pass dropped everything `unlit && !baked_lit`, so `casts_shadows: true` would have
been a field that read true and did nothing.

An id with nothing behind it draws **nothing** rather than falling back to PBR. Verified red:
restoring the fallback leaks 5770/16384 pixels.

*(the plan as written)* The largest one, and `ENGINE.md` §4 has a stake in it: a
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

**One `global_uniform_buffer`.** ✅ **Done, 2026-08-23** — as `SceneView`, an explicit view the
caller selects rather than a per-pass slice. `SceneState` gained `views: Vec<SceneView>` and
`active_view: Option<usize>`; every pass in the engine now binds through `view_bind_group()` and
writes the camera through `view_uniform_buffer()`, which is what makes setting one cursor enough.
Each view owns its uniform buffer, so two writes have nowhere to collide: write both, record both
into one encoder, submit once. The cluster table and light index list are shared rather than
duplicated — they are built from the scene, not the camera.

Measured in `render_to_texture`, offscreen target, three modes: the trap reads 54.42/139.18/84.39
across three bands, per-camera-submit reads 88.80/101.40/63.07, and `SceneView` in one encoder
reads 89.70/99.02/86.14.

**The third band was the honest limit, and it closed on 2026-08-24.** Rather than guess why it did
not match, the demo measured it: with a shadowless light the two agreed to within 1.5 on all three
bands, so all of the difference was the shadow cascades — *derived* from the camera into their own
buffers and still shared. Those and the cluster table now live on `SceneView` too, and the lower
band reads 62.82 against per-camera-submit's 63.07.

The textures did not need copying: render passes run in recording order, so each view's shadow pass
redraws the shared cascade array right before that view's main pass reads it. Only the uniforms
could not be shared. A second view costs ~460 KB rather than 144 MB.

What is still shared is **temporal** state — TAA history and SSGI accumulation are per-renderer.
The guard test disables both, because with them on a single frame carries a 500-pixel noise floor
that buries the camera's own difference.

Guarded by `two_cameras_in_one_encoder_render_two_different_frames`, which pins both halves: two
passes without a view must be **pixel-identical** (the trap is still there and still reproducible),
and two passes with one must differ.

**A frame the user's pass can read.** ❌ **This entry was wrong, and checking it is what showed
that** (2026-08-23). It said the frame could not be sampled because the surface is
`RENDER_ATTACHMENT` (+`COPY_SRC`) with no `TEXTURE_BINDING`. True of the *surface* — and irrelevant,
because the post-process chain does not read the surface. It reads `PostProcessState::hdr_texture`,
which is built `RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC`, and whose texture, view and bind
group are all public. Nothing needed opening. I had looked at the wrong target and written the
conclusion down as a gap.

What is actually missing is one level further in: **a reduction**. None of the nine compute shaders
in the tree is a reduction or a histogram, so a game that wants a number out of the frame has to
pull the whole frame to the CPU.

Measured by writing exactly that, in `auto_exposure`: read `hdr_texture` back, sample every eighth
pixel, drive `Camera::exposure` toward `0.18 / measured`. The loop closes and lands where the
arithmetic says it should (bright station 0.3656 → exposure 0.492; dark 0.0534 → 3.370), and the
gap between the two stations' rendered brightness falls from 118.18 to 20.93 — 82 % of what eye
adaptation is for.

And the cost was the finding: **10.1–10.9 ms per sample**, all of it `map_async` + `poll(Wait)`
stalling the GPU. So the real item was "a reduction pass", not "a bindable target".

✅ **That landed 2026-08-24** — `LuminanceReduce`, two dispatches — and measuring it corrected the
estimate. Recording the reduction costs **2 µs**, as expected. But reducing on the GPU and reading
the answer back still costs a whole frame's worth of stall (0.785 ms mean frame against the CPU
readback's 0.825 — a 5 % difference), because both stall the same way and the size of the copy
barely matters. The win is in not reading it back: `gpu-only` lands inside the baseline's own noise
(0.185/0.199 against 0.168/0.200), so the honest statement is "too small to measure" rather than a
number. `result_buffer()` is the hatch that makes it usable; wiring it into `post_process.wgsl` is
the remaining half.

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

1. **The hatch rule and its test.** ✅ **Done, 2026-08-23** — deliberately *after* the hatches,
   so it points at real doors rather than intentions. Six pairs registered; three defects found on
   the first run.
2. **`Option<Res<T>>`, the composite-parameter macro, `iter_combinations`.** ✅ **All done,
   2026-08-23** — `Option<P>`, `system_param!`, and read-only pair iteration.
3. **`Phase::User`, the `enabled` flags, `add_update_hook`, `VolumetricParams`.** ✅ **All four
   done, 2026-08-23.** *(as planned:)* A batch of
   incidental opens. Each is small; together they close `schedule_internals`, the SSR/volumetric
   on-off destruction, the `set_update` trap and a whole row of section B.
4. **The material bind-group builder.** ✅ **Done, 2026-08-23** as `AssetManager::material()`.
   Unlocks normal/MR/emissive/AO maps for hand-built scenes,
   which is the difference between 6 and 55 296 vertices for the same surface detail.
5. **Per-pass uniforms.** ✅ **Done** as `SceneView` (2026-08-23), **completed 2026-08-24** with
   per-view cascades and cluster tables. Temporal state (TAA, SSGI) is still per-renderer, measured
   rather than assumed.
6. ~~**A bindable HDR target.**~~ **Withdrawn 2026-08-23 — the target was already bindable.**
   What the item was reaching for is a **compute reduction**: a user sensor is writable today and
   measured at 10 ms a sample, which is what makes it unusable rather than impossible.
7. **User materials.** ✅ **Done, 2026-08-23.** The G-buffer budget settled the question rather
   than blocking it: 28 of 32 bytes spent means forward-only, measured rather than argued.

Items 5, 6 and 7 are also the three that would let the two unwired subsystems
(`gizmo-renderer::visibility` and `::gi`, `CAPABILITY_GAPS.md` §F) be plugged in without a user
having to reimplement them.

## What this document is not

It is not a promise of 1.0. `ENGINE.md` §4 is explicit that the staged-1.0 plan is not being
pursued and the workspace keeps one `0.x` line. Nothing here changes that. What it changes is the
shape of the surface underneath, so that when a 1.0 *is* defensible, it is defensible because the
engine stopped needing to break — not because it stopped moving.
