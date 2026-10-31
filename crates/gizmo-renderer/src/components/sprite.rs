use gizmo_math::{Vec2, Vec4};

use crate::sprite::SpriteImageId;

/// A textured quad the engine draws — in the world on the entity's plane, or on the screen.
///
/// The engine had no 2D path at all until this existed: no sprite, no 2D layering, nothing that
/// could put an image on the screen except an `egui` panel or a textured 3D cube.
/// `docs/CAPABILITY_GAPS.md` §A1.
///
/// The image comes from [`Renderer::load_sprite_image`](crate::Renderer::load_sprite_image) (or
/// its `_file` / `_rgba` siblings), and an id that was never handed out draws nothing rather than
/// a placeholder — the same rule [`Text`](super::Text) keeps for fonts, for the same reason.
///
/// # Where it is drawn
///
/// - [`SpriteSpace::World`]: a quad in the entity's **local XY plane**, `size` world units across,
///   placed by its `GlobalTransform` — so it moves, rotates and scales with the entity, and a 2D
///   game is an orthographic camera looking down `-Z` at a field of these. It does **not** turn to
///   face the camera (that is what a world-space [`Text`](super::Text) does); a sprite seen edge-on
///   is a line, which is what a card in a 3D scene should look like.
/// - [`SpriteSpace::Screen`]: window pixels, top-left origin, over the picture — a HUD.
///
/// # Order
///
/// Within each space, lower [`layer`](Self::layer) is drawn first. World sprites on the same layer
/// are then painted back to front by distance from the camera; screen sprites on the same layer in
/// entity order. World sprites test against the scene's depth and write none: a wall hides a
/// sprite behind it, and sprites never hide each other except by being painted over.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Sprite {
    /// Which loaded image to draw.
    pub image: SpriteImageId,
    /// The quad's size: world units in [`SpriteSpace::World`], pixels in [`SpriteSpace::Screen`].
    pub size: Vec2,
    /// Multiplied into the image. `a` scales its opacity.
    pub color: Vec4,
    /// The part of the image to draw, as `(u0, v0, u1, v1)` with `(0, 0)` the image's top-left.
    ///
    /// The whole image by default; a sub-rectangle is a frame of a sprite sheet —
    /// [`with_frame`](Self::with_frame) computes one for a regular grid.
    pub uv_rect: [f32; 4],
    /// Mirror the image left-to-right.
    pub flip_x: bool,
    /// Mirror the image top-to-bottom.
    pub flip_y: bool,
    /// Which point of the quad sits on the entity's position, as a fraction of its size with
    /// `(0, 0)` the **top-left** and `(1, 1)` the bottom-right — the convention
    /// [`TextAnchor::factors`](super::TextAnchor::factors) uses. `(0.5, 0.5)`, the centre, by
    /// default; `(0.5, 1.0)` stands a character on its feet.
    pub anchor: Vec2,
    /// Draw order within the space: lower first.
    pub layer: i32,
    /// On the entity's plane in the world, or in window pixels.
    pub space: SpriteSpace,
}

impl Sprite {
    /// A white-tinted world sprite `size` world units across, centred on the entity.
    #[must_use]
    pub fn world(image: SpriteImageId, size: Vec2) -> Self {
        Self {
            image,
            size,
            color: Vec4::ONE,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            flip_x: false,
            flip_y: false,
            anchor: Vec2::splat(0.5),
            layer: 0,
            space: SpriteSpace::World,
        }
    }

    /// A screen sprite `size` pixels across with its **top-left** at `position` (window pixels,
    /// origin top-left).
    ///
    /// Top-left rather than centred because that is what a HUD is laid out in, and what
    /// `gizmo-ui`'s `Node` publishes.
    #[must_use]
    pub fn screen(image: SpriteImageId, size: Vec2, position: Vec2) -> Self {
        Self {
            anchor: Vec2::ZERO,
            space: SpriteSpace::Screen { position },
            ..Self::world(image, size)
        }
    }

    /// Replaces the tint.
    #[must_use]
    pub fn with_color(mut self, color: Vec4) -> Self {
        self.color = color;
        self
    }

    /// Replaces the anchor — see [`anchor`](Self::anchor).
    #[must_use]
    pub fn with_anchor(mut self, anchor: Vec2) -> Self {
        self.anchor = anchor;
        self
    }

