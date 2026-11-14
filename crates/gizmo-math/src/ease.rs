//! Easing curves — [`Ease`], shared by `gizmo-animation`'s `Tween` and `gizmo-ui`'s `UiTween`.

/// How progress through an animation maps onto progress along it.
///
/// Every curve starts at 0 and ends at 1. `OutBack` and `OutElastic` overshoot past 1 on the way
/// — that is what they are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_runs_from_zero_to_one_and_clamps() {
        for e in Ease::ALL {
            assert_eq!(e.apply(0.0), 0.0, "{e:?}");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-6, "{e:?}");
            assert_eq!(e.apply(-1.0), e.apply(0.0), "{e:?}");
            assert_eq!(e.apply(2.0), e.apply(1.0), "{e:?}");
        }
    }
}
