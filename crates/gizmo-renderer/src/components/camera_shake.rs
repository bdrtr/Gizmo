//! [`CameraShake`] — trauma that shakes a camera and wears off.

use gizmo_math::Vec3;

/// Shakes the [`Camera`](crate::components::Camera) it sits on: a hit, an explosion, a landing.
///
/// [`add`](Self::add) trauma (`0..=1`); the shake is trauma², so small knocks stay small and big
/// ones are violent, and trauma wears off at [`decay`](Self::decay) per second. The motion is a
/// smooth noise rather than a random jump each frame — a jump per frame reads as a glitch, not
/// as the ground moving — and it is applied to the **picture only**: the draw path moves the
/// camera for the frame it draws and puts it back afterwards, so a game's camera code, its aim
/// ray and its audio listener never see the shake, and a shake can neither drift the camera nor
/// fight the code that moves it.
///
/// ```
/// use gizmo_renderer::components::CameraShake;
///
/// let mut shake = CameraShake::default();
/// shake.add(0.6);
/// shake.advance(0.1);
/// assert!(shake.trauma() < 0.6, "it wears off");
/// let (offset, yaw, pitch) = shake.sample();
/// assert!(offset.length() > 0.0 || yaw != 0.0 || pitch != 0.0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CameraShake {
    trauma: f32,
    time: f32,
    /// Trauma lost per second (default `1.0`: a full shake is over in a second).
    pub decay: f32,
    /// Largest sideways/vertical offset at full trauma, in metres — or world units for a 2D
    /// camera (default `0.3`).
    pub max_offset: f32,
    /// Largest yaw/pitch at full trauma, in radians (default `0.05`, about 3°). Zero it for a
    /// 2D camera, whose look direction should not tilt.
    pub max_angle: f32,
    /// How fast it shakes: the noise's rate in radians per second (default `18`, a base wobble
    /// near 3 Hz with faster ones on top). Higher is buzzier.
    pub frequency: f32,
}

impl Default for CameraShake {
    fn default() -> Self {
        Self { trauma: 0.0, time: 0.0, decay: 1.0, max_offset: 0.3, max_angle: 0.05, frequency: 18.0 }
    }
}

impl CameraShake {
    /// Adds trauma, capped at `1.0`. Negative or NaN amounts are ignored.
    pub fn add(&mut self, amount: f32) {
        if amount > 0.0 {
            self.trauma = (self.trauma + amount).min(1.0);
        }
    }

    /// The trauma left, `0..=1`.
    #[must_use]
    pub fn trauma(&self) -> f32 {
        self.trauma
    }

    /// Stops the shake at once.
    pub fn clear(&mut self) {
        self.trauma = 0.0;
    }

    /// Advances the noise and wears the trauma down by `dt` seconds. The draw path calls this on
    /// the animation clock, so a paused game's shake pauses with it.
    pub fn advance(&mut self, dt: f32) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        self.time += dt;
        // Wrap well before f32 loses the fractional part the noise needs.
        if self.time > 10_000.0 {
            self.time -= 10_000.0;
        }
        self.trauma = (self.trauma - self.decay.max(0.0) * dt).max(0.0);
    }

    /// This instant's displacement: an offset in the camera's own right/up plane (`x` right, `y`
    /// up, `z` zero) and a yaw and pitch, all scaled by trauma². Zero once the trauma is gone.
    #[must_use]
    pub fn sample(&self) -> (Vec3, f32, f32) {
        let k = self.trauma * self.trauma;
        if k == 0.0 {
            return (Vec3::ZERO, 0.0, 0.0);
        }
        let t = self.time * self.frequency;
        let offset = Vec3::new(noise(t, 0), noise(t, 1), 0.0) * (self.max_offset * k);
        (offset, noise(t, 2) * self.max_angle * k, noise(t, 3) * self.max_angle * k)
    }
}

gizmo_core::impl_component!(CameraShake);

/// Smooth noise in `[-1, 1]`: a sum of three sines at unrelated rates, a different set per
/// `channel`. Not Perlin — but continuous, never periodic within a shake, and free of the
/// allocation and seeding a gradient noise needs.
fn noise(t: f32, channel: u32) -> f32 {
    let c = channel as f32;
    let a = (t * 1.000 + c * 1.7).sin();
    let b = (t * 2.131 + c * 4.3).sin();
    let d = (t * 3.737 + c * 2.9).sin();
    (a * 0.5 + b * 0.3 + d * 0.2).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trauma_is_capped_wears_off_and_the_shake_goes_with_it() {
        let mut s = CameraShake::default();
        s.add(0.7);
        s.add(0.7);
        assert_eq!(s.trauma(), 1.0, "capped");
        s.add(-5.0);
        s.add(f32::NAN);
        assert_eq!(s.trauma(), 1.0);
        s.advance(0.25);
        assert!((s.trauma() - 0.75).abs() < 1e-6);
        s.advance(10.0);
        assert_eq!(s.trauma(), 0.0);
        assert_eq!(s.sample(), (Vec3::ZERO, 0.0, 0.0), "no trauma, no shake");
    }

    /// trauma²: half the trauma is a quarter of the shake, never more than the configured maxima,
    /// and the motion is continuous — one frame to the next moves a little, not a jump.
    #[test]
    fn the_shake_is_trauma_squared_bounded_and_smooth() {
        let mut full = CameraShake::default();
        let mut half = CameraShake::default();
        full.add(1.0);
        half.add(0.5);
        full.decay = 0.0;
        half.decay = 0.0;
        let mut peak_full: f32 = 0.0;
        let mut peak_half: f32 = 0.0;
        let mut last = full.sample().0;
        for _ in 0..600 {
            full.advance(1.0 / 60.0);
            half.advance(1.0 / 60.0);
            let (o, yaw, pitch) = full.sample();
            assert!(o.x.abs() <= 0.3 + 1e-6 && o.y.abs() <= 0.3 + 1e-6 && o.z == 0.0);
            assert!(yaw.abs() <= 0.05 + 1e-6 && pitch.abs() <= 0.05 + 1e-6);
            assert!((o - last).length() < 0.3, "a jump of {} in one frame", (o - last).length());
            last = o;
            peak_full = peak_full.max(o.length());
            peak_half = peak_half.max(half.sample().0.length());
        }
        assert!(peak_full > 0.15, "it does shake: peak {peak_full}");
        assert!((peak_half / peak_full - 0.25).abs() < 1e-3, "half trauma, quarter shake: {}", peak_half / peak_full);
    }
}
