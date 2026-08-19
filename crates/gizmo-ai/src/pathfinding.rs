//! A* pathfinding over a uniform obstacle grid ([`NavGrid`]).
//!
//! The grid is flat in the world XZ plane, addressed by integer [`GridPos`] cells of
//! `cell_size` metres; obstacles are normally rasterised from the world AABBs of a physics
//! world's non-dynamic colliders. Queries return waypoints in world metres — one cell centre
//! per step, 8-connected, with no smoothing or string-pulling.
//!
//! This is the structure the crate's own navigation systems drive: [`crate::system`] keeps a
//! `NavGrid` resource in sync and steers agents along its paths. The polygon-based
//! [`crate::navmesh`] is a separate structure that is built and queried independently.

use gizmo_math::Vec3;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Integer cell coordinate on a [`NavGrid`].
///
/// The grid is laid out in the world XZ plane: `x` and `z` are the in-plane cell indices
/// and `y` is a discrete layer index. Movement never changes `y` — [`NavGrid::neighbors`]
/// only steps in X/Z — so two cells on different layers are mutually unreachable.
///
/// Cell `(x, y, z)` covers the half-open world box `[x·cell_size, (x+1)·cell_size)` on each
/// axis; see [`NavGrid::world_to_grid`] and [`NavGrid::grid_to_world`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GridPos {
    /// Cell index along world +X, i.e. `floor(world.x / cell_size)`. Negative for negative
    /// world coordinates, and negative indices are never walkable.
    pub x: i32,
    /// Layer index, `floor(world.y / cell_size)`. Pathfinding stays within one layer; this
    /// only selects which horizontal slice of the obstacle set a query sees.
    pub y: i32,
    /// Cell index along world +Z, i.e. `floor(world.z / cell_size)`. Bounded by
    /// [`NavGrid::height`], not `width`.
    pub z: i32, // Katman veya yükseklik
}

impl GridPos {
    /// Builds a cell coordinate from raw indices. Nothing is validated or clamped: indices
    /// outside a grid's `width`/`height` are representable and simply never walkable.
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

/// Uniform obstacle grid for A* pathfinding over the world XZ plane.
///
/// The navigable area is the world-space box `origin + [0, width·cell_size) × [0,
/// height·cell_size)` in X/Z. Everything outside it is unwalkable, so **where the grid sits
/// matters as much as how big it is**: with the default `origin` of `Vec3::ZERO` the navigable
/// area is the positive quadrant only, and a scene authored around the world origin — which is
/// how the editor's default scene and every demo are laid out — has half its agents standing
/// out of bounds. [`NavGrid::centred_on`] and [`NavGrid::fitted_to`] place the grid instead of
/// leaving it in the corner.
///
/// The `y` layer is unbounded and never range-checked.
///
/// Obstacles are held in a plain hash set with no spatial acceleration; the set is normally
/// filled wholesale by [`NavGrid::update_from_physics_world`].
pub struct NavGrid {
    /// World position of the grid's `(0, 0, 0)` corner — the offset subtracted before a
    /// position is floored into a cell, and added back by [`NavGrid::grid_to_world`].
    ///
    /// `Vec3::ZERO` reproduces the original grid exactly (cell index = `floor(pos / cell_size)`).
    /// The `y` component shifts the *layer* indices too, which is rarely what a caller wants:
    /// [`NavGrid::centred_on`] therefore places X/Z and leaves `origin.y` at zero, so a layer
    /// index keeps meaning "this many cells above the world floor".
    pub origin: Vec3,
    /// Edge length of one cell in metres. Must be greater than zero — it is used as a
    /// divisor by [`NavGrid::world_to_grid`]. The world extent of the grid, and therefore
    /// how much of the scene is reachable, scales with it.
    pub cell_size: f32,
    /// Number of cells along X. Walkable indices are `0..width`; everything else is treated
    /// as blocked. Not enforced when inserting into `obstacles`.
    pub width: i32,
    /// Number of cells along **Z**, not Y — despite the name this is the depth of the XZ
    /// grid. Walkable indices are `0..height`.
    pub height: i32,
    /// Blocked cells. Membership makes a cell unwalkable regardless of bounds.
    /// [`NavGrid::update_from_physics_world`] *replaces* this set, so cells placed by hand
    /// with [`NavGrid::add_obstacle_world`] do not survive a rebuild.
    pub obstacles: HashSet<GridPos>,
    /// Rebuild request flag: `true` after [`NavGrid::new`], cleared by
    /// [`NavGrid::update_from_physics_world`]. Nothing in this type acts on it; it is polled
    /// by this crate's `system::ai_navmesh_rebuild_system`, which runs the rebuild when set.
    pub needs_rebuild: bool,
}

impl NavGrid {
    /// Cell size a host uses when it has to build a grid itself (see
    /// `system::ensure_nav_grid`): one metre, which is roughly a humanoid agent's footprint and
    /// keeps a fitted 1024-cell side at a kilometre.
    pub const DEFAULT_CELL_SIZE: f32 = 1.0;
    /// How many layers above and below `origin.y` a rebuild will rasterise.
    ///
    /// The grid is 2.5D — [`NavGrid::neighbors`] never changes the layer, so a query only ever
    /// consults the agent's own slice — but obstacles are stored per layer, so a rebuild has to
    /// pick a vertical range and there is no `depth` field to read it from. 64 layers each way is
    /// a 128-cell tall world at any cell size; more than that is scenery, not level. It is also
    /// what keeps the rebuild finite when a collider reports a non-finite AABB.
    pub const LAYER_LIMIT: i32 = 64;
    /// Margin [`NavGrid::fitted_to`] leaves around the static geometry, in cells, so an agent can
    /// path *around* the outermost wall instead of being pinned against the grid edge.
    pub const FIT_PADDING_CELLS: i32 = 4;
    /// Largest side [`NavGrid::fitted_to`] will produce. 1024² cells is ~1 M `GridPos` slots in
    /// the worst case and a 1 km square at the default 1 m cell — past that the answer is a
    /// coarser `cell_size`, not a bigger grid.
    pub const MAX_FIT_CELLS: i32 = 1024;
    /// Side used by [`NavGrid::fitted_to`] when there is no static geometry to measure.
    pub const FALLBACK_CELLS: i32 = 128;

