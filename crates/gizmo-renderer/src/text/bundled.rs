//! The one font the engine carries: **Noto Sans Regular** (version 2.015), behind the default-off
//! `bundled-font` feature.
//!
//! The engine shipped no font, and on the desktop it did not need to — `load_system_font` finds
//! the machine's. The browser has no file system to look in, so every web build fell back to the
//! synthetic face: ASCII shapes only, every Turkish letter (and every other non-ASCII one) blank.
//! This is the face [`Renderer::default_font`](crate::renderer::Renderer::default_font) uses when
//! the machine lends none.
//!
//! Licensed under the SIL Open Font License 1.1 — the text is `assets/fonts/OFL.txt` in this
//! crate, and a binary that carries the font carries that notice with it. Copyright 2022 The Noto
//! Project Authors (<https://github.com/notofonts/latin-greek-cyrillic>).

/// The face's bytes, for [`FontLibrary::load`](super::FontLibrary::load) or
/// [`Renderer::load_font`](crate::renderer::Renderer::load_font).
#[must_use]
pub fn noto_sans() -> Vec<u8> {
    NOTO_SANS.to_vec()
}

/// The font file itself, embedded.
pub const NOTO_SANS: &[u8] = include_bytes!("../../assets/fonts/NotoSans-Regular.ttf");

/// The licence the face is distributed under, to show or ship next to it.
pub const NOTO_SANS_LICENSE: &str = include_str!("../../assets/fonts/OFL.txt");

#[cfg(test)]
mod tests {
    use super::super::FontLibrary;

    /// Every letter of the Turkish alphabet has a real glyph — the reason the face is here.
    #[test]
    fn the_bundled_face_draws_turkish() {
        let mut lib = FontLibrary::new();
        let font = lib.load(super::noto_sans()).expect("the bundled face parses");
        let letters = "ÇĞİÖŞÜçğıöşü";
        let layout = lib.layout(font, 32.0, letters).expect("it lays out");
        assert_eq!(layout.glyphs.len(), letters.chars().count(), "a letter laid out as nothing");
        assert!(layout.glyphs.iter().all(|g| g.glyph != 0), "a letter fell to .notdef: {:?}", layout.glyphs);
        assert!(super::NOTO_SANS_LICENSE.contains("SIL OPEN FONT LICENSE Version 1.1"));
    }
}
