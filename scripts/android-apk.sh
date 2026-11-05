#!/usr/bin/env bash
# Builds the Android APK (`demo-android`), and with `run` installs it on the phone `adb` sees and
# starts it. Everything it needs is found where Android Studio puts it unless the environment says
# otherwise:
#
#   ANDROID_HOME   the SDK                      (default ~/Android/Sdk)
#   ANDROID_NDK_ROOT the NDK                    (default: the newest under $ANDROID_HOME/ndk)
#   JAVA_HOME      a JDK, for signing           (default ~/Android/jdk17, then the system's)
#   CARGO_APK_RELEASE_KEYSTORE / _PASSWORD      (default: ~/.android/debug.keystore, "android",
#                                                created on first use)
#
# One-time setup: `rustup target add aarch64-linux-android` and `cargo install cargo-apk`.
#
#   scripts/android-apk.sh          # → target/release/apk/gizmo-pinbol.apk
#   scripts/android-apk.sh run      # build, install over adb, start; logs: adb logcat -s gizmo
set -euo pipefail
cd "$(dirname "$0")/.."

export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_SDK_ROOT="${ANDROID_SDK_ROOT:-$ANDROID_HOME}"
if [ -z "${ANDROID_NDK_ROOT:-}" ]; then
    ANDROID_NDK_ROOT="$(ls -d "$ANDROID_HOME"/ndk/*/ 2>/dev/null | sort -V | tail -1)"
    export ANDROID_NDK_ROOT="${ANDROID_NDK_ROOT%/}"
fi
if [ -z "${JAVA_HOME:-}" ] && [ -d "$HOME/Android/jdk17" ]; then
    export JAVA_HOME="$HOME/Android/jdk17"
fi
[ -n "${JAVA_HOME:-}" ] && export PATH="$JAVA_HOME/bin:$PATH"

if [ -z "${CARGO_APK_RELEASE_KEYSTORE:-}" ]; then
    # The well-known Android debug key: fine for a phone on your desk, not for a store.
    export CARGO_APK_RELEASE_KEYSTORE="$HOME/.android/debug.keystore"
    export CARGO_APK_RELEASE_KEYSTORE_PASSWORD="android"
    if [ ! -f "$CARGO_APK_RELEASE_KEYSTORE" ]; then
        mkdir -p "$HOME/.android"
        keytool -genkeypair -keystore "$CARGO_APK_RELEASE_KEYSTORE" -storepass android \
            -alias androiddebugkey -keypass android -keyalg RSA -keysize 2048 -validity 10000 \
            -dname "CN=Android Debug,O=Android,C=US"
    fi
fi

if [ "${1:-}" = "run" ]; then
    cargo apk run -p demo-android --release
else
    cargo apk build -p demo-android --release
fi