    /// Creates an empty grid — no obstacles, `needs_rebuild` already set, so the first run
    /// of the rebuild system populates it from the physics world.
    ///
    /// `cell_size` is in metres and must be greater than zero; `width` and `height` are cell
    /// counts along X and Z respectively.
    pub fn new(cell_size: f32, width: i32, height: i32) -> Self {
        Self {
            origin: Vec3::ZERO,
            cell_size,
            width,
            height,
            obstacles: HashSet::new(),
            needs_rebuild: true,
        }
    }

    /// The same grid as [`NavGrid::new`], moved so its X/Z centre lands on `center`.
    ///
    /// `origin.y` stays at zero on purpose — see the [`NavGrid::origin`] field. Cell indices are
    /// still `floor((pos - origin) / cell_size)`, so a grid moved after obstacles were rasterised
    /// keeps the old cells and means something else by them; move it first, rebuild after.
    pub fn centred_on(cell_size: f32, width: i32, height: i32, center: Vec3) -> Self {
        let mut grid = Self::new(cell_size, width, height);
        grid.origin = Vec3::new(
            center.x - width as f32 * cell_size * 0.5,
            0.0,
            center.z - height as f32 * cell_size * 0.5,
        );
        grid
    }

    /// A grid sized and placed to cover the **static** geometry of `physics`, with
    /// [`NavGrid::FIT_PADDING_CELLS`] cells of margin so an agent can walk around the outermost
    /// wall rather than along it.
    ///
    /// This is what a host builds when the scene did not supply a grid of its own; the bounds are
    /// a measurement of the level rather than a guess. Three deliberate exclusions:
    ///
    /// - **dynamic bodies** — they move, and a grid fitted to where a crate happened to be resized
    ///   the world every time the level was reloaded;
    /// - **half-space [`Plane`](gizmo_physics_core::components::ColliderShape::Plane) colliders** —
    ///   the AABB of an infinite plane is a ±10 km sentinel, so one ground plane would fit a grid
    ///   of 20 000 cells a side (and see [`NavGrid::update_from_physics_world`] for why that is
    ///   worse than merely large);
    /// - **empty worlds** — with no static geometry to measure there is nothing to fit, so the
    ///   result is a [`NavGrid::FALLBACK_CELLS`]-square grid centred on the world origin.
    ///
    /// Each side is clamped to [`NavGrid::MAX_FIT_CELLS`]; a level larger than that is covered
    /// from its centre outwards and the rest is out of bounds (reported at warn level). Raise
    /// `cell_size` to cover more ground with the same cell count.
    pub fn fitted_to(
        cell_size: f32,
        physics: &gizmo_physics_rigid::world::PhysicsWorld,
    ) -> Self {
        let cell_size = if cell_size > 0.0 { cell_size } else { 1.0 };
        let mut min = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
        let mut max = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        let mut measured = 0usize;

        for i in 0..physics.entities.len() {
            if physics.rigid_bodies[i].body_type == gizmo_physics_rigid::components::BodyType::Dynamic
                || is_half_space(&physics.colliders[i])
            {
                continue;
            }
            let t = &physics.transforms[i];
            let aabb = physics.colliders[i].compute_aabb(t.position, t.rotation);
            let (lo, hi): (Vec3, Vec3) = (aabb.min.into(), aabb.max.into());
            if !lo.x.is_finite() || !hi.x.is_finite() || lo.x > hi.x {
                continue;
            }
            min = Vec3::new(min.x.min(lo.x), min.y.min(lo.y), min.z.min(lo.z));
            max = Vec3::new(max.x.max(hi.x), max.y.max(hi.y), max.z.max(hi.z));
            measured += 1;
        }

        if measured == 0 {
            tracing::info!(
                cells = Self::FALLBACK_CELLS,
                cell_size,
                "[AI] NavGrid: ölçülecek statik geometri yok — dünya merkezine varsayılan ızgara"
            );
            return Self::centred_on(cell_size, Self::FALLBACK_CELLS, Self::FALLBACK_CELLS, Vec3::ZERO);
        }

        let pad = Self::FIT_PADDING_CELLS as f32 * cell_size;
        let span_x = (max.x - min.x) + pad * 2.0;
        let span_z = (max.z - min.z) + pad * 2.0;
        let want_w = (span_x / cell_size).ceil() as i64;
        let want_h = (span_z / cell_size).ceil() as i64;
        let width = want_w.clamp(1, Self::MAX_FIT_CELLS as i64) as i32;
        let height = want_h.clamp(1, Self::MAX_FIT_CELLS as i64) as i32;
        if want_w > width as i64 || want_h > height as i64 {
            tracing::warn!(
                want_w,
                want_h,
                max = Self::MAX_FIT_CELLS,
                cell_size,
                "[AI] NavGrid: seviye ızgara sınırından büyük — merkezden kırpıldı, dışı ulaşılamaz"
            );
        }

        let center = Vec3::new((min.x + max.x) * 0.5, 0.0, (min.z + max.z) * 0.5);
        let grid = Self::centred_on(cell_size, width, height, center);
        tracing::info!(
            width,
            height,
            cell_size,
            origin_x = grid.origin.x,
            origin_z = grid.origin.z,
            static_bodies = measured,
            "[AI] NavGrid statik geometriye göre ölçüldü"
        );
        grid
    }

