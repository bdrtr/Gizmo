use std::collections::{HashMap, HashSet};
use taffy::geometry::{Rect as TaffyRect, Size as TaffySize};
use taffy::style::{
    AlignContent as TaffyAlignContent, AlignItems as TaffyAlignItems, Dimension,
    Display as TaffyDisplay, FlexDirection as TaffyFlexDirection, FlexWrap as TaffyFlexWrap,
    LengthPercentage, LengthPercentageAuto, Position as TaffyPosition, Style as TaffyStyle,
};
use taffy::{AvailableSpace, NodeId, TaffyTree};
use gizmo_math::Vec2;

use crate::components::{
    AlignContent, AlignItems, Display, FlexDirection, FlexWrap, PositionType, Style, UiRect, Val,
};

/// Shared layout state for the UI, stored as a resource.
///
/// This is **the only place in the crate that knows about `taffy`**. It owns the
/// layout tree, the entity-id → tree-node mapping, and the current window size
/// used as the available space for root nodes. The tree and the mapping are
/// private on purpose: exposing them would put `taffy` types back into this
/// crate's public API, which is exactly what the POD [`Style`] change removed.
/// Read the results through [`UiContext::relative_layout`], or — for absolute
/// window coordinates — through the `Node` component the layout system writes.
pub struct UiContext {
    /// The taffy layout tree backing all UI nodes.
    taffy: TaffyTree,
    /// Mapping from entity id to its corresponding taffy node.
    entity_to_node: HashMap<u32, NodeId>,
    /// Size of the window, used as available space for root layout.
    pub window_size: Vec2,
}

// SAFETY: `TaffyTree` is `!Send + !Sync` for exactly one reason — it stores
// `NodeData`, which embeds `taffy::Style`, which embeds `CompactLength`'s
// `*const ()` tagged-pointer union. Every `CompactLength` that can ever enter
// this tree is produced by `UiContext::to_taffy_style` below, and that function
// calls only `length()`, `percent()` and `auto()` — never `calc()`, the sole
// constructor of the pointer variant. So the union in this tree only ever holds
// an `f32` plus a tag: plain data, no interior mutability, no provenance,
// nothing to race on.
//
// This invariant is now **local to this file**, which is the point of the
// A3-followup change. Previously the component stored a user-supplied
// `taffy::Style` and the argument had to lean on taffy's `calc` feature being
// off — a Cargo feature the dependent crate cannot observe, and which any other
// crate in the user's graph could switch back on. Today the public [`Style`] is
// POD with no `calc` variant, so even with `calc` compiled in there is no path
// from safe code to a pointer-tagged length in this tree.
// `no_length_conversion_produces_a_calc_tagged_value` below asserts the tags.
//
// The remaining fields (`HashMap<u32, NodeId>`, `Vec2`) are plain data.
// `UiContext` is stored as a `World` resource, which requires `Send + Sync`.
unsafe impl Send for UiContext {}
// SAFETY: see the `Send` impl above — same invariant.
unsafe impl Sync for UiContext {}

impl Default for UiContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Converts a [`Val`] to a taffy [`Dimension`] (sizes, `flex_basis`).
fn to_dimension(val: Val) -> Dimension {
    match val {
        Val::Auto => Dimension::auto(),
        Val::Px(px) => Dimension::length(px),
        // taffy percentages are a 0.0..=1.0 fraction; `Val::Percent` is the CSS
        // 0.0..=100.0 scale. This division is the whole difference.
        Val::Percent(pct) => Dimension::percent(pct / 100.0),
    }
}

/// Converts a [`Val`] to a taffy [`LengthPercentageAuto`] (inset, margin).
fn to_length_percentage_auto(val: Val) -> LengthPercentageAuto {
    match val {
        Val::Auto => LengthPercentageAuto::auto(),
        Val::Px(px) => LengthPercentageAuto::length(px),
        Val::Percent(pct) => LengthPercentageAuto::percent(pct / 100.0),
    }
}

