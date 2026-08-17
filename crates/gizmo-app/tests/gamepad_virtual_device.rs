//! The gamepad backend against a real kernel device — not a mock.
//!
//! Everything else about gamepads is testable without hardware: the state machine lives in
//! `gizmo-core` and the gilrs crosswalk is a table with its own unit tests. What none of that
//! can answer is whether the *stack* agrees — whether a button pressed on a device shows up as
//! the button this engine names, whether "up" on a stick is +1 here when the kernel calls it a
//! negative number, and whether a controller yanked out of the port releases what it was
//! holding. Those are the questions that were wrong in every engine that ever got them wrong.
//!
//! So this test creates a **virtual Xbox 360 pad** through `/dev/uinput`, drives a scripted
//! sequence of inputs into it, and asserts on what the engine ends up believing.
//!
//! It is `#[ignore]`d, because it needs three things CI does not have: `/dev/uinput` writable by
//! the test user (a udev ACL, or the `input` group), Python with `python-evdev` installed, and a
//! session where creating an input device is allowed. Run it by hand:
//!
//! ```text
//! cargo test -p gizmo-app --test gamepad_virtual_device -- --ignored --nocapture
//! ```
//!
//! The device script is `tests/virtual_pad.py`, and it prints each step it performs so a failure
//! can be lined up against the log this test prints.
#![cfg(all(feature = "gamepad", target_os = "linux"))]

use gizmo_app::gamepad::GamepadBackend;
use gizmo_core::input::{GamepadAxis, GamepadButton, Input};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// One thing the engine observed, in the order it observed it.
///
/// Buttons are logged as edges (`+South` / `-South`) and axes as values, but only when the
/// value moves by enough to be a real change — a log of every frame's stick position would bury
/// the four numbers that matter.
type Log = Vec<String>;

