# Gizmo — Latent-Bug Fix Plan

> Source: an adversarial bug-hunt run over the modules that Phases 0–5 of
> `ARCHITECTURE_ROADMAP.md` split out of god-files (2026-07-13). 9 finder agents →
> each candidate confirmed only if ≥2 of 3 independent skeptics agreed (math /
> reachability / intent lenses). **6 confirmed** (5 distinct; one found from both the
> CPU-pack and shader-unpack ends), **4 false positives rejected** (listed at the end so
> they aren't re-chased).
>
> These are **pre-existing latent defects** surfaced by the modularization — none were
> introduced by the refactor, none are fixed yet. Each fix below is a **behaviour change**,
> so: one focused commit per fix, and verify with the noted test/demo before moving on.
> `cargo` package names: `gizmo-renderer`, `gizmo-engine` (crate dir `crates/gizmo`),
> `gizmo-core`, `gizmo-studio`. (Note: `-p gizmo` is wrong — the entry crate is
> `gizmo-engine`; and `| tail` masks cargo's exit code, so check exit status explicitly.)

Recommended order: **B → A → C → E → D** (trivial+high-value first; structural last).

---

## A. Tangent transformed by the inverse-transpose (normal) matrix — `HIGH`
**File:** `crates/gizmo-renderer/src/shaders/gbuffer.wgsl:129` (vertex shader)

**Symptom:** Normal-mapped meshes under **non-uniform scale or shear** get mis-aligned
TBN → normal-map X/Y axes rotated, wrong perturbed normal, skewed anisotropy/clear-coat.
Invisible for rigid/uniform transforms and for axis-aligned tangents (which is why it hid).
*Distinct from the 2026-07-13 TBN parallel-tangent fix (`0a5a84d`) — same code, deeper bug.*

**Root cause:** Line 129 builds the world tangent with `normal_mat` (= `inverse_transpose_3x3(model)`),
but a tangent is a **surface direction** and must be carried by the **plain model 3×3**, not the
inverse-transpose. The shader's own comment (lines 120-122) says exactly this, and the skinning
half (line 128, `skin_mat * tangent`) already does it right — only the model half is wrong.

**Fix:**
```wgsl
// line 128 stays:
let skinned_tangent = skin_mat * vec4<f32>(input.tangent.xyz, 0.0);
// line 129 — use the plain model 3x3, NOT normal_mat:
let model_mat3 = mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz);
out.world_tangent = vec4<f32>(model_mat3 * skinned_tangent.xyz, input.tangent.w);
```

**Verify:** `cargo test -p gizmo-renderer --lib core_shaders_compile` (WGSL still valid) +
`cargo test -p gizmo-engine --lib golden_render_tests`. The golden cube is uniform-scale so
it won't *catch* the difference — ideally add a headless case with a non-uniform-scaled,
normal-mapped mesh (e.g. scale (2,1,1)) and assert the lit result changes vs the buggy build,
or eyeball a stretched normal-mapped object in a demo.

**Effort:** ~1 line. **Risk:** low (matches documented intent; uniform-scale unchanged).

---

## B. PBR param packing overflows its field at the legal endpoint 1.0 — `HIGH`
**Files:** pack `crates/gizmo/src/systems/render/batching.rs:193`;
unpack `crates/gizmo-renderer/src/shaders/gbuffer.wgsl:222-225` (leave unpack unchanged).

**Symptom:** A material with `clear_coat = 1.0` renders with **zero** clear coat + a phantom
`subsurface ≈ 0.01`. Symmetrically `anisotropy = 1.0` reads back as 0 and leaks into clear_coat.
Both are legal clamped endpoints (`with_clear_coat`/`with_anisotropy` clamp to inclusive `[0,1]`).

**Root cause:** Packing is `floor(aniso*1000) + 1000*floor(cc*1000) + 1e6*floor(ss*100)`. The
aniso and clear_coat fields are 3 decimal digits (0..999), but `floor(1.0*1000) = 1000` — one
digit too many — carries into the next field.

**Fix (pack-side only, no shader change):** clamp the two 3-digit fields to 999.
```rust
let packed_params = (
    ($mat.anisotropy * 1000.0).floor().min(999.0)
    + 1000.0 * ($mat.clear_coat * 1000.0).floor().min(999.0)
    + 1000000.0 * ($mat.subsurface * 100.0).floor()   // top field — no field above it, leave as-is
) as f32;
```
`1.0` now packs as `999` → unpacks to `0.999` (imperceptible) instead of corrupting neighbours.

**⚠ Deeper caveat (worth a separate follow-up, not required for this fix):** the whole scheme
is f32-fragile — `packed_params as f32` loses integer precision past 2^24 ≈ 16.7M, so once
`subsurface ≥ ~0.17` (→ `floor(ss*100)*1e6 ≥ 17e6`) the low digits (aniso/cc) already round.
The robust long-term fix is dedicated `InstanceRaw` fields instead of decimal-packing one f32.

**Verify:** `cargo test -p gizmo-engine --lib golden_render_tests`; manually, a `clear_coat(1.0)`
material must show clear coat (not lost). Also unit-testable purely on the CPU: assert the packed
value for cc=1.0 unpacks (per the gbuffer formula) to a clear_coat ≥ 0.99 and subsurface == 0.

**Effort:** ~1 line. **Risk:** low.

---

## C. `Query::get`/`get_entity`/`contains` ignore archetype-level filters — `HIGH`
**File:** `crates/gizmo-core/src/query/access.rs:70-84` (`get_inner`)

**Symptom:** `query.get(e)` / `contains(e)` **disagree with `iter()`**. For `Query<(&Position,
With<Health>)>` and an entity `e` that has `Position` but not `Health`: `iter()` correctly skips
`e`, but `get(e)` returns `Some` and `contains(e)` returns `true`. Soundness-adjacent (callers
assume get/iter see the same set).

**Root cause:** `iter()` only visits archetypes in `self.matching_archetypes` (built with the
`With`/`Without` archetype-level predicate). `get_inner` instead indexes the entity's **own**
archetype directly and validates only via `fetch_raw` + `filter_row`; for table-storage
`With`/`Without`, presence is checked at the *archetype* level, not by `filter_row`, so it passes.

**Fix:** make `get_inner` honour the same archetype set as `iter()`. After computing `loc`:
```rust
// inside get_inner, before/after building `arch`:
if !self.matching_archetypes.contains(&(loc.archetype_id as usize)) {
    return None;
}
```
Confirm the element type of `matching_archetypes` matches `loc.archetype_id` (adjust the cast).
Perf: `contains` is O(#matching archetypes); if it shows up hot, keep `matching_archetypes`
sorted + `binary_search`, or add a `HashSet`. Correctness first. `par_inner` already iterates
`matching_archetypes`, so it's fine — `get_inner` is the only outlier; grep for other direct
`archetypes[loc.archetype_id]` accesses to be sure.

**Verify:** add a unit test in `query/tests.rs`: Position-only entity, `Query<(&Position,
With<Health>)>` ⇒ `iter().count()==0` **and** `get(id).is_none()` **and** `!contains(id)`
(the last two currently fail). Run `cargo test -p gizmo-core --lib` **and**
`cargo test -p gizmo-core --doc` (the dual-Mut soundness doctests must still pass).

**Effort:** small. **Risk:** medium — query is soundness-critical; add the test first (red→green).

---

## D. Off-screen shadow casters starve visible geometry on instance-buffer overflow — `MEDIUM`
**File:** `crates/gizmo/src/systems/render/batching.rs` (the flatten loop, ~lines 606-654,
around `first_instance`/`camera_count`/`extend(instances)`/`extend(shadow_instances)`)

**Symptom:** When total instances exceed `renderer.scene.instance_capacity` (8192), the tail
truncation (`&cache.instances[..max_instances]`) can drop **camera-visible** geometry of later
batches, because each batch is packed `[camera instances][shadow-only casters]` contiguously and
an early batch's shadow-only casters consume slots first. Which mesh vanishes is nondeterministic
(HashMap batch order). Edge case (only above 8192 instances).

**Root cause:** batch-major interleaving mixes shadow-only casters into the linear budget ahead
of other batches' camera instances.

**Fix (proper):** two-region layout — first pass appends **all** batches' camera-visible instances
(record each batch's `first_instance` + `camera_count`), second pass appends **all** shadow-only
casters. Then truncation drops shadow casters (only lose some off-screen shadows) before ever
dropping on-screen geometry. This makes a batch's camera and shadow ranges non-contiguous, so
`DrawItem` must gain a separate `shadow_first_instance` (+ shadow count), and the shadow-pass
recorders in `systems/render/passes/shadow.rs` must draw that separate range.
**Interim mitigation** if the full change is too big now: stop appending shadow-only casters once
camera instances approach capacity, and/or `log`/warn when truncation drops camera-visible
instances (currently silent) so it's at least diagnosable.

**Verify:** hard to unit-test (needs >8192 instances). Reason through the new ranges; a stress
scene with >8192 instanced objects + off-screen casters. Confirm `golden_render_tests` still pass
for the normal (sub-capacity) path.

**Effort:** medium-high (touches DrawItem + shadow recorder). **Risk:** medium. Lowest priority.

---

## E. glTF `AlphaMode::Mask` (alpha cutout) rendered as alpha-blend — `MEDIUM`
**File:** `crates/gizmo-renderer/src/asset/loaders/material.rs:274` (+ gbuffer shader for the discard)

**Symptom:** A glTF material with `alphaMode="MASK"` + `alphaCutoff` (fences, foliage cards,
decal atlases) is routed to the transparent/blended pass and `alphaCutoff` is never read →
soft translucent fringes and depth-sorting artifacts instead of a crisp per-texel cutout.

**Root cause:** `mat.is_transparent = material.alpha_mode() != Opaque || ...` treats `Mask` like
`Blend`. Mask should be **opaque geometry with a hard `discard`** at `alphaCutoff`.

**Fix — two parts (do both):**
1. **Routing** (material.rs:274) — only `Blend` is transparent:
   ```rust
   mat.is_transparent = material.alpha_mode() == gltf::material::AlphaMode::Blend
       || alpha < 0.99 || is_glass;
   ```
2. **Cutout discard** (the actual cutout):
   - Add `alpha_cutoff: f32` to the `Material` component (default `0.0` = no cutout). In the
     loader, for `AlphaMode::Mask` set it from `material.alpha_cutoff().unwrap_or(0.5)`.
   - Plumb it to the shader (a `MaterialParams` field / uniform, or an `InstanceRaw` slot).
   - In `gbuffer.wgsl` `fs_main`, after the albedo sample:
     `if (cutoff > 0.0 && albedo.a < cutoff) { discard; }`.
   Part 1 alone stops the translucent fringes but leaves cutout texels opaque (fence holes solid);
   part 2 is required for a correct cutout.

**Verify:** load a glTF with `alphaMode=MASK` (foliage/fence) in a demo — edges must be crisp
cutout, not translucent and not solid. `core_shaders_compile` + `golden_render_tests` for regressions.

**Effort:** medium (new Material field + shader plumbing + discard). **Risk:** low-medium.

---

## Rejected candidates (verification filtered these — do NOT re-chase)
- `deferred_lighting.wgsl` — "f16 position.w channel destroys anisotropy when subsurface packed" — **1/3**, refuted.
- `gbuffer.wgsl` — "bitangent collapses to zero when tangent.w == 0" — **0/3** (handedness `select(-1,1, w>=0)` never yields 0).
- `vehicle/dynamics.rs` — "point velocity about transform.position not COM" — **0/3** (consistent with how forces treat linear velocity).
- `narrowphase/contacts.rs` — "incident-corner penetration inflated for full-pierce through a thin box" — **0/3**.

---
*Bug-hunt workflow run id `wf_e35b70c1-2e7`. Full per-agent reasoning is in that run's
`journal.jsonl`. Memory pointer: `gizmo-latent-bugs-bughunt-2026-07-13`.*
