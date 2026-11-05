//! The world-reading half of text, shared by both draw paths — and the bridge from `gizmo-ui`.
//!
//! # Why it is in the facade
//!
//! The same reason as [`super::decals::collect_decals`]: it needs both halves and they live in
//! different crates. [`TextRenderer`](gizmo_renderer::text::TextRenderer) and
//! [`Text`](gizmo_renderer::components::Text) are in `gizmo-renderer`, and `GlobalTransform` —
//! which is where a world label *is* — is in `gizmo-physics-core`, which the renderer does not
//! depend on. This is the lowest crate that can see both.
//!
//! `gizmo-ui` makes that sharper rather than looser. It sits **above** `gizmo-app`, so the renderer
//! cannot see a `Node` and never will; the facade is the only place that sees a laid-out UI box and
//! a glyph atlas at once. So the bridge is here, and it is small: `Node` publishes absolute
//! window-pixel rects and `TextSpace::Screen` takes absolute window pixels, so connecting them is
//! an anchor and a colour rather than a layout engine.
//!
//! Being `pub` is the point. `gizmo-studio` calls this rather than keeping a second copy, so the
//! two paths cannot drift about where a label sits — which is exactly the divergence
//! `gizmo-studio/tests/render_parity.rs` exists to catch, and exactly what happened to decals
//! before that test was written.

use gizmo_math::{Vec2, Vec3, Vec4};

use crate::core::World;
use crate::renderer::components::Text;
use crate::renderer::Renderer;

/// One laid-out UI box: `(entity, top-left, size, background)`.
///
/// Deliberately carries no `gizmo-ui` type. Everything past [`ui_boxes`] is the same code whether
/// the `ui` feature is on or off, which is what keeps the feature from forking the draw loop.
type UiBox = (u32, Vec2, Vec2, Option<Vec4>);

/// Every entity `gizmo-ui` has laid out this frame — and, right after a [`Slider`](crate::ui::Slider)'s
/// own box, a second box for its fill: from the left edge to `value` of the width. Same entity, so
/// same layer and clip; queued after the background, so over it; and a background, so under every
/// label of the layer.
///
/// Empty without the feature — and empty is the honest answer rather than a `cfg` around the loop
/// below: a build with no UI crate has no UI boxes, which is the same thing a build with an empty
/// UI tree has.
#[cfg(feature = "ui")]
fn ui_boxes(world: &World) -> Vec<UiBox> {
    let nodes = world.borrow::<crate::ui::Node>();
    let backgrounds = world.borrow::<crate::ui::BackgroundColor>();
    let sliders = world.borrow::<crate::ui::Slider>();
    let sticks = world.borrow::<crate::ui::VirtualStick>();
    let bars = world.borrow::<crate::ui::ProgressBar>();
    let toggles = world.borrow::<crate::ui::Toggle>();
    let scrolls = world.borrow::<crate::ui::Scroll>();
    let mut out = Vec::new();
    for id in tree_order(world) {
        let Some(node) = nodes.get(id) else { continue };
        let start = out.len();
        out.push((id, node.position, node.size, backgrounds.get(id).map(|c| c.0)));
        if let Some(s) = sliders.get(id) {
            let w = node.size.x * s.value.clamp(0.0, 1.0);
            out.push((id, node.position, Vec2::new(w, node.size.y), Some(s.fill)));
        }
        // A scrolling list's thumb, while something overflows: brighter while it is held.
        if let Some(s) = scrolls.get(id) {
            if let Some((at, size)) = s.thumb(node) {
                let alpha = if s.is_dragging_thumb() { 0.7 } else { 0.4 };
                out.push((id, at, size, Some(Vec4::new(1.0, 1.0, 1.0, alpha))));
            }
        }
        if let Some(b) = bars.get(id) {
            let w = node.size.x * b.value.clamp(0.0, 1.0);
            out.push((id, node.position, Vec2::new(w, node.size.y), Some(b.fill)));
        }
        // A toggle's check box at the left edge: a frame, and a filled square inside while on.
        if let Some(t) = toggles.get(id) {
            let side = node.size.y * 0.5;
            let at = node.position + Vec2::new(node.size.y * 0.25, node.size.y * 0.25);
            out.push((id, at, Vec2::splat(side), Some(Vec4::new(1.0, 1.0, 1.0, 0.35))));
            if t.on {
                let inner = side * 0.6;
                out.push((id, at + Vec2::splat((side - inner) * 0.5), Vec2::splat(inner), Some(Vec4::new(0.35, 0.75, 1.0, 1.0))));
            }
        }
        // A stick's knob: at rest in the middle of its box; held, where the finger came down plus
        // the push (screen `y` is down, the stick's is up).
        if let Some(s) = sticks.get(id) {
            let side = node.size.x.min(node.size.y) * 0.35;
            let rest = if s.is_held() { s.centre() } else { node.position + node.size * 0.5 };
            let at = rest + Vec2::new(s.value().x, -s.value().y) * s.radius * display_density(world);
            out.push((id, at - Vec2::splat(side * 0.5), Vec2::splat(side), Some(s.knob)));
        }
        // A fading box fades all of its parts.
        let alpha = ui_opacity(world, id);
        if alpha < 1.0 {
            for b in &mut out[start..] {
                if let Some(c) = b.3.as_mut() {
                    c.w *= alpha;
                }
            }
        }
    }
    out
}

