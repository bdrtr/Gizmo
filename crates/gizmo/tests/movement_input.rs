//! Movement is read through one function, and this is what says so.
//!
//! # Why a source scan
//!
//! Reading WASD is four `if`s, and four `if`s is a shape everybody writes and nobody reviews.
//! Measured on 2026-08-17, before [`Input::move_axis`] existed: **18 places in this repository**
//! computed a movement direction from the keyboard — 16 demos, `SimpleApp`'s built-in fly camera
//! and the studio's editor camera — and they did not agree. Fourteen accumulated a direction and
//! normalised it; four (`showcase`, `cpu_physics`, `ocean_scene`, `advanced_physics`) added a
//! full-speed step per key and so moved 41 % faster on the diagonal. **Seventeen of the eighteen
//! had no gamepad support at all**, months after the engine grew gamepad support — including the
//! two that are engine code rather than a demo, so every game built on `SimpleApp` had a camera a
//! stick could not fly.
//!
//! A further eight files read movement-*named* keys for something else — throttle, turret aim, a
//! tool mode, a dial. They are in [`EXCEPTIONS`], with what the keys mean there.
//!
//! None of that was a hard problem. It was an *unwatched* one: nothing anywhere could tell that
//! the copies had diverged, so they diverged for as long as they existed. Fixing the nineteen
//! without leaving something that notices the twentieth would buy about as long.
//!
//! So this test scans for the shape rather than trusting the fix to stay applied. It is a
//! **ratchet**: [`EXCEPTIONS`] may shrink and may not grow without a reason written next to the
//! entry.
//!
//! # What it can and cannot see
//!
//! It matches "this file reads a movement key directly", which is a *proxy* for "this file rolls
//! its own movement" — a good one, because reading the keys is the unavoidable first step of doing
//! it by hand, but a proxy. It cannot see a file that gets movement wrong through
//! [`blend_move_axis`], and it cannot see one that reads a key for something that is not movement
//! at all. The latter is what [`EXCEPTIONS`] is for.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The movement keys, spelled the several ways this repository spells them.
///
/// Both `KeyCode::KeyW` and the fully-qualified `gizmo::winit::keyboard::KeyCode::KeyW` appear in
/// the tree, so the needle is the tail rather than the whole path.
const MOVEMENT_KEYS: [&str; 8] = [
    "KeyCode::KeyW",
    "KeyCode::KeyA",
    "KeyCode::KeyS",
    "KeyCode::KeyD",
    "KeyCode::ArrowUp",
    "KeyCode::ArrowDown",
    "KeyCode::ArrowLeft",
    "KeyCode::ArrowRight",
];

/// Files allowed to read a movement key directly, each with what those keys mean there instead.
///
/// Paths are workspace-relative. **This list may shrink; it may not grow without a reason that
/// survives being read aloud.** "It was easier" is not one — that is the shape the ratchet exists
/// to stop.
///
/// Read as a whole, it is also the answer to "what does W mean in this repository": throttle, aim,
/// a tool mode, a dial. Every entry here was arrived at by opening the file, which is the audit
/// that had never happened.
const EXCEPTIONS: &[(&str, &str)] = &[
    (
        "demo/src/bin/car_demo.rs",
        "a vehicle's controls are not a movement vector: W/S are throttle and brake, A/D drive a \
         steering-angle target, and the two axes are independent. Folding them into a unit disc \
         would take away throttle for steering — the correction `move_axis` applies, and exactly \
         the wrong one here.",
    ),
    (
        "demo/src/bin/hill_climb/update.rs",
        "the same, for a two-button hill-climb car: W/D/Up/Right are throttle, S/A/Down/Left are \
         brake. Four keys, two controls, no direction.",
    ),
    (
        "demo/src/bin/yikim.rs",
        "aiming a fixed turret: the keys drive yaw and pitch at a fixed rate and are clamped to \
         the turret's arc. That is a look axis, not a movement one — see the look/aim item in \
         docs/ENGINE.md §3, which is deliberately still open.",
    ),
    (
        "demo/src/bin/yikim_ustasi.rs",
        "the same turret aim, plus a `touched` check that lists the movement keys only to ask \
         whether the player has touched anything at all.",
    ),
    (
        "demo/src/bin/cloth_demo.rs",
        "Up/Down step the cloth's segment COUNT — a discrete parameter with its own repeat \
         cooldown, not a direction.",
    ),
    (
        "demo/src/bin/wind_tunnel.rs",
        "Up/Down are a dial: they raise and lower the wind speed in m/s.",
    ),
    (
        "crates/gizmo-studio/src/systems/shortcuts.rs",
        "editor shortcuts, and the reason the studio camera gates its fly keys behind the right \
         mouse button: W/E/Q are Translate/Rotate/Select, and Ctrl+S/D/A are save, duplicate and \
         select-all.",
    ),
    (
        "crates/gizmo-studio/src/systems/simulation.rs",
        "an `ActionMap` binding table for the fighting-game sample — arrows AND WASD both bound to \
         the logical Up/Down/Left/Right. A fighter's directions are digital by design: they feed \
         the motion buffer, where 'half a tilt' has no meaning.",
    ),
];

