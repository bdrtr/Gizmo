#![deny(clippy::undocumented_unsafe_blocks)]
#![warn(missing_docs)]
//! (`missing_docs` is a RATCHET, like everywhere else in this workspace, and this crate was the
//! last one to get the line — added 2026-08-18. It was already at zero, which is exactly why it
//! went unnoticed: a crate that happens to be documented and a crate that cannot stop being
//! documented look identical until someone adds a `pub fn`.)
//!
//! (`undocumented_unsafe_blocks` is a RATCHET: this crate carries no `unsafe` block without a
//! `// SAFETY:` line stating why it is sound, and the lint keeps it that way. Every crate in the
//! workspace except `gizmo-core` is at zero and denies it; `gizmo-core`'s ECS internals are the
//! measured remainder — see docs/ENGINE.md.)
//! `gizmo-audio` is the audio subsystem of the Gizmo engine.
//!
//! It is a thin, [`rodio`]-backed layer that exposes a small public surface:
//!
//! - [`AudioSource`] — an ECS component describing a 2D or 3D playable sound.
//! - [`AudioManager`] — a resource that loads sounds into memory and plays,
//!   updates and stops both global (stereo) and 3D spatial sinks.
//! - [`Mixer`] — the buses (music/sfx/ui/voice), the master gain and the environment modifier.
//!   **Every volume and speed that reaches rodio is composed there**, from what the game asked for
//!   times the gains that apply; nothing multiplies into a sink and nothing has to be undone. See
//!   the module's own docs for the measurement that shape came from.
//! - [`AudioError`] — the error type returned when loading or playing sounds fails.
//! - [`synth`] — sound made from numbers: a PCM → WAV encoder and [`synth::Blip`], a one-line
//!   effect generator, for a game with no audio files. Loaded with
//!   [`AudioManager::load_samples`] or, for a blip, `load_sound_bytes(name, blip.wav())`.
//!
//! Sounds are decoded from in-memory byte buffers (loaded once via
//! [`AudioManager::load_sound`]) to avoid per-play disk I/O. Spatial playback
//! tracks emitter and listener (ear) positions and attenuates volume by
//! distance. No `rodio` types appear in the public API, keeping the dependency
//! contract internal.

mod filter;
/// The master compressor — [`Compressor`](compressor::Compressor), on the summed output between the
/// equalizer and the limiter.
pub mod compressor;
/// The master equalizer — [`Equalizer`](equalizer::Equalizer), the first stage on the summed output.
pub mod equalizer;
/// The master limiter — [`Limiter`](limiter::Limiter), on the summed output.
pub mod limiter;
mod bus;
mod looping;
mod mixer;
mod playing;
/// Reverb — [`Reverb`](reverb::Reverb) and the stage every sound is wrapped in.
pub mod reverb;
pub use mixer::{Mixer, SendError};
pub mod synth;

use filter::{Muffle, MuffleControl};
use looping::{decode, Looping};

use rodio::stream::DeviceSinkBuilder;
use rodio::{MixerDeviceSink, Player};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ======================== ERRORS ========================

/// Errors that can occur while loading or playing a sound with the
/// [`AudioManager`].
#[derive(Debug)]
#[non_exhaustive]
pub enum AudioError {
    /// An I/O error occurred while reading the sound file.
    Io(std::io::Error),
    /// The requested sound file could not be found at the given path.
    NotFound(String),
    /// No usable audio output device/backend could be opened.
    Backend(String),
    /// A playback was requested for a sound name that has not been loaded
    /// into memory via [`AudioManager::load_sound`].
    NotLoaded(String),
    /// The in-memory sound bytes could not be decoded into a playable stream.
    Decode(String),
    /// A playing sound could not be moved to the asked position (its decoder cannot seek, or
    /// there is no such sound).
    Seek(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::Io(err) => write!(f, "IO Error: {}", err),
            AudioError::NotFound(path) => write!(f, "File not found: {}", path),
            AudioError::Backend(msg) => write!(f, "Audio backend error: {}", msg),
            AudioError::Seek(msg) => write!(f, "Seek failed: {}", msg),
            AudioError::NotLoaded(name) => {
                write!(f, "Sound '{}' is not loaded into memory", name)
            }
            AudioError::Decode(msg) => write!(f, "Failed to decode sound: {}", msg),
        }
    }
}

impl std::error::Error for AudioError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AudioError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for AudioError {
    fn from(err: std::io::Error) -> Self {
        AudioError::Io(err)
    }
}

// ======================== ECS COMPONENT ========================

/// ECS component for a sound source that can be played in 2D or 3D.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct AudioSource {
    /// Name of the loaded sound to play (see [`AudioManager::load_sound`]).
    pub sound_name: String,
    /// Whether the sound should be played as a 3D spatial source.
    pub is_3d: bool,
    /// Playback volume multiplier (1.0 = original volume).
    pub volume: f32,
    /// Playback pitch/speed multiplier (1.0 = original pitch).
    pub pitch: f32,
    /// Whether the sound should loop indefinitely.
    pub loop_sound: bool,
    /// Distance at which the sound is fully attenuated (silent), in metres.
    ///
    /// The falloff between here and there is linear, and it is the curve a listener actually
    /// hears — see [`spatial_gain`], which had to cancel rodio's own inverse-square term to make
    /// that true. Before it did, a source set to 100 m was under 1 % of its volume by 10 m, and
    /// raising this number barely moved that.
    pub max_distance: f32,
    /// The mixer bus this sound plays on — the player's SFX slider, music slider, and so on.
    /// See [`Mixer`]. Defaults to [`Mixer::DEFAULT_BUS`]; `#[serde(default)]` so a scene saved
    /// before buses existed still loads, onto that same bus.
    #[serde(default = "default_bus")]
    pub bus: String,
    /// Internal id of the active sink playing this source, if any.
    ///
    /// **Not persisted** — a sink id names a live sound on a device this process opened, and a
    /// scene file outlives both. Saving a scene while the game is playing used to write the id
    /// into it; reloading that file produced a source that believed it was already playing, and
    /// for a **looping** one that is permanent: the spatial system only clears a stale id for a
    /// one-shot, so the sound never started again. `has_played` next to it is skipped for the
    /// same reason.
    #[serde(skip)]
    pub _internal_sink_id: Option<u64>,
    /// Latches once this source has been auto-started, so a finished **one-shot** is not
    /// restarted every frame. (When a one-shot ends the spatial system clears
    /// `_internal_sink_id`; without this sentinel the auto-start guard would fire again
    /// next frame → infinite repeat.) Transient runtime state — not persisted.
    #[serde(skip)]
    pub has_played: bool,
}

impl Default for AudioSource {
    fn default() -> Self {
        Self::new("default")
    }
}

/// serde's fallback for [`AudioSource::bus`] — see that field.
fn default_bus() -> String {
    Mixer::DEFAULT_BUS.to_string()
}

/// The distance at which a 3D sound has faded out, in metres, when nothing says otherwise: an
/// [`AudioSource`]'s default, and the reach of a one-shot [`AudioManager::play_3d`].
pub const DEFAULT_MAX_DISTANCE: f32 = 100.0;

impl AudioSource {
    /// Creates a new [`AudioSource`] for the sound with the given name.
    pub fn new(name: &str) -> Self {
        Self {
            sound_name: name.to_string(),
            is_3d: true,
            volume: 1.0,
            pitch: 1.0,
            loop_sound: false,
            max_distance: DEFAULT_MAX_DISTANCE,
            bus: default_bus(),
            _internal_sink_id: None,
            has_played: false,
        }
    }

    /// Sets whether the sound loops, returning the modified source.
    pub fn with_loop(mut self, l: bool) -> Self {
        self.loop_sound = l;
        self
    }

    /// Sets the attenuation distance, returning the modified source.
    pub fn with_max_distance(mut self, dist: f32) -> Self {
        self.max_distance = dist;
        self
    }

    /// Routes this sound to a mixer bus, returning the modified source. See [`Mixer`].
    pub fn with_bus(mut self, bus: &str) -> Self {
        self.bus = bus.to_string();
        self
    }
}

/// How [`AudioManager::play_with`] and [`AudioManager::play_3d_with`] start a sound: on which
/// mixer bus, whether it loops, and at what volume and pitch. All of it holds from the sound's
/// first sample, which setting it after `play` cannot promise: the sound is on the device by then.
///
/// ```
/// # use gizmo_audio::{Mixer, PlayOptions};
/// let theme = PlayOptions::on_bus(Mixer::MUSIC).looped(true).volume(0.6);
/// assert_eq!((theme.bus.as_str(), theme.looped, theme.volume), (Mixer::MUSIC, true, 0.6));
/// assert_eq!(PlayOptions::default().bus, Mixer::DEFAULT_BUS);
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PlayOptions {
    /// The bus the sound plays on. See [`Mixer`].
    pub bus: String,
    /// Whether it plays again from the start when it ends.
    pub looped: bool,
    /// The volume it is asked to play at, as [`AudioManager::set_volume`] takes it; for a 3D
    /// sound, at the emitter, before the distance takes its share.
    pub volume: f32,
    /// The pitch, as [`AudioManager::set_pitch`] takes it.
    pub pitch: f32,
    /// For a 3D sound, the distance at which it has faded out, in metres. See
    /// [`AudioSource::max_distance`].
    pub max_distance: f32,
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self { bus: default_bus(), looped: false, volume: 1.0, pitch: 1.0, max_distance: DEFAULT_MAX_DISTANCE }
    }
}

impl PlayOptions {
    /// A one-shot on `bus`, at unity.
    pub fn on_bus(bus: &str) -> Self {
        Self { bus: bus.to_string(), ..Self::default() }
    }

    /// Sets whether it loops, returning the options.
    pub fn looped(mut self, looped: bool) -> Self {
        self.looped = looped;
        self
    }

    /// Sets the volume, returning the options.
    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    /// Sets the pitch, returning the options.
    pub fn pitch(mut self, pitch: f32) -> Self {
        self.pitch = pitch;
        self
    }

    /// Sets a 3D sound's reach, returning the options.
    pub fn max_distance(mut self, max_distance: f32) -> Self {
        self.max_distance = max_distance;
        self
    }
}

/// What one playing sound reads from its environment on the audio thread, its own and not shared:
/// the low-pass corner of its muffle and whether the reverb reaches it. Per sound, because a sound
/// on a dry bus has neither (`Mixer::set_bus_dry`); the reverb's shape stays shared.
#[derive(Debug)]
struct SinkEnvironment {
    muffle: Arc<MuffleControl>,
    reverb_send: Arc<AtomicBool>,
}

// ======================== AUDIO MANAGER ========================

