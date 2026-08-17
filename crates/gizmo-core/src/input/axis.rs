//! Movement read from the keyboard and a stick at once, as one analog direction.
//!
//! # Why this exists
//!
//! Every demo in this repository wrote its own version of "WASD moves the character", and the
//! versions disagree. Measured on 2026-08-17 across the 17 that read movement keys: thirteen
//! accumulate a direction and normalise it, and **four add a full-speed step per key** —
//! `showcase`, `cpu_physics`, `ocean_scene` and `advanced_physics` — so holding W and D together
//! moved them 41 % faster than either key alone. That is not an exotic bug; it is the default
//! outcome of writing four independent `if`s, which is the shape everyone writes first.
//!
//! Sixteen of the seventeen also had no stick support at all, and the one that did
//! (`platformer`) had to hand-roll the blend, because the two inputs are not the same kind of
//! thing:
//!
//! * A **key is binary.** Its direction is all it carries, and a diagonal press is √2 long, so
//!   it has to be brought back to unit length — twice over, as [`blend_move_axis`] explains.
//! * A **stick carries direction *and* amount.** Half a tilt is a walk and a full tilt is a run —
//!   the one thing a key cannot express — so its magnitude must survive into the result.
//!
//! [`blend_move_axis`] is those two rules in one place, and [`Input::move_axis`] is it wired to
//! the live keyboard and pad. The result is a vector in the **closed unit disc**: `x` to the
//! right, `y` forward, never longer than 1. It is deliberately basis-free — the caller multiplies
//! it by whatever right/forward vectors it already has — because the demos disagree about that
//! too (some work in world space, some in a camera-local frame) and that part is genuinely
//! theirs.

use super::{Gamepad, Input};

/// The four keys that stand in for a movement stick.
///
/// Codes are the caller's convention, as everywhere else in this module — but [`MoveKeys::WASD`]
/// and [`MoveKeys::ARROWS`] are the desktop one, and they are tied to the already-verified
/// [`NAMED_KEYS`](super::NAMED_KEYS) table by a test rather than being a second set of literals
/// to get wrong. (There has been one such second set before: the Lua API carried USB HID codes,
/// where `down` and `right` are winit's *ArrowRight* and *ArrowDown*, so a script reading the
/// arrows moved right when the player pressed down.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveKeys {
    /// The key that pushes `+y`.
    pub forward: u32,
    /// The key that pushes `-y`.
    pub back: u32,
    /// The key that pushes `-x`.
    pub left: u32,
    /// The key that pushes `+x`.
    pub right: u32,
}

impl MoveKeys {
    /// W / A / S / D, in the desktop key-code convention.
    pub const WASD: Self = Self {
        forward: 41, // KeyCode::KeyW
        back: 37,    // KeyCode::KeyS
        left: 19,    // KeyCode::KeyA
        right: 22,   // KeyCode::KeyD
    };

    /// The arrow keys, in the desktop key-code convention.
    pub const ARROWS: Self = Self {
        forward: 82, // KeyCode::ArrowUp
        back: 79,    // KeyCode::ArrowDown
        left: 80,    // KeyCode::ArrowLeft
        right: 81,   // KeyCode::ArrowRight
    };
}

impl Default for MoveKeys {
    /// [`MoveKeys::WASD`].
    fn default() -> Self {
        Self::WASD
    }
}

