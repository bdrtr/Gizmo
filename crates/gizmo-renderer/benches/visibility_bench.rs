//! What CPU frustum culling costs for a driving-map-sized scene, with and without a
//! spatial index.
//!
//! The workload is the one the engine actually runs every frame in
//! `gizmo::systems::render::batching::collect_draw_items`: one camera frustum plus
//! `CASCADE_COUNT` shadow-cascade light frusta, and `classify_visibility` per mesh.
//!
//! * **A** `cull/linear/8192` — today's loop, the baseline every other number is read against.
//! * **B** `cull/indexed/8192/refresh_all` — the index with *every* key dirty, i.e. the world
//!   where `Changed<GlobalTransform>` still matches everything. This must still beat A.
//! * **C** `cull/indexed/8192/refresh_movers` — the index with only the movers dirty: the target.
//! * **D** `cull/tree/build/8192` — filling an empty index (scene load / worst first frame).
//! * **E** `cull/masked_vs_unmasked/8192` — does `Frustum::test_aabb_masked`'s plane mask
//!   actually pay for itself in traversal, or is it only an unused API that now has a caller.

use criterion::{criterion_group, criterion_main, Criterion};
use gizmo_math::{Aabb, Frustum, Mat4, Vec3};
use gizmo_renderer::components::MaterialType;
use gizmo_renderer::csm::{directional_cascade_view_projs, CASCADE_COUNT};
use gizmo_renderer::frustum_cull::{classify_visibility, classify_visibility_world, Visibility};
use gizmo_renderer::visibility::RenderAabbTree;

const N: usize = 8192;
/// How many of `N` move every frame (a driving map: traffic + the player, the rest is world).
const MOVERS: usize = 292;

/// A deterministic LCG — no `rand` dev-dep, and the same scene every run.
struct Lcg(u64);
impl Lcg {
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f32) / (u32::MAX >> 1) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.next_f32() * (hi - lo)
    }
}

struct Scene {
    /// Local-space bounds, as `Mesh::bounds` would carry them.
    local: Vec<Aabb>,
    models: Vec<Mat4>,
    material: Vec<MaterialType>,
    camera: Frustum,
    cascades: Vec<Frustum>,
}

impl Scene {
    /// The same scene at an arbitrary mesh count, for the scaling sweep. Density is held
    /// constant — the world grows with the mesh count — so what varies is scene size, not how
    /// crowded the camera's view is.
    fn sized(n: usize) -> Self {
        let half = 1000.0 * (n as f32 / N as f32).sqrt();
        let mut s = Scene::new();
        let mut rng = Lcg(0x00c0_ffee_1234_5678);
        s.local.clear();
        s.models.clear();
        s.material.clear();
        for i in 0..n {
            let hx = rng.range(0.5, 6.0);
            let hy = rng.range(0.5, 4.0);
            let hz = rng.range(0.5, 6.0);
            s.local.push(Aabb::new(Vec3::new(-hx, -hy, -hz), Vec3::new(hx, hy, hz)));
            s.models.push(Mat4::from_translation(Vec3::new(
                rng.range(-half, half),
                rng.range(0.0, 100.0),
                rng.range(-half, half),
            )));
            s.material.push(match i % 32 {
                7 => MaterialType::Unlit,
                15 => MaterialType::Water,
                23 => MaterialType::BakedLit,
                _ => MaterialType::Pbr,
            });
        }
        s
    }

