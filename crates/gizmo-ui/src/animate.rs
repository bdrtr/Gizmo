//! Moving and fading UI boxes: [`UiTransform`], and [`UiTween`] to animate it.
//!
//! A box's place comes from its [`Style`], recomputed by the layout every frame, so
//! sliding a card from the deck to the hand, letting a dragged item follow the pointer or sending
//! it back when it is dropped in the wrong place meant rewriting `inset`s by hand each frame —
//! `birlestir` turned its dragged item `Absolute` for it, `duello` would have done it for every
//! card. A [`UiTransform`] is applied **after** layout, the way a CSS `transform` is: the box and
//! everything in it is drawn and hit-tested where it is moved to, and nothing around it reflows.

use gizmo_core::Entity;
use gizmo_core::commands::Commands;
use gizmo_core::query::Query;
use gizmo_core::system::Res;
use gizmo_math::{Ease, Vec2};

/// Where a UI box is drawn relative to where the layout put it, and how opaque it is.
///
/// - `translation`, in layout pixels (scaled by [`UiScale`](crate::UiScale) like every `Val::Px`):
///   moves the box **and its children**; clicks, drags and drop targets follow it, since they all
///   read the moved [`Node`](crate::Node). Siblings do not move to fill the gap.
/// - `opacity`, `0..=1`: multiplies the alpha of the box's background and text and of everything
///   under it — a whole card fades, not only its face. Applied by `gizmo-engine`'s draw path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiTransform {
    /// The offset from the laid-out place, in layout pixels.
    pub translation: Vec2,
    /// `1` opaque, `0` invisible (still laid out and still hit-tested).
    pub opacity: f32,
}

impl Default for UiTransform {
    fn default() -> Self {
        Self { translation: Vec2::ZERO, opacity: 1.0 }
    }
}

/// Eases an element's [`UiTransform`] from wherever it is to a target, over a time.
///
/// Starts from the transform's value **when it starts** (after any delay), not when it was added,
/// so a tween queued behind a drag picks up where the drag left it. An element without a
/// `UiTransform` is given a default one, and the tween runs from the next frame. Runs on real
/// time — a paused game's menus still move. Replacing the component replaces the animation;
/// a finished one stays, reporting [`is_finished`](Self::is_finished), until removed or replaced.
///
/// ```
/// # use gizmo_ui::UiTween;
/// # use gizmo_math::{Ease, Vec2};
/// // Back to its place, overshooting a little; and a fade-out on the way to a target.
/// let home = UiTween::translation(Vec2::ZERO, 0.3).with_ease(Ease::OutBack);
/// let away = UiTween::translation(Vec2::new(300.0, -200.0), 0.35).and_opacity(0.0);
/// assert!(!home.is_finished() && !away.is_finished());
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiTween {
    translation: Option<Vec2>,
    opacity: Option<f32>,
    seconds: f32,
    ease: Ease,
    delay: f32,
    elapsed: f32,
    from: Option<(Vec2, f32)>,
    finished: bool,
}

impl UiTween {
    fn new(seconds: f32) -> Self {
        Self {
            translation: None,
            opacity: None,
            seconds: seconds.max(0.0),
            ease: Ease::Linear,
            delay: 0.0,
            elapsed: 0.0,
            from: None,
            finished: false,
        }
    }

    /// Moves the translation to `to` in `seconds`.
    #[must_use]
    pub fn translation(to: Vec2, seconds: f32) -> Self {
        Self { translation: Some(to), ..Self::new(seconds) }
    }

    /// Fades the opacity to `to` in `seconds`.
    #[must_use]
    pub fn opacity(to: f32, seconds: f32) -> Self {
        Self { opacity: Some(to.clamp(0.0, 1.0)), ..Self::new(seconds) }
    }

    /// Also moves the translation to `to`, on the same clock.
    #[must_use]
    pub fn and_translation(mut self, to: Vec2) -> Self {
        self.translation = Some(to);
        self
    }

    /// Also fades the opacity to `to`, on the same clock.
    #[must_use]
    pub fn and_opacity(mut self, to: f32) -> Self {
        self.opacity = Some(to.clamp(0.0, 1.0));
        self
    }

    /// With an easing curve (default [`Ease::Linear`]).
    #[must_use]
    pub fn with_ease(mut self, ease: Ease) -> Self {
        self.ease = ease;
        self
    }

