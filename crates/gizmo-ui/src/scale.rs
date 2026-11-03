//! [`UiScale`] — one number that makes every pixel of a layout bigger or smaller.

use gizmo_math::Vec2;

/// How big a UI pixel is on screen: every [`Val::Px`](crate::Val::Px) of every `Style` (sizes,
/// padding, gaps, insets) is multiplied by it, and so is the font size of a `Text` that sits on a
/// UI node. Insert it as a resource; without one the scale is `1`.
///
/// A menu laid out for 1280×720 does not fit a 472×492 window, which is what a tiling window
/// manager hands a game — `birlestir` measured it. [`fit`](Self::fit) names the size the layout
/// was designed for and scales it to the window every frame, the whole layout or nothing cut off;
/// [`fixed`](Self::fixed) is a plain factor, for a "UI size" setting or a high-DPI screen.
///
/// The layout is computed in *layout pixels* (the window divided by the factor) and written to
/// each [`Node`](crate::Node) in window pixels, so everything that reads a `Node` — hit testing,
/// drags, drop targets, the facade's drawing — needs to know nothing. A
/// [`Scroll`](crate::Scroll)'s offset stays in window pixels for the same reason.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiScale {
    design: Option<Vec2>,
    factor: f32,
}

impl Default for UiScale {
    fn default() -> Self {
        Self::fixed(1.0)
    }
}

impl UiScale {
    /// The smallest factor allowed, so a minimised window does not divide by zero.
    pub const MIN: f32 = 0.05;

    /// A fixed factor: `2.0` draws every layout pixel as two window pixels.
    #[must_use]
    pub fn fixed(factor: f32) -> Self {
        let factor = if factor.is_finite() { factor.max(Self::MIN) } else { 1.0 };
        Self { design: None, factor }
    }

    /// Scales a layout designed for `width` × `height` to the window: the largest factor at which
    /// both still fit, up or down, recomputed every frame as the window changes.
    #[must_use]
    pub fn fit(width: f32, height: f32) -> Self {
        Self { design: Some(Vec2::new(width.max(1.0), height.max(1.0))), factor: 1.0 }
    }

    /// The factor in effect — for `fit`, the one the last layout used.
    #[must_use]
    pub fn factor(&self) -> f32 {
        self.factor
    }

    pub(crate) fn set_factor(&mut self, factor: f32) {
        self.factor = factor;
    }

    /// The factor for a window of `window` pixels.
    #[must_use]
    pub fn factor_for(&self, window: Vec2) -> f32 {
        match self.design {
            Some(d) if window.x > 0.0 && window.y > 0.0 => (window.x / d.x).min(window.y / d.y).max(Self::MIN),
            Some(_) => self.factor,
            None => self.factor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_takes_the_tighter_axis_and_fixed_is_fixed() {
        let s = UiScale::fit(1280.0, 720.0);
        assert_eq!(s.factor_for(Vec2::new(640.0, 720.0)), 0.5);
        assert_eq!(s.factor_for(Vec2::new(2560.0, 2000.0)), 2.0);
        assert_eq!(s.factor_for(Vec2::new(0.0, 0.0)), 1.0, "a minimised window keeps the last factor");
        assert_eq!(UiScale::fixed(1.5).factor_for(Vec2::new(100.0, 100.0)), 1.5);
        assert_eq!(UiScale::fixed(0.0).factor(), UiScale::MIN);
        assert_eq!(UiScale::fixed(f32::NAN).factor(), 1.0);
    }
}
