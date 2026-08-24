//! A minimal TrueType face, built in memory.
//!
//! # Why a font is built rather than shipped
//!
//! Text cannot be tested without a font, and this repository has no font asset. Adding one is a
//! *licensing* decision rather than a technical one — every usable typeface carries a licence
//! whose terms belong to whoever owns the project, not to whoever is writing the renderer — and
//! reaching for a system font instead makes every assertion depend on which distribution the test
//! ran on. Neither is a thing a test should decide.
//!
//! So the tests build their own face. [`synthetic_face`] emits a real TrueType file with a known
//! em size, known vertical metrics, a known advance and one glyph of known shape. Every number a
//! test asserts is then a *derivation* — the advance is [`ADVANCE`] because this file says so —
//! rather than a measurement of somebody else's typeface that a font update could move. It works
//! in CI and on wasm, where a system font is neither guaranteed nor reachable.
//!
//! # What it is not
//!
//! Not a fallback, not a default, and not something to render prose with: three glyphs, one of
//! which is a filled box and two of which are blank. It is `#[doc(hidden)]` for that reason —
//! reachable from a sibling crate's tests (the engine's golden render tests live in the facade,
//! not here), and not part of the documented surface.
//!
//! # The format, in the order the bytes come out
//!
//! A TrueType file is an offset table, then a directory of `(tag, checksum, offset, length)`
//! records sorted by tag, then the tables themselves. Seven tables are enough for a parser to
//! accept the file and rasterise from it: `cmap` (character → glyph), `glyf` (outlines), `head`
//! (em size, index format), `hhea` + `hmtx` (advances and vertical metrics), `loca` (where each
//! glyph's outline starts) and `maxp` (how many glyphs there are). Checksums are written as zero:
//! no parser this engine uses verifies them, and a test fixture that computes them would be
//! testing the checksum routine rather than the renderer.

/// Units per em. Chosen so that rasterising at this pixel size makes one font unit one pixel, and
/// every metric assertion is arithmetic instead of a scale factor.
pub const EM: u16 = 1000;
/// Distance from the baseline to the top of the tallest glyph, in font units.
pub const ASCENT: i16 = 800;
/// Distance from the baseline to the bottom of the lowest glyph, in font units. Negative, as the
/// font format has it.
pub const DESCENT: i16 = -200;
/// The font's recommended extra leading, in font units. Non-zero on purpose: a line-height
/// calculation that forgets the gap is wrong by exactly this and by nothing else.
pub const LINE_GAP: i16 = 100;
/// Every glyph's advance width, in font units.
pub const ADVANCE: u16 = 600;
/// The side of the filled box that is the one drawable glyph, in font units.
pub const GLYPH_BOX: i16 = 500;
/// The box's left edge, so the glyph has a left side bearing rather than sitting on the pen.
pub const GLYPH_LEFT: i16 = 50;

/// The one character with an outline.
pub const DRAWN_CHAR: char = 'A';
/// A character that advances the pen and draws nothing.
pub const BLANK_CHAR: char = ' ';

/// Builds the face. See the module docs.
#[must_use]
pub fn synthetic_face() -> Vec<u8> {
    // Glyph 0 is `.notdef` and glyph 2 is the blank: both are empty outlines, which the format
    // expresses as "this glyph's data is zero bytes long" in `loca` rather than as a glyph record.
    // Glyph 1 is the box.
    let box_glyph = simple_glyph_box(GLYPH_LEFT, 0, GLYPH_LEFT + GLYPH_BOX, GLYPH_BOX);
    let glyf = box_glyph.clone();
    // Short `loca` stores half-offsets, so every glyph must start on an even byte. `simple_glyph`
    // pads to that.
    debug_assert!(glyf.len().is_multiple_of(2), "glyf must be 2-byte aligned for short loca");
    let loca: Vec<u16> = vec![
        0,                       // glyph 0 starts at 0…
        0,                       // …and is empty, so glyph 1 starts at 0 too
        (glyf.len() / 2) as u16, // glyph 1 ends here, and glyph 2 starts…
        (glyf.len() / 2) as u16, // …and is empty
    ];

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"cmap", cmap()),
        (b"glyf", glyf),
        (b"head", head()),
        (b"hhea", hhea()),
        (b"hmtx", hmtx()),
        (b"loca", loca.iter().flat_map(|v| v.to_be_bytes()).collect()),
        (b"maxp", maxp()),
    ];
    // The directory must be sorted by tag; sorting here rather than trusting the literal above
    // means a table added in the wrong place is still a valid file.
    tables.sort_by(|a, b| a.0.cmp(b.0));

    let n = tables.len() as u16;
    let mut out = Vec::new();
    out.extend_from_slice(&0x0001_0000_u32.to_be_bytes()); // sfntVersion: TrueType outlines
    out.extend_from_slice(&n.to_be_bytes());
    let entry_selector = 15 - n.leading_zeros() as u16; // floor(log2(n))
    let search_range = (1u16 << entry_selector) * 16;
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&(n * 16 - search_range).to_be_bytes());

    // Every table starts on a 4-byte boundary; the first one starts after the directory.
    let mut offset = 12 + 16 * u32::from(n);
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        out.extend_from_slice(&0u32.to_be_bytes()); // checksum — see the module docs
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 12 + 16 * u32::from(n) + body.len() as u32;
    }
    out.extend_from_slice(&body);
    out
}

