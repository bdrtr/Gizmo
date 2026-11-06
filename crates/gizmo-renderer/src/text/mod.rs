//! Fonts, glyph rasterisation and text layout — the CPU half of drawing a string.
//!
//! # What this is
//!
//! Until this module the engine could not draw a glyph. Every demo that wanted a label used an
//! `egui` panel, which is a debug/editor UI layer rather than a game text system, and that is why
//! every one of them looked like a debug overlay. `docs/CAPABILITY_GAPS.md` §A2 is this gap and
//! §E names it the first to close.
//!
//! Three things live here and nothing else does:
//!
//! - [`FontLibrary`](crate::text::FontLibrary) owns the loaded faces and answers metric questions about them.
//! - [`GlyphAtlas`](crate::text::GlyphAtlas) rasterises glyphs on demand into one GPU texture and remembers where it put
//!   them.
//! - `layout` turns a string into placed glyphs, in text-local pixels.
//!
//! The GPU half — pipelines, the instance buffer, the pass — is [`crate::text::render`]. The split
//! is deliberate: everything in this file is testable with no adapter, and most of what can go
//! wrong with text (a baseline off by the ascent, an atlas that reuses a shelf, a UV rect flipped)
//! is in this half.
//!
//! # The dependency, and why it is sealed
//!
//! Rasterisation is `ab_glyph`: it turns an outline into coverage and answers metric questions.
//! Shaping and bidi are `harfrust` and `unicode-bidi`, in `shape`, which decide which glyphs go where.
//! Together they are sealable — none of their types appears on any `pub` signature
//! here. The surface is [`FontId`](crate::text::FontId), [`LineMetrics`](crate::text::LineMetrics), [`PositionedGlyph`](crate::text::PositionedGlyph) and [`GlyphEntry`](crate::text::GlyphEntry), all
//! ours. `gizmo-renderer` is a Stage B crate (`docs/ENGINE.md` §4), so a foreign type on its
//! surface would be *allowed*; it is sealed anyway because sealing it costs one newtype.
//!
//! # What it does not do
//!
//! One size and one face per text (colours can differ, see `Text::rich`), and no language setting:
//! a face's `locl` lookups see its defaults, and Latin text
//! gets no ligatures (see `shape`). Font fallback is a single chain for the whole library
//! ([`FontLibrary::add_fallback`](crate::text::FontLibrary::add_fallback)): a character the text's own
//! face has no glyph for is drawn from the first fallback face that has one. Base direction is found
//! per paragraph; a bidi control that opens on one wrapped line does not carry to the next. Lines
//! break on `\n`, and — given a width ([`FontLibrary::layout_wrapped`](crate::text::FontLibrary::layout_wrapped)) —
//! at spaces, which is right for scripts that separate words with spaces and wrong for those that do
//! not (Chinese, Japanese, Thai wrap only between characters here). Vertical text is not laid out.
//! These are named here rather than discovered later: a text system that silently draws the wrong
//! glyphs for a script is worse than one that says it cannot.

use std::collections::HashMap;

use ab_glyph::{Font as _, ScaleFont as _};

/// The GPU half: pipelines, the instance buffer and the draw.
pub mod render;
mod shape;
#[doc(hidden)]
pub mod synthetic;
#[cfg(feature = "bundled-font")]
pub mod bundled;

pub use render::TextRenderer;

/// A face loaded into a [`FontLibrary`].
///
/// Opaque on purpose — it is an index, and the library owning the face is what keeps it valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(pub(crate) u32);

impl FontId {
    /// The raw index, for a caller that needs to key its own map by font.
    #[must_use]
    pub fn index(self) -> u32 {
        self.0
    }
}

/// Why a font could not be loaded.
#[derive(Debug)]
#[non_exhaustive]
pub enum FontError {
    /// The bytes are not a font this engine can read.
    ///
    /// Deliberately carries no parser type: the underlying error is one line of prose with no
    /// structure worth matching on, and putting it here would make a rasteriser's next major a
    /// breaking change for anyone who wrote `match`.
    Invalid,
    /// The file could not be read.
    Io(std::io::Error),
    /// [`Renderer::load_system_font`](crate::Renderer::load_system_font) found no readable font
    /// in `GIZMO_FONT` or the platform's usual places (always, on the web).
    NotFound,
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid => write!(f, "not a readable font"),
            Self::Io(e) => write!(f, "could not read the font file: {e}"),
            Self::NotFound => write!(f, "no system font found (set GIZMO_FONT to a .ttf/.otf file)"),
        }
    }
}

impl std::error::Error for FontError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid | Self::NotFound => None,
            Self::Io(e) => Some(e),
        }
    }
}

/// A font's vertical metrics at one pixel size, in pixels.
///
/// [`ascent`](Self::ascent) is positive **above** the baseline and [`descent`](Self::descent) is
/// negative below it, which is the convention the font file itself uses — flipping one of them to
/// "both positive" reads nicer and then costs a sign error at every call site.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct LineMetrics {
    /// Distance from the baseline to the top of the tallest glyph. Positive.
    pub ascent: f32,
    /// Distance from the baseline to the bottom of the lowest glyph. Negative.
    pub descent: f32,
    /// The font's recommended extra space between lines.
    pub line_gap: f32,
}

impl LineMetrics {
    /// Baseline-to-baseline distance: `ascent - descent + line_gap`.
    #[must_use]
    pub fn line_height(self) -> f32 {
        self.ascent - self.descent + self.line_gap
    }
}

/// One glyph placed by `layout`, in text-local pixels.
///
/// The origin is the **first line's baseline at the left edge**, x growing right and y growing
/// *down* — the direction screen pixels run, so a screen-space caller adds nothing and a
/// world-space one flips y exactly once.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PositionedGlyph {
    /// The face the glyph comes from: the text's own, or a fallback that covers the character
    /// (see [`FontLibrary::add_fallback`]).
    pub font: FontId,
    /// The face's own glyph index, which is what the atlas is keyed by. Not a `char`: one
    /// character can map to several glyphs and several characters to one.
    pub glyph: u16,
    /// Pen position of this glyph's origin.
    pub x: f32,
    /// Baseline of the line this glyph is on.
    pub y: f32,
    /// Which line (0-based) the glyph is on — what [`TextLayout::align`] shifts it by.
    pub line: u32,
    /// The size to rasterise it at. The text's own size for its own face. A fallback face gets
    /// whatever gives it the same em as the text's face: a size here means a face's height from
    /// ascent to descent, and in Noto Sans Arabic that height takes 2.1 em against Noto Sans's 1.4.
    pub px: f32,
    /// Byte offset into the text of the characters this glyph draws: what a coloured span of
    /// [`Text`](crate::components::Text) is matched against.
    pub cluster: u32,
}

/// What `layout` produced: the glyphs, and the box they occupy.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct TextLayout {
    /// The placed glyphs, each line left to right on screen, which for right-to-left text is not
    /// the order it is read in. Glyphs with no outline (a space) are **not** here:
    /// they advance the pen and rasterise to nothing, so carrying them would mean every consumer
    /// filtering them out again.
    pub glyphs: Vec<PositionedGlyph>,
    /// Width of the widest line.
    pub width: f32,
    /// `lines * line_height`.
    pub height: f32,
    /// How many lines there are — from `\n` and from wrapping. Always at least 1, including for
    /// the empty string.
    pub lines: u32,
    /// Each line's own width, in order: `line_widths.len() == lines`. `width` is the largest.
    pub line_widths: Vec<f32>,
    /// Whether each line runs right to left, in order: `line_rtl.len() == lines`.
    pub line_rtl: Vec<bool>,
}