    /// Blocks the single cell containing `world_pos`.
    ///
    /// One cell only — the extent of whatever object sits at that point is not considered,
    /// so a large obstacle needs one call per cell it covers. No bounds check is performed;
    /// out-of-range cells can be inserted, which is harmless because they are unwalkable
    /// anyway. The next [`NavGrid::update_from_physics_world`] discards the entry.
    pub fn add_obstacle_world(&mut self, world_pos: Vec3) {
        let gp = self.world_to_grid(world_pos);
        self.obstacles.insert(gp);
    }

    /// Unblocks the whole cell containing `world_pos` — the position is floored to a cell,
    /// so this clears a cell, not a point. A no-op if that cell was not blocked.
    pub fn remove_obstacle_world(&mut self, world_pos: Vec3) {
        let gp = self.world_to_grid(world_pos);
        self.obstacles.remove(&gp);
    }

    /// Cell containing a world position (metres), component-wise `floor(pos / cell_size)`.
    ///
    /// Rounding is toward negative infinity on every axis, so a position `0.1 m` below the
    /// grid's `origin` maps to cell `-1`, not `0`. The result is not clamped and may lie
    /// outside the `width`/`height` bounds.
    pub fn world_to_grid(&self, pos: Vec3) -> GridPos {
        let local = pos - self.origin;
        GridPos {
            x: (local.x / self.cell_size).floor() as i32,
            y: (local.y / self.cell_size).floor() as i32,
            z: (local.z / self.cell_size).floor() as i32, // Genelde yer zemindir ama 3D de olabilir
        }
    }

    /// Centre of a cell in world space (metres): `(index + 0.5) · cell_size` on all three
    /// axes, including the `y` layer.
    ///
    /// Inverse of [`NavGrid::world_to_grid`] up to the cell centre —
    /// `world_to_grid(grid_to_world(c)) == c`. Waypoints returned by
    /// [`NavGrid::find_path`] are produced with this, so agents are routed cell centre to
    /// cell centre rather than along the geometric shortest line.
    pub fn grid_to_world(&self, gp: GridPos) -> Vec3 {
        self.origin
            + Vec3::new(
                (gp.x as f32 + 0.5) * self.cell_size,
                (gp.y as f32 + 0.5) * self.cell_size, // eğer y = Z ekseni yukarıysa buna göre güncelleyebiliriz
                (gp.z as f32 + 0.5) * self.cell_size,
            )
    }

    /// Whether a cell can be stood on: inside the X/Z bounds and not in `obstacles`.
    ///
    /// Only `x` is checked against `width` and `z` against `height`; the `y` layer has no
    /// range at all, so any layer is accepted as long as the cell is free.
    pub fn is_walkable(&self, pos: GridPos) -> bool {
        if pos.x < 0 || pos.x >= self.width || pos.z < 0 || pos.z >= self.height {
            return false;
        }
        !self.obstacles.contains(&pos) // Engel yoksa yürünebilir
    }

    /// Walkable neighbours of `pos` within its own layer — at most 8, and `y` is never
    /// changed, so this graph has no vertical connectivity.
    ///
    /// Ordering is stable: the four orthogonal steps (+X, −X, +Z, −Z) first, then the
    /// diagonals. A diagonal is emitted only when both flanking orthogonal cells are also
    /// walkable, so a route can never squeeze through the corner between two obstacles.
    ///
    /// `pos` itself is neither required to be walkable nor bounds-checked — only the eight
    /// candidates are — so a cell just outside the grid still reports the in-bounds cells
    /// beside it. The result is empty only when every candidate is blocked or out of bounds.
    // Yalnızca X,Z düzleminde Dört yön hareket algılayan komşuluk.
    pub fn neighbors(&self, pos: GridPos) -> Vec<GridPos> {
        let mut neighbors = Vec::with_capacity(8);
        let dirs = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        let diagonals = [(1, 1), (-1, -1), (-1, 1), (1, -1)];

        // 1. Düz yönler
        for (dx, dz) in dirs.iter() {
            let n = GridPos::new(pos.x + dx, pos.y, pos.z + dz);
            if self.is_walkable(n) {
                neighbors.push(n);
            }
        }

        // 2. Çapraz yönler (Köşeden geçerken her iki kenarın da açık olması şart! Yoksa çarpar)
        for (dx, dz) in diagonals.iter() {
            let n = GridPos::new(pos.x + dx, pos.y, pos.z + dz);
            let side1 = GridPos::new(pos.x + dx, pos.y, pos.z);
            let side2 = GridPos::new(pos.x, pos.y, pos.z + dz);

            if self.is_walkable(n) && self.is_walkable(side1) && self.is_walkable(side2) {
                neighbors.push(n);
            }
        }
        neighbors
    }

