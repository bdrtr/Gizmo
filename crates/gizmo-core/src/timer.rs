//! [`Timer`] — a countdown a game ticks with its frame time.
//!
//! Every game written against the engine grew its own: `spawn_timer -= dt; if spawn_timer <= 0.0
//! { spawn_timer = 1.5; … }`, a `cooldown: f32` on a tower, an `Option<f32>` that means "the round
//! is ending". Each of those is three lines, and each gets one of the same three things wrong: a
//! frame longer than the period fires once instead of twice, the leftover time is thrown away
//! (so a 1 s spawner drifts slower the lower the frame rate), or the reset value is written in
//! two places and one of them is changed.
//!
//! A `Timer` is a plain value: put it in a resource, a component or a local, and call
//! [`tick`](Timer::tick) with the frame's `dt` — nothing ticks it for you, so it runs on game time
//! (paused when your game is paused) and in the schedule you choose.
//!
//! ```
//! use gizmo_core::Timer;
//!
//! let mut spawn = Timer::repeating(0.5);
//! assert_eq!(spawn.tick(1.2), 2, "a long frame fires twice");
//! assert!((spawn.elapsed() - 0.2).abs() < 1e-6, "and keeps the leftover");
//!
//! let mut round = Timer::once(3.0);
//! round.tick(2.0);
//! assert!(!round.finished());
//! round.tick(2.0);
//! assert!(round.just_finished() && round.finished());
//! round.tick(2.0);
//! assert!(!round.just_finished() && round.finished(), "once: finished stays, just_finished does not");
//! ```

/// Whether a [`Timer`] stops at its end or starts over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum TimerMode {
    /// Runs once, then stays finished until [`Timer::reset`].
    Once,
    /// Starts over at its end, keeping the time past it.
    Repeating,
}

/// A countdown of `duration` seconds, advanced by [`tick`](Self::tick). See the
/// [module docs](self).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Timer {
    duration: f32,
    elapsed: f32,
    mode: TimerMode,
    paused: bool,
    finished: bool,
    fired: u32,
}

impl Timer {
    /// A timer that finishes once, `secs` seconds from now.
    #[must_use]
    pub fn once(secs: f32) -> Self {
        Self::new(secs, TimerMode::Once)
    }

    /// A timer that finishes every `secs` seconds.
    #[must_use]
    pub fn repeating(secs: f32) -> Self {
        Self::new(secs, TimerMode::Repeating)
    }

    /// A timer of `secs` seconds in `mode`. A negative or NaN duration is taken as zero.
    #[must_use]
    pub fn new(secs: f32, mode: TimerMode) -> Self {
        Self { duration: clean(secs), elapsed: 0.0, mode, paused: false, finished: false, fired: 0 }
    }

    /// Advances the timer by `dt` seconds and returns how many times it finished during this tick:
    /// `0` or `1` for a [`Once`](TimerMode::Once) timer; for a repeating one, every period the step
    /// crossed (a 1.2 s frame on a 0.5 s timer is `2`, with 0.2 s carried into the next period).
    ///
    /// A negative or NaN `dt`, or a paused timer, advances nothing and fires nothing. A repeating
    /// timer of zero duration fires once a tick rather than without end.
    pub fn tick(&mut self, dt: f32) -> u32 {
        self.fired = 0;
        if self.paused {
            return 0;
        }
        let dt = clean(dt);
        match self.mode {
            TimerMode::Once => {
                if self.finished {
                    return 0;
                }
                self.elapsed += dt;
                if self.elapsed >= self.duration {
                    self.elapsed = self.duration;
                    self.finished = true;
                    self.fired = 1;
                }
            }
            TimerMode::Repeating => {
                self.elapsed += dt;
                if self.duration <= 0.0 {
                    self.elapsed = 0.0;
                    self.fired = 1;
                } else if self.elapsed >= self.duration {
                    // `as` saturates, so an absurd step (an infinite one included) fires
                    // `u32::MAX` times rather than wrapping.
                    self.fired = (self.elapsed / self.duration).floor() as u32;
                    let rest = self.elapsed.rem_euclid(self.duration);
                    self.elapsed = if rest.is_finite() && rest < self.duration { rest } else { 0.0 };
                }
                self.finished = self.fired > 0;
            }
        }
        self.fired
    }

    /// Whether the last [`tick`](Self::tick) finished it — `true` for exactly that one tick.
    #[must_use]
    pub fn just_finished(&self) -> bool {
        self.fired > 0
    }

    /// How many times the last [`tick`](Self::tick) finished it (what `tick` returned).
    #[must_use]
    pub fn times_finished_this_tick(&self) -> u32 {
        self.fired
    }

