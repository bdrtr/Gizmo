//! Tweens: a `Transform` eased from where it is to where it should be.
//!
//! A card that slides into place, a door that swings shut, a pickup that bobs, a tile that moves
//! one square — every game wants these, and the engine had no way to say "go there in 0.3 s,
//! slowing down at the end". Clips ([`crate::clip`]) are authored keyframe tracks; a tween is the
//! one-line version decided at run time.
//!
//! ```
//! use gizmo_animation::tween::{Ease, Tween};
//! use gizmo_math::Vec3;
//!
//! // Slide to x = 3 in a third of a second, decelerating; it starts from wherever the entity is.
//! let slide = Tween::to_position(Vec3::new(3.0, 0.0, 0.0), 0.33).with_ease(Ease::OutCubic);
//! assert!(!slide.is_finished());
//! ```
//!
//! [`tween_system`](crate::tween::tween_system) advances them; the engine's frame driver calls it on the animation clock in
//! both draw paths (so `set_time_scale` and pause apply), before transforms are propagated — a
//! game does not have to.

use gizmo_core::query::Mut;
use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::Transform;

/// How progress through a tween maps onto progress along it.
///
/// Every curve starts at 0 and ends at 1. `OutBack` and `OutElastic` overshoot past 1 on the way
/// — that is what they are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Ease {
    /// Constant speed.
    #[default]
    Linear,
    /// Starts slow.
    InQuad,
    /// Ends slow.
    OutQuad,
    /// Slow at both ends.
    InOutQuad,
    /// Starts slower.
    InCubic,
    /// Ends slower — the usual choice for something arriving.
    OutCubic,
    /// Slower at both ends.
    InOutCubic,
    /// A sine-shaped start and end — gentle.
    InOutSine,
    /// Overshoots and settles back.
    OutBack,
    /// Bounces to rest.
    OutBounce,
    /// Overshoots in a decaying wobble.
    OutElastic,
}

impl Ease {
    /// The eased value of `t`, clamped into `[0, 1]` first.
    #[must_use]
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::InQuad => t * t,
            Self::OutQuad => 1.0 - (1.0 - t) * (1.0 - t),
            Self::InOutQuad => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
            Self::InCubic => t * t * t,
            Self::OutCubic => 1.0 - (1.0 - t).powi(3),
            Self::InOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Self::InOutSine => -((std::f32::consts::PI * t).cos() - 1.0) / 2.0,
            Self::OutBack => {
                const C1: f32 = 1.70158;
                const C3: f32 = C1 + 1.0;
                1.0 + C3 * (t - 1.0).powi(3) + C1 * (t - 1.0).powi(2)
            }
            Self::OutBounce => {
                const N: f32 = 7.5625;
                const D: f32 = 2.75;
                if t < 1.0 / D {
                    N * t * t
                } else if t < 2.0 / D {
                    let t = t - 1.5 / D;
                    N * t * t + 0.75
                } else if t < 2.5 / D {
                    let t = t - 2.25 / D;
                    N * t * t + 0.9375
                } else {
                    let t = t - 2.625 / D;
                    N * t * t + 0.984375
                }
            }
            Self::OutElastic => {
                if t == 0.0 || t == 1.0 {
                    t
                } else {
                    let c4 = std::f32::consts::TAU / 3.0;
                    2f32.powf(-10.0 * t) * ((t * 10.0 - 0.75) * c4).sin() + 1.0
                }
            }
        }
    }

    /// Every curve, for tests and pickers.
    pub const ALL: [Ease; 11] = [
        Self::Linear,
        Self::InQuad,
        Self::OutQuad,
        Self::InOutQuad,
        Self::InCubic,
        Self::OutCubic,
        Self::InOutCubic,
        Self::InOutSine,
        Self::OutBack,
        Self::OutBounce,
        Self::OutElastic,
    ];
}

/// What happens when a tween reaches its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Repeat {
    /// Stop at the end; [`Tween::is_finished`] turns true and the end pose stays.
    #[default]
    Once,
    /// Jump back to the start and go again, forever.
    Loop,
    /// Go back the way it came, then forward again, forever.
    PingPong,
}

impl Repeat {
    /// Where along one pass a tween `elapsed` seconds into a `duration`-second run is, `0..=1`
    /// before easing, and whether a [`Once`](Self::Once) run has reached its end. A zero
    /// duration is at its end at once.
    ///
    /// Shared by every kind of tween (this crate's [`Tween`], the renderer's colour tween), so
    /// they loop and ping-pong identically.
    #[must_use]
    pub fn position(self, elapsed: f32, duration: f32) -> (f32, bool) {
        if duration <= 0.0 {
            return (1.0, self == Self::Once);
        }
        let pass = elapsed / duration;
        let t = match self {
            Self::Once => pass.min(1.0),
            Self::Loop => pass.fract(),
            Self::PingPong => {
                let p = pass % 2.0;
                if p <= 1.0 { p } else { 2.0 - p }
            }
        };
        (t, self == Self::Once && t >= 1.0)
    }
}