impl TextLayout {
    /// Moves each line across the box: `0.0` keeps lines flush left (as laid out), `0.5` centres
    /// each one in the widest line's width, `1.0` makes them flush right. A one-line text does
    /// not move — its line *is* the box.
    pub fn align(&mut self, factor: f32) {
        if factor == 0.0 || !factor.is_finite() {
            return;
        }
        for g in &mut self.glyphs {
            let line_width = self.line_widths.get(g.line as usize).copied().unwrap_or(self.width);
            g.x += (self.width - line_width) * factor;
        }
    }

    /// Moves each line to the side its text starts on: right-to-left lines flush right, the rest
    /// flush left.
    pub fn align_start(&mut self) {
        for g in &mut self.glyphs {
            if self.line_rtl.get(g.line as usize).copied().unwrap_or(false) {
                let line_width = self.line_widths.get(g.line as usize).copied().unwrap_or(self.width);
                g.x += self.width - line_width;
            }
        }
    }
}

/// The loaded faces.
///
/// Faces are never unloaded: a [`FontId`] handed out stays valid for the library's lifetime. That
/// is a deliberate first cut and it is the same choice [`GlyphAtlas`] makes about glyphs — the
/// eviction policy is the interesting part of both, and a wrong one is worse than none.
#[derive(Default)]
pub struct FontLibrary {
    fonts: Vec<ab_glyph::FontVec>,
    /// Tried in order for a character the text's own face does not cover.
    fallbacks: Vec<FontId>,
    shaping: shape::Shaping,
    /// Layouts already worked out. Text is laid out again every frame it is drawn, and shaping a
    /// wrapped paragraph takes about a millisecond even in release.
    recent: std::sync::Mutex<HashMap<LayoutKey, HashMap<String, TextLayout>>>,
}

/// Face, size and wrap width, as bits: what a layout depends on, next to its text.
type LayoutKey = (u32, u32, Option<u32>);

/// Layouts kept per [`LayoutKey`]. A counter or a timer makes a new string every frame, so the map
/// is emptied at this size rather than left to grow.
const RECENT_LAYOUTS: usize = 256;

impl std::fmt::Debug for FontLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontLibrary").field("fonts", &self.fonts.len()).finish()
    }
}

impl FontLibrary {
    /// An empty library.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many faces are loaded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    /// Whether no face is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    /// Loads a face from `ttf`/`otf` bytes.
    ///
    /// # Errors
    ///
    /// [`FontError::Invalid`] if the bytes are not a font the rasteriser can parse.
    pub fn load(&mut self, bytes: Vec<u8>) -> Result<FontId, FontError> {
        let font = ab_glyph::FontVec::try_from_vec(bytes).map_err(|_| FontError::Invalid)?;
        if !self.shaping.add_face(font.as_slice()) {
            return Err(FontError::Invalid);
        }
        self.fonts.push(font);
        // `len` is bounded by how many fonts a process loads; the cast is unreachable in practice
        // and saturating is the honest answer rather than a panic in a renderer.
        Ok(FontId(u32::try_from(self.fonts.len() - 1).unwrap_or(u32::MAX)))
    }

    /// Loads a face from a file.
    ///
    /// # Errors
    ///
    /// [`FontError::Io`] if the file cannot be read, [`FontError::Invalid`] if it is not a font.
    pub fn load_file(&mut self, path: impl AsRef<std::path::Path>) -> Result<FontId, FontError> {
        let bytes = std::fs::read(path).map_err(FontError::Io)?;
        self.load(bytes)
    }

    /// The face behind an id, or `None` if the id came from another library.
    fn face(&self, font: FontId) -> Option<&ab_glyph::FontVec> {
        self.fonts.get(font.0 as usize)
    }

    /// Adds `font` to the end of the fallback chain: a character that a text's own face has no
    /// glyph for — a symbol, an emoji, a script the face does not cover — is drawn from the first
    /// face in the chain that has one, instead of as the face's empty "missing" glyph, at the em size
    /// of the text's own face. Shaping and kerning work within a stretch of one face, not across a
    /// change of face, and line height comes from the text's own face. An unknown id, or one already
    /// in the chain, changes nothing.
    pub fn add_fallback(&mut self, font: FontId) {
        if self.face(font).is_some() && !self.fallbacks.contains(&font) {
            self.fallbacks.push(font);
            self.recent.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
        }
    }

    /// The fallback chain, in the order it is tried.
    #[must_use]
    pub fn fallbacks(&self) -> &[FontId] {
        &self.fallbacks
    }

    /// Vertical metrics at `px`, or `None` for an id this library does not know.
    #[must_use]
    pub fn metrics(&self, font: FontId, px: f32) -> Option<LineMetrics> {
        let scaled = self.face(font)?.as_scaled(px);
        Some(LineMetrics {
            ascent: scaled.ascent(),
            descent: scaled.descent(),
            line_gap: scaled.line_gap(),
        })
    }

    /// Places `text` at `px`, breaking on `\n`.
    ///
    /// Returns `None` for an id this library does not know — which is a different answer from an
    /// empty layout, and the caller can tell the two apart.
    #[must_use]
    pub fn layout(&self, font: FontId, px: f32, text: &str) -> Option<TextLayout> {
        self.layout_wrapped(font, px, text, None)
    }

    /// [`layout`](Self::layout), also breaking any line wider than `max_width` pixels.
    ///
    /// Lines break at spaces, and the space a line breaks at is dropped. A word wider than
    /// `max_width` on its own is split between characters — text that overflows its box is the
    /// failure a width exists to prevent, so it is not allowed even for one long word. `None`, or
    /// a width that is not finite and positive, wraps nothing.
    #[must_use]
    pub fn layout_wrapped(&self, font: FontId, px: f32, text: &str, max_width: Option<f32>) -> Option<TextLayout> {
        let faces = Faces::new(self, font)?;
        let max_width = max_width.filter(|w| w.is_finite() && *w > 0.0);
        let key = (font.0, px.to_bits(), max_width.map(f32::to_bits));
        let mut recent = self.recent.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let same_style = recent.entry(key).or_default();
        if let Some(found) = same_style.get(text) {
            return Some(found.clone());
        }
        let laid_out = layout(&faces, px, text, max_width);
        if same_style.len() >= RECENT_LAYOUTS {
            same_style.clear();
        }
        same_style.insert(text.to_string(), laid_out.clone());
        Some(laid_out)
    }
}

