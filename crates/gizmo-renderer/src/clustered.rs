//! Clustered light culling: which lights can reach which part of the frame.
//!
//! The frame's view volume is cut into a grid of **clusters** — `x`×`y` screen tiles by `z`
//! exponentially-spaced depth slices — and each cluster gets the list of lights whose sphere of
//! influence touches it. A fragment then loops over its own cluster's list (typically a handful)
//! instead of every light in the scene, which is what makes a light count above a couple of dozen
//! affordable at all.
//!
//! # Why the assignment is on the CPU
//!
//! The textbook implementation builds cluster bounds and assigns lights in compute shaders (Doom
//! 2016). This does it on the CPU, deliberately, as the first implementation:
//!
//! * It is a **pure function**, so every rule in it is unit-testable with no GPU, no adapter and no
//!   pixel readback — which is how the rest of this engine's correctness is held, and clustering has
//!   exactly the kind of index arithmetic that silently half-works.
//! * The cost is **measured, not assumed** (release, this machine, 16×9×24 = 3456 clusters, lights
//!   of radius 6 spread through the volume):
//!
//!   | lights | ms/frame | (cluster, light) pairs |
//!   |---|---|---|
//!   | 8 | 0.047 | 161 |
//!   | 32 | 0.106 | 439 |
//!   | 64 | 0.201 | 843 |
//!   | 128 | 0.469 | 1638 |
//!   | 256 | 0.764 | 3180 |
//!
//!   0.1–0.2 ms at the light counts a scene actually has today, and 0.76 ms at 256 — which is 4.6 %
//!   of a 60 Hz frame and therefore the number that justifies a compute-shader build *when* a scene
//!   needs that many, rather than now.
//! * The upload is two storage buffers of a few tens of kilobytes.
//!
//! A compute-shader build is the follow-up, and its trigger is a measurement: this assignment
//! showing up in a frame profile. Until then the simpler version that can be *tested* wins.
//!
//! # The mapping the shader must agree with
//!
//! Both sides compute the same cluster index from a fragment's world position, and neither knows
//! the other's code, so the agreement is by construction:
//!
//! * **tile** — `uv = ndc.xy * 0.5 + 0.5`, `tile = floor(uv * dims.xy)`. Cluster `i` therefore spans
//!   `uv.x ∈ [i/x, (i+1)/x]`, which is exactly where this module puts its corner rays.
//! * **slice** — `floor(log(d) * z_scale + z_bias)` over the **view depth**
//!   `d = dot(world_pos - camera_pos, camera_forward)`, with the two constants from
//!   [`depth_params`]. Exponential rather than linear because a linear slicing spends most of its
//!   slices where there is nothing and lumps the whole near field into one.
//!
//! Nothing here depends on the projection matrix's depth convention (`0..1` vs `-1..1`, reverse-Z):
//! the only matrix operation is unprojecting NDC corners with `inv(view_proj)`, and depth comes from
//! a dot product with the camera's own forward vector.

use gizmo_math::{Aabb, Mat4, Vec3, Vec3A};

/// How many clusters the view volume is cut into.
///
/// The default is 16×9×24 = 3456, the shape the technique is usually shipped with: tiles that stay
/// roughly square at 16:9, and enough depth slices that a slice is thin where geometry is dense
/// (near the camera) without wasting slices on the far field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClusterGrid {
    /// Screen tiles across.
    pub x: u32,
    /// Screen tiles down.
    pub y: u32,
    /// Depth slices.
    pub z: u32,
}

impl Default for ClusterGrid {
    fn default() -> Self {
        Self { x: 16, y: 9, z: 24 }
    }
}

impl ClusterGrid {
    /// Total number of clusters.
    #[inline]
    pub fn count(&self) -> usize {
        (self.x as usize) * (self.y as usize) * (self.z as usize)
    }

    /// Flat index of a cluster, in the order the shader computes it: `x + y*X + z*X*Y`.
    #[inline]
    pub fn index(&self, x: u32, y: u32, z: u32) -> usize {
        (x + y * self.x + z * self.x * self.y) as usize
    }
}

