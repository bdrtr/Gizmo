//! Tests for [`RenderAabbTree`].
//!
//! The shape is taken from `crates/gizmo-physics-core/tests/proptest_broadphase.rs`, whose two
//! invariants ("margin 0 ⇒ exact", "fat margin ⇒ superset") are exactly the two that matter
//! here. The asymmetry runs through all of them: a **false positive** costs one exact test, a
//! **false negative** is geometry that silently stops being drawn. Every test that can only
//! check one direction checks that one.

use super::*;
use gizmo_math::{Mat4, Quat, Vec3};

// ── helpers ─────────────────────────────────────────────────────────────────

fn boxx(cx: f32, cy: f32, cz: f32, h: f32) -> Aabb {
    Aabb::new(
        Vec3::new(cx - h, cy - h, cz - h),
        Vec3::new(cx + h, cy + h, cz + h),
    )
}

/// A perspective camera at `eye` looking towards `at`.
fn cam(eye: Vec3, at: Vec3, far: f32) -> Frustum {
    let view = Mat4::look_at_rh(eye, at, Vec3::Y);
    let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_3, 16.0 / 9.0, 0.1, far);
    Frustum::from_matrix(&(proj * view))
}

/// Deterministic LCG — no dev-dep, same scene every run, and reproducible from the seed a
/// failure prints.
struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn f32_in(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.next_u32() as f32 / u32::MAX as f32) * (hi - lo)
    }
    fn usize_below(&mut self, n: usize) -> usize {
        (self.next_u32() as usize) % n
    }
}

/// Every key whose TIGHT box the frustum does not reject — the ground truth the tree's answer
/// is measured against.
fn brute_force(tight: &[(u32, Aabb)], f: &Frustum) -> Vec<u32> {
    let mut v: Vec<u32> = tight
        .iter()
        .filter(|(_, a)| f.intersects_aabb(*a))
        .map(|(k, _)| *k)
        .collect();
    v.sort_unstable();
    v
}

fn sorted(mut v: Vec<u32>) -> Vec<u32> {
    v.sort_unstable();
    v
}

// ── T1–T7: structure ────────────────────────────────────────────────────────

/// T1 — every query on an empty tree is a no-op, not a panic. The batching guard runs before
/// anything has been indexed on the very first frame.
#[test]
fn empty_tree_queries_return_nothing() {
    let t = RenderAabbTree::new();
    let f = cam(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), 100.0);
    let mut out = vec![];
    t.query_frustum(&f, &mut out);
    assert!(out.is_empty());
    t.query_frustum_full_mask(&f, &mut out);
    assert!(out.is_empty());
    t.query_aabb(&boxx(0.0, 0.0, 0.0, 1000.0), &mut out);
    assert!(out.is_empty());
    t.query_frusta(&[f], &mut out);
    assert!(out.is_empty());
    assert!(t.is_empty());
    assert_eq!(t.len(), 0);
    assert_eq!(t.height(), 0);
    #[cfg(debug_assertions)]
    t.validate();
}

/// T2 — the smoke case.
#[test]
fn one_leaf_inside_is_returned_outside_is_not() {
    let f = cam(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, 100.0);
    let mut t = RenderAabbTree::new();
    assert!(t.insert(0, boxx(0.0, 0.0, 0.0, 1.0)));
    let mut out = vec![];
    t.query_frustum(&f, &mut out);
    assert_eq!(out, vec![0]);

    let mut t2 = RenderAabbTree::new();
    assert!(t2.insert(0, boxx(0.0, 0.0, 500.0, 1.0))); // behind the camera
    out.clear();
    t2.query_frustum(&f, &mut out);
    assert!(out.is_empty());
}