/// Converts a [`Val`] to a taffy [`LengthPercentage`] (padding, border, gap).
///
/// These CSS properties have no `auto`, so [`Val::Auto`] collapses to zero —
/// documented on the corresponding [`Style`] fields.
fn to_length_percentage(val: Val) -> LengthPercentage {
    match val {
        Val::Auto => LengthPercentage::length(0.0),
        Val::Px(px) => LengthPercentage::length(px),
        Val::Percent(pct) => LengthPercentage::percent(pct / 100.0),
    }
}

fn rect_length_percentage_auto(rect: UiRect) -> TaffyRect<LengthPercentageAuto> {
    TaffyRect {
        left: to_length_percentage_auto(rect.left),
        right: to_length_percentage_auto(rect.right),
        top: to_length_percentage_auto(rect.top),
        bottom: to_length_percentage_auto(rect.bottom),
    }
}

fn rect_length_percentage(rect: UiRect) -> TaffyRect<LengthPercentage> {
    TaffyRect {
        left: to_length_percentage(rect.left),
        right: to_length_percentage(rect.right),
        top: to_length_percentage(rect.top),
        bottom: to_length_percentage(rect.bottom),
    }
}

fn to_align_items(value: AlignItems) -> TaffyAlignItems {
    match value {
        AlignItems::Start => TaffyAlignItems::Start,
        AlignItems::End => TaffyAlignItems::End,
        AlignItems::FlexStart => TaffyAlignItems::FlexStart,
        AlignItems::FlexEnd => TaffyAlignItems::FlexEnd,
        AlignItems::Center => TaffyAlignItems::Center,
        AlignItems::Baseline => TaffyAlignItems::Baseline,
        AlignItems::Stretch => TaffyAlignItems::Stretch,
    }
}

fn to_align_content(value: AlignContent) -> TaffyAlignContent {
    match value {
        AlignContent::Start => TaffyAlignContent::Start,
        AlignContent::End => TaffyAlignContent::End,
        AlignContent::FlexStart => TaffyAlignContent::FlexStart,
        AlignContent::FlexEnd => TaffyAlignContent::FlexEnd,
        AlignContent::Center => TaffyAlignContent::Center,
        AlignContent::Stretch => TaffyAlignContent::Stretch,
        AlignContent::SpaceBetween => TaffyAlignContent::SpaceBetween,
        AlignContent::SpaceEvenly => TaffyAlignContent::SpaceEvenly,
        AlignContent::SpaceAround => TaffyAlignContent::SpaceAround,
    }
}

impl UiContext {
    /// Creates an empty [`UiContext`] with a default window size.
    pub fn new() -> Self {
        Self {
            taffy: TaffyTree::new(),
            entity_to_node: HashMap::new(),
            window_size: Vec2::new(1280.0, 720.0), // Default window size
        }
    }

    /// Translates the POD [`Style`] component into the `taffy` style used by the
    /// layout algorithm.
    ///
    /// This is the crate's taffy boundary: it is the only function that names a
    /// taffy style type, and every field the component does not model is left at
    /// `taffy`'s own default (see the "Not modelled" section of [`Style`]).
    fn to_taffy_style(style: &Style) -> TaffyStyle {
        TaffyStyle {
            display: match style.display {
                Display::Flex => TaffyDisplay::Flex,
                Display::Block => TaffyDisplay::Block,
                Display::None => TaffyDisplay::None,
            },
            position: match style.position_type {
                PositionType::Relative => TaffyPosition::Relative,
                PositionType::Absolute => TaffyPosition::Absolute,
            },
            inset: rect_length_percentage_auto(style.inset),

            size: TaffySize {
                width: to_dimension(style.width),
                height: to_dimension(style.height),
            },
            min_size: TaffySize {
                width: to_dimension(style.min_width),
                height: to_dimension(style.min_height),
            },
            max_size: TaffySize {
                width: to_dimension(style.max_width),
                height: to_dimension(style.max_height),
            },
            aspect_ratio: style.aspect_ratio,

            margin: rect_length_percentage_auto(style.margin),
            padding: rect_length_percentage(style.padding),
            border: rect_length_percentage(style.border),

            align_items: style.align_items.map(to_align_items),
            align_self: style.align_self.map(to_align_items),
            align_content: style.align_content.map(to_align_content),
            justify_content: style.justify_content.map(to_align_content),

            flex_direction: match style.flex_direction {
                FlexDirection::Row => TaffyFlexDirection::Row,
                FlexDirection::Column => TaffyFlexDirection::Column,
                FlexDirection::RowReverse => TaffyFlexDirection::RowReverse,
                FlexDirection::ColumnReverse => TaffyFlexDirection::ColumnReverse,
            },
            flex_wrap: match style.flex_wrap {
                FlexWrap::NoWrap => TaffyFlexWrap::NoWrap,
                FlexWrap::Wrap => TaffyFlexWrap::Wrap,
                FlexWrap::WrapReverse => TaffyFlexWrap::WrapReverse,
            },
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            flex_basis: to_dimension(style.flex_basis),

            // taffy packs both gaps into one `Size`: width is the *column* gap
            // (horizontal), height the *row* gap (vertical).
            gap: TaffySize {
                width: to_length_percentage(style.column_gap),
                height: to_length_percentage(style.row_gap),
            },

            ..TaffyStyle::DEFAULT
        }
    }