/// Resource that owns the audio output device and manages loaded sounds and
/// active playback sinks (both global and 3D spatial).
pub struct AudioManager {
    // The device sink owns the live `cpal::Stream` — dropping it stops all audio — and hands out
    // the mixer that every player connects to. (rodio 0.22 merged the old `OutputStream` +
    // `OutputStreamHandle` pair into this one value; "sink" in *our* vocabulary still means one
    // playing sound, which rodio now calls a `Player`.)
    // Held, never read since the bus below took over: dropping it stops the stream. `None` for a
    // manager on an output of its own (the tests, which pull the mix themselves).
    _device_sink: Option<MixerDeviceSink>,
    // What every player connects to: a mixer of our own, whose sum goes through the master
    // limiter into the device's mixer. The device's mixer cannot be wrapped (it is the stream's
    // own), so the limiter needs a sum it owns to sit on.
    bus: rodio::mixer::Mixer,
    equalizer: Arc<equalizer::EqualizerControl>,
    compressor: Arc<compressor::CompressorControl>,
    limiter: Arc<limiter::LimiterControl>,

    // RAM'e (Memory) yüklenmiş ses dosyaları (Disk I/O darboğazını önler)
    sound_buffers: HashMap<String, Arc<[u8]>>,

    // Aktif uzamsal/normal çalıcıları takip edip parametrelerini güncellemek için
    active_spatial_sinks: HashMap<u64, playing::SpatialSink>,
    active_sinks: HashMap<u64, Player>,
    // Which loaded sound each live sink was started from. A sink id is what the engine holds, but
    // a NAME is what a script has: `audio.stop("music")` cannot mean anything else.
    sink_sounds: HashMap<u64, String>,
    next_sink_id: u64,
    // The last value [`AudioManager::set_all_paused`] was asked for, so a host can call it every
    // frame and only a CHANGE reaches the sinks — otherwise a game pausing one sound of its own
    // would have it resumed on the next frame by a host that is not paused.
    all_paused: bool,

    // Each live sound's own low-pass corner and reverb switch, read on the audio thread by the
    // stages inside its source — the only place rodio lets a filter be live. Per sound because a
    // dry bus has neither; written by `apply_mix` from the mixer like the volume.
    sink_env: HashMap<u64, SinkEnvironment>,
    // The reverb's shape every source reads: one store turns a cave on for every sound.
    reverb: std::sync::Arc<reverb::ReverbControl>,

    // The format every bus node and the master mix at.
    format: (rodio::ChannelCount, rodio::SampleRate),
    // Each bus's node: the mixer its sounds are connected to, its effect, the sends it feeds
    // (`bus`). Made the first time a sound starts on a bus or a send names it.
    nodes: HashMap<String, bus::BusNode>,
    // The live sends, `(from, to)`: what `Mixer::sends` says, built (`sync_routing`).
    links: std::collections::BTreeMap<(String, String), Arc<bus::SendLink>>,

    // Buses, master, environment, and one route per live sink. THE ONLY PLACE a volume or a
    // speed is decided: everything below writes `mixer.volume_for(id)` / `mixer.speed_for(id)`
    // and nothing computes a level of its own. That is what stops two writers from fighting over
    // one field, which is what the underwater muffle and the spatial system used to do.
    mixer: Mixer,
}

// SAFETY: wasm32'de (atomics/paylaşımlı-bellek OLMADAN) yürütme tek thread'dir —
// bir değer başka bir thread'e fiilen taşınamayacağı için bu impl'ler
// gözlemlenemez; cpal'ın WebAudio tipleri yalnızca ham JS handle'ları taşıdığı
// için !Send'dir. wgpu'nun `fragile-send-sync-non-atomic-wasm` deseninin
// birebir karşılığı. `not(target_feature = "atomics")` koşulu bilinçli: wasm
// threads etkinleştirilirse impl kaybolur ve World-resource kullanımı derleme
// hatasıyla yeniden değerlendirmeye zorlar (sessiz unsoundness yerine).
#[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
unsafe impl Send for AudioManager {}
// SAFETY: aynı gerekçe — tek thread'li wasm'da paylaşımlı erişim gözlemlenemez, ve `atomics`
// açılırsa bu impl de kaybolur. (Bu satır clippy için ayrıca yazıldı: gerekçe yukarıda tek blok
// hâlindeydi, lint her `unsafe impl`'in kendi başına belgelenmesini istiyor — ve haklı, çünkü
// biri silinip öteki kalabilir.)
#[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
unsafe impl Sync for AudioManager {}

impl std::fmt::Debug for AudioManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioManager")
            .field("loaded_sounds", &self.sound_buffers.len())
            .field("active_spatial_sinks", &self.active_spatial_sinks.len())
            .field("active_sinks", &self.active_sinks.len())
            .field("next_sink_id", &self.next_sink_id)
            .field("mixer", &self.mixer)
            .finish_non_exhaustive()
    }
}

/// The volume to write for a spatial sink at `distance`, so that what a listener hears follows
/// [`AudioSource::max_distance`] rather than rodio's own curve.
///
/// # Why this is not just `1 - d/max`
///
/// **rodio attenuates by distance underneath us, and it is an inverse-SQUARE law.** Its
/// `Spatial` source sets each ear's gain to `min(1/dist², 1)` (rodio 0.22.2,
/// `source/spatial.rs`), on top of whatever volume the sink is given. So the engine's linear
/// taper was never the curve anyone heard — it was multiplied by a much steeper one, and the
/// field documented as "the distance at which the sound is silent" was a cut-off far beyond the
/// point where the sound had already gone. Measured, for the default `max_distance = 100`:
///
/// | distance | taper | rodio | heard |
/// |---|---|---|---|
/// | 1 m | 0.99 | 1.0 | 99 % |
/// | 3 m | 0.97 | 0.111 | 10.8 % |
/// | 10 m | 0.90 | 0.010 | 0.9 % |
/// | 30 m | 0.70 | 0.0011 | 0.08 % |
///
/// A sound authored to carry 100 m was inaudible past about ten, and turning `max_distance` up
/// barely moved that — the term that was actually shaping the curve did not contain it.
///
/// So this **cancels** rodio's distance term (`d²`, using the listener-centre distance, and never
/// below 1 m where rodio itself clamps) and leaves the engine's taper as the curve. Cancelling is
/// safe by construction: the factor undoes an attenuation rodio is about to apply, so what reaches
/// the device is at most `base_volume`, never more — this cannot make anything louder than an
/// unspatialised sound of the same volume. What survives from rodio is the part that is its job:
/// the left/right *difference*, i.e. the panning, which scales both ears equally here and so keeps
/// its ratio.
pub fn spatial_gain(distance: f32, max_distance: f32, base_volume: f32) -> f32 {
    let taper = if max_distance > 0.0 {
        (1.0 - distance / max_distance).max(0.0)
    } else {
        1.0
    };
    // Below one metre rodio's own modifier is clamped to 1, so there is nothing to cancel.
    let rodio_distance_term = distance.max(1.0).powi(2);
    taper * base_volume * rodio_distance_term
}

/// The distance from the point between two ears to an emitter.
fn ear_distance(emitter: [f32; 3], left_ear: [f32; 3], right_ear: [f32; 3]) -> f32 {
    let centre = [0, 1, 2].map(|i| (left_ear[i] + right_ear[i]) / 2.0);
    (0..3).map(|i| (emitter[i] - centre[i]).powi(2)).sum::<f32>().sqrt()
}

/// A sound's source with its stages around it: the reverb, then the muffle, each reading this
/// sound's own switch.
fn wrap<I: rodio::Source>(input: I, reverb: &Arc<reverb::ReverbControl>, env: &SinkEnvironment) -> Muffle<reverb::ReverbStage<I>> {
    Muffle::new(reverb::ReverbStage::new(input, Arc::clone(reverb), Arc::clone(&env.reverb_send)), Arc::clone(&env.muffle))
}

/// The length a sound's bytes decode to, if the format states it.
fn decoded_duration(bytes: &Arc<[u8]>) -> Option<f32> {
    use rodio::Source;
    let decoder = decode(bytes).ok()?;
    decoder.total_duration().map(|d| d.as_secs_f32())
}

/// Clamp a playback-speed/pitch factor to a value that is safe for rodio's `Speed`
/// filter. A factor of `0.0` (or negative, or NaN) makes rodio compute a source
/// sample-rate of `(orig_rate * factor) as u32 == 0`, which trips a `from >= 1`
/// assert inside `SampleRateConverter::new` and PANICS on the cpal audio callback
/// thread, killing playback. `pitch = 0` is reachable from a scene-authored /
/// serde-deserialized `AudioSource.pitch` and from the near-field 3D-audio path.
pub(crate) fn sanitize_playback_speed(pitch: f32) -> f32 {
    if pitch.is_finite() {
        pitch.max(0.01)
    } else {
        1.0
    }
}

impl AudioManager {
    /// Creates a new audio manager bound to the default output device.
    ///
    /// # Web (WASM) note
    ///
    /// On `wasm32` the backend is the browser's `AudioContext` (via cpal's
    /// WebAudio backend). Browsers suspend an `AudioContext` created before a
    /// user gesture (autoplay policy): construct the `AudioManager` from an
    /// input handler (first click/keypress) rather than at startup, or the
    /// sinks will play silently.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::Backend`] if no audio output device is available
    /// or the default device cannot be opened.
    pub fn new() -> Result<Self, AudioError> {
        match DeviceSinkBuilder::open_default_sink() {
            Ok(mut device_sink) => {
                // rodio prints "Dropping DeviceSink, audio playing through this sink will stop"
                // to stderr on drop unless this is off. Here that drop *is* engine shutdown —
                // the manager owns the device for its whole life — so the warning is noise.
                device_sink.log_on_drop(false);
                let config = device_sink.config();
                let (mut manager, master) = Self::on_output(config.channel_count(), config.sample_rate());
                device_sink.mixer().add(master);
                manager._device_sink = Some(device_sink);
                log::info!("Gizmo Audio: Ses cihazı başlatıldı! 3D Uzamsal (Spatial) Motor Aktif.");
                Ok(manager)
            }
            Err(e) => {
                log::error!("Gizmo Audio Başarısız (Cihaz bulunamadı): {}", e);
                Err(AudioError::Backend(e.to_string()))
            }
        }
    }