/// Blends a key direction with a stick into the closed unit disc.
///
/// `keys` is the raw sum of the pressed keys — each component in `-1..=1`, so a diagonal arrives
/// as `(±1, ±1)` and is *longer* than a straight push. `stick` is an already-deadzoned stick
/// vector ([`Gamepad::left_stick`]).
///
/// Two corrections happen here, and they do **different** jobs — which is worth stating, because
/// the obvious explanation of the first one is wrong:
///
/// 1. **The sum is clamped to length 1, not per axis.** This is what bounds the speed, including
///    for the pure-keyboard diagonal: `(1, 1)` is √2 ≈ 1.41 long and comes back out at 1. It is
///    the correction the four demos above were missing entirely. Clamping *per component* instead
///    would leave the corners of a square reachable, i.e. the same 41 % by another route — the
///    same reason the pad's own deadzone is radial
///    (see [`apply_stick_deadzone`](super::apply_stick_deadzone)).
/// 2. **The key direction is normalised *before* the stick is added.** This one does not affect
///    the magnitude at all — the clamp above would fix that anyway — it makes the two inputs
///    **comparable**, so that they compose:
///
///    A stick can be at most 1 long. An un-normalised diagonal key push is √2 long. So without
///    this, a stick held hard against a held W+D can only cancel 71 % of it: the player pushes
///    back with everything the hardware has and still drifts diagonally. Normalising first is
///    what makes "the stick opposes the keys" mean the same thing in every direction.
///
/// Keys and stick *add*, so they compose the way a player expects: a stick tilted against a held
/// key cancels it, and a stick already at full tilt cannot be pushed past full speed by also
/// holding a key.
///
/// With no pad connected `stick` is `(0, 0)` and the result is exactly the normalised key
/// direction — the same expression the demos that got this right were already computing.
///
/// ```
/// use gizmo_core::input::blend_move_axis;
///
/// // One key: full speed.
/// assert_eq!(blend_move_axis((0.0, 1.0), (0.0, 0.0)), (0.0, 1.0));
///
/// // Two keys: still full speed, not 1.41×.
/// let (x, y) = blend_move_axis((1.0, 1.0), (0.0, 0.0));
/// assert!(((x * x + y * y).sqrt() - 1.0).abs() < 1e-6);
///
/// // Half a stick is a walk — the amount a key cannot express.
/// assert_eq!(blend_move_axis((0.0, 0.0), (0.0, 0.5)), (0.0, 0.5));
/// ```
#[must_use]
pub fn blend_move_axis(keys: (f32, f32), stick: (f32, f32)) -> (f32, f32) {
    let (kx, ky) = keys;
    let k_len = (kx * kx + ky * ky).sqrt();
    // Not for the magnitude — the clamp below already bounds that, and removing this line leaves
    // every speed assertion green. It is so that a full-tilt stick can cancel a diagonal key
    // push, which a √2-long key vector makes impossible.
    //
    // `> 1.0` rather than `> 0.0`: a single key is already unit length and must not be perturbed
    // by a divide, and no key at all must not divide by zero.
    let (kx, ky) = if k_len > 1.0 {
        (kx / k_len, ky / k_len)
    } else {
        (kx, ky)
    };

    let (x, y) = (kx + stick.0, ky + stick.1);
    let len = (x * x + y * y).sqrt();
    if len > 1.0 {
        (x / len, y / len)
    } else {
        (x, y)
    }
}

impl Input {
    /// The movement direction from WASD and the left stick — see [`blend_move_axis`].
    ///
    /// `x` right, `y` forward, never longer than 1. Multiply it by the caller's own right and
    /// forward vectors:
    ///
    /// ```
    /// # use gizmo_core::prelude::*;
    /// // `move_right` / `move_forward` are the caller's own basis vectors — this crate has no
    /// // vector type of its own, and the axis is deliberately basis-free.
    /// # let (move_right, move_forward) = ([1.0_f32, 0.0, 0.0], [0.0_f32, 0.0, 1.0]);
    /// # let input = Input::new();
    /// let (mx, my) = input.move_axis();
    /// let direction: [f32; 3] =
    ///     std::array::from_fn(|i| move_right[i] * mx + move_forward[i] * my);
    /// # assert_eq!(direction, [0.0, 0.0, 0.0]); // nothing held
    /// ```
    #[must_use]
    pub fn move_axis(&self) -> (f32, f32) {
        self.move_axis_with(MoveKeys::WASD)
    }

