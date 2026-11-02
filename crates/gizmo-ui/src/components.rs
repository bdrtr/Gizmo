use gizmo_math::{Vec2, Vec4};

/// A length used by [`Style`].
///
/// Plain old data: a tag plus one `f32`. This is deliberately *our* type and not
/// `taffy`'s — see the [`Style`] docs for why.
///
/// `Percent` uses the **CSS scale**, `0.0..=100.0`, not the `0.0..=1.0` fraction
/// `taffy` uses internally. `Val::Percent(50.0)` is half of the parent.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum Val {
    /// The layout algorithm decides. What "auto" means depends on the property:
    /// for a size it is content-based, for a margin/inset it is the CSS `auto`
    /// keyword, and for padding/border/gap — which have no `auto` in CSS — it is
    /// treated as zero.
    #[default]
    Auto,
    /// An absolute length in logical pixels.
    Px(f32),
    /// A percentage of the parent's corresponding dimension, on the CSS scale
    /// (`100.0` = 100%, i.e. the full parent dimension).
    Percent(f32),
}

impl Val {
    /// Zero pixels.
    pub const ZERO: Val = Val::Px(0.0);
}

/// Four per-side [`Val`]s, used for `inset`, `margin`, `padding` and `border`.
///
/// [`Default`] is [`UiRect::ZERO`], matching the CSS initial value of margin,
/// padding and border. `inset` is the exception and defaults to
/// [`UiRect::AUTO`]; [`Style::DEFAULT`] sets it explicitly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiRect {
    /// Left edge.
    pub left: Val,
    /// Right edge.
    pub right: Val,
    /// Top edge.
    pub top: Val,
    /// Bottom edge.
    pub bottom: Val,
}

impl UiRect {
    /// All four sides zero pixels.
    pub const ZERO: UiRect = UiRect::all(Val::ZERO);
    /// All four sides [`Val::Auto`].
    pub const AUTO: UiRect = UiRect::all(Val::Auto);

    /// A rect with the same value on all four sides.
    pub const fn all(value: Val) -> Self {
        Self { left: value, right: value, top: value, bottom: value }
    }

    /// A rect with one value on the left/right sides and another on top/bottom.
    pub const fn axes(horizontal: Val, vertical: Val) -> Self {
        Self { left: horizontal, right: horizontal, top: vertical, bottom: vertical }
    }

    /// A rect with an explicit value per side.
    pub const fn new(left: Val, right: Val, top: Val, bottom: Val) -> Self {
        Self { left, right, top, bottom }
    }
}

impl Default for UiRect {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Which layout algorithm lays out an element's children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Display {
    /// Flexbox. The default, and the layout mode this crate is built around.
    #[default]
    Flex,
    /// Block layout: children stack vertically.
    Block,
    /// The element and its whole subtree generate no boxes and are skipped.
    None,
}

/// How an element's `inset` is interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PositionType {
    /// Laid out in flow; `inset` nudges the element afterwards without moving
    /// its siblings. The CSS `position: relative` behaviour.
    #[default]
    Relative,
    /// Taken out of flow and positioned against the nearest positioned
    /// ancestor. The CSS `position: absolute` behaviour.
    Absolute,
}

/// Direction of the flexbox main axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum FlexDirection {
    /// Left to right.
    #[default]
    Row,
    /// Top to bottom.
    Column,
    /// Right to left.
    RowReverse,
    /// Bottom to top.
    ColumnReverse,
}

/// Whether flex items may wrap onto more than one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum FlexWrap {
    /// Everything on a single line.
    #[default]
    NoWrap,
    /// Wrap along the [`FlexDirection`].
    Wrap,
    /// Wrap in the opposite direction.
    WrapReverse,
}

/// Alignment of items along the cross axis.
///
/// Also used for `align_self` via the [`AlignSelf`] alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AlignItems {
    /// Packed toward the start of the axis.
    Start,
    /// Packed toward the end of the axis.
    End,
    /// Packed toward the flex-relative start (start, or end when the direction
    /// is reversed).
    FlexStart,
    /// Packed toward the flex-relative end.
    FlexEnd,
    /// Centred on the axis.
    Center,
    /// Aligned so the items' baselines line up.
    Baseline,
    /// Stretched to fill the container.
    Stretch,
}

/// Alignment of an element against its parent's cross axis, overriding the
/// parent's [`AlignItems`].
pub type AlignSelf = AlignItems;

