use std::sync::Arc;
use crate::clip::AnimationClip;
use std::collections::HashMap;
use gizmo_core::entity::Entity;

/// Marker component inserted onto entities that are currently targeted by an
/// [`AnimationPlayer`], so the animation system can query only animated transforms.
#[derive(Clone, Copy, Debug, Default)]
pub struct Animated;

impl gizmo_core::component::Component for Animated {
    fn storage_type() -> gizmo_core::component::StorageType {
        gizmo_core::component::StorageType::Table
    }
}


/// Component that drives an [`AnimationClip`] over time.
///
/// Attach this to the root entity of a hierarchy; the animation system advances
/// [`Self::elapsed_time`], resolves each track's `target_name` to a child entity
/// (caching the result in [`Self::target_entities`]), and writes the sampled
/// transform values to those entities.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AnimationPlayer {
    /// The clip currently being played, if any.
    pub clip: Option<Arc<AnimationClip>>,
    /// Current playback position, in seconds.
    pub elapsed_time: f32,
    /// Playback speed multiplier (`1.0` is real time).
    pub speed: f32,
    /// Whether playback is currently advancing.
    pub playing: bool,
    /// Whether the clip restarts from the beginning when it ends.
    pub looping: bool,
    /// Cache mapping each track's `target_name` to the resolved entity.
    pub target_entities: HashMap<String, Entity>,
}

impl Default for AnimationPlayer {
    fn default() -> Self {
        Self {
            clip: None,
            elapsed_time: 0.0,
            speed: 1.0,
            playing: true,
            looping: true,
            target_entities: HashMap::new(),
        }
    }
}

impl gizmo_core::component::Component for AnimationPlayer {
    fn storage_type() -> gizmo_core::component::StorageType {
        gizmo_core::component::StorageType::Table
    }
}

impl AnimationPlayer {
    /// Creates a player with default settings (equivalent to [`Default::default`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts playing `clip` from the beginning, clearing any cached target entities.
    pub fn play(&mut self, clip: Arc<AnimationClip>) -> &mut Self {
        // Clip-start is a genuine once-per-playthrough lifecycle event, so it is
        // logged at info!. Read the metadata before the Arc is moved into `self`.
        tracing::info!(
            clip = %clip.name,
            tracks = clip.tracks.len(),
            duration = clip.duration(),
            "[Animation] starting transform-track clip"
        );
        self.clip = Some(clip);
        self.elapsed_time = 0.0;
        self.playing = true;
        self.target_entities.clear(); // Need to re-resolve targets for the new clip
        self
    }

    /// Pauses playback, leaving [`Self::elapsed_time`] untouched.
    pub fn pause(&mut self) -> &mut Self {
        tracing::debug!(elapsed = self.elapsed_time, "[Animation] paused");
        self.playing = false;
        self
    }

    /// Resumes playback from the current position.
    pub fn resume(&mut self) -> &mut Self {
        tracing::debug!(elapsed = self.elapsed_time, "[Animation] resumed");
        self.playing = true;
        self
    }

