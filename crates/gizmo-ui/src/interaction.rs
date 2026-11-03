use gizmo_core::query::{Query, Mut};
use gizmo_core::system::{Res, ResMut};
use gizmo_core::input::{GamepadButton, Input};
use crate::components::{
    Clicked, ClipContent, Drag, DropTarget, Dropdown, Interaction, Node, Scroll, Slider, Toggle, Tooltip, ZIndex,
};
use gizmo_core::component::Parent;

/// The layer `entity` is on: its own [`ZIndex`], or the nearest one up its parents, or `0`.
fn layer_with(entity: u32, z: impl Fn(u32) -> Option<i32>, parent: impl Fn(u32) -> Option<u32>) -> i32 {
    let mut at = Some(entity);
    // Bounded: a hierarchy cycle (which the core prevents) must not hang a frame.
    for _ in 0..256 {
        let Some(id) = at else { break };
        if let Some(l) = z(id) {
            return l;
        }
        at = parent(id);
    }
    0
}

/// The layer `entity` is painted and hit-tested on — see [`ZIndex`]. For the facade's draw path,
/// which lays quads out by it.
#[must_use]
pub fn ui_layer(world: &gizmo_core::World, entity: u32) -> i32 {
    let z = world.borrow::<ZIndex>();
    let p = world.borrow::<Parent>();
    layer_with(entity, |id| z.get(id).map(|z| z.0), |id| p.get(id).map(|p| p.0))
}

/// Keyboard and gamepad navigation of the interactive elements — a resource.
///
/// A menu played with a gamepad, or by someone who reaches for Tab, needs a **focus**: one
/// element that the arrows move off and Enter presses. `gizmo-ui` had none ("no keyboard focus",
/// in its own docs and `ui_layout.rs`'s header), so every menu in a game was mouse-only.
///
/// **Off by default**, because the keys it takes — the arrows, Tab, Space, Enter, the d-pad and
/// the south face button — are a game's keys too. A menu turns it on while it is open:
/// `world.get_resource_mut::<UiNavigation>().unwrap().enabled = true`.
///
/// When on:
/// - **Down / Right / Tab / d-pad down, right** move the focus to the next element in reading
///   order (top to bottom, then left to right, by laid-out position); **Up / Left / Shift+Tab /
///   d-pad up, left** to the previous one. It wraps. The first press with nothing focused focuses
///   the first element.
/// - **Enter / Space / the south face button** click the focused element — its
///   [`Clicked`] is set for that frame, exactly as a mouse click would.
/// - The mouse moving over an element focuses it, so the two never disagree about which one is
///   current.
/// - The focused element reads as [`Interaction::Hovered`] when the mouse is not pressing it, so
///   the hover colour a game already paints is its focus highlight.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UiNavigation {
    /// Whether the keys above navigate. See the type's docs for why it is off by default.
    pub enabled: bool,
    focused: Option<u32>,
    /// The keys moved the focus this frame — so a slider the right arrow just reached does not
    /// also take that press as a step.
    moved_by_keys: bool,
    /// Up, down, left, right — held, they repeat (`HoldRepeat`): a held ↓ walks down the list and
    /// a held → steps a slider on, as every menu does. They used to fire on the press only.
    repeats: [gizmo_core::input::HoldRepeat; 4],
    /// This frame's left/right pulses, net (−1, 0, 1, or more on a long frame) — what a focused
    /// slider or dropdown steps by.
    sideways: i32,
}

impl UiNavigation {
    /// The focused element, if any.
    #[must_use]
    pub fn focused(&self) -> Option<u32> {
        self.focused
    }

    /// Moves the focus to `entity` (`None` clears it) — a menu opening on its first button.
    pub fn focus(&mut self, entity: Option<u32>) {
        self.focused = entity;
    }

    /// Whether this frame's keys moved the focus (rather than acting on what it was on).
    #[must_use]
    pub fn moved_this_frame(&self) -> bool {
        self.moved_by_keys
    }

    /// This frame's left/right steps for the focused element — ←/→ or the d-pad, repeating while
    /// held. Positive is right.
    #[must_use]
    pub fn sideways(&self) -> i32 {
        self.sideways
    }
}

fn key(name: &str) -> u32 {
    gizmo_core::input::keys::code_from_name(name).unwrap_or(u32::MAX)
}

/// Returns whether `point` lies within a node's bounding box.
///
/// The box is treated as the half-open interval
/// `[position, position + size)` on each axis, so that a point on the shared
/// edge of two adjacent elements belongs to exactly one of them.
fn point_in_node(point: (f32, f32), node: &Node) -> bool {
    point.0 >= node.position.x
        && point.0 < node.position.x + node.size.x
        && point.1 >= node.position.y
        && point.1 < node.position.y + node.size.y
}

/// Whether the mouse cursor is over **any** UI element — every laid-out [`Node`], a plain panel as
/// much as a button.
///
/// The question a game asks before it treats a click as a click on the world: placing a tower,
/// selecting a unit. Checking only the buttons' [`Interaction`] is the obvious answer and the
/// wrong one — `kale` did that first, and a click on the empty part of its button bar placed a
/// tower on the ground underneath. `false` without an [`Input`] resource.
pub fn pointer_over_ui(world: &gizmo_core::World) -> bool {
    let Some(input) = world.get_resource::<Input>() else { return false };
    let p = input.mouse_position();
    world.borrow::<Node>().iter().any(|(_, n)| point_in_node(p, n))
        || {
            let nodes = world.borrow::<Node>();
            let point = gizmo_math::Vec2::new(p.0, p.1);
            world
                .borrow::<Dropdown>()
                .iter()
                .any(|(id, d)| nodes.get(id).is_some_and(|n| d.option_at(n, point).is_some()))
        }
}

/// System that updates each element's [`Interaction`] state from the current
/// mouse position and button state, and its [`Clicked`] signal where it has one.
pub fn ui_interaction_system(
    input: Res<Input>,
    navigation: Option<ResMut<UiNavigation>>,
    mut interactions: Query<(&Node, Mut<Interaction>, Option<Mut<Clicked>>)>,
    layers: Query<(Option<&ZIndex>, Option<&Parent>)>,
    fields: Query<&crate::text_input::TextInput>,
    clippers: Query<(&Node, Option<&ClipContent>, Option<&Scroll>)>,
    sliders: Query<&Slider>,
    dropdowns: Query<(&Node, &Dropdown)>,
    time: Option<Res<gizmo_core::time::Time>>,
) {
    // The navigation keys repeat on the real frame time: a menu is used while the game is paused.
    let dt = time.map_or(0.0, |t| t.raw_dt());
    let mouse_pos = input.mouse_position();
    let is_clicked = input.is_mouse_button_pressed(0); // 0 is left click

    // Only the highest layer under the cursor is hovered — a panel in front takes the pointer
    // from what is behind it (see `ZIndex`).
    let layer_of = |id: u32| {
        layer_with(
            id,
            |e| layers.get(e).and_then(|(z, _)| z.map(|z| z.0)),
            |e| layers.get(e).and_then(|(_, p)| p.map(|p| p.0)),
        )
    };
    // Inside every clipping ancestor too: a list item scrolled out of its panel is not drawn, so it
    // must not be clickable where it would have been.
    let visible = |id: u32| {
        let mut at = layers.get(id).and_then(|(_, p)| p.map(|p| p.0));
        for _ in 0..256 {
            let Some(a) = at else { return true };
            if let Some((n, c, s)) = clippers.get(a) {
                if (c.is_some() || s.is_some()) && !point_in_node(mouse_pos, n) {
                    return false;
                }
                // The scrollbar's strip belongs to the bar: a press there is not on the list item
                // under it, or dragging the bar would click whatever it was released over.
                if s.is_some_and(|s| s.in_bar_strip(n, gizmo_math::Vec2::new(mouse_pos.0, mouse_pos.1))) {
                    return false;
                }
                // A list being flicked: the finger is scrolling, not tapping the item under it —
                // or every scroll would end by clicking whatever it was let go over.
                if s.is_some_and(|s| s.is_flicking()) {
                    return false;
                }
            }
            at = layers.get(a).and_then(|(_, p)| p.map(|p| p.0));
        }
        true
    };
    // An open dropdown's list takes the pointer: nothing under it is hovered or pressed.
    let pointer = gizmo_math::Vec2::new(mouse_pos.0, mouse_pos.1);
    let over_open_list = dropdowns.iter().any(|(_, (n, d))| d.option_at(n, pointer).is_some());
    let under: Vec<(u32, i32)> = interactions
        .iter_mut()
        .filter(|(id, (n, _, _))| !over_open_list && point_in_node(mouse_pos, n) && visible(*id))
        .map(|(id, _)| (id, layer_of(id)))
        .collect();
    let top = under.iter().map(|(_, l)| *l).max();
    let front = |id: u32| under.iter().any(|(e, l)| *e == id && Some(*l) == top);

    // Keyboard/gamepad focus — decided before the per-element pass so it can mark the focused one.
    // A text field that has the keyboard keeps it: Space and Enter are typing there, not
    // clicks, and the arrows are not focus moves.
    let typing = fields.iter().any(|(_, f)| f.is_focused());
    let (focused, activate) = match navigation {
        Some(mut nav) if nav.enabled && !typing => {
            navigate(&input, &mut nav, &mut interactions, &front, &|id| {
                sliders.get(id).is_some() || dropdowns.get(id).is_some()
            }, dt)
        }
        Some(mut nav) if nav.enabled => {
            nav.moved_by_keys = false;
            nav.sideways = 0;
            (nav.focused(), false)
        }
        _ => (None, false),
    };

    // Note: This is a simplified check that doesn't account for z-index or hierarchy properly yet.
    // In a real UI system, we would need to walk the tree from front to back.
    for (id, (_, mut interaction, clicked)) in interactions.iter_mut() {
        let was = *interaction;
        let is_hovered = front(id);

        let is_focused = focused == Some(id);
        if is_hovered && is_clicked {
            *interaction = Interaction::Pressed;
        } else if is_hovered || is_focused {
            *interaction = Interaction::Hovered;
        } else {
            *interaction = Interaction::None;
        }

        if let Some(mut clicked) = clicked {
            // Released over the element it was pressed on. Released elsewhere, `was` is still
            // `Pressed` but the new state is `None`: cancelled. Or activated from the keyboard.
            let now = (was == Interaction::Pressed && is_hovered && !is_clicked) || (is_focused && activate);
            if clicked.0 != now {
                clicked.0 = now;
            }
        }
    }
}