/// A text's face and the library's fallback chain: which face draws each character.
struct Faces<'a> {
    library: &'a FontLibrary,
    primary: (FontId, &'a ab_glyph::FontVec),
    chain: Vec<(FontId, &'a ab_glyph::FontVec)>,
}

impl<'a> Faces<'a> {
    fn new(library: &'a FontLibrary, font: FontId) -> Option<Self> {
        let primary = (font, library.face(font)?);
        let chain = library
            .fallbacks
            .iter()
            .filter(|&&f| f != font)
            .filter_map(|&f| library.face(f).map(|face| (f, face)))
            .collect();
        Some(Self { library, primary, chain })
    }

    /// The face that draws `c` and its glyph there: the text's own when it has one (glyph 0 is
    /// "missing"), else the first fallback that does, else the text's own missing glyph.
    fn pick(&self, c: char) -> (FontId, &'a ab_glyph::FontVec, ab_glyph::GlyphId) {
        let (font, face) = self.primary;
        let id = face.glyph_id(c);
        if id.0 != 0 {
            return (font, face, id);
        }
        self.chain
            .iter()
            .find_map(|&(f, fv)| {
                let g = fv.glyph_id(c);
                (g.0 != 0).then_some((f, fv, g))
            })
            .unwrap_or((font, face, id))
    }

    /// The size `font` is drawn at in a text of `px`: see [`PositionedGlyph::px`].
    fn px_for(&self, font: FontId, px: f32) -> f32 {
        let (primary_id, primary) = self.primary;
        match self.library.face(font) {
            Some(face) if font != primary_id => px * em_height(face) / em_height(primary),
            _ => px,
        }
    }

    /// Whether `font` has a glyph of its own for `c`.
    fn covers(&self, font: FontId, c: char) -> bool {
        self.library.face(font).is_some_and(|face| face.glyph_id(c).0 != 0)
    }
}

/// A face's height from ascent to descent, in ems.
fn em_height(face: &ab_glyph::FontVec) -> f32 {
    face.height_unscaled() / face.units_per_em().unwrap_or(1000.0)
}

/// The pen advance of `s` on one line — the width [`layout`] would give it.
fn measure(faces: &Faces<'_>, px: f32, s: &str) -> f32 {
    shape::shape_line(faces, px, s, None).width
}

/// Breaks one `\n`-free paragraph into lines no wider than `max` — see
/// [`FontLibrary::layout_wrapped`] for the rule.
fn wrap_paragraph(faces: &Faces<'_>, px: f32, paragraph: &str, max: f32, out: &mut Vec<String>) {
    let fits = |s: &str| measure(faces, px, s) <= max;
    let mut line = String::new();
    let mut started = false;
    // `split(' ')`, not `split_whitespace`: runs of spaces inside a line are kept as authored.
    for word in paragraph.split(' ') {
        let candidate = if started { format!("{line} {word}") } else { word.to_string() };
        if !started || fits(&candidate) {
            line = candidate;
            started = true;
        } else {
            out.push(std::mem::replace(&mut line, word.to_string()));
        }
        // A word too wide on its own: cut it between characters, at least one per line.
        while !fits(&line) && line.chars().nth(1).is_some() {
            let mut cut = line.char_indices().nth(1).map_or(line.len(), |(i, _)| i);
            for (i, _) in line.char_indices().skip(2) {
                if fits(&line[..i]) {
                    cut = i;
                } else {
                    break;
                }
            }
            let rest = line.split_off(cut);
            out.push(std::mem::replace(&mut line, rest));
        }
    }
    out.push(line);
}

/// Places `text` at `px`, breaking on `\n` and, given a width, wherever a line would exceed it.
/// The library-free half, so it can be tested against a face directly.
fn layout(faces: &Faces<'_>, px: f32, text: &str, max_width: Option<f32>) -> TextLayout {
    // Line height is the text's own face's, whatever a fallback glyph would want.
    let scaled = faces.primary.1.as_scaled(px);
    let metrics = LineMetrics {
        ascent: scaled.ascent(),
        descent: scaled.descent(),
        line_gap: scaled.line_gap(),
    };
    let line_height = metrics.line_height();

    let mut out = TextLayout { lines: 1, ..Default::default() };
    let mut widest: f32 = 0.0;
    // Each line with its paragraph's direction and where it starts in `text`, which is what a glyph's
    // `cluster` counts from.
    let mut lines: Vec<(std::borrow::Cow<'_, str>, Option<unicode_bidi::Level>, usize)> = Vec::new();
    let mut paragraph_start = 0;
    for paragraph in text.split('\n') {
        match max_width {
            None => lines.push((std::borrow::Cow::Borrowed(paragraph), None, paragraph_start)),
            Some(max) => {
                let level = shape::paragraph_level(paragraph);
                let mut cut = Vec::new();
                wrap_paragraph(faces, px, paragraph, max, &mut cut);
                // Every wrapped line is a piece of the paragraph, in order, with only the spaces
                // it broke at between them.
                let mut cursor = 0;
                for line in cut {
                    let at = paragraph[cursor..].find(line.as_str()).map_or(cursor, |i| cursor + i);
                    cursor = at + line.len();
                    lines.push((std::borrow::Cow::Owned(line), level, paragraph_start + at));
                }
            }
        }
        paragraph_start += paragraph.len() + 1;
    }
    for (line_index, (line, level, line_start)) in lines.iter().enumerate() {
        let baseline = metrics.ascent + line_index as f32 * line_height;
        let shaped = shape::shape_line(faces, px, line, *level);
        for g in shaped.glyphs {
            // A glyph with no outline — a space, or a codepoint this face does not cover — still
            // advanced the pen and rasterises to nothing. Keeping it out of `glyphs` is what lets
            // every consumer treat the list as "things to draw".
            let drawn = faces.library.face(g.font).is_some_and(|face| face.outline(ab_glyph::GlyphId(g.glyph)).is_some());
            if drawn {
                out.glyphs.push(PositionedGlyph {
                    font: g.font,
                    glyph: g.glyph,
                    x: g.x,
                    y: baseline + g.y,
                    line: line_index as u32,
                    px: g.px,
                    cluster: *line_start as u32 + g.cluster,
                });
            }
        }
        widest = widest.max(shaped.width);
        out.line_widths.push(shaped.width);
        out.line_rtl.push(shaped.rtl);
        out.lines = u32::try_from(line_index + 1).unwrap_or(u32::MAX);
    }
    out.width = widest;
    out.height = out.lines as f32 * line_height;
    out
}

/// Where one rasterised glyph sits in the atlas, and how to place it against a pen position.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct GlyphEntry {
    /// Texture coordinates of the glyph's box: `[u0, v0, u1, v1]`, already divided by the atlas
    /// size.
    pub uv: [f32; 4],
    /// The glyph's size in pixels.
    pub size: [f32; 2],
    /// Offset from the pen position (x) and the baseline (y) to the glyph box's **top-left**, with
    /// y growing down. Add it to a [`PositionedGlyph`] and the result is where the box goes.
    pub offset: [f32; 2],
}

/// The key a glyph is cached under.
///
/// `px` is rounded to a whole pixel: a size slider sweeping 15.9 → 16.1 must not rasterise three
/// copies of every glyph into an atlas that never evicts. The cost is that text at 15.9 px is
/// rasterised at 16 and scaled by 0.994 when drawn, which is invisible and bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: u32,
    glyph: u16,
    px: u32,
}

/// The rasterised glyphs, in one GPU texture.
///
/// Packing is a **shelf**: glyphs are placed left to right along a row whose height is the tallest
/// glyph in it, and a new row starts when the current one runs out of width. That wastes the space
/// above short glyphs on a tall row and it is the right first cut — a real packer matters when the
/// atlas is close to full, and this one reports when it is instead of pretending.
///
/// Nothing is ever evicted. See [`is_full`](Self::is_full) for what happens when it fills.
pub struct GlyphAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    packer: ShelfPacker,
    /// Texture coordinates of a fully-opaque patch, packed before any glyph.
    ///
    /// It is what lets a solid quad — a UI background, a highlight — go through the same pipeline
    /// and the same instance buffer as the text: sample this and the coverage is 1 everywhere, so
    /// the fragment is the instance's own colour. The alternative is a second pipeline whose only
    /// difference is that it does not sample a texture.
    solid_uv: [f32; 4],
    entries: HashMap<GlyphKey, GlyphEntry>,
    /// Set once a glyph did not fit. Sticky: the atlas does not recover by itself.
    full: bool,
}

/// Where the next glyph goes.
///
/// Its own struct rather than four fields on [`GlyphAtlas`] for one reason: it is the half of the
/// atlas with no wgpu in it and the half that can silently overlap two glyphs, so it has to be
/// testable without an adapter. A test that reimplements the placement it is checking proves the
/// reimplementation.
#[derive(Debug, Clone, Copy)]
struct ShelfPacker {
    /// The atlas's side length.
    size: u32,
    /// Left edge of the next glyph on the current shelf.
    cursor_x: u32,
    /// Top edge of the current shelf.
    shelf_y: u32,
    /// Height of the current shelf.
    shelf_h: u32,
}

