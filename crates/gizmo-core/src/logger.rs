//! Process-global in-memory log buffer, plus the optional `tracing` bridge that fills it.
//!
//! Records enter through the [`gizmo_log!`](crate::gizmo_log) macro. With the default-off
//! `tracing-layer` feature they can also enter through any `tracing` event the subscriber's
//! filter admits, once `init_tracing` has installed `GizmoTracingLayer` — `init_tracing` adds
//! its `EnvFilter` as a plain layer, which filters the whole stack, so an event that filter
//! rejects never reaches the layer. Storage is a single mutex-guarded `static` holding at
//! most 2048 entries; the oldest is evicted when full.
//!
//! Consequences of that being a `static` rather than a `World` resource: the buffer is shared
//! by every `World`, every plugin and every test in the process, it outlives any one of them,
//! and concurrent emitters interleave — so a caller can never assume the entry it just wrote
//! is the last one, and tests asserting on counts must serialise against each other.
//!
//! # Why the subscriber bridge is a feature and `tracing` itself is not
//!
//! `tracing` 0.1 is frozen — the `1.0`-in-all-but-name logging facade — and neither
//! `tracing::info!` nor `#[tracing::instrument]` puts a `tracing` type in any signature of
//! ours, so depending on it costs this crate's semver contract nothing. `tracing-subscriber`
//! 0.3 is the opposite on both counts: it moves, and the integration is *an impl of its
//! `Layer` trait on our public type*, which cannot be hidden — the whole point is to pass the
//! layer to `registry().with(..)`. A trait impl is as much a part of the public surface as a
//! public field, so a `tracing-subscriber` 0.4 would be a breaking change to `gizmo-core`.
//! Hence the split: `tracing` is an unconditional dependency, the subscriber integration is
//! opt-in behind `tracing-layer`, exactly as `bevy_reflect` sits behind `reflect`.

// Regression test for that seal, and it only makes sense in the configuration it describes —
// with `tracing-layer` ON the type is *supposed* to resolve, so the example is emitted only
// when the feature is off. Before the feature existed the type was unconditional and this
// example compiled, which is what makes it evidence rather than a restatement. Verified by
// hand (rustdoc ignores `compile_fail,E0nnn` codes on this toolchain): the diagnostic is
// E0425, cannot find value `GizmoTracingLayer` in module `gizmo_core::logger`.
#![cfg_attr(
    not(feature = "tracing-layer"),
    doc = r#"
# Default surface

Without `tracing-layer`, `tracing-subscriber` is not even a dependency and the bridge does not
exist:

```compile_fail
let _layer = gizmo_core::logger::GizmoTracingLayer;
```
"#
)]

use std::sync::Mutex;

/// Log level.
///
/// Severity increases with declaration order and the minimum-level filter compares the
/// variants *by discriminant* (`level as u8`), not through `Ord` — which is not derived.
/// Reordering these variants therefore silently changes which records are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogLevel {
    /// Routine progress. Discriminant 0, the lowest, so the default minimum level of `Info`
    /// admits everything. Printed to stdout.
    Info,
    /// A recoverable problem worth surfacing. Discriminant 1, which makes it the only useful
    /// middle threshold: `set_min_log_level(LogLevel::Warning)` is how a release build drops
    /// routine `Info` traffic while still keeping this and `Error`. Printed to stderr with a
    /// `[WARN]` tag, and it is the level `tracing::Level::WARN` maps to when records arrive
    /// through `GizmoTracingLayer` (the `tracing-layer` feature).
    Warning,
    /// A failure. Discriminant 2, the highest, so setting the minimum level to `Error`
    /// suppresses every other kind. Printed to stderr.
    Error,
}

/// A single log record.
#[derive(Clone)]
pub struct LogEntry {
    /// The already-formatted message body: `format!` arguments are expanded at emit time and
    /// the level, timestamp and source location are *not* prefixed onto it — those live in
    /// the sibling fields, so a consumer can lay them out however it likes.
    pub message: String,
    /// Severity this record was emitted at.
    ///
    /// It passed the minimum level that was in force *at emit time*. Since that threshold is
    /// global mutable state, a buffer read later can legitimately contain entries below the
    /// current minimum — filter again on read if you need consistency.
    pub level: LogLevel,
    /// Wall-clock time of emission, formatted `HH:MM:SS` — local time on native targets, UTC
    /// on wasm. Display only: there is no date, no sub-second component and no zone marker,
    /// so it is not sortable across a midnight boundary and not comparable between records
    /// produced on different targets.
    pub timestamp: String,
    /// Source file path (compile-time).
    pub file: &'static str,
    /// Source line number (compile-time).
    pub line: u32,
}