    /// A manager whose mix comes out of the returned source instead of a device: what
    /// [`new`](Self::new) hands the device, and what a test pulls samples from.
    fn on_output(channels: rodio::ChannelCount, rate: rodio::SampleRate) -> (Self, limiter::MasterOut) {
        let equalizer = equalizer::EqualizerControl::new();
        let compressor = compressor::CompressorControl::new();
        let limiter = limiter::LimiterControl::new();
        let (bus, master) =
            limiter::master_bus(channels, rate, Arc::clone(&equalizer), Arc::clone(&compressor), Arc::clone(&limiter));
        let manager = Self {
            _device_sink: None,
            bus,
            equalizer,
            compressor,
            limiter,
            sound_buffers: HashMap::new(),
            active_spatial_sinks: HashMap::new(),
            active_sinks: HashMap::new(),
            sink_sounds: HashMap::new(),
            next_sink_id: 1,
            all_paused: false,
            sink_env: HashMap::new(),
            reverb: reverb::ReverbControl::new(),
            format: (channels, rate),
            nodes: HashMap::new(),
            links: std::collections::BTreeMap::new(),
            mixer: Mixer::new(),
        };
        (manager, master)
    }

    /// Goes to disk, reads the sound and stores it in RAM as a byte array
    pub fn load_sound(&mut self, name: &str, path: &str) -> Result<(), AudioError> {
        let mut file =
            File::open(Path::new(path)).map_err(|_| AudioError::NotFound(path.to_string()))?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).map_err(AudioError::Io)?;
        self.sound_buffers.insert(name.to_string(), buffer.into());
        Ok(())
    }

    /// Registers an already-decoded-from-disk (or embedded / fetched) sound
    /// buffer under `name`. The bytes must be a complete audio file in a
    /// format rodio can decode (WAV/OGG/FLAC/MP3), exactly as
    /// [`load_sound`](Self::load_sound) would have read from disk.
    ///
    /// This is the loading path for targets without a filesystem (WASM, where
    /// assets arrive via `fetch`/`include_bytes!`) and for games that embed
    /// audio in the binary.
    pub fn load_sound_bytes(&mut self, name: &str, bytes: impl Into<Arc<[u8]>>) {
        self.sound_buffers.insert(name.to_string(), bytes.into());
    }

    /// Registers mono `samples` in `[-1, 1]` at `sample_rate` as a sound called `name` — sound
    /// generated in code rather than read from a file.
    ///
    /// Encoded as a WAV ([`synth::wav_from_samples`]) and stored like
    /// [`load_sound_bytes`](Self::load_sound_bytes) stores a file, so it plays through the same
    /// decoder and the same sinks as everything else.
    pub fn load_samples(&mut self, name: &str, samples: &[f32], sample_rate: u32) {
        self.load_sound_bytes(name, synth::wav_from_samples(samples, sample_rate));
    }

    /// Per-frame housekeeping: collects the sounds that have finished, then re-prices every live
    /// sink **if** anything on the [`Mixer`] moved since the last frame.
    ///
    /// The `if` is the point. A game holds the mixer through [`AudioManager::mixer_mut`] and can
    /// turn a bus gain whenever it likes; the manager cannot be told, so it asks — one bool test
    /// per frame when nothing moved, two atomic stores per live sink when something did.
    pub fn update(&mut self) {
        self.clean_dead_sinks();
        if self.mixer.take_dirty() {
            self.apply_mix_to_every_sink();
        }
    }

    /// [`update`](Self::update) plus the fades: advances every [`fade_to`](Self::fade_to) /
    /// [`fade_out`](Self::fade_out) by `dt` seconds, re-prices the sinks they moved and stops the
    /// ones whose fade-out ended.
    ///
    /// **The engine calls this once a frame** for an `AudioManager` in the world's resources
    /// (`gizmo-app`'s frame loop, with the frame's real `dt`, so a fade runs through a paused
    /// game). Call it yourself only if you hold the manager somewhere else.
    pub fn tick(&mut self, dt: f32) {
        let step = self.mixer.advance_fades(dt);
        for id in &step.changed {
            self.apply_mix(*id);
        }
        for id in step.stop {
            self.stop(id);
        }
        self.update();
    }

    /// Moves a playing sound's volume to `factor` × what it was asked to play at, over `secs`
    /// seconds, from wherever a previous fade left it. Advanced by [`tick`](Self::tick). Does
    /// nothing for a sound that is not playing.
    ///
    /// The factor sits apart from the sound's volume, its bus and the master, so a music slider
    /// moved mid-fade, or the 3D system rewriting a spatial sound's volume every frame, does not
    /// undo it.
    pub fn fade_to(&mut self, id: u64, factor: f32, secs: f32) {
        self.mixer.start_fade(id, factor, secs, false);
    }

    /// Fades a playing sound to silence over `secs` seconds, then stops it.
    pub fn fade_out(&mut self, id: u64, secs: f32) {
        self.mixer.start_fade(id, 0.0, secs, true);
    }

    /// Starts `name` looping on the [`Mixer::MUSIC`] bus from silence and fades it in over `secs`
    /// seconds, fading `from` (the track playing now, if any) out and stopping it — the switch
    /// from a menu's music to a level's.
    ///
    /// # Errors
    ///
    /// As [`play_looped`](Self::play_looped); `from` keeps playing if the new track cannot start.
    pub fn crossfade(&mut self, from: Option<u64>, name: &str, secs: f32) -> Result<u64, AudioError> {
        // Silent from its first sample: it reaches the device already on the music bus at fade 0.
        let id = self.start(name, None, &PlayOptions::on_bus(Mixer::MUSIC).looped(true), 0.0)?;
        self.mixer.start_fade(id, 1.0, secs, false);
        if let Some(old) = from {
            self.fade_out(old, secs);
        }
        Ok(id)
    }

    /// The buses, the master gain and the environment — read-only. See [`Mixer`].
    #[inline]
    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    /// The mixer, to turn a gain on or set a send. Changes are heard on the next
    /// [`AudioManager::update`], which every frame calls; use the `set_*` methods here when a change
    /// must be audible before then. A sound's bus is the one it started on
    /// ([`PlayOptions::on_bus`]): its sound is summed there.
    #[inline]
    pub fn mixer_mut(&mut self) -> &mut Mixer {
        &mut self.mixer
    }

    /// Writes this sink's composed volume, speed and environment to rodio. Every mutator ends here.
    fn apply_mix(&self, id: u64) {
        let volume = self.mixer.volume_for(id);
        let speed = self.mixer.speed_for(id);
        if let Some(env) = self.sink_env.get(&id) {
            env.muffle.set_cutoff_hz(self.mixer.cutoff_hz_for(id));
            env.reverb_send.store(self.mixer.reverb_reaches(id), Ordering::Relaxed);
        }
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            sink.set_volume(volume);
            sink.set_speed(speed);
        } else if let Some(sink) = self.active_sinks.get(&id) {
            sink.set_volume(volume);
            sink.set_speed(speed);
        }
    }

    /// The node of `bus`, made in the master if this is the first time it is asked for.
    fn node(&mut self, bus: &str) -> &bus::BusNode {
        let (channels, rate) = self.format;
        let master = &self.bus;
        self.nodes.entry(bus.to_string()).or_insert_with(|| bus::BusNode::new(channels, rate, master))
    }

    /// Builds what [`Mixer::sends`] says: a link for each send, fed by its from-bus's output and
    /// read in its to-bus's mixer, at the send's level and the to-bus's gain; a link the mixer no
    /// longer has is closed.
    fn sync_routing(&mut self) {
        let wanted: Vec<(String, String, f32)> = self.mixer.sends().map(|(a, b, l)| (a.to_string(), b.to_string(), l)).collect();
        let mut changed: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        self.links.retain(|(from, to), link| {
            let keep = wanted.iter().any(|(a, b, _)| a == from && b == to);
            if !keep {
                link.close();
                changed.insert(from.clone());
            }
            keep
        });
        for (from, to, level) in wanted {
            self.node(&from);
            let (channels, rate) = self.format;
            if !self.links.contains_key(&(from.clone(), to.clone())) {
                let link = bus::SendLink::new(channels);
                self.node(&to).mixer.add(bus::SendReturn::new(Arc::clone(&link), channels, rate));
                self.links.insert((from.clone(), to.clone()), link);
                changed.insert(from.clone());
            }
            let link = &self.links[&(from.clone(), to.clone())];
            link.set_level(level);
            link.set_gain(self.mixer.return_gain(&to));
        }
        for from in changed {
            let feeds: Vec<Arc<bus::SendLink>> = self.links.iter().filter(|((a, _), _)| *a == from).map(|(_, l)| Arc::clone(l)).collect();
            if let Some(node) = self.nodes.get(&from) {
                node.control.set_sends(feeds);
            }
        }
    }

    /// Puts a reverb on one bus's sum — its sounds and what other buses send into it — or `None`
    /// to take it off (the default; off, the bus is bit-exact). A return bus, fed by sends
    /// ([`Mixer::set_send`]) and heard beside the buses that feed it, wants
    /// [`Reverb::with_dry`](reverb::Reverb::with_dry)`(0.0)`: their sound is heard already. Apart from
    /// [`set_reverb`](Self::set_reverb), the space every sound is in; heard on the next buffer.
    ///
    /// ```no_run
    /// # use gizmo_audio::{AudioManager, Mixer, reverb::Reverb};
    /// # let mut audio = AudioManager::new().unwrap();
    /// audio.set_bus_reverb("hall", Some(Reverb::hall().with_dry(0.0)));
    /// audio.mixer_mut().set_send(Mixer::SFX, "hall", 0.5).unwrap();
    /// audio.update();
    /// ```
    pub fn set_bus_reverb(&mut self, bus: &str, reverb: Option<reverb::Reverb>) {
        self.node(bus).control.reverb.set(reverb.filter(|r| r.wet > 0.0));
    }

    /// The reverb on a bus's sum ([`set_bus_reverb`](Self::set_bus_reverb)), `None` for none.
    #[must_use]
    pub fn bus_reverb(&self, bus: &str) -> Option<reverb::Reverb> {
        self.nodes.get(bus).map(|n| n.control.reverb.get()).filter(|r| r.wet > 0.0)
    }

    /// Re-prices every live sink — what a master/bus/environment change means, since those apply
    /// to sounds that are already playing.
    fn apply_mix_to_every_sink(&mut self) {
        self.mixer.take_dirty(); // whatever asked for this, it is answered now
        self.sync_routing();
        for id in self.active_sinks.keys().chain(self.active_spatial_sinks.keys()) {
            self.apply_mix(*id);
        }
    }

    // ── Su-altı ses boğma (underwater muffle) ────────────────────────────────
    /// Turns the underwater "muffle" on or off: every sound is turned down and low-passed, except
    /// the ones on a [dry](Mixer::set_bus_dry) bus.
    ///
    /// Idempotent, and — since 2026-08-18 — actually *reversible*. It used to multiply each
    /// sink's volume by 0.4 and undo it with 2.5, which had two measured consequences on real
    /// hardware: a 3D sound lost the muffle on the **next frame**, because the spatial system
    /// overwrites the sink's volume every frame (so the muffle was unreachable for every 3D sound
    /// in the engine), and surfacing multiplied it by 2.5 regardless — a sound set to 0.22 while
    /// submerged came back at 0.55. The state lives in the [`Mixer`] now and the volume is
    /// composed from it, so there is nothing to undo.
    pub fn set_underwater(&mut self, on: bool) {
        self.mixer.set_underwater(on);
        if self.mixer.take_dirty() {
            self.apply_mix_to_every_sink();
        }
    }

    /// Is the underwater muffle mode currently active.
    #[inline]
    pub fn is_underwater(&self) -> bool {
        self.mixer.is_underwater()
    }

    /// Sets the space every sound plays in — [`Reverb::cave`](reverb::Reverb::cave),
    /// [`hall`](reverb::Reverb::hall), [`room`](reverb::Reverb::room), or any shape — or `None` for
    /// the open air. Heard by every sound at once, those already playing included, on the next
    /// buffer: it is one store, read by a stage inside each source. Off is bit-exact.
    pub fn set_reverb(&mut self, reverb: Option<reverb::Reverb>) {
        self.reverb.set(reverb.filter(|r| r.wet > 0.0));
    }

    /// The space set by [`set_reverb`](Self::set_reverb), `None` for the open air.
    #[must_use]
    pub fn reverb(&self) -> Option<reverb::Reverb> {
        Some(self.reverb.get()).filter(|r| r.wet > 0.0)
    }

    /// Puts an equalizer on the whole mix — [`Equalizer`](equalizer::Equalizer), up to four bands of
    /// shelves and bells — or `None` to take it off (the default). It is the first stage on the
    /// sum, before the compressor and the limiter; off, the mix comes through bit-exact. Heard from
    /// the next frame of audio.
    pub fn set_equalizer(&mut self, equalizer: Option<equalizer::Equalizer>) {
        self.equalizer.set(equalizer.as_ref());
    }

    /// The equalizer set by [`set_equalizer`](Self::set_equalizer), `None` when off.
    #[must_use]
    pub fn equalizer(&self) -> Option<equalizer::Equalizer> {
        self.equalizer.get()
    }

    /// Puts a compressor on the whole mix — [`Compressor::default`](compressor::Compressor::default)
    /// is −18 dBFS at 4:1 — or `None` to take it off (the default). Above its threshold the mix is
    /// turned down by its ratio, so loud and quiet sounds sit closer together; it comes after the
    /// equalizer and before the limiter, and below the threshold, or off, the mix comes through
    /// bit-exact. Heard from the next frame of audio.
    pub fn set_compressor(&mut self, compressor: Option<compressor::Compressor>) {
        self.compressor.set(compressor);
    }

    /// The compressor set by [`set_compressor`](Self::set_compressor), `None` when off.
    #[must_use]
    pub fn compressor(&self) -> Option<compressor::Compressor> {
        self.compressor.get()
    }

    /// The deepest the compressor has turned the mix down since this was last called, in dB (`0`
    /// = it has not acted). For a gain-reduction meter.
    pub fn take_compressor_reduction(&mut self) -> f32 {
        self.compressor.take_max_reduction()
    }

    /// Puts a limiter on the whole mix — [`Limiter::default`](limiter::Limiter::default) is a
    /// −1 dBFS ceiling — or `None` to take it off (the default). With it on, any number of sounds
    /// at once stay under the ceiling instead of clipping at the device; the mix is turned down
    /// only while it would pass the ceiling, and comes through bit-exact otherwise. Heard from the
    /// next frame of audio.
    pub fn set_limiter(&mut self, limiter: Option<limiter::Limiter>) {
        self.limiter.set(limiter);
    }

    /// The limiter set by [`set_limiter`](Self::set_limiter), `None` when off.
    #[must_use]
    pub fn limiter(&self) -> Option<limiter::Limiter> {
        self.limiter.get()
    }

    /// The deepest the limiter has turned the mix down since this was last called, as a gain
    /// (`1.0` = it has not acted; `0.5` = the mix was halved at some moment). For a meter, or to
    /// notice a scene that is mixed too hot.
    pub fn take_limiter_gain(&mut self) -> f32 {
        self.limiter.take_min_gain()
    }

    /// Plays a normal (Global/Stereo) sound (one-shot)
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NotLoaded`] if `name` was never loaded, or
    /// [`AudioError::Decode`] if the bytes cannot be decoded. Attaching the sound to the output
    /// device cannot fail here — the device was opened once, in [`AudioManager::new`].
    pub fn play(&mut self, name: &str) -> Result<u64, AudioError> {
        self.play_with(name, &PlayOptions::default())
    }

    /// Plays a normal (Global/Stereo) sound in a loop
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NotLoaded`] if `name` was never loaded, or
    /// [`AudioError::Decode`] if the bytes cannot be decoded. Attaching the sound to the output
    /// device cannot fail here — the device was opened once, in [`AudioManager::new`].
    pub fn play_looped(&mut self, name: &str) -> Result<u64, AudioError> {
        self.play_with(name, &PlayOptions::default().looped(true))
    }

    /// Plays a normal (Global/Stereo) sound as `options` say: on its bus from the first sample,
    /// looped or not.
    ///
    /// ```no_run
    /// # use gizmo_audio::{AudioManager, Mixer, PlayOptions};
    /// # let mut audio = AudioManager::new().unwrap();
    /// let theme = audio.play_with("theme", &PlayOptions::on_bus(Mixer::MUSIC).looped(true))?;
    /// # Ok::<(), gizmo_audio::AudioError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// As [`play`](Self::play).
    pub fn play_with(&mut self, name: &str, options: &PlayOptions) -> Result<u64, AudioError> {
        self.start(name, None, options, 1.0)
    }

    /// Starts a sound: a global one, or a spatial one at `(emitter, left ear, right ear)`, on
    /// `options.bus` at `fade` — routed, priced and given its environment **before** it reaches the
    /// device.
    ///
    /// The order is the point. A `Player` connected to the bus plays what is appended to it from
    /// the next buffer the audio thread pulls, at the volume it has then. Until 2026-11-26 the
    /// source was appended first and the volume written after, so a sound started underwater, on a
    /// muted bus, or by a `crossfade` from silence could play its first buffer at full volume, dry.
    fn start(
        &mut self,
        name: &str,
        spatial: Option<([f32; 3], [f32; 3], [f32; 3])>,
        options: &PlayOptions,
        fade: f32,
    ) -> Result<u64, AudioError> {
        if !self.sound_buffers.contains_key(name) {
            log::error!("AudioManager: '{}' adlı ses bellekte yok!", name);
            return Err(AudioError::NotLoaded(name.to_string()));
        }
        // Connected to its bus's node, where it is summed with the bus's other sounds.
        let bus = self.node(&options.bus).mixer.clone();
        let bytes = self.sound_buffers.get(name).ok_or_else(|| {
            log::error!("AudioManager: '{}' adlı ses bellekte yok!", name);
            AudioError::NotLoaded(name.to_string())
        })?;
        let decoder = decode(bytes).map_err(|e| AudioError::Decode(e.to_string()))?;
        let id = self.next_sink_id;
        self.next_sink_id = self.next_sink_id.wrapping_add(1);

        self.mixer.route(id, &options.bus);
        if fade != 1.0 {
            self.mixer.set_fade(id, fade);
        }
        let volume = match spatial {
            Some((emitter, left, right)) => spatial_gain(ear_distance(emitter, left, right), options.max_distance, options.volume),
            None => options.volume,
        };
        self.mixer.set_sink_volume(id, volume);
        self.mixer.set_sink_pitch(id, options.pitch);
        let env = SinkEnvironment { muffle: MuffleControl::new(), reverb_send: Arc::new(AtomicBool::new(false)) };
        env.muffle.set_cutoff_hz(self.mixer.cutoff_hz_for(id));
        env.reverb_send.store(self.mixer.reverb_reaches(id), Ordering::Relaxed);
        let (volume, speed) = (self.mixer.volume_for(id), self.mixer.speed_for(id));
        let reverb = &self.reverb;
        // The muffle and the reverb wrap the source rather than the player: a `Player` hands out no
        // way to reach what it is playing, so a stage that must be switchable later has to be in
        // the chain before `append` and read its own parameter afterwards. Priced first, appended
        // before it is connected, connected last (`playing`). Not `repeat_infinite` for a loop: its
        // buffer cannot seek (see `looping`).
        let source: Box<dyn rodio::Source + Send> = if options.looped {
            Box::new(wrap(Looping::new(Arc::clone(bytes), decoder), reverb, &env))
        } else {
            Box::new(wrap(decoder, reverb, &env))
        };
        let price = |player: &Player| {
            player.set_volume(volume);
            player.set_speed(speed);
        };
        match spatial {
            Some(at) => {
                let sink = playing::SpatialSink::connect(&bus, at, price, source);
                self.active_spatial_sinks.insert(id, sink);
            }
            None => {
                let sink = playing::connect(&bus, |player| {
                    price(player);
                    player.append(source);
                });
                self.active_sinks.insert(id, sink);
            }
        }
        self.sink_env.insert(id, env);
        self.sink_sounds.insert(id, name.to_string());
        Ok(id)
    }

    /// Plays a 3D Spatial sound (one-shot)
    ///
    /// It starts at the engine's distance curve ([`spatial_gain`], over [`DEFAULT_MAX_DISTANCE`]),
    /// as an [`AudioSource`]'s does — not at rodio's own `1/d²`, which is what a sound played
    /// here got until 2026-11-22 (nothing updated it after the start, as the spatial system does
    /// an `AudioSource`): a script's `audio.play_3d` ten metres away played at 1 %.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NotLoaded`] if `name` was never loaded, or
    /// [`AudioError::Decode`] if the bytes cannot be decoded. Attaching the sound to the output
    /// device cannot fail here — the device was opened once, in [`AudioManager::new`].
    pub fn play_3d(
        &mut self,
        name: &str,
        emitter_pos: [f32; 3],
        left_ear: [f32; 3],
        right_ear: [f32; 3],
    ) -> Result<u64, AudioError> {
        self.play_3d_with(name, emitter_pos, left_ear, right_ear, &PlayOptions::default())
    }

    /// Plays a 3D Spatial sound in a loop
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NotLoaded`] if `name` was never loaded, or
    /// [`AudioError::Decode`] if the bytes cannot be decoded. Attaching the sound to the output
    /// device cannot fail here — the device was opened once, in [`AudioManager::new`].
    pub fn play_3d_looped(
        &mut self,
        name: &str,
        emitter_pos: [f32; 3],
        left_ear: [f32; 3],
        right_ear: [f32; 3],
    ) -> Result<u64, AudioError> {
        self.play_3d_with(name, emitter_pos, left_ear, right_ear, &PlayOptions::default().looped(true))
    }

    /// Plays a 3D Spatial sound as `options` say: on its bus from the first sample, looped or
    /// not. What an [`AudioSource`] with a [`bus`](AudioSource::bus) starts with.
    ///
    /// # Errors
    ///
    /// As [`play_3d`](Self::play_3d).
    pub fn play_3d_with(
        &mut self,
        name: &str,
        emitter_pos: [f32; 3],
        left_ear: [f32; 3],
        right_ear: [f32; 3],
        options: &PlayOptions,
    ) -> Result<u64, AudioError> {
        self.start(name, Some((emitter_pos, left_ear, right_ear)), options, 1.0)
    }

    // ========== ECS SINK GÜNCELLEMELERİ ==========

    /// Updates an active spatial sink's emitter/ear positions and recomputes
    /// its volume based on distance attenuation and `base_volume`.
    pub fn update_spatial_sink(
        &mut self,
        id: u64,
        emitter_pos: [f32; 3],
        left_ear: [f32; 3],
        right_ear: [f32; 3],
        max_distance: f32,
        base_volume: f32,
    ) {
        let Some(sink) = self.active_spatial_sinks.get(&id) else {
            return;
        };
        sink.set_emitter_position(emitter_pos);
        sink.set_left_ear_position(left_ear);
        sink.set_right_ear_position(right_ear);

        let distance = ear_distance(emitter_pos, left_ear, right_ear);

        // Distance attenuation is INTENT, not the sink's volume — that distinction is the fix for
        // the muffle this call used to erase every frame (see `set_underwater`).
        self.mixer
            .set_sink_volume(id, spatial_gain(distance, max_distance, base_volume));
        self.apply_mix(id);
    }

    /// Sets the volume this sink was *asked* to play at. What reaches the device is that value
    /// times its bus, the master and the environment — so this survives a dive and a surface, and
    /// a bus gain applies on top of it rather than replacing it. See [`Mixer`].
    pub fn set_volume(&mut self, id: u64, volume: f32) {
        self.mixer.set_sink_volume(id, volume);
        self.apply_mix(id);
    }

    /// Sets the pitch/playback speed this sink was asked to play at (Doppler included). The
    /// underwater slow-down multiplies it; the clamp that keeps rodio's sample-rate converter out
    /// of its `from >= 1` assert is applied after that multiply, in [`Mixer::speed_for`].
    pub fn set_pitch(&mut self, id: u64, pitch: f32) {
        self.mixer.set_sink_pitch(id, pitch);
        self.apply_mix(id);
    }

    /// Where a playing sound is, in seconds of the sound from its start — the clock a rhythm
    /// game times its notes against, since the frame clock drifts from what the speaker plays
    /// (output latency, dropped frames, a stall). `None` for a sound that is not playing.
    ///
    /// The position of a sound played with a pitch other than 1 is in the *sound's* seconds, not
    /// the wall clock's. For a looped sound it keeps counting past the end — take it modulo
    /// [`duration`](Self::duration). Updated by the audio thread every few milliseconds.
    #[must_use]
    pub fn position(&self, id: u64) -> Option<f32> {
        if let Some(s) = self.active_spatial_sinks.get(&id) {
            return Some(s.get_pos().as_secs_f32());
        }
        self.active_sinks.get(&id).map(|s| s.get_pos().as_secs_f32())
    }

    /// Moves a playing sound to `secs` from its start — resuming a track where it was, skipping an
    /// intro, rewinding a practice section.
    ///
    /// # Errors
    ///
    /// [`AudioError::Seek`] when no such sound is playing, or its format cannot seek.
    pub fn seek(&mut self, id: u64, secs: f32) -> Result<(), AudioError> {
        let to = std::time::Duration::from_secs_f32(secs.max(0.0));
        let result = if let Some(s) = self.active_spatial_sinks.get(&id) {
            s.try_seek(to)
        } else if let Some(s) = self.active_sinks.get(&id) {
            s.try_seek(to)
        } else {
            return Err(AudioError::Seek(format!("no playing sound {id}")));
        };
        result.map_err(|e| AudioError::Seek(e.to_string()))
    }

    /// How long a loaded sound is, in seconds — `None` if it was never loaded or its format does
    /// not say (a WAV always does).
    #[must_use]
    pub fn duration(&self, name: &str) -> Option<f32> {
        self.sound_buffers.get(name).and_then(decoded_duration)
    }

    /// Stops the active sink with the given id.
    pub fn stop(&mut self, id: u64) {
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            sink.stop();
        } else if let Some(sink) = self.active_sinks.get(&id) {
            sink.stop();
        }
    }

    /// Pauses the active sink with the given id.
    pub fn pause(&mut self, id: u64) {
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            sink.pause();
        } else if let Some(sink) = self.active_sinks.get(&id) {
            sink.pause();
        }
    }

    /// Resumes the (paused) active sink with the given id.
    pub fn resume(&mut self, id: u64) {
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            sink.play();
        } else if let Some(sink) = self.active_sinks.get(&id) {
            sink.play();
        }
    }

    /// Cleans up the playing sounds that have finished (Sinks) like a Garbage Collector
    pub fn clean_dead_sinks(&mut self) {
        self.active_spatial_sinks.retain(|_, sink| !sink.empty());
        self.active_sinks.retain(|_, sink| !sink.empty());
        // A route outlives its sink otherwise: one leaked entry per sound ever played, which in a
        // session is every footstep.
        let live: HashSet<u64> = self
            .active_sinks
            .keys()
            .chain(self.active_spatial_sinks.keys())
            .copied()
            .collect();
        self.mixer.retain_routes(|id| live.contains(&id));
        self.sink_sounds.retain(|id, _| live.contains(id));
        self.sink_env.retain(|id, _| live.contains(id));
    }

    /// Stops every live sound that was started from `name`, and reports how many it stopped.
    ///
    /// A sink id is what [`AudioManager::play`] hands back and what engine code keeps, but a
    /// *name* is all a script ever has — `audio.stop("music")` cannot mean anything else. Stopping
    /// **all** of them is deliberate: a name is not an instance, so a game that fired the same
    /// footstep three times and asks for it to stop means all three.
    ///
    /// Returns `0` for a name nothing is playing, which is not an error — a script stopping a
    /// sound that has already finished is ordinary.
    pub fn stop_by_name(&mut self, name: &str) -> usize {
        let ids: Vec<u64> = self
            .sink_sounds
            .iter()
            .filter(|(_, sound)| sound.as_str() == name)
            .map(|(id, _)| *id)
            .collect();
        for id in &ids {
            self.stop(*id);
        }
        ids.len()
    }

    /// Is a sound registered under this name?
    ///
    /// The question a host has to ask before it can *supply* a missing one: `play` and its
    /// neighbours answer [`AudioError::NotLoaded`] after the fact, which is the right answer for a
    /// game that manages its own sounds and no help at all to a loader deciding what to read from
    /// disk. Loading is by name ([`AudioManager::load_sound`] /
    /// [`AudioManager::load_sound_bytes`]), so this is also what keeps a host from re-reading a
    /// file the game already registered under the same name — the game's bytes win.
    pub fn is_loaded(&self, name: &str) -> bool {
        self.sound_buffers.contains_key(name)
    }

    /// Stops every live sink, and reports how many it stopped.
    ///
    /// `stop_by_name` is the script's verb; this is the *session's*. The editor's ⏹ restores the
    /// scene it snapshotted, and a snapshot cannot restore a sound: the sinks live on the device
    /// behind this manager, which is a resource and not part of any scene. Without this, stopping
    /// a game left its looping ambience playing over the editor for the rest of the session.
    pub fn stop_all(&mut self) -> usize {
        let ids: Vec<u64> = self
            .active_spatial_sinks
            .keys()
            .chain(self.active_sinks.keys())
            .copied()
            .collect();
        for id in &ids {
            self.stop(*id);
        }
        ids.len()
    }

    /// Pause (or resume) every live sink, for a host that has paused the game.
    ///
    /// ⏸ is not ⏹: the scene is still there, the snapshot is still held, and the sounds should
    /// hold too — a paused editor used to keep playing the level's ambience over a frozen frame,
    /// because nothing on the pause path reached the device. (⏹ is [`AudioManager::stop_all`],
    /// which ends them instead.)
    ///
    /// **Only a change is pushed to the sinks**, so this is safe to call every frame: a game that
    /// paused one sound of its own keeps it paused while the host's answer stays the same. The
    /// exception is the transition itself — resuming the game resumes everything, including a
    /// sink the game had paused for its own reasons.
    ///
    /// Returns how many sinks were touched (`0` when nothing changed).
    pub fn set_all_paused(&mut self, paused: bool) -> usize {
        if paused == self.all_paused {
            return 0;
        }
        self.all_paused = paused;
        let ids: Vec<u64> = self
            .active_spatial_sinks
            .keys()
            .chain(self.active_sinks.keys())
            .copied()
            .collect();
        for id in &ids {
            if paused {
                self.pause(*id);
            } else {
                self.resume(*id);
            }
        }
        ids.len()
    }

    /// The loaded sound a live sink is playing, if the manager still knows it.
    pub fn sink_sound(&self, id: u64) -> Option<&str> {
        self.sink_sounds.get(&id).map(String::as_str)
    }

    /// Returns whether the sink with the given id is currently playing.
    pub fn is_playing(&self, id: u64) -> bool {
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            !sink.empty() && !sink.is_paused()
        } else if let Some(sink) = self.active_sinks.get(&id) {
            !sink.empty() && !sink.is_paused()
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{sanitize_playback_speed, AudioManager, AudioSource, Mixer};
    use rodio::{Decoder, Player, Source};
    use std::io::Cursor;

    /// A complete 16-bit mono PCM WAV file, `samples` frames of a cheap square wave.
    ///
    /// Built in-process rather than committed as a fixture: the point is to exercise the
    /// *decoder*, and a handful of bytes assembled here cannot rot, go missing from a package,
    /// or drag a binary blob into the repository.
    fn wav_bytes(samples: u32) -> Vec<u8> {
        const RATE: u32 = 8_000;
        let data_len = samples * 2;
        let mut v = Vec::with_capacity(44 + data_len as usize);
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
        v.extend_from_slice(&1u16.to_le_bytes()); // PCM
        v.extend_from_slice(&1u16.to_le_bytes()); // mono
        v.extend_from_slice(&RATE.to_le_bytes());
        v.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
        v.extend_from_slice(&2u16.to_le_bytes()); // block align
        v.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..samples {
            let s: i16 = if i % 32 < 16 { 8_000 } else { -8_000 };
            v.extend_from_slice(&s.to_le_bytes());
        }
        v
    }

    /// The playback path — decode bytes, queue them on a player, mutate volume/speed — works
    /// **without an output device**, so it is assertable on CI hardware that has no sound card.
    ///
    /// This is the regression test for the rodio 0.17 → 0.22 migration (2026-08-17). That upgrade
    /// replaced every decoder: WAV used to go through `hound`, and now goes through `symphonia`.
    /// A format the old backend accepted and the new one rejects would not break the build — it
    /// would make `AudioManager::play` return `Decode` at runtime, i.e. silence, which is exactly
    /// the failure a compile-checked migration misses.
    #[test]
    fn a_decoded_sound_queues_on_a_device_free_player() {
        let decoder = Decoder::new(Cursor::new(wav_bytes(256)))
            .expect("the WAV decoder must accept a plain 16-bit PCM file");

        // `Player::new` is the device-free half of `connect_new`: the queue output would be fed
        // to a mixer in real playback. Holding it keeps the queue alive for the assertions.
        let (player, _queue) = Player::new();
        assert!(player.empty(), "a fresh player holds no sound");

        player.append(decoder.repeat_infinite());
        assert!(!player.empty(), "the decoded sound must be queued on the player");
        assert!(!player.is_paused(), "playback starts running, as it did on 0.17");

        // The composed numbers land on a real `Player`: this is the half the mixer's own tests
        // cannot show, since they never touch rodio. `Mixer` decides, the player obeys.
        let mut mixer = Mixer::new();
        mixer.route(7, Mixer::SFX);
        mixer.set_underwater(true);
        player.set_volume(mixer.volume_for(7));
        player.set_speed(mixer.speed_for(7));
        assert!(
            player.volume() < 1.0,
            "underwater must turn the sound down: vol {}",
            player.volume()
        );
        assert_eq!(
            player.speed(),
            1.0,
            "and must NOT slow it: the muffle is a low-pass filter inside the source, not a \
             pitch shift — that stand-in was removed 2026-08-18"
        );
        assert!(mixer.cutoff_hz_for(7) > 0, "the corner is what carries the muffle now");

        // Not asserted here: that `stop()` empties the player. Emptiness is decremented as the
        // *mixer pulls samples*, so with no device nothing ever drains — which is also why
        // `clean_dead_sinks`'s garbage collection cannot be tested without hardware.
        player.stop();
    }

    /// End-to-end on **real hardware**: open the default device, play a sound of known length,
    /// and require that it has finished by the time it should have.
    ///
    /// A sound only drains as the device's callback pulls samples through it, so this passing is
    /// evidence the whole chain ran — device open, mixer, symphonia decode, sample-rate
    /// conversion — not merely that it type-checks. The device-free test above cannot show that.
    ///
    /// `#[ignore]` because CI runners have no sound card, where `AudioManager::new` legitimately
    /// fails. Run it by hand after touching the audio backend:
    ///
    /// ```text
    /// cargo test -p gizmo-audio -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a real audio output device — run with --ignored"]
    fn a_real_device_drains_a_played_sound() {
        let mut manager = match AudioManager::new() {
            Ok(m) => m,
            Err(e) => panic!("no audio output device on this machine: {e}"),
        };
        // 8000 frames at 8 kHz = exactly one second.
        manager.load_sound_bytes("beep", wav_bytes(8_000));

        let id = manager.play("beep").expect("a registered WAV must decode and play");
        assert!(manager.is_playing(id), "playback must start immediately");

        std::thread::sleep(std::time::Duration::from_millis(1_500));
        manager.update(); // the frame-time garbage collector

        assert!(
            !manager.is_playing(id),
            "a one-second sound must have drained after 1.5 s — if it has not, the device is \
             not pulling samples and nothing is audible"
        );

        // Again with the low-pass engaged. The filter's response is measured device-free in
        // `crate::filter`; what only a device can show is that a source with a biquad in it still
        // satisfies rodio's span/rate contract well enough to be pulled to the end. A filter that
        // stalls or mis-reports its span is silence, and every arithmetic test would still pass.
        manager.set_underwater(true);
        let filtered = manager.play("beep").expect("a registered WAV must decode and play");
        assert!(manager.is_playing(filtered));
        std::thread::sleep(std::time::Duration::from_millis(1_500));
        manager.update();
        assert!(
            !manager.is_playing(filtered),
            "a muffled sound must drain like any other — if it does not, the filter is not \
             passing the device's pulls through"
        );
    }

    /// REGRESSION, on **real hardware** (the numbers below were measured on a device, 2026-08-18).
    ///
    /// The mixer's own tests assert the composition; this asserts that the composition is what the
    /// device actually receives — i.e. that every path in `AudioManager` writes
    /// `mixer.volume_for(id)` and nothing writes a level of its own. It is the half that would
    /// still be broken if one call site kept multiplying into the sink.
    ///
    /// Before the mixer, on this same hardware:
    ///
    /// | step | volume | speed |
    /// |---|---|---|
    /// | 3D playing | 1.00 | 1.00 |
    /// | underwater | 0.40 | 0.85 |
    /// | after ONE `audio_spatial_system` frame | **1.00** | **1.00** |
    /// | surfaced | **2.50** | 1.00 |
    /// | 2D at 0.22, set while submerged, then surfaced | **0.55** | 1.00 |
    ///
    /// The `0.85` there is history too: the speed column stays at 1.00 now, because the muffle is
    /// a low-pass filter inside the source rather than a playback slow-down (`crate::filter`).
    ///
    /// `#[ignore]` for the usual reason: CI runners have no sound card, where `AudioManager::new`
    /// legitimately fails. `cargo test -p gizmo-audio -- --ignored`
    #[test]
    #[ignore = "needs a real audio output device — run with --ignored"]
    fn the_device_gets_the_mixers_numbers_and_nothing_accumulates() {
        fn spatial(m: &AudioManager, id: u64) -> (f32, f32) {
            let s = &m.active_spatial_sinks[&id];
            (s.volume(), s.speed())
        }
        fn global(m: &AudioManager, id: u64) -> (f32, f32) {
            let s = &m.active_sinks[&id];
            (s.volume(), s.speed())
        }
        let mut m = match AudioManager::new() {
            Ok(m) => m,
            Err(e) => panic!("no audio output device on this machine: {e}"),
        };
        m.load_sound_bytes("beep", wav_bytes(80_000)); // 10 s: long enough not to drain under us

        let (left, right) = ([-0.1f32, 0.0, 0.0], [0.1f32, 0.0, 0.0]);
        let id = m
            .play_3d_looped("beep", [0.0, 0.0, 0.0], left, right)
            .expect("a registered WAV must play");
        assert_eq!(spatial(&m, id), (1.0, 1.0), "a fresh sound plays at what it was given");

        m.set_underwater(true);
        assert_eq!(spatial(&m, id), (0.4, 1.0), "the muffle turns it down and does not detune it");
        assert!(m.sink_env[&id].muffle.cutoff_hz() > 0, "and the filter its source reads is engaged");

        // One frame of `audio_spatial_system` for a live 3D source: attenuation, then Doppler.
        // This is the write that used to erase the muffle — for EVERY 3D sound, one frame after
        // it was asked for.
        m.update_spatial_sink(id, [0.0, 0.0, 0.0], left, right, 100.0, 1.0);
        m.set_pitch(id, 1.0);
        assert_eq!(spatial(&m, id), (0.4, 1.0), "and it survives the frame that restates it");

        m.set_underwater(false);
        assert_eq!(spatial(&m, id), (1.0, 1.0), "surfacing is not a 2.5x amplifier");
        assert_eq!(m.sink_env[&id].muffle.cutoff_hz(), 0, "and the filter goes back to bypass, not to 20 kHz");

        // The global path, with a game moving a volume slider while submerged.
        let g = m.play_looped("beep").expect("a registered WAV must play");
        m.set_volume(g, 0.22);
        assert_eq!(global(&m, g), (0.22, 1.0));
        m.set_underwater(true);
        assert_eq!(global(&m, g), (0.088, 1.0));
        m.set_volume(g, 0.22); // the slider, moved underwater
        assert_eq!(global(&m, g), (0.088, 1.0), "a value set underwater is muffled, not doubled");
        m.set_underwater(false);
        assert_eq!(global(&m, g), (0.22, 1.0), "and it comes back as the value it was given");

        // A bus gain applies to what is already playing, through `update`'s dirty check.
        m.mixer_mut().set_bus_gain(Mixer::DEFAULT_BUS, 0.5);
        assert_eq!(global(&m, g), (0.22, 1.0), "a gain turned through mixer_mut waits for update");
        m.update();
        assert_eq!(global(&m, g), (0.11, 1.0), "and update is where it is heard");
    }

    /// A scene saved before buses existed must still load — and land somewhere audible.
    ///
    /// `AudioSource` is serialised into scene files, so a new field is a compatibility question
    /// before it is a feature. `#[serde(default)]` answers it, and this is what checks the answer:
    /// the field's absence must mean [`Mixer::DEFAULT_BUS`], not an empty bus name (which would be
    /// a *real* bus called `""`, at unity, invisible to every slider a settings panel draws).
    #[test]
    fn a_scene_saved_before_buses_still_loads_onto_one() {
        let pre_bus = r#"{
            "sound_name": "waterfall",
            "is_3d": true,
            "volume": 0.8,
            "pitch": 1.0,
            "loop_sound": true,
            "max_distance": 40.0,
            "_internal_sink_id": null
        }"#;
        let source: AudioSource =
            serde_json::from_str(pre_bus).expect("a pre-bus AudioSource must still deserialize");
        assert_eq!(source.bus, Mixer::DEFAULT_BUS);
        assert_eq!(source.sound_name, "waterfall");
        assert_eq!(source.max_distance, 40.0);

        // And the round trip keeps whatever the scene did say.
        let routed = AudioSource::new("theme").with_bus(Mixer::MUSIC);
        let json = serde_json::to_string(&routed).expect("serialize");
        let back: AudioSource = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.bus, Mixer::MUSIC);
        assert_eq!(back, routed);
    }

    #[test]
    fn playback_speed_never_reaches_zero() {
        // 0 / negative / NaN would make rodio's SampleRateConverter assert (from >= 1)
        // and panic the audio thread. All must clamp to a strictly-positive factor
        // such that `orig_rate * factor >= 1` for any realistic rate (>= ~100 Hz).
        assert!(sanitize_playback_speed(0.0) >= 0.01);
        assert!(sanitize_playback_speed(-2.0) >= 0.01);
        assert_eq!(sanitize_playback_speed(f32::NAN), 1.0);
        assert_eq!(sanitize_playback_speed(f32::INFINITY), 1.0);
        // A normal pitch passes through untouched.
        assert_eq!(sanitize_playback_speed(1.5), 1.5);
        assert_eq!(sanitize_playback_speed(0.5), 0.5);
    }

    /// A sink id names a live sound on a device this process opened; a scene file outlives both.
    ///
    /// Saving a scene *while playing* used to write the id into it, and reloading produced a
    /// source that believed it was already playing. For a looping source that is permanent — the
    /// spatial system only clears a stale id for a one-shot — so the sound never started again.
    #[test]
    fn a_saved_source_does_not_carry_a_live_sink_id() {
        let mut playing = AudioSource::new("music");
        playing._internal_sink_id = Some(7);
        playing.has_played = true;

        let json = serde_json::to_string(&playing).expect("AudioSource is serializable");
        assert!(
            !json.contains("_internal_sink_id"),
            "a runtime sink id must not reach the scene file: {json}"
        );

        let loaded: AudioSource = serde_json::from_str(&json).expect("and it round-trips");
        assert_eq!(loaded._internal_sink_id, None);
        assert!(!loaded.has_played, "the autostart latch is runtime state too");
        assert_eq!(loaded.sound_name, "music");

        // A file written BEFORE the field was skipped still loads, and the stale id is dropped.
        let old = r#"{"sound_name":"music","is_3d":true,"volume":1.0,"pitch":1.0,
            "loop_sound":false,"max_distance":100.0,"_internal_sink_id":7}"#;
        let recovered: AudioSource = serde_json::from_str(old).expect("old scenes still load");
        assert_eq!(recovered._internal_sink_id, None);
    }

}

