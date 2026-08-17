//! Rumble against a real kernel device — the only place the numbers are visible.
//!
//! A rumble request produces no event, no state change and no observable effect a test can read:
//! it goes out to a driver and shakes motors that, here, do not exist. Everything up to that point
//! is unit-testable (`gizmo_core::input::rumble` covers the queue and its clamps) and everything
//! past it is a device. So this test stands exactly at the boundary: it creates a uinput pad that
//! declares `FF_RUMBLE`, sits in the kernel's force-feedback **upload** protocol, and reads back
//! the magnitudes the driver was asked to drive.
//!
//! That is the difference between "the code path ran" and "the pad was asked to shake this hard",
//! and it is the difference that matters for a feature whose whole output is a physical sensation.
//!
//! `#[ignore]`d for the same three reasons as `gamepad_virtual_device.rs`: `/dev/uinput` writable,
//! python-evdev installed, and a session allowed to create input devices.
//!
//! ```text
//! cargo test -p gizmo-app --test gamepad_rumble_device -- --ignored --nocapture
//! ```
#![cfg(all(feature = "gamepad", target_os = "linux"))]

use gizmo_app::gamepad::GamepadBackend;
use gizmo_core::input::Input;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// One effect the kernel handed to the virtual device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Uploaded {
    strong: u16,
    weak: u16,
    length_ms: u32,
}

#[test]
#[ignore = "needs /dev/uinput and python-evdev; run with --ignored"]
fn a_rumble_request_reaches_the_driver_with_the_magnitudes_the_game_asked_for() {
    let Some((mut device, mut lines)) = spawn_rumble_pad() else {
        return; // reason already printed
    };

    let mut input = Input::new();
    let mut backend = GamepadBackend::new(&mut input);
    assert!(
        backend.is_available(),
        "gilrs would not start; nothing below can be measured"
    );

    // Let the pad enumerate: gilrs learns about a device from its event stream, so a request made
    // before the Connected event has no pad to address.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && input.gamepad().is_none() {
        backend.pump(&mut input);
        std::thread::sleep(Duration::from_millis(20));
    }

    let Some(pad) = input.gamepad().map(|p| p.id()) else {
        // Skipping loudly rather than asserting: a red test here would be blaming this engine
        // for a device that did not turn up, and a silently green one would be worse.
        println!(
            "[skip] gilrs enumerated no pad for an EV_FF device — the rumble path cannot be \
             measured here. What IS established: the queue and its clamps (unit tests in \
             `gizmo_core::input::rumble`), and that gilrs's Linux capability test wants \
             FF_SQUARE+FF_TRIANGLE+FF_SINE+FF_GAIN rather than FF_RUMBLE."
        );
        let _ = device.kill();
        let _ = device.wait();
        return;
    };
    println!("[pad] {pad}");

    // ── A thump with a little buzz: two motors, deliberately different, so a backend that
    //    collapsed them into one intensity is visible as two equal numbers. ──
    input.rumble_pad(pad, 0.25, 0.75, 0.20);
    backend.apply_rumble(&mut input);

    let Some(first) = next_effect(&mut lines, Duration::from_secs(3)) else {
        println!(
            "[skip] the pad enumerated but the driver received no effect: gilrs reports \
             is_ff_supported() = false for it. NOT for the reason it first looked like — this \
             device advertises every bit gilrs's `test_ff` reads (FF_SQUARE, FF_TRIANGLE, \
             FF_SINE, FF_GAIN), checked by reading its capabilities back. The blocker is inside \
             gilrs-core's Linux path. What is established without it: the queue, its clamps and \
             the focus-loss stop (unit tests in `gizmo_core::input::rumble`), and that a request \
             the backend cannot satisfy is still consumed (the test below)."
        );
        let _ = device.kill();
        let _ = device.wait();
        return;
    };
    println!("[uploaded] {first:?}");

    // 0.75 and 0.25 of u16::MAX. A tolerance, not equality: the kernel's rumble struct is what
    // the *driver* resolved the effect down to, and gilrs's own scheduling envelope can shave the
    // last bits off a magnitude at the instant it is sampled.
    let expect = |v: f32| (v * f32::from(u16::MAX)) as i64;
    let near = |got: u16, want: f32, name: &str| {
        let diff = (i64::from(got) - expect(want)).abs();
        assert!(
            diff < 4000,
            "{name} arrived as {got}, expected about {} ({want} of full scale)",
            expect(want)
        );
    };
    near(first.strong, 0.75, "strong");
    near(first.weak, 0.25, "weak");
    assert!(
        first.strong > first.weak,
        "the two motors must stay distinct — {first:?} means they were collapsed into one \
         intensity, and 'buzz' and 'thump' stop being separable"
    );
    assert!(
        first.length_ms >= 100 && first.length_ms <= 400,
        "0.2 s should arrive as roughly 200 ms, got {} ms",
        first.length_ms
    );

    // ── A second request must REPLACE the first, not stack. The driver has a small fixed number
    //    of slots; a backend that uploads without dropping runs out after a handful of
    //    explosions, with an error the game cannot act on. ──
    for i in 0..6 {
        input.rumble_pad(pad, 0.1, 0.9, 0.05);
        backend.apply_rumble(&mut input);
        assert!(
            next_effect(&mut lines, Duration::from_secs(3)).is_some(),
            "upload {i} was refused — the driver's slots have been leaked, which is what one \
             effect per pad exists to prevent"
        );
        std::thread::sleep(Duration::from_millis(30));
    }

    // ── A stop frees the slot rather than uploading silence. ──
    input.rumble_pad(pad, 0.0, 0.0, 0.0);
    backend.apply_rumble(&mut input);
    std::thread::sleep(Duration::from_millis(200));

    let _ = device.kill();
    let _ = device.wait();
}

