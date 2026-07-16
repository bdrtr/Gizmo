use gizmo_math::Vec3;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GridPos {
    pub x: i32,
    pub y: i32,
    pub z: i32, // Katman veya yükseklik
}

impl GridPos {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

pub struct NavGrid {
    pub cell_size: f32,
    pub width: i32,
    pub height: i32,
    pub obstacles: HashSet<GridPos>,
    pub needs_rebuild: bool,
}

impl NavGrid {
    pub fn new(cell_size: f32, width: i32, height: i32) -> Self {
        Self {
            cell_size,
            width,
            height,
            obstacles: HashSet::new(),
            needs_rebuild: true,
        }
    }

    pub fn add_obstacle_world(&mut self, world_pos: Vec3) {
        let gp = self.world_to_grid(world_pos);
        self.obstacles.insert(gp);
    }

    pub fn remove_obstacle_world(&mut self, world_pos: Vec3) {
        let gp = self.world_to_grid(world_pos);
        self.obstacles.remove(&gp);
    }

    pub fn world_to_grid(&self, pos: Vec3) -> GridPos {
        GridPos {
            x: (pos.x / self.cell_size).floor() as i32,
            y: (pos.y / self.cell_size).floor() as i32,
            z: (pos.z / self.cell_size).floor() as i32, // Genelde yer zemindir ama 3D de olabilir
        }
    }

    pub fn grid_to_world(&self, gp: GridPos) -> Vec3 {
        Vec3::new(
            (gp.x as f32 + 0.5) * self.cell_size,
            (gp.y as f32 + 0.5) * self.cell_size, // eğer y = Z ekseni yukarıysa buna göre güncelleyebiliriz
            (gp.z as f32 + 0.5) * self.cell_size,
        )
    }

    pub fn is_walkable(&self, pos: GridPos) -> bool {
        if pos.x < 0 || pos.x >= self.width || pos.z < 0 || pos.z >= self.height {
            return false;
        }
        !self.obstacles.contains(&pos) // Engel yoksa yürünebilir
    }

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
