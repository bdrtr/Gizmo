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
/// One font per `Text`, plus the library's fallback chain — see
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
    /// Wrap lines wider than this many glyph pixels — at spaces, or between characters for a
    /// word that cannot fit on its own. `None` (the default) breaks only at `\n`.
    ///
    /// In glyph pixels in both spaces, like [`size_px`](Self::size_px): a world label 256 px wide
    /// at 64 px per unit wraps at 4 world units.
    pub max_width: Option<f32>,
    /// How lines of different widths line up inside the text's box: on the side they start on (the
    /// default, so a right-to-left paragraph lines up on the right), left, centred or right. The box
    /// itself is placed by [`anchor`](Self::anchor); this moves lines within it, so it matters only
    /// for text with more than one line.
    pub align: TextAlign,
    /// Colours for parts of [`content`](Self::content), set by [`rich`](Self::rich): each entry is the
    /// byte offset where a colour starts, ascending. Empty for a text of one colour. Multiplied by
    /// [`color`](Self::color), so that still fades the whole string.
    pub spans: Vec<(u32, Vec4)>,
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
            max_width: None,
            align: TextAlign::Start,
            spans: Vec::new(),
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
            max_width: None,
            align: TextAlign::Start,
            spans: Vec::new(),
        }
    }

    /// White text at `size_px`, pinned to a point of the **window** that follows its size.
    ///
    /// `anchor` names both ends: the window's point (its centre, its top-right corner, …) and the
    /// part of the text's box that sits on it, so `TopRight` hangs the box from the top-right
    /// corner and `Center` centres it. `offset` is window pixels from there, y down — a HUD in
    /// the top-right corner 16 px in is `pinned(.., TextAnchor::TopRight, Vec2::new(-16.0, 16.0))`.
    /// See [`TextSpace::Pinned`] for why this is not a [`screen`](Self::screen) position.
    #[must_use]
    pub fn pinned(content: impl Into<String>, font: FontId, size_px: f32, anchor: TextAnchor, offset: Vec2) -> Self {
        Self {
            content: content.into(),
            font,
            size_px,
            color: Vec4::ONE,
            anchor,
            space: TextSpace::Pinned { at: anchor.factors(), offset },
            max_width: None,
            align: TextAlign::Start,
            spans: Vec::new(),
        }
    }

    /// White text at `size_px`, pinned to **any** point of the window: `at` is a fraction of its
    /// size (`(0.5, 0.3)` is centred, 30 % down), `offset` window pixels from there, y down.
    ///
    /// The part of the text's box that sits on the point is [`anchor`](Self::anchor), top-left
    /// by default — a title centred at 30 % is
    /// `pinned_at(.., Vec2::new(0.5, 0.3), Vec2::ZERO).with_anchor(TextAnchor::Center)`.
    /// [`pinned`](Self::pinned) is this with `at` taken from the anchor.
    #[must_use]
    pub fn pinned_at(content: impl Into<String>, font: FontId, size_px: f32, at: Vec2, offset: Vec2) -> Self {
        Self {
            content: content.into(),
            font,
            size_px,
            color: Vec4::ONE,
            anchor: TextAnchor::TopLeft,
            space: TextSpace::Pinned { at, offset },
            max_width: None,
            align: TextAlign::Start,
            spans: Vec::new(),
        }
    }

    /// Makes [`content`](Self::content) out of `parts`, each drawn in its own colour.
    ///
    /// ```ignore
    /// Text::screen("", font, 24.0, at).rich([("Can: ", WHITE), ("5", RED)])
    /// ```
    #[must_use]
    pub fn rich<S: AsRef<str>>(mut self, parts: impl IntoIterator<Item = (S, Vec4)>) -> Self {
        self.content.clear();
        self.spans.clear();
        for (part, color) in parts {
            self.spans.push((self.content.len() as u32, color));
            self.content.push_str(part.as_ref());
        }
        self
    }

    /// The colour of the character at byte offset `at`, [`color`](Self::color) included.
    #[must_use]
    pub fn color_at(&self, at: u32) -> Vec4 {
        let span = self.spans.partition_point(|(start, _)| *start <= at);
        match span.checked_sub(1) {
            Some(i) => self.spans[i].1 * self.color,
            None => self.color,
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

    /// Replaces the line alignment — see [`align`](Self::align).
    #[must_use]
    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// Wraps lines at `max_width` glyph pixels — see [`max_width`](Self::max_width).
    #[must_use]
    pub fn with_max_width(mut self, max_width: f32) -> Self {
        self.max_width = Some(max_width);
        self
    }
}

/// How the lines of a multi-line [`Text`] line up with each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TextAlign {
    /// On the side each line starts on: right for a right-to-left line, left for the rest.
    #[default]
    Start,
    /// Flush left — every line starts at the box's left edge.
    Left,
    /// Each line centred in the box.
    Center,
    /// Flush right.
    Right,
}

