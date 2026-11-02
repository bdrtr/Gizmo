//! [`Looping`] — a sound that plays again from the start when it ends, and can be seeked.
//!
//! rodio's `repeat_infinite` buffers its input, and its buffer does not seek, so a looped sound —
//! a level's music, an engine's hum — could not be moved at all: `AudioManager::seek`'s hardware
//! test found it. This loops by decoding the same bytes again (they are behind an `Arc`, so that
//! is a cursor and a header read, not a copy), and passes a seek to the decoder playing now, wrapped
//! into the sound's length.

use rodio::source::SeekError;
use rodio::{ChannelCount, Decoder, Sample, SampleRate, Source};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

pub(crate) type Bytes = Cursor<Arc<[u8]>>;

/// Decodes `bytes` **seekably**. `Decoder::new` on a cursor does not know the length, so symphonia
/// refuses random access (`RandomAccessNotSupported`) and every seek fails — and rodio's
/// `Player::try_seek` can report that as `Ok`, so nothing said so. Every decoder the engine makes
/// goes through here.
pub(crate) fn decode(bytes: &Arc<[u8]>) -> Result<Decoder<Bytes>, rodio::decoder::DecoderError> {
    Decoder::builder()
        .with_data(Cursor::new(Arc::clone(bytes)))
        .with_byte_len(bytes.len() as u64)
        .with_seekable(true)
        .build()
}

/// Plays `bytes` over and over. Ends only if the bytes stop decoding.
pub(crate) struct Looping {
    bytes: Arc<[u8]>,
    current: Decoder<Bytes>,
    /// The sound's length, if its format states it — what a seek past the end wraps by.
    length: Option<Duration>,
}

impl Looping {
    /// Loops `bytes`, starting with `first` (already decoded from them, so a bad file is reported
    /// by the caller before anything plays).
    pub(crate) fn new(bytes: Arc<[u8]>, first: Decoder<Bytes>) -> Self {
        let length = first.total_duration();
        Self { bytes, current: first, length }
    }
}

impl Iterator for Looping {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if let Some(s) = self.current.next() {
            return Some(s);
        }
        // The end: decode again from the top. A sound with no samples at all would spin here, so
        // one failed restart ends the loop.
        self.current = decode(&self.bytes).ok()?;
        self.current.next()
    }
}

impl Source for Looping {
    fn current_span_len(&self) -> Option<usize> {
        self.current.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.current.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.current.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let within = match self.length {
            Some(len) if !len.is_zero() => Duration::from_secs_f64(pos.as_secs_f64() % len.as_secs_f64()),
            _ => pos,
        };
        self.current.try_seek(within)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::wav_from_samples;

    fn ramp(n: usize) -> Arc<[u8]> {
        let samples: Vec<f32> = (0..n).map(|i| i as f32 / n as f32 * 0.9).collect();
        wav_from_samples(&samples, 1000).into()
    }

    fn looping(bytes: &Arc<[u8]>) -> Looping {
        Looping::new(Arc::clone(bytes), decode(bytes).unwrap())
    }

    /// It plays the sound, then the sound again from the top, without end.
    #[test]
    fn it_starts_again_at_the_end() {
        let bytes = ramp(100);
        let mut l = looping(&bytes);
        let first: Vec<f32> = (&mut l).take(100).collect();
        let second: Vec<f32> = (&mut l).take(100).collect();
        assert_eq!(first.len(), 100);
        assert_eq!(first, second, "the second pass is not the first");
        assert!(l.next().is_some());
        assert_eq!(l.total_duration(), None, "a loop has no length");
    }

    /// A seek lands where it says — and past the end, wrapped into the sound.
    #[test]
    fn it_seeks_and_wraps_a_seek_past_the_end() {
        let bytes = ramp(1000); // one second at 1 kHz
        let mut reference = looping(&bytes);
        let at_250: f32 = reference.nth(250).unwrap();
        let mut l = looping(&bytes);
        l.try_seek(Duration::from_millis(250)).expect("a WAV seeks");
        assert!((l.next().unwrap() - at_250).abs() < 1e-3);
        l.try_seek(Duration::from_millis(2250)).expect("past the end wraps");
        assert!((l.next().unwrap() - at_250).abs() < 1e-3, "2.25 s into a 1 s loop is 0.25 s");
    }
}
