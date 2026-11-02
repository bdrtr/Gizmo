//! Sound made from numbers: a PCM → WAV encoder and [`Blip`], a small effect generator.
//!
//! The engine could only play sound that arrived as a file — [`AudioManager::load_sound`] reads
//! one, [`AudioManager::load_sound_bytes`] takes one already in memory. A game with no audio
//! assets (a jam game, a prototype, a demo in this repository) therefore had no sound at all;
//! `kristal_avcisi` was written silent for exactly that reason. The repository's only WAV encoder
//! was a test helper.
//!
//! Generated sound goes through the **same** path as a file: [`wav_from_samples`] encodes it as
//! a 16-bit mono PCM WAV, and [`AudioManager::load_samples`] registers that like any other
//! buffer. There is no second playback path to keep in step with the first — the decoder that
//! plays a `.wav` from disk plays these.
//!
//! [`AudioManager::load_sound`]: crate::AudioManager::load_sound
//! [`AudioManager::load_sound_bytes`]: crate::AudioManager::load_sound_bytes
//! [`AudioManager::load_samples`]: crate::AudioManager::load_samples

/// The sample rate [`Blip::wav`] renders at.
pub const DEFAULT_SAMPLE_RATE: u32 = 44_100;

/// Encodes mono `samples` in `[-1, 1]` as a complete 16-bit PCM WAV file at `sample_rate`.
///
/// Out-of-range samples are clamped rather than wrapped (a wrapped overshoot is a full-scale
/// click), and a non-finite sample is written as silence.
///
/// ```
/// let wav = gizmo_audio::synth::wav_from_samples(&[0.0, 0.5, -0.5], 8_000);
/// assert_eq!(&wav[..4], b"RIFF");
/// assert_eq!(wav.len(), 44 + 3 * 2);
/// ```
#[must_use]
pub fn wav_from_samples(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    v.extend_from_slice(&2u16.to_le_bytes()); // block align
    v.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let s = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
        v.extend_from_slice(&((s * i16::MAX as f32).round() as i16).to_le_bytes());
    }
    v
}

/// The oscillator a [`Blip`] uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Wave {
    /// Pure and soft.
    Sine,
    /// Hollow and loud — the classic pickup and jump.
    Square,
    /// Between the two.
    Triangle,
    /// Bright and buzzy.
    Saw,
    /// White noise, for hits and explosions. Its pitch is how often a new random value is drawn,
    /// so a falling noise blip is a thud and a high one a hiss. Seeded: the same blip renders the
    /// same samples every time.
    Noise,
}

/// A short synthesised sound effect — one oscillator, a pitch slide and an envelope.
///
/// Enough for the sounds a small game makes: a rising square is a pickup, a falling one a hit, a
/// falling noise an explosion, a short high sine a UI tick.
///
/// ```
/// use gizmo_audio::synth::{Blip, Wave};
///
/// let pickup = Blip::new(Wave::Square, 660.0, 0.12).with_slide(1320.0).with_volume(0.4);
/// let samples = pickup.samples(8_000);
/// assert_eq!(samples.len(), 960); // 0.12 s at 8 kHz
/// assert!(samples.iter().all(|s| s.abs() <= 0.4 + 1e-6));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Blip {
    /// The oscillator.
    pub wave: Wave,
    /// Pitch at the start, in Hz.
    pub start_hz: f32,
    /// Pitch at the end, in Hz; the slide between is exponential, so it sounds even. Equal to
    /// `start_hz` for no slide.
    pub end_hz: f32,
    /// Length in seconds.
    pub seconds: f32,
    /// Fade-in time in seconds — a few milliseconds removes the click of a hard start. The rest
    /// of the sound decays linearly to silence, so it never ends on a click either.
    pub attack: f32,
    /// Peak amplitude, `0..=1`.
    pub volume: f32,
}

