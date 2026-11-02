//! Touch input — fingers on a touchscreen, and the first one standing in for the mouse.
//!
//! The windowed loop dropped every `WindowEvent::Touch`, so on a phone (the browser build runs on
//! one) or a touchscreen laptop nothing could be pressed at all. Now each finger is a [`Touch`],
//! and the **first finger down drives the mouse**: it moves the cursor and holds the left button,
//! so `Clicked`, `pointer_over_ui`, `gizmo::pointer` and every game written against the mouse
//! work under a finger without a line changed. A game that wants several fingers — two-finger
//! pan, a virtual stick and a fire button at once — reads [`Input::touches`].

use serde::{Deserialize, Serialize};

/// Where a finger is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TouchPhase {
    /// It came down.
    Started,
    /// It moved.
    Moved,
    /// It lifted.
    Ended,
    /// The platform took it away (a system gesture, the window losing focus) — treated as a lift.
    Cancelled,
}

/// One finger on the screen. Positions are window pixels, like the mouse's.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Touch {
    /// The platform's id for this finger, stable while it is down.
    pub id: u64,
    /// Where it is now.
    pub position: (f32, f32),
    /// Where it came down.
    pub start: (f32, f32),
    /// How far it moved this frame.
    pub delta: (f32, f32),
    /// It came down this frame.
    pub just_started: bool,
    /// It lifted (or was cancelled) this frame; it is gone next frame.
    pub just_ended: bool,
}

/// The touch half of [`Input`](super::Input): the fingers, and which one is the mouse.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Touches {
    pub(crate) fingers: Vec<Touch>,
    /// The finger driving the mouse, while it is down.
    pub(crate) primary: Option<u64>,
}

impl Touches {
    /// Drops the fingers that lifted last frame and clears this frame's edges and deltas.
    pub(crate) fn begin_frame(&mut self) {
        self.fingers.retain(|t| !t.just_ended);
        for t in &mut self.fingers {
            t.just_started = false;
            t.delta = (0.0, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TouchPhase::*;
    use crate::input::{mouse, Input};

    /// One frame boundary, the way the loop does it.
    fn next(input: &mut Input) {
        input.begin_frame();
    }

    /// A tap is a click: the first finger presses the left button where it lands — with no flick
    /// in `mouse_delta` from the cursor's old place — and lifting releases it.
    #[test]
    fn a_tap_is_a_left_click_where_the_finger_landed() {
        let mut input = Input::new();
        input.set_mouse_position(10.0, 10.0);
        input.on_touch(7, Started, 300.0, 200.0);
        assert!(input.is_mouse_button_just_pressed(mouse::LEFT));
        assert_eq!(input.mouse_position(), (300.0, 200.0));
        assert_eq!(input.mouse_delta(), (0.0, 0.0), "landing is not a 290-pixel flick");
        let t = input.primary_touch().expect("the finger drives the mouse");
        assert!(t.just_started && t.id == 7);
        next(&mut input);
        assert!(input.is_mouse_button_pressed(mouse::LEFT) && !input.is_mouse_button_just_pressed(mouse::LEFT));
        assert!(!input.touches()[0].just_started);
        input.on_touch(7, Ended, 300.0, 200.0);
        assert!(input.is_mouse_button_just_released(mouse::LEFT));
        assert!(input.touches()[0].just_ended, "readable on the frame it lifts");
        next(&mut input);
        assert!(input.touches().is_empty() && input.primary_touch().is_none());
        assert!(!input.is_mouse_button_pressed(mouse::LEFT));
    }

    /// A tap that starts and ends inside one frame still reads as a press that frame — the same
    /// fast-tap rule the mouse has.
    #[test]
    fn a_tap_inside_one_frame_is_not_lost() {
        let mut input = Input::new();
        input.on_touch(1, Started, 5.0, 5.0);
        input.on_touch(1, Ended, 5.0, 5.0);
        assert!(input.is_mouse_button_just_pressed(mouse::LEFT));
        assert!(input.is_mouse_button_just_released(mouse::LEFT));
    }

    /// Dragging moves the cursor and accumulates both deltas, the finger's and the mouse's.
    #[test]
    fn a_drag_moves_the_cursor_and_reports_its_delta() {
        let mut input = Input::new();
        input.on_touch(3, Started, 100.0, 100.0);
        next(&mut input);
        input.on_touch(3, Moved, 110.0, 95.0);
        input.on_touch(3, Moved, 130.0, 90.0);
        assert_eq!(input.mouse_position(), (130.0, 90.0));
        assert_eq!(input.mouse_delta(), (30.0, -10.0));
        let t = input.touch(3).unwrap();
        assert_eq!((t.delta, t.start, t.position), ((30.0, -10.0), (100.0, 100.0), (130.0, 90.0)));
        next(&mut input);
        assert_eq!(input.touch(3).unwrap().delta, (0.0, 0.0), "a delta is this frame's");
    }

    /// A second finger is a touch, not the mouse; when the first lifts, the second does not take
    /// the cursor over mid-gesture (that would jump it) — the next finger to land alone does.
    #[test]
    fn only_the_first_finger_is_the_mouse() {
        let mut input = Input::new();
        input.on_touch(1, Started, 10.0, 10.0);
        input.on_touch(2, Started, 500.0, 500.0);
        assert_eq!(input.mouse_position(), (10.0, 10.0));
        assert_eq!(input.touches().len(), 2);
        input.on_touch(2, Moved, 520.0, 500.0);
        assert_eq!(input.mouse_position(), (10.0, 10.0), "the second finger moved the cursor");
        input.on_touch(1, Ended, 10.0, 10.0);
        next(&mut input);
        assert!(input.primary_touch().is_none());
        assert!(!input.is_mouse_button_pressed(mouse::LEFT));
        input.on_touch(2, Moved, 540.0, 500.0);
        assert_eq!(input.mouse_position(), (10.0, 10.0));
        input.on_touch(2, Ended, 540.0, 500.0);
        next(&mut input);
        input.on_touch(9, Started, 70.0, 80.0);
        assert_eq!(input.primary_touch().map(|t| t.id), Some(9));
        assert!(input.is_mouse_button_just_pressed(mouse::LEFT));
    }

    /// Losing focus lifts every finger and the button they held, so nothing stays pressed.
    #[test]
    fn losing_focus_lifts_every_finger() {
        let mut input = Input::new();
        input.on_touch(1, Started, 1.0, 1.0);
        input.on_touch(2, Started, 2.0, 2.0);
        next(&mut input);
        input.release_all();
        assert!(input.touches().iter().all(|t| t.just_ended));
        next(&mut input);
        assert!(input.touches().is_empty());
        assert!(!input.is_mouse_button_pressed(mouse::LEFT));
    }

    /// A replay recorded before touch existed still loads.
    #[test]
    fn an_input_saved_without_touches_loads() {
        let mut v = serde_json::to_value(Input::new()).unwrap();
        v.as_object_mut().unwrap().remove("touches");
        let back: Input = serde_json::from_value(v).expect("an old recording");
        assert!(back.touches().is_empty());
    }
}