#[test]
#[ignore = "needs /dev/uinput and python-evdev; run with --ignored"]
fn a_virtual_pad_arrives_as_the_buttons_and_axes_this_engine_names() {
    let Some(mut device) = spawn_virtual_pad() else {
        return; // reason already printed
    };

    let mut input = Input::new();
    let mut backend = GamepadBackend::new(&mut input);
    assert!(
        backend.is_available(),
        "gilrs would not start; nothing below can be measured"
    );

    let mut log: Log = Vec::new();
    let mut last_axes = [f32::NAN; 6];
    let mut name = String::new();
    let mut focus_round_trip_done = false;
    let deadline = Instant::now() + Duration::from_secs(12);

    while Instant::now() < deadline {
        backend.pump(&mut input);

        // The script's last step holds Start down. That is the moment to prove the focus
        // round-trip, because it is the only moment something is held: losing focus must let
        // go of it, and regaining focus must take it back — and the second half is only
        // possible because the backend keeps its own mirror. gilrs cannot answer "what is this
        // pad doing", so a resync that asked it would silently restore nothing.
        let holding_start = input
            .gamepad()
            .is_some_and(|p| p.is_pressed(GamepadButton::Start));
        if holding_start && !focus_round_trip_done {
            focus_round_trip_done = true;
            let before = input.gamepad().map(|p| p.right_trigger()).unwrap_or(0.0);

            input.release_all(); // Alt-Tab
            let pad = input.gamepad().expect("the pad is still plugged in");
            assert!(
                !pad.is_pressed(GamepadButton::Start) && pad.is_just_released(GamepadButton::Start),
                "losing focus must let go of a held button"
            );

            backend.resync(&mut input); // focus comes back
            let pad = input.gamepad().expect("still plugged in");
            assert!(
                pad.is_pressed(GamepadButton::Start),
                "regaining focus must restore a button the player never released"
            );
            assert_eq!(
                pad.right_trigger(),
                before,
                "and the analog controls must come back where they were"
            );
            log.push("focus round-trip".to_string());
        }

        if let Some(pad) = input.gamepad() {
            if name.is_empty() {
                name = pad.name().to_string();
                log.push(format!("connected: {name}"));
            }
            for button in GamepadButton::ALL {
                if pad.is_just_pressed(button) {
                    log.push(format!("+{button:?}"));
                }
                if pad.is_just_released(button) {
                    log.push(format!("-{button:?}"));
                }
            }
            for (i, axis) in GamepadAxis::ALL.into_iter().enumerate() {
                let value = pad.axis(axis);
                if !(value - last_axes[i]).abs().lt(&0.02) {
                    last_axes[i] = value;
                    log.push(format!("{axis:?}={value:.2}"));
                }
            }
        } else if !name.is_empty() && !log.iter().any(|l| l == "disconnected") {
            log.push("disconnected".to_string());
        }

        // The movement axis every demo now reads, taken from the same live device rather than
        // from a hand-built `Input`. Its unit tests pin the arithmetic; what only a device can
        // answer is whether the stick reaches it at all, and with the sign the player expects.
        let (mx, my) = input.move_axis();
        if mx.abs() > 0.5 && !log.iter().any(|l| l.starts_with("move_axis right")) {
            log.push(format!("move_axis right = ({mx:.2}, {my:.2})"));
        }
        if my.abs() > 0.5 && !log.iter().any(|l| l.starts_with("move_axis up")) {
            log.push(format!("move_axis up = ({mx:.2}, {my:.2})"));
        }

        input.begin_frame();
        std::thread::sleep(Duration::from_millis(4));

        // The script ends by closing the device; stop as soon as we have seen that land.
        if log.iter().any(|l| l == "disconnected") {
            break;
        }
    }
    let _ = device.wait();

    for line in &log {
        println!("[observed] {line}");
    }

    let saw = |needle: &str| log.iter().any(|l| l == needle);
    let value_of = |prefix: &str| -> Vec<f32> {
        log.iter()
            .filter_map(|l| l.strip_prefix(prefix))
            .filter_map(|v| v.parse::<f32>().ok())
            .collect()
    };

    assert!(!name.is_empty(), "no pad ever connected; log: {log:?}");

    // ── Buttons: the press/release pair, and the naming that inverts ──────────────
    assert!(saw("+South") && saw("-South"), "face button, log: {log:?}");
    assert!(
        saw("+LeftBumper") && saw("-LeftBumper"),
        "BTN_TL must arrive as the BUMPER, not as a trigger — log: {log:?}"
    );
    assert!(
        !saw("+LeftTrigger"),
        "the shoulder button must not be reported as the analog trigger — log: {log:?}"
    );

    // ── Sticks: full deflection reaches ±1, and UP is POSITIVE ───────────────────
    let xs = value_of("LeftStickX=");
    assert!(
        xs.iter().any(|v| (*v - 1.0).abs() < 0.05),
        "stick pushed fully right must read +1, saw {xs:?}"
    );
    let ys = value_of("LeftStickY=");
    assert!(
        ys.iter().any(|v| (*v - 1.0).abs() < 0.05),
        "stick pushed UP must read +1 — the kernel calls that direction negative, so this is \
         the assertion that catches an inverted Y. Saw {ys:?}"
    );

    // ── …and that the same stick reaches `Input::move_axis`, which is what the demos read ──
    let moved_right = log.iter().find(|l| l.starts_with("move_axis right"));
    let moved_up = log.iter().find(|l| l.starts_with("move_axis up"));
    assert!(
        moved_right.is_some(),
        "the stick pushed right never reached `move_axis` — 16 demos read movement through it \
         and would not have moved. Log: {log:?}"
    );
    assert!(
        moved_up.is_some(),
        "the stick pushed up never reached `move_axis`. Log: {log:?}"
    );

    // ── Triggers: analog travel, on their own axes ───────────────────────────────
    let lt = value_of("LeftTrigger=");
    assert!(
        lt.iter().any(|v| (*v - 0.5).abs() < 0.1),
        "left trigger at half travel must read ~0.5, saw {lt:?}"
    );
    let rt = value_of("RightTrigger=");
    assert!(
        rt.iter().any(|v| (*v - 1.0).abs() < 0.05),
        "right trigger fully pulled must read 1.0, saw {rt:?}"
    );

    // ── A hat d-pad must arrive as the four buttons ──────────────────────────────
    assert!(
        saw("+DPadRight") && saw("-DPadRight"),
        "the hat's X axis must become d-pad buttons, log: {log:?}"
    );
    assert!(
        saw("+DPadUp"),
        "the hat's Y axis must become d-pad buttons, and up must be up, log: {log:?}"
    );

    // ── And the pad that vanishes while a button is held releases it ─────────────
    // `rposition`, because the focus round-trip above also released and re-pressed Start: what
    // must hold is that the LAST thing that happened to it was a release.
    let start_down = log.iter().rposition(|l| l == "+Start");
    let start_up = log.iter().rposition(|l| l == "-Start");
    assert!(start_down.is_some(), "Start was never pressed, log: {log:?}");
    assert!(
        focus_round_trip_done,
        "the focus round-trip never ran, so it proved nothing; log: {log:?}"
    );
    assert!(
        start_up > start_down,
        "unplugging the pad while Start was held must release it, log: {log:?}"
    );
    assert!(saw("disconnected"), "the pad must go away, log: {log:?}");
}

/// Starts `tests/virtual_pad.py` and waits for it to report the device node it created.
///
/// Returns `None` — after printing why — when the environment cannot host a virtual device.
/// That is a skip rather than a failure: this test is about the mapping, and a machine without
/// `/dev/uinput` access has nothing to say about it either way.
fn spawn_virtual_pad() -> Option<Child> {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/virtual_pad.py");
    let mut child = match Command::new("python3")
        .arg(script)
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
    let mut first = String::new();
    let mut reader = BufReader::new(stdout);
    if reader.read_line(&mut first).is_err() || !first.starts_with("device:") {
        println!(
            "[skip] the virtual pad did not come up (needs /dev/uinput writable and \
             python-evdev installed); script said: {first:?}"
        );
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    println!("[virtual pad] {}", first.trim());
    // Keep draining the script's progress lines so it never blocks on a full pipe.
    std::thread::spawn(move || {
        for line in reader.lines().map_while(Result::ok) {
            println!("[virtual pad] {line}");
        }
    });
    Some(child)
}
