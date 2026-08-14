//! Key **names** for the desktop key-code convention, in one place.
//!
//! # Why this exists
//!
//! [`Input`](super::Input) stores opaque `u32` key codes and this crate has no windowing
//! dependency, so the mapping from a physical key to a code is the caller's convention — see the
//! module docs. On desktop that convention is `winit::keyboard::KeyCode as u32`, and it is not
//! optional in practice: `gizmo-app` forwards `PhysicalKey::Code(k) => k as u32` for every key, so
//! every code that ever reaches `Input` on desktop is a winit discriminant.
//!
//! Anything that wants to say "the W key" without depending on winit therefore needs this table,
//! and until 2026-08-15 the Lua scripting API carried its own copy. That copy held **USB HID usage
//! codes** — `w = 17`, `a = 4`, `space = 44` — which is a different, equally real numbering that
//! winit does not use. Every entry was wrong, and two of them were wrong in the way that hurts
//! most: `down = 81` and `right = 79` are winit's ArrowRight and ArrowDown, so a script reading
//! the arrow keys moved right when the player pressed down.
//!
//! The numbers here are generated from winit's `KeyCode` declaration order, and
//! `gizmo-app` — the crate that actually has winit — carries the test that proves every one of
//! them, with no entry allowed to go unverified. That is the arrangement that makes this table
//! trustworthy from a crate that cannot see winit at all.

/// Every name this engine answers to, with its desktop key code.
///
/// Lower-case, ASCII. Sorted by nothing in particular — look up with [`code_from_name`].
pub const NAMED_KEYS: &[(&str, u32)] = &[
    ("a", 19), // KeyCode::KeyA
    ("b", 20), // KeyCode::KeyB
    ("c", 21), // KeyCode::KeyC
    ("d", 22), // KeyCode::KeyD
    ("e", 23), // KeyCode::KeyE
    ("f", 24), // KeyCode::KeyF
    ("g", 25), // KeyCode::KeyG
    ("h", 26), // KeyCode::KeyH
    ("i", 27), // KeyCode::KeyI
    ("j", 28), // KeyCode::KeyJ
    ("k", 29), // KeyCode::KeyK
    ("l", 30), // KeyCode::KeyL
    ("m", 31), // KeyCode::KeyM
    ("n", 32), // KeyCode::KeyN
    ("o", 33), // KeyCode::KeyO
    ("p", 34), // KeyCode::KeyP
    ("q", 35), // KeyCode::KeyQ
    ("r", 36), // KeyCode::KeyR
    ("s", 37), // KeyCode::KeyS
    ("t", 38), // KeyCode::KeyT
    ("u", 39), // KeyCode::KeyU
    ("v", 40), // KeyCode::KeyV
    ("w", 41), // KeyCode::KeyW
    ("x", 42), // KeyCode::KeyX
    ("y", 43), // KeyCode::KeyY
    ("z", 44), // KeyCode::KeyZ
    ("0", 5), // KeyCode::Digit0
    ("1", 6), // KeyCode::Digit1
    ("2", 7), // KeyCode::Digit2
    ("3", 8), // KeyCode::Digit3
    ("4", 9), // KeyCode::Digit4
    ("5", 10), // KeyCode::Digit5
    ("6", 11), // KeyCode::Digit6
    ("7", 12), // KeyCode::Digit7
    ("8", 13), // KeyCode::Digit8
    ("9", 14), // KeyCode::Digit9
    ("space", 62), // KeyCode::Space
    ("tab", 63), // KeyCode::Tab
    ("escape", 114), // KeyCode::Escape
    ("enter", 57), // KeyCode::Enter
    ("backspace", 52), // KeyCode::Backspace
    ("delete", 72), // KeyCode::Delete
    ("lshift", 60), // KeyCode::ShiftLeft
    ("rshift", 61), // KeyCode::ShiftRight
    ("lctrl", 55), // KeyCode::ControlLeft
    ("rctrl", 56), // KeyCode::ControlRight
    ("lalt", 50), // KeyCode::AltLeft
    ("ralt", 51), // KeyCode::AltRight
    ("up", 82), // KeyCode::ArrowUp
    ("down", 79), // KeyCode::ArrowDown
    ("left", 80), // KeyCode::ArrowLeft
    ("right", 81), // KeyCode::ArrowRight
    ("f1", 159), // KeyCode::F1
    ("f2", 160), // KeyCode::F2
    ("f3", 161), // KeyCode::F3
    ("f4", 162), // KeyCode::F4
    ("f5", 163), // KeyCode::F5
    ("f6", 164), // KeyCode::F6
    ("f7", 165), // KeyCode::F7
    ("f8", 166), // KeyCode::F8
    ("f9", 167), // KeyCode::F9
    ("f10", 168), // KeyCode::F10
    ("f11", 169), // KeyCode::F11
    ("f12", 170), // KeyCode::F12
];

/// The desktop key code for a key name, case-insensitively. `None` for a name not in
/// [`NAMED_KEYS`].
///
/// Linear over ~64 entries: this is called once per script key query, against a table that fits
/// in a cache line's worth of cache lines, and a map would cost more to build than it saves.
#[must_use]
pub fn code_from_name(name: &str) -> Option<u32> {
    NAMED_KEYS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, code)| *code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_lower_case() {
        let mut seen = std::collections::HashSet::new();
        for (name, _) in NAMED_KEYS {
            assert!(seen.insert(*name), "`{name}` is listed twice");
            assert_eq!(*name, name.to_ascii_lowercase(), "`{name}` must be lower-case");
        }
    }

    /// Two names may not share a code: that is what a transcribed table gets wrong, and it is the
    /// half of correctness that can be checked without winit. The other half — that each code is
    /// the RIGHT one — lives in `gizmo-app`, which can compare against the enum itself.
    #[test]
    fn codes_are_unique() {
        let mut seen = std::collections::HashMap::new();
        for (name, code) in NAMED_KEYS {
            if let Some(other) = seen.insert(*code, *name) {
                panic!("`{name}` and `{other}` both map to {code}");
            }
        }
    }

    #[test]
    fn lookup_is_case_insensitive_and_total_over_the_table() {
        for (name, code) in NAMED_KEYS {
            assert_eq!(code_from_name(name), Some(*code));
            assert_eq!(code_from_name(&name.to_ascii_uppercase()), Some(*code));
        }
        assert_eq!(code_from_name("no-such-key"), None);
    }

    /// The arrow keys, pinned by name because this is where the old scripting table was not just
    /// wrong but *swapped*: it had down = 81 and right = 79, which are each other's codes.
    #[test]
    fn the_arrows_are_not_swapped() {
        assert_eq!(code_from_name("up"), Some(82));
        assert_eq!(code_from_name("down"), Some(79));
        assert_eq!(code_from_name("left"), Some(80));
        assert_eq!(code_from_name("right"), Some(81));
    }
}