/// The opacity `entity` is drawn at: the product of every [`UiTransform`](crate::ui::UiTransform)
/// `opacity` from it up through its parents — a card fades with its face and its label.
#[cfg(feature = "ui")]
fn ui_opacity(world: &World, entity: u32) -> f32 {
    let transforms = world.borrow::<crate::ui::UiTransform>();
    if transforms.iter().next().is_none() {
        return 1.0;
    }
    let parents = world.borrow::<crate::core::component::Parent>();
    let mut alpha = 1.0;
    let mut at = Some(entity);
    // Bounded: a hierarchy cycle (which the core prevents) must not hang a frame.
    for _ in 0..256 {
        let Some(id) = at else { break };
        if let Some(t) = transforms.get(id) {
            alpha *= t.opacity.clamp(0.0, 1.0);
        }
        at = parents.get(id).map(|p| p.0);
    }
    alpha
}

#[cfg(not(feature = "ui"))]
fn ui_opacity(_world: &World, _entity: u32) -> f32 {
    1.0
}

/// Every UI node in painting order: a parent before its children, earlier siblings (in
/// `Children` order) before later ones and everything under them — depth first, as a page is
/// painted. The renderer keeps the queue order within a layer, so this is what puts an item over
/// the slot it sits in.
///
/// It used to be the node storage's own order, which is by *archetype*: a slot (`NodeBundle` +
/// a drop target) and the item inside it (`ButtonBundle` + a drag) live in different tables, and
/// whichever table came first was painted first — in `birlestir`, every slot over its own item.
#[cfg(feature = "ui")]
fn tree_order(world: &World) -> Vec<u32> {
    let nodes = world.borrow::<crate::ui::Node>();
    let parents = world.borrow::<crate::core::component::Parent>();
    let children = world.borrow::<crate::core::component::Children>();
    let parent_node = |id: u32| parents.get(id).map(|p| p.0).filter(|p| nodes.get(*p).is_some());
    // A node whose parent is not a node starts a tree of its own. Ids order the trees: spawned
    // earlier, painted earlier.
    let mut roots: Vec<u32> = nodes.iter().map(|(id, _)| id).filter(|&id| parent_node(id).is_none()).collect();
    roots.sort_unstable();
    let total = nodes.iter().count();
    let mut order = Vec::with_capacity(total);
    let mut stack: Vec<u32> = roots.into_iter().rev().collect();
    // Bounded: a `Children` list written by hand can name a node twice or form a loop.
    while let Some(id) = stack.pop() {
        if order.len() >= total {
            break;
        }
        order.push(id);
        if let Some(c) = children.get(id) {
            stack.extend(c.0.iter().rev().copied().filter(|&k| parent_node(k) == Some(id)));
        }
    }
    order
}

/// The [`UiScale`](crate::ui::UiScale) factor for labels on UI boxes, or `None` when it is `1`
/// (no resource, or a factor of one) — the common case, which then clones nothing.
#[cfg(feature = "ui")]
fn ui_text_scale(world: &World) -> Option<f32> {
    let density = display_density(world);
    let k = world.get_resource::<crate::ui::UiScale>().map_or(density, |s| s.on_display(density));
    (k != 1.0).then_some(k)
}

#[cfg(not(feature = "ui"))]
fn ui_text_scale(_world: &World) -> Option<f32> {
    None
}

/// Frame pixels per logical pixel (`DisplayScale`, 1 without one).
fn display_density(world: &World) -> f32 {
    world.get_resource::<crate::core::window::DisplayScale>().map_or(1.0, |d| d.get())
}

/// What a text's font is multiplied by: a label on a UI box grows with the layout, a HUD line with
/// the display, and a world label has its own `px_per_unit`.
fn text_scale(world: &World, on_a_box: bool, in_the_world: bool) -> Option<f32> {
    if in_the_world {
        None
    } else if on_a_box {
        ui_text_scale(world)
    } else {
        Some(display_density(world)).filter(|&k| k != 1.0)
    }
}

/// `item` at `k` times its size: the font, and the width it wraps at.
fn scale_text(item: &Text, k: f32) -> Text {
    let mut t = item.clone();
    t.size_px *= k;
    t.max_width = t.max_width.map(|w| w * k);
    t.space = t.space.with_scaled_margin(k);
    t
}

#[cfg(not(feature = "ui"))]
fn ui_boxes(_world: &World) -> Vec<UiBox> {
    Vec::new()
}

/// Where `entity`'s drawing is cut: the intersection of the boxes of every
/// [`ClipContent`](crate::ui::ClipContent) or [`Scroll`](crate::ui::Scroll) node from it up
/// through its ancestors (`[min, max]`,
/// window pixels), or `None` when none of them clips. An empty intersection is a zero-size rect,
/// which clips everything.
#[cfg(feature = "ui")]
pub(crate) fn ui_clip(world: &World, entity: u32) -> Option<[Vec2; 2]> {
    let clips = world.borrow::<crate::ui::ClipContent>();
    let scrolls = world.borrow::<crate::ui::Scroll>();
    let nodes = world.borrow::<crate::ui::Node>();
    let parents = world.borrow::<crate::core::component::Parent>();
    let mut out: Option<[Vec2; 2]> = None;
    let mut at = Some(entity);
    // Bounded: a hierarchy cycle (which the core prevents) must not hang a frame.
    for _ in 0..256 {
        let Some(id) = at else { break };
        if clips.get(id).is_some() || scrolls.get(id).is_some() {
            if let Some(n) = nodes.get(id) {
                let r = [n.position, n.position + n.size];
                out = Some(match out {
                    Some([lo, hi]) => [lo.max(r[0]), hi.min(r[1])],
                    None => r,
                });
            }
        }
        at = parents.get(id).map(|p| p.0);
    }
    out
}

