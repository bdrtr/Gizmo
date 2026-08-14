//! Polygon navigation mesh: a Recast-style build from static physics colliders, plus queries
//! over the resulting convex-polygon graph.
//!
//! [`NavMesh::build_from_physics`] runs five stages:
//! 1. **Voxelisation** — rasterise the world AABB of every non-dynamic collider into a flat
//!    XZ cell grid and mark those cells blocked, plus an `agent_radius` skirt around them.
//! 2. **Region building** — 4-way flood fill over the unblocked cells inside the configured
//!    world bounds; regions smaller than four cells are dropped.
//! 3. **Polygon generation** — greedily carve each region into axis-aligned rectangles, one
//!    [`NavPoly`] per rectangle.
//! 4. **Adjacency graph** — link polygons whose edges coincide or partially overlap.
//! 5. **Query** — [`NavMesh::find_path`] runs A* over that polygon graph.
//!
//! The representation is 2.5D at best: the grid is flat in XZ and each polygon carries a
//! single Y, so overlapping floors, ramps and slope limits are **not** modelled — the
//! `agent_height` and `max_slope` settings are carried on the mesh but never enforced.
//! `agent_radius` is the exception: voxelisation erodes the walkable area by it, so lateral
//! clearance IS modelled (to whole-cell resolution). There is no funnel / string-pulling stage
//! either; paths come out as the midpoints of the shared edges they cross.
//!
//! This crate's own AI navigation systems in [`crate::system`] drive the simpler grid in
//! [`crate::pathfinding`], not this module.

use gizmo_math::Vec3;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet};

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// One convex walkable polygon of a [`NavMesh`], in world space (metres).
///
/// Every field is plain data and none of them is recomputed from the others, so a
/// hand-built polygon has to keep `center`, `normal` and `area` consistent with `vertices`
/// itself. [`NavMesh::build_from_physics`] only ever emits axis-aligned rectangles lying flat
/// in XZ.
#[derive(Debug, Clone)]
pub struct NavPoly {
    /// Identity of this polygon inside its mesh, and the key the adjacency graph is
    /// expressed in. [`NavMesh::find_path`] resolves neighbours through an id → polygon map,
    /// so ids must be unique within one mesh or links silently resolve to the wrong polygon.
    /// A built mesh numbers its polygons `0..polygons.len()` in `polygons` order.
    pub id: u32,
    /// Corner positions in world metres, in boundary order (each consecutive pair, plus the
    /// wrap-around pair, is an edge).
    ///
    /// A built mesh emits exactly four corners, all sharing one Y, walking the rectangle as
    /// (min x, min z) → (max x, min z) → (max x, max z) → (min x, max z). Winding is not
    /// relied upon: [`NavPoly::contains_point_xz`] accepts either. Fewer than three vertices
    /// makes the polygon degenerate — containment then always answers `false`.
    pub vertices: Vec<Vec3>,
    /// Representative interior point in world metres — the rectangle centre for a built
    /// polygon.
    ///
    /// This is the position the polygon graph is searched on: A* step cost is the
    /// centre-to-centre distance between neighbours, and the heuristic is the distance from
    /// a centre to the goal, so a centre far off the polygon's actual middle distorts route
    /// choice.
    pub center: Vec3,
    /// Adjacency links as `(neighbour id, [edge_start, edge_end])`, where the two points are
    /// the shared — or, after greedy merging, merely overlapping — portion of the boundary in
    /// world metres.
    ///
    /// Links are stored symmetrically: both polygons list each other, with the *same* pair of
    /// endpoints, so the segment is not oriented with respect to either owner. The midpoint
    /// of this segment is what [`NavMesh::find_path`] emits as a waypoint when crossing here.
    pub neighbors: Vec<(u32, [Vec3; 2])>, // (neighbor_id, [edge_start, edge_end])
    /// Unit up-direction of the walkable surface. A built polygon is always flat, so this is
    /// always `(0, 1, 0)`; it is descriptive only — no query in this module reads it, and in
    /// particular neither containment nor pathfinding consults it or `NavMesh::max_slope`.
    pub normal: Vec3,
    /// Surface area in square metres, stored rather than derived. Built polygons set it to
    /// the rectangle's width × depth; [`NavMesh::stats`] sums it to report navigable area, so
    /// a stale value only misreports statistics — no query uses it.
    pub area: f32,
}

impl NavPoly {
    /// Tests containment in the XZ plane only — `point.y` is ignored entirely, so a point far
    /// above or below the polygon still counts as inside.
    ///
    /// Valid for **convex** polygons: it asks that the point lie on the same side of every
    /// edge, which accepts either winding, and counts a point exactly on an edge or vertex as
    /// inside (so two polygons sharing an edge both claim points on it). A concave polygon
    /// fails the test almost everywhere, including at genuinely interior points. Returns
    /// `false` for fewer than three vertices.
    pub fn contains_point_xz(&self, point: Vec3) -> bool {
        let n = self.vertices.len();
        if n < 3 {
            return false;
        }

        // Convex containment that works for EITHER winding: the point must lie on
        // the same side of every edge, i.e. all edge cross products share a sign
        // (zeros = on the edge line, treated as inside). The mesh emits CCW quads,
        // but the old `cross < 0 => outside` test silently failed for a CW polygon
        // (every interior point looked outside) — and the NavPoly doc even claimed
        // CW, so a caller following it would have been bitten.
        let mut sign = 0i32;
        for i in 0..n {
            let a = self.vertices[i];
            let b = self.vertices[(i + 1) % n];
            let cross = (b.x - a.x) * (point.z - a.z) - (b.z - a.z) * (point.x - a.x);
            if cross > 0.0 {
                if sign < 0 {
                    return false;
                }
                sign = 1;
            } else if cross < 0.0 {
                if sign > 0 {
                    return false;
                }
                sign = -1;
            }
        }
        true
    }