/// The parts of a `Transform` a tween drives. A part not set is left alone, so a tween of the
/// position does not fight an animation of the rotation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Pose {
    position: Option<Vec3>,
    rotation: Option<Quat>,
    scale: Option<Vec3>,
}

/// Eases an entity's `Transform` to a target over `duration` seconds — see the [module](self).
///
/// The start is taken from the entity the first frame the tween runs, so a tween can be built
/// before anyone knows where the entity will be (and replacing a running tween with a new one
/// starts the new one from wherever the old one had got to — no jump).
#[derive(Debug, Clone, PartialEq)]
pub struct Tween {
    from: Option<Pose>,
    to: Pose,
    duration: f32,
    elapsed: f32,
    ease: Ease,
    repeat: Repeat,
    finished: bool,
}

impl Tween {
    fn new(to: Pose, seconds: f32) -> Self {
        Self { from: None, to, duration: seconds.max(0.0), elapsed: 0.0, ease: Ease::Linear, repeat: Repeat::Once, finished: false }
    }

    /// To `position` in `seconds`.
    #[must_use]
    pub fn to_position(position: Vec3, seconds: f32) -> Self {
        Self::new(Pose { position: Some(position), ..Pose::default() }, seconds)
    }

    /// To `rotation` in `seconds`, along the shorter arc.
    #[must_use]
    pub fn to_rotation(rotation: Quat, seconds: f32) -> Self {
        Self::new(Pose { rotation: Some(rotation), ..Pose::default() }, seconds)
    }

    /// To `scale` in `seconds`.
    #[must_use]
    pub fn to_scale(scale: Vec3, seconds: f32) -> Self {
        Self::new(Pose { scale: Some(scale), ..Pose::default() }, seconds)
    }

    /// To all of `target`'s position, rotation and scale in `seconds`.
    #[must_use]
    pub fn to_transform(target: &Transform, seconds: f32) -> Self {
        Self::new(
            Pose { position: Some(target.position), rotation: Some(target.rotation), scale: Some(target.scale) },
            seconds,
        )
    }

    /// Also drives the position to `position`.
    #[must_use]
    pub fn and_position(mut self, position: Vec3) -> Self {
        self.to.position = Some(position);
        self
    }

    /// Also drives the rotation to `rotation`.
    #[must_use]
    pub fn and_rotation(mut self, rotation: Quat) -> Self {
        self.to.rotation = Some(rotation);
        self
    }

    /// Also drives the scale to `scale`.
    #[must_use]
    pub fn and_scale(mut self, scale: Vec3) -> Self {
        self.to.scale = Some(scale);
        self
    }

    /// Replaces the curve (linear by default).
    #[must_use]
    pub fn with_ease(mut self, ease: Ease) -> Self {
        self.ease = ease;
        self
    }

    /// Replaces what happens at the end (once by default).
    #[must_use]
    pub fn repeating(mut self, repeat: Repeat) -> Self {
        self.repeat = repeat;
        self
    }

    /// Whether a [`Repeat::Once`] tween has reached its end. A repeating one never finishes.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// How far through the current pass it is, `0..=1`, before easing.
    #[must_use]
    pub fn progress(&self) -> f32 {
        if self.duration <= 0.0 {
            1.0
        } else {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        }
    }

    /// Advances by `dt` and writes the pose into `transform`.
    pub fn advance(&mut self, dt: f32, transform: &mut Transform) {
        if self.finished {
            return;
        }
        let from = *self.from.get_or_insert(Pose {
            position: self.to.position.map(|_| transform.position),
            rotation: self.to.rotation.map(|_| transform.rotation),
            scale: self.to.scale.map(|_| transform.scale),
        });
        self.elapsed += dt.max(0.0);
        // Where along the path, before easing.
        let (t, finished) = self.repeat.position(self.elapsed, self.duration);
        self.finished = finished;
        let e = self.ease.apply(t);
        // At the end of a single pass, land exactly — no float residue from the lerp.
        let exact = self.finished;
        if let (Some(a), Some(b)) = (from.position, self.to.position) {
            transform.position = if exact { b } else { a + (b - a) * e };
        }
        if let (Some(a), Some(b)) = (from.rotation, self.to.rotation) {
            transform.rotation = if exact { b } else { a.slerp(b, e.clamp(0.0, 1.0)) };
        }
        if let (Some(a), Some(b)) = (from.scale, self.to.scale) {
            transform.scale = if exact { b } else { a + (b - a) * e };
        }
    }
}

gizmo_core::impl_component!(Tween);

