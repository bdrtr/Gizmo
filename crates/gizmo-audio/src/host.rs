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
