//! The app's half of hot-patching: the connection to the Dioxus CLI. `gizmo-core` calls game code
//! through subsecond's jump table (`gizmo_core::hotpatch`); this is what fills the table, from the CLI
//! that rebuilt what changed. The CLI says on its own console when a patch lands.

/// Connects to the CLI that started the game, once per process. Without the `hotpatching` feature, on
/// the browser, or when the game was not started by `dx serve --hot-patch`, it does nothing.
pub(crate) fn connect() {
    #[cfg(all(feature = "hotpatching", not(target_arch = "wasm32")))]
    {
        static CONNECTED: std::sync::Once = std::sync::Once::new();
        CONNECTED.call_once(dioxus_devtools::connect_subsecond);
    }
}
