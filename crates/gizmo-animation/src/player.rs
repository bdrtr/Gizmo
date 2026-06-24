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
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Builder: sets whether playback [`Self::looping`].
    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }
}