/// Maximum log capacity — behaves like a ring buffer.
const MAX_LOG_ENTRIES: usize = 2048;

/// Minimum log level — logs below this level are not recorded.
/// This value can be changed in order to suppress Info logs in a release build.
static MIN_LOG_LEVEL: Mutex<LogLevel> = Mutex::new(LogLevel::Info);

// Global logger. Mutex poisoning durumunda into_inner() ile kurtarma yapılır.
static GLOBAL_LOGS: Mutex<Vec<LogEntry>> = Mutex::new(Vec::new());
static LOG_VERSION: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Returns a version number for telling whether the logs have changed
pub fn log_version() -> usize {
    LOG_VERSION.load(std::sync::atomic::Ordering::Relaxed)
}

/// Helper that takes the mutex lock safely — recovers the data even if it is poisoned.
fn lock_logs() -> std::sync::MutexGuard<'static, Vec<LogEntry>> {
    match GLOBAL_LOGS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            // Bir thread panic yaptıysa bile log verisini kurtar
            eprintln!("[Logger] Mutex poisoned — veri kurtarılıyor");
            poisoned.into_inner()
        }
    }
}

fn lock_min_level() -> std::sync::MutexGuard<'static, LogLevel> {
    match MIN_LOG_LEVEL.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Adds a log record. **Do not call directly** — use the `gizmo_log!` macro.
#[doc(hidden)]
pub fn log_message(level: LogLevel, msg: String, file: &'static str, line: u32) {
    // Seviye filtresi
    let min_level = *lock_min_level();
    if (level as u8) < (min_level as u8) {
        return;
    }

    let mut logs = lock_logs();

    // Ring buffer: kapasiteyi aşarsa en eski log silinir
    if logs.len() >= MAX_LOG_ENTRIES {
        logs.remove(0);
    }

    #[cfg(target_arch = "wasm32")]
    let timestamp = {
        let now = web_time::SystemTime::now();
        let duration = now.duration_since(web_time::SystemTime::UNIX_EPOCH).unwrap_or_default();
        let secs = duration.as_secs();
        let mins = (secs / 60) % 60;
        let hours = (secs / 3600) % 24;
        let secs_of_min = secs % 60;
        format!("{:02}:{:02}:{:02}", hours, mins, secs_of_min)
    };
    #[cfg(not(target_arch = "wasm32"))]
    let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();

    logs.push(LogEntry {
        message: msg.clone(),
        level,
        timestamp: timestamp.clone(),
        file,
        line,
    });

    LOG_VERSION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    // Konsol çıktısı — Warning ve Error stderr'e gider
    match level {
        LogLevel::Info => println!("[{}] [INFO]  {}:{} — {}", timestamp, file, line, msg),
        LogLevel::Warning => eprintln!("[{}] [WARN]  {}:{} — {}", timestamp, file, line, msg),
        LogLevel::Error => eprintln!("[{}] [ERROR] {}:{} — {}", timestamp, file, line, msg),
    }
}

// ──── Public API ────

/// Takes a snapshot of all the logs (for reading).
/// Consumers such as the editor console should use this function.
pub fn get_logs<F, R>(f: F) -> R
where
    F: FnOnce(&[LogEntry]) -> R,
{
    let logs = lock_logs();
    f(&logs)
}