#[cfg(not(feature = "ui"))]
pub(crate) fn ui_clip(_world: &World, _entity: u32) -> Option<[Vec2; 2]> {
    None
}

/// What a [`TextInput`](crate::ui::TextInput) on `entity` shows instead of its `Text`'s content —
/// the value (with a caret while focused) — or `None` to draw the `Text` as authored (its
/// placeholder).
#[cfg(feature = "ui")]
fn text_input_display(world: &World, entity: u32) -> Option<String> {
    world.borrow::<crate::ui::TextInput>().get(entity).and_then(|f| f.display())
}

#[cfg(not(feature = "ui"))]
fn text_input_display(_world: &World, _entity: u32) -> Option<String> {
    None
}

/// A `Text` shown a character at a time — dialogue, a tutorial line, a typed-out intro. Put it
/// beside a `Text`: the label draws the first [`visible`](Self::visible) characters of what it
/// would otherwise show (its content, a `LocalizedText`'s string, a field's value), and a new
/// string starts typing from the beginning.
///
/// Advanced by both draw paths on the animation clock, so a paused game pauses it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Typewriter {
    /// How fast it types.
    pub chars_per_second: f32,
    shown: f32,
    total: usize,
    seen: u64,
    finished: bool,
    just_finished: bool,
}

impl Typewriter {
    /// Typing `chars_per_second` characters a second, from the beginning.
    #[must_use]
    pub fn new(chars_per_second: f32) -> Self {
        Self { chars_per_second, shown: 0.0, total: 0, seen: 0, finished: false, just_finished: false }
    }

    /// How many characters it shows now.
    #[must_use]
    pub fn visible(&self) -> usize {
        if self.finished {
            self.total
        } else {
            (self.shown as usize).min(self.total)
        }
    }

    /// Shows the rest at once — the "press to skip" of every dialogue box.
    pub fn skip(&mut self) {
        self.shown = self.total as f32;
    }

    /// Types the same string again from the start.
    pub fn restart(&mut self) {
        self.shown = 0.0;
        self.finished = false;
    }

    /// Everything is shown.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// It finished this frame — typed its last character, or was skipped.
    #[must_use]
    pub fn just_finished(&self) -> bool {
        self.just_finished
    }

    /// One frame: `text` is what the label would show, `dt` the animation step.
    fn step(&mut self, text: &str, dt: f32) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        let seen = h.finish();
        if seen != self.seen {
            // A new line of dialogue: from the top.
            self.seen = seen;
            self.total = text.chars().count();
            self.restart();
        }
        self.just_finished = false;
        if self.finished {
            return;
        }
        if dt > 0.0 && self.chars_per_second > 0.0 {
            self.shown += self.chars_per_second * dt;
        }
        if self.shown >= self.total as f32 {
            self.finished = true;
            self.just_finished = true;
        }
    }
}

gizmo_core::impl_component!(Typewriter);

/// Advances every [`Typewriter`] by `dt` against what its label would show. Both draw paths call
/// it, on the animation clock.
pub fn typewriter_system(world: &mut World, dt: f32) {
    let lines: Vec<(u32, String)> = {
        let typers = world.borrow::<Typewriter>();
        let texts = world.borrow::<Text>();
        typers
            .iter()
            .filter_map(|(id, _)| {
                let t = texts.get(id)?;
                let s = text_input_display(world, id)
                    .or_else(|| dropdown_display(world, id))
                    .or_else(|| localized_display(world, id))
                    .unwrap_or_else(|| t.content.clone());
                Some((id, s))
            })
            .collect()
    };
    let mut typers = world.borrow_mut::<Typewriter>();
    for (id, line) in lines {
        if let Some(mut t) = typers.get_mut(id) {
            let mut next = *t;
            next.step(&line, dt);
            if next != *t {
                *t = next;
            }
        }
    }
}

/// `full` cut to what a [`Typewriter`] on `entity` has typed so far, or `None` without one.
fn typed_display(world: &World, entity: u32, full: &str) -> Option<String> {
    let t = *world.borrow::<Typewriter>().get(entity)?;
    Some(full.chars().take(t.visible()).collect())
}

/// What a [`LocalizedText`](crate::app::locale::LocalizedText) on `entity` shows in the world's
/// [`Locale`](crate::app::locale::Locale) — or `None` (no label, or no locale) to draw the `Text`
/// as authored. Resolved at draw time, so a language switch relabels every one on the next frame.
fn localized_display(world: &World, entity: u32) -> Option<String> {
    let label = world.borrow::<crate::app::locale::LocalizedText>().get(entity).cloned()?;
    let locale = world.get_resource::<crate::app::locale::Locale>()?;
    Some(label.resolve(&locale))
}

