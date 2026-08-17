#!/usr/bin/env python3
"""A virtual gamepad that can rumble, used to read back what the engine actually uploaded.

`virtual_pad.py` proves inputs arrive; this proves the one thing that travels the other way. It
creates a uinput device declaring `FF_RUMBLE`, then sits in the kernel's force-feedback upload
protocol and prints the magnitudes of every effect the driver hands it.

That protocol is the whole point of this file. A rumble request has no observable effect a test
can assert on — no event comes back, and the motors are imaginary — so the only place the numbers
are visible is the moment the kernel passes the effect through to the device that declared it.
Reading them there is the difference between "the code path ran" and "the pad was asked to shake
this hard".

Prints, one per line:
    device: /dev/input/eventN
    effect: id=<n> strong=<0..65535> weak=<0..65535> length=<ms>
    erase: id=<n>
"""
import select
import sys
import time

from evdev import AbsInfo, UInput
from evdev import ecodes as e

STICK = AbsInfo(value=0, min=-32768, max=32767, fuzz=16, flat=128, resolution=0)

TRIGGER = AbsInfo(value=0, min=0, max=255, fuzz=0, flat=0, resolution=0)
HAT = AbsInfo(value=0, min=-1, max=1, fuzz=0, flat=0, resolution=0)

# The same button/axis set as `virtual_pad.py`: gilrs only calls a device a gamepad if it has
# buttons and at least two axes, and an SDL mapping is matched on vendor/product. A reduced pad
# is one more variable in a test whose subject is force feedback.
CAPS = {
    e.EV_KEY: [
        e.BTN_SOUTH, e.BTN_EAST, e.BTN_NORTH, e.BTN_WEST,
        e.BTN_TL, e.BTN_TR, e.BTN_SELECT, e.BTN_START, e.BTN_MODE,
        e.BTN_THUMBL, e.BTN_THUMBR,
    ],
    e.EV_ABS: [
        (e.ABS_X, STICK), (e.ABS_Y, STICK),
        (e.ABS_RX, STICK), (e.ABS_RY, STICK),
        (e.ABS_Z, TRIGGER), (e.ABS_RZ, TRIGGER),
        (e.ABS_HAT0X, HAT), (e.ABS_HAT0Y, HAT),
    ],
    # FF_RUMBLE is what the kernel needs to route uploads here. The other four are what GILRS
    # reads: its Linux capability test (`gilrs-core/src/platform/linux/gamepad.rs::test_ff`) asks
    # for FF_SQUARE && FF_TRIANGLE && FF_SINE && FF_GAIN — the periodic-waveform feature set of a
    # force-feedback wheel — and never looks at FF_RUMBLE at all. Worth knowing on its own: most
    # console-style pads on Linux advertise only FF_RUMBLE (xpad builds its device with
    # `input_ff_create_memless`), so gilrs reports no force feedback for them.
    #
    # MEASURED 2026-08-18, and NOT explained by that: this device advertises all five — verified
    # by reading its capabilities back, `[FF_RUMBLE, FF_PERIODIC, FF_SQUARE, FF_TRIANGLE,
    # FF_SINE, FF_GAIN]` — and gilrs still reports `is_ff_supported() == false`. So the blocker
    # is inside gilrs-core's Linux path rather than in what is declared here, and the engine's
    # own side (queue, clamps, one-effect-per-pad) is what the unit tests cover instead. An
    # earlier version of this comment blamed the missing bits; that was wrong, and the
    # measurement above is why.
    e.EV_FF: [e.FF_RUMBLE, e.FF_PERIODIC, e.FF_SQUARE, e.FF_TRIANGLE, e.FF_SINE, e.FF_GAIN],
}


def main():
    run_for = float(sys.argv[1]) if len(sys.argv) > 1 else 8.0
    # `max_effects` is the number of driver slots the device advertises. Deliberately small: the
    # engine holds ONE effect per pad and replaces it, so a leak shows up here as an upload that
    # fails rather than as a slow drift nobody notices.
    ui = UInput(CAPS, name="Virtual Rumble Pad", vendor=0x045E, product=0x028E,
                version=0x0110, max_effects=16)
    print(f"device: {ui.device.path}", flush=True)

    deadline = time.time() + run_for
    while time.time() < deadline:
        r, _, _ = select.select([ui.fd], [], [], 0.1)
        if not r:
            continue
        for event in ui.read():
            if event.type != e.EV_UINPUT:
                continue
            if event.code == e.UI_FF_UPLOAD:
                upload = ui.begin_upload(event.value)
                effect = upload.effect
                # `u.effect.u.ff_rumble_effect` carries the two magnitudes the driver resolved
                # the effect down to — which is what a motor would be driven with.
                rumble = effect.u.ff_rumble_effect
                print(
                    f"effect: id={effect.id} strong={rumble.strong_magnitude} "
                    f"weak={rumble.weak_magnitude} length={effect.replay.length}",
                    flush=True,
                )
                upload.retval = 0
                ui.end_upload(upload)
            elif event.code == e.UI_FF_ERASE:
                erase = ui.begin_erase(event.value)
                print(f"erase: id={erase.effect_id}", flush=True)
                erase.retval = 0
                ui.end_erase(erase)

    print("closing", flush=True)
    ui.close()


if __name__ == "__main__":
    main()
