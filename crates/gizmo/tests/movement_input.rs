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
        "crates/gizmo-app/src/windowed/event.rs",
        "the platform layer's logical-key fallback: it PRODUCES key codes from winit's logical \
         keys for layouts where the physical code is absent. It is the far side of the boundary \
         from anything that reads movement.",
    ),
    (
        "demo/src/bin/bevy_keyboard_input.rs",
        "the key is the SUBJECT, not a direction: this port of Bevy's `input/keyboard_input` \
         watches one key (A, as the original does) to show the three edges apart — pressed, \
         just-pressed, just-released — and nothing in it moves. Routing it through `move_axis` \
         would blend the very edges the demo exists to separate, and the stick has nothing to \
         contribute to a key-state readout.",
    ),
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

/// Where movement is read from: **every crate's `src`, and the demo crate's**.
///
/// The first version of this list named three places — `demo/src/bin`, `SimpleApp` and the
/// studio's camera — and it was wrong within the hour. It missed
/// `crates/gizmo/src/systems/fps_look.rs`, the engine's own first-person controller, whose module
/// doc opens by complaining that "the demos write it out BY HAND every frame"; and it missed
/// `demo/src/main.rs`, which had the 41 % diagonal. A *written* subject list is exactly the thing
/// §8 of docs/ENGINE.md says not to write, and it failed in the documented way: silently, by
/// covering less than it looked like it covered.
///
/// So the subjects are now scanned. `crates/*/src` picks up any crate, present or future; the
/// `tests/` directories are deliberately outside it, because a test that lists key codes to prove
/// them (`gizmo-app`'s `key_convention.rs`, and this file) is not movement code.
fn subjects(root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.join("demo/src")];
    for entry in std::fs::read_dir(root.join("crates"))
        .expect("crates/ is readable")
        .flatten()
    {
        let src = entry.path().join("src");
        if src.is_dir() {
            dirs.push(src);
        }
    }
    dirs.sort();
    dirs
}

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

/// Does this line read a movement key?
///
/// Comments are cut first, **including trailing ones** — `gizmo-core`'s `NAMED_KEYS` table is a
/// column of `("a", 19), // KeyCode::KeyA`, and counting those would have put the key table itself
/// on the offender list.
fn reads_a_movement_key(line: &str) -> bool {
    let code = match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    };
    MOVEMENT_KEYS.iter().any(|k| code.contains(k))
}

#[test]
fn movement_is_read_through_one_function() {
    let root = workspace();
    let excepted: BTreeSet<&str> = EXCEPTIONS.iter().map(|(path, _)| *path).collect();

    let mut files = Vec::new();
    for path in subjects(&root) {
        let before = files.len();
        rust_files(&path, &mut files);
        assert!(
            files.len() > before,
            "no sources under {} — a subject that scans to nothing is the one way a scanner \
             silently passes",
            path.display()
        );
    }
    files.sort();
    // Twenty crates plus forty demos: a scan that suddenly covers a handful has stopped being one.
    assert!(
        files.len() >= 300,
        "only {} sources scanned; expected every crate's src and the demo crate's",
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

/// The scan finds offenders; this finds *deserters*.
///
/// A file that stops calling the shared blend and stops reading movement keys — because someone
/// rewrote its camera against `ActionMap`, or moved it — passes the scan silently, since the scan
/// can only see what is there. These four are the movers that matter most, checked by name.
#[test]
fn the_known_movers_still_go_through_the_shared_blend() {
    let root = workspace();
    // The places that were converted on 2026-08-17. Named on purpose, unlike the scan above: the
    // scan's job is to find offenders anywhere, and this one's is to notice if a *known* mover
    // quietly stops going through the shared blend — by being deleted, moved, or rewritten. A
    // scan cannot ask that question, because it cannot tell "no movement here" from "no file
    // here".
    const KNOWN_MOVERS: [&str; 4] = [
        "demo/src/bin/platformer.rs",
        "demo/src/main.rs",
        "crates/gizmo/src/simple.rs",
        "crates/gizmo/src/systems/fps_look.rs",
    ];
    for mover in KNOWN_MOVERS {
        let path = root.join(mover);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{mover} is gone or unreadable ({e}) — if it moved, move \
                                        this entry with it"));
        assert!(
            text.contains("move_axis(") || text.contains("blend_move_axis("),
            "{mover} no longer reads movement through the shared blend"
        );
    }
}
