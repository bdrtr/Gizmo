//! **The engine's warnings reach the game.** No game set up a `tracing` subscriber, so every
//! `tracing::warn!` in the engine went nowhere; the app runtimes now install
//! `logger::install_console_subscriber` when nothing else has. Its own binary, because a global
//! subscriber is set once per process.

use gizmo_core::logger::{get_logs, install_console_subscriber};

#[test]
fn engine_warnings_reach_the_log_and_info_does_not() {
    if std::env::var_os("GIZMO_LOG").is_some() || std::env::var_os("RUST_LOG").is_some() {
        eprintln!("skipping: a log level is set in the environment, and this test is about the default");
        return;
    }
    assert!(install_console_subscriber(), "nothing was installed before");
    assert!(!install_console_subscriber(), "a second install leaves the first in place");

    tracing::warn!(cells = 12, "a grid clipped the level");
    tracing::error!("a texture would not load");
    tracing::info!("routine chatter");

    let (warn, error, info) = get_logs(|logs| {
        let has = |s: &str| logs.iter().any(|l| l.message.contains(s));
        (has("a grid clipped the level cells=12"), has("a texture would not load"), has("routine chatter"))
    });
    assert!(warn, "a warning, with its fields, reaches the log");
    assert!(error, "an error reaches the log");
    assert!(!info, "info stays out at the default level");
}
