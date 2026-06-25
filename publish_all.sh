#!/bin/bash
# Publish the Gizmo workspace crates to crates.io in dependency (topological)
# order, so each crate's path-deps already exist on the registry when it ships.
#
# STAGED VERSIONS: crates no longer share one workspace version. The Stage A core
# is on the `1.x` line and the Stage B graphics/integration crates stay on `0.y`
# (see RELEASING.md). This script therefore reads each crate's own version from
# its Cargo.toml instead of assuming a single uniform version.
#
# Usage:
#   ./publish_all.sh            # real publish
#   DRY_RUN=1 ./publish_all.sh  # `cargo publish --dry-run` for every crate (no upload)
#
# `gizmo-studio` is intentionally absent: it is `publish = false` (a binary/app),
# and `cargo publish` errors on it, which would abort the run.

set -euo pipefail

# Sleep between publishes to let the crates.io index propagate so the next
# crate's freshly-published path-dep is resolvable.
SLEEP_TIME=15
DRY_RUN="${DRY_RUN:-0}"

# Topological dependency order (foundations first, facade last) — matches
# RELEASING.md §5. [A] = Stage A (1.x), [B] = Stage B (0.y).
crates=(
    "crates/gizmo-math"             # [A] foundation; glam
    "crates/gizmo-core"             # [A] ECS
    "crates/gizmo-physics-core"     # [A]
    "crates/gizmo-physics-rigid"    # [A]
    "crates/gizmo-net"              # [A]
    "crates/gizmo-physics-soft"     # [A]
    "crates/gizmo-physics-dynamics" # [A]
    "crates/gizmo-audio"            # [A]
    "crates/gizmo-ai"               # [A]
    "crates/gizmo-renderer"         # [B]
    "crates/gizmo-window"           # [B]
    "crates/gizmo-scripting"        # [B]
    "crates/gizmo-scene"            # [A] (depends on gizmo-scripting on non-wasm)
    "crates/gizmo-editor"           # [B]
    "crates/gizmo-app"              # [B]
    "crates/gizmo-animation"        # [B] (transitively, via gizmo-app)
    "crates/gizmo-ui"               # [B]
    "crates/gizmo"                  # [B] facade — re-exports everything
)

total=${#crates[@]}

if [ "$DRY_RUN" = "1" ]; then
    echo "DRY RUN — no crates will be uploaded."
fi
echo "Publishing $total workspace crates to crates.io (staged versions)..."

for i in "${!crates[@]}"; do
    crate="${crates[$i]}"
    index=$((i + 1))
    version=$(grep -m1 '^version' "$crate/Cargo.toml" | sed -E 's/.*"(.*)".*/\1/')
    [ "$version" = "version.workspace = true" ] && version="(workspace)"
    echo "=========================================================="
    echo "[$index/$total] $crate  @  ${version}"
    echo "=========================================================="

    (
        cd "$crate"
        if [ "$DRY_RUN" = "1" ]; then
            cargo publish --locked --dry-run
        else
            # Real publish, with a robust "already published" guard so re-running
            # the script after a partial failure is idempotent.
            if ! output=$(cargo publish --locked 2>&1); then
                if echo "$output" | grep -qi "already exists\|already uploaded"; then
                    echo "Notice: this version already exists on crates.io. Skipping."
                else
                    echo "Error publishing $crate:"
                    echo "$output"
                    exit 1
                fi
            else
                echo "$output"
                echo "Successfully published!"
            fi
        fi
    )

    if [ "$DRY_RUN" != "1" ] && [ $index -lt $total ]; then
        echo "Waiting $SLEEP_TIME seconds for the crates.io index to update..."
        sleep $SLEEP_TIME
    fi
done

echo "=========================================================="
if [ "$DRY_RUN" = "1" ]; then
    echo "Dry run complete — all $total crates packaged cleanly."
else
    echo "Successfully published all $total workspace crates to crates.io!"
fi
echo "=========================================================="
