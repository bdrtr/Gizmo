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
        self.clip = Some(clip);
        self.elapsed_time = 0.0;
        self.playing = true;
        self.target_entities.clear(); // Need to re-resolve targets for the new clip
        self
    }

    /// Pauses playback, leaving [`Self::elapsed_time`] untouched.
    pub fn pause(&mut self) -> &mut Self {
        self.playing = false;
        self
    }

    /// Resumes playback from the current position.
    pub fn resume(&mut self) -> &mut Self {
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
    /// within the clip length; otherwise it is clamped to `duration` and playback
    /// stops the instant it *reaches* the end (`>=`), so a non-looping clip does
    /// not overshoot its final frame by one tick.
    pub fn advance(&mut self, dt: f32, duration: f32) {
        let safe_speed = if self.speed.is_finite() { self.speed } else { 1.0 };
        self.elapsed_time += dt * safe_speed;

        if self.looping {
            if duration > 0.0 {
                self.elapsed_time %= duration;
            }
        } else if self.elapsed_time >= duration {
            self.elapsed_time = duration;
            self.playing = false;
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
}