/// What a [`Dropdown`](crate::ui::Dropdown) on `entity` shows on its button: the selected option
/// and a down-arrow — or `None` without one.
#[cfg(feature = "ui")]
fn dropdown_display(world: &World, entity: u32) -> Option<String> {
    let d = world.borrow::<crate::ui::Dropdown>().get(entity).cloned()?;
    Some(format!("{}  ▾", d.selected_option().unwrap_or("")))
}

#[cfg(not(feature = "ui"))]
fn dropdown_display(_world: &World, _entity: u32) -> Option<String> {
    None
}

/// Every open dropdown's rows: `(entity, [(top-left, size, colour, label)])`.
#[cfg(feature = "ui")]
fn dropdown_lists(world: &World) -> Vec<(u32, Vec<(Vec2, Vec2, Vec4, String)>)> {
    let nodes = world.borrow::<crate::ui::Node>();
    world
        .borrow::<crate::ui::Dropdown>()
        .iter()
        .filter(|(_, d)| d.is_open())
        .filter_map(|(id, d)| {
            let node = nodes.get(id)?;
            let rows = d
                .options
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    let (at, size) = d.option_rect(node, i);
                    let colour = if d.hovered() == Some(i) {
                        Vec4::new(0.32, 0.38, 0.5, 0.98)
                    } else if i == d.selected {
                        Vec4::new(0.2, 0.32, 0.52, 0.98)
                    } else {
                        Vec4::new(0.12, 0.13, 0.17, 0.98)
                    };
                    (at, size, colour, label.clone())
                })
                .collect();
            Some((id, rows))
        })
        .collect()
}

/// Every tooltip that is up: its element, the pointer, what it says.
#[cfg(feature = "ui")]
fn tooltips(world: &World) -> Vec<(u32, Vec2, String)> {
    world
        .borrow::<crate::ui::Tooltip>()
        .iter()
        .filter_map(|(id, t)| t.shown_at().map(|p| (id, p, t.text.clone())))
        .collect()
}

#[cfg(not(feature = "ui"))]
fn tooltips(_world: &World) -> Vec<(u32, Vec2, String)> {
    Vec::new()
}

/// Where a tooltip's box goes: below and right of the pointer (clear of the cursor's arrow), or
/// flipped to the other side of it where that would leave the window, and never off its top-left.
/// `text` is the laid-out text's size, `pad` the margin around it.
fn tooltip_box(pointer: Vec2, text: Vec2, pad: f32, screen: Vec2) -> (Vec2, Vec2) {
    let size = text + Vec2::splat(2.0 * pad);
    let mut at = pointer + Vec2::new(14.0, 20.0);
    if at.x + size.x > screen.x {
        at.x = pointer.x - 6.0 - size.x;
    }
    if at.y + size.y > screen.y {
        at.y = pointer.y - 6.0 - size.y;
    }
    (at.max(Vec2::ZERO), size)
}

#[cfg(not(feature = "ui"))]
fn dropdown_lists(_world: &World) -> Vec<(u32, Vec<(Vec2, Vec2, Vec4, String)>)> {
    Vec::new()
}

/// The overlay layer `entity` is drawn on — its `ZIndex`, inherited through its parents.
#[cfg(feature = "ui")]
fn ui_layer(world: &World, entity: u32) -> i32 {
    crate::ui::ui_layer(world, entity)
}

#[cfg(not(feature = "ui"))]
fn ui_layer(_world: &World, _entity: u32) -> i32 {
    0
}

