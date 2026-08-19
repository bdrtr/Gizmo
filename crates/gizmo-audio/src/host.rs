//! Giving a `World` an audio device and the sounds a scene names.
//!
//! The half of the audio wiring that needs no renderer and no physics: who opens the output
//! device, when, and where `AudioSource::sound_name` is resolved from. The spatial half — the
//! listener, the Doppler shift, the per-frame sink updates — lives in the facade
//! (`gizmo::systems::audio`), because it reads a `Camera` and a `Transform`.
//!
//! It lives here rather than there because **both** of the engine's audio entry points need it
//! and only one of them is spatial: a Lua `audio.play` reaches the manager through the script
//! command path, in builds with no renderer feature at all.

use crate::{AudioManager, AudioSource};
use gizmo_core::World;

/// What a host has already tried, so it does not try again every frame.
///
/// Two latches, one resource, because they fail the same way: opening the output device and
/// reading a sound file are both expensive and both permanent enough that retrying at 60 Hz
/// turns a missing file into a scrolling log and a missing device into a per-frame stall.
#[derive(Default, Debug)]
pub struct AudioLoadState {
    /// Sound names a load has been attempted for, successful or not. Names, not paths: the
    /// manager is keyed by name and that is what a retry would collide with.
    pub attempted: std::collections::HashSet<String>,
    /// `AudioManager::new` failed once — there is no output device on this machine (a CI runner,
    /// a container, a headless server), and there will not be one later in the process.
    pub device_failed: bool,
}

/// The sound names a host still has to read off disk: named by a source, not loaded, not already
/// tried.
///
/// Split out from [`load_scene_sounds`] because it is the whole decision and the rest of that
/// function is a device: the rule that a name the game already loaded is left alone (its bytes
/// win over anything on disk) is one a test can pin without an audio card.
pub fn sounds_to_load(
    named: &[String],
    is_loaded: impl Fn(&str) -> bool,
    attempted: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in named {
        if name.is_empty() || attempted.contains(name) || is_loaded(name) || out.contains(name) {
            continue;
        }
        out.push(name.clone());
    }
    out
}

/// Opens the output device — once, and only for a world that has something to play.
///
/// The condition is deliberate on both sides. **Only when an `AudioSource` exists**, because
/// `AudioManager::new` opens a real device: a game with no audio in it should not hold one, and
/// on some backends holding one is audible (a hiss, a mixer entry, a bluetooth headset that
/// switches profile). **Only once**, because a machine with no device now has none in two
/// frames' time either, and re-probing at 60 Hz is a stall per frame plus a log line per frame.
///
/// A game that built its own manager keeps it: this never replaces one.
pub fn ensure_audio_manager(world: &mut World) {
    if world.get_resource::<AudioManager>().is_some() {
        return;
    }
    if world.borrow::<AudioSource>().entities().next().is_none() {
        return;
    }
    open_audio_device(world);
}

/// Opens the output device for a caller that already knows it needs one, and reports whether the
/// world has a manager afterwards.
///
/// Separate from [`ensure_audio_manager`] because there are two reasons to want a device and only
/// one of them is a component: a script calling `audio.play` is the other, and it can happen in a
/// scene with no `AudioSource` in it at all. The once-only latch lives here, so both callers share
/// it — a machine with no device is asked once per process, not once per caller.
pub fn open_audio_device(world: &mut World) -> bool {
    if world.get_resource::<AudioManager>().is_some() {
        return true;
    }
    if world
        .get_resource::<AudioLoadState>()
        .is_some_and(|s| s.device_failed)
    {
        return false;
    }

    match AudioManager::new() {
        Ok(manager) => {
            log::info!("[Audio] Ses isteyen bir şey var — çıkış cihazı açıldı");
            world.insert_resource(manager);
            true
        }
        Err(e) => {
            log::warn!("[Audio] Çıkış cihazı açılamadı — sahne sessiz kalacak: {}", e);
            world
                .get_resource_mut_or_default::<AudioLoadState>()
                .device_failed = true;
            false
        }
    }
}

