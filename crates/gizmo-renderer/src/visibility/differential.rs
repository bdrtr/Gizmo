//! **The** correctness test for the index: the draw set it produces must be *exactly* the draw
//! set the existing linear cull produces. Same entities, no more and no fewer, for the camera
//! frustum and for every shadow cascade separately.
//!
//! A culling bug does not crash and does not fail a smoke test. It makes geometry silently stop
//! being drawn — one building, at one camera angle, three levels into a map. Nothing catches
//! that except an exact differential against the path being replaced, so that is what this is.
//!
//! The two paths compared are:
//!
//! * **linear** — every mesh, `classify_visibility(frustum, cascades, model, local_bounds, …)`,
//!   which is what `collect_draw_items` and the studio pipeline do today.
//! * **indexed** — `RenderAabbTree::query_frustum` per frustum to get candidates, then
//!   `classify_visibility_world` on each candidate and nothing else.
//!
//! Equality is expected to be *exact*, not approximate, and that is a theorem rather than a
//! hope: leaf boxes are fattened, so `fat ⊇ tight`, so `Outside(fat) ⇒ Outside(tight)`, so the
//! candidate set is a superset of everything the exact test would keep; and the exact test is
//! then the same function on the same box. The only way the sets can differ is if the index has
//! a false negative, which is precisely the failure worth a test.

use super::RenderAabbTree;
use crate::components::MaterialType;
use crate::frustum_cull::{classify_visibility, classify_visibility_world, Visibility};
use gizmo_math::{Aabb, Frustum, Mat4, Quat, Vec3};

/// One renderable, in the shape `collect_draw_items` sees it.
#[derive(Clone, Copy)]
struct Item {
    local: Aabb,
    model: Mat4,
    material: MaterialType,
    transparent: bool,
    alpha: f32,
}

impl Item {
    fn world(&self) -> Aabb {
        self.local.transform(&self.model)
    }
    /// Camera-locked geometry is drawn somewhere the authored matrix does not describe, so it
    /// must never be indexed. Both paths agree to let it through untested.
    fn indexable(&self) -> bool {
        !crate::backdrop::is_camera_locked(self.material)
    }
}

struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0xda3e_39cb_94b9_5bdb)
    }
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn f(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.next_u32() as f32 / u32::MAX as f32) * (hi - lo)
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n
    }
}

fn frustum_at(eye: Vec3, at: Vec3, far: f32) -> Frustum {
    let view = Mat4::look_at_rh(eye, at, Vec3::Y);
    let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_3, 16.0 / 9.0, 0.1, far);
    Frustum::from_matrix(&(proj * view))
}

/// The draw set the engine builds today: for every item, the exact classification.
///
/// Returns (camera-visible ids, shadow-only ids), both sorted — the two lists
/// `collect_draw_items` fills as `instances` and `shadow_instances`.
fn linear_draw_sets(items: &[Item], cam: &Frustum, cascades: &[Frustum]) -> (Vec<u32>, Vec<u32>) {
    let mut camera = vec![];
    let mut shadow = vec![];
    for (i, it) in items.iter().enumerate() {
        match classify_visibility(
            cam,
            cascades,
            &it.model,
            it.local,
            it.material,
            it.transparent,
            it.alpha,
        ) {
            Visibility::Culled => {}
            Visibility::Camera => camera.push(i as u32),
            Visibility::ShadowOnly => shadow.push(i as u32),
        }
    }
    (camera, shadow)
}

