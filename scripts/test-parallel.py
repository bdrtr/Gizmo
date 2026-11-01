#!/usr/bin/env python3
"""Run the workspace's test binaries side by side instead of one after another.

`cargo test` builds every test binary, then runs them **sequentially**: each binary's tests run in
parallel inside it, but the next binary waits for the slowest test of the previous one. Measured
2026-11-01 on this 16-core machine, the 250 binaries of `cargo test --workspace` spend 307 s
running back to back, and most of that is a handful of long binaries leaving the other cores idle.

This script builds exactly what `cargo test` builds (`--no-run`), then runs the binaries in a pool
of `--jobs` workers, each the way cargo would: working directory = the package root, and the
`CARGO_MANIFEST_DIR` / `CARGO_PKG_*` environment cargo sets for a test run. Doc tests run as one
more job (`cargo test --doc`), unless `--no-doc`.

**Longest first.** Each run records how long every binary took (`target/test-parallel-times.json`)
and the next run starts the longest ones first, so the chain of GPU binaries (which wait on each
other, see below) is not left until the end. A binary with no recorded time counts as long.

**Doc tests compile with 4 threads, not one per core.** rustdoc builds each doc test with its own
rustc, as many at once as there are test threads; with 16 of those next to four running binaries,
the first measured run peaked at 11.5 GB of this machine's 13.

**GPU binaries are safe to run alongside.** Each GPU test binary takes a machine-wide `flock`
(`gizmo-gpu-tests.lock` in the temp dir) on its first GPU test and holds it until it exits — see
`hold_machine_gpu_lock` in `gizmo-renderer/src/test_gpu.rs`. So GPU binaries still run one at a
time, as under `cargo test`, while everything else overlaps them.

This does not replace `cargo test --workspace --no-fail-fast`; CI still runs that. It reports the
same thing faster: every binary runs (there is no fail-fast), and the exit code is non-zero if any
binary or the doc tests failed.

Usage:
    scripts/test-parallel.py                       # whole workspace, default features
    scripts/test-parallel.py -p gizmo-physics-rigid --features experimental-multibody
    scripts/test-parallel.py -- --ignored          # arguments after `--` go to every test binary
    scripts/test-parallel.py --jobs 6 --no-doc
"""

import argparse
import json
import os
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TIMES = os.path.join(ROOT, "target", "test-parallel-times.json")
DOC_LABEL = "doc tests"


def load_times():
    try:
        with open(TIMES) as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def save_times(times):
    try:
        with open(TIMES, "w") as f:
            json.dump(times, f, indent=0, sort_keys=True)
    except OSError:
        pass


def parse_args():
    argv = sys.argv[1:]
    test_args = []
    if "--" in argv:
        i = argv.index("--")
        argv, test_args = argv[:i], argv[i + 1:]
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--jobs", type=int, default=4,
                    help="test binaries running at once (default 4; each one already uses every "
                         "core for its own tests, so more mostly adds memory)")
    ap.add_argument("--no-doc", action="store_true", help="skip `cargo test --doc`")
    ap.add_argument("--slowest", type=int, default=8, help="how many of the slowest binaries to list")
    args, cargo_args = ap.parse_known_args(argv)
    if not any(a in ("-p", "--package") or a.startswith("--package=") for a in cargo_args):
        cargo_args = ["--workspace"] + cargo_args
    return args, cargo_args, test_args


def build(cargo_args):
    """`cargo test --no-run`; returns [(executable, package_root, package_name, version, target)]."""
    cmd = ["cargo", "test", "--no-run", "--message-format=json-render-diagnostics"] + cargo_args
    proc = subprocess.Popen(cmd, cwd=ROOT, stdout=subprocess.PIPE, text=True)
    binaries = []
    for line in proc.stdout:
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") != "compiler-artifact" or not msg.get("executable"):
            continue
        if not msg.get("profile", {}).get("test"):
            continue
        pkg_root = os.path.dirname(msg["manifest_path"])
        # package_id is `path+file:///…/crates/name#version` or `…#name@version`.
        pid = msg["package_id"]
        tail = pid.rsplit("#", 1)[-1]
        name, _, version = tail.partition("@")
        if not version:
            name, version = os.path.basename(pkg_root), tail
        kind = "+".join(msg["target"].get("kind", []))
        binaries.append((msg["executable"], pkg_root, name, version, msg["target"]["name"], kind))
    if proc.wait() != 0:
        sys.exit(f"build failed: {' '.join(cmd)}")
    return binaries