impl ShelfPacker {
    fn new(size: u32) -> Self {
        Self { size, cursor_x: GLYPH_PADDING, shelf_y: GLYPH_PADDING, shelf_h: 0 }
    }

    /// Reserves a `w`×`h` box, starting a new shelf when the current one runs out of width.
    /// `None` when the glyph does not fit anywhere — including "bigger than the whole atlas",
    /// which no shelf will ever hold and which must not wrap around.
    fn reserve(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let pad = GLYPH_PADDING;
        if w + 2 * pad > self.size || h + 2 * pad > self.size {
            return None;
        }
        if self.cursor_x + w + pad > self.size {
            self.shelf_y += self.shelf_h + pad;
            self.shelf_h = 0;
            self.cursor_x = pad;
        }
        if self.shelf_y + h + pad > self.size {
            return None;
        }
        let (x, y) = (self.cursor_x, self.shelf_y);
        self.cursor_x += w + pad;
        self.shelf_h = self.shelf_h.max(h);
        Some((x, y))
    }
}

impl std::fmt::Debug for GlyphAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphAtlas")
            .field("size", &self.packer.size)
            .field("glyphs", &self.entries.len())
            .field("full", &self.full)
            .finish()
    }
}

/// One pixel of padding around every glyph, so a linear sampler on one glyph cannot reach into its
/// neighbour. Without it, text at a non-integer position grows a faint edge of whatever was packed
/// beside it.
const GLYPH_PADDING: u32 = 1;

impl GlyphAtlas {
    /// Allocates a `size`×`size` single-channel atlas, its bind group, and the solid patch.
    ///
    /// 1024 is the size the engine uses: at 32 px that is roughly a thousand glyphs, and it costs
    /// one megabyte. The `queue` is needed at construction because the solid patch is uploaded
    /// here — an atlas that could not draw a background until its first glyph arrived would be a
    /// startup order nobody would guess.
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, size: u32) -> Self {
        let size = size.max(64);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Coverage, not colour: one byte per pixel, read as `.r` and multiplied into the
            // text's own colour by the shader.
            format: wgpu::TextureFormat::R8Unorm,
            // `COPY_SRC` is not needed to draw with; it is there so the atlas can be read back
            // and looked at — by a test asserting a glyph landed where the entry says, or by
            // anyone debugging text that renders as blank boxes. `PostProcessState::hdr_texture`
            // carries it for the same reason.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Glyph Atlas Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Glyph Atlas Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Glyph Atlas Bind Group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let mut packer = ShelfPacker::new(size);
        // Packed FIRST, so it exists whatever happens to the atlas later — a full atlas must still
        // be able to draw a background. `SOLID` is larger than the one texel a solid quad needs:
        // the UV below addresses the middle of it, so a linear sampler at any scale stays inside
        // the patch instead of blending in whatever glyph was packed beside it.
        const SOLID: u32 = 4;
        let (sx, sy) = packer
            .reserve(SOLID, SOLID)
            .expect("an atlas is at least 64 px and the solid patch is 4");
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: sx, y: sy, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &[0xFF; (SOLID * SOLID) as usize],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SOLID),
                rows_per_image: Some(SOLID),
            },
            wgpu::Extent3d { width: SOLID, height: SOLID, depth_or_array_layers: 1 },
        );
        let s = size as f32;
        let solid_uv = [
            (sx + 1) as f32 / s,
            (sy + 1) as f32 / s,
            (sx + SOLID - 1) as f32 / s,
            (sy + SOLID - 1) as f32 / s,
        ];

        Self {
            texture,
            view,
            bind_group,
            layout,
            packer,
            solid_uv,
            entries: HashMap::new(),
            full: false,
        }
    }

    /// The bind group holding the atlas texture and its sampler.
    #[must_use]
    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// Its layout, for building a pipeline that binds the atlas.
    #[must_use]
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// Texture coordinates of the fully-opaque patch — what a solid quad samples.
    #[must_use]
    pub fn solid_uv(&self) -> [f32; 4] {
        self.solid_uv
    }

    /// The atlas texture, for a caller that wants to look at it.
    #[must_use]
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// Its view.
    #[must_use]
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// The atlas's side length in pixels.
    #[must_use]
    pub fn size(&self) -> u32 {
        self.packer.size
    }

    /// How many distinct (font, glyph, size) triples are packed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is packed yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether a glyph has failed to fit.
    ///
    /// Sticky, and it never clears: once the atlas is full, later glyphs are simply not drawn, and
    /// a renderer that quietly stopped drawing some of the text is exactly the failure this flag
    /// exists to make visible. A caller that hits it wants a bigger atlas.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// Rasterises `glyph` at `px` if it is not already packed, and returns where it landed.
    ///
    /// `None` means the glyph has no outline (a space) **or** the atlas is full — the caller draws
    /// nothing either way, and [`is_full`](Self::is_full) separates the two.
    pub fn glyph(
        &mut self,
        queue: &wgpu::Queue,
        library: &FontLibrary,
        font: FontId,
        glyph: u16,
        px: f32,
    ) -> Option<GlyphEntry> {
        let key = GlyphKey { font: font.0, glyph, px: px.round().max(1.0) as u32 };
        if let Some(entry) = self.entries.get(&key) {
            return Some(*entry);
        }
        if self.full {
            return None;
        }
        let face = library.face(font)?;
        let scale = ab_glyph::PxScale::from(key.px as f32);
        let outlined = face.outline_glyph(ab_glyph::Glyph {
            id: ab_glyph::GlyphId(glyph),
            scale,
            position: ab_glyph::point(0.0, 0.0),
        })?;
        let bounds = outlined.px_bounds();
        let w = bounds.width().ceil().max(1.0) as u32;
        let h = bounds.height().ceil().max(1.0) as u32;

        let Some((x, y)) = self.packer.reserve(w, h) else {
            self.full = true;
            tracing::warn!(
                atlas = self.packer.size,
                packed = self.entries.len(),
                "[Text] glyph atlas is full; further glyphs will not be drawn"
            );
            return None;
        };

        let mut pixels = vec![0u8; (w * h) as usize];
        outlined.draw(|gx, gy, coverage| {
            if gx < w && gy < h {
                // Coverage is 0..1 area, and it is the only channel: the shader multiplies it into
                // the text colour's alpha.
                pixels[(gy * w + gx) as usize] = (coverage * 255.0 + 0.5).min(255.0) as u8;
            }
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );

        let s = self.packer.size as f32;
        let entry = GlyphEntry {
            uv: [x as f32 / s, y as f32 / s, (x + w) as f32 / s, (y + h) as f32 / s],
            size: [w as f32, h as f32],
            // `px_bounds` is in ab_glyph's coordinates: y grows DOWN and the origin is the
            // baseline, which is already the convention `PositionedGlyph` uses. So this is the
            // offset as-is, and the one place a sign would flip is a caller that decided baselines
            // grow up.
            offset: [bounds.min.x, bounds.min.y],
        };
        self.entries.insert(key, entry);
        Some(entry)
    }

}

#[cfg(test)]
mod tests {
    /// The bundled Noto Sans, then the Arabic and Devanagari test faces.
    fn shaping_faces() -> (FontLibrary, FontId, FontId, FontId) {
        let mut lib = FontLibrary::new();
        let latin = lib.load(include_bytes!("../../assets/fonts/NotoSans-Regular.ttf").to_vec()).expect("Noto Sans");
        let arabic = lib.load(include_bytes!("fixtures/noto-sans-arabic-subset.ttf").to_vec()).expect("Arabic");
        let deva = lib.load(include_bytes!("fixtures/noto-sans-devanagari-subset.ttf").to_vec()).expect("Devanagari");
        (lib, latin, arabic, deva)
    }