/// Distribution of free space between and around items.
///
/// Also used for `justify_content` via the [`JustifyContent`] alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AlignContent {
    /// Packed toward the start of the axis.
    Start,
    /// Packed toward the end of the axis.
    End,
    /// Packed toward the flex-relative start.
    FlexStart,
    /// Packed toward the flex-relative end.
    FlexEnd,
    /// Centred on the axis.
    Center,
    /// Stretched to fill the container.
    Stretch,
    /// First and last items flush with the edges, gaps distributed evenly.
    SpaceBetween,
    /// Gaps between, before and after the items are all equal.
    SpaceEvenly,
    /// The outer gaps are half the size of the gaps between items.
    SpaceAround,
}

/// Distribution of free space along the main axis.
pub type JustifyContent = AlignContent;

/// Layout style of a UI element.
///
/// This is **plain old data** — enums, `f32`s and `Option<f32>`, nothing else.
/// It is the ECS component, so it has to be `Send + Sync`, and being POD it is
/// so *derived*, with no `unsafe impl` anywhere in this crate for it.
///
/// The layout engine underneath is [`taffy`], but no `taffy` type appears in
/// this crate's public API. The conversion happens in one place —
/// `UiContext::to_taffy_style` in `layout.rs` — and the layout system calls it
/// once per styled entity per frame.
///
/// # Not modelled
///
/// This type covers the flexbox/block surface the crate's layout system
/// actually drives. Everything `taffy` supports that is **not** listed as a
/// field below is deliberately absent, and setting it is impossible rather than
/// merely awkward:
///
/// - **CSS Grid.** No `Display::Grid`, no `grid_template_*`, `grid_auto_*`,
///   `grid_row`/`grid_column` placement, no `fr` units, and no
///   `justify_items`/`justify_self` (both are `#[cfg(feature = "grid")]` in
///   `taffy` and ignored by flexbox — `align_items`/`align_self` are the
///   flexbox cross-axis pair, and they *are* modelled). `taffy`'s grid
///   algorithm is still compiled in (see `Cargo.toml`) but is unreachable from
///   here; exposing it needs a grid-track type of its own.
/// - **`overflow` and `scrollbar_width`.** This crate does no clipping and no
///   scrolling, so a scroll container would compute geometry nothing honours.
/// - **`box_sizing`.** `taffy`'s default, border-box, is always used.
/// - **`direction` (LTR/RTL)** and **`text_align`.** Text is the engine's `Text` component
///   (with its own alignment), placed in a node's box by the facade; layout never measures it.
/// - **`float` / `clear`**, **`item_is_table`**, **`item_is_replaced`**.
/// - **Intrinsic sizing keywords** (`min-content`, `max-content`,
///   `fit-content`) and **`calc()` lengths**: [`Val`] is `Auto`/`Px`/`Percent`
///   only. Excluding `calc()` is load-bearing, not an oversight — see the
///   `SAFETY` note on `UiContext` in `layout.rs`.
///
/// # Example
///
/// ```
/// use gizmo_ui::components::{Style, Val, UiRect, FlexDirection};
///
/// let style = Style {
///     width: Val::Percent(100.0),
///     height: Val::Px(64.0),
///     flex_direction: FlexDirection::Column,
///     padding: UiRect::all(Val::Px(8.0)),
///     ..Default::default()
/// };
/// assert_eq!(style.flex_shrink, 1.0); // CSS default, not 0.0
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// Layout algorithm used for this element's children.
    pub display: Display,
    /// How [`Style::inset`] is interpreted.
    pub position_type: PositionType,
    /// Per-side offset, applied according to [`Style::position_type`].
    pub inset: UiRect,

    /// Preferred width.
    pub width: Val,
    /// Preferred height.
    pub height: Val,
    /// Lower bound on the width.
    pub min_width: Val,
    /// Lower bound on the height.
    pub min_height: Val,
    /// Upper bound on the width.
    pub max_width: Val,
    /// Upper bound on the height.
    pub max_height: Val,
    /// Preferred width-divided-by-height ratio, if any.
    pub aspect_ratio: Option<f32>,

    /// Space outside the border box. `Val::Auto` here is a real CSS auto margin.
    pub margin: UiRect,
    /// Space between the border and the content. `Val::Auto` is treated as zero.
    pub padding: UiRect,
    /// Border thickness reserved by layout. `Val::Auto` is treated as zero.
    pub border: UiRect,

    /// Cross-axis alignment applied to the children of this element.
    pub align_items: Option<AlignItems>,
    /// Cross-axis alignment of this element, overriding the parent's
    /// [`Style::align_items`].
    pub align_self: Option<AlignSelf>,
    /// Cross-axis distribution of this element's lines of content.
    pub align_content: Option<AlignContent>,
    /// Main-axis distribution of this element's children.
    pub justify_content: Option<JustifyContent>,

    /// Direction of the main axis.
    pub flex_direction: FlexDirection,
    /// Whether children may wrap onto multiple lines.
    pub flex_wrap: FlexWrap,
    /// Share of leftover main-axis space this element grows into. CSS default `0.0`.
    pub flex_grow: f32,
    /// Rate at which this element shrinks when space is short. CSS default `1.0`.
    pub flex_shrink: f32,
    /// Initial main-axis size, before grow/shrink.
    pub flex_basis: Val,

    /// Gap between rows. `Val::Auto` is treated as zero.
    pub row_gap: Val,
    /// Gap between columns. `Val::Auto` is treated as zero.
    pub column_gap: Val,
}

