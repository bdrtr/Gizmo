#!/bin/bash
# End-to-end check of system hot-patching (`gizmo_core::hotpatch`): starts `demo/src/bin/hot_systems.rs`
# under `dx serve --hot-patch`, waits for its first line, edits the line its system returns, and waits
# for the new text — with the counter carrying on, which is what says the process was not restarted.
# Restores the file and stops `dx` whatever happens.
#
# Needs `dx` from `cargo install dioxus-cli --version 0.7.10 --locked`. CI has no `dx`, so this is a
# local check. The first build compiles the engine in debug and takes a few minutes.
#
#   scripts/hotpatch-check.sh            # exit 0 when the patch is seen
set -u
cd "$(dirname "$0")/.."
command -v dx >/dev/null || { echo "dx not found: cargo install dioxus-cli --version 0.7.10 --locked"; exit 2; }
F=demo/src/bin/hot_systems.rs
KEEP=$(mktemp)
LOG=$(mktemp)
cp "$F" "$KEEP"
setsid dx serve --hot-patch --interactive false -p demo --bin hot_systems --features hotpatching > "$LOG" 2>&1 < /dev/null &
DX=$!
cleanup() { cp "$KEEP" "$F"; kill -- -"$DX" 2>/dev/null; sleep 1; kill -9 -- -"$DX" 2>/dev/null; rm -f "$KEEP"; }
trap cleanup EXIT
wait_for() { # pattern, seconds
    for _ in $(seq 1 "$2"); do
        grep -q "$1" "$LOG" && return 0
        kill -0 "$DX" 2>/dev/null || return 2
        sleep 1
    done
    return 1
}
wait_for "merhaba, " 1500 || { echo "FAIL: no first line"; tail -20 "$LOG"; exit 1; }
sleep 3
sed -i 's/merhaba, {} saniye geçti/SELAM, {} saniye oldu/' "$F"
wait_for "SELAM, " 600 || { echo "FAIL: the patched line never came"; tail -20 "$LOG"; exit 1; }
before=$(grep -o "merhaba, [0-9]*" "$LOG" | tail -1 | grep -o "[0-9]*$")
after=$(grep -o "SELAM, [0-9]*" "$LOG" | head -1 | grep -o "[0-9]*$")
grep -E "merhaba|SELAM|Hot-patching" "$LOG" | sed 's/\x1b\[[0-9;]*m//g' | tail -4
if [ "$after" -gt "$before" ]; then
    echo "OK: patched without a restart (counter $before -> $after)"
else
    echo "FAIL: the counter went $before -> $after, so the program restarted"
    exit 1
fi
