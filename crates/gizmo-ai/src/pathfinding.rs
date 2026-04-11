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
    pub obstacles: HashSet<GridPos>,
}

impl NavGrid {
    pub fn new(cell_size: f32) -> Self {
        Self {
            cell_size,
            obstacles: HashSet::new(),
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
        // Zemin veya uzay sınırları kontrol edilebilir (Şimdilik limitsiz).
        !self.obstacles.contains(&pos) // Engel yoksa yürünebilir
    }

    // Yalnızca X,Z düzleminde Dört yön hareket algılayan komşuluk.
    // Eğer 3 boyutlu uçan ajan istersek y de eklenebilir.
    pub fn get_neighbors(&self, pos: GridPos) -> Vec<GridPos> {
        let mut neighbors = Vec::with_capacity(8);
        let dirs = [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1)];

        let diagonals = [(1, 0, 1), (-1, 0, -1), (-1, 0, 1), (1, 0, -1)];

        // 1. Düz yönler
        for (dx, dy, dz) in dirs.iter() {
            let n = GridPos::new(pos.x + dx, pos.y + dy, pos.z + dz);
            if self.is_walkable(n) {
                neighbors.push(n);
            }
        }

        // 2. Çapraz yönler (Köşeden geçerken her iki kenarın da açık olması şart! Yoksa çarpar)
        for (dx, dy, dz) in diagonals.iter() {
            let n = GridPos::new(pos.x + dx, pos.y + dy, pos.z + dz);
            let side1 = GridPos::new(pos.x + dx, pos.y, pos.z);
            let side2 = GridPos::new(pos.x, pos.y, pos.z + dz);

            if self.is_walkable(n) && self.is_walkable(side1) && self.is_walkable(side2) {
                neighbors.push(n);
            }
        }
        neighbors
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

/// Manhattan mesafe tahmini
fn heuristic(a: GridPos, b: GridPos) -> u32 {
    ((a.x - b.x).abs() + (a.y - b.y).abs() + (a.z - b.z).abs()) as u32 * 10
}

/// A* Pathfinding Fonksiyonu
pub fn find_path(grid: &NavGrid, start_world: Vec3, end_world: Vec3) -> Option<Vec<Vec3>> {
    let start = grid.world_to_grid(start_world);
    let end = grid.world_to_grid(end_world);

    if !grid.is_walkable(end) {
        return None; // Hedef duvar içinde
    }

    let mut open_set = BinaryHeap::new();
    let mut came_from: HashMap<GridPos, GridPos> = HashMap::new();
    let mut g_score: HashMap<GridPos, u32> = HashMap::new();

    open_set.push(AStarNode {
        pos: start,
        cost: 0,
    });
    g_score.insert(start, 0);

    let manhattan_dist = ((start.x - end.x).abs() + (start.z - end.z).abs()) as u32;
    let max_iterations = (manhattan_dist as usize * 10).clamp(500, 5000);

    let mut iterations = 0usize;
    while let Some(current_node) = open_set.pop() {
        iterations += 1;
        if iterations > max_iterations {
            eprintln!(
                "[AI] Pathfinding limit aşıldı ({}/{}). Ulaşılamaz rota?",
                iterations, max_iterations
            );
            break;
        }

        let current = current_node.pos;

        if current == end {
            // Yolu Geri İzle
            let mut path = Vec::new();
            let mut curr = end;
            while curr != start {
                path.push(grid.grid_to_world(curr));
                curr = *came_from.get(&curr).unwrap();
            }
            // Başlangıç noktasını da ekle (veya dahil etme)
            // path.push(grid.grid_to_world(start));

            path.reverse();
            return Some(path);
        }

        let curr_g = *g_score.get(&current).unwrap_or(&u32::MAX);

        for neighbor in grid.get_neighbors(current) {
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

    None // Yol bulunamadı
}
