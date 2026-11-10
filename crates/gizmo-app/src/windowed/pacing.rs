//! When the next frame's update starts, while a render thread draws the last one.
//!
//! With the draw on its own thread, a frame's update can start the moment the last frame was
//! handed over. When the draw is the slower half (a heavy scene, a GPU at its limit, a vsync wait),
//! that update then finishes early and **waits at the sync point** for the draw — holding input it
//! sampled a whole wait ago. Measured in a window (`demo/src/bin/render_thread.rs`, 2026-11-10):
//! with the update light, the render thread bought no frame rate and added 3 ms (2 000 cubes) to
//! 8 ms (20 000 cubes) between the start of a frame's update and the end of its draw.
//!
//! So the wait is moved to where it costs nothing: **before** the update, in the event loop, where
//! window input keeps arriving and is taken by the update that follows. [`FramePacing`] compares
//! the two sides — how long the render thread was busy with a frame, and how long the loop was
//! busy between handing that frame over and coming back for it — and asks for the difference, less
//! a margin, before the next update. The draw still overlaps the update; the update just starts as
//! late as it can and still meet the draw.
//!
//! The two busy times are measured, not inferred from the wait: a wait cannot tell a loop that is
//! exactly on time from one that is late on purpose — both wait for nothing — so a delay learned
//! from waits could only ever grow.

use std::time::Duration;

/// How long a frame's update may start later than it could, learned from the frames before it.
#[derive(Debug, Clone, Default)]
pub(crate) struct FramePacing {
    /// The loop's spare time per frame, smoothed — how much later it could have started.
    slack: Duration,
}

impl FramePacing {
    /// Weight of a new sample when the spare time grows: slowly, so one slow draw does not put the
    /// next update to sleep.
    const RISE: f64 = 0.2;
    /// …and when it shrinks: quickly, because a delay the draw no longer needs keeps the render
    /// thread idle, and that is frame rate lost.
    const FALL: f64 = 0.6;
    /// Kept in hand of the spare time, so a draw that runs a little long does not find the loop
    /// still asleep. Part fixed — timer and scheduler resolution — and part proportional, for
    /// jitter that grows with the frame.
    const MARGIN_FIXED: Duration = Duration::from_micros(500);
    const MARGIN_SHARE: f64 = 0.05;
    /// No pacing delay is longer, whatever the measurements say: a stall must not become a sleep.
    pub(crate) const LONGEST: Duration = Duration::from_millis(50);

    /// One frame came back from the render thread. `draw` is how long the render thread was busy
    /// with it; `loop_busy` is how long the loop was busy between handing it over and coming back
    /// for it, the delay it left on purpose not counted. Returns how long to leave before the next
    /// frame's update.
    pub(crate) fn record(&mut self, draw: Duration, loop_busy: Duration) -> Duration {
        let sample = draw.saturating_sub(loop_busy).min(Self::LONGEST * 2).as_secs_f64();
        let slack = self.slack.as_secs_f64();
        let weight = if sample < slack { Self::FALL } else { Self::RISE };
        self.slack = Duration::from_secs_f64(slack + weight * (sample - slack));
        self.delay()
    }

    /// How long to leave before the next frame's update, from what has been learned so far.
    pub(crate) fn delay(&self) -> Duration {
        let margin = Self::MARGIN_FIXED + self.slack.mul_f64(Self::MARGIN_SHARE);
        self.slack.saturating_sub(margin).min(Self::LONGEST)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: f64) -> Duration {
        Duration::from_secs_f64(v / 1e3)
    }

    fn as_ms(d: Duration) -> f64 {
        d.as_secs_f64() * 1e3
    }

    /// A loop that is the slower side has nothing to spare and is never delayed.
    #[test]
    fn a_loop_slower_than_the_draw_starts_at_once() {
        let mut pacing = FramePacing::default();
        for _ in 0..100 {
            assert_eq!(pacing.record(ms(6.0), ms(9.0)), Duration::ZERO);
        }
    }

    /// A scene whose draw takes 20 ms and whose update takes 5: the update learns to start 15 ms
    /// late less the margin, and never so late that the draw would have to wait for it.
    #[test]
    fn the_update_starts_as_late_as_it_can_and_still_meets_the_draw() {
        let mut pacing = FramePacing::default();
        let mut delay = Duration::ZERO;
        for _ in 0..200 {
            delay = pacing.record(ms(20.0), ms(5.0));
        }
        // 15 ms spare, less the margin: 0.5 ms and 5 % of it.
        assert!((13.25..14.25).contains(&as_ms(delay)), "the update starts {:.2} ms late; it could start 15 ms late", as_ms(delay));
        assert!(as_ms(delay) + 5.0 <= 20.0);
    }

    /// The scene changes: when the draw gets faster, the delay follows within a few frames rather
    /// than keeping the render thread idle.
    #[test]
    fn a_faster_draw_shortens_the_delay_at_once() {
        let mut pacing = FramePacing::default();
        for _ in 0..200 {
            pacing.record(ms(20.0), ms(4.0));
        }
        let mut frames = 0;
        while as_ms(pacing.record(ms(8.0), ms(4.0))) + 4.0 > 8.0 {
            frames += 1;
            assert!(frames < 8, "the delay kept an 8 ms draw idle for {frames} frames");
        }
    }

    /// One slow draw among fast ones does not put the next update to sleep.
    #[test]
    fn one_slow_draw_is_not_followed_by_a_long_sleep() {
        let mut pacing = FramePacing::default();
        for _ in 0..200 {
            pacing.record(ms(8.0), ms(4.0));
        }
        let after_spike = pacing.record(ms(40.0), ms(4.0));
        assert!(as_ms(after_spike) < 12.0, "one 40 ms draw asked for a {:.2} ms sleep", as_ms(after_spike));
    }

    /// A draw that jitters between 18 and 22 ms: the delay leaves room for the fast frames too.
    #[test]
    fn a_jittery_draw_keeps_its_margin() {
        let mut pacing = FramePacing::default();
        let mut longest = Duration::ZERO;
        for i in 0..400 {
            let draw = if i % 2 == 0 { 18.0 } else { 22.0 };
            let delay = pacing.record(ms(draw), ms(5.0));
            if i > 100 {
                longest = longest.max(delay);
            }
        }
        assert!(as_ms(longest) + 5.0 <= 18.0, "a {:.2} ms delay keeps an 18 ms draw waiting", as_ms(longest));
    }

    /// A stall — a draw that took half a second, again and again — does not turn into long sleeps.
    #[test]
    fn a_stall_is_not_slept_off() {
        let mut pacing = FramePacing::default();
        for _ in 0..50 {
            pacing.record(Duration::from_millis(500), Duration::ZERO);
        }
        assert!(pacing.delay() <= FramePacing::LONGEST);
    }
}
