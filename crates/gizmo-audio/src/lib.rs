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
//!
//! Sounds are decoded from in-memory byte buffers (loaded once via
//! [`AudioManager::load_sound`]) to avoid per-play disk I/O. Spatial playback
//! tracks emitter and listener (ear) positions and attenuates volume by
//! distance. No `rodio` types appear in the public API, keeping the dependency
//! contract internal.

mod filter;
mod mixer;
pub use mixer::Mixer;

use filter::{Muffle, MuffleControl};

use rodio::stream::DeviceSinkBuilder;
use rodio::{Decoder, MixerDeviceSink, Player, Source, SpatialPlayer};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;
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
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::Io(err) => write!(f, "IO Error: {}", err),
            AudioError::NotFound(path) => write!(f, "File not found: {}", path),
            AudioError::Backend(msg) => write!(f, "Audio backend error: {}", msg),
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

impl AudioSource {
    /// Creates a new [`AudioSource`] for the sound with the given name.
    pub fn new(name: &str) -> Self {
        Self {
            sound_name: name.to_string(),
            is_3d: true,
            volume: 1.0,
            pitch: 1.0,
            loop_sound: false,
            max_distance: 100.0, // Varsayılan değer
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

// ======================== AUDIO MANAGER ========================

/// Resource that owns the audio output device and manages loaded sounds and
/// active playback sinks (both global and 3D spatial).
pub struct AudioManager {
    // The device sink owns the live `cpal::Stream` — dropping it stops all audio — and hands out
    // the mixer that every player connects to. (rodio 0.22 merged the old `OutputStream` +
    // `OutputStreamHandle` pair into this one value; "sink" in *our* vocabulary still means one
    // playing sound, which rodio now calls a `Player`.)
    device_sink: MixerDeviceSink,

    // RAM'e (Memory) yüklenmiş ses dosyaları (Disk I/O darboğazını önler)
    sound_buffers: HashMap<String, Arc<[u8]>>,

    // Aktif uzamsal/normal çalıcıları takip edip parametrelerini güncellemek için
    active_spatial_sinks: HashMap<u64, SpatialPlayer>,
    active_sinks: HashMap<u64, Player>,
    // Which loaded sound each live sink was started from. A sink id is what the engine holds, but
    // a NAME is what a script has: `audio.stop("music")` cannot mean anything else.
    sink_sounds: HashMap<u64, String>,
    next_sink_id: u64,
    // The last value [`AudioManager::set_all_paused`] was asked for, so a host can call it every
    // frame and only a CHANGE reaches the sinks — otherwise a game pausing one sound of its own
    // would have it resumed on the next frame by a host that is not paused.
    all_paused: bool,

    // The live low-pass cutoff, shared with every source this manager has started and read on
    // the audio thread. Turning the muffle on is one atomic store here, not a walk over the sinks
    // — the filter is *inside* each source, which is the only place rodio lets it be live.
    muffle: std::sync::Arc<MuffleControl>,

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
                log::info!("Gizmo Audio: Ses cihazı başlatıldı! 3D Uzamsal (Spatial) Motor Aktif.");
                Ok(Self {
                    device_sink,
                    sound_buffers: HashMap::new(),
                    active_spatial_sinks: HashMap::new(),
                    active_sinks: HashMap::new(),
                    sink_sounds: HashMap::new(),
                    next_sink_id: 1,
                    all_paused: false,
                    muffle: MuffleControl::new(),
                    mixer: Mixer::new(),
                })
            }
            Err(e) => {
                log::error!("Gizmo Audio Başarısız (Cihaz bulunamadı): {}", e);
                Err(AudioError::Backend(e.to_string()))
            }
        }
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

    /// The buses, the master gain and the environment — read-only. See [`Mixer`].
    #[inline]
    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    /// The mixer, to turn a gain on. Changes are heard on the next [`AudioManager::update`],
    /// which every frame calls; use [`AudioManager::set_sink_bus`] and the `set_*` methods here
    /// when a change must be audible before then.
    #[inline]
    pub fn mixer_mut(&mut self) -> &mut Mixer {
        &mut self.mixer
    }

    /// Moves a live sound to a mixer bus and re-prices it immediately.
    ///
    /// This is how a sound reaches a bus other than [`Mixer::DEFAULT_BUS`]:
    ///
    /// ```no_run
    /// # use gizmo_audio::{AudioManager, Mixer};
    /// # let mut audio = AudioManager::new().unwrap();
    /// let id = audio.play_looped("theme")?;
    /// audio.set_sink_bus(id, Mixer::MUSIC);   // now follows the player's music slider
    /// # Ok::<(), gizmo_audio::AudioError>(())
    /// ```
    ///
    /// Starting a sound *already* on its bus would need a second set of `play` entry points; the
    /// trigger for adding them is a game that can hear the gap between these two calls — the sound
    /// is priced at the default bus's gain for as long as it takes to reach the next line, which
    /// is one mixer buffer at worst and inaudible unless that bus is muted.
    pub fn set_sink_bus(&mut self, id: u64, bus: &str) {
        self.mixer.set_sink_bus(id, bus);
        self.apply_mix(id);
    }

    /// Writes this sink's composed volume and speed to rodio. Every mutator ends here.
    fn apply_mix(&self, id: u64) {
        let volume = self.mixer.volume_for(id);
        let speed = self.mixer.speed_for(id);
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            sink.set_volume(volume);
            sink.set_speed(speed);
        } else if let Some(sink) = self.active_sinks.get(&id) {
            sink.set_volume(volume);
            sink.set_speed(speed);
        }
    }