    /// Axis-aligned bounds of the vertices as `(min, max)` in world metres, over all three
    /// axes — the Y span is real, not collapsed, even though queries work in XZ.
    ///
    /// A polygon with no vertices yields the inverted sentinel box
    /// `(f32::MAX…, f32::MIN…)`, which intersects nothing; callers must not assume
    /// `min <= max`.
    pub fn aabb(&self) -> (Vec3, Vec3) {
        let mut min = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
        let mut max = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
        for v in &self.vertices {
            min.x = min.x.min(v.x);
            min.y = min.y.min(v.y);
            min.z = min.z.min(v.z);
            max.x = max.x.max(v.x);
            max.y = max.y.max(v.y);
            max.z = max.z.max(v.z);
        }
        (min, max)
    }
}

/// A navigable surface as a graph of convex polygons, plus the settings it was built with.
///
/// Immutable once built: there is no incremental update, so a scene whose static geometry
/// changes needs a fresh [`NavMesh::build_from_physics`]. All queries are read-only and take
/// `&self`, so a built mesh can be shared freely across threads.
#[derive(Debug, Clone)]
pub struct NavMesh {
    /// Every polygon of the mesh, in generation order. Adjacency is expressed through
    /// [`NavPoly::id`], not through this index — the two agree only because a built mesh
    /// numbers polygons consecutively. Empty when the world had no walkable region of at
    /// least four cells, in which case every query returns `None`.
    pub polygons: Vec<NavPoly>,
    /// Voxel edge length in metres used during the build, copied from the config. Also the
    /// tolerance for deciding that two polygon edges coincide, so it bounds the finest
    /// detail the mesh can represent: gaps narrower than one cell close up, and obstacles are
    /// conservatively INFLATED outward to whole cells rather than dropped (voxelisation floors
    /// the AABB min and ceils the max, so a sub-cell obstacle still blocks at least one cell).
    pub cell_size: f32,
    /// Intended agent height in metres, carried from the config for the caller's benefit.
    /// **Not enforced** — the builder is 2D and performs no headroom or ceiling test, so a
    /// polygon may sit under an overhang lower than this.
    pub agent_height: f32,
    /// Agent radius in metres, carried from the config and **enforced** by the build: every
    /// obstacle's blocked cells are grown by `ceil(radius / cell_size)` cells, so no polygon
    /// comes within that many cells of an obstacle's AABB. Clearance is therefore quantised
    /// upwards to whole cells — the real gap is at least the radius, and up to one cell more.
    ///
    /// Through 0.9.0 this was descriptive only and polygons ran right up to the AABB.
    pub agent_radius: f32,
    /// Maximum walkable incline in **radians**, converted from the config's degrees.
    /// Descriptive only: the builder emits flat, +Y-facing polygons and no stage of build or
    /// query compares a surface against it.
    pub max_slope: f32,
    /// Wall-clock duration of the [`NavMesh::build_from_physics`] call that produced this
    /// mesh, in milliseconds. Diagnostic only; meaningless for a hand-assembled mesh.
    pub build_time_ms: f64,
}

/// Build settings for [`NavMesh::build_from_physics`].
///
/// [`Default`] gives 0.5 m cells, a 2.0 m × 0.5 m agent, a 45° slope limit and a
/// ±100 m world box in XZ.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NavMeshConfig {
    /// Voxel edge length in metres (default `0.5`). Must be greater than zero — it divides
    /// world coordinates. This is the accuracy/cost dial: the whole `world_min..world_max`
    /// box is enumerated cell by cell, so halving it quadruples the cell count and also
    /// tends to split the level into more polygons. See [`NavMesh::build_from_physics`] for
    /// what dominates build time.
    pub cell_size: f32,
    /// Agent height in metres (default `2.0`). Recorded on the built mesh but never checked;
    /// see [`NavMesh::agent_height`].
    pub agent_height: f32,
    /// Agent radius in metres (default `0.5`). Erodes the walkable area: each obstacle blocks
    /// its own cells plus a `ceil(radius / cell_size)`-cell skirt, so an agent that stays
    /// inside polygon interiors keeps its body clear of walls.
    ///
    /// It also positions the band whose floor height is sampled — that band is the same width
    /// and now sits just outside the eroded skirt, so it still supplies the Y of the polygons
    /// nearest each obstacle. Note both effects are quantised to whole cells, so raising this
    /// above a multiple of `cell_size` costs a full extra cell of walkable area on every side
    /// of every obstacle.
    pub agent_radius: f32,
    /// Slope limit in **degrees** (default `45.0`), stored on the mesh as radians. Currently
    /// inert; see [`NavMesh::max_slope`].
    pub max_slope_degrees: f32,
    /// Lower corner in world metres of the box to voxelise (default `(-100, -10, -100)`).
    /// Only X and Z are read — the Y component is ignored, as the build is 2D.
    ///
    /// Geometry outside this box is still rasterised as an obstacle, but no walkable cell is
    /// ever created outside it, so the box is the hard limit of where agents can be routed.
    pub world_min: Vec3,
    /// Upper corner in world metres of the box to voxelise (default `(100, 50, 100)`), with
    /// the same XZ-only treatment as `world_min`.
    ///
    /// Every cell inside the box that no static collider covers becomes walkable — including
    /// cells with no floor beneath them. Sizing this box to the actual playable area matters
    /// for both correctness and build cost.
    pub world_max: Vec3,
}

