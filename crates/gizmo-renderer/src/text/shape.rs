//! One line of text to placed glyphs: bidi first, then runs of one face and one script, then
//! shaping.
//!
//! The order is the point. Bidi says which stretches run right to left and in what order they
//! appear on the line. Each stretch is cut wherever the face or the script changes, because a
//! shaper works on one of each at a time. `harfrust` then turns each piece into glyphs, which is
//! where Arabic letters join, Devanagari vowels move and the face's own `GPOS` kerning applies.
//!
//! Latin text is shaped without ligatures. A face's `fi` hides the dot of the i, which in Turkish is
//! the difference between two letters, and asking for the language does not help when the face has
//! no Turkish entry, as Noto Sans has none. Arabic and Indic scripts keep theirs: they need them.
//!
//! Neither crate appears on a `pub` signature: the result is [`PositionedGlyph`](super::PositionedGlyph),
//! as it always was.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};

use ab_glyph::{Font as _, ScaleFont as _};
use harfrust::{script, Direction, Feature, FontRef, Script, ShapeOptions, ShapePlan, ShaperData, Tag, UnicodeBuffer};
use unicode_bidi::{BidiInfo, Level};

use super::{Faces, FontId};

/// What a face needs to be shaped, kept with the library because building it reads the font's
/// tables and a plan compiles its lookups: both are far too slow to redo per string.
#[derive(Default)]
pub(super) struct Shaping {
    /// One per loaded face, index for index with the library's fonts.
    data: Vec<ShaperData>,
    plans: Mutex<HashMap<PlanKey, Arc<ShapePlan>>>,
}

/// A face, a script and whether it runs right to left: all a plan depends on.
type PlanKey = (u32, Option<Script>, bool);

impl Shaping {
    /// Registers a face, or says it cannot be shaped.
    pub(super) fn add_face(&mut self, bytes: &[u8]) -> bool {
        let Ok(font) = FontRef::from_index(bytes, 0) else { return false };
        self.data.push(ShaperData::new(&font));
        true
    }
}

/// A shaped glyph: `x` from the start of the line, `y` from the baseline, growing down.
pub(super) struct ShapedGlyph {
    pub(super) font: FontId,
    pub(super) glyph: u16,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) px: f32,
    /// Byte offset into the line of the text this glyph draws.
    pub(super) cluster: u32,
}

/// One line, shaped: the glyphs from left to right, and how far the pen went.
#[derive(Default)]
pub(super) struct ShapedLine {
    pub(super) glyphs: Vec<ShapedGlyph>,
    pub(super) width: f32,
    /// Whether the line's base direction is right to left.
    pub(super) rtl: bool,
}

/// A paragraph's base direction, for lines cut from it: a right-to-left paragraph stays one on a
/// wrapped line that happens to start with a Latin word.
pub(super) fn paragraph_level(paragraph: &str) -> Option<Level> {
    BidiInfo::new(paragraph, None).paragraphs.first().map(|p| p.level)
}

/// Shapes `line`, which holds no `\n`. With `level` as `None` the line finds its own direction.
pub(super) fn shape_line(faces: &Faces<'_>, px: f32, line: &str, level: Option<Level>) -> ShapedLine {
    let mut out = ShapedLine::default();
    let bidi = BidiInfo::new(line, level);
    out.rtl = bidi.paragraphs.first().map(|p| p.level).or(level).is_some_and(|l| l.is_rtl());
    for paragraph in &bidi.paragraphs {
        let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
        for run in runs {
            let rtl = levels[run.start].is_rtl();
            let mut items = itemize(faces, line, run);
            // Runs arrive in visual order, but what is inside one is still in logical order.
            if rtl {
                items.reverse();
            }
            for item in &items {
                shape_item(faces, px, line, item, rtl, &mut out);
            }
        }
    }
    out
}

/// A stretch of one face and one script.
struct Item {
    range: Range<usize>,
    font: FontId,
    /// `None` while only characters of no script of their own have been seen.
    script: Option<Script>,
}

/// Cuts a bidi run wherever the face or the script changes. A character of no script of its own (a
/// space, a digit, a comma, a combining mark, a joiner) stays with what came before it while that
/// face covers it, so a mark is never shaped apart from its letter.
fn itemize(faces: &Faces<'_>, text: &str, run: Range<usize>) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut probe = UnicodeBuffer::new();
    for (offset, c) in text[run.clone()].char_indices() {
        let start = run.start + offset;
        let end = start + c.len_utf8();
        let script = script_of(&mut probe, c);
        if let Some(last) = items.last_mut() {
            if script.is_none() && faces.covers(last.font, c) {
                last.range.end = end;
                continue;
            }
            if faces.pick(c).0 == last.font && (last.script.is_none() || last.script == script) {
                last.script = script;
                last.range.end = end;
                continue;
            }
        }
        items.push(Item { range: start..end, font: faces.pick(c).0, script });
    }
    items
}

/// The runs [`itemize`] cuts all of `text` into.
#[cfg(test)]
pub(super) fn itemize_for_test(faces: &Faces<'_>, text: &str) -> Vec<Range<usize>> {
    itemize(faces, text, 0..text.len()).into_iter().map(|item| item.range).collect()
}

/// The script of `c`, or `None` for the Common and Inherited characters that take their
/// neighbours'. `harfrust` keeps its table private, so it is asked through a one-character buffer.
fn script_of(probe: &mut UnicodeBuffer, c: char) -> Option<Script> {
    probe.add(c, 0);
    probe.guess_segment_properties();
    let found = probe.script();
    probe.clear();
    (found != script::UNKNOWN).then_some(found)
}

/// Shapes one item and appends its glyphs, in visual order, at the line's pen.
fn shape_item(faces: &Faces<'_>, px: f32, text: &str, item: &Item, rtl: bool, out: &mut ShapedLine) {
    let library = faces.library;
    let (Some(face), Some(data)) = (library.face(item.font), library.shaping.data.get(item.font.0 as usize)) else {
        return;
    };
    let Ok(font) = FontRef::from_index(face.as_slice(), 0) else { return };
    let shaper = data.shaper(&font).build();
    let direction = if rtl { Direction::RightToLeft } else { Direction::LeftToRight };
    let no_ligatures = [Feature::new(Tag::new(b"liga"), 0, ..), Feature::new(Tag::new(b"clig"), 0, ..)];
    let features: &[Feature] = if item.script == Some(script::LATIN) { &no_ligatures } else { &[] };
    let plan = {
        let mut plans = library.shaping.plans.lock().unwrap_or_else(PoisonError::into_inner);
        plans
            .entry((item.font.0, item.script, rtl))
            .or_insert_with(|| Arc::new(ShapePlan::new(&shaper, direction, item.script, None, features)))
            .clone()
    };

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(&text[item.range.clone()]);
    buffer.set_direction(direction);
    if let Some(script) = item.script {
        buffer.set_script(script);
    }
    let shaped = shaper.shape(buffer, ShapeOptions::new().plan(Some(&plan)).features(features));

    // ab_glyph's scale, not px / units-per-em: `px` here means the face's ascent-to-descent height,
    // and the rasteriser draws every glyph at that scale.
    let px = faces.px_for(item.font, px);
    let scale = face.as_scaled(px).h_scale_factor();
    for (info, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        out.glyphs.push(ShapedGlyph {
            font: item.font,
            glyph: u16::try_from(info.glyph_id).unwrap_or(0),
            x: out.width + position.x_offset as f32 * scale,
            y: -(position.y_offset as f32) * scale,
            px,
            cluster: item.range.start as u32 + info.cluster,
        });
        out.width += position.x_advance as f32 * scale;
    }
}