/// Moves the focus for this frame's keys and mouse, and says whether the focused element was
/// activated. Returns the focus after the move.
fn navigate(
    input: &Input,
    nav: &mut UiNavigation,
    interactions: &mut Query<(&Node, Mut<Interaction>, Option<Mut<Clicked>>)>,
    hovered: &dyn Fn(u32) -> bool,
    horizontal_taken: &dyn Fn(u32) -> bool,
    dt: f32,
) -> (Option<u32>, bool) {
    // The focusable elements in reading order. A zero-size node is hidden (or not laid out yet).
    let mut order: Vec<(u32, f32, f32, bool)> = interactions
        .iter_mut()
        .filter(|(_, (n, _, _))| n.size.x > 0.0 && n.size.y > 0.0)
        .map(|(id, (n, _, _))| (id, n.position.y, n.position.x, hovered(id)))
        .collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)).then(a.0.cmp(&b.0)));
    if order.is_empty() {
        nav.focused = None;
        return (None, false);
    }
    if nav.focused.is_some_and(|f| !order.iter().any(|o| o.0 == f)) {
        nav.focused = None;
    }

    // The mouse moving onto an element takes the focus with it.
    let (dx, dy) = input.mouse_delta();
    if dx != 0.0 || dy != 0.0 {
        if let Some(hovered) = order.iter().find(|o| o.3) {
            nav.focused = Some(hovered.0);
        }
    }

    let pad = input.gamepad();
    let pad_just = |b: GamepadButton| pad.is_some_and(|p| p.is_just_pressed(b));
    let shift = input.is_key_pressed(key("lshift")) || input.is_key_pressed(key("rshift"));
    let tab = input.is_key_just_pressed(key("tab"));
    // Each direction held repeats (`HoldRepeat`): the key or the d-pad.
    let pad_held = |b: GamepadButton| pad.is_some_and(|p| p.is_pressed(b));
    let held = [
        input.is_key_pressed(key("up")) || pad_held(GamepadButton::DPadUp),
        input.is_key_pressed(key("down")) || pad_held(GamepadButton::DPadDown),
        input.is_key_pressed(key("left")) || pad_held(GamepadButton::DPadLeft),
        input.is_key_pressed(key("right")) || pad_held(GamepadButton::DPadRight),
    ];
    // A NEW press always fires, even if the key was down last frame too: two taps in consecutive
    // frames are two presses, not one long hold (the repeat would swallow the second).
    let pressed_now = [
        input.is_key_just_pressed(key("up")) || pad_just(GamepadButton::DPadUp),
        input.is_key_just_pressed(key("down")) || pad_just(GamepadButton::DPadDown),
        input.is_key_just_pressed(key("left")) || pad_just(GamepadButton::DPadLeft),
        input.is_key_just_pressed(key("right")) || pad_just(GamepadButton::DPadRight),
    ];
    let mut pulses = [0u32; 4];
    for (i, r) in nav.repeats.iter_mut().enumerate() {
        if pressed_now[i] {
            r.tick(false, 0.0);
        }
        pulses[i] = r.tick(held[i] || pressed_now[i], dt);
    }
    let [up, down, left, right] = pulses.map(|p| p > 0);
    nav.sideways = pulses[3] as i32 - pulses[2] as i32;
    // Left and right belong to a focused slider or dropdown: they move its value, not the focus.
    let sideways_free = !nav.focused.is_some_and(horizontal_taken);
    let next = (tab && !shift) || down || (sideways_free && right);
    let prev = (tab && shift) || up || (sideways_free && left);
    nav.moved_by_keys = next || prev;
    if next || prev {
        let n = order.len();
        let at = nav.focused.and_then(|f| order.iter().position(|o| o.0 == f));
        let i = match (at, next) {
            (None, _) => 0,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        nav.focused = Some(order[i].0);
    }

    let activate = nav.focused.is_some()
        && (input.is_key_just_pressed(key("enter"))
            || input.is_key_just_pressed(key("space"))
            || pad_just(GamepadButton::South));
    (nav.focused, activate)
}

/// Advances every [`Slider`]: a drag across it sets the value from where the pointer is on the
/// track (captured until the release), and the left/right arrows or the d-pad step it while it
/// has the navigation focus. Runs after `ui_interaction_system`.
pub fn ui_slider_system(
    input: Res<Input>,
    navigation: Option<Res<UiNavigation>>,
    mut sliders: Query<(&Node, &Interaction, Mut<Slider>)>,
) {
    let (x, y) = input.mouse_position();
    let pointer = gizmo_math::Vec2::new(x, y);
    let down_now = input.is_mouse_button_just_pressed(0);
    let held = input.is_mouse_button_pressed(0) && !input.is_mouse_button_just_released(0);
    // A focus the keys only just moved here is not a press on this slider.
    let focused = navigation
        .as_ref()
        .filter(|n| n.enabled && !n.moved_this_frame())
        .and_then(|n| n.focused());
    // ←/→ and the d-pad, repeating while held — read once, by the navigation.
    let keys = navigation.as_ref().map_or(0, |n| n.sideways());
    for (id, (node, interaction, mut slider)) in sliders.iter_mut() {
        let pressed_here = down_now && *interaction == Interaction::Pressed;
        let keys_here = if focused == Some(id) { keys } else { 0 };
        let mut next = *slider;
        next.step_frame(pressed_here, held, pointer, (node.position.x, node.size.x), keys_here);
        if next != *slider {
            *slider = next;
        }
    }
}

/// Opens, closes and picks for every [`Dropdown`] — see its docs. Runs after
/// `ui_interaction_system`, whose click it reads.
pub fn ui_dropdown_system(
    input: Res<Input>,
    navigation: Option<Res<UiNavigation>>,
    mut dropdowns: Query<(&Node, &Clicked, Mut<Dropdown>)>,
) {
    let (x, y) = input.mouse_position();
    let pointer = gizmo_math::Vec2::new(x, y);
    let press = input.is_mouse_button_just_pressed(0).then_some(pointer);
    let focused = navigation
        .as_ref()
        .filter(|n| n.enabled && !n.moved_this_frame())
        .and_then(|n| n.focused());
    // ←/→ and the d-pad, repeating while held — read once, by the navigation.
    let keys = navigation.as_ref().map_or(0, |n| n.sideways());
    for (id, (node, clicked, mut dd)) in dropdowns.iter_mut() {
        let mut next = dd.clone();
        let step = if focused == Some(id) { keys } else { 0 };
        next.step_frame(clicked.0, press, point_in_node((x, y), node), pointer, node, step);
        if next != *dd {
            *dd = next;
        }
    }
}

/// Flips every [`Toggle`] whose element was [`Clicked`] this frame. Runs after
/// `ui_interaction_system`, which sets the click.
pub fn ui_toggle_system(mut toggles: Query<(&Clicked, Mut<Toggle>)>) {
    for (_, (clicked, mut toggle)) in toggles.iter_mut() {
        let mut next = *toggle;
        next.step(clicked.0);
        if next != *toggle {
            *toggle = next;
        }
    }
}

/// Advances every [`Drag`]: a press that begins on the element (it is `Pressed` on the frame the
/// button went down) starts one, and it follows the pointer until the button is released,
/// wherever the pointer goes. Runs after `ui_interaction_system`, whose `Pressed` it reads.
pub fn ui_drag_system(input: Res<Input>, mut drags: Query<(&Interaction, Mut<Drag>)>) {
    let (x, y) = input.mouse_position();
    let pointer = gizmo_math::Vec2::new(x, y);
    let down_now = input.is_mouse_button_just_pressed(0);
    let held = input.is_mouse_button_pressed(0) && !input.is_mouse_button_just_released(0);
    for (_, (interaction, mut drag)) in drags.iter_mut() {
        let pressed_here = down_now && *interaction == Interaction::Pressed;
        // Only write when something changes, so `Changed<Drag>` means a moving drag.
        let mut next = *drag;
        next.step(pressed_here, held, pointer);
        if next != *drag {
            *drag = next;
        }
    }
}

