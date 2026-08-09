//! A spatial index for **skipping** meshes before the exact visibility test runs.
//!
//! # What this is, and what it deliberately is not
//!
//! [`RenderAabbTree`] is an incremental bounding-volume hierarchy over world-space AABBs,
//! keyed by a small dense `u32`. [`RenderAabbTree::query_frustum`] returns every key whose
//! stored box is not wholly outside a frustum. That set is a conservative **superset** of the
//! truly-visible set, and it is meant to be used as exactly that: a filter that lets a caller
//! skip most of the scene, in front of an unchanged exact test.
//!
//! It **never decides visibility.** [`crate::classify_visibility_world`] remains the sole
//! arbiter, run on every surviving candidate. That split is what makes the whole thing
//! provably safe rather than merely tested: leaf boxes are stored *fattened*, so
//! `fat ⊇ tight`, so `Outside(fat) ⇒ Outside(tight)`, so the candidate set contains every
//! key the exact test would have kept. The draw list is bit-identical to the one the linear
//! path produces — by construction, not by coincidence. A false positive costs one exact
//! test; a false negative would be geometry silently vanishing, and the structure cannot
//! produce one.
//!
//! It is **not a parallel scene graph.** It stores boxes and keys. It has no notion of
//! parenting, no transforms, no components, no ownership of anything, and it must never grow
//! one — the moment two structures both claim to know where an entity is, they disagree.
//!
//! # When this is worth it — read this before wiring it in
//!
//! **A BVH cull is not automatically faster than testing everything.** The linear cull is a
//! sequential scan over a packed array, which the hardware prefetcher handles almost perfectly;
//! a BVH is pointer chasing through a megabyte of nodes plus per-frame maintenance. At small
//! and medium scene sizes the scan wins, and it wins by a lot.
//!
//! Measured with `benches/visibility_bench.rs` on the reference machine — a 2 km × 2 km scene,
//! one camera plus four real CSM cascades, ~3.6 % of objects moving each frame, comparing a
//! whole frame of cull work (refresh + traversal + exact test on the survivors) against a whole
//! frame of the linear path:
//!
//! | meshes | linear | indexed | |
//! |---|---|---|---|
//! | 4 096 | 38 µs | 108 µs | linear wins 2.8× |
//! | 8 192 | 77 µs | 170 µs | linear wins 2.2× |
//! | 32 768 | 731 µs | 510 µs | **index wins 1.4×** |
//! | 131 072 | 2.91 ms | 2.62 ms | index wins 1.1× |
//!
//! So the crossover for *CPU cull time alone* is somewhere between 8 k and 32 k renderables.
//! Where the per-frame budget goes at 8 192 meshes: traversing five frusta 45 µs, re-binning
//! 292 movers 74 µs, sorting and deduplicating the union 15 µs, the exact test on the 1 540
//! survivors 36 µs. The traversal is not the problem — maintenance is, and maintenance is what
//! an incremental tree charges for the early-out that makes it cheap in the first place.
//!
//! **The crossover is a band, not a number, and the band moves with the scene.** A second,
//! independently written harness (`tests/visibility_independent.rs`) on a clustered
//! city-block generator instead of a uniform scatter puts camera-only selection at ≈8 k,
//! camera + cascades at ≈12–16 k and the whole frame at ≈16–24 k. Holding the mesh count fixed
//! at 16 000 and sweeping only the world size swings the linear scan's cost per mesh by ±70 %,
//! which is enough on its own to move the crossover a binary order of magnitude. Quote the
//! band; measure your own scene.
//!
//! **The workload that disqualifies the index outright: everything moves every frame.** The
//! fat-box early-out is the entire performance case, and a scene of movers never takes it.
//! Refreshing every key when every key really moved costs **2.4 ms/frame at 8 k and 12.9 ms at
//! 32 k** — not "slower than the linear cull", but past a 60 Hz frame budget by itself. A
//! particle field, a crowd, a cloth mesh per instance, anything vertex-animated on the CPU:
//! do not index it. Index the static world and test the movers linearly; the two sets can be
//! culled by different means in the same frame.
//!
//! The plane mask does earn its keep: the same traversal forced to test all six planes at every
//! level costs 71.0 µs against 48.7 µs masked, so `Frustum::test_aabb_masked` is worth ~31 % of
//! the walk. `cull/masked_vs_unmasked` measures exactly that, and
//! [`RenderAabbTree::query_frustum_full_mask`] is kept public so the comparison stays runnable.
//!
//! That is a statement about **CPU cull time**, and it is not the only reason to want this. The
//! other one is usually bigger: a renderer that submits every mesh pays for every mesh on the
//! GPU too — instance-buffer upload, vertex fetch, overdraw, draw calls — and *that* cost the
//! index removes at any scale. In the measured scene it takes 8 192 submitted instances down to
//! 1 540. If your loop currently culls nothing, that is the win, and the CPU table above is not
//! the number to read.
//!
//! Practical guidance:
//!
//! * **You cull nothing today** → use this. The submission saving dwarfs the cull cost.
//! * **You already cull linearly and have under ~16 k renderables** → keep your loop. Make sure
//!   you transform each box once, not once per frustum
//!   ([`classify_visibility_world`](crate::classify_visibility_world)); that alone was a 3.1×
//!   improvement to this engine's own cull, for fifteen lines and no new data structure.
//! * **Over the crossover band (~16–32 k, see above), or an expensive per-object test** → use
//!   this.
//! * **Most of your renderables move every frame** → do not use this at any size. The
//!   early-out is the whole performance case and you never take it.
//! * Measure on your own scene before believing any of the above. The numbers move with mesh
//!   count, spatial coherence, how much of the scene is on screen, and how much of it moves.
//!
//! # Cells vs. a BVH
//!
//! A BVH answers "what might be on screen". It does not answer "which region is this in",
//! "what should stream in next" or "where do I hang a designer-authored PVS" — it has no
//! stable region identity to hang any of those on. If you already have a cell/region grid,
//! keep it: the two are complementary, and this is not a replacement for it.
//!
//! # Relationship to `DynamicAabbTree`
//!
//! The physics broadphase has a very similar structure at
//! `crates/gizmo-physics-core/src/broadphase/aabb_tree.rs`. This is a deliberate second
//! implementation, not an oversight, and it is **not** a copy that drifted:
//! `gizmo-renderer` does not depend on `gizmo-physics-core` (and must not — that would put
//! the physics crates in every graphics build and undo their independent packageability),
//! and the physics crate's public surface is mid-1.0 freeze.
//!
//! Three divergences are deliberate and are the technical reason the duplication is worth
//! paying for:
//!
//! 1. **No key map.** The physics tree keys on `FxHashMap<u32, usize>` because
//!    `BodyHandle::INVALID` is `u32::MAX` and is a legal key, so a dense slot array would
//!    demand a 16 GB allocation. A render key is an entity id, which `gizmo-core` itself
//!    already uses to index a plain `Vec`. This tree does the same — direct indexing, no
//!    hashing, and no `rustc-hash` dependency anywhere near `gizmo-renderer`.
//! 2. **Frustum queries with plane masking, no pair or ray queries.** `query_pairs` and
//!    `query_ray` are broadphase concerns and are simply absent here;
//!    [`RenderAabbTree::query_frustum`] is the whole point and is what finally gives
//!    `Frustum::test_aabb_masked` a caller.
//! 3. **[`RenderAabbTree::validate`] checks geometry, not just structure.** The physics one
//!    documents that it "does not verify that a parent's box actually encloses its
//!    children's". Behind a pair query a refit bug is a missed contact; behind a cull query
//!    it is geometry that disappears, so the containment invariant is asserted here.
//!
//! # Caveats worth knowing before you index something
//!
//! * **Fail open. A key the index does not hold is not a culled key.** This is the one that
//!   deletes geometry from the screen, so it comes first. The guard must be
//!
//!   ```text
//!   if index.contains(key) && !candidates.contains(&key) { skip }
//!   ```
//!
//!   and never "iterate the candidate list". The two differ for every renderable the index
//!   never received: one spawned since the last maintenance pass, one whose `Mesh::bounds` is
//!   [`Aabb::empty`](gizmo_math::Aabb::empty) so [`RenderAabbTree::insert`] refused it, one
//!   deliberately excluded (camera-locked, below), one your maintenance simply missed. Under
//!   the candidate-list loop every one of those is invisible forever, with no panic, no log
//!   line and no failing test. Under the guard above each costs one exact test.
//!
//!   [`insert`](RenderAabbTree::insert) cannot help you here: it returns `false` both for
//!   "already inside its fat box, nothing to do" (the hot path, most keys, most frames) and for
//!   "refused, not stored". Use [`contains`](RenderAabbTree::contains) if you need to know.
//! * **Camera-locked materials must not be indexed at all.** A backdrop's drawn transform
//!   depends on the camera position (`crate::backdrop::camera_locked_model`), so its world
//!   box moves every frame and indexing it is both pointless and a staleness trap. Skip it
//!   in maintenance and let it through the guard unconditionally.
//! * **Skinned meshes are indexed at their rest pose.** `Mesh::bounds` is a static field
//!   describing the unanimated mesh; an animation that swings a limb outside it is not
//!   reflected. This is exactly what the linear path already does — indexing changes
//!   nothing about it — but a fat margin does not fix it either, so do not assume it does.
//! * **`Mesh::bounds` can change without the transform moving** (a handle swap, a hot
//!   reload). Whatever drives maintenance has to watch the mesh as well as the transform.
//! * **Eviction must be re-derived, not subscribed to.** Prefer
//!   [`retain`](RenderAabbTree::retain) over a despawn hook: `retain` re-asks a liveness
//!   predicate about every key it holds and therefore cannot desync, where a missed or
//!   double-delivered despawn event leaves a dead entity's box in the tree forever. A dead
//!   key never produces invisible geometry — nothing consults the tree about an entity that
//!   no longer exists — but it does accumulate: leaves that never leave, node counts that only
//!   grow, and candidate sets padded with boxes belonging to nothing.

#[cfg(test)]
mod differential;
mod set;
mod tree;

pub use set::VisibleSet;
pub use tree::{RenderAabbTree, NO_KEY};