    /// Whether it has finished: for a [`Once`](TimerMode::Once) timer, from its end until
    /// [`reset`](Self::reset); for a repeating one, the same as [`just_finished`](Self::just_finished).
    #[must_use]
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Seconds into the current period.
    #[must_use]
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }

    /// Seconds left in the current period.
    #[must_use]
    pub fn remaining(&self) -> f32 {
        (self.duration - self.elapsed).max(0.0)
    }

    /// How far through the current period, `0.0..=1.0` — for a cooldown bar or a fade. A
    /// zero-length timer reads `1.0`.
    #[must_use]
    pub fn fraction(&self) -> f32 {
        if self.duration <= 0.0 {
            1.0
        } else {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        }
    }

    /// The period, in seconds.
    #[must_use]
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// Changes the period, keeping the time already elapsed — clamped into the new one, so a
    /// period shortened below the time already spent fires on the next [`tick`](Self::tick).
    pub fn set_duration(&mut self, secs: f32) {
        self.duration = clean(secs);
        self.elapsed = self.elapsed.min(self.duration);
    }

    /// Once or repeating.
    #[must_use]
    pub fn mode(&self) -> TimerMode {
        self.mode
    }

    /// Back to the start of a period, not finished. Keeps the duration, the mode and whether it is
    /// paused.
    pub fn reset(&mut self) {
        self.elapsed = 0.0;
        self.finished = false;
        self.fired = 0;
    }

    /// Stops [`tick`](Self::tick) from advancing it.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Lets [`tick`](Self::tick) advance it again.
    pub fn unpause(&mut self) {
        self.paused = false;
    }

    /// Whether it is paused.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused
    }
}

/// A timer can sit on an entity — a tower's cooldown, a pickup's lifetime.
impl crate::Component for Timer {}

fn clean(secs: f32) -> f32 {
    if secs.is_nan() || secs < 0.0 {
        0.0
    } else {
        secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn a_repeating_timer_fires_every_period_crossed_and_keeps_the_rest() {
        let mut t = Timer::repeating(0.5);
        assert_eq!(t.tick(0.3), 0);
        assert!(!t.just_finished());
        assert_eq!(t.tick(0.3), 1, "0.6 crosses one period");
        assert!(near(t.elapsed(), 0.1), "the 0.1 past it is kept, not dropped: {}", t.elapsed());
        assert_eq!(t.tick(1.45), 3, "0.1 + 1.45 = 1.55 crosses three");
        assert!(near(t.elapsed(), 0.05));
        assert_eq!(t.tick(0.0), 0);
        assert!(!t.just_finished() && !t.finished());
    }

    /// The point of keeping the leftover: a 1 s spawner fires sixty times in sixty seconds at any
    /// frame rate, where `if t <= 0 { t = 1.0 }` loses a frame's worth each time.
    #[test]
    fn the_count_over_a_minute_does_not_depend_on_the_frame_rate() {
        for fps in [30.0_f32, 60.0, 144.0, 7.5, 45.5] {
            let mut t = Timer::repeating(1.0);
            let frames = (60.0 * fps).round() as u32;
            let fired: u32 = (0..frames).map(|_| t.tick(1.0 / fps)).sum();
            assert!((59..=60).contains(&fired), "{fps} fps fired {fired} times");
        }
    }

    #[test]
    fn a_once_timer_finishes_once_and_stays_finished_until_reset() {
        let mut t = Timer::once(1.0);
        assert_eq!(t.tick(0.6), 0);
        assert!(near(t.remaining(), 0.4) && near(t.fraction(), 0.6));
        assert_eq!(t.tick(5.0), 1, "a long frame still finishes it once");
        assert!(t.just_finished() && t.finished());
        assert_eq!(t.elapsed(), 1.0, "clamped at its end");
        assert_eq!(t.tick(1.0), 0);
        assert!(!t.just_finished() && t.finished());
        t.reset();
        assert!(!t.finished() && t.elapsed() == 0.0);
        assert_eq!(t.tick(1.0), 1, "exactly the duration finishes it");
    }

    #[test]
    fn paused_bad_and_zero_inputs_do_nothing_surprising() {
        let mut t = Timer::repeating(1.0);
        t.pause();
        assert_eq!(t.tick(10.0), 0);
        assert_eq!(t.elapsed(), 0.0);
        t.unpause();
        assert_eq!(t.tick(-3.0), 0, "time does not run backwards");
        assert_eq!(t.tick(f32::NAN), 0);
        assert_eq!(t.elapsed(), 0.0);

        let mut zero = Timer::repeating(0.0);
        assert_eq!(zero.tick(0.016), 1, "once a tick, not forever");
        assert_eq!(zero.fraction(), 1.0);
        assert_eq!(Timer::once(-1.0).duration(), 0.0);
        assert_eq!(Timer::repeating(0.001).tick(f32::INFINITY), u32::MAX, "saturates, no panic");
    }

    #[test]
    fn set_duration_keeps_the_elapsed_time() {
        let mut t = Timer::repeating(2.0);
        t.tick(1.5);
        t.set_duration(1.0);
        assert_eq!(t.elapsed(), 1.0, "clamped into the shorter period");
        assert_eq!(t.tick(0.0), 1, "which is over, so the next tick fires it");
        assert_eq!(t.tick(0.25), 0);
        t.set_duration(4.0);
        assert!(near(t.elapsed(), 0.25));
    }
}
