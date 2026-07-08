//! Küçük paylaşılan yardımcılar — biçimlendirme + JSON/Chrome-trace serileştirme.
//! (report / panel / trace modülleri arasında tek kaynak olsun diye buradadır.)

use std::fmt::Write as _;

/// İnsan dostu bayt biçimi.
pub(crate) fn human_bytes(b: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let f = b as f64;
    if f >= MB {
        format!("{:.2} MB", f / MB)
    } else if f >= KB {
        format!("{:.1} KB", f / KB)
    } else {
        format!("{b} B")
    }
}

/// JSON string kaçışı (yalnız gerekli karakterler + kontrol karakterleri).
pub(crate) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Tek bir Chrome-trace "complete" (`ph:"X"`) olayını `out`'a yazar. `args` verilirse
/// ham JSON gövdesi olarak eklenir (ör. `"frame":3`). İsim ve kategori kaçışlanır.
pub(crate) fn write_trace_event(
    out: &mut String,
    name: &str,
    cat: &str,
    tid: u64,
    ts_us: f64,
    dur_us: f64,
    args: Option<&str>,
) {
    let _ = write!(
        out,
        "{{\"name\":\"{}\",\"cat\":\"{}\",\"ph\":\"X\",\"pid\":1,\"tid\":{},\"ts\":{:.3},\"dur\":{:.3}",
        json_escape(name),
        json_escape(cat),
        tid,
        ts_us,
        dur_us,
    );
    if let Some(a) = args {
        let _ = write!(out, ",\"args\":{{{a}}}");
    }
    out.push('}');
}