/// Lays this frame's [`Text`] out, rasterises any glyph the atlas has not seen, and draws it —
/// with the UI backgrounds underneath.
///
/// Runs **after** TAA, into the HDR target and against the main depth buffer — the same slot the
/// debug gizmos use, and for the same reason: a temporal resolve smears an overlay that changes
/// every frame, and text is the most legible thing in a frame and so the most obviously smeared.
///
/// World labels are drawn here and depth-tested (a label behind a wall is behind the wall).
/// Screen text and UI backgrounds are only laid out and uploaded here: `Renderer::
/// run_post_processing` draws them onto the final image after tone mapping, so they are neither
/// exposed nor bloomed with the scene. A `Text` with no `GlobalTransform` still draws if it is screen-space — a
/// HUD element has no place in the world — and is skipped if it is not.
///
/// # What a `Node` on the same entity does
///
/// It **positions the text**: a screen-space `Text` beside a `gizmo-ui` [`Node`](crate::ui::Node)
/// is placed at the box's own corner, chosen by the text's anchor — `TopLeft` at the box's
/// top-left, `Center` in the middle of it — and the position the component itself carries is
/// ignored. That is the only thing putting both on one entity can sensibly mean, and it is what
/// makes a laid-out button able to have a label. A `Node` does **not** clip the text and does not
/// resize it: a string longer than its box overflows it, because `gizmo-ui` has no clipping.
/// Nor does it wrap the text to its width on its own: `gizmo-ui` never measures text, so a box
/// sized by layout can be narrower than any word, and wrapping to it would cut every label into
/// single characters. Give the `Text` a [`max_width`](Text::max_width) — the box's width, if that
/// is what you want — and it wraps there.
///
/// [`BackgroundColor`](crate::ui::BackgroundColor) is drawn as a solid quad over the box, under
/// every glyph in the frame. It had been written and read by nothing since the crate existed.
/// Overlapping boxes are painted by [`ZIndex`](crate::ui::ZIndex) layer, and within a layer in
/// tree order — a parent under its children, an earlier sibling under a later one.
pub fn record_text(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut Renderer,
    world: &World,
) {
    let texts = world.borrow::<Text>();
    let boxes = ui_boxes(world);
    let fade = world
        .get_resource::<crate::renderer::screen_fade::ScreenFade>()
        .and_then(|f| f.overlay());
    if texts.iter().next().is_none() && boxes.iter().all(|(_, _, _, bg)| bg.is_none()) && fade.is_none() {
        // Nothing to draw — but the overlay draws from what was last uploaded, so forget it, or
        // a HUD removed this frame would stay on screen.
        if let Some(text) = renderer.text.as_mut() {
            text.begin_frame();
        }
        return;
    }
    let transforms = world.borrow::<crate::prelude::GlobalTransform>();
    let screen = Vec2::new(renderer.config.width as f32, renderer.config.height as f32);

    // Split the borrow by hand: queueing needs `&mut` on the text renderer and `&` on the queue,
    // and both live on `Renderer`.
    let Renderer { text, queue, device, scene, post, depth_texture_view, .. } = renderer;
    let Some(text) = text.as_mut() else {
        return;
    };

    text.begin_frame();
    // Backgrounds first in the source too, though the renderer keeps them under the glyphs
    // whatever order they arrive in — `TextRenderer::queue_rect` says so.
    for (id, position, size, background) in &boxes {
        if let Some(color) = background {
            text.set_layer(ui_layer(world, *id));
            text.queue_rect_clipped(*position, *size, *color, screen, ui_clip(world, *id));
        }
    }

    for (id, item) in texts.iter() {
        // `TextSpace::needs_world_position` rather than a `match` here: the enum is
        // `#[non_exhaustive]`, so this file could only match it with a wildcard, and a wildcard is
        // where a third space would quietly become a HUD element. The decision lives beside the
        // enum, where it is a compile error instead.
        let (origin, anchor) = if item.space.needs_world_position() {
            let Some(t) = transforms.get(id) else {
                // A world label with no world position. Skipping is the honest answer: at the
                // origin it would sit somewhere plausible and wrong, which is harder to diagnose
                // than not being there.
                continue;
            };
            (t.matrix.w_axis.truncate(), None)
        } else {
            // A UI box on the same entity replaces the authored position. `factors()` is what
            // makes the text's own anchor pick the box's corner, so `Center` centres it in the
            // button rather than centring it on the button's top-left.
            let anchor = boxes
                .iter()
                .find(|(e, ..)| *e == id)
                .map(|(_, position, size, _)| *position + *size * item.anchor.factors());
            (Vec3::ZERO, anchor)
        };
        // A clipping UI box above it (or on it) cuts screen text to the box.
        let clip = if item.space.needs_world_position() { None } else { ui_clip(world, id) };
        text.set_layer(ui_layer(world, id));
        // A text field draws its value where its `Text` holds the placeholder; otherwise a
        // `LocalizedText` draws the current language's string (the placeholder too).
        // A `Typewriter` then shows only as much of that as it has typed.
        let field;
        let shown = text_input_display(world, id)
            .or_else(|| dropdown_display(world, id))
            .or_else(|| localized_display(world, id));
        let shown = typed_display(world, id, shown.as_deref().unwrap_or(&item.content)).or(shown);
        let item = match shown {
            Some(shown) => {
                field = {
                    let mut t = item.clone();
                    t.content = shown;
                    t
                };
                &field
            }
            None => item,
        };
        // A label on a UI box grows with the layout's `UiScale`, as the box did, and fades with
        // its `UiTransform`s.
        let scaled;
        let alpha = ui_opacity(world, id);
        let k = text_scale(world, anchor.is_some(), item.space.needs_world_position());
        let item = match (k, alpha < 1.0) {
            (None, false) => item,
            (k, _) => {
                let mut t = scale_text(item, k.unwrap_or(1.0));
                t.color.w *= alpha;
                scaled = t;
                &scaled
            }
        };
        text.queue_clipped(queue, item, origin, anchor, screen, clip);
    }
    // Tooltips above every UI layer, under an open list: light text on a dark box beside the
    // pointer, in the element's own font and size, kept inside the window.
    let k = ui_text_scale(world).unwrap_or(1.0);
    for (id, pointer, content) in tooltips(world) {
        let Some(style) = texts.get(id) else { continue };
        // The element's font and size, nothing else of its style: top-left anchored and
        // left-aligned (`Text::screen`'s own), light, wrapped.
        let item = Text::screen(content, style.font, style.size_px * k, Vec2::ZERO)
            .with_color(Vec4::new(0.95, 0.95, 0.97, 1.0))
            .with_max_width(320.0 * k);
        let Some(l) = text.library().layout_wrapped(item.font, item.size_px, &item.content, item.max_width) else {
            continue;
        };
        let (at, size) = tooltip_box(pointer, Vec2::new(l.width, l.height), 6.0 * k, screen);
        text.set_layer(i32::MAX - 2);
        text.queue_rect_clipped(at, size, Vec4::new(0.04, 0.045, 0.06, 0.94), screen, None);
        text.queue_clipped(queue, &item, Vec3::ZERO, Some(at + Vec2::splat(6.0 * k)), screen, None);
    }
    // Open dropdown lists, above every UI layer (under the screen fade): a row per option, the
    // hovered one lighter and the selected one marked, labelled in the button's own `Text` style.
    for (id, rows) in dropdown_lists(world) {
        let Some(style) = texts.get(id) else { continue };
        text.set_layer(i32::MAX - 1);
        for (at, size, colour, _) in &rows {
            text.queue_rect_clipped(*at, *size, *colour, screen, None);
        }
        for (at, size, _, label) in rows {
            let mut item = ui_text_scale(world).map_or_else(|| style.clone(), |k| scale_text(style, k));
            item.content = label;
            let anchor = at + size * item.anchor.factors();
            text.queue_clipped(queue, &item, Vec3::ZERO, Some(anchor), screen, None);
        }
    }
    // The screen fade over everything: one rect the size of the target on the highest layer.
    if let Some(color) = fade {
        text.set_layer(i32::MAX);
        text.queue_rect_clipped(Vec2::ZERO, screen, color, screen, None);
    }
    text.upload(device, queue);
    // Screen text and UI backgrounds are drawn later, onto the final image, by the overlay in
    // `Renderer::run_post_processing`. This pass is only for world labels.
    if text.queued().0 == 0 {
        return;
    }

    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Text Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &post.hdr_texture_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        // Loaded, not cleared: the world pipeline tests against the scene's own depth, which is
        // what puts a label behind the wall in front of it.
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth_texture_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    text.render(&mut pass, scene.view_bind_group());
}