/// Shows and hides every [`Tooltip`]: the pointer resting on its element (hovered, inside the
/// box — not merely focused — and no button held) for its delay. Runs after
/// `ui_interaction_system`, on real time.
pub fn ui_tooltip_system(
    input: Res<Input>,
    time: Option<Res<gizmo_core::time::Time>>,
    mut tips: Query<(&Node, &Interaction, Mut<Tooltip>)>,
) {
    let dt = time.map_or(0.0, |t| t.raw_dt());
    let (x, y) = input.mouse_position();
    let pointer = gizmo_math::Vec2::new(x, y);
    let held = input.is_mouse_button_pressed(0) || input.is_mouse_button_pressed(1);
    for (_, (node, interaction, mut tip)) in tips.iter_mut() {
        let resting = *interaction == Interaction::Hovered && point_in_node((x, y), node) && !held;
        // Only write when something changes, so `Changed<Tooltip>` is not every frame.
        let next = tip.next_state(resting, pointer, dt);
        if next != tip.state() {
            tip.set_state(next);
        }
    }
}

/// Finds the [`DropTarget`] under every drag — the rules are on [`DropTarget`] — and writes the
/// answer on both ends: [`DropTarget::hovered_by`] / [`DropTarget::dropped`], and
/// [`Drag::over`] / [`Drag::dropped_on`]. Runs after `ui_drag_system`, which ends a drag on its
/// release frame, so the drop is seen on that same frame.
pub fn ui_drop_system(
    mut drags: Query<Mut<Drag>>,
    mut targets: Query<(&Node, Mut<DropTarget>)>,
    layers: Query<(Option<&ZIndex>, Option<&Parent>)>,
    clippers: Query<(&Node, Option<&ClipContent>, Option<&Scroll>)>,
) {
    let parent = |id: u32| layers.get(id).and_then(|(_, p)| p.map(|p| p.0));
    let layer_of = |id: u32| layer_with(id, |e| layers.get(e).and_then(|(z, _)| z.map(|z| z.0)), parent);
    // `id` is `ancestor` or lies under it.
    let within = |id: u32, ancestor: u32| {
        let mut at = Some(id);
        for _ in 0..256 {
            match at {
                Some(a) if a == ancestor => return true,
                Some(a) => at = parent(a),
                None => return false,
            }
        }
        false
    };
    // Not clipped away at `point` by any ClipContent / Scroll above it.
    let seen = |id: u32, point: (f32, f32)| {
        let mut at = parent(id);
        for _ in 0..256 {
            let Some(a) = at else { return true };
            if let Some((n, c, s)) = clippers.get(a) {
                if (c.is_some() || s.is_some()) && !point_in_node(point, n) {
                    return false;
                }
            }
            at = parent(a);
        }
        true
    };

    // (dragged element, where, still held). Normally one: there is one pointer.
    let live: Vec<(u32, (f32, f32), bool)> = drags
        .iter_mut()
        .filter(|(_, d)| d.is_dragging() || d.just_ended())
        .map(|(id, d)| (id, (d.position().x, d.position().y), !d.just_ended()))
        .collect();
    let candidates: Vec<(u32, Node)> = if live.is_empty() {
        Vec::new()
    } else {
        targets.iter_mut().map(|(id, (n, _))| (id, *n)).collect()
    };
    let answers: Vec<(u32, Option<u32>, bool)> = live
        .iter()
        .map(|&(drag, point, held)| {
            let best = candidates
                .iter()
                .filter(|(t, n)| {
                    // A zero-size (hidden) target holds no point, so it drops out here too.
                    point_in_node(point, n) && !within(*t, drag) && seen(*t, point)
                })
                .max_by(|(a, na), (b, nb)| {
                    let area = |n: &Node| n.size.x * n.size.y;
                    layer_of(*a)
                        .cmp(&layer_of(*b))
                        .then(area(nb).total_cmp(&area(na)))
                        .then(a.cmp(b))
                })
                .map(|(t, _)| *t);
            (drag, best, held)
        })
        .collect();

    // Write only what changed, so `Changed<DropTarget>` / `Changed<Drag>` mean something.
    for (id, (_, mut target)) in targets.iter_mut() {
        let hovered = answers.iter().find(|a| a.1 == Some(id) && a.2).map(|a| a.0);
        let dropped = answers.iter().find(|a| a.1 == Some(id) && !a.2).map(|a| a.0);
        let next = DropTarget::answer(hovered, dropped);
        if *target != next {
            *target = next;
        }
    }
    for (id, mut drag) in drags.iter_mut() {
        let (over, dropped_on) = match answers.iter().find(|a| a.0 == id) {
            Some(&(_, t, true)) => (t, None),
            Some(&(_, t, false)) => (None, t),
            None => (None, None),
        };
        if drag.over() != over || drag.dropped_on() != dropped_on {
            drag.set_target(over, dropped_on);
        }
    }
}

