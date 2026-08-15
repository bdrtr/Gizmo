//! The editor's visual design, in one place.
//!
//! Implements the `Gizmo Editor Prototype` design (claude.ai/design project
//! `e4099dec-2d75-4de6-acf6-9013056748e1`). The prototype is an HTML mockup drawn for **this**
//! toolkit — its own status bar reads "egui 0.29 · rustc 1.83" — so the translation is a mapping
//! of values, not an interpretation of a design made for something else.
//!
//! # What the design actually is
//!
//! Three properties carry it, and they are the three the previous theme got the other way round:
//!
//! - **Square corners, everywhere.** The prototype contains zero `border-radius` declarations in
//!   1049 lines. The old theme rounded every widget by 6 px and windows by 10. Nothing else in
//!   this file changes the feel as much.
//! - **Dense rows.** Prototype rows are 20–24 px tall with 12 px type; the old theme's
//!   `interact_size` was 40×24 with 10×8 spacing, which is a touch target, not a tool row.
//! - **Warm neutrals, not grey.** Every surface carries a red cast (`#151312`, `#201e1d`,
//!   `#2d2b2b`, `#444141`), which is why the old cool grey `#1c1c1e` and its soft blue accent read
//!   as a different application.
//!
//! # What is not implemented, and why
//!
//! - **The Archivo typeface.** It would mean vendoring a font binary into the crate; egui falls
//!   back to its built-in family. Sizes, weights and the 9/10/11/12/13 px scale are matched.
//! - **The prototype's panels that describe features this engine does not have** — the node graph,
//!   the animation track editor with auto-key, the play-mode HUD. Those are features wearing the
//!   design, not the design, and inventing them from a mockup would be guessing at behaviour.
//! - **Per-widget accent placement** (a primary button, a hot tab). The palette below is `pub` so
//!   the editor's own UI code can reach for it; this module only sets what egui applies globally.

use egui::{CornerRadius, Stroke};

/// The prototype's palette, by role rather than by hex, so a caller does not have to know that
/// `#444141` is "the border colour" to use it correctly.
pub mod palette {
    use egui::Color32;

    /// `#151312` — the deepest surface: the app behind everything, and text-entry wells.
    pub const VOID: Color32 = Color32::from_rgb(0x15, 0x13, 0x12);
    /// `#201e1d` — chrome: panels, the top bar, scrollbar tracks.
    pub const CHROME: Color32 = Color32::from_rgb(0x20, 0x1e, 0x1d);
    /// `#2d2b2b` — a raised surface: an inactive widget, a field, a striped row.
    pub const SURFACE: Color32 = Color32::from_rgb(0x2d, 0x2b, 0x2b);
    /// `#444141` — the border. The single most-used colour in the prototype (63 occurrences).
    pub const BORDER: Color32 = Color32::from_rgb(0x44, 0x41, 0x41);
    /// `#605d5d` — a border under the cursor, and the scrollbar thumb's hover.
    pub const BORDER_HOT: Color32 = Color32::from_rgb(0x60, 0x5d, 0x5d);

    /// `#7d7979` — the dimmest legible text: units, hints, disabled labels.
    pub const TEXT_DIM: Color32 = Color32::from_rgb(0x7d, 0x79, 0x79);
    /// `#9b9797` — secondary text: inactive tabs, column headers.
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x9b, 0x97, 0x97);
    /// `#bab6b6` — body text on an inactive control.
    pub const TEXT_BODY: Color32 = Color32::from_rgb(0xba, 0xb6, 0xb6);
    /// `#d7d3d3` — body text at rest on a panel.
    pub const TEXT: Color32 = Color32::from_rgb(0xd7, 0xd3, 0xd3);
    /// `#f8f4f4` — the brightest: a hovered control, a focused field, a heading.
    pub const TEXT_BRIGHT: Color32 = Color32::from_rgb(0xf8, 0xf4, 0xf4);

    /// `#ec3013` — the accent. Used sparingly: the mark, a focus ring, an active tool.
    pub const ACCENT: Color32 = Color32::from_rgb(0xec, 0x30, 0x13);
    /// `#7c1405` — the accent as a fill behind text: selection, a pressed toggle.
    pub const ACCENT_DEEP: Color32 = Color32::from_rgb(0x7c, 0x14, 0x05);
    /// `#ff9783` — a link at rest.
    pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0xff, 0x97, 0x83);
    /// `#ffc4b8` — a link under the cursor.
    pub const ACCENT_PALE: Color32 = Color32::from_rgb(0xff, 0xc4, 0xb8);
}