def run_binary(exe, pkg_root, name, version, target, test_args):
    env = dict(os.environ)
    major, minor, patch = (version.split("-")[0].split(".") + ["0", "0", "0"])[:3]
    env.update({
        "CARGO_MANIFEST_DIR": pkg_root,
        "CARGO_PKG_NAME": name,
        "CARGO_PKG_VERSION": version,
        "CARGO_PKG_VERSION_MAJOR": major,
        "CARGO_PKG_VERSION_MINOR": minor,
        "CARGO_PKG_VERSION_PATCH": patch,
        "CARGO_CRATE_NAME": target.replace("-", "_"),
    })
    start = time.monotonic()
    proc = subprocess.run([exe] + test_args, cwd=pkg_root, env=env,
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                          errors="replace")
    return proc.returncode, proc.stdout, time.monotonic() - start


def run_doc(cargo_args, test_args):
    start = time.monotonic()
    cmd = ["cargo", "test", "--doc", "--no-fail-fast"] + cargo_args + ["--", "--test-threads=4"]
    cmd += test_args
    proc = subprocess.run(cmd, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          text=True, errors="replace")
    return proc.returncode, proc.stdout, time.monotonic() - start


def summary_counts(output):
    passed = failed = ignored = 0
    for line in output.splitlines():
        if line.startswith("test result:"):
            parts = line.replace(";", "").split()
            for i, word in enumerate(parts):
                if word in ("passed", "failed", "ignored") and i > 0 and parts[i - 1].isdigit():
                    n = int(parts[i - 1])
                    passed += n if word == "passed" else 0
                    failed += n if word == "failed" else 0
                    ignored += n if word == "ignored" else 0
    return passed, failed, ignored


def main():
    args, cargo_args, test_args = parse_args()
    t0 = time.monotonic()
    binaries = build(cargo_args)
    t_build = time.monotonic() - t0
    print(f"built {len(binaries)} test binaries in {t_build:.0f} s; running {args.jobs} at a time",
          flush=True)

    times = load_times()
    jobs = [] if args.no_doc else [(DOC_LABEL, lambda: run_doc(cargo_args, test_args))]
    for exe, root, name, version, target, kind in binaries:
        label = f"{name} :: {target} ({kind})"
        jobs.append((label, lambda e=exe, r=root, n=name, v=version, t=target:
                     run_binary(e, r, n, v, t, test_args)))
    # Longest first; unknown ones count as long so a new GPU binary is not started last.
    jobs.sort(key=lambda j: -times.get(j[0], 1e9))

    results = []  # (label, code, output, seconds)
    t1 = time.monotonic()
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = {pool.submit(fn): label for label, fn in jobs}
        for fut in as_completed(futures):
            label = futures[fut]
            code, out, secs = fut.result()
            results.append((label, code, out, secs))
            if code != 0:
                print(f"FAILED  {label}  ({secs:.1f} s)", flush=True)
    t_run = time.monotonic() - t1
    if not test_args:  # a filtered run's times say nothing about the full one
        times.update({label: round(secs, 2) for label, _, _, secs in results})
        save_times(times)

    failures = [r for r in results if r[1] != 0]
    for label, code, out, _ in failures:
        print(f"\n===== {label} (exit {code}) =====\n{out.rstrip()}")

    passed = failed = ignored = 0
    for _, _, out, _ in results:
        p, f, i = summary_counts(out)
        passed, failed, ignored = passed + p, failed + f, ignored + i
    serial = sum(r[3] for r in results)
    print(f"\nslowest:")
    for label, _, _, secs in sorted(results, key=lambda r: -r[3])[: args.slowest]:
        print(f"  {secs:7.1f} s  {label}")
    print(f"\n{len(results)} jobs, {len(failures)} failed · tests: {passed} passed, {failed} failed, "
          f"{ignored} ignored · build {t_build:.0f} s, run {t_run:.0f} s wall "
          f"({serial:.0f} s if run one after another)")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