impl Default for NavMeshConfig {
    fn default() -> Self {
        Self {
            cell_size: 0.5,
            agent_height: 2.0,
            agent_radius: 0.5,
            max_slope_degrees: 45.0,
            world_min: Vec3::new(-100.0, -10.0, -100.0),
            world_max: Vec3::new(100.0, 50.0, 100.0),
        }
    }
}

/// A 2D grid cell (the result of voxelisation)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct GridCell {
    x: i32,
    z: i32,
}

impl NavMesh {
    /// Voxelises the static geometry of `physics` and returns the navigable surface as a
    /// polygon graph.
    ///
    /// Only non-dynamic bodies contribute, and each contributes its
    /// world AABB rather than its actual shape — a sphere or a rotated box blocks its full
    /// bounding rectangle in XZ. Walkability is defined negatively: **every** cell of the
    /// `world_min..world_max` box that no such AABB covers is walkable, so a world with no
    /// static bodies comes out entirely navigable, and a hole in the floor stays navigable
    /// because nothing occupies it.
    ///
    /// Connected walkable cells are flood-filled with 4-way adjacency, so two areas touching
    /// only at a diagonal become separate regions; an isolated region of fewer than four
    /// cells is dropped altogether. Each surviving region is then carved greedily into
    /// axis-aligned rectangles starting from arbitrarily chosen cells, so the number and
    /// shape of the polygons follow the order those cells are visited in.
    ///
    /// **That order used to be a randomly-seeded `HashSet`'s**, which made the decomposition
    /// differ between two `build_from_physics` calls on the same world *inside one run* — ids,
    /// polygon shapes and therefore paths, all unreproducible. It cost a CI failure that could
    /// not be reproduced locally: `the_height_sampling_band_moves_outside_the_eroded_skirt`
    /// asked for the polygon covering a point and got one whose floor had been sampled from a
    /// cell near the obstacle (2.0) or from one that was not (0.0), depending on where the
    /// greedy expansion happened to start that run. The cell-keyed containers are `BTreeSet` /
    /// `BTreeMap` now, so a given world decomposes the same way every time. Ids are still not
    /// worth persisting across a *rebuild* — they follow the geometry, and the geometry follows
    /// the level — but they no longer change under you for no reason.
    ///
    /// Polygon Y is sampled, not computed: a rectangle takes the highest obstacle top surface
    /// recorded for the one cell the greedy expansion started from, and falls back to `0.0`
    /// when no static body was near that cell. Expect the wrong height on multi-level or
    /// stepped terrain.
    ///
    /// Runs single-threaded. Voxelisation and region building are linear in the number of
    /// cells of the configured world box — voxelisation walks each obstacle's footprint grown
    /// by twice the `agent_radius` skirt (the erosion, plus the height-sampling band beyond
    /// it), so a radius comparable to an obstacle's own size dominates that stage's cost for
    /// it. The adjacency stage that follows compares every
    /// pair of polygons unconditionally — recomputing one polygon's AABB inside the inner
    /// loop — so the overall cost is quadratic in the polygon count, which depends on how
    /// cluttered the level is and not only on the size of the box. Treat this as load-time
    /// work and measure it before calling it anywhere else. The elapsed time lands in
    /// [`NavMesh::build_time_ms`].
    #[tracing::instrument(skip_all, name = "navmesh_build")]
    pub fn build_from_physics(
        physics: &gizmo_physics_rigid::world::PhysicsWorld,
        config: &NavMeshConfig,
    ) -> Self {
        let start = Instant::now();
        let cell_size = config.cell_size;

        // 1. Voxelization: Statik collider'ları grid'e yaz
        let mut blocked: BTreeSet<GridCell> = BTreeSet::new();
        let mut walkable_y: BTreeMap<GridCell, f32> = BTreeMap::new();

        for i in 0..physics.entities.len() {
            let rb = &physics.rigid_bodies[i];
            if rb.is_dynamic() {
                continue;
            }

            let transform = &physics.transforms[i];
            let collider = &physics.colliders[i];

            let aabb = collider.compute_aabb(transform.position, transform.rotation);
            let min_x = (aabb.min.x / cell_size).floor() as i32;
            let max_x = (aabb.max.x / cell_size).ceil() as i32;
            let min_z = (aabb.min.z / cell_size).floor() as i32;
            let max_z = (aabb.max.z / cell_size).ceil() as i32;

            // Erozyon: engelin hücreleri ajan yarıçapı kadar dışa taşırılıp bloklanıyor.
            // Eskiden `margin` YALNIZ döngü sınırlarını genişletiyordu; `blocked.insert`
            // gerçek AABB'ye kapılı olduğu için `agent_radius` hiç clearance üretmiyordu ve
            // polygonlar duvarın dibine kadar geliyordu.
            let margin = (config.agent_radius / cell_size).ceil() as i32;
            let bx0 = min_x - margin;
            let bx1 = max_x + margin;
            let bz0 = min_z - margin;
            let bz1 = max_z + margin;

            // Yükseklik örnekleme bandı, erozyon sınırının DIŞINDA ve genişliği eskisiyle
            // aynı (`margin` hücre). Bandı yerinde bırakmak onu bloklu bölgenin içine
            // gömerdi: `walkable_y`'ye yazan tek yol bu band olduğu için her polygon
            // Y = 0.0 fallback'ine düşerdi.
            for x in (bx0 - margin)..=(bx1 + margin) {
                for z in (bz0 - margin)..=(bz1 + margin) {
                    let cell = GridCell { x, z };

                    if x >= bx0 && x <= bx1 && z >= bz0 && z <= bz1 {
                        blocked.insert(cell);
                        continue;
                    }

                    // Engelin üst yüzeyini yürünebilir zemin olarak kaydet
                    if !blocked.contains(&cell) {
                        let surface_y = aabb.max.y;
                        walkable_y
                            .entry(cell)
                            .and_modify(|y| *y = y.max(surface_y))
                            .or_insert(surface_y);
                    }
                }
            }
        }

        // 2. Region building: Yürünebilir hücreleri bağlı bölgelere ayır (flood fill)
        let world_min_x = (config.world_min.x / cell_size).floor() as i32;
        let world_max_x = (config.world_max.x / cell_size).ceil() as i32;
        let world_min_z = (config.world_min.z / cell_size).floor() as i32;
        let world_max_z = (config.world_max.z / cell_size).ceil() as i32;

        let mut all_walkable: BTreeSet<GridCell> = BTreeSet::new();
        for x in world_min_x..=world_max_x {
            for z in world_min_z..=world_max_z {
                let cell = GridCell { x, z };
                if !blocked.contains(&cell) {
                    all_walkable.insert(cell);
                }
            }
        }

        let mut visited: BTreeSet<GridCell> = BTreeSet::new();
        let mut regions: Vec<BTreeSet<GridCell>> = Vec::new();

        for &cell in &all_walkable {
            if visited.contains(&cell) {
                continue;
            }

            // Flood fill
            let mut region = BTreeSet::new();
            let mut stack = vec![cell];

            while let Some(current) = stack.pop() {
                if !all_walkable.contains(&current) || visited.contains(&current) {
                    continue;
                }
                visited.insert(current);
                region.insert(current);

                // 4-yön komşuluk
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let neighbor = GridCell {
                        x: current.x + dx,
                        z: current.z + dz,
                    };
                    if all_walkable.contains(&neighbor) && !visited.contains(&neighbor) {
                        stack.push(neighbor);
                    }
                }
            }

            if region.len() >= 4 {
                // Çok küçük bölgeleri atla
                regions.push(region);
            }
        }