impl Style {
    /// The default style, usable in `const` context.
    ///
    /// These values match CSS/`taffy` defaults, which are not all "zero": a flex
    /// container laid out as a row, `flex_shrink` of `1.0`, `auto` sizes and
    /// `auto` inset, but *zero* margin/padding/border/gap.
    pub const DEFAULT: Style = Style {
        display: Display::Flex,
        position_type: PositionType::Relative,
        inset: UiRect::AUTO,

        width: Val::Auto,
        height: Val::Auto,
        min_width: Val::Auto,
        min_height: Val::Auto,
        max_width: Val::Auto,
        max_height: Val::Auto,
        aspect_ratio: None,

        margin: UiRect::ZERO,
        padding: UiRect::ZERO,
        border: UiRect::ZERO,

        align_items: None,
        align_self: None,
        align_content: None,
        justify_content: None,

        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::NoWrap,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        flex_basis: Val::Auto,

        row_gap: Val::ZERO,
        column_gap: Val::ZERO,
    };
}

impl Default for Style {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Computed layout of a UI element, written back each frame by the layout system.
///
/// `size` is the element's width/height and `position` is its top-left corner,
/// both in window pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    /// Computed width and height in pixels.
    pub size: Vec2,
    /// Computed top-left position in window pixel coordinates.
    pub position: Vec2,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            size: Vec2::ZERO,
            position: Vec2::ZERO,
        }
    }
}

/// Current pointer interaction state of a UI element, updated each frame by the
/// interaction system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(Default)]
#[non_exhaustive]
pub enum Interaction {
    /// The pointer is neither over nor pressing the element.
    #[default]
    None,
    /// The pointer is over the element but not pressed.
    Hovered,
    /// The pointer is over the element and the primary button is held.
    Pressed,
}


/// Whether the element was **clicked** this frame — `Clicked(true)` for exactly the frame the
/// primary button is released over it after being pressed on it, `false` otherwise.
///
/// [`Interaction`] is a state recomputed every frame, so "was it clicked" had to be derived by
/// each game from the difference between two frames — `ui_layout.rs` said so in its header, and
/// a build button in a game is the first thing that needs it. The rule is the one desktop buttons
/// use: the click completes on release, and releasing **off** the element cancels it.
///
/// Part of [`ButtonBundle`](crate::ButtonBundle); an element without one simply has no click
/// signal, and its `Interaction` works as before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clicked(pub bool);

/// Makes an element draggable, and says how the drag is going — a slider's knob, an inventory
/// item, a window's title bar. Add it to any element that has an [`Interaction`].
///
/// A drag starts when the primary button goes **down on** the element (not when it slides onto it
/// already held) and lasts until the button is released — **wherever the pointer goes meanwhile**.
/// That capture is the point: a slider whose track is 8 px tall would otherwise lose the drag the
/// moment the hand wavers. Under a finger it works the same, since the first finger is the mouse.
///
/// The element is not moved for you — read [`delta`](Self::delta) or
/// [`position`](Self::position) and move or set what you like.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Drag {
    active: bool,
    start: Vec2,
    position: Vec2,
    just_started: bool,
    just_ended: bool,
}