/// `head`: em size, the bounding box, and which `loca` format is in use.
fn head() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(&0x0001_0000_u32.to_be_bytes()); // version
    t.extend_from_slice(&0x0001_0000_u32.to_be_bytes()); // fontRevision
    t.extend_from_slice(&0u32.to_be_bytes()); // checkSumAdjustment
    t.extend_from_slice(&0x5F0F_3CF5_u32.to_be_bytes()); // magicNumber, fixed by the spec
    t.extend_from_slice(&0u16.to_be_bytes()); // flags
    t.extend_from_slice(&EM.to_be_bytes());
    t.extend_from_slice(&0u64.to_be_bytes()); // created
    t.extend_from_slice(&0u64.to_be_bytes()); // modified
    t.extend_from_slice(&0i16.to_be_bytes()); // xMin
    t.extend_from_slice(&0i16.to_be_bytes()); // yMin
    t.extend_from_slice(&(GLYPH_LEFT + GLYPH_BOX).to_be_bytes()); // xMax
    t.extend_from_slice(&GLYPH_BOX.to_be_bytes()); // yMax
    t.extend_from_slice(&0u16.to_be_bytes()); // macStyle
    t.extend_from_slice(&8u16.to_be_bytes()); // lowestRecPPEM
    t.extend_from_slice(&2i16.to_be_bytes()); // fontDirectionHint
    t.extend_from_slice(&0i16.to_be_bytes()); // indexToLocFormat: 0 = short
    t.extend_from_slice(&0i16.to_be_bytes()); // glyphDataFormat
    t
}

/// `hhea`: the vertical metrics, and how many entries `hmtx` holds.
fn hhea() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(&0x0001_0000_u32.to_be_bytes()); // version
    t.extend_from_slice(&ASCENT.to_be_bytes());
    t.extend_from_slice(&DESCENT.to_be_bytes());
    t.extend_from_slice(&LINE_GAP.to_be_bytes());
    t.extend_from_slice(&ADVANCE.to_be_bytes()); // advanceWidthMax
    t.extend_from_slice(&0i16.to_be_bytes()); // minLeftSideBearing
    t.extend_from_slice(&0i16.to_be_bytes()); // minRightSideBearing
    t.extend_from_slice(&(GLYPH_LEFT + GLYPH_BOX).to_be_bytes()); // xMaxExtent
    t.extend_from_slice(&1i16.to_be_bytes()); // caretSlopeRise
    t.extend_from_slice(&0i16.to_be_bytes()); // caretSlopeRun
    t.extend_from_slice(&0i16.to_be_bytes()); // caretOffset
    for _ in 0..4 {
        t.extend_from_slice(&0i16.to_be_bytes()); // reserved
    }
    t.extend_from_slice(&0i16.to_be_bytes()); // metricDataFormat
    t.extend_from_slice(&NUM_GLYPHS.to_be_bytes()); // numberOfHMetrics
    t
}

/// How many glyphs the face declares: `.notdef`, the box, the blank.
const NUM_GLYPHS: u16 = 3;

/// `maxp` version 1.0 — the version `glyf` outlines require; 0.5 is for CFF.
fn maxp() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    t.extend_from_slice(&NUM_GLYPHS.to_be_bytes());
    t.extend_from_slice(&4u16.to_be_bytes()); // maxPoints — the box has four
    t.extend_from_slice(&1u16.to_be_bytes()); // maxContours
    for _ in 0..11 {
        t.extend_from_slice(&0u16.to_be_bytes());
    }
    t
}

/// `hmtx`: one `(advance, left side bearing)` per glyph.
fn hmtx() -> Vec<u8> {
    let mut t = Vec::new();
    for _ in 0..NUM_GLYPHS {
        t.extend_from_slice(&ADVANCE.to_be_bytes());
        t.extend_from_slice(&GLYPH_LEFT.to_be_bytes());
    }
    t
}