/// Clears all log records.
pub fn clear_logs() {
    lock_logs().clear();
    LOG_VERSION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// Takes all log records and deletes them from the queue (drain).
pub fn drain_logs() -> Vec<LogEntry> {
    let drained = lock_logs().drain(..).collect();
    LOG_VERSION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    drained
}

/// Returns the log entry count.
pub fn log_count() -> usize {
    lock_logs().len()
}

/// Sets the minimum log level.
/// Logs below this level are not recorded and are not written to the console.
pub fn set_min_log_level(level: LogLevel) {
    *lock_min_level() = level;
}

/// Global Logger Macro — source location information is added automatically.
///
/// # Usage
/// ```
/// use gizmo_core::gizmo_log;
/// use gizmo_core::logger::{get_logs, LogLevel};
/// # let subsystem = "renderer";
/// # let fps = 12.34_f32;
/// # let path = "assets/level.ron";
/// gizmo_log!(Info, "subsystem started: {}", subsystem);
/// gizmo_log!(Warning, "FPS low: {:.1}", fps);
/// gizmo_log!(Error, "file not found: {}", path);
///
/// // The message is formatted at emit time; the level and the call site's file/line live in
/// // separate fields.
/// let entry = get_logs(|entries| {
///     entries.iter().find(|e| e.message == "FPS low: 12.3").cloned()
/// })
/// .expect("the entry is in the global buffer");
/// assert_eq!(entry.level, LogLevel::Warning);
/// assert_eq!(entry.file, file!());
/// ```
#[macro_export]
macro_rules! gizmo_log {
    (Info, $($arg:tt)*) => {
        $crate::logger::log_message(
            $crate::logger::LogLevel::Info,
            format!($($arg)*),
            file!(), line!()
        )
    };
    (Warning, $($arg:tt)*) => {
        $crate::logger::log_message(
            $crate::logger::LogLevel::Warning,
            format!($($arg)*),
            file!(), line!()
        )
    };
    (Error, $($arg:tt)*) => {
        $crate::logger::log_message(
            $crate::logger::LogLevel::Error,
            format!($($arg)*),
            file!(), line!()
        )
    };
}

// ──── `tracing-subscriber` bridge — opt-in, see the module docs for why ────

#[cfg(feature = "tracing-layer")]
use tracing_subscriber::Layer;
#[cfg(feature = "tracing-layer")]
use tracing::Subscriber;
#[cfg(feature = "tracing-layer")]
use tracing_subscriber::layer::Context;
#[cfg(feature = "tracing-layer")]
use tracing_subscriber::registry::LookupSpan;

/// A custom tracing layer that forwards tracing events to Gizmo's internal UI logger.
///
/// Requires the default-off `tracing-layer` feature: it exists only to carry the
/// `tracing_subscriber::Layer` impl below, and that impl is what would drag a `0.x` crate
/// into this crate's semver contract if it were unconditional.
#[cfg(feature = "tracing-layer")]
pub struct GizmoTracingLayer;

#[cfg(feature = "tracing-layer")]
impl<S> Layer<S> for GizmoTracingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let level = match *meta.level() {
            tracing::Level::ERROR => LogLevel::Error,
            tracing::Level::WARN => LogLevel::Warning,
            _ => LogLevel::Info, // Map TRACE, DEBUG, INFO to Info
        };

        struct EventVisitor(String);
        impl tracing::field::Visit for EventVisitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = format!("{:?}", value);
                } else {
                    self.0.push_str(&format!(" {}={:?}", field.name(), value));
                }
            }
        }

        let mut visitor = EventVisitor(String::new());
        event.record(&mut visitor);

        // Remove quotes around the message if it was formatted as a debug string
        let mut msg = visitor.0;
        if msg.starts_with('"') && msg.ends_with('"') {
            msg = msg[1..msg.len()-1].to_string();
        }

        log_message(level, msg, meta.file().unwrap_or("unknown"), meta.line().unwrap_or(0));
    }
}