    /// 2 km × 2 km × 100 m of world geometry, a 500 m-far camera at the origin looking down −Z,
    /// and the four real CSM cascades for that camera.
    fn new() -> Self {
        let mut rng = Lcg(0x5eed_1234_abcd_ef01);
        let mut local = Vec::with_capacity(N);
        let mut models = Vec::with_capacity(N);
        let mut material = Vec::with_capacity(N);

        for i in 0..N {
            let hx = rng.range(0.5, 6.0);
            let hy = rng.range(0.5, 4.0);
            let hz = rng.range(0.5, 6.0);
            local.push(Aabb::new(Vec3::new(-hx, -hy, -hz), Vec3::new(hx, hy, hz)));
            let x = rng.range(-1000.0, 1000.0);
            let y = rng.range(0.0, 100.0);
            let z = rng.range(-1000.0, 1000.0);
            models.push(Mat4::from_translation(Vec3::new(x, y, z)));
            // Mostly opaque PBR world geometry (the shadow-caster path, i.e. the expensive one),
            // with a scattering of the other material classes.
            material.push(match i % 32 {
                7 => MaterialType::Unlit,
                15 => MaterialType::Water,
                23 => MaterialType::BakedLit,
                _ => MaterialType::Pbr,
            });
        }

        let cam_pos = Vec3::new(0.0, 2.0, 0.0);
        let cam_forward = Vec3::new(0.0, 0.0, -1.0);
        let aspect = 16.0 / 9.0;
        let fov_y = std::f32::consts::FRAC_PI_3;
        let z_near = 0.1;
        let view = Mat4::look_at_rh(cam_pos, cam_pos + cam_forward, Vec3::Y);
        let proj = Mat4::perspective_rh(fov_y, aspect, z_near, 500.0);
        let camera = Frustum::from_matrix(&(proj * view));

        let splits = [30.0f32, 90.0, 250.0, 500.0];
        let cascade_vp = directional_cascade_view_projs(
            cam_pos,
            cam_forward,
            aspect,
            fov_y,
            z_near,
            &splits,
            Vec3::new(-0.4, -1.0, -0.3),
            2048,
        );
        let cascades: Vec<Frustum> = cascade_vp.iter().map(Frustum::from_matrix).collect();
        assert_eq!(cascades.len(), CASCADE_COUNT);

        Self { local, models, material, camera, cascades }
    }

    fn world_aabb(&self, i: usize) -> Aabb {
        self.local[i].transform(&self.models[i])
    }

    /// `world_aabb`, with the entity displaced `dx` metres along +X.
    fn world_aabb_shifted(&self, i: usize, dx: f32) -> Aabb {
        let a = self.local[i].transform(&self.models[i]);
        if dx == 0.0 {
            return a;
        }
        let d = gizmo_math::Vec3A::new(dx, 0.0, 0.0);
        Aabb::new(a.min + d, a.max + d)
    }

    fn filled_tree(&self) -> RenderAabbTree {
        let mut t = RenderAabbTree::new();
        for i in 0..N {
            t.insert(i as u32, self.world_aabb(i));
        }
        t
    }
}

/// Bench A — the loop `collect_draw_items` runs today, verbatim in shape: per mesh, a
/// `Mat4`-transformed local box tested against the camera and every cascade.
///
/// One thing this can no longer reproduce: the pre-`classify_visibility_world` baseline.
/// `classify_visibility` now *delegates* to the world-space entry point, so it transforms the
/// box once, not once per frustum, and this bench measures the improved path. The 3.2×
/// recorded for that change was measured against the old body and is not re-derivable from
/// this file — to redo it, inline the old `visible_in_frustum(f, model, local)` per frustum
/// here.
fn bench_linear(c: &mut Criterion) {
    let scene = Scene::new();
    c.bench_function("cull/linear/8192", |b| {
        b.iter(|| {
            let mut drawn = 0usize;
            for i in 0..N {
                match classify_visibility(
                    &scene.camera,
                    &scene.cascades,
                    &scene.models[i],
                    scene.local[i],
                    scene.material[i],
                    false,
                    1.0,
                ) {
                    Visibility::Culled => {}
                    _ => drawn += 1,
                }
            }
            std::hint::black_box(drawn)
        })
    });
}