gizmo_core::impl_component!(AudioSource);

pub mod host;

#[cfg(test)]
mod spatial_gain_tests {
    use super::spatial_gain;

    /// What rodio 0.22.2 does to each ear underneath us: `min(1/dist², 1)`
    /// (`source/spatial.rs::set_positions`). Written out so the composed curve below is checked
    /// against the library's actual rule rather than against the same assumption twice.
    fn rodio_distance_modifier(distance: f32) -> f32 {
        (1.0 / (distance * distance)).min(1.0)
    }

    /// **What the listener hears follows `max_distance`.**
    ///
    /// The engine's taper used to be multiplied by rodio's inverse-square term, so a source
    /// authored to carry 100 m was at 0.9 % by 10 m. Composed with the cancellation, the heard
    /// level is the taper itself.
    #[test]
    fn the_heard_curve_is_the_linear_taper_the_field_documents() {
        for (distance, expected) in [(1.0, 0.99), (10.0, 0.90), (50.0, 0.50), (90.0, 0.10)] {
            let written = spatial_gain(distance, 100.0, 1.0);
            let heard = written * rodio_distance_modifier(distance);
            assert!(
                (heard - expected).abs() < 1e-4,
                "at {distance} m the listener hears {heard}, not the documented {expected}"
            );
        }
    }