    /// Waits `seconds` before starting — a hand of cards dealt one after another.
    #[must_use]
    pub fn with_delay(mut self, seconds: f32) -> Self {
        self.delay = seconds.max(0.0);
        self
    }

    /// It has arrived (and set the targets exactly).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Advances it by `dt` and writes the result into `t`. Returns whether `t` changed.
    pub(crate) fn step(&mut self, t: &mut UiTransform, mut dt: f32) -> bool {
        if self.finished {
            return false;
        }
        if self.delay > 0.0 {
            self.delay -= dt;
            if self.delay > 0.0 {
                return false;
            }
            // What the wait overran is time the animation has already been running.
            dt = -self.delay;
            self.delay = 0.0;
        }
        let (from_t, from_o) = *self.from.get_or_insert((t.translation, t.opacity));
        self.elapsed += dt;
        let done = self.elapsed >= self.seconds;
        let k = if done { 1.0 } else { self.ease.apply(self.elapsed / self.seconds) };
        let before = *t;
        if let Some(to) = self.translation {
            t.translation = if done { to } else { from_t + (to - from_t) * k };
        }
        if let Some(to) = self.opacity {
            // Overshooting curves may carry a position past its target; an opacity stays in range.
            t.opacity = if done { to } else { (from_o + (to - from_o) * k).clamp(0.0, 1.0) };
        }
        self.finished = done;
        *t != before
    }
}

/// Advances every [`UiTween`], and gives an element that has a tween but no [`UiTransform`] a
/// default one. Runs before the layout, on real time.
pub fn ui_tween_system(
    time: Option<Res<gizmo_core::time::Time>>,
    mut tweens: Query<(Entity, &mut UiTween, Option<&mut UiTransform>)>,
    mut commands: Commands,
) {
    let dt = time.map_or(0.0, |t| t.raw_dt());
    let mut bare = Vec::new();
    for (entity, mut tween, transform) in tweens.iter_mut() {
        let Some(mut transform) = transform else {
            bare.push(entity);
            continue;
        };
        if tween.is_finished() {
            continue;
        }
        let mut next = *transform;
        let mut tw = *tween;
        if tw.step(&mut next, dt) {
            *transform = next;
        }
        *tween = tw;
    }
    for entity in bare {
        commands.entity(entity).insert(UiTransform::default());
    }
}

gizmo_core::impl_component!(UiTransform, UiTween);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tween_waits_eases_from_where_it_starts_and_lands_exactly() {
        let mut t = UiTransform { translation: Vec2::new(100.0, 0.0), opacity: 1.0 };
        let mut tw = UiTween::translation(Vec2::ZERO, 1.0).and_opacity(0.0).with_delay(0.5);
        assert!(!tw.step(&mut t, 0.3), "still waiting");
        // Moved meanwhile (a drag): the tween starts from here, not from where it was added.
        t.translation = Vec2::new(200.0, 0.0);
        // 0.3 more: 0.1 past the wait, so 0.1 s in.
        assert!(tw.step(&mut t, 0.3));
        assert!((t.translation.x - 180.0).abs() < 1e-3, "{:?}", t.translation);
        assert!((t.opacity - 0.9).abs() < 1e-5);
        tw.step(&mut t, 0.4);
        assert!((t.translation.x - 100.0).abs() < 1e-3);
        assert!(!tw.is_finished());
        tw.step(&mut t, 0.75);
        assert!(tw.is_finished());
        assert_eq!(t, UiTransform { translation: Vec2::ZERO, opacity: 0.0 }, "lands exactly");
        assert!(!tw.step(&mut t, 0.1), "a finished tween writes nothing");
    }

    #[test]
    fn only_the_named_fields_move_and_opacity_stays_in_range() {
        let mut t = UiTransform { translation: Vec2::new(5.0, 5.0), opacity: 0.5 };
        let mut tw = UiTween::opacity(1.0, 1.0).with_ease(Ease::OutBack);
        for _ in 0..9 {
            tw.step(&mut t, 0.1);
            assert!(t.opacity <= 1.0, "an overshooting curve took opacity to {}", t.opacity);
            assert_eq!(t.translation, Vec2::new(5.0, 5.0), "translation was not asked for");
        }
        // A zero-length tween lands on its first step.
        let mut tw = UiTween::translation(Vec2::ONE, 0.0);
        tw.step(&mut t, 0.0);
        assert!(tw.is_finished());
        assert_eq!(t.translation, Vec2::ONE);
    }
}