/// T3 — the early-out the entire performance case rests on: a key that moved less than the fat
/// margin must not re-bin.
///
/// If this ever regresses the feature silently degrades into a per-frame rebuild, which is
/// slower than the linear cull it replaced and produces identical output — so nothing else
/// would catch it.
#[test]
fn insert_inside_the_fat_box_is_a_no_op() {
    let mut t = RenderAabbTree::with_fat_margin(1.0);
    for i in 0..64u32 {
        t.insert(i, boxx(i as f32 * 5.0, 0.0, 0.0, 1.0));
    }
    let h = t.height();
    let n = t.len();
    let stored = t.leaf_aabb(7).unwrap();

    // A nudge well inside the 1 m margin.
    assert!(
        !t.insert(7, boxx(0.4, 0.2, -0.3, 1.0).translated(35.0)),
        "a move inside the fat box must not re-bin"
    );
    assert_eq!(t.height(), h);
    assert_eq!(t.len(), n);
    assert_eq!(
        t.leaf_aabb(7).unwrap(),
        stored,
        "the stored box must be untouched, not merely equivalent"
    );

    // Re-inserting the exact same box early-outs too: containment is inclusive.
    assert!(!t.insert(7, boxx(35.0, 0.0, 0.0, 1.0)));
    #[cfg(debug_assertions)]
    t.validate();
}

/// T4 — a key that escapes its fat box is re-binned, and is then found at the new place and
/// not the old one.
#[test]
fn insert_outside_the_fat_box_rebins() {
    let mut t = RenderAabbTree::with_fat_margin(0.5);
    for i in 0..32u32 {
        t.insert(i, boxx(i as f32 * 4.0, 0.0, 0.0, 1.0));
    }
    assert!(t.insert(3, boxx(500.0, 0.0, 0.0, 1.0)), "a big move must re-bin");

    let mut out = vec![];
    t.query_aabb(&boxx(500.0, 0.0, 0.0, 2.0), &mut out);
    assert!(out.contains(&3), "not found at the new place");
    out.clear();
    t.query_aabb(&boxx(12.0, 0.0, 0.0, 1.0), &mut out);
    assert!(!out.contains(&3), "still found at the old place");
    assert_eq!(t.len(), 32, "re-binning must not change the key count");
    #[cfg(debug_assertions)]
    t.validate();
}

/// T5 — removal really removes, and the freed node slot is reused rather than leaked.
#[test]
fn remove_then_query_does_not_return_the_key() {
    let mut t = RenderAabbTree::new();
    for i in 0..16u32 {
        t.insert(i, boxx(i as f32 * 3.0, 0.0, 0.0, 1.0));
    }
    let nodes_before = t.nodes.len();
    assert!(t.remove(5));
    assert!(!t.remove(5), "removing twice must report the second as a no-op");
    assert!(!t.contains(5));
    assert_eq!(t.len(), 15);

    let mut out = vec![];
    t.query_aabb(&boxx(15.0, 0.0, 0.0, 2.0), &mut out);
    assert!(!out.contains(&5));

    t.insert(99, boxx(-50.0, 0.0, 0.0, 1.0));
    assert_eq!(
        t.nodes.len(),
        nodes_before,
        "the freed slots must be reused, not grown past"
    );
    #[cfg(debug_assertions)]
    t.validate();
}

/// T6 — `retain` evicts exactly the rejected keys and reports how many.
#[test]
fn retain_evicts_exactly_the_rejected_keys_and_reports_the_count() {
    let mut t = RenderAabbTree::new();
    for i in 0..40u32 {
        t.insert(i, boxx(i as f32 * 2.0, 0.0, 0.0, 0.5));
    }
    let dropped = t.retain(|k| k % 3 != 0);
    assert_eq!(dropped, 14, "0,3,..,39 is 14 keys");
    assert_eq!(t.len(), 26);
    for k in 0..40u32 {
        assert_eq!(t.contains(k), k % 3 != 0, "key {k}");
    }
    assert_eq!(t.retain(|_| true), 0);
    assert_eq!(t.retain(|_| false), 26);
    assert!(t.is_empty());
    #[cfg(debug_assertions)]
    t.validate();
}

/// T7 — `validate` really checks the geometric invariant.
///
/// The physics broadphase's `validate` documents that it does *not*, and a claim in a doc
/// comment is worth nothing behind a cull query: a parent box that fails to enclose its
/// children is precisely how a whole subtree reports `Outside` while holding visible geometry.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "does not enclose child")]
fn validate_catches_a_parent_that_does_not_enclose_its_child() {
    let mut t = RenderAabbTree::new();
    for i in 0..8u32 {
        t.insert(i, boxx(i as f32 * 3.0, 0.0, 0.0, 1.0));
    }
    t.validate();
    // Shove one leaf far outside its ancestors' bounds without refitting them.
    t.corrupt_node_aabb(4, boxx(10_000.0, 10_000.0, 10_000.0, 1.0));
    t.validate();
}

