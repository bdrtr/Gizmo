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

/// Shows or hides the status and navigation bars — a tablet's taskbar with them — over the
/// game. Hidden, a swipe from the edge shows them for a moment and they go again by themselves.
///
/// What [`WindowMode`](gizmo_core::window::WindowMode) does on Android: `Fullscreen` hides them,
/// `Windowed` shows them. winit's own fullscreen does nothing there, so a game asking for it kept
/// the bars, and on a tablet the taskbar covered the bottom of the game.
///
/// The bars are the window's views, which Android lets only its UI thread touch, and the engine
/// runs on its own thread; so this hands the activity a `Runnable` to run there
/// (`Activity.runOnUiThread`). The `Runnable` is a class of ours, `com.bdrtr.gizmo.SystemBars`
/// (`android/SystemBars.java`), carried in the library as dex and loaded from memory
/// (`InMemoryDexClassLoader`, API 26 — the apps' minimum). Rebuild it with `scripts/android-dex.sh`.
///
/// Does nothing before [`set_app`]. A failure is logged, never raised: the game runs on with
/// the bars.
pub(crate) fn set_system_bars(visible: bool) {
    let Some(app) = app() else { return };
    if let Err(e) = run_system_bars(&app, visible) {
        tracing::warn!(error = %e, visible, "[Android] the system bars could not be changed");
    }
}

/// The dex of `SystemBars.java`.
static SYSTEM_BARS_DEX: &[u8] = include_bytes!("../android/system_bars.dex");

/// The loaded `SystemBars` class, kept for the process: its class loader holds the dex.
static SYSTEM_BARS_CLASS: OnceLock<jni::objects::GlobalRef> = OnceLock::new();

fn run_system_bars(app: &AndroidApp, visible: bool) -> Result<(), String> {
    use jni::objects::{JClass, JObject, JValue};
    // SAFETY: android-activity hands out the process's `JavaVM*` and the activity's global
    // `jobject`; both live as long as the app.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.map_err(|e| e.to_string())?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    // SAFETY: the activity's global reference, from the same handle; borrowed, never deleted
    // (`JObject` frees nothing when dropped).
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let result = (|| -> jni::errors::Result<()> {
        let class = match SYSTEM_BARS_CLASS.get() {
            Some(class) => class,
            None => {
                // SAFETY: the buffer is the static dex, alive for the process; Java only reads it.
                let dex = unsafe { env.new_direct_byte_buffer(SYSTEM_BARS_DEX.as_ptr().cast_mut(), SYSTEM_BARS_DEX.len()) }?;
                let parent = env.call_method(&activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
                let loader = env.new_object(
                    "dalvik/system/InMemoryDexClassLoader",
                    "(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V",
                    &[JValue::Object(&dex), JValue::Object(&parent)],
                )?;
                let name = env.new_string("com.bdrtr.gizmo.SystemBars")?;
                let class = env
                    .call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])?
                    .l()?;
                let global = env.new_global_ref(class)?;
                let _ = SYSTEM_BARS_CLASS.set(global);
                SYSTEM_BARS_CLASS.get().expect("just set")
            }
        };
        let class = JClass::from(env.new_local_ref(class.as_obj())?);
        let runnable = env.new_object(&class, "(Landroid/app/Activity;Z)V", &[JValue::Object(&activity), JValue::Bool(visible.into())])?;
        env.call_method(&activity, "runOnUiThread", "(Ljava/lang/Runnable;)V", &[JValue::Object(&runnable)])?;
        Ok(())
    })();
    if let Err(e) = result {
        // A Java exception stays pending on the thread until cleared, and the next JNI call
        // would abort the process.
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        return Err(e.to_string());
    }
    Ok(())
}