impl TextAlign {
    /// How far across the slack each line moves: 0 left, 0.5 centre, 1 right, and 0 for
    /// [`Start`](Self::Start), which depends on the line and goes through
    /// [`TextLayout::align_start`](crate::text::TextLayout::align_start) — the argument
    /// [`TextLayout::align`](crate::text::TextLayout::align) takes.
    #[must_use]
    pub fn factor(self) -> f32 {
        match self {
            Self::Start | Self::Left => 0.0,
            Self::Center => 0.5,
            Self::Right => 1.0,
        }
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
    /// Window pixels measured from a point that moves with the window: `at`, a fraction of the
    /// window's size (`(0, 0)` top-left, `(1, 1)` bottom-right), plus `offset`.
    ///
    /// [`Screen`](Self::Screen) is absolute, so a title centred at `(640, 240)` for a 1280-wide
    /// window is off-screen in a 472-wide one — which is where `kristal_avcisi`'s menu went the
    /// first time it ran under a tiling window manager. Before this existed every HUD that wanted
    /// a centre or a right-hand corner re-wrote its own `position` from `WindowInfo` each frame
    /// (`demo/src/bin/text.rs`, `kule_yikici`).
    Pinned {
        /// The point, as a fraction of the window's width and height, y down.
        at: Vec2,
        /// Window pixels from that point, y down.
        offset: Vec2,
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
            Self::Screen { .. } | Self::Pinned { .. } => false,
            Self::World { .. } => true,
        }
    }

    /// Where on a `viewport`-sized window the text's anchor point lands, in window pixels — or
    /// `None` for a text that lives in the world.
    ///
    /// The one place a [`Pinned`](Self::Pinned) fraction becomes a pixel, so the renderer and any
    /// host that wants to know where a label went agree.
    #[must_use]
    pub fn screen_point(self, viewport: Vec2) -> Option<Vec2> {
        match self {
            Self::Screen { position } => Some(position),
            Self::Pinned { at, offset } => Some(viewport * at + offset),
            Self::World { .. } => None,
        }
    }

    /// This space with a [`Pinned`](Self::Pinned) margin at `k` times its size; anything else is
    /// unchanged. A margin is a length the author wrote, while a [`Screen`](Self::Screen) position
    /// is a place in the frame — often computed from it, as a world point's pixel is — and stays.
    #[must_use]
    pub fn with_scaled_margin(self, k: f32) -> Self {
        match self {
            Self::Pinned { at, offset } => Self::Pinned { at, offset: offset * k },
            other => other,
        }
    }

    /// The authored window-pixel anchor, or `None` for a text that lives in the world — or is
    /// [`Pinned`](Self::Pinned), whose pixel depends on the window
    /// ([`screen_point`](Self::screen_point) answers that).
    ///
    /// Here for the same reason as [`needs_world_position`](Self::needs_world_position): reading
    /// the position out of the enum is a `match`, and a `match` downstream is a wildcard. A host
    /// that overrides the position — the UI bridge does, from a `Node` — needs the authored value
    /// as its fallback, and this is how it gets one without opening the enum.
    #[must_use]
    pub fn screen_position(self) -> Option<Vec2> {
        match self {
            Self::Screen { position } => Some(position),
            Self::Pinned { .. } | Self::World { .. } => None,
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