/// The most lights one cluster will carry.
///
/// A bound is required (the index buffer is sized from it), and it is a *per-cluster* bound rather
/// than a scene-wide one — which is the whole point of clustering: a scene may hold hundreds of
/// lights as long as no single cluster is inside more than this many of them. Overflow drops the
/// lights that come last in [`assign_lights`]'s input order, which is the ranked order
/// `collect_scene_lights` produced, so what is dropped is what mattered least.
pub const MAX_LIGHTS_PER_CLUSTER: usize = 32;

/// Bytes the cluster table needs: one `(offset, count)` pair per cluster.
pub fn table_bytes(grid: ClusterGrid) -> u64 {
    (grid.count() * std::mem::size_of::<[u32; 2]>()) as u64
}

/// Bytes the light-index list needs, at its **worst case** — every cluster full.
///
/// Sized for the worst case on purpose: the buffers are then allocated once, the bind group never
/// has to be rebuilt mid-frame because a list grew, and there is no truncation path to get wrong.
/// At the default grid that is 3456 × 32 × 4 = 442 KB of VRAM, which is not worth a cleverer
/// scheme.
pub fn index_bytes(grid: ClusterGrid) -> u64 {
    (grid.count() * MAX_LIGHTS_PER_CLUSTER * std::mem::size_of::<u32>()) as u64
}

/// A light, as this module needs it: a sphere of influence in world space.
#[derive(Debug, Clone, Copy)]
pub struct LightSphere {
    /// World-space centre.
    pub center: Vec3,
    /// Radius past which the light contributes nothing. The lighting shaders window their
    /// attenuation with `clamp(1 - (d/r)^4, 0, 1)`, so this is exact rather than a heuristic.
    pub radius: f32,
}

/// The camera, as this module needs it.
#[derive(Debug, Clone, Copy)]
pub struct ClusterView {
    /// The frame's view-projection — the same matrix the fragment shader will project with.
    pub view_proj: Mat4,
    /// World-space camera position.
    pub camera_pos: Vec3,
    /// Normalised camera forward; `dot(p - camera_pos, forward)` is the view depth.
    pub forward: Vec3,
    /// Near plane, > 0.
    pub near: f32,
    /// Far plane, > near.
    pub far: f32,
}

/// One frame's light lists.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClusterAssignment {
    /// `(offset, count)` into [`indices`](Self::indices) for every cluster, in
    /// [`ClusterGrid::index`] order. Length is always `grid.count()`.
    pub table: Vec<[u32; 2]>,
    /// Light indices — positions in the `lights` slice handed to [`assign_lights`].
    pub indices: Vec<u32>,
    /// How many (cluster, light) pairs were dropped because a cluster was already full.
    ///
    /// Reported rather than silently swallowed: a frame that drops assignments is a frame whose
    /// lighting is wrong somewhere, and the caller can say so once instead of the renderer looking
    /// mysteriously dim.
    pub dropped: u32,
}

/// The two constants the shader needs to turn a view depth into a slice index.
///
/// `slice = floor(log(d) * scale + bias)`, clamped to `0 ..= grid.z - 1`. Derived from the
/// exponential slicing `d_k = near * (far/near)^(k/z)`, which this module also uses, so the two
/// cannot disagree about where a slice begins.
pub fn depth_params(grid: ClusterGrid, near: f32, far: f32) -> [f32; 2] {
    let ratio = (far / near).max(1.000_001);
    let scale = grid.z as f32 / ratio.ln();
    let bias = -(near.ln() * scale);
    [scale, bias]
}

/// View depth at which slice `k` begins.
#[inline]
fn slice_depth(grid: ClusterGrid, near: f32, far: f32, k: u32) -> f32 {
    near * (far / near).powf(k as f32 / grid.z as f32)
}

/// The slice a view depth falls in, by the same formula the shader uses.
#[inline]
pub fn slice_of_depth(grid: ClusterGrid, near: f32, far: f32, depth: f32) -> u32 {
    let [scale, bias] = depth_params(grid, near, far);
    let raw = depth.max(near).ln() * scale + bias;
    (raw.floor().max(0.0) as u32).min(grid.z.saturating_sub(1))
}