    fn glyph_ids(lib: &FontLibrary, font: FontId, text: &str) -> Vec<u16> {
        lib.layout(font, 32.0, text).expect("laid out").glyphs.iter().map(|g| g.glyph).collect()
    }

    /// **Arabic letters join, and the first one read is the rightmost.** س has one form alone and
    /// another at the start of a word, and which it takes is the shaper's whole job here.
    #[test]
    fn arabic_letters_join_and_run_right_to_left() {
        let (lib, _, arabic, _) = shaping_faces();
        let alone = glyph_ids(&lib, arabic, "س")[0];
        let spaced = glyph_ids(&lib, arabic, "س ل ا م");
        let joined = glyph_ids(&lib, arabic, "سلام");
        assert_eq!(spaced.len(), 4);
        assert_eq!(*spaced.last().expect("four"), alone, "read first, so drawn last from the left");
        assert_eq!(joined.len(), 4);
        assert_ne!(*joined.last().expect("four"), alone, "the word's first letter takes its initial form");
    }

    /// **Devanagari reorders and forms conjuncts.** The vowel ि is typed after क and drawn before it,
    /// and क्ष is one glyph from three characters.
    #[test]
    fn devanagari_reorders_its_vowel_and_forms_a_conjunct() {
        let (lib, _, _, deva) = shaping_faces();
        let ka = glyph_ids(&lib, deva, "क")[0];
        let ki = glyph_ids(&lib, deva, "कि");
        assert_eq!(ki.len(), 2);
        assert_eq!(ki[1], ka, "the consonant comes second on screen");
        assert_eq!(glyph_ids(&lib, deva, "क्ष").len(), 1);
    }

    /// **A script change inside one face is a new run**, and a character of no script of its own
    /// stays with what came before it.
    #[test]
    fn a_change_of_script_within_a_face_starts_a_new_run() {
        let (lib, latin, _, _) = shaping_faces();
        let faces = Faces::new(&lib, latin).expect("a face");
        let cut = |text: &str| -> Vec<String> {
            shape::itemize_for_test(&faces, text).into_iter().map(|r| text[r].to_string()).collect()
        };
        assert_eq!(cut("abc, где"), ["abc, ", "где"]);
        assert_eq!(cut("12 abc"), ["12 abc"], "digits take the script that follows them");
        assert_eq!(cut("12 "), ["12 "], "a run with no script at all is still one run");
    }

    /// **The paragraph's first strong letter sets the direction.** Latin first: Latin on the left,
    /// Arabic after it. Arabic first: the Arabic is read first, so it is on the right.
    #[test]
    fn mixed_text_is_ordered_by_its_paragraph_direction() {
        let (mut lib, latin, arabic, _) = shaping_faces();
        lib.add_fallback(arabic);
        let mean_x = |text: &str, font: FontId| {
            let l = lib.layout(latin, 32.0, text).expect("laid out");
            let xs: Vec<f32> = l.glyphs.iter().filter(|g| g.font == font).map(|g| g.x).collect();
            assert!(!xs.is_empty(), "{text}: no glyph from that face");
            xs.iter().sum::<f32>() / xs.len() as f32
        };
        assert!(mean_x("abc سلام", latin) < mean_x("abc سلام", arabic), "left-to-right paragraph");
        assert!(mean_x("سلام abc", latin) < mean_x("سلام abc", arabic), "right-to-left paragraph");
        // The Latin word is read first in the first string and second in the other, and in both it
        // ends up on the left. Reading order tells them apart only with three pieces.
        let first = lib.layout(latin, 32.0, "سلام abc سلام").expect("laid out");
        let arabic_xs: Vec<f32> = first.glyphs.iter().filter(|g| g.font == arabic).map(|g| g.x).collect();
        let latin_xs: Vec<f32> = first.glyphs.iter().filter(|g| g.font == latin).map(|g| g.x).collect();
        let (lo, hi) = (latin_xs[0], latin_xs[latin_xs.len() - 1]);
        assert!(arabic_xs.iter().any(|x| *x < lo) && arabic_xs.iter().any(|x| *x > hi), "abc stays between the two words");
        assert!(latin_xs.windows(2).all(|w| w[0] < w[1]), "abc still reads left to right inside");
    }

    /// **Inside a right-to-left run the pieces are reversed too.** The Arabic face has no space, so
    /// the space comes from the Latin face and cuts this run into three pieces. The word read first
    /// must still be on the right.
    #[test]
    fn words_split_by_a_change_of_face_keep_their_right_to_left_order() {
        let (mut lib, latin, arabic, _) = shaping_faces();
        lib.add_fallback(arabic);
        let second_word = glyph_ids(&lib, arabic, "مرحبا");
        let both = glyph_ids(&lib, latin, "سلام مرحبا");
        assert_eq!(both[..second_word.len()], second_word[..], "the word read second comes first from the left");
    }

    /// **A mark goes where the face's `GPOS` says.** Above a capital that is higher than the mark was
    /// drawn, which only the vertical offset can do; y grows down.
    #[test]
    fn a_mark_is_lifted_over_a_tall_letter() {
        let (lib, latin, _, _) = shaping_faces();
        let l = lib.layout(latin, 32.0, "Q\u{301}").expect("laid out");
        assert_eq!(l.glyphs.len(), 2, "no precomposed Q with an acute, so a letter and a mark");
        assert!(l.glyphs[1].y < l.glyphs[0].y - 1.0, "mark at {}, letter at {}", l.glyphs[1].y, l.glyphs[0].y);
    }

    /// **A wrapped line keeps its paragraph's direction**, even when it starts with a Latin word and
    /// would call itself left to right on its own.
    #[test]
    fn a_wrapped_line_keeps_the_direction_of_its_paragraph() {
        let (mut lib, latin, arabic, _) = shaping_faces();
        lib.add_fallback(arabic);
        let wide = |s: &str| lib.layout(latin, 32.0, s).expect("laid out").width;
        let width = wide("abc سلام").max(wide("سلام سلام")) + 2.0;
        let l = lib.layout_wrapped(latin, 32.0, "سلام سلام abc سلام", Some(width)).expect("laid out");
        assert_eq!(l.lines, 2);
        let on_second = |font: FontId| l.glyphs.iter().filter(|g| g.line == 1 && g.font == font).map(|g| g.x).sum::<f32>();
        let count = |font: FontId| l.glyphs.iter().filter(|g| g.line == 1 && g.font == font).count() as f32;
        assert!(on_second(latin) / count(latin) > on_second(arabic) / count(arabic), "read first, so on the right");
    }

    /// **Kerning comes from `GPOS`.** Noto Sans has no `kern` table, so the engine drew it unkerned
    /// until shaping arrived.
    #[test]
    fn a_face_with_only_gpos_kerning_is_kerned() {
        let (lib, latin, _, _) = shaping_faces();
        let width = |s: &str| lib.layout(latin, 32.0, s).expect("laid out").width;
        assert!(width("AV") < width("A") + width("V") - 0.5, "AV {} against {} + {}", width("AV"), width("A"), width("V"));
    }

    /// **A base letter and its combining mark compose**, rather than the mark landing after the letter.
    #[test]
    fn a_combining_mark_composes_with_its_letter() {
        let (lib, latin, _, _) = shaping_faces();
        assert_eq!(glyph_ids(&lib, latin, "e\u{301}"), glyph_ids(&lib, latin, "é"));
    }

    /// **Latin text has no ligatures**: Noto Sans's `fi` hides the dot that tells i from dotless i.
    #[test]
    fn latin_text_keeps_i_and_dotless_i_apart() {
        let (lib, latin, _, _) = shaping_faces();
        let i = glyph_ids(&lib, latin, "i")[0];
        let dotless = glyph_ids(&lib, latin, "ı")[0];
        assert_eq!(glyph_ids(&lib, latin, "fi")[1], i);
        assert_eq!(glyph_ids(&lib, latin, "fı")[1], dotless);
    }