    /// Creates the layout node for `entity` if it has none, or pushes the
    /// current [`Style`] into the existing one.
    ///
    /// Errors from taffy are swallowed: on the rare allocation/insertion failure
    /// the entity is skipped for this frame and retried on the next one, which
    /// is safe because every read path goes through [`Self::relative_layout`].
    pub(crate) fn sync_style(&mut self, entity: u32, style: &Style) {
        let taffy_style = Self::to_taffy_style(style);
        match self.entity_to_node.get(&entity) {
            Some(&node_id) => {
                let _ = self.taffy.set_style(node_id, taffy_style);
            }
            None => {
                if let Ok(node_id) = self.taffy.new_leaf(taffy_style) {
                    self.entity_to_node.insert(entity, node_id);
                }
            }
        }
    }

    /// Drops the layout nodes of every tracked entity that is not in `keep`.
    pub(crate) fn retain_entities(&mut self, keep: &HashSet<u32>) {
        let stale: Vec<(u32, NodeId)> = self
            .entity_to_node
            .iter()
            .filter(|(entity, _)| !keep.contains(*entity))
            .map(|(&entity, &node_id)| (entity, node_id))
            .collect();
        for (entity, node_id) in stale {
            let _ = self.taffy.remove(node_id);
            self.entity_to_node.remove(&entity);
        }
    }

    /// Replaces `entity`'s children in the layout tree. Child ids with no layout
    /// node of their own are skipped; a no-op if `entity` itself is untracked.
    pub(crate) fn set_children(&mut self, entity: u32, children: &[u32]) {
        let Some(&node_id) = self.entity_to_node.get(&entity) else {
            return;
        };
        let taffy_children: Vec<NodeId> = children
            .iter()
            .filter_map(|child| self.entity_to_node.get(child).copied())
            .collect();
        let _ = self.taffy.set_children(node_id, &taffy_children);
    }

    /// Computes layout for the subtree rooted at `entity`, against the current
    /// [`UiContext::window_size`] as available space.
    pub(crate) fn compute_root_layout(&mut self, entity: u32) {
        let Some(&node_id) = self.entity_to_node.get(&entity) else {
            return;
        };
        let available_space = TaffySize {
            width: AvailableSpace::Definite(self.window_size.x),
            height: AvailableSpace::Definite(self.window_size.y),
        };
        let _ = self.taffy.compute_layout(node_id, available_space);
    }

    /// The last computed `(size, position)` of `entity`, in pixels.
    ///
    /// The position is **parent-relative**, straight out of the layout
    /// algorithm. The `Node` component written by the layout system holds the
    /// *absolute* window position instead; that is the one hit-testing uses.
    pub fn relative_layout(&self, entity: u32) -> Option<(Vec2, Vec2)> {
        let &node_id = self.entity_to_node.get(&entity)?;
        let layout = self.taffy.layout(node_id).ok()?;
        Some((
            Vec2::new(layout.size.width, layout.size.height),
            Vec2::new(layout.location.x, layout.location.y),
        ))
    }