/// The cluster a world position belongs to — the **CPU half of the mirror**.
///
/// Byte-for-byte the arithmetic `gizmo_cluster_index` in `common.wgsl` performs, and it exists so
/// that agreement can be *tested* rather than reasoned about: the shader decides which cluster a
/// fragment reads, this module decides which clusters a light is written to, and a disagreement of
/// one tile or one slice is invisible in any scene whose lights are large enough to be assigned to
/// the neighbours too. Two guards use it — a pure one (a tiny light lands in exactly this cluster)
/// and a GPU one (the shader, evaluated on the same points, returns exactly this).
///
/// Note what it is *not*: the assignment does not call this. Lights are boxes, not points, so they
/// are tested against cluster bounds; this is the point-sized case, which is what a fragment is.
pub fn cluster_of_point(grid: ClusterGrid, view: ClusterView, world_pos: Vec3) -> u32 {
    let clip = view.view_proj * gizmo_math::Vec4::new(world_pos.x, world_pos.y, world_pos.z, 1.0);
    let inv_w = if clip.w.abs() < 1e-9 { 1.0 } else { 1.0 / clip.w };
    let uv_x = (clip.x * inv_w * 0.5 + 0.5).clamp(0.0, 0.999_999);
    let uv_y = (clip.y * inv_w * 0.5 + 0.5).clamp(0.0, 0.999_999);
    let tile_x = ((uv_x * grid.x as f32) as u32).min(grid.x - 1);
    let tile_y = ((uv_y * grid.y as f32) as u32).min(grid.y - 1);

    let depth = (world_pos - view.camera_pos).dot(view.forward).max(1e-6);
    let [scale, bias] = depth_params(grid, view.near, view.far);
    let raw = depth.ln() * scale + bias;
    let slice = (raw.floor().max(0.0) as u32).min(grid.z - 1);

    tile_x + tile_y * grid.x + slice * grid.x * grid.y
}

