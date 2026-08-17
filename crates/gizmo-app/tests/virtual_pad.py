#!/usr/bin/env python3
"""Create a virtual Xbox-360-shaped gamepad on /dev/uinput and drive a scripted sequence.

Used to verify the engine's gamepad backend end to end without hardware. Prints one line per
step so the reader side can be lined up against it.
"""
import sys
import time

from evdev import AbsInfo, UInput
from evdev import ecodes as e

STICK = AbsInfo(value=0, min=-32768, max=32767, fuzz=16, flat=128, resolution=0)
TRIGGER = AbsInfo(value=0, min=0, max=255, fuzz=0, flat=0, resolution=0)
HAT = AbsInfo(value=0, min=-1, max=1, fuzz=0, flat=0, resolution=0)

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
}


def main():
    hold = float(sys.argv[1]) if len(sys.argv) > 1 else 0.35
    ui = UInput(CAPS, name="Microsoft X-Box 360 pad", vendor=0x045E,
                product=0x028E, version=0x0110)
    print(f"device: {ui.device.path}", flush=True)
    # Let the reader's device enumeration catch up before anything is pressed.
    time.sleep(1.5)

    def step(label, writes):
        for kind, code, value in writes:
            ui.write(kind, code, value)
        ui.syn()
        print(f"step: {label}", flush=True)
        time.sleep(hold)

    # Face button, held then released — the press/release pair.
    step("south down", [(e.EV_KEY, e.BTN_SOUTH, 1)])
    step("south up", [(e.EV_KEY, e.BTN_SOUTH, 0)])
    # Left shoulder: this is the one that must NOT arrive as a trigger.
    step("bumper down", [(e.EV_KEY, e.BTN_TL, 1)])
    step("bumper up", [(e.EV_KEY, e.BTN_TL, 0)])
    # Left stick fully right, then fully "up" — which on evdev is ABS_Y at its MINIMUM.
    step("stick right", [(e.EV_ABS, e.ABS_X, 32767)])
    step("stick centre x", [(e.EV_ABS, e.ABS_X, 0)])
    step("stick up", [(e.EV_ABS, e.ABS_Y, -32768)])
    step("stick centre y", [(e.EV_ABS, e.ABS_Y, 0)])
    # Analog triggers, half and full.
    step("left trigger half", [(e.EV_ABS, e.ABS_Z, 128)])
    step("right trigger full", [(e.EV_ABS, e.ABS_RZ, 255)])
    step("triggers released", [(e.EV_ABS, e.ABS_Z, 0), (e.EV_ABS, e.ABS_RZ, 0)])
    # D-pad as a hat: right, then up (ABS_HAT0Y minimum is up).
    step("hat right", [(e.EV_ABS, e.ABS_HAT0X, 1)])
    step("hat centre", [(e.EV_ABS, e.ABS_HAT0X, 0)])
    step("hat up", [(e.EV_ABS, e.ABS_HAT0Y, -1)])
    step("hat centre", [(e.EV_ABS, e.ABS_HAT0Y, 0)])
    # Start, held to the end so the reader can see it still down when the pad vanishes.
    step("start down", [(e.EV_KEY, e.BTN_START, 1)])

    print("closing", flush=True)
    ui.close()


if __name__ == "__main__":
    main()