    /// The cancellation can only ever *undo* an attenuation, never add gain: at every distance the
    /// heard level stays at or below the volume the source asked for.
    #[test]
    fn nothing_is_ever_louder_than_the_volume_it_asked_for() {
        for step in 0..200 {
            let distance = step as f32 * 0.5;
            let heard = spatial_gain(distance, 100.0, 0.8) * rodio_distance_modifier(distance);
            assert!(heard <= 0.8 + 1e-4, "at {distance} m the sound gained volume: {heard}");
        }
    }

    /// Past `max_distance` the sound is silent, which is the field's whole promise — and a
    /// `max_distance` of zero is the "no distance model" escape hatch, not a division by zero.
    #[test]
    fn beyond_the_limit_is_silence_and_zero_means_no_falloff() {
        assert_eq!(spatial_gain(100.0, 100.0, 1.0), 0.0);
        assert_eq!(spatial_gain(1_000.0, 100.0, 1.0), 0.0);
        assert_eq!(spatial_gain(50.0, 0.0, 0.7), 0.7 * 50.0_f32.powi(2));
    }
}

#[cfg(test)]
mod position_tests {
    use super::{decoded_duration, synth, AudioManager};
    use std::sync::Arc;

    /// The length a sound's bytes decode to — what `duration` reports, and a rhythm game's loop
    /// length.
    #[test]
    fn a_wavs_duration_is_read_from_its_bytes() {
        let wav: Arc<[u8]> = synth::wav_from_samples(&vec![0.0; 66_150], 44_100).into();
        let d = decoded_duration(&wav).expect("a WAV states its length");
        assert!((d - 1.5).abs() < 1e-3, "{d}");
        let junk: Arc<[u8]> = vec![1u8, 2, 3].into();
        assert_eq!(decoded_duration(&junk), None);
    }