/// Assign `lights` to the clusters of `view`.
///
/// Conservative in the only direction a cull may be: a light is kept for every cluster it *might*
/// touch. Cluster bounds are the world-space AABB of the cluster's eight corners, which contains
/// the cluster and a little more, and a sphere is tested against that AABB.
///
/// Lights are visited in the order given and each cluster keeps the first
/// [`MAX_LIGHTS_PER_CLUSTER`] that reach it, so the caller's ordering decides what survives an
/// overfull cluster.
pub fn assign_lights(
    grid: ClusterGrid,
    view: ClusterView,
    lights: &[LightSphere],
) -> ClusterAssignment {
    let cluster_count = grid.count();
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); cluster_count];
    let mut dropped = 0u32;

    // Corner rays: one per tile corner, so (x+1)·(y+1) of them, each a unit direction from the
    // camera through that corner of the screen. Unprojecting NDC is the only place a projection
    // matrix is consulted, and it is consulted the same way for every corner.
    let inv = view.view_proj.inverse();
    let unproject = |ndc_x: f32, ndc_y: f32| -> Vec3 {
        // z = 1 is the far plane under every convention wgpu supports; the ray is what matters,
        // not the point, so which plane it lands on does not.
        let p = inv * gizmo_math::Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
        if p.w.abs() < 1e-9 {
            return view.forward;
        }
        let world = Vec3::new(p.x / p.w, p.y / p.w, p.z / p.w);
        (world - view.camera_pos).normalize_or_zero()
    };
    let mut rays = Vec::with_capacity(((grid.x + 1) * (grid.y + 1)) as usize);
    for j in 0..=grid.y {
        for i in 0..=grid.x {
            let ndc_x = (i as f32 / grid.x as f32) * 2.0 - 1.0;
            let ndc_y = (j as f32 / grid.y as f32) * 2.0 - 1.0;
            rays.push(unproject(ndc_x, ndc_y));
        }
    }
    let ray = |i: u32, j: u32| rays[(i + j * (grid.x + 1)) as usize];

    // A corner ray, walked out to a given view depth. Dividing by `dot(dir, forward)` is what makes
    // this the point at that *depth* rather than at that distance — the difference is the cosine at
    // the edge of a wide field of view, and getting it wrong tilts every cluster bound outward.
    let at_depth = |dir: Vec3, depth: f32| -> Vec3A {
        let cos = dir.dot(view.forward);
        let t = if cos.abs() < 1e-6 { depth } else { depth / cos };
        Vec3A::from(view.camera_pos + dir * t)
    };

    for (light_index, light) in lights.iter().enumerate() {
        if !light.center.is_finite() || !light.radius.is_finite() || light.radius <= 0.0 {
            continue;
        }
        let depth = (light.center - view.camera_pos).dot(view.forward);
        let near_d = depth - light.radius;
        let far_d = depth + light.radius;
        // Entirely behind the camera or entirely past the far plane: no cluster can see it.
        if far_d <= 0.0 || near_d >= view.far {
            continue;
        }
        let z0 = slice_of_depth(grid, view.near, view.far, near_d.max(view.near));
        let z1 = slice_of_depth(grid, view.near, view.far, far_d.min(view.far));

        let center = Vec3A::from(light.center);
        for z in z0..=z1 {
            let d0 = slice_depth(grid, view.near, view.far, z);
            let d1 = slice_depth(grid, view.near, view.far, z + 1);
            for j in 0..grid.y {
                for i in 0..grid.x {
                    // The cluster's eight corners, as an AABB. Built per cluster rather than cached
                    // because the cache would be 3456 AABBs rebuilt every frame anyway, and this
                    // way the bound cannot go stale against a moved camera.
                    let mut bounds = Aabb::empty();
                    for (ci, cj) in [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)] {
                        let dir = ray(ci, cj);
                        bounds.extend(at_depth(dir, d0));
                        bounds.extend(at_depth(dir, d1));
                    }
                    if !sphere_touches_aabb(center, light.radius, bounds) {
                        continue;
                    }
                    let list = &mut lists[grid.index(i, j, z)];
                    if list.len() < MAX_LIGHTS_PER_CLUSTER {
                        list.push(light_index as u32);
                    } else {
                        dropped += 1;
                    }
                }
            }
        }
    }

    let mut table = Vec::with_capacity(cluster_count);
    let mut indices = Vec::new();
    for list in &lists {
        table.push([indices.len() as u32, list.len() as u32]);
        indices.extend_from_slice(list);
    }
    ClusterAssignment { table, indices, dropped }
}

