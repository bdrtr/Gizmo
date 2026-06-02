use std::sync::Arc;
use crate::clip::AnimationClip;
use std::collections::HashMap;
use gizmo_core::entity::Entity;

#[derive(Clone, Copy, Debug, Default)]
pub struct Animated;

impl gizmo_core::component::Component for Animated {
    fn storage_type() -> gizmo_core::component::StorageType {
        gizmo_core::component::StorageType::Table
    }
}


#[derive(Clone)]
pub struct AnimationPlayer {
    pub clip: Option<Arc<AnimationClip>>,
    pub elapsed_time: f32,
    pub speed: f32,
    pub playing: bool,
    pub looping: bool,
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn play(&mut self, clip: Arc<AnimationClip>) -> &mut Self {
        self.clip = Some(clip);
        self.elapsed_time = 0.0;
        self.playing = true;
        self.target_entities.clear(); // Need to re-resolve targets for the new clip
        self
    }

    pub fn pause(&mut self) -> &mut Self {
        self.playing = false;
        self
    }

    pub fn resume(&mut self) -> &mut Self {
        self.playing = true;
        self
    }

    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }
}
