//! On-screen controls for touchscreens: [`VirtualStick`] and [`VirtualButton`].
//!
//! A twin-stick game on a phone needs two thumbs at once — one moving, one aiming or firing — and
//! the first finger is the only one that drives the mouse, so nothing built on
//! [`Interaction`] can follow the second. These controls read
//! [`Input::touches`](gizmo_core::input::Input::touches) directly: each one **claims the finger
//! that lands on it** and follows that finger until it lifts, wherever it goes, so a stick and a
//! button are held by two fingers independently. With no fingers on the screen they answer to the
//! mouse instead, so a desktop build can be played and tested with them.
//!
//! Put one on a UI node (a [`NodeBundle`] is enough — they need no
//! `Interaction`): the node's box is where a finger lands on it. Under `gizmo-engine` a stick's
//! knob is drawn over its background, at where the stick points.

use gizmo_core::input::Input;
use gizmo_core::query::{Mut, Query};
use gizmo_core::system::Res;
use gizmo_math::{Vec2, Vec4};

use crate::components::Node;

/// The mouse, when it stands in for a finger.
const MOUSE: u64 = u64::MAX;

/// A thumbstick drawn on the screen: [`value`](Self::value) is where the finger holding it is,
/// relative to where it came down, scaled by [`radius`](Self::radius) and clamped to the unit
/// circle — `y` **up**, like a gamepad's stick, so it can be used in place of one.
///
/// The stick is centred where the finger lands (a "floating" stick): a thumb that comes down a
/// little off the drawn centre does not start with a push it did not mean.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualStick {
    /// How far, in logical pixels, the finger travels for a full push.
    pub radius: f32,
    /// The knob's colour (under `gizmo-engine`).
    pub knob: Vec4,
    value: Vec2,
    finger: Option<u64>,
    centre: Vec2,
    /// Whether it has had a box yet — see [`touch_controls_system`] for what that changes.
    laid_out: bool,
}

impl VirtualStick {
    /// A stick at rest, `radius` pixels for a full push.
    #[must_use]
    pub fn new(radius: f32) -> Self {
        Self { radius: radius.max(1.0), knob: Vec4::new(1.0, 1.0, 1.0, 0.6), value: Vec2::ZERO, finger: None, centre: Vec2::ZERO, laid_out: false }
    }

    /// Where it points: `-1..=1` on each axis, inside the unit circle, `y` up. Zero when no finger
    /// holds it.
    #[must_use]
    pub fn value(&self) -> Vec2 {
        self.value
    }

    /// Whether a finger (or the mouse) holds it.
    #[must_use]
    pub fn is_held(&self) -> bool {
        self.finger.is_some()
    }

    /// Where the finger came down, window pixels — the knob's rest point while held.
    #[must_use]
    pub fn centre(&self) -> Vec2 {
        self.centre
    }
}

/// A button drawn on the screen that any finger can hold, independently of the others.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VirtualButton {
    finger: Option<u64>,
    just_pressed: bool,
    just_released: bool,
    /// Whether it has had a box yet — see [`touch_controls_system`] for what that changes.
    laid_out: bool,
}

impl VirtualButton {
    /// Held right now.
    #[must_use]
    pub fn is_pressed(&self) -> bool {
        self.finger.is_some()
    }

    /// Pressed this frame.
    #[must_use]
    pub fn just_pressed(&self) -> bool {
        self.just_pressed
    }

    /// Released this frame.
    #[must_use]
    pub fn just_released(&self) -> bool {
        self.just_released
    }
}

gizmo_core::impl_component!(VirtualStick, VirtualButton);

/// One pointer this frame: id, position, came down, lifted.
type Pointer = (u64, Vec2, bool, bool);

/// The fingers — or, with none on the screen, the mouse as one.
fn pointers(input: &Input) -> Vec<Pointer> {
    let touches = input.touches();
    if !touches.is_empty() {
        return touches
            .iter()
            .map(|t| (t.id, Vec2::new(t.position.0, t.position.1), t.just_started, t.just_ended))
            .collect();
    }
    let (x, y) = input.mouse_position();
    let down = input.is_mouse_button_pressed(0) || input.is_mouse_button_just_released(0);
    if !down {
        return Vec::new();
    }
    vec![(MOUSE, Vec2::new(x, y), input.is_mouse_button_just_pressed(0), input.is_mouse_button_just_released(0))]
}