/// Bench A2 — what `collect_draw_items` ACTUALLY pays per mesh before it knows whether to draw.
///
/// Bench A measures `classify_visibility` alone. The real loop reaches that call only after
/// composing the model matrix (`trans.matrix * Mat4::from_translation(mesh.center_offset)`) and
/// running it through `camera_locked_model`. That work is per mesh, it happens whether or not
/// the mesh is visible, and it is exactly what a skip guard placed before it elides — so this,
/// not A, is the baseline the facade guard has to beat.
fn bench_linear_full(c: &mut Criterion) {
    let scene = Scene::new();
    let cam_pos = Vec3::new(0.0, 2.0, 0.0);
    let offsets: Vec<Vec3> = (0..N).map(|i| Vec3::splat((i % 7) as f32 * 0.01)).collect();
    c.bench_function("cull/linear_full/8192", |b| {
        b.iter(|| {
            let mut drawn = 0usize;
            for (i, off) in offsets.iter().enumerate() {
                let center_mat = Mat4::from_translation(*off);
                let model = scene.models[i] * center_mat;
                let drawn_model =
                    gizmo_renderer::camera_locked_model(scene.material[i], &model, cam_pos);
                let world = scene.local[i].transform(&drawn_model);
                match classify_visibility_world(
                    &scene.camera,
                    &scene.cascades,
                    world,
                    scene.material[i],
                    false,
                    1.0,
                ) {
                    Visibility::Culled => {}
                    _ => drawn += 1,
                }
            }
            std::hint::black_box(drawn)
        })
    });
}

/// Benches B and C — the indexed path, doing the WHOLE frame's cull work: refresh the index
/// for the dirty set, run the five frustum walks, then run the exact `classify_visibility_world`
/// on every candidate. The only difference between them is how many entities the refresh visits.
///
/// The tree is mutated in place rather than cloned per iteration — a 16 k-node clone is ~1 MB of
/// memcpy and would have measured the allocator, not the cull. The movers are toggled between
/// two positions 2 m apart, further than the 1 m fat margin, so **every** mover genuinely
/// re-bins on **every** iteration. Real traffic at 0.5 m/frame re-bins about every third frame,
/// so this deliberately overstates the refresh cost rather than flattering it.
fn bench_indexed(c: &mut Criterion) {
    fn all_frusta(s: &Scene) -> Vec<Frustum> {
        let mut v = Vec::with_capacity(1 + s.cascades.len());
        v.push(s.camera);
        v.extend_from_slice(&s.cascades);
        v
    }

    /// One frame: refresh `dirty` keys, walk, exact-test the survivors.
    fn frame(
        tree: &mut RenderAabbTree,
        scene: &Scene,
        frusta: &[Frustum],
        dirty: usize,
        phase: bool,
        cand: &mut Vec<u32>,
    ) -> usize {
        let shift = if phase { 2.0 } else { 0.0 };
        for i in 0..dirty {
            // Only the movers actually move. In bench B the refresh still VISITS every key —
            // that is what "change detection is broken" means — but a static's box is
            // unchanged, so it must hit the fat-box early-out. Shifting everything here would
            // have measured a full 8 192-leaf rebuild per frame, which is not the scenario.
            let dx = if i < MOVERS { shift } else { 0.0 };
            tree.insert(i as u32, scene.world_aabb_shifted(i, dx));
        }
        tree.query_frusta(frusta, cand);
        let mut drawn = 0usize;
        for &k in cand.iter() {
            let i = k as usize;
            let shifted = if i < MOVERS { shift } else { 0.0 };
            match classify_visibility_world(
                &scene.camera,
                &scene.cascades,
                scene.world_aabb_shifted(i, shifted),
                scene.material[i],
                false,
                1.0,
            ) {
                Visibility::Culled => {}
                _ => drawn += 1,
            }
        }
        drawn
    }

    let scene = Scene::new();
    let frusta = all_frusta(&scene);

    // Report the shape of the workload once — a candidate set that is most of the scene means
    // the index is not the bottleneck and the numbers below should be read with that in mind.
    {
        let tree = scene.filled_tree();
        let mut cand = Vec::new();
        tree.query_frusta(&frusta, &mut cand);
        let mut cam_only = Vec::new();
        tree.query_frustum(&scene.camera, &mut cam_only);
        eprintln!(
            "scene: {N} meshes, tree height {}, camera candidates {}, camera+cascade candidates {} ({:.1}%)",
            tree.height(),
            cam_only.len(),
            cand.len(),
            100.0 * cand.len() as f32 / N as f32,
        );
    }

    // B — every key visited by the refresh (the world where `Changed<GlobalTransform>` still
    // matches everything).
    {
        let mut tree = scene.filled_tree();
        let mut cand = Vec::with_capacity(N);
        let mut phase = false;
        c.bench_function("cull/indexed/8192/refresh_all", |b| {
            b.iter(|| {
                phase = !phase;
                std::hint::black_box(frame(&mut tree, &scene, &frusta, N, phase, &mut cand))
            })
        });
    }

    // C — only the movers visited (the target: step 1 landed).
    {
        let mut tree = scene.filled_tree();
        let mut cand = Vec::with_capacity(N);
        let mut phase = false;
        c.bench_function("cull/indexed/8192/refresh_movers", |b| {
            b.iter(|| {
                phase = !phase;
                std::hint::black_box(frame(&mut tree, &scene, &frusta, MOVERS, phase, &mut cand))
            })
        });
    }
}