    /// On real hardware: a playing sound's position advances with what the device has played,
    /// `seek` moves it — on a looped sound too, which is what a rhythm game's restart does — and
    /// a sound that is not playing has no position and cannot seek.
    ///
    /// `cargo test -p gizmo-audio -- --ignored position`
    #[test]
    #[ignore = "needs a real audio output device — run with --ignored"]
    fn a_playing_sound_reports_and_moves_its_position() {
        let mut audio = AudioManager::new().expect("an audio output device");
        audio.load_samples("tone", &vec![0.0; 44_100 * 4], 44_100);
        assert!((audio.duration("tone").unwrap() - 4.0).abs() < 1e-3);
        assert_eq!(audio.duration("nothing"), None);
        let id = audio.play_looped("tone").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let p = audio.position(id).expect("playing");
        assert!((0.1..1.0).contains(&p), "after 0.3 s the position is {p}");
        audio.seek(id, 2.5).expect("a looped WAV seeks");
        std::thread::sleep(std::time::Duration::from_millis(50));
        let p = audio.position(id).unwrap();
        assert!((2.4..3.0).contains(&p), "after seeking to 2.5 s the position is {p}");
        assert!(audio.position(u64::MAX).is_none());
        assert!(audio.seek(u64::MAX, 1.0).is_err());
    }

