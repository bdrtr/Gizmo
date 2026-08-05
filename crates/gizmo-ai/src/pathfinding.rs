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
/// The navigable area is the world-space quadrant `[0, width·cell_size) × [0, height·cell_size)`
/// in X/Z — the grid origin is the world origin, with no offset, so negative world
/// coordinates are always unwalkable. The `y` layer is unbounded and never range-checked.
///
/// Obstacles are held in a plain hash set with no spatial acceleration; the set is normally
/// filled wholesale by [`NavGrid::update_from_physics_world`].
pub struct NavGrid {
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
    /// Creates an empty grid — no obstacles, `needs_rebuild` already set, so the first run
    /// of the rebuild system populates it from the physics world.
    ///
    /// `cell_size` is in metres and must be greater than zero; `width` and `height` are cell
    /// counts along X and Z respectively.
    pub fn new(cell_size: f32, width: i32, height: i32) -> Self {
        Self {
            cell_size,
            width,
            height,
            obstacles: HashSet::new(),
            needs_rebuild: true,
        }
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
    /// Rounding is toward negative infinity on every axis, so world `-0.1 m` maps to cell
    /// `-1`, not `0`. The result is not clamped and may lie outside the `width`/`height`
    /// bounds.
    pub fn world_to_grid(&self, pos: Vec3) -> GridPos {
        GridPos {
            x: (pos.x / self.cell_size).floor() as i32,
            y: (pos.y / self.cell_size).floor() as i32,
            z: (pos.z / self.cell_size).floor() as i32, // Genelde yer zemindir ama 3D de olabilir
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
        Vec3::new(
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

    /// Fizik dünyasındaki statik nesneleri tarayıp navigasyon engel ızgarasını (NavMesh) günceller
    ///
    /// Rebuilds `obstacles` from scratch out of every non-dynamic body in `physics`:
    /// each body's world AABB is rasterised into cells on all
    /// three axes, so a tall box blocks several `y` layers. Dynamic bodies are skipped, and
    /// the previous contents of `obstacles` — including hand-placed cells — are discarded.
    /// A world with no static bodies leaves the set empty, making everything in bounds
    /// walkable.
    ///
    /// Cells outside the `width`/`height` bounds are inserted too; they are redundant, since
    /// out-of-bounds cells are unwalkable regardless.
    ///
    /// Clears `needs_rebuild` on completion. Cost grows with the number of cells the static
    /// AABBs cover — i.e. with the cube of `1/cell_size` for fixed geometry — so this is an
    /// on-demand rebuild, not a per-frame operation. Native builds rasterise on scoped
    /// threads, one per chunk of `max(1, entity_count / 8)` bodies.
    #[tracing::instrument(skip_all, name = "navgrid_rebuild")]
    pub fn update_from_physics_world(&mut self, physics: &gizmo_physics_rigid::world::PhysicsWorld) {
        let cell_size = self.cell_size;
        let entity_count = physics.entities.len();

        let world_to_grid_fn = |pos: Vec3| -> GridPos {
            GridPos {
                x: (pos.x / cell_size).floor() as i32,
                y: (pos.y / cell_size).floor() as i32,
                z: (pos.z / cell_size).floor() as i32,
            }
        };

        // Native: fan the AABB→grid rasterisation out across OS threads.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let chunk_size = (physics.entities.len() / 8).max(1);

            self.obstacles = std::thread::scope(|s| {
                let mut handles = Vec::new();

                let mut start = 0;
                while start < physics.entities.len() {
                    let end = (start + chunk_size).min(physics.entities.len());
                    handles.push(s.spawn(move || {
                        let mut local_obs = HashSet::new();
                        for i in start..end {
                            let rb = &physics.rigid_bodies[i];
                            if rb.body_type == gizmo_physics_rigid::components::BodyType::Dynamic {
                                continue;
                            }
                            let transform = &physics.transforms[i];
                            let collider = &physics.colliders[i];

                            let aabb =
                                collider.compute_aabb(transform.position, transform.rotation);
                            let min_grid = world_to_grid_fn(aabb.min.into());
                            let max_grid = world_to_grid_fn(aabb.max.into());

                            for x in min_grid.x..=max_grid.x {
                                for y in min_grid.y..=max_grid.y {
                                    for z in min_grid.z..=max_grid.z {
                                        local_obs.insert(GridPos::new(x, y, z));
                                    }
                                }
                            }
                        }
                        local_obs
                    }));
                    start = end;
                }

                let mut combined = HashSet::new();
                for handle in handles {
                    combined.extend(handle.join().unwrap());
                }
                combined
            });
        }

        // wasm32 has no OS threads → rasterise the obstacle grid single-threaded.
        #[cfg(target_arch = "wasm32")]
        {
            let mut combined = HashSet::new();
            for i in 0..physics.entities.len() {
                let rb = &physics.rigid_bodies[i];
                if rb.body_type == gizmo_physics_rigid::components::BodyType::Dynamic {
                    continue;
                }
                let transform = &physics.transforms[i];
                let collider = &physics.colliders[i];

                let aabb = collider.compute_aabb(transform.position, transform.rotation);
                let min_grid = world_to_grid_fn(aabb.min.into());
                let max_grid = world_to_grid_fn(aabb.max.into());

                for x in min_grid.x..=max_grid.x {
                    for y in min_grid.y..=max_grid.y {
                        for z in min_grid.z..=max_grid.z {
                            combined.insert(GridPos::new(x, y, z));
                        }
                    }
                }
            }
            self.obstacles = combined;
        }

        self.needs_rebuild = false;

        // Yeniden oluşturma seyrek (needs_rebuild ile tetiklenir) — çıkışta AGGREGATE detay.
        tracing::debug!(
            entity_count,
            obstacle_count = self.obstacles.len(),
            cell_size,
            "[AI] NavGrid engel ızgarası PhysicsWorld'den yeniden oluşturuldu"
        );
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

/// Octile mesafe tahmini (Çapraz harekete uygun)
fn heuristic(a: GridPos, b: GridPos) -> u32 {
    let dx = (a.x - b.x).unsigned_abs();
    let dz = (a.z - b.z).unsigned_abs();
    let (mn, mx) = if dx < dz { (dx, dz) } else { (dz, dx) };
    14 * mn + 10 * (mx - mn)
}

impl NavGrid {
    /// A* Pathfinding Fonksiyonu
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
}
