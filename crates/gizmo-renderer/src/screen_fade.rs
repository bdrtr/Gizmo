//! [`ScreenFade`](crate::screen_fade::ScreenFade) — a colour over the whole picture: fading to black between levels, a red flash
//! when the player is hit.

use gizmo_core::World;
use gizmo_math::{Vec3, Vec4};

/// A colour laid over the whole frame, above every UI layer, and moved over time — a resource.
///
/// A scene change wants the screen to go dark, the level to swap behind it, and the picture to
/// come back; a hit wants a red flash. Before this a game built a full-window UI node and faded
/// it by hand: `ColorTween` drives sprites, text and materials, not a UI background, and a node
/// sits on *a* layer rather than above all of them.
///
/// ```
/// use gizmo_renderer::screen_fade::ScreenFade;
///
/// let mut fade = ScreenFade::default();
/// fade.fade_out(0.5);
/// fade.advance(0.25);
/// assert_eq!(fade.alpha(), 0.5);
/// fade.advance(0.25);
/// assert!(fade.just_finished() && fade.is_opaque()); // swap the level now
/// fade.fade_in(0.5);
/// ```
///
/// Both draw paths advance it every frame on the **real** frame time (so a fade runs while the
/// game is paused) and paint it over everything. Insert one with
/// `world.get_resource_mut_or_default::<ScreenFade>()`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenFade {
    color: Vec3,
    alpha: f32,
    from: f32,
    to: f32,
    secs: f32,
    t: f32,
    fading: bool,
    just_finished: bool,
}

impl Default for ScreenFade {
    /// Clear, black, still.
    fn default() -> Self {
        Self { color: Vec3::ZERO, alpha: 0.0, from: 0.0, to: 0.0, secs: 0.0, t: 0.0, fading: false, just_finished: false }
    }
}

impl ScreenFade {
    /// Fades to fully covered over `secs` seconds, in the current colour (black unless
    /// [`set_color`](Self::set_color) said otherwise).
    pub fn fade_out(&mut self, secs: f32) {
        self.fade_to(1.0, secs);
    }

    /// Fades to clear over `secs` seconds.
    pub fn fade_in(&mut self, secs: f32) {
        self.fade_to(0.0, secs);
    }

    /// Fades to `alpha` (`0` clear … `1` covered) over `secs` seconds from wherever it is. `secs
    /// <= 0` lands on the next [`advance`](Self::advance).
    pub fn fade_to(&mut self, alpha: f32, secs: f32) {
        self.from = self.alpha;
        self.to = clamp01(alpha);
        self.secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        self.t = 0.0;
        self.fading = true;
        self.just_finished = false;
    }

    /// Jumps to `color` at `alpha` and fades back to clear over `secs` — a hit, a pickup.
    pub fn flash(&mut self, color: Vec3, alpha: f32, secs: f32) {
        self.color = color;
        self.alpha = clamp01(alpha);
        self.fade_in(secs);
    }

    /// The colour it covers the screen with. Linear, like every colour in the engine.
    pub fn set_color(&mut self, color: Vec3) {
        self.color = color;
    }

    /// Clear at once, no fade.
    pub fn clear(&mut self) {
        *self = Self { color: self.color, ..Self::default() };
    }

    /// Advances the fade by `dt` seconds. The draw paths call this; a game does not.
    pub fn advance(&mut self, dt: f32) {
        self.just_finished = false;
        if !self.fading {
            return;
        }
        if dt.is_finite() && dt > 0.0 {
            self.t += dt;
        }
        let k = if self.secs <= 0.0 { 1.0 } else { (self.t / self.secs).min(1.0) };
        self.alpha = self.from + (self.to - self.from) * k;
        if k >= 1.0 {
            self.alpha = self.to;
            self.fading = false;
            self.just_finished = true;
        }
    }

    /// How covered the screen is, `0..=1`.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// Whether a fade is under way.
    #[must_use]
    pub fn is_fading(&self) -> bool {
        self.fading
    }

    /// Whether the last [`advance`](Self::advance) ended a fade — `true` for that one frame: the
    /// moment to swap a level behind a black screen.
    #[must_use]
    pub fn just_finished(&self) -> bool {
        self.just_finished
    }

    /// Whether the screen is fully covered.
    #[must_use]
    pub fn is_opaque(&self) -> bool {
        self.alpha >= 1.0
    }

    /// What to paint over the frame, or `None` while clear.
    #[must_use]
    pub fn overlay(&self) -> Option<Vec4> {
        (self.alpha > 0.0).then(|| self.color.extend(self.alpha))
    }
}

fn clamp01(a: f32) -> f32 {
    if a.is_nan() {
        0.0
    } else {
        a.clamp(0.0, 1.0)
    }
}

/// Advances the world's [`ScreenFade`], if it has one, by `dt` (the real frame time). Called by
/// both draw paths.
pub fn screen_fade_system(world: &mut World, dt: f32) {
    if let Some(mut fade) = world.get_resource_mut::<ScreenFade>() {
        fade.advance(dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_runs_linearly_lands_and_reports_its_end_once() {
        let mut f = ScreenFade::default();
        assert_eq!(f.overlay(), None);
        f.fade_out(1.0);
        f.advance(0.25);
        assert_eq!(f.alpha(), 0.25);
        assert!(!f.just_finished() && f.is_fading());
        f.advance(2.0);
        assert_eq!(f.alpha(), 1.0);
        assert!(f.just_finished() && f.is_opaque() && !f.is_fading());
        f.advance(0.1);
        assert!(!f.just_finished(), "only the frame it ended");
        assert_eq!(f.overlay(), Some(Vec4::new(0.0, 0.0, 0.0, 1.0)));
        // Reversed mid-way: from where it is, not from the far end.
        f.fade_in(1.0);
        f.advance(0.5);
        f.fade_out(1.0);
        f.advance(0.0);
        assert_eq!(f.alpha(), 0.5);
    }

    #[test]
    fn a_flash_jumps_then_clears_and_odd_inputs_are_tamed() {
        let mut f = ScreenFade::default();
        f.flash(Vec3::new(1.0, 0.0, 0.0), 0.6, 0.3);
        assert_eq!(f.overlay(), Some(Vec4::new(1.0, 0.0, 0.0, 0.6)), "visible before any advance");
        f.advance(0.3);
        assert_eq!(f.overlay(), None);
        f.fade_to(7.0, f32::NAN);
        f.advance(f32::NAN);
        assert_eq!(f.alpha(), 1.0, "clamped, and a NaN duration is instant");
        f.clear();
        assert_eq!(f.alpha(), 0.0);
        assert!(!f.is_fading());
    }

    #[test]
    fn the_system_advances_the_resource_and_ignores_a_world_without_one() {
        let mut world = World::new();
        screen_fade_system(&mut world, 1.0);
        world.insert_resource(ScreenFade::default());
        world.get_resource_mut::<ScreenFade>().unwrap().fade_out(1.0);
        screen_fade_system(&mut world, 0.5);
        assert_eq!(world.get_resource::<ScreenFade>().unwrap().alpha(), 0.5);
    }
}