    /// Scans the static objects in the physics world and updates the navigation obstacle grid
    /// (NavMesh)
    ///
    /// Rebuilds `obstacles` from scratch out of every non-dynamic body in `physics`: each body's
    /// world AABB is rasterised into cells on all three axes, so a tall box blocks several `y`
    /// layers. Dynamic bodies are skipped, and the previous contents of `obstacles` — including
    /// hand-placed cells — are discarded. A world with no static bodies leaves the set empty,
    /// making everything in bounds walkable.
    ///
    /// # What is deliberately NOT rasterised
    ///
    /// **Half-space [`Plane`](gizmo_physics_core::components::ColliderShape::Plane) colliders are
    /// skipped entirely**, and both halves of that matter. A plane is infinite, so
    /// `compute_aabb` hands back a ±10 000 m cube around its position — a finite sentinel meant
    /// for a broadphase, not a description of geometry. Rasterising it at the default 1 m cell is
    /// ~8·10¹² cell insertions, i.e. a hang and then an out-of-memory kill; and if it somehow
    /// finished, every cell of every layer would be an obstacle and `find_path` would return
    /// `None` for every query in the scene. The floor is not an obstacle. The cost is that a
    /// half-space used as a *wall* is not one either — model walls that navigation must see as
    /// boxes.
    ///
    /// **Cells outside the grid are not stored.** Out-of-bounds cells are unwalkable regardless,
    /// so they were always redundant; clipping them is what bounds the work of one rebuild to the
    /// grid's own size instead of the geometry's. Vertically the grid has no bounds of its own,
    /// so the layer range is clipped to ±[`NavGrid::LAYER_LIMIT`] around `origin.y` — enough for
    /// any scene an XZ-planar grid can describe, and finite even when a collider reports an
    /// infinite AABB (an `f32::INFINITY` bound saturates to `i32::MAX` when floored into a cell,
    /// which is a loop that never ends). Clipping is reported at warn level: it means part of the
    /// level is outside the navigable area.
    ///
    /// Clears `needs_rebuild` on completion. Cost grows with the number of cells the static AABBs
    /// cover, now bounded by `width · height · (2 · LAYER_LIMIT + 1)`, so this is still an
    /// on-demand rebuild rather than a per-frame operation. Native builds rasterise on scoped
    /// threads, one per chunk of `max(1, entity_count / 8)` bodies.
    pub fn update_from_physics_world(&mut self, physics: &gizmo_physics_rigid::world::PhysicsWorld) {
        let cell_size = self.cell_size;
        let entity_count = physics.entities.len();
        let bounds = GridBounds {
            origin: self.origin,
            cell_size,
            width: self.width,
            height: self.height,
        };

        // Native: fan the AABB→grid rasterisation out across OS threads.
        #[cfg(not(target_arch = "wasm32"))]
        let (obstacles, clipped) = {
            let chunk_size = (physics.entities.len() / 8).max(1);

            std::thread::scope(|s| {
                let mut handles = Vec::new();

                let mut start = 0;
                while start < physics.entities.len() {
                    let end = (start + chunk_size).min(physics.entities.len());
                    handles.push(s.spawn(move || {
                        let mut local_obs = HashSet::new();
                        let mut local_clipped = 0u32;
                        for i in start..end {
                            local_clipped += bounds.rasterise(physics, i, &mut local_obs) as u32;
                        }
                        (local_obs, local_clipped)
                    }));
                    start = end;
                }

                let mut combined = HashSet::new();
                let mut clipped = 0u32;
                for handle in handles {
                    let (local_obs, local_clipped) = handle.join().unwrap();
                    combined.extend(local_obs);
                    clipped += local_clipped;
                }
                (combined, clipped)
            })
        };

        // wasm32 has no OS threads → rasterise the obstacle grid single-threaded.
        #[cfg(target_arch = "wasm32")]
        let (obstacles, clipped) = {
            let mut combined = HashSet::new();
            let mut clipped = 0u32;
            for i in 0..physics.entities.len() {
                clipped += bounds.rasterise(physics, i, &mut combined) as u32;
            }
            (combined, clipped)
        };

        self.obstacles = obstacles;
        self.needs_rebuild = false;

        if clipped > 0 {
            tracing::warn!(
                clipped,
                width = self.width,
                height = self.height,
                cell_size,
                "[AI] NavGrid: statik gövdeler ızgara dışına taştı — o kısım ulaşılamaz kalıyor"
            );
        }

        // Yeniden oluşturma seyrek (needs_rebuild ile tetiklenir) — çıkışta AGGREGATE detay.
        tracing::debug!(
            entity_count,
            obstacle_count = self.obstacles.len(),
            cell_size,
            "[AI] NavGrid engel ızgarası PhysicsWorld'den yeniden oluşturuldu"
        );
    }
}


/// Is this collider an infinite half-space?
///
/// Worth its own function because the answer is used twice and means the same thing both times:
/// a plane's AABB is a ±10 km sentinel rather than geometry, so it must not be measured
/// ([`NavGrid::fitted_to`]) and must not be rasterised
/// ([`NavGrid::update_from_physics_world`]).
fn is_half_space(collider: &gizmo_physics_core::components::Collider) -> bool {
    matches!(
        collider.shape,
        gizmo_physics_core::components::ColliderShape::Plane(_)
    )
}

/// The four numbers that place a [`NavGrid`] in the world, split out so a rebuild can hand them
/// to worker threads without borrowing the grid it is about to overwrite.
#[derive(Clone, Copy)]
struct GridBounds {
    origin: Vec3,
    cell_size: f32,
    width: i32,
    height: i32,
}

impl GridBounds {
    /// Cell containing `pos`. Same rule as [`NavGrid::world_to_grid`] — and deliberately a second
    /// copy of it rather than a call, because a rebuild runs this per axis per body and the grid
    /// itself is not available to the worker threads.
    fn cell(&self, pos: Vec3) -> GridPos {
        let local = pos - self.origin;
        GridPos {
            x: (local.x / self.cell_size).floor() as i32,
            y: (local.y / self.cell_size).floor() as i32,
            z: (local.z / self.cell_size).floor() as i32,
        }
    }