#[cfg(all(test, feature = "ui"))]
mod clip_tests {
    use super::*;
    use crate::core::hierarchy::HierarchyExt;
    use crate::ui::{ClipContent, Node};

    fn node(x: f32, y: f32, w: f32, h: f32) -> Node {
        Node { position: Vec2::new(x, y), size: Vec2::new(w, h) }
    }

    /// A clipping panel cuts what is under it to its box; a nested one narrows it to the
    /// intersection; a node that does not clip passes its parent's clip down; nothing above
    /// clips, nothing is cut.
    #[test]
    fn clips_are_the_intersection_of_the_clipping_ancestors() {
        let mut world = World::new();
        let panel = world.spawn_bundle((node(0.0, 0.0, 100.0, 100.0), ClipContent));
        let inner = world.spawn_bundle((node(50.0, 50.0, 200.0, 200.0), ClipContent));
        let plain = world.spawn_bundle((node(60.0, 60.0, 10.0, 10.0),));
        let label = world.spawn();
        world.add_child(panel, inner);
        world.add_child(inner, plain);
        world.add_child(plain, label);
        let loose = world.spawn_bundle((node(0.0, 0.0, 10.0, 10.0),));

        assert_eq!(ui_clip(&world, panel.id()), Some([Vec2::ZERO, Vec2::splat(100.0)]));
        assert_eq!(ui_clip(&world, label.id()), Some([Vec2::splat(50.0), Vec2::splat(100.0)]), "the intersection");
        assert_eq!(ui_clip(&world, plain.id()), Some([Vec2::splat(50.0), Vec2::splat(100.0)]));
        assert_eq!(ui_clip(&world, loose.id()), None);
    }

