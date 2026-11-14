//! # Satranç on Android
//!
//! `satranc` (`games::satranc`) as an APK: `scripts/android-apk.sh satranc`. The host is the same
//! as `demo-android`'s, which has the details. A tap is a click: the first finger is the mouse.

/// The app's entry point: Android calls it on the activity's thread.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: gizmo::app::android::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("gizmo"));
    gizmo::app::android::set_app(app);
    if let Err(e) = games::satranc::app().run() {
        log::error!("satranc could not start: {e}");
    }
}