    /// **A fallback face is as big as the text's own.** A size means a face's height from ascent to
    /// descent, and Noto Sans Arabic is far taller in ems than Noto Sans, so at the same number its
    /// letters came out about two thirds as big beside Latin.
    #[test]
    fn a_fallback_face_is_drawn_at_the_em_size_of_the_text() {
        let (mut lib, latin, arabic, _) = shaping_faces();
        lib.add_fallback(arabic);
        let l = lib.layout(latin, 40.0, "Aس").expect("laid out");
        let em = |g: &PositionedGlyph| g.px / em_height(lib.face(g.font).expect("a face"));
        assert_eq!(l.glyphs[0].px, 40.0, "the text's own face keeps the size asked for");
        assert_eq!(l.glyphs[1].font, arabic);
        assert!((em(&l.glyphs[0]) - em(&l.glyphs[1])).abs() < 0.01, "{} against {}", em(&l.glyphs[0]), em(&l.glyphs[1]));
        // The Arabic face on its own at that size gives the advance the fallback was given.
        let alone = lib.layout(arabic, l.glyphs[1].px, "س").expect("laid out");
        assert!((l.width - l.glyphs[1].x - alone.width).abs() < 0.01);
    }

    /// **A glyph knows which bytes of the text it draws,** across lines, wraps and multi-byte letters,
    /// and in a right-to-left run, where the screen order is the reverse of the text's.
    #[test]
    fn a_glyph_knows_where_in_the_text_it_came_from() {
        let (mut lib, latin, arabic, _) = shaping_faces();
        let clusters = |l: &TextLayout| l.glyphs.iter().map(|g| g.cluster).collect::<Vec<_>>();
        assert_eq!(clusters(&lib.layout(latin, 32.0, "ab\ncd").expect("laid out")), [0, 1, 3, 4]);
        assert_eq!(clusters(&lib.layout(latin, 32.0, "şğa").expect("laid out")), [0, 2, 4], "bytes, not characters");

        let word = lib.layout(latin, 32.0, "kelime").expect("laid out").width;
        let wrapped = lib.layout_wrapped(latin, 32.0, "iki kelime", Some(word + 2.0)).expect("laid out");
        assert_eq!(wrapped.lines, 2);
        assert_eq!(clusters(&wrapped), [0, 1, 2, 4, 5, 6, 7, 8, 9], "the second line starts after the space");

        lib.add_fallback(arabic);
        let rtl = lib.layout(arabic, 32.0, "س ل").expect("laid out");
        assert_eq!(clusters(&rtl), [3, 0], "the letter read second is drawn first from the left");
    }

    /// **A right-to-left paragraph starts on the right.** Each line of it is flush with the right edge
    /// of the box, and left-to-right lines beside it stay on the left.
    #[test]
    fn align_start_puts_each_line_on_the_side_it_starts_on() {
        let (mut lib, latin, arabic, _) = shaping_faces();
        lib.add_fallback(arabic);
        let mut l = lib.layout(latin, 32.0, "سلام\niki kelime uzun").expect("laid out");
        assert_eq!(l.line_rtl, [true, false]);
        let before: Vec<f32> = l.glyphs.iter().map(|g| g.x).collect();
        l.align_start();
        let moved = l.glyphs.iter().zip(&before).filter(|(g, _)| g.line == 0).map(|(g, b)| g.x - b).next().expect("a glyph");
        assert!((moved - (l.width - l.line_widths[0])).abs() < 1e-3, "the Arabic line moved {moved}");
        assert!(l.glyphs.iter().zip(&before).filter(|(g, _)| g.line == 1).all(|(g, b)| g.x == *b), "the Latin line stayed");
    }

    /// **Only Latin gives up `liga`.** Thai writes ฤๅ as one glyph, through that same feature.
    #[test]
    fn other_scripts_keep_their_ligatures() {
        let mut lib = FontLibrary::new();
        let thai = lib.load(include_bytes!("fixtures/noto-sans-thai-subset.ttf").to_vec()).expect("Thai");
        assert_eq!(glyph_ids(&lib, thai, "ฤๅ").len(), 1);
    }

    /// A layout remembered before a fallback was added must not outlive it: the same string now has
    /// glyphs from the new face.
    #[test]
    fn adding_a_fallback_forgets_the_layouts_it_changes() {
        let (mut lib, latin, arabic, _) = shaping_faces();
        let before = lib.layout(latin, 32.0, "سلام").expect("laid out");
        assert!(before.glyphs.iter().all(|g| g.font == latin));
        assert_eq!(lib.layout(latin, 32.0, "سلام").expect("laid out"), before, "remembered, and the same");
        lib.add_fallback(arabic);
        let after = lib.layout(latin, 32.0, "سلام").expect("laid out");
        assert!(after.glyphs.iter().all(|g| g.font == arabic), "still the layout from before the fallback");
    }

    /// Each style remembers its own layouts: another size or width is another answer.
    #[test]
    fn remembered_layouts_are_kept_apart_by_size_and_width() {
        let (lib, latin, _, _) = shaping_faces();
        let small = lib.layout(latin, 16.0, "iki kelime").expect("laid out");
        let large = lib.layout(latin, 32.0, "iki kelime").expect("laid out");
        assert!(large.width > small.width * 1.5);
        let word = lib.layout(latin, 32.0, "kelime").expect("laid out").width;
        let wrapped = lib.layout_wrapped(latin, 32.0, "iki kelime", Some(word + 2.0)).expect("laid out");
        assert_eq!((large.lines, wrapped.lines), (1, 2));
    }

    /// A string a frame must not grow the memory without bound.
    #[test]
    fn remembered_layouts_stay_bounded() {
        let (lib, latin, _, _) = shaping_faces();
        for i in 0..RECENT_LAYOUTS * 3 {
            let _ = lib.layout(latin, 16.0, &i.to_string());
        }
        let recent = lib.recent.lock().expect("not poisoned");
        assert!(recent.values().all(|m| m.len() <= RECENT_LAYOUTS));
    }


    /// A character the text's face lacks is drawn from the first fallback that has it, at the
    /// pen position the text's own advance gives; the text's own characters stay its own; with no
    /// chain nothing changes; unknown and repeated ids do not join the chain.
    #[test]
    fn a_missing_character_is_drawn_from_the_fallback_chain() {
        use super::synthetic::{synthetic_face, synthetic_face_drawing, ADVANCE, EM};
        let mut lib = FontLibrary::new();
        let a = lib.load(synthetic_face()).unwrap();
        let b = lib.load(synthetic_face_drawing('B')).unwrap();
        let fonts = |l: &TextLayout| l.glyphs.iter().map(|g| (g.font, g.glyph)).collect::<Vec<_>>();

        let before = lib.layout(a, 100.0, "AB").unwrap();
        assert_eq!(fonts(&before), vec![(a, 1)], "without a chain the B is the face's missing glyph");

        lib.add_fallback(FontId(99));
        lib.add_fallback(b);
        lib.add_fallback(b);
        assert_eq!(lib.fallbacks(), &[b]);
        let after = lib.layout(a, 100.0, "AB").unwrap();
        assert_eq!(fonts(&after), vec![(a, 1), (b, 1)]);
        let step = f32::from(ADVANCE) / f32::from(EM) * 100.0;
        assert!((after.glyphs[1].x - step).abs() < 1e-3, "{}", after.glyphs[1].x);
        assert_eq!(after.width, before.width, "the same advances either way");
        // The fallback's own face, used directly, does not fall back to itself — and the chain
        // holds nothing else, so its missing `A` stays missing.
        assert_eq!(fonts(&lib.layout(b, 100.0, "AB").unwrap()), vec![(b, 1)]);
    }
    use super::*;