    /// A slider is its box, then its fill: `value` of the width from the left, in its fill colour,
    /// queued after the background so it paints over it — and the label still finds the box.
    #[test]
    fn a_slider_adds_its_fill_after_its_box() {
        let mut world = World::new();
        let fill = Vec4::new(0.0, 1.0, 0.0, 1.0);
        let s = world.spawn_bundle((
            node(10.0, 20.0, 200.0, 30.0),
            crate::ui::BackgroundColor(Vec4::ONE),
            crate::ui::Slider::new(0.25).with_fill(fill),
        ));
        let boxes = ui_boxes(&world);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0], (s.id(), Vec2::new(10.0, 20.0), Vec2::new(200.0, 30.0), Some(Vec4::ONE)));
        assert_eq!(boxes[1], (s.id(), Vec2::new(10.0, 20.0), Vec2::new(50.0, 30.0), Some(fill)));
    }

    /// Within a layer, boxes come out in tree order whatever tables the nodes live in: a parent
    /// before its children, siblings in `Children` order. Storage order put a slot over its item.
    #[test]
    fn ui_boxes_come_out_parent_before_child_whatever_the_archetype() {
        use crate::core::hierarchy::HierarchyExt;
        let mut world = World::new();
        // The item's table is created first, so storage order would yield it before its slot.
        let item = world.spawn_bundle((node(4.0, 4.0, 10.0, 10.0), crate::ui::BackgroundColor(Vec4::ONE), crate::ui::Clicked(false)));
        let slot = world.spawn_bundle((node(0.0, 0.0, 18.0, 18.0), crate::ui::BackgroundColor(Vec4::ZERO)));
        let second = world.spawn_bundle((node(20.0, 0.0, 18.0, 18.0), crate::ui::BackgroundColor(Vec4::ZERO)));
        let other = world.spawn_bundle((node(40.0, 0.0, 18.0, 18.0), crate::ui::BackgroundColor(Vec4::ZERO), crate::ui::Clicked(false)));
        world.add_child(slot, second);
        world.add_child(slot, item);
        let ids: Vec<u32> = ui_boxes(&world).iter().map(|b| b.0).collect();
        assert_eq!(ids, vec![slot.id(), second.id(), item.id(), other.id()]);
    }

    /// A `UiTransform`'s opacity fades a box and everything under it — background, extra parts,
    /// children — multiplied down the tree.
    #[test]
    fn opacity_is_inherited_down_the_tree() {
        use crate::core::hierarchy::HierarchyExt;
        let mut world = World::new();
        let card = world.spawn_bundle((
            node(0.0, 0.0, 50.0, 50.0),
            crate::ui::BackgroundColor(Vec4::new(1.0, 0.0, 0.0, 1.0)),
            crate::ui::UiTransform { translation: Vec2::ZERO, opacity: 0.5 },
        ));
        let face = world.spawn_bundle((
            node(5.0, 5.0, 40.0, 40.0),
            crate::ui::BackgroundColor(Vec4::new(0.0, 1.0, 0.0, 0.8)),
            crate::ui::ProgressBar { value: 1.0, fill: Vec4::ONE },
            crate::ui::UiTransform { translation: Vec2::ZERO, opacity: 0.5 },
        ));
        world.add_child(card, face);
        assert_eq!(ui_opacity(&world, face.id()), 0.25);
        let alphas: Vec<f32> = ui_boxes(&world).iter().map(|b| b.3.unwrap().w).collect();
        assert_eq!(alphas, vec![0.5, 0.2, 0.25]);
    }

    /// A tooltip's box sits below-right of the pointer, flips where it would leave the window, and
    /// never goes off the top-left.
    #[test]
    fn a_tooltip_box_stays_in_the_window() {
        let screen = Vec2::new(800.0, 600.0);
        let text = Vec2::new(100.0, 20.0);
        assert_eq!(tooltip_box(Vec2::new(100.0, 100.0), text, 5.0, screen), (Vec2::new(114.0, 120.0), Vec2::new(110.0, 30.0)));
        let (at, size) = tooltip_box(Vec2::new(780.0, 590.0), text, 5.0, screen);
        assert_eq!(at, Vec2::new(780.0 - 6.0 - 110.0, 590.0 - 6.0 - 30.0));
        assert!(at.x + size.x <= screen.x && at.y + size.y <= screen.y);
        let (at, _) = tooltip_box(Vec2::new(50.0, 5.0), Vec2::new(900.0, 700.0), 5.0, screen);
        assert_eq!(at, Vec2::ZERO, "too big for the window: pinned to its corner");
    }

    /// A HUD line follows the display and a world label does not; a screen position stays where it
    /// is, because it is a place in the frame, while a pinned text's margin grows with its font.
    #[test]
    fn a_hud_line_scales_with_the_display_and_a_world_label_does_not() {
        use crate::core::window::DisplayScale;
        let mut world = World::new();
        assert_eq!(text_scale(&world, false, false), None, "no resource: nothing changes");
        world.insert_resource(DisplayScale::from_window(800.0, 400.0, 1.0));
        assert_eq!(text_scale(&world, false, false), Some(2.0));
        assert_eq!(text_scale(&world, false, true), None);
        let font = crate::renderer::text::FontLibrary::default()
            .load(crate::renderer::text::synthetic::synthetic_face())
            .expect("the synthetic face parses");
        let pinned = scale_text(&Text::pinned("x", font, 22.0, crate::renderer::components::TextAnchor::TopLeft, Vec2::splat(16.0)), 2.0);
        assert_eq!(pinned.size_px, 44.0);
        assert_eq!(pinned.space.screen_point(Vec2::new(800.0, 600.0)), Some(Vec2::splat(32.0)));
        let placed = scale_text(&Text::screen("x", font, 22.0, Vec2::splat(16.0)), 2.0);
        assert_eq!(placed.space.screen_point(Vec2::new(800.0, 600.0)), Some(Vec2::splat(16.0)));
    }

    /// A label on a UI box is drawn at the `UiScale` factor: its font and its wrap width, and
    /// nothing is cloned at a factor of one.
    #[test]
    fn ui_labels_follow_the_ui_scale() {
        let mut world = World::new();
        assert_eq!(ui_text_scale(&world), None);
        world.insert_resource(crate::ui::UiScale::fixed(1.0));
        assert_eq!(ui_text_scale(&world), None);
        world.insert_resource(crate::ui::UiScale::fixed(2.0));
        assert_eq!(ui_text_scale(&world), Some(2.0));
        let font = crate::renderer::text::FontLibrary::default()
            .load(crate::renderer::text::synthetic::synthetic_face())
            .expect("the synthetic face parses");
        let t = Text::screen("x", font, 20.0, Vec2::ZERO).with_max_width(100.0);
        let k = scale_text(&t, 2.0);
        assert_eq!((k.size_px, k.max_width), (40.0, Some(200.0)));
    }

    /// A `LocalizedText` shows the current language's string, follows a language switch, and
    /// steps aside (the `Text` as authored) when there is no `Locale` — and the draw loop asks it.
    #[test]
    fn a_localized_label_follows_the_language() {
        use crate::app::locale::{Locale, LocalizedText};
        let mut world = World::new();
        let label = world.spawn_bundle((LocalizedText::new("menu.score").with_arg("n", 3),)).id();
        assert_eq!(localized_display(&world, label), None, "no locale: the Text as authored");
        let mut locale = Locale::new("tr");
        locale.insert("tr", "menu.score", "Skor: {n}");
        locale.insert("en", "menu.score", "Score: {n}");
        world.insert_resource(locale);
        assert_eq!(localized_display(&world, label).as_deref(), Some("Skor: 3"));
        world.get_resource_mut::<Locale>().unwrap().set_language("en");
        assert_eq!(localized_display(&world, label).as_deref(), Some("Score: 3"));
        let plain = world.spawn().id();
        assert_eq!(localized_display(&world, plain), None);

        let code: String = include_str!("text.rs")
            .split(concat!("mod ", "clip_tests"))
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(
            // `let shown =` is record_text's own line — the typewriter has the same chain, so a
            // looser match would find that one and pass with the draw loop's removed.
            code.contains(concat!("letshown=text_input_display(world,id)", ".or_else(||dropdown_display(world,id))", ".or_else(||localized_display(world,id));")),
            "record_text no longer draws localized labels"
        );
    }

    /// A progress bar is its box and then its fill; a toggle adds its check box, filled only while
    /// on.
    #[test]
    fn a_progress_bar_and_a_toggle_draw_their_parts() {
        let mut world = World::new();
        let bar = world.spawn_bundle((node(0.0, 0.0, 200.0, 20.0), crate::ui::ProgressBar::new(0.75)));
        let boxes = ui_boxes(&world);
        assert_eq!(boxes.len(), 2);
        assert_eq!((boxes[1].0, boxes[1].2), (bar.id(), Vec2::new(150.0, 20.0)));
        let off = world.spawn_bundle((node(0.0, 100.0, 200.0, 40.0), crate::ui::Toggle::new(false)));
        let on = world.spawn_bundle((node(0.0, 200.0, 200.0, 40.0), crate::ui::Toggle::new(true)));
        let boxes = ui_boxes(&world);
        let parts = |id: u32| boxes.iter().filter(|b| b.0 == id).count();
        assert_eq!((parts(off.id()), parts(on.id())), (2, 3), "box + frame, and + mark when on");
    }

    /// A stick's knob sits in the middle of its box at rest — and is a box after the stick's own.
    #[test]
    fn a_virtual_stick_draws_its_knob_in_the_middle_at_rest() {
        let mut world = World::new();
        let s = world.spawn_bundle((node(0.0, 0.0, 200.0, 100.0), crate::ui::VirtualStick::new(40.0)));
        let boxes = ui_boxes(&world);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[1].0, s.id());
        assert_eq!(boxes[1].2, Vec2::splat(35.0));
        assert_eq!(boxes[1].1 + boxes[1].2 * 0.5, Vec2::new(100.0, 50.0));
    }

    /// A typewriter shows its label a character at a time, finishes once, skips, and starts a new
    /// string from the top — and the draw loop cuts the label to it.
    #[test]
    fn a_typewriter_types_finishes_and_restarts_on_a_new_line() {
        let mut world = World::new();
        let font = crate::renderer::text::FontLibrary::default()
            .load(crate::renderer::text::synthetic::synthetic_face())
            .expect("the synthetic face parses");
        let e = world
            .spawn_bundle((Text::screen("Merhaba", font, 10.0, Vec2::ZERO), Typewriter::new(10.0)))
            .id();
        let tw = |w: &World| *w.borrow::<Typewriter>().get(e).unwrap();
        typewriter_system(&mut world, 0.35);
        assert_eq!(tw(&world).visible(), 3);
        assert_eq!(typed_display(&world, e, "Merhaba").as_deref(), Some("Mer"));
        typewriter_system(&mut world, 1.0);
        assert!(tw(&world).is_finished() && tw(&world).just_finished());
        typewriter_system(&mut world, 0.1);
        assert!(!tw(&world).just_finished(), "finished once");
        world.borrow_mut::<Text>().get_mut(e).unwrap().content = "Yeni bir satır".into();
        typewriter_system(&mut world, 0.2);
        assert_eq!(tw(&world).visible(), 2, "a new line types from the top");
        world.borrow_mut::<Typewriter>().get_mut(e).unwrap().skip();
        typewriter_system(&mut world, 0.0);
        assert!(tw(&world).just_finished() && tw(&world).visible() == 14);

        let code: String = include_str!("text.rs")
            .split(concat!("mod ", "clip_tests"))
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(code.contains(concat!("typed_display(world,id,", "shown.as_deref()")), "record_text no longer cuts typed labels");
    }

    /// A scrolling list clips like a `ClipContent` panel: its rows scroll out of sight.
    #[test]
    fn a_scroll_node_clips_its_children() {
        let mut world = World::new();
        let list = world.spawn_bundle((node(10.0, 10.0, 100.0, 50.0), crate::ui::Scroll::default()));
        let row = world.spawn_bundle((node(10.0, -20.0, 100.0, 40.0),));
        world.add_child(list, row);
        assert_eq!(ui_clip(&world, row.id()), Some([Vec2::splat(10.0), Vec2::new(110.0, 60.0)]));
    }
}