impl Blip {
    /// A `seconds`-long `wave` at `hz`, no slide, at half volume with a 5 ms attack.
    #[must_use]
    pub fn new(wave: Wave, hz: f32, seconds: f32) -> Self {
        Self { wave, start_hz: hz, end_hz: hz, seconds, attack: 0.005, volume: 0.5 }
    }

    /// Slides the pitch to `end_hz` over the sound.
    #[must_use]
    pub fn with_slide(mut self, end_hz: f32) -> Self {
        self.end_hz = end_hz;
        self
    }

    /// Sets the peak amplitude, clamped into `0..=1`.
    #[must_use]
    pub fn with_volume(mut self, volume: f32) -> Self {
        self.volume = volume.clamp(0.0, 1.0);
        self
    }

    /// Sets the fade-in time, in seconds.
    #[must_use]
    pub fn with_attack(mut self, attack: f32) -> Self {
        self.attack = attack.max(0.0);
        self
    }

    /// Renders the sound at `sample_rate`: `seconds × sample_rate` samples in
    /// `[-volume, volume]`.
    #[must_use]
    pub fn samples(&self, sample_rate: u32) -> Vec<f32> {
        let rate = sample_rate.max(1) as f32;
        let n = (self.seconds.max(0.0) * rate).round() as usize;
        let start = self.start_hz.max(1.0);
        let end = self.end_hz.max(1.0);
        let attack = (self.attack * rate).max(1.0);
        let mut phase = 0.0f32;
        let mut noise_state: u32 = 0x9E37_79B9;
        let mut noise_value = 0.0f32;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / n as f32;
            // Exponential glide: equal ratios in equal times, which is how pitch is heard.
            let hz = start * (end / start).powf(t);
            let before = phase;
            phase = (phase + hz / rate).fract();
            let raw = match self.wave {
                Wave::Sine => (before * std::f32::consts::TAU).sin(),
                Wave::Square => {
                    if before < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                Wave::Triangle => 1.0 - 4.0 * (before - 0.5).abs(),
                Wave::Saw => 2.0 * before - 1.0,
                Wave::Noise => {
                    // A new value each time the phase wraps: the pitch is the redraw rate.
                    if phase < before || i == 0 {
                        noise_state ^= noise_state << 13;
                        noise_state ^= noise_state >> 17;
                        noise_state ^= noise_state << 5;
                        noise_value = noise_state as f32 / u32::MAX as f32 * 2.0 - 1.0;
                    }
                    noise_value
                }
            };
            let rise = ((i as f32 + 1.0) / attack).min(1.0);
            let fall = 1.0 - t;
            out.push(raw * self.volume * rise * fall);
        }
        out
    }