// ── T8–T12: queries ─────────────────────────────────────────────────────────

/// Build a random scene and return (tree, tight boxes).
fn random_scene(seed: u64, n: usize, margin: f32) -> (RenderAabbTree, Vec<(u32, Aabb)>) {
    let mut rng = Lcg::new(seed);
    let mut t = RenderAabbTree::with_fat_margin(margin);
    let mut tight = Vec::with_capacity(n);
    for k in 0..n as u32 {
        let h = rng.f32_in(0.2, 8.0);
        let a = boxx(
            rng.f32_in(-300.0, 300.0),
            rng.f32_in(-40.0, 40.0),
            rng.f32_in(-300.0, 300.0),
            h,
        );
        t.insert(k, a);
        tight.push((k, a));
    }
    (t, tight)
}

fn random_frustum(rng: &mut Lcg) -> Frustum {
    let eye = Vec3::new(
        rng.f32_in(-350.0, 350.0),
        rng.f32_in(-50.0, 50.0),
        rng.f32_in(-350.0, 350.0),
    );
    let dir = Vec3::new(
        rng.f32_in(-1.0, 1.0),
        rng.f32_in(-1.0, 1.0),
        rng.f32_in(-1.0, 1.0),
    );
    let dir = if dir.length_squared() < 1e-4 { Vec3::NEG_Z } else { dir.normalize() };
    cam(eye, eye + dir, rng.f32_in(20.0, 800.0))
}

/// T8 — the criterion-4 test: the masked walk and the unmasked walk must agree, exactly.
///
/// `Frustum::test_aabb_masked`'s sharp edge is that `mask == 0` returns `Inside`
/// unconditionally — for *anything*, including a box behind the camera. `frustum.rs` pins that
/// on the primitive. Nothing anywhere pinned it on a **traversal**, and a traversal is where
/// that edge becomes dangerous: propagate a mask one level too eagerly and whole subtrees get
/// accepted or dropped on a plane nobody tested.
#[test]
fn the_masked_walk_agrees_with_the_unmasked_walk() {
    for seed in 0..24u64 {
        let (t, _) = random_scene(seed, 200, 0.5);
        let mut rng = Lcg::new(seed ^ 0xabcd);
        for _ in 0..8 {
            let f = random_frustum(&mut rng);
            let mut masked = vec![];
            let mut plain = vec![];
            t.query_frustum(&f, &mut masked);
            t.query_frustum_full_mask(&f, &mut plain);
            assert_eq!(
                sorted(masked),
                sorted(plain),
                "seed {seed}: plane masking changed the answer, not just the work"
            );
        }
    }
}

/// T9 — **the only direction that can produce invisible geometry.** For every live key, if the
/// frustum does not reject its TIGHT box then the query must return it.
///
/// The converse is explicitly allowed to fail: the tree answers against fattened boxes and is
/// meant to over-report.
#[test]
fn query_frustum_never_misses_a_visible_key() {
    for seed in 0..24u64 {
        let (t, tight) = random_scene(seed, 300, 1.5);
        let mut rng = Lcg::new(seed ^ 0x1234);
        for _ in 0..8 {
            let f = random_frustum(&mut rng);
            let mut got = vec![];
            t.query_frustum(&f, &mut got);
            let got = sorted(got);
            for k in brute_force(&tight, &f) {
                assert!(
                    got.binary_search(&k).is_ok(),
                    "seed {seed}: key {k} intersects the frustum but the index dropped it"
                );
            }
        }
    }
}