/// The same draw set via the index: candidates first, exact test second.
fn indexed_draw_sets(
    items: &[Item],
    tree: &RenderAabbTree,
    cam: &Frustum,
    cascades: &[Frustum],
) -> (Vec<u32>, Vec<u32>) {
    // Exactly what the guard would do: union of the camera frustum and every cascade.
    let mut frusta = Vec::with_capacity(1 + cascades.len());
    frusta.push(*cam);
    frusta.extend_from_slice(cascades);
    let mut candidates = vec![];
    tree.query_frusta(&frusta, &mut candidates);

    let mut camera = vec![];
    let mut shadow = vec![];
    for (i, it) in items.iter().enumerate() {
        let id = i as u32;
        // THE GUARD, and it is deliberately keyed on `tree.contains`, not on the caller's own
        // idea of what "should" be indexed. Skip only what the index KNOWS about and did not
        // nominate; anything it never received falls through and is drawn.
        //
        // Written the other way — `if it.indexable() && !nominated { skip }`, or worse, a loop
        // over `candidates` — the guard silently deletes every renderable the maintenance pass
        // missed: spawned this frame, refused by `insert` (an empty `Mesh::bounds` is), or just
        // forgotten. `tree.contains` cannot get out of step with the tree, and a caller-side
        // predicate can. It also means camera-locked geometry needs no special case here: it is
        // never inserted, so `contains` is false and it passes.
        if tree.contains(id) && candidates.binary_search(&id).is_err() {
            continue;
        }
        match classify_visibility_world(
            cam,
            cascades,
            it.world(),
            it.material,
            it.transparent,
            it.alpha,
        ) {
            Visibility::Culled => {}
            Visibility::Camera => camera.push(id),
            Visibility::ShadowOnly => shadow.push(id),
        }
    }
    (camera, shadow)
}

fn build_index(items: &[Item]) -> RenderAabbTree {
    let mut t = RenderAabbTree::new();
    for (i, it) in items.iter().enumerate() {
        if it.indexable() {
            t.insert(i as u32, it.world());
        }
    }
    t
}

/// Assert both draw sets match, and additionally that they match *per cascade* — a union over
/// cascades could mask a per-cascade error, and a mesh reaching the wrong cascade's shadow map
/// is a real artifact.
fn assert_sets_match(items: &[Item], tree: &RenderAabbTree, cam: &Frustum, cascades: &[Frustum], ctx: &str) {
    let (lin_cam, lin_shadow) = linear_draw_sets(items, cam, cascades);
    let (idx_cam, idx_shadow) = indexed_draw_sets(items, tree, cam, cascades);
    assert_eq!(lin_cam, idx_cam, "{ctx}: camera-visible set differs");
    assert_eq!(lin_shadow, idx_shadow, "{ctx}: shadow-only set differs");

    // Per cascade, in isolation: treat each cascade as the only one and compare again.
    for (ci, casc) in cascades.iter().enumerate() {
        let one = [*casc];
        let (lc, ls) = linear_draw_sets(items, cam, &one);
        let (ic, is) = indexed_draw_sets(items, tree, cam, &one);
        assert_eq!(lc, ic, "{ctx}: cascade {ci} camera set differs");
        assert_eq!(ls, is, "{ctx}: cascade {ci} shadow set differs");
    }
}

/// Four cascade frusta from the real CSM builder, so the ortho boxes are the shapes the engine
/// actually produces — deliberately non-nesting, each from its own disjoint depth slice.
fn real_cascades(cam_pos: Vec3, forward: Vec3) -> Vec<Frustum> {
    crate::csm::directional_cascade_view_projs(
        cam_pos,
        forward,
        16.0 / 9.0,
        std::f32::consts::FRAC_PI_3,
        0.1,
        &[30.0, 90.0, 250.0, 500.0],
        Vec3::new(-0.4, -1.0, -0.3),
        2048,
    )
    .iter()
    .map(Frustum::from_matrix)
    .collect()
}

fn random_items(rng: &mut Lcg, n: usize) -> Vec<Item> {
    (0..n)
        .map(|_| {
            let h = Vec3::new(rng.f(0.2, 8.0), rng.f(0.2, 6.0), rng.f(0.2, 8.0));
            Item {
                local: Aabb::new(-h, h),
                model: Mat4::from_scale_rotation_translation(
                    Vec3::splat(rng.f(0.5, 2.5)),
                    Quat::from_rotation_y(rng.f(0.0, std::f32::consts::TAU)),
                    Vec3::new(rng.f(-400.0, 400.0), rng.f(-20.0, 60.0), rng.f(-400.0, 400.0)),
                ),
                material: match rng.below(8) {
                    0 => MaterialType::Unlit,
                    1 => MaterialType::Backdrop,
                    2 => MaterialType::Skybox,
                    3 => MaterialType::Grid,
                    4 => MaterialType::Water,
                    5 => MaterialType::BakedLit,
                    _ => MaterialType::Pbr,
                },
                transparent: rng.below(6) == 0,
                alpha: if rng.below(5) == 0 { rng.f(0.1, 0.98) } else { 1.0 },
            }
        })
        .collect()
}