/// Layout has given it somewhere to be touched.
fn has_box(node: &Node) -> bool {
    node.size.x > 0.0 && node.size.y > 0.0
}

fn inside(p: Vec2, node: &Node) -> bool {
    p.x >= node.position.x && p.y >= node.position.y && p.x < node.position.x + node.size.x && p.y < node.position.y + node.size.y
}

/// Advances every [`VirtualStick`] and [`VirtualButton`] from this frame's fingers (or mouse).
/// A finger already holding one control is not claimed by another.
///
/// A control claims a finger that **lands** on it — and, on the first frame it has a box, one
/// already down inside it that nothing holds. Without that second half, a control made in answer
/// to a touch could never have that touch: it has no box until layout runs, and by the next frame
/// the finger is no longer landing. `nisanci` makes its sticks on the first touch, and on a phone
/// that touch did nothing; the thumb had to lift and come down again. A finger that slides onto a
/// control that has been on the screen all along still claims nothing.
pub fn touch_controls_system(
    input: Res<Input>,
    mut sticks: Query<(&Node, Mut<VirtualStick>)>,
    mut buttons: Query<(&Node, Mut<VirtualButton>)>,
    display: Option<Res<gizmo_core::window::DisplayScale>>,
) {
    // A thumb travels the same distance on every screen: the radius is logical pixels, and fingers
    // arrive in frame pixels.
    let density = display.map_or(1.0, |d| d.get());
    let pointers = pointers(&input);
    let held_by = |id: u64| pointers.iter().find(|p| p.0 == id && !p.3).map(|p| p.1);
    let mut claimed: Vec<u64> = Vec::new();
    for (_, s) in sticks.iter_mut() {
        claimed.extend(s.finger);
    }
    for (_, b) in buttons.iter_mut() {
        claimed.extend(b.finger);
    }

    for (node, mut stick) in sticks.iter_mut() {
        let mut next = *stick;
        if let Some(f) = next.finger {
            match held_by(f) {
                Some(p) => {
                    let v = (p - next.centre) / (next.radius * density);
                    let v = Vec2::new(v.x, -v.y);
                    next.value = if v.length() > 1.0 { v.normalize() } else { v };
                }
                None => {
                    next.finger = None;
                    next.value = Vec2::ZERO;
                }
            }
        }
        let fresh = !next.laid_out;
        if next.finger.is_none() {
            if let Some(&(id, p, _, _)) =
                pointers.iter().find(|p| (p.2 || fresh) && !p.3 && !claimed.contains(&p.0) && inside(p.1, node))
            {
                next.finger = Some(id);
                next.centre = p;
                next.value = Vec2::ZERO;
                claimed.push(id);
            }
        }
        next.laid_out |= has_box(node);
        if next != *stick {
            *stick = next;
        }
    }

    for (node, mut button) in buttons.iter_mut() {
        let mut next = *button;
        next.just_pressed = false;
        next.just_released = false;
        if let Some(f) = next.finger {
            if held_by(f).is_none() {
                next.finger = None;
                next.just_released = true;
            }
        }
        let fresh = !next.laid_out;
        if next.finger.is_none() {
            // A finger lifting over a control that has just appeared is not a tap on it: it did
            // not come down there, and it is not staying.
            if let Some(&(id, _, _, lifted)) =
                pointers.iter().find(|p| (p.2 || (fresh && !p.3)) && !claimed.contains(&p.0) && inside(p.1, node))
            {
                // A tap inside one frame is a press and a release, both reported.
                next.just_pressed = true;
                if lifted {
                    next.just_released = true;
                } else {
                    next.finger = Some(id);
                    claimed.push(id);
                }
            }
        }
        next.laid_out |= has_box(node);
        if next != *button {
            *button = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::input::TouchPhase::*;
    use gizmo_core::system::{IntoSystemConfig, Schedule};

    fn node(x: f32, y: f32, w: f32, h: f32) -> Node {
        Node { position: Vec2::new(x, y), size: Vec2::new(w, h) }
    }

    fn run(world: &mut gizmo_core::World) {
        let mut s = Schedule::new();
        s.add_di_system(touch_controls_system.into_config());
        s.build();
        s.run(world, 0.0);
        world.get_resource_mut::<Input>().unwrap().begin_frame();
    }

    fn touch(world: &mut gizmo_core::World, id: u64, phase: gizmo_core::input::TouchPhase, x: f32, y: f32) {
        world.get_resource_mut::<Input>().unwrap().on_touch(id, phase, x, y);
    }

    fn pad() -> (gizmo_core::World, u32, u32) {
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        let stick = world.spawn_bundle((node(0.0, 400.0, 200.0, 200.0), VirtualStick::new(50.0))).id();
        let fire = world.spawn_bundle((node(600.0, 400.0, 200.0, 200.0), VirtualButton::default())).id();
        (world, stick, fire)
    }

    /// Two thumbs at once: one pushes the stick, the other holds the button, and each follows its
    /// own finger — the stick even past its box, the second finger never moving the stick.
    #[test]
    fn a_stick_and_a_button_are_held_by_two_fingers() {
        let (mut world, stick, fire) = pad();
        let s = |w: &gizmo_core::World| *w.borrow::<VirtualStick>().get(stick).unwrap();
        let b = |w: &gizmo_core::World| *w.borrow::<VirtualButton>().get(fire).unwrap();

        touch(&mut world, 1, Started, 100.0, 500.0);
        run(&mut world);
        assert!(s(&world).is_held() && s(&world).value() == Vec2::ZERO, "a finger landing is not a push");
        touch(&mut world, 1, Moved, 125.0, 500.0);
        touch(&mut world, 2, Started, 700.0, 500.0);
        run(&mut world);
        assert_eq!(s(&world).value(), Vec2::new(0.5, 0.0));
        assert!(b(&world).is_pressed() && b(&world).just_pressed());
        touch(&mut world, 1, Moved, 100.0, 300.0); // up and out of its box
        touch(&mut world, 2, Moved, 900.0, 900.0);
        run(&mut world);
        assert!((s(&world).value() - Vec2::new(0.0, 1.0)).length() < 1e-5, "y is up, and clamped: {:?}", s(&world).value());
        assert!(b(&world).is_pressed() && !b(&world).just_pressed());
        touch(&mut world, 2, Ended, 900.0, 900.0);
        run(&mut world);
        assert!(b(&world).just_released() && !b(&world).is_pressed());
        assert!(s(&world).is_held(), "the button's finger lifting let go of the stick");
        touch(&mut world, 1, Ended, 100.0, 300.0);
        run(&mut world);
        assert!(!s(&world).is_held() && s(&world).value() == Vec2::ZERO);
    }

    /// A finger landing outside, or one already holding the stick sliding over the button, claims
    /// nothing; a tap inside one frame is a press and a release.
    #[test]
    fn only_a_finger_landing_on_a_control_claims_it() {
        let (mut world, stick, fire) = pad();
        let b = |w: &gizmo_core::World| *w.borrow::<VirtualButton>().get(fire).unwrap();
        touch(&mut world, 5, Started, 400.0, 100.0);
        run(&mut world);
        assert!(!world.borrow::<VirtualStick>().get(stick).unwrap().is_held());
        touch(&mut world, 1, Started, 100.0, 500.0);
        run(&mut world);
        touch(&mut world, 1, Moved, 700.0, 500.0);
        run(&mut world);
        assert!(!b(&world).is_pressed(), "the stick's finger slid onto the button and pressed it");
        touch(&mut world, 9, Started, 700.0, 500.0);
        touch(&mut world, 9, Ended, 700.0, 500.0);
        run(&mut world);
        assert!(b(&world).just_pressed() && b(&world).just_released() && !b(&world).is_pressed());
    }

    /// A control made in answer to a touch takes that touch: on its first frame with a box it
    /// claims a finger already down inside it. Until layout gives it a box there is nothing to be
    /// inside, and after that first frame a finger has to land on it again.
    #[test]
    fn a_control_made_under_a_finger_takes_it() {
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        // The finger comes down before any control exists.
        touch(&mut world, 1, Started, 100.0, 500.0);
        touch(&mut world, 2, Started, 700.0, 500.0);
        run(&mut world);
        // Made in answer to it, with no box yet: layout has not run.
        let stick = world.spawn_bundle((node(0.0, 0.0, 0.0, 0.0), VirtualStick::new(50.0))).id();
        let fire = world.spawn_bundle((node(0.0, 0.0, 0.0, 0.0), VirtualButton::default())).id();
        run(&mut world);
        let s = |w: &gizmo_core::World| *w.borrow::<VirtualStick>().get(stick).unwrap();
        let b = |w: &gizmo_core::World| *w.borrow::<VirtualButton>().get(fire).unwrap();
        assert!(!s(&world).is_held() && !b(&world).is_pressed(), "claimed with no box to be inside");
        // Layout places them; both fingers are still down.
        *world.borrow_mut::<Node>().get_mut(stick).unwrap() = node(0.0, 400.0, 200.0, 200.0);
        *world.borrow_mut::<Node>().get_mut(fire).unwrap() = node(600.0, 400.0, 200.0, 200.0);
        touch(&mut world, 1, Moved, 120.0, 500.0);
        run(&mut world);
        assert!(s(&world).is_held(), "the stick made under a finger did not take it");
        assert_eq!(s(&world).centre(), Vec2::new(120.0, 500.0), "centred where the finger is, not a push");
        assert!(b(&world).is_pressed() && b(&world).just_pressed(), "the button made under a finger did not take it");
        touch(&mut world, 1, Moved, 145.0, 500.0);
        run(&mut world);
        assert_eq!(s(&world).value(), Vec2::new(0.5, 0.0));
        // After its first frame a resting finger no longer counts: lift and slide back in.
        touch(&mut world, 2, Ended, 700.0, 500.0);
        run(&mut world);
        touch(&mut world, 3, Started, 400.0, 100.0);
        run(&mut world);
        touch(&mut world, 3, Moved, 700.0, 500.0);
        run(&mut world);
        assert!(!b(&world).is_pressed(), "a finger sliding onto a settled button pressed it");
    }

    /// A full push is the same distance for a thumb on any screen: fingers arrive in frame pixels,
    /// and on a dense display 50 logical pixels are more of them.
    #[test]
    fn a_full_push_is_the_radius_in_logical_pixels() {
        let (mut world, stick, _) = pad();
        world.insert_resource(gizmo_core::window::DisplayScale::from_window(800.0, 400.0, 1.0));
        touch(&mut world, 1, Started, 100.0, 500.0);
        run(&mut world);
        touch(&mut world, 1, Moved, 150.0, 500.0); // 50 frame pixels: half of 50 logical at density 2
        run(&mut world);
        assert_eq!(world.borrow::<VirtualStick>().get(stick).unwrap().value(), Vec2::new(0.5, 0.0));
    }

    /// A finger lifting over a control on its first frame is no tap.
    #[test]
    fn a_finger_lifting_over_a_new_button_is_not_a_tap() {
        let mut world = gizmo_core::World::new();
        world.insert_resource(Input::new());
        touch(&mut world, 1, Started, 700.0, 500.0);
        run(&mut world);
        let fire = world.spawn_bundle((node(600.0, 400.0, 200.0, 200.0), VirtualButton::default())).id();
        touch(&mut world, 1, Ended, 700.0, 500.0);
        run(&mut world);
        let b = *world.borrow::<VirtualButton>().get(fire).unwrap();
        assert!(!b.just_pressed() && !b.just_released() && !b.is_pressed());
    }

    /// With no fingers on the screen, the mouse drives them.
    #[test]
    fn the_mouse_stands_in_without_fingers() {
        let (mut world, stick, _) = pad();
        {
            let mut input = world.get_resource_mut::<Input>().unwrap();
            input.set_mouse_position(100.0, 500.0);
            input.on_mouse_button_pressed(0);
        }
        run(&mut world);
        world.get_resource_mut::<Input>().unwrap().on_mouse_moved(100.0, 550.0);
        run(&mut world);
        assert_eq!(world.borrow::<VirtualStick>().get(stick).unwrap().value(), Vec2::new(0.0, -1.0));
        world.get_resource_mut::<Input>().unwrap().on_mouse_button_released(0);
        run(&mut world);
        assert!(!world.borrow::<VirtualStick>().get(stick).unwrap().is_held());
    }
}