/// T10 — the test that earns the duplication. Randomised insert / move / remove / retain
/// sequences against a brute-force mirror, checking after every single operation that
///
/// * the query is a superset of the truth (never a false negative),
/// * every returned key is live and appears exactly once,
/// * and the tree's own invariants still hold.
#[test]
fn random_ops_then_query_matches_brute_force() {
    for seed in 0..16u64 {
        let mut rng = Lcg::new(seed);
        let margin = [0.0f32, 0.25, 2.0][seed as usize % 3];
        let mut t = RenderAabbTree::with_fat_margin(margin);
        // key -> its current tight box, or None if not present.
        let mut mirror: Vec<Option<Aabb>> = vec![None; 120];

        for step in 0..400 {
            let k = rng.usize_below(mirror.len()) as u32;
            match rng.usize_below(10) {
                0..=5 => {
                    let a = boxx(
                        rng.f32_in(-200.0, 200.0),
                        rng.f32_in(-30.0, 30.0),
                        rng.f32_in(-200.0, 200.0),
                        rng.f32_in(0.2, 6.0),
                    );
                    t.insert(k, a);
                    mirror[k as usize] = Some(a);
                }
                6..=7 => {
                    // A small nudge — the fat-box early-out path. The MIRROR still moves even
                    // when the tree does not re-bin, which is exactly the case where the
                    // stored box is stale-but-conservative and the superset claim is doing
                    // real work.
                    if let Some(a) = mirror[k as usize] {
                        let d = Vec3::new(
                            rng.f32_in(-0.3, 0.3),
                            rng.f32_in(-0.3, 0.3),
                            rng.f32_in(-0.3, 0.3),
                        );
                        let moved = Aabb::new(a.min + gizmo_math::Vec3A::from(d), a.max + gizmo_math::Vec3A::from(d));
                        t.insert(k, moved);
                        mirror[k as usize] = Some(moved);
                    }
                }
                8 => {
                    t.remove(k);
                    mirror[k as usize] = None;
                }
                _ => {
                    let cut = rng.next_u32() % 4;
                    t.retain(|key| key % 4 != cut);
                    for (i, slot) in mirror.iter_mut().enumerate() {
                        if (i as u32) % 4 == cut {
                            *slot = None;
                        }
                    }
                }
            }

            #[cfg(debug_assertions)]
            t.validate();

            let live: Vec<(u32, Aabb)> = mirror
                .iter()
                .enumerate()
                .filter_map(|(i, a)| a.map(|a| (i as u32, a)))
                .collect();
            assert_eq!(t.len(), live.len(), "seed {seed} step {step}: key count drifted");

            let f = random_frustum(&mut rng);
            let mut got = vec![];
            t.query_frustum(&f, &mut got);

            // (b) every returned key is live and returned once.
            let mut seen = std::collections::HashSet::new();
            for &k in &got {
                assert!(seen.insert(k), "seed {seed} step {step}: key {k} returned twice");
                assert!(
                    mirror[k as usize].is_some(),
                    "seed {seed} step {step}: dead key {k} returned"
                );
            }
            // (a) superset of the truth.
            let got = sorted(got);
            for k in brute_force(&live, &f) {
                assert!(
                    got.binary_search(&k).is_ok(),
                    "seed {seed} step {step}: visible key {k} missing from the query"
                );
            }
        }
    }
}

/// T11 — at margin 0 the superset collapses to equality.
///
/// This is what makes every other over-inclusion attributable: if the answer is exact with no
/// margin, then any extra key at a non-zero margin came from the margin, and any extra key at
/// zero margin is a bug in the traversal.
#[test]
fn fat_margin_zero_makes_the_query_exact() {
    for seed in 0..16u64 {
        let (t, tight) = random_scene(seed, 250, 0.0);
        assert_eq!(t.fat_margin(), 0.0);
        let mut rng = Lcg::new(seed ^ 0xfeed);
        for _ in 0..8 {
            let f = random_frustum(&mut rng);
            let mut got = vec![];
            t.query_frustum(&f, &mut got);
            assert_eq!(
                sorted(got),
                brute_force(&tight, &f),
                "seed {seed}: with no margin the index must be exact"
            );
        }
    }
}

/// T12 — the `Inside` / `collect_subtree` fast path emits every key exactly once.
///
/// A duplicate here would be invisible in the draw list (the guard is a membership test) but
/// would double-count in `VisibleSet::len` and in any buffer sized off it.
#[test]
fn a_frustum_containing_everything_returns_every_key_exactly_once() {
    let mut t = RenderAabbTree::new();
    for i in 0..500u32 {
        t.insert(i, boxx(i as f32 * 0.1 - 25.0, 0.0, -30.0, 0.2));
    }
    // A camera far enough back that the whole cluster is strictly inside every plane.
    let f = cam(Vec3::new(0.0, 0.0, 400.0), Vec3::new(0.0, 0.0, -30.0), 5000.0);
    let mut out = vec![];
    t.query_frustum(&f, &mut out);
    assert_eq!(out.len(), 500, "expected every key exactly once");
    assert_eq!(sorted(out), (0..500u32).collect::<Vec<_>>());
}

