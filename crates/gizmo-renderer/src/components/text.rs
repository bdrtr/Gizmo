use gizmo_math::{Vec2, Vec4};

use crate::text::FontId;

/// A string the engine draws — on the screen, or in the world.
///
/// The engine could not draw a glyph until this existed; every demo that wanted a label used an
/// `egui` panel, which is why they all looked like debug overlays. `docs/CAPABILITY_GAPS.md` §A2.
///
/// The font comes from [`Renderer::load_font`](crate::Renderer::load_font), and the string is
/// laid out and rasterised every frame it is drawn: there is no cached mesh, so changing
/// [`content`](Self::content) costs nothing beyond the layout, and the *glyphs* are cached in the
/// atlas rather than the string.
///
/// # What it does not do
///
/// One font per `Text`, no wrapping (lines break on `\n` and nowhere else), no shaping — see
/// [`crate::text`] for what that rules out. A `Text` with an unknown [`FontId`] draws nothing
/// rather than falling back to another font: a silent substitution is how a missing asset stops
/// looking like a missing asset.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Text {
    /// The string. `\n` breaks a line.
    pub content: String,
    /// Which loaded face to draw it with.
    pub font: FontId,
    /// Cap height in pixels — the size the glyphs are rasterised at.
    ///
    /// In [`TextSpace::World`] this is still *pixels*: the glyphs are rasterised at this size and
    /// the quad is then scaled by [`TextSpace::World::px_per_unit`], so raising this sharpens a
    /// world label instead of enlarging it.
    pub size_px: f32,
    /// Multiplied into the glyph coverage. `a` scales the whole string's opacity.
    pub color: Vec4,
    /// Which part of the text's box sits at the anchor point.
    pub anchor: TextAnchor,
    /// Screen pixels, or a camera-facing quad in the world.
    pub space: TextSpace,
}

impl Text {
    /// White text at `size_px`, anchored at its top-left, in screen space at `position`.
    #[must_use]
    pub fn screen(content: impl Into<String>, font: FontId, size_px: f32, position: Vec2) -> Self {
        Self {
            content: content.into(),
            font,
            size_px,
            color: Vec4::ONE,
            anchor: TextAnchor::TopLeft,
            space: TextSpace::Screen { position },
        }
    }

    /// White text at `size_px`, centred on the entity's `GlobalTransform`, facing the camera.
    ///
    /// `px_per_unit` is how many glyph pixels fit in one world unit: 64 makes a 32 px string half
    /// a unit tall. Centred rather than top-left because a world label is a marker on a thing,
    /// and a marker whose corner is on the thing reads as misplaced.
    #[must_use]
    pub fn world(content: impl Into<String>, font: FontId, size_px: f32, px_per_unit: f32) -> Self {
        Self {
            content: content.into(),
            font,
            size_px,
            color: Vec4::ONE,
            anchor: TextAnchor::Center,
            space: TextSpace::World { px_per_unit },
        }
    }

    /// Replaces the colour.
    #[must_use]
    pub fn with_color(mut self, color: Vec4) -> Self {
        self.color = color;
        self
    }

    /// Replaces the anchor.
    #[must_use]
    pub fn with_anchor(mut self, anchor: TextAnchor) -> Self {
        self.anchor = anchor;
        self
    }
}

/// Where a [`Text`] is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum TextSpace {
    /// Window pixels, origin at the **top-left** — the same coordinates the mouse arrives in and
    /// the same ones `gizmo-ui`'s `Node` reports, so a UI rect can be handed straight over.
    Screen {
        /// The anchor point, in window pixels.
        position: Vec2,
    },
    /// At the entity's `GlobalTransform`, as a quad turned to face the camera.
    World {
        /// Glyph pixels per world unit. Larger means smaller text.
        px_per_unit: f32,
    },
}

impl TextSpace {
    /// Whether this space needs the entity's world position.
    ///
    /// It exists so the draw loops do not have to `match` on this enum. `TextSpace` is
    /// `#[non_exhaustive]`, so a downstream crate is *obliged* to write a wildcard — and a
    /// wildcard is where a variant nobody remembered gets silently treated as something else.
    /// `crate::routing`'s module docs record the two capabilities that died in exactly that arm.
    /// Inside this crate the match is exhaustive, so a third space is a compile error **here**,
    /// once, instead of a silent misplacement out there.
    #[must_use]
    pub fn needs_world_position(self) -> bool {
        match self {
            Self::Screen { .. } => false,
            Self::World { .. } => true,
        }
    }
}

/// Which corner or edge of a text's box sits on its anchor point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TextAnchor {
    /// The default: the anchor is the top-left of the box, which is how screen coordinates read.
    #[default]
    TopLeft,
    /// Top edge, centred horizontally.
    TopCenter,
    /// Top-right corner.
    TopRight,
    /// Left edge, centred vertically.
    CenterLeft,
    /// The middle of the box.
    Center,
    /// Right edge, centred vertically.
    CenterRight,
    /// Bottom-left corner.
    BottomLeft,
    /// Bottom edge, centred horizontally.
    BottomCenter,
    /// Bottom-right corner.
    BottomRight,
}

impl TextAnchor {
    /// How much of the box's width and height to subtract from the anchor point, as fractions.
    ///
    /// `(0, 0)` is top-left and `(1, 1)` is bottom-right, in a y-grows-down space — so a caller
    /// computes `top_left = anchor - factors * size` and the nine variants collapse to one line.
    #[must_use]
    pub fn factors(self) -> Vec2 {
        let (x, y) = match self {
            Self::TopLeft => (0.0, 0.0),
            Self::TopCenter => (0.5, 0.0),
            Self::TopRight => (1.0, 0.0),
            Self::CenterLeft => (0.0, 0.5),
            Self::Center => (0.5, 0.5),
            Self::CenterRight => (1.0, 0.5),
            Self::BottomLeft => (0.0, 1.0),
            Self::BottomCenter => (0.5, 1.0),
            Self::BottomRight => (1.0, 1.0),
        };
        Vec2::new(x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The nine anchors are nine distinct placements, and the two corners are opposite.
    ///
    /// Written because `factors` is a table, and a table is where a copy-paste puts `TopRight`'s
    /// row under `TopCenter` — which no compiler catches and which reads as "the anchor is
    /// slightly off" rather than as a bug.
    #[test]
    fn every_anchor_is_a_different_place_and_the_corners_are_opposite() {
        let all = [
            TextAnchor::TopLeft,
            TextAnchor::TopCenter,
            TextAnchor::TopRight,
            TextAnchor::CenterLeft,
            TextAnchor::Center,
            TextAnchor::CenterRight,
            TextAnchor::BottomLeft,
            TextAnchor::BottomCenter,
            TextAnchor::BottomRight,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.factors(), b.factors(), "{a:?} and {b:?} anchor to the same point");
            }
        }
        assert_eq!(TextAnchor::TopLeft.factors(), Vec2::ZERO);
        assert_eq!(TextAnchor::BottomRight.factors(), Vec2::ONE);
        assert_eq!(TextAnchor::Center.factors(), Vec2::splat(0.5));
        assert_eq!(TextAnchor::default(), TextAnchor::TopLeft);
    }
}