/// Reads the sounds a scene's [`AudioSource`]s name, for the ones nothing has loaded.
///
/// `AudioSource::sound_name` is a *registered name* — `play` answers `NotLoaded` for anything
/// else — and registering one meant calling [`AudioManager::load_sound`] from game code. Scene
/// data cannot call anything, so a scene-authored source named a sound that never existed. Here
/// the name is also tried **as a path**, exactly as `MeshSource` is a path: a source saying
/// `demo/assets/audio/engine.wav` now plays that file.
///
/// Two rules, both of them about not taking something over:
///
/// - **A name the game already loaded is left alone.** `load_sound_bytes` from an embedded asset,
///   a wasm `fetch`, a differently-named file — those win over anything on disk with a matching
///   name, because the game said so and the scene only named it.
/// - **One attempt per name.** A missing file is reported once, not once per frame; the failure
///   is latched in [`AudioLoadState`], which is also what stops a 60 Hz retry storm on a path
///   that is a name and nothing else (`"boom"`).
pub fn load_scene_sounds(world: &mut World) {
    let named: Vec<String> = {
        let sources = world.borrow::<AudioSource>();
        sources.iter().map(|(_, s)| s.sound_name.clone()).collect()
    };
    load_named_sounds(world, &named);
}

/// The same rule for names that came from somewhere other than a component — a script's
/// `audio.play("assets/sfx/jump.wav")`, which had exactly the scene's problem: the API existed,
/// the command was applied, and the name had never been registered by anything.
///
/// Takes `&mut World` only to create [`AudioLoadState`] on the first call; the caller must not be
/// holding an `AudioManager` borrow.
pub fn load_named_sounds(world: &mut World, named: &[String]) {
    if world.get_resource::<AudioManager>().is_none() {
        return;
    }
    if world.get_resource::<AudioLoadState>().is_none() {
        world.insert_resource(AudioLoadState::default());
    }

    let Some(mut audio) = world.get_resource_mut::<AudioManager>() else {
        return;
    };
    let Some(mut state) = world.get_resource_mut::<AudioLoadState>() else {
        return;
    };

    for name in sounds_to_load(named, |n| audio.is_loaded(n), &state.attempted) {
        state.attempted.insert(name.clone());
        match audio.load_sound(&name, &name) {
            Ok(()) => log::info!("[Audio] Ses yüklendi: {}", name),
            Err(e) => log::warn!(
                "[Audio] Ses okunamadı, bu isim yalnız oyun kodu yüklerse çalar: {} ({})",
                name,
                e
            ),
        }
    }
}

/// Should this **flat** (non-spatial) source be started this frame?
///
/// The mirror of the spatial system's own guard, and the `has_played` latch means the same thing
/// here: when a one-shot finishes, its sink id is cleared, and without the latch the guard would
/// fire again on the next frame — a sound that repeats forever.
fn should_start_flat(source: &AudioSource) -> bool {
    !source.is_3d && !source.has_played && source._internal_sink_id.is_none()
}

/// Starts the scene's flat sources, and keeps the live ones matching their component.
///
/// A source with `is_3d = false` is the scene's music, its ambience, its UI click — and **nothing
/// started one.** The spatial system's auto-start requires `is_3d`, deliberately (a flat sound has
/// no position to update), so the flag that was meant to select *how* a source is played selected
/// *whether* it was played at all: authoring music into a level made no sound in the editor's ▶
/// and none in an exported game.
///
/// Lives here rather than beside the spatial system because a flat sound needs no camera and no
/// transform: this is the half of scene audio that works in a headless or render-less build.
///
/// Per frame, per flat source: start it if it has not been started; and while it plays, push its
/// `volume` and `pitch` down to the sink, so a script or the inspector turning either of them has
/// an effect on a sound that is already playing — the same promise the spatial path keeps.
pub fn play_flat_sources(world: &mut World) {
    let Some(mut audio) = world.get_resource_mut::<AudioManager>() else {
        return;
    };
    // SAFETY: the only other borrow held here is the `AudioManager` *resource* guard above, which
    // is a different storage entirely, so this view aliases nothing. (The spatial system takes the
    // same view the same way, for the same reason: a resource guard borrows the world immutably,
    // and the checked `borrow_mut` wants `&mut World`.)
    let mut sources = unsafe { world.borrow_mut_unchecked::<AudioSource>() };

    for (_, mut source) in sources.iter_mut() {
        if source.is_3d {
            continue;
        }

        if should_start_flat(&source) {
            let started = if source.loop_sound {
                audio.play_looped(&source.sound_name)
            } else {
                audio.play(&source.sound_name)
            };
            match started {
                Ok(id) => {
                    audio.set_sink_bus(id, &source.bus);
                    audio.set_volume(id, source.volume);
                    audio.set_pitch(id, source.pitch);
                    source._internal_sink_id = Some(id);
                }
                Err(e) => {
                    log::warn!("[Audio] 2D ses çalınamadı '{}': {}", source.sound_name, e);
                    source._internal_sink_id = None;
                }
            }
            // One attempt, whether it worked or not — the same latch the spatial path uses, and
            // the reason a missing sound is one warning rather than sixty a second.
            source.has_played = true;
            continue;
        }

        if let Some(sink) = source._internal_sink_id {
            if !audio.is_playing(sink) {
                if !source.loop_sound {
                    source._internal_sink_id = None;
                }
                continue;
            }
            audio.set_volume(sink, source.volume);
            audio.set_pitch(sink, source.pitch);
        }
    }
}

