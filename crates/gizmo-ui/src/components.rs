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
    /// CSS grid: children are placed in the tracks of [`Style::grid_template_columns`] and
    /// [`Style::grid_template_rows`] (implicit ones sized by `grid_auto_*`), in order or where
    /// their [`Style::grid_column`] / [`Style::grid_row`] put them. The gaps are
    /// [`Style::column_gap`] and [`Style::row_gap`].
    Grid,
}

/// One column's or row's size in a [`Display::Grid`] container.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum GridTrack {
    /// Exactly this many layout pixels.
    Px(f32),
    /// This percentage of the container's inner size.
    Percent(f32),
    /// A share of the space the fixed tracks leave: `Fr(1.0)` beside `Fr(2.0)` gets a third.
    Fr(f32),
    /// As big as its contents (and stretched into leftover space when nothing is `Fr`).
    Auto,
}

/// The tracks of a grid axis — up to [`GridTracks::MAX`] of them, held inline so [`Style`] stays
/// `Copy`.
///
/// ```
/// use gizmo_ui::components::{GridTrack, GridTracks};
/// let inventory = GridTracks::repeat(6, GridTrack::Px(64.0));
/// let sidebar = GridTracks::new(&[GridTrack::Px(200.0), GridTrack::Fr(1.0)]);
/// assert_eq!(inventory.len(), 6);
/// assert_eq!(sidebar.as_slice()[1], GridTrack::Fr(1.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridTracks {
    tracks: [GridTrack; GridTracks::MAX],
    len: u8,
}

impl GridTracks {
    /// The most tracks one axis holds. Past it a grid is better built from nested ones.
    pub const MAX: usize = 16;
    /// No explicit tracks: every column (row) is implicit, sized by `grid_auto_columns` (`_rows`).
    pub const NONE: GridTracks = GridTracks { tracks: [GridTrack::Auto; GridTracks::MAX], len: 0 };

    /// These tracks, in order. **Panics** past [`MAX`](Self::MAX) — a silently shorter grid would
    /// put its last items somewhere else.
    pub fn new(tracks: &[GridTrack]) -> Self {
        assert!(tracks.len() <= Self::MAX, "{} grid tracks, the most is {}", tracks.len(), Self::MAX);
        let mut out = Self::NONE;
        out.tracks[..tracks.len()].copy_from_slice(tracks);
        out.len = tracks.len() as u8;
        out
    }

    /// `count` copies of `track` — CSS `repeat(count, track)`. **Panics** past [`MAX`](Self::MAX).
    pub fn repeat(count: usize, track: GridTrack) -> Self {
        Self::new(&vec![track; count])
    }

    /// The tracks, in order.
    pub fn as_slice(&self) -> &[GridTrack] {
        &self.tracks[..self.len as usize]
    }

    /// How many.
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// None at all.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for GridTracks {
    fn default() -> Self {
        Self::NONE
    }
}

/// Where a grid item goes along one axis. Lines are numbered from 1, as in CSS; negative lines
/// count from the end (`-1` is the last line).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum GridPlacement {
    /// The next free cell, in order — the default.
    #[default]
    Auto,
    /// Starting at this line, one track wide.
    Line(i16),
    /// The next free place, `n` tracks wide.
    Span(u16),
    /// Starting at `line`, `span` tracks wide.
    LineSpan(i16, u16),
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
/// - **Most of CSS Grid's extras.** [`Display::Grid`] with explicit tracks
///   ([`GridTracks`] of [`GridTrack`]: pixels, percent, `fr`, auto), one size for
///   implicit tracks, and line/span placement is modelled. Not: named lines and
///   areas, `minmax()`, `auto-fill`/`auto-fit` repetition, `grid_auto_flow`
///   (always row), several implicit-track sizes, and `justify_items` /
///   `justify_self` (`align_items`/`align_self` are modelled and apply).
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

    /// [`Display::Grid`]: the explicit columns, left to right.
    pub grid_template_columns: GridTracks,
    /// [`Display::Grid`]: the explicit rows, top to bottom.
    pub grid_template_rows: GridTracks,
    /// [`Display::Grid`]: the width of every column past the explicit ones.
    pub grid_auto_columns: GridTrack,
    /// [`Display::Grid`]: the height of every row past the explicit ones — an inventory's
    /// fixed cell height, with only its columns declared.
    pub grid_auto_rows: GridTrack,
    /// In a grid parent: which columns this element takes.
    pub grid_column: GridPlacement,
    /// In a grid parent: which rows this element takes.
    pub grid_row: GridPlacement,
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

        grid_template_columns: GridTracks::NONE,
        grid_template_rows: GridTracks::NONE,
        grid_auto_columns: GridTrack::Auto,
        grid_auto_rows: GridTrack::Auto,
        grid_column: GridPlacement::Auto,
        grid_row: GridPlacement::Auto,
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
    over: Option<u32>,
    dropped_on: Option<u32>,
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

    /// The drop system's answer for this frame.
    pub(crate) fn set_target(&mut self, over: Option<u32>, dropped_on: Option<u32>) {
        self.over = over;
        self.dropped_on = dropped_on;
    }

    /// The [`DropTarget`] the drag is over right now, if any — for highlighting where it would
    /// land. `None` when no drag is under way.
    #[must_use]
    pub fn over(&self) -> Option<u32> {
        self.over
    }

    /// The [`DropTarget`] the drag was let go over, on the frame it ended
    /// ([`just_ended`](Self::just_ended)); `None` on every other frame, and when it was let go
    /// over no target.
    #[must_use]
    pub fn dropped_on(&self) -> Option<u32> {
        self.dropped_on
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

/// An element a [`Drag`] can be let go over — an inventory slot, a bin, an order on a board.
///
/// "What was it dropped on" is not "what is under the pointer": the dragged element follows the
/// pointer, so *it* is under the pointer, and everything beneath it reads
/// [`Interaction::None`]. `ui_drop_system` asks the other question — which target holds the
/// drag's position — and answers on both ends: here, and in [`Drag::over`] /
/// [`Drag::dropped_on`].
///
/// When several targets hold the point, the one on the highest [`ZIndex`] layer wins, and among
/// those the smallest (so a slot inside a panel that is a target too gets the drop, not the
/// panel). A target the drag cannot see does not count: one scrolled or clipped out of its
/// [`ClipContent`] / [`Scroll`] ancestor, one with no size (hidden, not laid out), and the dragged
/// element itself and everything under it — an item can be a target for other items and still be
/// dragged without dropping onto itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropTarget {
    hovered_by: Option<u32>,
    dropped: Option<u32>,
}

