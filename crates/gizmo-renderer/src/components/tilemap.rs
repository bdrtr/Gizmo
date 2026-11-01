use gizmo_math::{Mat4, Vec2, Vec3, Vec4};

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

    /// The cells a camera can see, as `[x0, y0, x1, y1)` (ends exclusive), given the matrix from
    /// the map's local plane to clip space (`view_proj * entity_matrix`).
    ///
    /// The four corners of the screen are cast through the view onto the map's plane, and the
    /// cells under the hull of those four points — plus one cell of margin — are the visible
    /// ones. **Conservative**: when any corner's ray misses the plane (a perspective camera that
    /// sees past the horizon, a view edge-on to the map) or the matrix cannot be inverted, the
    /// whole map is returned. Never smaller than what is on screen; that is what the tests hold.
    #[must_use]
    pub fn visible_cells(&self, clip_from_map: Mat4) -> [u32; 4] {
        let all = [0, 0, self.width, self.height];
        if self.width == 0 || self.height == 0 || !(self.tile_size.x > 0.0 && self.tile_size.y > 0.0) {
            return all;
        }
        let inv = clip_from_map.inverse();
        if !inv.is_finite() {
            return all;
        }
        let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for ndc in [Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0), Vec2::new(-1.0, 1.0), Vec2::new(1.0, 1.0)] {
            // Near (z = 0) and far (z = 1) points of this corner's ray, in map space.
            let unproject = |z: f32| {
                let p = inv * Vec4::new(ndc.x, ndc.y, z, 1.0);
                (p.w.abs() > 1e-12).then(|| p.truncate() / p.w)
            };
            let (Some(a), Some(b)) = (unproject(0.0), unproject(1.0)) else { return all };
            let Some(hit) = ray_hits_plane(a, b) else { return all };
            lo = lo.min(hit);
            hi = hi.max(hit);
        }
        if !(lo.is_finite() && hi.is_finite()) {
            return all;
        }
        // Map space: x right, y UP; cells count rows DOWN from y = 0.
        let col = |x: f32| (x / self.tile_size.x).floor();
        let row = |y: f32| (-y / self.tile_size.y).floor();
        let clamp = |v: f32, max: u32| v.clamp(0.0, max as f32) as u32;
        let x0 = clamp(col(lo.x) - 1.0, self.width);
        let x1 = clamp(col(hi.x) + 2.0, self.width);
        let y0 = clamp(row(hi.y) - 1.0, self.height);
        let y1 = clamp(row(lo.y) + 2.0, self.height);
        [x0, y0, x1.max(x0), y1.max(y0)]
    }

    /// Each non-empty tile as the [`Sprite`] that draws it, with its offset from the map's origin.
    /// What the draw path queues; a tile whose number is past the tileset's last cell is skipped.
    pub fn tile_sprites(&self) -> impl Iterator<Item = (Vec2, Sprite)> + '_ {
        self.tile_sprites_in([0, 0, self.width, self.height])
    }

    /// [`tile_sprites`](Self::tile_sprites) for the cells `[x0, y0, x1, y1)` only — what the draw
    /// path asks for with [`visible_cells`](Self::visible_cells), so an off-screen tile costs
    /// nothing.
    pub fn tile_sprites_in(&self, cells: [u32; 4]) -> impl Iterator<Item = (Vec2, Sprite)> + '_ {
        let [x0, y0, x1, y1] = cells;
        let (x1, y1) = (x1.min(self.width), y1.min(self.height));
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
        let sheet = self.tileset_columns.saturating_mul(self.tileset_rows);
        (y0..y1)
            .flat_map(move |y| (x0..x1).map(move |x| (x, y)))
            .filter_map(move |(x, y)| self.get(x, y).map(|t| (x, y, t)))
            .filter(move |&(_, _, t)| t < sheet)
            .map(move |(x, y, t)| {
            let mut s = base.clone();
            s.set_frame(t, self.tileset_columns, self.tileset_rows);
            (Vec2::new(x as f32 * self.tile_size.x, -(y as f32) * self.tile_size.y), s)
        })
    }
}