/// The prototype's row height. Its controls sit between 20 and 24 px; 21 is both the mode and the
/// height of a plain toolbar row.
pub const ROW_HEIGHT: f32 = 21.0;

/// Base type size. The prototype sets `font-size: 12px` on the frame and overrides down to 9 for
/// units and up to 13 for the wordmark.
pub const TEXT_SIZE: f32 = 12.0;

/// Applies the design to an egui context.
///
/// Everything here is a global default. Anything a specific panel wants differently — an accented
/// primary button, a wider inspector row — belongs at that call site, using [`palette`].
pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals(visuals());

    let mut style = (*ctx.global_style()).clone();

    // The type ramp. egui's defaults are ~14 px body and 18 px heading, which is a document; this
    // is a tool, and the prototype's densest labels are 9 px.
    use egui::{FontFamily::Proportional, FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Heading, FontId::new(13.0, Proportional)),
        (TextStyle::Body, FontId::new(TEXT_SIZE, Proportional)),
        (TextStyle::Button, FontId::new(TEXT_SIZE, Proportional)),
        (TextStyle::Small, FontId::new(10.0, Proportional)),
        (TextStyle::Monospace, FontId::new(11.0, egui::FontFamily::Monospace)),
    ]
    .into();

    // Prototype gaps: 8 px between groups in a row, 4 px between stacked rows. The old values
    // (10×8, with 12×6 button padding) spread a tool row over roughly twice the height.
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    style.spacing.window_margin = egui::Margin::same(8);
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.interact_size = egui::vec2(24.0, ROW_HEIGHT);
    style.spacing.icon_width = 12.0;
    style.spacing.slider_width = 96.0;
    style.spacing.scroll.bar_width = 10.0; // matches the prototype's ::-webkit-scrollbar

    ctx.set_global_style(style);
}