impl DropTarget {
    /// The dragged element over this target right now, if any.
    #[must_use]
    pub fn hovered_by(&self) -> Option<u32> {
        self.hovered_by
    }

    /// The element let go over this target this frame, if any — the frame its
    /// [`Drag::just_ended`] is `true`.
    #[must_use]
    pub fn dropped(&self) -> Option<u32> {
        self.dropped
    }

    pub(crate) fn answer(hovered_by: Option<u32>, dropped: Option<u32>) -> Self {
        Self { hovered_by, dropped }
    }
}

/// A line of help that appears beside the pointer when it rests on the element — what a stage
/// becomes when merged, what a button does, what an item is worth.
///
/// Shown once the pointer has stayed on the element for [`delay`](Self::delay) seconds (real
/// time, so a paused game still explains itself) with no button held; hidden the moment it
/// leaves, a button goes down, or [`text`](Self::text) is empty. Keyboard focus alone does not
/// show it: a tooltip sits at the pointer, and a focused element may be nowhere near it.
/// `ui_tooltip_system` keeps [`shown_at`](Self::shown_at) — the pointer, in window pixels —
/// which is all a draw path needs.
///
/// Under `gizmo-engine` it is drawn above every UI layer (under an open dropdown list), in the
/// font of the element's own `Text` at that `Text`'s size, light on a dark box, `\n` for lines,
/// wrapped at 320 layout pixels, and kept inside the window. An element with no `Text` has no
/// font to draw it in, and its tooltip is not drawn — as a `Dropdown`'s list borrows its button's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tooltip {
    /// What it says. Change it whenever; an empty string shows nothing.
    pub text: String,
    /// Seconds of resting before it appears.
    pub delay: f32,
    rested: f32,
    shown_at: Option<Vec2>,
}

impl Tooltip {
    /// A tooltip with the usual half-second delay.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), delay: 0.5, rested: 0.0, shown_at: None }
    }

    /// With another delay; `0` shows it at once.
    #[must_use]
    pub fn with_delay(mut self, seconds: f32) -> Self {
        self.delay = seconds.max(0.0);
        self
    }

    /// Where the pointer is while the tooltip is up (window pixels), `None` while it is not.
    #[must_use]
    pub fn shown_at(&self) -> Option<Vec2> {
        self.shown_at
    }

    /// Its state after one more frame — `resting` is the pointer on the element with no button
    /// held — without writing it, so the system can write only a change.
    pub(crate) fn next_state(&self, resting: bool, pointer: Vec2, dt: f32) -> (f32, Option<Vec2>) {
        if resting && !self.text.is_empty() {
            let rested = self.rested + dt;
            (rested, (rested >= self.delay).then_some(pointer))
        } else {
            (0.0, None)
        }
    }

    pub(crate) fn state(&self) -> (f32, Option<Vec2>) {
        (self.rested, self.shown_at)
    }

    pub(crate) fn set_state(&mut self, (rested, shown_at): (f32, Option<Vec2>)) {
        self.rested = rested;
        self.shown_at = shown_at;
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

/// A select list: a button showing its [`selected`](Self::selected) option, which opens a list of
/// all of them below itself — resolution, quality, difficulty. Put it on a
/// [`ButtonBundle`](crate::ButtonBundle) with a `Text` (whose font, size and colour the options
/// use).
///
/// - A click opens it; a press on an option picks it and closes the list; a press anywhere else
///   closes it. While open, the list takes the pointer: nothing under it is hovered or pressed.
/// - With the [`UiNavigation`](crate::UiNavigation) focus, ←/→ and the d-pad step through the
///   options without opening it, as they step a [`Slider`].
/// - Under `gizmo-engine` the label shows the selected option, and the open list is drawn above
///   every UI layer.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Dropdown {
    /// The options, in order.
    pub options: Vec<String>,
    /// Which one is selected (clamped into range when used).
    pub selected: usize,
    open: bool,
    hovered: Option<usize>,
    changed: bool,
}