    /// A one-shot 3D sound starts on the engine's distance curve, as an `AudioSource` does: ten
    /// metres away its sink is asked for `spatial_gain(10, 100, 1)` — the factor that cancels
    /// rodio's own `1/d²` — and not left at 1, which played it at 1 %.
    ///
    /// `cargo test -p gizmo-audio -- --ignored one_shot`
    #[test]
    #[ignore = "needs a real audio output device — run with --ignored"]
    fn a_one_shot_3d_sound_starts_on_the_engine_s_distance_curve() {
        let mut audio = AudioManager::new().expect("an audio output device");
        audio.load_samples("tone", &vec![0.0; 44_100], 44_100);
        let id = audio.play_3d("tone", [10.0, 0.0, 0.0], [-0.1, 0.0, 0.0], [0.1, 0.0, 0.0]).unwrap();
        let asked = audio.mixer().volume_for(id);
        let expected = crate::spatial_gain(10.0, crate::DEFAULT_MAX_DISTANCE, 1.0);
        assert!((asked - expected).abs() < 1e-3 * expected, "asked for {asked}, the curve says {expected}");
    }

    /// On real hardware: the master bus (and its limiter) carries sound played after a stretch
    /// of silence — an idle rodio mixer reads as finished and would be unplugged — and eight loud
    /// sounds at once are turned down by the limiter, which reports it.
    ///
    /// `cargo test -p gizmo-audio -- --ignored limiter`
    #[test]
    #[ignore = "needs a real audio output device — run with --ignored"]
    fn the_master_limiter_turns_a_hot_mix_down_on_a_device() {
        let mut audio = AudioManager::new().expect("an audio output device");
        audio.set_limiter(Some(crate::limiter::Limiter::default()));
        std::thread::sleep(std::time::Duration::from_millis(300));
        let tone: Vec<f32> = (0..44_100)
            .map(|i| 0.6 * (std::f32::consts::TAU * 220.0 * i as f32 / 44_100.0).sin())
            .collect();
        audio.load_samples("tone", &tone, 44_100);
        let first = audio.play("tone").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(audio.position(first).unwrap_or(0.0) > 0.05, "sound after silence must play");
        audio.stop(first);
        audio.take_limiter_gain();
        for _ in 0..8 {
            audio.play("tone").unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        let g = audio.take_limiter_gain();
        assert!(g < 0.3, "eight voices at 0.6 sum near 4.8; the limiter's gain was {g}");
        audio.set_limiter(None);
        assert_eq!(audio.limiter(), None);
    }
}

/// A manager on an output of its own, its mix pulled here: no device, so these run everywhere.
#[cfg(test)]
mod device_free_tests {
    use super::{reverb, AudioManager, Mixer, PlayOptions};

    const RATE: u32 = 8_000;

    /// A manager whose mix this test pulls, mono at 8 kHz, with a second of a 2 kHz tone loaded
    /// as `"tone"`: above the underwater corner, so the muffle changes it.
    fn on_output() -> (AudioManager, crate::limiter::MasterOut) {
        let (mut m, out) = AudioManager::on_output(
            std::num::NonZero::new(1).unwrap(),
            std::num::NonZero::new(RATE).unwrap(),
        );
        let tone: Vec<f32> = (0..RATE).map(|i| 0.5 * (i as f32 * 2.0 * std::f32::consts::PI * 2_000.0 / RATE as f32).sin()).collect();
        m.load_samples("tone", &tone, RATE);
        (m, out)
    }

    fn pull(out: &mut crate::limiter::MasterOut, n: usize) -> Vec<f32> {
        (0..n).map(|_| out.next().unwrap_or(0.0)).collect()
    }

    fn energy(s: &[f32]) -> f32 {
        s.iter().map(|x| x * x).sum()
    }

    /// A sound started on a muted bus is silent from its first sample. Until 2026-11-26 a sound
    /// could only start on the default bus and be moved after (`play`, then a bus change), and
    /// the source reached the device before its volume was written.
    #[test]
    fn a_sound_started_on_a_muted_bus_is_silent_from_its_first_sample() {
        let (mut m, mut out) = on_output();
        m.mixer_mut().set_bus_muted(Mixer::MUSIC, true);
        m.update();
        let id = m.play_with("tone", &PlayOptions::on_bus(Mixer::MUSIC)).unwrap();
        assert_eq!(m.mixer().sink_bus(id), Some(Mixer::MUSIC));
        assert!(pull(&mut out, 2_000).iter().all(|&s| s == 0.0), "a muted bus let its first samples through");

        let (mut m, mut out) = on_output();
        m.play_with("tone", &PlayOptions::on_bus(Mixer::SFX)).unwrap();
        assert!(energy(&pull(&mut out, 2_000)) > 1.0, "premise: the tone is heard");
    }