    /// [`Input::move_axis`] with the movement keys spelled out — arrow keys, a second player's
    /// half of the keyboard, or whatever a config file said.
    #[must_use]
    pub fn move_axis_with(&self, keys: MoveKeys) -> (f32, f32) {
        let axis = |neg: u32, pos: u32| {
            f32::from(self.is_key_pressed(pos)) - f32::from(self.is_key_pressed(neg))
        };
        let stick = self
            .gamepad()
            .map(Gamepad::left_stick)
            .unwrap_or((0.0, 0.0));
        blend_move_axis(
            (axis(keys.left, keys.right), axis(keys.back, keys.forward)),
            stick,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{code_from_name, GamepadAxis, GamepadId};

    fn magnitude((x, y): (f32, f32)) -> f32 {
        (x * x + y * y).sqrt()
    }

    /// The defect four demos in this repository shipped: four independent `if`s that each add a
    /// full step, so the diagonal is √2 long.
    #[test]
    fn a_diagonal_key_push_is_not_faster_than_a_straight_one() {
        let straight = blend_move_axis((0.0, 1.0), (0.0, 0.0));
        let diagonal = blend_move_axis((1.0, 1.0), (0.0, 0.0));
        assert!((magnitude(straight) - 1.0).abs() < 1e-6, "{straight:?}");
        assert!(
            (magnitude(diagonal) - 1.0).abs() < 1e-6,
            "a diagonal moved at {} of full speed — the 41 % bug",
            magnitude(diagonal)
        );
        // …and it is a *diagonal*, not a snapped axis: both components survive, equally.
        assert!((diagonal.0 - diagonal.1).abs() < 1e-6, "{diagonal:?}");
    }

    /// The thing a key cannot express, and therefore the reason the stick is not simply treated
    /// as four more buttons.
    #[test]
    fn the_stick_carries_its_own_magnitude() {
        assert_eq!(blend_move_axis((0.0, 0.0), (0.0, 0.5)), (0.0, 0.5));
        assert_eq!(blend_move_axis((0.0, 0.0), (0.0, 1.0)), (0.0, 1.0));
        // A stick tilted diagonally at half is half a diagonal, not half of each axis clamped.
        let half_diagonal = (0.5_f32 / 2.0_f32.sqrt(), 0.5_f32 / 2.0_f32.sqrt());
        let out = blend_move_axis((0.0, 0.0), half_diagonal);
        assert!((magnitude(out) - 0.5).abs() < 1e-6, "{out:?}");
    }

    #[test]
    fn keys_and_stick_together_never_exceed_full_speed() {
        // Same direction, both at full: still 1.
        let both = blend_move_axis((0.0, 1.0), (0.0, 1.0));
        assert!((magnitude(both) - 1.0).abs() < 1e-6, "{both:?}");
        // Key forward, stick sideways at full: the corner of the square is not reachable.
        let corner = blend_move_axis((0.0, 1.0), (1.0, 0.0));
        assert!((magnitude(corner) - 1.0).abs() < 1e-6, "{corner:?}");
        // The worst case a per-axis clamp would let through: diagonal keys plus a diagonal stick.
        let worst = blend_move_axis((1.0, 1.0), (0.7, 0.7));
        assert!(magnitude(worst) <= 1.0 + 1e-6, "{worst:?}");
    }

    #[test]
    fn a_stick_pushed_against_a_held_key_cancels_it() {
        let out = blend_move_axis((0.0, 1.0), (0.0, -1.0));
        assert!(magnitude(out) < 1e-6, "{out:?}");
    }

    /// The discriminator for normalising the key direction, and the only test that notices it.
    ///
    /// Deleting that normalisation leaves every speed assertion in this file green, because the
    /// final clamp bounds the magnitude either way. What it breaks is *this*: an un-normalised
    /// W+D is √2 long, a stick is at most 1, and the player pushing back with everything the
    /// hardware has still drifts diagonally at 0.41 of full speed.
    #[test]
    fn a_full_stick_can_cancel_a_diagonal_key_push_too() {
        let opposing = -1.0 / 2.0_f32.sqrt();
        let out = blend_move_axis((1.0, 1.0), (opposing, opposing));
        assert!(
            magnitude(out) < 1e-6,
            "a full-tilt stick could not cancel W+D — it left {} of full speed",
            magnitude(out)
        );
    }

    #[test]
    fn no_input_is_exactly_zero() {
        // Exactly, not nearly: a caller that tests `!= 0.0` to decide whether the player is
        // moving must not see drift from a normalise that ran when it should not have.
        assert_eq!(blend_move_axis((0.0, 0.0), (0.0, 0.0)), (0.0, 0.0));
    }

    #[test]
    fn without_a_pad_the_result_is_the_key_direction_alone() {
        let mut input = Input::new();
        assert_eq!(input.move_axis(), (0.0, 0.0));

        input.on_key_pressed(MoveKeys::WASD.forward);
        assert_eq!(input.move_axis(), (0.0, 1.0));

        input.on_key_pressed(MoveKeys::WASD.right);
        let (x, y) = input.move_axis();
        assert!((magnitude((x, y)) - 1.0).abs() < 1e-6);
        assert!(x > 0.0 && y > 0.0, "({x}, {y})");

        // Opposite keys cancel rather than fighting over which `if` ran last.
        input.on_key_pressed(MoveKeys::WASD.back);
        input.on_key_pressed(MoveKeys::WASD.left);
        assert_eq!(input.move_axis(), (0.0, 0.0));
    }

    #[test]
    fn move_axis_reads_the_left_stick() {
        let mut input = Input::new();
        let id = GamepadId::new(0);
        input.on_gamepad_connected(id, "test pad");
        // Past the 0.15 radial deadzone, and not at full tilt: the walk case.
        input.on_gamepad_axis(id, GamepadAxis::LeftStickY, 0.6);
        let (_, y) = input.move_axis();
        assert!(y > 0.0 && y < 1.0, "a half-tilted stick should walk, got {y}");

        // A key on top of it reaches full speed and no further.
        input.on_key_pressed(MoveKeys::WASD.forward);
        let out = input.move_axis();
        assert!((magnitude(out) - 1.0).abs() < 1e-6, "{out:?}");
    }

    #[test]
    fn arrow_keys_work_the_same_way() {
        let mut input = Input::new();
        input.on_key_pressed(MoveKeys::ARROWS.left);
        assert_eq!(input.move_axis_with(MoveKeys::ARROWS), (-1.0, 0.0));
        // …and WASD does not answer for them.
        assert_eq!(input.move_axis(), (0.0, 0.0));
    }

    /// The 16 demos converted to [`Input::move_axis`] on 2026-08-17 had one of two shapes, and
    /// twelve of them had the *correct* one: accumulate a key direction, then `normalize_or_zero`.
    /// Replacing that with this function is only safe if the two agree on every key combination —
    /// so that is checked here rather than argued, over all 81 of them.
    ///
    /// (The other four had no normalisation at all. Those are the ones this changed on purpose.)
    #[test]
    fn the_keyboard_half_reproduces_what_the_demos_already_computed() {
        // What a demo used to write: sum the pressed keys, then normalise if non-zero.
        fn as_the_demos_had_it(kx: f32, ky: f32) -> (f32, f32) {
            let len = (kx * kx + ky * ky).sqrt();
            if len > 0.0 {
                (kx / len, ky / len)
            } else {
                (0.0, 0.0)
            }
        }
        for kx in [-1.0_f32, 0.0, 1.0] {
            for ky in [-1.0_f32, 0.0, 1.0] {
                let now = blend_move_axis((kx, ky), (0.0, 0.0));
                let before = as_the_demos_had_it(kx, ky);
                assert!(
                    (now.0 - before.0).abs() < 1e-6 && (now.1 - before.1).abs() < 1e-6,
                    "keys ({kx}, {ky}): was {before:?}, now {now:?} — the conversion was supposed \
                     to leave keyboard play untouched"
                );
            }
        }
    }

    /// The other half of that promise, for the demos whose movement has a vertical axis (Q/E,
    /// Space/Ctrl) that the stick has no counterpart for. Those kept their own vector and had
    /// their `normalize` turned into a **clamp** — because a normalise would push a half-tilted
    /// stick back up to full speed. For keys the two are the same thing, which is what makes the
    /// swap safe, and that is what this checks.
    #[test]
    fn clamping_instead_of_normalising_is_the_same_for_keys() {
        for v in [
            (1.0_f32, 0.0_f32, 0.0_f32),
            (1.0, 1.0, 0.0),
            (1.0, 1.0, 1.0),
            (0.0, 1.0, 1.0),
            (-1.0, 1.0, -1.0),
        ] {
            let len = (v.0 * v.0 + v.1 * v.1 + v.2 * v.2).sqrt();
            assert!(
                len >= 1.0,
                "any non-empty key combination is at least unit length — that is why a clamp is \
                 enough; {v:?} was {len}"
            );
        }
        // …and a stick's short vector is exactly what the clamp preserves and a normalise would
        // not: half a tilt must stay half.
        let half = blend_move_axis((0.0, 0.0), (0.0, 0.5));
        assert_eq!(half, (0.0, 0.5));
    }

    /// The presets are literals, and literals are how the Lua table came to hold HID codes. This
    /// ties them to the one table `gizmo-app` proves against winit, so there is still exactly one
    /// set of numbers anybody has to trust.
    #[test]
    fn the_presets_match_the_verified_key_table() {
        for (name, code) in [
            ("w", MoveKeys::WASD.forward),
            ("s", MoveKeys::WASD.back),
            ("a", MoveKeys::WASD.left),
            ("d", MoveKeys::WASD.right),
            ("up", MoveKeys::ARROWS.forward),
            ("down", MoveKeys::ARROWS.back),
            ("left", MoveKeys::ARROWS.left),
            ("right", MoveKeys::ARROWS.right),
        ] {
            assert_eq!(
                code_from_name(name),
                Some(code),
                "MoveKeys disagrees with NAMED_KEYS about `{name}`"
            );
        }
    }
}
