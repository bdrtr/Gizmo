//! The desktop key-code convention, checked against the crate that defines it.
//!
//! `gizmo-core` publishes key NAMES with `winit::keyboard::KeyCode as u32` codes and cannot see
//! winit to prove them — that is the whole reason it has no windowing dependency. This crate can,
//! and it is the same crate that forwards `PhysicalKey::Code(k) => k as u32` into `Input`, so the
//! convention this test pins is the one the engine actually receives.
//!
//! It exists because that table was transcribed by hand once already, in the Lua scripting API,
//! from USB HID usage codes — a different and equally real numbering. Every entry was wrong, and
//! `down`/`right` were each other's codes, so a script reading the arrow keys moved the player
//! right when they pressed down. Nothing caught it because nothing compared the two numberings.

use gizmo_core::input::{code_from_name, NAMED_KEYS};
use winit::keyboard::KeyCode;

/// Every name in the table, next to the winit variant it claims to be.
///
/// A second, independent list on purpose: its job is to disagree. The coverage assertion below is
/// what keeps it honest — an entry added to `NAMED_KEYS` and not to this list fails the test
/// rather than slipping through unverified.
const EXPECTED: &[(&str, KeyCode)] = &[
        ("a", KeyCode::KeyA),
        ("b", KeyCode::KeyB),
        ("c", KeyCode::KeyC),
        ("d", KeyCode::KeyD),
        ("e", KeyCode::KeyE),
        ("f", KeyCode::KeyF),
        ("g", KeyCode::KeyG),
        ("h", KeyCode::KeyH),
        ("i", KeyCode::KeyI),
        ("j", KeyCode::KeyJ),
        ("k", KeyCode::KeyK),
        ("l", KeyCode::KeyL),
        ("m", KeyCode::KeyM),
        ("n", KeyCode::KeyN),
        ("o", KeyCode::KeyO),
        ("p", KeyCode::KeyP),
        ("q", KeyCode::KeyQ),
        ("r", KeyCode::KeyR),
        ("s", KeyCode::KeyS),
        ("t", KeyCode::KeyT),
        ("u", KeyCode::KeyU),
        ("v", KeyCode::KeyV),
        ("w", KeyCode::KeyW),
        ("x", KeyCode::KeyX),
        ("y", KeyCode::KeyY),
        ("z", KeyCode::KeyZ),
        ("0", KeyCode::Digit0),
        ("1", KeyCode::Digit1),
        ("2", KeyCode::Digit2),
        ("3", KeyCode::Digit3),
        ("4", KeyCode::Digit4),
        ("5", KeyCode::Digit5),
        ("6", KeyCode::Digit6),
        ("7", KeyCode::Digit7),
        ("8", KeyCode::Digit8),
        ("9", KeyCode::Digit9),
        ("space", KeyCode::Space),
        ("tab", KeyCode::Tab),
        ("escape", KeyCode::Escape),
        ("enter", KeyCode::Enter),
        ("backspace", KeyCode::Backspace),
        ("delete", KeyCode::Delete),
        ("lshift", KeyCode::ShiftLeft),
        ("rshift", KeyCode::ShiftRight),
        ("lctrl", KeyCode::ControlLeft),
        ("rctrl", KeyCode::ControlRight),
        ("lalt", KeyCode::AltLeft),
        ("ralt", KeyCode::AltRight),
        ("up", KeyCode::ArrowUp),
        ("down", KeyCode::ArrowDown),
        ("left", KeyCode::ArrowLeft),
        ("right", KeyCode::ArrowRight),
        ("f1", KeyCode::F1),
        ("f2", KeyCode::F2),
        ("f3", KeyCode::F3),
        ("f4", KeyCode::F4),
        ("f5", KeyCode::F5),
        ("f6", KeyCode::F6),
        ("f7", KeyCode::F7),
        ("f8", KeyCode::F8),
        ("f9", KeyCode::F9),
        ("f10", KeyCode::F10),
        ("f11", KeyCode::F11),
        ("f12", KeyCode::F12),
];

#[test]
fn every_named_key_is_the_winit_code_it_claims_to_be() {
    for (name, key) in EXPECTED {
        assert_eq!(
            code_from_name(name),
            Some(*key as u32),
            "`{name}` disagrees with {:?}: the desktop key convention is `KeyCode as u32`",
            key
        );
    }
}

#[test]
fn no_entry_escapes_verification() {
    for (name, _) in NAMED_KEYS {
        assert!(
            EXPECTED.iter().any(|(n, _)| n == name),
            "`{name}` is in gizmo-core's table and not in this test's — add it here with its \
             winit variant, or the table grows a row nothing has checked"
        );
    }
    assert_eq!(EXPECTED.len(), NAMED_KEYS.len(), "the two lists must cover the same names");
}

/// The specific pair the old scripting table had swapped, kept as its own test so a regression
/// reads as what it is rather than as one line in a loop of sixty.
#[test]
fn down_and_right_are_not_each_others_codes() {
    assert_eq!(code_from_name("down"), Some(KeyCode::ArrowDown as u32));
    assert_eq!(code_from_name("right"), Some(KeyCode::ArrowRight as u32));
    assert_ne!(code_from_name("down"), code_from_name("right"));
}
