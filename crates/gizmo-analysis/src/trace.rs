//! Otomatik span yakalama katmanı (`trace` özelliği).
//!
//! Motorun HER `#[tracing::instrument]` / `span!` span'ini otomatik yakalar — collector'ların
//! göremediği paralel/çapraz-thread ayrıntıyı da. Kaydı `chrome://tracing`/Perfetto için
//! alev-grafiğine aktarır. Kurulum:
//! ```ignore
//! use gizmo_analysis::trace::{GizmoTraceLayer, TraceSink};
//! use tracing_subscriber::prelude::*;
//! let sink = TraceSink::new();
//! tracing_subscriber::registry().with(GizmoTraceLayer::new(sink.clone())).init();
//! // ... motoru çalıştır ...
//! std::fs::write("engine_trace.json", sink.to_chrome_trace()).unwrap();
//! ```

use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::span::Id;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// Yakalanan tek bir span kaydı.
#[derive(Debug, Clone)]
pub struct TraceRecord {
    pub name: &'static str,
    pub target: String,
    /// Layer epoch'undan itibaren başlangıç (ns).
    pub start_ns: u64,
    /// Span'in duvar-saati süresi (ilk giriş → kapanış), ns.
    pub dur_ns: u64,
    /// Thread kimliği (hash).
    pub thread: u64,
}

/// Thread-güvenli span kaydı deposu (Layer ile paylaşılır).
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

    /// Tüm kayıtların kopyası.
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

/// Span başlangıç anını span'in extension'ında saklamak için.
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