    /// Whether `entity` currently has a layout node.
    pub fn is_tracked(&self, entity: u32) -> bool {
        self.entity_to_node.contains_key(&entity)
    }

    /// How many entities currently have a layout node.
    pub fn tracked_count(&self) -> usize {
        self.entity_to_node.len()
    }

    /// How many nodes the layout tree holds — including any not reachable from
    /// an entity. Should track [`Self::tracked_count`]; a divergence means the
    /// tree is leaking nodes.
    pub fn node_count(&self) -> usize {
        self.taffy.total_node_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taffy::style::CompactLength;

    /// The two `unsafe impl`s above are still load-bearing, and this is the only
    /// place left that says so.
    ///
    /// `Style`'s pair went away with the POD rewrite, but `UiContext` owns a
    /// `TaffyTree` and stays structurally `!Send + !Sync`; it is inserted with
    /// `World::insert_resource`, whose bound is `T: Send + Sync + 'static`, so
    /// deleting either impl breaks the build. The predecessor of this assertion
    /// lived in `components.rs` as `style_and_ui_context_are_send_and_sync`; the
    /// rewrite narrowed that test to `Style` and dropped the `UiContext` half,
    /// which left the remaining `unsafe` with nothing asserting it in-file.
    #[test]
    fn ui_context_is_send_and_sync_via_the_unsafe_impls() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<UiContext>();
    }

    #[test]
    fn fresh_context_is_empty_with_documented_fallback_size() {
        // A brand-new context must carry no nodes and no entity mapping — the
        // layout system relies on this to (re)build the taffy tree from scratch.
        let ctx = UiContext::new();
        assert_eq!(ctx.tracked_count(), 0, "no entity mapping yet");
        assert_eq!(ctx.node_count(), 0, "no taffy nodes yet");
        // The 1280x720 fallback is load-bearing: it is the available space used
        // for root layout until a real WindowInfo resize updates window_size.
        assert_eq!(ctx.window_size, Vec2::new(1280.0, 720.0));
    }

    #[test]
    fn default_matches_new() {
        // `Default` is documented as delegating to `new`; the observable empty
        // state and fallback size must be identical.
        let d = UiContext::default();
        let n = UiContext::new();
        assert_eq!(d.window_size, n.window_size);
        assert_eq!(d.tracked_count(), n.tracked_count());
        assert_eq!(d.node_count(), n.node_count());
    }

    /// The default POD style must convert to taffy's own default style, field
    /// for field. This is the cheapest possible guard against the conversion
    /// silently changing a default that the component does not set explicitly —
    /// `flex_shrink`, `inset: auto`, `margin: zero`, `box_sizing`, and every
    /// unmodelled field left to `..TaffyStyle::DEFAULT`.
    #[test]
    fn default_pod_style_converts_to_taffy_default() {
        assert_eq!(UiContext::to_taffy_style(&Style::DEFAULT), TaffyStyle::DEFAULT);
    }

