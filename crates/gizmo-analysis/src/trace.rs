//! The automatic span-capture layer (the `trace` feature).
//!
//! Captures EVERY `#[tracing::instrument]` / `span!` span in the engine automatically — including
//! the parallel and cross-thread detail collectors cannot see — and exports the record as a flame
//! chart for `chrome://tracing`/Perfetto. Setup:
//! ```
//! use gizmo_analysis::trace::{GizmoTraceLayer, TraceSink};
//! use tracing_subscriber::prelude::*;
//!
//! let sink = TraceSink::new();
//! tracing_subscriber::registry().with(GizmoTraceLayer::new(sink.clone())).init();
//!
//! // ... run the engine ...
//! tracing::info_span!("ecs_update").in_scope(|| { /* one frame */ });
//!
//! // Every span that closes becomes a record; to write it out:
//! // `std::fs::write("engine_trace.json", sink.to_chrome_trace())`.
//! assert_eq!(sink.len(), 1);
//! assert!(sink.to_chrome_trace().contains("\"name\":\"ecs_update\""));
//! ```

use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::span::Id;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// A single captured span record.
#[derive(Debug, Clone)]
pub struct TraceRecord {
    pub name: &'static str,
    pub target: String,
    /// The start, in nanoseconds since the layer's epoch.
    pub start_ns: u64,
    /// The span's wall-clock duration (first entry → close), in nanoseconds.
    pub dur_ns: u64,
    /// The thread identity (hashed).
    pub thread: u64,
}

/// The thread-safe store of span records (shared with the layer).
#[derive(Clone, Default)]
pub struct TraceSink {
    inner: Arc<Mutex<Vec<TraceRecord>>>,
}

impl TraceSink {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// A copy of every record.
    pub fn records(&self) -> Vec<TraceRecord> {
        self.inner.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.clear();
        }
    }

    fn push(&self, r: TraceRecord) {
        if let Ok(mut g) = self.inner.lock() {
            g.push(r);
        }
    }

    /// Chrome Tracing JSON (Perfetto / `chrome://tracing`).
    pub fn to_chrome_trace(&self) -> String {
        let records = self.records();
        let mut s = String::from("{\"traceEvents\":[");
        for (i, r) in records.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            crate::util::write_trace_event(
                &mut s,
                r.name,
                &r.target,
                r.thread % 100_000,
                r.start_ns as f64 / 1000.0,
                r.dur_ns as f64 / 1000.0,
                None,
            );
        }
        s.push_str("],\"displayTimeUnit\":\"ms\"}");
        s
    }
}

fn thread_id_u64() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::thread::current().id().hash(&mut h);
    h.finish()
}

/// Used to keep a span's start instant in the span's own extensions.
struct SpanStart(Instant);

/// Motorun tüm span'lerini yakalayan `tracing_subscriber::Layer`.
pub struct GizmoTraceLayer {
    sink: TraceSink,
    epoch: Instant,
}

impl GizmoTraceLayer {
    pub fn new(sink: TraceSink) -> Self {
        Self {
            sink,
            epoch: Instant::now(),
        }
    }
}

impl<S> Layer<S> for GizmoTraceLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut ext = span.extensions_mut();
            if ext.get_mut::<SpanStart>().is_none() {
                ext.insert(SpanStart(Instant::now()));
            }
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(&id) {
            let start = span.extensions_mut().remove::<SpanStart>();
            if let Some(SpanStart(t)) = start {
                let dur_ns = t.elapsed().as_nanos() as u64;
                let start_ns = t.saturating_duration_since(self.epoch).as_nanos() as u64;
                self.sink.push(TraceRecord {
                    name: span.name(),
                    target: span.metadata().target().to_string(),
                    start_ns,
                    dur_ns,
                    thread: thread_id_u64(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(name: &'static str, start_ns: u64, dur_ns: u64, thread: u64) -> TraceRecord {
        TraceRecord { name, target: "gizmo::test".to_string(), start_ns, dur_ns, thread }
    }

    #[test]
    fn empty_sink_is_empty_and_emits_valid_shell() {
        let sink = TraceSink::new();
        assert!(sink.is_empty());
        assert_eq!(sink.len(), 0);
        assert!(sink.records().is_empty());
        assert_eq!(sink.to_chrome_trace(), "{\"traceEvents\":[],\"displayTimeUnit\":\"ms\"}");
    }

    #[test]
    fn push_is_observable_and_clear_resets() {
        let sink = TraceSink::new();
        sink.push(rec("a", 0, 1000, 1));
        sink.push(rec("b", 1000, 2000, 1));
        assert_eq!(sink.len(), 2);
        assert!(!sink.is_empty());
        assert_eq!(sink.records().len(), 2);

        sink.clear();
        assert!(sink.is_empty());
        assert_eq!(sink.len(), 0);
    }

    #[test]
    fn chrome_trace_scales_ns_to_us_and_wraps_thread_id() {
        let sink = TraceSink::new();
        // start_ns=3000 → ts 3.000 us; dur_ns=1500 → dur 1.500 us; thread mod 100_000.
        sink.push(rec("solve", 3000, 1500, 123_456));
        let out = sink.to_chrome_trace();
        assert!(out.contains("\"name\":\"solve\""));
        assert!(out.contains("\"ts\":3.000"), "ns→us scale wrong: {out}");
        assert!(out.contains("\"dur\":1.500"), "ns→us scale wrong: {out}");
        assert!(out.contains("\"tid\":23456"), "thread must be reduced mod 100000: {out}");
        // Record target becomes the trace category.
        assert!(out.contains("\"cat\":\"gizmo::test\""));
    }

    #[test]
    fn chrome_trace_comma_separates_multiple_events() {
        let sink = TraceSink::new();
        sink.push(rec("x", 0, 1000, 1));
        sink.push(rec("y", 1000, 1000, 1));
        let out = sink.to_chrome_trace();
        // Two objects, one separating comma between them, wrapped in the array shell.
        assert!(out.starts_with("{\"traceEvents\":[{"));
        assert!(out.ends_with("}],\"displayTimeUnit\":\"ms\"}"));
        assert_eq!(out.matches("\"ph\":\"X\"").count(), 2, "one event per record");
        assert_eq!(out.matches("},{").count(), 1, "exactly one inter-event separator");
    }
}