    /// Adds body `i`'s cells to `out`, clipped to the grid. Returns `true` if anything was
    /// clipped away — that is a level bigger than its navigable area, which the caller reports.
    ///
    /// Skips dynamic bodies (they move) and half-spaces (see [`is_half_space`]).
    fn rasterise(
        &self,
        physics: &gizmo_physics_rigid::world::PhysicsWorld,
        i: usize,
        out: &mut HashSet<GridPos>,
    ) -> bool {
        if physics.rigid_bodies[i].body_type == gizmo_physics_rigid::components::BodyType::Dynamic
            || is_half_space(&physics.colliders[i])
        {
            return false;
        }

        let transform = &physics.transforms[i];
        let aabb = physics.colliders[i].compute_aabb(transform.position, transform.rotation);
        let lo = self.cell(aabb.min.into());
        let hi = self.cell(aabb.max.into());

        // An inverted AABB (the `empty` sentinel, or a hull with no vertices) has min > max on
        // every axis; the clamped ranges below are then empty and nothing is inserted, which is
        // the right answer for "no geometry" and not a case worth warning about.
        let x0 = lo.x.max(0);
        let x1 = hi.x.min(self.width - 1);
        let z0 = lo.z.max(0);
        let z1 = hi.z.min(self.height - 1);
        let y0 = lo.y.max(-NavGrid::LAYER_LIMIT);
        let y1 = hi.y.min(NavGrid::LAYER_LIMIT);

        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    out.insert(GridPos::new(x, y, z));
                }
            }
        }

        lo.x <= hi.x && (lo.x < x0 || hi.x > x1 || lo.z < z0 || hi.z > z1 || lo.y < y0 || hi.y > y1)
    }
}

#[derive(Copy, Clone, PartialEq)]
struct AStarNode {
    pos: GridPos,
    cost: u32, // f_cost
}

impl Eq for AStarNode {}

// BinaryHeap büyükten küçüğe sıralar, küçük cost için tersine çalışması lazım.
impl Ord for AStarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .cmp(&self.cost) // Ters çevirildi
            .then_with(|| self.pos.x.cmp(&other.pos.x))
            .then_with(|| self.pos.y.cmp(&other.pos.y))
            .then_with(|| self.pos.z.cmp(&other.pos.z))
    }
}

impl PartialOrd for AStarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Octile distance estimate (suited to diagonal movement)
fn heuristic(a: GridPos, b: GridPos) -> u32 {
    let dx = (a.x - b.x).unsigned_abs();
    let dz = (a.z - b.z).unsigned_abs();
    let (mn, mx) = if dx < dz { (dx, dz) } else { (dz, dx) };
    14 * mn + 10 * (mx - mn)
}