/// Initializes the global tracing subscriber with the Gizmo UI logger and console output.
///
/// Requires the default-off `tracing-layer` feature — it builds a `tracing_subscriber`
/// registry, so it cannot exist when that crate is not linked. Callers that only want the
/// in-memory buffer need nothing: `gizmo_log!` fills it either way.
#[cfg(feature = "tracing-layer")]
pub fn init_tracing() {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    // Use RUST_LOG environment variable if set, otherwise default to info
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,wgpu=warn,naga=warn,gizmo_core=debug"));

    // Set up standard console output for tracing
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .without_time();

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(GizmoTracingLayer);

    // Try to set global default. This might fail if another test/part already initialized tracing
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Clear the logs before every test
    fn setup() -> MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().expect("logger test lock poisoned");
        clear_logs();
        set_min_log_level(LogLevel::Info);
        guard
    }

    #[test]
    fn test_log_and_read() {
        let _guard = setup();
        log_message(LogLevel::Info, "test mesajı".into(), "test.rs", 1);

        get_logs(|logs| {
            assert_eq!(logs.len(), 1);
            assert_eq!(logs[0].message, "test mesajı");
            assert_eq!(logs[0].level, LogLevel::Info);
            assert_eq!(logs[0].file, "test.rs");
            assert_eq!(logs[0].line, 1);
        });
    }

    #[test]
    fn test_drain_clears() {
        let _guard = setup();
        log_message(LogLevel::Warning, "w1".into(), "test.rs", 10);
        log_message(LogLevel::Error, "e1".into(), "test.rs", 20);

        let drained = drain_logs();
        assert_eq!(drained.len(), 2);
        assert_eq!(log_count(), 0);
    }

    #[test]
    fn test_clear_logs() {
        let _guard = setup();
        log_message(LogLevel::Info, "clear me".into(), "test.rs", 1);
        assert_eq!(log_count(), 1);

        clear_logs();
        assert_eq!(log_count(), 0);
    }

    #[test]
    fn test_ring_buffer_capacity() {
        let _guard = setup();
        // Kapasiteyi aşacak kadar log yaz
        for i in 0..MAX_LOG_ENTRIES + 500 {
            log_message(
                LogLevel::Info,
                format!("cap_test_{}", i),
                "test.rs",
                i as u32,
            );
        }

        let count = log_count();
        // Paralel testler de log ekleyebilir, bu yüzden tam MAX_LOG_ENTRIES olmayabilir
        // ama asla aşmamalı
        assert!(
            count <= MAX_LOG_ENTRIES,
            "ring buffer kapasitesi aşıldı: {} > {}",
            count,
            MAX_LOG_ENTRIES
        );
    }

    #[test]
    fn test_min_level_filter() {
        let _guard = setup();
        set_min_log_level(LogLevel::Warning);

        log_message(LogLevel::Info, "filtered".into(), "test.rs", 1);
        assert_eq!(log_count(), 0, "Info filtrelenmeli");

        log_message(LogLevel::Warning, "kept".into(), "test.rs", 2);
        assert_eq!(log_count(), 1, "Warning geçmeli");

        log_message(LogLevel::Error, "also kept".into(), "test.rs", 3);
        assert_eq!(log_count(), 2, "Error geçmeli");
    }

    #[test]
    fn test_log_level_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(LogLevel::Info);
        set.insert(LogLevel::Warning);
        set.insert(LogLevel::Error);
        assert_eq!(set.len(), 3);
    }

    /// The positive half of the `tracing-layer` seal. The negative half — that the bridge is
    /// absent under the default feature set — is the `compile_fail` doc-test in the module
    /// header, which only exists when the feature is off.
    #[cfg(feature = "tracing-layer")]
    mod tracing_layer {
        use super::*;

        /// Static assertion rather than a behavioural test: with the feature on,
        /// `GizmoTracingLayer` must still satisfy the bound `registry().with(..)` needs.
        /// This is what catches gating *too much* — a `#[cfg]` in the wrong place would
        /// leave the type but lose the impl, and nothing else in this crate would notice.
        #[test]
        fn layer_impl_is_present_for_the_registry() {
            fn assert_layer<L, S>()
            where
                L: tracing_subscriber::Layer<S>,
                S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
            {
            }
            assert_layer::<GizmoTracingLayer, tracing_subscriber::Registry>();
        }

        /// Guard: an admitted event still lands in the global buffer, message and level
        /// intact. Scoped with `with_default` so it does not install a process-global
        /// subscriber and perturb the rest of the suite.
        #[test]
        fn events_reach_the_global_buffer() {
            use tracing_subscriber::layer::SubscriberExt;

            let _guard = setup();
            let subscriber = tracing_subscriber::registry().with(GizmoTracingLayer);
            tracing::subscriber::with_default(subscriber, || {
                tracing::warn!("bridged event");
            });

            let hit = get_logs(|logs| {
                logs.iter()
                    .any(|e| e.message == "bridged event" && e.level == LogLevel::Warning)
            });
            assert!(hit, "the layer forwarded the event into the global buffer");
        }
    }
}
