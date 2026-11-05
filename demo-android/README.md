# demo-android — the games as an Android app

The games of `games/` built into APKs that install on a phone and open full screen from their
icon, one APK per game: **Gizmo Pinbol** (`pinbol`, touch flippers; this crate) and **Gizmo
Nişancı** (`nisanci`, two touch sticks; `demo-android-nisanci`, the same host for another game).
Each is the same game the desktop binary and the browser build run.

```bash
scripts/android-apk.sh                # → target/release/apk/gizmo-pinbol.apk
scripts/android-apk.sh nisanci        # → target/release/apk/gizmo-nisanci.apk
scripts/android-apk.sh nisanci run    # build, install on the phone adb sees, start it
adb logcat -s gizmo                   # the engine's log
```

CI's `android` job builds both on every push and keeps them as the run's `gizmo-apks` artifact.

## What it needs

- The Android SDK with an NDK (Android Studio's SDK Manager puts both under `~/Android/Sdk`).
- A JDK, only for signing (`keytool`, `apksigner`); Android Studio ships one.
- `rustup target add aarch64-linux-android` and `cargo install cargo-apk`.

`scripts/android-apk.sh` finds all of them where Android Studio puts them; the environment
variables it reads are listed at its top. Without a keystore configured it signs with the
well-known Android debug key (`~/.android/debug.keystore`, created on first use) — enough to
install on your own phone, not to publish.

## How it runs

Android loads `libdemo_android.so` into a `NativeActivity` and calls `android_main`, which hands
the activity to the engine (`gizmo::app::android::set_app`) and runs the game's `App` as on the
desktop. What is different on a phone:

- **The render path is the browser's** (`gizmo_forward_only`, set by `gizmo-renderer`'s and
  `gizmo`'s build scripts): one forward pass, four bind groups, no shadows, deferred lighting,
  SSAO or TAA, and the same render-size budget. An Adreno 618's Vulkan driver allows four bind
  groups, and the desktop path asks for six.
- **The surface comes and goes.** Android destroys an app's window when it leaves the screen;
  `suspended` lets the surface go and the next `resumed` makes another on the new window, so the
  game picks up where it was.
- **Text** uses the bundled Noto Sans (`bundled-font`): the engine does not look in
  `/system/fonts`.
- **The C++ runtime** (`meshopt` is C++) is linked statically by `gizmo-renderer`'s build script.

Only `aarch64` (every phone of the last decade) is built. The launcher icon is `media/logo.png` at
Android's five densities (`res/mipmap-*/ic_launcher.png`).