/// Closest-point sphere/AABB overlap test.
#[inline]
fn sphere_touches_aabb(center: Vec3A, radius: f32, aabb: Aabb) -> bool {
    let closest = center.max(aabb.min).min(aabb.max);
    (closest - center).length_squared() <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(near: f32, far: f32) -> ClusterView {
        // Camera at the origin looking down -Z, the convention the engine's cameras use.
        let eye = Vec3::ZERO;
        let forward = Vec3::new(0.0, 0.0, -1.0);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 16.0 / 9.0, near, far);
        let view_mat = Mat4::look_at_rh(eye, eye + forward, Vec3::Y);
        ClusterView {
            view_proj: proj * view_mat,
            camera_pos: eye,
            forward,
            near,
            far,
        }
    }

    fn total(a: &ClusterAssignment) -> usize {
        a.table.iter().map(|[_, c]| *c as usize).sum()
    }

    /// The table always covers the grid, and an empty scene fills no cluster.
    #[test]
    fn no_lights_means_no_assignments() {
        let grid = ClusterGrid::default();
        let a = assign_lights(grid, view(0.1, 100.0), &[]);
        assert_eq!(a.table.len(), grid.count());
        assert!(a.indices.is_empty());
        assert_eq!(total(&a), 0);
        assert_eq!(a.dropped, 0);
    }

    /// The slice mapping the shader uses must invert this module's own slice boundaries.
    ///
    /// This is the agreement clustering lives or dies on: the CPU decides *which* cluster a light
    /// goes in and the shader decides *which* cluster a fragment reads, and if the two disagree by
    /// one slice the lighting is subtly wrong in a way no single frame makes obvious.
    #[test]
    fn the_slice_mapping_inverts_the_slice_boundaries() {
        let grid = ClusterGrid::default();
        let (near, far) = (0.1f32, 500.0);
        for k in 0..grid.z {
            let start = slice_depth(grid, near, far, k);
            // Just inside the slice, and just before its end.
            let end = slice_depth(grid, near, far, k + 1);
            let inside = start * 1.000_01;
            let before_end = end * 0.999_99;
            assert_eq!(slice_of_depth(grid, near, far, inside), k, "start of slice {k}");
            assert_eq!(slice_of_depth(grid, near, far, before_end), k, "end of slice {k}");
        }
        // Out of range clamps rather than indexing past the grid.
        assert_eq!(slice_of_depth(grid, near, far, 0.0), 0);
        assert_eq!(slice_of_depth(grid, near, far, -5.0), 0);
        assert_eq!(slice_of_depth(grid, near, far, far * 10.0), grid.z - 1);
    }

    /// A small light in front of the camera lands in a handful of clusters, not all of them —
    /// which is the entire claim of the technique.
    #[test]
    fn a_small_light_touches_few_clusters() {
        let grid = ClusterGrid::default();
        let a = assign_lights(
            grid,
            view(0.1, 100.0),
            &[LightSphere { center: Vec3::new(0.0, 0.0, -10.0), radius: 1.0 }],
        );
        let touched = total(&a);
        assert!(touched > 0, "a light 10 units in front of the camera must reach some cluster");
        assert!(
            touched < grid.count() / 10,
            "a 1-unit light reached {touched} of {} clusters — the cull is not culling",
            grid.count()
        );
        // And the clusters it reached are around the centre of the screen, where it is.
        for z in 0..grid.z {
            for j in 0..grid.y {
                for i in 0..grid.x {
                    if a.table[grid.index(i, j, z)][1] > 0 {
                        assert!(
                            (i as i32 - grid.x as i32 / 2).abs() <= 2
                                && (j as i32 - grid.y as i32 / 2).abs() <= 2,
                            "cluster ({i},{j},{z}) is far from screen centre but got the light"
                        );
                    }
                }
            }
        }
    }

    /// A point-sized light lands in exactly the cluster its own coordinates name.
    ///
    /// This is the geometry half of the mirror: [`assign_lights`] reaches its answer from cluster
    /// *bounds* and [`cluster_of_point`] from the *arithmetic* the shader uses, and if those two
    /// disagree the lighting is wrong in a way no single frame shows. Sampled across the volume
    /// rather than at one point, because a boundary case is exactly where an off-by-one lives.
    #[test]
    fn a_point_light_lands_in_the_cluster_its_coordinates_name() {
        let grid = ClusterGrid::default();
        let v = view(0.1, 200.0);
        // Spread through the volume, including off-centre and deep, and deliberately near tile
        // boundaries (the 0.37/0.63 fractions land inside tiles; ±5.9 is close to the frustum edge).
        let mut checked = 0;
        for depth in [0.5f32, 1.0, 3.7, 9.0, 25.0, 60.0, 150.0] {
            for (fx, fy) in [(0.0f32, 0.0f32), (0.37, -0.2), (-0.63, 0.44), (0.9, 0.9)] {
                // At this depth the half-extent of the view is depth*tan(fov/2) = depth (fov 90°),
                // scaled by aspect in x.
                let p = Vec3::new(fx * depth * (16.0 / 9.0), fy * depth, -depth);
                let want = cluster_of_point(grid, v, p);
                let a = assign_lights(grid, v, &[LightSphere { center: p, radius: 0.02 }]);
                let got: Vec<usize> = a
                    .table
                    .iter()
                    .enumerate()
                    .filter(|(_, [_, c])| *c > 0)
                    .map(|(i, _)| i)
                    .collect();
                assert!(
                    got.contains(&(want as usize)),
                    "a point light at {p:?} (depth {depth}) was assigned to {got:?} but a fragment \
                     there reads cluster {want}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 28, "the sample grid shrank — this guard is only as good as its spread");
    }

    /// A light that swallows the whole view volume is in every cluster. The conservative direction
    /// matters more than the tight one: a missed cluster is a visibly unlit patch.
    #[test]
    fn a_light_containing_the_view_reaches_every_cluster() {
        let grid = ClusterGrid::default();
        let a = assign_lights(
            grid,
            view(0.1, 100.0),
            &[LightSphere { center: Vec3::ZERO, radius: 1000.0 }],
        );
        assert_eq!(total(&a), grid.count(), "every cluster must carry the light");
        assert!(a.table.iter().all(|[_, c]| *c == 1));
    }

    /// A light behind the camera reaches nothing, and one past the far plane reaches nothing.
    #[test]
    fn lights_outside_the_view_volume_are_dropped_entirely() {
        let grid = ClusterGrid::default();
        let v = view(0.1, 100.0);
        let behind = assign_lights(
            grid,
            v,
            &[LightSphere { center: Vec3::new(0.0, 0.0, 50.0), radius: 5.0 }],
        );
        assert_eq!(total(&behind), 0, "a light behind the camera lights nothing");

        let beyond = assign_lights(
            grid,
            v,
            &[LightSphere { center: Vec3::new(0.0, 0.0, -500.0), radius: 5.0 }],
        );
        assert_eq!(total(&beyond), 0, "a light past the far plane lights nothing");

        // But one straddling the near plane is kept: it lights what the camera can see.
        let straddling = assign_lights(
            grid,
            v,
            &[LightSphere { center: Vec3::new(0.0, 0.0, -0.05), radius: 2.0 }],
        );
        assert!(total(&straddling) > 0, "a light around the near plane must be kept");
    }

    /// The offsets in the table describe the index buffer exactly — contiguous, in cluster order,
    /// and summing to its length. The shader indexes with them, so an off-by-one here reads another
    /// cluster's lights.
    #[test]
    fn the_table_describes_the_index_buffer() {
        let grid = ClusterGrid { x: 4, y: 4, z: 4 };
        let a = assign_lights(
            grid,
            view(0.1, 50.0),
            &[
                LightSphere { center: Vec3::new(0.0, 0.0, -5.0), radius: 3.0 },
                LightSphere { center: Vec3::new(2.0, 1.0, -20.0), radius: 8.0 },
                LightSphere { center: Vec3::ZERO, radius: 500.0 },
            ],
        );
        let mut expected_offset = 0u32;
        for (cluster, [offset, count]) in a.table.iter().enumerate() {
            assert_eq!(*offset, expected_offset, "cluster {cluster} offset is not contiguous");
            assert!(
                (*offset + *count) as usize <= a.indices.len(),
                "cluster {cluster} runs past the index buffer"
            );
            expected_offset += count;
        }
        assert_eq!(expected_offset as usize, a.indices.len());
        assert_eq!(total(&a), a.indices.len());
        // Every index names a real light.
        assert!(a.indices.iter().all(|&i| (i as usize) < 3));
    }

    /// An overfull cluster keeps the first lights and says how many it turned away.
    #[test]
    fn an_overfull_cluster_reports_what_it_dropped() {
        let grid = ClusterGrid { x: 1, y: 1, z: 1 };
        // Every light contains the whole (single-cluster) view volume.
        let lights: Vec<LightSphere> = (0..MAX_LIGHTS_PER_CLUSTER + 5)
            .map(|_| LightSphere { center: Vec3::ZERO, radius: 1000.0 })
            .collect();
        let a = assign_lights(grid, view(0.1, 50.0), &lights);
        assert_eq!(a.table[0][1] as usize, MAX_LIGHTS_PER_CLUSTER);
        assert_eq!(a.dropped, 5, "the five that did not fit must be reported");
        // The ones kept are the first, which is the caller's ranked order.
        assert_eq!(a.indices, (0..MAX_LIGHTS_PER_CLUSTER as u32).collect::<Vec<_>>());
    }

    /// Non-finite or zero-radius input is skipped rather than poisoning the assignment.
    #[test]
    fn garbage_lights_are_skipped() {
        let grid = ClusterGrid { x: 2, y: 2, z: 2 };
        let a = assign_lights(
            grid,
            view(0.1, 50.0),
            &[
                LightSphere { center: Vec3::new(f32::NAN, 0.0, -5.0), radius: 5.0 },
                LightSphere { center: Vec3::new(0.0, 0.0, -5.0), radius: f32::INFINITY },
                LightSphere { center: Vec3::new(0.0, 0.0, -5.0), radius: 0.0 },
            ],
        );
        assert_eq!(total(&a), 0);
    }
}


/// The GPU half of the mirror: does the shader's `gizmo_cluster_index` agree with
/// [`cluster_of_point`]?
///
/// Both halves of clustering are "obviously right" arithmetic in two languages, and the pixel guards
/// cannot separate them: measured 2026-08-17, adding `+ 1u` to the shader's slice left **every**
/// frame guard green, because a light large enough to see is a light assigned to the neighbouring
/// slices too. So the two implementations are evaluated on the same points and compared directly.
#[cfg(test)]
mod gpu_mirror {
    use super::*;
    use crate::frame_uniforms::{CameraFrame, EnvironmentFrame, SceneFrame, ShadowFrame, SunFrame};
    use crate::gpu_types::{LightData, SceneUniforms};

    /// The compute shader under test: it calls the production function, composed from the
    /// production module, on positions the test chooses.
    const MIRROR_WGSL: &str = r#"
#import gizmo::common::{SceneUniforms, gizmo_cluster_index}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;
@group(0) @binding(1) var<storage, read> points: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> out_cluster: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&points)) {
        return;
    }
    out_cluster[i] = gizmo_cluster_index(scene, points[i].xyz);
}
"#;

    fn scene_frame(view: ClusterView) -> SceneFrame {
        SceneFrame {
            camera: CameraFrame {
                view_proj: view.view_proj,
                position: view.camera_pos,
                forward: view.forward,
                near: view.near,
                far: view.far,
                exposure: 1.0,
            },
            sun: SunFrame { direction: Vec3::NEG_Y, color: [0.0; 4], present: false },
            lights: [LightData::default(); crate::frame_uniforms::MAX_LIGHTS],
            num_lights: 0,
            shadows: ShadowFrame {
                cascade_view_projs: [Mat4::IDENTITY; 4],
                cascade_splits: [0.0; 4],
                point_caster: None,
                point_shadows_enabled: false,
            },
            environment: EnvironmentFrame::default(),
            elapsed_time: 0.0,
        }
    }

    #[test]
    fn the_shader_and_the_cpu_agree_on_every_clusters_index() {
        let _gpu = crate::test_gpu::gpu_lock();
        let Some((device, queue)) = pollster::block_on(crate::test_gpu::headless_device()) else {
            eprintln!("skipping: no usable GPU adapter");
            return;
        };

        let grid = ClusterGrid::default();
        let eye = Vec3::new(3.0, 2.0, 7.0);
        let forward = Vec3::new(-0.3, -0.2, -1.0).normalize();
        let (near, far) = (0.1f32, 300.0);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_3, 16.0 / 9.0, near, far);
        let view = ClusterView {
            view_proj: proj * Mat4::look_at_rh(eye, eye + forward, Vec3::Y),
            camera_pos: eye,
            forward,
            near,
            far,
        };

        // Sample the INTERIOR of clusters: the centre of a tile, at the geometric centre of a slice.
        //
        // Not the boundaries, and that is a measurement decision rather than a weaker test. A point
        // sitting exactly on a tile edge can land either side of it, because the two sides reach
        // `uv` through different float arithmetic (glam's matrix multiply and the GPU's are both
        // correct and not bit-identical) — an earlier version of this test sampled the frustum edges
        // and reported 13 of 97 points off by one *tile*, all of them boundary cases. Boundary
        // jitter is harmless by construction: cluster bounds are the AABB of a cluster's corners, so
        // adjacent clusters overlap slightly and a light on the seam is assigned to both. What must
        // not differ is the interior, and there the demand is exact equality.
        //
        // Points are produced by unprojecting, the same way `assign_lights` builds its corner rays,
        // rather than from a hand-written tan() — the projection is the thing under test, so it
        // should not appear twice in different spellings.
        let inv = view.view_proj.inverse();
        let world_at = |uv_x: f32, uv_y: f32, depth: f32| -> Vec3 {
            let ndc = gizmo_math::Vec4::new(uv_x * 2.0 - 1.0, uv_y * 2.0 - 1.0, 1.0, 1.0);
            let p = inv * ndc;
            let dir = (Vec3::new(p.x / p.w, p.y / p.w, p.z / p.w) - eye).normalize();
            eye + dir * (depth / dir.dot(forward))
        };
        let mut points: Vec<[f32; 4]> = Vec::new();
        for k in 0..grid.z {
            // Geometric centre of the slice, which is the middle of an exponentially-sliced range.
            let depth = (slice_depth(grid, near, far, k) * slice_depth(grid, near, far, k + 1))
                .sqrt();
            for (ti, tj) in [(0u32, 0u32), (grid.x / 2, grid.y / 2), (grid.x - 1, grid.y - 1), (1, grid.y - 2)] {
                let p = world_at(
                    (ti as f32 + 0.5) / grid.x as f32,
                    (tj as f32 + 0.5) / grid.y as f32,
                    depth,
                );
                points.push([p.x, p.y, p.z, 0.0]);
            }
        }
        // And one behind the camera: its cluster must still be a legal index rather than garbage.
        points.push([eye.x, eye.y, eye.z + 10.0, 0.0]);
        let count = points.len();

        // Uniform: the real `SceneUniforms`, so the shader reads the same cluster fields the
        // renderer uploads (derived from the camera, not hand-written here).
        let uniforms = SceneUniforms::new(&scene_frame(view));
        let uniform_buffer = wgpu::util::DeviceExt::create_buffer_init(
            &device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("mirror_scene"),
                contents: bytemuck::cast_slice(&[uniforms]),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let point_buffer = wgpu::util::DeviceExt::create_buffer_init(
            &device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("mirror_points"),
                contents: bytemuck::cast_slice(&points),
                usage: wgpu::BufferUsages::STORAGE,
            },
        );
        let out_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mirror_out"),
            size: (count * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mirror_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1, true),
                storage(2, false),
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mirror_bg"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: point_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: out_buffer.as_entire_binding() },
            ],
        });

        // Composed the way the renderer composes its own shaders, so the function under test is the
        // one that ships — not a copy pasted into the test.
        let module = crate::pipeline::shaders::load_shader_composed(
            &device,
            "",
            MIRROR_WGSL,
            "cluster_mirror",
        );
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mirror_pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("mirror_pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("mirror_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(count.div_ceil(64) as u32, 1, 1);
        }
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mirror_readback"),
            size: (count * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&out_buffer, 0, &staging, 0, (count * 4) as u64);
        queue.submit(Some(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| {
            let _ = tx.send(v);
        });
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().unwrap().unwrap();
        // wgpu 30 made this fallible; the range is the whole buffer just mapped, so a failure is a
        // programming error rather than a runtime condition.
        let data = slice.get_mapped_range().expect("mapped range");
        let from_shader: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&data).to_vec();
        drop(data);
        staging.unmap();

        assert_eq!(from_shader.len(), count);
        let mut mismatches = Vec::new();
        for (i, p) in points.iter().enumerate() {
            let world = Vec3::new(p[0], p[1], p[2]);
            let want = cluster_of_point(grid, view, world);
            if from_shader[i] != want {
                mismatches.push(format!(
                    "point {i} {world:?}: shader {} vs cpu {want}",
                    from_shader[i]
                ));
            }
        }
        assert!(
            mismatches.is_empty(),
            "{} of {count} sample points land in different clusters on the two sides:\n  {}",
            mismatches.len(),
            mismatches.join("\n  ")
        );
        assert!(
            from_shader.iter().all(|&c| (c as usize) < grid.count()),
            "the shader produced a cluster index outside the grid"
        );
    }
}