// ── degenerate cases ────────────────────────────────────────────────────────

/// A box sitting exactly on a frustum plane, a box larger than the whole frustum, and a
/// zero-extent point box. All three are cases where a "conservative" test that is subtly
/// non-conservative loses geometry, and all three appear in real scenes (ground planes,
/// skydomes, marker objects).
#[test]
fn degenerate_boxes_are_still_never_missed() {
    let eye = Vec3::new(0.0, 0.0, 50.0);
    let f = cam(eye, Vec3::ZERO, 200.0);
    let mut t = RenderAabbTree::with_fat_margin(0.0);

    // 0: a box far larger than the frustum, swallowing the camera whole.
    t.insert(0, boxx(0.0, 0.0, 0.0, 5000.0));
    // 1: a degenerate point box on the view axis.
    t.insert(1, Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)));
    // 2: a flat plane box (zero thickness) — a ground quad.
    t.insert(2, Aabb::new(Vec3::new(-100.0, 0.0, -100.0), Vec3::new(100.0, 0.0, 100.0)));
    // 3: something plainly out of view.
    t.insert(3, boxx(0.0, 0.0, 900.0, 1.0));

    let mut out = vec![];
    t.query_frustum(&f, &mut out);
    let got = sorted(out);
    for k in [0u32, 1, 2] {
        assert!(got.contains(&k), "key {k} must survive the cull");
    }
    // And it agrees with the exact per-box test, in both directions, at margin 0.
    let tight = vec![
        (0u32, boxx(0.0, 0.0, 0.0, 5000.0)),
        (1, Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0))),
        (2, Aabb::new(Vec3::new(-100.0, 0.0, -100.0), Vec3::new(100.0, 0.0, 100.0))),
        (3, boxx(0.0, 0.0, 900.0, 1.0)),
    ];
    assert_eq!(got, brute_force(&tight, &f));
}

/// An empty/inverted box is refused rather than stored. A leaf holding `Aabb::empty()` would
/// merge `+INF`/`−INF` into every ancestor and take the whole subtree's bounds with it.
///
/// The rejection runs in **both** profiles — `Aabb::empty()` is what `Aabb::from_points`
/// returns for a mesh with no vertices, i.e. a data condition, so it must not be a
/// debug-only panic. It used to be, which made the assertions below `#[cfg(not(debug_assertions))]`
/// and therefore absent from every `cargo test` CI ever runs.
#[test]
fn an_empty_aabb_is_refused_in_both_profiles() {
    let mut t = RenderAabbTree::new();
    t.insert(0, boxx(0.0, 0.0, 0.0, 1.0));

    assert!(!t.insert(1, Aabb::empty()), "an empty box must be refused");
    assert!(!t.contains(1));
    assert_eq!(t.len(), 1);

    // An inverted box (min > max on one axis only) is the same refusal.
    assert!(!t.insert(2, Aabb::new(Vec3::new(5.0, -1.0, -1.0), Vec3::new(-5.0, 1.0, 1.0))));
    assert!(!t.contains(2));

    // The tree the refused keys never joined is still intact and still usable.
    assert_eq!(t.len(), 1);
    assert!(t.insert(1, boxx(10.0, 0.0, 0.0, 1.0)), "the key is still free afterwards");
    assert!(t.contains(1));
    #[cfg(debug_assertions)]
    t.validate();
}