/// The headline: randomised scenes × randomised cameras, indexed draw set == linear draw set.
#[test]
fn the_indexed_draw_set_equals_the_linear_draw_set() {
    for seed in 0..20u64 {
        let mut rng = Lcg::new(seed);
        let items = random_items(&mut rng, 400);
        let tree = build_index(&items);

        for shot in 0..6 {
            let eye = Vec3::new(rng.f(-450.0, 450.0), rng.f(-30.0, 70.0), rng.f(-450.0, 450.0));
            let dir = Vec3::new(rng.f(-1.0, 1.0), rng.f(-0.6, 0.6), rng.f(-1.0, 1.0));
            let dir = if dir.length_squared() < 1e-4 { Vec3::NEG_Z } else { dir.normalize() };
            let cam = frustum_at(eye, eye + dir, rng.f(50.0, 600.0));
            let cascades = real_cascades(eye, dir);
            assert_sets_match(&items, &tree, &cam, &cascades, &format!("seed {seed} shot {shot}"));
        }
    }
}

/// An empty scene: no panic, no candidates, both paths draw nothing.
#[test]
fn an_empty_scene_agrees() {
    let items: Vec<Item> = vec![];
    let tree = build_index(&items);
    let cam = frustum_at(Vec3::ZERO, Vec3::NEG_Z, 100.0);
    let cascades = real_cascades(Vec3::ZERO, Vec3::NEG_Z);
    assert_sets_match(&items, &tree, &cam, &cascades, "empty scene");
}

/// The degenerate geometry the plan named: a box sitting exactly on a frustum plane, a box
/// larger than the whole frustum, a zero-volume point and a zero-thickness ground plane.
///
/// Each is a case where a subtly non-conservative test loses geometry, and each is ordinary in
/// a real scene.
#[test]
fn degenerate_placements_agree() {
    let eye = Vec3::new(0.0, 0.0, 50.0);
    let cam = frustum_at(eye, Vec3::ZERO, 200.0);
    let cascades = real_cascades(eye, Vec3::NEG_Z);

    let unit = |h: f32| Aabb::new(Vec3::splat(-h), Vec3::splat(h));
    let mut items = vec![
        // Larger than the frustum, swallowing the camera.
        Item { local: unit(5000.0), model: Mat4::IDENTITY, material: MaterialType::Pbr, transparent: false, alpha: 1.0 },
        // A point.
        Item { local: Aabb::new(Vec3::ZERO, Vec3::ZERO), model: Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0)), material: MaterialType::Pbr, transparent: false, alpha: 1.0 },
        // A zero-thickness ground quad.
        Item { local: Aabb::new(Vec3::new(-200.0, 0.0, -200.0), Vec3::new(200.0, 0.0, 200.0)), model: Mat4::IDENTITY, material: MaterialType::BakedLit, transparent: false, alpha: 1.0 },
        // Far behind the camera.
        Item { local: unit(2.0), model: Mat4::from_translation(Vec3::new(0.0, 0.0, 900.0)), material: MaterialType::Pbr, transparent: false, alpha: 1.0 },
    ];

    // A box whose face lands exactly on the far plane, and one exactly on the near plane.
    // `test_aabb` is inclusive (`< 0.0` rejects), so "exactly on" must count as visible in both
    // paths — and it must count the same way in both.
    items.push(Item {
        local: unit(1.0),
        model: Mat4::from_translation(Vec3::new(0.0, 0.0, eye.z - 200.0 + 1.0)),
        material: MaterialType::Pbr,
        transparent: false,
        alpha: 1.0,
    });
    items.push(Item {
        local: unit(1.0),
        model: Mat4::from_translation(Vec3::new(0.0, 0.0, eye.z - 0.1 - 1.0)),
        material: MaterialType::Pbr,
        transparent: false,
        alpha: 1.0,
    });

    let tree = build_index(&items);
    assert_sets_match(&items, &tree, &cam, &cascades, "degenerate placements");
}

