//! [`ColorTween`] — a colour eased to a target: fading out, flashing, tinting.

use gizmo_animation::tween::{Ease, Repeat};
use gizmo_core::World;
use gizmo_math::Vec4;

use crate::components::{Material, Sprite, Text};

/// Eases an entity's colour to `to` over a duration — the colour of its [`Sprite`], its
/// [`Text`] and its [`Material`] (albedo), whichever it has.
///
/// [`Tween`](gizmo_animation::tween::Tween) drives a `Transform` and nothing else, so a balloon
/// that grows as it pops could not also fade: `balon` faded its sprite's alpha by hand, with its
/// own timer. Same curves ([`Ease`]), same repeats ([`Repeat`]), same rule for where it starts —
/// the colour the entity has the first frame it runs.
///
/// ```
/// use gizmo_renderer::components::ColorTween;
/// use gizmo_renderer::tween::Ease;
/// use gizmo_math::Vec4;
///
/// // Fade to transparent in a fifth of a second.
/// let fade = ColorTween::to(Vec4::new(1.0, 1.0, 1.0, 0.0), 0.2).with_ease(Ease::InQuad);
/// assert!(!fade.is_finished());
/// ```
///
/// Advanced by the draw path on the animation clock, in both the game's and the editor's.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ColorTween {
    from: Option<Vec4>,
    to: Vec4,
    duration: f32,
    elapsed: f32,
    ease: Ease,
    repeat: Repeat,
    finished: bool,
    delay: f32,
}

impl ColorTween {
    /// To `color` (linear RGBA) in `seconds`.
    #[must_use]
    pub fn to(color: Vec4, seconds: f32) -> Self {
        Self {
            from: None,
            to: color,
            duration: seconds.max(0.0),
            elapsed: 0.0,
            ease: Ease::Linear,
            repeat: Repeat::Once,
            finished: false,
            delay: 0.0,
        }
    }

    /// To the current colour with alpha `alpha` — a fade, without naming the colour. The colour
    /// is read the first frame it runs.
    #[must_use]
    pub fn fade_to(alpha: f32, seconds: f32) -> Self {
        // A NaN marks "keep the start's RGB"; resolved when the start is captured.
        Self::to(Vec4::new(f32::NAN, f32::NAN, f32::NAN, alpha), seconds)
    }

    /// Replaces the curve (linear by default).
    #[must_use]
    pub fn with_ease(mut self, ease: Ease) -> Self {
        self.ease = ease;
        self
    }

    /// Waits `seconds` before starting, and takes its start colour when it starts — the same rule
    /// as [`Tween::with_delay`](gizmo_animation::tween::Tween::with_delay).
    #[must_use]
    pub fn with_delay(mut self, seconds: f32) -> Self {
        self.delay = if seconds.is_finite() { seconds.max(0.0) } else { 0.0 };
        self
    }

    /// Replaces what happens at the end (once by default).
    #[must_use]
    pub fn repeating(mut self, repeat: Repeat) -> Self {
        self.repeat = repeat;
        self
    }

    /// Whether a [`Repeat::Once`] tween has reached its end.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Advances by `dt` from `current` (the colour the entity has now) and returns the colour to
    /// write.
    pub fn advance(&mut self, dt: f32, current: Vec4) -> Vec4 {
        if self.finished {
            return current;
        }
        let mut dt = dt.max(0.0);
        if self.delay > 0.0 {
            if dt < self.delay {
                self.delay -= dt;
                return current;
            }
            dt -= self.delay;
            self.delay = 0.0;
        }
        if self.from.is_none() {
            if self.to.x.is_nan() {
                self.to = Vec4::new(current.x, current.y, current.z, self.to.w);
            }
            self.from = Some(current);
        }
        let from = self.from.unwrap_or(current);
        self.elapsed += dt;
        let (t, finished) = self.repeat.position(self.elapsed, self.duration);
        self.finished = finished;
        if finished {
            return self.to;
        }
        from + (self.to - from) * self.ease.apply(t)
    }
}

