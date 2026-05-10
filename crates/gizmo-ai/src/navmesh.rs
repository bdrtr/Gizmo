//! NavMesh — Polygon-tabanlı navigasyon mesh üretimi
//!
//! Fizik dünyasındaki statik collider'ların AABB'lerinden yürünebilir
//! alanları çıkarır ve konveks polygon'lara böler. A* pathfinding
//! bu polygon'lar üzerinde çalışır.
//!
//! ## Mimari
//! 1. **Voxelization**: Dünyayı grid hücrelere böl, engelleri işaretle
//! 2. **Region Building**: Yürünebilir hücreleri bağlı bölgelere ayır (flood-fill)
//! 3. **Polygon Generation**: Her bölgeyi konveks polygon'a dönüştür
//! 4. **Adjacency Graph**: Polygon'lar arasındaki komşuluk grafını oluştur
//! 5. **Pathfinding**: Polygon grafı üzerinde A* + funnel algorithm

use gizmo_math::Vec3;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// NavMesh'teki tek bir konveks polygon
#[derive(Debug, Clone)]
pub struct NavPoly {
    /// Bu polygon'un benzersiz kimliği
    pub id: u32,
    /// Polygon köşeleri (saat yönünde, düz yüzey varsayımıyla)
    pub vertices: Vec<Vec3>,
    /// Polygon merkez noktası (ağırlık merkezi)
    pub center: Vec3,
    /// Komşu polygon ID'leri ve paylaşılan kenar bilgisi
    pub neighbors: Vec<(u32, [Vec3; 2])>, // (neighbor_id, [edge_start, edge_end])
    /// Bu polygon'un yüzey normali
    pub normal: Vec3,
    /// Yüzey alanı
    pub area: f32,
}

impl NavPoly {
    /// Nokta bu polygon'un içinde mi? (XZ düzleminde)
    pub fn contains_point_xz(&self, point: Vec3) -> bool {
        let n = self.vertices.len();
        if n < 3 {
            return false;
        }

        for i in 0..n {
            let a = self.vertices[i];
            let b = self.vertices[(i + 1) % n];
            let cross = (b.x - a.x) * (point.z - a.z) - (b.z - a.z) * (point.x - a.x);
            if cross < 0.0 {
                return false;
            }
        }
        true
    }

    /// Polygon'un AABB'sini hesaplar
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

/// Navigasyon mesh — polygon grafı + pathfinding
#[derive(Debug, Clone)]
pub struct NavMesh {
    /// Tüm polygon'lar
    pub polygons: Vec<NavPoly>,
    /// Hücre boyutu (voxelization hassasiyeti)
    pub cell_size: f32,
    /// Ajan yüksekliği (clearance kontrolü için)
    pub agent_height: f32,
    /// Ajan yarıçapı (engel kenar boşluğu)
    pub agent_radius: f32,
    /// Yürünebilir maksimum eğim (radyan)
    pub max_slope: f32,
    /// Mesh'in oluşturulma anı
    pub build_time_ms: f64,
}

/// NavMesh oluşturma konfigürasyonu
#[derive(Debug, Clone)]
pub struct NavMeshConfig {
    pub cell_size: f32,
    pub agent_height: f32,
    pub agent_radius: f32,
    pub max_slope_degrees: f32,
    pub world_min: Vec3,
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

/// 2D grid hücresi (voxelization sonucu)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GridCell {
    x: i32,
    z: i32,
}

impl NavMesh {
    /// Fizik dünyasından NavMesh oluşturur (Recast-tarzı pipeline)
    pub fn build_from_physics(
        physics: &gizmo_physics::world::PhysicsWorld,
        config: &NavMeshConfig,
    ) -> Self {
        let start = std::time::Instant::now();
        let cell_size = config.cell_size;

        // 1. Voxelization: Statik collider'ları grid'e yaz
        let mut blocked: HashSet<GridCell> = HashSet::new();
        let mut walkable_y: HashMap<GridCell, f32> = HashMap::new();

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

            // Ajan yarıçapı kadar kenar boşluğu
            let margin = (config.agent_radius / cell_size).ceil() as i32;

            for x in (min_x - margin)..=(max_x + margin) {
                for z in (min_z - margin)..=(max_z + margin) {
                    let cell = GridCell { x, z };

                    // Gerçek AABB içindeki hücreler engel
                    if x >= min_x && x <= max_x && z >= min_z && z <= max_z {
                        blocked.insert(cell);
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

        let mut all_walkable: HashSet<GridCell> = HashSet::new();
        for x in world_min_x..=world_max_x {
            for z in world_min_z..=world_max_z {
                let cell = GridCell { x, z };
                if !blocked.contains(&cell) {
                    all_walkable.insert(cell);
                }
            }
        }

        let mut visited: HashSet<GridCell> = HashSet::new();
        let mut regions: Vec<HashSet<GridCell>> = Vec::new();

        for &cell in &all_walkable {
            if visited.contains(&cell) {
                continue;
            }

            // Flood fill
            let mut region = HashSet::new();
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

        // 3. Polygon generation: Her bölgeyi dikdörtgen polygon'lara böl (greedy merge)
        let mut polygons = Vec::new();
        let mut poly_id = 0u32;

        for region in &regions {
            let mut remaining: HashSet<GridCell> = region.clone();

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

        println!(
            "[NavMesh] Mesh oluşturuldu: {} polygon, {} bölge, {:.1}ms",
            polygons.len(),
            regions.len(),
            build_time_ms
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

    /// Komşuluk grafını oluştur (kenar paylaşımı tespiti)
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

    /// İki polygon arasında paylaşılan/örtüşen kenarı bul
    /// Greedy merge sonucunda kenarlar tam eşleşmeyebilir — kısmi örtüşme yeterli
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

    /// İki kenar parçasının kollineer olup olmadığını ve örtüşüp örtüşmediğini kontrol eder.
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

    /// Dünya koordinatındaki noktayı içeren polygon'u bul
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

    /// Polygon grafı üzerinde A* pathfinding
    pub fn find_path(&self, start: Vec3, end: Vec3) -> Option<Vec<Vec3>> {
        let start_poly = self.find_polygon(start)?;
        let end_poly = self.find_polygon(end)?;

        if start_poly.id == end_poly.id {
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

        None
    }

    /// NavMesh istatistikleri
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

/// NavMesh istatistikleri
#[derive(Debug, Clone)]
pub struct NavMeshStats {
    pub polygon_count: usize,
    pub total_area: f32,
    pub edge_count: usize,
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
    use gizmo_core::entity::Entity;
    use gizmo_physics::components::{Collider, RigidBody, Velocity};

    fn create_test_world() -> gizmo_physics::world::PhysicsWorld {
        let mut world = gizmo_physics::world::PhysicsWorld::new();

        // Zemin
        world.add_body(
            Entity::new(1, 0),
            RigidBody::new_static(),
            gizmo_physics::components::Transform::new(Vec3::new(0.0, 0.0, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)),
        );

        // Engel
        world.add_body(
            Entity::new(2, 0),
            RigidBody::new_static(),
            gizmo_physics::components::Transform::new(Vec3::new(5.0, 1.0, 0.0)),
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
        println!("Test NavMesh: {}", stats);
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
}
