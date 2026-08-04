#!/bin/bash
# Publish the Gizmo workspace crates to crates.io in dependency (topological)
# order, so each crate's path-deps already exist on the registry when it ships.
#
# VERSIONS: the workspace currently ships at one uniform `0.x` version, inherited from
# `[workspace.package] version` in the root Cargo.toml. A later release will adopt the
# STAGED model (Stage A core on `1.x`, Stage B graphics/integration on `0.y`; see
# docs/ENGINE.md §4), at which point crates will no longer share a version. The version
# lookup below already handles both: it resolves `version.workspace = true` against the
# root manifest, and reads a literal version when a crate declares its own. The [A]/[B]
# tags mark the eventual stage split.
#
# Usage:
#   ./publish_all.sh            # real publish
#   DRY_RUN=1 ./publish_all.sh  # see the caveat below — this is NOT a full rehearsal
#
# ─── DRY_RUN cannot validate a version bump, and it is important to know why ───
#
# `cargo publish --dry-run` performs a full verify build with the path dependencies
# REPLACED by their registry versions. After bumping the workspace to a version that is
# not yet on crates.io, every crate except the first tier fails to resolve: gizmo-core
# 0.9.0 does not exist on the registry yet, so the dry run for gizmo-physics-core cannot
# build. That is inherent to how dry-run works, not a bug here.
#
# So DRY_RUN is useful for catching packaging problems (missing files, bad metadata,
# `include`/`exclude` mistakes) on a release that does NOT change the version, and for
# the foundation crates on one that does. For everything else the real pre-flight checks
# are `cargo package --list`, the feature-powerset CI job, and `cargo deny`.
#
# `gizmo-studio` is intentionally absent: it is `publish = false` (a binary/app),
# and `cargo publish` errors on it, which would abort the run.

set -euo pipefail

# Sleep between publishes to let the crates.io index propagate so the next
# crate's freshly-published path-dep is resolvable.
SLEEP_TIME=15
DRY_RUN="${DRY_RUN:-0}"

# Topological dependency order (foundations first, facade last) — matches
# docs/ENGINE.md §4. [A] = Stage A (1.x), [B] = Stage B (0.y).
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
    "crates/gizmo-animation"        # [B] MUST precede gizmo-renderer (renderer normal-deps it)
    "crates/gizmo-renderer"         # [B] (depends on gizmo-animation)
    "crates/gizmo-window"           # [B]
    "crates/gizmo-scripting"        # [B] (depends on gizmo-animation)
    "crates/gizmo-scene"            # [A] (depends on gizmo-scripting on non-wasm)
    "crates/gizmo-editor"           # [B] (depends on gizmo-renderer/scene/scripting)
    "crates/gizmo-app"              # [B] (depends on gizmo-editor/renderer/scene/scripting)
    "crates/gizmo-ui"               # [B]
    "crates/gizmo-analysis"         # [B] (opt-depends on gizmo-app/gizmo-physics-rigid; facade opt-dep)
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
    # Resolve the crate's effective version. Almost every crate here writes
    # `version.workspace = true`, so a naive `grep '^version'` matches that line, finds no
    # quotes to extract, and prints the line back verbatim — which is what this script used
    # to do while its header claimed to "read each crate's own version". Handle both forms.
    version_line=$(grep -m1 -E '^version' "$crate/Cargo.toml" || true)
    if [[ "$version_line" == *"workspace = true"* ]]; then
        version=$(grep -m1 -E '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')
    else
        version=$(echo "$version_line" | sed -E 's/.*"(.*)".*/\1/')
    fi
    [ -z "$version" ] && version="(unknown)"
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