/// Where movement is read from. Directories are scanned; a file is taken as itself.
///
/// Scanned rather than listed wherever a directory will do, so demo number 41 is covered the day
/// it is written — a written list of demos is a list that goes stale, and this whole file exists
/// because of things nobody looked at.
const SUBJECTS: [&str; 3] = [
    "demo/src/bin",
    "crates/gizmo/src/simple.rs",
    "crates/gizmo-studio/src/systems",
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/gizmo sits two levels below the workspace root")
        .to_path_buf()
}

fn rust_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().is_some_and(|e| e == "rs") {
            out.push(path.to_path_buf());
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        rust_files(&entry.path(), out);
    }
}

/// Does this line read a movement key, ignoring comments?
fn reads_a_movement_key(line: &str) -> bool {
    let code = line.trim_start();
    if code.starts_with("//") {
        return false;
    }
    MOVEMENT_KEYS.iter().any(|k| code.contains(k))
}

#[test]
fn movement_is_read_through_one_function() {
    let root = workspace();
    let excepted: BTreeSet<&str> = EXCEPTIONS.iter().map(|(path, _)| *path).collect();

    let mut files = Vec::new();
    for subject in SUBJECTS {
        let path = root.join(subject);
        let before = files.len();
        rust_files(&path, &mut files);
        assert!(
            files.len() > before,
            "no sources under {} — the subject list has gone stale, which is the one way a \
             scanner silently passes",
            path.display()
        );
    }
    files.sort();
    // The demo directory alone is around forty files; a scan that suddenly covers three has
    // stopped being a scan.
    assert!(
        files.len() >= 30,
        "only {} sources scanned; expected the whole demo directory and the two engine cameras",
        files.len()
    );

    let mut offenders = Vec::new();
    let mut unused_exceptions: BTreeSet<&str> = excepted.clone();

    for file in &files {
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");

        let text = std::fs::read_to_string(file).unwrap_or_default();
        let hits: Vec<usize> = text
            .lines()
            .enumerate()
            .filter(|(_, l)| reads_a_movement_key(l))
            .map(|(i, _)| i + 1)
            .collect();

        if hits.is_empty() {
            continue;
        }
        if excepted.contains(rel.as_str()) {
            unused_exceptions.remove(rel.as_str());
            continue;
        }
        // A file that goes through the shared blend is allowed to name the keys: feeding
        // `blend_move_axis` its key half means writing the four key codes out, which is exactly
        // what the studio's camera does so that it can gate the keys and leave the stick ungated.
        // This is the rule's real shape — "movement keys are read only where the shared blend is"
        // — and it is weaker than line-level analysis on purpose: a scan that tried to tell a
        // good `KeyCode::KeyW` from a bad one by its neighbours would be the thing that goes
        // wrong silently.
        if text.contains("move_axis(") || text.contains("blend_move_axis(") {
            continue;
        }
        offenders.push(format!(
            "{rel}: reads movement keys at line(s) {} — use `Input::move_axis` (or \
             `blend_move_axis`, if the keys need their own gating) so the stick works and the \
             diagonal is not 41 % faster",
            hits.iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    assert!(
        offenders.is_empty(),
        "movement is being read by hand again:\n  {}",
        offenders.join("\n  ")
    );

    // The other direction: an exception whose file no longer reads those keys is a stale licence,
    // and a ratchet that keeps handing out licences it no longer needs is not ratcheting.
    assert!(
        unused_exceptions.is_empty(),
        "these exceptions are no longer needed — delete them:\n  {}",
        unused_exceptions.into_iter().collect::<Vec<_>>().join("\n  ")
    );
}

/// The subjects have to be places that *do* move something, or the scan above proves nothing.
///
/// A path renamed out from under `SUBJECTS` would leave this file passing over an empty set, and
/// the first assertion only catches a directory that vanished entirely. This one checks that each
/// subject still contains movement code at all — spelled as a call to the function they were all
/// converted to.
#[test]
fn every_subject_still_contains_the_movement_it_is_watching() {
    let root = workspace();
    for subject in SUBJECTS {
        let mut files = Vec::new();
        rust_files(&root.join(subject), &mut files);
        let uses_axis = files.iter().any(|f| {
            let text = std::fs::read_to_string(f).unwrap_or_default();
            text.contains("move_axis") || text.contains("blend_move_axis")
        });
        assert!(
            uses_axis,
            "nothing under {subject} reads movement any more — either it moved somewhere this \
             test is not looking, or `SUBJECTS` is out of date"
        );
    }
}