    /// The sound as a WAV file at [`DEFAULT_SAMPLE_RATE`], ready for
    /// [`AudioManager::load_sound_bytes`](crate::AudioManager::load_sound_bytes).
    #[must_use]
    pub fn wav(&self) -> Vec<u8> {
        wav_from_samples(&self.samples(DEFAULT_SAMPLE_RATE), DEFAULT_SAMPLE_RATE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::{Decoder, Source};
    use std::io::Cursor;

    /// What rodio decodes back is what was encoded: the rate, one channel, the length and the
    /// values (to 16-bit precision). This is the property that matters — a header the decoder
    /// rejects would be silence at runtime, not an error at build time.
    #[test]
    fn the_decoder_reads_back_what_was_encoded() {
        let samples: Vec<f32> = (0..400).map(|i| ((i as f32) * 0.05).sin() * 0.8).collect();
        let wav = wav_from_samples(&samples, 11_025);
        let decoder = Decoder::new(Cursor::new(wav)).expect("rodio must accept the encoded WAV");
        assert_eq!(decoder.sample_rate().get(), 11_025);
        assert_eq!(decoder.channels().get(), 1);
        let back: Vec<f32> = decoder.collect();
        assert_eq!(back.len(), samples.len());
        for (a, b) in samples.iter().zip(&back) {
            assert!((a - b).abs() < 1e-3, "{a} came back as {b}");
        }
    }

    #[test]
    fn out_of_range_and_non_finite_samples_are_clamped_not_wrapped() {
        let wav = wav_from_samples(&[2.0, -3.0, f32::NAN, f32::INFINITY], 8_000);
        let read = |i: usize| i16::from_le_bytes([wav[44 + 2 * i], wav[45 + 2 * i]]);
        assert_eq!(read(0), i16::MAX);
        assert_eq!(read(1), -i16::MAX);
        assert_eq!(read(2), 0, "NaN must be silence");
        assert_eq!(read(3), 0, "infinity must be silence");
    }

    /// A blip is as long as asked, starts and ends at silence (no click either side), and its
    /// peak is the volume.
    #[test]
    fn a_blip_has_its_length_its_peak_and_no_clicks() {
        for wave in [Wave::Sine, Wave::Square, Wave::Triangle, Wave::Saw, Wave::Noise] {
            let s = Blip::new(wave, 440.0, 0.25).with_volume(0.6).samples(8_000);
            assert_eq!(s.len(), 2_000, "{wave:?}");
            assert!(s[0].abs() < 0.6 * 0.05, "{wave:?} starts with a click: {}", s[0]);
            assert!(s[s.len() - 1].abs() < 0.6 * 0.01, "{wave:?} ends with a click: {}", s[s.len() - 1]);
            let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak <= 0.6 + 1e-6 && peak > 0.3, "{wave:?} peak {peak}");
        }
    }

    /// The slide is heard: a sweep from 200 to 1600 Hz crosses zero far more often at the end
    /// than at the start, in the ratio of the pitches.
    #[test]
    fn a_slide_raises_the_pitch_across_the_sound() {
        let s = Blip::new(Wave::Sine, 200.0, 1.0).with_slide(1600.0).with_attack(0.0).samples(32_000);
        let crossings = |w: &[f32]| w.windows(2).filter(|p| (p[0] < 0.0) != (p[1] < 0.0)).count();
        let tenth = s.len() / 10;
        let early = crossings(&s[..tenth]) as f32;
        let late = crossings(&s[s.len() - tenth..]) as f32;
        // Over the first tenth the pitch runs 200 → ~246 Hz, over the last ~1300 → 1600 Hz:
        // about six times as many crossings.
        assert!(late / early > 4.5, "early {early}, late {late}");
        let flat = Blip::new(Wave::Sine, 200.0, 1.0).with_attack(0.0).samples(32_000);
        let (fe, fl) = (crossings(&flat[..tenth]) as f32, crossings(&flat[flat.len() - tenth..]) as f32);
        assert!((fe - fl).abs() <= 2.0, "no slide, yet {fe} vs {fl} crossings");
    }

    /// Noise is reproducible, and actually noisy — not stuck, not periodic at the redraw rate.
    #[test]
    fn noise_is_seeded_and_varied() {
        let a = Blip::new(Wave::Noise, 4_000.0, 0.1).samples(8_000);
        let b = Blip::new(Wave::Noise, 4_000.0, 0.1).samples(8_000);
        assert_eq!(a, b, "the same blip rendered different noise");
        // Sign changes, not distinct values: the envelope alone makes every sample distinct, so a
        // generator stuck on one value would pass a distinctness check. Redrawn every other
        // sample, about a quarter of neighbouring pairs should change sign.
        let flips = a.windows(2).filter(|p| (p[0] < 0.0) != (p[1] < 0.0)).count();
        assert!(flips > a.len() / 8, "only {flips} sign changes in {} samples", a.len());
    }

    #[test]
    fn the_wav_is_at_the_default_rate() {
        let wav = Blip::new(Wave::Square, 880.0, 0.1).wav();
        let decoder = Decoder::new(Cursor::new(wav)).expect("decodes");
        assert_eq!(decoder.sample_rate().get(), DEFAULT_SAMPLE_RATE);
        assert_eq!(decoder.count(), (0.1 * DEFAULT_SAMPLE_RATE as f32).round() as usize);
    }
}