    use super::synthetic::{synthetic_face, ADVANCE, ASCENT, DESCENT, EM, GLYPH_BOX};

    /// The face every test here uses — see [`super::synthetic`] for why it is built rather than
    /// shipped.
    fn library() -> (FontLibrary, FontId) {
        let mut lib = FontLibrary::new();
        let id = lib.load(synthetic_face()).expect("the synthetic face must parse");
        (lib, id)
    }

    /// One em in pixels, where a font unit is a pixel and every assertion below is arithmetic.
    const PX: f32 = EM as f32;

    #[test]
    fn a_synthetic_font_loads_and_reports_the_metrics_it_was_built_with() {
        let (lib, id) = library();
        let m = lib.metrics(id, PX).expect("metrics for a known id");
        // At a scale of exactly one em, a font unit is a pixel.
        assert!((m.ascent - ASCENT as f32).abs() < 0.5, "ascent {} vs {ASCENT}", m.ascent);
        assert!((m.descent - DESCENT as f32).abs() < 0.5, "descent {} vs {DESCENT}", m.descent);
        assert!(m.line_height() > m.ascent, "line height must clear the ascent");
    }

    #[test]
    fn an_unknown_id_is_none_rather_than_a_panic() {
        let (lib, _) = library();
        assert!(lib.metrics(FontId(99), 16.0).is_none());
        assert!(lib.layout(FontId(99), 16.0, "x").is_none());
    }

    #[test]
    fn the_pen_advances_by_the_advance_the_font_declares() {
        let (lib, id) = library();
        let one = lib.layout(id, PX, "A").expect("layout");
        let two = lib.layout(id, PX, "AA").expect("layout");
        assert_eq!(one.glyphs.len(), 1);
        assert_eq!(two.glyphs.len(), 2);
        assert!((two.glyphs[1].x - ADVANCE as f32).abs() < 0.5, "second glyph at {}", two.glyphs[1].x);
        assert!(
            (two.width - one.width - ADVANCE as f32).abs() < 0.5,
            "two glyphs are one advance wider than one: {} vs {}",
            two.width,
            one.width
        );
    }

    /// A space advances the pen and produces no glyph to draw.
    ///
    /// Both halves matter. Dropping it from `glyphs` is the contract; forgetting to advance the
    /// pen for it is the bug that contract invites, and it shows up as "A B" rendering as "AB".
    #[test]
    fn a_space_advances_without_producing_a_glyph() {
        let (lib, id) = library();
        let joined = lib.layout(id, PX, "AA").expect("layout");
        let spaced = lib.layout(id, PX, "A A").expect("layout");
        assert_eq!(spaced.glyphs.len(), 2, "the space must not be in the draw list");
        assert!(
            spaced.glyphs[1].x > joined.glyphs[1].x + 1.0,
            "the space did not advance the pen: {} vs {}",
            spaced.glyphs[1].x,
            joined.glyphs[1].x
        );
    }

    #[test]
    fn newlines_make_lines_and_the_box_grows_by_one_line_height() {
        let (lib, id) = library();
        let one = lib.layout(id, PX, "A").expect("layout");
        let two = lib.layout(id, PX, "A\nA").expect("layout");
        let m = lib.metrics(id, PX).expect("metrics");
        assert_eq!(one.lines, 1);
        assert_eq!(two.lines, 2);
        assert!((two.height - one.height - m.line_height()).abs() < 0.01);
        assert!(
            (two.glyphs[1].y - two.glyphs[0].y - m.line_height()).abs() < 0.01,
            "the second line's baseline is one line height below the first"
        );
        assert!(
            (two.width - one.width).abs() < 0.01,
            "two identical lines are as wide as one, not twice as wide"
        );
    }

    /// Words wrap at spaces; the box is as wide as the widest line and never wider than the width.
    /// Every advance is 600 px at this size, so each expectation is arithmetic.
    #[test]
    fn a_width_wraps_at_spaces_and_drops_the_space_it_breaks_at() {
        let (lib, id) = library();
        let adv = ADVANCE as f32;
        // "AA AA AA" in 2.2 advances: each "AA" fits, "AA AA" (5) does not.
        let w = lib.layout_wrapped(id, PX, "AA AA AA", Some(2.2 * adv)).expect("layout");
        assert_eq!(w.lines, 3);
        assert!((w.width - 2.0 * adv).abs() < 0.5, "width {}", w.width);
        assert_eq!(w.glyphs.len(), 6);
        assert!(w.glyphs.iter().all(|g| g.x < 2.0 * adv), "a broken-at space shifted a line start");
        // Wide enough for two words on a line: "AA AA" / "AA".
        let two = lib.layout_wrapped(id, PX, "AA AA AA", Some(5.5 * adv)).expect("layout");
        assert_eq!(two.lines, 2);
        // No width, or a nonsense one, wraps nothing — the old behaviour.
        for none in [None, Some(0.0), Some(f32::NAN), Some(-5.0)] {
            assert_eq!(lib.layout_wrapped(id, PX, "AA AA AA", none).expect("layout").lines, 1);
        }
        // `\n` still breaks, and wrapping happens within each paragraph.
        let paras = lib.layout_wrapped(id, PX, "AA AA\nAA", Some(2.2 * adv)).expect("layout");
        assert_eq!(paras.lines, 3);
    }

    /// One word wider than the width is cut between characters rather than overflowing; a width
    /// narrower than one character still puts one character on each line.
    #[test]
    fn a_word_wider_than_the_width_is_cut_between_characters() {
        let (lib, id) = library();
        let adv = ADVANCE as f32;
        let cut = lib.layout_wrapped(id, PX, "AAAAA", Some(2.5 * adv)).expect("layout");
        assert_eq!(cut.lines, 3, "AA / AA / A");
        assert!(cut.width <= 2.5 * adv);
        let tiny = lib.layout_wrapped(id, PX, "AAA", Some(1.0)).expect("layout");
        assert_eq!((tiny.lines, tiny.glyphs.len()), (3, 3), "one character per line, none lost");
    }

    /// Centred and right-aligned lines sit in the widest line's box; left is what layout gives.
    #[test]
    fn align_moves_each_line_across_the_widest_one() {
        let (lib, id) = library();
        let adv = ADVANCE as f32;
        let base = lib.layout(id, PX, "AAA\nA").expect("layout");
        assert_eq!(base.line_widths.len(), 2);
        assert!((base.line_widths[0] - 3.0 * adv).abs() < 0.5 && (base.line_widths[1] - adv).abs() < 0.5);
        let short_line_x = |l: &TextLayout| l.glyphs.iter().find(|g| g.line == 1).unwrap().x;

        let mut centre = base.clone();
        centre.align(0.5);
        assert!((short_line_x(&centre) - adv).abs() < 0.5, "centred: one advance in, got {}", short_line_x(&centre));
        let mut right = base.clone();
        right.align(1.0);
        assert!((short_line_x(&right) - 2.0 * adv).abs() < 0.5);
        // The widest line never moves, and neither does the box.
        assert_eq!(right.glyphs[0].x, base.glyphs[0].x);
        assert_eq!((right.width, right.height), (base.width, base.height));
    }

    #[test]
    fn the_empty_string_is_one_empty_line_rather_than_zero() {
        let (lib, id) = library();
        let empty = lib.layout(id, PX, "").expect("layout");
        assert_eq!(empty.lines, 1);
        assert_eq!(empty.width, 0.0);
        assert!(empty.glyphs.is_empty());
        assert!(empty.height > 0.0, "an empty line still occupies a line");
    }