/// **An entity that moved since the index was built.**
///
/// Two cases, and they are not the same case:
///
/// * A move *smaller* than the fat margin does not re-bin, so the index answers against a box
///   that is stale — and the sets must still match, because a stale fat box is still a
///   superset of where the geometry now is. This is the property the margin buys, and it is
///   the one that would be silently wrong if `insert`'s early-out were ever widened.
/// * A move *larger* than the margin must be re-inserted by whatever maintains the index. Once
///   it is, the sets match again. Before it is, they may not — and the test asserts that too,
///   because "the index must be refreshed for everything that moved" is a contract worth
///   pinning rather than assuming.
#[test]
fn an_entity_that_moved_since_the_index_was_built() {
    let mut rng = Lcg::new(99);
    let mut items = random_items(&mut rng, 250);
    // Make the movers plain opaque PBR so they are unambiguously cullable and castable.
    for it in items.iter_mut().take(30) {
        it.material = MaterialType::Pbr;
        it.transparent = false;
        it.alpha = 1.0;
    }
    let mut tree = build_index(&items);

    let eye = Vec3::new(0.0, 10.0, 200.0);
    let dir = Vec3::NEG_Z;
    let cam = frustum_at(eye, eye + dir, 500.0);
    let cascades = real_cascades(eye, dir);
    assert_sets_match(&items, &tree, &cam, &cascades, "before any movement");

    // (1) Sub-margin drift, index deliberately NOT refreshed.
    let margin = tree.fat_margin();
    let nudge = Mat4::from_translation(Vec3::new(margin * 0.4, 0.0, margin * 0.3));
    for it in items.iter_mut().take(30) {
        it.model = nudge * it.model;
    }
    assert_sets_match(&items, &tree, &cam, &cascades, "sub-margin drift, index not refreshed");

    // (2) A large move. Refresh exactly the movers, as maintenance would, then compare.
    let jump = Mat4::from_translation(Vec3::new(-150.0, 0.0, -180.0));
    for it in items.iter_mut().take(30) {
        it.model = jump * it.model;
    }
    for (i, it) in items.iter().enumerate().take(30) {
        tree.insert(i as u32, it.world());
    }
    assert_sets_match(&items, &tree, &cam, &cascades, "large move, index refreshed");

    // (3) And the contract, stated as a test: skip the refresh and the index CAN lose
    // geometry. This is not a bug in the tree — it is why maintenance is not optional.
    //
    // The direction matters. An object that moves OUT of view while the index still holds its
    // old box costs a wasted exact test and nothing else, which is why (1) and (2) above pass
    // so easily. The failure mode is the other direction: an object indexed far away that
    // moves INTO view. The index never nominates it, the guard skips it, and it is simply not
    // drawn — invisible geometry, no error, no log line.
    let hidden = items.len();
    items.push(Item {
        local: Aabb::new(Vec3::splat(-3.0), Vec3::splat(3.0)),
        model: Mat4::from_translation(Vec3::new(9_000.0, 0.0, 9_000.0)),
        material: MaterialType::Pbr,
        transparent: false,
        alpha: 1.0,
    });
    tree.insert(hidden as u32, items[hidden].world());
    assert_sets_match(&items, &tree, &cam, &cascades, "far-away newcomer, indexed");

    // Now walk it right in front of the camera and DO NOT refresh the index.
    items[hidden].model = Mat4::from_translation(Vec3::new(0.0, 10.0, 150.0));
    let (lin_cam, _) = linear_draw_sets(&items, &cam, &cascades);
    let (idx_cam, _) = indexed_draw_sets(&items, &tree, &cam, &cascades);
    assert!(
        lin_cam.contains(&(hidden as u32)),
        "premise: the moved object really is on screen now"
    );
    assert!(
        !idx_cam.contains(&(hidden as u32)),
        "premise of the maintenance contract: an object that moves into view without being \
         re-inserted really does vanish. If this ever starts passing, staleness stopped being \
         reachable and every other test here is proving less than it claims"
    );
    assert_ne!(lin_cam, idx_cam);

    // …and one `insert` puts it back.
    tree.insert(hidden as u32, items[hidden].world());
    assert_sets_match(&items, &tree, &cam, &cascades, "after the refresh that was missing");
}