/// Everything audio does in one frame that needs neither a camera nor a transform.
///
/// The engine's audio entry point for a running game: **this is what nothing called.**
/// `AudioManager` was constructed in exactly three places in the tree — two demos and a wasm demo
/// — and in none of the engine's own hosts, so an `AudioSource` placed in the editor, saved into a
/// scene and carried into an exported game was read by a system that returns on its first line
/// without that resource.
///
/// Called from `PlayLoop::step`, so audio belongs to the *running* game: the editor at rest does
/// not open a device and does not play a scene's ambience over the person editing it. A build with
/// the renderer runs the spatial half (`gizmo::systems::audio::audio_spatial_system`) straight
/// after this one.
pub fn host_frame(world: &mut World) {
    ensure_audio_manager(world);
    load_scene_sounds(world);
    play_flat_sources(world);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flat auto-start latch, without a device — the same shape as the spatial one, and the
    /// same reason: a finished one-shot clears its sink id, so the guard must not fire again.
    #[test]
    fn a_finished_flat_one_shot_does_not_restart() {
        let mut s = AudioSource::new("music");
        s.is_3d = false;
        assert!(should_start_flat(&s), "a fresh flat source starts once");

        s.has_played = true;
        s._internal_sink_id = Some(3);
        assert!(!should_start_flat(&s), "not while it is playing");

        s._internal_sink_id = None; // the one-shot ended
        assert!(!should_start_flat(&s), "and not after it ended");
    }

    /// A 3D source is not this function's business — the spatial system starts those, because it
    /// is the one that knows where they are.
    #[test]
    fn a_spatial_source_is_left_to_the_spatial_system() {
        let s = AudioSource::new("boom"); // is_3d = true
        assert!(!should_start_flat(&s));
    }

    /// The whole flat path on a world with no device: silent, not fatal, and nothing latched on a
    /// source it never tried.
    #[test]
    fn flat_playback_is_inert_without_a_manager() {
        let mut world = World::new();
        let e = world.spawn();
        let mut source = AudioSource::new("music");
        source.is_3d = false;
        world.add_component(e, source);

        play_flat_sources(&mut world);

        let sources = world.borrow::<AudioSource>();
        assert!(!sources.get(e.id()).unwrap().has_played);
    }

    /// **End to end on a real sound card**: a scene's flat source — its music — plays, with no
    /// game code and no camera in the world at all.
    ///
    /// `#[ignore]` because a CI runner has no output device. Run with
    /// `cargo test -p gizmo-audio --lib -- --ignored`.
    #[test]
    #[ignore = "needs an audio output device"]
    fn a_flat_scene_source_plays_with_no_camera_in_the_world() {
        let mut world = World::new();
        let e = world.spawn();
        let mut source = AudioSource::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../demo/assets/audio/engine.wav"
        ));
        source.is_3d = false;
        world.add_component(e, source);
        // The world deliberately has no Transform and no Camera: a flat sound needs neither.

        host_frame(&mut world);

        let manager = world
            .get_resource::<AudioManager>()
            .expect("a scene with a source opens the device");
        let sources = world.borrow::<AudioSource>();
        let sink = sources
            .get(e.id())
            .unwrap()
            ._internal_sink_id
            .expect("the flat source must have been started");
        assert!(manager.is_playing(sink));
    }
}
