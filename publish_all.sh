#!/bin/bash
# Yelbegen-engine crates.io auto-publish script in dependency order

# Exit on any failure
set -e

# Crucial sleep duration to allow crates.io index propagation
SLEEP_TIME=15

echo "Starting publishing workspace crates to crates.io (Version 0.1.3)..."

# Array of crates in exact topological dependency order
crates=(
    "crates/gizmo-math"
    "crates/gizmo-core"
    "crates/gizmo-physics-core"
    "crates/gizmo-physics-rigid"
    "crates/gizmo-physics-soft"
    "crates/gizmo-physics-dynamics"
    "crates/gizmo-renderer"
    "crates/gizmo-window"
    "crates/gizmo-audio"
    "crates/gizmo-scene"
    "crates/gizmo-scripting"
    "crates/gizmo-network"
    "crates/gizmo-ai"
    "crates/gizmo-ui"
    "crates/gizmo-editor"
    "crates/gizmo-app"
    "crates/gizmo"
)

total=${#crates[@]}

for i in "${!crates[@]}"; do
    crate="${crates[$i]}"
    index=$((i + 1))
    echo "=========================================================="
    echo "[$index/$total] Publishing: $crate..."
    echo "=========================================================="
    
    # Enter crate directory and publish
    (cd "$crate" && cargo publish)
    
    # If not the last crate, sleep to let crates.io index update
    if [ $index -lt $total ]; then
        echo "Waiting $SLEEP_TIME seconds for crates.io index to update..."
        sleep $SLEEP_TIME
    fi
done

echo "=========================================================="
echo "Successfully published all workspace crates to crates.io!"
echo "=========================================================="