impl Dropdown {
    /// A list of `options` with `selected` chosen.
    #[must_use]
    pub fn new(options: impl IntoIterator<Item = impl Into<String>>, selected: usize) -> Self {
        Self { options: options.into_iter().map(Into::into).collect(), selected, ..Default::default() }
    }

    /// The selected option's text, `None` with no options.
    #[must_use]
    pub fn selected_option(&self) -> Option<&str> {
        let i = self.selected.min(self.options.len().checked_sub(1)?);
        self.options.get(i).map(String::as_str)
    }

    /// Whether its list is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens or closes its list from code — closing it on Escape, opening it for a tutorial.
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        if !open {
            self.hovered = None;
        }
    }

    /// The option under the pointer while open.
    #[must_use]
    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// The selection changed this frame.
    #[must_use]
    pub fn just_changed(&self) -> bool {
        self.changed
    }

    /// Where option `i` is drawn while open: a row the button's size, below it.
    #[must_use]
    pub fn option_rect(&self, node: &Node, i: usize) -> (Vec2, Vec2) {
        (node.position + Vec2::new(0.0, node.size.y * (i + 1) as f32), node.size)
    }

    /// The option at `point` while open, if any.
    #[must_use]
    pub fn option_at(&self, node: &Node, point: Vec2) -> Option<usize> {
        if !self.open {
            return None;
        }
        (0..self.options.len()).find(|&i| {
            let (at, size) = self.option_rect(node, i);
            point.x >= at.x && point.y >= at.y && point.x < at.x + size.x && point.y < at.y + size.y
        })
    }

    /// One frame: `clicked` its button's click, `press` a press that began this frame (and where),
    /// `step` ←/→ while focused (−1, 0, 1).
    pub(crate) fn step_frame(&mut self, clicked: bool, press: Option<Vec2>, on_button: bool, pointer: Vec2, node: &Node, step: i32) {
        self.changed = false;
        let before = self.selected;
        self.hovered = self.option_at(node, pointer);
        if self.open {
            if let Some(p) = press {
                match self.option_at(node, p) {
                    Some(i) => {
                        self.selected = i;
                        self.open = false;
                    }
                    // A press on the button itself is left to its click, which closes the list.
                    None if !on_button => self.open = false,
                    None => {}
                }
            }
        }
        if clicked {
            self.open = !self.open;
        }
        let n = self.options.len();
        if step != 0 && n > 0 && !self.open {
            self.selected = (self.selected.min(n - 1) as i32 + step).rem_euclid(n as i32) as usize;
        }
        if !self.open {
            self.hovered = None;
        }
        self.changed = self.selected != before;
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
    /// While the scrollbar's thumb is held: where on the thumb the pointer took it.
    grab: Option<f32>,
    /// A finger on the content: where it came down, where it was last frame, and whether it has
    /// travelled far enough to be a scroll rather than a tap.
    flick: Option<Flick>,
    /// Pixels per second the content keeps sliding after a flick, decaying.
    velocity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Flick {
    start: f32,
    last: f32,
    scrolling: bool,
}

/// How far a finger travels before a press on a list's content becomes a scroll (and stops being
/// a tap on the item under it).
const FLICK_SLOP: f32 = 8.0;
/// How fast a flick's slide dies away: the velocity is multiplied by this every second.
const FLICK_DECAY_PER_SECOND: f32 = 0.05;

impl Default for Scroll {
    fn default() -> Self {
        Self { offset: Vec2::ZERO, speed: 40.0, max: Vec2::ZERO, grab: None, flick: None, velocity: 0.0 }
    }
}

/// The vertical scrollbar's width, and its gap from the box's right edge.
const BAR_WIDTH: f32 = 6.0;
const BAR_GAP: f32 = 2.0;
/// How wide the strip that takes the pointer for the bar is — wider than the bar, for a thumb.
const BAR_GRAB_WIDTH: f32 = 16.0;

impl Scroll {
    /// Pixels per mouse-wheel notch instead of the default 40.
    #[must_use]
    pub fn with_speed(mut self, px: f32) -> Self {
        self.speed = px;
        self
    }

    /// The vertical scrollbar's thumb in `node`'s box — top-left and size, window pixels — or
    /// `None` when nothing overflows. Its length is the visible share of the content and its place
    /// the offset's share of the travel; under `gizmo-engine` it is drawn, and it can be dragged.
    #[must_use]
    pub fn thumb(&self, node: &Node) -> Option<(Vec2, Vec2)> {
        if self.max.y <= 0.0 || node.size.y <= 0.0 {
            return None;
        }
        let track = node.size.y;
        let length = (track * track / (track + self.max.y)).max(track.min(20.0));
        let travel = track - length;
        let y = node.position.y + travel * (self.offset.y / self.max.y).clamp(0.0, 1.0);
        let x = node.position.x + node.size.x - BAR_WIDTH - BAR_GAP;
        Some((Vec2::new(x, y), Vec2::new(BAR_WIDTH, length)))
    }

    /// Whether `point` is in the strip along the right edge that belongs to the scrollbar — which
    /// is only there while something overflows.
    pub(crate) fn in_bar_strip(&self, node: &Node, point: Vec2) -> bool {
        self.max.y > 0.0
            && point.x >= node.position.x + node.size.x - BAR_GRAB_WIDTH
            && point.x < node.position.x + node.size.x
            && point.y >= node.position.y
            && point.y < node.position.y + node.size.y
    }

    /// One frame of the thumb's drag: `pressed` a press that began this frame, `held` the button,
    /// `pointer` where it is.
    pub(crate) fn drag_thumb(&mut self, node: &Node, pressed: bool, held: bool, pointer: Vec2) {
        let Some((at, size)) = self.thumb(node) else {
            self.grab = None;
            return;
        };
        if pressed && self.in_bar_strip(node, pointer) {
            // On the thumb: hold it where it was taken. On the track: centre it under the pointer.
            let on_thumb = pointer.y >= at.y && pointer.y < at.y + size.y;
            self.grab = Some(if on_thumb { pointer.y - at.y } else { size.y * 0.5 });
        }
        if !held {
            self.grab = None;
        }
        if let Some(grab) = self.grab {
            let travel = node.size.y - size.y;
            if travel > 0.0 {
                let share = ((pointer.y - grab - node.position.y) / travel).clamp(0.0, 1.0);
                self.offset.y = share * self.max.y;
            }
        }
    }

    /// One frame of a finger on the content: `pressed` a finger came down in the box this frame,
    /// `held` it is still down, `y` where it is, `dt` the frame. Past [`FLICK_SLOP`] the content
    /// follows the finger; let go, it keeps sliding at the finger's last speed, decaying.
    pub(crate) fn flick_frame(&mut self, pressed: bool, held: bool, y: f32, dt: f32) {
        if pressed && self.grab.is_none() {
            self.flick = Some(Flick { start: y, last: y, scrolling: false });
            self.velocity = 0.0;
        }
        match self.flick.as_mut() {
            Some(f) if held => {
                if !f.scrolling && (y - f.start).abs() > FLICK_SLOP {
                    f.scrolling = true;
                }
                if f.scrolling {
                    let dy = y - f.last;
                    self.offset.y = (self.offset.y - dy).clamp(0.0, self.max.y.max(0.0));
                    if dt > 0.0 {
                        self.velocity = -dy / dt;
                    }
                }
                f.last = y;
            }
            Some(_) => {
                self.flick = None;
            }
            None => {
                // Sliding on after a flick.
                if self.velocity != 0.0 && dt > 0.0 {
                    self.offset.y = (self.offset.y + self.velocity * dt).clamp(0.0, self.max.y.max(0.0));
                    self.velocity *= FLICK_DECAY_PER_SECOND.powf(dt);
                    if self.velocity.abs() < 5.0 || self.offset.y <= 0.0 || self.offset.y >= self.max.y {
                        self.velocity = 0.0;
                    }
                }
            }
        }
    }

    /// A finger is scrolling the content right now — a press under it is a scroll, not a tap.
    #[must_use]
    pub fn is_flicking(&self) -> bool {
        self.flick.is_some_and(|f| f.scrolling)
    }

    /// Whether the thumb is being dragged.
    #[must_use]
    pub fn is_dragging_thumb(&self) -> bool {
        self.grab.is_some()
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
gizmo_core::impl_component!(Style, Node, Interaction, Clicked, BackgroundColor, UiRoot, ClipContent, ZIndex, Scroll, Drag, DropTarget, Slider, ProgressBar, Toggle, Dropdown, Tooltip);

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
