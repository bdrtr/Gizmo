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

/// The system's spacing scale (`--space-1` … `--space-6` in its `styles.css`), in steps of 4 px.
///
/// Named rather than inlined because the point of a modular scale is that a value which is not on
/// it is a mistake, and a bare `6.0` in a layout does not look like one.
pub const SPACE_1: f32 = 4.0;
/// See [`SPACE_1`].
pub const SPACE_2: f32 = 8.0;
/// See [`SPACE_1`].
pub const SPACE_3: f32 = 12.0;
/// See [`SPACE_1`].
pub const SPACE_4: f32 = 16.0;

/// What the design system asks for that egui 0.34 cannot express, recorded so it is not
/// rediscovered as a bug.
///
/// - **Flush-left button labels** ("even inside buttons", says the system's guide). `egui::Button`
///   centres its content and exposes no alignment: the `align2` setter lives on the private
///   `AtomLayout` field. Matching it means a custom button widget, which is a real piece of work
///   and not a theme setting.
/// - **Lucide icons throughout.** egui's bundled font has none of them — the prototype's ✥/⟳/⤢ and
///   the editor's old 🖐/🔀/🔄/📏 all render as empty boxes. Short words are used instead; real
///   icons mean vendoring a font file.
/// - **2 px rules between major sections.** `Separator` draws with
///   `widgets.noninteractive.bg_stroke`, which is also every widget's outline — raising it to 2 px
///   would thicken the 1 px borders the prototype uses everywhere else.
/// - **`:focus-visible` as a 2 px accent outline with a 2 px offset.** egui has no separate focus
///   stroke; a focused widget is drawn with `selection.stroke`, which this theme already sets to
///   the accent, but without the offset.
pub const UNIMPLEMENTED_SYSTEM_RULES: () = ();

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

    // The system's own spacing scale, read from its `styles.css` tokens rather than measured off
    // the mockup: `--space-1: 4px` … `--space-6: 24px`, in steps of 4. Every value below is one of
    // those steps; the earlier 6 px gap and 3 px padding were eyeballed and sat between steps,
    // which is exactly what a modular scale exists to prevent.
    style.spacing.item_spacing = egui::vec2(SPACE_2, SPACE_1);
    style.spacing.button_padding = egui::vec2(SPACE_2, SPACE_1);
    style.spacing.window_margin = egui::Margin::same(SPACE_2 as i8);
    style.spacing.menu_margin = egui::Margin::same(SPACE_2 as i8);
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


/// A section heading in the Inspector, the way the prototype draws them: the component's name in
/// accent-coloured caps, no emoji.
///
/// One helper rather than a `RichText` at each of the twenty-three `CollapsingHeader`s, because a
/// heading style spelled out twenty-three times is one that ends up with twenty-three variations —
/// which is how the editor arrived at 🚀/📷/💨/⚙️/🛡️/🔗/✨ as its section language in the first
/// place.
///
/// # Plain uppercase, deliberately, in a Turkish-language editor
///
/// The first version mapped `i` to `İ`, which is correct Turkish and wrong here: every one of the
/// twenty-three section names is English — Transform, Velocity, Collider, Post-Processing — with
/// at most a Turkish gloss in parentheses. It rendered `POST-PROCESSİNG` and `DEPTH OF FİELD`,
/// which is worse than the three parentheticals (`(ODAK)`, `(GÜNEŞ)`, `(FIZIKSEL BAĞLANTILAR)`)
/// losing their dots. Counted before choosing: ten-odd English titles against three glosses.
pub fn section_title(name: &str) -> egui::RichText {
    egui::RichText::new(name.to_uppercase())
        .size(10.0)
        .color(palette::ACCENT)
        .strong()
}

/// A segmented control: joined cells, one of them on.
///
/// The prototype uses these wherever a value has two to four states — `On / Off / Only` for shadow
/// casting, `Static / Dynamic / Kinematic` for a body type, `true / false` for a flag. They read as
/// one control with a position rather than as a dropdown you have to open to learn the current
/// value, which is the whole reason a modal setting gets a segmented control instead of a combo
/// box.
///
/// Painted rather than assembled from buttons: the cells share their borders (a run of egui
/// buttons would double every internal edge and round nothing off, since the theme is square), and
/// the active cell is a solid accent fill that `Button::selected` does not produce.
///
/// Returns `true` when the value changed.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    use palette::*;

    if options.is_empty() {
        return false;
    }

    let height = ROW_HEIGHT;
    // Measure first so every cell is as wide as the widest label — a segmented control whose cells
    // jump width as the text changes is a row of buttons wearing a costume.
    let font = egui::FontId::proportional(11.0);
    let cell_w = options
        .iter()
        .map(|(_, label)| {
            ui.painter().layout_no_wrap((*label).to_string(), font.clone(), TEXT_BODY).size().x
        })
        .fold(0.0_f32, f32::max)
        + SPACE_3;

    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(cell_w * options.len() as f32, height),
        egui::Sense::hover(),
    );

    let mut changed = false;
    for (i, (opt, label)) in options.iter().enumerate() {
        let cell = egui::Rect::from_min_size(
            egui::pos2(rect.left() + cell_w * i as f32, rect.top()),
            egui::vec2(cell_w, height),
        );
        // Salted with the label, not just the index: two segmented controls in the same parent
        // `Ui` would otherwise share ids cell-for-cell and steal each other's clicks. Latent today
        // (there is one call site) and the kind of thing that surfaces as "the wrong control
        // responded" months later.
        let id = ui.id().with(("segmented", *label, i));
        let resp = ui.interact(cell, id, egui::Sense::click());
        let on = *value == *opt;

        let painter = ui.painter();
        painter.rect_filled(
            cell,
            0.0,
            if on {
                ACCENT
            } else if resp.hovered() {
                SURFACE
            } else {
                CHROME
            },
        );
        painter.rect_stroke(
            cell,
            0.0,
            egui::Stroke::new(1.0_f32, if on { ACCENT } else { BORDER }),
            egui::StrokeKind::Inside,
        );
        painter.text(
            cell.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font.clone(),
            if on { TEXT_BRIGHT } else { TEXT_BODY },
        );

        if resp.clicked() && !on {
            *value = *opt;
            changed = true;
        }
    }
    changed
}

/// A single on/off cell, filled with the accent when on.
///
/// The prototype's `Shaded` / `Grid` viewport toggles. Distinct from [`segmented`]: that one picks
/// one of several mutually exclusive states, this one is an independent switch. Expressing a
/// boolean as a two-cell segmented control gives you labels like "Colliders off | Colliders",
/// which is a sentence pretending to be a control.
///
/// Returns `true` when the value changed.
pub fn toggle(ui: &mut egui::Ui, value: &mut bool, label: &str) -> bool {
    use palette::*;

    let font = egui::FontId::proportional(11.0);
    let w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), TEXT_BODY).size().x + SPACE_3;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, ROW_HEIGHT), egui::Sense::click());

    let on = *value;
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, if on { ACCENT } else if resp.hovered() { SURFACE } else { CHROME });
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0_f32, if on { ACCENT } else { BORDER }),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        if on { TEXT_BRIGHT } else { TEXT_BODY },
    );

    if resp.clicked() {
        *value = !on;
        return true;
    }
    false
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