    /// Pins the auto / percent / px distinction across all three taffy length
    /// types, including the two places the mapping is *not* the identity:
    /// `Val::Percent` divides by 100 (CSS scale → taffy fraction), and
    /// `Val::Auto` collapses to zero for padding/border/gap, which have no
    /// `auto` in CSS.
    #[test]
    fn val_auto_percent_and_px_map_to_distinct_taffy_lengths() {
        // Dimension (sizes, flex_basis)
        assert_eq!(to_dimension(Val::Auto), Dimension::auto());
        assert_eq!(to_dimension(Val::Px(120.0)), Dimension::length(120.0));
        assert_eq!(to_dimension(Val::Percent(50.0)), Dimension::percent(0.5));
        assert_eq!(to_dimension(Val::Percent(100.0)), Dimension::percent(1.0));
        // The three are genuinely different values, not the same thing spelled
        // three ways — a conversion that mapped everything to `auto` would pass
        // the first assertion alone.
        assert_ne!(to_dimension(Val::Auto), to_dimension(Val::Px(0.0)));
        assert_ne!(to_dimension(Val::Px(50.0)), to_dimension(Val::Percent(50.0)));

        // LengthPercentageAuto (inset, margin) keeps `auto`.
        assert_eq!(to_length_percentage_auto(Val::Auto), LengthPercentageAuto::auto());
        assert_eq!(to_length_percentage_auto(Val::Px(8.0)), LengthPercentageAuto::length(8.0));
        assert_eq!(
            to_length_percentage_auto(Val::Percent(25.0)),
            LengthPercentageAuto::percent(0.25)
        );

        // LengthPercentage (padding, border, gap) has no `auto`: it becomes zero.
        assert_eq!(to_length_percentage(Val::Auto), LengthPercentage::length(0.0));
        assert_eq!(to_length_percentage(Val::Px(8.0)), LengthPercentage::length(8.0));
        assert_eq!(to_length_percentage(Val::Percent(25.0)), LengthPercentage::percent(0.25));
    }

    /// The soundness argument for the two `unsafe impl`s above is that no
    /// `CompactLength` in this tree can ever be the pointer-tagged `calc()`
    /// variant. That used to rest on taffy's `calc` Cargo feature being off —
    /// unobservable from here, and re-enabled by any other crate in the graph.
    /// Now it rests on this conversion, so assert it directly: every length the
    /// conversion can emit carries a plain length / percent / auto tag.
    ///
    /// (This replaces the old `calc_invariant_is_documented_not_enforced` test
    /// in `components.rs`, which could only restate the caveat because the
    /// component handed taffy a style it did not build.)
    #[test]
    fn no_length_conversion_produces_a_calc_tagged_value() {
        let plain = [
            CompactLength::LENGTH_TAG,
            CompactLength::PERCENT_TAG,
            CompactLength::AUTO_TAG,
        ];
        for val in [Val::Auto, Val::Px(0.0), Val::Px(-3.5), Val::Percent(0.0), Val::Percent(100.0)]
        {
            assert!(
                plain.contains(&to_dimension(val).into_raw().tag()),
                "Dimension from {val:?} is not a plain length"
            );
            assert!(
                plain.contains(&to_length_percentage_auto(val).into_raw().tag()),
                "LengthPercentageAuto from {val:?} is not a plain length"
            );
            assert!(
                plain.contains(&to_length_percentage(val).into_raw().tag()),
                "LengthPercentage from {val:?} is not a plain length"
            );
        }
    }

