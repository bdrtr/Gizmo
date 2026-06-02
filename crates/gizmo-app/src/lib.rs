#[cfg(feature = "editor")]
pub mod dev_console;
pub mod plugin;

pub use plugin::Plugin;

#[cfg(feature = "window")]
pub mod windowed;
#[cfg(feature = "window")]
pub use windowed::*;

#[cfg(not(feature = "window"))]
pub mod headless;
#[cfg(not(feature = "window"))]
pub use headless::*;

pub fn setup_panic_hook() {
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        let _ = console_log::init_with_level(log::Level::Debug);
        let _ = tracing_wasm::try_set_as_global_default();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::panic::set_hook(Box::new(|panic_info| {
            let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
                *s
            } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
                s.as_str()
            } else {
                "Bilinmeyen hata"
            };

            let location = if let Some(loc) = panic_info.location() {
                format!("{}:{}", loc.file(), loc.line())
            } else {
                "Bilinmeyen konum".to_string()
            };

            let error_msg = format!("Gizmo Engine Coktu!\n\nKonum: {}\nHata: {}\n", location, message);
            tracing::error!("{}", error_msg);

            #[cfg(feature = "window")]
            {
                let backtrace = backtrace::Backtrace::new();
                tracing::info!("--- BACKTRACE ---\n{:?}", backtrace);
                rfd::MessageDialog::new()
                    .set_title("Gizmo Engine Fatal Error")
                    .set_description(&error_msg)
                    .set_level(rfd::MessageLevel::Error)
                    .show();
            }
        }));
    }
}