impl Drag {
    /// Whether a drag is under way (`true` from the press through the release frame's end).
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.active
    }

    /// Where the drag began, window pixels.
    #[must_use]
    pub fn start(&self) -> Vec2 {
        self.start
    }

    /// Where the pointer is now (or was when the drag ended), window pixels.
    #[must_use]
    pub fn position(&self) -> Vec2 {
        self.position
    }

    /// How far the pointer has moved since the drag began.
    #[must_use]
    pub fn delta(&self) -> Vec2 {
        self.position - self.start
    }

    /// The drag began this frame.
    #[must_use]
    pub fn just_started(&self) -> bool {
        self.just_started
    }

    /// The drag ended this frame — the drop.
    #[must_use]
    pub fn just_ended(&self) -> bool {
        self.just_ended
    }

    /// Advances the drag for one frame: `pressed_here` is a press that began on the element this
    /// frame, `held` the button's state, `pointer` where it is.
    pub(crate) fn step(&mut self, pressed_here: bool, held: bool, pointer: Vec2) {
        self.just_started = false;
        self.just_ended = false;
        if self.active {
            self.position = pointer;
            if !held {
                self.active = false;
                self.just_ended = true;
            }
        } else if pressed_here {
            self.active = true;
            self.just_started = true;
            self.start = pointer;
            self.position = pointer;
        }
    }
}

/// A horizontal slider: a value in `0.0..=1.0` set by dragging across the element, or with the
/// left/right arrows and the d-pad while it has the [`UiNavigation`](crate::UiNavigation) focus.
/// Put it on a [`ButtonBundle`](crate::ButtonBundle) — it needs the `Interaction` — with a `Text`
/// for its label if it has one.
///
/// Under `gizmo-engine` the [`fill`](Self::fill) is painted over the element's background from its
/// left edge to `value` of its width, below its label. The drag is captured like a [`Drag`]: it
/// keeps following the pointer off the track until the release.
///
/// Every menu with a volume setting had to build this from a drag, a child node for the fill and
/// two `ZIndex` layers to keep the fill under the label — about forty lines in `menu`, and none
/// of it reachable from the keyboard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slider {
    /// The value, `0.0..=1.0`. Set it to move the slider; the system clamps it.
    pub value: f32,
    /// How far one arrow or d-pad press moves it (default `0.05`).
    pub step: f32,
    /// The colour of the filled part (default a light blue).
    pub fill: Vec4,
    drag: Drag,
    changed: bool,
}

impl Slider {
    /// A slider at `value` (clamped into `0.0..=1.0`).
    #[must_use]
    pub fn new(value: f32) -> Self {
        Self {
            value: if value.is_nan() { 0.0 } else { value.clamp(0.0, 1.0) },
            step: 0.05,
            fill: Vec4::new(0.25, 0.5, 0.85, 0.9),
            drag: Drag::default(),
            changed: false,
        }
    }

    /// With another arrow/d-pad step.
    #[must_use]
    pub fn with_step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// With another fill colour.
    #[must_use]
    pub fn with_fill(mut self, fill: Vec4) -> Self {
        self.fill = fill;
        self
    }

    /// The value moved this frame — by a drag or a key.
    #[must_use]
    pub fn just_changed(&self) -> bool {
        self.changed
    }

    /// Being dragged right now.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.drag.is_dragging()
    }

    /// A drag ended this frame — the moment to save a setting, rather than on every frame of it.
    #[must_use]
    pub fn just_released(&self) -> bool {
        self.drag.just_ended()
    }

    /// One frame: the drag (with the track's left edge and width), then a key step.
    pub(crate) fn step_frame(&mut self, pressed_here: bool, held: bool, pointer: Vec2, track: (f32, f32), keys: i32) {
        let before = self.value;
        self.drag.step(pressed_here, held, pointer);
        if self.drag.is_dragging() || self.drag.just_ended() {
            self.value = ((pointer.x - track.0) / track.1.max(1.0)).clamp(0.0, 1.0);
        }
        if keys != 0 {
            self.value = (self.value + keys as f32 * self.step).clamp(0.0, 1.0);
        }
        if self.value.is_nan() {
            self.value = 0.0;
        }
        self.value = self.value.clamp(0.0, 1.0);
        self.changed = self.value != before;
    }
}

impl Default for Slider {
    fn default() -> Self {
        Self::new(0.0)
    }
}