/// **A renderable the index never received must still be drawn.**
///
/// The counterpart to `an_entity_that_moved_since_the_index_was_built`: that one pins that a
/// *stale* box loses geometry, this one pins that a *missing* box does not. Between them they
/// fix the guard's shape — skip only what the index holds and rejected, never "iterate the
/// candidate list".
///
/// Two ways a key legitimately misses the index, both reachable without a bug in the tree:
/// spawned after the last maintenance pass, and refused by `insert` (an `Aabb::empty()` was
/// passed — bounds not computed yet, an authored mesh with no geometry). A third, camera-locked
/// materials, has its own test below.
#[test]
fn a_renderable_the_index_never_received_is_drawn_not_skipped() {
    let mut rng = Lcg::new(4242);
    let mut items = random_items(&mut rng, 80);
    let mut tree = build_index(&items);

    let eye = Vec3::new(0.0, 5.0, 60.0);
    let cam = frustum_at(eye, Vec3::new(0.0, 0.0, -40.0), 400.0);
    let cascades = real_cascades(eye, Vec3::NEG_Z);

    // (1) Spawned this frame, right in front of the camera, maintenance has not run yet.
    let spawned = items.len();
    items.push(Item {
        local: Aabb::new(Vec3::splat(-2.0), Vec3::splat(2.0)),
        model: Mat4::from_translation(Vec3::new(0.0, 0.0, -20.0)),
        material: MaterialType::Pbr,
        transparent: false,
        alpha: 1.0,
    });
    assert!(!tree.contains(spawned as u32), "premise: not indexed yet");

    // (2) Maintenance offered the index an `Aabb::empty()` — bounds not yet computed for an
    // asset still loading, say. `insert` refuses it and returns the same `false` the no-op
    // early-out returns, so the caller cannot tell it was refused. The object is drawable all
    // the same: the renderer has a real box for it, the index does not.
    let vertexless = items.len();
    items.push(Item {
        local: Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0)),
        model: Mat4::from_translation(Vec3::new(3.0, 0.0, -25.0)),
        material: MaterialType::Pbr,
        transparent: false,
        alpha: 1.0,
    });
    assert!(
        !tree.insert(vertexless as u32, Aabb::empty()),
        "an empty box must be refused, and refusal is indistinguishable from the no-op early-out"
    );
    assert!(!tree.contains(vertexless as u32));

    // The whole point: both are on screen, and the guarded indexed path must draw both.
    let (lin_cam, _) = linear_draw_sets(&items, &cam, &cascades);
    assert!(
        lin_cam.contains(&(spawned as u32)) && lin_cam.contains(&(vertexless as u32)),
        "premise: both un-indexed newcomers really are camera-visible"
    );
    assert_sets_match(&items, &tree, &cam, &cascades, "un-indexed renderables");

    // And the negative control: the guard is not simply inert. Index the spawned object at a
    // place it is not, and it DOES vanish — so the pass above came from the fail-open guard,
    // not from the guard never skipping anything.
    tree.insert(
        spawned as u32,
        Aabb::new(Vec3::splat(9_000.0), Vec3::splat(9_010.0)),
    );
    let (idx_cam, _) = indexed_draw_sets(&items, &tree, &cam, &cascades);
    assert!(
        lin_cam.contains(&(spawned as u32)),
        "premise: the linear path still draws it"
    );
    assert!(
        !idx_cam.contains(&(spawned as u32)),
        "control: once the index HOLDS a (wrong) box for the key, the guard does skip it"
    );
}

/// Despawn/respawn onto the same key. The index must not answer with the dead entity's box.
#[test]
fn a_reused_key_gets_the_new_box_not_the_dead_ones() {
    let mut rng = Lcg::new(7);
    let mut items = random_items(&mut rng, 120);
    items[50] = Item {
        local: Aabb::new(Vec3::splat(-2.0), Vec3::splat(2.0)),
        model: Mat4::from_translation(Vec3::new(0.0, 0.0, -30.0)),
        material: MaterialType::Pbr,
        transparent: false,
        alpha: 1.0,
    };
    let mut tree = build_index(&items);

    // "Despawn" 50 and "spawn" a new entity onto the same key, 500 m away.
    tree.remove(50);
    items[50] = Item {
        local: Aabb::new(Vec3::splat(-2.0), Vec3::splat(2.0)),
        model: Mat4::from_translation(Vec3::new(500.0, 0.0, -30.0)),
        material: MaterialType::Pbr,
        transparent: false,
        alpha: 1.0,
    };
    tree.insert(50, items[50].world());
    assert_eq!(tree.leaf_aabb(50).unwrap().center().x.round(), 500.0);

    let eye = Vec3::new(0.0, 0.0, 40.0);
    let cam = frustum_at(eye, Vec3::new(0.0, 0.0, -30.0), 300.0);
    let cascades = real_cascades(eye, Vec3::NEG_Z);
    assert_sets_match(&items, &tree, &cam, &cascades, "reused key");
}