gizmo_core::impl_component!(ColorTween);

/// Advances every [`ColorTween`] by `dt` and writes the colour into the entity's [`Sprite`],
/// [`Text`] and [`Material`] — each one it has. The start is taken from the first of those (in
/// that order). A paused clock (`dt <= 0`) changes nothing.
pub fn color_tween_system(world: &mut World, dt: f32) {
    if dt <= 0.0 {
        return;
    }
    let mut q = world.query_mut::<(&mut ColorTween, Option<&mut Sprite>, Option<&mut Text>, Option<&mut Material>)>();
    for (mut tween, sprite, text, material) in q.iter_mut() {
        if tween.is_finished() {
            continue;
        }
        let current = sprite
            .as_ref()
            .map(|s| s.color)
            .or_else(|| text.as_ref().map(|t| t.color))
            .or_else(|| material.as_ref().map(|m| m.albedo));
        let Some(current) = current else { continue };
        let c = tween.advance(dt, current);
        if let Some(mut s) = sprite {
            s.color = c;
        }
        if let Some(mut t) = text {
            t.color = c;
        }
        if let Some(mut m) = material {
            m.albedo = c;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_eases_from_where_it_is_and_lands_exactly() {
        let mut t = ColorTween::to(Vec4::new(0.0, 0.0, 0.0, 0.0), 1.0);
        let c = t.advance(0.5, Vec4::ONE);
        assert!((c - Vec4::splat(0.5)).length() < 1e-5, "{c:?}");
        let c = t.advance(0.6, c);
        assert!(t.is_finished());
        assert_eq!(c, Vec4::ZERO);
    }

    /// A fade keeps the colour and moves only alpha.
    /// Delayed: the colour holds, the start is taken when it starts, the rest of the frame counts.
    #[test]
    fn a_delayed_colour_tween_waits_then_starts_from_the_colour_then() {
        let mut t = ColorTween::to(Vec4::ZERO, 1.0).with_delay(0.5);
        assert_eq!(t.advance(0.4, Vec4::ONE), Vec4::ONE);
        let c = t.advance(0.6, Vec4::new(0.5, 0.5, 0.5, 1.0)); // 0.1 left of the delay, 0.5 of the tween
        assert!((c.x - 0.25).abs() < 1e-5 && (c.w - 0.5).abs() < 1e-5, "{c:?}");
    }

    #[test]
    fn a_fade_moves_only_alpha() {
        let mut t = ColorTween::fade_to(0.0, 1.0);
        let red = Vec4::new(1.0, 0.2, 0.1, 1.0);
        let c = t.advance(0.5, red);
        assert_eq!((c.x, c.y, c.z), (1.0, 0.2, 0.1));
        assert!((c.w - 0.5).abs() < 1e-5);
        assert_eq!(t.advance(1.0, c), Vec4::new(1.0, 0.2, 0.1, 0.0));
    }

    /// The system writes every colour the entity has, and a paused clock writes none.
    #[test]
    fn the_system_writes_the_sprite_and_the_text() {
        let mut world = World::new();
        let image = crate::sprite::SpriteImageId(0);
        let font = crate::text::FontId(0);
        let e = world.spawn_bundle((
            Sprite::world(image, gizmo_math::Vec2::ONE),
            Text::screen("x", font, 10.0, gizmo_math::Vec2::ZERO),
            ColorTween::to(Vec4::new(0.0, 0.0, 0.0, 1.0), 1.0),
        ));
        color_tween_system(&mut world, 0.0);
        assert_eq!(world.query::<&Sprite>().get(e).unwrap().color, Vec4::ONE, "a paused clock changed it");
        color_tween_system(&mut world, 1.0);
        assert_eq!(world.query::<&Sprite>().get(e).unwrap().color, Vec4::new(0.0, 0.0, 0.0, 1.0));
        assert_eq!(world.query::<&Text>().get(e).unwrap().color, Vec4::new(0.0, 0.0, 0.0, 1.0));
    }
}