/// Where the segment `a → b`, extended into a line, meets the plane `z = 0` — `None` when it runs
/// parallel to the plane or meets it only behind `a` (the camera).
fn ray_hits_plane(a: Vec3, b: Vec3) -> Option<Vec2> {
    let d = b - a;
    if d.z.abs() < 1e-9 {
        return None;
    }
    let t = -a.z / d.z;
    (t >= 0.0).then(|| (a + d * t).truncate())
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

    fn ortho_view(centre: Vec2, half: Vec2) -> Mat4 {
        let proj = Mat4::orthographic_rh(-half.x, half.x, -half.y, half.y, 0.1, 100.0);
        let view = Mat4::look_at_rh(centre.extend(10.0), centre.extend(0.0), Vec3::Y);
        proj * view
    }

    fn big_map() -> Tilemap {
        let mut m = Tilemap::new(SpriteImageId(0), 1, 1, 1000, 500, Vec2::ONE);
        m.fill(0, 0, 1000, 500, Some(0));
        m
    }

    /// A small orthographic view of a big map asks for a small window of it — and every tile
    /// whose quad reaches the screen is inside that window.
    #[test]
    fn an_orthographic_view_sees_a_window_that_holds_every_tile_on_screen() {
        let m = big_map();
        // Looking at x ∈ [90, 110], y ∈ [-55, -45] → columns 90..110, rows 45..55.
        let vp = ortho_view(Vec2::new(100.0, -50.0), Vec2::new(10.0, 5.0));
        let [x0, y0, x1, y1] = m.visible_cells(vp);
        assert!((x1 - x0) * (y1 - y0) < 400, "asked for {} cells of 500 000", (x1 - x0) * (y1 - y0));
        let window: std::collections::HashSet<_> = m.tile_sprites_in([x0, y0, x1, y1]).map(|(o, _)| o.to_array().map(f32::to_bits)).collect();
        let mut on_screen = 0;
        for (offset, s) in m.tile_sprites() {
            // Any corner inside the clip box means the tile is on screen.
            let visible = [(0.0, 0.0), (1.0, 0.0), (0.0, -1.0), (1.0, -1.0)].iter().any(|&(dx, dy)| {
                let c = vp * Vec4::new(offset.x + dx * s.size.x, offset.y + dy * s.size.y, 0.0, 1.0);
                c.x.abs() < c.w && c.y.abs() < c.w
            });
            if visible {
                on_screen += 1;
                assert!(window.contains(&offset.to_array().map(f32::to_bits)), "tile at {offset:?} is on screen but culled");
            }
        }
        assert!(on_screen >= 200, "the probe saw only {on_screen} tiles — it checks nothing");
    }

    /// A perspective camera tilted to see past the horizon cannot bound the map, so it gets all of
    /// it; one looking straight at it gets a window.
    #[test]
    fn a_view_that_sees_the_horizon_gets_the_whole_map() {
        let m = big_map();
        let proj = Mat4::perspective_rh(1.0, 1.6, 0.1, 1000.0);
        let straight = proj * Mat4::look_at_rh(Vec3::new(100.0, -50.0, 20.0), Vec3::new(100.0, -50.0, 0.0), Vec3::Y);
        let [x0, y0, x1, y1] = m.visible_cells(straight);
        assert!(x1 - x0 < 100 && y1 - y0 < 100, "{:?}", [x0, y0, x1, y1]);
        let grazing = proj * Mat4::look_at_rh(Vec3::new(100.0, -50.0, 2.0), Vec3::new(100.0, 50.0, 1.5), Vec3::Z);
        assert_eq!(m.visible_cells(grazing), [0, 0, 1000, 500]);
        assert_eq!(m.visible_cells(Mat4::ZERO), [0, 0, 1000, 500], "a singular matrix culls nothing");
    }
}