    /// Replaces the layer.
    #[must_use]
    pub fn with_layer(mut self, layer: i32) -> Self {
        self.layer = layer;
        self
    }

    /// Replaces the mirroring.
    #[must_use]
    pub fn with_flip(mut self, flip_x: bool, flip_y: bool) -> Self {
        self.flip_x = flip_x;
        self.flip_y = flip_y;
        self
    }

    /// Replaces the image sub-rectangle — see [`uv_rect`](Self::uv_rect).
    #[must_use]
    pub fn with_uv_rect(mut self, uv_rect: [f32; 4]) -> Self {
        self.uv_rect = uv_rect;
        self
    }

    /// Shows frame `index` of a sheet cut into `columns × rows` equal cells, counted left to
    /// right and then top to bottom.
    ///
    /// An index past the last cell wraps, so `frame = tick % count` and `frame = tick` animate the
    /// same way. A sheet with zero columns or rows has no frames and leaves the rectangle alone.
    #[must_use]
    pub fn with_frame(mut self, index: u32, columns: u32, rows: u32) -> Self {
        if let Some(rect) = sheet_frame(index, columns, rows) {
            self.uv_rect = rect;
        }
        self
    }

    /// Changes the frame in place — what an animation system calls every tick. See
    /// [`with_frame`](Self::with_frame).
    pub fn set_frame(&mut self, index: u32, columns: u32, rows: u32) {
        if let Some(rect) = sheet_frame(index, columns, rows) {
            self.uv_rect = rect;
        }
    }
}

/// Cell `index` of a `columns × rows` grid, as a UV rectangle. `None` for an empty grid.
fn sheet_frame(index: u32, columns: u32, rows: u32) -> Option<[f32; 4]> {
    let count = columns.checked_mul(rows).filter(|&n| n > 0)?;
    let i = index % count;
    let (col, row) = (i % columns, i / columns);
    let (w, h) = (1.0 / columns as f32, 1.0 / rows as f32);
    Some([col as f32 * w, row as f32 * h, (col + 1) as f32 * w, (row + 1) as f32 * h])
}

/// Where a [`Sprite`] is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum SpriteSpace {
    /// In the entity's local XY plane, placed by its `GlobalTransform`.
    World,
    /// Window pixels, origin at the **top-left**.
    Screen {
        /// Where the anchor point goes, in window pixels.
        position: Vec2,
    },
}

impl SpriteSpace {
    /// Whether this space needs the entity's world transform.
    ///
    /// Here so a draw loop outside this crate does not have to `match` a `#[non_exhaustive]` enum
    /// with a wildcard — the same reasoning as
    /// [`TextSpace::needs_world_position`](super::TextSpace::needs_world_position).
    #[must_use]
    pub fn needs_world_transform(self) -> bool {
        match self {
            Self::World => true,
            Self::Screen { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_count_left_to_right_then_down_and_wrap() {
        // 4 × 2: frame 5 is column 1 of row 1.
        assert_eq!(sheet_frame(5, 4, 2), Some([0.25, 0.5, 0.5, 1.0]));
        assert_eq!(sheet_frame(0, 4, 2), Some([0.0, 0.0, 0.25, 0.5]));
        assert_eq!(sheet_frame(8, 4, 2), sheet_frame(0, 4, 2), "past the last cell wraps");
    }

    #[test]
    fn an_empty_sheet_leaves_the_rectangle_alone() {
        let s = Sprite::world(SpriteImageId(0), Vec2::ONE).with_uv_rect([0.1, 0.2, 0.3, 0.4]);
        assert_eq!(s.clone().with_frame(3, 0, 4).uv_rect, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(s.with_frame(3, 4, 0).uv_rect, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(sheet_frame(0, u32::MAX, u32::MAX), None, "an overflowing grid is not a grid");
    }

    #[test]
    fn screen_sprites_default_to_their_top_left_and_world_ones_to_their_centre() {
        let s = Sprite::screen(SpriteImageId(0), Vec2::ONE, Vec2::new(3.0, 4.0));
        assert_eq!(s.anchor, Vec2::ZERO);
        assert!(!s.space.needs_world_transform());
        let w = Sprite::world(SpriteImageId(0), Vec2::ONE);
        assert_eq!(w.anchor, Vec2::splat(0.5));
        assert!(w.space.needs_world_transform());
    }
}