/// A bar filled to [`value`](Self::value) (`0.0..=1.0`) of its width — health, loading, a song's
/// progress. Not interactive: set `value` and it shows it. Under `gizmo-engine` the
/// [`fill`](Self::fill) is painted over the element's background, under its label, like a
/// [`Slider`]'s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressBar {
    /// How full, `0.0..=1.0` (clamped when drawn).
    pub value: f32,
    /// The filled part's colour.
    pub fill: Vec4,
}

impl ProgressBar {
    /// A bar at `value`, filled green.
    #[must_use]
    pub fn new(value: f32) -> Self {
        Self { value, fill: Vec4::new(0.3, 0.8, 0.4, 0.95) }
    }

    /// With another fill colour.
    #[must_use]
    pub fn with_fill(mut self, fill: Vec4) -> Self {
        self.fill = fill;
        self
    }
}

impl Default for ProgressBar {
    fn default() -> Self {
        Self::new(0.0)
    }
}

/// An on/off switch: every [`Clicked`] of its element flips [`on`](Self::on) — so the mouse, a
/// finger, Enter and the gamepad's south button all flip it, through the same click a button
/// gets. Put it on a [`ButtonBundle`](crate::ButtonBundle). Under `gizmo-engine` a check box is
/// drawn at the element's left edge, filled while on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Toggle {
    /// On or off. Set it to change it without a click (it does not count as a change then).
    pub on: bool,
    changed: bool,
}

impl Toggle {
    /// A switch starting `on` or off.
    #[must_use]
    pub fn new(on: bool) -> Self {
        Self { on, changed: false }
    }

    /// It flipped this frame.
    #[must_use]
    pub fn just_changed(&self) -> bool {
        self.changed
    }

    pub(crate) fn step(&mut self, clicked: bool) {
        self.changed = clicked;
        if clicked {
            self.on = !self.on;
        }
    }
}

/// Fill color of a UI element, stored as a linear RGBA vector with each
/// channel in the `0.0..=1.0` range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackgroundColor(pub Vec4);

impl Default for BackgroundColor {
    fn default() -> Self {
        Self(Vec4::new(1.0, 1.0, 1.0, 1.0))
    }
}

/// Clips what is drawn inside this node to its box: its own label, and every descendant's
/// background and label (a nested clipper narrows it further — the intersection).
///
/// Without it a long label runs out of its button and a scrolled list spills over the panel it
/// sits in — `gizmo-ui` had no clipping at all, and `ui_layout`'s fourth button overflowed on
/// purpose to show it. The trimming happens on the CPU per glyph and per background quad, so it
/// costs no extra draw call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClipContent;

/// Which layer a UI node — and everything under it that does not set its own — is painted and
/// hit-tested on. Higher is in front; the default (no component) is `0`.
///
/// Every background used to be painted under every label, and every node under the cursor
/// reported `Hovered`: a menu opened over the HUD showed the HUD's labels through itself and let
/// clicks reach the buttons beneath. Now layers paint low to high (backgrounds, then labels, per
/// layer) and only the highest layer under the cursor is hovered. Within one layer the old rule
/// holds — overlapping nodes on the same layer are the author's to separate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZIndex(pub i32);

/// Scrolls a node's children: they are laid out as usual, then moved up (and left) by
/// [`offset`](Self::offset), and clipped to the node's box as if it carried [`ClipContent`]. The
/// mouse wheel over the node scrolls it by [`speed`](Self::speed) pixels a notch; the layout keeps
/// the offset between zero and how far the children reach past the box.
///
/// Its direct children never shrink to fit it (their `flex_shrink` is taken as zero): CSS's
/// default `flex-shrink: 1`, which [`Style`] otherwise keeps, would squash a long list into the
/// box and leave nothing to scroll.
///
/// A list longer than its panel could be clipped since `ClipContent`, but not reached; a
/// leaderboard or an inventory needs this.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scroll {
    /// How far the content is scrolled, in pixels: `y` down, `x` right. Set it to jump; the
    /// layout clamps it.
    pub offset: Vec2,
    /// Pixels per mouse-wheel notch.
    pub speed: f32,
    max: Vec2,
}

impl Default for Scroll {
    fn default() -> Self {
        Self { offset: Vec2::ZERO, speed: 40.0, max: Vec2::ZERO }
    }
}

impl Scroll {
    /// Pixels per mouse-wheel notch instead of the default 40.
    #[must_use]
    pub fn with_speed(mut self, px: f32) -> Self {
        self.speed = px;
        self
    }

