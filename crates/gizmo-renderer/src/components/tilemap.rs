use gizmo_math::{Vec2, Vec4};

use crate::sprite::SpriteImageId;

use super::sprite::{Sprite, SpriteSpace};

/// A grid of tiles cut from one tileset image, drawn on the entity's plane — a level's ground,
/// walls and platforms as one component instead of one entity per tile.
///
/// Before this a 2D level was hundreds of [`Sprite`] entities: `sprite_2d`'s ground alone was 240
/// (`docs/CAPABILITY_GAPS.md` §C9). A tilemap is one entity, and because every tile shares the
/// tileset image the whole map still costs **one draw call** — it goes through the same sprite
/// queue, sort and batching as world sprites, on both draw paths.
///
/// # Coordinates
///
/// The tileset is cut into `tileset_columns × tileset_rows` equal cells, numbered left to right
/// then top to bottom — the rule [`Sprite::with_frame`] uses — and a tile is the number of its
/// cell. The map is `width × height` tiles, `tile_size` world units each, laid out the way it reads
/// in a text file: tile `(0, 0)` is the **top-left**, `x` grows right and `y` grows **down**. The
/// entity's origin is the map's top-left corner, so the map hangs right and down from it in the
/// entity's local XY plane, placed by its `GlobalTransform` like a world sprite.
///
/// Within the world-sprite queue a tilemap sorts on its [`layer`](Self::layer) like a sprite.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Tilemap {
    /// The tileset image, from [`Renderer::load_sprite_image`](crate::Renderer::load_sprite_image)
    /// or its siblings.
    pub tileset: SpriteImageId,
    /// Cells across the tileset image.
    pub tileset_columns: u32,
    /// Cells down the tileset image.
    pub tileset_rows: u32,
    /// The size of one tile, in world units.
    pub tile_size: Vec2,
    /// Multiplied into every tile.
    pub color: Vec4,
    /// Draw order among world sprites: lower first.
    pub layer: i32,
    width: u32,
    height: u32,
    /// Row-major, `width * height` long; [`Self::EMPTY`] where there is no tile.
    tiles: Vec<u32>,
}

impl Tilemap {
    /// The stored value of a cell with no tile.
    const EMPTY: u32 = u32::MAX;

    /// An empty `width × height` map over a `tileset_columns × tileset_rows` tileset.
    #[must_use]
    pub fn new(
        tileset: SpriteImageId,
        tileset_columns: u32,
        tileset_rows: u32,
        width: u32,
        height: u32,
        tile_size: Vec2,
    ) -> Self {
        let len = (width as usize).saturating_mul(height as usize);
        Self {
            tileset,
            tileset_columns,
            tileset_rows,
            tile_size,
            color: Vec4::ONE,
            layer: 0,
            width,
            height,
            tiles: vec![Self::EMPTY; len],
        }
    }

