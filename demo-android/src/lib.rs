//! # Gizmo on Android
//!
//! The games of `games/` as an Android app. `cargo apk build -p demo-android --release` builds an
//! APK; `cargo apk run -p demo-android --release` installs it on a phone over `adb` and starts it.
//! See `demo-android/README.md` for the tools it needs.
//!
//! Android loads this library into a `NativeActivity` and calls [`android_main`]; on the desktop
//! the crate is empty.

/// The app's entry point: Android calls it on the activity's thread.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: gizmo::app::android::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("gizmo"));
    gizmo::app::android::set_app(app);
    if let Err(e) = games::pinbol::app().run() {
        log::error!("pinbol could not start: {e}");
    }
}
