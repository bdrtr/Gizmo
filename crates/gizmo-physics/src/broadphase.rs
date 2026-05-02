use gizmo_core::entity::Entity;
use gizmo_math::{Aabb, Vec3};
use std::collections::{HashMap, HashSet};

/// Spatial hash grid for broadphase collision detection
pub struct SpatialHash {
    cell_size: f32,
    grid: HashMap<GridCell, Vec<Entity>>,
    global_entities: Vec<Entity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GridCell {
    x: i32,
    y: i32,
    z: i32,
}

impl GridCell {
    fn from_position(pos: Vec3, cell_size: f32) -> Self {
        Self {
            x: (pos.x / cell_size).floor() as i32,
            y: (pos.y / cell_size).floor() as i32,
            z: (pos.z / cell_size).floor() as i32,
        }
    }

    /// Get all neighboring cells (including self) - 27 cells total
    fn neighbors(&self) -> Vec<GridCell> {
        let mut neighbors = Vec::with_capacity(27);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    neighbors.push(GridCell {
                        x: self.x + dx,
                        y: self.y + dy,
                        z: self.z + dz,
                    });
                }
            }
        }
        neighbors
    }
}

/// Extension trait for AABB to work with spatial hash
trait AabbExt {
    fn overlapping_cells(&self, cell_size: f32) -> Vec<GridCell>;
}

impl AabbExt for Aabb {
    /// Get all grid cells this AABB overlaps
    fn overlapping_cells(&self, cell_size: f32) -> Vec<GridCell> {
        let min_cell = GridCell::from_position(self.min.into(), cell_size);
        let max_cell = GridCell::from_position(self.max.into(), cell_size);

        let mut cells = Vec::new();
        for x in min_cell.x..=max_cell.x {
            for y in min_cell.y..=max_cell.y {
                for z in min_cell.z..=max_cell.z {
                    cells.push(GridCell { x, y, z });
                }
            }
        }
        cells
    }
}

impl SpatialHash {
    pub fn new(cell_size: f32) -> Self {
        Self {
            cell_size,
            grid: HashMap::new(),
            global_entities: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.grid.clear();
        self.global_entities.clear();
    }

    /// Insert an entity with its AABB into the spatial hash
    pub fn insert(&mut self, entity: Entity, aabb: Aabb) {
        let min_cell = GridCell::from_position(aabb.min.into(), self.cell_size);
        let max_cell = GridCell::from_position(aabb.max.into(), self.cell_size);

        let dx = (max_cell.x - min_cell.x).abs();
        let dy = (max_cell.y - min_cell.y).abs();
        let dz = (max_cell.z - min_cell.z).abs();

        if dx > 100 || dy > 100 || dz > 100 {
            // Very large AABB, put in global entities to avoid OOM
            self.global_entities.push(entity);
            return;
        }

        let cells = aabb.overlapping_cells(self.cell_size);
        for cell in cells {
            self.grid.entry(cell).or_insert_with(Vec::new).push(entity);
        }
    }

    /// Query potential collision pairs
    pub fn query_pairs(&self) -> Vec<(Entity, Entity)> {
        let mut pairs = HashSet::new();

        for entities in self.grid.values() {
            // Check all pairs within the same cell
            for i in 0..entities.len() {
                for j in (i + 1)..entities.len() {
                    let a = entities[i];
                    let b = entities[j];
                    // Store in sorted order to avoid duplicates
                    let pair = if a.id() < b.id() { (a, b) } else { (b, a) };
                    pairs.insert(pair);
                }
            }
        }

        if !self.global_entities.is_empty() {
            let mut all_entities: HashSet<Entity> = self.grid.values().flatten().copied().collect();
            all_entities.extend(self.global_entities.iter().copied());

            for &global_ent in &self.global_entities {
                for &other_ent in &all_entities {
                    if global_ent != other_ent {
                        let pair = if global_ent.id() < other_ent.id() { (global_ent, other_ent) } else { (other_ent, global_ent) };
                        pairs.insert(pair);
                    }
                }
            }
        }

        let mut sorted_pairs: Vec<_> = pairs.into_iter().collect();
        sorted_pairs.sort_by(|a, b| a.0.id().cmp(&b.0.id()).then(a.1.id().cmp(&b.1.id())));
        sorted_pairs
    }

    /// Query entities near a point
    pub fn query_point(&self, point: Vec3, _radius: f32) -> Vec<Entity> {
        let cell = GridCell::from_position(point, self.cell_size);
        let mut entities = Vec::new();

        for neighbor in cell.neighbors() {
            if let Some(cell_entities) = self.grid.get(&neighbor) {
                entities.extend_from_slice(cell_entities);
            }
        }

        entities
    }

    /// Query entities within an AABB
    pub fn query_aabb(&self, aabb: Aabb) -> Vec<Entity> {
        let cells = aabb.overlapping_cells(self.cell_size);
        let mut entities = HashSet::new();

        for cell in cells {
            if let Some(cell_entities) = self.grid.get(&cell) {
                entities.extend(cell_entities.iter().copied());
            }
        }

        entities.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spatial_hash_basic() {
        let mut hash = SpatialHash::new(10.0);

        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(2, 0);
        let e3 = Entity::new(3, 0);

        let aabb1 = Aabb::from_center_half_extents(Vec3::new(0.0, 0.0, 0.0), Vec3::splat(1.0));
        let aabb2 = Aabb::from_center_half_extents(Vec3::new(1.0, 0.0, 0.0), Vec3::splat(1.0));
        let aabb3 = Aabb::from_center_half_extents(Vec3::new(100.0, 0.0, 0.0), Vec3::splat(1.0));

        hash.insert(e1, aabb1);
        hash.insert(e2, aabb2);
        hash.insert(e3, aabb3);

        let pairs = hash.query_pairs();

        // e1 and e2 should be in the same cell, e3 should be separate
        assert!(pairs.contains(&(e1, e2)) || pairs.contains(&(e2, e1)));
        assert!(!pairs.contains(&(e1, e3)) && !pairs.contains(&(e3, e1)));
    }
}
