use super::keyframe::Track;
use gizmo_math::{Quat, Vec3};

#[derive(Clone, Debug)]
pub struct AnimationClip {
    pub name: String,
    pub duration: f32,
    pub translations: Vec<Track<Vec3>>,
    pub rotations: Vec<Track<Quat>>,
    pub scales: Vec<Track<Vec3>>,
}