    /// The POD style must drive taffy to exactly the layout the equivalent
    /// hand-written `taffy::Style` produces. Builds the same three-node tree
    /// twice — once through `UiContext` from POD styles, once directly from
    /// taffy styles — and compares every computed box.
    ///
    /// Exercises sizing (px + percent), padding, and the flex settings
    /// (`flex_direction`, `flex_grow`, `column_gap`), i.e. the fields most
    /// likely to be mis-wired in the conversion.
    #[test]
    fn pod_style_computes_the_same_layout_as_the_equivalent_taffy_style() {
        let pod_parent = Style {
            width: Val::Px(400.0),
            height: Val::Px(200.0),
            padding: UiRect::new(Val::Px(10.0), Val::Px(5.0), Val::Px(20.0), Val::Px(0.0)),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(12.0),
            ..Default::default()
        };
        let pod_fixed = Style {
            width: Val::Px(80.0),
            height: Val::Percent(50.0),
            ..Default::default()
        };
        let pod_grower = Style { flex_grow: 1.0, height: Val::Px(30.0), ..Default::default() };

        let taffy_parent = TaffyStyle {
            size: TaffySize { width: Dimension::length(400.0), height: Dimension::length(200.0) },
            padding: TaffyRect {
                left: LengthPercentage::length(10.0),
                right: LengthPercentage::length(5.0),
                top: LengthPercentage::length(20.0),
                bottom: LengthPercentage::length(0.0),
            },
            flex_direction: TaffyFlexDirection::Row,
            gap: TaffySize {
                width: LengthPercentage::length(12.0),
                height: LengthPercentage::length(0.0),
            },
            ..TaffyStyle::DEFAULT
        };
        let taffy_fixed = TaffyStyle {
            size: TaffySize { width: Dimension::length(80.0), height: Dimension::percent(0.5) },
            ..TaffyStyle::DEFAULT
        };
        let taffy_grower = TaffyStyle {
            flex_grow: 1.0,
            size: TaffySize { width: Dimension::auto(), height: Dimension::length(30.0) },
            ..TaffyStyle::DEFAULT
        };

        // Path A: the production path — POD styles through UiContext.
        let mut ctx = UiContext::new();
        ctx.window_size = Vec2::new(1024.0, 768.0);
        ctx.sync_style(1, &pod_parent);
        ctx.sync_style(2, &pod_fixed);
        ctx.sync_style(3, &pod_grower);
        ctx.set_children(1, &[2, 3]);
        ctx.compute_root_layout(1);

        // Path B: a raw taffy tree built from the hand-written styles.
        let mut raw: TaffyTree = TaffyTree::new();
        let raw_fixed = raw.new_leaf(taffy_fixed).unwrap();
        let raw_grower = raw.new_leaf(taffy_grower).unwrap();
        let raw_parent = raw.new_with_children(taffy_parent, &[raw_fixed, raw_grower]).unwrap();
        raw.compute_layout(
            raw_parent,
            TaffySize {
                width: AvailableSpace::Definite(1024.0),
                height: AvailableSpace::Definite(768.0),
            },
        )
        .unwrap();

        for (entity, raw_node, name) in
            [(1u32, raw_parent, "parent"), (2, raw_fixed, "fixed"), (3, raw_grower, "grower")]
        {
            let (size, pos) = ctx.relative_layout(entity).expect("laid out");
            let expected = raw.layout(raw_node).unwrap();
            assert_eq!(
                (size.x, size.y),
                (expected.size.width, expected.size.height),
                "{name} size"
            );
            assert_eq!(
                (pos.x, pos.y),
                (expected.location.x, expected.location.y),
                "{name} position"
            );
        }

        // Guard against the whole assertion being vacuous: the flex settings must
        // actually have done something. `grower` fills the leftover main-axis
        // space, so it is wider than nothing and the two children do not overlap.
        let (fixed_size, fixed_pos) = ctx.relative_layout(2).unwrap();
        let (grower_size, grower_pos) = ctx.relative_layout(3).unwrap();
        // 80px wide; the height is 50% of the parent's *content* box, which is
        // 200px minus the 20px top padding = 180px, so 90px.
        assert_eq!(fixed_size, Vec2::new(80.0, 90.0));
        assert_eq!(fixed_pos, Vec2::new(10.0, 20.0), "pushed in by the parent's left/top padding");
        assert!(grower_size.x > 0.0, "flex_grow child must take the leftover width");
        assert_eq!(
            grower_pos.x,
            fixed_pos.x + fixed_size.x + 12.0,
            "column_gap must sit between the two children"
        );
    }

    /// Node lifecycle at the `UiContext` level: styles create nodes, dropping an
    /// entity from the keep-set frees both the mapping and the taffy node.
    #[test]
    fn retain_entities_frees_dropped_nodes() {
        let mut ctx = UiContext::new();
        ctx.sync_style(7, &Style::default());
        ctx.sync_style(8, &Style::default());
        assert!(ctx.is_tracked(7) && ctx.is_tracked(8));
        assert_eq!(ctx.tracked_count(), 2);
        assert_eq!(ctx.node_count(), 2);

        let keep: HashSet<u32> = [7].into_iter().collect();
        ctx.retain_entities(&keep);
        assert!(ctx.is_tracked(7));
        assert!(!ctx.is_tracked(8));
        assert_eq!(ctx.tracked_count(), 1);
        assert_eq!(ctx.node_count(), 1, "the dropped entity's taffy node must be freed");
    }
}