impl NavGrid {
    /// A* Pathfinding Function
    ///
    /// Returns the world-space centre of every cell to step onto, in order. The start cell
    /// is **excluded** and the destination cell is the last element; when both positions
    /// fall in the same cell the result is `Some` with an empty vector.
    ///
    /// The search stays inside the layer (`GridPos::y`) of `start_world`, because
    /// [`NavGrid::neighbors`] never changes the layer. A destination whose Y floors to a
    /// different layer is therefore unreachable and yields `None`.
    ///
    /// Step costs are integers — 10 orthogonal, 14 diagonal (10·√2 rounded) — paired with a
    /// matching octile heuristic, so the route is a shortest 8-connected path under that
    /// cost model. The waypoints are raw cell centres: no string-pulling or smoothing is
    /// applied, and the path is not clamped to the grid bounds any tighter than
    /// [`NavGrid::is_walkable`] already does.
    ///
    /// Returns `None` when the start or destination cell is blocked or out of bounds, when
    /// no connected route exists, or when the search exceeds its hard cap of 25 000 queue
    /// POPS — the cap makes a huge or heavily fragmented map fail fast instead of stalling the
    /// frame, and is reported at warn level.
    ///
    /// Pops, not expansions: there is no decrease-key, so an improving relaxation pushes a
    /// duplicate entry (up to one per incoming neighbour) and popping an already-settled cell
    /// still spends budget. The number of DISTINCT cells reached before the cap trips can
    /// therefore be several times below 25 000 — do not size a grid against it as if it were
    /// a cell count.
    #[tracing::instrument(skip_all, name = "navgrid_find_path")]
    pub fn find_path(&self, start_world: Vec3, end_world: Vec3) -> Option<Vec<Vec3>> {
        let start = self.world_to_grid(start_world);
        let end = self.world_to_grid(end_world);

        if !self.is_walkable(end) || !self.is_walkable(start) {
            // Beklenen sorgu sonucu (hedef/başlangıç duvar içinde ya da sınır dışı) — çağıran
            // None'ı ele alır, ama sessiz dönmek yerine sebebi göster.
            tracing::debug!(
                start = ?start,
                end = ?end,
                start_walkable = self.is_walkable(start),
                end_walkable = self.is_walkable(end),
                "[AI] Pathfinding erken çıkış: başlangıç/hedef yürünebilir değil"
            );
            return None; // Hedef duvar içinde
        }

        let mut open_set = BinaryHeap::new();
        let mut came_from: HashMap<GridPos, GridPos> = HashMap::new();
        let mut g_score: HashMap<GridPos, u32> = HashMap::new();
        let mut closed_set: HashSet<GridPos> = HashSet::new();

        open_set.push(AStarNode {
            pos: start,
            cost: 0,
        });
        g_score.insert(start, 0);

        let max_iterations = 25_000usize;

        let mut iterations = 0usize;
        while let Some(current_node) = open_set.pop() {
            iterations += 1;
            if iterations > max_iterations {
                // Kurtarılabilir: yol döndürülmez, ajan hareketsiz kalır. Ama gerçek bir
                // sorunu (çok büyük/parçalı harita, ulaşılamaz hedef) gizleyebilir → warn!.
                tracing::warn!(
                    iterations,
                    max_iterations,
                    start = ?start,
                    end = ?end,
                    "[AI] Pathfinding iterasyon limiti aşıldı — ulaşılamaz/çok uzak rota, yol yok"
                );
                break;
            }

            let current = current_node.pos;

            if closed_set.contains(&current) {
                continue;
            }
            closed_set.insert(current);

            if current == end {
                // Yolu Geri İzle
                let mut path = Vec::new();
                let mut curr = end;
                while curr != start {
                    path.push(self.grid_to_world(curr));
                    curr = match came_from.get(&curr) {
                        Some(p) => *p,
                        None => break,
                    };
                }
                path.reverse();
                tracing::debug!(
                    path_len = path.len(),
                    iterations,
                    "[AI] Pathfinding yol buldu"
                );
                return Some(path);
            }

            let curr_g = *g_score.get(&current).unwrap_or(&u32::MAX);

            for neighbor in self.neighbors(current) {
                // Çaprazlar 14, düzler 10 birim maliyet.
                let move_cost = if neighbor.x != current.x && neighbor.z != current.z {
                    14
                } else {
                    10
                };
                let tentative_g = curr_g + move_cost;

                if tentative_g < *g_score.get(&neighbor).unwrap_or(&u32::MAX) {
                    came_from.insert(neighbor, current);
                    g_score.insert(neighbor, tentative_g);

                    let f_score = tentative_g + heuristic(neighbor, end);
                    open_set.push(AStarNode {
                        pos: neighbor,
                        cost: f_score,
                    });
                }
            }
        }

        // Açık liste tükendi: bağlı bir yol yok. Beklenen sorgu sonucu → debug!.
        tracing::debug!(
            iterations,
            start = ?start,
            end = ?end,
            "[AI] Pathfinding yol bulamadı (açık liste tükendi)"
        );
        None // Yol bulunamadı
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use gizmo_physics_core::BodyHandle;
    use gizmo_physics_core::components::Collider;
    use gizmo_physics_rigid::components::{RigidBody, Velocity};
    use gizmo_physics_rigid::world::PhysicsWorld;
    use gizmo_physics_core::components::Transform as PhysTransform;

    /// A physics world holding one static body with `collider` at `pos`.
    fn static_world(collider: Collider, pos: Vec3) -> PhysicsWorld {
        let mut physics = PhysicsWorld::new();
        physics.add_body(
            BodyHandle::from_id(1),
            RigidBody::new_static(),
            PhysTransform::new(pos),
            Velocity::default(),
            collider,
        );
        physics
    }

    // cell_size 1.0 → cell (x,z) center is world (x+0.5, y+0.5, z+0.5).
    fn center(x: i32, z: i32) -> Vec3 {
        Vec3::new(x as f32 + 0.5, 0.5, z as f32 + 0.5)
    }

    // Verify a returned path is actually walkable & connected: every waypoint is a
    // walkable cell, consecutive cells are 4/8-neighbours, and it ends at `end`.
    fn assert_valid_path(grid: &NavGrid, start: GridPos, end: GridPos, path: &[Vec3]) {
        assert!(!path.is_empty(), "path should have at least the destination");
        let cells: Vec<GridPos> = path.iter().map(|w| grid.world_to_grid(*w)).collect();
        assert_eq!(*cells.last().unwrap(), end, "path must end at the destination");
        let mut prev = start;
        for &c in &cells {
            assert!(grid.is_walkable(c), "path steps onto a non-walkable cell {c:?}");
            let dx = (c.x - prev.x).abs();
            let dz = (c.z - prev.z).abs();
            assert!(
                dx <= 1 && dz <= 1 && (dx + dz) > 0,
                "path step {prev:?} -> {c:?} is not a single-cell move"
            );
            prev = c;
        }
    }

    #[test]
    fn straight_path_is_optimal_length() {
        let grid = NavGrid::new(1.0, 20, 20);
        let path = grid.find_path(center(0, 5), center(5, 5)).expect("path exists");
        // 5 cells east, no diagonals needed → exactly 5 steps (start excluded).
        assert_eq!(path.len(), 5, "straight path should be 5 steps, got {}", path.len());
        assert_valid_path(&grid, GridPos::new(0, 0, 5), GridPos::new(5, 0, 5), &path);
    }

    #[test]
    fn diagonal_path_is_optimal_length() {
        let grid = NavGrid::new(1.0, 20, 20);
        let path = grid.find_path(center(0, 0), center(5, 5)).expect("path exists");
        // Pure diagonal → 5 diagonal steps.
        assert_eq!(path.len(), 5, "diagonal path should be 5 steps, got {}", path.len());
        assert_valid_path(&grid, GridPos::new(0, 0, 0), GridPos::new(5, 0, 5), &path);
    }

    #[test]
    fn path_routes_around_a_wall() {
        let mut grid = NavGrid::new(1.0, 20, 20);
        // Vertical wall at x=3 blocking z=0..=8, leaving a gap at z=9.
        for z in 0..=8 {
            grid.obstacles.insert(GridPos::new(3, 0, z));
        }
        let start = GridPos::new(1, 0, 4);
        let end = GridPos::new(6, 0, 4);
        let path = grid.find_path(center(1, 4), center(6, 4)).expect("path around wall exists");
        assert_valid_path(&grid, start, end, &path);
        // The path must never step on the wall.
        for w in &path {
            assert!(grid.world_to_grid(*w).x != 3 || grid.world_to_grid(*w).z > 8);
        }
    }

    #[test]
    fn unreachable_target_returns_none() {
        let mut grid = NavGrid::new(1.0, 20, 20);
        // Fully wall off the target cell (5,5) on all 8 sides.
        for dx in -1..=1 {
            for dz in -1..=1 {
                if dx == 0 && dz == 0 {
                    continue;
                }
                grid.obstacles.insert(GridPos::new(5 + dx, 0, 5 + dz));
            }
        }
        assert!(
            grid.find_path(center(0, 0), center(5, 5)).is_none(),
            "a fully-walled target must be unreachable"
        );
    }

    #[test]
    fn diagonal_does_not_cut_corners() {
        let mut grid = NavGrid::new(1.0, 20, 20);
        // Obstacles at (1,0) and (0,1): the diagonal (0,0)->(1,1) would clip the
        // corner between them and must be forbidden.
        grid.obstacles.insert(GridPos::new(1, 0, 0));
        grid.obstacles.insert(GridPos::new(0, 0, 1));
        let n = grid.neighbors(GridPos::new(0, 0, 0));
        assert!(
            !n.contains(&GridPos::new(1, 0, 1)),
            "diagonal move must be blocked when both flanking cells are obstacles"
        );
    }

    #[test]
    fn start_or_end_in_obstacle_returns_none() {
        let mut grid = NavGrid::new(1.0, 20, 20);
        grid.obstacles.insert(GridPos::new(5, 0, 5));
        assert!(grid.find_path(center(0, 0), center(5, 5)).is_none(), "end in wall");
        assert!(grid.find_path(center(5, 5), center(0, 0)).is_none(), "start in wall");
        // Out-of-bounds target too.
        assert!(grid.find_path(center(0, 0), center(50, 50)).is_none(), "end out of bounds");
    }

    /// **The floor is not an obstacle, and a rebuild that meets one must still finish.**
    ///
    /// A half-space `Plane` reports a ±10 000 m AABB — a broadphase sentinel, not geometry. The
    /// rebuild used to rasterise it: ~8·10¹² cell insertions at a 1 m cell, so this test did not
    /// fail against the old code, it never returned (measured with a timeout — see the commit).
    /// And had it returned, every cell of every layer would have been an obstacle: `find_path`
    /// returns `None` for a blocked start, so a single ground plane made the whole scene
    /// unnavigable.
    #[test]
    fn a_ground_plane_does_not_swallow_the_entire_grid() {
        let physics = static_world(Collider::plane(Vec3::new(0.0, 1.0, 0.0), 0.0), Vec3::ZERO);
        let mut grid = NavGrid::new(1.0, 16, 16);
        grid.update_from_physics_world(&physics);

        assert!(
            grid.obstacles.is_empty(),
            "a half-space is the floor, not an obstacle: {} cells blocked",
            grid.obstacles.len()
        );
        assert!(
            grid.find_path(center(0, 0), center(15, 15)).is_some(),
            "the scene must still be navigable with a ground plane in it"
        );
    }

    /// A body far bigger than the grid contributes only the cells that exist.
    ///
    /// The obstacle set used to grow with the *geometry*, not with the grid — cells outside the
    /// bounds were inserted and then ignored by `is_walkable`, so the wasted work was invisible
    /// until something reported an AABB of a few kilometres.
    #[test]
    fn a_static_body_never_writes_a_cell_outside_the_grid() {
        let physics = static_world(Collider::box_collider(Vec3::splat(2_000.0)), Vec3::ZERO);
        let mut grid = NavGrid::new(1.0, 8, 8);
        grid.update_from_physics_world(&physics);

        let layers = (2 * NavGrid::LAYER_LIMIT + 1) as usize;
        assert!(
            grid.obstacles.len() <= 8 * 8 * layers,
            "{} cells is more than the grid can hold",
            grid.obstacles.len()
        );
        for cell in &grid.obstacles {
            assert!(
                cell.x >= 0 && cell.x < 8 && cell.z >= 0 && cell.z < 8,
                "{cell:?} is outside the grid"
            );
            assert!(
                cell.y.abs() <= NavGrid::LAYER_LIMIT,
                "{cell:?} is outside the layer window"
            );
        }
    }

    /// The grid can sit where the scene is.
    ///
    /// Without an `origin` the navigable area was the positive quadrant, so an agent at
    /// `(-10, 0, -10)` — an ordinary spot in a scene laid out around the world origin — was
    /// permanently out of bounds and `find_path` refused every query it made.
    #[test]
    fn a_placed_grid_makes_negative_coordinates_navigable() {
        let grid = NavGrid::centred_on(1.0, 40, 40, Vec3::ZERO);
        let start = Vec3::new(-10.0, 0.0, -10.0);
        let end = Vec3::new(8.0, 0.0, 6.0);

        assert!(grid.is_walkable(grid.world_to_grid(start)));
        assert!(grid.is_walkable(grid.world_to_grid(end)));
        assert!(grid.find_path(start, end).is_some());

        // The round trip still holds with an offset: a cell centre maps back to its own cell.
        let cell = grid.world_to_grid(start);
        assert_eq!(grid.world_to_grid(grid.grid_to_world(cell)), cell);

        // ...and the default grid is what the offset one is being contrasted with.
        assert!(
            !NavGrid::new(1.0, 40, 40).is_walkable(NavGrid::new(1.0, 40, 40).world_to_grid(start)),
            "an unplaced grid still starts at the world origin"
        );
    }

    /// A fitted grid covers the level it was measured from, including the parts at negative
    /// coordinates, and does not let one ground plane decide its size.
    #[test]
    fn a_fitted_grid_covers_the_static_geometry_it_measured() {
        let mut physics = static_world(Collider::box_collider(Vec3::splat(1.0)), Vec3::new(-20.0, 0.0, -20.0));
        physics.add_body(
            BodyHandle::from_id(2),
            RigidBody::new_static(),
            PhysTransform::new(Vec3::new(20.0, 0.0, 20.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::splat(1.0)),
        );
        physics.add_body(
            BodyHandle::from_id(3),
            RigidBody::new_static(),
            PhysTransform::new(Vec3::ZERO),
            Velocity::default(),
            Collider::plane(Vec3::new(0.0, 1.0, 0.0), 0.0),
        );

        let grid = NavGrid::fitted_to(1.0, &physics);

        assert!(
            grid.width < 100 && grid.height < 100,
            "the ±10 km plane sentinel leaked into the fit: {}×{}",
            grid.width,
            grid.height
        );
        for corner in [Vec3::new(-20.0, 0.0, -20.0), Vec3::new(20.0, 0.0, 20.0)] {
            assert!(
                grid.is_walkable(grid.world_to_grid(corner)),
                "{corner:?} is outside a grid fitted to it"
            );
        }
    }

    /// With nothing to measure, a fit still produces a usable grid rather than a 1×1 one.
    #[test]
    fn a_fit_with_no_static_geometry_falls_back_to_a_grid_around_the_origin() {
        let grid = NavGrid::fitted_to(1.0, &PhysicsWorld::new());
        assert_eq!(grid.width, NavGrid::FALLBACK_CELLS);
        assert!(grid.is_walkable(grid.world_to_grid(Vec3::ZERO)));
        assert!(grid.is_walkable(grid.world_to_grid(Vec3::new(-30.0, 0.0, -30.0))));
    }

}