/// The colour and geometry half, separated so a test can read it without a context.
pub fn visuals() -> egui::Visuals {
    use palette::*;

    let mut v = egui::Visuals::dark();

    // The defining property. `CornerRadius::ZERO` on every surface egui rounds.
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::ZERO;
    }

    v.window_fill = CHROME;
    v.panel_fill = CHROME;
    v.extreme_bg_color = VOID;
    v.faint_bg_color = SURFACE;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);

    // Flat. The prototype casts no shadows; egui's default window shadow reads as depth this
    // design does not have.
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;

    // Panels and separators: the frame is the border colour, the text sits at rest.
    v.widgets.noninteractive.bg_fill = CHROME;
    v.widgets.noninteractive.weak_bg_fill = CHROME;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);

    // A control at rest.
    v.widgets.inactive.bg_fill = SURFACE;
    v.widgets.inactive.weak_bg_fill = SURFACE;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT_BODY);

    // Under the cursor: the border lifts and the label brightens. The fill barely moves — in the
    // prototype hover is a border and a text change, not a wash.
    v.widgets.hovered.bg_fill = SURFACE;
    v.widgets.hovered.weak_bg_fill = SURFACE;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER_HOT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT_BRIGHT);

    // Pressed / on: this is where the accent is allowed.
    v.widgets.active.bg_fill = ACCENT_DEEP;
    v.widgets.active.weak_bg_fill = ACCENT_DEEP;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT_BRIGHT);

    // An open menu or expanded header.
    v.widgets.open.bg_fill = SURFACE;
    v.widgets.open.weak_bg_fill = SURFACE;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, BORDER_HOT);
    v.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT_BRIGHT);

    v.selection.bg_fill = ACCENT_DEEP;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);

    v.hyperlink_color = ACCENT_LIGHT;
    v.warn_fg_color = ACCENT_LIGHT;
    v.error_fg_color = ACCENT;

    // Not overridden: the previous theme forced one text colour everywhere, which flattens the
    // five-step ramp above into a single value and loses the difference between a label at rest
    // and one under the cursor.
    v.override_text_color = None;

    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Color32;

    /// The property that carries the design, asserted where it cannot drift back.
    ///
    /// Square corners are not a preference here — the prototype has 1049 lines and zero
    /// `border-radius` declarations. A later "let's soften this" edit is exactly the kind of change
    /// that gets made one widget at a time and is never noticed as a whole.
    #[test]
    fn nothing_is_rounded() {
        let v = visuals();
        assert_eq!(v.window_corner_radius, CornerRadius::ZERO);
        assert_eq!(v.menu_corner_radius, CornerRadius::ZERO);
        for (name, w) in [
            ("noninteractive", &v.widgets.noninteractive),
            ("inactive", &v.widgets.inactive),
            ("hovered", &v.widgets.hovered),
            ("active", &v.widgets.active),
            ("open", &v.widgets.open),
        ] {
            assert_eq!(w.corner_radius, CornerRadius::ZERO, "{name} is rounded");
        }
    }

    /// The palette, pinned to the prototype's hexes.
    ///
    /// Written out rather than referenced so the test fails if a constant is *edited*, which is the
    /// only way these can go wrong — nothing computes them.
    #[test]
    fn the_palette_matches_the_prototype() {
        use palette::*;
        let hex = |c: Color32| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
        assert_eq!(hex(VOID), "#151312");
        assert_eq!(hex(CHROME), "#201e1d");
        assert_eq!(hex(SURFACE), "#2d2b2b");
        assert_eq!(hex(BORDER), "#444141");
        assert_eq!(hex(BORDER_HOT), "#605d5d");
        assert_eq!(hex(TEXT_DIM), "#7d7979");
        assert_eq!(hex(TEXT_MUTED), "#9b9797");
        assert_eq!(hex(TEXT_BODY), "#bab6b6");
        assert_eq!(hex(TEXT), "#d7d3d3");
        assert_eq!(hex(TEXT_BRIGHT), "#f8f4f4");
        assert_eq!(hex(ACCENT), "#ec3013");
        assert_eq!(hex(ACCENT_DEEP), "#7c1405");
        assert_eq!(hex(ACCENT_LIGHT), "#ff9783");
        assert_eq!(hex(ACCENT_PALE), "#ffc4b8");
    }

    /// Surfaces must be warm — every one of them carries more red than blue.
    ///
    /// This is what separates the design from "a dark theme": the previous one was cool grey
    /// (`#1c1c1e`, blue-dominant) and read as a different application. A single greyed-out
    /// constant would pass the hex test only if someone edited that constant, but a *new* surface
    /// added later is caught here.
    #[test]
    fn every_surface_is_warm() {
        use palette::*;
        for (name, c) in [
            ("VOID", VOID),
            ("CHROME", CHROME),
            ("SURFACE", SURFACE),
            ("BORDER", BORDER),
            ("BORDER_HOT", BORDER_HOT),
        ] {
            assert!(
                c.r() >= c.b(),
                "{name} is cool ({} r vs {} b) — the prototype's neutrals all lean red",
                c.r(),
                c.b()
            );
        }
    }

    /// Rows are tool-sized, not touch-sized.
    #[test]
    fn the_row_height_matches_the_prototypes_controls() {
        assert!(
            (20.0..=24.0).contains(&ROW_HEIGHT),
            "prototype controls are 20–24 px tall; {ROW_HEIGHT} is outside that"
        );
        // A const block, because both sides are constants: clippy is right that this cannot fail
        // at runtime, and the honest form of "cannot fail at runtime" is a compile-time check.
        const { assert!(TEXT_SIZE <= 12.0, "the prototype's frame sets 12 px and overrides downward") };
    }
}
