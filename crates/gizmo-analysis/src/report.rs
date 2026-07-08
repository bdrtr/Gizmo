//! Dışa aktarım — insan-okunur metin, JSON, CSV zaman-serisi ve Chrome-trace.
//!
//! Chrome-trace çıktısı `chrome://tracing` veya Perfetto'da açılıp her span'in en ufak
//! ayrıntısına zoom yapılabilir bir alev-grafiği (flame chart) verir.

use crate::analyzer::Analyzer;
use crate::metrics::MetricKind;
use crate::util::{human_bytes, json_escape};
use gizmo_core::world::short_type_name;
use std::fmt::Write as _;

/// Sonlu olmayan değerleri JSON-güvenli hale getir (null yerine 0).
fn json_num(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

impl Analyzer {
    /// Konsol/HUD için özet metin raporu.
    pub fn report_text(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "╔══ Gizmo Analysis ── frame {} ──", self.frame());
        let _ = writeln!(
            s,
            "║ FPS ~{:.1} | frame {:.3} ms",
            self.estimated_fps(),
            self.stats("frame_ms").map(|x| x.mean).unwrap_or(0.0)
        );

        if let Some(fs) = self.stats("frame_ms") {
            let _ = writeln!(
                s,
                "║ frame_ms: min {:.2} | p50 {:.2} | p95 {:.2} | p99 {:.2} | max {:.2}  (n={})",
                fs.min, fs.p50, fs.p95, fs.p99, fs.max, fs.count
            );
        }

        if let Some(last) = self.last() {
            let e = &last.ecs;
            let _ = writeln!(
                s,
                "║ ECS: {} entities | {} archetypes ({} non-empty) | {} component types | {} resources | {}",
                e.entities,
                e.archetypes,
                e.non_empty_archetypes,
                e.registered_components,
                e.resources,
                human_bytes(e.component_bytes),
            );

            // En kalabalık archetype'lar.
            if !last.archetypes.is_empty() {
                let _ = writeln!(s, "║ ── top archetypes (by entities) ──");
                for a in last.archetypes.iter().take(6) {
                    let names: Vec<&str> = a
                        .components
                        .iter()
                        .map(|c| short_type_name(c.name))
                        .collect();
                    let _ = writeln!(
                        s,
                        "║   #{:<3} {:>7} ent  {:>9}  [{}]",
                        a.id,
                        a.entity_count,
                        human_bytes(a.bytes),
                        names.join(", ")
                    );
                }
            }

            // En pahalı span'ler.
            if !last.spans.is_empty() {
                let mut spans = last.spans.clone();
                spans.sort_by(|x, y| y.ms.partial_cmp(&x.ms).unwrap_or(std::cmp::Ordering::Equal));
                let _ = writeln!(s, "║ ── hottest spans (this frame) ──");
                for sp in spans.iter().take(6) {
                    let _ = writeln!(
                        s,
                        "║   {}{:<24} {:>8.3} ms",
                        "  ".repeat(sp.depth as usize),
                        short_type_name(sp.name),
                        sp.ms
                    );
                }
            }

            // Collector metrik grupları.
            for (group, entries) in &last.groups {
                if group == "ecs" {
                    continue; // yukarıda özetlendi
                }
                let _ = write!(s, "║ [{group}] ");
                let parts: Vec<String> = entries
                    .iter()
                    .map(|(n, v)| format!("{n}={v:.3}"))
                    .collect();
                let _ = writeln!(s, "{}", parts.join("  "));
            }
        }
        let _ = writeln!(s, "╚═════════════════════════════════════");
        s
    }

    /// Son snapshot + tüm metrik istatistiklerini JSON olarak (bağımlılıksız, elle).
    pub fn to_json(&self) -> String {
        let mut s = String::new();
        s.push('{');
        let _ = write!(s, "\"frame\":{}", self.frame());
        let _ = write!(s, ",\"estimated_fps\":{}", json_num(self.estimated_fps()));

        // ECS + son frame.
        if let Some(last) = self.last() {
            let e = &last.ecs;
            let _ = write!(
                s,
                ",\"ecs\":{{\"entities\":{},\"archetypes\":{},\"non_empty_archetypes\":{},\"registered_components\":{},\"sparse_set_components\":{},\"resources\":{},\"component_bytes\":{},\"tick\":{}}}",
                e.entities, e.archetypes, e.non_empty_archetypes, e.registered_components,
                e.sparse_set_components, e.resources, e.component_bytes, e.tick
            );

            // Archetype tablosu.
            s.push_str(",\"archetypes\":[");
            for (i, a) in last.archetypes.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                let _ = write!(
                    s,
                    "{{\"id\":{},\"entities\":{},\"bytes\":{},\"components\":[",
                    a.id, a.entity_count, a.bytes
                );
                for (j, c) in a.components.iter().enumerate() {
                    if j > 0 {
                        s.push(',');
                    }
                    let _ = write!(
                        s,
                        "{{\"name\":\"{}\",\"item_size\":{},\"count\":{},\"bytes\":{}}}",
                        json_escape(c.name),
                        c.item_size,
                        c.count,
                        c.bytes
                    );
                }
                s.push_str("]}");
            }
            s.push(']');

            // Collector metrik grupları.
            s.push_str(",\"groups\":{");
            for (gi, (group, entries)) in last.groups.iter().enumerate() {
                if gi > 0 {
                    s.push(',');
                }
                let _ = write!(s, "\"{}\":{{", json_escape(group));
                for (ei, (name, value)) in entries.iter().enumerate() {
                    if ei > 0 {
                        s.push(',');
                    }
                    let _ = write!(s, "\"{}\":{}", json_escape(name), json_num(*value));
                }
                s.push('}');
            }
            s.push('}');
        }

        // Metrik istatistikleri.
        s.push_str(",\"metrics\":{");
        for (i, (name, series)) in self.metrics().iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let st = series.stats();
            let _ = write!(
                s,
                "\"{}\":{{\"kind\":\"{}\",\"count\":{},\"last\":{},\"min\":{},\"max\":{},\"mean\":{},\"stddev\":{},\"p50\":{},\"p95\":{},\"p99\":{}}}",
                json_escape(name),
                series.kind.as_str(),
                st.count,
                json_num(st.last),
                json_num(st.min),
                json_num(st.max),
                json_num(st.mean),
                json_num(st.stddev),
                json_num(st.p50),
                json_num(st.p95),
                json_num(st.p99),
            );
        }
        s.push('}');

        s.push('}');
        s
    }

    /// Frame-indeksli CSV zaman-serisi (çekirdek sütunlar). Excel/pandas/gnuplot için.
    pub fn to_csv(&self) -> String {
        let mut s = String::new();
        s.push_str("frame,timestamp_ms,frame_ms,entities,archetypes,component_bytes,resources\n");
        for f in self.history() {
            let _ = writeln!(
                s,
                "{},{:.3},{:.4},{},{},{},{}",
                f.frame,
                f.timestamp_ns as f64 / 1_000_000.0,
                f.frame_ms,
                f.ecs.entities,
                f.ecs.archetypes,
                f.ecs.component_bytes,
                f.ecs.resources,
            );
        }
        s
    }

    /// Chrome Tracing JSON — geçmişteki her span bir "complete" (ph:"X") olay olur.
    /// `chrome://tracing` veya `ui.perfetto.dev` ile açılıp zoom yapılabilir.
    pub fn to_chrome_trace(&self) -> String {
        let mut s = String::new();
        s.push_str("{\"traceEvents\":[");
        let mut first = true;
        let mut args = String::new();
        for f in self.history() {
            for sp in &f.spans {
                if !first {
                    s.push(',');
                }
                first = false;
                let ts_us = sp.start_ns as f64 / 1000.0;
                let dur_us = (sp.end_ns.saturating_sub(sp.start_ns)) as f64 / 1000.0;
                args.clear();
                let _ = write!(args, "\"frame\":{}", f.frame);
                crate::util::write_trace_event(&mut s, sp.name, "gizmo", 1, ts_us, dur_us, Some(&args));
            }
        }
        s.push_str("],\"displayTimeUnit\":\"ms\"}");
        s
    }

    /// Belirli bir metrik türündeki tüm serilerin adları (filtreli sorgu kolaylığı).
    pub fn metric_names_of_kind(&self, kind: MetricKind) -> Vec<&str> {
        self.metrics()
            .iter()
            .filter(|(_, s)| s.kind == kind)
            .map(|(n, _)| n)
            .collect()
    }
}