/// The mouse wheel scrolls the innermost [`Scroll`] node under the cursor — by its `speed` a
/// notch, wheel up showing earlier content. The layout clamps the result.
pub fn ui_scroll_system(
    input: Res<Input>,
    time: Option<Res<gizmo_core::time::Time>>,
    navigation: Option<Res<UiNavigation>>,
    nodes: Query<&Node>,
    parents: Query<&Parent>,
    mut scrolls: Query<(&Node, Mut<Scroll>)>,
) {
    let dt = time.map_or(0.0, |t| t.raw_dt());
    // The keys moved the focus: bring it into view in the nearest scrolling list around it. A held
    // ↓ walking down a long menu used to walk the focus out of sight.
    if let Some(focused) = navigation.as_ref().filter(|n| n.enabled && n.moved_this_frame()).and_then(|n| n.focused()) {
        if let Some(item) = nodes.get(focused).copied() {
            let mut at = parents.get(focused).map(|p| p.0);
            for _ in 0..256 {
                let Some(a) = at else { break };
                if let Some((list, mut scroll)) = scrolls.get_mut(a) {
                    let (top, bottom) = (list.position.y, list.position.y + list.size.y);
                    let (item_top, item_bottom) = (item.position.y, item.position.y + item.size.y);
                    let shift = if item_top < top {
                        item_top - top
                    } else if item_bottom > bottom {
                        (item_bottom - bottom).min(item_top - top)
                    } else {
                        0.0
                    };
                    if shift != 0.0 {
                        let max = scroll.max_offset().y;
                        scroll.offset.y = (scroll.offset.y + shift).clamp(0.0, max.max(0.0));
                    }
                    break;
                }
                at = parents.get(a).map(|p| p.0);
            }
        }
    }
    // The scrollbar first: a press in the strip takes the thumb, and it follows the pointer until
    // the release — what a touchpad without a wheel, and a finger, scroll a list with.
    let p = input.mouse_position();
    let pointer = gizmo_math::Vec2::new(p.0, p.1);
    let pressed = input.is_mouse_button_just_pressed(0);
    let held = input.is_mouse_button_pressed(0) && !input.is_mouse_button_just_released(0);
    // A finger on a list's content scrolls it (and flings it), the way a phone list moves. Only
    // a finger: a mouse dragging across a list is selecting or dragging, not scrolling.
    let finger = input.primary_touch().is_some() || !input.touches().is_empty();
    // The innermost list under a finger that just came down takes it.
    let flick_target = (pressed && finger)
        .then(|| {
            scrolls
                .iter_mut()
                .filter(|(_, (n, s))| point_in_node(p, n) && !s.in_bar_strip(n, pointer))
                .min_by(|a, b| (a.1 .0.size.x * a.1 .0.size.y).total_cmp(&(b.1 .0.size.x * b.1 .0.size.y)))
                .map(|(id, _)| id)
        })
        .flatten();
    for (id, (node, mut scroll)) in scrolls.iter_mut() {
        let mut next = *scroll;
        if pressed || scroll.is_dragging_thumb() {
            next.drag_thumb(node, pressed, held, pointer);
        }
        next.flick_frame(flick_target == Some(id), held, pointer.y, dt);
        if next != *scroll {
            *scroll = next;
        }
    }

    let wheel = input.mouse_scroll();
    if wheel == 0.0 {
        return;
    }
    let target = scrolls
        .iter_mut()
        .filter(|(_, (n, _))| point_in_node(p, n))
        .min_by(|a, b| (a.1 .0.size.x * a.1 .0.size.y).total_cmp(&(b.1 .0.size.x * b.1 .0.size.y)))
        .map(|(id, _)| id);
    if let Some(id) = target {
        if let Some((_, mut s)) = scrolls.get_mut(id) {
            let step = s.speed;
            s.offset.y -= wheel * step;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec2;

    fn node(pos: (f32, f32), size: (f32, f32)) -> Node {
        Node {
            position: Vec2::new(pos.0, pos.1),
            size: Vec2::new(size.0, size.1),
        }
    }

    /// One frame of the real system over `world`, with the mouse at `at` and the primary button
    /// `down` or not.
    fn frame(world: &mut gizmo_core::World, at: (f32, f32), down: bool) {
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        {
            let mut input = world.get_resource_mut::<Input>().unwrap();
            input.begin_frame();
            input.set_mouse_position(at.0, at.1);
            if down {
                input.on_mouse_button_pressed(0);
            } else {
                input.on_mouse_button_released(0);
            }
        }
        let mut s = Schedule::new();
        s.add_di_system(ui_interaction_system.into_config());
        s.build();
        s.run(world, 0.0);
    }

    fn button_world() -> (gizmo_core::World, u32, u32) {
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        let button = world.spawn_bundle(crate::ButtonBundle {
            node: node((0.0, 0.0), (100.0, 40.0)),
            ..Default::default()
        });
        // A plain interactive node without a `Clicked`: it must keep working.
        let plain = world.spawn_bundle((node((200.0, 0.0), (100.0, 40.0)), Interaction::None));
        (world, button.id(), plain.id())
    }

    fn clicked(world: &gizmo_core::World, id: u32) -> bool {
        world.borrow::<Clicked>().get(id).is_some_and(|c| c.0)
    }

    /// A plain panel counts as UI, not only a button; outside every node is the world.
    #[test]
    fn a_plain_panel_is_ui_too() {
        let (mut world, _, _) = button_world();
        world.spawn_bundle((node((0.0, 600.0), (800.0, 64.0)),)); // a bar, no Interaction
        for (at, over) in [((50.0, 20.0), true), ((400.0, 620.0), true), ((400.0, 300.0), false)] {
            world.get_resource_mut::<Input>().unwrap().set_mouse_position(at.0, at.1);
            assert_eq!(pointer_over_ui(&world), over, "at {at:?}");
        }
    }

    /// Press on, release on: clicked for exactly that one frame.
    #[test]
    fn a_click_completes_on_release_and_lasts_one_frame() {
        let (mut world, b, plain) = button_world();
        frame(&mut world, (50.0, 20.0), false);
        assert!(!clicked(&world, b), "hovering is not clicking");
        frame(&mut world, (50.0, 20.0), true);
        assert!(!clicked(&world, b), "pressing is not clicking yet");
        frame(&mut world, (50.0, 20.0), false);
        assert!(clicked(&world, b), "released over the button: clicked");
        frame(&mut world, (50.0, 20.0), false);
        assert!(!clicked(&world, b), "a click lasts one frame");

        // The node without a `Clicked` still tracks its interaction.
        frame(&mut world, (250.0, 20.0), true);
        assert_eq!(world.borrow::<Interaction>().get(plain).copied(), Some(Interaction::Pressed));
    }

    /// Three buttons in a column (deliberately spawned out of reading order) and navigation on.
    fn menu() -> (gizmo_core::World, [u32; 3]) {
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        world.insert_resource(UiNavigation { enabled: true, ..Default::default() });
        let mid = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 100.0), (100.0, 40.0)), ..Default::default() });
        let top = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 0.0), (100.0, 40.0)), ..Default::default() });
        let low = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 200.0), (100.0, 40.0)), ..Default::default() });
        (world, [top.id(), mid.id(), low.id()])
    }

    /// One frame with `key` just pressed (if any), the mouse parked off every button.
    fn press(world: &mut gizmo_core::World, key_name: Option<&str>) {
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        {
            let mut input = world.get_resource_mut::<Input>().unwrap();
            input.begin_frame();
            input.set_mouse_position(500.0, 500.0);
            if let Some(k) = key_name {
                input.on_key_pressed(key(k));
            }
        }
        let mut s = Schedule::new();
        s.add_di_system(ui_interaction_system.into_config());
        s.build();
        s.run(world, 0.0);
        if let Some(k) = key_name {
            world.get_resource_mut::<Input>().unwrap().on_key_released(key(k));
        }
    }

    fn focused(world: &gizmo_core::World) -> Option<u32> {
        world.get_resource::<UiNavigation>().unwrap().focused()
    }

    /// Down walks the buttons in reading order — top to bottom, whatever order they were spawned
    /// in — and wraps; Up goes back and wraps the other way.
    #[test]
    fn the_arrows_walk_the_buttons_in_reading_order_and_wrap() {
        let (mut world, [top, mid, low]) = menu();
        let mut seen = Vec::new();
        for _ in 0..4 {
            press(&mut world, Some("down"));
            seen.push(focused(&world));
        }
        assert_eq!(seen, vec![Some(top), Some(mid), Some(low), Some(top)]);
        press(&mut world, Some("up"));
        assert_eq!(focused(&world), Some(low), "up from the first wraps to the last");
        press(&mut world, Some("tab"));
        assert_eq!(focused(&world), Some(top), "tab is next");
    }

    /// Enter clicks the focused button for one frame and no other; the focused one reads Hovered.
    #[test]
    fn enter_clicks_the_focused_button_once() {
        let (mut world, [top, mid, _]) = menu();
        press(&mut world, Some("down"));
        press(&mut world, Some("down"));
        assert_eq!(world.borrow::<Interaction>().get(mid).copied(), Some(Interaction::Hovered), "focus shows as hover");
        assert_eq!(world.borrow::<Interaction>().get(top).copied(), Some(Interaction::None));
        press(&mut world, Some("enter"));
        assert!(clicked(&world, mid) && !clicked(&world, top));
        press(&mut world, None);
        assert!(!clicked(&world, mid), "a click lasts one frame");
    }

    /// Off (the default), the same keys do nothing — they are the game's.
    #[test]
    fn navigation_off_leaves_the_keys_to_the_game() {
        let (mut world, [_, mid, _]) = menu();
        world.get_resource_mut::<UiNavigation>().unwrap().enabled = false;
        press(&mut world, Some("down"));
        press(&mut world, Some("enter"));
        assert_eq!(focused(&world), None);
        assert!(!clicked(&world, mid));
        assert!(!UiNavigation::default().enabled, "on by default would steal the arrows");
    }

    /// While a text field has the keyboard, the navigation keys are typing, not navigation.
    #[test]
    fn a_focused_text_field_keeps_the_keys_from_navigation() {
        let (mut world, [top, mid, _]) = menu();
        press(&mut world, Some("down"));
        assert_eq!(focused(&world), Some(top));
        let mut field = crate::text_input::TextInput::new(0);
        field.set_focused(true);
        world.spawn_bundle((field,));
        press(&mut world, Some("down"));
        assert_eq!(focused(&world), Some(top), "the arrow moved the focus while typing");
        press(&mut world, Some("enter"));
        assert!(!clicked(&world, top) && !clicked(&world, mid), "Enter clicked a button while typing");
    }

    /// The mouse moving onto a button takes the focus, so the arrows continue from there.
    #[test]
    fn the_mouse_moving_onto_a_button_focuses_it() {
        let (mut world, [_, mid, low]) = menu();
        {
            let mut input = world.get_resource_mut::<Input>().unwrap();
            input.begin_frame();
            input.on_mouse_moved(50.0, 120.0);
        }
        let mut s = gizmo_core::system::Schedule::new();
        use gizmo_core::system::IntoSystemConfig;
        s.add_di_system(ui_interaction_system.into_config());
        s.build();
        s.run(&mut world, 0.0);
        assert_eq!(focused(&world), Some(mid));
        press(&mut world, Some("down"));
        assert_eq!(focused(&world), Some(low));
    }

    /// Two buttons on top of each other: only the one on the higher layer is hovered and clicked;
    /// with no `ZIndex` both are, as before. A child without one takes its parent's.
    #[test]
    fn only_the_front_layer_under_the_cursor_is_hovered() {
        use gizmo_core::hierarchy::HierarchyExt;
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        let back = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 0.0), (100.0, 100.0)), ..Default::default() });
        let panel = world.spawn_bundle((node((0.0, 0.0), (300.0, 300.0)), ZIndex(2)));
        let front = world.spawn_bundle(crate::ButtonBundle { node: node((50.0, 50.0), (100.0, 100.0)), ..Default::default() });
        world.add_child(panel, front);
        frame(&mut world, (75.0, 75.0), false);
        let state = |id| world.borrow::<Interaction>().get(id).copied();
        assert_eq!(state(front.id()), Some(Interaction::Hovered), "the front button, layer 2 through its parent");
        assert_eq!(state(back.id()), Some(Interaction::None), "the button behind took the pointer too");
        assert_eq!(ui_layer(&world, front.id()), 2);
        assert_eq!(ui_layer(&world, back.id()), 0);
        frame(&mut world, (75.0, 75.0), true);
        frame(&mut world, (75.0, 75.0), false);
        assert!(clicked(&world, front.id()) && !clicked(&world, back.id()), "the click went through");
    }

    /// A button scrolled (or cut) out of its panel is not drawn, so it is not under the pointer
    /// either — not even where its box would be.
    #[test]
    fn a_button_outside_its_clipping_panel_is_not_hovered() {
        use gizmo_core::hierarchy::HierarchyExt;
        for clip in [true, false] {
            let mut world = gizmo_core::World::new();
            world.insert_resource(Input::new());
            let panel = world.spawn_bundle((node((0.0, 0.0), (100.0, 100.0)),));
            if clip {
                world.add_component(panel, Scroll::default());
            } else {
                world.add_component(panel, ClipContent);
            }
            let shown = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 60.0), (100.0, 40.0)), ..Default::default() });
            let hidden = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 100.0), (100.0, 40.0)), ..Default::default() });
            world.add_child(panel, shown);
            world.add_child(panel, hidden);
            let state = |w: &gizmo_core::World, id| w.borrow::<Interaction>().get(id).copied();
            frame(&mut world, (50.0, 120.0), false);
            assert_eq!(state(&world, hidden.id()), Some(Interaction::None), "clip={clip}: the hidden button took the pointer");
            frame(&mut world, (50.0, 80.0), false);
            assert_eq!(state(&world, shown.id()), Some(Interaction::Hovered), "clip={clip}: the visible one did not");
        }
    }

    /// The wheel scrolls the innermost scrolling panel under the cursor, and nothing else.
    #[test]
    fn the_wheel_scrolls_the_innermost_panel_under_the_cursor() {
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        let outer = world.spawn_bundle((node((0.0, 0.0), (300.0, 300.0)), Scroll::default()));
        let inner = world.spawn_bundle((node((0.0, 0.0), (100.0, 100.0)), Scroll::default().with_speed(25.0)));
        let wheel = |world: &mut gizmo_core::World, at: (f32, f32), notches: f32| {
            {
                let mut input = world.get_resource_mut::<Input>().unwrap();
                input.begin_frame();
                input.set_mouse_position(at.0, at.1);
                input.on_mouse_scroll(notches);
            }
            let mut s = Schedule::new();
            s.add_di_system(ui_scroll_system.into_config());
            s.build();
            s.run(world, 0.0);
        };
        let offset = |w: &gizmo_core::World, e: gizmo_core::Entity| w.borrow::<Scroll>().get(e.id()).unwrap().offset.y;
        wheel(&mut world, (50.0, 50.0), -2.0);
        assert_eq!((offset(&world, inner), offset(&world, outer)), (50.0, 0.0), "two notches down, at its own speed");
        wheel(&mut world, (200.0, 200.0), -1.0);
        assert_eq!((offset(&world, inner), offset(&world, outer)), (50.0, 40.0));
        wheel(&mut world, (50.0, 50.0), 1.0);
        assert_eq!(offset(&world, inner), 25.0, "wheel up goes back");
        wheel(&mut world, (500.0, 500.0), -1.0);
        assert_eq!((offset(&world, inner), offset(&world, outer)), (25.0, 40.0), "outside both");
    }

    /// One frame of interaction then drag, the order `register` schedules them in.
    fn drag_frame(world: &mut gizmo_core::World, at: (f32, f32), down: bool) {
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        frame(world, at, down);
        let mut s = Schedule::new();
        s.add_di_system(ui_drag_system.into_config());
        s.build();
        s.run(world, 0.0);
    }

    /// A drag starts on a press on the element, follows the pointer **off** it (captured) until
    /// the release, reports the drop for one frame — and a held button sliding onto the element
    /// starts nothing.
    #[test]
    fn a_drag_is_captured_from_press_to_release() {
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        let knob = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 0.0), (100.0, 40.0)), ..Default::default() });
        world.add_component(knob, Drag::default());
        let drag = |w: &gizmo_core::World| *w.borrow::<Drag>().get(knob.id()).unwrap();

        drag_frame(&mut world, (50.0, 20.0), true);
        let d = drag(&world);
        assert!(d.is_dragging() && d.just_started());
        assert_eq!(d.start(), Vec2::new(50.0, 20.0));
        drag_frame(&mut world, (300.0, 200.0), true);
        let d = drag(&world);
        assert!(d.is_dragging() && !d.just_started(), "lost the drag when the pointer left the element");
        assert_eq!(d.delta(), Vec2::new(250.0, 180.0));
        assert_eq!(world.borrow::<Interaction>().get(knob.id()), Some(&Interaction::None));
        drag_frame(&mut world, (310.0, 200.0), false);
        let d = drag(&world);
        assert!(!d.is_dragging() && d.just_ended());
        assert_eq!(d.position(), Vec2::new(310.0, 200.0), "dropped where it was released");
        drag_frame(&mut world, (310.0, 200.0), false);
        assert!(!drag(&world).just_ended(), "the drop is one frame");

        // Pressed elsewhere, then slid on with the button held: not a drag of this element.
        drag_frame(&mut world, (500.0, 500.0), true);
        drag_frame(&mut world, (50.0, 20.0), true);
        assert!(!drag(&world).is_dragging());
    }

    /// Interaction, drag, then drop — the order `register` schedules them in.
    fn drop_frame(world: &mut gizmo_core::World, at: (f32, f32), down: bool) {
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        drag_frame(world, at, down);
        let mut s = Schedule::new();
        s.add_di_system(ui_drop_system.into_config());
        s.build();
        s.run(world, 0.0);
    }

    /// An item dragged over a slot lights it while held and lands on it on release — even though
    /// the item, following the pointer, is what is under the pointer; the answer is on both ends
    /// and lasts the one frame of the drop.
    #[test]
    fn a_drag_is_dropped_on_the_target_under_it() {
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        let slot = world.spawn_bundle((node((200.0, 0.0), (80.0, 80.0)), DropTarget::default())).id();
        let other = world.spawn_bundle((node((400.0, 0.0), (80.0, 80.0)), DropTarget::default())).id();
        let item = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 0.0), (80.0, 80.0)), ..Default::default() });
        world.add_component(item, Drag::default());
        let item = item.id();
        let target = |w: &gizmo_core::World, id: u32| *w.borrow::<DropTarget>().get(id).unwrap();
        let drag = |w: &gizmo_core::World| *w.borrow::<Drag>().get(item).unwrap();

        drop_frame(&mut world, (40.0, 40.0), true);
        assert_eq!(drag(&world).over(), None);
        // The item now sits over the slot, as a game moves it — and is still under the pointer.
        world.borrow_mut::<Node>().get_mut(item).unwrap().position = Vec2::new(200.0, 0.0);
        drop_frame(&mut world, (240.0, 40.0), true);
        assert_eq!(target(&world, slot).hovered_by(), Some(item));
        assert_eq!(target(&world, other).hovered_by(), None);
        assert_eq!(drag(&world).over(), Some(slot));
        assert_eq!(target(&world, slot).dropped(), None, "held is not dropped");

        drop_frame(&mut world, (240.0, 40.0), false);
        assert_eq!(target(&world, slot).dropped(), Some(item));
        assert_eq!(target(&world, slot).hovered_by(), None, "a dropped drag no longer hovers");
        assert_eq!(drag(&world).dropped_on(), Some(slot));
        assert_eq!(drag(&world).over(), None);

        drop_frame(&mut world, (240.0, 40.0), false);
        assert_eq!(target(&world, slot).dropped(), None, "the drop is one frame");
        assert_eq!(drag(&world).dropped_on(), None, "the drop is one frame");

        // Let go over nothing: no target hears of it.
        drop_frame(&mut world, (40.0, 40.0), true);
        drop_frame(&mut world, (600.0, 300.0), false);
        assert_eq!(drag(&world).dropped_on(), None);
        assert_eq!(target(&world, slot).dropped(), None);
        assert_eq!(target(&world, other).dropped(), None);
    }

    /// Which target wins: the higher layer, then the smaller of nested ones; never the dragged
    /// element or what is under it; never one clipped out of view; never one with no size.
    #[test]
    fn the_drop_goes_to_the_front_innermost_visible_target_not_the_dragged_one() {
        use gizmo_core::hierarchy::HierarchyExt;
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        let spawn = |w: &mut gizmo_core::World, pos: (f32, f32), size: (f32, f32)| {
            w.spawn_bundle((node(pos, size), DropTarget::default()))
        };
        // A panel that is a target, with a slot inside it.
        let panel = spawn(&mut world, (0.0, 200.0), (300.0, 300.0));
        let inner = spawn(&mut world, (100.0, 300.0), (80.0, 80.0));
        world.add_child(panel, inner);
        // The dragged item is itself a target (items take other items), and has a target child.
        let item = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 0.0), (80.0, 80.0)), ..Default::default() });
        world.add_component(item, Drag::default());
        world.add_component(item, DropTarget::default());
        let badge = spawn(&mut world, (0.0, 0.0), (20.0, 20.0));
        world.add_child(item, badge);
        let (panel, inner, item, badge) = (panel.id(), inner.id(), item.id(), badge.id());
        let dropped_on = |w: &gizmo_core::World| w.borrow::<Drag>().get(item).unwrap().dropped_on();
        let throw = |w: &mut gizmo_core::World, at: (f32, f32)| {
            // Back at rest, picked up, carried to `at`, let go.
            w.borrow_mut::<Node>().get_mut(item).unwrap().position = Vec2::ZERO;
            w.borrow_mut::<Node>().get_mut(badge).unwrap().position = Vec2::ZERO;
            drop_frame(w, (10.0, 10.0), true);
            // The item (and its badge) follow the pointer, as a game moves them.
            w.borrow_mut::<Node>().get_mut(item).unwrap().position = Vec2::new(at.0 - 10.0, at.1 - 10.0);
            w.borrow_mut::<Node>().get_mut(badge).unwrap().position = Vec2::new(at.0 - 10.0, at.1 - 10.0);
            drop_frame(w, at, false);
        };

        throw(&mut world, (130.0, 330.0));
        assert_eq!(dropped_on(&world), Some(inner), "nested: the smaller one");
        throw(&mut world, (20.0, 220.0));
        assert_eq!(dropped_on(&world), Some(panel), "outside the slot: the panel");

        // A layer in front wins even though it is bigger.
        let front = spawn(&mut world, (0.0, 200.0), (400.0, 400.0));
        world.add_component(front, ZIndex(3));
        throw(&mut world, (130.0, 330.0));
        assert_eq!(dropped_on(&world), Some(front.id()), "the front layer");
        world.despawn(front);

        // The slot scrolled out of its clipping panel: not a target where it would have been.
        world.add_component(world.entity(panel).unwrap(), ClipContent);
        world.borrow_mut::<Node>().get_mut(inner).unwrap().position = Vec2::new(100.0, 520.0);
        throw(&mut world, (130.0, 550.0));
        assert_eq!(dropped_on(&world), None, "a clipped-away slot took the drop");

        // A hidden (zero-size) target is no target.
        world.borrow_mut::<Node>().get_mut(inner).unwrap().position = Vec2::new(100.0, 300.0);
        world.borrow_mut::<Node>().get_mut(inner).unwrap().size = Vec2::ZERO;
        throw(&mut world, (130.0, 330.0));
        assert_eq!(dropped_on(&world), Some(panel));

        // Neither the item nor its badge — both under the pointer the whole time — took it.
        assert!(world.borrow::<DropTarget>().get(item).unwrap().dropped().is_none());
        assert!(world.borrow::<DropTarget>().get(badge).unwrap().dropped().is_none());
    }

    /// A tooltip appears after its delay of resting on the element, follows the pointer, and goes
    /// at once when the pointer leaves or a button goes down; focus alone does not show it, nor
    /// does an empty text.
    #[test]
    fn a_tooltip_shows_after_resting_and_hides_on_leave_or_press() {
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        let mut time = gizmo_core::time::Time::default();
        time.update(0.2);
        world.insert_resource(time);
        let e = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 0.0), (100.0, 40.0)), ..Default::default() });
        world.add_component(e, Tooltip::new("Fidan").with_delay(0.5));
        let id = e.id();
        let tick = |w: &mut gizmo_core::World, at: (f32, f32), down: bool| {
            frame(w, at, down);
            let mut s = Schedule::new();
            s.add_di_system(ui_tooltip_system.into_config());
            s.build();
            s.run(w, 0.0);
            w.borrow::<Tooltip>().get(id).unwrap().shown_at()
        };
        assert_eq!(tick(&mut world, (10.0, 10.0), false), None, "0.2 s");
        assert_eq!(tick(&mut world, (10.0, 10.0), false), None, "0.4 s");
        assert_eq!(tick(&mut world, (12.0, 10.0), false), Some(Vec2::new(12.0, 10.0)), "0.6 s");
        assert_eq!(tick(&mut world, (30.0, 20.0), false), Some(Vec2::new(30.0, 20.0)), "follows");
        assert_eq!(tick(&mut world, (300.0, 20.0), false), None, "left");
        assert_eq!(tick(&mut world, (10.0, 10.0), false), None, "the wait starts again");
        tick(&mut world, (10.0, 10.0), false);
        assert!(tick(&mut world, (10.0, 10.0), false).is_some());
        assert_eq!(tick(&mut world, (10.0, 10.0), true), None, "a press hides it");
        // The right button leaves the element `Hovered`, and still hides it (a context menu).
        tick(&mut world, (10.0, 10.0), false);
        tick(&mut world, (10.0, 10.0), false);
        assert!(tick(&mut world, (10.0, 10.0), false).is_some());
        world.get_resource_mut::<Input>().unwrap().on_mouse_button_pressed(1);
        let mut s = Schedule::new();
        s.add_di_system(ui_tooltip_system.into_config());
        s.build();
        s.run(&mut world, 0.0);
        assert_eq!(world.borrow::<Tooltip>().get(id).unwrap().shown_at(), None, "right button");
        world.get_resource_mut::<Input>().unwrap().on_mouse_button_released(1);
        tick(&mut world, (10.0, 10.0), false);
        tick(&mut world, (10.0, 10.0), false);
        world.borrow_mut::<Tooltip>().get_mut(id).unwrap().text.clear();
        assert_eq!(tick(&mut world, (10.0, 10.0), false), None, "empty says nothing");

        // Focused by the keys, the pointer elsewhere: `Hovered`, but no tooltip.
        world.borrow_mut::<Tooltip>().get_mut(id).unwrap().text = "Fidan".into();
        world.insert_resource(UiNavigation { enabled: true, ..Default::default() });
        world.get_resource_mut::<UiNavigation>().unwrap().focus(Some(id));
        for _ in 0..5 {
            assert_eq!(tick(&mut world, (300.0, 300.0), false), None);
        }
        assert_eq!(world.borrow::<Interaction>().get(id), Some(&Interaction::Hovered));
    }

    /// Interaction then sliders, the order `register` schedules them in, with the mouse at `at`
    /// and `down` or not, and `key_name` just pressed.
    fn slider_frame(world: &mut gizmo_core::World, at: (f32, f32), down: bool, key_name: Option<&str>) {
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        {
            let mut input = world.get_resource_mut::<Input>().unwrap();
            input.begin_frame();
            input.set_mouse_position(at.0, at.1);
            if down {
                input.on_mouse_button_pressed(0);
            } else {
                input.on_mouse_button_released(0);
            }
            if let Some(k) = key_name {
                input.on_key_pressed(key(k));
            }
        }
        let mut s = Schedule::new();
        s.add_di_system(ui_interaction_system.into_config().label("i"));
        s.add_di_system(ui_slider_system.into_config().after("i"));
        s.build();
        s.run(world, 0.0);
        if let Some(k) = key_name {
            world.get_resource_mut::<Input>().unwrap().on_key_released(key(k));
        }
    }

    /// Dragging sets the value from the pointer's place on the track, keeps it captured past the
    /// end (clamped), and reports the change and the release.
    #[test]
    fn dragging_a_slider_sets_its_value_from_the_pointer() {
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        let s = world.spawn_bundle(crate::ButtonBundle { node: node((100.0, 0.0), (200.0, 30.0)), ..Default::default() });
        world.add_component(s, Slider::new(0.5));
        let get = |w: &gizmo_core::World| *w.borrow::<Slider>().get(s.id()).unwrap();

        slider_frame(&mut world, (150.0, 15.0), true, None);
        assert!((get(&world).value - 0.25).abs() < 1e-6 && get(&world).just_changed());
        slider_frame(&mut world, (900.0, 400.0), true, None);
        assert_eq!(get(&world).value, 1.0, "captured off the track, and clamped");
        slider_frame(&mut world, (900.0, 400.0), true, None);
        assert!(!get(&world).just_changed(), "no move, no change");
        slider_frame(&mut world, (50.0, 400.0), false, None);
        assert!(get(&world).just_released() && get(&world).value == 0.0, "the release position counts");
        slider_frame(&mut world, (250.0, 15.0), false, None);
        assert_eq!(get(&world).value, 0.0, "hovering is not dragging");
    }

    /// With the focus, left and right step the slider and do not move the focus; up and down still
    /// do. Without it, the arrows leave the slider alone.
    #[test]
    fn the_arrows_step_a_focused_slider_instead_of_moving_the_focus() {
        let (mut world, [top, mid, _]) = menu();
        world.add_component(world.get_entity(mid).unwrap(), Slider::new(0.5).with_step(0.1));
        world.get_resource_mut::<UiNavigation>().unwrap().focus(Some(mid));
        let value = |w: &gizmo_core::World| w.borrow::<Slider>().get(mid).unwrap().value;
        slider_frame(&mut world, (500.0, 500.0), false, Some("right"));
        assert!((value(&world) - 0.6).abs() < 1e-6);
        assert_eq!(focused(&world), Some(mid), "right moved the focus off the slider");
        slider_frame(&mut world, (500.0, 500.0), false, Some("left"));
        slider_frame(&mut world, (500.0, 500.0), false, Some("left"));
        assert!((value(&world) - 0.4).abs() < 1e-6);
        slider_frame(&mut world, (500.0, 500.0), false, Some("up"));
        assert_eq!(focused(&world), Some(top), "up still moves the focus");
        slider_frame(&mut world, (500.0, 500.0), false, Some("right"));
        assert!((value(&world) - 0.4).abs() < 1e-6, "an unfocused slider took the key");
    }

    /// A toggle flips once per click — the frame the click completes — and reports that frame
    /// only; Enter on a focused one is a click too.
    #[test]
    fn a_toggle_flips_once_per_click() {
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        let (mut world, button, _) = button_world();
        world.add_component(world.get_entity(button).unwrap(), Toggle::new(false));
        let step = |w: &mut gizmo_core::World, at: (f32, f32), down: bool| {
            frame(w, at, down);
            let mut s = Schedule::new();
            s.add_di_system(ui_toggle_system.into_config());
            s.build();
            s.run(w, 0.0);
        };
        let t = |w: &gizmo_core::World| *w.borrow::<Toggle>().get(button).unwrap();
        step(&mut world, (50.0, 20.0), true);
        assert!(!t(&world).on, "a press is not a click");
        step(&mut world, (50.0, 20.0), false);
        assert!(t(&world).on && t(&world).just_changed());
        step(&mut world, (50.0, 20.0), false);
        assert!(t(&world).on && !t(&world).just_changed(), "one flip per click");
        step(&mut world, (50.0, 20.0), true);
        step(&mut world, (50.0, 20.0), false);
        assert!(!t(&world).on, "the second click flips it back");
    }

    /// The scrollbar: its thumb is the visible share of the content, dragging it scrolls in
    /// proportion (captured past the box, clamped), a press on the track jumps there, and the
    /// release lets go — and a press in the bar's strip is not a press on the list item under it.
    #[test]
    fn the_scrollbar_thumb_drags_the_list_and_owns_its_strip() {
        use gizmo_core::hierarchy::HierarchyExt;
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        let list_node = node((0.0, 0.0), (100.0, 100.0));
        let mut scroll = Scroll::default();
        scroll.fit(Vec2::new(0.0, 300.0)); // 400 px of content in a 100 px box
        let list = world.spawn_bundle((list_node, scroll));
        let item = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 0.0), (100.0, 40.0)), ..Default::default() });
        world.add_child(list, item);

        let (at, size) = scroll.thumb(&list_node).expect("it overflows");
        assert_eq!((at, size), (Vec2::new(92.0, 0.0), Vec2::new(6.0, 25.0)), "a quarter of the track");

        let run = |w: &mut gizmo_core::World, p: (f32, f32), down: bool| {
            frame(w, p, down);
            let mut s = Schedule::new();
            s.add_di_system(ui_scroll_system.into_config());
            s.build();
            s.run(w, 0.0);
        };
        let offset = |w: &gizmo_core::World| w.borrow::<Scroll>().get(list.id()).unwrap().offset.y;
        run(&mut world, (95.0, 10.0), true); // on the thumb, 10 px into it
        assert_eq!(world.borrow::<Interaction>().get(item.id()), Some(&Interaction::None), "the press went to the item under the bar");
        run(&mut world, (300.0, 47.5), true); // thumb top to 37.5 of 75 travel, off the box
        assert!((offset(&world) - 150.0).abs() < 1e-3, "{}", offset(&world));
        run(&mut world, (95.0, 500.0), true);
        assert_eq!(offset(&world), 300.0, "clamped at the end");
        run(&mut world, (95.0, 500.0), false);
        run(&mut world, (95.0, 0.0), false);
        assert_eq!(offset(&world), 300.0, "released: moving does not scroll");
        // A press on the item itself, left of the strip, is the item's.
        run(&mut world, (50.0, 10.0), true);
        assert_eq!(world.borrow::<Interaction>().get(item.id()), Some(&Interaction::Pressed));
    }

    /// A dropdown opens on a click, takes the pointer while open (the button under its list is
    /// neither hovered nor clicked), picks on a press on an option, closes without a change on a
    /// press elsewhere, and steps with ←/→ while focused.
    #[test]
    fn a_dropdown_opens_picks_and_owns_the_pointer_while_open() {
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        world.insert_resource(UiNavigation { enabled: true, ..Default::default() });
        let dd = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 0.0), (100.0, 30.0)), ..Default::default() });
        world.add_component(dd, Dropdown::new(["Low", "Medium", "High"], 0));
        let below = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 40.0), (100.0, 30.0)), ..Default::default() });
        let run = |w: &mut gizmo_core::World, at: (f32, f32), down: bool, key_name: Option<&str>| {
            {
                let mut input = w.get_resource_mut::<Input>().unwrap();
                input.begin_frame();
                input.set_mouse_position(at.0, at.1);
                if down { input.on_mouse_button_pressed(0) } else { input.on_mouse_button_released(0) }
                if let Some(k) = key_name { input.on_key_pressed(key(k)) }
            }
            let mut s = Schedule::new();
            s.add_di_system(ui_interaction_system.into_config().label("i"));
            s.add_di_system(ui_dropdown_system.into_config().after("i"));
            s.build();
            s.run(w, 0.0);
            if let Some(k) = key_name {
                w.get_resource_mut::<Input>().unwrap().on_key_released(key(k));
            }
        };
        let get = |w: &gizmo_core::World| w.borrow::<Dropdown>().get(dd.id()).unwrap().clone();

        run(&mut world, (50.0, 15.0), true, None);
        run(&mut world, (50.0, 15.0), false, None);
        assert!(get(&world).is_open(), "a click opens it");
        run(&mut world, (50.0, 45.0), false, None);
        assert_eq!(get(&world).hovered(), Some(0));
        assert_eq!(world.borrow::<Interaction>().get(below.id()), Some(&Interaction::None), "the list let the pointer through");
        run(&mut world, (50.0, 75.0), true, None); // on "Medium", over nothing else... and on `below`'s edge row
        let d = get(&world);
        assert!(!d.is_open() && d.selected == 1 && d.just_changed() && d.selected_option() == Some("Medium"));
        run(&mut world, (50.0, 75.0), false, None);
        assert!(!world.borrow::<Clicked>().get(below.id()).unwrap().0, "picking an option clicked what was under it");

        // Open, then press elsewhere: closed, unchanged.
        run(&mut world, (50.0, 15.0), true, None);
        run(&mut world, (50.0, 15.0), false, None);
        run(&mut world, (500.0, 500.0), true, None);
        let d = get(&world);
        assert!(!d.is_open() && d.selected == 1 && !d.just_changed());
        run(&mut world, (500.0, 500.0), false, None);

        // The keyboard steps it while focused, wrapping.
        world.get_resource_mut::<UiNavigation>().unwrap().focus(Some(dd.id()));
        run(&mut world, (500.0, 500.0), false, Some("right"));
        assert_eq!(get(&world).selected, 2);
        run(&mut world, (500.0, 500.0), false, Some("right"));
        assert_eq!(get(&world).selected, 0, "wraps");
        assert_eq!(world.get_resource::<UiNavigation>().unwrap().focused(), Some(dd.id()), "→ moved the focus off it");
    }

    /// A finger on a list: a tap still clicks the item; a drag past the slop scrolls the content
    /// with the finger and cancels the item's click; let go, it slides on and slows to a stop; and
    /// the same drag with the mouse does not scroll.
    #[test]
    fn a_finger_flicks_a_list_and_a_tap_still_taps() {
        use gizmo_core::hierarchy::HierarchyExt;
        use gizmo_core::input::TouchPhase::{self, *};
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        world.insert_resource(gizmo_core::time::Time::new());
        let mut scroll = Scroll::default();
        scroll.fit(Vec2::new(0.0, 1000.0));
        let list = world.spawn_bundle((node((0.0, 0.0), (100.0, 200.0)), scroll));
        let item = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, 0.0), (80.0, 40.0)), ..Default::default() });
        world.add_child(list, item);
        let step = |w: &mut gizmo_core::World, touch: Option<(TouchPhase, f32)>| {
            {
                let mut input = w.get_resource_mut::<Input>().unwrap();
                input.begin_frame();
                if let Some((phase, y)) = touch {
                    input.on_touch(1, phase, 40.0, y);
                }
            }
            w.get_resource_mut::<gizmo_core::time::Time>().unwrap().update(1.0 / 60.0);
            let mut s = Schedule::new();
            s.add_di_system(ui_scroll_system.into_config().label("s"));
            s.add_di_system(ui_interaction_system.into_config().after("s"));
            s.build();
            s.run(w, 0.0);
        };
        let offset = |w: &gizmo_core::World| w.borrow::<Scroll>().get(list.id()).unwrap().offset.y;
        let clicked = |w: &gizmo_core::World| w.borrow::<Clicked>().get(item.id()).unwrap().0;

        // A tap: down and up in place — the item is clicked, nothing scrolls.
        step(&mut world, Some((Started, 20.0)));
        step(&mut world, Some((Ended, 20.0)));
        assert!(clicked(&world) && offset(&world) == 0.0, "a tap on a list item stopped being a tap");
        step(&mut world, None);

        // A drag down that ends over the item: past the slop the content follows the finger, and
        // the release — over the item it started on — clicks nothing.
        world.borrow_mut::<Scroll>().get_mut(list.id()).unwrap().offset.y = 500.0;
        step(&mut world, Some((Started, 5.0)));
        for y in [9.0, 16.0, 25.0, 35.0] {
            step(&mut world, Some((Moved, y)));
        }
        let dragged = offset(&world);
        assert!(dragged <= 485.0, "the content did not follow the finger: {dragged}");
        step(&mut world, Some((Ended, 35.0)));
        assert!(!clicked(&world), "a scroll ended by clicking the item it was let go over");
        // The fling: it keeps going the same way, slower and slower, then stops.
        step(&mut world, None);
        let a = offset(&world);
        for _ in 0..200 {
            step(&mut world, None);
        }
        let b = offset(&world);
        step(&mut world, None);
        assert!(a < dragged && b < a, "no slide after the release: {dragged} {a} {b}");
        assert_eq!(offset(&world), b, "it never stopped");
        assert!(b > 0.0, "it slid all the way to the edge — the fling does not slow down: {b}");

        // The mouse dragging across the list does not scroll it.
        let before = offset(&world);
        for (y, down) in [(100.0, true), (60.0, true), (20.0, true), (20.0, false)] {
            {
                let mut input = world.get_resource_mut::<Input>().unwrap();
                input.begin_frame();
                input.set_mouse_position(40.0, y);
                if down { input.on_mouse_button_pressed(0) } else { input.on_mouse_button_released(0) }
            }
            let mut s = Schedule::new();
            s.add_di_system(ui_scroll_system.into_config());
            s.build();
            s.run(&mut world, 0.0);
        }
        assert_eq!(offset(&world), before, "a mouse drag scrolled the list");
    }

    /// Holding ↓ walks down the list: one step on the press, then — after the delay — one every
    /// interval, as every menu does; and two taps in consecutive frames are two steps, not one
    /// long hold.
    #[test]
    fn a_held_arrow_repeats_and_quick_taps_all_count() {
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        let (mut world, [top, mid, low]) = menu();
        world.insert_resource(gizmo_core::time::Time::new());
        let frame_with = |w: &mut gizmo_core::World, press: bool, hold: bool| {
            {
                let mut input = w.get_resource_mut::<Input>().unwrap();
                input.begin_frame();
                input.set_mouse_position(500.0, 500.0);
                if press {
                    input.on_key_pressed(key("down"));
                }
                if !hold {
                    input.on_key_released(key("down"));
                }
            }
            w.get_resource_mut::<gizmo_core::time::Time>().unwrap().update(1.0 / 60.0);
            let mut s = Schedule::new();
            s.add_di_system(ui_interaction_system.into_config());
            s.build();
            s.run(w, 0.0);
        };
        // Held for one second: the press, then repeats from 0.4 s every 0.1 s.
        let mut seen = vec![];
        frame_with(&mut world, true, true);
        seen.push(focused(&world));
        for _ in 0..60 {
            frame_with(&mut world, false, true);
            if focused(&world) != *seen.last().unwrap() {
                seen.push(focused(&world));
            }
        }
        assert!(seen.len() >= 6, "holding ↓ did not repeat: {seen:?}");
        assert_eq!(&seen[..3], &[Some(top), Some(mid), Some(low)], "{seen:?}");
        frame_with(&mut world, false, false);

        // Two taps in consecutive frames.
        world.get_resource_mut::<UiNavigation>().unwrap().focus(Some(top));
        frame_with(&mut world, true, false);
        frame_with(&mut world, true, false);
        assert_eq!(focused(&world), Some(low), "the second quick tap was swallowed as a hold");
    }

    /// Moving the focus with the keys onto an item scrolled out of sight scrolls the list to show
    /// it — below the box, so its bottom meets the box's; above, so its top does.
    #[test]
    fn the_keys_scroll_the_focused_item_into_view() {
        use gizmo_core::hierarchy::HierarchyExt;
        use gizmo_core::system::{IntoSystemConfig, Schedule};
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        world.insert_resource(UiNavigation { enabled: true, ..Default::default() });
        let mut scroll = Scroll::default();
        scroll.fit(Vec2::new(0.0, 300.0));
        let list = world.spawn_bundle((node((0.0, 0.0), (100.0, 100.0)), scroll));
        // Laid out as if unscrolled: four 40 px items stacked at 0, 50, 100, 150.
        let items: Vec<u32> = (0..4)
            .map(|i| {
                let e = world.spawn_bundle(crate::ButtonBundle { node: node((0.0, i as f32 * 50.0), (100.0, 40.0)), ..Default::default() });
                world.add_child(list, e);
                e.id()
            })
            .collect();
        let offset = |w: &gizmo_core::World| w.borrow::<Scroll>().get(list.id()).unwrap().offset.y;
        let run = |w: &mut gizmo_core::World, k: Option<&str>| {
            press(w, k);
            let mut s = Schedule::new();
            s.add_di_system(ui_scroll_system.into_config());
            s.build();
            s.run(w, 0.0);
        };
        world.get_resource_mut::<UiNavigation>().unwrap().focus(Some(items[2]));
        run(&mut world, Some("down")); // onto the item at 150..190, below the 0..100 box
        assert_eq!(focused(&world), Some(items[3]));
        assert_eq!(offset(&world), 90.0, "its bottom (190) should meet the box's (100)");
        // Scrolled now, the layout would have moved the items; move them as it would.
        for (i, id) in items.iter().enumerate() {
            world.borrow_mut::<Node>().get_mut(*id).unwrap().position.y = i as f32 * 50.0 - 90.0;
        }
        run(&mut world, Some("down")); // wraps to the first item, at -90, above the box
        assert_eq!(focused(&world), Some(items[0]));
        assert_eq!(offset(&world), 0.0, "its top should meet the box's");
        // A focus moved by the mouse does not scroll.
        run(&mut world, None);
        assert_eq!(offset(&world), 0.0);
    }

    /// Pressed on, dragged off, released: cancelled, like a desktop button.
    #[test]
    fn releasing_off_the_button_cancels_the_click() {
        let (mut world, b, _) = button_world();
        // Checked after EVERY frame: a click reported while dragging off is still a click.
        for (at, down) in [((50.0, 20.0), true), ((150.0, 20.0), true), ((150.0, 20.0), false), ((150.0, 20.0), false)] {
            frame(&mut world, at, down);
            assert!(!clicked(&world, b), "clicked at {at:?}, button down = {down}");
        }
    }

    #[test]
    fn shared_edge_belongs_to_exactly_one_node() {
        // Element A spans x=[0,100), element B spans x=[100,200); both share the
        // edge at x=100. The point on that edge must hover exactly one element.
        let a = node((0.0, 0.0), (100.0, 50.0));
        let b = node((100.0, 0.0), (100.0, 50.0));

        let point = (100.0, 25.0);
        assert!(!point_in_node(point, &a), "left node must not claim the shared right edge");
        assert!(point_in_node(point, &b), "right node owns its left edge");
    }

    #[test]
    fn interior_and_lower_bounds_are_inclusive() {
        let n = node((10.0, 20.0), (30.0, 40.0));
        // Top-left corner (inclusive lower bound) is inside.
        assert!(point_in_node((10.0, 20.0), &n));
        // Interior point is inside.
        assert!(point_in_node((25.0, 40.0), &n));
        // Exclusive upper bounds are outside.
        assert!(!point_in_node((40.0, 40.0), &n));
        assert!(!point_in_node((25.0, 60.0), &n));
    }

    #[test]
    fn hit_test_requires_both_axes_inside() {
        // A hit is the logical AND of the two per-axis intervals: being inside on
        // one axis but outside on the other must never register as a hit.
        let n = node((0.0, 0.0), (100.0, 100.0));
        assert!(!point_in_node((50.0, 150.0), &n), "x inside, y above upper");
        assert!(!point_in_node((150.0, 50.0), &n), "y inside, x past right");
        assert!(!point_in_node((-1.0, 50.0), &n), "y inside, x below left");
        assert!(!point_in_node((50.0, -1.0), &n), "x inside, y below top");
        // Only when both axes are inside is it a hit.
        assert!(point_in_node((50.0, 50.0), &n));
    }

    #[test]
    fn degenerate_boxes_contain_no_point() {
        // With the half-open [pos, pos+size) rule a zero-size box is empty: its
        // own corner is excluded because the upper bound equals the lower bound.
        let empty = node((30.0, 40.0), (0.0, 0.0));
        assert!(!point_in_node((30.0, 40.0), &empty));
        assert!(!point_in_node((29.999, 40.0), &empty));
        // A box degenerate on a single axis is empty too, even on-axis.
        let flat = node((0.0, 0.0), (100.0, 0.0)); // zero height
        assert!(!point_in_node((50.0, 0.0), &flat));
        let thin = node((0.0, 0.0), (0.0, 100.0)); // zero width
        assert!(!point_in_node((0.0, 50.0), &thin));
    }

    #[test]
    fn negative_positions_are_hit_tested_correctly() {
        // Node spanning [-50, 50) on both axes; the math must not assume the
        // origin is non-negative.
        let n = node((-50.0, -50.0), (100.0, 100.0));
        assert!(point_in_node((-50.0, -50.0), &n), "inclusive lower corner");
        assert!(point_in_node((-25.0, 0.0), &n));
        assert!(point_in_node((49.9, 49.9), &n));
        assert!(!point_in_node((50.0, 0.0), &n), "exclusive upper x");
        assert!(!point_in_node((0.0, 50.0), &n), "exclusive upper y");
        assert!(!point_in_node((-51.0, 0.0), &n), "below lower x");
    }

    #[test]
    fn adjacent_cells_tile_the_plane_as_a_partition() {
        // A 2x2 grid of 10x10 cells covering [0,20) x [0,20). The half-open
        // interval rule guarantees a *partition*: every point in the covered
        // region belongs to EXACTLY one cell — no gaps on shared edges, no
        // double-coverage. This is the core reason the interval is half-open.
        let cells = [
            node((0.0, 0.0), (10.0, 10.0)),
            node((10.0, 0.0), (10.0, 10.0)),
            node((0.0, 10.0), (10.0, 10.0)),
            node((10.0, 10.0), (10.0, 10.0)),
        ];
        for i in 0..20 {
            for j in 0..20 {
                let p = (i as f32, j as f32);
                let hits = cells.iter().filter(|c| point_in_node(p, c)).count();
                assert_eq!(hits, 1, "point {p:?} must land in exactly one cell (got {hits})");
            }
        }
        // Points just past the far edges belong to no cell (upper bound exclusive).
        assert_eq!(cells.iter().filter(|c| point_in_node((20.0, 5.0), c)).count(), 0);
        assert_eq!(cells.iter().filter(|c| point_in_node((5.0, 20.0), c)).count(), 0);
    }
}