        // Voxelizasyon + bölge oluşturma ara-detayı (build seyrek olduğundan debug! uygun).
        tracing::debug!(
            entity_count = physics.entities.len(),
            blocked_cells = blocked.len(),
            walkable_cells = all_walkable.len(),
            region_count = regions.len(),
            "[NavMesh] Voxelizasyon + bölge oluşturma tamamlandı"
        );

        // 3. Polygon generation: Her bölgeyi dikdörtgen polygon'lara böl (greedy merge)
        let mut polygons = Vec::new();
        let mut poly_id = 0u32;

        for region in &regions {
            let mut remaining: BTreeSet<GridCell> = region.clone();

            while !remaining.is_empty() {
                // İlk hücreyi al
                let start_cell = *remaining.iter().next().unwrap();

                // Greedy genişletme: Mümkün olduğunca büyük dikdörtgen bul
                let mut max_x = start_cell.x;
                let mut max_z = start_cell.z;

                // X yönünde genişlet
                while remaining.contains(&GridCell {
                    x: max_x + 1,
                    z: start_cell.z,
                }) {
                    max_x += 1;
                }

                // Z yönünde genişlet (tüm x satırı doluysa)
                'z_expand: loop {
                    for x in start_cell.x..=max_x {
                        if !remaining.contains(&GridCell { x, z: max_z + 1 }) {
                            break 'z_expand;
                        }
                    }
                    max_z += 1;
                }

                // Dikdörtgendeki hücreleri kaldır
                for x in start_cell.x..=max_x {
                    for z in start_cell.z..=max_z {
                        remaining.remove(&GridCell { x, z });
                    }
                }

                // Polygon oluştur
                let y = walkable_y.get(&start_cell).copied().unwrap_or(0.0);

                let min_world = Vec3::new(
                    start_cell.x as f32 * cell_size,
                    y,
                    start_cell.z as f32 * cell_size,
                );
                let max_world = Vec3::new(
                    (max_x + 1) as f32 * cell_size,
                    y,
                    (max_z + 1) as f32 * cell_size,
                );

                let vertices = vec![
                    Vec3::new(min_world.x, y, min_world.z),
                    Vec3::new(max_world.x, y, min_world.z),
                    Vec3::new(max_world.x, y, max_world.z),
                    Vec3::new(min_world.x, y, max_world.z),
                ];

                let center = Vec3::new(
                    (min_world.x + max_world.x) * 0.5,
                    y,
                    (min_world.z + max_world.z) * 0.5,
                );

                let w = max_world.x - min_world.x;
                let h = max_world.z - min_world.z;

                polygons.push(NavPoly {
                    id: poly_id,
                    vertices,
                    center,
                    neighbors: Vec::new(),
                    normal: Vec3::new(0.0, 1.0, 0.0),
                    area: w * h,
                });
                poly_id += 1;
            }
        }

        // 4. Adjacency graph: Komşuluk ilişkilerini kenar paylaşımıyla bul
        Self::build_adjacency(&mut polygons, cell_size);

        let build_time_ms = start.elapsed().as_secs_f64() * 1000.0;

        tracing::info!(
            polygon_count = polygons.len(),
            region_count = regions.len(),
            build_time_ms,
            "[NavMesh] Mesh oluşturuldu"
        );

        Self {
            polygons,
            cell_size,
            agent_height: config.agent_height,
            agent_radius: config.agent_radius,
            max_slope: config.max_slope_degrees.to_radians(),
            build_time_ms,
        }
    }

    /// Build the adjacency graph (detection of shared edges)
    fn build_adjacency(polygons: &mut [NavPoly], tolerance: f32) {
        let n = polygons.len();
        let mut adjacency: Vec<Vec<(u32, [Vec3; 2])>> = vec![Vec::new(); n];

        for i in 0..n {
            let (aabb_min_i, aabb_max_i) = polygons[i].aabb();
            for j in (i + 1)..n {
                let (aabb_min_j, aabb_max_j) = polygons[j].aabb();

                // Hızlı AABB ön test — uzak polygon'ları atla
                if aabb_max_i.x + tolerance < aabb_min_j.x
                    || aabb_max_j.x + tolerance < aabb_min_i.x
                    || aabb_max_i.z + tolerance < aabb_min_j.z
                    || aabb_max_j.z + tolerance < aabb_min_i.z
                {
                    continue;
                }

                // Kenar paylaşımı kontrolü
                if let Some(edge) =
                    Self::find_shared_edge(&polygons[i].vertices, &polygons[j].vertices, tolerance)
                {
                    let id_i = polygons[i].id;
                    let id_j = polygons[j].id;
                    adjacency[i].push((id_j, edge));
                    adjacency[j].push((id_i, edge));
                }
            }
        }

        for (i, neighbors) in adjacency.into_iter().enumerate() {
            polygons[i].neighbors = neighbors;
        }
    }

    /// Find the shared/overlapping edge between two polygons
    /// As a result of the greedy merge the edges may not match exactly — partial overlap is enough
    fn find_shared_edge(verts_a: &[Vec3], verts_b: &[Vec3], tolerance: f32) -> Option<[Vec3; 2]> {
        let tol_sq = tolerance * tolerance;

        for i in 0..verts_a.len() {
            let a1 = verts_a[i];
            let a2 = verts_a[(i + 1) % verts_a.len()];

            for j in 0..verts_b.len() {
                let b1 = verts_b[j];
                let b2 = verts_b[(j + 1) % verts_b.len()];

                // 1. Tam kenar eşleşmesi (her iki yönde)
                let match_1 =
                    (a1 - b1).length_squared() < tol_sq && (a2 - b2).length_squared() < tol_sq;
                let match_2 =
                    (a1 - b2).length_squared() < tol_sq && (a2 - b1).length_squared() < tol_sq;

                if match_1 || match_2 {
                    return Some([a1, a2]);
                }

                // 2. Kısmi kenar örtüşmesi: kenarlar aynı doğru üzerinde ve örtüşüyorsa
                if let Some(edge) = Self::check_edge_overlap(a1, a2, b1, b2, tolerance) {
                    return Some(edge);
                }
            }
        }
        None
    }

    /// Checks whether two edge segments are collinear and whether they overlap.
    fn check_edge_overlap(
        a1: Vec3,
        a2: Vec3,
        b1: Vec3,
        b2: Vec3,
        tolerance: f32,
    ) -> Option<[Vec3; 2]> {
        // Kenarların yönleri
        let da = a2 - a1;
        let db = b2 - b1;

        // XZ düzleminde çalışıyoruz
        let da_len = (da.x * da.x + da.z * da.z).sqrt();
        let db_len = (db.x * db.x + db.z * db.z).sqrt();

        if da_len < 0.001 || db_len < 0.001 {
            return None;
        }

        // Kollineerlik: cross product ≈ 0
        let cross = da.x * db.z - da.z * db.x;
        if cross.abs() / (da_len * db_len) > 0.01 {
            return None;
        } // Paralel değil

        // b1'in a kenarına olan noktadan doğruya uzaklığı
        let ab = b1 - a1;
        let dist_to_line = (ab.x * da.z - ab.z * da.x).abs() / da_len;
        if dist_to_line > tolerance {
            return None;
        } // Aynı doğru üzerinde değil

        // Kenarın yönünde parametrik projeksiyonlar
        let dir = Vec3::new(da.x / da_len, 0.0, da.z / da_len);
        let t_a1: f32 = 0.0;
        let t_a2: f32 = da_len;
        let t_b1 = (b1.x - a1.x) * dir.x + (b1.z - a1.z) * dir.z;
        let t_b2 = (b2.x - a1.x) * dir.x + (b2.z - a1.z) * dir.z;

        let b_min = t_b1.min(t_b2);
        let b_max = t_b1.max(t_b2);

        // Örtüşme aralığı
        let overlap_min = t_a1.max(b_min);
        let overlap_max = t_a2.min(b_max);

        if overlap_max - overlap_min > tolerance * 0.5 {
            let p1 = Vec3::new(a1.x + dir.x * overlap_min, a1.y, a1.z + dir.z * overlap_min);
            let p2 = Vec3::new(a1.x + dir.x * overlap_max, a1.y, a1.z + dir.z * overlap_max);
            Some([p1, p2])
        } else {
            None
        }
    }

    /// Locates the polygon a world position sits on, matching in XZ only (`pos.y` is
    /// ignored).
    ///
    /// **Never fails for a non-empty mesh.** When the point is outside every polygon this
    /// falls back to the polygon with the nearest `center`, however far away that is, so a
    /// `Some` result is not evidence that `pos` is on the mesh — check
    /// [`NavPoly::contains_point_xz`] yourself if that matters. Among polygons that do
    /// contain the point (they can overlap on shared edges) the nearest centre wins.
    ///
    /// Returns `None` only when the mesh has no polygons. Cost is linear in the polygon
    /// count; there is no spatial index.
    pub fn find_polygon(&self, pos: Vec3) -> Option<&NavPoly> {
        let mut best: Option<(&NavPoly, f32)> = None;

        for poly in &self.polygons {
            if poly.contains_point_xz(pos) {
                let dist = (poly.center - pos).length_squared();
                if best.is_none() || dist < best.unwrap().1 {
                    best = Some((poly, dist));
                }
            }
        }

        // Fallback: En yakın polygon'u döndür
        if best.is_none() {
            // Nokta hiçbir polygonun İÇİNDE değil — en yakın merkeze düşülüyor. Sessiz
            // degradasyon (yol mesh dışına çıkabilir), bu yüzden izlenebilir olması için trace!.
            tracing::trace!(pos = ?pos, "[NavMesh] Nokta hiçbir polygonda değil, en yakın polygona düşülüyor");
            let mut closest_dist = f32::MAX;
            for poly in &self.polygons {
                let dist = (poly.center - pos).length_squared();
                if dist < closest_dist {
                    closest_dist = dist;
                    best = Some((poly, dist));
                }
            }
        }

        best.map(|(p, _)| p)
    }

    /// A* over the polygon graph, returning waypoints in world metres.
    ///
    /// The result is the midpoint of each shared edge the route crosses, in travel order,
    /// with `end` appended as the final waypoint. `start` is not included, and the waypoints
    /// are *not* the geometric shortest path: no funnel or string-pulling stage exists, so
    /// routes visibly zig-zag through the middle of each portal. Search cost is the distance
    /// between polygon centres, which is itself only an approximation of travel distance.
    ///
    /// When both endpoints resolve to the same polygon the result is `vec![end]` — a single
    /// straight hop, with no check that the segment stays inside the polygon.
    ///
    /// Endpoints are resolved with [`NavMesh::find_polygon`], which snaps to the nearest
    /// polygon rather than failing, so a `start` or `end` off the mesh silently produces a
    /// path from or to somewhere else. Returns `None` only for an empty mesh or when the two
    /// polygons lie in disconnected regions. There is no iteration cap: the search runs to
    /// exhaustion, which is bounded by the polygon count but can be costly on a large mesh
    /// with an unreachable target.
    #[tracing::instrument(skip_all, name = "navmesh_find_path")]
    pub fn find_path(&self, start: Vec3, end: Vec3) -> Option<Vec<Vec3>> {
        let start_poly = match self.find_polygon(start) {
            Some(p) => p,
            None => {
                tracing::debug!(
                    polygon_count = self.polygons.len(),
                    "[NavMesh] Başlangıç noktası için polygon bulunamadı (mesh boş?)"
                );
                return None;
            }
        };
        let end_poly = match self.find_polygon(end) {
            Some(p) => p,
            None => {
                tracing::debug!(
                    polygon_count = self.polygons.len(),
                    "[NavMesh] Hedef noktası için polygon bulunamadı (mesh boş?)"
                );
                return None;
            }
        };

        if start_poly.id == end_poly.id {
            tracing::trace!(poly_id = start_poly.id, "[NavMesh] Başlangıç ve hedef aynı polygonda");
            return Some(vec![end]);
        }

        // A* polygon grafı üzerinde
        let poly_map: HashMap<u32, &NavPoly> = self.polygons.iter().map(|p| (p.id, p)).collect();

        #[derive(Clone, PartialEq)]
        struct Node {
            poly_id: u32,
            f_cost: f32,
        }
        impl Eq for Node {}
        impl Ord for Node {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                other.f_cost.total_cmp(&self.f_cost)
            }
        }
        impl PartialOrd for Node {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        let mut open = BinaryHeap::new();
        let mut came_from: HashMap<u32, (u32, [Vec3; 2])> = HashMap::new();
        let mut g_score: HashMap<u32, f32> = HashMap::new();
        let mut closed: HashSet<u32> = HashSet::new();

        open.push(Node {
            poly_id: start_poly.id,
            f_cost: 0.0,
        });
        g_score.insert(start_poly.id, 0.0);

        while let Some(current) = open.pop() {
            if closed.contains(&current.poly_id) {
                continue;
            }
            closed.insert(current.poly_id);

            if current.poly_id == end_poly.id {
                // Yolu geri izle — kenar orta noktaları + funnel basitleştirme
                let mut path = Vec::new();
                let mut curr_id = end_poly.id;

                while let Some((prev_id, edge)) = came_from.get(&curr_id) {
                    let edge_mid = Vec3::new(
                        (edge[0].x + edge[1].x) * 0.5,
                        (edge[0].y + edge[1].y) * 0.5,
                        (edge[0].z + edge[1].z) * 0.5,
                    );
                    path.push(edge_mid);
                    curr_id = *prev_id;
                }
                path.reverse();
                path.push(end);
                tracing::debug!(
                    path_len = path.len(),
                    start_poly = start_poly.id,
                    end_poly = end_poly.id,
                    "[NavMesh] Polygon yolu bulundu"
                );
                return Some(path);
            }

            let current_poly = match poly_map.get(&current.poly_id) {
                Some(p) => p,
                None => continue,
            };

            let curr_g = *g_score.get(&current.poly_id).unwrap_or(&f32::MAX);

            for &(neighbor_id, ref _edge) in &current_poly.neighbors {
                if closed.contains(&neighbor_id) {
                    continue;
                }

                let neighbor_poly = match poly_map.get(&neighbor_id) {
                    Some(p) => p,
                    None => continue,
                };

                let move_cost = (current_poly.center - neighbor_poly.center).length();
                let tentative_g = curr_g + move_cost;

                if tentative_g < *g_score.get(&neighbor_id).unwrap_or(&f32::MAX) {
                    g_score.insert(neighbor_id, tentative_g);
                    let h = (neighbor_poly.center - end).length();
                    came_from.insert(neighbor_id, (current.poly_id, *_edge));
                    open.push(Node {
                        poly_id: neighbor_id,
                        f_cost: tentative_g + h,
                    });
                }
            }
        }

        tracing::debug!(
            start_poly = start_poly.id,
            end_poly = end_poly.id,
            "[NavMesh] Polygon grafında yol bulunamadı (bağlantısız bölgeler?)"
        );
        None
    }

    /// Summarises the mesh for logging and tuning. Recomputed on each call by walking every
    /// polygon; nothing is cached.
    pub fn stats(&self) -> NavMeshStats {
        let total_area: f32 = self.polygons.iter().map(|p| p.area).sum();
        let total_edges: usize = self.polygons.iter().map(|p| p.neighbors.len()).sum();

        NavMeshStats {
            polygon_count: self.polygons.len(),
            total_area,
            edge_count: total_edges / 2, // Her kenar çift sayılıyor
            build_time_ms: self.build_time_ms,
        }
    }
}