/// A camera-locked backdrop is never indexed and must never be skipped by the guard.
///
/// Sits next to `a_camera_locked_backdrop_must_be_culled_against_its_locked_transform` in
/// `frustum_cull.rs` on purpose: one grep finds both halves of the same hazard. Culling a
/// backdrop against its authored matrix is how 191 backdrop meshes never reached the screen;
/// *indexing* it against that matrix would be the same bug with a new hiding place.
#[test]
fn a_camera_locked_backdrop_is_never_indexed_and_never_skipped() {
    let authored = Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0));
    let items = vec![
        Item { local: Aabb::new(Vec3::splat(-2.0), Vec3::splat(2.0)), model: authored, material: MaterialType::Backdrop, transparent: false, alpha: 1.0 },
        Item { local: Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0)), model: Mat4::from_translation(Vec3::new(0.0, 0.0, -20.0)), material: MaterialType::Pbr, transparent: false, alpha: 1.0 },
    ];
    let tree = build_index(&items);
    assert!(!tree.contains(0), "a camera-locked backdrop must not be in the index");
    assert!(tree.contains(1));

    // A camera 900 m from where the backdrop's authored matrix says it is — exactly the case
    // where indexing it would have dropped it.
    let eye = Vec3::new(0.0, 0.0, 905.0);
    let cam = frustum_at(eye, Vec3::new(0.0, 0.0, -30.0), 2000.0);
    let cascades = real_cascades(eye, Vec3::NEG_Z);

    let (_, _) = linear_draw_sets(&items, &cam, &cascades);
    let (idx_cam, idx_shadow) = indexed_draw_sets(&items, &tree, &cam, &cascades);
    // Whatever the exact test decides about the backdrop, the index must not have pre-empted
    // it: the guard let it through, so the two paths agree.
    assert_sets_match(&items, &tree, &cam, &cascades, "camera-locked backdrop");
    let _ = (idx_cam, idx_shadow);
}

/// A scene where everything is on screen — the `Inside` fast path carrying the whole tree —
/// and a scene where nothing is. Both extremes must still agree exactly.
#[test]
fn all_visible_and_none_visible_both_agree() {
    let mut rng = Lcg::new(3);
    let items: Vec<Item> = (0..300)
        .map(|_| Item {
            local: Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0)),
            model: Mat4::from_translation(Vec3::new(rng.f(-20.0, 20.0), rng.f(-10.0, 10.0), rng.f(-60.0, -40.0))),
            material: MaterialType::Pbr,
            transparent: false,
            alpha: 1.0,
        })
        .collect();
    let tree = build_index(&items);

    let eye = Vec3::new(0.0, 0.0, 300.0);
    let cam_all = frustum_at(eye, Vec3::new(0.0, 0.0, -50.0), 2000.0);
    let cascades = real_cascades(eye, Vec3::NEG_Z);
    let (lin, _) = linear_draw_sets(&items, &cam_all, &cascades);
    assert_eq!(lin.len(), 300, "premise: this camera sees the whole cluster");
    assert_sets_match(&items, &tree, &cam_all, &cascades, "everything visible");

    let far_eye = Vec3::new(50_000.0, 0.0, 0.0);
    let cam_none = frustum_at(far_eye, far_eye + Vec3::X, 100.0);
    let far_cascades = real_cascades(far_eye, Vec3::X);
    let (lin_none, sh_none) = linear_draw_sets(&items, &cam_none, &far_cascades);
    assert!(lin_none.is_empty() && sh_none.is_empty(), "premise: nothing is visible");
    assert_sets_match(&items, &tree, &cam_none, &far_cascades, "nothing visible");
}
