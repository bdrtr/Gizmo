#!/usr/bin/env python3
"""Play every demo that has a headless `--test` mode, and fail if any of them fails.

The games in `demo/src/bin/` are the engine's end-to-end scenarios: each one that takes `--test`
plays itself (an autopilot, a fixed timestep, no window) and exits non-zero when what it checks is
wrong — the tower fell, the ball left the table, the joint came apart. None of them ran in any
gate until 2026-11-04, so a physics change could break a game and nobody would hear about it:
that day a CCD fix moved `robot`'s balls into a path that exposed a 3 cm reach error, caught only
because the game happened to be run by hand.

A binary is picked up when its source contains the string `"--test"`, so a new game joins by
having the mode. Each runs with `DISPLAY` and `WAYLAND_DISPLAY` removed from its environment:
a binary that ignores the flag cannot open a window on the desktop of whoever runs this — it fails
instead, which is the right answer, since it has no headless mode to test. (The day this was
written, a background loop handed `--test` to `kapi`, which has no such mode; it sat open on the
user's screen for an hour and forty minutes.)

Release build: the physics is far too slow unoptimised (see CLAUDE.md).

Usage:
    scripts/game-tests.py                 # build, then play all of them, 4 at a time
    scripts/game-tests.py pinbol robot    # just these
    scripts/game-tests.py --jobs 2 --timeout 600
    scripts/game-tests.py --no-build      # use what target/release already holds
"""

import argparse
import glob
import os
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN_DIR = os.path.join(ROOT, "demo", "src", "bin")


def discover():
    """`(bin name, source path)` for every demo binary whose source names `"--test"`."""
    found = []
    for path in sorted(glob.glob(os.path.join(BIN_DIR, "*.rs"))):
        found.append((os.path.splitext(os.path.basename(path))[0], path))
    for path in sorted(glob.glob(os.path.join(BIN_DIR, "*", "main.rs"))):
        found.append((os.path.basename(os.path.dirname(path)), path))
    out = []
    for name, path in found:
        with open(path, encoding="utf-8") as f:
            if '"--test"' in f.read():
                out.append((name, path))
    return out


def play(name, timeout):
    exe = os.path.join(ROOT, "target", "release", name)
    env = {k: v for k, v in os.environ.items() if k not in ("DISPLAY", "WAYLAND_DISPLAY")}
    start = time.monotonic()
    try:
        p = subprocess.run([exe, "--test"], cwd=ROOT, env=env, capture_output=True, text=True, timeout=timeout)
        code, out = p.returncode, (p.stdout + p.stderr)
    except subprocess.TimeoutExpired as e:
        code = "timeout"
        out = (e.stdout or b"").decode(errors="replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
    lines = [l for l in out.splitlines() if l.strip()]
    return name, code, time.monotonic() - start, lines


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("names", nargs="*", help="only these binaries")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--timeout", type=float, default=300.0, help="seconds per game")
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    games = discover()
    if args.names:
        known = {n for n, _ in games}
        missing = [n for n in args.names if n not in known]
        if missing:
            sys.exit(f"no headless --test mode in: {', '.join(missing)}")
        games = [g for g in games if g[0] in args.names]
    if not games:
        sys.exit("found no binary with a --test mode — a scan that finds nothing must not pass")

    if not args.no_build:
        cmd = ["cargo", "build", "--release", "-p", "demo"]
        for name, _ in games:
            cmd += ["--bin", name]
        print(f"building {len(games)} binaries…", flush=True)
        if subprocess.run(cmd, cwd=ROOT).returncode != 0:
            sys.exit("build failed")

    failed = []
    t0 = time.monotonic()
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(play, name, args.timeout) for name, _ in games]
        for fut in as_completed(futures):
            name, code, secs, lines = fut.result()
            ok = code == 0
            last = lines[-1] if lines else "(no output)"
            print(f"{'ok    ' if ok else 'FAILED'} {name:<18} {secs:6.1f} s  {last}", flush=True)
            if not ok:
                failed.append(name)
                for l in lines[-15:-1]:
                    print(f"         | {l}")
                print(f"         exit: {code}")

    print(f"\n{len(games)} games, {len(failed)} failed, {time.monotonic() - t0:.0f} s wall")
    if failed:
        print("failed: " + " ".join(sorted(failed)))
        sys.exit(1)


if __name__ == "__main__":
    main()