    /// How far it can scroll, from the last layout: the children's extent past the box.
    #[must_use]
    pub fn max_offset(&self) -> Vec2 {
        self.max
    }

    /// Records the extent and clamps the offset into it — the layout's half.
    pub(crate) fn fit(&mut self, max: Vec2) {
        self.max = max.max(Vec2::ZERO);
        self.offset = self.offset.clamp(Vec2::ZERO, self.max);
    }
}

/// Marker component for the root of a UI tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiRoot;
gizmo_core::impl_component!(Style, Node, Interaction, Clicked, BackgroundColor, UiRoot, ClipContent, ZIndex, Scroll, Drag, Slider, ProgressBar, Toggle);

#[cfg(test)]
mod tests {
    use super::*;

    /// `Style` must be `Send + Sync` because it is an ECS component
    /// (`Component: 'static + Send + Sync + Clone`).
    ///
    /// Before the A3-followup change this was only true thanks to two
    /// hand-written `unsafe impl`s, because the component stored a
    /// `taffy::style::Style` by value and that type is structurally `!Send`.
    /// The POD `Style` gets both auto traits derived, so those `unsafe impl`s
    /// are gone — this test now asserts a property the compiler proves rather
    /// than one we asserted by hand.
    #[test]
    fn style_is_send_and_sync_without_unsafe() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Style>();
        assert_send_sync::<Val>();
        assert_send_sync::<UiRect>();
    }

    /// `Style` is plain old data — it must stay `Copy`, with no heap ownership,
    /// no interior mutability and no pointers. `Copy` is the cheapest available
    /// proof of that: it stops compiling the moment a `Vec`, `Rc`, `String` or
    /// raw pointer field is added, which is exactly the class of change that
    /// would put an `unsafe impl` back into this file.
    #[test]
    fn style_is_copy_plain_old_data() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<Style>();
        let a = Style { width: Val::Px(3.0), ..Default::default() };
        let b = a; // copy, not move
        assert_eq!(a, b);
        assert_eq!(a.width, Val::Px(3.0));
    }

    /// The defaults are a contract: the bundles spawn `Style::default()`, so a
    /// drift here silently changes the layout of every element that does not
    /// set a field. In particular the non-zero ones (flex row, `flex_shrink`
    /// 1.0, `auto` sizes/inset but zero margin/padding/gap) are CSS defaults,
    /// not the `Default::default()` of each field type.
    #[test]
    fn defaults_are_css_defaults_not_zero() {
        let s = Style::default();
        assert_eq!(s, Style::DEFAULT);
        assert_eq!(s.display, Display::Flex);
        assert_eq!(s.position_type, PositionType::Relative);
        assert_eq!(s.flex_direction, FlexDirection::Row);
        assert_eq!(s.flex_wrap, FlexWrap::NoWrap);
        assert_eq!(s.flex_grow, 0.0);
        assert_eq!(s.flex_shrink, 1.0, "CSS default is 1.0; 0.0 would disable shrinking");
        assert_eq!(s.flex_basis, Val::Auto);
        assert_eq!(s.width, Val::Auto);
        assert_eq!(s.aspect_ratio, None);
        // `inset` defaults to auto, the other three rects to zero. An all-auto
        // margin means CSS auto margins (centring), which is NOT the default.
        assert_eq!(s.inset, UiRect::AUTO);
        assert_eq!(s.margin, UiRect::ZERO);
        assert_eq!(s.padding, UiRect::ZERO);
        assert_eq!(s.border, UiRect::ZERO);
        assert_eq!(s.row_gap, Val::ZERO);
        assert_eq!(s.column_gap, Val::ZERO);
    }

    /// `UiRect::default()` is ZERO, not AUTO, even though `Val::default()` is
    /// `Auto`. The two differ on purpose (CSS margin/padding/border start at
    /// zero) and the difference is documented on both types, so pin it.
    #[test]
    fn ui_rect_default_is_zero_while_val_default_is_auto() {
        assert_eq!(Val::default(), Val::Auto);
        assert_eq!(UiRect::default(), UiRect::ZERO);
        assert_ne!(UiRect::default(), UiRect::AUTO);
        assert_eq!(
            UiRect::axes(Val::Px(4.0), Val::Percent(10.0)),
            UiRect::new(Val::Px(4.0), Val::Px(4.0), Val::Percent(10.0), Val::Percent(10.0))
        );
    }
}
