use gizmo_core::entity::Entity;
use gizmo_math::{Aabb, Vec3};
use std::collections::{HashMap, HashSet};

/// Spatial hash grid for broadphase collision detection
pub struct SpatialHash {
    cell_size: f32,
    grid: HashMap<GridCell, Vec<Entity>>,
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
        let min_cell = GridCell::from_position(self.min().into(), cell_size);
        let max_cell = GridCell::from_position(self.max().into(), cell_size);

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
        }
    }

    pub fn clear(&mut self) {
        self.grid.clear();
    }

    /// Insert an entity with its AABB into the spatial hash
    pub fn insert(&mut self, entity: Entity, aabb: Aabb) {
        let cells = aabb.overlapping_cells(self.cell_size);
        for cell in cells {
            self.grid.entry(cell).or_insert_with(Vec::new).push(entity);
        }
    }

    /// Query potential collision pairs
    /// Returns a set of entity pairs that might be colliding
    pub fn query_pairs(&self) -> HashSet<(Entity, Entity)> {
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

        pairs
    }

    /// Query entities near a point
    pub fn query_point(&self, point: Vec3, radius: f32) -> Vec<Entity> {
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

        let e1 = Entity::from_raw(1);
        let e2 = Entity::from_raw(2);
        let e3 = Entity::from_raw(3);

        let aabb1 = Aabb::from_center_half_extents(Vec3::new(0.0, 0.0, 0.0).into(), Vec3::splat(1.0).into());
        let aabb2 = Aabb::from_center_half_extents(Vec3::new(1.0, 0.0, 0.0).into(), Vec3::splat(1.0).into());
        let aabb3 = Aabb::from_center_half_extents(Vec3::new(100.0, 0.0, 0.0).into(), Vec3::splat(1.0).into());

        hash.insert(e1, aabb1);
        hash.insert(e2, aabb2);
        hash.insert(e3, aabb3);

        let pairs = hash.query_pairs();

        // e1 and e2 should be in the same cell, e3 should be separate
        assert!(pairs.contains(&(e1, e2)) || pairs.contains(&(e2, e1)));
        assert!(!pairs.contains(&(e1, e3)) && !pairs.contains(&(e3, e1)));
    }
}
