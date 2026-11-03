//! [`HoldRepeat`] — a held key or button that fires again and again, like a keyboard's auto-repeat
//! but on the game's clock and at the game's rate.

/// Fires once when a key goes down, then — if it is still held — again after
/// [`delay`](Self::delay) and every [`interval`](Self::interval) after that: holding ↓ walks a
/// menu's focus down the list, holding → walks a character across a grid.
///
/// `Input::is_key_just_pressed` fires only on the press (the operating system's own repeats are
/// not new presses), so holding a key did nothing more than tapping it — in every menu and every
/// grid game. Feed it whether the key is held and the frame's `dt`:
///
/// ```
/// use gizmo_core::input::HoldRepeat;
///
/// let mut walk = HoldRepeat::new(0.3, 0.1);
/// assert_eq!(walk.tick(true, 0.016), 1, "the press itself");
/// assert_eq!(walk.tick(true, 0.25), 0, "still within the delay");
/// assert_eq!(walk.tick(true, 0.05), 1, "the delay is over");
/// assert_eq!(walk.tick(true, 0.25), 2, "a long frame catches up");
/// assert_eq!(walk.tick(false, 0.016), 0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoldRepeat {
    /// Seconds from the press to the first repeat.
    pub delay: f32,
    /// Seconds between repeats after that.
    pub interval: f32,
    held_for: Option<f32>,
    next: f32,
}

impl HoldRepeat {
    /// A repeat that waits `delay` seconds, then fires every `interval`.
    #[must_use]
    pub fn new(delay: f32, interval: f32) -> Self {
        Self { delay: delay.max(0.0), interval: interval.max(0.001), held_for: None, next: 0.0 }
    }

    /// One frame: `held` whether the key is down now. Returns how many times it fires this frame —
    /// 1 on the press, then one per repeat that came due (a long frame can owe several).
    pub fn tick(&mut self, held: bool, dt: f32) -> u32 {
        if !held {
            self.held_for = None;
            return 0;
        }
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        match self.held_for.as_mut() {
            None => {
                self.held_for = Some(0.0);
                self.next = self.delay;
                1
            }
            Some(t) => {
                *t += dt;
                let mut fired = 0;
                while *t >= self.next && fired < 64 {
                    fired += 1;
                    self.next += self.interval;
                }
                fired
            }
        }
    }

    /// Whether the key is being held (it fired its press and has not been released).
    #[must_use]
    pub fn is_held(&self) -> bool {
        self.held_for.is_some()
    }
}

impl Default for HoldRepeat {
    /// A menu's feel: 0.4 s before the first repeat, then ten a second.
    fn default() -> Self {
        Self::new(0.4, 0.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_fires_on_the_press_then_after_the_delay_then_at_the_interval() {
        // Steps of 1/64 s, exact in binary, so the times compare exactly.
        let mut r = HoldRepeat::new(0.5, 0.25);
        let mut fired = vec![];
        for frame in 0..80 {
            if r.tick(true, 1.0 / 64.0) > 0 {
                fired.push(frame);
            }
        }
        assert_eq!(fired, vec![0, 32, 48, 64], "frames: the press, 0.5 s, 0.75 s, 1 s");
    }

    #[test]
    fn a_release_starts_it_over() {
        let mut r = HoldRepeat::new(0.5, 0.2);
        assert_eq!(r.tick(true, 0.01), 1);
        assert_eq!(r.tick(true, 0.6), 1);
        assert_eq!(r.tick(false, 0.01), 0);
        assert!(!r.is_held());
        assert_eq!(r.tick(true, 0.01), 1, "a new press fires at once");
        assert_eq!(r.tick(true, 0.3), 0, "and waits the whole delay again");
        assert_eq!(r.tick(true, f32::NAN), 0);
    }
}