/// Size and cost summary of a built [`NavMesh`], from [`NavMesh::stats`].
///
/// A snapshot of the mesh as it was when taken, not a live view. Its [`Display`] rendering is
/// a one-line Turkish log summary.
///
/// [`Display`]: std::fmt::Display
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NavMeshStats {
    /// Number of convex polygons. Driven by `cell_size` and by how well the greedy rectangle
    /// carving happened to fit the region, so it is a rough proxy for query cost rather than
    /// a property of the level.
    pub polygon_count: usize,
    /// Navigable surface in square metres: the sum of every [`NavPoly::area`]. Built
    /// rectangles tile their region without overlapping, so for a built mesh this is the true
    /// walkable area — remembering that "walkable" here means only "not covered by a static
    /// collider".
    pub total_area: f32,
    /// Number of adjacency links, counted once per pair (the raw neighbour lists hold each
    /// link twice). Compared against `polygon_count` it indicates fragmentation: far fewer
    /// edges than polygons means the mesh is split into poorly connected islands.
    pub edge_count: usize,
    /// Build duration in milliseconds, copied from [`NavMesh::build_time_ms`] — not the cost
    /// of collecting these statistics.
    pub build_time_ms: f64,
}

impl std::fmt::Display for NavMeshStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NavMesh: {} polygon, {:.0}m² alan, {} kenar, {:.1}ms",
            self.polygon_count, self.total_area, self.edge_count, self.build_time_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_rigid::BodyHandle;
    use gizmo_physics_core::components::Collider;
    use gizmo_physics_rigid::{RigidBody, Velocity};

    fn create_test_world() -> gizmo_physics_rigid::world::PhysicsWorld {
        let mut world = gizmo_physics_rigid::world::PhysicsWorld::new();

        // Zemin
        world.add_body(
            BodyHandle::from_id(1),
            RigidBody::new_static(),
            gizmo_physics_core::Transform::new(Vec3::new(0.0, 0.0, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)),
        );

        // Engel
        world.add_body(
            BodyHandle::from_id(2),
            RigidBody::new_static(),
            gizmo_physics_core::Transform::new(Vec3::new(5.0, 1.0, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(2.0, 2.0, 2.0)),
        );

        world
    }

    #[test]
    fn test_navmesh_build() {
        let world = create_test_world();
        let config = NavMeshConfig {
            cell_size: 1.0,
            world_min: Vec3::new(-25.0, -5.0, -25.0),
            world_max: Vec3::new(25.0, 10.0, 25.0),
            ..Default::default()
        };

        let mesh = NavMesh::build_from_physics(&world, &config);
        assert!(!mesh.polygons.is_empty(), "En az bir polygon olmalı");

        let stats = mesh.stats();
        tracing::info!("Test NavMesh: {}", stats);
    }

    #[test]
    fn test_navmesh_pathfinding() {
        let world = create_test_world();
        let config = NavMeshConfig {
            cell_size: 1.0,
            world_min: Vec3::new(-25.0, -5.0, -25.0),
            world_max: Vec3::new(25.0, 10.0, 25.0),
            ..Default::default()
        };

        let mesh = NavMesh::build_from_physics(&world, &config);

        let path = mesh.find_path(Vec3::new(-5.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0));

        assert!(path.is_some(), "Yol bulunmalı");
        let path = path.unwrap();
        assert!(!path.is_empty(), "Yol en az bir waypoint içermeli");
    }

    #[test]
    fn test_navmesh_find_polygon() {
        let world = create_test_world();
        let config = NavMeshConfig {
            cell_size: 1.0,
            world_min: Vec3::new(-25.0, -5.0, -25.0),
            world_max: Vec3::new(25.0, 10.0, 25.0),
            ..Default::default()
        };

        let mesh = NavMesh::build_from_physics(&world, &config);

        // Açık alandaki bir nokta bir polygon'a düşmeli
        let poly = mesh.find_polygon(Vec3::new(-5.0, 0.0, -5.0));
        assert!(poly.is_some(), "Polygon bulunmalı");
    }

    /// A world with ONE obstacle and no floor. `create_test_world`'s ground is itself a static
    /// collider, so the builder blocks the entire floor and the only walkable cells are the
    /// ones off its edge — useless for measuring clearance around the obstacle.
    fn world_with_one_obstacle(half_extents: Vec3) -> gizmo_physics_rigid::world::PhysicsWorld {
        let mut world = gizmo_physics_rigid::world::PhysicsWorld::new();
        world.add_body(
            BodyHandle::from_id(1),
            RigidBody::new_static(),
            gizmo_physics_core::Transform::new(Vec3::new(0.0, 1.0, 0.0)),
            Velocity::default(),
            Collider::box_collider(half_extents),
        );
        world
    }

    fn config_with_radius(agent_radius: f32) -> NavMeshConfig {
        NavMeshConfig {
            cell_size: 1.0,
            agent_radius,
            world_min: Vec3::new(-10.0, -5.0, -10.0),
            world_max: Vec3::new(10.0, 10.0, 10.0),
            ..Default::default()
        }
    }

    /// Does any polygon actually cover this point? `find_polygon` is not usable as the oracle
    /// here — it falls back to the nearest polygon CENTRE when containment fails, so it answers
    /// `Some` for points that are nowhere on the mesh.
    fn covered(mesh: &NavMesh, p: Vec3) -> bool {
        mesh.polygons.iter().any(|poly| poly.contains_point_xz(p))
    }

    /// `agent_radius` must remove walkable area next to an obstacle, not just describe it.
    ///
    /// The 2×2×2 m obstacle at the origin occupies cells x,z ∈ [-1, 1] after the builder's
    /// floor/ceil inflation. With `cell_size = 1` and `agent_radius = 1`, one more cell each
    /// way must go: the point at x = 1.5 sits in cell x = 1 — inside the obstacle's own
    /// footprint — and x = 2.5 sits in cell 2, the eroded skirt. Both must be off the mesh,
    /// and the pre-fix build covers the second one.
    #[test]
    fn agent_radius_erodes_the_walkable_area() {
        let world = world_with_one_obstacle(Vec3::splat(1.0));
        let mesh = NavMesh::build_from_physics(&world, &config_with_radius(1.0));

        assert!(
            !covered(&mesh, Vec3::new(1.5, 0.0, 0.0)),
            "the obstacle's own cells must never be walkable"
        );
        assert!(
            !covered(&mesh, Vec3::new(2.5, 0.0, 0.0)),
            "a cell within one agent radius of the obstacle must be eroded away — this is the \
             clearance `agent_radius` promises, and the pre-fix build leaves it walkable"
        );
        assert!(
            covered(&mesh, Vec3::new(3.5, 0.0, 0.0)),
            "erosion must stop at the radius: the next cell out is still walkable"
        );
    }

    /// …and it is proportional, not a blanket one-cell skirt: with a zero radius the polygons
    /// run right up to the obstacle's AABB, which is the pre-fix extent for every radius.
    #[test]
    fn a_zero_agent_radius_erodes_nothing() {
        let world = world_with_one_obstacle(Vec3::splat(1.0));
        let mesh = NavMesh::build_from_physics(&world, &config_with_radius(0.0));

        assert!(
            !covered(&mesh, Vec3::new(1.5, 0.0, 0.0)),
            "the obstacle itself is still blocked"
        );
        assert!(
            covered(&mesh, Vec3::new(2.5, 0.0, 0.0)),
            "with no radius there is no skirt to erode"
        );
    }

    /// The floor-height band survives the erosion.
    ///
    /// `walkable_y` is written only by that band, so leaving it inside the newly blocked skirt
    /// silently drops every polygon to the `0.0` fallback — measured, not assumed: that variant
    /// puts this polygon at `y = 0`. The band is moved outward instead, keeping its width, so
    /// the first walkable cell past the skirt still reports the obstacle's top surface,
    /// `y = 1 + 1 = 2`.
    #[test]
    fn the_height_sampling_band_moves_outside_the_eroded_skirt() {
        let world = world_with_one_obstacle(Vec3::splat(1.0));
        let mesh = NavMesh::build_from_physics(&world, &config_with_radius(1.0));

        let poly = mesh
            .polygons
            .iter()
            .find(|p| p.contains_point_xz(Vec3::new(3.5, 0.0, 0.0)))
            .expect("the first cell past the skirt is walkable");
        assert_eq!(
            poly.center.y, 2.0,
            "that polygon must still take the obstacle's top surface as its floor height"
        );
    }

    fn unit_square(vertices: Vec<Vec3>) -> NavPoly {
        NavPoly {
            id: 0,
            vertices,
            center: Vec3::new(0.5, 0.0, 0.5),
            neighbors: Vec::new(),
            normal: Vec3::new(0.0, 1.0, 0.0),
            area: 1.0,
        }
    }

    // contains_point_xz must work for BOTH windings. The old CCW-only test
    // reported every interior point of a CW polygon as outside (and the NavPoly
    // doc claimed CW), so find_polygon would silently fall back to nearest-center.
    #[test]
    fn contains_point_xz_is_winding_agnostic() {
        let inside = Vec3::new(0.5, 0.0, 0.5);
        let outside = Vec3::new(1.5, 0.0, 0.5);
        let corners = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let ccw = unit_square(corners.to_vec());
        let mut cw_v = corners.to_vec();
        cw_v.reverse();
        let cw = unit_square(cw_v);

        assert!(ccw.contains_point_xz(inside), "CCW: interior point inside");
        assert!(!ccw.contains_point_xz(outside), "CCW: exterior point outside");
        assert!(cw.contains_point_xz(inside), "CW: interior point must be inside too");
        assert!(!cw.contains_point_xz(outside), "CW: exterior point outside");
    }
}