    /// Re-prices every live sink — what a master/bus/environment change means, since those apply
    /// to sounds that are already playing.
    fn apply_mix_to_every_sink(&mut self) {
        self.mixer.take_dirty(); // whatever asked for this, it is answered now
        // One store, heard on the next buffer, by every sound at once — including the ones this
        // manager will start later, since they are handed the same control.
        self.muffle.set_cutoff_hz(self.mixer.environment_cutoff_hz());

        for (id, sink) in &self.active_sinks {
            sink.set_volume(self.mixer.volume_for(*id));
            sink.set_speed(self.mixer.speed_for(*id));
        }
        for (id, sink) in &self.active_spatial_sinks {
            sink.set_volume(self.mixer.volume_for(*id));
            sink.set_speed(self.mixer.speed_for(*id));
        }
    }

    // ── Su-altı ses boğma (underwater muffle) ────────────────────────────────
    /// Turns the underwater "muffle" on or off: every sound is turned down and slowed slightly,
    /// which stands in for a low-pass rodio's `Player` cannot do live.
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

    /// Plays a normal (Global/Stereo) sound (one-shot)
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NotLoaded`] if `name` was never loaded, or
    /// [`AudioError::Decode`] if the bytes cannot be decoded. Attaching the sound to the output
    /// device cannot fail here — the device was opened once, in [`AudioManager::new`].
    pub fn play(&mut self, name: &str) -> Result<u64, AudioError> {
        self.play_internal(name, false)
    }

    /// Plays a normal (Global/Stereo) sound in a loop
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NotLoaded`] if `name` was never loaded, or
    /// [`AudioError::Decode`] if the bytes cannot be decoded. Attaching the sound to the output
    /// device cannot fail here — the device was opened once, in [`AudioManager::new`].
    pub fn play_looped(&mut self, name: &str) -> Result<u64, AudioError> {
        self.play_internal(name, true)
    }

    fn play_internal(&mut self, name: &str, looped: bool) -> Result<u64, AudioError> {
        let bytes = self.sound_buffers.get(name).ok_or_else(|| {
            log::error!("AudioManager: '{}' adlı ses bellekte yok!", name);
            AudioError::NotLoaded(name.to_string())
        })?;
        let cursor = Cursor::new(Arc::clone(bytes));
        let decoder = Decoder::new(cursor).map_err(|e| AudioError::Decode(e.to_string()))?;
        let sink = Player::connect_new(self.device_sink.mixer());
        // The muffle wraps the source rather than the player: a `Player` hands out no way to reach
        // what it is playing, so a filter that must be switchable later has to be in the chain
        // before `append` and read its own parameter afterwards.
        if looped {
            sink.append(Muffle::new(decoder.repeat_infinite(), Arc::clone(&self.muffle)));
        } else {
            sink.append(Muffle::new(decoder, Arc::clone(&self.muffle)));
        }
        let id = self.next_sink_id;
        self.next_sink_id = self.next_sink_id.wrapping_add(1);

        // Routed before it is priced, priced before it is returned: a sound that starts while the
        // listener is underwater, or on a bus the player has turned down, is already at the right
        // volume on its first buffer.
        self.active_sinks.insert(id, sink);
        self.sink_sounds.insert(id, name.to_string());
        self.mixer.route(id, Mixer::DEFAULT_BUS);
        self.apply_mix(id);
        Ok(id)
    }

    /// Plays a 3D Spatial sound (one-shot)
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
        self.play_3d_internal(name, emitter_pos, left_ear, right_ear, false)
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
        self.play_3d_internal(name, emitter_pos, left_ear, right_ear, true)
    }

    fn play_3d_internal(
        &mut self,
        name: &str,
        emitter_pos: [f32; 3],
        left_ear: [f32; 3],
        right_ear: [f32; 3],
        looped: bool,
    ) -> Result<u64, AudioError> {
        let bytes = self.sound_buffers.get(name).ok_or_else(|| {
            log::error!("AudioManager: '{}' adlı 3D ses bellekte yok!", name);
            AudioError::NotLoaded(name.to_string())
        })?;
        let cursor = Cursor::new(Arc::clone(bytes));
        let decoder = Decoder::new(cursor).map_err(|e| AudioError::Decode(e.to_string()))?;
        let sink =
            SpatialPlayer::connect_new(self.device_sink.mixer(), emitter_pos, left_ear, right_ear);
        if looped {
            sink.append(Muffle::new(decoder.repeat_infinite(), Arc::clone(&self.muffle)));
        } else {
            sink.append(Muffle::new(decoder, Arc::clone(&self.muffle)));
        }

        let id = self.next_sink_id;
        self.next_sink_id = self.next_sink_id.wrapping_add(1);

        self.active_spatial_sinks.insert(id, sink);
        self.sink_sounds.insert(id, name.to_string());
        self.mixer.route(id, Mixer::DEFAULT_BUS);
        self.apply_mix(id);
        Ok(id)
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

        let listener_pos = [
            (left_ear[0] + right_ear[0]) / 2.0,
            (left_ear[1] + right_ear[1]) / 2.0,
            (left_ear[2] + right_ear[2]) / 2.0,
        ];
        let dx = emitter_pos[0] - listener_pos[0];
        let dy = emitter_pos[1] - listener_pos[1];
        let dz = emitter_pos[2] - listener_pos[2];
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();

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
        assert!(mixer.environment_cutoff_hz() > 0, "the corner is what carries the muffle now");

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
        assert!(m.muffle.cutoff_hz() > 0, "and the filter every source reads is engaged");

        // One frame of `audio_spatial_system` for a live 3D source: attenuation, then Doppler.
        // This is the write that used to erase the muffle — for EVERY 3D sound, one frame after
        // it was asked for.
        m.update_spatial_sink(id, [0.0, 0.0, 0.0], left, right, 100.0, 1.0);
        m.set_pitch(id, 1.0);
        assert_eq!(spatial(&m, id), (0.4, 1.0), "and it survives the frame that restates it");

        m.set_underwater(false);
        assert_eq!(spatial(&m, id), (1.0, 1.0), "surfacing is not a 2.5x amplifier");
        assert_eq!(m.muffle.cutoff_hz(), 0, "and the filter goes back to bypass, not to 20 kHz");

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
        m.set_sink_bus(g, Mixer::MUSIC);
        m.mixer_mut().set_bus_gain(Mixer::MUSIC, 0.5);
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