/// Bench D — filling an empty index. Paid at scene load, never per frame.
fn bench_build(c: &mut Criterion) {
    let scene = Scene::new();
    let boxes: Vec<Aabb> = (0..N).map(|i| scene.world_aabb(i)).collect();
    c.bench_function("cull/tree/build/8192", |b| {
        b.iter(|| {
            let mut t = RenderAabbTree::new();
            for (i, bx) in boxes.iter().enumerate() {
                t.insert(i as u32, *bx);
            }
            std::hint::black_box(t.len())
        })
    });
}

/// Bench E — does the plane mask earn its keep? Same traversal, same tree, the only
/// difference is whether the reduced mask is propagated to children or `FULL_MASK` is
/// forced at every level.
fn bench_masked(c: &mut Criterion) {
    let scene = Scene::new();
    let tree = scene.filled_tree();
    let mut all = Vec::with_capacity(1 + scene.cascades.len());
    all.push(scene.camera);
    all.extend_from_slice(&scene.cascades);

    let mut g = c.benchmark_group("cull/masked_vs_unmasked/8192");
    g.bench_function("masked", |b| {
        let mut out = Vec::with_capacity(4096);
        b.iter(|| {
            out.clear();
            for f in &all {
                tree.query_frustum(f, &mut out);
            }
            std::hint::black_box(out.len())
        })
    });
    g.bench_function("unmasked", |b| {
        let mut out = Vec::with_capacity(4096);
        b.iter(|| {
            out.clear();
            for f in &all {
                tree.query_frustum_full_mask(f, &mut out);
            }
            std::hint::black_box(out.len())
        })
    });
    g.finish();
}

/// Decomposition of bench C — which term of the indexed frame actually costs what. Diagnostic;
/// not a gate.
fn bench_parts(c: &mut Criterion) {
    let scene = Scene::new();
    let mut frusta = Vec::new();
    frusta.push(scene.camera);
    frusta.extend_from_slice(&scene.cascades);
    let tree = scene.filled_tree();

    let mut g = c.benchmark_group("cull/parts/8192");
    g.bench_function("walk_only_5_frusta", |b| {
        let mut out = Vec::with_capacity(8192);
        b.iter(|| {
            out.clear();
            for f in &frusta {
                tree.query_frustum(f, &mut out);
            }
            std::hint::black_box(out.len())
        })
    });
    g.bench_function("walk_plus_sort_dedup", |b| {
        let mut out = Vec::with_capacity(8192);
        b.iter(|| {
            tree.query_frusta(&frusta, &mut out);
            std::hint::black_box(out.len())
        })
    });
    g.bench_function("rebin_292_movers", |b| {
        let mut t = tree.clone();
        let mut phase = false;
        b.iter(|| {
            phase = !phase;
            let shift = if phase { 2.0 } else { 0.0 };
            for i in 0..MOVERS {
                t.insert(i as u32, scene.world_aabb_shifted(i, shift));
            }
            std::hint::black_box(t.len())
        })
    });
    g.bench_function("earlyout_8192_statics", |b| {
        let mut t = tree.clone();
        b.iter(|| {
            for i in 0..N {
                t.insert(i as u32, scene.world_aabb_shifted(i, 0.0));
            }
            std::hint::black_box(t.len())
        })
    });
    g.bench_function("exact_test_1540_candidates", |b| {
        let mut cand = Vec::new();
        tree.query_frusta(&frusta, &mut cand);
        b.iter(|| {
            let mut drawn = 0usize;
            for &k in &cand {
                let i = k as usize;
                match classify_visibility_world(
                    &scene.camera, &scene.cascades, scene.world_aabb_shifted(i, 0.0),
                    scene.material[i], false, 1.0,
                ) { Visibility::Culled => {} _ => drawn += 1 }
            }
            std::hint::black_box(drawn)
        })
    });
    g.finish();
}

