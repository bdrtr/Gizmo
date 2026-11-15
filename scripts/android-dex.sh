#!/usr/bin/env bash
# Rebuilds `crates/gizmo-app/android/system_bars.dex` from `SystemBars.java` — the one class the
# Android host runs on the UI thread (`gizmo_app::android`). The dex is committed, so building an
# APK needs no Java compiler; run this after changing the Java source, and commit both.
#
# Needs a JDK (javac) and the Android SDK's build-tools (d8) and platform (android.jar).
set -euo pipefail
cd "$(dirname "$0")/.."

export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
if [ -z "${JAVA_HOME:-}" ] && [ -d "$HOME/Android/jdk17" ]; then
    export JAVA_HOME="$HOME/Android/jdk17"
fi
[ -n "${JAVA_HOME:-}" ] && export PATH="$JAVA_HOME/bin:$PATH"

platform="$(ls -d "$ANDROID_HOME"/platforms/android-* | sort -V | tail -1)"
d8="$(ls -d "$ANDROID_HOME"/build-tools/*/ | sort -V | tail -1)d8"
src=crates/gizmo-app/android
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

# Java 8 bytecode, against the newest platform (the class checks the API level itself); the
# apps' minimum is API 26.
javac --release 8 -classpath "$platform/android.jar" -d "$out" "$src/SystemBars.java" 2>&1 \
    | grep -v "warning: \[options\]" || true
"$d8" --release --min-api 26 --lib "$platform/android.jar" --output "$out" "$out"/com/bdrtr/gizmo/*.class
cp "$out/classes.dex" "$src/system_bars.dex"
echo "wrote $src/system_bars.dex ($(wc -c < "$src/system_bars.dex") bytes)"
