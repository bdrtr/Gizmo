//! Running on Android.
//!
//! An Android app is a library the system loads into an activity; its entry point is
//! `android_main(app: AndroidApp)`, not `main`. winit can build its event loop there only from that
//! `AndroidApp`, and `App::run` is where the loop is built — so the host passes the handle over
//! first:
//!
//! ```ignore
//! #[no_mangle]
//! fn android_main(app: gizmo::app::android::AndroidApp) {
//!     gizmo::app::android::set_app(app);
//!     my_game::app().run().expect("the game could not start");
//! }
//! ```

use std::sync::OnceLock;

pub use winit::platform::android::activity::AndroidApp;

static APP: OnceLock<AndroidApp> = OnceLock::new();

/// Hands the activity to the engine. Call it once, before `App::run`; a second call is ignored
/// (the process has one activity).
pub fn set_app(app: AndroidApp) {
    let _ = APP.set(app);
}

/// The activity [`set_app`] was given.
#[must_use]
pub fn app() -> Option<AndroidApp> {
    APP.get().cloned()
}