    /// Builds a map from text, one string per row, top row first: each character is looked up in
    /// `legend` and becomes that tile; a character `legend` returns `None` for is empty. The map is
    /// as wide as the longest row.
    ///
    /// ```
    /// # use gizmo_renderer::components::Tilemap;
    /// # fn tileset() -> gizmo_renderer::sprite::SpriteImageId { unimplemented!() }
    /// # fn build() -> Tilemap {
    /// let map = Tilemap::from_rows(tileset(), 2, 1, gizmo_math::Vec2::ONE, &[
    ///     "      ##  ",
    ///     "==========",
    /// ], |c| match c { '=' => Some(0), '#' => Some(1), _ => None });
    /// # map }
    /// ```
    #[must_use]
    pub fn from_rows(
        tileset: SpriteImageId,
        tileset_columns: u32,
        tileset_rows: u32,
        tile_size: Vec2,
        rows: &[&str],
        legend: impl Fn(char) -> Option<u32>,
    ) -> Self {
        let width = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u32;
        let mut map = Self::new(tileset, tileset_columns, tileset_rows, width, rows.len() as u32, tile_size);
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                map.set(x as u32, y as u32, legend(c));
            }
        }
        map
    }

    /// Replaces the tint.
    #[must_use]
    pub fn with_color(mut self, color: Vec4) -> Self {
        self.color = color;
        self
    }

    /// Replaces the layer.
    #[must_use]
    pub fn with_layer(mut self, layer: i32) -> Self {
        self.layer = layer;
        self
    }

    /// The map's size in tiles, `(width, height)`.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        (x < self.width && y < self.height).then(|| y as usize * self.width as usize + x as usize)
    }

    /// The tile at `(x, y)`, or `None` for an empty cell or one outside the map.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> Option<u32> {
        let t = *self.tiles.get(self.index(x, y)?)?;
        (t != Self::EMPTY).then_some(t)
    }

    /// Puts `tile` at `(x, y)` (`None` empties it). Returns `false`, changing nothing, when the
    /// cell is outside the map. `Some(u32::MAX)` is the empty marker and empties the cell too.
    pub fn set(&mut self, x: u32, y: u32, tile: Option<u32>) -> bool {
        match self.index(x, y) {
            Some(i) => {
                self.tiles[i] = tile.unwrap_or(Self::EMPTY);
                true
            }
            None => false,
        }
    }

    /// Sets every cell of the rectangle starting at `(x, y)`, `w × h` tiles, clipped to the map.
    pub fn fill(&mut self, x: u32, y: u32, w: u32, h: u32, tile: Option<u32>) {
        for ty in y..y.saturating_add(h).min(self.height) {
            for tx in x..x.saturating_add(w).min(self.width) {
                self.set(tx, ty, tile);
            }
        }
    }

    /// Every non-empty cell, as `(x, y, tile)`, row by row.
    pub fn tiles(&self) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
        let w = self.width.max(1) as usize;
        self.tiles
            .iter()
            .enumerate()
            .filter(|(_, t)| **t != Self::EMPTY)
            .map(move |(i, t)| ((i % w) as u32, (i / w) as u32, *t))
    }

    /// The cell under a point in the map's **local** plane (the entity's frame, origin at the
    /// map's top-left, Y up) — what a game asks to find the ground under a character. `None`
    /// outside the map. The point's tile, if any, is then [`get`](Self::get).
    #[must_use]
    pub fn cell_at(&self, local: Vec2) -> Option<(u32, u32)> {
        if !(self.tile_size.x > 0.0 && self.tile_size.y > 0.0) {
            return None;
        }
        let fx = (local.x / self.tile_size.x).floor();
        let fy = (-local.y / self.tile_size.y).floor();
        if !(fx >= 0.0 && fy >= 0.0 && fx < self.width as f32 && fy < self.height as f32) {
            return None;
        }
        Some((fx as u32, fy as u32))
    }

    /// The centre of cell `(x, y)` in the map's local plane — the inverse of
    /// [`cell_at`](Self::cell_at).
    #[must_use]
    pub fn cell_center(&self, x: u32, y: u32) -> Vec2 {
        Vec2::new((x as f32 + 0.5) * self.tile_size.x, -(y as f32 + 0.5) * self.tile_size.y)
    }

    /// Each non-empty tile as the [`Sprite`] that draws it, with its offset from the map's origin.
    /// What the draw path queues; a tile whose number is past the tileset's last cell is skipped.
    pub fn tile_sprites(&self) -> impl Iterator<Item = (Vec2, Sprite)> + '_ {
        let base = Sprite {
            image: self.tileset,
            size: self.tile_size,
            color: self.color,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            flip_x: false,
            flip_y: false,
            anchor: Vec2::ZERO,
            layer: self.layer,
            space: SpriteSpace::World,
        };
        let cells = self.tileset_columns.saturating_mul(self.tileset_rows);
        self.tiles().filter(move |&(_, _, t)| t < cells).map(move |(x, y, t)| {
            let mut s = base.clone();
            s.set_frame(t, self.tileset_columns, self.tileset_rows);
            (Vec2::new(x as f32 * self.tile_size.x, -(y as f32) * self.tile_size.y), s)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> Tilemap {
        Tilemap::from_rows(SpriteImageId(0), 2, 2, Vec2::new(2.0, 1.0), &["ab.", "..b", "a"], |c| match c {
            'a' => Some(0),
            'b' => Some(3),
            _ => None,
        })
    }

    #[test]
    fn rows_read_top_down_and_the_widest_row_sets_the_width() {
        let m = map();
        assert_eq!(m.size(), (3, 3));
        assert_eq!((m.get(0, 0), m.get(1, 0), m.get(2, 0)), (Some(0), Some(3), None));
        assert_eq!(m.get(2, 1), Some(3));
        assert_eq!(m.get(0, 2), Some(0));
        assert_eq!(m.get(9, 9), None);
        let all: Vec<_> = m.tiles().collect();
        assert_eq!(all, vec![(0, 0, 0), (1, 0, 3), (2, 1, 3), (0, 2, 0)]);
    }

    #[test]
    fn set_and_fill_stay_inside_the_map() {
        let mut m = Tilemap::new(SpriteImageId(0), 1, 1, 4, 3, Vec2::ONE);
        assert!(!m.set(4, 0, Some(0)));
        m.fill(2, 1, 10, 10, Some(0));
        assert_eq!(m.tiles().count(), 4, "2 columns × 2 rows survive the clip");
        m.fill(0, 0, 4, 3, None);
        assert_eq!(m.tiles().count(), 0);
        assert!(Tilemap::new(SpriteImageId(0), 1, 1, 0, 5, Vec2::ONE).tiles().next().is_none());
    }

    /// The map hangs right and DOWN from the origin: a point just below-right of the origin is
    /// cell (0, 0), and `cell_center` inverts `cell_at`.
    #[test]
    fn cells_hang_right_and_down_from_the_origin() {
        let m = map(); // tiles 2 × 1
        assert_eq!(m.cell_at(Vec2::new(0.1, -0.1)), Some((0, 0)));
        assert_eq!(m.cell_at(Vec2::new(4.5, -1.5)), Some((2, 1)));
        assert_eq!(m.cell_at(Vec2::new(0.1, 0.1)), None, "above the map");
        assert_eq!(m.cell_at(Vec2::new(-0.1, -0.1)), None, "left of the map");
        assert_eq!(m.cell_at(Vec2::new(6.0, -0.5)), None, "past the right edge");
        for (x, y) in [(0, 0), (2, 1), (1, 2)] {
            assert_eq!(m.cell_at(m.cell_center(x, y)), Some((x, y)));
        }
    }

    /// Each tile becomes a top-left-anchored sprite at its cell's corner, showing its tileset cell.
    #[test]
    fn tiles_become_sprites_at_their_cells_showing_their_cells() {
        let m = map().with_layer(4).with_color(Vec4::new(1.0, 0.5, 0.5, 1.0));
        let sprites: Vec<_> = m.tile_sprites().collect();
        assert_eq!(sprites.len(), 4);
        let (offset, s) = &sprites[2]; // (2, 1) = tile 3
        assert_eq!(*offset, Vec2::new(4.0, -1.0));
        assert_eq!(s.uv_rect, [0.5, 0.5, 1.0, 1.0], "cell 3 of a 2 × 2 sheet is bottom-right");
        assert_eq!((s.anchor, s.size, s.layer), (Vec2::ZERO, Vec2::new(2.0, 1.0), 4));
        assert_eq!(s.color, Vec4::new(1.0, 0.5, 0.5, 1.0));
        // A tile number past the sheet draws nothing rather than wrapping onto another cell.
        let mut bad = Tilemap::new(SpriteImageId(0), 2, 1, 1, 1, Vec2::ONE);
        bad.set(0, 0, Some(2));
        assert_eq!(bad.tile_sprites().count(), 0);
    }
}