/// Where does the index start winning? The linear cull is a sequential scan the prefetcher
/// loves; the BVH is pointer chasing. At 8 192 meshes that is not obviously a good trade, and
/// the answer is a measurement, not an opinion.
fn bench_scaling(c: &mut Criterion) {
    let mut g = c.benchmark_group("cull/scaling");
    for &n in &[4096usize, 8192, 32768, 131072] {
        let scene = Scene::sized(n);
        let mut frusta = vec![scene.camera];
        frusta.extend_from_slice(&scene.cascades);
        let boxes: Vec<Aabb> = (0..n).map(|i| scene.world_aabb(i)).collect();

        g.bench_with_input(criterion::BenchmarkId::new("linear", n), &n, |b, &n| {
            b.iter(|| {
                let mut drawn = 0usize;
                for (i, bx) in boxes.iter().enumerate().take(n) {
                    match classify_visibility_world(
                        &scene.camera, &scene.cascades, *bx,
                        scene.material[i], false, 1.0,
                    ) { Visibility::Culled => {} _ => drawn += 1 }
                }
                std::hint::black_box(drawn)
            })
        });

        let mut tree = RenderAabbTree::new();
        for (i, bx) in boxes.iter().enumerate() {
            tree.insert(i as u32, *bx);
        }
        let movers = n / 28; // same ~3.6% mover fraction at every size
        g.bench_with_input(criterion::BenchmarkId::new("indexed", n), &n, |b, &n| {
            let _ = n;
            let mut t = tree.clone();
            let mut cand = Vec::with_capacity(1 << 16);
            let mut phase = false;
            b.iter(|| {
                phase = !phase;
                let shift = if phase { 2.0 } else { 0.0 };
                for i in 0..movers {
                    t.insert(i as u32, scene.world_aabb_shifted(i, shift));
                }
                // `query_frusta`, NOT five bare `query_frustum` appends: the union has to be
                // sorted and deduplicated, or this bench pays the exact test two or three times
                // for every mesh that lands in more than one cascade — and skips the sort the
                // real frame pays. Both errors are small and they point opposite ways, which is
                // exactly why they must not be left in a bench whose only job is locating a
                // crossover. This has to be the same frame `cull/indexed/*` measures.
                t.query_frusta(&frusta, &mut cand);
                let mut drawn = 0usize;
                for &k in &cand {
                    let i = k as usize;
                    let dx = if i < movers { shift } else { 0.0 };
                    match classify_visibility_world(
                        &scene.camera, &scene.cascades, scene.world_aabb_shifted(i, dx),
                        scene.material[i], false, 1.0,
                    ) { Visibility::Culled => {} _ => drawn += 1 }
                }
                std::hint::black_box(drawn)
            })
        });
    }
    g.finish();
}

criterion_group!(benches, bench_linear, bench_linear_full, bench_indexed, bench_build, bench_masked, bench_parts, bench_scaling);
criterion_main!(benches);
