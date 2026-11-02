use gizmo_core::query::{Query, Mut};
use gizmo_core::system::{Res, ResMut};
use gizmo_core::input::{GamepadButton, Input};
use crate::components::{Clicked, Interaction, Node};

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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UiNavigation {
    /// Whether the keys above navigate. See the type's docs for why it is off by default.
    pub enabled: bool,
    focused: Option<u32>,
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
}

/// System that updates each element's [`Interaction`] state from the current
/// mouse position and button state, and its [`Clicked`] signal where it has one.
pub fn ui_interaction_system(
    input: Res<Input>,
    navigation: Option<ResMut<UiNavigation>>,
    mut interactions: Query<(&Node, Mut<Interaction>, Option<Mut<Clicked>>)>,
) {
    let mouse_pos = input.mouse_position();
    let is_clicked = input.is_mouse_button_pressed(0); // 0 is left click

    // Keyboard/gamepad focus — decided before the per-element pass so it can mark the focused one.
    let (focused, activate) = match navigation {
        Some(mut nav) if nav.enabled => navigate(&input, &mut nav, &mut interactions, mouse_pos),
        _ => (None, false),
    };

    // Note: This is a simplified check that doesn't account for z-index or hierarchy properly yet.
    // In a real UI system, we would need to walk the tree from front to back.
    for (id, (node, mut interaction, clicked)) in interactions.iter_mut() {
        let was = *interaction;
        let is_hovered = point_in_node(mouse_pos, node);

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
    mouse: (f32, f32),
) -> (Option<u32>, bool) {
    // The focusable elements in reading order. A zero-size node is hidden (or not laid out yet).
    let mut order: Vec<(u32, f32, f32, bool)> = interactions
        .iter_mut()
        .filter(|(_, (n, _, _))| n.size.x > 0.0 && n.size.y > 0.0)
        .map(|(id, (n, _, _))| (id, n.position.y, n.position.x, point_in_node(mouse, n)))
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
    let next = (tab && !shift)
        || input.is_key_just_pressed(key("down"))
        || input.is_key_just_pressed(key("right"))
        || pad_just(GamepadButton::DPadDown)
        || pad_just(GamepadButton::DPadRight);
    let prev = (tab && shift)
        || input.is_key_just_pressed(key("up"))
        || input.is_key_just_pressed(key("left"))
        || pad_just(GamepadButton::DPadUp)
        || pad_just(GamepadButton::DPadLeft);
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
        world.insert_resource(UiNavigation { enabled: true, focused: None });
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
