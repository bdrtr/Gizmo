#!/bin/bash
# Yelbegen-engine crates.io auto-publish script in dependency order

# Exit on any failure
set -e

# Crucial sleep duration to allow crates.io index propagation
SLEEP_TIME=15

echo "Starting publishing workspace crates to crates.io (Version 0.1.7)..."

# Array of crates in exact topological dependency order
crates=(
    "crates/gizmo-math"
    "crates/gizmo-core"
    "crates/gizmo-physics-core"
    "crates/gizmo-physics-rigid"
    "crates/gizmo-net"
    "crates/gizmo-physics-soft"
    "crates/gizmo-physics-dynamics"
    "crates/gizmo-renderer"
    "crates/gizmo-window"
    "crates/gizmo-audio"
    "crates/gizmo-ai"
    "crates/gizmo-scripting"
    "crates/gizmo-scene"
    "crates/gizmo-editor"
    "crates/gizmo-app"
    "crates/gizmo-animation"
    "crates/gizmo-ui"
    "crates/gizmo"
    "crates/gizmo-studio"
)

total=${#crates[@]}

for i in "${!crates[@]}"; do
    crate="${crates[$i]}"
    index=$((i + 1))
    echo "=========================================================="
    echo "[$index/$total] Publishing: $crate..."
    echo "=========================================================="
    
    # Enter crate directory and publish with robust already-exists check
    (
        cd "$crate"
        # Run cargo publish and capture stderr/stdout
        if ! output=$(cargo publish 2>&1); then
            # If failed, check if it's just because the version already exists
            if echo "$output" | grep -qi "already exists"; then
                echo "Notice: Crate version already exists on crates.io. Skipping..."
            else
                echo "Error publishing $crate:"
                echo "$output"
                exit 1
            fi
        else
            echo "$output"
            echo "Successfully published!"
        fi
    )
    
    # If not the last crate, sleep to let crates.io index update
    if [ $index -lt $total ]; then
        echo "Waiting $SLEEP_TIME seconds for crates.io index to update..."
        sleep $SLEEP_TIME
    fi
done

echo "=========================================================="
echo "Successfully published all workspace crates to crates.io!"
echo "=========================================================="