    /// A dry bus plays as it was mixed under water and in a cave: sample for sample what the same
    /// sound plays with no environment at all, while the same sound on a bus in the world is
    /// muffled. Until 2026-11-26 the environment reached every bus, a menu's music too.
    #[test]
    fn a_dry_bus_plays_as_mixed_under_water_and_in_a_cave() {
        let (mut m, mut out) = on_output();
        m.play_with("tone", &PlayOptions::on_bus(Mixer::MUSIC)).unwrap();
        let reference = pull(&mut out, 4_000);

        let submerged = |bus: &str| {
            let (mut m, mut out) = on_output();
            m.set_underwater(true);
            m.set_reverb(Some(reverb::Reverb::cave()));
            m.mixer_mut().set_bus_dry(Mixer::MUSIC, true);
            m.update();
            m.play_with("tone", &PlayOptions::on_bus(bus)).unwrap();
            pull(&mut out, 4_000)
        };
        assert_eq!(submerged(Mixer::MUSIC), reference, "the dry bus heard the environment");
        let in_the_world = submerged(Mixer::SFX);
        assert!(
            energy(&in_the_world) < 0.2 * energy(&reference),
            "premise: a bus in the world is muffled ({} of {})",
            energy(&in_the_world),
            energy(&reference)
        );
    }

    /// Setting a bus dry reaches the sounds already playing on it, out of the water and out of the
    /// cave: the samples after are the unaffected sound's.
    #[test]
    fn a_bus_set_dry_mid_sound_leaves_the_environment_at_once() {
        let (mut m, mut out) = on_output();
        m.play_with("tone", &PlayOptions::on_bus(Mixer::MUSIC)).unwrap();
        let reference = pull(&mut out, 2_000);

        let (mut m, mut out) = on_output();
        m.set_underwater(true);
        m.set_reverb(Some(reverb::Reverb::cave()));
        m.play_with("tone", &PlayOptions::on_bus(Mixer::MUSIC)).unwrap();
        let muffled = pull(&mut out, 1_000);
        assert!(energy(&muffled) < 0.2 * energy(&reference[..1_000]), "premise: it starts muffled");
        m.mixer_mut().set_bus_dry(Mixer::MUSIC, true);
        m.update();
        // The filter is inside the source and is off from the next sample; the level is the
        // player's, which rodio reads every 5 ms (40 samples here). Past that, it is the dry sound.
        const PLAYER_CONTROL_PERIOD: usize = RATE as usize / 200;
        let after = pull(&mut out, 1_000);
        assert_eq!(after[PLAYER_CONTROL_PERIOD..], reference[1_000 + PLAYER_CONTROL_PERIOD..], "the bus left the water, its sound did not");
    }

    /// A crossfade's new track starts from silence: its first samples are nothing, and it is
    /// heard once the fade has moved.
    #[test]
    fn a_crossfade_starts_its_track_from_silence() {
        let (mut m, mut out) = on_output();
        let id = m.crossfade(None, "tone", 1.0).unwrap();
        assert_eq!(m.mixer().sink_bus(id), Some(Mixer::MUSIC));
        assert!(pull(&mut out, 400).iter().all(|&s| s == 0.0), "the new track came in loud");
        m.tick(0.5);
        assert!(energy(&pull(&mut out, 400)) > 1.0, "and the fade brings it in");
    }

    /// A sound started at a volume plays at it from its first sample: the scene's `AudioSource`
    /// volume used to be written after `play`, with the sound already on the device at unity.
    #[test]
    fn a_sound_plays_at_its_volume_from_its_first_sample() {
        let (mut m, mut out) = on_output();
        m.play_with("tone", &PlayOptions::default().volume(0.25)).unwrap();
        let peak = pull(&mut out, 2_000).iter().fold(0.0f32, |p, s| p.max(s.abs()));
        assert!(peak <= 0.125 + 1e-6 && peak > 0.1, "peak {peak}: the tone is 0.5 at unity");
    }

    /// The output of `m` over `n` samples with a tone on the default bus, after `setup`.
    fn with_tone(n: usize, setup: impl Fn(&mut AudioManager)) -> Vec<f32> {
        let (mut m, mut out) = on_output();
        setup(&mut m);
        m.update();
        m.play_with("tone", &PlayOptions::default()).unwrap();
        pull(&mut out, n)
    }

    /// A send copies what a bus puts out into another, at its level and a frame late (one sample,
    /// mono): the mix is the bus's sound plus half of it again, one sample on.
    #[test]
    fn a_send_copies_a_bus_into_another_a_frame_late() {
        let reference = with_tone(800, |_| {});
        let sent = with_tone(800, |m| m.mixer_mut().set_send(Mixer::DEFAULT_BUS, "copy", 0.5).unwrap());
        let late = |lag: usize| (lag..800).all(|n| (sent[n] - reference[n] - 0.5 * reference[n - lag]).abs() < 1e-5);
        assert!(late(1) || late(0), "the mix is not the bus plus half of it again: {:?} / {:?}", &sent[..6], &reference[..6]);
    }

    /// What is sent comes out under the to-bus's gain and mute, and under the from-bus's: a send is
    /// after the fader.
    #[test]
    fn a_send_comes_out_under_both_buses_gains() {
        let reference = with_tone(800, |_| {});
        let returned_muted = with_tone(800, |m| {
            m.mixer_mut().set_send(Mixer::DEFAULT_BUS, "copy", 0.5).unwrap();
            m.mixer_mut().set_bus_muted("copy", true);
        });
        assert_eq!(returned_muted, reference, "a muted return bus let its send through");
        let source_muted = with_tone(800, |m| {
            m.mixer_mut().set_send(Mixer::DEFAULT_BUS, "copy", 0.5).unwrap();
            m.mixer_mut().set_bus_muted(Mixer::DEFAULT_BUS, true);
        });
        assert!(source_muted.iter().all(|&s| s == 0.0), "a muted bus still sent its sound");
    }

    /// A return bus with a reverb that keeps none of the dry sound adds the space alone: the mix is
    /// the dry sound plus a tail that rings on after it. A bus reverb with nothing sent to it, and a
    /// send taken away, leave the mix as it was, sample for sample.
    #[test]
    fn a_reverb_return_adds_the_space_and_nothing_else() {
        let blip = |m: &mut AudioManager, out: &mut crate::limiter::MasterOut| {
            m.load_samples("blip", &[0.5; 2_000], RATE);
            m.play_with("blip", &PlayOptions::default()).unwrap();
            pull(out, 8_000)
        };
        let (mut m, mut out) = on_output();
        let reference = blip(&mut m, &mut out);
        let end = reference.iter().rposition(|&s| s != 0.0).expect("premise: the blip is heard");

        let (mut m, mut out) = on_output();
        m.set_bus_reverb("hall", Some(reverb::Reverb::hall().with_dry(0.0)));
        m.update();
        let alone = blip(&mut m, &mut out);
        assert_eq!(alone, reference, "a return nobody sends to changed the mix");

        m.mixer_mut().set_send(Mixer::DEFAULT_BUS, "hall", 1.0).unwrap();
        m.update();
        let sent = blip(&mut m, &mut out);
        assert!(energy(&sent[end + 100..]) > 1e-3, "the hall did not ring on after the sound ({})", energy(&sent[end + 100..]));
        // Freeverb's shortest comb is 200 samples at 8 kHz: until then the hall adds nothing, and what
        // is heard is the bus's own sound, once.
        assert_eq!(sent[..150], reference[..150], "the dry sound was not the bus's own, once");

        m.mixer_mut().set_send(Mixer::DEFAULT_BUS, "hall", 0.0).unwrap();
        m.update();
        pull(&mut out, 40_000); // the tail rings down, to some millionths
        let after = blip(&mut m, &mut out);
        let off = after.iter().zip(&reference).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(off < 1e-4, "a send taken away still sent: {off} off");
    }

    /// A bus's reverb is on that bus's sum alone: a sound on another bus is untouched by it.
    #[test]
    fn a_bus_reverb_reaches_that_bus_alone() {
        let reference = with_tone(2_000, |_| {});
        let elsewhere = with_tone(2_000, |m| m.set_bus_reverb(Mixer::MUSIC, Some(reverb::Reverb::cave())));
        assert_eq!(elsewhere, reference, "another bus's reverb reached the default bus");
        let here = with_tone(2_000, |m| m.set_bus_reverb(Mixer::DEFAULT_BUS, Some(reverb::Reverb::cave())));
        assert_ne!(here, reference, "premise: the bus's own reverb is heard");
        assert_eq!(m_bus_reverb(), Some(reverb::Reverb::cave()));
    }

    fn m_bus_reverb() -> Option<reverb::Reverb> {
        let (mut m, _out) = on_output();
        m.set_bus_reverb(Mixer::MUSIC, Some(reverb::Reverb::cave()));
        m.bus_reverb(Mixer::MUSIC)
    }

    /// A sound plays from the first sample its player is pulled for, every sample of it, at its
    /// own rate. rodio built a player's converter when it was connected, from its empty queue's
    /// 48 kHz: this 400-sample sound came out 86 samples late and as 66 (2026-11-26).
    #[test]
    fn a_sound_plays_whole_and_at_its_rate_from_its_first_sample() {
        let ramp: Vec<f32> = (0..400).map(|i| 0.1 + i as f32 * 1e-3).collect();
        for spatial in [false, true] {
            let (mut m, mut out) = on_output();
            m.load_samples("ramp", &ramp, RATE);
            if spatial {
                m.play_3d_with("ramp", [0.0, 0.0, 0.5], [-0.1, 0.0, 0.0], [0.1, 0.0, 0.0], &PlayOptions::default()).unwrap();
            } else {
                m.play_with("ramp", &PlayOptions::default()).unwrap();
            }
            let heard = pull(&mut out, 1_000);
            let (start, end) = (heard.iter().position(|&s| s != 0.0), heard.iter().rposition(|&s| s != 0.0));
            // A spatial sound is stereo, and folding its frames onto this mono output can leave one
            // sample of its last frame after it.
            let last = if spatial { 399..=400 } else { 399..=399 };
            assert!(start == Some(0) && end.is_some_and(|e| last.contains(&e)), "spatial {spatial}: the sound is heard over {start:?}..{end:?}");
            if !spatial {
                // 16-bit on the way through the WAV it was loaded as.
                assert!(heard[..400].iter().zip(&ramp).all(|(h, r)| (h - r).abs() < 1e-4), "the sound is not itself");
            } else {
                let (ratio, asked) = (heard[398] / heard[0], ramp[398] / ramp[0]);
                assert!((ratio / asked - 1.0).abs() < 1e-2, "spatial: the ramp is bent ({ratio}, not {asked})");
            }
        }
    }

    /// A finished sound's environment goes with it, as its route does.
    #[test]
    fn a_dead_sinks_environment_is_collected() {
        let (mut m, mut out) = on_output();
        m.load_samples("blip", &[0.5; 80], RATE);
        m.play("blip").unwrap();
        assert_eq!(m.sink_env.len(), 1);
        pull(&mut out, 400);
        m.update();
        assert_eq!((m.sink_env.len(), m.mixer().routed_sinks()), (0, 0));
    }
}