/// `query_frusta` returns the deduplicated, sorted union — a mesh in the camera frustum *and*
/// in three cascades is one candidate, not four.
#[test]
fn query_frusta_is_a_sorted_deduplicated_union() {
    let mut t = RenderAabbTree::new();
    for i in 0..60u32 {
        t.insert(i, boxx(i as f32 * 2.0 - 60.0, 0.0, -40.0, 1.0));
    }
    let a = cam(Vec3::new(0.0, 0.0, 40.0), Vec3::new(0.0, 0.0, -40.0), 500.0);
    let b = cam(Vec3::new(0.0, 0.0, 41.0), Vec3::new(0.0, 0.0, -40.0), 500.0);
    let c = cam(Vec3::new(900.0, 0.0, 0.0), Vec3::new(1000.0, 0.0, 0.0), 100.0);

    let mut single = vec![];
    t.query_frustum(&a, &mut single);
    let single = sorted(single);
    assert!(!single.is_empty(), "premise: frustum a sees something");

    let mut union = vec![];
    t.query_frusta(&[a, b, c], &mut union);
    assert!(union.windows(2).all(|w| w[0] < w[1]), "must be sorted and deduplicated");
    for k in single {
        assert!(union.binary_search(&k).is_ok());
    }

    // `query_frusta` clears its output; `query_frustum` appends to it. Both are documented,
    // and mixing them up is how a candidate list silently grows across frames.
    let mut reused = vec![7, 7, 7];
    t.query_frusta(&[c], &mut reused);
    assert!(reused.is_empty(), "query_frusta must clear");
    let mut appended = vec![u32::MAX - 1];
    t.query_frustum(&c, &mut appended);
    assert_eq!(appended, vec![u32::MAX - 1], "query_frustum must append");
}

/// A negative margin is clamped, not honoured: shrinking a leaf box below the geometry it
/// bounds is the one thing this structure must be incapable of.
#[test]
fn a_negative_fat_margin_is_clamped_to_zero() {
    let t = RenderAabbTree::with_fat_margin(-5.0);
    assert_eq!(t.fat_margin(), 0.0);
}

/// `clear` empties the tree but keeps its capacity and its margin, and the tree is usable
/// afterwards — a scene reload path.
#[test]
fn clear_empties_but_keeps_the_margin_and_stays_usable() {
    let mut t = RenderAabbTree::with_fat_margin(3.0);
    for i in 0..50u32 {
        t.insert(i, boxx(i as f32, 0.0, 0.0, 1.0));
    }
    t.clear();
    assert!(t.is_empty());
    assert_eq!(t.fat_margin(), 3.0);
    assert!(!t.contains(10));
    assert_eq!(t.height(), 0);
    assert!(t.insert(10, boxx(0.0, 0.0, 0.0, 1.0)));
    assert_eq!(t.len(), 1);
    #[cfg(debug_assertions)]
    t.validate();
}

/// The tree stays shallow as it grows. A degenerate chain would still be *correct* and would
/// quietly cost O(N) per query, i.e. it would look exactly like the problem this replaced.
#[test]
fn the_tree_stays_balanced_enough_to_be_sublinear() {
    let (t, _) = random_scene(7, 4096, 1.0);
    let ideal = 12; // log2(4096)
    assert!(
        t.height() <= (ideal * 3) as u32,
        "height {} for 4096 leaves — the SAH/AVL insert has degenerated",
        t.height()
    );
}

/// A rotated + scaled model matrix goes through `Aabb::transform`, and the transformed box is
/// what gets indexed. Pinned because this is the exact composition the maintenance system
/// performs, and getting the operand order wrong there produces a plausible-looking but
/// wrongly-placed box.
#[test]
fn an_indexed_box_is_the_transformed_one() {
    let local = Aabb::new(Vec3::new(-1.0, -2.0, -3.0), Vec3::new(1.0, 2.0, 3.0));
    let model = Mat4::from_scale_rotation_translation(
        Vec3::new(2.0, 2.0, 2.0),
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        Vec3::new(100.0, 5.0, -20.0),
    );
    let world = local.transform(&model);
    let mut t = RenderAabbTree::with_fat_margin(0.0);
    t.insert(1, world);
    assert_eq!(t.leaf_aabb(1).unwrap(), world);

    let mut out = vec![];
    t.query_aabb(&boxx(100.0, 5.0, -20.0, 0.1), &mut out);
    assert_eq!(out, vec![1], "the box must be where the transform put it");
}

// Small convenience used by T3.
trait Translated {
    fn translated(self, dx: f32) -> Aabb;
}
impl Translated for Aabb {
    fn translated(self, dx: f32) -> Aabb {
        let d = gizmo_math::Vec3A::new(dx, 0.0, 0.0);
        Aabb::new(self.min + d, self.max + d)
    }
}