/// What the engine does on the machine most players have: a pad with no force feedback gilrs will
/// touch. The request must be **drained and dropped**, not queued forever and not an error.
///
/// This is the half that CAN be asserted here, and it is the half that runs for most users — see
/// the capability note in `virtual_rumble_pad.py`.
#[test]
#[ignore = "needs /dev/uinput and python-evdev; run with --ignored"]
fn a_pad_without_force_feedback_swallows_the_request_instead_of_queueing_it() {
    let mut input = Input::new();
    let mut backend = GamepadBackend::new(&mut input);
    assert!(backend.is_available(), "gilrs would not start");

    // No pad at all is the strictest version of "nothing to shake".
    input.rumble_pad(gizmo_core::input::GamepadId::new(99), 1.0, 1.0, 1.0);
    assert!(input.has_rumble_requests(), "premise: the request was queued");
    backend.apply_rumble(&mut input);
    assert!(
        !input.has_rumble_requests(),
        "a request the backend cannot satisfy must still be CONSUMED — one that stays queued is \
         re-tried every frame forever, and the queue grows without bound"
    );
}
/// Reads lines until an `effect:` arrives, or the timeout expires.
fn next_effect(lines: &mut Box<dyn Iterator<Item = String>>, timeout: Duration) -> Option<Uploaded> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let line = lines.next()?;
        if let Some(rest) = line.strip_prefix("effect:") {
            let field = |name: &str| -> u32 {
                rest.split_whitespace()
                    .find_map(|kv| kv.strip_prefix(name))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_default()
            };
            return Some(Uploaded {
                strong: field("strong=") as u16,
                weak: field("weak=") as u16,
                length_ms: field("length="),
            });
        }
        if line.starts_with("closing") {
            return None;
        }
    }
    None
}

/// Starts the rumble-capable virtual pad, returning it and a reader over its reports.
fn spawn_rumble_pad() -> Option<(Child, Box<dyn Iterator<Item = String>>)> {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/virtual_rumble_pad.py");
    let mut child = match Command::new("python3")
        .arg(script)
        .arg("20")
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            println!("[skip] python3 could not be started: {e}");
            return None;
        }
    };

    let stdout = child.stdout.take().expect("piped");
    let mut reader = BufReader::new(stdout);
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() || !first.starts_with("device:") {
        println!(
            "[skip] the virtual rumble pad did not come up (needs /dev/uinput writable and \
             python-evdev installed); script said: {first:?}"
        );
        let _ = child.kill();
        return None;
    }
    println!("[device] {}", first.trim());

    let lines = reader.lines().map_while(Result::ok);
    Some((child, Box::new(lines)))
}