    /// Builder: sets the playback [`Self::speed`] multiplier.
    ///
    /// A non-finite `speed` (`NaN`/`±∞`) is rejected and falls back to `1.0` so
    /// it cannot poison [`Self::elapsed_time`] during playback.
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = if speed.is_finite() { speed } else { 1.0 };
        self
    }

    /// Builder: sets whether playback [`Self::looping`].
    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    /// Advances playback by `dt` seconds against a clip of `duration` seconds.
    ///
    /// A non-finite [`Self::speed`] (`NaN`/`±∞`) falls back to `1.0` so it cannot
    /// poison [`Self::elapsed_time`]. When [`Self::looping`], `elapsed_time` wraps
    /// within the clip length; otherwise it is clamped to `[0, duration]` and
    /// playback stops the instant it *reaches* an end, so a non-looping clip does
    /// not overshoot its final frame by one tick.
    ///
    /// Reverse playback (`speed < 0`) is supported: looping wraps with
    /// `rem_euclid` (a plain `%` keeps the dividend's sign, so a negative time
    /// stays negative and the sampler pins the pose at frame 0 — see the sibling
    /// skeletal fix in `gizmo-renderer::animation_system`); non-looping reverse
    /// completes at the clip start (`elapsed_time <= 0`) instead of running
    /// unbounded-negative with `playing` stuck true.
    pub fn advance(&mut self, dt: f32, duration: f32) {
        let safe_speed = if self.speed.is_finite() { self.speed } else { 1.0 };
        self.elapsed_time += dt * safe_speed;

        if self.looping {
            if duration > 0.0 {
                // rem_euclid, not `%`: wraps negative (reverse-playback) times
                // back into [0, duration) rather than leaving them negative.
                let before = self.elapsed_time;
                self.elapsed_time = self.elapsed_time.rem_euclid(duration);
                // Only fires on the frames the playhead actually crosses a loop
                // boundary (in-range values are unchanged by rem_euclid), so this
                // stays quiet on the per-frame hot path — hence trace!, not debug!.
                if self.elapsed_time != before {
                    tracing::trace!(
                        from = before,
                        to = self.elapsed_time,
                        duration,
                        "[Animation] loop wrapped"
                    );
                }
            }
        } else if safe_speed < 0.0 {
            // Reverse, non-looping: complete at the start of the clip. Gated on
            // `speed < 0` so a frozen (`speed == 0`) or forward clip sitting at 0
            // is not spuriously stopped.
            if self.elapsed_time <= 0.0 {
                self.elapsed_time = 0.0;
                self.playing = false;
                tracing::debug!(duration, "[Animation] reverse clip reached start; playback stopped");
            }
        } else if self.elapsed_time >= duration {
            self.elapsed_time = duration;
            self.playing = false;
            tracing::debug!(duration, "[Animation] clip reached end; playback stopped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_speed_rejects_non_finite() {
        // NaN / Inf must not survive into `speed`, or they would poison
        // `elapsed_time` in the animation system.
        assert_eq!(AnimationPlayer::new().with_speed(f32::NAN).speed, 1.0);
        assert_eq!(AnimationPlayer::new().with_speed(f32::INFINITY).speed, 1.0);
        assert_eq!(
            AnimationPlayer::new().with_speed(f32::NEG_INFINITY).speed,
            1.0
        );
    }

    #[test]
    fn with_speed_keeps_finite_values() {
        assert_eq!(AnimationPlayer::new().with_speed(2.5).speed, 2.5);
        assert_eq!(AnimationPlayer::new().with_speed(-1.0).speed, -1.0);
        assert_eq!(AnimationPlayer::new().with_speed(0.0).speed, 0.0);
    }

    #[test]
    fn advance_non_looping_stops_exactly_at_duration() {
        // Regression for the off-by-one termination: reaching duration exactly must
        // stop playback (`>=`), not overshoot by one tick (which `>` would allow).
        let mut p = AnimationPlayer::new().looping(false);
        p.elapsed_time = 0.0;
        p.advance(1.0, 1.0); // lands exactly on duration
        assert_eq!(p.elapsed_time, 1.0);
        assert!(!p.playing, "non-looping clip must stop the instant it reaches duration");
    }

    #[test]
    fn advance_non_looping_does_not_stop_early() {
        let mut p = AnimationPlayer::new().looping(false);
        p.elapsed_time = 0.0;
        p.advance(1.0, 2.0); // still mid-clip
        assert_eq!(p.elapsed_time, 1.0);
        assert!(p.playing, "clip must keep playing before it reaches duration");
    }

    #[test]
    fn advance_non_finite_speed_does_not_poison_elapsed_time() {
        let mut p = AnimationPlayer::new().looping(false).with_speed(1.0);
        p.speed = f32::NAN; // simulate a directly-mutated player
        p.advance(0.5, 10.0);
        assert!(p.elapsed_time.is_finite());
        assert_eq!(p.elapsed_time, 0.5, "NaN speed must fall back to 1.0");
    }

    #[test]
    fn advance_looping_wraps_within_duration() {
        let mut p = AnimationPlayer::new().looping(true);
        p.elapsed_time = 0.0;
        p.advance(1.5, 1.0);
        assert!((p.elapsed_time - 0.5).abs() < 1e-6, "looping time must wrap into [0, duration)");
        assert!(p.playing);
    }

    #[test]
    fn advance_looping_reverse_wraps_to_end_not_frame_zero() {
        // Regression: reverse looping playback used `%=`, which keeps the sign, so a
        // negative time stayed negative and the sampler pinned the pose at frame 0
        // forever. `rem_euclid` must wrap it back near the end of the clip.
        let mut p = AnimationPlayer::new().looping(true).with_speed(-1.0);
        p.elapsed_time = 0.0;
        p.advance(0.1, 2.0); // 0 - 0.1 = -0.1 → should wrap to 1.9, NOT stay at -0.1
        assert!(
            (p.elapsed_time - 1.9).abs() < 1e-6,
            "reverse looping time must wrap near the clip end, got {}",
            p.elapsed_time
        );
        assert!(p.elapsed_time >= 0.0, "wrapped time must never be negative");
        assert!(p.playing, "looping clip keeps playing");
    }

    #[test]
    fn advance_non_looping_reverse_stops_at_start() {
        // Regression: non-looping reverse playback ran `elapsed_time` unbounded-negative
        // and `playing` never became false (only `>= duration` was checked). It must
        // complete at the clip start.
        let mut p = AnimationPlayer::new().looping(false).with_speed(-1.0);
        p.elapsed_time = 0.5;
        p.advance(1.0, 2.0); // 0.5 - 1.0 = -0.5 → clamp to 0 and stop
        assert_eq!(p.elapsed_time, 0.0, "reverse non-looping must clamp to the start");
        assert!(!p.playing, "reverse non-looping must stop when it reaches the start");
    }

    #[test]
    fn advance_non_looping_zero_speed_does_not_stop() {
        // A frozen clip (speed == 0) sitting mid-clip must not be spuriously stopped by
        // the reverse-completion branch.
        let mut p = AnimationPlayer::new().looping(false).with_speed(0.0);
        p.elapsed_time = 0.0;
        p.advance(1.0, 2.0);
        assert_eq!(p.elapsed_time, 0.0);
        assert!(p.playing, "speed==0 must keep playing, not trip the reverse-stop branch");
    }

    // ── Playback control state transitions ─────────────────────────────

    #[test]
    fn play_resets_time_targets_and_marks_playing() {
        let mut p = AnimationPlayer::new();
        p.elapsed_time = 5.0;
        p.playing = false;
        p.target_entities.insert("bone".into(), Entity::new(3, 0)); // stale cache
        p.play(Arc::new(AnimationClip::default()));
        assert!(p.clip.is_some(), "clip must be set");
        assert_eq!(p.elapsed_time, 0.0, "play restarts from the beginning");
        assert!(p.playing, "play resumes playback");
        assert!(
            p.target_entities.is_empty(),
            "targets must be re-resolved for the new clip"
        );
    }

    #[test]
    fn pause_and_resume_toggle_playing_without_touching_time() {
        let mut p = AnimationPlayer::new();
        p.elapsed_time = 1.25;
        p.pause();
        assert!(!p.playing);
        assert_eq!(p.elapsed_time, 1.25, "pause must not rewind");
        p.resume();
        assert!(p.playing);
        assert_eq!(p.elapsed_time, 1.25, "resume continues from the same spot");
    }

    #[test]
    fn looping_builder_sets_flag() {
        assert!(!AnimationPlayer::new().looping(false).looping);
        assert!(AnimationPlayer::new().looping(true).looping);
    }

    #[test]
    fn advance_speed_multiplier_scales_dt() {
        // elapsed += dt * speed. speed 2.0, dt 0.5 → +1.0.
        let mut p = AnimationPlayer::new().looping(false).with_speed(2.0);
        p.elapsed_time = 0.0;
        p.advance(0.5, 10.0);
        assert!((p.elapsed_time - 1.0).abs() < 1e-6, "got {}", p.elapsed_time);
        assert!(p.playing);
    }

    #[test]
    fn advance_looping_wraps_across_multiple_periods() {
        // A dt larger than the clip length wraps modulo the duration (rem_euclid),
        // not just once: 2.7 over a 1.0 clip lands at 0.7.
        let mut p = AnimationPlayer::new().looping(true);
        p.elapsed_time = 0.0;
        p.advance(2.7, 1.0);
        assert!((p.elapsed_time - 0.7).abs() < 1e-5, "got {}", p.elapsed_time);
        assert!(p.playing);
    }

    #[test]
    fn advance_looping_zero_duration_does_not_wrap_or_nan() {
        // duration == 0 must skip the rem_euclid (division by zero → NaN); the time
        // just accumulates and stays finite.
        let mut p = AnimationPlayer::new().looping(true);
        p.elapsed_time = 0.0;
        p.advance(0.5, 0.0);
        assert!(p.elapsed_time.is_finite(), "must not become NaN on zero-length clip");
        assert_eq!(p.elapsed_time, 0.5);
    }
}