/// Advances every [`Tween`] by `dt` and writes its pose into the entity's `Transform`.
///
/// The engine's frame driver calls this on the animation clock, before transform propagation, in
/// the game's and the editor's draw paths alike. A finished tween stays on the entity (its
/// [`is_finished`](Tween::is_finished) is how a game learns the move ended) and costs a branch.
pub fn tween_system(world: &mut World, dt: f32) {
    if dt <= 0.0 {
        return;
    }
    if let Some(mut q) = world.query_mut::<(Mut<Tween>, Mut<Transform>)>() {
        for (_, (mut tween, mut transform)) in q.iter_mut() {
            if !tween.is_finished() {
                tween.advance(dt, &mut transform);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_starts_at_zero_and_ends_at_one() {
        for e in Ease::ALL {
            assert!(e.apply(0.0).abs() < 1e-5, "{e:?} at 0 = {}", e.apply(0.0));
            assert!((e.apply(1.0) - 1.0).abs() < 1e-5, "{e:?} at 1 = {}", e.apply(1.0));
            assert_eq!(e.apply(-1.0), e.apply(0.0), "{e:?} must clamp below");
            assert_eq!(e.apply(2.0), e.apply(1.0), "{e:?} must clamp above");
        }
        // The shapes: out-curves are ahead at the half, in-curves behind, back overshoots.
        assert!(Ease::OutCubic.apply(0.5) > 0.5 && Ease::InCubic.apply(0.5) < 0.5);
        assert!((Ease::InOutQuad.apply(0.5) - 0.5).abs() < 1e-6);
        assert!((0..100).any(|i| Ease::OutBack.apply(i as f32 / 100.0) > 1.0), "OutBack never overshot");
        assert!((0..100).all(|i| Ease::OutBounce.apply(i as f32 / 100.0) <= 1.0 + 1e-6), "a bounce never passes the end");
    }

    #[test]
    fn a_tween_starts_where_the_entity_is_and_lands_exactly() {
        let mut t = Transform::new(Vec3::new(1.0, 0.0, 0.0));
        let mut tw = Tween::to_position(Vec3::new(3.0, 0.0, 0.0), 1.0);
        tw.advance(0.5, &mut t);
        assert!((t.position.x - 2.0).abs() < 1e-5, "linear half-way: {}", t.position.x);
        assert!(!tw.is_finished());
        tw.advance(0.7, &mut t);
        assert!(tw.is_finished());
        assert_eq!(t.position, Vec3::new(3.0, 0.0, 0.0), "the end is exact");
        // A finished tween does nothing more.
        t.position = Vec3::ZERO;
        tw.advance(1.0, &mut t);
        assert_eq!(t.position, Vec3::ZERO);
    }

    /// A tween of one part leaves the others alone.
    #[test]
    fn only_the_parts_asked_for_are_driven() {
        let mut t = Transform::new(Vec3::new(5.0, 5.0, 5.0)).with_scale(Vec3::splat(2.0));
        let q = Quat::from_rotation_y(1.0);
        let mut tw = Tween::to_rotation(q, 1.0);
        tw.advance(1.0, &mut t);
        assert!(t.rotation.angle_between(q) < 1e-5);
        assert_eq!(t.position, Vec3::new(5.0, 5.0, 5.0));
        assert_eq!(t.scale, Vec3::splat(2.0));
        let mut both = Tween::to_scale(Vec3::ONE, 1.0).and_position(Vec3::ZERO);
        both.advance(1.0, &mut t);
        assert_eq!((t.position, t.scale), (Vec3::ZERO, Vec3::ONE));
    }

    #[test]
    fn loop_wraps_and_ping_pong_comes_back() {
        let mut t = Transform::new(Vec3::ZERO);
        let mut lp = Tween::to_position(Vec3::X, 1.0).repeating(Repeat::Loop);
        lp.advance(1.25, &mut t);
        assert!((t.position.x - 0.25).abs() < 1e-5, "loop at 1.25 s: {}", t.position.x);
        assert!(!lp.is_finished());

        let mut t = Transform::new(Vec3::ZERO);
        let mut pp = Tween::to_position(Vec3::X, 1.0).repeating(Repeat::PingPong);
        pp.advance(1.25, &mut t);
        assert!((t.position.x - 0.75).abs() < 1e-5, "ping-pong at 1.25 s: {}", t.position.x);
        pp.advance(0.75, &mut t);
        assert!(t.position.x.abs() < 1e-5, "back at the start at 2 s: {}", t.position.x);
    }

    #[test]
    fn a_zero_duration_tween_lands_at_once() {
        let mut t = Transform::new(Vec3::ZERO);
        let mut tw = Tween::to_position(Vec3::Y, 0.0);
        tw.advance(0.016, &mut t);
        assert_eq!(t.position, Vec3::Y);
        assert!(tw.is_finished());
    }

    /// The system drives every entity's tween through its `Transform`, and a paused clock (dt 0)
    /// moves nothing.
    #[test]
    fn the_system_drives_the_transform() {
        let mut world = World::new();
        let e = world.spawn_bundle((Transform::new(Vec3::ZERO), Tween::to_position(Vec3::new(0.0, 2.0, 0.0), 1.0)));
        tween_system(&mut world, 0.0);
        assert_eq!(world.borrow::<Transform>().get(e.id()).unwrap().position, Vec3::ZERO, "a paused clock moved it");
        tween_system(&mut world, 0.5);
        assert!((world.borrow::<Transform>().get(e.id()).unwrap().position.y - 1.0).abs() < 1e-5);
    }
}