    #[test]
    fn layout_scales_with_the_pixel_size() {
        let (lib, id) = library();
        let small = lib.layout(id, PX / 2.0, "AA").expect("layout");
        let big = lib.layout(id, PX, "AA").expect("layout");
        assert!(
            (big.width - 2.0 * small.width).abs() < 1.0,
            "doubling the size doubles the width: {} vs {}",
            big.width,
            small.width
        );
    }

    /// The shelf packer, exercised without a GPU.
    ///
    /// Every rectangle it hands out is checked against every earlier one for overlap — the
    /// property, not a sample of it.
    #[test]
    fn the_shelf_packer_never_overlaps_two_glyphs() {
        let mut atlas = ShelfPacker::new(64);
        let mut placed: Vec<(u32, u32, u32, u32)> = Vec::new();
        // Sizes chosen to force several shelves and to mix tall with short, which is where a
        // shelf packer goes wrong.
        for i in 0..40u32 {
            let w = 3 + i % 11;
            let h = 2 + (i * 7) % 9;
            let Some((x, y)) = atlas.reserve(w, h) else { break };
            assert!(x + w <= atlas.size && y + h <= atlas.size, "ran off the atlas");
            for &(px, py, pw, ph) in &placed {
                let disjoint = x + w <= px || px + pw <= x || y + h <= py || py + ph <= y;
                assert!(disjoint, "({x},{y},{w},{h}) overlaps ({px},{py},{pw},{ph})");
            }
            placed.push((x, y, w, h));
        }
        assert!(placed.len() > 10, "only {} rectangles fitted in 64²", placed.len());
    }

    #[test]
    fn a_glyph_larger_than_the_atlas_is_refused_rather_than_wrapped() {
        let mut atlas = ShelfPacker::new(64);
        assert!(atlas.reserve(80, 8).is_none(), "a too-wide glyph must not be placed");
        assert!(atlas.reserve(8, 80).is_none(), "a too-tall glyph must not be placed");
        // …and refusing it must not have disturbed the cursor, so ordinary glyphs still fit.
        assert!(atlas.reserve(8, 8).is_some());
    }

    /// The glyph the synthetic font declares is a box of known size, so the outline the rasteriser
    /// produces can be checked against arithmetic rather than against a picture.
    #[test]
    fn the_synthetic_glyph_outlines_at_the_size_it_declares() {
        let (lib, id) = library();
        let face = lib.face(id).expect("face");
        let scaled = face.as_scaled(PX);
        let outlined = scaled
            .outline_glyph(ab_glyph::Glyph {
                id: face.glyph_id('A'),
                scale: ab_glyph::PxScale::from(PX),
                position: ab_glyph::point(0.0, 0.0),
            })
            .expect("the box glyph has an outline");
        let b = outlined.px_bounds();
        assert!(
            (b.width() - GLYPH_BOX as f32).abs() < 2.0,
            "the box is {} px wide, the font says {GLYPH_BOX}",
            b.width()
        );
        assert!(
            (b.height() - GLYPH_BOX as f32).abs() < 2.0,
            "the box is {} px tall, the font says {GLYPH_BOX}",
            b.height()
        );
    }

    /// A rasterised glyph reaches the atlas texture, where the entry says it is.
    ///
    /// The CPU tests above check what the atlas *claims*; this checks the texture. Both halves are
    /// needed and the second is the one that catches a UV rect divided by the wrong size, a row
    /// pitch that is not the glyph width, or an origin passed in the wrong order — none of which
    /// change any number the CPU side reports.
    ///
    /// The synthetic face's one glyph is a filled box, so the expected image is arithmetic: at
    /// `px` the box is `GLYPH_BOX/EM * px` on a side and every pixel inside it is fully covered.
    #[test]
    fn a_rasterised_glyph_lands_where_the_atlas_says_it_did() {
        let _gpu = crate::test_gpu::gpu_lock();
        let Some((device, queue)) = pollster::block_on(crate::test_gpu::headless_device()) else {
            eprintln!("skipping: no usable GPU adapter");
            return;
        };
        let (lib, id) = library();
        let mut atlas = GlyphAtlas::new(&device, &queue, 64);
        assert!(atlas.is_empty());

        const SIZE: f32 = 32.0;
        let glyph = lib.layout(id, SIZE, "A").expect("layout").glyphs[0].glyph;
        let entry = atlas
            .glyph(&queue, &lib, id, glyph, SIZE)
            .expect("the box glyph rasterises");

        let expected = f32::from(GLYPH_BOX) / f32::from(EM) * SIZE;
        assert!(
            (entry.size[0] - expected).abs() <= 1.0 && (entry.size[1] - expected).abs() <= 1.0,
            "the box is {:?} px, the font's own numbers say {expected}",
            entry.size
        );
        assert_eq!(atlas.len(), 1);

        // A second request must hit the cache: same entry, and nothing new packed. Without this,
        // an atlas that never evicts fills up at the frame rate.
        let again = atlas.glyph(&queue, &lib, id, glyph, SIZE).expect("cached");
        assert_eq!(again, entry, "the second lookup rasterised a second copy");
        assert_eq!(atlas.len(), 1, "the cache did not hold");
        // …and a different size is a different glyph, because the raster is size-specific.
        atlas.glyph(&queue, &lib, id, glyph, SIZE * 2.0).expect("a second size");
        assert_eq!(atlas.len(), 2, "two sizes must be two entries");

        let pixels = read_atlas(&device, &queue, &atlas);
        let n = atlas.size() as usize;
        let x0 = (entry.uv[0] * atlas.size() as f32).round() as usize;
        let y0 = (entry.uv[1] * atlas.size() as f32).round() as usize;
        let w = entry.size[0].round() as usize;
        let h = entry.size[1].round() as usize;

        // The interior of a filled box is fully covered. Sampled one pixel in from every edge, so
        // the assertion is about the fill rather than about how the rasteriser antialiases a seam.
        let mut interior_min = 255u8;
        for y in (y0 + 1)..(y0 + h - 1) {
            for x in (x0 + 1)..(x0 + w - 1) {
                interior_min = interior_min.min(pixels[y * n + x]);
            }
        }
        assert!(
            interior_min > 200,
            "the darkest pixel inside the box is {interior_min}; a filled glyph should be opaque"
        );

        // And everything outside the entry is untouched, which is what proves the entry's
        // rectangle is where the upload actually went.
        let mut outside_max = 0u8;
        for y in 0..n {
            for x in 0..n {
                let inside_first = x >= x0 && x < x0 + w && y >= y0 && y < y0 + h;
                // The second size was packed too; skip the whole first shelf rather than model it.
                let on_a_used_shelf = y < y0 + (h.max(2 * h));
                if !inside_first && !on_a_used_shelf {
                    outside_max = outside_max.max(pixels[y * n + x]);
                }
            }
        }
        assert_eq!(outside_max, 0, "something was written outside every packed glyph");
    }

    /// The atlas texture as one byte per pixel.
    fn read_atlas(device: &wgpu::Device, queue: &wgpu::Queue, atlas: &GlyphAtlas) -> Vec<u8> {
        let n = atlas.size();
        // `copy_texture_to_buffer` wants rows padded to 256 bytes; the atlas is R8 so a row is
        // `n` bytes and the padding has to be taken back out below.
        let row = n.div_ceil(256) * 256;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atlas-readback"),
            size: u64::from(row * n),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: atlas.texture(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(n),
                },
            },
            wgpu::Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        );
        queue.submit(std::iter::once(enc.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().expect("readback channel").expect("readback");
        let padded = slice
            .get_mapped_range()
            .expect("a just-mapped buffer's full range is always valid")
            .to_vec();
        let mut out = Vec::with_capacity((n * n) as usize);
        for y in 0..n as usize {
            let start = y * row as usize;
            out.extend_from_slice(&padded[start..start + n as usize]);
        }
        out
    }
}