/// `cmap`: a format-4 subtable mapping exactly two characters.
///
/// Format 4 is segment-based and its last segment must be `0xFFFF → 0xFFFF`; parsers reject a
/// table without it, and the one here maps the two real characters plus that sentinel.
fn cmap() -> Vec<u8> {
    let segments: [(u16, u16, u16); 3] = [
        (BLANK_CHAR as u16, BLANK_CHAR as u16, 2),
        (DRAWN_CHAR as u16, DRAWN_CHAR as u16, 1),
        (0xFFFF, 0xFFFF, 0),
    ];
    let seg_count = segments.len() as u16;

    let mut sub = Vec::new();
    sub.extend_from_slice(&4u16.to_be_bytes()); // format
    let length = 16 + seg_count * 8;
    sub.extend_from_slice(&length.to_be_bytes());
    sub.extend_from_slice(&0u16.to_be_bytes()); // language
    sub.extend_from_slice(&(seg_count * 2).to_be_bytes());
    let entry_selector = 15 - seg_count.leading_zeros() as u16;
    let search_range = (1u16 << entry_selector) * 2;
    sub.extend_from_slice(&search_range.to_be_bytes());
    sub.extend_from_slice(&entry_selector.to_be_bytes());
    sub.extend_from_slice(&(seg_count * 2 - search_range).to_be_bytes());
    for (_, end, _) in segments {
        sub.extend_from_slice(&end.to_be_bytes());
    }
    sub.extend_from_slice(&0u16.to_be_bytes()); // reservedPad
    for (start, _, _) in segments {
        sub.extend_from_slice(&start.to_be_bytes());
    }
    for (start, _, glyph) in segments {
        // idDelta is added to the character code modulo 65536. The sentinel segment maps 0xFFFF to
        // glyph 0 with a delta of 1, which is what every real font writes there.
        let delta = if glyph == 0 { 1i16 } else { (glyph as i32 - start as i32) as i16 };
        sub.extend_from_slice(&delta.to_be_bytes());
    }
    for _ in segments {
        sub.extend_from_slice(&0u16.to_be_bytes()); // idRangeOffset: no glyph array
    }
    debug_assert_eq!(sub.len(), usize::from(length), "the declared cmap length must be the real one");

    let mut t = Vec::new();
    t.extend_from_slice(&0u16.to_be_bytes()); // version
    t.extend_from_slice(&1u16.to_be_bytes()); // numTables
    t.extend_from_slice(&3u16.to_be_bytes()); // platformID: Windows
    t.extend_from_slice(&1u16.to_be_bytes()); // encodingID: Unicode BMP
    t.extend_from_slice(&12u32.to_be_bytes()); // offset to the subtable
    t.extend_from_slice(&sub);
    t
}

/// One filled rectangle as a simple glyph: four on-curve points, one contour.
///
/// Wound **clockwise in the font's y-up space** — up the left edge, across the top, down the
/// right — which is the direction TrueType gives an outer contour. A reversed contour is not a
/// parse error; it is a glyph that rasterises to nothing under a non-zero fill, which is a much
/// more confusing way to fail.
fn simple_glyph_box(x0: i16, y0: i16, x1: i16, y1: i16) -> Vec<u8> {
    let points: [(i16, i16); 4] = [(x0, y0), (x0, y1), (x1, y1), (x1, y0)];

    let mut t = Vec::new();
    t.extend_from_slice(&1i16.to_be_bytes()); // numberOfContours
    t.extend_from_slice(&x0.to_be_bytes());
    t.extend_from_slice(&y0.to_be_bytes());
    t.extend_from_slice(&x1.to_be_bytes());
    t.extend_from_slice(&y1.to_be_bytes());
    t.extend_from_slice(&3u16.to_be_bytes()); // endPtsOfContours: the last point's index
    t.extend_from_slice(&0u16.to_be_bytes()); // instructionLength
    // ON_CURVE_POINT for every point, and neither coordinate stored short — the long form costs
    // two bytes a coordinate and saves writing the delta-packing rules a font builder does not need.
    t.extend(std::iter::repeat_n(0x01u8, points.len()));
    // Coordinates are deltas from the previous point, the first being a delta from the origin.
    let mut prev = 0i16;
    for (x, _) in points {
        t.extend_from_slice(&(x - prev).to_be_bytes());
        prev = x;
    }
    let mut prev = 0i16;
    for (_, y) in points {
        t.extend_from_slice(&(y - prev).to_be_bytes());
        prev = y;
    }
    while t.len() % 2 != 0 {
        t.push(0);
    }
    t
}
